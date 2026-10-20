//! HTML export and import.
//!
//! Export produces a standalone HTML document (or a clipboard fragment) with all formatting resolved
//! into inline CSS, semantic tags for the common character formats, nested `<ul>`/`<ol>` lists,
//! tables with spans, `data:` URI images, links, bookmarks and foot/endnotes.
//!
//! Import is a forgiving HTML parser (implied end tags, entities, `<style>` class rules) that maps
//! paragraphs, headings, inline formatting, CSS, lists, tables, images (`data:` URIs), links, `<pre>`
//! and Word's clipboard HTML conventions (`mso-list`, `mso-tab-count`, `MsoTitle`, `_ftnN` notes,
//! `<!--StartFragment-->` markers) to the document model.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::io::util::{
    base64_decode, base64_encode, css_color, fmt_num, format_date_now, highlight_color, image_data, image_mime, natural_size, parse_length,
    HIGHLIGHTS,
};
use crate::io::xml::{Element, Node};
use crate::model::*;
use crate::styles::{resolve_chars, resolve_para, ResolvedChars, ResolvedPara};

// ===================================================================================================
// Writer
// ===================================================================================================

/// A standalone HTML document.
pub fn write(doc: &Document) -> String {
    let mut w = HtmlWriter::new(doc, false);
    let mut body = String::new();
    w.blocks(&mut body, &doc.body);
    w.notes(&mut body);
    let p = &doc.properties;
    let dc = resolve_chars(doc, "Normal", &CharProps::default());
    let pg = &doc.page;
    let mut out = String::from("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n");
    let _ = writeln!(out, "<meta name=\"generator\" content=\"RWord\">");
    let title = if p.title.is_empty() { "Document" } else { &p.title };
    let _ = writeln!(out, "<title>{}</title>", esc(title));
    for (n, v) in [("author", &p.author), ("description", &p.description), ("keywords", &p.keywords), ("subject", &p.subject)] {
        if !v.is_empty() {
            let _ = writeln!(out, "<meta name=\"{}\" content=\"{}\">", n, esc(v));
        }
    }
    out.push_str("<style>\n");
    let _ = writeln!(
        out,
        "@page {{ size: {}pt {}pt; margin: {}pt {}pt {}pt {}pt; }}",
        fmt_num(pg.width),
        fmt_num(pg.height),
        fmt_num(pg.margin_top),
        fmt_num(pg.margin_right),
        fmt_num(pg.margin_bottom),
        fmt_num(pg.margin_left)
    );
    let bg = pg.page_color.map(|c| format!(" background: #{};", c.to_hex())).unwrap_or_default();
    let _ = writeln!(
        out,
        "body {{ font-family: {}; font-size: {}pt; color: #{};{} max-width: {}pt; margin: 2em auto; padding: 0 1em; }}",
        css_font(&dc.font),
        fmt_num(dc.size),
        dc.color.to_hex(),
        bg,
        fmt_num(pg.text_width())
    );
    out.push_str("p, h1, h2, h3, h4, h5, h6, li { margin: 0; }\ntable { border-collapse: collapse; }\ntd, th { padding: 0 5.4pt; }\n");
    out.push_str("</style>\n</head>\n<body>\n");
    out.push_str(&body);
    out.push_str("</body>\n</html>\n");
    out
}

/// HTML for the clipboard: the given blocks with every style resolved into inline CSS.
pub fn write_fragment(blocks: &[Block], doc: &Document) -> String {
    let mut w = HtmlWriter::new(doc, true);
    let mut body = String::new();
    w.blocks(&mut body, blocks);
    body
}

struct HtmlWriter<'a> {
    doc: &'a Document,
    fragment: bool,
    counters: HashMap<u32, [u32; 9]>,
    /// Foot/endnote ids in order of first reference → displayed number.
    footnotes: Vec<u32>,
    endnotes: Vec<u32>,
    base: ResolvedChars,
    /// Section of the body block being written.
    cur_sec: usize,
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\u{A0}' => o.push_str("&nbsp;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => {}
            c => o.push(c),
        }
    }
    o
}

fn css_font(f: &str) -> String {
    let generic = match f.to_ascii_lowercase().as_str() {
        "courier new" | "consolas" | "courier" | "lucida console" => "monospace",
        "times new roman" | "georgia" | "cambria" | "garamond" | "book antiqua" => "serif",
        _ => "sans-serif",
    };
    if f.contains(' ') || f.contains(',') {
        format!("'{}', {}", f.replace('\'', ""), generic)
    } else {
        format!("{}, {}", f, generic)
    }
}

fn css_border(b: &Option<Border>) -> String {
    match b {
        Some(b) if b.is_visible() => {
            let style = match b.style {
                BorderStyle::Double => "double",
                BorderStyle::Dotted => "dotted",
                BorderStyle::Dashed => "dashed",
                _ => "solid",
            };
            let w = if b.style == BorderStyle::Double { b.width * 3.0 } else { b.width };
            format!("{}pt {} #{}", fmt_num(w.max(0.25)), style, b.color.to_hex())
        }
        _ => "none".into(),
    }
}

fn field_text(f: &Field) -> String {
    match f {
        Field::PageNumber => "1".into(),
        Field::PageCount => "1".into(),
        Field::Date(pic) => format_date_now(pic),
        Field::Other { result, .. } => result.clone(),
    }
}

fn field_instr(f: &Field) -> String {
    match f {
        Field::PageNumber => "PAGE".into(),
        Field::PageCount => "NUMPAGES".into(),
        Field::Date(pic) => format!("DATE \\@ \"{}\"", pic),
        Field::Other { instr, .. } => instr.clone(),
    }
}

fn lower_roman(n: usize) -> String {
    crate::styles::roman(n as u32).to_lowercase()
}

impl<'a> HtmlWriter<'a> {
    fn new(doc: &'a Document, fragment: bool) -> Self {
        let mut w = HtmlWriter {
            doc,
            fragment,
            counters: HashMap::new(),
            footnotes: Vec::new(),
            endnotes: Vec::new(),
            base: resolve_chars(doc, "Normal", &CharProps::default()),
            cur_sec: 0,
        };
        if fragment {
            // Nothing is inherited from a stylesheet: every property is written.
            w.base.font.clear();
            w.base.size = -1.0;
            w.base.color = Color::rgb(1, 2, 3);
        }
        w
    }

    fn blocks(&mut self, out: &mut String, blocks: &[Block]) {
        // Stack of open lists: (list id, ordered).
        let mut stack: Vec<(u32, bool)> = Vec::new();
        // Section breaks (body only) become page breaks before the next section's first paragraph.
        let top = std::ptr::eq(blocks.as_ptr(), self.doc.body.as_ptr());
        let mut break_next = false;
        let mut sec = 0;
        for b in blocks {
            let brk = std::mem::take(&mut break_next);
            if top {
                self.cur_sec = sec;
                if let Block::Para(p) = b {
                    if p.section.is_some() {
                        sec += 1;
                        break_next = self.doc.section_page(sec).start != SectionStart::Continuous;
                    }
                }
            }
            match b {
                Block::Para(p) => {
                    let mut rp = resolve_para(self.doc, &p.props);
                    rp.page_break_before |= brk;
                    if let Some(l) = rp.list {
                        self.list_item(out, &mut stack, p, &rp, l);
                    } else {
                        close_lists(out, &mut stack, 0);
                        self.paragraph(out, p, &rp, false);
                    }
                }
                Block::Table(t) => {
                    close_lists(out, &mut stack, 0);
                    self.table(out, t);
                }
            }
        }
        close_lists(out, &mut stack, 0);
    }

    fn level_number(&mut self, l: ListRef) -> u32 {
        let def = self.doc.list(l.list);
        let level = (l.level as usize).min(8);
        let start = def.and_then(|d| d.levels.get(level)).map(|x| x.start.max(1)).unwrap_or(1);
        let c = self.counters.entry(l.list).or_insert([0; 9]);
        for slot in c.iter_mut().skip(level + 1) {
            *slot = 0;
        }
        c[level] = if c[level] == 0 { start } else { c[level] + 1 };
        c[level]
    }

    fn list_item(&mut self, out: &mut String, stack: &mut Vec<(u32, bool)>, p: &Paragraph, rp: &ResolvedPara, l: ListRef) {
        let def = self.doc.list(l.list);
        let lvl = def.and_then(|d| d.levels.get(l.level as usize));
        let ordered = lvl.map(|x| x.format != NumFormat::Bullet).unwrap_or(false);
        let number = self.level_number(l);
        let depth = l.level as usize + 1;
        // A different list at this depth (or a different kind) closes the old one.
        if stack.len() >= depth && stack[depth - 1] != (l.list, ordered) {
            close_lists(out, stack, depth - 1);
        }
        if stack.len() > depth {
            close_lists(out, stack, depth);
        }
        if stack.len() == depth {
            out.push_str("</li>\n");
        }
        while stack.len() < depth {
            let cur_depth = stack.len();
            let this = cur_depth + 1 == depth;
            let (o, lv) = if this {
                (ordered, lvl)
            } else {
                let lv = def.and_then(|d| d.levels.get(cur_depth));
                (lv.map(|x| x.format != NumFormat::Bullet).unwrap_or(false), lv)
            };
            let tag = if o { "ol" } else { "ul" };
            let lst = match lv.map(|x| (x.format, x.text.as_str())) {
                Some((NumFormat::Decimal, _)) | Some((NumFormat::DecimalZero, _)) => "decimal",
                Some((NumFormat::LowerLetter, _)) => "lower-alpha",
                Some((NumFormat::UpperLetter, _)) => "upper-alpha",
                Some((NumFormat::LowerRoman, _)) => "lower-roman",
                Some((NumFormat::UpperRoman, _)) => "upper-roman",
                Some((NumFormat::None, _)) => "none",
                Some((NumFormat::Bullet, t)) => match t {
                    "◦" | "o" | "○" => "circle",
                    "▪" | "■" | "§" | "\u{F0A7}" => "square",
                    _ => "disc",
                },
                None => "disc",
            };
            let start = if this && o && number != 1 { format!(" start=\"{}\"", number) } else { String::new() };
            let _ = write!(out, "<{}{} style=\"list-style-type:{};margin-top:0;margin-bottom:0\">", tag, start, lst);
            if this {
                out.push('\n');
            } else {
                // An intermediate item stays open; the next level nests inside it.
                out.push_str("<li style=\"list-style-type:none\">");
            }
            stack.push((if this { l.list } else { u32::MAX }, o));
        }
        out.push_str("<li>");
        self.paragraph(out, p, rp, true);
    }

    fn para_css(&self, rp: &ResolvedPara, in_list: bool) -> String {
        let mut s = String::new();
        let align = match rp.align {
            Align::Left => None,
            Align::Center => Some("center"),
            Align::Right => Some("right"),
            Align::Justify => Some("justify"),
        };
        if let Some(a) = align {
            let _ = write!(s, "text-align:{};", a);
        }
        let (il, ifi) = if in_list { (0.0, 0.0) } else { (rp.indent_left, rp.indent_first) };
        let _ = write!(
            s,
            "margin:{}pt {}pt {}pt {}pt;",
            fmt_num(rp.space_before),
            fmt_num(rp.indent_right),
            fmt_num(rp.space_after),
            fmt_num(il)
        );
        if ifi != 0.0 {
            let _ = write!(s, "text-indent:{}pt;", fmt_num(ifi));
        }
        match rp.line_spacing {
            LineSpacing::Multiple(m) => {
                if (m - 1.0).abs() > 0.001 || self.fragment {
                    let _ = write!(s, "line-height:{}%;", fmt_num((m * 115.0).round()));
                }
            }
            LineSpacing::AtLeast(v) | LineSpacing::Exactly(v) => {
                let _ = write!(s, "line-height:{}pt;", fmt_num(v));
            }
        }
        if let Some(c) = rp.shading {
            let _ = write!(s, "background-color:#{};", c.to_hex());
        }
        let b = &rp.borders;
        for (side, bd) in [("top", &b.top), ("right", &b.right), ("bottom", &b.bottom), ("left", &b.left)] {
            if let Some(x) = bd.filter(|x| x.is_visible()) {
                let _ = write!(s, "border-{}:{};padding-{}:{}pt;", side, css_border(bd), side, fmt_num(x.space));
            }
        }
        if rp.page_break_before {
            s.push_str("page-break-before:always;");
        }
        if rp.keep_next {
            s.push_str("page-break-after:avoid;");
        }
        s
    }

    fn paragraph(&mut self, out: &mut String, p: &Paragraph, rp: &ResolvedPara, in_list: bool) {
        let style = rp.style.as_str();
        let tag = match style {
            "Heading1" | "Title" => "h1",
            "Heading2" => "h2",
            "Heading3" => "h3",
            "Heading4" => "h4",
            "Heading5" => "h5",
            "Heading6" => "h6",
            _ => "p",
        };
        let class = if style != "Normal" && !(style.starts_with("Heading") && tag != "p") { format!(" class=\"{}\"", esc(style)) } else { String::new() };
        let mut css = self.para_css(rp, in_list);
        if tag != "p" {
            // Neutralise the browser's heading look; runs carry their own formatting.
            let base = resolve_chars(self.doc, "Normal", &CharProps::default());
            let _ = write!(css, "font-weight:normal;font-size:{}pt;", fmt_num(base.size));
        }
        // Text boxes cannot live inside a <p>: they precede their anchor paragraph as floating blocks.
        for r in &p.runs {
            if let Inline::TextBox(tb) = &r.content {
                self.textbox(out, tb);
            }
        }
        let _ = write!(out, "<{}{} style=\"{}\">", tag, class, css);
        let mut inner = String::new();
        self.runs(&mut inner, p);
        if inner.is_empty() {
            inner.push_str("<br>");
        }
        out.push_str(&inner);
        let _ = writeln!(out, "</{}>", tag);
    }

    /// CSS placing a floating object (`;`-prefixed): float left/right by alignment or offset, or a
    /// centred / top-and-bottom block.
    fn float_css(&self, f: &Float, width: f32) -> String {
        let d = fmt_num(f.dist);
        let pg = self.doc.section_page(self.cur_sec);
        let text_w = (pg.width - pg.margin_left - pg.margin_right).max(72.0);
        let side = match f.h_align {
            Some(FloatAlign::Start) => Some("left"),
            Some(FloatAlign::End) => Some("right"),
            Some(FloatAlign::Center) => None,
            None => {
                let x = if f.h_rel == HRel::Page { f.x - pg.margin_left } else { f.x };
                Some(if x + width / 2.0 > text_w / 2.0 { "right" } else { "left" })
            }
        };
        match side {
            Some(_) if f.wrap == WrapMode::TopBottom => format!(";display:block;margin:{d}pt 0"),
            Some("left") => format!(";float:left;margin:0 {d}pt {d}pt 0"),
            Some(_) => format!(";float:right;margin:0 0 {d}pt {d}pt"),
            None => format!(";display:block;margin:{d}pt auto"),
        }
    }

    fn textbox(&mut self, out: &mut String, tb: &TextBox) {
        let mut css = format!("width:{}pt;padding:{}pt;box-sizing:border-box;", fmt_num(tb.width), fmt_num(tb.padding));
        let _ = write!(css, "{}:{}pt;", if tb.auto_height { "min-height" } else { "height" }, fmt_num(tb.height));
        if let Some(b) = tb.border.filter(|b| b.is_visible()) {
            let _ = write!(css, "border:{};", css_border(&Some(b)));
        }
        if let Some(c) = tb.fill {
            let _ = write!(css, "background-color:#{};", c.to_hex());
        }
        match &tb.float {
            Some(f) => css.push_str(self.float_css(f, tb.width).trim_start_matches(';')),
            None => css.push_str("display:inline-block"),
        }
        let _ = writeln!(out, "<div class=\"textbox\" style=\"{}\">", css);
        let blocks: Vec<Block> = tb.paragraphs.iter().cloned().map(Block::Para).collect();
        self.blocks(out, &blocks);
        out.push_str("</div>\n");
    }

    fn runs(&mut self, out: &mut String, p: &Paragraph) {
        let style = p.style_id().to_string();
        let mut i = 0;
        let mut at_start = true;
        while i < p.runs.len() {
            if let Some(link) = &p.runs[i].props.link {
                let mut j = i;
                while j < p.runs.len() && p.runs[j].props.link.as_ref() == Some(link) {
                    j += 1;
                }
                let _ = write!(out, "<a href=\"{}\">", esc(link));
                for k in i..j {
                    self.run(out, &style, &p.runs[k], &mut at_start);
                }
                out.push_str("</a>");
                i = j;
            } else {
                self.run(out, &style, &p.runs[i], &mut at_start);
                i += 1;
            }
        }
    }

    fn char_css(&self, rc: &ResolvedChars) -> String {
        let b = &self.base;
        let mut s = String::new();
        if rc.font != b.font {
            let _ = write!(s, "font-family:{};", css_font(&rc.font).replace('\'', "&#39;"));
        }
        if rc.size != b.size {
            let _ = write!(s, "font-size:{}pt;", fmt_num(rc.size));
        }
        if rc.color != b.color {
            let _ = write!(s, "color:#{};", rc.color.to_hex());
        }
        if let Some(h) = rc.highlight.or(rc.shading) {
            let _ = write!(s, "background-color:#{};", h.to_hex());
        }
        if rc.caps {
            s.push_str("text-transform:uppercase;");
        }
        if rc.small_caps {
            s.push_str("font-variant:small-caps;");
        }
        if rc.hidden {
            s.push_str("display:none;");
        }
        if rc.spacing != 0.0 {
            let _ = write!(s, "letter-spacing:{}pt;", fmt_num(rc.spacing));
        }
        match rc.underline {
            Underline::Double => s.push_str("text-decoration:underline double;"),
            Underline::Dotted => s.push_str("text-decoration:underline dotted;"),
            Underline::Dashed => s.push_str("text-decoration:underline dashed;"),
            Underline::Wavy => s.push_str("text-decoration:underline wavy;"),
            _ => {}
        }
        if rc.double_strike && !rc.strike {
            s.push_str("text-decoration-line:line-through;text-decoration-style:double;");
        }
        s
    }

    fn run(&mut self, out: &mut String, style: &str, r: &Run, at_start: &mut bool) {
        if let Some(rev) = &r.props.revision {
            if rev.kind == RevisionKind::Delete {
                return;
            }
        }
        let rc = resolve_chars(self.doc, style, &r.props);
        let mut open = String::new();
        let mut close: Vec<&str> = Vec::new();
        let css = self.char_css(&rc);
        let inner = match &r.content {
            Inline::Text(t) => {
                let s = text_html(t, at_start);
                if s.is_empty() {
                    return;
                }
                s
            }
            Inline::Tab => {
                *at_start = false;
                "<span style=\"white-space:pre\">\t</span>".to_string()
            }
            Inline::LineBreak | Inline::ColumnBreak => {
                *at_start = true;
                out.push_str("<br>");
                return;
            }
            Inline::PageBreak => {
                *at_start = true;
                out.push_str("<br style=\"page-break-before:always;clear:both\">");
                return;
            }
            Inline::TextBox(_) => return,
            Inline::Image(img) => {
                *at_start = false;
                let mime = image_mime(&img.data.format);
                let float = img.float.map(|f| self.float_css(&f, img.width)).unwrap_or_default();
                let _ = write!(
                    out,
                    "<img src=\"data:{};base64,{}\" width=\"{}\" height=\"{}\" style=\"width:{}pt;height:{}pt{}\" alt=\"{}\">",
                    mime,
                    base64_encode(&img.data.bytes),
                    (img.width / 0.75).round() as i64,
                    (img.height / 0.75).round() as i64,
                    fmt_num(img.width),
                    fmt_num(img.height),
                    float,
                    esc(&img.alt)
                );
                return;
            }
            Inline::Field(f) => {
                *at_start = false;
                format!("<span data-field=\"{}\">{}</span>", esc(&field_instr(f)), esc(&field_text(f)))
            }
            Inline::FootnoteRef(id) | Inline::EndnoteRef(id) => {
                *at_start = false;
                let foot = matches!(r.content, Inline::FootnoteRef(_));
                let list = if foot { &mut self.footnotes } else { &mut self.endnotes };
                let n = match list.iter().position(|x| x == id) {
                    Some(i) => i + 1,
                    None => {
                        list.push(*id);
                        list.len()
                    }
                };
                let (pre, label) = if foot { ("ftn", n.to_string()) } else { ("edn", lower_roman(n)) };
                if self.fragment {
                    format!("<sup>{}</sup>", label)
                } else {
                    format!("<sup><a href=\"#_{pre}{n}\" id=\"_{pre}ref{n}\">{label}</a></sup>")
                }
            }
            Inline::Bookmark(name) => {
                let _ = write!(out, "<a id=\"{}\"></a>", esc(name));
                return;
            }
        };
        if rc.bold {
            open.push_str("<b>");
            close.push("</b>");
        }
        if rc.italic {
            open.push_str("<i>");
            close.push("</i>");
        }
        if rc.underline != Underline::None {
            open.push_str("<u>");
            close.push("</u>");
        }
        if rc.strike {
            open.push_str("<s>");
            close.push("</s>");
        }
        match rc.vert_align {
            VertAlign::Superscript if !matches!(r.content, Inline::FootnoteRef(_) | Inline::EndnoteRef(_)) => {
                open.push_str("<sup>");
                close.push("</sup>");
            }
            VertAlign::Subscript => {
                open.push_str("<sub>");
                close.push("</sub>");
            }
            _ => {}
        }
        if !css.is_empty() {
            let _ = write!(out, "<span style=\"{}\">", css);
        }
        out.push_str(&open);
        out.push_str(&inner);
        for c in close.iter().rev() {
            out.push_str(c);
        }
        if !css.is_empty() {
            out.push_str("</span>");
        }
    }

    fn table(&mut self, out: &mut String, t: &Table) {
        let width: f32 = t.columns.iter().sum();
        let margin = match t.align {
            Align::Center => "margin-left:auto;margin-right:auto;".to_string(),
            Align::Right => "margin-left:auto;margin-right:0;".to_string(),
            _ if t.indent != 0.0 => format!("margin-left:{}pt;", fmt_num(t.indent)),
            _ => String::new(),
        };
        let _ = writeln!(out, "<table style=\"border-collapse:collapse;width:{}pt;{}\">", fmt_num(width), margin);
        out.push_str("<colgroup>");
        for c in &t.columns {
            let _ = write!(out, "<col style=\"width:{}pt\">", fmt_num(*c));
        }
        out.push_str("</colgroup>\n");
        let ncols = t.columns.len();
        let nrows = t.rows.len();
        let header_rows = t.rows.iter().take_while(|r| r.header).count();
        if header_rows > 0 {
            out.push_str("<thead>\n");
        }
        for (ri, row) in t.rows.iter().enumerate() {
            if ri == header_rows && header_rows > 0 {
                out.push_str("</thead>\n<tbody>\n");
            }
            match row.height {
                Some(h) => {
                    let _ = write!(out, "<tr style=\"height:{}pt\">", fmt_num(h));
                }
                None => out.push_str("<tr>"),
            }
            let mut gc = 0usize;
            for cell in &row.cells {
                let span = cell.span.max(1) as usize;
                if cell.vmerge == VMerge::Continue {
                    gc += span;
                    continue;
                }
                let mut rowspan = 1;
                if cell.vmerge == VMerge::Restart {
                    for r2 in t.rows.iter().skip(ri + 1) {
                        let mut g = 0usize;
                        let mut found = false;
                        for c2 in &r2.cells {
                            if g == gc {
                                found = c2.vmerge == VMerge::Continue;
                                break;
                            }
                            g += c2.span.max(1) as usize;
                        }
                        if !found {
                            break;
                        }
                        rowspan += 1;
                    }
                }
                let last_col = gc + span >= ncols;
                let last_row = ri + rowspan >= nrows;
                let cb = cell.borders.clone().unwrap_or_default();
                let top = cb.top.or(if ri == 0 { t.borders.top } else { t.borders.between });
                let bottom = cb.bottom.or(if last_row { t.borders.bottom } else { t.borders.between });
                let left = cb.left.or(if gc == 0 { t.borders.left } else { t.borders.inside_v });
                let right = cb.right.or(if last_col { t.borders.right } else { t.borders.inside_v });
                let mut css = format!(
                    "border-top:{};border-right:{};border-bottom:{};border-left:{};",
                    css_border(&top),
                    css_border(&right),
                    css_border(&bottom),
                    css_border(&left)
                );
                let w: f32 = t.columns.iter().skip(gc).take(span).sum();
                let _ = write!(css, "width:{}pt;", fmt_num(w));
                let _ = write!(css, "padding:0 {}pt;", fmt_num(t.cell_margin));
                css.push_str(match cell.valign {
                    VAlign::Top => "vertical-align:top;",
                    VAlign::Center => "vertical-align:middle;",
                    VAlign::Bottom => "vertical-align:bottom;",
                });
                let shade = cell.shading.or(if row.header {
                    t.header_shading
                } else if ri % 2 == 1 {
                    t.band_shading
                } else {
                    None
                });
                if let Some(c) = shade {
                    let _ = write!(css, "background-color:#{};", c.to_hex());
                }
                let tag = if row.header { "th" } else { "td" };
                if row.header {
                    css.push_str("font-weight:normal;text-align:left;");
                }
                out.push('<');
                out.push_str(tag);
                if span > 1 {
                    let _ = write!(out, " colspan=\"{}\"", span);
                }
                if rowspan > 1 {
                    let _ = write!(out, " rowspan=\"{}\"", rowspan);
                }
                let _ = write!(out, " style=\"{}\">", css);
                let blocks: Vec<Block> = cell.paragraphs.iter().cloned().map(Block::Para).collect();
                self.blocks(out, &blocks);
                let _ = write!(out, "</{}>", tag);
                gc += span;
            }
            out.push_str("</tr>\n");
        }
        if header_rows > 0 {
            out.push_str(if header_rows < nrows { "</tbody>\n" } else { "</thead>\n" });
        }
        out.push_str("</table>\n");
    }

    fn notes(&mut self, out: &mut String) {
        for foot in [true, false] {
            let ids = if foot { self.footnotes.clone() } else { self.endnotes.clone() };
            if ids.is_empty() {
                continue;
            }
            let _ = writeln!(out, "<div class=\"{}\">\n<hr style=\"width:33%;margin-left:0\">", if foot { "footnotes" } else { "endnotes" });
            for (i, id) in ids.iter().enumerate() {
                let n = i + 1;
                let (pre, label) = if foot { ("ftn", n.to_string()) } else { ("edn", lower_roman(n)) };
                let notes = if foot { &self.doc.footnotes } else { &self.doc.endnotes };
                let paras = notes.iter().find(|x| x.id == *id).map(|n| n.paragraphs.clone()).unwrap_or_default();
                let _ = writeln!(out, "<div id=\"_{pre}{n}\">");
                for (pi, p) in paras.iter().enumerate() {
                    let rp = resolve_para(self.doc, &p.props);
                    let css = self.para_css(&rp, false);
                    let _ = write!(out, "<p style=\"{}\">", css);
                    if pi == 0 {
                        let _ = write!(out, "<a href=\"#_{pre}ref{n}\"><sup>{label}</sup></a> ");
                    }
                    // The note's own reference mark is replaced by the back link above.
                    let mut q = p.clone();
                    q.runs.retain(|r| !matches!(r.content, Inline::FootnoteRef(_) | Inline::EndnoteRef(_)));
                    if let Some(Run { content: Inline::Text(t), .. }) = q.runs.first_mut() {
                        *t = t.trim_start().to_string();
                    }
                    let mut inner = String::new();
                    self.runs(&mut inner, &q);
                    out.push_str(&inner);
                    out.push_str("</p>\n");
                }
                out.push_str("</div>\n");
            }
            out.push_str("</div>\n");
        }
    }
}

fn close_lists(out: &mut String, stack: &mut Vec<(u32, bool)>, keep: usize) {
    while stack.len() > keep {
        let (_, ordered) = stack.pop().unwrap();
        out.push_str("</li>");
        out.push_str(if ordered { "</ol>\n" } else { "</ul>\n" });
    }
}

/// Escape run text, keeping runs of spaces visible.
fn text_html(t: &str, at_start: &mut bool) -> String {
    let mut o = String::new();
    let mut prev_space = *at_start;
    for c in t.chars() {
        match c {
            ' ' => {
                if prev_space {
                    o.push_str("&nbsp;");
                    prev_space = false;
                } else {
                    o.push(' ');
                    prev_space = true;
                }
                continue;
            }
            '\t' => o.push_str("<span style=\"white-space:pre\">\t</span>"),
            '\n' => o.push_str("<br>"),
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\u{A0}' => o.push_str("&nbsp;"),
            c if (c as u32) < 0x20 => {}
            c => o.push(c),
        }
        prev_space = false;
    }
    if !t.is_empty() {
        *at_start = prev_space;
    }
    o
}

// ===================================================================================================
// HTML parser
// ===================================================================================================

const VOID: &[&str] = &["br", "img", "hr", "meta", "link", "input", "col", "area", "base", "wbr", "source", "param", "embed", "track", "keygen"];
const RAW: &[&str] = &["script", "style", "textarea", "title", "xmp"];
const BLOCKS: &[&str] = &[
    "p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "table", "pre", "blockquote", "hr", "dl", "address", "section", "article",
    "header", "footer", "nav", "aside", "main", "figure", "figcaption", "form", "fieldset", "center", "li", "dt", "dd",
];

fn open_index(stack: &[Element], names: &[&str], stop: &[&str]) -> Option<usize> {
    for (i, e) in stack.iter().enumerate().rev() {
        if names.contains(&e.name.as_str()) {
            return Some(i);
        }
        if stop.contains(&e.name.as_str()) {
            return None;
        }
    }
    None
}

fn pop_node(stack: &mut Vec<Element>) {
    if stack.len() > 1 {
        let e = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(Node::Element(e));
    }
}

fn pop_to(stack: &mut Vec<Element>, len: usize) {
    while stack.len() > len.max(1) {
        pop_node(stack);
    }
}

fn flush_text(stack: &mut [Element], text: &mut String) {
    if !text.is_empty() {
        let t = decode_entities(text);
        stack.last_mut().unwrap().children.push(Node::Text(t));
        text.clear();
    }
}

/// Parse HTML into a tree rooted at a synthetic `#root` element. Names are lower-cased.
pub(crate) fn parse_html(src: &str) -> Element {
    let mut stack: Vec<Element> = vec![Element::new("#root")];
    let b = src.as_bytes();
    let mut i = 0;
    let mut text = String::new();
    while i < b.len() {
        if b[i] != b'<' {
            let next = src[i..].find('<').map(|x| i + x).unwrap_or(b.len());
            text.push_str(&src[i..next]);
            i = next;
            continue;
        }
        if src[i..].starts_with("<!--") {
            flush_text(&mut stack, &mut text);
            let (body, end) = match src[i + 4..].find("-->") {
                Some(e) => (&src[i + 4..i + 4 + e], i + 4 + e + 3),
                None => (&src[i + 4..], b.len()),
            };
            // Keep fragment markers as processing hints.
            let t = body.trim();
            if t.eq_ignore_ascii_case("StartFragment") || t.eq_ignore_ascii_case("EndFragment") {
                let mut e = Element::new("#marker");
                e.attrs.push(("v".into(), t.to_ascii_lowercase()));
                stack.last_mut().unwrap().children.push(Node::Element(e));
            }
            i = end;
            continue;
        }
        if src[i..].starts_with("<![CDATA[") {
            let end = src[i..].find("]]>").map(|e| i + e).unwrap_or(b.len());
            text.push_str(&src[i + 9..end].replace('&', "&amp;"));
            i = (end + 3).min(b.len());
            continue;
        }
        if src[i..].starts_with("<!") || src[i..].starts_with("<?") {
            i = src[i..].find('>').map(|e| i + e + 1).unwrap_or(b.len());
            continue;
        }
        let closing = b.get(i + 1) == Some(&b'/');
        let ns = if closing { i + 2 } else { i + 1 };
        let mut ne = ns;
        while ne < b.len() && (b[ne].is_ascii_alphanumeric() || b[ne] == b':' || b[ne] == b'-' || b[ne] == b'_') {
            ne += 1;
        }
        if ne == ns || !b[ns].is_ascii_alphabetic() {
            // A lone '<'.
            text.push_str("&lt;");
            i += 1;
            continue;
        }
        let name = src[ns..ne].to_ascii_lowercase();
        // Attributes up to '>' (respecting quotes).
        let mut j = ne;
        let mut attrs = Vec::new();
        let mut self_close = false;
        loop {
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= b.len() {
                break;
            }
            if b[j] == b'>' {
                j += 1;
                break;
            }
            if b[j] == b'/' {
                self_close = true;
                j += 1;
                continue;
            }
            let as_ = j;
            while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'=' && b[j] != b'>' && !(b[j] == b'/' && b.get(j + 1) == Some(&b'>')) {
                j += 1;
            }
            let an = src[as_..j].to_ascii_lowercase();
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            let mut val = String::new();
            if j < b.len() && b[j] == b'=' {
                j += 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
                    let q = b[j];
                    let vs = j + 1;
                    j = vs;
                    while j < b.len() && b[j] != q {
                        j += 1;
                    }
                    val = src[vs..j].to_string();
                    j = (j + 1).min(b.len());
                } else {
                    let vs = j;
                    while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                        j += 1;
                    }
                    val = src[vs..j].to_string();
                }
            }
            if an.is_empty() {
                j += 1;
            } else {
                attrs.push((an, decode_entities(&val)));
            }
        }
        i = j;
        flush_text(&mut stack, &mut text);
        if closing {
            if let Some(pos) = stack.iter().rposition(|e| e.name == name) {
                if pos > 0 {
                    pop_to(&mut stack, pos);
                }
            } else if name == "br" {
                stack.last_mut().unwrap().children.push(Node::Element(Element::new("br")));
            }
            continue;
        }
        // Implied end tags.
        if BLOCKS.contains(&name.as_str()) {
            if let Some(p) = open_index(&stack, &["p"], &["table", "td", "th", "li", "div", "blockquote", "button"]) {
                pop_to(&mut stack, p);
            }
        }
        let implied: Option<(&[&str], &[&str])> = match name.as_str() {
            "li" => Some((&["li"], &["ul", "ol", "table"])),
            "dt" | "dd" => Some((&["dt", "dd"], &["dl", "table"])),
            "tr" => Some((&["tr"], &["table"])),
            "td" | "th" => Some((&["td", "th"], &["tr", "table"])),
            "thead" | "tbody" | "tfoot" => Some((&["thead", "tbody", "tfoot"], &["table"])),
            "option" => Some((&["option"], &["select"])),
            _ => None,
        };
        if let Some((names, stop)) = implied {
            if let Some(p) = open_index(&stack, names, stop) {
                pop_to(&mut stack, p);
            }
        }
        let mut e = Element::new(&name);
        e.attrs = attrs;
        if VOID.contains(&name.as_str()) || (self_close && !RAW.contains(&name.as_str()) && !BLOCKS.contains(&name.as_str()) && !matches!(name.as_str(), "span" | "a" | "td" | "th" | "tr" | "b" | "i" | "u" | "font")) {
            stack.last_mut().unwrap().children.push(Node::Element(e));
            continue;
        }
        if RAW.contains(&name.as_str()) {
            let close = format!("</{}", name);
            let lower = src[i..].to_ascii_lowercase();
            let end = lower.find(&close).map(|e| i + e).unwrap_or(b.len());
            let raw = &src[i..end];
            let content = if name == "title" || name == "textarea" { decode_entities(raw) } else { raw.to_string() };
            e.children.push(Node::Text(content));
            stack.last_mut().unwrap().children.push(Node::Element(e));
            i = src[end..].find('>').map(|x| end + x + 1).unwrap_or(b.len());
            continue;
        }
        stack.push(e);
    }
    flush_text(&mut stack, &mut text);
    pop_to(&mut stack, 1);
    stack.pop().unwrap()
}

const ENTITIES: &[(&str, char)] = &[
    ("amp", '&'), ("lt", '<'), ("gt", '>'), ("quot", '"'), ("apos", '\''), ("nbsp", '\u{A0}'), ("ensp", '\u{2002}'),
    ("emsp", '\u{2003}'), ("thinsp", '\u{2009}'), ("zwnj", '\u{200C}'), ("zwj", '\u{200D}'), ("shy", '\u{AD}'), ("copy", '©'),
    ("reg", '®'), ("trade", '™'), ("hellip", '…'), ("mdash", '—'), ("ndash", '–'), ("lsquo", '‘'), ("rsquo", '’'), ("sbquo", '‚'),
    ("ldquo", '“'), ("rdquo", '”'), ("bdquo", '„'), ("laquo", '«'), ("raquo", '»'), ("lsaquo", '‹'), ("rsaquo", '›'), ("bull", '•'),
    ("middot", '·'), ("deg", '°'), ("euro", '€'), ("pound", '£'), ("yen", '¥'), ("cent", '¢'), ("curren", '¤'), ("sect", '§'),
    ("para", '¶'), ("times", '×'), ("divide", '÷'), ("plusmn", '±'), ("micro", 'µ'), ("iexcl", '¡'), ("iquest", '¿'), ("ordf", 'ª'),
    ("ordm", 'º'), ("sup1", '¹'), ("sup2", '²'), ("sup3", '³'), ("frac14", '¼'), ("frac12", '½'), ("frac34", '¾'), ("acute", '´'),
    ("cedil", '¸'), ("uml", '¨'), ("macr", '¯'), ("not", '¬'), ("brvbar", '¦'), ("dagger", '†'), ("Dagger", '‡'), ("permil", '‰'),
    ("prime", '′'), ("Prime", '″'), ("larr", '←'), ("rarr", '→'), ("uarr", '↑'), ("darr", '↓'), ("harr", '↔'), ("le", '≤'), ("ge", '≥'),
    ("ne", '≠'), ("asymp", '≈'), ("infin", '∞'), ("minus", '−'), ("alpha", 'α'), ("beta", 'β'), ("gamma", 'γ'), ("delta", 'δ'),
    ("pi", 'π'), ("sigma", 'σ'), ("mu", 'μ'), ("omega", 'ω'), ("Omega", 'Ω'), ("hearts", '♥'), ("check", '✓'),
    ("Agrave", 'À'), ("Aacute", 'Á'), ("Acirc", 'Â'), ("Atilde", 'Ã'), ("Auml", 'Ä'), ("Aring", 'Å'), ("AElig", 'Æ'), ("Ccedil", 'Ç'),
    ("Egrave", 'È'), ("Eacute", 'É'), ("Ecirc", 'Ê'), ("Euml", 'Ë'), ("Igrave", 'Ì'), ("Iacute", 'Í'), ("Icirc", 'Î'), ("Iuml", 'Ï'),
    ("ETH", 'Ð'), ("Ntilde", 'Ñ'), ("Ograve", 'Ò'), ("Oacute", 'Ó'), ("Ocirc", 'Ô'), ("Otilde", 'Õ'), ("Ouml", 'Ö'), ("Oslash", 'Ø'),
    ("Ugrave", 'Ù'), ("Uacute", 'Ú'), ("Ucirc", 'Û'), ("Uuml", 'Ü'), ("Yacute", 'Ý'), ("THORN", 'Þ'), ("szlig", 'ß'), ("agrave", 'à'),
    ("aacute", 'á'), ("acirc", 'â'), ("atilde", 'ã'), ("auml", 'ä'), ("aring", 'å'), ("aelig", 'æ'), ("ccedil", 'ç'), ("egrave", 'è'),
    ("eacute", 'é'), ("ecirc", 'ê'), ("euml", 'ë'), ("igrave", 'ì'), ("iacute", 'í'), ("icirc", 'î'), ("iuml", 'ï'), ("eth", 'ð'),
    ("ntilde", 'ñ'), ("ograve", 'ò'), ("oacute", 'ó'), ("ocirc", 'ô'), ("otilde", 'õ'), ("ouml", 'ö'), ("oslash", 'ø'), ("ugrave", 'ù'),
    ("uacute", 'ú'), ("ucirc", 'û'), ("uuml", 'ü'), ("yacute", 'ý'), ("thorn", 'þ'), ("yuml", 'ÿ'), ("OElig", 'Œ'), ("oelig", 'œ'),
    ("Scaron", 'Š'), ("scaron", 'š'), ("Yuml", 'Ÿ'), ("fnof", 'ƒ'), ("circ", 'ˆ'), ("tilde", '˜'),
];

/// Decode HTML character references.
pub(crate) fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let end = rest.bytes().take(40).position(|b| b == b';');
        let mut done = false;
        if let Some(e) = end {
            let name = &rest[1..e];
            let c = if let Some(num) = name.strip_prefix('#') {
                let v = if let Some(h) = num.strip_prefix(['x', 'X']) { u32::from_str_radix(h, 16).ok() } else { num.parse().ok() };
                v.map(|v| match v {
                    // Windows-1252 code points sent as numeric references.
                    0x80..=0x9F => crate::io::util::cp1252_char(v as u8),
                    _ => char::from_u32(v).unwrap_or('\u{FFFD}'),
                })
            } else {
                ENTITIES.iter().find(|(n, _)| *n == name).or_else(|| ENTITIES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name))).map(|(_, c)| *c)
            };
            if let Some(c) = c {
                out.push(c);
                rest = &rest[e + 1..];
                done = true;
            }
        } else {
            // Legacy references without ';' (e.g. "&nbsp").
            for (n, c) in [("nbsp", '\u{A0}'), ("amp", '&'), ("lt", '<'), ("gt", '>'), ("quot", '"')] {
                if rest[1..].starts_with(n) && !rest[1 + n.len()..].starts_with(|c: char| c.is_ascii_alphanumeric()) {
                    out.push(c);
                    rest = &rest[1 + n.len()..];
                    done = true;
                    break;
                }
            }
        }
        if !done {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

// ===================================================================================================
// Reader
// ===================================================================================================

/// Parse an HTML document.
pub fn read(src: &str) -> Document {
    let root = parse_html(src);
    let mut doc = Document::new();
    let mut r = HtmlReader::new(&root, &doc);
    let body = root.find("body").unwrap_or(&root);
    r.walk_children(body, &Ctx::default());
    r.flush();
    let blocks = std::mem::take(&mut r.blocks);
    let lists = std::mem::take(&mut r.lists);
    let footnotes = std::mem::take(&mut r.footnotes);
    let endnotes = std::mem::take(&mut r.endnotes);
    drop(r);
    doc.body = blocks;
    doc.lists = lists;
    doc.footnotes = footnotes;
    doc.endnotes = endnotes;
    if let Some(t) = root.find("title") {
        let t = t.text().trim().to_string();
        if t != "Document" {
            doc.properties.title = t;
        }
    }
    for m in find_all(&root, "meta") {
        let content = m.attr("content").unwrap_or("").to_string();
        match m.attr("name").map(|s| s.to_ascii_lowercase()).as_deref() {
            Some("author") => doc.properties.author = content,
            Some("description") => doc.properties.description = content,
            Some("keywords") => doc.properties.keywords = content,
            Some("subject") => doc.properties.subject = content,
            _ => {}
        }
    }
    if doc.body.is_empty() {
        doc.body.push(Block::Para(Paragraph::new()));
    }
    doc
}

/// Parse clipboard HTML into blocks (empty when nothing usable). Lists become paragraphs with their
/// labels as text because bare blocks cannot carry list definitions; [`read_fragment_into`] keeps
/// real lists by adding their definitions to a document.
pub fn read_fragment(html: &str) -> Vec<Block> {
    let tmp = Document::new();
    let (mut blocks, lists) = fragment_blocks(html, &tmp);
    if lists.is_empty() {
        return blocks;
    }
    let mut ldoc = Document::new();
    ldoc.lists = lists;
    let mut counter = crate::styles::ListCounter::default();
    for b in blocks.iter_mut() {
        match b {
            Block::Para(p) => labelize(p, &ldoc, &mut counter),
            Block::Table(t) => {
                for row in &mut t.rows {
                    for c in &mut row.cells {
                        for p in &mut c.paragraphs {
                            labelize(p, &ldoc, &mut counter);
                        }
                    }
                }
            }
        }
    }
    blocks
}

fn labelize(p: &mut Paragraph, ldoc: &Document, counter: &mut crate::styles::ListCounter) {
    let Some(Some(l)) = p.props.list else { return };
    if let Some((label, font)) = counter.next_label(ldoc, l) {
        let mut props = p.runs.first().map(|r| r.props.clone()).unwrap_or_default();
        props.link = None;
        if font.is_some() {
            props.font = font;
        }
        p.runs.insert(0, Run { content: Inline::Text(format!("{} ", label)), props });
        if let Some(lvl) = ldoc.list(l.list).and_then(|d| d.levels.get(l.level as usize)) {
            p.props.indent_left = Some(lvl.indent);
            p.props.indent_first = Some(-lvl.hanging);
        }
    }
    p.props.list = None;
}

/// Parse clipboard HTML into blocks, adding the list definitions they use to `doc` (with fresh ids).
pub fn read_fragment_into(html: &str, doc: &mut Document) -> Vec<Block> {
    let (mut blocks, lists) = fragment_blocks(html, doc);
    let mut map = HashMap::new();
    for l in lists {
        let old = l.id;
        let new = doc.add_list(l);
        map.insert(old, new);
    }
    let remap = |p: &mut Paragraph| {
        if let Some(Some(l)) = &mut p.props.list {
            if let Some(n) = map.get(&l.list) {
                l.list = *n;
            }
        }
    };
    for b in blocks.iter_mut() {
        match b {
            Block::Para(p) => remap(p),
            Block::Table(t) => {
                for row in &mut t.rows {
                    for c in &mut row.cells {
                        c.paragraphs.iter_mut().for_each(remap);
                    }
                }
            }
        }
    }
    blocks
}

fn fragment_blocks(html: &str, doc: &Document) -> (Vec<Block>, Vec<ListDef>) {
    // CF_HTML clipboard data starts with a "Version:..." header.
    let start = html.find('<').unwrap_or(html.len());
    let root = parse_html(&html[start..]);
    let mut r = HtmlReader::new(&root, doc);
    let body = root.find("body").unwrap_or(&root);
    // With fragment markers, only the content between them counts.
    r.fragment_state = if has_marker(body) { FragState::Before } else { FragState::Inside };
    r.walk_children(body, &Ctx::default());
    r.flush();
    let mut blocks = std::mem::take(&mut r.blocks);
    let lists = std::mem::take(&mut r.lists);
    // Drop empty paragraphs that come from markup around the fragment.
    while matches!(blocks.last(), Some(Block::Para(p)) if p.runs.is_empty()) {
        blocks.pop();
    }
    while matches!(blocks.first(), Some(Block::Para(p)) if p.runs.is_empty()) {
        blocks.remove(0);
    }
    (blocks, lists)
}

fn has_marker(e: &Element) -> bool {
    e.elems().any(|c| c.name == "#marker" || has_marker(c))
}

fn find_all<'a>(e: &'a Element, name: &str) -> Vec<&'a Element> {
    fn go<'a>(e: &'a Element, name: &str, v: &mut Vec<&'a Element>) {
        for c in e.elems() {
            if c.name == name {
                v.push(c);
            }
            go(c, name, v);
        }
    }
    let mut v = Vec::new();
    go(e, name, &mut v);
    v
}

#[derive(Clone, Default)]
struct Ctx {
    chars: CharProps,
    para: ParaProps,
    pre: bool,
    /// Inline `white-space: pre`: spaces and tabs are kept, newlines are line breaks.
    ws_pre: bool,
    /// The innermost open list: (list id, level).
    list: Option<(u32, u8)>,
    in_table_cell: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum FragState {
    Before,
    Inside,
    After,
}

type CssRule = (String, String, Vec<(String, String)>);

struct HtmlReader<'d> {
    doc: &'d Document,
    css: Vec<CssRule>,
    blocks: Vec<Block>,
    cur: Option<Paragraph>,
    /// Whether the last character added to `cur` was a collapsible space.
    last_space: bool,
    lists: Vec<ListDef>,
    footnotes: Vec<Note>,
    endnotes: Vec<Note>,
    fragment_state: FragState,
    /// Word `mso-list` ids (lfo) → our list ids.
    mso_lists: HashMap<String, u32>,
}

/// Parse a CSS declaration block.
fn parse_decls(s: &str) -> Vec<(String, String)> {
    s.split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            let v = v.trim().trim_end_matches("!important").trim();
            Some((k.trim().to_ascii_lowercase(), v.to_string()))
        })
        .collect()
}

fn parse_stylesheet(css: &str) -> Vec<CssRule> {
    // Strip comments.
    let mut s = String::new();
    let mut rest = css;
    while let Some(p) = rest.find("/*") {
        s.push_str(&rest[..p]);
        rest = rest[p + 2..].split_once("*/").map(|x| x.1).unwrap_or("");
    }
    s.push_str(rest);
    let s = s.replace("<!--", "").replace("-->", "");
    let mut out = Vec::new();
    for rule in s.split('}') {
        let Some((sel, body)) = rule.split_once('{') else { continue };
        let sel = sel.trim();
        if sel.starts_with('@') {
            continue;
        }
        let decls = parse_decls(body);
        for one in sel.split(',') {
            let one = one.trim();
            if one.is_empty() || one.contains([' ', '>', ':', '[', '+', '~', '#']) {
                continue;
            }
            let (tag, class) = match one.split_once('.') {
                Some((t, c)) => (t.to_ascii_lowercase(), c.to_string()),
                None => (one.to_ascii_lowercase(), String::new()),
            };
            out.push((tag, class, decls.clone()));
        }
    }
    out
}

fn font_size_kw(v: &str, parent: f32) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    Some(match v.as_str() {
        "xx-small" => 7.0,
        "x-small" => 7.5,
        "small" => 10.0,
        "medium" => 12.0,
        "large" => 13.5,
        "x-large" => 18.0,
        "xx-large" => 24.0,
        "xxx-large" => 36.0,
        "smaller" => parent / 1.2,
        "larger" => parent * 1.2,
        _ => {
            if let Some(p) = v.strip_suffix('%') {
                return p.trim().parse::<f32>().ok().map(|x| parent * x / 100.0);
            }
            if !v.ends_with("rem") {
                if let Some(e) = v.strip_suffix("em") {
                    return e.trim().parse::<f32>().ok().map(|x| parent * x);
                }
            }
            return parse_length(&v);
        }
    })
}

fn first_family(v: &str) -> Option<String> {
    let f = v.split(',').next()?.trim().trim_matches(['"', '\'']).trim();
    let f = match f.to_ascii_lowercase().as_str() {
        "" | "inherit" | "initial" => return None,
        "serif" => "Times New Roman".to_string(),
        "sans-serif" | "system-ui" | "-apple-system" => "Arial".to_string(),
        "monospace" => "Courier New".to_string(),
        _ => f.to_string(),
    };
    Some(f)
}

fn css_border_parse(v: &str) -> Option<Border> {
    let mut b = Border::single();
    let mut any_style = false;
    for part in v.split_whitespace() {
        let p = part.to_ascii_lowercase();
        match p.as_str() {
            "none" | "hidden" => return None,
            "solid" | "groove" | "ridge" | "inset" | "outset" => {
                b.style = BorderStyle::Single;
                any_style = true;
            }
            "double" => {
                b.style = BorderStyle::Double;
                any_style = true;
            }
            "dotted" => {
                b.style = BorderStyle::Dotted;
                any_style = true;
            }
            "dashed" => {
                b.style = BorderStyle::Dashed;
                any_style = true;
            }
            "thin" => b.width = 0.75,
            "medium" => b.width = 2.25,
            "thick" => b.width = 3.75,
            _ => {
                if let Some(w) = parse_length(&p) {
                    b.width = w;
                } else if let Some(c) = css_color(part) {
                    b.color = c;
                }
            }
        }
    }
    if !any_style || b.width <= 0.0 {
        return None;
    }
    if b.style == BorderStyle::Double {
        b.width = (b.width / 3.0).max(0.25);
    }
    b.width = round2(b.width);
    b.space = 0.0;
    Some(b)
}

fn round2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

fn norm_mult(m: f32) -> f32 {
    let r = round2(m);
    if (r - 1.0).abs() <= 0.02 {
        1.0
    } else {
        r
    }
}

fn parse_align(a: &str) -> Option<Align> {
    match a.trim().to_ascii_lowercase().as_str() {
        "center" | "middle" => Some(Align::Center),
        "right" => Some(Align::Right),
        "justify" => Some(Align::Justify),
        "left" => Some(Align::Left),
        _ => None,
    }
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// `#_ftn3` / `#_edn2` (Word) → (is footnote, id).
fn note_ref(href: &str) -> Option<(bool, u32)> {
    let h = href.strip_prefix('#')?;
    let h = h.strip_prefix('_').unwrap_or(h);
    if let Some(id) = h.strip_prefix("ftn").and_then(|n| n.parse::<u32>().ok()) {
        return Some((true, id));
    }
    if let Some(id) = h.strip_prefix("edn").and_then(|n| n.parse::<u32>().ok()) {
        return Some((false, id));
    }
    None
}

/// A `<div id="_ftn1">` / `<div id="ftn1">` holding note text → (is footnote, id).
fn note_target(e: &Element) -> Option<(bool, u32)> {
    if e.name != "div" {
        return None;
    }
    let s = e.attr("id")?;
    let s = s.strip_prefix('_').unwrap_or(s);
    if let Some(n) = s.strip_prefix("ftn") {
        return n.parse().ok().map(|n| (true, n));
    }
    if let Some(n) = s.strip_prefix("edn") {
        return n.parse().ok().map(|n| (false, n));
    }
    None
}

/// Text of the `mso-list:Ignore` label inside a Word list paragraph.
fn mso_label(e: &Element) -> Option<String> {
    for c in e.elems() {
        if c.attr("style").map(|s| s.replace(' ', "").to_ascii_lowercase().contains("mso-list:ignore")).unwrap_or(false) {
            let t = c.text();
            return Some(t.trim_matches(|ch: char| ch.is_whitespace() || ch == '\u{A0}').to_string());
        }
        if let Some(l) = mso_label(c) {
            return Some(l);
        }
    }
    None
}

impl<'d> HtmlReader<'d> {
    fn new(root: &Element, doc: &'d Document) -> Self {
        let mut css = Vec::new();
        for s in find_all(root, "style") {
            css.extend(parse_stylesheet(&s.text()));
        }
        HtmlReader {
            doc,
            css,
            blocks: Vec::new(),
            cur: None,
            last_space: true,
            lists: Vec::new(),
            footnotes: Vec::new(),
            endnotes: Vec::new(),
            fragment_state: FragState::Inside,
            mso_lists: HashMap::new(),
        }
    }

    /// Effective declarations of an element: stylesheet rules (tag, .class, tag.class) then `style`.
    fn decls(&self, e: &Element) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if !self.css.is_empty() {
            let classes: Vec<&str> = e.attr("class").map(|c| c.split_whitespace().collect()).unwrap_or_default();
            for (tag, class, d) in &self.css {
                if matches!(tag.as_str(), "body" | "html") || (tag.is_empty() && class.is_empty()) {
                    continue;
                }
                let tag_ok = tag.is_empty() || tag == "*" || *tag == e.name;
                let class_ok = class.is_empty() || classes.iter().any(|c| c == class);
                if tag_ok && class_ok {
                    out.extend(d.iter().cloned());
                }
            }
        }
        if let Some(s) = e.attr("style") {
            out.extend(parse_decls(s));
        }
        out
    }

    fn apply_char_css(&self, c: &mut CharProps, d: &[(String, String)]) {
        let parent_size = c.size.unwrap_or(12.0);
        for (k, v) in d {
            let lv = v.to_ascii_lowercase();
            match k.as_str() {
                "font-family" => {
                    if let Some(f) = first_family(v) {
                        c.font = Some(f);
                    }
                }
                "font-size" => {
                    if let Some(s) = font_size_kw(v, parent_size).filter(|s| *s > 0.0) {
                        c.size = Some((s * 2.0).round() / 2.0);
                    }
                }
                "font-weight" => {
                    c.bold = Some(match lv.as_str() {
                        "bold" | "bolder" => true,
                        "normal" | "lighter" => false,
                        n => n.parse::<u32>().map(|w| w >= 600).unwrap_or(false),
                    })
                }
                "font-style" => c.italic = Some(lv.contains("italic") || lv.contains("oblique")),
                "text-decoration" | "text-decoration-line" => {
                    if lv.contains("none") {
                        c.underline = Some(Underline::None);
                        c.strike = Some(false);
                    }
                    if lv.contains("underline") {
                        c.underline = Some(if lv.contains("double") {
                            Underline::Double
                        } else if lv.contains("dotted") {
                            Underline::Dotted
                        } else if lv.contains("dashed") {
                            Underline::Dashed
                        } else if lv.contains("wavy") {
                            Underline::Wavy
                        } else {
                            Underline::Single
                        });
                    }
                    if lv.contains("line-through") {
                        c.strike = Some(true);
                    }
                }
                "text-decoration-style" => {
                    if c.underline.map(|u| u != Underline::None).unwrap_or(false) {
                        c.underline = Some(match lv.as_str() {
                            "double" => Underline::Double,
                            "dotted" => Underline::Dotted,
                            "dashed" => Underline::Dashed,
                            "wavy" => Underline::Wavy,
                            _ => Underline::Single,
                        });
                    } else if lv == "double" && c.strike == Some(true) {
                        c.strike = Some(false);
                        c.double_strike = Some(true);
                    }
                }
                "color" => {
                    if let Some(col) = css_color(v) {
                        c.color = Some(col);
                    }
                }
                "background" | "background-color" => {
                    if let Some(col) = css_color(v.split_whitespace().next().unwrap_or("")) {
                        if HIGHLIGHTS.iter().any(|(_, h)| *h == col) && col != Color::WHITE && col != Color::BLACK {
                            c.highlight = Some(col);
                        } else {
                            c.shading = Some(col);
                        }
                    }
                }
                "mso-highlight" => {
                    if let Some(col) = highlight_color(v).or_else(|| css_color(v)) {
                        c.highlight = Some(col);
                    }
                }
                "vertical-align" => match lv.as_str() {
                    "super" | "top" => c.vert_align = Some(VertAlign::Superscript),
                    "sub" | "bottom" => c.vert_align = Some(VertAlign::Subscript),
                    "baseline" => c.vert_align = Some(VertAlign::Baseline),
                    _ => {}
                },
                "text-transform" => c.caps = Some(lv == "uppercase"),
                "font-variant" | "font-variant-caps" => c.small_caps = Some(lv.contains("small-caps")),
                "display" if lv == "none" => c.hidden = Some(true),
                "visibility" if lv == "hidden" => c.hidden = Some(true),
                "letter-spacing" => {
                    if let Some(l) = parse_length(v) {
                        c.spacing = Some(round2(l));
                    }
                }
                _ => {}
            }
        }
    }

    fn apply_para_css(&self, p: &mut ParaProps, d: &[(String, String)], is_list: bool) {
        let sides = |v: &str| -> Vec<f32> { v.split_whitespace().filter_map(|x| if x == "auto" { Some(0.0) } else { parse_length(x) }).collect() };
        for (k, v) in d {
            let lv = v.trim().to_ascii_lowercase();
            match k.as_str() {
                "text-align" => {
                    p.align = match lv.as_str() {
                        "center" => Some(Align::Center),
                        "right" | "end" => Some(Align::Right),
                        "justify" => Some(Align::Justify),
                        "left" | "start" => Some(Align::Left),
                        _ => p.align,
                    }
                }
                "margin" => {
                    let s = sides(v);
                    let (t, r, b, l) = match s.len() {
                        1 => (s[0], s[0], s[0], s[0]),
                        2 => (s[0], s[1], s[0], s[1]),
                        3 => (s[0], s[1], s[2], s[1]),
                        4 => (s[0], s[1], s[2], s[3]),
                        _ => continue,
                    };
                    p.space_before = Some(round2(t));
                    p.space_after = Some(round2(b));
                    p.indent_right = Some(round2(r));
                    if !is_list {
                        p.indent_left = Some(round2(l));
                    }
                }
                "margin-top" => p.space_before = parse_length(v).map(round2).or(p.space_before),
                "margin-bottom" => p.space_after = parse_length(v).map(round2).or(p.space_after),
                "margin-left" if !is_list => p.indent_left = parse_length(v).map(round2).or(p.indent_left),
                "margin-right" => p.indent_right = parse_length(v).map(round2).or(p.indent_right),
                "text-indent" => p.indent_first = parse_length(v).map(round2).or(p.indent_first),
                "line-height" => {
                    p.line_spacing = if lv == "normal" {
                        None
                    } else if let Some(pc) = lv.strip_suffix('%') {
                        pc.trim().parse::<f32>().ok().map(|x| LineSpacing::Multiple(norm_mult(x / 115.0)))
                    } else if let Ok(x) = lv.parse::<f32>() {
                        Some(LineSpacing::Multiple(norm_mult(x / 1.15)))
                    } else {
                        parse_length(v).map(|x| LineSpacing::Exactly(round2(x)))
                    }
                }
                "background" | "background-color" => {
                    if let Some(c) = css_color(v.split_whitespace().next().unwrap_or("")) {
                        p.shading = Some(c);
                    }
                }
                "page-break-before" | "break-before" if lv == "always" || lv == "page" => p.page_break_before = Some(true),
                "page-break-after" | "break-after" if lv == "avoid" => p.keep_next = Some(true),
                "border" | "border-top" | "border-bottom" | "border-left" | "border-right" => {
                    let b = css_border_parse(v);
                    let bs = p.borders.get_or_insert_with(Borders::default);
                    match k.as_str() {
                        "border" => {
                            bs.top = b;
                            bs.bottom = b;
                            bs.left = b;
                            bs.right = b;
                        }
                        "border-top" => bs.top = b,
                        "border-bottom" => bs.bottom = b,
                        "border-left" => bs.left = b,
                        _ => bs.right = b,
                    }
                }
                "padding-top" | "padding-bottom" | "padding-right" | "padding-left" => {
                    if let (Some(bs), Some(l)) = (p.borders.as_mut(), parse_length(v)) {
                        let b = match k.as_str() {
                            "padding-top" => &mut bs.top,
                            "padding-bottom" => &mut bs.bottom,
                            "padding-left" => &mut bs.left,
                            _ => &mut bs.right,
                        };
                        if let Some(b) = b {
                            b.space = round2(l);
                        }
                    }
                }
                _ => {}
            }
        }
        if p.borders.as_ref().map(|b| b.is_empty()).unwrap_or(false) {
            p.borders = None;
        }
    }

    // ---- paragraph building ---------------------------------------------------------------------

    fn active(&self) -> bool {
        self.fragment_state == FragState::Inside
    }

    fn ensure_para(&mut self, ctx: &Ctx) {
        if self.cur.is_none() && self.active() {
            let mut p = Paragraph::new();
            p.props = ctx.para.clone();
            self.cur = Some(p);
            self.last_space = true;
        }
    }

    fn push_inline(&mut self, ctx: &Ctx, content: Inline) {
        if !self.active() {
            return;
        }
        self.ensure_para(ctx);
        let props = ctx.chars.clone();
        let p = self.cur.as_mut().unwrap();
        if let (Inline::Text(t), Some(last)) = (&content, p.runs.last_mut()) {
            if last.props == props {
                if let Inline::Text(lt) = &mut last.content {
                    lt.push_str(t);
                    return;
                }
            }
        }
        p.runs.push(Run { content, props });
    }

    fn text(&mut self, ctx: &Ctx, t: &str) {
        if !self.active() {
            return;
        }
        if ctx.pre {
            let t = t.replace("\r\n", "\n");
            for (i, line) in t.split('\n').enumerate() {
                if i > 0 {
                    // Each line of preformatted text is its own paragraph.
                    let props = self.cur.as_ref().map(|p| p.props.clone()).unwrap_or_else(|| ctx.para.clone());
                    self.finish_para(true);
                    let mut p = Paragraph::new();
                    p.props = props;
                    self.cur = Some(p);
                }
                let mut buf = String::new();
                for c in line.chars() {
                    if c == '\t' {
                        if !buf.is_empty() {
                            self.push_inline(ctx, Inline::Text(std::mem::take(&mut buf)));
                        }
                        self.push_inline(ctx, Inline::Tab);
                    } else {
                        buf.push(if c == '\u{A0}' { ' ' } else { c });
                    }
                }
                if buf.is_empty() {
                    self.ensure_para(ctx);
                } else {
                    self.push_inline(ctx, Inline::Text(buf));
                }
            }
            return;
        }
        if ctx.ws_pre {
            let mut buf = String::new();
            for ch in t.chars() {
                let special = match ch {
                    '\t' => Some(Inline::Tab),
                    '\n' => Some(Inline::LineBreak),
                    _ => None,
                };
                if let Some(inl) = special {
                    if !buf.is_empty() {
                        self.push_inline(ctx, Inline::Text(std::mem::take(&mut buf)));
                    }
                    self.push_inline(ctx, inl);
                } else if ch != '\r' {
                    buf.push(if ch == '\u{A0}' { ' ' } else { ch });
                }
            }
            if !buf.is_empty() {
                self.push_inline(ctx, Inline::Text(buf));
            }
            self.last_space = false;
            return;
        }
        let mut buf = String::new();
        let mut last_space = if self.cur.is_some() { self.last_space } else { true };
        for c in t.chars() {
            if matches!(c, ' ' | '\n' | '\r' | '\t' | '\u{C}') {
                if !last_space {
                    buf.push(' ');
                    last_space = true;
                }
            } else if c == '\u{A0}' {
                buf.push(' ');
                last_space = false;
            } else {
                buf.push(c);
                last_space = false;
            }
        }
        if buf.is_empty() {
            return;
        }
        self.push_inline(ctx, Inline::Text(buf));
        self.last_space = last_space;
    }

    /// Close the current paragraph. `force` keeps it even when empty.
    fn finish_para(&mut self, force: bool) {
        let Some(mut p) = self.cur.take() else { return };
        self.last_space = true;
        if let Some(Run { content: Inline::Text(t), .. }) = p.runs.last_mut() {
            let n = t.trim_end_matches(' ').len();
            t.truncate(n);
        }
        p.runs.retain(|r| !matches!(&r.content, Inline::Text(t) if t.is_empty()));
        // A trailing <br> does not create a visible line.
        if matches!(p.runs.last(), Some(Run { content: Inline::LineBreak, .. })) {
            p.runs.pop();
        }
        if p.runs.is_empty() && !force {
            return;
        }
        self.blocks.push(Block::Para(p));
    }

    fn flush(&mut self) {
        self.finish_para(false);
    }

    // ---- tree walk ------------------------------------------------------------------------------

    fn walk_children(&mut self, e: &Element, ctx: &Ctx) {
        for n in &e.children {
            match n {
                Node::Text(t) => self.text(ctx, t),
                Node::Element(c) => self.element(c, ctx),
            }
        }
    }

    fn style_for_class(&self, e: &Element) -> Option<String> {
        let class = e.attr("class")?;
        for c in class.split_whitespace() {
            let c = c.strip_prefix("Mso").unwrap_or(c);
            let c = if c.starts_with("ListParagraph") { "ListParagraph" } else { c };
            if c == "Normal" {
                return None;
            }
            if let Some(s) = self.doc.styles.iter().find(|s| s.id.eq_ignore_ascii_case(c) && s.kind == StyleKind::Paragraph) {
                return Some(s.id.clone());
            }
        }
        None
    }

    fn note(&mut self, e: &Element, ctx: &Ctx, foot: bool, id: u32) {
        self.flush();
        let saved = std::mem::take(&mut self.blocks);
        let c2 = Ctx {
            para: ParaProps { style: Some(if foot { "FootnoteText" } else { "EndnoteText" }.into()), ..Default::default() },
            chars: CharProps::default(),
            list: None,
            ..ctx.clone()
        };
        self.walk_children(e, &c2);
        self.flush();
        let mut paras: Vec<Paragraph> = std::mem::replace(&mut self.blocks, saved)
            .into_iter()
            .filter_map(|b| if let Block::Para(p) = b { Some(p) } else { None })
            .collect();
        // The first paragraph starts with the note's own reference mark.
        if let Some(p) = paras.first_mut() {
            p.runs.retain(|r| !matches!(r.content, Inline::FootnoteRef(_) | Inline::EndnoteRef(_)));
            if let Some(Run { content: Inline::Text(t), .. }) = p.runs.first_mut() {
                *t = t.trim_start().to_string();
            }
            let mark = Run {
                content: if foot { Inline::FootnoteRef(id) } else { Inline::EndnoteRef(id) },
                props: CharProps { style: Some(if foot { "FootnoteReference" } else { "EndnoteReference" }.into()), ..Default::default() },
            };
            p.runs.insert(0, mark);
            p.runs.insert(1, Run::text(" "));
            p.runs.retain(|r| !matches!(&r.content, Inline::Text(t) if t.is_empty()));
        }
        if paras.is_empty() {
            paras.push(Paragraph::new());
        }
        let note = Note { id, paragraphs: paras };
        if foot {
            self.footnotes.push(note);
        } else {
            self.endnotes.push(note);
        }
    }

    fn element(&mut self, e: &Element, ctx: &Ctx) {
        let name = e.name.as_str();
        if name == "#marker" {
            match e.attr("v") {
                Some("startfragment") => self.fragment_state = FragState::Inside,
                Some("endfragment") => {
                    self.flush();
                    self.fragment_state = FragState::After;
                }
                _ => {}
            }
            return;
        }
        if matches!(
            name,
            "script" | "style" | "head" | "title" | "meta" | "link" | "noscript" | "template" | "button" | "select" | "input" | "textarea" | "iframe" | "object" | "svg" | "math" | "xml"
        ) {
            return;
        }
        if let Some((foot, id)) = note_target(e) {
            if self.active() {
                self.note(e, ctx, foot, id);
            }
            return;
        }
        let d = self.decls(e);
        let display_none = d.iter().any(|(k, v)| k == "display" && v.trim().eq_ignore_ascii_case("none"));
        if display_none && !matches!(name, "span" | "font" | "b" | "i" | "u" | "s") {
            return;
        }
        // Word list labels.
        if d.iter().any(|(k, v)| k == "mso-list" && v.trim().eq_ignore_ascii_case("ignore")) {
            return;
        }
        if let Some((_, v)) = d.iter().find(|(k, _)| k == "mso-tab-count") {
            let n = v.trim().parse::<usize>().unwrap_or(1).clamp(1, 20);
            for _ in 0..n {
                self.push_inline(ctx, Inline::Tab);
            }
            self.last_space = true;
            return;
        }
        let mut c = ctx.clone();
        match name {
            "b" | "strong" => c.chars.bold = Some(true),
            "i" | "em" | "cite" | "var" | "dfn" => c.chars.italic = Some(true),
            "u" | "ins" => {
                if c.chars.underline.is_none_or(|u| u == Underline::None) {
                    c.chars.underline = Some(Underline::Single);
                }
            }
            "s" | "strike" | "del" => c.chars.strike = Some(true),
            "sup" => c.chars.vert_align = Some(VertAlign::Superscript),
            "sub" => c.chars.vert_align = Some(VertAlign::Subscript),
            "code" | "kbd" | "samp" | "tt" => c.chars.font = Some("Courier New".into()),
            "mark" => c.chars.highlight = Some(Color::YELLOW),
            "small" => c.chars.size = Some(((c.chars.size.unwrap_or(11.0) / 1.2) * 2.0).round() / 2.0),
            "big" => c.chars.size = Some(((c.chars.size.unwrap_or(11.0) * 1.2) * 2.0).round() / 2.0),
            "font" => {
                if let Some(f) = e.attr("face").and_then(first_family) {
                    c.chars.font = Some(f);
                }
                if let Some(col) = e.attr("color").and_then(css_color) {
                    c.chars.color = Some(col);
                }
                if let Some(s) = e.attr("size").and_then(|s| s.trim().parse::<i32>().ok()) {
                    let sizes = [7.5, 10.0, 12.0, 13.5, 18.0, 24.0, 36.0];
                    c.chars.size = Some(sizes[(s.clamp(1, 7) - 1) as usize]);
                }
            }
            "a" => {
                let href = e.attr("href").map(|h| h.trim()).unwrap_or("");
                if ["#_ftnref", "#_ednref", "#ftnref", "#ednref"].iter().any(|p| href.starts_with(p)) {
                    // Back link from a note to its reference: the note gets its own mark.
                    return;
                }
                if let Some((foot, id)) = note_ref(href) {
                    let style = if foot { "FootnoteReference" } else { "EndnoteReference" };
                    let rc = Ctx { chars: CharProps { style: Some(style.into()), ..ctx.chars.clone() }, ..ctx.clone() };
                    self.push_inline(&rc, if foot { Inline::FootnoteRef(id) } else { Inline::EndnoteRef(id) });
                    self.last_space = false;
                    return;
                }
                let anchor = e.attr("id").or_else(|| e.attr("name")).unwrap_or("");
                if !anchor.is_empty() && !anchor.starts_with("_ftn") && !anchor.starts_with("_edn") {
                    self.push_inline(&Ctx { chars: CharProps::default(), ..ctx.clone() }, Inline::Bookmark(anchor.to_string()));
                }
                if !href.is_empty() && !href.to_ascii_lowercase().starts_with("javascript:") {
                    c.chars.link = Some(href.to_string());
                    if c.chars.style.is_none() && self.doc.style("Hyperlink").is_some() {
                        c.chars.style = Some("Hyperlink".into());
                    }
                }
            }
            _ => {}
        }
        self.apply_char_css(&mut c.chars, &d);
        if let Some((_, v)) = d.iter().find(|(k, _)| k == "white-space") {
            c.ws_pre = matches!(v.trim(), "pre" | "pre-wrap" | "break-spaces");
        }
        if let Some(field) = e.attr("data-field") {
            let f = crate::io::docx::field_from_instr(field, &e.text());
            self.push_inline(&c, Inline::Field(f));
            self.last_space = false;
            return;
        }
        match name {
            "br" => {
                let pb = d.iter().any(|(k, v)| (k == "page-break-before" || k == "break-before") && matches!(v.trim(), "always" | "page"));
                self.push_inline(&c, if pb { Inline::PageBreak } else { Inline::LineBreak });
                self.last_space = true;
            }
            "img" => self.image(e, &c, &d),
            "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "dt" | "dd" | "address" | "figcaption" | "caption" => self.para_element(e, &c, &d),
            "pre" | "listing" | "plaintext" => {
                self.flush();
                let mut c2 = c.clone();
                c2.pre = true;
                if c2.chars.font.is_none() || c2.chars.font == ctx.chars.font {
                    c2.chars.font = Some("Courier New".into());
                }
                c2.chars.size = c2.chars.size.or(Some(10.0));
                c2.para.space_after = Some(0.0);
                c2.para.space_before = Some(0.0);
                c2.para.list = None;
                self.apply_para_css(&mut c2.para, &d, false);
                // A newline right after <pre> and one before </pre> are not content.
                let mut e2 = e.clone();
                if let Some(Node::Text(t)) = e2.children.first_mut() {
                    if let Some(s) = t.strip_prefix('\n') {
                        *t = s.to_string();
                    }
                }
                if let Some(Node::Text(t)) = e2.children.last_mut() {
                    if let Some(s) = t.strip_suffix('\n') {
                        *t = s.to_string();
                    }
                }
                self.walk_children(&e2, &c2);
                let force = self.cur.is_some();
                self.finish_para(force);
            }
            "ul" | "ol" | "dir" | "menu" => self.list_element(e, &c, &d),
            "table" => self.table(e, &c, &d),
            "hr" => {
                self.flush();
                if self.active() {
                    let mut p = Paragraph::new();
                    let line = Border { style: BorderStyle::Single, width: 0.75, color: Color::rgb(0xA0, 0xA0, 0xA0), space: 1.0 };
                    p.props.borders = Some(Borders { bottom: Some(line), ..Default::default() });
                    self.blocks.push(Block::Para(p));
                }
            }
            "blockquote" => {
                self.flush();
                let mut c2 = c.clone();
                c2.para.indent_left = Some(c.para.indent_left.unwrap_or(0.0) + 36.0);
                c2.para.indent_right = Some(c.para.indent_right.unwrap_or(0.0) + 36.0);
                self.apply_para_css(&mut c2.para, &d, false);
                self.walk_children(e, &c2);
                self.flush();
            }
            "div" | "section" | "article" | "header" | "footer" | "nav" | "aside" | "main" | "figure" | "form" | "fieldset" | "center" | "dl" | "body"
            | "html" | "tbody" | "thead" | "tfoot" => {
                self.flush();
                let mut c2 = c.clone();
                if name == "center" {
                    c2.para.align = Some(Align::Center);
                }
                if let Some(a) = e.attr("align").and_then(parse_align) {
                    c2.para.align = Some(a);
                }
                // Only alignment and page breaks carry over from containers.
                let mut pp = ParaProps::default();
                self.apply_para_css(&mut pp, &d, false);
                if pp.align.is_some() {
                    c2.para.align = pp.align;
                }
                if pp.page_break_before.is_some() {
                    c2.para.page_break_before = pp.page_break_before;
                }
                self.walk_children(e, &c2);
                self.flush();
            }
            _ => self.walk_children(e, &c),
        }
    }

    /// Paragraph properties for a block element, from the context and the element's CSS.
    fn block_props(&mut self, e: &Element, c: &Ctx, d: &[(String, String)]) -> ParaProps {
        let name = e.name.as_str();
        let mut pp = ParaProps { align: c.para.align, indent_left: c.para.indent_left, indent_right: c.para.indent_right, ..Default::default() };
        pp.page_break_before = c.para.page_break_before;
        if c.in_table_cell {
            pp.space_after = Some(0.0);
        }
        if let Some(h) = name.strip_prefix('h').and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=6).contains(n)) {
            pp.style = Some(format!("Heading{}", h));
        }
        if let Some(s) = self.style_for_class(e) {
            pp.style = Some(s);
        }
        if let Some(a) = e.attr("align").and_then(parse_align) {
            pp.align = Some(a);
        }
        if name == "li" {
            if let Some((list, level)) = c.list {
                pp.list = Some(Some(ListRef { list, level }));
            }
        }
        if name == "dd" {
            pp.indent_left = Some(pp.indent_left.unwrap_or(0.0) + 36.0);
        }
        // Word clipboard lists: <p style="mso-list:l0 level1 lfo1">.
        if let Some((_, v)) = d.iter().find(|(k, _)| k == "mso-list") {
            if let Some(lr) = self.mso_list(e, v) {
                pp.list = Some(Some(lr));
            }
        }
        let is_list = matches!(pp.list, Some(Some(_)));
        self.apply_para_css(&mut pp, d, is_list);
        pp
    }

    fn para_element(&mut self, e: &Element, c: &Ctx, d: &[(String, String)]) {
        self.flush();
        let mut c2 = c.clone();
        c2.para = self.block_props(e, c, d);
        // Page breaks from a container apply to its first paragraph only.
        if c.para.page_break_before == Some(true) {
            c2.para.page_break_before = c2.para.page_break_before.or(Some(true));
        }
        if e.name == "dt" {
            c2.chars.bold = Some(true);
        }
        if e.name != "li" {
            self.walk_children(e, &c2);
            self.ensure_para(&c2);
            let force = self.active();
            self.finish_para(force);
            return;
        }
        // A list item: inline content forms the item paragraph; block children follow it.
        let lvl = c.list.map(|l| l.1).unwrap_or(0);
        let follow = Ctx { para: ParaProps { indent_left: Some(36.0 * (lvl as f32 + 1.0)), ..Default::default() }, list: c.list, ..c.clone() };
        let mut item_done = false;
        for n in &e.children {
            match n {
                Node::Element(ch) if matches!(ch.name.as_str(), "p" | "div") && !item_done && self.cur.is_none() => {
                    // <li><p>text</p></li>: the paragraph is the item itself.
                    let mut c3 = c2.clone();
                    let dd = self.decls(ch);
                    self.apply_para_css(&mut c3.para, &dd, true);
                    self.walk_children(ch, &c3);
                    self.ensure_para(&c3);
                    let force = self.active();
                    self.finish_para(force);
                    item_done = true;
                }
                Node::Element(ch) if BLOCKS.contains(&ch.name.as_str()) || ch.name == "table" => {
                    if !item_done {
                        self.ensure_para(&c2);
                        let force = self.active();
                        self.finish_para(force);
                        item_done = true;
                    }
                    self.flush();
                    self.element(ch, &follow);
                }
                Node::Element(ch) => {
                    if item_done && self.cur.is_none() {
                        self.element(ch, &follow);
                    } else {
                        self.element(ch, &c2);
                    }
                }
                Node::Text(t) => {
                    if item_done && self.cur.is_none() {
                        if !t.trim().is_empty() {
                            self.text(&follow, t);
                        }
                    } else {
                        self.text(&c2, t);
                    }
                }
            }
        }
        if !item_done {
            self.ensure_para(&c2);
            let force = self.active();
            self.finish_para(force);
        } else {
            self.flush();
        }
    }

    fn mso_list(&mut self, e: &Element, v: &str) -> Option<ListRef> {
        let mut lid = String::new();
        let mut level = 0u8;
        let mut lfo = String::new();
        for part in v.split_whitespace() {
            if let Some(l) = part.strip_prefix("level") {
                level = l.parse::<u8>().unwrap_or(1).saturating_sub(1).min(8);
            } else if let Some(l) = part.strip_prefix("lfo") {
                lfo = l.to_string();
            } else if part.starts_with('l') && part[1..].chars().all(|c| c.is_ascii_digit()) {
                lid = part.to_string();
            }
        }
        if lid.is_empty() && lfo.is_empty() {
            return None;
        }
        let key = if lfo.is_empty() { lid } else { format!("lfo{}", lfo) };
        // The ignored label tells bullets from numbers.
        let label = mso_label(e).unwrap_or_default();
        let t = label.trim_end_matches(['.', ')']).trim_start_matches('(');
        let ordered = !t.is_empty() && (t.chars().all(|c| c.is_ascii_digit()) || (t.chars().count() <= 4 && t.chars().all(|c| c.is_ascii_alphabetic())) && label.len() > t.len());
        let id = match self.mso_lists.get(&key) {
            Some(id) => *id,
            None => {
                let id = self.lists.len() as u32 + 1;
                self.lists.push(if ordered { ListDef::numbered(id) } else { ListDef::bullets(id) });
                self.mso_lists.insert(key, id);
                id
            }
        };
        let seen = self.blocks.iter().any(|b| matches!(b, Block::Para(p) if matches!(p.props.list, Some(Some(r)) if r.list == id && r.level == level)));
        if seen {
            return Some(ListRef { list: id, level });
        }
        if let Some(lvl) = self.lists.iter_mut().find(|l| l.id == id).and_then(|d| d.levels.get_mut(level as usize)) {
            if ordered {
                let first = t.chars().next().unwrap_or('1');
                lvl.format = if first.is_ascii_digit() {
                    NumFormat::Decimal
                } else if t.chars().all(|c| "ivxl".contains(c)) {
                    NumFormat::LowerRoman
                } else if t.chars().all(|c| "IVXL".contains(c)) {
                    NumFormat::UpperRoman
                } else if first.is_ascii_lowercase() {
                    NumFormat::LowerLetter
                } else {
                    NumFormat::UpperLetter
                };
                if let Ok(n) = t.parse::<u32>() {
                    lvl.start = n.max(1);
                }
                let suffix = if label.ends_with(')') { ")" } else { "." };
                lvl.text = format!("%{}{}", level + 1, suffix);
            } else {
                lvl.format = NumFormat::Bullet;
                let b = match label.trim() {
                    "o" => "◦",
                    "§" | "\u{F0A7}" => "▪",
                    "·" | "\u{F0B7}" | "" => "•",
                    other => other,
                };
                lvl.text = b.to_string();
                lvl.font = None;
            }
        }
        Some(ListRef { list: id, level })
    }

    fn list_element(&mut self, e: &Element, c: &Ctx, d: &[(String, String)]) {
        self.flush();
        let ordered = e.name == "ol";
        let (id, level) = match c.list {
            Some((id, lvl)) => (id, (lvl + 1).min(8)),
            None => {
                let id = self.lists.len() as u32 + 1;
                self.lists.push(if ordered { ListDef::numbered(id) } else { ListDef::bullets(id) });
                (id, 0)
            }
        };
        let style_type = d
            .iter()
            .rev()
            .find(|(k, _)| k == "list-style-type" || k == "list-style")
            .map(|(_, v)| v.trim().to_string())
            .or_else(|| e.attr("type").map(|t| t.to_string()));
        let kind = style_type.as_deref().map(|s| s.split_whitespace().next().unwrap_or("").to_string()).unwrap_or_default();
        if let Some(lvl) = self.lists.iter_mut().find(|l| l.id == id).and_then(|d| d.levels.get_mut(level as usize)) {
            let (fmt, bullet) = match kind.as_str() {
                "decimal" | "1" => (NumFormat::Decimal, None),
                "decimal-leading-zero" => (NumFormat::DecimalZero, None),
                "lower-alpha" | "lower-latin" | "a" => (NumFormat::LowerLetter, None),
                "upper-alpha" | "upper-latin" | "A" => (NumFormat::UpperLetter, None),
                "lower-roman" | "i" => (NumFormat::LowerRoman, None),
                "upper-roman" | "I" => (NumFormat::UpperRoman, None),
                "disc" => (NumFormat::Bullet, Some("•")),
                "circle" => (NumFormat::Bullet, Some("◦")),
                "square" => (NumFormat::Bullet, Some("▪")),
                "none" => (NumFormat::None, None),
                _ if ordered => (if lvl.format == NumFormat::Bullet { NumFormat::Decimal } else { lvl.format }, None),
                _ => (NumFormat::Bullet, None),
            };
            if fmt == NumFormat::Bullet {
                lvl.format = NumFormat::Bullet;
                if let Some(b) = bullet {
                    lvl.text = b.to_string();
                } else if !matches!(lvl.text.as_str(), "•" | "◦" | "▪") {
                    lvl.text = ["•", "◦", "▪"][level as usize % 3].to_string();
                }
                lvl.font = None;
            } else {
                lvl.format = fmt;
                if !lvl.text.contains('%') {
                    lvl.text = format!("%{}.", level + 1);
                }
            }
            if let Some(s) = e.attr("start").and_then(|s| s.trim().parse::<u32>().ok()) {
                lvl.start = s;
            }
        }
        let c2 = Ctx { list: Some((id, level)), para: ParaProps { align: c.para.align, ..Default::default() }, ..c.clone() };
        for n in &e.children {
            match n {
                Node::Element(ch) if ch.name == "li" || matches!(ch.name.as_str(), "ul" | "ol") => self.element(ch, &c2),
                Node::Element(ch) => self.element(ch, &Ctx { list: c.list, ..c2.clone() }),
                Node::Text(t) => {
                    if !t.trim().is_empty() {
                        self.text(&c2, t);
                    }
                }
            }
        }
        self.flush();
    }

    fn image(&mut self, e: &Element, c: &Ctx, d: &[(String, String)]) {
        if !self.active() {
            return;
        }
        let src = e.attr("src").unwrap_or("").trim();
        let alt = e.attr("alt").unwrap_or("").to_string();
        let bytes = src.strip_prefix("data:").and_then(|rest| {
            let (meta, payload) = rest.split_once(',')?;
            if meta.ends_with(";base64") {
                base64_decode(payload)
            } else {
                Some(percent_decode(payload))
            }
        });
        let data = bytes.map(image_data).filter(|d| d.pixel_size.is_some() || matches!(d.format.as_str(), "png" | "jpeg" | "gif" | "bmp"));
        let Some(data) = data else {
            if !alt.is_empty() {
                self.text(c, &format!("[{}]", alt));
            }
            return;
        };
        let mut w = None;
        let mut h = None;
        let mut float = None;
        for (k, v) in d {
            match k.as_str() {
                "width" => w = parse_length(v).or(w),
                "height" => h = parse_length(v).or(h),
                "float" => {
                    float = match v.trim() {
                        "left" => Some(FloatAlign::Start),
                        "right" => Some(FloatAlign::End),
                        _ => None,
                    }
                }
                _ => {}
            }
        }
        if float.is_none() {
            float = match e.attr("align").map(|a| a.trim().to_ascii_lowercase()) {
                Some(a) if a == "left" => Some(FloatAlign::Start),
                Some(a) if a == "right" => Some(FloatAlign::End),
                _ => None,
            };
        }
        let px = |v: &str| if v.ends_with('%') { None } else { v.trim().trim_end_matches("px").parse::<f32>().ok().map(|x| x * 0.75) };
        w = w.or_else(|| e.attr("width").and_then(px));
        h = h.or_else(|| e.attr("height").and_then(px));
        let (nw, nh) = natural_size(&data, 468.0);
        let (w, h) = match (w, h) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, nh * w / nw),
            (None, Some(h)) => (nw * h / nh, h),
            (None, None) => (nw, nh),
        };
        let float = float.map(|a| Float { wrap: WrapMode::Square, h_rel: HRel::Column, v_rel: VRel::Paragraph, h_align: Some(a), ..Default::default() });
        let img = Image { data, width: round2(w), height: round2(h), alt, float };
        self.push_inline(c, Inline::Image(img));
        self.last_space = false;
    }

    fn table(&mut self, e: &Element, c: &Ctx, d: &[(String, String)]) {
        self.flush();
        if !self.active() {
            // The fragment may start inside a table: take the cells' content as paragraphs.
            self.walk_children(e, c);
            self.flush();
            return;
        }
        let mut rows: Vec<(&Element, bool)> = Vec::new();
        fn collect<'a>(e: &'a Element, header: bool, rows: &mut Vec<(&'a Element, bool)>) {
            for ch in e.elems() {
                match ch.name.as_str() {
                    "tr" => rows.push((ch, header)),
                    "thead" => collect(ch, true, rows),
                    "tbody" | "tfoot" => collect(ch, false, rows),
                    _ => {}
                }
            }
        }
        collect(e, false, &mut rows);
        if rows.is_empty() {
            self.walk_children(e, c);
            self.flush();
            return;
        }
        let mut any_border = e.attr("border").and_then(|b| b.trim().parse::<f32>().ok()).unwrap_or(0.0) > 0.0;
        let mut table = Table { style: None, borders: Borders::default(), ..Default::default() };
        // Remaining rowspan and colspan of merged cells per grid column.
        let mut pending: Vec<(u32, u32)> = Vec::new();
        let mut col_widths: Vec<Option<f32>> = Vec::new();
        fn continues(gc: &mut usize, row: &mut Row, pending: &mut [(u32, u32)]) {
            while *gc < pending.len() && pending[*gc].0 > 0 {
                pending[*gc].0 -= 1;
                let span = pending[*gc].1.max(1);
                row.cells.push(Cell { vmerge: VMerge::Continue, span, ..Default::default() });
                *gc += span as usize;
            }
        }
        for (ri, (tr, thead)) in rows.iter().enumerate() {
            let mut row = Row { cells: Vec::new(), height: None, header: *thead };
            if let Some((_, v)) = self.decls(tr).iter().find(|(k, _)| k == "height") {
                row.height = parse_length(v).map(round2);
            }
            let tds: Vec<&Element> = tr.elems().filter(|x| x.name == "td" || x.name == "th").collect();
            if ri == 0 && !tds.is_empty() && tds.iter().all(|x| x.name == "th") {
                row.header = true;
            }
            let mut gc = 0usize;
            for td in tds {
                continues(&mut gc, &mut row, &mut pending);
                let colspan = td.attr("colspan").and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(1).clamp(1, 64);
                let rowspan = td.attr("rowspan").and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(1).clamp(1, 1000);
                let dd = self.decls(td);
                let mut cell = Cell { span: colspan, ..Default::default() };
                let mut cb = Borders::default();
                let mut has_cb = false;
                for (k, v) in &dd {
                    match k.as_str() {
                        "background" | "background-color" => cell.shading = css_color(v.split_whitespace().next().unwrap_or("")),
                        "vertical-align" => {
                            cell.valign = match v.trim().to_ascii_lowercase().as_str() {
                                "middle" | "center" => VAlign::Center,
                                "bottom" => VAlign::Bottom,
                                _ => VAlign::Top,
                            }
                        }
                        "width" => cell.width = parse_length(v).map(round2).or(cell.width),
                        "border" | "border-top" | "border-bottom" | "border-left" | "border-right" => {
                            let b = css_border_parse(v);
                            has_cb = true;
                            match k.as_str() {
                                "border" => {
                                    cb.top = b;
                                    cb.bottom = b;
                                    cb.left = b;
                                    cb.right = b;
                                }
                                "border-top" => cb.top = b,
                                "border-bottom" => cb.bottom = b,
                                "border-left" => cb.left = b,
                                _ => cb.right = b,
                            }
                        }
                        _ => {}
                    }
                }
                if cell.shading.is_none() {
                    cell.shading = td.attr("bgcolor").and_then(css_color);
                }
                if let Some(va) = td.attr("valign") {
                    cell.valign = match va.to_ascii_lowercase().as_str() {
                        "middle" | "center" => VAlign::Center,
                        "bottom" => VAlign::Bottom,
                        _ => VAlign::Top,
                    };
                }
                if cell.width.is_none() {
                    cell.width = td.attr("width").and_then(|w| if w.ends_with('%') { None } else { w.trim().parse::<f32>().ok().map(|x| round2(x * 0.75)) });
                }
                if has_cb && [cb.top, cb.bottom, cb.left, cb.right].iter().any(|b| b.is_some()) {
                    any_border = true;
                    cell.borders = Some(cb);
                }
                // Cell content.
                let saved_blocks = std::mem::take(&mut self.blocks);
                let saved_cur = self.cur.take();
                let mut cc = Ctx { list: None, in_table_cell: true, para: ParaProps { space_after: Some(0.0), ..Default::default() }, ..c.clone() };
                cc.para.align = td.attr("align").and_then(parse_align);
                let mut ppd = ParaProps::default();
                self.apply_para_css(&mut ppd, &dd, false);
                cc.para.align = ppd.align.or(cc.para.align);
                if td.name == "th" && !dd.iter().any(|(k, _)| k == "font-weight") {
                    cc.chars.bold = Some(true);
                }
                if td.name == "th" && cc.para.align.is_none() {
                    cc.para.align = Some(Align::Center);
                }
                self.apply_char_css(&mut cc.chars, &dd);
                self.walk_children(td, &cc);
                self.flush();
                let content = std::mem::replace(&mut self.blocks, saved_blocks);
                self.cur = saved_cur;
                let mut paras = Vec::new();
                for b in content {
                    match b {
                        Block::Para(p) => paras.push(p),
                        Block::Table(t) => {
                            // Nested tables are flattened to one paragraph per row.
                            for r in t.rows {
                                let mut p = Paragraph::new();
                                for cl in r.cells.iter().filter(|cl| cl.vmerge != VMerge::Continue) {
                                    if !p.runs.is_empty() {
                                        p.runs.push(Run { content: Inline::Tab, props: CharProps::default() });
                                    }
                                    for (j, cp) in cl.paragraphs.iter().enumerate() {
                                        if j > 0 {
                                            p.runs.push(Run::text(" "));
                                        }
                                        p.runs.extend(cp.runs.iter().cloned());
                                    }
                                }
                                paras.push(p);
                            }
                        }
                    }
                }
                if paras.is_empty() {
                    let mut p = Paragraph::new();
                    p.props = cc.para.clone();
                    paras.push(p);
                }
                cell.paragraphs = paras;
                if rowspan > 1 {
                    cell.vmerge = VMerge::Restart;
                }
                let need = gc + colspan as usize;
                if col_widths.len() < need {
                    col_widths.resize(need, None);
                }
                if pending.len() < need {
                    pending.resize(need, (0, 1));
                }
                if colspan == 1 && col_widths[gc].is_none() {
                    col_widths[gc] = cell.width;
                }
                if rowspan > 1 {
                    pending[gc] = (rowspan - 1, colspan);
                }
                row.cells.push(cell);
                gc += colspan as usize;
            }
            continues(&mut gc, &mut row, &mut pending);
            if !row.cells.is_empty() {
                table.rows.push(row);
            }
        }
        if table.rows.is_empty() {
            return;
        }
        let ncols = table.rows.iter().map(|r| r.cells.iter().map(|c| c.span.max(1) as usize).sum::<usize>()).max().unwrap_or(1).max(1);
        for r in &mut table.rows {
            let n: usize = r.cells.iter().map(|c| c.span.max(1) as usize).sum();
            for _ in n..ncols {
                r.cells.push(Cell::default());
            }
        }
        // <col> widths.
        let mut colgroup: Vec<f32> = Vec::new();
        for ch in e.elems() {
            let cols: Vec<&Element> = match ch.name.as_str() {
                "colgroup" => ch.elems().filter(|x| x.name == "col").collect(),
                "col" => vec![ch],
                _ => vec![],
            };
            for col in cols {
                let span = col.attr("span").and_then(|s| s.parse::<usize>().ok()).unwrap_or(1).max(1);
                let w = self
                    .decls(col)
                    .iter()
                    .find(|(k, _)| k == "width")
                    .and_then(|(_, v)| parse_length(v))
                    .or_else(|| col.attr("width").and_then(|w| w.trim().parse::<f32>().ok().map(|x| x * 0.75)));
                colgroup.extend(std::iter::repeat_n(w.unwrap_or(0.0), span));
            }
        }
        let mut table_width = None;
        let mut auto = (false, false);
        for (k, v) in d {
            let v = v.trim();
            match k.as_str() {
                "width" => table_width = if let Some(p) = v.strip_suffix('%') { p.parse::<f32>().ok().map(|p| 468.0 * p / 100.0) } else { parse_length(v) },
                "margin-left" => {
                    if v == "auto" {
                        auto.0 = true;
                    } else if let Some(l) = parse_length(v) {
                        table.indent = round2(l);
                    }
                }
                "margin-right" => auto.1 = v == "auto",
                "margin" => {
                    let parts: Vec<&str> = v.split_whitespace().collect();
                    if parts.len() >= 2 && parts[1] == "auto" {
                        auto = (true, true);
                    }
                }
                "border" | "border-top" if css_border_parse(v).is_some() => any_border = true,
                _ => {}
            }
        }
        if table_width.is_none() {
            table_width = e.attr("width").and_then(|w| {
                let w = w.trim();
                if let Some(p) = w.strip_suffix('%') {
                    p.parse::<f32>().ok().map(|p| 468.0 * p / 100.0)
                } else {
                    w.parse::<f32>().ok().map(|x| x * 0.75)
                }
            });
        }
        table.align = match (auto, e.attr("align").map(|a| a.to_ascii_lowercase())) {
            ((true, true), _) => Align::Center,
            ((true, false), _) => Align::Right,
            (_, Some(a)) if a == "center" => Align::Center,
            (_, Some(a)) if a == "right" => Align::Right,
            _ => Align::Left,
        };
        let mut widths: Vec<Option<f32>> =
            (0..ncols).map(|i| colgroup.get(i).copied().filter(|w| *w > 0.0).or_else(|| col_widths.get(i).copied().flatten())).collect();
        let known: f32 = widths.iter().flatten().sum();
        let unknown = widths.iter().filter(|w| w.is_none()).count();
        let total = table_width.unwrap_or(if unknown == 0 { known } else { 468.0f32.max(known + 36.0 * unknown as f32) });
        let each = if unknown > 0 { ((total - known) / unknown as f32).max(18.0) } else { 0.0 };
        for w in widths.iter_mut() {
            w.get_or_insert(each);
        }
        table.columns = widths.into_iter().map(|w| round2(w.unwrap_or(each))).collect();
        if any_border {
            let b = table.rows.iter().flat_map(|r| r.cells.iter()).find_map(|c| c.borders.as_ref().and_then(|b| b.top.or(b.left))).unwrap_or_else(Border::single);
            table.borders = Borders::all(b);
            table.style = Some("TableGrid".into());
            // Cell borders identical to the table's are redundant.
            for r in &mut table.rows {
                for cl in &mut r.cells {
                    if let Some(cb) = &cl.borders {
                        let same = |x: &Option<Border>| x.map(|x| x == b).unwrap_or(false);
                        if same(&cb.top) && same(&cb.bottom) && same(&cb.left) && same(&cb.right) {
                            cl.borders = None;
                        }
                    }
                }
            }
        }
        self.blocks.push(Block::Table(table));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities() {
        assert_eq!(decode_entities("a &amp; b &lt;&gt; &#233;&#x20AC; &nbsp;x &bogus; &copy"), "a & b <> é€ \u{A0}x &bogus; &copy");
        assert_eq!(decode_entities("&#150;"), "–");
    }

    #[test]
    fn implied_end_tags() {
        let r = parse_html("<p>one<p>two<ul><li>a<li>b</ul><table><tr><td>x<td>y<tr><td>z</table>");
        let names: Vec<&str> = r.elems().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["p", "p", "ul", "table"]);
        assert_eq!(r.child("ul").unwrap().elems().count(), 2);
        let t = r.child("table").unwrap();
        assert_eq!(t.elems().count(), 2);
        assert_eq!(t.elems().next().unwrap().elems().count(), 2);
    }
}
