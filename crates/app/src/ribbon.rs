//! Ribbon (tabs + command groups) and the File menu.

use egui::{pos2, vec2, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, Ui};
use wordcore::*;

use crate::app::{App, MarkupMode, Pending, RibbonTab, SidePanel};
use crate::canvas::sc;
use crate::dialogs::Dialog;
use crate::theme;

const BODY_H: f32 = 80.0;

pub fn show(app: &mut App, ui: &mut Ui) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.x = 2.0;
        file_menu(app, ui);
        ui.add_space(6.0);
        for (tab, name) in [
            (RibbonTab::Home, "Home"),
            (RibbonTab::Insert, "Insert"),
            (RibbonTab::Design, "Design"),
            (RibbonTab::Layout, "Layout"),
            (RibbonTab::References, "References"),
            (RibbonTab::Review, "Review"),
            (RibbonTab::View, "View"),
        ] {
            let sel = app.ribbon_tab == tab;
            let text = RichText::new(name).size(13.0).color(if sel { theme::ACCENT_DARK } else { theme::TEXT });
            let r = ui.add(egui::Button::new(if sel { text.strong() } else { text }).frame_when_inactive(false).min_size(vec2(58.0, 24.0)));
            if sel {
                ui.painter().hline(r.rect.x_range().shrink(10.0), r.rect.bottom() - 1.0, Stroke::new(3.0, theme::ACCENT));
            }
            if r.clicked() {
                app.ribbon_tab = tab;
            }
        }
        if app.sel.head.path.cell.is_some() {
            ui.add_space(8.0);
            ui.label(RichText::new("Table Tools").size(12.0).color(theme::ACCENT));
        }
        if app.story() != Story::Body {
            ui.add_space(8.0);
            if ui.add(egui::Button::new(RichText::new("Close Header and Footer").size(12.0).color(Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                app.close_header();
            }
        }
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            let tip = app.redo_label().map(|l| format!("Redo {} ({})", l, sc("Y"))).unwrap_or_else(|| "Can't Redo".into());
            if ui.add_enabled(app.can_redo(), egui::Button::new("↷").frame_when_inactive(false)).on_hover_text(tip).clicked() {
                app.redo();
            }
            let tip = app.undo_label().map(|l| format!("Undo {} ({})", l, sc("Z"))).unwrap_or_else(|| "Can't Undo".into());
            if ui.add_enabled(app.can_undo(), egui::Button::new("↶").frame_when_inactive(false)).on_hover_text(tip).clicked() {
                app.undo();
            }
            if ui.add(egui::Button::new("💾").frame_when_inactive(false)).on_hover_text(format!("Save ({})", sc("S"))).clicked() {
                app.save();
            }
        });
    });
    let body = egui::Frame::new()
        .fill(Color32::WHITE)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .inner_margin(egui::Margin::symmetric(6, 4))
        .corner_radius(4.0)
        .outer_margin(egui::Margin { left: 6, right: 6, top: 0, bottom: 6 });
    body.show(ui, |ui| {
        ui.set_height(BODY_H);
        ui.set_width(ui.available_width());
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        combo_frame(ui, false);
        egui::ScrollArea::horizontal().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
            ui.horizontal_top(|ui| match app.ribbon_tab {
                RibbonTab::Home => home(app, ui),
                RibbonTab::Insert => insert(app, ui),
                RibbonTab::Design => design(app, ui),
                RibbonTab::Layout => layout_tab(app, ui),
                RibbonTab::References => references(app, ui),
                RibbonTab::Review => review(app, ui),
                RibbonTab::View => view(app, ui),
            });
        });
    });
}

fn file_menu(app: &mut App, ui: &mut Ui) {
    let btn = egui::Button::new(RichText::new("  File  ").color(Color32::WHITE).size(13.0)).fill(theme::ACCENT).min_size(vec2(54.0, 24.0));
    let resp = ui.add(btn);
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(240.0);
        if ui.add(egui::Button::new("New").shortcut_text(sc("N"))).clicked() {
            app.request(Pending::New);
            ui.close();
        }
        if ui.add(egui::Button::new("Open...").shortcut_text(sc("O"))).clicked() {
            app.request(Pending::Open(None));
            ui.close();
        }
        let recent = app.recent.clone();
        ui.add_enabled_ui(!recent.is_empty(), |ui| {
            ui.menu_button("Open Recent", |ui| {
                for p in recent {
                    let label = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    if ui.button(label).on_hover_text(p.display().to_string()).clicked() {
                        app.request(Pending::Open(Some(p.clone())));
                        ui.close();
                    }
                }
            });
        });
        ui.separator();
        if ui.add(egui::Button::new("Save").shortcut_text(sc("S"))).clicked() {
            app.save();
            ui.close();
        }
        if ui.add(egui::Button::new("Save As...").shortcut_text("F12")).clicked() {
            app.save_as();
            ui.close();
        }
        if ui.button("Export to PDF...").clicked() {
            app.export_pdf();
            ui.close();
        }
        ui.separator();
        if ui.add(egui::Button::new("Print...").shortcut_text(sc("P"))).on_hover_text("Opens the document as a PDF in your system viewer, ready to print").clicked() {
            app.print();
            ui.close();
        }
        if ui.button("Page Setup...").clicked() {
            app.dialog = Some(Dialog::page_setup(app));
            ui.close();
        }
        ui.separator();
        if ui.button("Properties...").clicked() {
            app.dialog = Some(Dialog::Properties(Box::new(app.doc.properties.clone())));
            ui.close();
        }
        if ui.button("Options...").clicked() {
            app.dialog = Some(Dialog::Options {
                author: app.doc.settings.author.clone(),
                autocorrect: crate::proof::autocorrect_enabled(),
                spelling: crate::proof::CHECK_SPELLING.load(std::sync::atomic::Ordering::Relaxed),
            });
            ui.close();
        }
        if ui.button("Keyboard Shortcuts").clicked() {
            app.dialog = Some(Dialog::Shortcuts);
            ui.close();
        }
        if ui.button("About RWord").clicked() {
            app.dialog = Some(Dialog::About);
            ui.close();
        }
        ui.separator();
        if ui.button("Close").clicked() {
            app.request(Pending::Close);
            ui.close();
        }
    });
}

// ----------------------------------------------------------------------
// building blocks
// ----------------------------------------------------------------------

fn group(ui: &mut Ui, name: &str, add: impl FnOnce(&mut Ui)) {
    let top = ui.cursor().top();
    let inner = ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
        let content = ui.horizontal_top(|ui| add(ui)).response.rect;
        let label = ui.painter().layout_no_wrap(name.to_string(), FontId::proportional(11.0), theme::MUTED);
        let w = content.width().max(label.size().x + 8.0);
        let rest = (top + BODY_H - ui.cursor().top()).max(0.0);
        let (r, _) = ui.allocate_exact_size(vec2(w, rest), Sense::hover());
        ui.painter().galley(pos2(content.left() + (w - label.size().x) / 2.0, r.bottom() - label.size().y), label, theme::MUTED);
    });
    let r = inner.response.rect;
    ui.add_space(5.0);
    ui.painter().vline(r.right() + 5.0, top + 4.0..=top + BODY_H - 4.0, Stroke::new(1.0, theme::BORDER));
    ui.add_space(6.0);
}

fn combo_frame(ui: &mut Ui, on: bool) {
    ui.visuals_mut().widgets.inactive.bg_stroke = if on { Stroke::new(1.0, Color32::from_gray(180)) } else { Stroke::NONE };
}

/// A large icon button with a label under it.
fn big(ui: &mut Ui, icon: &str, label: &str, tip: &str) -> egui::Response {
    let w = (ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.0), theme::TEXT).size().x + 12.0).max(46.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, BODY_H - 18.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect(rect, 3.0, Color32::from_gray(232), Stroke::new(1.0, Color32::from_gray(205)), egui::StrokeKind::Inside);
    }
    p.text(pos2(rect.center().x, rect.top() + 20.0), Align2::CENTER_CENTER, icon, FontId::proportional(24.0), theme::ACCENT_DARK);
    let g = p.layout(label.to_string(), FontId::proportional(12.0), theme::TEXT, rect.width());
    p.galley(pos2(rect.center().x - g.size().x / 2.0, rect.top() + 36.0), g, theme::TEXT);
    resp.on_hover_text(tip)
}

/// A large button that opens a menu.
fn big_menu(ui: &mut Ui, icon: &str, label: &str, tip: &str, add: impl FnOnce(&mut Ui)) {
    let r = big(ui, icon, &format!("{} ▾", label), tip);
    egui::Popup::menu(&r).show(|ui| {
        ui.set_min_width(180.0);
        add(ui)
    });
}

fn small(ui: &mut Ui, text: impl Into<egui::WidgetText>, tip: &str) -> egui::Response {
    ui.add(egui::Button::new(text).frame_when_inactive(false).min_size(vec2(24.0, 22.0))).on_hover_text(tip)
}

fn toggle(ui: &mut Ui, on: bool, text: impl Into<egui::WidgetText>, tip: &str) -> egui::Response {
    ui.add(egui::Button::selectable(on, text).min_size(vec2(24.0, 22.0))).on_hover_text(tip)
}

/// Toggle with a painted alignment icon (0 left, 1 centre, 2 right, 3 justify).
fn align_toggle(ui: &mut Ui, on: bool, which: u8, tip: &str) -> egui::Response {
    let resp = ui.add(egui::Button::selectable(on, " ").min_size(vec2(24.0, 22.0))).on_hover_text(tip);
    let c = resp.rect.center();
    let stroke = Stroke::new(1.3, Color32::from_gray(60));
    for (i, w) in [12.0f32, 8.0, 12.0, 8.0].iter().enumerate() {
        let w = if which == 3 { 12.0 } else { *w };
        let y = c.y - 4.5 + i as f32 * 3.0;
        let (x0, x1) = match which {
            0 | 3 => (c.x - 6.0, c.x - 6.0 + w),
            1 => (c.x - w / 2.0, c.x + w / 2.0),
            _ => (c.x + 6.0 - w, c.x + 6.0),
        };
        ui.painter().hline(x0..=x1, y, stroke);
    }
    resp
}

/// Office colour palette; returns the picked colour (`Some(None)` = automatic/none).
pub fn palette(ui: &mut Ui, auto_label: Option<&str>) -> Option<Option<Color>> {
    let mut picked = None;
    if let Some(l) = auto_label {
        if ui.button(l).clicked() {
            picked = Some(None);
        }
    }
    ui.label(RichText::new("Theme Colors").small().color(theme::MUTED));
    for (ri, row) in theme::PALETTE.iter().enumerate() {
        if ri == 5 {
            ui.label(RichText::new("Standard Colors").small().color(theme::MUTED));
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for c in row {
                let (r, resp) = ui.allocate_exact_size(vec2(15.0, 15.0), Sense::click());
                ui.painter().rect_filled(r, 0.0, theme::c32(theme::from_u32(*c)));
                ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, if resp.hovered() { Color32::from_rgb(0xF2, 0x94, 0x36) } else { Color32::from_gray(200) }), egui::StrokeKind::Inside);
                if resp.on_hover_text(format!("#{:06X}", c)).clicked() {
                    picked = Some(Some(theme::from_u32(*c)));
                }
            }
        });
    }
    ui.add_space(4.0);
    let mut c32 = Color32::from_gray(128);
    ui.horizontal(|ui| {
        ui.label("More Colors:");
        if ui.color_edit_button_srgba(&mut c32).changed() {
            picked = Some(Some(theme::from_c32(c32)));
        }
    });
    picked
}

fn highlight_palette(ui: &mut Ui) -> Option<Option<Color>> {
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(5.0 * 26.0);
        for (c, name) in theme::HIGHLIGHTS {
            let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
            ui.painter().rect_filled(r.shrink(2.0), 0.0, theme::c32(theme::from_u32(c)));
            if resp.hovered() {
                ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_rgb(0xF2, 0x94, 0x36)), egui::StrokeKind::Inside);
            }
            if resp.on_hover_text(name).clicked() {
                picked = Some(Some(theme::from_u32(c)));
            }
        }
    });
    if ui.button("No Color").clicked() {
        picked = Some(None);
    }
    picked
}

/// Split button with a colour bar: (clicked main part, picked colour).
fn color_split(ui: &mut Ui, label: RichText, current: Color, tip: &str, picker: impl FnOnce(&mut Ui) -> Option<Option<Color>>) -> (bool, Option<Option<Color>>) {
    let mut apply = false;
    let mut pick = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let r = ui.add(egui::Button::new(label).frame_when_inactive(false).min_size(vec2(24.0, 22.0))).on_hover_text(tip);
        ui.painter().rect_filled(Rect::from_min_max(pos2(r.rect.left() + 4.0, r.rect.bottom() - 5.0), pos2(r.rect.right() - 4.0, r.rect.bottom() - 2.0)), 0.0, theme::c32(current));
        if r.clicked() {
            apply = true;
        }
        ui.menu_button("▾", |ui| {
            if let Some(c) = picker(ui) {
                pick = Some(c);
                ui.close();
            }
        });
    });
    (apply, pick)
}

/// "Wrap Text" choices for the selected picture or text box.
pub fn wrap_menu(app: &mut App, ui: &mut Ui) {
    let cur = match app.selected_object() {
        Some((_, Inline::Image(i))) => i.float.map(|f| f.wrap),
        Some((_, Inline::TextBox(t))) => t.float.map(|f| f.wrap),
        _ => return,
    };
    for (n, w) in [
        ("In Line with Text", None),
        ("Square", Some(WrapMode::Square)),
        ("Tight", Some(WrapMode::Tight)),
        ("Top and Bottom", Some(WrapMode::TopBottom)),
        ("Behind Text", Some(WrapMode::Behind)),
        ("In Front of Text", Some(WrapMode::InFront)),
    ] {
        if ui.selectable_label(cur == w, n).clicked() {
            app.set_wrap(w);
            ui.close();
        }
    }
}

pub fn apply_format_painter(app: &mut App) {
    if let Some((chars, para, sticky)) = app.format_painter.clone() {
        app.checkpoint("Format Painter", crate::app::EditKind::Other);
        let (a, b) = app.sel.range();
        let c2 = chars.clone();
        app.doc.apply_chars(a, b, &move |c| {
            let keep = (c.link.clone(), c.revision.clone(), c.comments.clone());
            *c = c2.clone();
            c.link = keep.0;
            c.revision = keep.1;
            c.comments = keep.2;
        });
        app.doc.apply_para(a, b, &move |p| *p = para.clone());
        app.changed();
        if !sticky {
            app.format_painter = None;
        }
    }
}

// ----------------------------------------------------------------------
// Home
// ----------------------------------------------------------------------

const SIZES: [f32; 21] = [8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0, 72.0, 96.0, 120.0, 144.0, 200.0];

fn home(app: &mut App, ui: &mut Ui) {
    let st = app.current_resolved();
    let pp = app.current_para();
    group(ui, "Clipboard", |ui| {
        let r = big(ui, "📋", "Paste", &format!("Paste ({})", sc("V")));
        if r.clicked() {
            app.paste(None, false);
        }
        ui.vertical(|ui| {
            if small(ui, "✂ Cut", &format!("Cut ({})", sc("X"))).clicked() {
                app.copy(true);
            }
            if small(ui, "⎘ Copy", &format!("Copy ({})", sc("C"))).clicked() {
                app.copy(false);
            }
            let fp = toggle(ui, app.format_painter.is_some(), "🖊 Format Painter", "Copy formatting from one place and apply it to another. Double-click to apply it to many places.");
            if fp.double_clicked() {
                start_painter(app, true);
            } else if fp.clicked() {
                if app.format_painter.is_some() {
                    app.format_painter = None;
                } else {
                    start_painter(app, false);
                }
            }
        });
    });
    group(ui, "Font", |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                combo_frame(ui, true);
                let cur_font = st.font.clone();
                egui::ComboBox::from_id_salt("font-name").width(140.0).height(400.0).selected_text(&cur_font).show_ui(ui, |ui| {
                    for f in wordcore::fonts::picker_families() {
                        let face = app.db.face_for(f, false, false);
                        let text = RichText::new(f).font(crate::fonts::font_id(face, 14.0));
                        if ui.selectable_label(f == cur_font, text).clicked() {
                            app.set_font(f);
                        }
                    }
                });
                let size_txt = if (st.size - st.size.round()).abs() < 0.01 { format!("{}", st.size as i32) } else { format!("{:.1}", st.size) };
                egui::ComboBox::from_id_salt("font-size").width(48.0).height(400.0).selected_text(size_txt).show_ui(ui, |ui| {
                    for s in SIZES {
                        if ui.selectable_label((s - st.size).abs() < 0.01, format!("{}", s)).clicked() {
                            app.set_size(s);
                        }
                    }
                });
                combo_frame(ui, false);
                if small(ui, "A⁺", "Increase Font Size").clicked() {
                    app.grow_font(true);
                }
                if small(ui, "A⁻", "Decrease Font Size").clicked() {
                    app.grow_font(false);
                }
                ui.menu_button("Aa", |ui| {
                    for (i, n) in ["Sentence case.", "lowercase", "UPPERCASE", "Capitalize Each Word", "tOGGLE cASE"].iter().enumerate() {
                        if ui.button(*n).clicked() {
                            app.change_case(i as u8);
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text("Change Case");
                if small(ui, "A×", "Clear All Formatting").clicked() {
                    app.clear_formatting();
                }
            });
            ui.horizontal(|ui| {
                if toggle(ui, st.bold, RichText::new("B").strong(), &format!("Bold ({})", sc("B"))).clicked() {
                    app.toggle_bold();
                }
                if toggle(ui, st.italic, RichText::new("I").italics(), &format!("Italic ({})", sc("I"))).clicked() {
                    app.toggle_italic();
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    if toggle(ui, st.underline != Underline::None, RichText::new("U").underline(), &format!("Underline ({})", sc("U"))).clicked() {
                        app.toggle_underline(Underline::Single);
                    }
                    ui.menu_button("▾", |ui| {
                        for (u, n) in [(Underline::Single, "Single"), (Underline::Double, "Double"), (Underline::Thick, "Thick"), (Underline::Dotted, "Dotted"), (Underline::Dashed, "Dashed"), (Underline::Wavy, "Wave"), (Underline::Words, "Words only")] {
                            if ui.button(n).clicked() {
                                app.apply_chars("Underline", move |c| c.underline = Some(u));
                                ui.close();
                            }
                        }
                    });
                });
                if toggle(ui, st.strike, RichText::new("ab").strikethrough(), "Strikethrough").clicked() {
                    app.toggle_strike();
                }
                if toggle(ui, st.vert_align == VertAlign::Subscript, "x₂", &format!("Subscript ({})", sc("="))).clicked() {
                    app.toggle_vert(VertAlign::Subscript);
                }
                if toggle(ui, st.vert_align == VertAlign::Superscript, "x²", &format!("Superscript ({})", sc("Shift+="))).clicked() {
                    app.toggle_vert(VertAlign::Superscript);
                }
                let (apply, pick) = color_split(ui, RichText::new("ab").background_color(theme::c32(app.highlight_color)), app.highlight_color, "Text Highlight Color", highlight_palette);
                if apply {
                    let c = app.highlight_color;
                    app.set_highlight(Some(c));
                }
                if let Some(c) = pick {
                    if let Some(c) = c {
                        app.highlight_color = c;
                    }
                    app.set_highlight(c);
                }
                let (apply, pick) = color_split(ui, RichText::new("A").strong(), app.font_color, "Font Color", |ui| palette(ui, Some("Automatic")));
                if apply {
                    let c = app.font_color;
                    app.set_color(Some(c));
                }
                if let Some(c) = pick {
                    if let Some(c) = c {
                        app.font_color = c;
                    }
                    app.set_color(c);
                }
                if small(ui, "⋯", &format!("Font dialog ({})", sc("D"))).clicked() {
                    app.dialog = Some(Dialog::font(app));
                }
            });
        });
    });
    group(ui, "Paragraph", |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let list = app.doc.para(&app.sel.head.path).and_then(|p| crate::app::resolved_list(&app.doc, p));
                let kind = list.and_then(|l| app.doc.list(l.list)).map(|d| if d.levels[0].format == NumFormat::Bullet { 0 } else if d.levels[1].text.contains("%1") { 2 } else { 1 });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    if toggle(ui, kind == Some(0), "☰•", "Bullets").clicked() {
                        app.toggle_list(0, None);
                    }
                    ui.menu_button("▾", |ui| {
                        ui.label(RichText::new("Bullet Library").small().color(theme::MUTED));
                        ui.horizontal(|ui| {
                            for b in ["•", "○", "■", "◆", "➢", "✓", "–"] {
                                if ui.add(egui::Button::new(RichText::new(b).size(18.0)).min_size(vec2(30.0, 30.0))).clicked() {
                                    let mut d = ListDef::bullets(0);
                                    d.levels[0].text = b.to_string();
                                    app.toggle_list(0, Some(d));
                                    ui.close();
                                }
                            }
                        });
                    });
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    if toggle(ui, kind == Some(1), "☰1", "Numbering").clicked() {
                        app.toggle_list(1, None);
                    }
                    ui.menu_button("▾", |ui| {
                        ui.label(RichText::new("Numbering Library").small().color(theme::MUTED));
                        for (fmt, text, label) in [
                            (NumFormat::Decimal, "%1.", "1. 2. 3."),
                            (NumFormat::Decimal, "%1)", "1) 2) 3)"),
                            (NumFormat::UpperRoman, "%1.", "I. II. III."),
                            (NumFormat::UpperLetter, "%1.", "A. B. C."),
                            (NumFormat::LowerLetter, "%1)", "a) b) c)"),
                            (NumFormat::LowerLetter, "%1.", "a. b. c."),
                            (NumFormat::LowerRoman, "%1.", "i. ii. iii."),
                        ] {
                            if ui.button(label).clicked() {
                                let mut d = ListDef::numbered(0);
                                d.levels[0].format = fmt;
                                d.levels[0].text = text.into();
                                app.toggle_list(1, Some(d));
                                ui.close();
                            }
                        }
                    });
                });
                if toggle(ui, kind == Some(2), "1.1", "Multilevel List").clicked() {
                    app.toggle_list(2, None);
                }
                if small(ui, "⇤", "Decrease Indent").clicked() {
                    app.change_indent(-1);
                }
                if small(ui, "⇥", "Increase Indent").clicked() {
                    app.change_indent(1);
                }
                if small(ui, "A↓Z", "Sort selected paragraphs").clicked() {
                    sort_paragraphs(app);
                }
                if toggle(ui, app.show_marks, "¶", "Show/Hide formatting marks").clicked() {
                    app.show_marks = !app.show_marks;
                    app.layout_dirty = true;
                }
            });
            ui.horizontal(|ui| {
                for (al, which, tip) in [(Align::Left, 0u8, "Align Left"), (Align::Center, 1, "Center"), (Align::Right, 2, "Align Right"), (Align::Justify, 3, "Justify")] {
                    if align_toggle(ui, pp.align == al, which, &format!("{} ({})", tip, sc(["L", "E", "R", "J"][which as usize]))).clicked() {
                        app.set_align(al);
                    }
                }
                ui.menu_button("↕", |ui| {
                    for (v, n) in [(1.0, "1.0"), (1.08, "1.08"), (1.15, "1.15"), (1.5, "1.5"), (2.0, "2.0"), (2.5, "2.5"), (3.0, "3.0")] {
                        let on = matches!(pp.line_spacing, LineSpacing::Multiple(m) if (m - v).abs() < 0.01);
                        if ui.selectable_label(on, n).clicked() {
                            app.set_line_spacing(LineSpacing::Multiple(v));
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button("Line Spacing Options...").clicked() {
                        app.dialog = Some(Dialog::paragraph(app));
                        ui.close();
                    }
                    let before = pp.space_before > 0.0;
                    if ui.button(if before { "Remove Space Before Paragraph" } else { "Add Space Before Paragraph" }).clicked() {
                        app.apply_para("Spacing", move |p| p.space_before = Some(if before { 0.0 } else { 12.0 }));
                        ui.close();
                    }
                    let after = pp.space_after > 0.0;
                    if ui.button(if after { "Remove Space After Paragraph" } else { "Add Space After Paragraph" }).clicked() {
                        app.apply_para("Spacing", move |p| p.space_after = Some(if after { 0.0 } else { 8.0 }));
                        ui.close();
                    }
                })
                .response
                .on_hover_text("Line and Paragraph Spacing");
                ui.menu_button("🎨", |ui| {
                    if let Some(c) = palette(ui, Some("No Color")) {
                        app.apply_para("Shading", move |p| p.shading = c);
                        ui.close();
                    }
                })
                .response
                .on_hover_text("Shading");
                ui.menu_button("▦", |ui| {
                    let b = Border::single();
                    let presets: [(&str, Borders); 7] = [
                        ("Bottom Border", Borders { bottom: Some(b), ..Default::default() }),
                        ("Top Border", Borders { top: Some(b), ..Default::default() }),
                        ("Left Border", Borders { left: Some(b), ..Default::default() }),
                        ("Right Border", Borders { right: Some(b), ..Default::default() }),
                        ("Outside Borders", Borders { top: Some(b), bottom: Some(b), left: Some(b), right: Some(b), ..Default::default() }),
                        ("All Borders", Borders::all(b)),
                        ("No Border", Borders::default()),
                    ];
                    for (n, bs) in presets {
                        if ui.button(n).clicked() {
                            if app.sel.head.path.cell.is_some() && n != "Bottom Border" && n != "Top Border" {
                                let bs2 = bs.clone();
                                app.with_cell_borders(bs2);
                            } else {
                                app.apply_para("Borders", move |p| p.borders = Some(bs.clone()));
                            }
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button("Horizontal Line").clicked() {
                        app.insert_horizontal_line();
                        ui.close();
                    }
                })
                .response
                .on_hover_text("Borders");
                if small(ui, "⋯", "Paragraph Settings").clicked() {
                    app.dialog = Some(Dialog::paragraph(app));
                }
            });
        });
    });
    group(ui, "Styles", |ui| {
        let cur = app.doc.para(&app.sel.head.path).map(|p| p.style_id().to_string()).unwrap_or_else(|| "Normal".into());
        let quick: Vec<Style> = app.doc.styles.iter().filter(|s| s.quick && s.kind == StyleKind::Paragraph).cloned().collect();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for s in quick.iter().take(7) {
                if style_tile(app, ui, s, s.id == cur).clicked() {
                    app.apply_style(&s.id);
                }
            }
            ui.vertical(|ui| {
                ui.menu_button("▾", |ui| {
                    ui.set_min_width(220.0);
                    for s in app.doc.styles.clone().iter().filter(|s| s.kind != StyleKind::Table) {
                        let marker = if s.kind == StyleKind::Character { "a " } else { "¶ " };
                        if ui.selectable_label(s.id == cur, format!("{}{}", marker, s.name)).clicked() {
                            app.apply_style(&s.id);
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button("Styles Pane").clicked() {
                        app.side_panel = Some(SidePanel::Styles);
                        ui.close();
                    }
                })
                .response
                .on_hover_text("More styles");
            });
        });
    });
    group(ui, "Editing", |ui| {
        ui.vertical(|ui| {
            if small(ui, "🔍 Find", &format!("Find ({})", sc("F"))).clicked() {
                app.show_nav = true;
                app.focus_search = true;
            }
            if small(ui, "⇄ Replace", &format!("Replace ({})", sc("H"))).clicked() {
                app.dialog = Some(Dialog::find_replace(app, true));
            }
            if small(ui, "⬚ Select All", &format!("Select All ({})", sc("A"))).clicked() {
                app.select_all();
            }
        });
    });
}

fn start_painter(app: &mut App, sticky: bool) {
    let (a, b) = app.sel.range();
    let mut c = app.doc.common_chars(a, b);
    c.link = None;
    c.revision = None;
    c.comments.clear();
    let p = app.current_para_props();
    app.format_painter = Some((c, p, sticky));
}

fn style_tile(app: &App, ui: &mut Ui, s: &Style, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(74.0, 56.0), Sense::click());
    let p = ui.painter();
    let bg = if selected { theme::ACCENT_LIGHT } else if resp.hovered() { Color32::from_gray(240) } else { Color32::WHITE };
    p.rect(rect, 2.0, bg, Stroke::new(1.0, if selected { theme::ACCENT } else { theme::BORDER }), egui::StrokeKind::Inside);
    let rc = wordcore::styles::resolve_chars(&app.doc, &s.id, &CharProps::default());
    let face = app.db.face_for(&rc.font, rc.bold, rc.italic);
    let size = (rc.size * 1.1).clamp(9.0, 18.0);
    let sample = if rc.caps { "AABBCC".to_string() } else { "AaBbCc".to_string() };
    let painter = ui.painter_at(rect.shrink(2.0));
    crate::fonts::draw_text(&painter, pos2(rect.left() + 6.0, rect.top() + 28.0), &sample, face, size, theme::c32(rc.color), false, 0.0, app.db);
    p.text(pos2(rect.center().x, rect.bottom() - 9.0), Align2::CENTER_CENTER, &s.name, FontId::proportional(10.5), theme::TEXT);
    resp.on_hover_text(&s.name)
}

fn sort_paragraphs(app: &mut App) {
    let (a, b) = app.sel.range();
    if a.path.cell.is_some() || a.path.story != Story::Body {
        return;
    }
    let (s, e) = (a.path.block, b.path.block);
    if e <= s {
        return;
    }
    if app.doc.body[s..=e].iter().any(|b| matches!(b, Block::Table(_))) {
        return;
    }
    app.checkpoint("Sort", crate::app::EditKind::Other);
    let mut paras: Vec<Block> = app.doc.body[s..=e].to_vec();
    paras.sort_by_key(|b| b.as_para().map(|p| p.text().to_lowercase()).unwrap_or_default());
    app.doc.body.splice(s..=e, paras);
    let end = app.doc.end_of(Story::Body);
    let last = app.doc.clamp(wordcore::edit::Pos::new(e, usize::MAX));
    let _ = end;
    app.select(wordcore::edit::Pos::new(s, 0), last);
    app.changed();
}

// ----------------------------------------------------------------------
// Insert
// ----------------------------------------------------------------------

fn insert(app: &mut App, ui: &mut Ui) {
    group(ui, "Pages", |ui| {
        if big(ui, "📄", "Blank Page", "Insert a blank page").clicked() {
            app.insert_page_break();
            app.insert_page_break();
        }
        if big(ui, "⤵", "Page Break", &format!("Page Break ({})", sc("Enter"))).clicked() {
            app.insert_page_break();
        }
    });
    group(ui, "Tables", |ui| {
        let r = big(ui, "▦", "Table ▾", "Insert Table");
        egui::Popup::menu(&r).show(|ui| {
            ui.label("Insert Table");
            let hover = ui.memory(|m| m.data.get_temp::<(usize, usize)>(egui::Id::unique("tbl-hover"))).unwrap_or((0, 0));
            let mut new_hover = hover;
            let mut chosen = None;
            for row in 0..8 {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for col in 0..10 {
                        let (rr, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                        let on = row < hover.0 && col < hover.1;
                        ui.painter().rect(rr, 0.0, if on { theme::ACCENT_LIGHT } else { Color32::WHITE }, Stroke::new(1.0, if on { theme::ACCENT } else { Color32::from_gray(180) }), egui::StrokeKind::Inside);
                        if resp.hovered() {
                            new_hover = (row + 1, col + 1);
                        }
                        if resp.clicked() {
                            chosen = Some((row + 1, col + 1));
                        }
                    }
                });
            }
            ui.memory_mut(|m| m.data.insert_temp(egui::Id::unique("tbl-hover"), new_hover));
            ui.label(if new_hover.0 > 0 { format!("{}x{} Table", new_hover.1, new_hover.0) } else { String::new() });
            if let Some((r, c)) = chosen {
                app.insert_table(r, c);
                ui.close();
            }
            ui.separator();
            if ui.button("Insert Table...").clicked() {
                app.dialog = Some(Dialog::InsertTable { rows: 2, cols: 5 });
                ui.close();
            }
        });
    });
    group(ui, "Illustrations", |ui| {
        if big(ui, "🖼", "Pictures", "Insert a picture from a file").clicked() {
            if let Some(p) = crate::fileio::picture_dialog() {
                match crate::images::load_file(&p) {
                    Ok(img) => app.insert_image(img, "Insert Picture"),
                    Err(e) => app.dialog = Some(Dialog::Message { title: "Insert Picture".into(), text: e }),
                }
            }
        }
    });
    group(ui, "Links", |ui| {
        if big(ui, "🔗", "Link", &format!("Insert Link ({})", sc("K"))).clicked() {
            app.dialog = Some(Dialog::hyperlink(app));
        }
        if big(ui, "🔖", "Bookmark", "Insert a bookmark").clicked() {
            app.dialog = Some(Dialog::Bookmark { name: String::new() });
        }
    });
    group(ui, "Comments", |ui| {
        if big(ui, "💬", "Comment", "Add a comment about the selection").clicked() {
            app.dialog = Some(Dialog::Comment { text: String::new() });
        }
    });
    group(ui, "Header & Footer", |ui| {
        if big(ui, "▔", "Header", "Edit the header").clicked() {
            app.edit_header(false, None);
        }
        if big(ui, "▁", "Footer", "Edit the footer").clicked() {
            app.edit_header(true, None);
        }
        big_menu(ui, "#", "Page Number", "Insert page numbers", |ui| {
            for (label, footer, al, total) in [
                ("Bottom of Page — Center", true, Align::Center, false),
                ("Bottom of Page — Right", true, Align::Right, false),
                ("Bottom of Page — Page X of Y", true, Align::Center, true),
                ("Top of Page — Right", false, Align::Right, false),
                ("Top of Page — Center", false, Align::Center, false),
            ] {
                if ui.button(label).clicked() {
                    app.insert_page_numbers(footer, al, total);
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Current Position").clicked() {
                app.insert_inline(Inline::Field(Field::PageNumber), "Page Number");
                ui.close();
            }
        });
    });
    group(ui, "Text", |ui| {
        if big(ui, "🗐", "Text Box", "Insert a text box you can place anywhere on the page").clicked() {
            app.insert_text_box();
        }
        ui.vertical(|ui| {
            if small(ui, "📅 Date & Time", "Insert the current date or time").clicked() {
                app.dialog = Some(Dialog::DateTime { selected: 1, update: false });
            }
            ui.menu_button("📑 Quick Parts", |ui| {
                for (n, f) in [("Page", Field::PageNumber), ("NumPages", Field::PageCount)] {
                    if ui.button(format!("Field: {}", n)).clicked() {
                        app.insert_inline(Inline::Field(f.clone()), "Insert Field");
                        ui.close();
                    }
                }
                if ui.button("Field: Author").clicked() {
                    let a = app.doc.properties.author.clone();
                    app.insert_inline(Inline::Field(Field::Other { instr: "AUTHOR".into(), result: a }), "Insert Field");
                    ui.close();
                }
                if ui.button("Field: NumWords").clicked() {
                    app.insert_inline(Inline::Field(Field::Other { instr: "NUMWORDS".into(), result: String::new() }), "Insert Field");
                    ui.close();
                }
            });
            if small(ui, "― Horizontal Line", "Insert a horizontal line").clicked() {
                app.insert_horizontal_line();
            }
        });
    });
    group(ui, "Symbols", |ui| {
        if big(ui, "Ω", "Symbol", "Insert a symbol").clicked() {
            app.dialog = Some(Dialog::Symbol { recent: Vec::new() });
        }
    });
}

// ----------------------------------------------------------------------
// Design
// ----------------------------------------------------------------------

fn design(app: &mut App, ui: &mut Ui) {
    group(ui, "Document Formatting", |ui| {
        for (name, heading_font, body_font, color) in [
            ("Office", "Calibri Light", "Calibri", Color::ACCENT1),
            ("Classic", "Times New Roman", "Times New Roman", Color::rgb(0x1F, 0x1F, 0x1F)),
            ("Modern", "Arial", "Arial", Color::rgb(0x2E, 0x74, 0xB5)),
            ("Elegant", "Cambria", "Cambria", Color::rgb(0x7B, 0x23, 0x23)),
            ("Friendly", "Georgia", "Verdana", Color::rgb(0x38, 0x76, 0x1D)),
        ] {
            let (rect, resp) = ui.allocate_exact_size(vec2(70.0, BODY_H - 18.0), Sense::click());
            let p = ui.painter();
            p.rect(rect, 2.0, if resp.hovered() { Color32::from_gray(240) } else { Color32::WHITE }, Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Inside);
            let hf = app.db.face_for(heading_font, false, false);
            let bf = app.db.face_for(body_font, false, false);
            let painter = ui.painter_at(rect);
            crate::fonts::draw_text(&painter, pos2(rect.left() + 5.0, rect.top() + 18.0), "Title", hf, 15.0, theme::c32(color), false, 0.0, app.db);
            crate::fonts::draw_text(&painter, pos2(rect.left() + 5.0, rect.top() + 32.0), "Body text", bf, 9.5, Color32::from_gray(60), false, 0.0, app.db);
            p.text(pos2(rect.center().x, rect.bottom() - 8.0), Align2::CENTER_CENTER, name, FontId::proportional(10.5), theme::TEXT);
            if resp.on_hover_text(format!("{}: headings in {}, body in {}", name, heading_font, body_font)).clicked() {
                app.apply_theme(heading_font, body_font, color);
            }
        }
    });
    group(ui, "Page Background", |ui| {
        big_menu(ui, "💧", "Watermark", "Add a watermark behind the text", |ui| {
            for t in ["CONFIDENTIAL", "DRAFT", "DO NOT COPY", "SAMPLE", "URGENT"] {
                if ui.button(t).clicked() {
                    let mut s = app.page_setup();
                    s.watermark = Some(t.into());
                    app.set_page_setup(s);
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Custom Watermark...").clicked() {
                app.dialog = Some(Dialog::Watermark { text: app.page_setup().watermark.clone().unwrap_or_else(|| "DRAFT".into()) });
                ui.close();
            }
            if ui.button("Remove Watermark").clicked() {
                let mut s = app.page_setup();
                s.watermark = None;
                app.set_page_setup(s);
                ui.close();
            }
        });
        big_menu(ui, "🎨", "Page Color", "Change the page colour", |ui| {
            if let Some(c) = palette(ui, Some("No Color")) {
                let mut s = app.page_setup();
                s.page_color = c;
                app.set_page_setup(s);
                ui.close();
            }
        });
        big_menu(ui, "▢", "Page Borders", "Add a border around the page", |ui| {
            for (n, b) in [
                ("Box (thin)", Some(Border::single())),
                ("Box (thick)", Some(Border { style: BorderStyle::Thick, width: 2.25, ..Border::single() })),
                ("Double", Some(Border { style: BorderStyle::Double, width: 1.5, ..Border::single() })),
                ("Shadow blue", Some(Border { color: Color::ACCENT1, width: 1.5, ..Border::single() })),
                ("None", None),
            ] {
                if ui.button(n).clicked() {
                    let mut s = app.page_setup();
                    s.page_borders = b.map(|b| Borders { top: Some(b), bottom: Some(b), left: Some(b), right: Some(b), ..Default::default() });
                    app.set_page_setup(s);
                    ui.close();
                }
            }
        });
    });
}

// ----------------------------------------------------------------------
// Layout
// ----------------------------------------------------------------------

fn layout_tab(app: &mut App, ui: &mut Ui) {
    group(ui, "Page Setup", |ui| {
        big_menu(ui, "⊡", "Margins", "Set the page margins", |ui| {
            for (n, t, b, l, r) in [
                ("Normal  (1\" all round)", 72.0, 72.0, 72.0, 72.0),
                ("Narrow  (0.5\")", 36.0, 36.0, 36.0, 36.0),
                ("Moderate  (1\" / 0.75\")", 72.0, 72.0, 54.0, 54.0),
                ("Wide  (1\" / 2\")", 72.0, 72.0, 144.0, 144.0),
                ("Mirrored-like  (1\" / 1.25\")", 72.0, 72.0, 90.0, 72.0),
            ] {
                if ui.button(n).clicked() {
                    let mut s = app.page_setup();
                    s.margin_top = t;
                    s.margin_bottom = b;
                    s.margin_left = l;
                    s.margin_right = r;
                    app.set_page_setup(s);
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Custom Margins...").clicked() {
                app.dialog = Some(Dialog::page_setup(app));
                ui.close();
            }
        });
        big_menu(ui, "⟲", "Orientation", "Portrait or landscape", |ui| {
            let land = { let ps = app.page_setup(); ps.width > ps.height };
            if ui.selectable_label(!land, "Portrait").clicked() && land {
                let mut s = app.page_setup();
                std::mem::swap(&mut s.width, &mut s.height);
                app.set_page_setup(s);
                ui.close();
            }
            if ui.selectable_label(land, "Landscape").clicked() && !land {
                let mut s = app.page_setup();
                std::mem::swap(&mut s.width, &mut s.height);
                app.set_page_setup(s);
                ui.close();
            }
        });
        big_menu(ui, "📃", "Size", "Choose a paper size", |ui| {
            for (n, w, h) in PAPER_SIZES {
                let label = format!("{}  ({:.2}\" × {:.2}\")", n, w / 72.0, h / 72.0);
                if ui.button(label).clicked() {
                    let mut s = app.page_setup();
                    let land = s.width > s.height;
                    s.width = if land { *h } else { *w };
                    s.height = if land { *w } else { *h };
                    app.set_page_setup(s);
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("More Paper Sizes...").clicked() {
                app.dialog = Some(Dialog::page_setup(app));
                ui.close();
            }
        });
        big_menu(ui, "▥", "Columns", "Split text into columns", |ui| {
            for n in 1..=3u32 {
                if ui.selectable_label(app.page_setup().columns == n, ["One", "Two", "Three"][n as usize - 1]).clicked() {
                    let mut s = app.page_setup();
                    s.columns = n;
                    app.set_page_setup(s);
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("More Columns...").clicked() {
                app.dialog = Some(Dialog::Columns { n: app.page_setup().columns, gap: app.page_setup().column_gap });
                ui.close();
            }
        });
        big_menu(ui, "⤵", "Breaks", "Add page and column breaks", |ui| {
            if ui.button("Page break").clicked() {
                app.insert_page_break();
                ui.close();
            }
            if ui.button("Column break").clicked() {
                app.insert_inline(Inline::ColumnBreak, "Column Break");
                ui.close();
            }
            if ui.button("Text wrapping (line break)").clicked() {
                app.insert_inline(Inline::LineBreak, "Line Break");
                ui.close();
            }
            ui.separator();
            ui.label(RichText::new("Section Breaks").strong());
            for (n, k) in [
                ("Next Page", SectionStart::NextPage),
                ("Continuous", SectionStart::Continuous),
                ("Even Page", SectionStart::EvenPage),
                ("Odd Page", SectionStart::OddPage),
            ] {
                if ui.button(n).clicked() {
                    app.insert_section_break(k);
                    ui.close();
                }
            }
        });
    });
    group(ui, "Arrange", |ui| {
        let has = app.selected_object().is_some();
        ui.add_enabled_ui(has, |ui| {
            big_menu(ui, "▭", "Wrap Text", "Choose how text wraps around the selected picture or text box", |ui| wrap_menu(app, ui));
        });
    });
    let pp = app.current_para();
    group(ui, "Paragraph", |ui| {
        Grid2::show(ui, |ui| {
            ui.label("Indent");
            ui.label("");
            ui.label("Spacing");
            ui.label("");
            ui.end_row();
            ui.label("Left:");
            let mut l = pp.indent_left / 72.0;
            if ui.add(egui::DragValue::new(&mut l).speed(0.05).range(-10.0..=20.0).fixed_decimals(1).suffix("\"")).changed() {
                let v = l * 72.0;
                app.apply_para("Indent", move |p| p.indent_left = Some(v));
            }
            ui.label("Before:");
            let mut b = pp.space_before;
            if ui.add(egui::DragValue::new(&mut b).speed(1.0).range(0.0..=1584.0).suffix(" pt")).changed() {
                app.apply_para("Spacing", move |p| p.space_before = Some(b));
            }
            ui.end_row();
            ui.label("Right:");
            let mut r = pp.indent_right / 72.0;
            if ui.add(egui::DragValue::new(&mut r).speed(0.05).range(-10.0..=20.0).fixed_decimals(1).suffix("\"")).changed() {
                let v = r * 72.0;
                app.apply_para("Indent", move |p| p.indent_right = Some(v));
            }
            ui.label("After:");
            let mut a = pp.space_after;
            if ui.add(egui::DragValue::new(&mut a).speed(1.0).range(0.0..=1584.0).suffix(" pt")).changed() {
                app.apply_para("Spacing", move |p| p.space_after = Some(a));
            }
            ui.end_row();
        });
    });
    if app.sel.head.path.cell.is_some() {
        table_group(app, ui);
    }
}

struct Grid2;
impl Grid2 {
    fn show(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
        egui::Grid::new(ui.next_auto_id()).num_columns(4).spacing([6.0, 3.0]).show(ui, add);
    }
}

fn table_group(app: &mut App, ui: &mut Ui) {
    group(ui, "Rows & Columns", |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                if small(ui, "⬆ Insert Above", "Insert a row above").clicked() {
                    app.table_insert_row(true);
                }
                if small(ui, "⬇ Insert Below", "Insert a row below").clicked() {
                    app.table_insert_row(false);
                }
            });
            ui.horizontal(|ui| {
                if small(ui, "⬅ Insert Left", "Insert a column to the left").clicked() {
                    app.table_insert_col(true);
                }
                if small(ui, "➡ Insert Right", "Insert a column to the right").clicked() {
                    app.table_insert_col(false);
                }
            });
            ui.menu_button("✖ Delete", |ui| {
                if ui.button("Delete Rows").clicked() {
                    app.table_delete_row();
                    ui.close();
                }
                if ui.button("Delete Columns").clicked() {
                    app.table_delete_col();
                    ui.close();
                }
                if ui.button("Delete Table").clicked() {
                    app.table_delete();
                    ui.close();
                }
            });
        });
    });
    group(ui, "Merge & Style", |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                if small(ui, "⊞ Merge Right", "Merge with the cell to the right").clicked() {
                    app.table_merge_right();
                }
                if small(ui, "⊟ Split", "Split cell").clicked() {
                    app.table_split_cell();
                }
            });
            ui.horizontal(|ui| {
                if small(ui, "↔ Distribute", "Distribute columns evenly").clicked() {
                    app.table_distribute_columns();
                }
                if small(ui, "☰ Header Row", "Repeat first row as header on each page").clicked() {
                    app.table_toggle_header();
                }
            });
            ui.horizontal(|ui| {
                ui.menu_button("🎨 Shading", |ui| {
                    if let Some(c) = palette(ui, Some("No Color")) {
                        app.table_set_shading(c);
                        ui.close();
                    }
                });
                ui.menu_button("▦ Styles", |ui| {
                    let b = Border::single();
                    for (n, h, band, bc) in [
                        ("Table Grid", None, None, Color::BLACK),
                        ("Blue Header", Some(Color::rgb(0x44, 0x72, 0xC4)), Some(Color::rgb(0xD9, 0xE2, 0xF3)), Color::rgb(0x8E, 0xAA, 0xDB)),
                        ("Orange Header", Some(Color::rgb(0xED, 0x7D, 0x31)), Some(Color::rgb(0xFB, 0xE4, 0xD5)), Color::rgb(0xF4, 0xB0, 0x83)),
                        ("Green Header", Some(Color::rgb(0x70, 0xAD, 0x47)), Some(Color::rgb(0xE2, 0xEF, 0xD9)), Color::rgb(0xA8, 0xD0, 0x8D)),
                        ("Gray Banded", Some(Color::rgb(0x59, 0x59, 0x59)), Some(Color::rgb(0xF2, 0xF2, 0xF2)), Color::rgb(0xBF, 0xBF, 0xBF)),
                    ] {
                        if ui.button(n).clicked() {
                            app.table_style(h, band, Borders::all(Border { color: bc, ..b }));
                            ui.close();
                        }
                    }
                    if ui.button("No Borders").clicked() {
                        app.table_style(None, None, Borders::default());
                        ui.close();
                    }
                });
            });
        });
    });
}

// ----------------------------------------------------------------------
// References
// ----------------------------------------------------------------------

fn references(app: &mut App, ui: &mut Ui) {
    group(ui, "Table of Contents", |ui| {
        if big(ui, "☰", "Table of Contents", "Insert a table of contents built from your headings").clicked() {
            app.insert_toc();
        }
        if big(ui, "⟳", "Update Table", "Update the table of contents").clicked() {
            app.insert_toc();
        }
    });
    group(ui, "Footnotes", |ui| {
        if big(ui, "AB¹", "Insert Footnote", "Add a note at the bottom of the page").clicked() {
            app.insert_footnote(false);
        }
        if big(ui, "ⁿ", "Insert Endnote", "Add a note at the end of the document").clicked() {
            app.insert_footnote(true);
        }
    });
    group(ui, "Captions", |ui| {
        if big(ui, "🏷", "Insert Caption", "Add a numbered caption").clicked() {
            app.dialog = Some(Dialog::Caption { label: "Figure".into() });
        }
    });
}

// ----------------------------------------------------------------------
// Review
// ----------------------------------------------------------------------

fn review(app: &mut App, ui: &mut Ui) {
    group(ui, "Proofing", |ui| {
        if big(ui, "✔", "Spelling", "Spelling & Grammar (F7)").clicked() {
            app.side_panel = Some(SidePanel::Spelling);
            crate::proof::spelling_next(app);
        }
        if big(ui, "📖", "Thesaurus", "Find synonyms (Shift+F7)").clicked() {
            let w = crate::proof::word_for_thesaurus(app);
            app.side_panel = Some(SidePanel::Thesaurus { word: w });
        }
        if big(ui, "123", "Word Count", "Count words, characters, pages").clicked() {
            app.dialog = Some(Dialog::WordCount);
        }
    });
    group(ui, "Comments", |ui| {
        if big(ui, "💬", "New Comment", "Comment on the selection").clicked() {
            app.dialog = Some(Dialog::Comment { text: String::new() });
        }
        ui.vertical(|ui| {
            if small(ui, "🗑 Delete", "Delete the comment at the cursor").clicked() {
                let ids = app.current_chars().comments;
                let at = app.doc.para(&app.sel.head.path).map(|p| p.props_at(app.sel.head.offset + 1).comments).unwrap_or_default();
                let id = ids.first().or(at.first()).copied();
                match id {
                    Some(id) => app.delete_comment(id),
                    None => app.status = "Place the cursor in commented text to delete its comment.".into(),
                }
            }
            if small(ui, "☰ Show Comments", "Show the comments pane").clicked() {
                app.side_panel = Some(SidePanel::Comments);
            }
        });
    });
    group(ui, "Tracking", |ui| {
        let on = app.doc.settings.track_changes;
        let r = big(ui, if on { "📝●" } else { "📝" }, if on { "Tracking: On" } else { "Track Changes" }, "Keep track of changes made to this document");
        if r.clicked() {
            app.toggle_track_changes();
        }
        ui.vertical(|ui| {
            combo_frame(ui, true);
            let label = match app.markup {
                MarkupMode::AllMarkup => "All Markup",
                MarkupMode::NoMarkup => "No Markup",
                MarkupMode::Original => "Original",
            };
            egui::ComboBox::from_id_salt("markup").width(110.0).selected_text(label).show_ui(ui, |ui| {
                for (m, n) in [(MarkupMode::AllMarkup, "All Markup"), (MarkupMode::NoMarkup, "No Markup"), (MarkupMode::Original, "Original")] {
                    if ui.selectable_value(&mut app.markup, m, n).clicked() {
                        app.layout_dirty = true;
                    }
                }
            });
            combo_frame(ui, false);
        });
    });
    group(ui, "Changes", |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.menu_button("✔ Accept", |ui| {
                    if ui.button("Accept and Move to Next").clicked() {
                        app.accept_reject(true, false);
                        ui.close();
                    }
                    if ui.button("Accept All Changes").clicked() {
                        app.accept_reject(true, true);
                        ui.close();
                    }
                });
                ui.menu_button("✖ Reject", |ui| {
                    if ui.button("Reject and Move to Next").clicked() {
                        app.accept_reject(false, false);
                        ui.close();
                    }
                    if ui.button("Reject All Changes").clicked() {
                        app.accept_reject(false, true);
                        ui.close();
                    }
                });
            });
            ui.horizontal(|ui| {
                if small(ui, "◀ Previous", "Previous change").clicked() {
                    app.next_change(false);
                }
                if small(ui, "Next ▶", "Next change").clicked() {
                    app.next_change(true);
                }
            });
        });
    });
}

// ----------------------------------------------------------------------
// View
// ----------------------------------------------------------------------

fn view(app: &mut App, ui: &mut Ui) {
    group(ui, "Show", |ui| {
        ui.vertical(|ui| {
            ui.checkbox(&mut app.show_ruler, "Ruler");
            ui.checkbox(&mut app.show_nav, "Navigation Pane");
            if ui.checkbox(&mut app.show_marks, "Formatting Marks").changed() {
                app.layout_dirty = true;
            }
        });
    });
    group(ui, "Zoom", |ui| {
        if big(ui, "🔍", "Zoom", "Choose a zoom level").clicked() {
            app.dialog = Some(Dialog::Zoom { pct: (app.zoom * 100.0).round() });
        }
        if big(ui, "1:1", "100%", "Zoom to 100%").clicked() {
            app.zoom = 1.0;
        }
        ui.vertical(|ui| {
            if small(ui, "▯ One Page", "Fit a whole page in the window").clicked() {
                app.zoom_mode = Some(crate::app::ZoomMode::OnePage);
            }
            if small(ui, "↔ Page Width", "Fit the page width to the window").clicked() {
                app.zoom_mode = Some(crate::app::ZoomMode::PageWidth);
            }
        });
    });
}

impl App {
    pub fn with_cell_borders(&mut self, b: Borders) {
        self.table_style_borders(b);
    }
}
