//! Rich Text Format reader and writer.
//!
//! The writer emits fully resolved formatting (RTF readers do not apply style definitions), a font and
//! colour table, a stylesheet with the used paragraph styles, Word list tables, tables (`\trowd`/`\cellx`),
//! images (`\pict`), fields, bookmarks, notes, comments, revisions, headers/footers and page setup.
//! The reader covers the common subset produced by Word, LibreOffice and WordPad.

use crate::io::util::*;
use crate::model::*;
use crate::styles::{resolve_chars, resolve_para, ListCounter};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

// ======================================================================================
// Writer
// ======================================================================================

struct RtfWriter<'a> {
    doc: &'a Document,
    /// Paragraph formatting that continues the current paragraph after a page break (readers such as
    /// LibreOffice only honour page breaks at paragraph starts, so `\page` splits the paragraph).
    page_cont: Option<String>,
    fonts: Vec<String>,
    colors: Vec<Color>,
    styles: Vec<String>,
    authors: Vec<String>,
    list_ls: HashMap<u32, usize>,
    counter: ListCounter,
    comment_ranges: HashMap<u32, (usize, usize)>,
    run_counter: usize,
    /// `\sectd` + properties + headers of every section (index = section).
    sect_heads: Vec<String>,
    sect_idx: usize,
    shape_id: usize,
}

/// Escape text for RTF (ASCII output: cp1252 as `\'hh`, everything else as `\uN?`).
fn rtf_text(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\t' => out.push_str("\\tab "),
            '\n' | '\u{000B}' => out.push_str("\\line "),
            '\u{000C}' => out.push_str("\\page "),
            '\u{00A0}' => out.push_str("\\~"),
            '\u{00AD}' => out.push_str("\\-"),
            '\u{2011}' => out.push_str("\\_"),
            '\r' => {}
            c if (' '..='~').contains(&c) => out.push(c),
            c => {
                if let Some(b) = cp1252_encode(c).filter(|b| *b >= 0x80) {
                    let _ = write!(out, "\\'{:02x}", b);
                } else {
                    let mut buf = [0u16; 2];
                    for u in c.encode_utf16(&mut buf) {
                        let _ = write!(out, "\\u{}?", *u as i16);
                    }
                }
            }
        }
    }
}

fn tw(v: f32) -> i64 {
    twips(v)
}

impl<'a> RtfWriter<'a> {
    fn font(&mut self, name: &str) -> usize {
        if let Some(i) = self.fonts.iter().position(|f| f == name) {
            return i;
        }
        self.fonts.push(name.to_string());
        self.fonts.len() - 1
    }

    /// Colour table index (1-based; 0 is "auto").
    fn color(&mut self, c: Color) -> usize {
        if let Some(i) = self.colors.iter().position(|x| *x == c) {
            return i + 1;
        }
        self.colors.push(c);
        self.colors.len()
    }

    fn style_index(&mut self, id: &str) -> usize {
        if let Some(i) = self.styles.iter().position(|s| s == id) {
            return i;
        }
        self.styles.push(id.to_string());
        self.styles.len() - 1
    }

    fn author(&mut self, a: &str) -> usize {
        if let Some(i) = self.authors.iter().position(|x| x == a) {
            return i;
        }
        self.authors.push(a.to_string());
        self.authors.len() - 1
    }

    fn border(&mut self, out: &mut String, tag: &str, b: &Option<Border>) {
        let Some(b) = b else { return };
        let _ = write!(out, "\\{}", tag);
        if !b.is_visible() {
            out.push_str("\\brdrnone");
            return;
        }
        out.push_str(match b.style {
            BorderStyle::Double => "\\brdrdb",
            BorderStyle::Dotted => "\\brdrdot",
            BorderStyle::Dashed => "\\brdrdash",
            BorderStyle::Thick => "\\brdrth",
            _ => "\\brdrs",
        });
        let ci = self.color(b.color);
        let _ = write!(out, "\\brdrw{}\\brsp{}\\brdrcf{}", tw(b.width).clamp(1, 255), tw(b.space), ci);
    }

    fn char_fmt(&mut self, para_style: &str, props: &CharProps) -> String {
        let c = resolve_chars(self.doc, para_style, props);
        let mut s = String::new();
        let fi = self.font(&c.font);
        let _ = write!(s, "\\f{}\\fs{}", fi, (c.size * 2.0).round() as i64);
        if c.bold {
            s.push_str("\\b");
        }
        if c.italic {
            s.push_str("\\i");
        }
        s.push_str(match c.underline {
            Underline::None => "",
            Underline::Single => "\\ul",
            Underline::Double => "\\uldb",
            Underline::Thick => "\\ulth",
            Underline::Dotted => "\\uld",
            Underline::Dashed => "\\uldash",
            Underline::Wavy => "\\ulwave",
            Underline::Words => "\\ulw",
        });
        if c.strike {
            s.push_str("\\strike");
        }
        if c.double_strike {
            s.push_str("\\striked1");
        }
        if c.color != Color::BLACK {
            let ci = self.color(c.color);
            let _ = write!(s, "\\cf{}", ci);
        }
        if let Some(h) = c.highlight {
            let named = highlight_color(highlight_name(h)).unwrap_or(h);
            let ci = self.color(named);
            let _ = write!(s, "\\highlight{}", ci);
        }
        if let Some(sh) = c.shading {
            let ci = self.color(sh);
            let _ = write!(s, "\\chcbpat{}", ci);
        }
        match c.vert_align {
            VertAlign::Superscript => s.push_str("\\super"),
            VertAlign::Subscript => s.push_str("\\sub"),
            VertAlign::Baseline => {}
        }
        if c.caps {
            s.push_str("\\caps");
        }
        if c.small_caps {
            s.push_str("\\scaps");
        }
        if c.hidden {
            s.push_str("\\v");
        }
        if c.spacing != 0.0 {
            let _ = write!(s, "\\expndtw{}", tw(c.spacing));
        }
        if let Some(r) = &props.revision {
            let a = self.author(if r.author.is_empty() { "Unknown" } else { &r.author });
            let dttm = dttm(&r.date);
            match r.kind {
                RevisionKind::Insert => {
                    let _ = write!(s, "\\revised\\revauth{}\\revdttm{}", a, dttm);
                }
                RevisionKind::Delete => {
                    let _ = write!(s, "\\deleted\\revauthdel{}\\revdttmdel{}", a, dttm);
                }
            }
        }
        s
    }

    fn para_fmt(&mut self, p: &Paragraph, in_table: bool) -> String {
        let r = resolve_para(self.doc, &p.props);
        let mut s = String::from("\\pard\\plain");
        let si = self.style_index(&r.style);
        let _ = write!(s, "\\s{}", si);
        if let Some(l) = r.outline_level {
            let _ = write!(s, "\\outlinelevel{}", l);
        }
        s.push_str(match r.align {
            Align::Left => "\\ql",
            Align::Center => "\\qc",
            Align::Right => "\\qr",
            Align::Justify => "\\qj",
        });
        let _ = write!(s, "\\li{}\\ri{}\\fi{}\\sb{}\\sa{}", tw(r.indent_left), tw(r.indent_right), tw(r.indent_first), tw(r.space_before), tw(r.space_after));
        match r.line_spacing {
            LineSpacing::Multiple(m) => {
                let _ = write!(s, "\\sl{}\\slmult1", (m * 240.0).round() as i64);
            }
            LineSpacing::AtLeast(v) => {
                let _ = write!(s, "\\sl{}\\slmult0", tw(v));
            }
            LineSpacing::Exactly(v) => {
                let _ = write!(s, "\\sl-{}\\slmult0", tw(v));
            }
        }
        if r.contextual_spacing {
            s.push_str("\\contextualspace");
        }
        if r.keep_next {
            s.push_str("\\keepn");
        }
        if r.keep_lines {
            s.push_str("\\keep");
        }
        if r.page_break_before {
            s.push_str("\\pagebb");
        }
        s.push_str(if r.widow_control { "\\widctlpar" } else { "\\nowidctlpar" });
        if let Some(c) = r.shading {
            let ci = self.color(c);
            let _ = write!(s, "\\cbpat{}", ci);
        }
        let b = r.borders.clone();
        for (tag, br) in [("brdrt", &b.top), ("brdrb", &b.bottom), ("brdrl", &b.left), ("brdrr", &b.right), ("brdrbtw", &b.between)] {
            if br.map(|x| x.is_visible()).unwrap_or(false) {
                self.border(&mut s, tag, br);
            }
        }
        for t in &r.tabs {
            s.push_str(match t.align {
                TabAlign::Center => "\\tqc",
                TabAlign::Right => "\\tqr",
                TabAlign::Decimal => "\\tqdec",
                _ => "",
            });
            s.push_str(match t.leader {
                TabLeader::Dot => "\\tldot",
                TabLeader::Hyphen => "\\tlhyph",
                TabLeader::Underscore => "\\tlul",
                TabLeader::None => "",
            });
            let _ = write!(s, "\\tx{}", tw(t.pos));
        }
        if let Some(l) = r.list {
            if let Some(ls) = self.list_ls.get(&l.list) {
                let _ = write!(s, "\\ls{}\\ilvl{}", ls, l.level);
            }
        }
        if in_table {
            s.push_str("\\intbl");
        }
        s.push(' ');
        // Visible list label for readers without list support.
        if let Some(l) = r.list {
            if let Some((label, font)) = self.counter.next_label(self.doc, l) {
                let cf = self.char_fmt(&r.style, &p.mark);
                let font = font.map(|f| format!("\\f{}", self.font(&f))).unwrap_or_default();
                let _ = write!(s, "{{\\listtext\\pard\\plain {}{} ", cf, font);
                rtf_text(&mut s, &label);
                s.push_str("\\tab}");
            }
        }
        s
    }

    fn blocks(&mut self, out: &mut String, blocks: &[Block], body: bool) {
        let top = body && std::ptr::eq(blocks.as_ptr(), self.doc.body.as_ptr());
        for b in blocks {
            match b {
                Block::Para(p) => {
                    self.paragraph(out, p, false, body);
                    if top && p.section.is_some() {
                        // The section break takes the place of the paragraph mark.
                        self.sect_idx += 1;
                        out.push_str("\\sect\n");
                        if let Some(h) = self.sect_heads.get(self.sect_idx) {
                            out.push_str(h);
                        }
                    } else {
                        out.push_str("\\par\n");
                    }
                }
                Block::Table(t) => self.table(out, t, body),
            }
        }
    }

    fn paragraph(&mut self, out: &mut String, p: &Paragraph, in_table: bool, body: bool) {
        self.page_cont = None;
        if !in_table && p.runs.iter().any(|r| r.content == Inline::PageBreak && r.props.link.is_none()) {
            let mut cont = p.clone();
            cont.props.page_break_before = Some(true);
            cont.props.list = Some(None);
            cont.runs.clear();
            self.page_cont = Some(self.para_fmt(&cont, false));
        }
        let pf = self.para_fmt(p, in_table);
        out.push_str(&pf);
        let style = p.style_id().to_string();
        let mut i = 0;
        while i < p.runs.len() {
            // Group consecutive runs sharing a hyperlink into one HYPERLINK field.
            if let Some(link) = &p.runs[i].props.link {
                let mut j = i;
                while j < p.runs.len() && p.runs[j].props.link.as_ref() == Some(link) {
                    j += 1;
                }
                let instr = match link.strip_prefix('#') {
                    Some(a) => format!("HYPERLINK \\l \"{}\"", a),
                    None => format!("HYPERLINK \"{}\"", link),
                };
                out.push_str("{\\field{\\*\\fldinst {");
                rtf_text(out, &instr);
                out.push_str("}}{\\fldrslt ");
                for k in i..j {
                    self.run(out, &style, &p.runs[k], body);
                }
                out.push_str("}}");
                i = j;
            } else {
                self.run(out, &style, &p.runs[i], body);
                i += 1;
            }
        }
    }

    fn run(&mut self, out: &mut String, style: &str, r: &Run, body: bool) {
        let idx = self.run_counter;
        if body {
            self.run_counter += 1;
            let mut starts: Vec<u32> = self.comment_ranges.iter().filter(|(_, v)| v.0 == idx).map(|(k, _)| *k).collect();
            starts.sort_unstable();
            for c in starts {
                let _ = write!(out, "{{\\*\\atrfstart {}}}", c);
            }
        }
        let fmt = self.char_fmt(style, &r.props);
        match &r.content {
            Inline::Text(t) => {
                let _ = write!(out, "{{{} ", fmt);
                rtf_text(out, t);
                out.push('}');
            }
            Inline::Tab => {
                let _ = write!(out, "{{{}\\tab}}", fmt);
            }
            Inline::LineBreak => {
                let _ = write!(out, "{{{}\\line}}", fmt);
            }
            Inline::PageBreak => match (&self.page_cont, &r.props.link) {
                (Some(cont), None) => {
                    out.push_str("\\par\n");
                    out.push_str(cont);
                }
                _ => out.push_str("\\page "),
            },
            Inline::ColumnBreak => out.push_str("\\column "),
            Inline::Bookmark(n) => {
                out.push_str("{\\*\\bkmkstart ");
                rtf_text(out, n);
                out.push_str("}{\\*\\bkmkend ");
                rtf_text(out, n);
                out.push('}');
            }
            Inline::Field(f) => {
                let (instr, result) = match f {
                    Field::PageNumber => ("PAGE".to_string(), "1".to_string()),
                    Field::PageCount => ("NUMPAGES".to_string(), "1".to_string()),
                    Field::Date(pic) => (format!("DATE \\\\@ \"{}\"", pic), format_date_now(pic)),
                    Field::Other { instr, result } => (instr.replace('\\', "\\\\"), result.clone()),
                };
                let _ = write!(out, "{{\\field{{\\*\\fldinst {{{} ", fmt);
                // `instr` is already escaped for backslashes; escape braces and non-ASCII only.
                for c in instr.chars() {
                    match c {
                        '{' => out.push_str("\\{"),
                        '}' => out.push_str("\\}"),
                        c if c.is_ascii() => out.push(c),
                        c => rtf_text(out, &c.to_string()),
                    }
                }
                let _ = write!(out, "}}}}{{\\fldrslt {{{} ", fmt);
                rtf_text(out, &result);
                out.push_str("}}}");
            }
            Inline::Image(img) => match (pict_rtf(img), &img.float) {
                (None, _) => {
                    // No RTF encoding for this format: keep the alt text.
                    let _ = write!(out, "{{{} ", fmt);
                    rtf_text(out, &format!("[{}]", if img.alt.is_empty() { "image" } else { &img.alt }));
                    out.push('}');
                }
                (Some(pict), None) => {
                    let _ = write!(out, "{{\\*\\shppict{}}}", pict);
                }
                (Some(pict), Some(f)) => {
                    let mut extra = format!("{{\\sp{{\\sn shapeType}}{{\\sv 75}}}}{{\\sp{{\\sn pib}}{{\\sv {}}}}}", pict);
                    if !img.alt.is_empty() {
                        extra.push_str("{\\sp{\\sn wzDescription}{\\sv ");
                        rtf_text(&mut extra, &img.alt);
                        extra.push_str("}}");
                    }
                    self.shape(out, Some(f), img.width, img.height, &extra, "");
                    // Readers without shape support show the picture in line.
                    let _ = write!(out, "{{\\shprslt{{\\*\\shppict{}}}}}}}", pict);
                }
            },
            Inline::TextBox(tb) => {
                let mut extra = String::from("{\\sp{\\sn shapeType}{\\sv 202}}{\\sp{\\sn fTextBox}{\\sv 1}}");
                match tb.fill {
                    Some(c) => {
                        let _ = write!(extra, "{{\\sp{{\\sn fillColor}}{{\\sv {}}}}}{{\\sp{{\\sn fFilled}}{{\\sv 1}}}}", bgr(c));
                    }
                    None => extra.push_str("{\\sp{\\sn fFilled}{\\sv 0}}"),
                }
                match tb.border.filter(|b| b.is_visible()) {
                    Some(b) => {
                        let _ = write!(
                            extra,
                            "{{\\sp{{\\sn lineColor}}{{\\sv {}}}}}{{\\sp{{\\sn lineWidth}}{{\\sv {}}}}}{{\\sp{{\\sn fLine}}{{\\sv 1}}}}",
                            bgr(b.color),
                            emu(b.width)
                        );
                    }
                    None => extra.push_str("{\\sp{\\sn fLine}{\\sv 0}}"),
                }
                let ins = emu(tb.padding.max(0.0));
                let _ = write!(
                    extra,
                    "{{\\sp{{\\sn dxTextLeft}}{{\\sv {ins}}}}}{{\\sp{{\\sn dxTextRight}}{{\\sv {ins}}}}}{{\\sp{{\\sn dyTextTop}}{{\\sv {}}}}}{{\\sp{{\\sn dyTextBottom}}{{\\sv {}}}}}",
                    emu(tb.padding / 2.0),
                    emu(tb.padding / 2.0)
                );
                if tb.auto_height {
                    extra.push_str("{\\sp{\\sn fFitShapeToText}{\\sv 1}}");
                }
                let mut text = String::from("{\\shptxt ");
                let paras: Vec<Paragraph> = if tb.paragraphs.is_empty() { vec![Paragraph::new()] } else { tb.paragraphs.clone() };
                for (i, p) in paras.iter().enumerate() {
                    if i > 0 {
                        text.push_str("\\par\n");
                    }
                    self.paragraph(&mut text, p, false, false);
                }
                text.push('}');
                self.shape(out, tb.float.as_ref(), tb.width, tb.height, &extra, &text);
                out.push('}');
            }
            Inline::FootnoteRef(id) | Inline::EndnoteRef(id) => {
                let foot = matches!(r.content, Inline::FootnoteRef(_));
                let notes = if foot { &self.doc.footnotes } else { &self.doc.endnotes };
                let Some(note) = notes.iter().find(|n| n.id == *id).cloned() else { return };
                let _ = write!(out, "{{{}\\super\\chftn}}{{\\footnote{} ", fmt, if foot { "" } else { "\\ftnalt" });
                let n = note.paragraphs.len();
                for (k, p) in note.paragraphs.iter().enumerate() {
                    let mut p = p.clone();
                    if p.props.style.is_none() {
                        p.props.style = Some(if foot { "FootnoteText" } else { "EndnoteText" }.into());
                    }
                    let style = p.style_id().to_string();
                    let pf = self.para_fmt(&p, false);
                    out.push_str(&pf);
                    if k == 0 {
                        let f = self.char_fmt(&style, &CharProps::default());
                        let _ = write!(out, "{{{}\\super\\chftn}}{{{} }}", f, f);
                    }
                    for r in &p.runs {
                        self.run(out, &style, r, false);
                    }
                    if k + 1 < n {
                        out.push_str("\\par\n");
                    }
                }
                out.push('}');
            }
        }
        if body {
            let mut ends: Vec<u32> = self.comment_ranges.iter().filter(|(_, v)| v.1 == idx).map(|(k, _)| *k).collect();
            ends.sort_unstable();
            for c in ends {
                let _ = write!(out, "{{\\*\\atrfend {}}}", c);
                self.annotation(out, c);
            }
        }
    }

    fn annotation(&mut self, out: &mut String, id: u32) {
        let Some(c) = self.doc.comments.iter().find(|c| c.id == id).cloned() else { return };
        out.push_str("{\\*\\atnid ");
        rtf_text(out, &c.initials);
        out.push_str("}{\\*\\atnauthor ");
        rtf_text(out, &c.author);
        let _ = write!(out, "}}\\chatn{{\\*\\annotation{{\\*\\atnref {}}}{{\\*\\atndate {}}}", id, dttm(&c.date));
        let n = c.paragraphs.len();
        for (k, p) in c.paragraphs.iter().enumerate() {
            let pf = self.para_fmt(p, false);
            out.push_str(&pf);
            let style = p.style_id().to_string();
            for r in &p.runs {
                self.run(out, &style, r, false);
            }
            if k + 1 < n {
                out.push_str("\\par ");
            }
        }
        out.push('}');
    }

    fn table(&mut self, out: &mut String, t: &Table, body: bool) {
        let ncols = t.rows.iter().map(|r| r.cells.iter().map(|c| c.span.max(1) as usize).sum::<usize>()).max().unwrap_or(1).max(1);
        let mut cols = t.columns.clone();
        cols.resize(ncols, if cols.is_empty() { 72.0 } else { cols.iter().sum::<f32>() / cols.len() as f32 });
        let nrows = t.rows.len();
        for (ri, row) in t.rows.iter().enumerate() {
            let mut def = format!("\\trowd\\trgaph{}\\trleft{}", tw(t.cell_margin), tw(t.indent - t.cell_margin));
            def.push_str(match t.align {
                Align::Center => "\\trqc",
                Align::Right => "\\trqr",
                _ => "\\trql",
            });
            if let Some(h) = row.height {
                let _ = write!(def, "\\trrh{}", tw(h));
            }
            if row.header {
                def.push_str("\\trhdr");
            }
            let tb = t.borders.clone();
            for (tag, b) in [("trbrdrt", &tb.top), ("trbrdrl", &tb.left), ("trbrdrb", &tb.bottom), ("trbrdrr", &tb.right), ("trbrdrh", &tb.between), ("trbrdrv", &tb.inside_v)] {
                self.border(&mut def, tag, b);
            }
            let mut x = t.indent - t.cell_margin;
            let mut gc = 0usize;
            for cell in &row.cells {
                let span = cell.span.max(1) as usize;
                let w: f32 = cols.iter().skip(gc).take(span).sum();
                let last_row = ri + 1 == nrows;
                let mut b = Borders {
                    top: if ri == 0 { tb.top } else { tb.between },
                    bottom: if last_row { tb.bottom } else { tb.between },
                    left: if gc == 0 { tb.left } else { tb.inside_v },
                    right: if gc + span >= ncols { tb.right } else { tb.inside_v },
                    ..Default::default()
                };
                if let Some(cb) = &cell.borders {
                    b.top = cb.top.or(b.top);
                    b.bottom = cb.bottom.or(b.bottom);
                    b.left = cb.left.or(b.left);
                    b.right = cb.right.or(b.right);
                }
                gc += span;
                match cell.vmerge {
                    VMerge::Restart => def.push_str("\\clvmgf"),
                    VMerge::Continue => def.push_str("\\clvmrg"),
                    VMerge::None => {}
                }
                def.push_str(match cell.valign {
                    VAlign::Top => "\\clvertalt",
                    VAlign::Center => "\\clvertalc",
                    VAlign::Bottom => "\\clvertalb",
                });
                for (tag, br) in [("clbrdrt", &b.top), ("clbrdrl", &b.left), ("clbrdrb", &b.bottom), ("clbrdrr", &b.right)] {
                    self.border(&mut def, tag, br);
                }
                let shading = cell.shading.or(if ri == 0 { t.header_shading } else { None }).or(if ri % 2 == 1 { t.band_shading } else { None });
                if let Some(c) = shading {
                    let ci = self.color(c);
                    let _ = write!(def, "\\clcbpat{}", ci);
                }
                x += w;
                let _ = write!(def, "\\cellx{}", tw(x));
            }
            out.push_str(&def);
            out.push('\n');
            for cell in &row.cells {
                let paras: Vec<Paragraph> = if cell.paragraphs.is_empty() { vec![Paragraph::new()] } else { cell.paragraphs.clone() };
                let n = paras.len();
                for (k, p) in paras.iter().enumerate() {
                    self.paragraph(out, p, true, body);
                    out.push_str(if k + 1 < n { "\\par\n" } else { "\\cell\n" });
                }
            }
            out.push_str(&def);
            out.push_str("\\row\n");
        }
        out.push_str("\\pard\n");
    }

    fn list_table(&mut self) -> String {
        if self.doc.lists.is_empty() {
            return String::new();
        }
        let mut s = String::from("{\\*\\listtable\n");
        let mut ov = String::from("{\\*\\listoverridetable\n");
        let lists = self.doc.lists.clone();
        for (i, l) in lists.iter().enumerate() {
            let listid = 1000 + l.id;
            let _ = write!(s, "{{\\list\\listtemplateid{}\\listhybrid", listid);
            for lv in l.levels.iter().take(9) {
                let nfc = match lv.format {
                    NumFormat::Decimal => 0,
                    NumFormat::UpperRoman => 1,
                    NumFormat::LowerRoman => 2,
                    NumFormat::UpperLetter => 3,
                    NumFormat::LowerLetter => 4,
                    NumFormat::Bullet => 23,
                    NumFormat::DecimalZero => 22,
                    NumFormat::None => 255,
                };
                let _ = write!(s, "{{\\listlevel\\levelnfc{nfc}\\levelnfcn{nfc}\\leveljc0\\leveljcn0\\levelfollow0\\levelstartat{}\\levelspace0\\levelindent0", lv.start);
                // leveltext: length byte, then text with \'0N placeholders.
                let mut body = String::new();
                let mut numbers = String::new();
                let mut len = 0;
                if lv.format == NumFormat::Bullet {
                    rtf_text(&mut body, &lv.text);
                    len = lv.text.chars().count();
                } else {
                    let mut it = lv.text.chars().peekable();
                    while let Some(c) = it.next() {
                        if c == '%' {
                            if let Some(d) = it.peek().and_then(|d| d.to_digit(10)) {
                                it.next();
                                len += 1;
                                let _ = write!(body, "\\'{:02x}", d.saturating_sub(1));
                                let _ = write!(numbers, "\\'{:02x}", len);
                                continue;
                            }
                        }
                        len += 1;
                        rtf_text(&mut body, &c.to_string());
                    }
                }
                let _ = write!(s, "{{\\leveltext\\'{:02x}{};}}{{\\levelnumbers{};}}", len.min(255), body, numbers);
                if let Some(f) = &lv.font {
                    let fi = self.font(f);
                    let _ = write!(s, "\\f{}", fi);
                }
                let _ = write!(s, "\\fi{}\\li{}\\lin{}}}", -tw(lv.hanging), tw(lv.indent), tw(lv.indent));
            }
            let _ = writeln!(s, "{{\\listname ;}}\\listid{}}}", listid);
            let _ = writeln!(ov, "{{\\listoverride\\listid{}\\listoverridecount0\\ls{}}}", listid, i + 1);
        }
        s.push_str("}\n");
        ov.push_str("}\n");
        s + &ov
    }
}

/// Encode an ISO 8601 date as an RTF DTTM value.
fn bgr(c: Color) -> u32 {
    c.r as u32 | (c.g as u32) << 8 | (c.b as u32) << 16
}

/// `{\pict …}` for an image (None when RTF cannot carry its format).
fn pict_rtf(img: &Image) -> Option<String> {
    let kind = match img.data.format.as_str() {
        "png" => "\\pngblip",
        "jpeg" | "jpg" => "\\jpegblip",
        "emf" => "\\emfblip",
        "wmf" => "\\wmetafile8",
        _ => return None,
    };
    let (pw, ph) = img.data.pixel_size.unwrap_or(((img.width / 0.75) as u32, (img.height / 0.75) as u32));
    let mut out = format!("{{\\pict{}\\picw{}\\pich{}\\picwgoal{}\\pichgoal{}\n", kind, pw, ph, tw(img.width), tw(img.height));
    for chunk in img.data.bytes.chunks(64) {
        for b in chunk {
            let _ = write!(out, "{:02x}", b);
        }
        out.push('\n');
    }
    out.push('}');
    Some(out)
}

impl<'a> RtfWriter<'a> {
    /// Open a `{\shp{\*\shpinst …}` group (the caller closes it). `float` None = in line with text.
    fn shape(&mut self, out: &mut String, float: Option<&Float>, w: f32, h: f32, props: &str, inner: &str) {
        self.shape_id += 1;
        let f = float.copied().unwrap_or(Float { wrap: WrapMode::InFront, h_rel: HRel::Character, v_rel: VRel::Line, ..Default::default() });
        let (x, y) = (if f.h_align.is_some() { 0.0 } else { f.x }, if f.v_align.is_some() { 0.0 } else { f.y });
        let _ = write!(out, "{{\\shp{{\\*\\shpinst\\shpleft{}\\shptop{}\\shpright{}\\shpbottom{}\\shpfhdr0", tw(x), tw(y), tw(x + w), tw(y + h));
        // Like Word: the legacy anchor keyword, then "ignore" so the posrelh/posrelv properties apply.
        out.push_str(match f.h_rel {
            HRel::Page => "\\shpbxpage\\shpbxignore",
            HRel::Margin => "\\shpbxmargin\\shpbxignore",
            HRel::Column | HRel::Character => "\\shpbxcolumn\\shpbxignore",
        });
        out.push_str(match f.v_rel {
            VRel::Page => "\\shpbypage\\shpbyignore",
            VRel::Margin => "\\shpbymargin\\shpbyignore",
            VRel::Paragraph | VRel::Line => "\\shpbypara\\shpbyignore",
        });
        let (wr, behind) = match f.wrap {
            WrapMode::Square => (2, 0),
            WrapMode::Tight => (4, 0),
            WrapMode::TopBottom => (1, 0),
            WrapMode::Behind => (3, 1),
            WrapMode::InFront => (3, 0),
        };
        let _ = write!(out, "\\shpwr{}\\shpwrk0\\shpfblwtxt{}\\shpz{}\\shplid{}", wr, behind, self.shape_id, 1024 + self.shape_id);
        let posrelh = match f.h_rel {
            HRel::Margin => 0,
            HRel::Page => 1,
            HRel::Column => 2,
            HRel::Character => 3,
        };
        let posrelv = match f.v_rel {
            VRel::Margin => 0,
            VRel::Page => 1,
            VRel::Paragraph => 2,
            VRel::Line => 3,
        };
        let _ = write!(out, "{{\\sp{{\\sn posrelh}}{{\\sv {}}}}}{{\\sp{{\\sn posrelv}}{{\\sv {}}}}}", posrelh, posrelv);
        let posh = match f.h_align {
            None => 0,
            Some(FloatAlign::Start) => 1,
            Some(FloatAlign::Center) => 2,
            Some(FloatAlign::End) => 3,
        };
        let posv = match f.v_align {
            None => 0,
            Some(FloatAlign::Start) => 1,
            Some(FloatAlign::Center) => 2,
            Some(FloatAlign::End) => 3,
        };
        let _ = write!(out, "{{\\sp{{\\sn posh}}{{\\sv {}}}}}{{\\sp{{\\sn posv}}{{\\sv {}}}}}", posh, posv);
        if behind == 1 {
            out.push_str("{\\sp{\\sn fBehindDocument}{\\sv 1}}");
        }
        if float.is_none() {
            out.push_str("{\\sp{\\sn fPseudoInline}{\\sv 1}}");
        }
        let d = emu(f.dist.max(0.0));
        for n in ["dxWrapDistLeft", "dxWrapDistRight", "dyWrapDistTop", "dyWrapDistBottom"] {
            let _ = write!(out, "{{\\sp{{\\sn {}}}{{\\sv {}}}}}", n, d);
        }
        out.push_str(props);
        out.push_str(inner);
        out.push('}');
    }

    /// `\sectd` and the section properties of a page setup.
    fn sect_props(&mut self, p: &PageSetup) -> String {
        let mut s = String::from("\\sectd");
        s.push_str(match p.start {
            SectionStart::NextPage => "\\sbkpage",
            SectionStart::Continuous => "\\sbknone",
            SectionStart::OddPage => "\\sbkodd",
            SectionStart::EvenPage => "\\sbkeven",
        });
        if p.width > p.height {
            s.push_str("\\lndscpsxn");
        }
        let _ = write!(
            s,
            "\\pgwsxn{}\\pghsxn{}\\marglsxn{}\\margrsxn{}\\margtsxn{}\\margbsxn{}\\headery{}\\footery{}",
            tw(p.width),
            tw(p.height),
            tw(p.margin_left),
            tw(p.margin_right),
            tw(p.margin_top),
            tw(p.margin_bottom),
            tw(p.header_distance),
            tw(p.footer_distance)
        );
        if p.columns > 1 {
            let _ = write!(s, "\\cols{}\\colsx{}", p.columns, tw(p.column_gap));
        }
        if p.title_page {
            s.push_str("\\titlepg");
        }
        if p.page_number_start != 1 {
            let _ = write!(s, "\\pgnrestart\\pgnstarts{}", p.page_number_start);
        }
        if let Some(b) = p.page_borders.clone() {
            for (tag, br) in [("pgbrdrt", &b.top), ("pgbrdrl", &b.left), ("pgbrdrb", &b.bottom), ("pgbrdrr", &b.right)] {
                self.border(&mut s, tag, br);
            }
        }
        s.push('\n');
        s
    }
}

fn dttm(date: &str) -> u32 {
    let n = |a: usize, b: usize| date.get(a..b).and_then(|s| s.parse::<u32>().ok());
    let (Some(y), Some(mo), Some(d)) = (n(0, 4), n(5, 7), n(8, 10)) else { return 0 };
    let h = n(11, 13).unwrap_or(0);
    let mi = n(14, 16).unwrap_or(0);
    (mi & 63) | (h & 31) << 6 | (d & 31) << 11 | (mo & 15) << 16 | (y.saturating_sub(1900) & 511) << 20
}

fn dttm_to_iso(v: i64) -> String {
    if v <= 0 {
        return String::new();
    }
    let v = v as u32;
    let mi = v & 63;
    let h = (v >> 6) & 31;
    let d = (v >> 11) & 31;
    let mo = (v >> 16) & 15;
    let y = ((v >> 20) & 511) + 1900;
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:00Z", y, mo, d, h, mi)
}

/// Serialise a document as RTF.
pub fn write(doc: &Document) -> String {
    let mut w = RtfWriter {
        page_cont: None,
        doc,
        fonts: Vec::new(),
        colors: Vec::new(),
        styles: vec!["Normal".into()],
        authors: vec!["Unknown".into()],
        list_ls: HashMap::new(),
        counter: ListCounter::default(),
        comment_ranges: HashMap::new(),
        run_counter: 0,
        sect_heads: Vec::new(),
        sect_idx: 0,
        shape_id: 0,
    };
    let deff = doc.default_chars.font.clone().unwrap_or_else(|| "Calibri".into());
    w.font(&deff);
    for (i, l) in doc.lists.iter().enumerate() {
        w.list_ls.insert(l.id, i + 1);
    }
    let mut idx = 0usize;
    let mut ranges: HashMap<u32, (usize, usize)> = HashMap::new();
    for b in &doc.body {
        let paras: Vec<&Paragraph> = match b {
            Block::Para(p) => vec![p],
            Block::Table(t) => t.rows.iter().flat_map(|r| &r.cells).flat_map(|c| &c.paragraphs).collect(),
        };
        for p in paras {
            for r in &p.runs {
                for &c in &r.props.comments {
                    let e = ranges.entry(c).or_insert((idx, idx));
                    e.1 = idx;
                }
                idx += 1;
            }
        }
    }
    w.comment_ranges = ranges;

    // Section properties and headers/footers (a section without header groups inherits its
    // predecessor's, so groups are written where the displayed content changes; the first section
    // always gets what it shows).
    let lists = w.list_table();
    let secs = doc.sections();
    let nsec = secs.len() + 1;
    let mut prev: [Option<&Vec<Block>>; 4] = [None; 4];
    for i in 0..nsec {
        let (page, hf) = match secs.get(i) {
            Some(s) => (&s.page, s.headers.as_ref().or(if i == 0 { Some(doc.section_headers(0)) } else { None })),
            None => (&doc.page, Some(&doc.headers)),
        };
        let mut head = w.sect_props(page);
        if let Some(hf) = hf {
            let parts = [("header", HfKind::Header), ("footer", HfKind::Footer), ("headerf", HfKind::FirstHeader), ("footerf", HfKind::FirstFooter)];
            for (k, (tag, kind)) in parts.into_iter().enumerate() {
                let blocks = hf.part(kind);
                let needed = match prev[k] {
                    Some(p) => p != blocks,
                    None => !blocks.is_empty(),
                };
                if !needed {
                    continue;
                }
                prev[k] = Some(blocks);
                let mut s = String::new();
                w.blocks(&mut s, blocks, false);
                let _ = writeln!(head, "{{\\{} {}}}", tag, s);
            }
        }
        w.sect_heads.push(head);
    }
    // Body (fills the font / colour / style tables).
    let mut body = String::new();
    w.blocks(&mut body, &doc.body, true);

    let mut out = String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0\\uc1\n");
    out.push_str("{\\fonttbl");
    for (i, f) in w.fonts.iter().enumerate() {
        let _ = write!(out, "{{\\f{}\\fnil\\fcharset0 ", i);
        rtf_text(&mut out, f);
        out.push_str(";}");
    }
    out.push_str("}\n{\\colortbl;");
    for c in &w.colors {
        let _ = write!(out, "\\red{}\\green{}\\blue{};", c.r, c.g, c.b);
    }
    out.push_str("}\n");
    // Stylesheet (formatting resolved so other readers show the style faithfully).
    out.push_str("{\\stylesheet");
    let styles = w.styles.clone();
    for (i, id) in styles.iter().enumerate() {
        let name = doc.style(id).map(|s| crate::io::docx::docx_style_name(&s.id, &s.name)).unwrap_or_else(|| id.clone());
        let p = Paragraph { props: ParaProps { style: Some(id.clone()), ..Default::default() }, ..Default::default() };
        let pf = w.para_fmt(&p, false).replace("\\pard\\plain", "");
        let cf = w.char_fmt(id, &CharProps::default());
        let based = doc
            .style(id)
            .and_then(|s| s.based_on.as_ref())
            .and_then(|b| styles.iter().position(|x| x == b))
            .map(|b| format!("\\sbasedon{b}"))
            .unwrap_or_default();
        let _ = write!(out, "{{{}{}{} ", pf.trim_end(), based, cf);
        rtf_text(&mut out, &name);
        out.push_str(";}");
        let _ = i;
    }
    out.push_str("}\n");
    out.push_str(&lists);
    if w.authors.len() > 1 {
        out.push_str("{\\*\\revtbl ");
        for a in &w.authors {
            out.push('{');
            rtf_text(&mut out, a);
            out.push_str(";}");
        }
        out.push_str("}\n");
    }
    let p = &doc.properties;
    out.push_str("{\\info");
    for (tag, v) in [("title", &p.title), ("subject", &p.subject), ("author", &p.author), ("keywords", &p.keywords), ("doccomm", &p.description), ("operator", &p.last_modified_by)] {
        if !v.is_empty() {
            let _ = write!(out, "{{\\{} ", tag);
            rtf_text(&mut out, v);
            out.push('}');
        }
    }
    for (tag, v) in [("creatim", &p.created), ("revtim", &p.modified)] {
        let n = |a: usize, b: usize| v.get(a..b).and_then(|s| s.parse::<u32>().ok());
        if let (Some(y), Some(mo), Some(d)) = (n(0, 4), n(5, 7), n(8, 10)) {
            let _ = write!(out, "{{\\{}\\yr{}\\mo{}\\dy{}\\hr{}\\min{}}}", tag, y, mo, d, n(11, 13).unwrap_or(0), n(14, 16).unwrap_or(0));
        }
    }
    out.push_str("}\n");
    // Document defaults: the first section's page.
    let pg = doc.section_page(0);
    let _ = write!(
        out,
        "\\paperw{}\\paperh{}\\margl{}\\margr{}\\margt{}\\margb{}\\deftab{}",
        tw(pg.width),
        tw(pg.height),
        tw(pg.margin_left),
        tw(pg.margin_right),
        tw(pg.margin_top),
        tw(pg.margin_bottom),
        tw(doc.settings.default_tab)
    );
    if pg.width > pg.height {
        out.push_str("\\landscape");
    }
    if doc.settings.track_changes {
        out.push_str("\\revisions");
    }
    out.push_str("\\fet2\\ftnbj\\aenddoc\n");
    out.push_str(&w.sect_heads[0]);
    out.push_str(&body);
    out.push('}');
    // Colours or fonts added while writing the stylesheet/page borders: rebuild tables if they grew.
    if out.contains("\\colortbl;") {
        let mut table = String::from("{\\colortbl;");
        for c in &w.colors {
            let _ = write!(table, "\\red{}\\green{}\\blue{};", c.r, c.g, c.b);
        }
        table.push('}');
        let start = out.find("{\\colortbl;").unwrap();
        let end = start + out[start..].find('}').unwrap() + 1;
        out.replace_range(start..end, &table);
        let mut ft = String::from("{\\fonttbl");
        for (i, f) in w.fonts.iter().enumerate() {
            let _ = write!(ft, "{{\\f{}\\fnil\\fcharset0 ", i);
            rtf_text(&mut ft, f);
            ft.push_str(";}");
        }
        ft.push('}');
        let start = out.find("{\\fonttbl").unwrap();
        let end = start + out[start..].find("}\n").unwrap() + 1;
        out.replace_range(start..end, &ft);
    }
    out
}

// ======================================================================================
// Reader
// ======================================================================================

#[derive(Debug)]
enum Tok {
    Open,
    Close,
    Word(String, Option<i32>),
    /// `\'hh`
    Hex(u8),
    /// Other control symbols (`\~`, `\-`, `\*` …).
    Sym(char),
    Text(String),
    Bin(Vec<u8>),
}

fn tokenize(b: &[u8]) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut text = Vec::new();
    let flush = |text: &mut Vec<u8>, out: &mut Vec<Tok>| {
        if !text.is_empty() {
            out.push(Tok::Text(cp1252_decode(text)));
            text.clear();
        }
    };
    while i < b.len() {
        let c = b[i];
        match c {
            b'{' => {
                flush(&mut text, &mut out);
                out.push(Tok::Open);
                i += 1;
            }
            b'}' => {
                flush(&mut text, &mut out);
                out.push(Tok::Close);
                i += 1;
            }
            b'\\' => {
                flush(&mut text, &mut out);
                i += 1;
                if i >= b.len() {
                    break;
                }
                let c = b[i];
                if c.is_ascii_alphabetic() {
                    let start = i;
                    while i < b.len() && b[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    let word = String::from_utf8_lossy(&b[start..i]).into_owned();
                    let mut param = None;
                    let ns = i;
                    if i < b.len() && (b[i] == b'-' || b[i].is_ascii_digit()) {
                        i += 1;
                        while i < b.len() && b[i].is_ascii_digit() {
                            i += 1;
                        }
                        param = std::str::from_utf8(&b[ns..i]).ok().and_then(|s| s.parse::<i64>().ok()).map(|v| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
                    }
                    if i < b.len() && b[i] == b' ' {
                        i += 1;
                    }
                    if word == "bin" {
                        let n = param.unwrap_or(0).max(0) as usize;
                        let end = (i + n).min(b.len());
                        out.push(Tok::Bin(b[i..end].to_vec()));
                        i = end;
                        continue;
                    }
                    out.push(Tok::Word(word, param));
                } else if c == b'\'' {
                    let h = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
                    match h {
                        Some(v) => {
                            out.push(Tok::Hex(v));
                            i += 3;
                        }
                        None => i += 1,
                    }
                } else if c == b'\n' || c == b'\r' {
                    out.push(Tok::Word("par".into(), None));
                    i += 1;
                } else {
                    match c {
                        b'\\' | b'{' | b'}' => text.push(c),
                        _ => out.push(Tok::Sym(c as char)),
                    }
                    i += 1;
                }
            }
            b'\r' | b'\n' => i += 1,
            _ => {
                text.push(c);
                i += 1;
            }
        }
    }
    flush(&mut text, &mut out);
    out
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Dest {
    Normal,
    Skip,
    FontTable,
    FontEntry,
    ColorTable,
    Stylesheet,
    StyleEntry,
    Info(InfoField),
    Pict,
    FldInst,
    FldRslt,
    ListTable,
    List,
    ListLevel,
    LevelText,
    LevelNumbers,
    ListOverrideTable,
    ListOverride,
    BkmkStart,
    BkmkEnd,
    AtnId,
    AtnAuthor,
    AtnRef,
    AtnDate,
    AtrfStart,
    AtrfEnd,
    RevTbl,
    /// Inside a `\shp` / `\shpinst`.
    Shape,
    /// A shape property `{\sp …}`, its name and value.
    Sp,
    Sn,
    Sv,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum InfoField {
    Title,
    Subject,
    Author,
    Keywords,
    Comment,
    Operator,
    Created,
    Revised,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum BorderTarget {
    None,
    Para(u8),
    Cell(u8),
    Row(u8),
    Page(u8),
}

#[derive(Clone)]
struct GState {
    dest: Dest,
    chars: CharProps,
    para: ParaProps,
    uc: usize,
    in_table: bool,
    /// This group opened a story (footnote/header/annotation) that must be closed with it.
    story: Option<StoryKind>,
    /// This group is a `\field`.
    field: bool,
    /// This group is a `\shp`.
    shape: bool,
    font_charset_symbol: bool,
    list_level_ref: Option<ListRef>,
    rev_author: Option<usize>,
    rev_date: i64,
    rev_kind: Option<RevisionKind>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum StoryKind {
    Footnote,
    Endnote,
    Header,
    Footer,
    FirstHeader,
    FirstFooter,
    Annotation,
    /// The text of a text box shape (`\shptxt`).
    ShapeText,
    Skip,
}

#[derive(Default)]
struct Story {
    blocks: Vec<Block>,
    para: Paragraph,
    /// Pending table rows / cells.
    table: Option<Table>,
    row_cells: Vec<Cell>,
    cell_paras: Vec<Paragraph>,
    row_defs: Vec<(f32, Cell, Borders)>,
    row_props: Row,
    row_align: Align,
    row_left: f32,
    row_gap: f32,
    row_borders: Borders,
}

#[derive(Clone, Default)]
struct FieldCtx {
    instr: String,
    result: String,
    swallow: bool,
    props: Option<CharProps>,
    link: Option<String>,
    in_result: bool,
}

#[derive(Default, Clone)]
struct LevelBuild {
    nfc: i32,
    start: u32,
    text_raw: Vec<u32>,
    li: f32,
    fi: f32,
    font: Option<usize>,
}

struct RtfReader {
    doc: Document,
    fonts: HashMap<i32, String>,
    colors: Vec<Option<Color>>,
    color_cur: (u8, u8, u8, bool),
    styles: HashMap<i32, String>,
    style_cur: (i32, String),
    font_cur: (i32, String),
    stack: Vec<GState>,
    stories: Vec<(StoryKind, Story)>,
    fields: Vec<FieldCtx>,
    pict_hex: String,
    pict_kind: String,
    pict_size: (f32, f32),
    pict_scale: (f32, f32),
    pict_px: (f32, f32),
    info_text: String,
    info_date: (u32, u32, u32, u32, u32),
    border: BorderTarget,
    pending_borders: (Borders, Borders, Borders, Borders),
    cell_shading: Option<Color>,
    cell_vmerge: VMerge,
    cell_valign: VAlign,
    cell_hmerge: bool,
    lists: HashMap<i32, ListDef>,
    list_cur: (i32, Vec<ListLevel>),
    level_cur: LevelBuild,
    override_cur: (i32, i32),
    ls_map: HashMap<i32, u32>,
    bookmark: String,
    atn: (String, String, String, String),
    atn_ranges: HashMap<String, u32>,
    active_comments: Vec<u32>,
    authors: Vec<String>,
    author_cur: String,
    skip_chars: usize,
    deff: i32,
    images: HashMap<u64, Arc<ImageData>>,
    pending_tab: (TabAlign, TabLeader),
    style_based: Option<i32>,
    style_based_pending: Vec<(String, i32)>,
    /// Document-level page defaults (`\paperw` …) that `\sectd` restores; `doc.page` is the current section's.
    def_page: PageSetup,
    landscape: bool,
    /// The current section has header/footer groups of its own.
    own_headers: bool,
    /// Finished sections: page setup, effective headers, own headers?
    sections: Vec<(PageSetup, HeadersFooters, bool)>,
    last_was_par: bool,
    shape: Option<ShapeBuild>,
}

/// A `\shp` being read.
#[derive(Default)]
struct ShapeBuild {
    rect: [i32; 4],
    bx: Option<HRel>,
    by: Option<VRel>,
    wr: Option<i32>,
    behind: bool,
    props: HashMap<String, String>,
    sn: String,
    sv: String,
    pict: Option<Image>,
    text: Option<Vec<Paragraph>>,
}

/// Read an RTF document.
pub fn read(bytes: &[u8]) -> Result<Document, String> {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(16)]);
    if !head.trim_start().starts_with("{\\rtf") {
        return Err("Not an RTF document".into());
    }
    let toks = tokenize(bytes);
    let mut doc = Document::new();
    doc.body.clear();
    doc.default_chars.size = Some(12.0);
    doc.default_para.space_after = Some(0.0);
    doc.default_para.line_spacing = Some(LineSpacing::Multiple(1.0));
    let base = GState {
        dest: Dest::Normal,
        chars: CharProps::default(),
        para: ParaProps::default(),
        uc: 1,
        in_table: false,
        story: None,
        field: false,
        shape: false,
        font_charset_symbol: false,
        list_level_ref: None,
        rev_author: None,
        rev_date: 0,
        rev_kind: None,
    };
    let mut r = RtfReader {
        doc,
        fonts: HashMap::new(),
        colors: Vec::new(),
        color_cur: (0, 0, 0, false),
        styles: HashMap::new(),
        style_cur: (0, String::new()),
        font_cur: (0, String::new()),
        stack: vec![base],
        stories: vec![(StoryKind::Skip, Story::default())],
        fields: Vec::new(),
        pict_hex: String::new(),
        pict_kind: String::new(),
        pict_size: (0.0, 0.0),
        pict_scale: (100.0, 100.0),
        pict_px: (0.0, 0.0),
        info_text: String::new(),
        info_date: (0, 0, 0, 0, 0),
        border: BorderTarget::None,
        pending_borders: Default::default(),
        cell_shading: None,
        cell_vmerge: VMerge::None,
        cell_valign: VAlign::Top,
        cell_hmerge: false,
        lists: HashMap::new(),
        list_cur: (0, Vec::new()),
        level_cur: LevelBuild::default(),
        override_cur: (0, 0),
        ls_map: HashMap::new(),
        bookmark: String::new(),
        atn: Default::default(),
        atn_ranges: HashMap::new(),
        active_comments: Vec::new(),
        authors: Vec::new(),
        author_cur: String::new(),
        skip_chars: 0,
        deff: 0,
        images: HashMap::new(),
        pending_tab: (TabAlign::Left, TabLeader::None),
        style_based: None,
        style_based_pending: Vec::new(),
        def_page: PageSetup::default(),
        landscape: false,
        own_headers: false,
        sections: Vec::new(),
        last_was_par: false,
        shape: None,
    };
    let mut expect_dest = false;
    for t in toks {
        match t {
            Tok::Open => {
                let top = r.stack.last().cloned().unwrap();
                r.stack.push(GState { story: None, field: false, shape: false, ..top });
                r.skip_chars = 0;
            }
            Tok::Close => {
                if r.stack.len() <= 1 {
                    continue;
                }
                let st = r.stack.pop().unwrap();
                r.end_group(&st);
                expect_dest = false;
            }
            Tok::Sym('*') => {
                expect_dest = true;
                continue;
            }
            Tok::Word(w, p) => {
                let was_star = expect_dest;
                expect_dest = false;
                r.word(&w, p, was_star);
            }
            Tok::Hex(h) => r.hex(h),
            Tok::Sym(c) => {
                let s = match c {
                    '~' => "\u{00A0}",
                    '-' => "\u{00AD}",
                    '_' => "\u{2011}",
                    _ => "",
                };
                if !s.is_empty() {
                    r.text(s);
                }
            }
            Tok::Text(s) => r.text(&s),
            Tok::Bin(data) => {
                if r.st().dest == Dest::Pict {
                    for b in data {
                        r.pict_hex.push_str(&format!("{b:02x}"));
                    }
                }
            }
        }
    }
    // Flush the body.
    while r.stack.len() > 1 {
        let st = r.stack.pop().unwrap();
        r.end_group(&st);
    }
    let (_, mut story) = r.stories.pop().unwrap();
    if !story.para.runs.is_empty() || story.blocks.is_empty() {
        r.finish_para(&mut story);
    }
    r.flush_table(&mut story);
    for (id, b) in std::mem::take(&mut r.style_based_pending) {
        if let Some(base) = r.styles.get(&b).cloned() {
            if base != id {
                if let Some(s) = r.doc.styles.iter_mut().find(|s| s.id == id) {
                    s.based_on = Some(base);
                }
            }
        }
    }
    r.finish_sections(&mut story.blocks);
    let mut doc = r.doc;
    doc.body = story.blocks;
    if doc.body.is_empty() {
        doc.body.push(Block::Para(Paragraph::new()));
    }
    if let Some(f) = r.fonts.get(&r.deff) {
        doc.default_chars.font = Some(f.clone());
    }
    let mut lists: Vec<ListDef> = doc.lists.clone();
    lists.sort_by_key(|l| l.id);
    doc.lists = lists;
    Ok(doc)
}

impl RtfReader {
    fn st(&mut self) -> &mut GState {
        self.stack.last_mut().unwrap()
    }

    fn color(&self, i: i32) -> Option<Color> {
        if i <= 0 {
            return None;
        }
        self.colors.get(i as usize).copied().flatten()
    }

    fn story(&mut self) -> &mut Story {
        &mut self.stories.last_mut().unwrap().1
    }

    fn word(&mut self, w: &str, p: Option<i32>, star: bool) {
        let dest = self.st().dest;
        let v = p.unwrap_or(1);
        let on = p.map(|x| x != 0).unwrap_or(true);
        // Destinations.
        let new_dest = match w {
            "fonttbl" => Some(Dest::FontTable),
            "colortbl" => Some(Dest::ColorTable),
            "stylesheet" => Some(Dest::Stylesheet),
            "info" => Some(dest),
            "title" => Some(Dest::Info(InfoField::Title)),
            "subject" => Some(Dest::Info(InfoField::Subject)),
            "author" => Some(Dest::Info(InfoField::Author)),
            "keywords" => Some(Dest::Info(InfoField::Keywords)),
            "doccomm" => Some(Dest::Info(InfoField::Comment)),
            "operator" => Some(Dest::Info(InfoField::Operator)),
            "creatim" => Some(Dest::Info(InfoField::Created)),
            "revtim" => Some(Dest::Info(InfoField::Revised)),
            "pict" => Some(Dest::Pict),
            "fldinst" => Some(Dest::FldInst),
            "fldrslt" => Some(Dest::FldRslt),
            "listtable" => Some(Dest::ListTable),
            "list" if dest == Dest::ListTable => Some(Dest::List),
            "listlevel" => Some(Dest::ListLevel),
            "leveltext" => Some(Dest::LevelText),
            "levelnumbers" => Some(Dest::LevelNumbers),
            "listoverridetable" => Some(Dest::ListOverrideTable),
            "listoverride" => Some(Dest::ListOverride),
            "bkmkstart" => Some(Dest::BkmkStart),
            "bkmkend" => Some(Dest::BkmkEnd),
            "atnid" => Some(Dest::AtnId),
            "atnauthor" => Some(Dest::AtnAuthor),
            "atnref" => Some(Dest::AtnRef),
            "atndate" => Some(Dest::AtnDate),
            "atrfstart" => Some(Dest::AtrfStart),
            "atrfend" => Some(Dest::AtrfEnd),
            "revtbl" => Some(Dest::RevTbl),
            "shpinst" if self.shape.is_some() => Some(Dest::Shape),
            "sp" if self.shape.is_some() => Some(Dest::Sp),
            "sn" if self.shape.is_some() => Some(Dest::Sn),
            "sv" if self.shape.is_some() => Some(Dest::Sv),
            "shprslt" if self.shape.is_some() => Some(Dest::Skip),
            "shppict" => Some(dest),
            "nonshppict" | "listtext" | "pntext" | "pn" | "listname" | "xmlnstbl" | "generator" | "themedata" | "colorschememapping"
            | "latentstyles" | "datastore" | "rsidtbl" | "mmathPr" | "pgdsctbl" | "falt" | "panose" | "fname" | "filetbl" | "objdata"
            | "result" | "userprops" | "docvar" | "ftnsep" | "ftnsepc" | "aftnsep" | "aftnsepc" | "headerl" | "footerl" | "headerr"
            | "footerr" | "tc" | "xe" | "template" | "company" | "manager" | "category" | "comment" | "hlinkbase" | "nesttableprops"
            | "nonesttables" | "background" | "shpgrp" | "wgrffmtfilter" | "fchars" | "lchars"
            | "pgptbl" | "protusertbl" | "defchp" | "defpap" | "blipuid" | "picprop" | "bkmkcolf" | "bkmkcoll" | "atnicn" | "atntime"
            | "atnparent" | "formfield" | "ud" | "upr" | "do" => Some(Dest::Skip),
            _ => None,
        };
        if let Some(d) = new_dest {
            if dest == Dest::Skip {
                return;
            }
            // `\*\shppict` is processed; `\*\listtext` etc. skipped.
            self.st().dest = d;
            match w {
                "pict" => {
                    self.pict_hex.clear();
                    self.pict_kind.clear();
                    self.pict_size = (0.0, 0.0);
                    self.pict_scale = (100.0, 100.0);
                    self.pict_px = (0.0, 0.0);
                }
                "creatim" | "revtim" => self.info_date = (0, 0, 0, 0, 0),
                "title" | "subject" | "author" | "keywords" | "doccomm" | "operator" | "bkmkstart" | "bkmkend" | "atnid" | "atnauthor"
                | "atnref" | "atndate" | "atrfstart" | "atrfend" => {
                    self.info_text.clear();
                    self.bookmark.clear();
                }
                "fldinst" => {
                    if self.fields.is_empty() {
                        self.fields.push(FieldCtx::default());
                    }
                }
                "fldrslt" => {
                    if let Some(f) = self.fields.last_mut() {
                        f.in_result = true;
                        let kw = f.instr.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
                        f.swallow = !matches!(kw.as_str(), "HYPERLINK" | "TOC" | "INDEX" | "");
                        if kw == "HYPERLINK" {
                            f.link = rtf_hyperlink(&f.instr);
                        }
                    }
                }
                "sp" => {
                    if let Some(sh) = self.shape.as_mut() {
                        sh.sn.clear();
                        sh.sv.clear();
                    }
                }
                "list" => self.list_cur = (0, Vec::new()),
                "listlevel" => self.level_cur = LevelBuild { start: 1, ..Default::default() },
                "listoverride" => self.override_cur = (0, 0),
                "stylesheet" => {}
                _ => {}
            }
            return;
        }
        if star && !matches!(dest, Dest::Skip) {
            // Unknown ignorable destination.
            if !matches!(w, "shppict" | "footnote" | "annotation") {
                self.st().dest = Dest::Skip;
                return;
            }
        }
        if dest == Dest::Skip {
            return;
        }
        match dest {
            Dest::FontTable | Dest::FontEntry => {
                if w == "f" {
                    self.font_cur = (v, String::new());
                    self.st().dest = Dest::FontEntry;
                }
                if w == "fcharset" {
                    self.st().font_charset_symbol = v == 2;
                }
                return;
            }
            Dest::ColorTable => {
                match w {
                    "red" => self.color_cur = (v as u8, self.color_cur.1, self.color_cur.2, true),
                    "green" => self.color_cur = (self.color_cur.0, v as u8, self.color_cur.2, true),
                    "blue" => self.color_cur = (self.color_cur.0, self.color_cur.1, v as u8, true),
                    _ => {}
                }
                return;
            }
            Dest::Stylesheet | Dest::StyleEntry => {
                match w {
                    "s" | "cs" | "ds" | "ts" => {
                        self.style_cur = (if w == "s" { v } else { -1 - v.abs() }, String::new());
                        self.style_based = None;
                        let st = self.st();
                        st.dest = Dest::StyleEntry;
                        st.chars = CharProps::default();
                        st.para = ParaProps::default();
                        return;
                    }
                    "sbasedon" => {
                        self.style_based = Some(v);
                        return;
                    }
                    _ if dest == Dest::Stylesheet => return,
                    "snext" | "slink" | "additive" | "sautoupd" | "shidden" | "slocked" | "spersonal" | "scompose" | "sreply" | "styrsid"
                    | "ssemihidden" | "sqformat" | "spriority" | "sunhideused" | "par" | "tab" => return,
                    // Formatting words describe the style; fall through to the normal handling.
                    _ => {}
                }
            }
            Dest::Info(_) => {
                match w {
                    "yr" => self.info_date.0 = v as u32,
                    "mo" => self.info_date.1 = v as u32,
                    "dy" => self.info_date.2 = v as u32,
                    "hr" => self.info_date.3 = v as u32,
                    "min" => self.info_date.4 = v as u32,
                    _ => {}
                }
                return;
            }
            Dest::Pict => {
                match w {
                    "pngblip" => self.pict_kind = "png".into(),
                    "jpegblip" => self.pict_kind = "jpeg".into(),
                    "emfblip" => self.pict_kind = "emf".into(),
                    "wmetafile" => self.pict_kind = "wmf".into(),
                    "dibitmap" | "wbitmap" | "macpict" | "pmmetafile" => self.pict_kind = "unsupported".into(),
                    "picwgoal" => self.pict_size.0 = v as f32 / 20.0,
                    "pichgoal" => self.pict_size.1 = v as f32 / 20.0,
                    "picw" => self.pict_px.0 = v as f32,
                    "pich" => self.pict_px.1 = v as f32,
                    "picscalex" => self.pict_scale.0 = v as f32,
                    "picscaley" => self.pict_scale.1 = v as f32,
                    _ => {}
                }
                return;
            }
            Dest::List | Dest::ListLevel => {
                match w {
                    "listid" if dest == Dest::List => self.list_cur.0 = v,
                    "levelnfc" => self.level_cur.nfc = v,
                    "levelstartat" => self.level_cur.start = v.max(0) as u32,
                    "li" | "lin" => self.level_cur.li = v as f32 / 20.0,
                    "fi" => self.level_cur.fi = v as f32 / 20.0,
                    "f" => self.level_cur.font = Some(v as usize),
                    _ => {}
                }
                return;
            }
            Dest::ListOverride => {
                match w {
                    "listid" => self.override_cur.0 = v,
                    "ls" => self.override_cur.1 = v,
                    _ => {}
                }
                return;
            }
            Dest::LevelText | Dest::LevelNumbers => {
                if w == "u" {
                    self.level_cur.text_raw.push(v as u16 as u32);
                    self.skip_chars = self.st().uc;
                }
                return;
            }
            Dest::FldInst => {
                return;
            }
            Dest::Shape | Dest::Sp | Dest::Sn => {
                if let Some(sh) = self.shape.as_mut() {
                    match w {
                        "shpleft" => sh.rect[0] = v,
                        "shptop" => sh.rect[1] = v,
                        "shpright" => sh.rect[2] = v,
                        "shpbottom" => sh.rect[3] = v,
                        "shpbxpage" => sh.bx = Some(HRel::Page),
                        "shpbxmargin" => sh.bx = Some(HRel::Margin),
                        "shpbxcolumn" => sh.bx = Some(HRel::Column),
                        "shpbxignore" => sh.bx = None,
                        "shpbypage" => sh.by = Some(VRel::Page),
                        "shpbymargin" => sh.by = Some(VRel::Margin),
                        "shpbypara" => sh.by = Some(VRel::Paragraph),
                        "shpbyignore" => sh.by = None,
                        "shpwr" => sh.wr = Some(v),
                        "shpfblwtxt" => sh.behind = v != 0,
                        "shptxt" => {
                            self.open_story(StoryKind::ShapeText);
                        }
                        _ => {}
                    }
                }
                return;
            }
            _ => {}
        }
        self.format_word(w, p, v, on);
    }

    fn format_word(&mut self, w: &str, p: Option<i32>, v: i32, on: bool) {
        let in_field_inst = false;
        let _ = in_field_inst;
        match w {
            // Document / section.
            "deff" => self.deff = v,
            "paperw" | "paperh" | "margl" | "margr" | "margt" | "margb" => {
                // Document defaults also apply to the current section.
                let val = v as f32 / 20.0;
                for pg in [&mut self.def_page, &mut self.doc.page] {
                    match w {
                        "paperw" => pg.width = val,
                        "paperh" => pg.height = val,
                        "margl" => pg.margin_left = val,
                        "margr" => pg.margin_right = val,
                        "margt" => pg.margin_top = val,
                        _ => pg.margin_bottom = val,
                    }
                }
            }
            "pgwsxn" => self.doc.page.width = v as f32 / 20.0,
            "pghsxn" => self.doc.page.height = v as f32 / 20.0,
            "marglsxn" => self.doc.page.margin_left = v as f32 / 20.0,
            "margrsxn" => self.doc.page.margin_right = v as f32 / 20.0,
            "margtsxn" => self.doc.page.margin_top = v as f32 / 20.0,
            "margbsxn" => self.doc.page.margin_bottom = v as f32 / 20.0,
            "headery" => self.doc.page.header_distance = v as f32 / 20.0,
            "footery" => self.doc.page.footer_distance = v as f32 / 20.0,
            "landscape" | "lndscpsxn" => {
                self.landscape = true;
                if w == "landscape" {
                    let pg = &mut self.def_page;
                    if pg.width < pg.height {
                        std::mem::swap(&mut pg.width, &mut pg.height);
                    }
                }
                let pg = &mut self.doc.page;
                if pg.width < pg.height {
                    std::mem::swap(&mut pg.width, &mut pg.height);
                }
            }
            "cols" => self.doc.page.columns = v.max(1) as u32,
            "colsx" => self.doc.page.column_gap = v as f32 / 20.0,
            "titlepg" => self.doc.page.title_page = true,
            "pgnstarts" => self.doc.page.page_number_start = v.max(0) as u32,
            "deftab" => self.doc.settings.default_tab = (v as f32 / 20.0).max(1.0),
            "revisions" => self.doc.settings.track_changes = true,
            "sbknone" => self.doc.page.start = SectionStart::Continuous,
            "sbkpage" | "sbkcol" => self.doc.page.start = SectionStart::NextPage,
            "sbkodd" => self.doc.page.start = SectionStart::OddPage,
            "sbkeven" => self.doc.page.start = SectionStart::EvenPage,
            "sectd" => {
                // Section defaults: the document's page setup; headers stay inherited.
                self.doc.page = PageSetup { page_borders: None, ..self.def_page.clone() };
                self.landscape = false;
            }
            "sect" => self.end_section(),
            "shp" => {
                self.st().shape = true;
                self.st().dest = Dest::Shape;
                self.shape = Some(ShapeBuild::default());
            }
            // Stories.
            "footnote" => self.open_story(StoryKind::Footnote),
            "ftnalt" => {
                if let Some(s) = self.stories.last_mut() {
                    if s.0 == StoryKind::Footnote {
                        s.0 = StoryKind::Endnote;
                    }
                }
            }
            "header" => self.open_story(StoryKind::Header),
            "footer" => self.open_story(StoryKind::Footer),
            "headerf" => self.open_story(StoryKind::FirstHeader),
            "footerf" => self.open_story(StoryKind::FirstFooter),
            "annotation" => self.open_story(StoryKind::Annotation),
            "field" => {
                self.st().field = true;
                self.fields.push(FieldCtx::default());
            }
            // Character formatting.
            "plain" => {
                let st = self.st();
                st.chars = CharProps { link: st.chars.link.clone(), ..Default::default() };
                st.rev_kind = None;
            }
            "f" => {
                let name = self.fonts.get(&v).cloned();
                self.st().chars.font = name;
            }
            "fs" => self.st().chars.size = Some(v as f32 / 2.0),
            "b" => self.st().chars.bold = Some(on),
            "i" => self.st().chars.italic = Some(on),
            "ul" => self.st().chars.underline = Some(if on { Underline::Single } else { Underline::None }),
            "uldb" => self.st().chars.underline = Some(Underline::Double),
            "ulth" | "ulthd" | "ulthdash" => self.st().chars.underline = Some(Underline::Thick),
            "uld" => self.st().chars.underline = Some(Underline::Dotted),
            "uldash" | "uldashd" | "uldashdd" | "ulldash" => self.st().chars.underline = Some(Underline::Dashed),
            "ulwave" | "ulhwave" | "ululdbwave" => self.st().chars.underline = Some(Underline::Wavy),
            "ulw" => self.st().chars.underline = Some(Underline::Words),
            "ulnone" => self.st().chars.underline = Some(Underline::None),
            "strike" => self.st().chars.strike = Some(on),
            "striked" => self.st().chars.double_strike = Some(on),
            "cf" => {
                let c = self.color(v);
                self.st().chars.color = c;
            }
            "highlight" => {
                let c = self.color(v);
                self.st().chars.highlight = c;
            }
            "chcbpat" | "cb" => {
                let c = self.color(v);
                self.st().chars.shading = c;
            }
            "super" => self.st().chars.vert_align = Some(if on { VertAlign::Superscript } else { VertAlign::Baseline }),
            "sub" => self.st().chars.vert_align = Some(if on { VertAlign::Subscript } else { VertAlign::Baseline }),
            "nosupersub" => self.st().chars.vert_align = Some(VertAlign::Baseline),
            "caps" => self.st().chars.caps = Some(on),
            "scaps" => self.st().chars.small_caps = Some(on),
            "v" => self.st().chars.hidden = Some(on),
            "expndtw" => self.st().chars.spacing = Some(v as f32 / 20.0),
            "expnd" => self.st().chars.spacing = Some(v as f32 / 4.0),
            "uc" => self.st().uc = v.max(0) as usize,
            "u" => {
                let c = v as u16;
                let s = if (0xD800..0xDC00).contains(&c) {
                    // High surrogate: wait for the low one.
                    self.author_cur = format!("\u{0}{}", c);
                    String::new()
                } else if (0xDC00..0xE000).contains(&c) {
                    let hi = self.author_cur.strip_prefix('\u{0}').and_then(|h| h.parse::<u16>().ok());
                    self.author_cur.clear();
                    hi.map(|h| String::from_utf16_lossy(&[h, c])).unwrap_or_default()
                } else {
                    char::from_u32(c as u32).map(|c| c.to_string()).unwrap_or_default()
                };
                if !s.is_empty() {
                    self.text_raw(&s);
                }
                self.skip_chars = self.st().uc;
            }
            "revised" => self.st().rev_kind = Some(RevisionKind::Insert),
            "deleted" => self.st().rev_kind = Some(RevisionKind::Delete),
            "revauth" | "revauthdel" => self.st().rev_author = Some(v.max(0) as usize),
            "revdttm" | "revdttmdel" => self.st().rev_date = v as i64,
            // Paragraph formatting.
            "pard" => {
                let st = self.st();
                st.para = ParaProps::default();
                st.in_table = false;
                self.border = BorderTarget::None;
            }
            "s" => {
                let id = self.styles.get(&v).cloned();
                self.st().para.style = id.filter(|s| s != "Normal");
            }
            "cs" => {
                let id = self.styles.get(&(-1 - v.abs())).cloned();
                self.st().chars.style = id;
            }
            "ql" => self.st().para.align = Some(Align::Left),
            "qc" => self.st().para.align = Some(Align::Center),
            "qr" => self.st().para.align = Some(Align::Right),
            "qj" | "qd" => self.st().para.align = Some(Align::Justify),
            "li" | "lin" => self.st().para.indent_left = Some(v as f32 / 20.0),
            "ri" | "rin" => self.st().para.indent_right = Some(v as f32 / 20.0),
            "fi" => self.st().para.indent_first = Some(v as f32 / 20.0),
            "sb" => self.st().para.space_before = Some(v as f32 / 20.0),
            "sa" => self.st().para.space_after = Some(v as f32 / 20.0),
            "sl" => {
                self.st().para.line_spacing = Some(if v == 0 {
                    LineSpacing::Multiple(1.0)
                } else if v < 0 {
                    LineSpacing::Exactly(-v as f32 / 20.0)
                } else {
                    LineSpacing::AtLeast(v as f32 / 20.0)
                })
            }
            "slmult" => {
                if v == 1 {
                    if let Some(LineSpacing::AtLeast(x)) = self.st().para.line_spacing {
                        self.st().para.line_spacing = Some(LineSpacing::Multiple(((x * 20.0 / 240.0) * 100.0).round() / 100.0));
                    }
                }
            }
            "contextualspace" => self.st().para.contextual_spacing = Some(true),
            "keepn" => self.st().para.keep_next = Some(true),
            "keep" => self.st().para.keep_lines = Some(true),
            "pagebb" => self.st().para.page_break_before = Some(true),
            "widctlpar" => self.st().para.widow_control = Some(true),
            "nowidctlpar" => self.st().para.widow_control = Some(false),
            "outlinelevel" => self.st().para.outline_level = Some(v.clamp(0, 8) as u8),
            "cbpat" => {
                let c = self.color(v);
                self.st().para.shading = c;
            }
            "intbl" => self.st().in_table = true,
            "itap" => {
                if v == 0 {
                    self.st().in_table = false;
                }
            }
            "ls" => {
                let id = self.ls_map.get(&v).copied();
                let lvl = self.st().list_level_ref.map(|l| l.level).unwrap_or(0);
                if let Some(id) = id {
                    self.st().para.list = Some(Some(ListRef { list: id, level: lvl }));
                }
            }
            "ilvl" => {
                let lvl = v.clamp(0, 8) as u8;
                if let Some(Some(l)) = self.st().para.list.as_mut() {
                    l.level = lvl;
                }
                self.st().list_level_ref = Some(ListRef { list: 0, level: lvl });
            }
            "tx" | "tb" => {
                let leader = self.pending_tab.1;
                let align = self.pending_tab.0;
                self.st().para.tabs.push(TabStop { pos: v as f32 / 20.0, align, leader });
                self.pending_tab = (TabAlign::Left, TabLeader::None);
            }
            "tqr" => self.pending_tab.0 = TabAlign::Right,
            "tqc" => self.pending_tab.0 = TabAlign::Center,
            "tqdec" => self.pending_tab.0 = TabAlign::Decimal,
            "tldot" | "tlmdot" => self.pending_tab.1 = TabLeader::Dot,
            "tlhyph" => self.pending_tab.1 = TabLeader::Hyphen,
            "tlul" | "tlth" | "tleq" => self.pending_tab.1 = TabLeader::Underscore,
            // Borders.
            "brdrt" => self.border = BorderTarget::Para(0),
            "brdrb" => self.border = BorderTarget::Para(1),
            "brdrl" => self.border = BorderTarget::Para(2),
            "brdrr" => self.border = BorderTarget::Para(3),
            "brdrbtw" => self.border = BorderTarget::Para(4),
            "clbrdrt" => self.border = BorderTarget::Cell(0),
            "clbrdrb" => self.border = BorderTarget::Cell(1),
            "clbrdrl" => self.border = BorderTarget::Cell(2),
            "clbrdrr" => self.border = BorderTarget::Cell(3),
            "trbrdrt" => self.border = BorderTarget::Row(0),
            "trbrdrb" => self.border = BorderTarget::Row(1),
            "trbrdrl" => self.border = BorderTarget::Row(2),
            "trbrdrr" => self.border = BorderTarget::Row(3),
            "trbrdrh" => self.border = BorderTarget::Row(4),
            "trbrdrv" => self.border = BorderTarget::Row(5),
            "pgbrdrt" => self.border = BorderTarget::Page(0),
            "pgbrdrb" => self.border = BorderTarget::Page(1),
            "pgbrdrl" => self.border = BorderTarget::Page(2),
            "pgbrdrr" => self.border = BorderTarget::Page(3),
            "brdrs" | "brdrdb" | "brdrdot" | "brdrdash" | "brdrth" | "brdrnone" | "brdrsh" | "brdrhair" | "brdrdashsm" | "brdrtriple" => {
                let style = match w {
                    "brdrdb" | "brdrtriple" => BorderStyle::Double,
                    "brdrdot" => BorderStyle::Dotted,
                    "brdrdash" | "brdrdashsm" => BorderStyle::Dashed,
                    "brdrth" => BorderStyle::Thick,
                    "brdrnone" => BorderStyle::None,
                    _ => BorderStyle::Single,
                };
                self.modify_border(|b| {
                    b.style = style;
                    if style == BorderStyle::None {
                        b.width = 0.0;
                    }
                });
            }
            "brdrw" => self.modify_border(|b| b.width = v as f32 / 20.0),
            "brsp" => self.modify_border(|b| b.space = v as f32 / 20.0),
            "brdrcf" => {
                let c = self.color(v).unwrap_or(Color::BLACK);
                self.modify_border(|b| b.color = c);
            }
            // Tables.
            "trowd" => {
                let s = self.story();
                s.row_defs.clear();
                s.row_props = Row::default();
                s.row_align = Align::Left;
                s.row_left = 0.0;
                s.row_borders = Borders::default();
                self.cell_shading = None;
                self.cell_vmerge = VMerge::None;
                self.cell_valign = VAlign::Top;
                self.cell_hmerge = false;
                self.pending_borders.1 = Borders::default();
            }
            "trgaph" => self.story().row_gap = v as f32 / 20.0,
            "trleft" => self.story().row_left = v as f32 / 20.0,
            "trqc" => self.story().row_align = Align::Center,
            "trqr" => self.story().row_align = Align::Right,
            "trql" => self.story().row_align = Align::Left,
            "trrh" => self.story().row_props.height = if v != 0 { Some(v.unsigned_abs() as f32 / 20.0) } else { None },
            "trhdr" => self.story().row_props.header = true,
            "clvmgf" => self.cell_vmerge = VMerge::Restart,
            "clvmrg" => self.cell_vmerge = VMerge::Continue,
            "clmrg" => self.cell_hmerge = true,
            "clvertalt" => self.cell_valign = VAlign::Top,
            "clvertalc" => self.cell_valign = VAlign::Center,
            "clvertalb" => self.cell_valign = VAlign::Bottom,
            "clcbpat" => self.cell_shading = self.color(v),
            "cellx" => {
                let cell = Cell {
                    shading: self.cell_shading.take(),
                    vmerge: std::mem::take(&mut self.cell_vmerge),
                    valign: std::mem::take(&mut self.cell_valign),
                    span: if std::mem::take(&mut self.cell_hmerge) { 0 } else { 1 },
                    ..Default::default()
                };
                let b = std::mem::take(&mut self.pending_borders.1);
                self.story().row_defs.push((v as f32 / 20.0, cell, b));
            }
            "cell" | "nestcell" => {
                if w == "nestcell" {
                    self.text_raw("\t");
                    return;
                }
                let mut s = std::mem::take(self.story());
                self.finish_para_into_cell(&mut s);
                let paras = std::mem::take(&mut s.cell_paras);
                s.row_cells.push(Cell { paragraphs: paras, ..Default::default() });
                *self.story() = s;
            }
            "row" => {
                let mut s = std::mem::take(self.story());
                self.finish_row(&mut s);
                *self.story() = s;
            }
            "nestrow" => {}
            // Special characters.
            "par" => {
                let mut s = std::mem::take(self.story());
                if self.st().in_table {
                    self.finish_para_into_cell(&mut s);
                } else {
                    self.finish_para(&mut s);
                }
                *self.story() = s;
                self.last_was_par = true;
            }
            "line" => self.inline(Inline::LineBreak),
            "tab" => self.inline(Inline::Tab),
            "page" => self.inline(Inline::PageBreak),
            "column" => self.inline(Inline::ColumnBreak),
            "emdash" => self.text_raw("—"),
            "endash" => self.text_raw("–"),
            "bullet" => self.text_raw("•"),
            "lquote" => self.text_raw("‘"),
            "rquote" => self.text_raw("’"),
            "ldblquote" => self.text_raw("“"),
            "rdblquote" => self.text_raw("”"),
            "emspace" => self.text_raw("\u{2003}"),
            "enspace" => self.text_raw("\u{2002}"),
            "qmspace" => self.text_raw("\u{2005}"),
            "zwj" => self.text_raw("\u{200D}"),
            "zwnj" => self.text_raw("\u{200C}"),
            "chpgn" => self.inline(Inline::Field(Field::PageNumber)),
            "chdate" => self.inline(Inline::Field(Field::Date("M/d/yyyy".into()))),
            _ => {
                let _ = p;
            }
        }
    }

    fn modify_border(&mut self, f: impl FnOnce(&mut Border)) {
        let target = self.border;
        let slot: Option<&mut Option<Border>> = match target {
            BorderTarget::None => None,
            BorderTarget::Para(i) => {
                let b = self.st().para.borders.get_or_insert_with(Borders::default);
                Some(match i {
                    0 => &mut b.top,
                    1 => &mut b.bottom,
                    2 => &mut b.left,
                    3 => &mut b.right,
                    _ => &mut b.between,
                })
            }
            BorderTarget::Cell(i) => {
                let b = &mut self.pending_borders.1;
                Some(match i {
                    0 => &mut b.top,
                    1 => &mut b.bottom,
                    2 => &mut b.left,
                    _ => &mut b.right,
                })
            }
            BorderTarget::Row(i) => {
                let b = &mut self.stories.last_mut().unwrap().1.row_borders;
                Some(match i {
                    0 => &mut b.top,
                    1 => &mut b.bottom,
                    2 => &mut b.left,
                    3 => &mut b.right,
                    4 => &mut b.between,
                    _ => &mut b.inside_v,
                })
            }
            BorderTarget::Page(i) => {
                let b = self.doc.page.page_borders.get_or_insert_with(Borders::default);
                Some(match i {
                    0 => &mut b.top,
                    1 => &mut b.bottom,
                    2 => &mut b.left,
                    _ => &mut b.right,
                })
            }
        };
        if let Some(slot) = slot {
            let b = slot.get_or_insert(Border { style: BorderStyle::Single, width: 0.5, color: Color::BLACK, space: 0.0 });
            f(b);
        }
    }

    fn open_story(&mut self, kind: StoryKind) {
        self.st().story = Some(kind);
        self.st().dest = Dest::Normal;
        self.st().para = ParaProps::default();
        self.st().in_table = false;
        self.stories.push((kind, Story::default()));
    }

    fn hex(&mut self, h: u8) {
        match self.st().dest {
            Dest::LevelText | Dest::LevelNumbers => {
                if self.skip_chars > 0 {
                    self.skip_chars -= 1;
                    return;
                }
                self.level_cur.text_raw.push(h as u32);
            }
            _ => {
                if self.skip_chars > 0 {
                    self.skip_chars -= 1;
                    return;
                }
                let c = if self.st().font_charset_symbol { char::from_u32(0xF000 + h as u32).unwrap_or('?') } else { cp1252_char(h) };
                self.text_raw(&c.to_string());
            }
        }
    }

    fn text(&mut self, s: &str) {
        let mut s = s;
        let mut skipped = String::new();
        if self.skip_chars > 0 {
            let n = self.skip_chars.min(s.chars().count());
            let idx = s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len());
            skipped = s[idx..].to_string();
            self.skip_chars -= n;
            s = &skipped;
        }
        let s = s.to_string();
        let _ = &skipped;
        match self.st().dest {
            Dest::LevelText => self.level_cur.text_raw.extend(s.chars().map(|c| c as u32)),
            Dest::LevelNumbers => {}
            _ => self.text_raw(&s),
        }
    }

    /// Text after control-word processing, routed by destination.
    fn text_raw(&mut self, s: &str) {
        let dest = self.st().dest;
        match dest {
            Dest::Skip | Dest::ListTable | Dest::List | Dest::ListLevel | Dest::ListOverrideTable | Dest::ListOverride | Dest::LevelNumbers => {}
            Dest::LevelText => self.level_cur.text_raw.extend(s.chars().map(|c| c as u32)),
            Dest::FontTable => {}
            Dest::FontEntry => self.font_cur.1.push_str(s),
            Dest::ColorTable => {
                for c in s.chars() {
                    if c == ';' {
                        let (r, g, b, set) = self.color_cur;
                        self.colors.push(if set { Some(Color::rgb(r, g, b)) } else { None });
                        self.color_cur = (0, 0, 0, false);
                    }
                }
            }
            Dest::Stylesheet => {}
            Dest::StyleEntry => self.style_cur.1.push_str(s),
            Dest::RevTbl => self.author_cur.push_str(s),
            Dest::Info(_) | Dest::BkmkStart | Dest::BkmkEnd | Dest::AtnId | Dest::AtnAuthor | Dest::AtnRef | Dest::AtnDate | Dest::AtrfStart
            | Dest::AtrfEnd => self.info_text.push_str(s),
            Dest::Pict => self.pict_hex.push_str(s),
            Dest::Shape | Dest::Sp => {}
            Dest::Sn => {
                if let Some(sh) = self.shape.as_mut() {
                    sh.sn.push_str(s);
                }
            }
            Dest::Sv => {
                if let Some(sh) = self.shape.as_mut() {
                    sh.sv.push_str(s);
                }
            }
            Dest::FldInst => {
                if let Some(f) = self.fields.last_mut() {
                    f.instr.push_str(s);
                }
            }
            Dest::Normal | Dest::FldRslt => {
                if s.is_empty() {
                    return;
                }
                self.inline(Inline::Text(s.to_string()));
            }
        }
    }

    fn current_props(&self) -> CharProps {
        let st = self.stack.last().unwrap();
        let mut p = st.chars.clone();
        if let Some(k) = st.rev_kind {
            let author = st.rev_author.and_then(|i| self.authors.get(i).cloned()).unwrap_or_default();
            p.revision = Some(Revision { kind: k, author, date: dttm_to_iso(st.rev_date) });
        }
        if !self.active_comments.is_empty() {
            p.comments = self.active_comments.clone();
        }
        if p.link.is_none() {
            if let Some(l) = self.fields.iter().rev().find_map(|f| f.link.clone().filter(|_| f.in_result)) {
                p.link = Some(l);
            }
        }
        p
    }

    fn inline(&mut self, content: Inline) {
        self.last_was_par = false;
        let props = self.current_props();
        if let Some(f) = self.fields.last_mut() {
            if !f.in_result {
                // Text in a field group but outside fldinst/fldrslt: ignore.
                if self.stack.last().unwrap().dest != Dest::FldRslt {
                    if let Inline::Text(t) = &content {
                        f.instr.push_str(t);
                    }
                    return;
                }
            }
            if f.swallow {
                f.result.push_str(&content.plain());
                if f.props.is_none() {
                    f.props = Some(props);
                }
                return;
            }
        }
        let para_props = self.st().para.clone();
        let story = self.story();
        if story.para.runs.is_empty() {
            story.para.props = para_props;
        }
        story.para.runs.push(Run { content, props });
    }

    fn finish_para(&mut self, s: &mut Story) {
        self.flush_table(s);
        let mut p = std::mem::take(&mut s.para);
        if p.runs.is_empty() {
            p.props = self.st().para.clone();
        }
        p.props.style = p.props.style.filter(|x| x != "Normal");
        crate::io::docx::merge_runs(&mut p.runs);
        s.blocks.push(Block::Para(p));
    }

    fn finish_para_into_cell(&mut self, s: &mut Story) {
        let mut p = std::mem::take(&mut s.para);
        if p.runs.is_empty() {
            p.props = self.st().para.clone();
        }
        crate::io::docx::merge_runs(&mut p.runs);
        s.cell_paras.push(p);
    }

    fn finish_row(&mut self, s: &mut Story) {
        let cells = std::mem::take(&mut s.row_cells);
        let defs = s.row_defs.clone();
        let t = s.table.get_or_insert_with(|| Table { borders: Borders::default(), style: None, ..Default::default() });
        t.align = s.row_align;
        t.cell_margin = if s.row_gap > 0.0 { s.row_gap } else { t.cell_margin };
        t.indent = s.row_left + t.cell_margin;
        if t.rows.is_empty() {
            t.borders = s.row_borders.clone();
        }
        let mut row = Row { cells: Vec::new(), height: s.row_props.height, header: s.row_props.header };
        let mut prev_x = s.row_left;
        let mut bounds = Vec::new();
        for (i, c) in cells.into_iter().enumerate() {
            let (x, def, b) = defs.get(i).cloned().unwrap_or((prev_x + 72.0, Cell::default(), Borders::default()));
            bounds.push(x);
            prev_x = x;
            if def.span == 0 {
                // \clmrg: merged into the previous cell.
                if let Some(last) = row.cells.last_mut() {
                    last.span += 1;
                    continue;
                }
            }
            let has_b = b != Borders::default();
            row.cells.push(Cell {
                paragraphs: if c.paragraphs.is_empty() { vec![Paragraph::new()] } else { c.paragraphs },
                span: 1,
                vmerge: def.vmerge,
                shading: def.shading,
                valign: def.valign,
                borders: if has_b { Some(b) } else { None },
                width: None,
            });
        }
        // Record the right edges so spans can be computed against the table grid at the end.
        row.cells.iter_mut().zip(bounds.iter()).for_each(|(c, x)| c.width = Some(*x));
        t.rows.push(row);
    }

    fn flush_table(&mut self, s: &mut Story) {
        if !s.cell_paras.is_empty() || !s.row_cells.is_empty() {
            // Unterminated row: close it.
            if !s.cell_paras.is_empty() {
                let paras = std::mem::take(&mut s.cell_paras);
                s.row_cells.push(Cell { paragraphs: paras, ..Default::default() });
            }
            self.finish_row(s);
        }
        let Some(mut t) = s.table.take() else { return };
        // Grid from all right edges.
        let mut edges: Vec<i64> = t.rows.iter().flat_map(|r| r.cells.iter().filter_map(|c| c.width.map(|w| (w * 20.0).round() as i64))).collect();
        edges.sort_unstable();
        edges.dedup();
        let left = ((t.indent - t.cell_margin) * 20.0).round() as i64;
        let mut prev = left;
        t.columns = edges.iter().map(|e| {
            let w = (*e - prev).max(20) as f32 / 20.0;
            prev = *e;
            w
        }).collect();
        for r in t.rows.iter_mut() {
            let mut start_idx = 0usize;
            for c in r.cells.iter_mut() {
                let right = c.width.map(|w| (w * 20.0).round() as i64).unwrap_or(0);
                let end_idx = edges.iter().position(|e| *e == right).map(|i| i + 1).unwrap_or(start_idx + 1);
                c.span = (end_idx.saturating_sub(start_idx)).max(1) as u32;
                start_idx = end_idx;
                c.width = None;
            }
        }
        if t.columns.is_empty() {
            t.columns = vec![72.0];
        }
        // Cell borders equal to the table's derived ones are redundant.
        let tb = t.borders.clone();
        if tb == Borders::default() {
            if let Some(b) = t.rows.first().and_then(|r| r.cells.first()).and_then(|c| c.borders.clone()) {
                t.borders = Borders { top: b.top, bottom: b.bottom, left: b.left, right: b.right, between: b.bottom, inside_v: b.right };
            }
        }
        s.blocks.push(Block::Table(t));
    }

    fn end_group(&mut self, st: &GState) {
        let parent_dest = self.stack.last().map(|s| s.dest).unwrap_or(Dest::Normal);
        match st.dest {
            Dest::FontEntry if parent_dest != Dest::FontEntry => {
                let name = self.font_cur.1.trim().trim_end_matches(';').trim().to_string();
                if !name.is_empty() {
                    self.fonts.insert(self.font_cur.0, name);
                }
            }
            Dest::StyleEntry if parent_dest != Dest::StyleEntry => {
                let name = self.style_cur.1.trim().trim_end_matches(';').trim().to_string();
                let id = crate::io::docx::builtin_id_for_name(&name)
                    .map(|s| s.to_string())
                    .or_else(|| self.doc.style_by_name(&name).map(|s| s.id.clone()))
                    .unwrap_or_else(|| {
                        let id: String = name.chars().filter(|c| !c.is_whitespace()).collect();
                        if self.style_cur.0 >= 0 && !id.is_empty() && self.doc.style(&id).is_none() {
                            self.doc.styles.push(Style { id: id.clone(), name: name.clone(), kind: StyleKind::Paragraph, based_on: Some("Normal".into()), ..Default::default() });
                        }
                        id
                    });
                if let Some(style) = self.doc.styles.iter_mut().find(|s| s.id == id) {
                    style.chars = st.chars.clone();
                    style.para = st.para.clone();
                    style.para.style = None;
                    style.chars.style = None;
                    if self.style_cur.0 < 0 {
                        style.kind = StyleKind::Character;
                    }
                }
                if let Some(b) = self.style_based {
                    self.style_based_pending.push((id.clone(), b));
                }
                self.styles.insert(self.style_cur.0, id);
            }
            Dest::Info(f) if parent_dest != st.dest => {
                let t = self.info_text.trim().to_string();
                let (y, mo, d, h, mi) = self.info_date;
                let date = if y > 0 { format!("{:04}-{:02}-{:02}T{:02}:{:02}:00Z", y, mo.max(1), d.max(1), h, mi) } else { String::new() };
                let p = &mut self.doc.properties;
                match f {
                    InfoField::Title => p.title = t,
                    InfoField::Subject => p.subject = t,
                    InfoField::Author => p.author = t,
                    InfoField::Keywords => p.keywords = t,
                    InfoField::Comment => p.description = t,
                    InfoField::Operator => p.last_modified_by = t,
                    InfoField::Created => p.created = date,
                    InfoField::Revised => p.modified = date,
                }
                self.info_text.clear();
            }
            Dest::Pict if parent_dest != Dest::Pict => self.finish_pict(),
            Dest::Sp if parent_dest != Dest::Sp => {
                if let Some(sh) = self.shape.as_mut() {
                    let name = std::mem::take(&mut sh.sn).trim().to_string();
                    let value = std::mem::take(&mut sh.sv).trim().to_string();
                    if !name.is_empty() {
                        sh.props.insert(name, value);
                    }
                }
            }
            Dest::LevelText if parent_dest != Dest::LevelText => {}
            Dest::ListLevel if parent_dest != Dest::ListLevel => {
                let lb = std::mem::take(&mut self.level_cur);
                let format = match lb.nfc {
                    0 => NumFormat::Decimal,
                    1 => NumFormat::UpperRoman,
                    2 => NumFormat::LowerRoman,
                    3 => NumFormat::UpperLetter,
                    4 => NumFormat::LowerLetter,
                    23 => NumFormat::Bullet,
                    22 => NumFormat::DecimalZero,
                    255 => NumFormat::None,
                    _ => NumFormat::Decimal,
                };
                let mut text = String::new();
                if let Some((&len, rest)) = lb.text_raw.split_first() {
                    for &c in rest.iter().take(len as usize) {
                        if c < 9 && format != NumFormat::Bullet {
                            let _ = write!(text, "%{}", c + 1);
                        } else if let Some(ch) = char::from_u32(c) {
                            text.push(ch);
                        }
                    }
                }
                let mut font = lb.font.and_then(|f| self.fonts.get(&(f as i32)).cloned());
                if format == NumFormat::Bullet {
                    if let Some(m) = map_symbol_bullet(&text, font.as_deref()) {
                        text = m;
                        font = None;
                    }
                }
                self.list_cur.1.push(ListLevel { format, text, start: lb.start.max(1), indent: lb.li, hanging: -lb.fi, font });
            }
            Dest::List if parent_dest != Dest::List => {
                let (id, levels) = std::mem::take(&mut self.list_cur);
                let mut def = ListDef::numbered(0);
                for (i, l) in levels.into_iter().enumerate().take(9) {
                    def.levels[i] = l;
                }
                self.lists.insert(id, def);
            }
            Dest::ListOverride if parent_dest != Dest::ListOverride => {
                let (listid, ls) = self.override_cur;
                if let Some(def) = self.lists.get(&listid).cloned() {
                    let id = self.doc.add_list(def);
                    self.ls_map.insert(ls, id);
                }
            }
            Dest::RevTbl if parent_dest != Dest::RevTbl => {
                self.authors = self.author_cur.split(';').map(|a| a.trim().to_string()).collect();
                if self.authors.last().map(|a| a.is_empty()).unwrap_or(false) {
                    self.authors.pop();
                }
                self.author_cur.clear();
            }
            Dest::BkmkStart if parent_dest != Dest::BkmkStart => {
                let name = std::mem::take(&mut self.info_text).trim().to_string();
                if !name.is_empty() && name != "_GoBack" {
                    // Bookmarks are emitted in the parent destination.
                    let saved = self.st().dest;
                    self.st().dest = Dest::Normal;
                    self.inline(Inline::Bookmark(name));
                    self.st().dest = saved;
                }
            }
            Dest::BkmkEnd if parent_dest != Dest::BkmkEnd => self.info_text.clear(),
            Dest::AtnId if parent_dest != st.dest => self.atn.0 = std::mem::take(&mut self.info_text).trim().to_string(),
            Dest::AtnAuthor if parent_dest != st.dest => self.atn.1 = std::mem::take(&mut self.info_text).trim().to_string(),
            Dest::AtnRef if parent_dest != st.dest => self.atn.2 = std::mem::take(&mut self.info_text).trim().to_string(),
            Dest::AtnDate if parent_dest != st.dest => self.atn.3 = std::mem::take(&mut self.info_text).trim().to_string(),
            Dest::AtrfStart if parent_dest != st.dest => {
                let key = std::mem::take(&mut self.info_text).trim().to_string();
                let id = self.doc.next_comment_id().max(self.atn_ranges.values().max().map(|m| m + 1).unwrap_or(1));
                self.atn_ranges.insert(key, id);
                self.active_comments.push(id);
                self.active_comments.sort_unstable();
            }
            Dest::AtrfEnd if parent_dest != st.dest => {
                let key = std::mem::take(&mut self.info_text).trim().to_string();
                if let Some(id) = self.atn_ranges.get(&key) {
                    let id = *id;
                    self.active_comments.retain(|x| *x != id);
                }
            }
            _ => {}
        }
        if st.shape {
            self.finish_shape();
        }
        if st.field {
            if let Some(f) = self.fields.pop() {
                if f.swallow || !f.in_result {
                    let field = crate::io::docx::field_from_instr(&f.instr, &f.result);
                    let props = f.props.unwrap_or_else(|| self.current_props());
                    let saved = self.st().dest;
                    self.st().dest = Dest::Normal;
                    let para_props = self.st().para.clone();
                    let story = self.story();
                    if story.para.runs.is_empty() {
                        story.para.props = para_props;
                    }
                    story.para.runs.push(Run { content: Inline::Field(field), props });
                    self.st().dest = saved;
                }
            }
        }
        if let Some(kind) = st.story {
            let (k, mut story) = self.stories.pop().unwrap();
            if !story.para.runs.is_empty() || !story.cell_paras.is_empty() || story.blocks.is_empty() {
                // Use the group's final paragraph props for the last paragraph.
                if story.para.runs.is_empty() {
                    story.para.props = st.para.clone();
                }
                let mut p = std::mem::take(&mut story.para);
                p.props.style = p.props.style.filter(|x| x != "Normal");
                crate::io::docx::merge_runs(&mut p.runs);
                self.flush_table(&mut story);
                story.blocks.push(Block::Para(p));
            } else {
                self.flush_table(&mut story);
            }
            let kind = if kind == StoryKind::Footnote { k } else { kind };
            let paras: Vec<Paragraph> = story.blocks.iter().filter_map(|b| b.as_para().cloned()).collect();
            match kind {
                StoryKind::Footnote | StoryKind::Endnote => {
                    let id = self.doc.next_note_id();
                    let mut paras = paras;
                    // Strip the leading space after the note mark.
                    if let Some(Run { content: Inline::Text(t), .. }) = paras.first_mut().and_then(|p| p.runs.first_mut()) {
                        if let Some(rest) = t.strip_prefix(' ') {
                            *t = rest.to_string();
                        }
                    }
                    if let Some(p) = paras.first_mut() {
                        if matches!(p.runs.first(), Some(Run { content: Inline::Text(t), .. }) if t.is_empty()) {
                            p.runs.remove(0);
                        }
                    }
                    let note = Note { id, paragraphs: paras };
                    let content = if kind == StoryKind::Footnote {
                        self.doc.footnotes.push(note);
                        Inline::FootnoteRef(id)
                    } else {
                        self.doc.endnotes.push(note);
                        Inline::EndnoteRef(id)
                    };
                    let mut props = self.current_props();
                    props.vert_align = None;
                    let para_props = self.st().para.clone();
                    let s = self.story();
                    if s.para.runs.is_empty() {
                        s.para.props = para_props;
                    }
                    s.para.runs.push(Run { content, props });
                }
                StoryKind::Header | StoryKind::Footer | StoryKind::FirstHeader | StoryKind::FirstFooter => {
                    let slot = match kind {
                        StoryKind::Header => HfKind::Header,
                        StoryKind::Footer => HfKind::Footer,
                        StoryKind::FirstHeader => HfKind::FirstHeader,
                        _ => HfKind::FirstFooter,
                    };
                    *self.doc.headers.part_mut(slot) = story.blocks;
                    self.own_headers = true;
                }
                StoryKind::ShapeText => {
                    let mut paras = paras;
                    if paras.is_empty() {
                        paras.push(Paragraph::new());
                    }
                    if let Some(sh) = self.shape.as_mut() {
                        sh.text = Some(paras);
                    }
                }
                StoryKind::Annotation => {
                    let (initials, author, key, date) = std::mem::take(&mut self.atn);
                    let id = self.atn_ranges.get(&key).copied().unwrap_or_else(|| self.doc.next_comment_id());
                    let date = date.parse::<i64>().map(dttm_to_iso).unwrap_or_default();
                    self.doc.comments.push(Comment { id, author, initials, date, paragraphs: paras, resolved: false });
                    self.doc.comments.sort_by_key(|c| c.id);
                }
                StoryKind::Skip => {}
            }
        }
    }

    fn finish_pict(&mut self) {
        let hex: Vec<u8> = self.pict_hex.bytes().filter(|b| b.is_ascii_hexdigit()).collect();
        let bytes: Vec<u8> = hex.chunks_exact(2).filter_map(|c| u8::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok()).collect();
        self.pict_hex.clear();
        if bytes.is_empty() || self.pict_kind == "unsupported" {
            return;
        }
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut h);
            h.finish()
        };
        let data = match self.images.get(&key) {
            Some(d) => d.clone(),
            None => {
                let d = image_data(bytes);
                self.images.insert(key, d.clone());
                d
            }
        };
        let (mut w, mut h) = self.pict_size;
        if w <= 0.0 || h <= 0.0 {
            let n = natural_size(&data, 468.0);
            w = if self.pict_px.0 > 0.0 { self.pict_px.0 * 0.75 } else { n.0 };
            h = if self.pict_px.1 > 0.0 { self.pict_px.1 * 0.75 } else { n.1 };
        }
        w *= self.pict_scale.0 / 100.0;
        h *= self.pict_scale.1 / 100.0;
        let img = Image { data, width: w, height: h, alt: String::new(), float: None };
        if let Some(sh) = self.shape.as_mut() {
            if self.stack.iter().any(|g| g.dest == Dest::Sv) {
                sh.pict = Some(img);
                return;
            }
        }
        // Emit in the enclosing destination.
        let saved = self.st().dest;
        self.st().dest = Dest::Normal;
        self.inline(Inline::Image(img));
        self.st().dest = saved;
    }

    /// A finished `\shp`: a floating picture or text box.
    fn finish_shape(&mut self) {
        let Some(sh) = self.shape.take() else { return };
        let prop = |n: &str| sh.props.get(n).and_then(|v| v.trim().parse::<i64>().ok());
        let emu = |n: &str, d: f32| prop(n).map(|v| v as f32 / 12700.0).unwrap_or(d);
        let color = |n: &str, d: Color| prop(n).map(|v| Color::rgb((v & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, ((v >> 16) & 0xFF) as u8)).unwrap_or(d);
        let width = (sh.rect[2] - sh.rect[0]) as f32 / 20.0;
        let height = (sh.rect[3] - sh.rect[1]) as f32 / 20.0;
        let float = if prop("fPseudoInline") == Some(1) {
            None
        } else {
            let mut f = Float {
                x: sh.rect[0] as f32 / 20.0,
                y: sh.rect[1] as f32 / 20.0,
                h_rel: sh.bx.unwrap_or(match prop("posrelh") {
                    Some(0) => HRel::Margin,
                    Some(1) => HRel::Page,
                    Some(3) => HRel::Character,
                    _ => HRel::Column,
                }),
                v_rel: sh.by.unwrap_or(match prop("posrelv") {
                    Some(0) => VRel::Margin,
                    Some(1) => VRel::Page,
                    Some(3) => VRel::Line,
                    _ => VRel::Paragraph,
                }),
                ..Default::default()
            };
            f.h_align = match prop("posh") {
                Some(1 | 4) => Some(FloatAlign::Start),
                Some(2) => Some(FloatAlign::Center),
                Some(3 | 5) => Some(FloatAlign::End),
                _ => None,
            };
            f.v_align = match prop("posv") {
                Some(1 | 4) => Some(FloatAlign::Start),
                Some(2) => Some(FloatAlign::Center),
                Some(3 | 5) => Some(FloatAlign::End),
                _ => None,
            };
            let behind = sh.behind || prop("fBehindDocument") == Some(1);
            f.wrap = match sh.wr {
                Some(1) => WrapMode::TopBottom,
                Some(3) if behind => WrapMode::Behind,
                Some(3) => WrapMode::InFront,
                Some(4 | 5) => WrapMode::Tight,
                _ => WrapMode::Square,
            };
            f.dist = ["dxWrapDistLeft", "dxWrapDistRight", "dyWrapDistTop", "dyWrapDistBottom"]
                .iter()
                .map(|n| emu(n, if n.starts_with("dx") { 9.0 } else { 0.0 }))
                .fold(0.0, f32::max);
            Some(f)
        };
        let content = if let Some(paragraphs) = sh.text {
            let fill = if prop("fFilled") == Some(0) { None } else { Some(color("fillColor", Color::WHITE)) };
            let border = if prop("fLine") == Some(0) {
                None
            } else {
                Some(Border { style: BorderStyle::Single, width: emu("lineWidth", 0.75), color: color("lineColor", Color::BLACK), space: 0.0 })
            };
            Inline::TextBox(Box::new(TextBox {
                paragraphs,
                width: if width > 0.0 { width } else { 144.0 },
                height: if height > 0.0 { height } else { 72.0 },
                float,
                fill,
                border,
                padding: emu("dxTextLeft", 7.2),
                auto_height: prop("fFitShapeToText") == Some(1),
            }))
        } else if let Some(mut img) = sh.pict {
            if width > 0.0 && height > 0.0 {
                img.width = width;
                img.height = height;
            }
            img.float = float;
            img.alt = sh.props.get("wzDescription").cloned().unwrap_or_default();
            Inline::Image(img)
        } else {
            return;
        };
        let saved = self.st().dest;
        self.st().dest = Dest::Normal;
        self.inline(content);
        self.st().dest = saved;
    }

    /// `\sect`: the current paragraph ends the current section.
    fn end_section(&mut self) {
        if self.stories.len() != 1 || self.st().in_table {
            return;
        }
        let mut s = std::mem::take(self.story());
        // "…\par\sect": the section break belongs to the paragraph just finished.
        let reuse = self.last_was_par && s.para.runs.is_empty() && s.table.is_none() && matches!(s.blocks.last(), Some(Block::Para(p)) if p.section.is_none());
        if !reuse {
            self.finish_para(&mut s);
        }
        if let Some(Block::Para(p)) = s.blocks.last_mut() {
            p.section = Some(Box::default());
        }
        *self.story() = s;
        let mut page = self.doc.page.clone();
        if self.landscape && page.width < page.height {
            std::mem::swap(&mut page.width, &mut page.height);
        }
        self.sections.push((page, self.doc.headers.clone(), std::mem::take(&mut self.own_headers)));
    }

    /// Fill in the section marks of the body (see [`Self::end_section`]).
    fn finish_sections(&mut self, blocks: &mut [Block]) {
        if self.landscape && self.doc.page.width < self.doc.page.height {
            let pg = &mut self.doc.page;
            std::mem::swap(&mut pg.width, &mut pg.height);
        }
        let last_hf = self.doc.headers.clone();
        let mut decided: Vec<Option<HeadersFooters>> = Vec::new();
        let mut out = Vec::new();
        for (page, hf, own) in std::mem::take(&mut self.sections) {
            let resolved = decided.iter().rev().find_map(|h| h.as_ref()).unwrap_or(&last_hf);
            let headers = if own || *resolved != hf { Some(hf) } else { None };
            decided.push(headers.clone());
            out.push(Section { page, headers });
        }
        let mut it = out.into_iter();
        for b in blocks.iter_mut() {
            if let Block::Para(p) = b {
                if p.section.is_some() {
                    p.section = it.next().map(Box::new);
                }
            }
        }
    }
}

fn rtf_hyperlink(instr: &str) -> Option<String> {
    let rest = instr.trim().get(9..)?.trim();
    let quoted = |s: &str| -> Option<String> {
        let s = s.trim();
        if let Some(r) = s.strip_prefix('"') {
            r.split('"').next().map(|x| x.to_string())
        } else {
            s.split_whitespace().next().map(|x| x.to_string())
        }
    };
    if let Some(i) = rest.find("\\l") {
        let anchor = quoted(&rest[i + 2..])?;
        let before = rest[..i].trim();
        return Some(if before.is_empty() { format!("#{}", anchor) } else { format!("{}#{}", quoted(before)?, anchor) });
    }
    quoted(rest)
}
