//! The document canvas: draws pages and handles mouse and keyboard input.

use egui::{pos2, vec2, Color32, CursorIcon, Event, Key, Modifiers, Pos2, Rect, Sense, Stroke, Ui};
use wordcore::edit::{ordered, Pos};
use wordcore::layout::{LineStyle, Shape};
use wordcore::{Inline, Story};

use crate::app::{App, Selection};
use crate::dialogs::Dialog;
use crate::fonts::draw_text;
use crate::theme;

const GAP: f32 = 18.0;
const TOP: f32 = 20.0;

/// Screen points per document point.
pub fn scale(app: &App) -> f32 {
    app.zoom * 96.0 / 72.0
}

/// Per-frame geometry of the canvas.
pub struct Geometry {
    pub origin: Pos2,
    pub page_x: f32,
    pub page_tops: Vec<f32>,
    pub k: f32,
}

impl Geometry {
    pub fn page_screen_rect(&self, app: &App, i: usize) -> Rect {
        self.page_rect(app, i)
    }
    fn page_rect(&self, app: &App, i: usize) -> Rect {
        let p = &app.layout.pages[i];
        Rect::from_min_size(pos2(self.origin.x + self.page_x, self.origin.y + self.page_tops[i]), vec2(p.width * self.k, p.height * self.k))
    }
    fn to_screen(&self, app: &App, page: usize, x: f32, y: f32) -> Pos2 {
        let r = self.page_rect(app, page);
        pos2(r.left() + x * self.k, r.top() + y * self.k)
    }
    /// Screen point → (page, x, y) in document points.
    fn to_doc(&self, app: &App, p: Pos2) -> Option<(usize, f32, f32)> {
        if app.layout.pages.is_empty() {
            return None;
        }
        let mut best = 0;
        for i in 0..app.layout.pages.len() {
            let r = self.page_rect(app, i);
            if p.y >= r.top() - GAP / 2.0 {
                best = i;
            }
        }
        let r = self.page_rect(app, best);
        Some((best, (p.x - r.left()) / self.k, (p.y - r.top()) / self.k))
    }
}

pub fn show(app: &mut App, ui: &mut Ui) {
    app.ensure_layout();
    let k = scale(app);
    let markup_w = if !app.doc.comments.is_empty() && app.markup == crate::app::MarkupMode::AllMarkup { 230.0 } else { 0.0 };
    let max_w = app.layout.pages.iter().map(|p| p.width).fold(0.0f32, f32::max) * k + markup_w;
    let mut tops = Vec::with_capacity(app.layout.pages.len());
    let mut y = TOP;
    for p in &app.layout.pages {
        tops.push(y);
        y += p.height * k + GAP;
    }
    let total_h = y + TOP;
    let caret_target = if app.scroll_to_caret {
        app.layout.caret_on_page(app.sel.head, app.layout.page_of(app.sel.anchor).unwrap_or(0)).map(|c| (c.page, c.x, c.top, c.height))
    } else {
        None
    };
    let page_target = app.scroll_to_page.take();
    egui::ScrollArea::both().auto_shrink([false, false]).id_salt("canvas-scroll").show(ui, |ui| {
        let avail = ui.available_width();
        let content_w = (max_w + 40.0).max(avail);
        let (rect, resp) = ui.allocate_exact_size(vec2(content_w, total_h), Sense::click_and_drag());
        let page_x = ((content_w - max_w) / 2.0).max(20.0);
        let geo = Geometry { origin: rect.min, page_x, page_tops: tops.clone(), k };
        app.canvas_origin_x = rect.min.x + page_x;
        // Scroll requests.
        if let Some((page, x, top, h)) = caret_target {
            if page < tops.len() {
                let p = geo.to_screen(app, page, x, top);
                let r = Rect::from_min_size(p - vec2(30.0, 30.0), vec2(60.0, h * k + 60.0));
                ui.scroll_to_rect(r, None);
            }
            app.scroll_to_caret = false;
        }
        if let Some(pg) = page_target {
            if pg < tops.len() {
                let r = geo.page_rect(app, pg);
                ui.scroll_to_rect(Rect::from_min_size(r.min, vec2(10.0, 10.0)), Some(egui::Align::TOP));
            }
        }
        handle_pointer(app, ui, &resp, &geo);
        paint(app, ui, &geo, rect);
        let id = resp.id;
        handle_keys(app, ui, id, &resp);
        context_menu(app, &resp);
    });
}

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

fn line_shape(painter: &egui::Painter, a: Pos2, b: Pos2, width: f32, color: Color32, style: LineStyle) {
    let stroke = Stroke::new(width.max(0.6), color);
    match style {
        LineStyle::Solid => {
            painter.line_segment([a, b], stroke);
        }
        LineStyle::Dotted => {
            painter.extend(egui::Shape::dotted_line(&[a, b], color, (width * 2.5).max(2.0), (width * 0.6).max(0.6)));
        }
        LineStyle::Dashed => {
            painter.extend(egui::Shape::dashed_line(&[a, b], stroke, (width * 5.0).max(4.0), (width * 3.0).max(3.0)));
        }
        LineStyle::Double => {
            let d = (width * 1.5).max(1.5);
            let n = if (a.y - b.y).abs() < 0.01 { vec2(0.0, d) } else { vec2(d, 0.0) };
            painter.line_segment([a - n / 2.0, b - n / 2.0], Stroke::new((width * 0.6).max(0.6), color));
            painter.line_segment([a + n / 2.0, b + n / 2.0], Stroke::new((width * 0.6).max(0.6), color));
        }
        LineStyle::Wavy => {
            wavy(painter, a.x, b.x, a.y, color, width);
        }
    }
}

fn wavy(painter: &egui::Painter, x0: f32, x1: f32, y: f32, color: Color32, width: f32) {
    let mut pts = Vec::new();
    let step = 2.5;
    let mut x = x0;
    let mut up = true;
    while x <= x1 {
        pts.push(pos2(x, y + if up { -1.2 } else { 1.2 }));
        up = !up;
        x += step;
    }
    if pts.len() >= 2 {
        painter.add(egui::Shape::line(pts, Stroke::new(width.max(0.8), color)));
    }
}

fn paint(app: &mut App, ui: &mut Ui, geo: &Geometry, full: Rect) {
    let painter = ui.painter_at(full);
    let clip = ui.clip_rect();
    painter.rect_filled(clip, 0.0, theme::CANVAS_BG);
    let k = geo.k;
    let (sa, sb) = app.sel.range();
    let story = app.story();
    let focused = app.canvas_focused;
    let time = ui.input(|i| i.time);
    let caret_on = ((time - app.caret_blink) * 2.0).rem_euclid(2.0) < 1.3 || !focused;
    if focused {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(400));
    }
    let spell = crate::proof::visible_issues(app, geo, clip);
    let mut tex = std::mem::take(&mut app.textures);
    let mut ime_rect = None;
    for (i, page) in app.layout.pages.iter().enumerate() {
        let r = geo.page_rect(app, i);
        if !r.intersects(clip) {
            continue;
        }
        // shadow + paper
        painter.rect_filled(r.translate(vec2(2.0, 3.0)), 0.0, Color32::from_black_alpha(40));
        painter.rect_filled(r, 0.0, Color32::WHITE);
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_gray(200)), egui::StrokeKind::Outside);
        let pclip = painter.with_clip_rect(r.intersect(clip));
        let to = |x: f32, y: f32| pos2(r.left() + x * k, r.top() + y * k);
        // dim body while editing header/footer and vice versa
        let editing_hf = story != Story::Body;
        // corner marks of the text area (Word's crop marks)
        let ps = &app.doc.page;
        let mk = Stroke::new(1.0, Color32::from_gray(190));
        for (cx, cy, dx, dy) in [
            (ps.margin_left, page.body_top, -1.0, -1.0),
            (page.width - ps.margin_right, page.body_top, 1.0, -1.0),
            (ps.margin_left, page.body_bottom, -1.0, 1.0),
            (page.width - ps.margin_right, page.body_bottom, 1.0, 1.0),
        ] {
            let p = to(cx, cy);
            pclip.line_segment([p, p + vec2(dx * 12.0 * k.min(1.5), 0.0)], mk);
            pclip.line_segment([p, p + vec2(0.0, dy * 12.0 * k.min(1.5))], mk);
        }
        for s in &page.shapes {
            match s {
                Shape::Rect { x, y, w, h, color } => {
                    pclip.rect_filled(Rect::from_min_size(to(*x, *y), vec2(w * k, h * k)), 0.0, theme::c32(*color));
                }
                Shape::Line { x1, y1, x2, y2, width, color, style } => {
                    line_shape(&pclip, to(*x1, *y1), to(*x2, *y2), width * k, theme::c32(*color), *style);
                }
                Shape::Text { x, y, text, face, size, color, spacing, fake_bold, .. } => {
                    let p = to(*x, *y);
                    if p.y < clip.top() - 100.0 || p.y > clip.bottom() + 100.0 {
                        continue;
                    }
                    draw_text(&pclip, p, text, *face, size * k, theme::c32(*color), *fake_bold, spacing * k, app.db);
                }
                Shape::Image { x, y, w, h, image } => {
                    let rr = Rect::from_min_size(to(*x, *y), vec2(w * k, h * k));
                    match crate::images::texture(&mut tex, ui.ctx(), image) {
                        Some(tex) => {
                            pclip.image(tex, rr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        None => {
                            pclip.rect(rr, 0.0, Color32::from_gray(235), Stroke::new(1.0, Color32::GRAY), egui::StrokeKind::Inside);
                        }
                    }
                }
                Shape::Watermark { cx, cy, text, face, size, color, angle } => {
                    let fid = crate::fonts::font_id(*face, size * k);
                    let galley = pclip.layout_no_wrap(text.clone(), fid, theme::c32(*color));
                    let a = -angle.to_radians();
                    let half = galley.size() / 2.0;
                    let c = to(*cx, *cy);
                    let rot = egui::emath::Rot2::from_angle(a);
                    let pos = c - rot * half;
                    pclip.add(egui::epaint::TextShape::new(pos, galley, theme::c32(*color)).with_angle(a));
                }
            }
        }
        // Header/footer editing boundary.
        if editing_hf {
            let dash = Stroke::new(1.0, theme::ACCENT);
            let hy = page.body_top - 4.0;
            let fy = page.body_bottom + 4.0;
            pclip.extend(egui::Shape::dashed_line(&[to(0.0, hy), to(page.width, hy)], dash, 4.0, 3.0));
            pclip.extend(egui::Shape::dashed_line(&[to(0.0, fy), to(page.width, fy)], dash, 4.0, 3.0));
            let tag = |y: f32, label: &str| {
                let g = pclip.layout_no_wrap(label.to_string(), egui::FontId::proportional(11.0), Color32::WHITE);
                let pr = Rect::from_min_size(to(4.0, y) + vec2(0.0, 2.0), g.size() + vec2(8.0, 2.0));
                pclip.rect_filled(pr, 2.0, theme::ACCENT);
                pclip.galley(pr.min + vec2(4.0, 1.0), g, Color32::WHITE);
            };
            let first = i == 0 && app.doc.page.title_page;
            tag(hy, if first { "First Page Header" } else { "Header" });
            tag(fy, if first { "First Page Footer" } else { "Footer" });
            // dim body
            pclip.rect_filled(Rect::from_min_max(to(0.0, page.body_top), to(page.width, page.body_bottom)), 0.0, Color32::from_white_alpha(110));
        } else {
            // dim headers/footers
            let top_r = Rect::from_min_max(to(0.0, 0.0), to(page.width, page.body_top - 2.0));
            let bot_r = Rect::from_min_max(to(0.0, page.body_bottom + 2.0), to(page.width, page.height));
            pclip.rect_filled(top_r, 0.0, Color32::from_white_alpha(100));
            pclip.rect_filled(bot_r, 0.0, Color32::from_white_alpha(100));
        }
        // Spelling squiggles.
        for (pg, x0, x1, y) in &spell {
            if *pg == i {
                wavy(&pclip, r.left() + x0 * k, r.left() + x1 * k, r.top() + y * k + 2.0, Color32::from_rgb(0xE0, 0x20, 0x20), 1.0);
            }
        }
        // Formatting marks.
        if app.show_marks {
            let mark_col = Color32::from_rgb(0x4A, 0x6E, 0xB8);
            for l in &page.lines {
                let Some(p) = app.doc.para(&l.path) else { continue };
                let chars = p.chars_vec();
                for off in l.start..l.end {
                    let c = chars.get(off).copied().unwrap_or(' ');
                    let x0 = l.caret_x(off);
                    let x1 = l.caret_x(off + 1);
                    let cy = l.baseline - 3.0;
                    match c {
                        ' ' => {
                            pclip.circle_filled(to((x0 + x1) / 2.0, cy), (0.9 * k).max(0.8), mark_col);
                        }
                        '\t' => {
                            let a = to(x0 + 1.0, cy);
                            let b = to(x1 - 1.0, cy);
                            pclip.arrow(a, b - a, Stroke::new(0.8, mark_col));
                        }
                        _ => {}
                    }
                }
                if l.last {
                    let x = l.caret_x(l.end) + 1.0;
                    let fid = egui::FontId::proportional(11.0 * k);
                    pclip.text(to(x, l.baseline), egui::Align2::LEFT_BOTTOM, if l.path.cell.is_some() && p.is_empty() && false { "¤" } else { "¶" }, fid, mark_col);
                }
            }
        }
        // Selection.
        if sa != sb {
            for (x, y, w, h) in app.layout.selection_rects(&app.doc, sa, sb, i) {
                pclip.rect_filled(Rect::from_min_size(to(x, y), vec2(w * k, h * k)), 0.0, theme::SELECTION);
            }
        }
        // Find highlights.
        if !app.find.matches.is_empty() && app.show_nav {
            for (a, b) in app.find.matches.iter().take(2000) {
                for (x, y, w, h) in app.layout.selection_rects(&app.doc, *a, *b, i) {
                    pclip.rect_filled(Rect::from_min_size(to(x, y), vec2(w * k, h * k)), 0.0, Color32::from_rgba_unmultiplied(255, 220, 0, 90));
                }
            }
        }
        // Caret.
        if sa == sb && caret_on {
            if let Some(c) = app.layout.caret_on_page(app.sel.head, i) {
                if c.page == i {
                    let top = to(c.x, c.top);
                    // caret height: line height, but not taller than ~1.3x text
                    let h = c.height * k;
                    painter.line_segment([top + vec2(0.0, 1.0), top + vec2(0.0, h - 1.0)], Stroke::new(1.5, Color32::BLACK));
                    ime_rect = Some(Rect::from_min_size(top, vec2(2.0, h)));
                }
            }
        }
        // Comment balloons in the markup area.
        if !page.comment_marks.is_empty() && app.markup == crate::app::MarkupMode::AllMarkup {
            let mut y_next = 0.0f32;
            for m in &page.comment_marks {
                let Some(c) = app.doc.comments.iter().find(|c| c.id == m.id) else { continue };
                let bx = r.right() + 14.0;
                let by = (r.top() + m.y * k).max(y_next);
                let text: String = c.paragraphs.iter().map(|p| p.text()).collect::<Vec<_>>().join("\n");
                let g = painter.layout(format!("{}\n{}", c.author, text), egui::FontId::proportional(11.5), theme::TEXT, 196.0);
                let br = Rect::from_min_size(pos2(bx, by), vec2(206.0, g.size().y + 10.0));
                let anchor = to(m.x, m.y);
                painter.line_segment([anchor, pos2(r.right(), anchor.y)], Stroke::new(1.0, Color32::from_rgb(0xD0, 0x80, 0x40)));
                painter.line_segment([pos2(r.right(), anchor.y), pos2(bx, by + 8.0)], Stroke::new(1.0, Color32::from_rgb(0xD0, 0x80, 0x40)));
                painter.rect(br, 4.0, Color32::from_rgb(0xFD, 0xF3, 0xE6), Stroke::new(1.0, Color32::from_rgb(0xE0, 0xA0, 0x60)), egui::StrokeKind::Inside);
                painter.galley(br.min + vec2(5.0, 5.0), g, theme::TEXT);
                y_next = br.bottom() + 6.0;
            }
        }
    }
    app.textures = tex;
    if ime_rect.is_some() {
        app.ime_rect = ime_rect;
    }
}

// ---------------------------------------------------------------------------
// Pointer
// ---------------------------------------------------------------------------

fn hit(app: &App, geo: &Geometry, p: Pos2) -> Option<(usize, Pos, bool)> {
    let (page, x, y) = geo.to_doc(app, p)?;
    let pg = &app.layout.pages[page];
    let in_hf = y < pg.body_top - 2.0 || y > pg.body_bottom + 2.0;
    let body = app.story() == Story::Body;
    // While editing headers, clicks in the header/footer zone hit header lines.
    let want_body = if body { true } else { !in_hf };
    let pos = app.layout.hit(page, x, y, want_body).or_else(|| app.layout.hit(page, x, y, !want_body))?;
    Some((page, pos, in_hf))
}

fn handle_pointer(app: &mut App, ui: &mut Ui, resp: &egui::Response, geo: &Geometry) {
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Text);
        // link hover hint
        if let Some(p) = resp.hover_pos() {
            if let Some((_, pos, _)) = hit(app, geo, p) {
                if let Some(url) = app.link_at(pos) {
                    let tip = if cfg!(target_os = "macos") { "⌘+Click to follow link" } else { "Ctrl+Click to follow link" };
                    resp.clone().on_hover_text(format!("{}\n{}", url, tip));
                }
            }
        }
    }
    let mods = ui.input(|i| i.modifiers);
    if resp.clicked() || resp.drag_started() || resp.double_clicked() || resp.triple_clicked() || resp.secondary_clicked() {
        resp.request_focus();
        app.canvas_focused = true;
    }
    let Some(pointer) = resp.interact_pointer_pos() else { return };
    let Some((page, pos, in_hf)) = hit(app, geo, pointer) else { return };
    if resp.secondary_clicked() {
        // Keep the selection if clicking inside it.
        let (a, b) = app.sel.range();
        if !(pos >= a && pos <= b && a != b) {
            app.set_caret(pos, false);
        }
        return;
    }
    // Header/footer switching.
    if resp.double_clicked() && in_hf && app.story() == Story::Body {
        let footer = geo.to_doc(app, pointer).map(|(_, _, y)| y > app.layout.pages[page].body_bottom).unwrap_or(false);
        app.edit_header(footer, page == 0);
        return;
    }
    if (resp.clicked() || resp.double_clicked()) && !in_hf && app.story() != Story::Body {
        app.close_header();
        if let Some((_, pos, _)) = hit(app, geo, pointer) {
            app.set_caret(pos, false);
        }
        return;
    }
    if resp.triple_clicked() {
        let (a, b) = app.doc.paragraph_range(pos);
        let b = app.doc.right(b);
        let b = if b.path == a.path { b } else { b.path.at(0) };
        let b = if b.path != a.path { b } else { app.doc.paragraph_range(pos).1 };
        app.select(a, b);
        return;
    }
    if resp.double_clicked() {
        let (a, b) = app.doc.word_at(pos);
        app.select(a, b);
        return;
    }
    if resp.clicked() {
        if mods.command {
            if let Some(url) = app.link_at(pos) {
                follow_link(app, ui.ctx(), &url);
                return;
            }
            // Ctrl+click selects the sentence.
            let (a, b) = app.doc.sentence_at(pos);
            app.select(a, b);
            return;
        }
        // Format painter applies on click-release.
        if app.format_painter.is_some() && !app.sel.is_empty() {
            crate::ribbon::apply_format_painter(app);
            return;
        }
        app.set_caret(pos, mods.shift);
        if let Some(Inline::Image(_)) = image_at(app, pos) {
            app.select(pos, app.doc.right(pos));
        }
        app.goal_x = None;
        return;
    }
    if resp.drag_started() {
        app.set_caret(pos, mods.shift);
    } else if resp.dragged() {
        if app.sel.head != pos {
            app.sel.head = app.doc.clamp(pos);
            app.caret_blink = ui.input(|i| i.time);
        }
        // auto-scroll near edges
        let clip = ui.clip_rect();
        if pointer.y > clip.bottom() - 10.0 {
            ui.scroll_with_delta(vec2(0.0, -12.0));
        } else if pointer.y < clip.top() + 10.0 {
            ui.scroll_with_delta(vec2(0.0, 12.0));
        }
    }
    if resp.drag_stopped() && app.format_painter.is_some() && !app.sel.is_empty() {
        crate::ribbon::apply_format_painter(app);
    }
}

fn image_at(app: &App, pos: Pos) -> Option<Inline> {
    let p = app.doc.para(&pos.path)?;
    let (ri, _) = p.run_at(pos.offset)?;
    match &p.runs[ri].content {
        i @ Inline::Image(_) => Some(i.clone()),
        _ => None,
    }
}

pub fn follow_link(app: &mut App, ctx: &egui::Context, url: &str) {
    if let Some(name) = url.strip_prefix('#') {
        // Internal bookmark / TOC link.
        if let Some(block) = name.strip_prefix("_Toc").and_then(|n| n.parse::<usize>().ok()) {
            let p = app.doc.clamp(Pos::new(block, 0));
            app.set_caret(p, false);
            return;
        }
        for path in app.doc.para_paths(Story::Body) {
            if let Some(p) = app.doc.para(&path) {
                let mut acc = 0;
                for r in &p.runs {
                    if let Inline::Bookmark(b) = &r.content {
                        if b == name {
                            app.set_caret(path.at(acc), false);
                            return;
                        }
                    }
                    acc += r.len();
                }
            }
        }
        app.status = format!("Bookmark \"{}\" not found.", name);
    } else {
        ctx.open_url(egui::OpenUrl::new_tab(url));
    }
}

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------

fn handle_keys(app: &mut App, ui: &mut Ui, id: egui::Id, resp: &egui::Response) {
    let focused_other = ui.memory(|m| m.focused().map(|f| f != id).unwrap_or(false));
    if app.dialog.as_ref().map(|d| d.is_modal()).unwrap_or(false) {
        return;
    }
    if focused_other {
        app.canvas_focused = false;
        return;
    }
    if !resp.has_focus() {
        resp.request_focus();
    }
    app.canvas_focused = true;
    ui.memory_mut(|m| m.set_focus_lock_filter(id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true }));
    // IME position
    if let Some(r) = app.ime_rect.take() {
        ui.ctx().output_mut(|o| o.ime = Some(egui::output::IMEOutput { rect: r, cursor_rect: r, purpose: Default::default(), should_interrupt_composition: false }));
    }
    let events = ui.input(|i| i.events.clone());
    for ev in events {
        match ev {
            Event::Text(t) => {
                if t.chars().all(|c| !c.is_control()) {
                    app.type_text(&t);
                }
            }
            Event::Ime(egui::ImeEvent::Commit(t)) => app.type_text(&t),
            Event::Copy => {
                app.copy(false);
            }
            Event::Cut => {
                app.copy(true);
            }
            Event::Paste(s) => {
                let shift = ui.input(|i| i.modifiers.shift);
                app.paste(Some(s), shift);
            }
            Event::Key { key, pressed: true, modifiers, .. } => key_press(app, ui.ctx(), key, modifiers),
            _ => {}
        }
    }
}

fn word_mod(m: &Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        m.alt
    } else {
        m.ctrl
    }
}

fn key_press(app: &mut App, ctx: &egui::Context, key: Key, m: Modifiers) {
    let cmd = m.command;
    let shift = m.shift;
    let word = word_mod(&m);
    let mac_line = cfg!(target_os = "macos") && m.mac_cmd;
    app.caret_blink = ctx.input(|i| i.time);
    match key {
        Key::ArrowLeft | Key::ArrowRight => {
            let right = key == Key::ArrowRight;
            if !shift && !app.sel.is_empty() && !word && !mac_line {
                let (a, b) = app.sel.range();
                app.set_caret(if right { b } else { a }, false);
                return;
            }
            let p = app.sel.head;
            let np = if mac_line {
                app.ensure_layout();
                app.layout.line_bounds(p).map(|(a, b)| if right { b } else { a }).unwrap_or(p)
            } else if word {
                if right { app.doc.word_right(p) } else { app.doc.word_left(p) }
            } else if right {
                app.doc.right(p)
            } else {
                app.doc.left(p)
            };
            app.set_caret(np, shift);
        }
        Key::ArrowUp | Key::ArrowDown => {
            app.ensure_layout();
            let down = key == Key::ArrowDown;
            let p = app.sel.head;
            if cmd && !cfg!(target_os = "macos") {
                // Ctrl+Up/Down: previous/next paragraph start.
                let np = if down { app.doc.next_path(&p.path).map(|x| x.at(0)).unwrap_or(app.doc.end_of(p.path.story)) } else if p.offset > 0 { p.path.at(0) } else { app.doc.prev_path(&p.path).map(|x| x.at(0)).unwrap_or(p) };
                app.set_caret(np, shift);
                return;
            }
            if mac_line {
                let np = if down { app.doc.end_of(p.path.story) } else { app.doc.start_of(p.path.story) };
                app.set_caret(np, shift);
                return;
            }
            let x = app.goal_x.or_else(|| app.layout.caret(p).map(|c| c.x)).unwrap_or(0.0);
            let np = app.layout.vertical(p, x, down).unwrap_or(if down { app.doc.end_of(p.path.story) } else { app.doc.start_of(p.path.story) });
            app.set_caret(np, shift);
            app.goal_x = Some(x);
        }
        Key::Home | Key::End => {
            app.ensure_layout();
            let p = app.sel.head;
            let end = key == Key::End;
            let np = if cmd {
                if end { app.doc.end_of(p.path.story) } else { app.doc.start_of(p.path.story) }
            } else {
                app.layout.line_bounds(p).map(|(a, b)| if end { b } else { a }).unwrap_or(p)
            };
            app.set_caret(np, shift);
        }
        Key::PageUp | Key::PageDown => {
            app.ensure_layout();
            let p = app.sel.head;
            if let Some(c) = app.layout.caret(p) {
                let pages = app.layout.pages.len();
                let target = if key == Key::PageDown { (c.page + 1).min(pages - 1) } else { c.page.saturating_sub(1) };
                let np = app.layout.hit(target, c.x, c.top + 2.0, p.path.story == Story::Body).unwrap_or(p);
                app.set_caret(np, shift);
            }
        }
        Key::Backspace => app.backspace(word),
        Key::Delete => app.delete_forward(word),
        Key::Enter => {
            if shift {
                app.insert_inline(Inline::LineBreak, "Typing");
            } else if cmd {
                app.insert_page_break();
            } else {
                app.enter();
            }
        }
        Key::Tab => app.tab(shift),
        Key::Escape => {
            if app.format_painter.is_some() {
                app.format_painter = None;
            } else if app.story() != Story::Body {
                app.close_header();
            } else if !app.sel.is_empty() {
                let h = app.sel.head;
                app.set_caret(h, false);
            }
        }
        Key::F7 => {
            if shift {
                let w = crate::proof::word_for_thesaurus(app);
                app.side_panel = Some(crate::app::SidePanel::Thesaurus { word: w });
            } else {
                app.side_panel = Some(crate::app::SidePanel::Spelling);
                crate::proof::spelling_next(app);
            }
        }
        Key::F12 => {
            app.save_as();
        }
        _ if cmd => command_key(app, key, m),
        _ => {}
    }
}

fn command_key(app: &mut App, key: Key, m: Modifiers) {
    let shift = m.shift;
    let alt = m.alt;
    match key {
        Key::B => app.toggle_bold(),
        Key::I => app.toggle_italic(),
        Key::U => app.toggle_underline(if shift { wordcore::Underline::Double } else { wordcore::Underline::Single }),
        Key::Z if shift => app.redo(),
        Key::Z => app.undo(),
        Key::Y => app.redo(),
        Key::A => app.select_all(),
        Key::S if shift => {
            app.save_as();
        }
        Key::S => {
            app.save();
        }
        Key::O => app.request(crate::app::Pending::Open(None)),
        Key::N if shift => app.apply_style("Normal"),
        Key::N => app.request(crate::app::Pending::New),
        Key::P => app.print(),
        Key::F => {
            app.show_nav = true;
            app.focus_search = true;
        }
        Key::H => app.dialog = Some(Dialog::find_replace(app, true)),
        Key::G => app.dialog = Some(Dialog::GoTo { page: String::new() }),
        Key::E => app.set_align(wordcore::Align::Center),
        Key::L if shift => app.toggle_list(0, None),
        Key::L => app.set_align(wordcore::Align::Left),
        Key::R => app.set_align(wordcore::Align::Right),
        Key::J => app.set_align(wordcore::Align::Justify),
        Key::Num1 if alt => app.apply_style("Heading1"),
        Key::Num2 if alt => app.apply_style("Heading2"),
        Key::Num3 if alt => app.apply_style("Heading3"),
        Key::Num1 => app.set_line_spacing(wordcore::LineSpacing::Multiple(1.0)),
        Key::Num2 => app.set_line_spacing(wordcore::LineSpacing::Multiple(2.0)),
        Key::Num5 => app.set_line_spacing(wordcore::LineSpacing::Multiple(1.5)),
        Key::Num0 => {
            let cur = app.current_para().space_before;
            let v = if cur > 0.0 { 0.0 } else { 12.0 };
            app.apply_para("Spacing", move |p| p.space_before = Some(v));
        }
        Key::CloseBracket => app.grow_font(true),
        Key::OpenBracket => app.grow_font(false),
        Key::Period if shift => app.grow_font(true),
        Key::Comma if shift => app.grow_font(false),
        Key::Equals | Key::Plus => app.toggle_vert(if shift || key == Key::Plus { wordcore::VertAlign::Superscript } else { wordcore::VertAlign::Subscript }),
        Key::K => app.dialog = Some(Dialog::hyperlink(app)),
        Key::D => app.dialog = Some(Dialog::font(app)),
        Key::M if shift => app.change_indent(-1),
        Key::M => app.change_indent(1),
        Key::Space => app.apply_chars("Clear Formatting", |c| {
            let keep = (c.link.clone(), c.revision.clone(), c.comments.clone(), c.style.clone());
            *c = wordcore::CharProps::default();
            c.link = keep.0;
            c.revision = keep.1;
            c.comments = keep.2;
            if keep.3.as_deref() == Some("Hyperlink") {
                c.style = keep.3;
            }
        }),
        Key::W => app.request(crate::app::Pending::Close),
        Key::Q => app.request(crate::app::Pending::Close),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Context menu
// ---------------------------------------------------------------------------

fn context_menu(app: &mut App, resp: &egui::Response) {
    resp.context_menu(|ui| {
        ui.set_min_width(200.0);
        // Spelling suggestions first.
        if crate::proof::context_suggestions(app, ui) {
            ui.separator();
        }
        let has_sel = !app.sel.is_empty();
        if ui.add_enabled(has_sel, egui::Button::new("Cut").shortcut_text(sc("X"))).clicked() {
            app.copy(true);
            ui.close();
        }
        if ui.add_enabled(has_sel, egui::Button::new("Copy").shortcut_text(sc("C"))).clicked() {
            app.copy(false);
            ui.close();
        }
        if ui.add(egui::Button::new("Paste").shortcut_text(sc("V"))).clicked() {
            app.paste(None, false);
            ui.close();
        }
        if ui.button("Paste as Plain Text").clicked() {
            app.paste(None, true);
            ui.close();
        }
        ui.separator();
        if ui.button("Font...").clicked() {
            app.dialog = Some(Dialog::font(app));
            ui.close();
        }
        if ui.button("Paragraph...").clicked() {
            app.dialog = Some(Dialog::paragraph(app));
            ui.close();
        }
        if app.sel.head.path.cell.is_some() {
            ui.separator();
            ui.menu_button("Insert", |ui| {
                if ui.button("Insert Rows Above").clicked() {
                    app.table_insert_row(true);
                    ui.close();
                }
                if ui.button("Insert Rows Below").clicked() {
                    app.table_insert_row(false);
                    ui.close();
                }
                if ui.button("Insert Columns to the Left").clicked() {
                    app.table_insert_col(true);
                    ui.close();
                }
                if ui.button("Insert Columns to the Right").clicked() {
                    app.table_insert_col(false);
                    ui.close();
                }
            });
            ui.menu_button("Delete", |ui| {
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
            if ui.button("Merge with Cell to the Right").clicked() {
                app.table_merge_right();
                ui.close();
            }
            if ui.button("Split Cell").clicked() {
                app.table_split_cell();
                ui.close();
            }
            if ui.button("Distribute Columns Evenly").clicked() {
                app.table_distribute_columns();
                ui.close();
            }
        }
        ui.separator();
        if app.link_at(app.sel.head).is_some() {
            if ui.button("Edit Link...").clicked() {
                app.dialog = Some(Dialog::hyperlink(app));
                ui.close();
            }
            if ui.button("Open Link").clicked() {
                if let Some(u) = app.link_at(app.sel.head) {
                    let ctx = ui.ctx().clone();
                    follow_link(app, &ctx, &u);
                }
                ui.close();
            }
            if ui.button("Remove Link").clicked() {
                app.remove_link();
                ui.close();
            }
        } else if ui.add(egui::Button::new("Link...").shortcut_text(sc("K"))).clicked() {
            app.dialog = Some(Dialog::hyperlink(app));
            ui.close();
        }
        if ui.button("New Comment").clicked() {
            app.dialog = Some(Dialog::Comment { text: String::new() });
            ui.close();
        }
        if ui.button("Synonyms...").clicked() {
            let w = crate::proof::word_for_thesaurus(app);
            app.side_panel = Some(crate::app::SidePanel::Thesaurus { word: w });
            ui.close();
        }
    });
}

pub fn sc(k: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{}", k)
    } else {
        format!("Ctrl+{}", k)
    }
}

/// Is `pos` within the current selection?
#[allow(dead_code)]
pub fn in_selection(sel: &Selection, pos: Pos) -> bool {
    let (a, b) = ordered(sel.anchor, sel.head);
    pos >= a && pos <= b
}
