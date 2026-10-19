//! Dialog boxes.

use egui::{Align2, ComboBox, DragValue, Grid, RichText, TextEdit, Ui};
use wordcore::*;

use crate::app::{App, Pending};
use crate::theme;

#[derive(Clone, Debug)]
pub struct FontForm {
    pub font: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: Underline,
    pub color: Color,
    pub strike: bool,
    pub double_strike: bool,
    pub vert: VertAlign,
    pub caps: bool,
    pub small_caps: bool,
    pub hidden: bool,
    pub spacing: f32,
}

#[derive(Clone, Debug)]
pub struct ParaForm {
    pub align: Align,
    pub left: f32,
    pub right: f32,
    /// 0 none, 1 first line, 2 hanging
    pub special: u8,
    pub special_by: f32,
    pub before: f32,
    pub after: f32,
    /// 0 single, 1 1.5, 2 double, 3 at least, 4 exactly, 5 multiple
    pub ls_kind: u8,
    pub ls_value: f32,
    pub contextual: bool,
    pub keep_next: bool,
    pub keep_lines: bool,
    pub page_break_before: bool,
    pub widow: bool,
    pub outline: Option<u8>,
}

#[derive(Clone, Debug)]
pub struct PageForm {
    pub setup: PageSetup,
    pub unit_inches: bool,
}

#[derive(Clone, Debug)]
pub enum Dialog {
    Message { title: String, text: String },
    SaveChanges,
    Font(Box<FontForm>),
    Paragraph(Box<ParaForm>),
    PageSetup(Box<PageForm>),
    FindReplace { replace: bool },
    GoTo { page: String },
    InsertTable { rows: usize, cols: usize },
    Hyperlink { text: String, url: String, has_sel: bool },
    Comment { text: String },
    EditNote { id: u32, endnote: bool, text: String },
    WordCount,
    Symbol { recent: Vec<char> },
    DateTime { selected: usize, update: bool },
    Columns { n: u32, gap: f32 },
    Watermark { text: String },
    Bookmark { name: String },
    Caption { label: String },
    Options { author: String, autocorrect: bool, spelling: bool },
    Properties(Box<DocProperties>),
    About,
    Zoom { pct: f32 },
    Shortcuts,
}

impl Dialog {
    pub fn is_modal(&self) -> bool {
        !matches!(self, Dialog::FindReplace { .. } | Dialog::WordCount)
    }

    pub fn font(app: &App) -> Dialog {
        let r = app.current_resolved();
        Dialog::Font(Box::new(FontForm {
            font: r.font,
            size: r.size,
            bold: r.bold,
            italic: r.italic,
            underline: r.underline,
            color: r.color,
            strike: r.strike,
            double_strike: r.double_strike,
            vert: r.vert_align,
            caps: r.caps,
            small_caps: r.small_caps,
            hidden: r.hidden,
            spacing: r.spacing,
        }))
    }

    pub fn paragraph(app: &App) -> Dialog {
        let p = app.current_para();
        let (ls_kind, ls_value) = match p.line_spacing {
            LineSpacing::Multiple(m) if (m - 1.0).abs() < 0.01 => (0, 1.0),
            LineSpacing::Multiple(m) if (m - 1.5).abs() < 0.01 => (1, 1.5),
            LineSpacing::Multiple(m) if (m - 2.0).abs() < 0.01 => (2, 2.0),
            LineSpacing::Multiple(m) => (5, m),
            LineSpacing::AtLeast(v) => (3, v),
            LineSpacing::Exactly(v) => (4, v),
        };
        Dialog::Paragraph(Box::new(ParaForm {
            align: p.align,
            left: p.indent_left + p.indent_first.min(0.0),
            right: p.indent_right,
            special: if p.indent_first > 0.0 { 1 } else if p.indent_first < 0.0 { 2 } else { 0 },
            special_by: p.indent_first.abs(),
            before: p.space_before,
            after: p.space_after,
            ls_kind,
            ls_value,
            contextual: p.contextual_spacing,
            keep_next: p.keep_next,
            keep_lines: p.keep_lines,
            page_break_before: p.page_break_before,
            widow: p.widow_control,
            outline: p.outline_level,
        }))
    }

    pub fn page_setup(app: &App) -> Dialog {
        Dialog::PageSetup(Box::new(PageForm { setup: app.doc.page.clone(), unit_inches: true }))
    }

    pub fn find_replace(app: &mut App, replace: bool) -> Dialog {
        if !app.sel.is_empty() {
            let t = app.selected_text();
            if !t.contains('\n') && t.len() < 200 {
                app.find.query = t;
            }
        }
        Dialog::FindReplace { replace }
    }

    pub fn hyperlink(app: &App) -> Dialog {
        let url = app.link_at(app.sel.head).unwrap_or_default();
        Dialog::Hyperlink { text: if app.sel.is_empty() { String::new() } else { app.selected_text() }, url, has_sel: !app.sel.is_empty() }
    }
}

/// Points ↔ display units (inches or cm).
fn to_unit(pt: f32, inches: bool) -> f32 {
    if inches {
        pt / 72.0
    } else {
        pt / 72.0 * 2.54
    }
}
fn from_unit(v: f32, inches: bool) -> f32 {
    if inches {
        v * 72.0
    } else {
        v / 2.54 * 72.0
    }
}

fn unit_drag(ui: &mut Ui, pt: &mut f32, inches: bool, min: f32) {
    let mut v = to_unit(*pt, inches);
    let r = ui.add(DragValue::new(&mut v).speed(0.01).range(to_unit(min, inches)..=100.0).fixed_decimals(2).suffix(if inches { "\"" } else { " cm" }));
    if r.changed() {
        *pt = from_unit(v, inches);
    }
}

fn pt_drag(ui: &mut Ui, pt: &mut f32, min: f32, max: f32) {
    ui.add(DragValue::new(pt).speed(0.5).range(min..=max).suffix(" pt"));
}

fn window<'a>(title: &'a str) -> egui::Window<'a> {
    egui::Window::new(title).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, -40.0]).order(egui::Order::Foreground)
}

fn ok_cancel(ui: &mut Ui) -> (bool, bool) {
    let mut ok = false;
    let mut cancel = false;
    ui.add_space(6.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button("Cancel").clicked() {
            cancel = true;
        }
        if ui.add(egui::Button::new(RichText::new("   OK   ").color(egui::Color32::WHITE)).fill(theme::ACCENT)).clicked() {
            ok = true;
        }
    });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        cancel = true;
    }
    (ok, cancel)
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.dialog.take() else { return };
    let mut keep = true;
    match &mut d {
        Dialog::Message { title, text } => {
            window(title.as_str()).show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(text.as_str());
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("   OK   ").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape)) {
                        keep = false;
                    }
                });
            });
        }
        Dialog::SaveChanges => {
            let name = app.path.as_ref().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Document1".into());
            window("RWord").show(ctx, |ui| {
                ui.label(RichText::new(format!("Do you want to save your changes to \"{}\"?", name)).size(14.0));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Save").color(egui::Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                        keep = false;
                        if app.save() {
                            if let Some(p) = app.pending.take() {
                                app.run_pending(p);
                            }
                        } else {
                            app.pending = None;
                        }
                    }
                    if ui.button("Don't Save").clicked() {
                        keep = false;
                        app.modified = false;
                        if let Some(p) = app.pending.take() {
                            app.run_pending(p);
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                        app.pending = None;
                    }
                });
            });
        }
        Dialog::Font(f) => {
            window("Font").show(ctx, |ui| {
                ui.set_width(420.0);
                Grid::new("fontgrid").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label("Font:");
                    ui.label("Font style:");
                    ui.label("Size:");
                    ui.end_row();
                    ComboBox::from_id_salt("dlg-font").width(170.0).selected_text(&f.font).show_ui(ui, |ui| {
                        for name in wordcore::fonts::picker_families() {
                            ui.selectable_value(&mut f.font, name.to_string(), name);
                        }
                    });
                    let styles = ["Regular", "Italic", "Bold", "Bold Italic"];
                    let mut idx = (f.bold as usize) * 2 + f.italic as usize;
                    ComboBox::from_id_salt("dlg-style").width(110.0).selected_text(styles[idx]).show_ui(ui, |ui| {
                        for (i, s) in styles.iter().enumerate() {
                            ui.selectable_value(&mut idx, i, *s);
                        }
                    });
                    f.bold = idx >= 2;
                    f.italic = idx % 2 == 1;
                    ui.add(DragValue::new(&mut f.size).speed(0.5).range(1.0..=1638.0));
                    ui.end_row();
                    ui.label("Underline style:");
                    ui.label("Font color:");
                    ui.end_row();
                    let uls = [(Underline::None, "(none)"), (Underline::Single, "Single"), (Underline::Words, "Words only"), (Underline::Double, "Double"), (Underline::Thick, "Thick"), (Underline::Dotted, "Dotted"), (Underline::Dashed, "Dashed"), (Underline::Wavy, "Wave")];
                    let cur = uls.iter().find(|u| u.0 == f.underline).map(|u| u.1).unwrap_or("(none)");
                    ComboBox::from_id_salt("dlg-ul").width(170.0).selected_text(cur).show_ui(ui, |ui| {
                        for (u, n) in uls {
                            ui.selectable_value(&mut f.underline, u, n);
                        }
                    });
                    let mut c = theme::c32(f.color);
                    if ui.color_edit_button_srgba(&mut c).changed() {
                        f.color = theme::from_c32(c);
                    }
                    ui.end_row();
                });
                ui.separator();
                ui.label(RichText::new("Effects").strong());
                Grid::new("fx").num_columns(2).show(ui, |ui| {
                    ui.checkbox(&mut f.strike, "Strikethrough");
                    ui.checkbox(&mut f.small_caps, "Small caps");
                    ui.end_row();
                    ui.checkbox(&mut f.double_strike, "Double strikethrough");
                    ui.checkbox(&mut f.caps, "All caps");
                    ui.end_row();
                    let mut sup = f.vert == VertAlign::Superscript;
                    let mut sub = f.vert == VertAlign::Subscript;
                    if ui.checkbox(&mut sup, "Superscript").changed() {
                        f.vert = if sup { VertAlign::Superscript } else { VertAlign::Baseline };
                    }
                    ui.checkbox(&mut f.hidden, "Hidden");
                    ui.end_row();
                    if ui.checkbox(&mut sub, "Subscript").changed() {
                        f.vert = if sub { VertAlign::Subscript } else { VertAlign::Baseline };
                    }
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    ui.label("Character spacing:");
                    ui.add(DragValue::new(&mut f.spacing).speed(0.1).range(-10.0..=50.0).suffix(" pt"));
                });
                ui.separator();
                ui.label(RichText::new("Preview").small().color(theme::MUTED));
                let face = app.db.face_for(&f.font, f.bold, f.italic);
                let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 50.0), egui::Sense::hover());
                ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Inside);
                let sample = if f.caps { f.font.to_uppercase() } else { f.font.clone() };
                crate::fonts::draw_text(ui.painter(), egui::pos2(r.center().x - 80.0, r.center().y + f.size.min(30.0) * 0.4), &sample, face, f.size.min(30.0) * 1.33, theme::c32(f.color), false, 0.0, app.db);
                let (ok, cancel) = ok_cancel(ui);
                if ok {
                    let f2 = (**f).clone();
                    app.apply_chars("Font", move |c| {
                        c.font = Some(f2.font.clone());
                        c.size = Some(f2.size);
                        c.bold = Some(f2.bold);
                        c.italic = Some(f2.italic);
                        c.underline = Some(f2.underline);
                        c.color = Some(f2.color);
                        c.strike = Some(f2.strike);
                        c.double_strike = Some(f2.double_strike);
                        c.vert_align = Some(f2.vert);
                        c.caps = Some(f2.caps);
                        c.small_caps = Some(f2.small_caps);
                        c.hidden = Some(f2.hidden);
                        c.spacing = if f2.spacing.abs() < 0.01 { None } else { Some(f2.spacing) };
                    });
                }
                if ok || cancel {
                    keep = false;
                }
            });
        }
        Dialog::Paragraph(f) => {
            window("Paragraph").show(ctx, |ui| {
                ui.set_width(440.0);
                ui.label(RichText::new("General").strong());
                ui.horizontal(|ui| {
                    ui.label("Alignment:");
                    let names = [(Align::Left, "Left"), (Align::Center, "Centered"), (Align::Right, "Right"), (Align::Justify, "Justified")];
                    let cur = names.iter().find(|a| a.0 == f.align).map(|a| a.1).unwrap_or("Left");
                    ComboBox::from_id_salt("pal").selected_text(cur).show_ui(ui, |ui| {
                        for (a, n) in names {
                            ui.selectable_value(&mut f.align, a, n);
                        }
                    });
                    ui.label("Outline level:");
                    let ol = f.outline.map(|l| format!("Level {}", l + 1)).unwrap_or_else(|| "Body Text".into());
                    ComboBox::from_id_salt("pol").selected_text(ol).show_ui(ui, |ui| {
                        ui.selectable_value(&mut f.outline, None, "Body Text");
                        for l in 0..9u8 {
                            ui.selectable_value(&mut f.outline, Some(l), format!("Level {}", l + 1));
                        }
                    });
                });
                ui.separator();
                ui.label(RichText::new("Indentation").strong());
                Grid::new("ind").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Left:");
                    unit_drag(ui, &mut f.left, true, -100.0);
                    ui.label("Special:");
                    let sp = ["(none)", "First line", "Hanging"];
                    ComboBox::from_id_salt("psp").selected_text(sp[f.special as usize]).show_ui(ui, |ui| {
                        for (i, s) in sp.iter().enumerate() {
                            ui.selectable_value(&mut f.special, i as u8, *s);
                        }
                    });
                    ui.end_row();
                    ui.label("Right:");
                    unit_drag(ui, &mut f.right, true, -100.0);
                    ui.label("By:");
                    unit_drag(ui, &mut f.special_by, true, 0.0);
                    ui.end_row();
                });
                ui.separator();
                ui.label(RichText::new("Spacing").strong());
                Grid::new("spc").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Before:");
                    pt_drag(ui, &mut f.before, 0.0, 1584.0);
                    ui.label("Line spacing:");
                    let ls = ["Single", "1.5 lines", "Double", "At least", "Exactly", "Multiple"];
                    ComboBox::from_id_salt("pls").selected_text(ls[f.ls_kind as usize]).show_ui(ui, |ui| {
                        for (i, s) in ls.iter().enumerate() {
                            if ui.selectable_value(&mut f.ls_kind, i as u8, *s).clicked() {
                                f.ls_value = match i {
                                    0 => 1.0,
                                    1 => 1.5,
                                    2 => 2.0,
                                    3 | 4 => 12.0,
                                    _ => 1.15,
                                };
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("After:");
                    pt_drag(ui, &mut f.after, 0.0, 1584.0);
                    ui.label("At:");
                    if f.ls_kind >= 3 {
                        if f.ls_kind == 5 {
                            ui.add(DragValue::new(&mut f.ls_value).speed(0.01).range(0.06..=132.0).suffix(" lines"));
                        } else {
                            pt_drag(ui, &mut f.ls_value, 0.7, 1584.0);
                        }
                    } else {
                        ui.label("");
                    }
                    ui.end_row();
                });
                ui.checkbox(&mut f.contextual, "Don't add space between paragraphs of the same style");
                ui.separator();
                ui.label(RichText::new("Line and Page Breaks").strong());
                ui.checkbox(&mut f.widow, "Widow/Orphan control");
                ui.checkbox(&mut f.keep_next, "Keep with next");
                ui.checkbox(&mut f.keep_lines, "Keep lines together");
                ui.checkbox(&mut f.page_break_before, "Page break before");
                let (ok, cancel) = ok_cancel(ui);
                if ok {
                    let f2 = (**f).clone();
                    app.apply_para("Paragraph", move |p| {
                        p.align = Some(f2.align);
                        let first = match f2.special {
                            1 => f2.special_by,
                            2 => -f2.special_by,
                            _ => 0.0,
                        };
                        p.indent_left = Some(f2.left + if first < 0.0 { -first } else { 0.0 });
                        p.indent_first = Some(first);
                        p.indent_right = Some(f2.right);
                        p.space_before = Some(f2.before);
                        p.space_after = Some(f2.after);
                        p.line_spacing = Some(match f2.ls_kind {
                            0 => LineSpacing::Multiple(1.0),
                            1 => LineSpacing::Multiple(1.5),
                            2 => LineSpacing::Multiple(2.0),
                            3 => LineSpacing::AtLeast(f2.ls_value),
                            4 => LineSpacing::Exactly(f2.ls_value),
                            _ => LineSpacing::Multiple(f2.ls_value),
                        });
                        p.contextual_spacing = Some(f2.contextual);
                        p.keep_next = Some(f2.keep_next);
                        p.keep_lines = Some(f2.keep_lines);
                        p.page_break_before = Some(f2.page_break_before);
                        p.widow_control = Some(f2.widow);
                        p.outline_level = f2.outline;
                    });
                }
                if ok || cancel {
                    keep = false;
                }
            });
        }
        Dialog::PageSetup(f) => {
            window("Page Setup").show(ctx, |ui| {
                ui.set_width(400.0);
                let inch = f.unit_inches;
                ui.horizontal(|ui| {
                    ui.label("Units:");
                    ui.radio_value(&mut f.unit_inches, true, "Inches");
                    ui.radio_value(&mut f.unit_inches, false, "Centimeters");
                });
                ui.label(RichText::new("Margins").strong());
                let s = &mut f.setup;
                Grid::new("mg").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Top:");
                    unit_drag(ui, &mut s.margin_top, inch, 0.0);
                    ui.label("Bottom:");
                    unit_drag(ui, &mut s.margin_bottom, inch, 0.0);
                    ui.end_row();
                    ui.label("Left:");
                    unit_drag(ui, &mut s.margin_left, inch, 0.0);
                    ui.label("Right:");
                    unit_drag(ui, &mut s.margin_right, inch, 0.0);
                    ui.end_row();
                });
                ui.label(RichText::new("Orientation").strong());
                ui.horizontal(|ui| {
                    let portrait = s.width <= s.height;
                    if ui.radio(portrait, "Portrait").clicked() && !portrait {
                        std::mem::swap(&mut s.width, &mut s.height);
                    }
                    if ui.radio(!portrait, "Landscape").clicked() && portrait {
                        std::mem::swap(&mut s.width, &mut s.height);
                    }
                });
                ui.label(RichText::new("Paper").strong());
                let (pw, ph) = if s.width <= s.height { (s.width, s.height) } else { (s.height, s.width) };
                let cur = PAPER_SIZES.iter().find(|p| (p.1 - pw).abs() < 2.0 && (p.2 - ph).abs() < 2.0).map(|p| p.0).unwrap_or("Custom");
                ui.horizontal(|ui| {
                    ui.label("Paper size:");
                    ComboBox::from_id_salt("paper").selected_text(cur).show_ui(ui, |ui| {
                        for (n, w, h) in PAPER_SIZES {
                            if ui.selectable_label(*n == cur, *n).clicked() {
                                let land = s.width > s.height;
                                s.width = if land { *h } else { *w };
                                s.height = if land { *w } else { *h };
                            }
                        }
                    });
                });
                Grid::new("ps").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Width:");
                    unit_drag(ui, &mut s.width, inch, 72.0);
                    ui.label("Height:");
                    unit_drag(ui, &mut s.height, inch, 72.0);
                    ui.end_row();
                });
                ui.label(RichText::new("Headers and footers").strong());
                ui.checkbox(&mut s.title_page, "Different first page");
                Grid::new("hf").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Header:");
                    unit_drag(ui, &mut s.header_distance, inch, 0.0);
                    ui.label("Footer:");
                    unit_drag(ui, &mut s.footer_distance, inch, 0.0);
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    ui.label("Start page numbering at:");
                    ui.add(DragValue::new(&mut s.page_number_start).range(0..=9999));
                });
                let (ok, cancel) = ok_cancel(ui);
                if ok {
                    let setup = f.setup.clone();
                    app.set_page_setup(setup);
                }
                if ok || cancel {
                    keep = false;
                }
            });
        }
        Dialog::FindReplace { replace } => {
            let mut open = true;
            egui::Window::new("Find and Replace").open(&mut open).collapsible(false).resizable(false).default_pos([300.0, 160.0]).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(replace, false, "Find");
                    ui.selectable_value(replace, true, "Replace");
                });
                Grid::new("fr").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("Find what:");
                    let r = ui.add(TextEdit::singleline(&mut app.find.query).desired_width(260.0));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        app.find_next(true);
                    }
                    ui.end_row();
                    if *replace {
                        ui.label("Replace with:");
                        ui.add(TextEdit::singleline(&mut app.find.replace).desired_width(260.0));
                        ui.end_row();
                    }
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut app.find.opts.match_case, "Match case");
                    ui.checkbox(&mut app.find.opts.whole_word, "Find whole words only");
                });
                ui.horizontal(|ui| {
                    if *replace {
                        if ui.button("Replace").clicked() {
                            app.replace_one();
                        }
                        if ui.button("Replace All").clicked() {
                            app.replace_all();
                        }
                    }
                    if ui.button("Find Previous").clicked() {
                        app.find_next(false);
                    }
                    if ui.button("Find Next").clicked() {
                        app.find_next(true);
                    }
                });
                if !app.status.is_empty() {
                    ui.label(RichText::new(&app.status).small().color(theme::MUTED));
                }
            });
            if !open {
                keep = false;
            }
        }
        Dialog::GoTo { page } => {
            window("Go To").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Enter page number:");
                    let r = ui.add(TextEdit::singleline(page).desired_width(80.0));
                    r.request_focus();
                });
                let (ok, cancel) = ok_cancel(ui);
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ok || enter {
                    if let Ok(n) = page.trim().parse::<usize>() {
                        app.ensure_layout();
                        let idx = app.layout.pages.iter().position(|p| p.number as usize == n).unwrap_or(n.saturating_sub(1)).min(app.layout.pages.len().saturating_sub(1));
                        if let Some(l) = app.layout.pages.get(idx).and_then(|p| p.lines.iter().find(|l| l.path.story == Story::Body)) {
                            let pos = l.path.at(l.start);
                            app.set_caret(pos, false);
                        }
                        app.scroll_to_page = Some(idx);
                    }
                    keep = false;
                }
                if cancel {
                    keep = false;
                }
            });
        }
        Dialog::InsertTable { rows, cols } => {
            window("Insert Table").show(ctx, |ui| {
                Grid::new("it").num_columns(2).show(ui, |ui| {
                    ui.label("Number of columns:");
                    ui.add(DragValue::new(cols).range(1..=63));
                    ui.end_row();
                    ui.label("Number of rows:");
                    ui.add(DragValue::new(rows).range(1..=500));
                    ui.end_row();
                });
                let (ok, cancel) = ok_cancel(ui);
                if ok {
                    app.insert_table(*rows, *cols);
                }
                if ok || cancel {
                    keep = false;
                }
            });
        }
        Dialog::Hyperlink { text, url, has_sel } => {
            window("Insert Hyperlink").show(ctx, |ui| {
                Grid::new("hl").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("Text to display:");
                    ui.add_enabled(!*has_sel, TextEdit::singleline(text).desired_width(280.0));
                    ui.end_row();
                    ui.label("Address:");
                    ui.add(TextEdit::singleline(url).desired_width(280.0).hint_text("https://"));
                    ui.end_row();
                });
                ui.label(RichText::new("Tip: use #bookmark to link to a place in this document.").small().color(theme::MUTED));
                let (ok, cancel) = ok_cancel(ui);
                if ok && !url.trim().is_empty() {
                    let t = if *has_sel { String::new() } else { text.clone() };
                    // Editing an existing link: select it first.
                    if !*has_sel {
                        if let Some((a, b)) = app.link_range(app.sel.head) {
                            app.select(a, b);
                            app.insert_link("", url);
                        } else {
                            app.insert_link(&t, url);
                        }
                    } else {
                        app.insert_link("", url);
                    }
                }
                if ok || cancel {
                    keep = false;
                }
            });
        }
        Dialog::Comment { text } => {
            window("New Comment").show(ctx, |ui| {
                ui.label(format!("{}:", app.doc.settings.author));
                let r = ui.add(TextEdit::multiline(text).desired_width(320.0).desired_rows(4));
                if !r.has_focus() && text.is_empty() {
                    r.request_focus();
                }
                let (ok, cancel) = ok_cancel(ui);
                if ok && !text.trim().is_empty() {
                    app.insert_comment(text.trim());
                }
                if ok || cancel {
                    keep = false;
                }
            });
        }
        Dialog::EditNote { id, endnote, text } => {
            window(if *endnote { "Endnote" } else { "Footnote" }).show(ctx, |ui| {
                ui.label("Note text:");
                let r = ui.add(TextEdit::multiline(text).desired_width(340.0).desired_rows(3));
                if !r.has_focus() && text.is_empty() {
                    r.request_focus();
                }
                let (ok, cancel) = ok_cancel(ui);
                if ok {
                    let (id, en, t) = (*id, *endnote, text.clone());
                    let list = if en { &mut app.doc.endnotes } else { &mut app.doc.footnotes };
                    if let Some(n) = list.iter_mut().find(|n| n.id == id) {
                        if let Some(p) = n.paragraphs.first_mut() {
                            p.runs.retain(|r| !matches!(r.content, Inline::Text(_)));
                            p.runs.push(Run::text(format!(" {}", t)));
                        }
                    }
                    app.layout_dirty = true;
                    app.modified = true;
                }
                if ok || cancel {
                    keep = false;
                }
            });
        }
        Dialog::WordCount => {
            app.ensure_layout();
            let stats = crate::proof::stats(app);
            let mut open = true;
            egui::Window::new("Word Count").open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, -40.0]).show(ctx, |ui| {
                Grid::new("wc").num_columns(2).spacing([30.0, 6.0]).show(ui, |ui| {
                    for (k, v) in stats {
                        ui.label(k);
                        ui.label(RichText::new(v).strong());
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
                if ui.button("Close").clicked() {
                    keep = false;
                }
            });
            if !open {
                keep = false;
            }
        }
        Dialog::Symbol { recent } => {
            window("Symbol").show(ctx, |ui| {
                const SYMS: &str = "©®™§¶†‡•·…–—‘’“”«»‹›°±×÷≠≈≤≥∞√∑∏∫∂∆∇πΩμαβγδεθλσφψω€£¥¢₹₽¼½¾¹²³←↑→↓↔⇒⇔✓✗★☆♠♣♥♦☺☹♪♫☐☑☒♀♂∈∉∩∪⊂⊃∀∃¬∧∨ÀÁÂÃÄÅÆÇÈÉÊËÌÍÎÏÑÒÓÔÕÖØÙÚÛÜÝßàáâãäåæçèéêëìíîïñòóôõöøùúûüýÿ¿¡";
                if !recent.is_empty() {
                    ui.label(RichText::new("Recently used").small().color(theme::MUTED));
                    ui.horizontal_wrapped(|ui| {
                        for c in recent.clone() {
                            if ui.add(egui::Button::new(RichText::new(c.to_string()).size(18.0)).min_size(egui::vec2(30.0, 30.0))).clicked() {
                                app.type_text(&c.to_string());
                            }
                        }
                    });
                    ui.separator();
                }
                ui.set_max_width(16.0 * 34.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
                    for c in SYMS.chars() {
                        if ui.add(egui::Button::new(RichText::new(c.to_string()).size(18.0)).min_size(egui::vec2(32.0, 32.0))).on_hover_text(format!("U+{:04X}", c as u32)).clicked() {
                            app.type_text(&c.to_string());
                            recent.retain(|x| *x != c);
                            recent.insert(0, c);
                            recent.truncate(16);
                        }
                    }
                });
                ui.add_space(6.0);
                if ui.button("Close").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    keep = false;
                }
            });
        }
        Dialog::DateTime { selected, update } => {
            let formats = ["M/d/yyyy", "dddd, MMMM d, yyyy", "MMMM d, yyyy", "d MMMM yyyy", "dd/MM/yyyy", "yyyy-MM-dd", "d-MMM-yy", "MMM. d, yy", "h:mm am/pm", "h:mm:ss am/pm", "HH:mm", "M/d/yyyy h:mm am/pm"];
            window("Date and Time").show(ctx, |ui| {
                ui.label("Available formats:");
                let now = wordcore::util::now_parts();
                for (i, f) in formats.iter().enumerate() {
                    ui.selectable_value(selected, i, wordcore::util::format_date(f, now));
                }
                ui.checkbox(update, "Update automatically");
                let (ok, cancel) = ok_cancel(ui);
                if ok {
                    let f = formats[*selected].to_string();
                    if *update {
                        app.insert_inline(Inline::Field(Field::Date(f)), "Insert Date");
                    } else {
                        let t = wordcore::util::format_date(&f, now);
                        app.type_text(&t);
                    }
                }
                if ok || cancel {
                    keep = false;
                }
            });
        }
        Dialog::Columns { n, gap } => {
            window("Columns").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (k, name) in [(1u32, "One"), (2, "Two"), (3, "Three")] {
                        ui.selectable_value(n, k, name);
                    }
                });
                Grid::new("cols").num_columns(2).show(ui, |ui| {
                    ui.label("Number of columns:");
                    ui.add(DragValue::new(n).range(1..=12));
                    ui.end_row();
                    ui.label("Spacing:");
                    unit_drag(ui, gap, true, 0.0);
                    ui.end_row();
                });
                let (ok, cancel) = ok_cancel(ui);
                if ok {
                    let mut s = app.doc.page.clone();
                    s.columns = *n;
                    s.column_gap = *gap;
                    app.set_page_setup(s);
                }
                if ok || cancel {
                    keep = false;
                }
            });
        }
        Dialog::Watermark { text } => {
            window("Printed Watermark").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Text:");
                    ComboBox::from_id_salt("wm").selected_text(text.as_str()).show_ui(ui, |ui| {
                        for t in ["CONFIDENTIAL", "DRAFT", "DO NOT COPY", "SAMPLE", "URGENT", "ASAP", "ORIGINAL", "PERSONAL", "TOP SECRET"] {
                            ui.selectable_value(text, t.to_string(), t);
                        }
                    });
                });
                ui.add(TextEdit::singleline(text).desired_width(260.0));
                let mut remove = false;
                let (ok, cancel) = ui
                    .horizontal(|ui| {
                        if ui.button("No watermark").clicked() {
                            remove = true;
                        }
                        ok_cancel(ui)
                    })
                    .inner;
                if ok || remove {
                    let mut s = app.doc.page.clone();
                    s.watermark = if remove || text.trim().is_empty() { None } else { Some(text.trim().to_string()) };
                    app.set_page_setup(s);
                }
                if ok || cancel || remove {
                    keep = false;
                }
            });
        }
        Dialog::Bookmark { name } => {
            let existing: Vec<String> = app
                .doc
                .para_paths(Story::Body)
                .iter()
                .filter_map(|p| app.doc.para(p))
                .flat_map(|p| p.runs.iter().filter_map(|r| if let Inline::Bookmark(b) = &r.content { Some(b.clone()) } else { None }).collect::<Vec<_>>())
                .collect();
            window("Bookmark").show(ctx, |ui| {
                ui.label("Bookmark name:");
                ui.add(TextEdit::singleline(name).desired_width(260.0));
                for b in &existing {
                    ui.horizontal(|ui| {
                        ui.label(b);
                        if ui.small_button("Go To").clicked() {
                            let c = ui.ctx().clone();
                            crate::canvas::follow_link(app, &c, &format!("#{}", b));
                        }
                    });
                }
                let (ok, cancel) = ok_cancel(ui);
                let valid = !name.is_empty() && !name.contains(' ');
                if ok && valid {
                    app.insert_inline(Inline::Bookmark(name.clone()), "Bookmark");
                }
                if ok && !valid {
                    app.status = "Bookmark names can't contain spaces.".into();
                }
                if ok || cancel {
                    keep = false;
                }
            });
        }
        Dialog::Caption { label } => {
            window("Caption").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Label:");
                    ComboBox::from_id_salt("cap").selected_text(label.as_str()).show_ui(ui, |ui| {
                        for l in ["Figure", "Table", "Equation"] {
                            ui.selectable_value(label, l.to_string(), l);
                        }
                    });
                });
                let (ok, cancel) = ok_cancel(ui);
                if ok {
                    let l = label.clone();
                    app.insert_caption(&l);
                }
                if ok || cancel {
                    keep = false;
                }
            });
        }
        Dialog::Options { author, autocorrect, spelling } => {
            window("RWord Options").show(ctx, |ui| {
                ui.label(RichText::new("Personalize your copy of RWord").strong());
                ui.horizontal(|ui| {
                    ui.label("User name:");
                    ui.add(TextEdit::singleline(author).desired_width(220.0));
                });
                ui.separator();
                ui.label(RichText::new("Proofing").strong());
                ui.checkbox(spelling, "Check spelling as you type");
                ui.checkbox(autocorrect, "AutoCorrect and smart quotes as you type");
                let (ok, cancel) = ok_cancel(ui);
                if ok {
                    app.doc.settings.author = author.clone();
                    crate::fileio::save_user_name(author);
                    crate::proof::set_autocorrect(*autocorrect);
                    crate::proof::CHECK_SPELLING.store(*spelling, std::sync::atomic::Ordering::Relaxed);
                }
                if ok || cancel {
                    keep = false;
                }
            });
        }
        Dialog::Properties(p) => {
            window("Properties").show(ctx, |ui| {
                Grid::new("props").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    for (label, field) in [("Title:", &mut p.title), ("Subject:", &mut p.subject), ("Author:", &mut p.author), ("Keywords:", &mut p.keywords), ("Comments:", &mut p.description)] {
                        ui.label(label);
                        ui.add(TextEdit::singleline(field).desired_width(280.0));
                        ui.end_row();
                    }
                    ui.label("Created:");
                    ui.label(&p.created);
                    ui.end_row();
                    ui.label("Modified:");
                    ui.label(&p.modified);
                    ui.end_row();
                });
                let (ok, cancel) = ok_cancel(ui);
                if ok {
                    app.doc.properties = (**p).clone();
                    app.modified = true;
                }
                if ok || cancel {
                    keep = false;
                }
            });
        }
        Dialog::About => {
            window("About RWord").show(ctx, |ui| {
                ui.label(RichText::new("RWord").size(22.0).color(theme::ACCENT).strong());
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.label("A word processor written in Rust, in the style of Microsoft Word.");
                ui.label("Opens and saves .docx, .odt, .rtf, .html, .txt and .md; exports PDF.");
                ui.add_space(4.0);
                ui.label(RichText::new("Bundled fonts: Carlito, Caladea and Liberation (SIL Open Font License).").small().color(theme::MUTED));
                ui.add_space(6.0);
                if ui.button("   OK   ").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    keep = false;
                }
            });
        }
        Dialog::Zoom { pct } => {
            window("Zoom").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for z in [200.0, 150.0, 100.0, 75.0, 50.0] {
                        if ui.selectable_label((*pct - z).abs() < 0.5, format!("{}%", z)).clicked() {
                            *pct = z;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Percent:");
                    ui.add(DragValue::new(pct).range(10.0..=500.0).suffix("%"));
                });
                let (ok, cancel) = ok_cancel(ui);
                if ok {
                    app.zoom = *pct / 100.0;
                }
                if ok || cancel {
                    keep = false;
                }
            });
        }
        Dialog::Shortcuts => {
            window("Keyboard Shortcuts").show(ctx, |ui| {
                let m = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };
                Grid::new("sc").num_columns(2).striped(true).show(ui, |ui| {
                    for (k, v) in [
                        ("B / I / U", "Bold / Italic / Underline"),
                        ("Z / Y", "Undo / Redo"),
                        ("X / C / V", "Cut / Copy / Paste"),
                        ("F / H", "Find / Replace"),
                        ("E / L / R / J", "Center / Left / Right / Justify"),
                        ("1 / 2 / 5", "Single / Double / 1.5 line spacing"),
                        ("[ / ]", "Smaller / larger font"),
                        ("= / Shift+=", "Subscript / Superscript"),
                        ("K", "Insert link"),
                        ("Enter", "Page break"),
                        ("Alt+1/2/3", "Heading 1/2/3"),
                        ("Shift+N", "Normal style"),
                        ("Shift+L", "Bullets"),
                        ("M / Shift+M", "Indent / Outdent"),
                        ("D", "Font dialog"),
                        ("Space", "Clear character formatting"),
                        ("S / O / N / P", "Save / Open / New / Print"),
                    ] {
                        ui.label(RichText::new(format!("{}{}", m, k)).monospace());
                        ui.label(v);
                        ui.end_row();
                    }
                    ui.label(RichText::new("Shift+Enter").monospace());
                    ui.label("Line break");
                    ui.end_row();
                    ui.label(RichText::new("F7 / Shift+F7").monospace());
                    ui.label("Spelling / Thesaurus");
                    ui.end_row();
                });
                if ui.button("Close").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    keep = false;
                }
            });
        }
    }
    if keep && app.dialog.is_none() {
        app.dialog = Some(d);
    }
}

#[allow(dead_code)]
fn _pending_used(p: Pending) -> Pending {
    p
}
