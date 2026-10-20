//! Page layout: line breaking, pagination, tables, headers/footers, footnotes.
//!
//! The output is a list of pages, each with a flat draw list ([`Shape`]) used by
//! both the screen renderer and the PDF exporter, and the laid-out lines with
//! caret positions used for hit-testing and cursor movement.
//! All coordinates are points from the page's top-left corner.

use std::collections::HashMap;
use std::sync::Arc;

use crate::edit::{CellRef, ParaPath, Pos};
use crate::fonts::{FaceId, FontDb};
use crate::model::*;
use crate::styles::{resolve_chars, resolve_para, ListCounter, ResolvedChars, ResolvedPara};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineStyle {
    Solid,
    Dotted,
    Dashed,
    Double,
    Wavy,
}

/// One drawing primitive.
#[derive(Clone, Debug)]
pub enum Shape {
    Rect { x: f32, y: f32, w: f32, h: f32, color: Color },
    Line { x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: Color, style: LineStyle },
    /// Text drawn left to right from `x` on the baseline `y`.
    Text {
        x: f32,
        y: f32,
        text: String,
        face: FaceId,
        size: f32,
        color: Color,
        /// Extra space after every character (character spacing), points.
        spacing: f32,
        fake_bold: bool,
        fake_italic: bool,
        /// Hyperlink target, if any (for PDF links / clicking).
        link: Option<String>,
    },
    Image { x: f32, y: f32, w: f32, h: f32, image: Arc<ImageData> },
    /// Rotated text centred at (cx, cy); angle in degrees counter-clockwise.
    Watermark { cx: f32, cy: f32, text: String, face: FaceId, size: f32, color: Color, angle: f32 },
}

/// A laid-out line of text, used for caret placement and hit-testing.
#[derive(Clone, Debug)]
pub struct Line {
    pub path: ParaPath,
    pub start: usize,
    pub end: usize,
    pub top: f32,
    pub height: f32,
    pub baseline: f32,
    /// Horizontal extent of the line's text box (between indents).
    pub left: f32,
    pub right: f32,
    /// x of the caret at each offset `start..=end`.
    pub carets: Vec<f32>,
    /// True when this line ends its paragraph.
    pub last: bool,
}

impl Line {
    pub fn caret_x(&self, offset: usize) -> f32 {
        let i = offset.saturating_sub(self.start).min(self.carets.len().saturating_sub(1));
        self.carets.get(i).copied().unwrap_or(self.left)
    }

    /// Offset whose caret is closest to `x`.
    pub fn offset_at(&self, x: f32) -> usize {
        let mut best = (f32::MAX, self.start);
        let n = if self.last { self.carets.len() } else { self.carets.len().saturating_sub(1).max(1) };
        for (i, cx) in self.carets.iter().enumerate().take(n) {
            let d = (cx - x).abs();
            if d < best.0 {
                best = (d, self.start + i);
            }
        }
        best.1
    }
}

/// A comment anchor on a page (for drawing balloons in the margin).
#[derive(Clone, Debug)]
pub struct CommentMark {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug)]
pub struct Page {
    pub index: usize,
    /// Displayed page number.
    pub number: u32,
    pub width: f32,
    pub height: f32,
    pub shapes: Vec<Shape>,
    pub lines: Vec<Line>,
    /// Top/bottom of the body text area.
    pub body_top: f32,
    pub body_bottom: f32,
    pub comment_marks: Vec<CommentMark>,
}

/// A heading found during layout (navigation pane, table of contents).
#[derive(Clone, Debug)]
pub struct Heading {
    pub path: ParaPath,
    pub level: u8,
    pub text: String,
    pub page: usize,
}

#[derive(Clone, Debug, Default)]
pub struct DocLayout {
    pub pages: Vec<Page>,
    /// For each paragraph: its lines as (page, line index).
    pub index: HashMap<ParaPath, Vec<(usize, usize)>>,
    pub headings: Vec<Heading>,
    /// Number of lines in the body (Word count dialog).
    pub body_lines: usize,
}

#[derive(Clone, Debug, Default)]
pub struct LayoutOptions {
    /// Show hidden text.
    pub show_hidden: bool,
    /// Page count from the previous layout (for NUMPAGES fields).
    pub page_count_hint: u32,
    /// Show tracked deletions (Simple Markup off / All Markup).
    pub show_deletions: bool,
    /// Draw tracked changes in colour (All Markup); false shows the final text plainly.
    pub show_markup: bool,
}

impl LayoutOptions {
    pub fn new() -> Self {
        LayoutOptions { show_hidden: false, page_count_hint: 1, show_deletions: true, show_markup: true }
    }
}

// ---------------------------------------------------------------------------
// Paragraph line breaking
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum PieceKind {
    Char(char),
    Space,
    Tab,
    LineBreak,
    PageBreak,
    ColumnBreak,
    Image(Arc<ImageData>),
    /// Unbreakable text box (fields, note references).
    Boxed(String),
    Zero,
}

#[derive(Clone, Debug)]
struct Piece {
    kind: PieceKind,
    width: f32,
    face: FaceId,
    size: f32,
    /// Index into the paragraph's resolved style table.
    style: usize,
    ascent: f32,
    descent: f32,
    line_height: f32,
    /// Positive raises the glyph (superscript).
    shift: f32,
    /// For images: their height.
    image_h: f32,
    hidden: bool,
    /// Tab: leader to draw.
    leader: TabLeader,
}

/// Context passed down for field evaluation and note numbering.
#[derive(Clone, Copy, Debug, Default)]
struct FieldCtx {
    page_number: u32,
    page_count: u32,
}

struct ParaLines {
    lines: Vec<PLine>,
    pieces: Vec<Piece>,
    styles: Vec<ResolvedChars>,
    rp: ResolvedPara,
    /// List label to draw on the first line: (text, x, face, size, color).
    label: Option<(String, f32, FaceId, f32, Color)>,
    /// Footnote/endnote ids referenced, by piece index.
    note_refs: Vec<(usize, u32)>,
}

#[derive(Clone, Debug)]
struct PLine {
    /// Piece range [a, b).
    a: usize,
    b: usize,
    /// x of each piece (relative to the column's left edge), length b - a.
    xs: Vec<f32>,
    /// x after the last visible piece.
    end_x: f32,
    height: f32,
    /// Baseline offset from line top.
    baseline: f32,
    left: f32,
    right: f32,
    /// Ends with a page/column break piece.
    brk: Option<PieceKind>,
    last: bool,
    /// Extra width per space for justification.
    justify: f32,
}

fn display_char(c: char, rc: &ResolvedChars) -> (char, f32) {
    if rc.caps {
        return (c.to_uppercase().next().unwrap_or(c), 1.0);
    }
    if rc.small_caps && c.is_lowercase() {
        return (c.to_uppercase().next().unwrap_or(c), 0.8);
    }
    (c, 1.0)
}

fn vert_scale(rc: &ResolvedChars) -> (f32, f32) {
    match rc.vert_align {
        VertAlign::Baseline => (1.0, 0.0),
        VertAlign::Superscript => (0.65, rc.size * 0.33),
        VertAlign::Subscript => (0.65, -rc.size * 0.14),
    }
}

fn field_text(doc: &Document, f: &Field, ctx: FieldCtx) -> String {
    match f {
        Field::PageNumber => ctx.page_number.to_string(),
        Field::PageCount => ctx.page_count.max(1).to_string(),
        Field::Date(pic) => crate::util::format_date(pic, crate::util::now_parts()),
        Field::Other { instr, result } => {
            let i = instr.trim().to_uppercase();
            if i.starts_with("NUMWORDS") {
                return doc.plain_text().split_whitespace().count().to_string();
            }
            if i.starts_with("FILENAME") || i.starts_with("TITLE") {
                if !doc.properties.title.is_empty() && i.starts_with("TITLE") {
                    return doc.properties.title.clone();
                }
            }
            if i.starts_with("AUTHOR") && !doc.properties.author.is_empty() {
                return doc.properties.author.clone();
            }
            result.clone()
        }
    }
}

struct Ctx<'a> {
    doc: &'a Document,
    db: &'a FontDb,
    opts: &'a LayoutOptions,
    /// Footnote id → display number.
    note_numbers: &'a HashMap<u32, u32>,
    endnote_numbers: &'a HashMap<u32, u32>,
}

fn build_pieces(cx: &Ctx, para: &Paragraph, rp: &ResolvedPara, fc: FieldCtx) -> (Vec<Piece>, Vec<ResolvedChars>, Vec<(usize, u32)>) {
    let doc = cx.doc;
    let db = cx.db;
    let mut pieces = Vec::with_capacity(para.len() + 1);
    let mut styles: Vec<ResolvedChars> = Vec::new();
    let mut note_refs = Vec::new();
    for run in &para.runs {
        let mut rc = resolve_chars(doc, &rp.style, &run.props);
        if matches!(run.content, Inline::FootnoteRef(_) | Inline::EndnoteRef(_)) && run.props.vert_align.is_none() {
            rc.vert_align = VertAlign::Superscript;
        }
        let deleted = matches!(&rc.revision, Some(r) if r.kind == RevisionKind::Delete);
        let hidden = (rc.hidden && !cx.opts.show_hidden) || (deleted && !cx.opts.show_deletions);
        let si = match styles.iter().position(|s| *s == rc) {
            Some(i) => i,
            None => {
                styles.push(rc.clone());
                styles.len() - 1
            }
        };
        let (vscale, shift) = vert_scale(&rc);
        let face_id = db.face_for(&rc.font, rc.bold, rc.italic);
        let mk = |kind: PieceKind, face: FaceId, size: f32, width: f32| {
            let f = db.face(face);
            Piece {
                kind,
                width: if hidden { 0.0 } else { width },
                face,
                size,
                style: si,
                ascent: f.ascent * size + shift.max(0.0),
                descent: f.descent * size + (-shift).max(0.0),
                line_height: f.line_height() * size,
                shift,
                image_h: 0.0,
                hidden,
                leader: TabLeader::None,
            }
        };
        match &run.content {
            Inline::Text(s) => {
                for c in s.chars() {
                    let (dc, scale) = display_char(c, &rc);
                    let size = rc.size * vscale * scale;
                    let fid = db.resolve_char(face_id, dc);
                    let w = db.face(fid).advance(dc) * size + rc.spacing;
                    let kind = if c == ' ' || c == '\u{00A0}' && false { PieceKind::Space } else { PieceKind::Char(dc) };
                    let mut p = mk(kind, fid, size, w);
                    if scale != 1.0 || vscale != 1.0 {
                        // Keep line metrics of the full-size font.
                        let f = db.face(face_id);
                        p.ascent = p.ascent.max(f.ascent * rc.size * 0.8);
                        p.line_height = f.line_height() * rc.size;
                    }
                    pieces.push(p);
                }
            }
            Inline::Tab => pieces.push(mk(PieceKind::Tab, face_id, rc.size, 0.0)),
            Inline::LineBreak => pieces.push(mk(PieceKind::LineBreak, face_id, rc.size, 0.0)),
            Inline::PageBreak => pieces.push(mk(PieceKind::PageBreak, face_id, rc.size, 0.0)),
            Inline::ColumnBreak => pieces.push(mk(PieceKind::ColumnBreak, face_id, rc.size, 0.0)),
            Inline::TextBox(_) => pieces.push(mk(PieceKind::Zero, face_id, rc.size, 0.0)),
            Inline::Bookmark(_) => {
                let mut p = mk(PieceKind::Zero, face_id, rc.size, 0.0);
                p.ascent = 0.0;
                p.descent = 0.0;
                p.line_height = 0.0;
                pieces.push(p)
            }
            Inline::Image(img) => {
                let mut p = mk(PieceKind::Image(img.data.clone()), face_id, rc.size, img.width);
                p.ascent = img.height;
                p.descent = 0.0;
                p.image_h = img.height;
                p.line_height = img.height;
                pieces.push(p);
            }
            Inline::Field(f) => {
                let t = field_text(doc, f, fc);
                let size = rc.size * vscale;
                let w: f32 = t.chars().map(|c| db.face(db.resolve_char(face_id, c)).advance(c) * size + rc.spacing).sum();
                pieces.push(mk(PieceKind::Boxed(t), face_id, size, w));
            }
            Inline::FootnoteRef(id) | Inline::EndnoteRef(id) => {
                let n = if matches!(run.content, Inline::FootnoteRef(_)) {
                    cx.note_numbers.get(id).copied().unwrap_or(*id)
                } else {
                    cx.endnote_numbers.get(id).copied().unwrap_or(*id)
                };
                let t = if matches!(run.content, Inline::FootnoteRef(_)) { n.to_string() } else { crate::styles::roman(n).to_lowercase() };
                let size = rc.size * vscale;
                let w: f32 = t.chars().map(|c| db.face(face_id).advance(c) * size).sum();
                if matches!(run.content, Inline::FootnoteRef(_)) {
                    note_refs.push((pieces.len(), *id));
                }
                pieces.push(mk(PieceKind::Boxed(t), face_id, size, w));
            }
        }
    }
    (pieces, styles, note_refs)
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

/// Next tab stop position (relative to column left) after `x`.
fn next_tab(rp: &ResolvedPara, default_tab: f32, x: f32, first_line: bool) -> (f32, TabAlign, TabLeader) {
    let eps = 0.01;
    let mut best: Option<(f32, TabAlign, TabLeader)> = None;
    for t in &rp.tabs {
        if t.align == TabAlign::Clear {
            continue;
        }
        if t.pos > x + eps && best.map(|b| t.pos < b.0).unwrap_or(true) {
            best = Some((t.pos, t.align, t.leader));
        }
    }
    // Hanging indent acts as an implicit tab stop on the first line.
    if first_line && rp.indent_first < 0.0 && rp.indent_left > x + eps && best.map(|b| rp.indent_left < b.0).unwrap_or(true) {
        best = Some((rp.indent_left, TabAlign::Left, TabLeader::None));
    }
    if let Some(b) = best {
        return b;
    }
    let dt = if default_tab > 1.0 { default_tab } else { 36.0 };
    let pos = ((x + eps) / dt).floor() * dt + dt;
    (pos, TabAlign::Left, TabLeader::None)
}

fn layout_paragraph(cx: &Ctx, para: &Paragraph, width: f32, fc: FieldCtx, counter: &mut ListCounter) -> ParaLines {
    let doc = cx.doc;
    let db = cx.db;
    let rp = resolve_para(doc, &para.props);
    let (mut pieces, mut styles, note_refs) = build_pieces(cx, para, &rp, fc);
    // Paragraph mark metrics (empty paragraphs and line height floor).
    let mark_rc = resolve_chars(doc, &rp.style, &if para.runs.is_empty() { para.mark.clone() } else { para.runs.last().map(|r| r.props.clone()).unwrap_or_default() });
    let mark_face = db.face_for(&mark_rc.font, mark_rc.bold, mark_rc.italic);
    let mf = db.face(mark_face);
    let (mark_asc, mark_desc, mark_lh) = (mf.ascent * mark_rc.size, mf.descent * mark_rc.size, mf.line_height() * mark_rc.size);
    styles.push(mark_rc.clone());

    // List label.
    let mut label = None;
    let mut label_end = None;
    if let Some(l) = rp.list {
        if let Some((text, font)) = counter.next_label(doc, l) {
            if !text.is_empty() {
                let first_rc = para.runs.first().map(|r| resolve_chars(doc, &rp.style, &r.props)).unwrap_or(mark_rc.clone());
                let fam = font.unwrap_or_else(|| first_rc.font.clone());
                let fam = if fam.eq_ignore_ascii_case("Symbol") || fam.eq_ignore_ascii_case("Wingdings") { first_rc.font.clone() } else { fam };
                let face = db.face_for(&fam, first_rc.bold, first_rc.italic);
                let text = match text.as_str() {
                    "\u{F0B7}" | "" => "•".to_string(),
                    "\u{F0A7}" => "▪".to_string(),
                    "o" if l.level % 3 == 1 => "◦".to_string(),
                    _ => text,
                };
                let w: f32 = text.chars().map(|c| db.face(db.resolve_char(face, c)).advance(c) * first_rc.size).sum();
                let x = rp.indent_left + rp.indent_first;
                label_end = Some(x + w);
                label = Some((text, x, face, first_rc.size, first_rc.color));
            }
        }
    }

    let avail_right = width - rp.indent_right;
    let mut lines: Vec<PLine> = Vec::new();
    let n = pieces.len();
    let mut i = 0;
    let mut first = true;
    loop {
        let left = rp.indent_left + if first { rp.indent_first } else { 0.0 };
        let mut x = left;
        if first {
            if let Some(le) = label_end {
                // Number followed by a tab to the text position.
                let (pos, _, _) = next_tab(&rp, doc.settings.default_tab, le, true);
                x = if le < rp.indent_left - 0.01 { rp.indent_left } else { pos };
            }
        }
        let line_start = i;
        let mut xs: Vec<f32> = Vec::new();
        let mut last_break: Option<usize> = None; // piece index after which we can break
        let mut brk = None;
        let mut j = i;
        let mut overflow_at: Option<usize> = None;
        while j < n {
            let k = pieces[j].kind.clone();
            match k {
                PieceKind::LineBreak | PieceKind::PageBreak | PieceKind::ColumnBreak => {
                    xs.push(x);
                    j += 1;
                    brk = Some(k);
                    break;
                }
                PieceKind::Tab => {
                    let st = &styles[pieces[j].style];
                    let _ = st;
                    let (pos, align, leader) = next_tab(&rp, doc.settings.default_tab, x, first);
                    // width of following segment for right/center/decimal tabs
                    let mut seg = 0.0;
                    let mut dec = None;
                    let mut q = j + 1;
                    while q < n && !matches!(pieces[q].kind, PieceKind::Tab | PieceKind::LineBreak | PieceKind::PageBreak | PieceKind::ColumnBreak) {
                        if dec.is_none() && matches!(pieces[q].kind, PieceKind::Char('.') | PieceKind::Char(',')) {
                            dec = Some(seg);
                        }
                        seg += pieces[q].width;
                        q += 1;
                    }
                    let target = match align {
                        TabAlign::Right => pos - seg,
                        TabAlign::Center => pos - seg / 2.0,
                        TabAlign::Decimal => pos - dec.unwrap_or(seg),
                        _ => pos,
                    };
                    let w = if target > x { target - x } else if align == TabAlign::Left { pos - x } else { 0.0 };
                    pieces[j].width = if pieces[j].hidden { 0.0 } else { w.max(0.0) };
                    pieces[j].leader = leader;
                    if x + pieces[j].width > avail_right + 0.01 && j > i {
                        overflow_at = Some(j);
                        break;
                    }
                    xs.push(x);
                    x += pieces[j].width;
                    last_break = Some(j);
                    j += 1;
                }
                PieceKind::Space => {
                    xs.push(x);
                    x += pieces[j].width;
                    last_break = Some(j);
                    j += 1;
                }
                _ => {
                    let w = pieces[j].width;
                    if x + w > avail_right + 0.01 && j > i {
                        overflow_at = Some(j);
                        break;
                    }
                    xs.push(x);
                    x += w;
                    let c = match pieces[j].kind {
                        PieceKind::Char(c) => Some(c),
                        _ => None,
                    };
                    if let Some(c) = c {
                        let next_is_letter = matches!(pieces.get(j + 1).map(|p| &p.kind), Some(PieceKind::Char(n)) if n.is_alphanumeric());
                        if (c == '-' || c == '\u{2013}' || c == '\u{2014}' || c == '/') && next_is_letter || is_cjk(c) {
                            last_break = Some(j);
                        }
                        if matches!(pieces.get(j + 1).map(|p| &p.kind), Some(PieceKind::Char(n)) if is_cjk(*n)) {
                            last_break = Some(j);
                        }
                    }
                    if matches!(pieces[j].kind, PieceKind::Image(_)) {
                        last_break = Some(j);
                    }
                    j += 1;
                }
            }
        }
        let mut end = j;
        if let Some(o) = overflow_at {
            match last_break {
                Some(b) if b >= i => {
                    end = b + 1;
                }
                _ => end = o.max(i + 1),
            }
            // Hang trailing spaces: include following spaces in this line.
            while end < n && pieces[end].kind == PieceKind::Space {
                end += 1;
            }
            xs.truncate(end - line_start);
            while xs.len() < end - line_start {
                // spaces appended past the overflow: position after previous
                let prev = xs.last().copied().unwrap_or(left);
                let pw = pieces[line_start + xs.len() - 1].width;
                xs.push(prev + pw);
            }
            brk = None;
        }
        // Visible end x (exclude trailing spaces and breaks).
        let mut vis_end = end;
        while vis_end > line_start && matches!(pieces[vis_end - 1].kind, PieceKind::Space | PieceKind::LineBreak | PieceKind::PageBreak | PieceKind::ColumnBreak) {
            vis_end -= 1;
        }
        let end_x = if vis_end > line_start { xs[vis_end - 1 - line_start] + pieces[vis_end - 1].width } else { x.min(xs.first().copied().unwrap_or(x)) };
        let is_last = end >= n && overflow_at.is_none();
        // Line metrics.
        let mut asc: f32 = 0.0;
        let mut desc: f32 = 0.0;
        let mut lh: f32 = 0.0;
        for p in &pieces[line_start..end] {
            if p.hidden {
                continue;
            }
            asc = asc.max(p.ascent);
            desc = desc.max(p.descent);
            lh = lh.max(p.line_height);
        }
        if asc == 0.0 && desc == 0.0 {
            asc = mark_asc;
            desc = mark_desc;
            lh = mark_lh;
        }
        if let Some((_, _, _, size, _)) = &label {
            if first {
                asc = asc.max(size * 0.9);
            }
        }
        let natural = lh.max(asc + desc);
        let (height, baseline) = match rp.line_spacing {
            LineSpacing::Multiple(m) => {
                let h = natural * m.max(0.1);
                (h, h - desc - (natural - asc - desc).max(0.0) * 0.0 - (natural - asc - desc).max(0.0) / 2.0 * 0.0 - (lh - asc - desc).max(0.0) * 0.0)
            }
            LineSpacing::AtLeast(v) => {
                let h = natural.max(v);
                (h, h - desc)
            }
            LineSpacing::Exactly(v) => {
                let h = v.max(1.0);
                (h, (h - desc).max(h * 0.75))
            }
        };
        // Alignment.
        let content_w = end_x - left;
        let avail = avail_right - left;
        let mut shift = 0.0;
        let mut justify = 0.0;
        let ends_with_break = matches!(brk, Some(PieceKind::LineBreak));
        match rp.align {
            Align::Left => {}
            Align::Center => shift = ((avail - content_w) / 2.0).max(0.0),
            Align::Right => shift = (avail - content_w).max(0.0),
            Align::Justify => {
                if !is_last && !ends_with_break && brk.is_none() {
                    // Spaces after the last tab, before visible end.
                    let last_tab = (line_start..vis_end).rev().find(|&q| pieces[q].kind == PieceKind::Tab);
                    let from = last_tab.map(|t| t + 1).unwrap_or(line_start);
                    let spaces = (from..vis_end).filter(|&q| pieces[q].kind == PieceKind::Space).count();
                    if spaces > 0 && avail > content_w {
                        justify = (avail_right - end_x) / spaces as f32;
                        let mut add = 0.0;
                        for q in line_start..end {
                            xs[q - line_start] += add;
                            if q >= from && q < vis_end && pieces[q].kind == PieceKind::Space {
                                add += justify;
                            }
                        }
                    }
                }
            }
        }
        if shift > 0.0 {
            for v in xs.iter_mut() {
                *v += shift;
            }
        }
        let end_x = if justify > 0.0 { avail_right } else { end_x + shift };
        lines.push(PLine { a: line_start, b: end, xs, end_x, height, baseline, left, right: avail_right, brk: brk.clone(), last: is_last, justify });
        first = false;
        i = end;
        if i >= n {
            // A trailing line break needs an empty line after it.
            if matches!(brk, Some(PieceKind::LineBreak)) {
                let l = rp.indent_left;
                lines.push(PLine {
                    a: n,
                    b: n,
                    xs: vec![],
                    end_x: l,
                    height: mark_lh.max(1.0) * match rp.line_spacing {
                        LineSpacing::Multiple(m) => m,
                        _ => 1.0,
                    },
                    baseline: mark_asc,
                    left: l,
                    right: avail_right,
                    brk: None,
                    last: true,
                    justify: 0.0,
                });
            } else if let Some(l) = lines.last_mut() {
                l.last = true;
            }
            break;
        }
    }
    if lines.is_empty() {
        let l = rp.indent_left + rp.indent_first;
        let (h, b) = match rp.line_spacing {
            LineSpacing::Multiple(m) => (mark_lh * m, mark_lh * m - mark_desc),
            LineSpacing::AtLeast(v) => (mark_lh.max(v), mark_lh.max(v) - mark_desc),
            LineSpacing::Exactly(v) => (v, (v - mark_desc).max(v * 0.75)),
        };
        let x0 = match rp.align {
            Align::Center => l + (avail_right - l) / 2.0,
            Align::Right => avail_right,
            _ => label_end.map(|le| if le < rp.indent_left { rp.indent_left } else { next_tab(&rp, doc.settings.default_tab, le, true).0 }).unwrap_or(l),
        };
        lines.push(PLine { a: 0, b: 0, xs: vec![], end_x: x0, height: h, baseline: b, left: l, right: avail_right, brk: None, last: true, justify: 0.0 });
    }
    ParaLines { lines, pieces, styles, rp, label, note_refs }
}

/// Emit shapes and a [`Line`] for one laid-out line at the given origin.
#[allow(clippy::too_many_arguments)]
fn emit_line(cx: &Ctx, pl: &ParaLines, li: usize, path: ParaPath, ox: f32, top: f32, page: &mut Page, first_line: bool, offset_base: &[usize]) {
    let line = &pl.lines[li];
    let base_y = top + line.baseline;
    let pieces = &pl.pieces;
    // Label.
    if first_line {
        if let Some((text, x, face, size, color)) = &pl.label {
            page.shapes.push(Shape::Text {
                x: ox + x,
                y: base_y,
                text: text.clone(),
                face: cx.db.resolve_char(*face, text.chars().next().unwrap_or(' ')),
                size: *size,
                color: *color,
                spacing: 0.0,
                fake_bold: false,
                fake_italic: false,
                link: None,
            });
        }
    }
    // Backgrounds first (highlight, shading, comments).
    let mut q = line.a;
    while q < line.b {
        let p = &pieces[q];
        let st = &pl.styles[p.style];
        let bg = st.highlight.or(st.shading).or_else(|| if st.comments.is_empty() { None } else { Some(Color::rgb(0xFC, 0xE4, 0xA8)) });
        if let (Some(color), false) = (bg, p.hidden) {
            let x0 = ox + line.xs[q - line.a];
            let mut r = q;
            let mut x1 = x0;
            while r < line.b && pl.pieces[r].style == p.style {
                x1 = ox + line.xs[r - line.a] + pieces[r].width + if pieces[r].kind == PieceKind::Space { line.justify } else { 0.0 };
                r += 1;
            }
            // Don't extend highlight over trailing spaces at line end.
            let asc = st.size * 0.95;
            let desc = st.size * 0.3;
            page.shapes.push(Shape::Rect { x: x0, y: base_y - asc, w: (x1 - x0).max(0.0), h: asc + desc, color });
            if !st.comments.is_empty() && !page.comment_marks.iter().any(|m| st.comments.contains(&m.id)) {
                for id in &st.comments {
                    page.comment_marks.push(CommentMark { id: *id, x: x0, y: base_y - asc });
                }
            }
            q = r;
        } else {
            q += 1;
        }
    }
    // Text runs grouped by style + face.
    let mut q = line.a;
    while q < line.b {
        let p = &pieces[q];
        if p.hidden {
            q += 1;
            continue;
        }
        let st = &pl.styles[p.style];
        let color = match (&st.revision, cx.opts.show_markup) {
            (Some(r), true) => revision_color(&r.author),
            _ => st.color,
        };
        let face = cx.db.face(p.face);
        match &p.kind {
            PieceKind::Char(_) | PieceKind::Space => {
                // Group consecutive chars with same style & face and no justification gaps.
                let x0 = ox + line.xs[q - line.a];
                let mut text = String::new();
                let mut r = q;
                while r < line.b {
                    let pr = &pieces[r];
                    if pr.style != p.style || pr.face != p.face || pr.size != p.size || pr.hidden {
                        break;
                    }
                    match pr.kind {
                        PieceKind::Char(c) => text.push(c),
                        PieceKind::Space => {
                            if line.justify > 0.0 {
                                // break group at justified spaces so words are positioned exactly
                                if !text.is_empty() {
                                    break;
                                }
                                r += 1;
                                break;
                            }
                            text.push(' ')
                        }
                        _ => break,
                    }
                    r += 1;
                }
                let trimmed = text.trim_end();
                if !trimmed.is_empty() {
                    page.shapes.push(Shape::Text {
                        x: x0,
                        y: base_y - p.shift,
                        text: if text.trim().is_empty() { String::new() } else { text.clone() },
                        face: p.face,
                        size: p.size,
                        color,
                        spacing: st.spacing,
                        fake_bold: face.fake_bold,
                        fake_italic: face.fake_italic,
                        link: st.link.clone(),
                    });
                }
                if r == q {
                    r += 1;
                }
                q = r;
            }
            PieceKind::Boxed(t) => {
                page.shapes.push(Shape::Text {
                    x: ox + line.xs[q - line.a],
                    y: base_y - p.shift,
                    text: t.clone(),
                    face: p.face,
                    size: p.size,
                    color,
                    spacing: st.spacing,
                    fake_bold: face.fake_bold,
                    fake_italic: face.fake_italic,
                    link: st.link.clone(),
                });
                q += 1;
            }
            PieceKind::Image(data) => {
                let x = ox + line.xs[q - line.a];
                page.shapes.push(Shape::Image { x, y: base_y - p.image_h, w: p.width, h: p.image_h, image: data.clone() });
                q += 1;
            }
            PieceKind::Tab => {
                if p.leader != TabLeader::None && p.width > 6.0 {
                    let x = ox + line.xs[q - line.a];
                    let ch = match p.leader {
                        TabLeader::Dot => '.',
                        TabLeader::Hyphen => '-',
                        _ => '_',
                    };
                    let cw = face.advance(ch) * p.size;
                    if cw > 0.0 {
                        // Align leader characters to a grid like Word.
                        let start = ((x + 2.0) / cw).ceil() * cw;
                        let count = ((x + p.width - 2.0 - start) / cw).floor().max(0.0) as usize;
                        if count > 0 {
                            page.shapes.push(Shape::Text {
                                x: start,
                                y: base_y,
                                text: std::iter::repeat_n(ch, count).collect(),
                                face: p.face,
                                size: p.size,
                                color,
                                spacing: 0.0,
                                fake_bold: false,
                                fake_italic: false,
                                link: None,
                            });
                        }
                    }
                }
                q += 1;
            }
            _ => q += 1,
        }
    }
    // Decorations: underline, strike, revisions.
    let mut q = line.a;
    while q < line.b {
        let p = &pieces[q];
        let st = &pl.styles[p.style];
        let rev = if cx.opts.show_markup { st.revision.as_ref() } else { None };
        let ul = match rev {
            Some(r) if r.kind == RevisionKind::Insert => Underline::Single,
            _ => st.underline,
        };
        let ul = if st.link.is_some() && ul == Underline::None && st.color == Color::BLUE { Underline::Single } else { ul };
        let strike = st.strike || st.double_strike || matches!(rev, Some(r) if r.kind == RevisionKind::Delete);
        if p.hidden || (ul == Underline::None && !strike) {
            q += 1;
            continue;
        }
        let mut r = q;
        while r < line.b && pieces[r].style == p.style && !pieces[r].hidden {
            r += 1;
        }
        // Exclude trailing spaces at line end.
        let mut e = r;
        while e > q && pieces[e - 1].kind == PieceKind::Space && e == line.b {
            e -= 1;
        }
        if e > q {
            let x0 = ox + line.xs[q - line.a];
            let x1 = ox + line.xs[e - 1 - line.a] + pieces[e - 1].width;
            let face = cx.db.face(p.face);
            let color = match rev {
                Some(rv) => revision_color(&rv.author),
                None => st.color,
            };
            let size = st.size;
            let thick = (face.underline_thickness * size).max(0.5);
            if ul != Underline::None {
                let y = base_y + (face.underline_pos * size).max(1.0) - p.shift.min(0.0) * 0.0;
                let style = match ul {
                    Underline::Dotted => LineStyle::Dotted,
                    Underline::Dashed => LineStyle::Dashed,
                    Underline::Double => LineStyle::Double,
                    Underline::Wavy => LineStyle::Wavy,
                    _ => LineStyle::Solid,
                };
                let w = if ul == Underline::Thick { thick * 2.0 } else { thick };
                if ul == Underline::Words {
                    // underline word segments only
                    let mut s = q;
                    while s < e {
                        while s < e && pieces[s].kind == PieceKind::Space {
                            s += 1;
                        }
                        let mut t = s;
                        while t < e && pieces[t].kind != PieceKind::Space {
                            t += 1;
                        }
                        if t > s {
                            let a = ox + line.xs[s - line.a];
                            let b = ox + line.xs[t - 1 - line.a] + pieces[t - 1].width;
                            page.shapes.push(Shape::Line { x1: a, y1: y, x2: b, y2: y, width: w, color, style: LineStyle::Solid });
                        }
                        s = t;
                    }
                } else {
                    page.shapes.push(Shape::Line { x1: x0, y1: y, x2: x1, y2: y, width: w, color, style });
                }
            }
            if strike {
                let y = base_y - p.shift - face.strike_pos * p.size.max(size * 0.65);
                let style = if st.double_strike { LineStyle::Double } else { LineStyle::Solid };
                page.shapes.push(Shape::Line { x1: x0, y1: y, x2: x1, y2: y, width: thick, color, style });
            }
        }
        q = r;
    }
    // Change bar for tracked changes.
    if cx.opts.show_markup && pieces[line.a..line.b].iter().any(|p| pl.styles[p.style].revision.is_some()) {
        let x = (ox - 10.0).max(4.0).min(ox);
        let x = if path.cell.is_some() { x } else { x.min(cx.doc.page.margin_left - 12.0).max(4.0) };
        page.shapes.push(Shape::Line { x1: x, y1: top, x2: x, y2: top + line.height, width: 1.0, color: Color::rgb(0x60, 0x60, 0x60), style: LineStyle::Solid });
    }
    // Caret positions: map piece indices to char offsets (1 piece per char).
    let start = offset_base[line.a];
    let end_off = offset_base[line.b];
    let mut carets = Vec::with_capacity(end_off - start + 1);
    for q in line.a..line.b {
        carets.push(ox + line.xs[q - line.a]);
    }
    let after = if line.b > line.a {
        let lp = line.b - 1;
        let w = if matches!(pieces[lp].kind, PieceKind::LineBreak | PieceKind::PageBreak | PieceKind::ColumnBreak) { 0.0 } else { pieces[lp].width };
        ox + line.xs[lp - line.a] + w + if pieces[lp].kind == PieceKind::Space { line.justify } else { 0.0 }
    } else {
        ox + line.end_x
    };
    carets.push(after);
    page.lines.push(Line {
        path,
        start,
        end: end_off,
        top,
        height: line.height,
        baseline: base_y,
        left: ox + line.left,
        right: ox + line.right,
        carets,
        last: line.last,
    });
}

/// A stable colour per revision author.
pub fn revision_color(author: &str) -> Color {
    const COLORS: [Color; 6] = [
        Color::rgb(0xB7, 0x1C, 0x1C),
        Color::rgb(0x15, 0x65, 0xC0),
        Color::rgb(0x2E, 0x7D, 0x32),
        Color::rgb(0x6A, 0x1B, 0x9A),
        Color::rgb(0xE6, 0x51, 0x00),
        Color::rgb(0x00, 0x83, 0x8F),
    ];
    let h = author.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    COLORS[(h % COLORS.len() as u32) as usize]
}

fn border_line(page: &mut Page, b: &Border, x1: f32, y1: f32, x2: f32, y2: f32) {
    if !b.is_visible() {
        return;
    }
    let style = match b.style {
        BorderStyle::Dotted => LineStyle::Dotted,
        BorderStyle::Dashed => LineStyle::Dashed,
        BorderStyle::Double => LineStyle::Double,
        _ => LineStyle::Solid,
    };
    let w = if b.style == BorderStyle::Thick { b.width.max(1.5) } else { b.width };
    page.shapes.push(Shape::Line { x1, y1, x2, y2, width: w, color: b.color, style });
}

// ---------------------------------------------------------------------------
// Pagination
// ---------------------------------------------------------------------------

struct Pager<'a> {
    cx: &'a Ctx<'a>,
    pages: Vec<Page>,
    index: HashMap<ParaPath, Vec<(usize, usize)>>,
    col: u32,
    y: f32,
    body_top: f32,
    body_bottom: f32,
    /// Footnotes placed on the current page (ids) and their total height.
    page_notes: Vec<u32>,
    notes_height: f32,
    counter: ListCounter,
    headings: Vec<Heading>,
    prev_para: Option<(String, f32, bool, Borders)>,
    body_lines: usize,
    /// Footnote content laid out: id → (height, ParaLines list)
    note_cache: HashMap<u32, f32>,
}

impl<'a> Pager<'a> {
    fn page_setup(&self) -> &PageSetup {
        &self.cx.doc.page
    }

    fn col_x(&self) -> f32 {
        let ps = self.page_setup();
        ps.margin_left + self.col as f32 * (ps.column_width() + ps.column_gap)
    }

    fn col_width(&self) -> f32 {
        self.page_setup().column_width()
    }

    fn new_page(&mut self) {
        let ps = self.page_setup().clone();
        let index = self.pages.len();
        self.pages.push(Page {
            index,
            number: ps.page_number_start + index as u32,
            width: ps.width,
            height: ps.height,
            shapes: Vec::new(),
            lines: Vec::new(),
            body_top: self.body_top,
            body_bottom: self.body_bottom,
            comment_marks: Vec::new(),
        });
        self.col = 0;
        self.y = self.body_top;
        self.page_notes.clear();
        self.notes_height = 0.0;
        self.prev_para = None;
    }

    /// Move to the next column or page.
    fn next_frame(&mut self) {
        if self.col + 1 < self.page_setup().columns.max(1) {
            self.col += 1;
            self.y = self.body_top;
        } else {
            self.new_page();
        }
    }

    fn bottom(&self) -> f32 {
        self.body_bottom - self.notes_height
    }

    fn at_top(&self) -> bool {
        (self.y - self.body_top).abs() < 0.01
    }

    fn page(&mut self) -> &mut Page {
        self.pages.last_mut().unwrap()
    }

    fn fc(&self) -> FieldCtx {
        FieldCtx { page_number: self.pages.last().map(|p| p.number).unwrap_or(1), page_count: self.cx.opts.page_count_hint }
    }

    fn note_height(&mut self, id: u32) -> f32 {
        if let Some(h) = self.note_cache.get(&id) {
            return *h;
        }
        let cx = self.cx;
        let w = self.col_width();
        let mut h = 0.0;
        if let Some(n) = cx.doc.footnotes.iter().find(|n| n.id == id) {
            let mut counter = ListCounter::default();
            for p in &n.paragraphs {
                let pl = layout_paragraph(cx, p, w, FieldCtx::default(), &mut counter);
                h += pl.rp.space_before + pl.lines.iter().map(|l| l.height).sum::<f32>() + pl.rp.space_after;
            }
        }
        self.note_cache.insert(id, h);
        h
    }

    /// Lay out a body paragraph.
    fn place_paragraph(&mut self, para: &Paragraph, path: ParaPath, next: Option<&Block>) {
        let cx = self.cx;
        let width = self.col_width();
        let fc = self.fc();
        let pl = layout_paragraph(cx, para, width, fc, &mut self.counter);
        let rp = pl.rp.clone();
        let offset_base = offset_table(para);
        // Spacing with contextual suppression.
        let mut before = rp.space_before;
        if let Some((ps, pa, pctx, _)) = &self.prev_para {
            if rp.contextual_spacing && *ps == rp.style || *pctx && *ps == rp.style {
                before = 0.0;
                if *pctx && *ps == rp.style {
                    // undo previous paragraph's space after
                    self.y -= pa;
                }
            }
        }
        if rp.page_break_before && !(self.at_top() && self.col == 0) {
            self.new_page();
        }
        let total_h: f32 = pl.lines.iter().map(|l| l.height).sum();
        let bottom = self.bottom();
        // keep lines together
        let page_h = self.body_bottom - self.body_top;
        if !self.at_top() && rp.keep_lines && self.y + before + total_h > bottom && total_h <= page_h {
            self.next_frame();
        }
        // keep with next: heading + first lines of next paragraph
        if !self.at_top() && rp.keep_next {
            let mut need = before + total_h + rp.space_after;
            if let Some(Block::Para(np)) = next {
                let mut c2 = self.counter.clone();
                let npl = layout_paragraph(cx, np, width, fc, &mut c2);
                let n2: f32 = npl.lines.iter().take(2).map(|l| l.height).sum();
                need += npl.rp.space_before + n2;
            }
            if self.y + need > self.bottom() && need <= page_h {
                self.next_frame();
            }
        }
        // widow/orphan control: avoid a single first line at page bottom
        if !self.at_top() && rp.widow_control && pl.lines.len() > 1 {
            let first2: f32 = pl.lines.iter().take(2).map(|l| l.height).sum();
            if self.y + before + pl.lines[0].height <= self.bottom() && self.y + before + first2 > self.bottom() {
                self.next_frame();
            }
        }
        if !self.at_top() {
            self.y += before;
        } else if self.pages.len() > 1 || self.col > 0 {
            self.y += before; // Word keeps space before at top of page except after explicit breaks
        } else {
            self.y += before;
        }
        let x0 = self.col_x();
        let para_top = self.y;
        let mut seg_top = self.y;
        let n = pl.lines.len();
        let mut li = 0;
        let mut first_on_page = true;
        while li < n {
            let line_h = pl.lines[li].height;
            // footnotes referenced on this line
            let refs: Vec<u32> = pl.note_refs.iter().filter(|(pi, _)| *pi >= pl.lines[li].a && *pi < pl.lines[li].b).map(|(_, id)| *id).collect();
            let mut extra = 0.0;
            for id in &refs {
                if !self.page_notes.contains(id) {
                    extra += self.note_height(*id);
                }
            }
            if !refs.is_empty() && self.page_notes.is_empty() {
                extra += 12.0; // separator
            }
            let fits = self.y + line_h + extra <= self.bottom() + 0.01;
            // widow control: don't leave the last line alone at the top of the next page
            let widow = rp.widow_control && n > 2 && li == n - 2 && !self.at_top() && fits && {
                let next_fits = self.y + line_h + pl.lines[n - 1].height <= self.bottom() + 0.01;
                !next_fits
            } && li >= 2;
            if (!fits || widow) && !(self.at_top() && first_on_page) {
                // finish paragraph decoration on this page
                self.decorate_para(&rp, x0, seg_top, self.y, li == 0, false);
                self.next_frame();
                seg_top = self.y;
                first_on_page = true;
                continue;
            }
            for id in refs {
                if !self.page_notes.contains(&id) {
                    if self.page_notes.is_empty() {
                        self.notes_height += 12.0;
                    }
                    self.notes_height += self.note_height(id);
                    self.page_notes.push(id);
                }
            }
            let top = self.y;
            let pi = self.pages.len() - 1;
            let page = self.pages.last_mut().unwrap();
            emit_line(cx, &pl, li, path, x0, top, page, li == 0, &offset_base);
            let line_idx = page.lines.len() - 1;
            self.index.entry(path).or_default().push((pi, line_idx));
            if path.story == Story::Body {
                self.body_lines += 1;
            }
            self.y += line_h;
            first_on_page = false;
            // explicit breaks
            match &pl.lines[li].brk {
                Some(PieceKind::PageBreak) => {
                    self.decorate_para(&rp, x0, seg_top, self.y, li == 0, false);
                    self.new_page();
                    seg_top = self.y;
                    first_on_page = true;
                }
                Some(PieceKind::ColumnBreak) => {
                    self.decorate_para(&rp, x0, seg_top, self.y, li == 0, false);
                    self.next_frame();
                    seg_top = self.y;
                    first_on_page = true;
                }
                _ => {}
            }
            li += 1;
        }
        let _ = para_top;
        let joined = self.prev_para.as_ref().map(|p| p.3 == rp.borders && !rp.borders.is_empty()).unwrap_or(false);
        self.decorate_para(&rp, x0, seg_top, self.y, !joined, true);
        if let Some(level) = rp.outline_level {
            if path.cell.is_none() && path.story == Story::Body && level < 9 {
                let text = para.text().trim().to_string();
                if !text.is_empty() {
                    self.headings.push(Heading { path, level, text, page: self.pages.len() - 1 });
                }
            }
        }
        self.y += rp.space_after;
        self.prev_para = Some((rp.style.clone(), rp.space_after, rp.contextual_spacing, rp.borders.clone()));
    }

    /// Shading and borders behind a paragraph segment [top, bottom).
    fn decorate_para(&mut self, rp: &ResolvedPara, x0: f32, top: f32, bottom: f32, draw_top: bool, draw_bottom: bool) {
        if bottom <= top {
            return;
        }
        let width = self.col_width();
        let l = x0 + rp.indent_left.min(rp.indent_left + rp.indent_first);
        let r = x0 + width - rp.indent_right;
        let b = &rp.borders;
        let pad_l = b.left.map(|x| x.space + x.width).unwrap_or(0.0);
        let pad_r = b.right.map(|x| x.space + x.width).unwrap_or(0.0);
        let pad_t = b.top.map(|x| x.space).unwrap_or(0.0);
        let pad_b = b.bottom.map(|x| x.space).unwrap_or(0.0);
        let page = self.page();
        if let Some(c) = rp.shading {
            // Insert behind text: shading goes first among this paragraph's shapes.
            page.shapes.insert(0, Shape::Rect { x: l - pad_l, y: top - if draw_top { pad_t } else { 0.0 }, w: r - l + pad_l + pad_r, h: bottom - top + pad_b * draw_bottom as i32 as f32 + if draw_top { pad_t } else { 0.0 }, color: c });
        }
        if let Some(t) = &b.top {
            if draw_top {
                border_line(page, t, l - pad_l, top - pad_t, r + pad_r, top - pad_t);
            } else if let Some(bt) = &b.between {
                border_line(page, bt, l - pad_l, top - pad_t, r + pad_r, top - pad_t);
            }
        }
        if let (Some(bb), true) = (&b.bottom, draw_bottom) {
            border_line(page, bb, l - pad_l, bottom + pad_b, r + pad_r, bottom + pad_b);
        }
        if let Some(lb) = &b.left {
            border_line(page, lb, l - pad_l, top - pad_t, l - pad_l, bottom + pad_b);
        }
        if let Some(rb) = &b.right {
            border_line(page, rb, r + pad_r, top - pad_t, r + pad_r, bottom + pad_b);
        }
    }

    fn place_table(&mut self, t: &Table, block: usize, story: Story) {
        let cx = self.cx;
        if t.rows.is_empty() {
            return;
        }
        let avail = self.col_width();
        let ncols = t.columns.len().max(t.rows.iter().map(|r| r.cells.iter().map(|c| c.span.max(1) as usize).sum::<usize>()).max().unwrap_or(1));
        let mut cols: Vec<f32> = (0..ncols).map(|i| t.columns.get(i).copied().unwrap_or(72.0).max(4.0)).collect();
        let total: f32 = cols.iter().sum();
        let max_w = avail - t.indent.max(0.0) + 8.0;
        if total > max_w {
            let s = max_w / total;
            for c in &mut cols {
                *c *= s;
            }
        }
        let total: f32 = cols.iter().sum();
        let mut tx = self.col_x() + t.indent;
        match t.align {
            Align::Center => tx = self.col_x() + (avail - total) / 2.0,
            Align::Right => tx = self.col_x() + avail - total,
            _ => {}
        }
        let col_x: Vec<f32> = std::iter::once(0.0).chain(cols.iter().scan(0.0, |a, w| {
            *a += w;
            Some(*a)
        })).collect();
        let margin = t.cell_margin;
        let pad_v = 0.0;
        // Lay out every cell.
        let fc = self.fc();
        let mut rows: Vec<Vec<Option<TableCellLayout>>> = Vec::new();
        let mut row_h: Vec<f32> = Vec::new();
        for r in &t.rows {
            let mut v = Vec::new();
            let mut gc = 0usize;
            let mut h = r.height.unwrap_or(0.0);
            for c in &r.cells {
                let span = c.span.max(1) as usize;
                let x = col_x[gc.min(ncols)];
                let w = col_x[(gc + span).min(ncols)] - x;
                gc += span;
                if c.vmerge == VMerge::Continue {
                    v.push(None);
                    continue;
                }
                let inner = (w - 2.0 * margin).max(6.0);
                let mut paras = Vec::new();
                let mut ch = 0.0;
                let np = c.paragraphs.len();
                for (pi, p) in c.paragraphs.iter().enumerate() {
                    let pl = layout_paragraph(cx, p, inner, fc, &mut self.counter);
                    let sb = if pi == 0 { 0.0 } else { pl.rp.space_before };
                    let sa = if pi + 1 == np { 0.0 } else { pl.rp.space_after };
                    ch += sb + pl.lines.iter().map(|l| l.height).sum::<f32>() + sa;
                    let ob = offset_table(p);
                    paras.push((pl, sb, sa, ob));
                }
                let height = ch + 2.0 * pad_v + 2.0;
                if c.vmerge == VMerge::None {
                    h = h.max(height);
                }
                let _ = x;
                v.push(Some(TableCellLayout { paras, height }));
            }
            rows.push(v);
            row_h.push(h.max(6.0));
        }
        // Vertically merged cells: ensure group height fits content.
        for (ri, r) in t.rows.iter().enumerate() {
            for (ci, c) in r.cells.iter().enumerate() {
                if c.vmerge == VMerge::Restart {
                    let mut end = ri + 1;
                    while end < t.rows.len() && t.rows[end].cells.get(ci).map(|x| x.vmerge == VMerge::Continue).unwrap_or(false) {
                        end += 1;
                    }
                    let need = rows[ri][ci].as_ref().map(|x| x.height).unwrap_or(0.0);
                    let have: f32 = row_h[ri..end].iter().sum();
                    if need > have {
                        row_h[end - 1] += need - have;
                    }
                }
            }
        }
        let header_rows: Vec<usize> = (0..t.rows.len()).take_while(|&i| t.rows[i].header).collect();
        let mut ri = 0;
        let mut first_row_on_page = true;
        while ri < t.rows.len() {
            let h = row_h[ri];
            if self.y + h > self.bottom() + 0.01 && !self.at_top() {
                self.next_frame();
                first_row_on_page = true;
                // repeat header rows
                if !header_rows.is_empty() && ri > header_rows.len() - 1 {
                    for &hr in &header_rows {
                        self.draw_row(t, hr, &rows[hr], row_h[hr], tx, total, &col_x, margin, block, story, true, true, &row_h);
                        self.y += row_h[hr];
                    }
                    first_row_on_page = false;
                }
            }
            let top_edge = ri == 0 || first_row_on_page;
            self.draw_row(t, ri, &rows[ri], h, tx, total, &col_x, margin, block, story, top_edge, false, &row_h);
            self.y += h;
            first_row_on_page = false;
            ri += 1;
        }
        self.prev_para = None;
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_row<C: CellLike>(&mut self, t: &Table, ri: usize, cells: &[Option<C>], h: f32, tx: f32, total: f32, col_x: &[f32], margin: f32, block: usize, story: Story, top_edge: bool, repeated: bool, row_h: &[f32]) {
        let cx = self.cx;
        let top = self.y;
        let pi = self.pages.len() - 1;
        let row = &t.rows[ri];
        let ncols = col_x.len() - 1;
        let mut gc = 0usize;
        for (ci, cell) in row.cells.iter().enumerate() {
            let span = cell.span.max(1) as usize;
            let x = tx + col_x[gc.min(ncols)];
            let w = col_x[(gc + span).min(ncols)] - col_x[gc.min(ncols)];
            gc += span;
            // merged group height
            let mut ch = h;
            if cell.vmerge == VMerge::Restart {
                let mut end = ri + 1;
                while end < t.rows.len() && t.rows[end].cells.get(ci).map(|x| x.vmerge == VMerge::Continue).unwrap_or(false) {
                    end += 1;
                }
                ch = row_h[ri..end].iter().sum();
            }
            let shade = cell.shading.or_else(|| {
                if ri == 0 && t.header_shading.is_some() {
                    t.header_shading
                } else if ri > 0 && ri % 2 == 1 && t.band_shading.is_some() {
                    t.band_shading
                } else {
                    None
                }
            });
            let page = self.pages.last_mut().unwrap();
            if cell.vmerge != VMerge::Continue {
                if let Some(c) = shade {
                    page.shapes.push(Shape::Rect { x, y: top, w, h: ch, color: c });
                }
            }
            // Borders.
            let b = cell.borders.as_ref();
            let tb = &t.borders;
            let pick = |cellb: Option<Option<Border>>, outer: Option<Border>, inner: Option<Border>, is_outer: bool| -> Option<Border> {
                match cellb {
                    Some(Some(x)) => Some(x),
                    _ => if is_outer { outer } else { inner },
                }
            };
            let last_col = gc >= ncols;
            let first_col = gc - span == 0;
            let is_last_row = ri + 1 == t.rows.len() || (cell.vmerge == VMerge::Restart && {
                let mut end = ri + 1;
                while end < t.rows.len() && t.rows[end].cells.get(ci).map(|x| x.vmerge == VMerge::Continue).unwrap_or(false) {
                    end += 1;
                }
                end == t.rows.len()
            });
            if cell.vmerge != VMerge::Continue {
                if let Some(bd) = pick(b.map(|b| b.top), tb.top, tb.between, ri == 0 || top_edge) {
                    border_line(page, &bd, x, top, x + w, top);
                }
            }
            let bottom_y = top + if cell.vmerge == VMerge::Restart { ch } else { h };
            if cell.vmerge != VMerge::Restart || true {
                if cell.vmerge == VMerge::None || cell.vmerge == VMerge::Restart || ri + 1 == t.rows.len() || t.rows[ri + 1].cells.get(ci).map(|c| c.vmerge != VMerge::Continue).unwrap_or(true) {
                    if let Some(bd) = pick(b.map(|b| b.bottom), tb.bottom, tb.between, is_last_row) {
                        if cell.vmerge != VMerge::Restart || is_last_row || true {
                            border_line(page, &bd, x, bottom_y.min(top + ch), x + w, bottom_y.min(top + ch));
                        }
                    }
                }
            }
            if let Some(bd) = pick(b.map(|b| b.left), tb.left, tb.inside_v, first_col) {
                border_line(page, &bd, x, top, x, top + h);
            }
            if let Some(bd) = pick(b.map(|b| b.right), tb.right, tb.inside_v, last_col) {
                border_line(page, &bd, x + w, top, x + w, top + h);
            }
            let _ = total;
            // Content.
            if let Some(Some(cl)) = cells.get(ci) {
                if cell.vmerge == VMerge::Continue {
                    continue;
                }
                let content_h = cl.height();
                let mut y = top + 1.0 + match cell.valign {
                    VAlign::Top => 0.0,
                    VAlign::Center => ((ch - content_h) / 2.0).max(0.0),
                    VAlign::Bottom => (ch - content_h).max(0.0),
                };
                for (pidx, (pl, sb, sa, ob)) in cl.paras().iter().enumerate() {
                    y += sb;
                    let path = ParaPath { story, block, cell: Some(CellRef { row: ri, col: ci, para: pidx }) };
                    let ptop = y;
                    for li in 0..pl.lines.len() {
                        let page = self.pages.last_mut().unwrap();
                        emit_line(cx, pl, li, path, x + margin, y, page, li == 0, ob);
                        if !repeated {
                            let idx = page.lines.len() - 1;
                            self.index.entry(path).or_default().push((pi, idx));
                        } else {
                            page.lines.pop();
                        }
                        y += pl.lines[li].height;
                    }
                    if pl.rp.shading.is_some() || !pl.rp.borders.is_empty() {
                        let save = (self.col,);
                        let _ = save;
                        let rp = pl.rp.clone();
                        let page = self.pages.last_mut().unwrap();
                        if let Some(c) = rp.shading {
                            page.shapes.insert(0, Shape::Rect { x: x + margin, y: ptop, w: w - 2.0 * margin, h: y - ptop, color: c });
                        }
                    }
                    y += sa;
                }
            }
        }
    }
}

trait CellLike {
    fn height(&self) -> f32;
    fn paras(&self) -> &[(ParaLines, f32, f32, Vec<usize>)];
}

/// Map piece index → char offset (pieces are 1:1 with chars, so this is the identity plus the end).
fn offset_table(p: &Paragraph) -> Vec<usize> {
    (0..=p.len()).collect()
}

/// Lay out a whole document.
pub fn layout(doc: &Document, db: &FontDb, opts: &LayoutOptions) -> DocLayout {
    // Footnote numbering in reference order.
    let mut note_numbers = HashMap::new();
    let mut endnote_numbers = HashMap::new();
    for path in doc.para_paths(Story::Body) {
        if let Some(p) = doc.para(&path) {
            for r in &p.runs {
                match r.content {
                    Inline::FootnoteRef(id) => {
                        let n = note_numbers.len() as u32 + 1;
                        note_numbers.entry(id).or_insert(n);
                    }
                    Inline::EndnoteRef(id) => {
                        let n = endnote_numbers.len() as u32 + 1;
                        endnote_numbers.entry(id).or_insert(n);
                    }
                    _ => {}
                }
            }
        }
    }
    let cx = Ctx { doc, db, opts, note_numbers: &note_numbers, endnote_numbers: &endnote_numbers };
    let ps = &doc.page;
    // Header/footer extents (measured on page 1) may push the body area.
    let hf_height = |blocks: &Vec<Block>| -> f32 {
        let mut c = ListCounter::default();
        let mut h = 0.0;
        for b in blocks {
            if let Block::Para(p) = b {
                let pl = layout_paragraph(&cx, p, ps.text_width(), FieldCtx { page_number: 1, page_count: 1 }, &mut c);
                h += pl.rp.space_before + pl.lines.iter().map(|l| l.height).sum::<f32>() + pl.rp.space_after;
            }
        }
        h
    };
    let header_h = hf_height(&doc.headers.header).max(hf_height(&doc.headers.first_header));
    let footer_h = hf_height(&doc.headers.footer).max(hf_height(&doc.headers.first_footer));
    let body_top = ps.margin_top.max(ps.header_distance + header_h + 4.0 * (header_h > 0.0) as i32 as f32);
    let body_bottom = (ps.height - ps.margin_bottom).min(ps.height - ps.footer_distance - footer_h - 4.0 * (footer_h > 0.0) as i32 as f32);
    let body_bottom = body_bottom.max(body_top + 72.0);

    let mut pager = Pager {
        cx: &cx,
        pages: Vec::new(),
        index: HashMap::new(),
        col: 0,
        y: body_top,
        body_top,
        body_bottom,
        page_notes: Vec::new(),
        notes_height: 0.0,
        counter: ListCounter::default(),
        headings: Vec::new(),
        prev_para: None,
        body_lines: 0,
        note_cache: HashMap::new(),
    };
    pager.new_page();
    // Track footnotes per page to draw at page end.
    let mut notes_by_page: Vec<Vec<u32>> = vec![Vec::new()];
    let blocks = &doc.body;
    for (bi, b) in blocks.iter().enumerate() {
        let before_pages = pager.pages.len();
        let notes_before = pager.page_notes.clone();
        match b {
            Block::Para(p) => pager.place_paragraph(p, ParaPath::body(bi), blocks.get(bi + 1)),
            Block::Table(t) => pager.place_table(t, bi, Story::Body),
        }
        // record notes for pages
        while notes_by_page.len() < pager.pages.len() {
            notes_by_page.push(Vec::new());
        }
        if pager.pages.len() == before_pages {
            for id in &pager.page_notes {
                if !notes_before.contains(id) && !notes_by_page[before_pages - 1].contains(id) {
                    notes_by_page[before_pages - 1].push(*id);
                }
            }
        } else {
            // The paragraph spanned pages: attribute notes to the last page, and earlier ones are gone
            // from page_notes (reset), so collect what remains for the current page.
            let last = pager.pages.len() - 1;
            for id in &pager.page_notes {
                if !notes_by_page[last].contains(id) {
                    notes_by_page[last].push(*id);
                }
            }
        }
    }
    // Endnotes at the end of the document.
    if !doc.endnotes.is_empty() && !endnote_numbers.is_empty() {
        let mut ends: Vec<(&u32, &u32)> = endnote_numbers.iter().collect();
        ends.sort_by_key(|(_, n)| **n);
        let x0 = pager.col_x();
        let w = pager.col_width().min(144.0);
        pager.y += 12.0;
        if pager.y > pager.bottom() {
            pager.next_frame();
        }
        let y = pager.y;
        pager.page().shapes.push(Shape::Line { x1: x0, y1: y, x2: x0 + w, y2: y, width: 0.5, color: Color::BLACK, style: LineStyle::Solid });
        pager.y += 6.0;
        for (id, n) in ends {
            if let Some(note) = doc.endnotes.iter().find(|x| x.id == *id) {
                for (k, p) in note.paragraphs.iter().enumerate() {
                    let mut p = p.clone();
                    if k == 0 && !p.runs.iter().any(|r| matches!(r.content, Inline::EndnoteRef(_))) {
                        p.runs.insert(0, Run { content: Inline::Text(format!("{} ", crate::styles::roman(*n).to_lowercase())), props: CharProps { vert_align: Some(VertAlign::Superscript), ..Default::default() } });
                    }
                    if p.props.style.is_none() {
                        p.props.style = Some("EndnoteText".into());
                    }
                    pager.place_paragraph(&p, ParaPath { story: Story::Body, block: usize::MAX - 1, cell: None }, None);
                }
            }
        }
    }
    let total_pages = pager.pages.len();
    let body_lines = pager.body_lines;
    let mut pages = std::mem::take(&mut pager.pages);
    let index = std::mem::take(&mut pager.index);
    let headings = std::mem::take(&mut pager.headings);
    drop(pager);
    while notes_by_page.len() < pages.len() {
        notes_by_page.push(Vec::new());
    }

    // Headers, footers, footnotes, page decorations.
    for (pi, page) in pages.iter_mut().enumerate() {
        let fc = FieldCtx { page_number: page.number, page_count: total_pages as u32 };
        let first = pi == 0 && ps.title_page;
        let (hdr, hstory) = if first { (&doc.headers.first_header, Story::FirstHeader) } else { (&doc.headers.header, Story::Header) };
        let (ftr, fstory) = if first { (&doc.headers.first_footer, Story::FirstFooter) } else { (&doc.headers.footer, Story::Footer) };
        let mut extra = Vec::new();
        // header
        let mut y = ps.header_distance;
        let mut counter = ListCounter::default();
        let mut hf_index: Vec<(ParaPath, usize)> = Vec::new();
        for (bi, b) in hdr.iter().enumerate() {
            if let Block::Para(p) = b {
                let pl = layout_paragraph(&cx, p, ps.text_width(), fc, &mut counter);
                y += pl.rp.space_before;
                let ob = offset_table(p);
                for li in 0..pl.lines.len() {
                    emit_line(&cx, &pl, li, ParaPath { story: hstory, block: bi, cell: None }, ps.margin_left, y, page, li == 0, &ob);
                    hf_index.push((ParaPath { story: hstory, block: bi, cell: None }, page.lines.len() - 1));
                    y += pl.lines[li].height;
                }
                y += pl.rp.space_after;
            }
        }
        // footer: measure then place from the bottom
        let mut counter = ListCounter::default();
        let mut fls = Vec::new();
        let mut fh = 0.0;
        for (bi, b) in ftr.iter().enumerate() {
            if let Block::Para(p) = b {
                let pl = layout_paragraph(&cx, p, ps.text_width(), fc, &mut counter);
                fh += pl.rp.space_before + pl.lines.iter().map(|l| l.height).sum::<f32>() + pl.rp.space_after;
                fls.push((bi, p, pl));
            }
        }
        let mut y = ps.height - ps.footer_distance - fh;
        for (bi, p, pl) in &fls {
            y += pl.rp.space_before;
            let ob = offset_table(p);
            for li in 0..pl.lines.len() {
                emit_line(&cx, pl, li, ParaPath { story: fstory, block: *bi, cell: None }, ps.margin_left, y, page, li == 0, &ob);
                hf_index.push((ParaPath { story: fstory, block: *bi, cell: None }, page.lines.len() - 1));
                y += pl.lines[li].height;
            }
            y += pl.rp.space_after;
        }
        for (path, li) in hf_index {
            extra.push((path, li));
        }
        // footnotes
        let notes = &notes_by_page[pi];
        if !notes.is_empty() {
            let mut counter = ListCounter::default();
            let mut lays = Vec::new();
            let mut total = 0.0;
            for id in notes {
                if let Some(n) = doc.footnotes.iter().find(|n| n.id == *id) {
                    for (k, p) in n.paragraphs.iter().enumerate() {
                        let mut p = p.clone();
                        if k == 0 && !p.runs.iter().any(|r| matches!(r.content, Inline::FootnoteRef(_))) {
                            p.runs.insert(0, Run { content: Inline::FootnoteRef(*id), props: CharProps::default() });
                        }
                        if p.props.style.is_none() {
                            p.props.style = Some("FootnoteText".into());
                        }
                        let pl = layout_paragraph(&cx, &p, ps.column_width(), fc, &mut counter);
                        total += pl.rp.space_before + pl.lines.iter().map(|l| l.height).sum::<f32>() + pl.rp.space_after;
                        lays.push((p, pl));
                    }
                }
            }
            let mut y = page.body_bottom - total;
            page.shapes.push(Shape::Line { x1: ps.margin_left, y1: y - 6.0, x2: ps.margin_left + 144.0, y2: y - 6.0, width: 0.5, color: Color::BLACK, style: LineStyle::Solid });
            for (p, pl) in &lays {
                y += pl.rp.space_before;
                let ob = offset_table(p);
                for li in 0..pl.lines.len() {
                    emit_line(&cx, pl, li, ParaPath { story: Story::Body, block: usize::MAX, cell: None }, ps.margin_left, y, page, li == 0, &ob);
                    page.lines.pop(); // not editable in place
                    y += pl.lines[li].height;
                }
                y += pl.rp.space_after;
            }
        }
        // page colour, borders and watermark go behind everything
        let mut back = Vec::new();
        if let Some(c) = ps.page_color {
            back.push(Shape::Rect { x: 0.0, y: 0.0, w: ps.width, h: ps.height, color: c });
        }
        if let Some(wm) = &ps.watermark {
            if !wm.is_empty() {
                let face = db.face_for("Calibri", false, false);
                let f = db.face(face);
                let diag = (ps.width.powi(2) + ps.height.powi(2)).sqrt();
                let tw: f32 = wm.chars().map(|c| f.advance(c)).sum::<f32>().max(1.0);
                let size = (diag * 0.6 / tw).min(160.0);
                let angle = (ps.height / ps.width).atan().to_degrees();
                back.push(Shape::Watermark { cx: ps.width / 2.0, cy: ps.height / 2.0, text: wm.clone(), face, size, color: Color::rgb(0xD9, 0xD9, 0xD9), angle });
            }
        }
        if let Some(b) = &ps.page_borders {
            let (l, t, r, bt) = (ps.margin_left - 24.0, ps.margin_top - 24.0, ps.width - ps.margin_right + 24.0, ps.height - ps.margin_bottom + 24.0);
            if let Some(x) = &b.top {
                border_line_vec(&mut back, x, l, t, r, t);
            }
            if let Some(x) = &b.bottom {
                border_line_vec(&mut back, x, l, bt, r, bt);
            }
            if let Some(x) = &b.left {
                border_line_vec(&mut back, x, l, t, l, bt);
            }
            if let Some(x) = &b.right {
                border_line_vec(&mut back, x, r, t, r, bt);
            }
        }
        if !back.is_empty() {
            back.append(&mut page.shapes);
            page.shapes = back;
        }
        let _ = extra;
    }
    // Index header/footer lines too (only the ones on each page, keyed per path: first occurrence wins).
    let mut index = index;
    for (pi, page) in pages.iter().enumerate() {
        for (li, l) in page.lines.iter().enumerate() {
            if l.path.story != Story::Body {
                index.entry(l.path).or_default().push((pi, li));
            }
        }
    }
    DocLayout { pages, index, headings, body_lines }
}

fn border_line_vec(v: &mut Vec<Shape>, b: &Border, x1: f32, y1: f32, x2: f32, y2: f32) {
    let mut p = Page { index: 0, number: 0, width: 0.0, height: 0.0, shapes: Vec::new(), lines: Vec::new(), body_top: 0.0, body_bottom: 0.0, comment_marks: Vec::new() };
    border_line(&mut p, b, x1, y1, x2, y2);
    v.append(&mut p.shapes);
}

// ---------------------------------------------------------------------------
// Queries on a layout
// ---------------------------------------------------------------------------

/// Where to draw the caret.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretRect {
    pub page: usize,
    pub x: f32,
    pub top: f32,
    pub height: f32,
}

impl DocLayout {
    /// Line containing `pos` (page, line index).
    pub fn line_of(&self, pos: Pos) -> Option<(usize, usize)> {
        let lines = self.index.get(&pos.path)?;
        // For header/footer stories, prefer the first page occurrence.
        let mut best = None;
        for &(pi, li) in lines {
            let l = &self.pages[pi].lines[li];
            if pos.offset >= l.start && (pos.offset < l.end || (pos.offset == l.end && (l.last || l.end == l.start))) {
                return Some((pi, li));
            }
            if pos.offset >= l.start {
                best = Some((pi, li));
            }
        }
        best.or_else(|| lines.first().copied())
    }

    /// Line containing `pos`, preferring `page` for repeated stories (headers/footers).
    pub fn line_of_on_page(&self, pos: Pos, page: usize) -> Option<(usize, usize)> {
        if pos.path.story == Story::Body {
            return self.line_of(pos);
        }
        let lines = self.index.get(&pos.path)?;
        let on: Vec<_> = lines.iter().filter(|(p, _)| *p == page).copied().collect();
        for &(pi, li) in &on {
            let l = &self.pages[pi].lines[li];
            if pos.offset >= l.start && pos.offset <= l.end {
                return Some((pi, li));
            }
        }
        on.first().copied().or_else(|| self.line_of(pos))
    }

    pub fn caret(&self, pos: Pos) -> Option<CaretRect> {
        let (pi, li) = self.line_of(pos)?;
        let l = &self.pages[pi].lines[li];
        Some(CaretRect { page: pi, x: l.caret_x(pos.offset), top: l.top, height: l.height })
    }

    pub fn caret_on_page(&self, pos: Pos, page: usize) -> Option<CaretRect> {
        let (pi, li) = self.line_of_on_page(pos, page)?;
        let l = &self.pages[pi].lines[li];
        Some(CaretRect { page: pi, x: l.caret_x(pos.offset), top: l.top, height: l.height })
    }

    /// Hit-test a point on a page. `stories` filters which stories may be hit.
    pub fn hit(&self, page: usize, x: f32, y: f32, body: bool) -> Option<Pos> {
        let p = self.pages.get(page)?;
        let mut best: Option<(f32, &Line)> = None;
        for l in &p.lines {
            let is_body = l.path.story == Story::Body;
            if is_body != body {
                continue;
            }
            // vertical distance, then horizontal distance (for multi-column / tables)
            let dy = if y < l.top { l.top - y } else if y > l.top + l.height { y - l.top - l.height } else { 0.0 };
            let dx = if x < l.left - 20.0 { l.left - 20.0 - x } else if x > l.right + 20.0 { x - l.right - 20.0 } else { 0.0 };
            let d = dy * 4.0 + dx;
            if best.map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, l));
            }
        }
        best.map(|(_, l)| l.path.at(l.offset_at(x)))
    }

    /// Position on the line above/below `pos`, keeping horizontal position `x`.
    pub fn vertical(&self, pos: Pos, x: f32, down: bool) -> Option<Pos> {
        let (pi, li) = self.line_of(pos)?;
        let cur = &self.pages[pi].lines[li];
        // Search lines in reading order: same page lines sorted by top, then adjacent pages.
        let story_filter = |l: &Line| (l.path.story == Story::Body) == (cur.path.story == Story::Body);
        let mut cands: Vec<(usize, &Line)> = Vec::new();
        let range: Vec<usize> = if down { (pi..self.pages.len().min(pi + 3)).collect() } else { (pi.saturating_sub(2)..=pi).rev().collect() };
        for p in range {
            for l in &self.pages[p].lines {
                if !story_filter(l) {
                    continue;
                }
                let after = if p != pi {
                    (p > pi) == down
                } else if down {
                    l.top > cur.top + 0.5
                } else {
                    l.top < cur.top - 0.5
                };
                if after {
                    cands.push((p, l));
                }
            }
        }
        // nearest vertically, then horizontally overlapping x
        let key = |(p, l): &(usize, &Line)| {
            let pd = if *p == pi { 0.0 } else { 100_000.0 * (*p as f32 - pi as f32).abs() };
            let vy = if down { l.top - cur.top } else { cur.top - l.top };
            let off = if *p == pi { vy } else if down { l.top } else { 10_000.0 - l.top };
            let hx = if x < l.left { l.left - x } else if x > l.right { x - l.right } else { 0.0 };
            pd + off.abs() * 1.0 + hx * 0.5
        };
        let best = cands.into_iter().min_by(|a, b| key(a).total_cmp(&key(b)))?;
        Some(best.1.path.at(best.1.offset_at(x)))
    }

    /// Start and end positions of the line containing `pos` (Home/End).
    pub fn line_bounds(&self, pos: Pos) -> Option<(Pos, Pos)> {
        let (pi, li) = self.line_of(pos)?;
        let l = &self.pages[pi].lines[li];
        let end = if l.last { l.end } else { l.end.saturating_sub(1).max(l.start) };
        // Keep trailing space before a soft wrap inside the line: End goes after it like Word.
        let end = if !l.last { l.end } else { end };
        Some((l.path.at(l.start), l.path.at(end)))
    }

    /// Rectangles covering the selection [a, b) on a page: (x, y, w, h).
    pub fn selection_rects(&self, doc: &Document, a: Pos, b: Pos, page: usize) -> Vec<(f32, f32, f32, f32)> {
        let (a, b) = crate::edit::ordered(a, b);
        let mut out = Vec::new();
        let Some(p) = self.pages.get(page) else { return out };
        for l in &p.lines {
            if l.path < a.path || l.path > b.path {
                continue;
            }
            if l.path.story != a.path.story {
                continue;
            }
            let s = if l.path == a.path { a.offset.max(l.start) } else { l.start };
            let e = if l.path == b.path { b.offset.min(l.end) } else { l.end };
            if l.path == a.path && a.offset > l.end {
                continue;
            }
            if l.path == b.path && b.offset < l.start {
                continue;
            }
            if s > e {
                continue;
            }
            let x0 = l.caret_x(s);
            let mut x1 = l.caret_x(e);
            // Selection continues past the paragraph mark when it extends further.
            if l.last && (l.path != b.path) {
                x1 += 5.0;
            }
            if x1 > x0 || (l.last && l.path != b.path) {
                out.push((x0, l.top, (x1 - x0).max(4.0), l.height));
            }
        }
        let _ = doc;
        out
    }

    /// Total number of pages.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Page number (0-based page index) of a position.
    pub fn page_of(&self, pos: Pos) -> Option<usize> {
        self.line_of(pos).map(|(p, _)| p)
    }
}

// CellLike impl for the local struct in place_table is provided via a blanket newtype below.
impl CellLike for TableCellLayout {
    fn height(&self) -> f32 {
        self.height
    }
    fn paras(&self) -> &[(ParaLines, f32, f32, Vec<usize>)] {
        &self.paras
    }
}

struct TableCellLayout {
    paras: Vec<(ParaLines, f32, f32, Vec<usize>)>,
    height: f32,
}
