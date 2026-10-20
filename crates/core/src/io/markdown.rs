//! Markdown (CommonMark + GitHub extensions) export and import.
//!
//! Export: headings, bold/italic/strike, inline code and fenced code blocks, links, images (as
//! `data:` URIs), nested lists, block quotes, GFM pipe tables, footnotes (`[^1]`) and rules.
//!
//! Import: ATX/setext headings, paragraphs with soft/hard breaks, emphasis, strike, code spans,
//! fenced/indented code, block quotes, nested bullet/ordered lists, GFM tables with alignment,
//! links (inline, reference and autolinks), images, footnotes, horizontal rules, backslash escapes,
//! entities and simple inline/block HTML.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::io::util::{base64_decode, base64_encode, image_data, image_mime, natural_size};
use crate::model::*;
use crate::styles::{resolve_chars, resolve_para};

const MONO: &[&str] = &["courier new", "courier", "consolas", "menlo", "monaco", "lucida console", "source code pro", "dejavu sans mono", "liberation mono", "monospace"];

fn is_mono(font: &str) -> bool {
    MONO.contains(&font.to_ascii_lowercase().as_str())
}

// ===================================================================================================
// Writer
// ===================================================================================================

/// Write the document as Markdown.
pub fn write(doc: &Document) -> String {
    let mut w = MdWriter { doc, counters: HashMap::new(), notes: Vec::new(), last_list: None };
    let mut out = String::new();
    w.blocks(&mut out, &doc.body);
    if !w.notes.is_empty() {
        out.push('\n');
        let notes = std::mem::take(&mut w.notes);
        for (n, (foot, id)) in notes.iter().enumerate() {
            let list = if *foot { &doc.footnotes } else { &doc.endnotes };
            let paras = list.iter().find(|x| x.id == *id).map(|x| x.paragraphs.clone()).unwrap_or_default();
            let mut texts = Vec::new();
            for p in &paras {
                let mut q = p.clone();
                q.runs.retain(|r| !matches!(r.content, Inline::FootnoteRef(_) | Inline::EndnoteRef(_)));
                texts.push(w.inline(&q).trim().to_string());
            }
            let _ = writeln!(out, "[^{}]: {}", n + 1, texts.join("\n\n    "));
        }
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

struct MdWriter<'a> {
    doc: &'a Document,
    counters: HashMap<u32, [u32; 9]>,
    /// Notes in order of reference: (is footnote, id).
    notes: Vec<(bool, u32)>,
    last_list: Option<u32>,
}

#[derive(Clone, Copy, PartialEq, Default)]
struct Flags {
    bold: bool,
    italic: bool,
    strike: bool,
}

/// Escape Markdown punctuation in running text.
fn esc_text(s: &str, table: bool) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '*' | '_' | '`' | '[' | ']' | '<' | '~' => {
                o.push('\\');
                o.push(c);
            }
            '|' if table => o.push_str("\\|"),
            '\u{A0}' => o.push_str("&nbsp;"),
            c => o.push(c),
        }
    }
    o
}

/// Escape characters that would start a block construct at the beginning of a line.
fn esc_line_start(s: &str) -> String {
    let t = s.trim_start();
    let lead = &s[..s.len() - t.len()];
    let needs = t.starts_with('#')
        || t.starts_with('>')
        || ((t.starts_with('-') || t.starts_with('+')) && (t.len() == 1 || t[1..].starts_with(' ')))
        || t.starts_with("===")
        || t.starts_with("---")
        || {
            let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
            digits > 0 && digits < 10 && (t[digits..].starts_with(". ") || t[digits..].starts_with(") ") || t[digits..] == *"." || t[digits..] == *")")
        };
    if !needs {
        return s.to_string();
    }
    if t.starts_with(|c: char| c.is_ascii_digit()) {
        let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
        return format!("{}{}\\{}", lead, &t[..digits], &t[digits..]);
    }
    format!("{}\\{}", lead, t)
}

impl<'a> MdWriter<'a> {
    fn blocks(&mut self, out: &mut String, blocks: &[Block]) {
        let mut i = 0;
        let mut in_list = false;
        while i < blocks.len() {
            match &blocks[i] {
                Block::Table(t) => {
                    if in_list {
                        out.push('\n');
                        in_list = false;
                    }
                    self.table(out, t);
                    out.push('\n');
                    i += 1;
                }
                Block::Para(p) => {
                    let rp = resolve_para(self.doc, &p.props);
                    if let Some(l) = rp.list {
                        // A different list at the top level is separated by a blank line.
                        if in_list && l.level == 0 && self.last_list != Some(l.list) {
                            out.push('\n');
                        }
                        if l.level == 0 {
                            self.last_list = Some(l.list);
                        }
                        self.list_item(out, p, l);
                        in_list = true;
                        i += 1;
                        continue;
                    }
                    if in_list {
                        out.push('\n');
                        in_list = false;
                    }
                    // Fenced code: consecutive paragraphs set entirely in a monospace font.
                    if self.is_code(p) {
                        let mut lines = Vec::new();
                        while i < blocks.len() {
                            match &blocks[i] {
                                Block::Para(q) if self.is_code(q) && resolve_para(self.doc, &q.props).list.is_none() => {
                                    lines.push(code_text(q));
                                    i += 1;
                                }
                                _ => break,
                            }
                        }
                        let body = lines.join("\n");
                        let fence = if body.contains("```") { "~~~~" } else { "```" };
                        let _ = writeln!(out, "{}\n{}\n{}\n", fence, body, fence);
                        continue;
                    }
                    self.paragraph(out, p, &rp.style);
                    i += 1;
                }
            }
        }
        if in_list {
            out.push('\n');
        }
    }

    fn is_code(&self, p: &Paragraph) -> bool {
        let style = p.style_id();
        let mut any = false;
        for r in &p.runs {
            match &r.content {
                Inline::Text(t) => {
                    if t.trim().is_empty() {
                        continue;
                    }
                    let rc = resolve_chars(self.doc, style, &r.props);
                    if !is_mono(&rc.font) || r.props.link.is_some() {
                        return false;
                    }
                    any = true;
                }
                Inline::Tab | Inline::LineBreak | Inline::Bookmark(_) => {}
                _ => return false,
            }
        }
        any
    }

    fn paragraph(&mut self, out: &mut String, p: &Paragraph, style: &str) {
        let text = self.inline(p);
        let heading = match style {
            "Title" | "Heading1" => Some(1),
            "Heading2" => Some(2),
            "Heading3" => Some(3),
            "Heading4" => Some(4),
            "Heading5" => Some(5),
            "Heading6" => Some(6),
            _ => None,
        };
        // A rule: an empty paragraph with only a bottom border.
        if text.trim().is_empty() {
            let rp = resolve_para(self.doc, &p.props);
            if rp.borders.bottom.map(|b| b.is_visible()).unwrap_or(false) {
                out.push_str("---\n\n");
            }
            return;
        }
        if let Some(h) = heading {
            let one_line = text.replace("\\\n", " ");
            let _ = writeln!(out, "{} {}\n", "#".repeat(h), one_line.trim());
            return;
        }
        let text = esc_line_start(&text);
        if matches!(style, "Quote" | "IntenseQuote") {
            for line in text.split('\n') {
                let _ = writeln!(out, "> {}", line);
            }
            out.push('\n');
            return;
        }
        let _ = writeln!(out, "{}\n", text);
    }

    fn list_item(&mut self, out: &mut String, p: &Paragraph, l: ListRef) {
        let def = self.doc.list(l.list);
        let level = (l.level as usize).min(8);
        let lvl = def.and_then(|d| d.levels.get(level));
        let ordered = lvl.map(|x| !matches!(x.format, NumFormat::Bullet | NumFormat::None)).unwrap_or(false);
        let start = lvl.map(|x| x.start.max(1)).unwrap_or(1);
        let c = self.counters.entry(l.list).or_insert([0; 9]);
        for slot in c.iter_mut().skip(level + 1) {
            *slot = 0;
        }
        c[level] = if c[level] == 0 { start } else { c[level] + 1 };
        let n = c[level];
        let indent = "    ".repeat(level);
        let marker = if ordered { format!("{}.", n) } else { "-".to_string() };
        let cont = format!("{}{}", indent, " ".repeat(marker.len() + 1));
        let text = esc_line_start(&self.inline(p));
        let text = text.replace('\n', &format!("\n{}", cont));
        let _ = writeln!(out, "{}{} {}", indent, marker, text);
    }

    fn table(&mut self, out: &mut String, t: &Table) {
        let ncols = t.columns.len().max(t.rows.iter().map(|r| r.cells.iter().map(|c| c.span.max(1) as usize).sum::<usize>()).max().unwrap_or(1));
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut aligns = vec![Align::Left; ncols];
        for (ri, row) in t.rows.iter().enumerate() {
            let mut cells = Vec::new();
            for cell in &row.cells {
                let text = if cell.vmerge == VMerge::Continue {
                    String::new()
                } else {
                    let parts: Vec<String> = cell.paragraphs.iter().map(|p| self.inline_table(p)).filter(|s| !s.is_empty()).collect();
                    parts.join("<br>")
                };
                if ri == 0 {
                    if let (Some(p), Some(a)) = (cell.paragraphs.first(), aligns.get_mut(cells.len())) {
                        *a = resolve_para(self.doc, &p.props).align;
                    }
                }
                cells.push(text);
                for _ in 1..cell.span.max(1) {
                    cells.push(String::new());
                }
            }
            cells.resize(ncols, String::new());
            rows.push(cells);
        }
        if rows.is_empty() {
            return;
        }
        let line = |cells: &[String]| format!("| {} |", cells.iter().map(|c| if c.is_empty() { " ".to_string() } else { c.clone() }).collect::<Vec<_>>().join(" | "));
        let _ = writeln!(out, "{}", line(&rows[0]));
        let delim: Vec<String> = aligns
            .iter()
            .map(|a| match a {
                Align::Center => ":---:".to_string(),
                Align::Right => "---:".to_string(),
                _ => "---".to_string(),
            })
            .collect();
        let _ = writeln!(out, "|{}|", delim.iter().map(|d| format!(" {} ", d)).collect::<Vec<_>>().join("|"));
        for r in rows.iter().skip(1) {
            let _ = writeln!(out, "{}", line(r));
        }
    }

    fn inline_table(&mut self, p: &Paragraph) -> String {
        self.inline_impl(p, true).replace("\\\n", "<br>")
    }

    fn inline(&mut self, p: &Paragraph) -> String {
        self.inline_impl(p, false)
    }

    /// Inline Markdown for a paragraph's runs.
    fn inline_impl(&mut self, p: &Paragraph, table: bool) -> String {
        let style = p.style_id().to_string();
        let base = resolve_chars(self.doc, &style, &CharProps::default());
        let mut out = String::new();
        let mut open = Flags::default();
        let mut pending_ws = String::new();
        let mut i = 0;
        // Formatting delimiters: closers go before pending whitespace, openers after it.
        fn close_to(out: &mut String, open: &mut Flags, want: Flags) {
            let close_bold = open.bold && !want.bold;
            let close_italic = open.italic && (!want.italic || close_bold);
            let close_strike = open.strike && (!want.strike || close_bold || close_italic);
            if close_strike {
                out.push_str("~~");
                open.strike = false;
            }
            if close_italic {
                out.push('*');
                open.italic = false;
            }
            if close_bold {
                out.push_str("**");
                open.bold = false;
            }
        }
        fn open_to(out: &mut String, open: &mut Flags, want: Flags) {
            if want.bold && !open.bold {
                out.push_str("**");
                open.bold = true;
            }
            if want.italic && !open.italic {
                out.push('*');
                open.italic = true;
            }
            if want.strike && !open.strike {
                out.push_str("~~");
                open.strike = true;
            }
        }
        fn set_flags(out: &mut String, open: &mut Flags, want: Flags, ws: &mut String) {
            close_to(out, open, want);
            out.push_str(&ws.replace('\n', "\\\n").replace('\t', " "));
            ws.clear();
            open_to(out, open, want);
        }
        while i < p.runs.len() {
            let r = &p.runs[i];
            if let Some(rev) = &r.props.revision {
                if rev.kind == RevisionKind::Delete {
                    i += 1;
                    continue;
                }
            }
            // Links group consecutive runs.
            if let Some(link) = r.props.link.clone() {
                let mut j = i;
                let mut inner = Paragraph::new();
                while j < p.runs.len() && p.runs[j].props.link.as_deref() == Some(link.as_str()) {
                    let mut q = p.runs[j].clone();
                    q.props.link = None;
                    if q.props.style.as_deref() == Some("Hyperlink") {
                        q.props.style = None;
                        q.props.color = None;
                        q.props.underline = None;
                    }
                    inner.runs.push(q);
                    j += 1;
                }
                inner.props = p.props.clone();
                let text = self.inline_impl(&inner, table);
                set_flags(&mut out, &mut open, Flags::default(), &mut pending_ws);
                let url = link.replace(' ', "%20").replace('(', "%28").replace(')', "%29");
                let _ = write!(out, "[{}]({})", text.trim(), url);
                i = j;
                continue;
            }
            let rc = resolve_chars(self.doc, &style, &r.props);
            let flags = Flags { bold: rc.bold && !base.bold, italic: rc.italic && !base.italic, strike: rc.strike || rc.double_strike };
            match &r.content {
                Inline::Text(t) => {
                    if rc.hidden {
                        i += 1;
                        continue;
                    }
                    if is_mono(&rc.font) && !is_mono(&base.font) && !t.trim().is_empty() {
                        let core = t.trim();
                        let lead = &t[..t.len() - t.trim_start().len()];
                        let trail = &t[t.trim_end().len()..];
                        pending_ws.push_str(lead);
                        set_flags(&mut out, &mut open, flags, &mut pending_ws);
                        let mut ticks = 1;
                        while core.contains(&"`".repeat(ticks)) {
                            ticks += 1;
                        }
                        let fence = "`".repeat(ticks);
                        let pad = if core.starts_with('`') || core.ends_with('`') { " " } else { "" };
                        let _ = write!(out, "{fence}{pad}{core}{pad}{fence}");
                        pending_ws.push_str(trail);
                        i += 1;
                        continue;
                    }
                    // Superscript, subscript and underline have no Markdown syntax: inline HTML.
                    let mut tags: Vec<&str> = Vec::new();
                    match rc.vert_align {
                        VertAlign::Superscript => tags.push("sup"),
                        VertAlign::Subscript => tags.push("sub"),
                        VertAlign::Baseline => {}
                    }
                    if rc.underline != Underline::None && rc.underline != base.underline {
                        tags.push("u");
                    }
                    if !tags.is_empty() && !t.trim().is_empty() {
                        let core = t.trim();
                        pending_ws.push_str(&t[..t.len() - t.trim_start().len()]);
                        set_flags(&mut out, &mut open, flags, &mut pending_ws);
                        for tg in &tags {
                            let _ = write!(out, "<{}>", tg);
                        }
                        out.push_str(&esc_text(core, table));
                        for tg in tags.iter().rev() {
                            let _ = write!(out, "</{}>", tg);
                        }
                        pending_ws.push_str(&t[t.trim_end().len()..]);
                        i += 1;
                        continue;
                    }
                    // Split into whitespace and non-whitespace pieces.
                    let mut rest = t.as_str();
                    while !rest.is_empty() {
                        let ws_len = rest.len() - rest.trim_start().len();
                        if ws_len > 0 {
                            pending_ws.push_str(&rest[..ws_len]);
                            rest = &rest[ws_len..];
                            continue;
                        }
                        let word_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                        set_flags(&mut out, &mut open, flags, &mut pending_ws);
                        out.push_str(&esc_text(&rest[..word_end], table));
                        rest = &rest[word_end..];
                    }
                }
                Inline::Tab => pending_ws.push('\t'),
                Inline::LineBreak | Inline::ColumnBreak | Inline::PageBreak => {
                    pending_ws.clear();
                    set_flags(&mut out, &mut open, Flags::default(), &mut pending_ws);
                    out.push_str("\\\n");
                }
                Inline::TextBox(_) => {}
                Inline::Image(img) => {
                    set_flags(&mut out, &mut open, Flags::default(), &mut pending_ws);
                    let _ = write!(out, "![{}](data:{};base64,{})", esc_text(&img.alt, table), image_mime(&img.data.format), base64_encode(&img.data.bytes));
                }
                Inline::Field(f) => {
                    let text = match f {
                        Field::PageNumber | Field::PageCount => "1".to_string(),
                        Field::Date(pic) => crate::io::util::format_date_now(pic),
                        Field::Other { result, .. } => result.clone(),
                    };
                    set_flags(&mut out, &mut open, flags, &mut pending_ws);
                    out.push_str(&esc_text(&text, table));
                }
                Inline::FootnoteRef(id) | Inline::EndnoteRef(id) => {
                    let key = (matches!(r.content, Inline::FootnoteRef(_)), *id);
                    let n = match self.notes.iter().position(|x| *x == key) {
                        Some(n) => n + 1,
                        None => {
                            self.notes.push(key);
                            self.notes.len()
                        }
                    };
                    set_flags(&mut out, &mut open, Flags::default(), &mut pending_ws);
                    let _ = write!(out, "[^{}]", n);
                }
                Inline::Bookmark(_) => {}
            }
            i += 1;
        }
        close_to(&mut out, &mut open, Flags::default());
        out
    }
}

fn code_text(p: &Paragraph) -> String {
    let mut s = String::new();
    for r in &p.runs {
        match &r.content {
            Inline::Text(t) => s.push_str(t),
            Inline::Tab => s.push('\t'),
            Inline::LineBreak => s.push('\n'),
            _ => {}
        }
    }
    s
}

// ===================================================================================================
// Reader
// ===================================================================================================

/// Parse Markdown into a document.
pub fn read(src: &str) -> Document {
    let mut doc = Document::new();
    let src = src.trim_start_matches('\u{FEFF}').replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = src.split('\n').collect();
    let mut r = MdReader { doc: &mut doc, refs: HashMap::new(), note_labels: HashMap::new(), notes: Vec::new() };
    // Collect link reference definitions and footnote definitions first.
    let lines = r.collect_definitions(&lines);
    let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    let blocks = r.blocks(&refs);
    let notes = std::mem::take(&mut r.notes);
    drop(r);
    doc.body = blocks;
    for (id, paras) in notes {
        doc.footnotes.push(Note { id, paragraphs: paras });
    }
    if doc.body.is_empty() {
        doc.body.push(Block::Para(Paragraph::new()));
    }
    doc
}

struct MdReader<'d> {
    doc: &'d mut Document,
    refs: HashMap<String, String>,
    note_labels: HashMap<String, u32>,
    notes: Vec<(u32, Vec<Paragraph>)>,
}

fn indent_of(line: &str) -> usize {
    let mut n = 0;
    for c in line.chars() {
        match c {
            ' ' => n += 1,
            '\t' => n += 4 - n % 4,
            _ => break,
        }
    }
    n
}

/// Remove up to `n` columns of indentation.
fn dedent(line: &str, n: usize) -> &str {
    let mut col = 0;
    for (i, c) in line.char_indices() {
        if col >= n {
            return &line[i..];
        }
        match c {
            ' ' => col += 1,
            '\t' => col += 4 - col % 4,
            _ => return &line[i..],
        }
    }
    ""
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

fn is_hr(line: &str) -> bool {
    if indent_of(line) > 3 {
        return false;
    }
    let t: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3 && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*') || t.chars().all(|c| c == '_'))
}

fn atx_heading(line: &str) -> Option<(usize, String)> {
    if indent_of(line) > 3 {
        return None;
    }
    let t = line.trim_start();
    let level = t.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &t[level..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let mut text = rest.trim().to_string();
    // Optional closing sequence.
    let trimmed = text.trim_end_matches('#');
    if trimmed.is_empty() || trimmed.ends_with(' ') {
        text = trimmed.trim_end().to_string();
    }
    Some((level, text))
}

fn fence_start(line: &str) -> Option<(char, usize, usize)> {
    let ind = indent_of(line);
    if ind > 3 {
        return None;
    }
    let t = line.trim_start();
    let ch = t.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let n = t.chars().take_while(|c| *c == ch).count();
    if n < 3 || (ch == '`' && t[n..].contains('`')) {
        return None;
    }
    Some((ch, n, ind))
}

/// A list item marker: (marker indent, content indent, ordered, number, rest of line).
fn list_marker(line: &str) -> Option<(usize, usize, bool, u32, &str)> {
    let ind = indent_of(line);
    let t = line.trim_start();
    let (ordered, num, mlen) = if t.starts_with(['-', '*', '+']) {
        (false, 0, 1)
    } else {
        let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 || digits > 9 || !t[digits..].starts_with(['.', ')']) {
            return None;
        }
        (true, t[..digits].parse().unwrap_or(1), digits + 1)
    };
    let rest = &t[mlen..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let spaces = rest.chars().take_while(|c| *c == ' ').count();
    let spaces = if spaces == 0 || spaces > 4 { 1 } else { spaces };
    let content = rest.get(spaces.min(rest.len())..).unwrap_or("");
    Some((ind, ind + mlen + spaces, ordered, num, content))
}

fn table_delim(line: &str) -> Option<Vec<Align>> {
    let t = line.trim();
    if !t.contains('-') || !t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t')) {
        return None;
    }
    let cells = split_row(t);
    let mut out = Vec::new();
    for c in cells {
        let c = c.trim();
        if c.is_empty() || !c.contains('-') {
            return None;
        }
        out.push(match (c.starts_with(':'), c.ends_with(':')) {
            (true, true) => Align::Center,
            (false, true) => Align::Right,
            _ => Align::Left,
        });
    }
    Some(out)
}

fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = if t.ends_with('|') && !t.ends_with("\\|") { &t[..t.len() - 1] } else { t };
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    let mut in_code = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cur.push('|');
                chars.next();
            }
            '`' => {
                in_code = !in_code;
                cur.push(c);
            }
            '|' if !in_code => cells.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    cells.push(cur);
    cells
}

impl<'d> MdReader<'d> {
    fn collect_definitions(&mut self, lines: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        let mut i = 0;
        let mut in_fence: Option<(char, usize)> = None;
        while i < lines.len() {
            let line = lines[i];
            if let Some((ch, n)) = in_fence {
                let t = line.trim_start();
                if t.chars().take_while(|c| *c == ch).count() >= n && t.trim_start_matches(ch).trim().is_empty() {
                    in_fence = None;
                }
                out.push(line.to_string());
                i += 1;
                continue;
            }
            if let Some((ch, n, _)) = fence_start(line) {
                in_fence = Some((ch, n));
                out.push(line.to_string());
                i += 1;
                continue;
            }
            let t = line.trim_start();
            if indent_of(line) <= 3 && t.starts_with('[') {
                if let Some(close) = t.find("]:") {
                    let label = &t[1..close];
                    let rest = t[close + 2..].trim();
                    if let Some(note) = label.strip_prefix('^') {
                        // Footnote definition with indented continuation lines.
                        let mut text = vec![rest.to_string()];
                        i += 1;
                        while i < lines.len() && (indent_of(lines[i]) >= 4 || (is_blank(lines[i]) && i + 1 < lines.len() && indent_of(lines[i + 1]) >= 4)) {
                            text.push(dedent(lines[i], 4).to_string());
                            i += 1;
                        }
                        let id = self.note_id(note);
                        let body: Vec<&str> = text.iter().map(|s| s.as_str()).collect();
                        let blocks = self.blocks(&body);
                        let mut paras: Vec<Paragraph> = blocks.into_iter().filter_map(|b| if let Block::Para(p) = b { Some(p) } else { None }).collect();
                        for p in &mut paras {
                            p.props.style = Some("FootnoteText".into());
                        }
                        if let Some(p) = paras.first_mut() {
                            p.runs.insert(0, Run::text(" "));
                            p.runs.insert(0, Run { content: Inline::FootnoteRef(id), props: CharProps { style: Some("FootnoteReference".into()), ..Default::default() } });
                        }
                        self.notes.push((id, paras));
                        continue;
                    }
                    if !label.is_empty() && !rest.is_empty() && !label.contains(']') {
                        let url = rest.split_whitespace().next().unwrap_or("").trim_start_matches('<').trim_end_matches('>');
                        self.refs.insert(label.to_lowercase(), url.to_string());
                        i += 1;
                        continue;
                    }
                }
            }
            out.push(line.to_string());
            i += 1;
        }
        out
    }

    fn note_id(&mut self, label: &str) -> u32 {
        let n = self.note_labels.len() as u32 + 1;
        *self.note_labels.entry(label.to_string()).or_insert(n)
    }

    /// Parse block structure.
    fn blocks(&mut self, lines: &[&str]) -> Vec<Block> {
        let mut out: Vec<Block> = Vec::new();
        let mut para: Vec<&str> = Vec::new();
        let mut i = 0;
        let flush = |me: &mut Self, para: &mut Vec<&str>, out: &mut Vec<Block>| {
            if !para.is_empty() {
                let p = me.paragraph(para, ParaProps::default());
                out.push(Block::Para(p));
                para.clear();
            }
        };
        while i < lines.len() {
            let line = lines[i];
            if is_blank(line) {
                flush(self, &mut para, &mut out);
                i += 1;
                continue;
            }
            // Setext heading underline.
            if !para.is_empty() && indent_of(line) <= 3 {
                let t = line.trim();
                if !t.is_empty() && (t.chars().all(|c| c == '=') || (t.chars().all(|c| c == '-') && para.len() == 1)) {
                    let level = if t.starts_with('=') { 1 } else { 2 };
                    let mut p = self.paragraph(&para, ParaProps::default());
                    p.props.style = Some(format!("Heading{}", level));
                    para.clear();
                    out.push(Block::Para(p));
                    i += 1;
                    continue;
                }
            }
            if let Some((level, text)) = atx_heading(line) {
                flush(self, &mut para, &mut out);
                let mut p = self.paragraph(&[text.as_str()], ParaProps::default());
                p.props.style = Some(format!("Heading{}", level));
                out.push(Block::Para(p));
                i += 1;
                continue;
            }
            if is_hr(line) {
                flush(self, &mut para, &mut out);
                let mut p = Paragraph::new();
                let b = Border { style: BorderStyle::Single, width: 0.75, color: Color::rgb(0xA0, 0xA0, 0xA0), space: 1.0 };
                p.props.borders = Some(Borders { bottom: Some(b), ..Default::default() });
                out.push(Block::Para(p));
                i += 1;
                continue;
            }
            if let Some((ch, n, ind)) = fence_start(line) {
                flush(self, &mut para, &mut out);
                i += 1;
                let mut code = Vec::new();
                while i < lines.len() {
                    let t = lines[i].trim_start();
                    if indent_of(lines[i]) <= 3 && t.chars().take_while(|c| *c == ch).count() >= n && t.trim_start_matches(ch).trim().is_empty() {
                        i += 1;
                        break;
                    }
                    code.push(dedent(lines[i], ind));
                    i += 1;
                }
                out.extend(code_blocks(&code));
                continue;
            }
            if para.is_empty() && indent_of(line) >= 4 {
                // Indented code block.
                let mut code = Vec::new();
                while i < lines.len() && (indent_of(lines[i]) >= 4 || is_blank(lines[i])) {
                    code.push(dedent(lines[i], 4));
                    i += 1;
                }
                while code.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
                    code.pop();
                }
                out.extend(code_blocks(&code));
                continue;
            }
            if indent_of(line) <= 3 && line.trim_start().starts_with('>') {
                flush(self, &mut para, &mut out);
                let mut inner: Vec<String> = Vec::new();
                while i < lines.len() && !is_blank(lines[i]) {
                    let t = lines[i].trim_start();
                    if let Some(rest) = t.strip_prefix('>') {
                        inner.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
                    } else if !inner.is_empty() && !is_hr(lines[i]) && atx_heading(lines[i]).is_none() && list_marker(lines[i]).is_none() {
                        inner.push(lines[i].to_string()); // lazy continuation
                    } else {
                        break;
                    }
                    i += 1;
                }
                let refs: Vec<&str> = inner.iter().map(|s| s.as_str()).collect();
                let blocks = self.blocks(&refs);
                for mut b in blocks {
                    if let Block::Para(p) = &mut b {
                        if p.props.style.is_none() {
                            p.props.style = Some("Quote".into());
                        }
                    }
                    out.push(b);
                }
                continue;
            }
            if let Some((_, _, ordered, _, _)) = list_marker(line) {
                // An ordered list interrupting a paragraph must start at 1 (CommonMark).
                let interrupts_ok = para.is_empty() || !ordered || list_marker(line).map(|m| m.3 == 1).unwrap_or(false);
                let empty_item = list_marker(line).map(|m| m.4.trim().is_empty()).unwrap_or(true);
                if interrupts_ok && !(empty_item && !para.is_empty()) {
                    flush(self, &mut para, &mut out);
                    i = self.list(lines, i, &mut out);
                    continue;
                }
            }
            if para.is_empty() && line.contains('|') && i + 1 < lines.len() {
                if let Some(aligns) = table_delim(lines[i + 1]) {
                    let header = split_row(line);
                    if header.len() == aligns.len() {
                        let mut rows = vec![header];
                        i += 2;
                        while i < lines.len() && !is_blank(lines[i]) && lines[i].contains('|') {
                            rows.push(split_row(lines[i]));
                            i += 1;
                        }
                        out.push(Block::Table(self.table(rows, &aligns)));
                        continue;
                    }
                }
            }
            if para.is_empty() && indent_of(line) <= 3 && is_html_block_start(line.trim_start()) {
                let mut html = String::new();
                while i < lines.len() && !is_blank(lines[i]) {
                    html.push_str(lines[i]);
                    html.push('\n');
                    i += 1;
                }
                let blocks = crate::io::html::read_fragment_into(&html, self.doc);
                out.extend(blocks);
                continue;
            }
            para.push(line);
            i += 1;
        }
        flush(self, &mut para, &mut out);
        out
    }

    /// Parse a list starting at `start`; returns the index after it.
    fn list(&mut self, lines: &[&str], start: usize, out: &mut Vec<Block>) -> usize {
        // Stack of open levels: (marker indent, content indent, ordered, list id).
        let mut stack: Vec<(usize, usize, bool)> = Vec::new();
        let mut list_id = 0u32;
        let mut i = start;
        let mut item: Vec<String> = Vec::new();
        let mut item_level: Option<u8> = None;
        let mut blank_before = false;
        let flush_item = |me: &mut Self, item: &mut Vec<String>, level: Option<u8>, list_id: u32, out: &mut Vec<Block>| {
            if let Some(level) = level {
                let refs: Vec<&str> = item.iter().map(|s| s.as_str()).collect();
                // The first paragraph is the list item; further blocks are indented continuations.
                let blocks = me.blocks(&refs);
                let mut first = true;
                for b in blocks {
                    match b {
                        Block::Para(mut p) if first => {
                            p.props.list = Some(Some(ListRef { list: list_id, level }));
                            out.push(Block::Para(p));
                        }
                        Block::Para(mut p) => {
                            if p.props.list.is_none() {
                                p.props.indent_left = Some(36.0 * (level as f32 + 1.0));
                            } else if let Some(Some(l)) = &mut p.props.list {
                                // Nested list parsed inside the item (only when indentation is unusual).
                                l.level = (l.level + level + 1).min(8);
                            }
                            out.push(Block::Para(p));
                        }
                        other => out.push(other),
                    }
                    first = false;
                }
                if first {
                    let mut p = Paragraph::new();
                    p.props.list = Some(Some(ListRef { list: list_id, level }));
                    out.push(Block::Para(p));
                }
            }
            item.clear();
        };
        while i < lines.len() {
            let line = lines[i];
            if is_blank(line) {
                // A blank line ends the list unless an indented continuation or another item follows.
                let next = lines[i + 1..].iter().find(|l| !is_blank(l));
                let continues = match next {
                    Some(n) => list_marker(n).is_some() && indent_of(n) <= stack.last().map(|s| s.1).unwrap_or(0) + 3 && !is_hr(n) || stack.last().map(|s| indent_of(n) >= s.1).unwrap_or(false),
                    None => false,
                };
                if !continues {
                    break;
                }
                blank_before = true;
                item.push(String::new());
                i += 1;
                continue;
            }
            let marker = if is_hr(line) && !(line.trim_start().starts_with(['-', '*']) && list_marker(line).map(|m| !m.4.trim().is_empty()).unwrap_or(false)) {
                None
            } else {
                list_marker(line)
            };
            if let Some((mi, ci, ordered, num, rest)) = marker {
                // Indented beyond the current item's content: nested list item.
                while let Some(top) = stack.last() {
                    if mi < top.0 {
                        stack.pop();
                    } else {
                        break;
                    }
                }
                let level;
                match stack.last() {
                    Some(top) if mi >= top.1 => {
                        // Nested.
                        if stack.len() >= 9 {
                            level = 8;
                        } else {
                            stack.push((mi, ci, ordered));
                            level = (stack.len() - 1) as u8;
                            self.set_level(list_id, level, ordered, num);
                        }
                    }
                    Some(top) => {
                        // Sibling at this level (a different marker kind at level 0 starts a new list).
                        let lvl = (stack.len() - 1) as u8;
                        if top.2 != ordered && lvl == 0 {
                            flush_item(self, &mut item, item_level, list_id, out);
                            item_level = None;
                            list_id = self.new_list(ordered);
                            self.set_level(list_id, 0, ordered, num);
                        } else if top.2 != ordered {
                            self.set_level(list_id, lvl, ordered, num);
                        }
                        let n = stack.len();
                        stack[n - 1] = (mi, ci, ordered);
                        level = lvl;
                    }
                    None => {
                        if list_id == 0 {
                            list_id = self.new_list(ordered);
                        }
                        stack.push((mi, ci, ordered));
                        level = 0;
                        self.set_level(list_id, 0, ordered, num);
                    }
                }
                flush_item(self, &mut item, item_level, list_id, out);
                item_level = Some(level);
                item.push(rest.to_string());
                blank_before = false;
                i += 1;
                continue;
            }
            // Continuation of the current item.
            let ci = stack.last().map(|s| s.1).unwrap_or(0);
            if indent_of(line) >= ci {
                item.push(dedent(line, ci).to_string());
                i += 1;
                continue;
            }
            if !blank_before && atx_heading(line).is_none() && fence_start(line).is_none() && !line.trim_start().starts_with('>') {
                // Lazy continuation line.
                item.push(line.trim_start().to_string());
                i += 1;
                continue;
            }
            break;
        }
        flush_item(self, &mut item, item_level, list_id, out);
        i
    }

    fn new_list(&mut self, ordered: bool) -> u32 {
        let id = self.doc.lists.iter().map(|l| l.id).max().unwrap_or(0) + 1;
        self.doc.lists.push(if ordered { ListDef::numbered(id) } else { ListDef::bullets(id) });
        id
    }

    fn set_level(&mut self, list: u32, level: u8, ordered: bool, num: u32) {
        let Some(def) = self.doc.lists.iter_mut().find(|l| l.id == list) else { return };
        let Some(lvl) = def.levels.get_mut(level as usize) else { return };
        if ordered {
            if lvl.format == NumFormat::Bullet {
                lvl.format = NumFormat::Decimal;
                lvl.text = format!("%{}.", level + 1);
            }
            lvl.start = num.max(1);
        } else if lvl.format != NumFormat::Bullet {
            lvl.format = NumFormat::Bullet;
            lvl.text = ["•", "◦", "▪"][level as usize % 3].to_string();
            lvl.font = None;
        }
    }

    fn table(&mut self, rows: Vec<Vec<String>>, aligns: &[Align]) -> Table {
        let ncols = aligns.len().max(1);
        let w = (468.0 / ncols as f32 * 100.0).round() / 100.0;
        let mut t = Table { columns: vec![w; ncols], ..Default::default() };
        for (ri, cells) in rows.into_iter().enumerate() {
            let mut row = Row { cells: Vec::new(), height: None, header: ri == 0 };
            for (c, align) in aligns.iter().enumerate().take(ncols) {
                let text = cells.get(c).map(|s| s.trim()).unwrap_or("");
                let mut props = ParaProps { space_after: Some(0.0), ..Default::default() };
                if *align != Align::Left {
                    props.align = Some(*align);
                }
                let mut chars = CharProps::default();
                if ri == 0 {
                    chars.bold = Some(true);
                }
                let mut paras = Vec::new();
                for part in split_br(text) {
                    let mut p = Paragraph { props: props.clone(), ..Default::default() };
                    p.runs = self.inline(&part, &chars);
                    paras.push(p);
                }
                row.cells.push(Cell { paragraphs: paras, ..Default::default() });
            }
            t.rows.push(row);
        }
        t
    }

    fn paragraph(&mut self, lines: &[&str], props: ParaProps) -> Paragraph {
        // Join lines: soft breaks become spaces, hard breaks (two spaces or backslash) line breaks.
        let mut text = String::new();
        for (k, l) in lines.iter().enumerate() {
            let last = k + 1 == lines.len();
            let l = l.trim_start();
            if last {
                text.push_str(l.trim_end());
            } else if l.ends_with("  ") {
                text.push_str(l.trim_end());
                text.push('\n');
            } else if l.ends_with('\\') && !l.ends_with("\\\\") {
                text.push_str(&l[..l.len() - 1]);
                text.push('\n');
            } else {
                text.push_str(l.trim_end());
                text.push(' ');
            }
        }
        let mut p = Paragraph { props, ..Default::default() };
        p.runs = self.inline(&text, &CharProps::default());
        p
    }

    // ---- inline ---------------------------------------------------------------------------------

    fn inline(&mut self, text: &str, base: &CharProps) -> Vec<Run> {
        let mut runs: Vec<Run> = Vec::new();
        let chars: Vec<char> = text.chars().collect();
        let mut st = InlineState { bold: false, italic: false, strike: false, underline: false, sup: false, sub: false, mark: false };
        let mut buf = String::new();
        let mut i = 0;
        let props_of = |st: &InlineState| -> CharProps {
            let mut c = base.clone();
            if st.bold {
                c.bold = Some(true);
            }
            if st.italic {
                c.italic = Some(true);
            }
            if st.strike {
                c.strike = Some(true);
            }
            if st.underline {
                c.underline = Some(Underline::Single);
            }
            if st.sup {
                c.vert_align = Some(VertAlign::Superscript);
            }
            if st.sub {
                c.vert_align = Some(VertAlign::Subscript);
            }
            if st.mark {
                c.highlight = Some(Color::YELLOW);
            }
            c
        };
        fn push(runs: &mut Vec<Run>, content: Inline, props: CharProps) {
            if let (Inline::Text(t), Some(last)) = (&content, runs.last_mut()) {
                if last.props == props {
                    if let Inline::Text(lt) = &mut last.content {
                        lt.push_str(t);
                        return;
                    }
                }
            }
            if matches!(&content, Inline::Text(t) if t.is_empty()) {
                return;
            }
            runs.push(Run { content, props });
        }
        macro_rules! flush {
            () => {
                if !buf.is_empty() {
                    let t = crate::io::html::decode_entities(&std::mem::take(&mut buf)).replace('\u{A0}', " ");
                    push(&mut runs, Inline::Text(t), props_of(&st));
                }
            };
        }
        while i < chars.len() {
            let c = chars[i];
            match c {
                '\\' if i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() => {
                    // Escaped punctuation; '&' stays literal through entity decoding.
                    if chars[i + 1] == '&' {
                        buf.push_str("&amp;");
                    } else {
                        buf.push(chars[i + 1]);
                    }
                    i += 2;
                }
                '\n' => {
                    flush!();
                    push(&mut runs, Inline::LineBreak, props_of(&st));
                    i += 1;
                }
                '`' => {
                    let n = chars[i..].iter().take_while(|c| **c == '`').count();
                    // Find the closing run of exactly n backticks.
                    let mut j = i + n;
                    let mut found = None;
                    while j < chars.len() {
                        if chars[j] == '`' {
                            let m = chars[j..].iter().take_while(|c| **c == '`').count();
                            if m == n {
                                found = Some(j);
                                break;
                            }
                            j += m;
                        } else {
                            j += 1;
                        }
                    }
                    match found {
                        Some(j) => {
                            flush!();
                            let mut code: String = chars[i + n..j].iter().collect();
                            code = code.replace('\n', " ");
                            if code.len() >= 2 && code.starts_with(' ') && code.ends_with(' ') && !code.trim().is_empty() {
                                code = code[1..code.len() - 1].to_string();
                            }
                            let mut p = props_of(&st);
                            p.font = Some("Courier New".into());
                            push(&mut runs, Inline::Text(code), p);
                            i = j + n;
                        }
                        None => {
                            buf.extend(std::iter::repeat_n('`', n));
                            i += n;
                        }
                    }
                }
                '!' if chars.get(i + 1) == Some(&'[') => {
                    if let Some((alt, target, end)) = self.link_at(&chars, i + 1) {
                        flush!();
                        let img = make_image(&alt, &target);
                        match img {
                            Some(img) => push(&mut runs, Inline::Image(img), props_of(&st)),
                            None => buf.push_str(&format!("[{}]", alt)),
                        }
                        i = end;
                    } else {
                        buf.push('!');
                        i += 1;
                    }
                }
                '[' => {
                    // Footnote reference.
                    if chars.get(i + 1) == Some(&'^') {
                        if let Some(close) = chars[i..].iter().position(|c| *c == ']') {
                            let label: String = chars[i + 2..i + close].iter().collect();
                            if !label.is_empty() && !label.contains(char::is_whitespace) && self.note_labels.contains_key(&label) {
                                flush!();
                                let id = self.note_id(&label);
                                push(&mut runs, Inline::FootnoteRef(id), CharProps { style: Some("FootnoteReference".into()), ..Default::default() });
                                i += close + 1;
                                continue;
                            }
                        }
                    }
                    if let Some((label, target, end)) = self.link_at(&chars, i) {
                        flush!();
                        let mut base2 = props_of(&st);
                        base2.link = Some(target);
                        base2.style = Some("Hyperlink".into());
                        let inner = self.inline(&label, &base2);
                        for r in inner {
                            push(&mut runs, r.content, r.props);
                        }
                        i = end;
                    } else {
                        buf.push('[');
                        i += 1;
                    }
                }
                '<' => {
                    let rest: String = chars[i..].iter().take(400).collect();
                    // Autolink.
                    if let Some(end) = rest.find('>') {
                        let inner = &rest[1..end];
                        let is_url = (inner.starts_with("http://") || inner.starts_with("https://") || inner.starts_with("mailto:") || inner.starts_with("ftp://")) && !inner.contains(char::is_whitespace);
                        let is_mail = !inner.contains(char::is_whitespace) && inner.contains('@') && !inner.contains(':') && !inner.starts_with('/');
                        if is_url || is_mail {
                            flush!();
                            let mut p = props_of(&st);
                            p.link = Some(if is_mail { format!("mailto:{}", inner) } else { inner.to_string() });
                            p.style = Some("Hyperlink".into());
                            push(&mut runs, Inline::Text(inner.to_string()), p);
                            i += inner.chars().count() + 2;
                            continue;
                        }
                        // Simple inline HTML tags.
                        let tag = inner.trim().trim_end_matches('/').trim().to_ascii_lowercase();
                        let (closing, name) = match tag.strip_prefix('/') {
                            Some(n) => (true, n.trim().to_string()),
                            None => (false, tag.split_whitespace().next().unwrap_or("").to_string()),
                        };
                        let known = matches!(name.as_str(), "br" | "b" | "strong" | "i" | "em" | "u" | "ins" | "s" | "del" | "strike" | "sup" | "sub" | "mark" | "span" | "kbd" | "code");
                        if known {
                            flush!();
                            match name.as_str() {
                                "br" => push(&mut runs, Inline::LineBreak, props_of(&st)),
                                "b" | "strong" => st.bold = !closing,
                                "i" | "em" => st.italic = !closing,
                                "u" | "ins" => st.underline = !closing,
                                "s" | "del" | "strike" => st.strike = !closing,
                                "sup" => st.sup = !closing,
                                "sub" => st.sub = !closing,
                                "mark" => st.mark = !closing,
                                _ => {}
                            }
                            i += inner.chars().count() + 2;
                            continue;
                        }
                        if inner.starts_with("!--") {
                            if let Some(e) = rest.find("-->") {
                                flush!();
                                i += rest[..e + 3].chars().count();
                                continue;
                            }
                        }
                    }
                    buf.push('<');
                    i += 1;
                }
                '*' | '_' | '~' => {
                    let n = chars[i..].iter().take_while(|x| **x == c).count();
                    let prev = if i == 0 { ' ' } else { chars[i - 1] };
                    let next = chars.get(i + n).copied().unwrap_or(' ');
                    let left_flank = !next.is_whitespace() && (!is_punct(next) || prev.is_whitespace() || is_punct(prev));
                    let right_flank = !prev.is_whitespace() && (!is_punct(prev) || next.is_whitespace() || is_punct(next));
                    let (can_open, can_close) = if c == '_' {
                        (left_flank && (!right_flank || is_punct(prev)), right_flank && (!left_flank || is_punct(next)))
                    } else {
                        (left_flank, right_flank)
                    };
                    if c == '~' {
                        if n == 2 || n == 1 {
                            if st.strike && can_close {
                                flush!();
                                st.strike = false;
                                i += n;
                                continue;
                            }
                            if !st.strike && can_open && has_closer(&chars, i + n, c, n) {
                                flush!();
                                st.strike = true;
                                i += n;
                                continue;
                            }
                        }
                        buf.extend(std::iter::repeat_n(c, n));
                        i += n;
                        continue;
                    }
                    let mut used = 0;
                    let mut remaining = n;
                    // Closing first (inner-most: italic before bold when both are open).
                    if can_close {
                        if remaining >= 3 && st.bold && st.italic {
                            flush!();
                            st.bold = false;
                            st.italic = false;
                            used += 3;
                            remaining -= 3;
                        }
                        if remaining >= 2 && st.bold {
                            flush!();
                            st.bold = false;
                            used += 2;
                            remaining -= 2;
                        }
                        if remaining >= 1 && st.italic {
                            flush!();
                            st.italic = false;
                            used += 1;
                            remaining -= 1;
                        }
                    }
                    if can_open && remaining > 0 {
                        if remaining >= 2 && !st.bold && has_closer(&chars, i + n, c, 2) {
                            flush!();
                            st.bold = true;
                            used += 2;
                            remaining -= 2;
                        }
                        if remaining >= 1 && !st.italic && has_closer(&chars, i + n, c, 1) {
                            flush!();
                            st.italic = true;
                            used += 1;
                        }
                    }
                    if used < n {
                        buf.extend(std::iter::repeat_n(c, n - used));
                    }
                    i += n;
                }
                'h' | 'w' if starts_url(&chars, i) && (i == 0 || !chars[i - 1].is_alphanumeric()) => {
                    // GFM autolink literal.
                    let mut j = i;
                    while j < chars.len() && !chars[j].is_whitespace() && chars[j] != '<' {
                        j += 1;
                    }
                    while j > i && matches!(chars[j - 1], '.' | ',' | ':' | ';' | '!' | '?' | ')' | '*' | '_' | '~' | '\'' | '"') {
                        j -= 1;
                    }
                    let url: String = chars[i..j].iter().collect();
                    flush!();
                    let mut p = props_of(&st);
                    p.link = Some(if url.starts_with("www.") { format!("http://{}", url) } else { url.clone() });
                    p.style = Some("Hyperlink".into());
                    push(&mut runs, Inline::Text(url), p);
                    i = j;
                }
                _ => {
                    buf.push(c);
                    i += 1;
                }
            }
        }
        flush!();
        // Collapse repeated whitespace from joined lines and trim the paragraph's ends.
        if let Some(Run { content: Inline::Text(t), .. }) = runs.first_mut() {
            *t = t.trim_start().to_string();
        }
        if let Some(Run { content: Inline::Text(t), .. }) = runs.last_mut() {
            *t = t.trim_end().to_string();
        }
        runs.retain(|r| !matches!(&r.content, Inline::Text(t) if t.is_empty()));
        runs
    }

    /// `[label](target)` or `[label][ref]` / `[label]` starting at `i` ('['): (label, target, end index).
    fn link_at(&self, chars: &[char], i: usize) -> Option<(String, String, usize)> {
        // Matching ']' with nesting.
        let mut depth = 0;
        let mut j = i;
        let mut close = None;
        while j < chars.len() {
            match chars[j] {
                '\\' => j += 1,
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(j);
                        break;
                    }
                }
                '`' => {
                    // Skip code spans.
                    if let Some(k) = chars[j + 1..].iter().position(|c| *c == '`') {
                        j += k + 1;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        let close = close?;
        let label: String = chars[i + 1..close].iter().collect();
        if chars.get(close + 1) == Some(&'(') {
            let mut k = close + 2;
            let mut depth = 1;
            let start = k;
            let mut in_angle = false;
            while k < chars.len() {
                match chars[k] {
                    '\\' => k += 1,
                    '<' if k == start => in_angle = true,
                    '>' if in_angle => in_angle = false,
                    '(' if !in_angle => depth += 1,
                    ')' if !in_angle => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                k += 1;
            }
            if k >= chars.len() {
                return None;
            }
            let inner: String = chars[start..k].iter().collect();
            let inner = inner.trim();
            let url = if let Some(rest) = inner.strip_prefix('<') {
                rest.split('>').next().unwrap_or("").to_string()
            } else {
                inner.split_whitespace().next().unwrap_or("").to_string()
            };
            let url = unescape_md(&url);
            return Some((label, url, k + 1));
        }
        // Reference links.
        if chars.get(close + 1) == Some(&'[') {
            if let Some(end) = chars[close + 2..].iter().position(|c| *c == ']') {
                let r: String = chars[close + 2..close + 2 + end].iter().collect();
                let key = if r.is_empty() { label.to_lowercase() } else { r.to_lowercase() };
                if let Some(url) = self.refs.get(&key) {
                    return Some((label, url.clone(), close + 3 + end));
                }
            }
        }
        if let Some(url) = self.refs.get(&label.to_lowercase()) {
            return Some((label, url.clone(), close + 1));
        }
        None
    }
}

#[derive(Clone, Copy)]
struct InlineState {
    bold: bool,
    italic: bool,
    strike: bool,
    underline: bool,
    sup: bool,
    sub: bool,
    mark: bool,
}

fn is_punct(c: char) -> bool {
    c.is_ascii_punctuation() || (!c.is_alphanumeric() && !c.is_whitespace())
}

/// Whether a closing delimiter run of `c` of at least `n` exists after `from` (same line or later).
fn has_closer(chars: &[char], from: usize, c: char, n: usize) -> bool {
    let mut j = from;
    while j < chars.len() {
        if chars[j] == '\\' {
            j += 2;
            continue;
        }
        if chars[j] == '`' {
            let m = chars[j..].iter().take_while(|x| **x == '`').count();
            if let Some(k) = (j + m..chars.len()).find(|&k| chars[k..].iter().take_while(|x| **x == '`').count() == m) {
                j = k + m;
                continue;
            }
        }
        if chars[j] == c {
            let m = chars[j..].iter().take_while(|x| **x == c).count();
            let prev = chars[j - 1];
            if m >= n && !prev.is_whitespace() {
                return true;
            }
            j += m;
            continue;
        }
        j += 1;
    }
    false
}

fn starts_url(chars: &[char], i: usize) -> bool {
    let s: String = chars[i..chars.len().min(i + 8)].iter().collect();
    s.starts_with("http://") || s.starts_with("https://") || (s.starts_with("www.") && chars.len() > i + 5)
}

fn unescape_md(s: &str) -> String {
    let mut o = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            if let Some(n) = it.peek().copied() {
                if n.is_ascii_punctuation() {
                    o.push(n);
                    it.next();
                    continue;
                }
            }
        }
        o.push(c);
    }
    crate::io::html::decode_entities(&o)
}

fn split_br(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut rest = s;
    loop {
        let lower = rest.to_ascii_lowercase();
        let pos = ["<br>", "<br/>", "<br />"].iter().filter_map(|t| lower.find(t).map(|p| (p, t.len()))).min();
        match pos {
            Some((p, l)) => {
                parts.push(rest[..p].trim().to_string());
                rest = &rest[p + l..];
            }
            None => {
                parts.push(rest.trim().to_string());
                break;
            }
        }
    }
    parts
}

fn is_html_block_start(t: &str) -> bool {
    let lower = t.to_ascii_lowercase();
    ["<div", "<table", "<p>", "<p ", "<pre", "<ul", "<ol", "<h1", "<h2", "<h3", "<h4", "<h5", "<h6", "<blockquote", "<center", "<hr", "<section", "<figure", "<dl"]
        .iter()
        .any(|p| lower.starts_with(p))
}

fn make_image(alt: &str, target: &str) -> Option<Image> {
    let rest = target.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    if !meta.ends_with(";base64") {
        return None;
    }
    let bytes = base64_decode(payload)?;
    let data = image_data(bytes);
    data.pixel_size?;
    let (w, h) = natural_size(&data, 468.0);
    Some(Image { data, width: w, height: h, alt: alt.to_string(), float: None })
}

/// Paragraphs for a code block: one per line, monospace, no spacing.
fn code_blocks(lines: &[&str]) -> Vec<Block> {
    let mut out = Vec::new();
    for l in lines {
        let mut p = Paragraph::new();
        p.props.space_after = Some(0.0);
        p.props.space_before = Some(0.0);
        p.props.line_spacing = Some(LineSpacing::Multiple(1.0));
        let props = CharProps { font: Some("Courier New".into()), size: Some(10.0), ..Default::default() };
        let mut buf = String::new();
        for c in l.chars() {
            if c == '\t' {
                if !buf.is_empty() {
                    p.runs.push(Run { content: Inline::Text(std::mem::take(&mut buf)), props: props.clone() });
                }
                p.runs.push(Run { content: Inline::Tab, props: props.clone() });
            } else {
                buf.push(c);
            }
        }
        if !buf.is_empty() {
            p.runs.push(Run { content: Inline::Text(buf), props: props.clone() });
        }
        p.mark = props;
        out.push(Block::Para(p));
    }
    if let Some(Block::Para(p)) = out.last_mut() {
        p.props.space_after = Some(8.0);
    }
    out
}
