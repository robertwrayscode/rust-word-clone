//! Built-in styles and style resolution (style inheritance → concrete formatting).

use crate::model::*;

fn pstyle(id: &str, name: &str, based_on: Option<&str>, next: Option<&str>, para: ParaProps, chars: CharProps, quick: bool) -> Style {
    Style {
        id: id.into(),
        name: name.into(),
        kind: StyleKind::Paragraph,
        based_on: based_on.map(Into::into),
        next: next.map(Into::into),
        para,
        chars,
        quick,
    }
}

fn cstyle(id: &str, name: &str, chars: CharProps, quick: bool) -> Style {
    Style { id: id.into(), name: name.into(), kind: StyleKind::Character, chars, quick, ..Default::default() }
}

fn heading(level: u8, size: f32, color: Color, before: f32, italic: bool, light: bool) -> Style {
    pstyle(
        &format!("Heading{}", level),
        &format!("Heading {}", level),
        Some("Normal"),
        Some("Normal"),
        ParaProps {
            space_before: Some(before),
            space_after: Some(if level == 1 { 0.0 } else { 0.0 }),
            keep_next: Some(true),
            keep_lines: Some(true),
            outline_level: Some(level - 1),
            line_spacing: Some(LineSpacing::Multiple(1.08)),
            ..Default::default()
        },
        CharProps {
            font: Some("Calibri Light".into()).filter(|_| light).or(Some("Calibri".into())),
            size: Some(size),
            color: Some(color),
            italic: if italic { Some(true) } else { None },
            ..Default::default()
        },
        level <= 3,
    )
}

/// The styles of Word's Normal template (approximately).
pub fn builtin_styles() -> Vec<Style> {
    let mut v = vec![
        pstyle("Normal", "Normal", None, None, ParaProps::default(), CharProps::default(), true),
        pstyle(
            "NoSpacing",
            "No Spacing",
            Some("Normal"),
            None,
            ParaProps { space_after: Some(0.0), line_spacing: Some(LineSpacing::Multiple(1.0)), ..Default::default() },
            CharProps::default(),
            true,
        ),
        heading(1, 16.0, Color::ACCENT1, 12.0, false, true),
        heading(2, 13.0, Color::ACCENT1, 2.0, false, true),
        heading(3, 12.0, Color::ACCENT1_DARK, 2.0, false, true),
        heading(4, 11.0, Color::ACCENT1, 2.0, true, true),
        heading(5, 11.0, Color::ACCENT1, 2.0, false, true),
        heading(6, 11.0, Color::ACCENT1_DARK, 2.0, false, true),
        pstyle(
            "Title",
            "Title",
            Some("Normal"),
            Some("Normal"),
            ParaProps { space_after: Some(0.0), line_spacing: Some(LineSpacing::Multiple(1.0)), contextual_spacing: Some(true), ..Default::default() },
            CharProps { font: Some("Calibri Light".into()), size: Some(28.0), spacing: Some(-0.5), ..Default::default() },
            true,
        ),
        pstyle(
            "Subtitle",
            "Subtitle",
            Some("Normal"),
            Some("Normal"),
            ParaProps { space_after: Some(8.0), ..Default::default() },
            CharProps { color: Some(Color::rgb(0x5A, 0x5A, 0x5A)), spacing: Some(0.75), ..Default::default() },
            true,
        ),
        pstyle(
            "Quote",
            "Quote",
            Some("Normal"),
            Some("Normal"),
            ParaProps {
                space_before: Some(10.0),
                indent_left: Some(43.2),
                indent_right: Some(43.2),
                align: Some(Align::Center),
                ..Default::default()
            },
            CharProps { italic: Some(true), color: Some(Color::rgb(0x40, 0x40, 0x40)), ..Default::default() },
            true,
        ),
        pstyle(
            "IntenseQuote",
            "Intense Quote",
            Some("Normal"),
            Some("Normal"),
            ParaProps {
                space_before: Some(18.0),
                space_after: Some(18.0),
                indent_left: Some(43.2),
                indent_right: Some(43.2),
                align: Some(Align::Center),
                borders: Some(Borders {
                    top: Some(Border { color: Color::ACCENT1, space: 10.0, ..Border::single() }),
                    bottom: Some(Border { color: Color::ACCENT1, space: 10.0, ..Border::single() }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            CharProps { italic: Some(true), color: Some(Color::ACCENT1), ..Default::default() },
            true,
        ),
        pstyle(
            "ListParagraph",
            "List Paragraph",
            Some("Normal"),
            None,
            ParaProps { indent_left: Some(36.0), contextual_spacing: Some(true), ..Default::default() },
            CharProps::default(),
            true,
        ),
        pstyle(
            "Caption",
            "Caption",
            Some("Normal"),
            Some("Normal"),
            ParaProps { space_after: Some(10.0), line_spacing: Some(LineSpacing::Multiple(1.0)), ..Default::default() },
            CharProps { italic: Some(true), size: Some(9.0), color: Some(Color::rgb(0x44, 0x54, 0x6A)), ..Default::default() },
            false,
        ),
        pstyle(
            "Header",
            "Header",
            Some("Normal"),
            None,
            ParaProps {
                space_after: Some(0.0),
                line_spacing: Some(LineSpacing::Multiple(1.0)),
                tabs: vec![
                    TabStop { pos: 234.0, align: TabAlign::Center, leader: TabLeader::None },
                    TabStop { pos: 468.0, align: TabAlign::Right, leader: TabLeader::None },
                ],
                ..Default::default()
            },
            CharProps::default(),
            false,
        ),
        pstyle(
            "Footer",
            "Footer",
            Some("Normal"),
            None,
            ParaProps {
                space_after: Some(0.0),
                line_spacing: Some(LineSpacing::Multiple(1.0)),
                tabs: vec![
                    TabStop { pos: 234.0, align: TabAlign::Center, leader: TabLeader::None },
                    TabStop { pos: 468.0, align: TabAlign::Right, leader: TabLeader::None },
                ],
                ..Default::default()
            },
            CharProps::default(),
            false,
        ),
        pstyle(
            "FootnoteText",
            "Footnote Text",
            Some("Normal"),
            None,
            ParaProps { space_after: Some(0.0), line_spacing: Some(LineSpacing::Multiple(1.0)), ..Default::default() },
            CharProps { size: Some(10.0), ..Default::default() },
            false,
        ),
        pstyle(
            "EndnoteText",
            "Endnote Text",
            Some("Normal"),
            None,
            ParaProps { space_after: Some(0.0), line_spacing: Some(LineSpacing::Multiple(1.0)), ..Default::default() },
            CharProps { size: Some(10.0), ..Default::default() },
            false,
        ),
        pstyle(
            "CommentText",
            "Comment Text",
            Some("Normal"),
            None,
            ParaProps { space_after: Some(0.0), line_spacing: Some(LineSpacing::Multiple(1.0)), ..Default::default() },
            CharProps { size: Some(10.0), ..Default::default() },
            false,
        ),
        pstyle(
            "TOCHeading",
            "TOC Heading",
            Some("Heading1"),
            Some("Normal"),
            ParaProps { outline_level: None, ..Default::default() },
            CharProps::default(),
            false,
        ),
    ];
    for lvl in 1..=6u8 {
        v.push(pstyle(
            &format!("TOC{}", lvl),
            &format!("TOC {}", lvl),
            Some("Normal"),
            Some("Normal"),
            ParaProps {
                indent_left: Some(11.0 * (lvl as f32 - 1.0)),
                space_after: Some(5.0),
                tabs: vec![TabStop { pos: 467.5, align: TabAlign::Right, leader: TabLeader::Dot }],
                ..Default::default()
            },
            CharProps::default(),
            false,
        ));
    }
    v.push(cstyle("Hyperlink", "Hyperlink", CharProps { color: Some(Color::BLUE), underline: Some(Underline::Single), ..Default::default() }, false));
    v.push(cstyle("Strong", "Strong", CharProps { bold: Some(true), ..Default::default() }, true));
    v.push(cstyle("Emphasis", "Emphasis", CharProps { italic: Some(true), ..Default::default() }, true));
    v.push(cstyle(
        "IntenseEmphasis",
        "Intense Emphasis",
        CharProps { italic: Some(true), color: Some(Color::ACCENT1), ..Default::default() },
        true,
    ));
    v.push(cstyle(
        "SubtleEmphasis",
        "Subtle Emphasis",
        CharProps { italic: Some(true), color: Some(Color::rgb(0x40, 0x40, 0x40)), ..Default::default() },
        true,
    ));
    v.push(cstyle(
        "IntenseReference",
        "Intense Reference",
        CharProps { bold: Some(true), small_caps: Some(true), color: Some(Color::ACCENT1), spacing: Some(0.25), ..Default::default() },
        true,
    ));
    v.push(cstyle("BookTitle", "Book Title", CharProps { bold: Some(true), italic: Some(true), spacing: Some(0.25), ..Default::default() }, true));
    v.push(cstyle("FootnoteReference", "Footnote Reference", CharProps { vert_align: Some(VertAlign::Superscript), ..Default::default() }, false));
    v.push(cstyle("EndnoteReference", "Endnote Reference", CharProps { vert_align: Some(VertAlign::Superscript), ..Default::default() }, false));
    v.push(Style { id: "TableGrid".into(), name: "Table Grid".into(), kind: StyleKind::Table, ..Default::default() });
    v
}

/// Fully resolved character formatting.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedChars {
    pub font: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: Underline,
    pub strike: bool,
    pub double_strike: bool,
    pub color: Color,
    pub highlight: Option<Color>,
    pub shading: Option<Color>,
    pub vert_align: VertAlign,
    pub caps: bool,
    pub small_caps: bool,
    pub hidden: bool,
    pub spacing: f32,
    pub link: Option<String>,
    pub revision: Option<Revision>,
    pub comments: Vec<u32>,
}

/// Fully resolved paragraph formatting.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedPara {
    pub style: String,
    pub align: Align,
    pub indent_left: f32,
    pub indent_right: f32,
    pub indent_first: f32,
    pub space_before: f32,
    pub space_after: f32,
    pub line_spacing: LineSpacing,
    pub contextual_spacing: bool,
    pub keep_next: bool,
    pub keep_lines: bool,
    pub page_break_before: bool,
    pub widow_control: bool,
    pub list: Option<ListRef>,
    pub shading: Option<Color>,
    pub borders: Borders,
    pub tabs: Vec<TabStop>,
    pub outline_level: Option<u8>,
}

/// Chain of style ids from root to `id` (inclusive), guarding against cycles.
fn chain<'a>(doc: &'a Document, id: &str) -> Vec<&'a Style> {
    let mut out = Vec::new();
    let mut cur = doc.style(id);
    while let Some(s) = cur {
        if out.iter().any(|x: &&Style| x.id == s.id) || out.len() > 16 {
            break;
        }
        out.push(s);
        cur = s.based_on.as_deref().and_then(|b| doc.style(b));
    }
    out.reverse();
    out
}

/// The paragraph properties a style contributes (including its ancestors), on top of document defaults.
pub fn style_para_props(doc: &Document, style: &str) -> ParaProps {
    let mut p = doc.default_para.clone();
    for s in chain(doc, style) {
        p.merge(&s.para);
    }
    p
}

/// Character properties contributed by a style chain (no document defaults).
pub fn style_char_props(doc: &Document, style: &str) -> CharProps {
    let mut c = CharProps::default();
    for s in chain(doc, style) {
        c.merge(&s.chars);
    }
    c
}

pub fn resolve_para(doc: &Document, props: &ParaProps) -> ResolvedPara {
    let style = props.style.clone().unwrap_or_else(|| "Normal".into());
    let mut p = style_para_props(doc, &style);
    p.merge(props);
    let list = p.list.flatten();
    // List levels supply indents unless the paragraph overrides them directly.
    let (mut il, mut ifirst) = (p.indent_left.unwrap_or(0.0), p.indent_first.unwrap_or(0.0));
    if let Some(l) = list {
        if let Some(lvl) = doc.list(l.list).and_then(|d| d.levels.get(l.level as usize)) {
            if props.indent_left.is_none() {
                il = lvl.indent;
            }
            if props.indent_first.is_none() {
                ifirst = -lvl.hanging;
            }
        }
    }
    ResolvedPara {
        style,
        align: p.align.unwrap_or_default(),
        indent_left: il,
        indent_right: p.indent_right.unwrap_or(0.0),
        indent_first: ifirst,
        space_before: p.space_before.unwrap_or(0.0),
        space_after: p.space_after.unwrap_or(0.0),
        line_spacing: p.line_spacing.unwrap_or_default(),
        contextual_spacing: p.contextual_spacing.unwrap_or(false),
        keep_next: p.keep_next.unwrap_or(false),
        keep_lines: p.keep_lines.unwrap_or(false),
        page_break_before: p.page_break_before.unwrap_or(false),
        widow_control: p.widow_control.unwrap_or(true),
        list,
        shading: p.shading,
        borders: p.borders.unwrap_or_default(),
        tabs: p.tabs,
        outline_level: p.outline_level,
    }
}

/// Resolve run formatting: document defaults < paragraph style < character style < direct formatting.
pub fn resolve_chars(doc: &Document, para_style: &str, props: &CharProps) -> ResolvedChars {
    let mut c = doc.default_chars.clone();
    c.merge(&style_char_props(doc, para_style));
    if let Some(cs) = &props.style {
        c.merge(&style_char_props(doc, cs));
    }
    c.merge(props);
    let mut bold = c.bold.unwrap_or(false);
    let mut italic = c.italic.unwrap_or(false);
    // Word toggles bold/italic when direct formatting matches a character style's value; keep it simple.
    if props.style.is_none() {
        bold = c.bold.unwrap_or(bold);
        italic = c.italic.unwrap_or(italic);
    }
    ResolvedChars {
        font: c.font.unwrap_or_else(|| "Calibri".into()),
        size: c.size.unwrap_or(11.0).clamp(1.0, 1638.0),
        bold,
        italic,
        underline: c.underline.unwrap_or_default(),
        strike: c.strike.unwrap_or(false),
        double_strike: c.double_strike.unwrap_or(false),
        color: c.color.unwrap_or(Color::BLACK),
        highlight: c.highlight,
        shading: c.shading,
        vert_align: c.vert_align.unwrap_or_default(),
        caps: c.caps.unwrap_or(false),
        small_caps: c.small_caps.unwrap_or(false),
        hidden: c.hidden.unwrap_or(false),
        spacing: c.spacing.unwrap_or(0.0),
        link: c.link,
        revision: c.revision,
        comments: c.comments,
    }
}

/// Render a list number for `n` in the given format.
pub fn format_number(n: u32, fmt: NumFormat) -> String {
    match fmt {
        NumFormat::Decimal => n.to_string(),
        NumFormat::DecimalZero => format!("{:02}", n),
        NumFormat::LowerLetter => letters(n),
        NumFormat::UpperLetter => letters(n).to_uppercase(),
        NumFormat::LowerRoman => roman(n).to_lowercase(),
        NumFormat::UpperRoman => roman(n),
        NumFormat::Bullet | NumFormat::None => String::new(),
    }
}

fn letters(n: u32) -> String {
    // Word style: a..z, aa..zz, aaa..
    if n == 0 {
        return String::new();
    }
    let idx = (n - 1) % 26;
    let rep = (n - 1) / 26 + 1;
    std::iter::repeat_n((b'a' + idx as u8) as char, rep as usize).collect()
}

pub fn roman(mut n: u32) -> String {
    if n == 0 || n >= 4000 {
        return n.to_string();
    }
    let table = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut s = String::new();
    for (v, r) in table {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

/// Computes list labels for paragraphs in document order.
#[derive(Default, Debug, Clone)]
pub struct ListCounter {
    counters: std::collections::HashMap<u32, [u32; 9]>,
}

impl ListCounter {
    /// Advance the counter for `r` and return the label text (e.g. "2.", "b)", "•").
    pub fn next_label(&mut self, doc: &Document, r: ListRef) -> Option<(String, Option<String>)> {
        let def = doc.list(r.list)?;
        let level = (r.level as usize).min(8);
        let lvl = def.levels.get(level)?;
        let c = self.counters.entry(r.list).or_insert([0; 9]);
        // Counters start at level start - 1; deeper levels reset.
        for (i, slot) in c.iter_mut().enumerate().skip(level + 1) {
            let _ = i;
            *slot = 0;
        }
        c[level] = if c[level] == 0 { lvl.start.max(1) } else { c[level] + 1 };
        // Ancestors that were never started count as their start value.
        for i in 0..level {
            if c[i] == 0 {
                c[i] = def.levels.get(i).map(|l| l.start.max(1)).unwrap_or(1);
            }
        }
        let snapshot = *c;
        if lvl.format == NumFormat::Bullet {
            return Some((lvl.text.clone(), lvl.font.clone()));
        }
        if lvl.format == NumFormat::None {
            return Some((String::new(), None));
        }
        let mut out = String::new();
        let mut chars = lvl.text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '%' {
                if let Some(d) = chars.peek().and_then(|d| d.to_digit(10)) {
                    chars.next();
                    let li = (d as usize).saturating_sub(1).min(8);
                    let f = def.levels.get(li).map(|l| l.format).unwrap_or_default();
                    let f = if f == NumFormat::Bullet { NumFormat::Decimal } else { f };
                    out.push_str(&format_number(snapshot[li], f));
                    continue;
                }
            }
            out.push(ch);
        }
        Some((out, lvl.font.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_resolution() {
        let doc = Document::new();
        let p = resolve_para(&doc, &ParaProps { style: Some("Heading1".into()), ..Default::default() });
        assert_eq!(p.space_before, 12.0);
        assert!(p.keep_next);
        assert_eq!(p.outline_level, Some(0));
        let c = resolve_chars(&doc, "Heading1", &CharProps::default());
        assert_eq!(c.size, 16.0);
        assert_eq!(c.font, "Calibri Light");
        let c = resolve_chars(&doc, "Normal", &CharProps { bold: Some(true), ..Default::default() });
        assert!(c.bold);
        assert_eq!(c.size, 11.0);
    }

    #[test]
    fn numbering_labels() {
        let mut doc = Document::new();
        let id = doc.add_list(ListDef::multilevel(0));
        let mut lc = ListCounter::default();
        let r0 = ListRef { list: id, level: 0 };
        let r1 = ListRef { list: id, level: 1 };
        assert_eq!(lc.next_label(&doc, r0).unwrap().0, "1.");
        assert_eq!(lc.next_label(&doc, r1).unwrap().0, "1.1.");
        assert_eq!(lc.next_label(&doc, r1).unwrap().0, "1.2.");
        assert_eq!(lc.next_label(&doc, r0).unwrap().0, "2.");
        assert_eq!(lc.next_label(&doc, r1).unwrap().0, "2.1.");
        assert_eq!(roman(1994), "MCMXCIV");
        assert_eq!(letters(27), "aa");
    }
}
