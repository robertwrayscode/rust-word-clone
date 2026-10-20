//! The window shell: ribbon, ruler, navigation pane, side panes, status bar, and the eframe loop.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use egui::{pos2, vec2, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, Ui};
use wordcore::{Story, StyleKind};

use crate::app::{App, MarkupMode, Pending, RibbonTab, SidePanel, ZoomMode};
use crate::dialogs::Dialog;
use crate::theme;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame_no += 1;

        if self.frame_no == 3 {
            let actions = std::mem::take(&mut self.args.actions);
            for a in &actions {
                run_action(self, a);
            }
        }

        // Drag and drop to open.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if let Some(p) = dropped.into_iter().next() {
            let is_image = p.extension().and_then(|e| e.to_str()).map(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif" | "bmp")).unwrap_or(false);
            if is_image {
                if let Ok(img) = crate::images::load_file(&p) {
                    self.insert_image(img, "Insert Picture");
                }
            } else {
                self.request(Pending::Open(Some(p)));
            }
        }

        // Documents opened from Finder (macOS).
        for p in crate::macos_open::take() {
            self.request(Pending::Open(Some(p)));
        }

        // Window close.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            if self.modified {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if !matches!(self.dialog, Some(Dialog::SaveChanges)) {
                    self.request(Pending::Close);
                }
            } else {
                self.allow_close = true;
            }
        }
        if self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        global_shortcuts(self, &ctx);

        egui::Panel::top("ribbon")
            .frame(egui::Frame::new().fill(theme::PANEL_BG).inner_margin(egui::Margin::ZERO).stroke(Stroke::new(1.0, theme::BORDER)))
            .show(ui, |ui| crate::ribbon::show(self, ui));
        egui::Panel::bottom("status")
            .exact_size(26.0)
            .frame(egui::Frame::new().fill(theme::ACCENT).inner_margin(egui::Margin::symmetric(10, 2)))
            .show(ui, |ui| status_bar(self, ui));
        if self.show_nav {
            egui::Panel::left("nav")
                .default_size(250.0)
                .size_range(180.0..=420.0)
                .resizable(true)
                .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(egui::Margin::same(10)).stroke(Stroke::new(1.0, theme::BORDER)))
                .show(ui, |ui| nav_pane(self, ui));
        }
        if self.side_panel.is_some() {
            egui::Panel::right("side")
                .default_size(270.0)
                .size_range(200.0..=460.0)
                .resizable(true)
                .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(egui::Margin::same(10)).stroke(Stroke::new(1.0, theme::BORDER)))
                .show(ui, |ui| side_pane(self, ui));
        }
        egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(theme::CANVAS_BG)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            apply_zoom_mode(self, ui.available_size());
            if self.show_ruler {
                ruler(self, ui);
            }
            crate::canvas::show(self, ui);
        });

        crate::dialogs::show(self, &ctx);

        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.title()));

        // Automated screenshot (testing aid).
        if let Some(s) = &mut self.screenshot {
            if s.done.load(Ordering::SeqCst) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                self.allow_close = true;
            } else if !s.requested && self.frame_no >= 8 && ctx.input(|i| i.time) > 1.5 {
                s.requested = true;
                let path = s.path.clone();
                let done = s.done.clone();
                let c2 = ctx.clone();
                ctx.request_screenshot(move |img| {
                    let mut rgba = Vec::with_capacity(img.pixels.len() * 4);
                    for p in &img.pixels {
                        rgba.extend_from_slice(&p.to_srgba_unmultiplied());
                    }
                    let png = crate::png::encode_rgba(img.size[0] as u32, img.size[1] as u32, &rgba);
                    if let Err(e) = std::fs::write(&path, png) {
                        eprintln!("screenshot: {}", e);
                    }
                    done.store(true, Ordering::SeqCst);
                    c2.request_repaint();
                });
            }
            ctx.request_repaint();
        }
    }
}

/// Scripted actions for automated screenshots: `tab:Insert`, `nav`, `panel:styles`, `zoom:120`, ...
fn run_action(app: &mut App, a: &str) {
    let (cmd, arg) = a.split_once(':').unwrap_or((a, ""));
    match cmd {
        "tab" => {
            app.ribbon_tab = match arg {
                "Insert" => RibbonTab::Insert,
                "Design" => RibbonTab::Design,
                "Layout" => RibbonTab::Layout,
                "References" => RibbonTab::References,
                "Review" => RibbonTab::Review,
                "View" => RibbonTab::View,
                _ => RibbonTab::Home,
            }
        }
        "nav" => app.show_nav = true,
        "marks" => app.show_marks = !app.show_marks,
        "panel" => {
            app.side_panel = match arg {
                "styles" => Some(SidePanel::Styles),
                "comments" => Some(SidePanel::Comments),
                "spelling" => Some(SidePanel::Spelling),
                _ => Some(SidePanel::Thesaurus { word: arg.trim_start_matches("thesaurus=").to_string() }),
            }
        }
        "zoom" => {
            if let Ok(z) = arg.parse::<f32>() {
                app.zoom = z / 100.0;
            }
        }
        "onepage" => app.zoom_mode = Some(ZoomMode::OnePage),
        "selectall" => app.select_all(),
        "type" => {
            for c in arg.chars() {
                app.type_text(&c.to_string());
            }
        }
        "enter" => app.enter(),
        "end" => {
            let p = app.doc.end_of(Story::Body);
            app.set_caret(p, false);
        }
        "dialog" => {
            app.dialog = match arg {
                "wordcount" => Some(Dialog::WordCount),
                "about" => Some(Dialog::About),
                "shortcuts" => Some(Dialog::Shortcuts),
                "font" => Some(Dialog::font(app)),
                "paragraph" => Some(Dialog::paragraph(app)),
                "pagesetup" => Some(Dialog::page_setup(app)),
                "find" => Some(Dialog::FindReplace { replace: true }),
                "table" => Some(Dialog::InsertTable { rows: 3, cols: 4 }),
                "symbol" => Some(Dialog::Symbol { recent: Vec::new() }),
                "options" => Some(Dialog::Options { author: app.doc.settings.author.clone(), autocorrect: true, spelling: true }),
                _ => None,
            }
        }
        "save" => {
            app.save_to(PathBuf::from(arg));
        }
        "pdf" => {
            app.export_pdf_to(PathBuf::from(arg));
        }
        "open" => app.open_path(PathBuf::from(arg)),
        _ => eprintln!("unknown --action {}", a),
    }
    app.scroll_to_caret = true;
}

fn global_shortcuts(app: &mut App, ctx: &egui::Context) {
    // Ctrl + mouse wheel zooms.
    let (zoom_delta, cmd) = ctx.input(|i| (i.zoom_delta(), i.modifiers.command));
    if cmd && (zoom_delta - 1.0).abs() > 0.001 {
        app.zoom = (app.zoom * zoom_delta).clamp(0.1, 5.0);
        app.zoom_mode = None;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::F7)) {
        if ctx.input(|i| i.modifiers.shift) {
            let w = crate::proof::word_for_thesaurus(app);
            app.side_panel = Some(SidePanel::Thesaurus { word: w });
        } else {
            app.side_panel = Some(SidePanel::Spelling);
        }
    }
}

fn apply_zoom_mode(app: &mut App, avail: egui::Vec2) {
    let Some(mode) = app.zoom_mode else { return };
    let Some(p) = app.layout.pages.first() else { return };
    let k = 96.0 / 72.0;
    let ruler = if app.show_ruler { 22.0 } else { 0.0 };
    let z = match mode {
        ZoomMode::OnePage => ((avail.y - ruler - 46.0) / (p.height * k)).min((avail.x - 60.0) / (p.width * k)),
        ZoomMode::PageWidth => (avail.x - 70.0) / (p.width * k),
    };
    let z = (z * 100.0).floor() / 100.0;
    if z.is_finite() && z > 0.1 && (z - app.zoom).abs() > 0.005 {
        app.zoom = z.clamp(0.1, 5.0);
    }
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

thread_local! {
    static WORDS: RefCell<(u64, usize)> = const { RefCell::new((u64::MAX, 0)) };
}

fn cached_word_count(app: &App) -> usize {
    // Recount at most a few times a second.
    let bucket = app.frame_no / 15;
    WORDS.with(|w| {
        let mut w = w.borrow_mut();
        if w.0 != bucket {
            *w = (bucket, crate::proof::word_count(&app.doc));
        }
        w.1
    })
}

fn status_text(ui: &mut Ui, text: impl Into<String>) -> egui::Response {
    ui.add(egui::Label::new(RichText::new(text.into()).color(Color32::WHITE).size(12.0)).sense(Sense::click()))
}

fn status_bar(app: &mut App, ui: &mut Ui) {
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        let pages = app.layout.pages.len().max(1);
        let page = app.layout.page_of(app.sel.head).unwrap_or(0) + 1;
        if status_text(ui, format!("Page {} of {}", page.min(pages), pages)).on_hover_text("Go To (Ctrl+G)").clicked() {
            app.dialog = Some(Dialog::GoTo { page: page.to_string() });
        }
        let words = cached_word_count(app);
        let label = if app.sel.is_empty() {
            format!("{} {}", words, if words == 1 { "word" } else { "words" })
        } else {
            format!("{} of {} words", app.selection_word_count(), words)
        };
        if status_text(ui, label).on_hover_text("Word Count").clicked() {
            app.dialog = Some(Dialog::WordCount);
        }
        if crate::proof::CHECK_SPELLING.load(Ordering::Relaxed) && status_text(ui, "📖 English (US)").on_hover_text("Check spelling (F7)").clicked() {
            app.side_panel = Some(SidePanel::Spelling);
        }
        if app.doc.settings.track_changes && status_text(ui, "Track Changes: On").clicked() {
            app.toggle_track_changes();
        }
        if !app.status.is_empty() {
            status_text(ui, app.status.clone());
        }
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if status_text(ui, format!("{:.0}%", app.zoom * 100.0)).on_hover_text("Zoom").clicked() {
                app.dialog = Some(Dialog::Zoom { pct: (app.zoom * 100.0).round() });
            }
            if status_text(ui, "+").clicked() {
                app.zoom = ((app.zoom * 10.0).round() / 10.0 + 0.1).min(5.0);
                app.zoom_mode = None;
            }
            let mut pct = app.zoom * 100.0;
            ui.style_mut().visuals.selection.bg_fill = Color32::WHITE;
            if ui.add(egui::Slider::new(&mut pct, 10.0..=500.0).logarithmic(true).show_value(false)).changed() {
                app.zoom = pct / 100.0;
                app.zoom_mode = None;
            }
            if status_text(ui, "−").clicked() {
                app.zoom = ((app.zoom * 10.0).round() / 10.0 - 0.1).max(0.1);
                app.zoom_mode = None;
            }
            ui.add_space(10.0);
            if status_text(ui, "▤").on_hover_text("Navigation Pane").clicked() {
                app.show_nav = !app.show_nav;
            }
            if status_text(ui, "¶").on_hover_text("Show/Hide formatting marks").clicked() {
                app.show_marks = !app.show_marks;
                app.layout_dirty = true;
            }
        });
    });
}

// ---------------------------------------------------------------------------
// Ruler
// ---------------------------------------------------------------------------

fn ruler(app: &mut App, ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, theme::PANEL_BG);
    p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, theme::BORDER));
    let k = crate::canvas::scale(app);
    let ps = &app.page_setup();
    let x0 = app.canvas_origin_x;
    let band = Rect::from_min_max(pos2(x0, rect.top() + 4.0), pos2(x0 + ps.width * k, rect.bottom() - 4.0));
    p.rect_filled(band, 0.0, Color32::from_gray(205));
    let text_l = x0 + ps.margin_left * k;
    let text_r = x0 + (ps.width - ps.margin_right) * k;
    p.rect_filled(Rect::from_min_max(pos2(text_l, band.top()), pos2(text_r, band.bottom())), 0.0, Color32::WHITE);
    // Ticks every 1/8 inch, numbers every inch, measured from the left margin.
    let inch = 72.0 * k;
    let step = inch / 8.0;
    if step > 2.0 {
        let mut i: i32 = -((ps.margin_left * k / step).floor() as i32);
        loop {
            let x = text_l + i as f32 * step;
            if x > x0 + ps.width * k {
                break;
            }
            if i != 0 {
                if i % 8 == 0 {
                    p.text(pos2(x, band.center().y), Align2::CENTER_CENTER, (i.abs() / 8).to_string(), FontId::proportional(9.5), theme::MUTED);
                } else if i % 4 == 0 {
                    p.vline(x, band.center().y - 3.0..=band.center().y + 3.0, Stroke::new(1.0, theme::MUTED));
                } else if i % 2 == 0 || step > 6.0 {
                    p.vline(x, band.center().y - 1.0..=band.center().y + 1.0, Stroke::new(1.0, theme::MUTED));
                }
            }
            i += 1;
        }
    }
    // Indent markers of the current paragraph.
    if app.sel.head.path.cell.is_none() {
        let pp = app.current_para();
        let left = text_l + pp.indent_left * k;
        let first = left + pp.indent_first * k;
        let right = text_r - pp.indent_right * k;
        let col = Color32::from_gray(90);
        p.add(egui::Shape::convex_polygon(vec![pos2(first - 4.0, band.top()), pos2(first + 4.0, band.top()), pos2(first, band.top() + 5.0)], col, Stroke::NONE));
        p.add(egui::Shape::convex_polygon(vec![pos2(left - 4.0, band.bottom()), pos2(left + 4.0, band.bottom()), pos2(left, band.bottom() - 5.0)], col, Stroke::NONE));
        p.add(egui::Shape::convex_polygon(vec![pos2(right - 4.0, band.bottom()), pos2(right + 4.0, band.bottom()), pos2(right, band.bottom() - 5.0)], col, Stroke::NONE));
        for t in &pp.tabs {
            let x = text_l + t.pos * k;
            p.line_segment([pos2(x, band.bottom() - 6.0), pos2(x, band.bottom() - 1.0)], Stroke::new(1.5, Color32::BLACK));
            p.line_segment([pos2(x, band.bottom() - 1.0), pos2(x + 5.0, band.bottom() - 1.0)], Stroke::new(1.5, Color32::BLACK));
        }
    }
}

// ---------------------------------------------------------------------------
// Navigation pane
// ---------------------------------------------------------------------------

fn pane_header(ui: &mut Ui, title: &str) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(16.0).color(theme::TEXT));
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new("🗙").frame_when_inactive(false)).on_hover_text("Close").clicked() {
                close = true;
            }
        });
    });
    ui.add_space(4.0);
    close
}

fn nav_pane(app: &mut App, ui: &mut Ui) {
    if pane_header(ui, "Navigation") {
        app.show_nav = false;
        return;
    }
    let r = ui.add(egui::TextEdit::singleline(&mut app.search_text).hint_text("Search document").desired_width(f32::INFINITY));
    if app.focus_search {
        r.request_focus();
        app.focus_search = false;
    }
    if r.changed() {
        app.find.query = app.search_text.clone();
        app.find_update();
    }
    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        app.find_next(true);
        r.request_focus();
    }
    ui.add_space(6.0);
    if !app.search_text.is_empty() {
        let n = app.find.matches.len();
        ui.horizontal(|ui| {
            ui.label(RichText::new(if n == 0 { "No results".to_string() } else { format!("Result {} of {}", (app.find.current + 1).min(n), n) }).color(theme::MUTED));
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("▼").clicked() {
                    app.find_next(true);
                }
                if ui.small_button("▲").clicked() {
                    app.find_next(false);
                }
            });
        });
        ui.separator();
    }
    ui.label(RichText::new("Headings").strong());
    ui.add_space(4.0);
    let headings: Vec<_> = app.layout.headings.iter().map(|h| (h.path.clone(), h.level, h.text.clone())).collect();
    if headings.is_empty() {
        ui.label(RichText::new("Create an interactive outline of your document.\n\nIt's a great way to keep track of where you are or quickly move your content around.\n\nTo get started, go to the Home tab and apply Heading styles to the headings in your document.").color(theme::MUTED).size(12.0));
        return;
    }
    let cur = app.sel.head.path.block;
    egui::ScrollArea::vertical().id_salt("nav-scroll").show(ui, |ui| {
        let mut active = None;
        for (i, (path, _, _)) in headings.iter().enumerate() {
            if path.block <= cur {
                active = Some(i);
            }
        }
        for (i, (path, level, text)) in headings.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(*level as f32 * 14.0);
                let t = RichText::new(if text.is_empty() { "(empty heading)" } else { text }).size(13.0);
                let t = if Some(i) == active { t.color(theme::ACCENT_DARK).strong() } else { t };
                if ui.add(egui::Button::new(t).frame_when_inactive(Some(i) == active).fill(if Some(i) == active { theme::ACCENT_LIGHT } else { Color32::TRANSPARENT })).clicked() {
                    app.set_caret(path.at(0), false);
                    app.scroll_to_caret = true;
                    app.canvas_focused = true;
                }
            });
        }
    });
}

// ---------------------------------------------------------------------------
// Side panes
// ---------------------------------------------------------------------------

fn side_pane(app: &mut App, ui: &mut Ui) {
    let Some(panel) = app.side_panel.clone() else { return };
    let title = match &panel {
        SidePanel::Styles => "Styles",
        SidePanel::Comments => "Comments",
        SidePanel::Thesaurus { .. } => "Thesaurus",
        SidePanel::Spelling => "Editor",
    };
    if pane_header(ui, title) {
        app.side_panel = None;
        return;
    }
    match panel {
        SidePanel::Styles => styles_pane(app, ui),
        SidePanel::Comments => comments_pane(app, ui),
        SidePanel::Spelling => crate::proof::spelling_panel(app, ui),
        SidePanel::Thesaurus { mut word } => {
            crate::proof::thesaurus_panel(app, ui, &mut word);
            if let Some(SidePanel::Thesaurus { word: w }) = &mut app.side_panel {
                *w = word;
            }
        }
    }
}

fn styles_pane(app: &mut App, ui: &mut Ui) {
    let current = app.current_para_props().style.unwrap_or_else(|| "Normal".into());
    let styles: Vec<(String, String, StyleKind)> = app.doc.styles.iter().filter(|s| s.kind != StyleKind::Table).map(|s| (s.id.clone(), s.name.clone(), s.kind)).collect();
    egui::ScrollArea::vertical().id_salt("styles-pane").show(ui, |ui| {
        for (id, name, kind) in styles {
            let sel = id == current;
            ui.horizontal(|ui| {
                let r = ui.add_sized([ui.available_width() - 24.0, 22.0], egui::Button::new(RichText::new(&name).size(13.0)).frame_when_inactive(sel).fill(if sel { theme::ACCENT_LIGHT } else { Color32::TRANSPARENT }));
                ui.label(RichText::new(if kind == StyleKind::Character { "a" } else { "¶" }).color(theme::MUTED));
                if r.clicked() {
                    app.apply_style(&id);
                    app.canvas_focused = true;
                }
            });
        }
    });
    ui.separator();
    if ui.button("Clear All").on_hover_text("Clear formatting").clicked() {
        app.clear_formatting();
    }
}

fn comments_pane(app: &mut App, ui: &mut Ui) {
    if app.markup != MarkupMode::AllMarkup {
        ui.label(RichText::new("Comments are hidden in this markup view.").color(theme::MUTED));
    }
    if ui.button("+ New Comment").clicked() {
        app.dialog = Some(Dialog::Comment { text: String::new() });
    }
    ui.add_space(6.0);
    if app.doc.comments.is_empty() {
        ui.label(RichText::new("There are no comments in this document.").color(theme::MUTED));
        return;
    }
    let comments: Vec<(u32, String, String, String)> = app
        .doc
        .comments
        .iter()
        .map(|c| (c.id, c.author.clone(), c.date.chars().take(10).collect(), c.paragraphs.iter().map(|p| p.text()).collect::<Vec<_>>().join("\n")))
        .collect();
    let mut delete = None;
    egui::ScrollArea::vertical().id_salt("comments-pane").show(ui, |ui| {
        for (id, author, date, text) in comments {
            egui::Frame::new().fill(Color32::from_rgb(0xFA, 0xFA, 0xFA)).stroke(Stroke::new(1.0, theme::BORDER)).inner_margin(egui::Margin::same(8)).corner_radius(4).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&author).strong());
                    ui.label(RichText::new(&date).color(theme::MUTED).size(11.0));
                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("🗑").on_hover_text("Delete comment").clicked() {
                            delete = Some(id);
                        }
                    });
                });
                ui.label(text);
            });
            ui.add_space(6.0);
        }
    });
    if let Some(id) = delete {
        app.delete_comment(id);
    }
}
