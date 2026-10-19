//! OpenDocument Text (.odt) reader and writer (a practical subset: paragraphs, headings, spans, lists,
//! tables, images, notes, comments, fields, page setup, headers and footers).

use crate::io::package::{Package, ZipBuilder};
use crate::io::util::*;
use crate::io::xml::{self, esc, esc_attr, Element, Node};
use crate::model::*;
use crate::styles::resolve_para;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

const MIME: &str = "application/vnd.oasis.opendocument.text";

/// Model style id ↔ LibreOffice programmatic style name.
const STYLE_MAP: &[(&str, &str)] = &[
    ("Normal", "Standard"),
    ("Heading1", "Heading_20_1"),
    ("Heading2", "Heading_20_2"),
    ("Heading3", "Heading_20_3"),
    ("Heading4", "Heading_20_4"),
    ("Heading5", "Heading_20_5"),
    ("Heading6", "Heading_20_6"),
    ("Heading7", "Heading_20_7"),
    ("Heading8", "Heading_20_8"),
    ("Heading9", "Heading_20_9"),
    ("Title", "Title"),
    ("Subtitle", "Subtitle"),
    ("Quote", "Quotations"),
    ("ListParagraph", "List_20_Paragraph"),
    ("Caption", "Caption"),
    ("Header", "Header"),
    ("Footer", "Footer"),
    ("FootnoteText", "Footnote"),
    ("EndnoteText", "Endnote"),
    ("TOCHeading", "Contents_20_Heading"),
    ("TOC1", "Contents_20_1"),
    ("TOC2", "Contents_20_2"),
    ("TOC3", "Contents_20_3"),
    ("TOC4", "Contents_20_4"),
    ("TOC5", "Contents_20_5"),
    ("TOC6", "Contents_20_6"),
    ("Hyperlink", "Internet_20_link"),
    ("Strong", "Strong_20_Emphasis"),
    ("Emphasis", "Emphasis"),
];

/// Encode a style id as an ODF style name (NCName), LibreOffice style (`_20_` for space).
fn odf_name(id: &str) -> String {
    if let Some((_, n)) = STYLE_MAP.iter().find(|(i, _)| *i == id) {
        return n.to_string();
    }
    let mut s = String::new();
    for (i, c) in id.chars().enumerate() {
        if c.is_ascii_alphanumeric() || (i > 0 && matches!(c, '-' | '.')) {
            s.push(c);
        } else {
            let _ = write!(s, "_{:x}_", c as u32);
        }
    }
    if s.is_empty() {
        s.push_str("_20_");
    }
    s
}

/// Decode an ODF style name to a model style id.
fn model_id(name: &str) -> String {
    if let Some((i, _)) = STYLE_MAP.iter().find(|(_, n)| *n == name) {
        return i.to_string();
    }
    // Decode `_xx_` sequences.
    let mut out = String::new();
    let mut rest = name;
    while let Some(i) = rest.find('_') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        if let Some(j) = tail.find('_') {
            if j > 0 && j <= 6 {
                if let Some(c) = u32::from_str_radix(&tail[..j], 16).ok().and_then(char::from_u32) {
                    out.push(c);
                    rest = &tail[j + 1..];
                    continue;
                }
            }
        }
        out.push('_');
        rest = tail;
    }
    out.push_str(rest);
    if let Some((i, _)) = STYLE_MAP.iter().find(|(_, n)| n.replace("_20_", " ") == out) {
        return i.to_string();
    }
    // Ids are identifiers: drop spaces ("Text body" → "Textbody").
    out.replace(' ', "")
}

fn pt(v: f32) -> String {
    format!("{}pt", fmt_num(v))
}

fn color_hex(c: Color) -> String {
    format!("#{}", c.to_hex().to_ascii_lowercase())
}

fn border_str(b: &Option<Border>) -> String {
    match b {
        Some(b) if b.is_visible() => {
            let style = match b.style {
                BorderStyle::Double => "double",
                BorderStyle::Dotted => "dotted",
                BorderStyle::Dashed => "dashed",
                _ => "solid",
            };
            let w = if b.style == BorderStyle::Double { b.width * 3.0 } else { b.width };
            format!("{} {} {}", pt(w), style, color_hex(b.color))
        }
        _ => "none".into(),
    }
}

fn parse_border_str(s: &str) -> Option<Border> {
    let s = s.trim();
    if s.is_empty() || s == "none" || s == "hidden" {
        return None;
    }
    let mut b = Border::single();
    let mut width = None;
    for tok in s.split_whitespace() {
        if tok.starts_with('#') || named_color(tok).is_some() {
            if let Some(c) = css_color(tok) {
                b.color = c;
                continue;
            }
        }
        match tok {
            "solid" => b.style = BorderStyle::Single,
            "double" => b.style = BorderStyle::Double,
            "dotted" => b.style = BorderStyle::Dotted,
            "dashed" => b.style = BorderStyle::Dashed,
            "none" | "hidden" => return None,
            "thin" => width = Some(0.5),
            "medium" => width = Some(1.0),
            "thick" => width = Some(2.25),
            t => {
                if let Some(w) = parse_length(t) {
                    width = Some(w);
                }
            }
        }
    }
    let mut w = width.unwrap_or(0.5);
    if b.style == BorderStyle::Double {
        w /= 3.0;
    }
    b.width = (w * 8.0).round() / 8.0;
    b.space = 0.0;
    Some(b)
}

// ======================================================================================
// Writer
// ======================================================================================

fn text_props_attrs(c: &CharProps) -> String {
    let mut s = String::new();
    if let Some(f) = &c.font {
        let _ = write!(s, r#" style:font-name="{}""#, esc_attr(f));
    }
    if let Some(v) = c.size {
        let _ = write!(s, r#" fo:font-size="{}""#, pt(v));
    }
    if let Some(b) = c.bold {
        let w = if b { "bold" } else { "normal" };
        let _ = write!(s, r#" fo:font-weight="{w}" style:font-weight-asian="{w}" style:font-weight-complex="{w}""#);
    }
    if let Some(i) = c.italic {
        let v = if i { "italic" } else { "normal" };
        let _ = write!(s, r#" fo:font-style="{v}" style:font-style-asian="{v}" style:font-style-complex="{v}""#);
    }
    if let Some(u) = c.underline {
        let (style, extra) = match u {
            Underline::None => ("none", ""),
            Underline::Single => ("solid", ""),
            Underline::Double => ("solid", r#" style:text-underline-type="double""#),
            Underline::Thick => ("solid", r#" style:text-underline-width="bold""#),
            Underline::Dotted => ("dotted", ""),
            Underline::Dashed => ("dash", ""),
            Underline::Wavy => ("wave", ""),
            Underline::Words => ("solid", r#" style:text-underline-mode="skip-white-space""#),
        };
        let _ = write!(s, r#" style:text-underline-style="{}"{}"#, style, extra);
        if u != Underline::None {
            if u != Underline::Thick {
                s.push_str(r#" style:text-underline-width="auto""#);
            }
            s.push_str(r#" style:text-underline-color="font-color""#);
        }
    }
    if c.double_strike == Some(true) {
        s.push_str(r#" style:text-line-through-style="solid" style:text-line-through-type="double""#);
    } else if let Some(st) = c.strike {
        let _ = write!(s, r#" style:text-line-through-style="{}""#, if st { "solid" } else { "none" });
    }
    if let Some(col) = c.color {
        let _ = write!(s, r#" fo:color="{}""#, color_hex(col));
    }
    if let Some(bg) = c.highlight.or(c.shading) {
        let _ = write!(s, r#" fo:background-color="{}""#, color_hex(bg));
    }
    if let Some(v) = c.vert_align {
        let p = match v {
            VertAlign::Baseline => "0% 100%",
            VertAlign::Superscript => "super 58%",
            VertAlign::Subscript => "sub 58%",
        };
        let _ = write!(s, r#" style:text-position="{}""#, p);
    }
    if let Some(v) = c.caps {
        let _ = write!(s, r#" fo:text-transform="{}""#, if v { "uppercase" } else { "none" });
    }
    if let Some(v) = c.small_caps {
        let _ = write!(s, r#" fo:font-variant="{}""#, if v { "small-caps" } else { "normal" });
    }
    if let Some(v) = c.hidden {
        let _ = write!(s, r#" text:display="{}""#, if v { "none" } else { "true" });
    }
    if let Some(v) = c.spacing {
        let _ = write!(s, r#" fo:letter-spacing="{}""#, if v == 0.0 { "normal".to_string() } else { pt(v) });
    }
    s
}

fn para_props_xml(p: &ParaProps, extra_attrs: &str) -> String {
    let mut a = String::from(extra_attrs);
    if let Some(al) = p.align {
        let v = match al {
            Align::Left => "start",
            Align::Center => "center",
            Align::Right => "end",
            Align::Justify => "justify",
        };
        let _ = write!(a, r#" fo:text-align="{}""#, v);
    }
    if let Some(v) = p.indent_left {
        let _ = write!(a, r#" fo:margin-left="{}""#, pt(v));
    }
    if let Some(v) = p.indent_right {
        let _ = write!(a, r#" fo:margin-right="{}""#, pt(v));
    }
    if let Some(v) = p.indent_first {
        let _ = write!(a, r#" fo:text-indent="{}""#, pt(v));
    }
    if let Some(v) = p.space_before {
        let _ = write!(a, r#" fo:margin-top="{}""#, pt(v));
    }
    if let Some(v) = p.space_after {
        let _ = write!(a, r#" fo:margin-bottom="{}""#, pt(v));
    }
    match p.line_spacing {
        Some(LineSpacing::Multiple(m)) => {
            let _ = write!(a, r#" fo:line-height="{}%""#, fmt_num(m * 100.0));
        }
        Some(LineSpacing::AtLeast(v)) => {
            let _ = write!(a, r#" style:line-height-at-least="{}""#, pt(v));
        }
        Some(LineSpacing::Exactly(v)) => {
            let _ = write!(a, r#" fo:line-height="{}""#, pt(v));
        }
        None => {}
    }
    if let Some(v) = p.contextual_spacing {
        let _ = write!(a, r#" loext:contextual-spacing="{}""#, v);
    }
    if let Some(v) = p.keep_next {
        let _ = write!(a, r#" fo:keep-with-next="{}""#, if v { "always" } else { "auto" });
    }
    if let Some(v) = p.keep_lines {
        let _ = write!(a, r#" fo:keep-together="{}""#, if v { "always" } else { "auto" });
    }
    if let Some(v) = p.page_break_before {
        if !a.contains("fo:break-before") {
            let _ = write!(a, r#" fo:break-before="{}""#, if v { "page" } else { "auto" });
        }
    }
    if let Some(v) = p.widow_control {
        let n = if v { 2 } else { 0 };
        let _ = write!(a, r#" fo:widows="{n}" fo:orphans="{n}""#);
    }
    if let Some(c) = p.shading {
        let _ = write!(a, r#" fo:background-color="{}""#, color_hex(c));
    }
    if let Some(b) = &p.borders {
        for (side, br) in [("top", &b.top), ("bottom", &b.bottom), ("left", &b.left), ("right", &b.right)] {
            if br.is_some() {
                let _ = write!(a, r#" fo:border-{}="{}""#, side, border_str(br));
                if let Some(br) = br {
                    let _ = write!(a, r#" fo:padding-{}="{}""#, side, pt(br.space));
                }
            }
        }
    }
    let mut children = String::new();
    if !p.tabs.is_empty() {
        children.push_str("<style:tab-stops>");
        for t in &p.tabs {
            let ty = match t.align {
                TabAlign::Left | TabAlign::Clear => "left",
                TabAlign::Center => "center",
                TabAlign::Right => "right",
                TabAlign::Decimal => "char",
            };
            let _ = write!(children, r#"<style:tab-stop style:position="{}" style:type="{}""#, pt(t.pos), ty);
            if t.align == TabAlign::Decimal {
                children.push_str(r#" style:char=".""#);
            }
            let leader = match t.leader {
                TabLeader::None => None,
                TabLeader::Dot => Some(("dotted", ".")),
                TabLeader::Hyphen => Some(("dash", "-")),
                TabLeader::Underscore => Some(("solid", "_")),
            };
            if let Some((st, ch)) = leader {
                let _ = write!(children, r#" style:leader-style="{}" style:leader-text="{}""#, st, ch);
            }
            children.push_str("/>");
        }
        children.push_str("</style:tab-stops>");
    }
    if a.is_empty() && children.is_empty() {
        return String::new();
    }
    if children.is_empty() {
        format!("<style:paragraph-properties{}/>", a)
    } else {
        format!("<style:paragraph-properties{}>{}</style:paragraph-properties>", a, children)
    }
}

#[derive(Default)]
struct AutoStyles {
    prefix: String,
    entries: HashMap<String, String>, // key → name
    xml: String,
    counters: HashMap<&'static str, usize>,
}

impl AutoStyles {
    fn get(&mut self, family: &'static str, letter: &str, key: String, body: impl FnOnce(&str) -> String) -> String {
        let full_key = format!("{}|{}", family, key);
        if let Some(n) = self.entries.get(&full_key) {
            return n.clone();
        }
        let c = self.counters.entry(family).or_insert(0);
        *c += 1;
        let name = format!("{}{}{}", self.prefix, letter, c);
        self.xml.push_str(&body(&name));
        self.entries.insert(full_key, name.clone());
        name
    }

    /// `master`: attach the paragraph to the "Standard" master page (first paragraph of the body).
    fn para(&mut self, parent: &str, p: &ParaProps, c: &CharProps, extra: &str, master: bool) -> String {
        let pp = para_props_xml(p, extra);
        let tp = text_props_attrs(c);
        let parent_attr = if parent.is_empty() { String::new() } else { format!(r#" style:parent-style-name="{}""#, parent) };
        let master = if master { r#" style:master-page-name="Standard""# } else { "" };
        let key = format!("{parent}|{pp}|{tp}|{master}");
        self.get("paragraph", "P", key, |name| {
            let mut s = format!(r#"<style:style style:name="{}" style:family="paragraph"{}{}>"#, name, parent_attr, master);
            s.push_str(&pp);
            if !tp.is_empty() {
                let _ = write!(s, "<style:text-properties{}/>", tp);
            }
            s.push_str("</style:style>");
            s
        })
    }

    fn text(&mut self, c: &CharProps) -> Option<String> {
        let tp = text_props_attrs(c);
        if tp.is_empty() {
            return None;
        }
        Some(self.get("text", "T", tp.clone(), |name| {
            format!(r#"<style:style style:name="{}" style:family="text"><style:text-properties{}/></style:style>"#, name, tp)
        }))
    }

    fn raw(&mut self, family: &'static str, letter: &str, body_inner: String) -> String {
        let key = body_inner.clone();
        self.get(family, letter, key, |name| {
            format!(r#"<style:style style:name="{}" style:family="{}">{}</style:style>"#, name, family, body_inner)
        })
    }
}

struct OdtWriter<'a> {
    doc: &'a Document,
    pictures: HashMap<usize, String>,
    picture_files: Vec<(String, Vec<u8>)>,
    frame_id: usize,
    table_id: usize,
    note_ids: HashMap<(bool, u32), usize>,
    list_styles: HashMap<u32, String>,
    comment_ranges: HashMap<u32, (usize, usize)>,
    run_counter: usize,
    fonts: Vec<String>,
}

const NS: &str = concat!(
    r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" "#,
    r#"xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" "#,
    r#"xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" "#,
    r#"xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" "#,
    r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" "#,
    r#"xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" "#,
    r#"xmlns:xlink="http://www.w3.org/1999/xlink" "#,
    r#"xmlns:dc="http://purl.org/dc/elements/1.1/" "#,
    r#"xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" "#,
    r#"xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0" "#,
    r#"xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" "#,
    r#"xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0" "#,
    r#"xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0" "#,
    r#"office:version="1.3""#
);

/// Serialise a document as .odt bytes.
pub fn write(doc: &Document) -> Result<Vec<u8>, String> {
    let mut w = OdtWriter {
        doc,
        pictures: HashMap::new(),
        picture_files: Vec::new(),
        frame_id: 0,
        table_id: 0,
        note_ids: HashMap::new(),
        list_styles: HashMap::new(),
        comment_ranges: HashMap::new(),
        run_counter: 0,
        fonts: Vec::new(),
    };
    for (i, n) in doc.footnotes.iter().enumerate() {
        w.note_ids.insert((true, n.id), i + 1);
    }
    for (i, n) in doc.endnotes.iter().enumerate() {
        w.note_ids.insert((false, n.id), i + 1);
    }
    w.compute_comment_ranges();
    w.collect_fonts();

    // content.xml
    let mut auto = AutoStyles::default();
    for l in &doc.lists {
        let name = format!("L{}", l.id);
        auto.xml.push_str(&list_style_xml(&name, l));
        w.list_styles.insert(l.id, name);
    }
    let mut body = String::new();
    w.blocks(&mut body, &doc.body, &mut auto, true);
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {NS}>{fonts}<office:automatic-styles>{auto}</office:automatic-styles><office:body><office:text><text:sequence-decls><text:sequence-decl text:display-outline-level="0" text:name="Illustration"/><text:sequence-decl text:display-outline-level="0" text:name="Table"/></text:sequence-decls>{body}</office:text></office:body></office:document-content>"#,
        fonts = w.font_decls(),
        auto = auto.xml
    );

    // styles.xml (named styles, page layout, master page with header/footer)
    let mut mauto = AutoStyles { prefix: "M".into(), ..Default::default() };
    for l in &doc.lists {
        mauto.xml.push_str(&list_style_xml(&format!("L{}", l.id), l));
    }
    let master = w.master_page(&mut mauto);
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-styles {NS}>{fonts}<office:styles>{named}</office:styles><office:automatic-styles>{layout}{mauto}</office:automatic-styles><office:master-styles>{master}</office:master-styles></office:document-styles>"#,
        fonts = w.font_decls(),
        named = w.named_styles(),
        layout = w.page_layout(),
        mauto = mauto.xml,
        master = master
    );

    let p = &doc.properties;
    let mut meta = String::from(r#"<?xml version="1.0" encoding="UTF-8"?><office:document-meta "#);
    meta.push_str(NS);
    meta.push_str("><office:meta><meta:generator>RWord</meta:generator>");
    for (tag, v) in [
        ("dc:title", &p.title),
        ("dc:subject", &p.subject),
        ("meta:initial-creator", &p.author),
        ("meta:keyword", &p.keywords),
        ("dc:description", &p.description),
        ("dc:creator", &p.last_modified_by),
        ("meta:creation-date", &p.created),
        ("dc:date", &p.modified),
    ] {
        if !v.is_empty() {
            let v = if tag.ends_with("date") { v.trim_end_matches('Z').to_string() } else { v.clone() };
            let _ = write!(meta, "<{tag}>{}</{tag}>", esc(&v));
        }
    }
    meta.push_str("</office:meta></office:document-meta>");

    let settings = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-settings {NS}><office:settings><config:config-item-set config:name="ooo:configuration-settings"><config:config-item config:name="TabsRelativeToIndent" config:type="boolean">false</config:config-item></config:config-item-set></office:settings></office:document-settings>"#
    );

    let mut manifest = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3">"#,
    );
    let _ = write!(manifest, r#"<manifest:file-entry manifest:full-path="/" manifest:version="1.3" manifest:media-type="{MIME}"/>"#);
    for f in ["content.xml", "styles.xml", "meta.xml", "settings.xml"] {
        let _ = write!(manifest, r#"<manifest:file-entry manifest:full-path="{f}" manifest:media-type="text/xml"/>"#);
    }
    for (name, _) in &w.picture_files {
        let ext = name.rsplit('.').next().unwrap_or("png");
        let _ = write!(manifest, r#"<manifest:file-entry manifest:full-path="{}" manifest:media-type="{}"/>"#, name, image_mime(ext));
    }
    manifest.push_str("</manifest:manifest>");

    let mut z = ZipBuilder::new();
    z.add_stored("mimetype", MIME.as_bytes())?;
    z.add("content.xml", content.as_bytes())?;
    z.add("styles.xml", styles.as_bytes())?;
    z.add("meta.xml", meta.as_bytes())?;
    z.add("settings.xml", settings.as_bytes())?;
    for (name, data) in &w.picture_files {
        z.add(name, data)?;
    }
    z.add("META-INF/manifest.xml", manifest.as_bytes())?;
    z.finish()
}

fn list_style_xml(name: &str, l: &ListDef) -> String {
    let mut s = format!(r#"<text:list-style style:name="{}">"#, name);
    for (i, lv) in l.levels.iter().enumerate().take(10) {
        let props = format!(
            r#"<style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" text:list-tab-stop-position="{ind}" fo:text-indent="{ti}" fo:margin-left="{ind}"/></style:list-level-properties>"#,
            ind = pt(lv.indent),
            ti = pt(-lv.hanging)
        );
        let font = lv.font.as_ref().map(|f| format!(r#"<style:text-properties style:font-name="{}"/>"#, esc_attr(f))).unwrap_or_default();
        if lv.format == NumFormat::Bullet {
            let ch = lv.text.chars().next().unwrap_or('•');
            let _ = write!(
                s,
                r#"<text:list-level-style-bullet text:level="{}" text:bullet-char="{}">{}{}</text:list-level-style-bullet>"#,
                i + 1,
                esc_attr(&ch.to_string()),
                props,
                font
            );
        } else {
            let fmt = match lv.format {
                NumFormat::Decimal => "1",
                NumFormat::DecimalZero => "01, 02, 03, ...",
                NumFormat::LowerLetter => "a",
                NumFormat::UpperLetter => "A",
                NumFormat::LowerRoman => "i",
                NumFormat::UpperRoman => "I",
                _ => "",
            };
            // Split "%1.%2." into prefix / suffix and count displayed levels.
            let t = &lv.text;
            let (prefix, suffix) = match (t.find('%'), t.rfind('%')) {
                (Some(f), Some(l)) => (&t[..f], t.get(l + 2..).unwrap_or("")),
                _ => (t.as_str(), ""),
            };
            let display = t.matches('%').count().max(1);
            let _ = write!(
                s,
                r#"<text:list-level-style-number text:level="{}" style:num-prefix="{}" style:num-suffix="{}" style:num-format="{}" text:start-value="{}"{}>{}{}</text:list-level-style-number>"#,
                i + 1,
                esc_attr(prefix),
                esc_attr(suffix),
                esc_attr(fmt),
                lv.start.max(1),
                if display > 1 { format!(r#" text:display-levels="{}""#, display) } else { String::new() },
                props,
                font
            );
        }
    }
    s.push_str("</text:list-style>");
    s
}

impl<'a> OdtWriter<'a> {
    fn compute_comment_ranges(&mut self) {
        let mut idx = 0usize;
        let mut ranges: HashMap<u32, (usize, usize)> = HashMap::new();
        for_each_para(&self.doc.body, &mut |p| {
            for r in &p.runs {
                for &c in &r.props.comments {
                    let e = ranges.entry(c).or_insert((idx, idx));
                    e.1 = idx;
                }
                idx += 1;
            }
        });
        self.comment_ranges = ranges;
    }

    fn collect_fonts(&mut self) {
        let mut fonts = Vec::new();
        let mut add = |f: &Option<String>| {
            if let Some(f) = f {
                if !fonts.contains(f) {
                    fonts.push(f.clone());
                }
            }
        };
        add(&self.doc.default_chars.font);
        for s in &self.doc.styles {
            add(&s.chars.font);
        }
        for l in &self.doc.lists {
            for lv in &l.levels {
                add(&lv.font);
            }
        }
        let doc = self.doc;
        for blocks in [&doc.body, &doc.headers.header, &doc.headers.footer, &doc.headers.first_header, &doc.headers.first_footer] {
            for_each_para(blocks, &mut |p| p.runs.iter().for_each(|r| add(&r.props.font)));
        }
        for n in doc.footnotes.iter().chain(&doc.endnotes) {
            n.paragraphs.iter().for_each(|p| p.runs.iter().for_each(|r| add(&r.props.font)));
        }
        self.fonts = fonts;
    }

    fn font_decls(&self) -> String {
        let mut s = String::from("<office:font-face-decls>");
        for f in &self.fonts {
            let _ = write!(s, r#"<style:font-face style:name="{0}" svg:font-family="&apos;{0}&apos;"/>"#, esc_attr(f));
        }
        s.push_str("</office:font-face-decls>");
        s
    }

    fn named_styles(&self) -> String {
        let doc = self.doc;
        let mut s = String::new();
        let mut dc = doc.default_chars.clone();
        dc.style = None;
        let _ = write!(
            s,
            r#"<style:default-style style:family="paragraph">{}<style:text-properties{}/></style:default-style>"#,
            para_props_xml(
                &ParaProps { style: None, ..doc.default_para.clone() },
                &format!(r#" style:tab-stop-distance="{}""#, pt(doc.settings.default_tab))
            ),
            text_props_attrs(&dc)
        );
        for st in &doc.styles {
            let family = match st.kind {
                StyleKind::Paragraph => "paragraph",
                StyleKind::Character => "text",
                StyleKind::Table => continue,
            };
            let _ = write!(
                s,
                r#"<style:style style:name="{}" style:display-name="{}" style:family="{}""#,
                odf_name(&st.id),
                esc_attr(&st.name),
                family
            );
            if let Some(b) = &st.based_on {
                if doc.style(b).is_some() && b != &st.id {
                    let _ = write!(s, r#" style:parent-style-name="{}""#, odf_name(b));
                }
            }
            if let Some(n) = &st.next {
                if doc.style(n).is_some() && st.kind == StyleKind::Paragraph {
                    let _ = write!(s, r#" style:next-style-name="{}""#, odf_name(n));
                }
            }
            if let Some(l) = st.para.outline_level {
                let _ = write!(s, r#" style:default-outline-level="{}""#, l + 1);
            }
            if let Some(Some(l)) = st.para.list {
                if self.list_styles.contains_key(&l.list) {
                    let _ = write!(s, r#" style:list-style-name="L{}""#, l.list);
                }
            }
            s.push('>');
            if st.kind == StyleKind::Paragraph {
                s.push_str(&para_props_xml(&st.para, ""));
            }
            let tp = text_props_attrs(&CharProps { style: None, ..st.chars.clone() });
            if !tp.is_empty() {
                let _ = write!(s, "<style:text-properties{}/>", tp);
            }
            s.push_str("</style:style>");
        }
        // Graphic style for inline images.
        s.push_str(r#"<style:style style:name="Graphics" style:family="graphic"><style:graphic-properties text:anchor-type="as-char" svg:y="0in" style:vertical-pos="top" style:vertical-rel="baseline"/></style:style>"#);
        s
    }

    fn has_hf(&self) -> (bool, bool) {
        let d = self.doc;
        let h = !d.headers.header.is_empty() || (d.page.title_page && !d.headers.first_header.is_empty());
        let f = !d.headers.footer.is_empty() || (d.page.title_page && !d.headers.first_footer.is_empty());
        (h, f)
    }

    fn page_layout(&self) -> String {
        let p = &self.doc.page;
        let (has_h, has_f) = self.has_hf();
        let top = if has_h { p.header_distance } else { p.margin_top };
        let bottom = if has_f { p.footer_distance } else { p.margin_bottom };
        let mut s = format!(
            r#"<style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="{}" fo:page-height="{}" style:print-orientation="{}" fo:margin-top="{}" fo:margin-bottom="{}" fo:margin-left="{}" fo:margin-right="{}""#,
            pt(p.width),
            pt(p.height),
            if p.width > p.height { "landscape" } else { "portrait" },
            pt(top),
            pt(bottom),
            pt(p.margin_left),
            pt(p.margin_right)
        );
        if let Some(c) = p.page_color {
            let _ = write!(s, r#" fo:background-color="{}""#, color_hex(c));
        }
        if let Some(b) = &p.page_borders {
            for (side, br) in [("top", &b.top), ("bottom", &b.bottom), ("left", &b.left), ("right", &b.right)] {
                if br.is_some() {
                    let _ = write!(s, r#" fo:border-{}="{}""#, side, border_str(br));
                    if let Some(br) = br {
                        let _ = write!(s, r#" fo:padding-{}="{}""#, side, pt(br.space));
                    }
                }
            }
        }
        s.push('>');
        if p.columns > 1 {
            let _ = write!(s, r#"<style:columns fo:column-count="{}" fo:column-gap="{}"/>"#, p.columns, pt(p.column_gap));
        }
        s.push_str("</style:page-layout-properties>");
        if has_h {
            let _ = write!(
                s,
                r#"<style:header-style><style:header-footer-properties fo:min-height="{}" fo:margin-bottom="0pt" style:dynamic-spacing="true"/></style:header-style>"#,
                pt((p.margin_top - p.header_distance).max(0.0))
            );
        } else {
            s.push_str("<style:header-style/>");
        }
        if has_f {
            let _ = write!(
                s,
                r#"<style:footer-style><style:header-footer-properties fo:min-height="{}" fo:margin-top="0pt" style:dynamic-spacing="true"/></style:footer-style>"#,
                pt((p.margin_bottom - p.footer_distance).max(0.0))
            );
        } else {
            s.push_str("<style:footer-style/>");
        }
        s.push_str("</style:page-layout>");
        s
    }

    fn master_page(&mut self, auto: &mut AutoStyles) -> String {
        let doc = self.doc;
        let mut s = String::from(r#"<style:master-page style:name="Standard" style:page-layout-name="pm1">"#);
        let (has_h, has_f) = self.has_hf();
        let parts: [(&str, &Vec<Block>, bool); 4] = [
            ("style:header", &doc.headers.header, true),
            ("style:header-first", &doc.headers.first_header, doc.page.title_page),
            ("style:footer", &doc.headers.footer, true),
            ("style:footer-first", &doc.headers.first_footer, doc.page.title_page),
        ];
        for (tag, blocks, allowed) in parts {
            let is_header = tag.starts_with("style:header");
            if !allowed || (is_header && !has_h) || (!is_header && !has_f) {
                continue;
            }
            let _ = write!(s, "<{}>", tag);
            if blocks.is_empty() {
                s.push_str("<text:p/>");
            }
            self.blocks(&mut s, blocks, auto, false);
            let _ = write!(s, "</{}>", tag);
        }
        s.push_str("</style:master-page>");
        s
    }

    fn blocks(&mut self, out: &mut String, blocks: &[Block], auto: &mut AutoStyles, body: bool) {
        // Open list nesting: (list id, per-level "item open" flags).
        let mut list: Option<(u32, Vec<bool>)> = None;
        fn close_list(out: &mut String, list: &mut Option<(u32, Vec<bool>)>) {
            if let Some((_, levels)) = list.take() {
                for open in levels.iter().rev() {
                    if *open {
                        out.push_str("</text:list-item>");
                    }
                    out.push_str("</text:list>");
                }
            }
        }
        for (bi, b) in blocks.iter().enumerate() {
            match b {
                Block::Para(p) => {
                    let lref = p.props.list.flatten().filter(|l| self.list_styles.contains_key(&l.list));
                    match lref {
                        Some(l) => {
                            if list.as_ref().map(|(id, _)| *id != l.list).unwrap_or(true) {
                                close_list(out, &mut list);
                                let _ = write!(out, r#"<text:list text:style-name="{}">"#, self.list_styles[&l.list]);
                                list = Some((l.list, vec![false]));
                            }
                            let levels = &mut list.as_mut().unwrap().1;
                            let target = l.level as usize + 1;
                            while levels.len() > target {
                                if levels.pop().unwrap() {
                                    out.push_str("</text:list-item>");
                                }
                                out.push_str("</text:list>");
                            }
                            while levels.len() < target {
                                if !*levels.last().unwrap() {
                                    out.push_str("<text:list-item>");
                                    *levels.last_mut().unwrap() = true;
                                }
                                out.push_str("<text:list>");
                                levels.push(false);
                            }
                            if *levels.last().unwrap() {
                                out.push_str("</text:list-item>");
                            }
                            out.push_str("<text:list-item>");
                            *levels.last_mut().unwrap() = true;
                        }
                        None => close_list(out, &mut list),
                    }
                    self.paragraph(out, p, auto, body, body && bi == 0);
                }
                Block::Table(t) => {
                    close_list(out, &mut list);
                    self.table(out, t, auto, body);
                }
            }
        }
        close_list(out, &mut list);
    }

    fn paragraph(&mut self, out: &mut String, p: &Paragraph, auto: &mut AutoStyles, body: bool, first: bool) {
        let style_id = p.props.style.clone().unwrap_or_else(|| "Normal".into());
        let parent = if self.doc.style(&style_id).is_some() { odf_name(&style_id) } else { "Standard".into() };
        let heading = resolve_para(self.doc, &p.props).outline_level;
        let mut direct = p.props.clone();
        direct.style = None;
        direct.list = None;
        let mut extra = String::new();
        if first && self.doc.page.page_number_start != 1 {
            let _ = write!(extra, r#" style:page-number="{}""#, self.doc.page.page_number_start);
        }
        // Split the paragraph at page/column breaks.
        let mut segments: Vec<(Vec<&Run>, Option<&'static str>)> = vec![(Vec::new(), None)];
        for r in &p.runs {
            match r.content {
                Inline::PageBreak => segments.push((Vec::new(), Some("page"))),
                Inline::ColumnBreak => segments.push((Vec::new(), Some("column"))),
                _ => segments.last_mut().unwrap().0.push(r),
            }
        }
        let mark = CharProps { link: None, revision: None, comments: vec![], ..p.mark.clone() };
        for (si, (runs, brk)) in segments.into_iter().enumerate() {
            let mut ex = if si == 0 { extra.clone() } else { String::new() };
            if let Some(b) = brk {
                let _ = write!(ex, r#" fo:break-before="{}""#, b);
            }
            let master = first && si == 0;
            let has_direct = direct != ParaProps::default() || !ex.is_empty() || !mark.is_empty() || master;
            let sname = if has_direct { auto.para(&parent, &direct, &mark, &ex, master) } else { parent.clone() };
            let (tag, lvl) = match heading {
                Some(l) => ("text:h", format!(r#" text:outline-level="{}""#, l + 1)),
                None => ("text:p", String::new()),
            };
            let _ = write!(out, r#"<{} text:style-name="{}"{}>"#, tag, sname, lvl);
            self.runs(out, &runs, auto, body);
            let _ = write!(out, "</{}>", tag);
        }
    }

    fn runs(&mut self, out: &mut String, runs: &[&Run], auto: &mut AutoStyles, body: bool) {
        let mut open_link: Option<&str> = None;
        for r in runs {
            let link = r.props.link.as_deref();
            if link != open_link {
                if open_link.is_some() {
                    out.push_str("</text:a>");
                }
                if let Some(l) = link {
                    let _ = write!(out, r#"<text:a xlink:type="simple" xlink:href="{}">"#, esc_attr(l));
                }
                open_link = link;
            }
            let idx = self.run_counter;
            if body {
                self.run_counter += 1;
                let mut starts: Vec<u32> = self.comment_ranges.iter().filter(|(_, v)| v.0 == idx).map(|(k, _)| *k).collect();
                starts.sort_unstable();
                for c in starts {
                    self.annotation(out, c, auto);
                }
            }
            // Deleted text is omitted: the export shows the document with tracked changes accepted.
            if !r.props.revision.as_ref().map(|v| v.kind == RevisionKind::Delete).unwrap_or(false) {
                self.run(out, r, auto);
            }
            if body {
                let mut ends: Vec<u32> = self.comment_ranges.iter().filter(|(_, v)| v.1 == idx).map(|(k, _)| *k).collect();
                ends.sort_unstable();
                for c in ends {
                    let _ = write!(out, r#"<office:annotation-end office:name="__Annotation__{}"/>"#, c);
                }
            }
        }
        if open_link.is_some() {
            out.push_str("</text:a>");
        }
    }

    fn annotation(&mut self, out: &mut String, id: u32, auto: &mut AutoStyles) {
        let Some(c) = self.doc.comments.iter().find(|c| c.id == id) else { return };
        let _ = write!(out, r#"<office:annotation office:name="__Annotation__{}"><dc:creator>{}</dc:creator>"#, id, esc(&c.author));
        if !c.date.is_empty() {
            let _ = write!(out, "<dc:date>{}</dc:date>", esc(c.date.trim_end_matches('Z')));
        }
        for p in &c.paragraphs {
            out.push_str("<text:p>");
            let runs: Vec<&Run> = p.runs.iter().collect();
            self.runs(out, &runs, auto, false);
            out.push_str("</text:p>");
        }
        if c.paragraphs.is_empty() {
            out.push_str("<text:p/>");
        }
        out.push_str("</office:annotation>");
    }

    fn run(&mut self, out: &mut String, r: &Run, auto: &mut AutoStyles) {
        let mut props = CharProps { link: None, revision: None, comments: vec![], ..r.props.clone() };
        let named = props.style.take().filter(|s| self.doc.style(s).is_some()).map(|s| odf_name(&s));
        let direct = auto.text(&props);
        let spans: Vec<String> = named.into_iter().chain(direct).collect();
        for s in &spans {
            let _ = write!(out, r#"<text:span text:style-name="{}">"#, s);
        }
        self.content(out, r, auto);
        for _ in &spans {
            out.push_str("</text:span>");
        }
    }

    fn content(&mut self, out: &mut String, r: &Run, auto: &mut AutoStyles) {
        match &r.content {
            Inline::Text(t) => text_xml(out, t),
            Inline::Tab => out.push_str("<text:tab/>"),
            Inline::LineBreak => out.push_str("<text:line-break/>"),
            Inline::PageBreak | Inline::ColumnBreak => {}
            Inline::Bookmark(n) => {
                let _ = write!(out, r#"<text:bookmark text:name="{}"/>"#, esc_attr(n));
            }
            Inline::Field(f) => match f {
                Field::PageNumber => out.push_str(r#"<text:page-number text:select-page="current">1</text:page-number>"#),
                Field::PageCount => out.push_str("<text:page-count>1</text:page-count>"),
                Field::Date(pic) => {
                    let _ = write!(out, r#"<text:date text:fixed="false">{}</text:date>"#, esc(&format_date_now(pic)));
                }
                Field::Other { result, .. } => text_xml(out, result),
            },
            Inline::Image(img) => self.image(out, img),
            Inline::FootnoteRef(id) | Inline::EndnoteRef(id) => {
                let foot = matches!(r.content, Inline::FootnoteRef(_));
                let Some(&n) = self.note_ids.get(&(foot, *id)) else { return };
                let notes = if foot { &self.doc.footnotes } else { &self.doc.endnotes };
                let Some(note) = notes.iter().find(|x| x.id == *id) else { return };
                let (class, prefix, style) = if foot { ("footnote", "ftn", "FootnoteText") } else { ("endnote", "edn", "EndnoteText") };
                let cite = if foot { n.to_string() } else { crate::styles::roman(n as u32).to_lowercase() };
                let _ = write!(
                    out,
                    r#"<text:note text:id="{}{}" text:note-class="{}"><text:note-citation>{}</text:note-citation><text:note-body>"#,
                    prefix, n, class, cite
                );
                for p in &note.paragraphs {
                    let mut p = p.clone();
                    if p.props.style.is_none() {
                        p.props.style = Some(style.into());
                    }
                    self.paragraph(out, &p, auto, false, false);
                }
                if note.paragraphs.is_empty() {
                    out.push_str("<text:p/>");
                }
                out.push_str("</text:note-body></text:note>");
            }
        }
    }

    fn image(&mut self, out: &mut String, img: &Image) {
        let key = Arc::as_ptr(&img.data) as usize;
        let name = match self.pictures.get(&key) {
            Some(n) => n.clone(),
            None => {
                let n = format!("Pictures/image{}.{}", self.picture_files.len() + 1, image_ext(&img.data.format));
                self.picture_files.push((n.clone(), img.data.bytes.clone()));
                self.pictures.insert(key, n.clone());
                n
            }
        };
        self.frame_id += 1;
        let _ = write!(
            out,
            r#"<draw:frame draw:style-name="Graphics" draw:name="Image{}" text:anchor-type="as-char" svg:width="{}" svg:height="{}" draw:z-index="0"><draw:image xlink:href="{}" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/>"#,
            self.frame_id,
            pt(img.width),
            pt(img.height),
            name
        );
        if !img.alt.is_empty() {
            let _ = write!(out, "<svg:desc>{}</svg:desc>", esc(&img.alt));
        }
        out.push_str("</draw:frame>");
    }

    fn table(&mut self, out: &mut String, t: &Table, auto: &mut AutoStyles, body: bool) {
        self.table_id += 1;
        let tname = format!("Table{}", self.table_id);
        let ncols = t.rows.iter().map(|r| r.cells.iter().map(|c| c.span.max(1) as usize).sum::<usize>()).max().unwrap_or(1).max(1);
        let mut cols = t.columns.clone();
        cols.resize(ncols, if cols.is_empty() { 72.0 } else { cols.iter().sum::<f32>() / cols.len() as f32 });
        let width: f32 = cols.iter().sum();
        let align = match t.align {
            Align::Center => "center",
            Align::Right => "right",
            _ => "left",
        };
        let tstyle = auto.raw(
            "table",
            "Tbl",
            format!(
                r#"<style:table-properties style:width="{}" table:align="{}" fo:margin-left="{}" table:border-model="collapsing"/>"#,
                pt(width),
                align,
                pt(t.indent)
            ),
        );
        let _ = write!(out, r#"<table:table table:name="{}" table:style-name="{}">"#, tname, tstyle);
        for c in &cols {
            let cs = auto.raw("table-column", "Col", format!(r#"<style:table-column-properties style:column-width="{}"/>"#, pt(*c)));
            let _ = write!(out, r#"<table:table-column table:style-name="{}"/>"#, cs);
        }
        let continue_at = |r: usize, g: usize| -> bool {
            let mut gc = 0;
            for c in &t.rows[r].cells {
                if gc == g {
                    return c.vmerge == VMerge::Continue;
                }
                gc += c.span.max(1) as usize;
            }
            false
        };
        let nrows = t.rows.len();
        let header_rows = t.rows.iter().take_while(|r| r.header).count();
        for (ri, row) in t.rows.iter().enumerate() {
            if ri == 0 && header_rows > 0 {
                out.push_str("<table:table-header-rows>");
            }
            match row.height {
                Some(h) => {
                    let rs = auto.raw("table-row", "Row", format!(r#"<style:table-row-properties style:min-row-height="{}"/>"#, pt(h)));
                    let _ = write!(out, r#"<table:table-row table:style-name="{}">"#, rs);
                }
                None => out.push_str("<table:table-row>"),
            }
            let mut gc = 0usize;
            for cell in &row.cells {
                let span = cell.span.max(1) as usize;
                if cell.vmerge == VMerge::Continue {
                    for _ in 0..span {
                        out.push_str("<table:covered-table-cell/>");
                    }
                    gc += span;
                    continue;
                }
                let mut rows_spanned = 1;
                if cell.vmerge == VMerge::Restart {
                    while ri + rows_spanned < nrows && continue_at(ri + rows_spanned, gc) {
                        rows_spanned += 1;
                    }
                }
                let last_row = ri + rows_spanned >= nrows;
                let last_col = gc + span >= ncols;
                let tb = &t.borders;
                let mut b = Borders {
                    top: if ri == 0 { tb.top } else { tb.between },
                    bottom: if last_row { tb.bottom } else { tb.between },
                    left: if gc == 0 { tb.left } else { tb.inside_v },
                    right: if last_col { tb.right } else { tb.inside_v },
                    ..Default::default()
                };
                if let Some(cb) = &cell.borders {
                    b.top = cb.top.or(b.top);
                    b.bottom = cb.bottom.or(b.bottom);
                    b.left = cb.left.or(b.left);
                    b.right = cb.right.or(b.right);
                }
                let shading = cell
                    .shading
                    .or(if ri == 0 { t.header_shading } else { None })
                    .or(if ri % 2 == 1 { t.band_shading } else { None });
                let valign = match cell.valign {
                    VAlign::Top => "top",
                    VAlign::Center => "middle",
                    VAlign::Bottom => "bottom",
                };
                let mut props = format!(
                    r#"<style:table-cell-properties style:vertical-align="{}" fo:padding-left="{m}" fo:padding-right="{m}" fo:padding-top="0pt" fo:padding-bottom="0pt" fo:border-top="{}" fo:border-bottom="{}" fo:border-left="{}" fo:border-right="{}""#,
                    valign,
                    border_str(&b.top),
                    border_str(&b.bottom),
                    border_str(&b.left),
                    border_str(&b.right),
                    m = pt(t.cell_margin)
                );
                if let Some(c) = shading {
                    let _ = write!(props, r#" fo:background-color="{}""#, color_hex(c));
                }
                props.push_str("/>");
                let cs = auto.raw("table-cell", "Cell", props);
                let _ = write!(out, r#"<table:table-cell table:style-name="{}" office:value-type="string""#, cs);
                if span > 1 {
                    let _ = write!(out, r#" table:number-columns-spanned="{}""#, span);
                }
                if rows_spanned > 1 {
                    let _ = write!(out, r#" table:number-rows-spanned="{}""#, rows_spanned);
                }
                out.push('>');
                let blocks: Vec<Block> = cell.paragraphs.iter().cloned().map(Block::Para).collect();
                if blocks.is_empty() {
                    out.push_str("<text:p/>");
                }
                self.blocks(out, &blocks, auto, body);
                out.push_str("</table:table-cell>");
                for _ in 1..span {
                    out.push_str("<table:covered-table-cell/>");
                }
                gc += span;
            }
            out.push_str("</table:table-row>");
            if ri + 1 == header_rows {
                out.push_str("</table:table-header-rows>");
            }
        }
        out.push_str("</table:table>");
    }
}

fn for_each_para(blocks: &[Block], f: &mut dyn FnMut(&Paragraph)) {
    for b in blocks {
        match b {
            Block::Para(p) => f(p),
            Block::Table(t) => {
                for c in t.rows.iter().flat_map(|r| &r.cells) {
                    c.paragraphs.iter().for_each(&mut *f);
                }
            }
        }
    }
}

/// Text with ODF whitespace encoding: every space after the first of a run, and any space at the start
/// or end of a text chunk, becomes `text:s` so it survives whitespace collapsing.
fn text_xml(out: &mut String, t: &str) {
    let chars: Vec<char> = t.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' => {
                let mut n = 0;
                while i + n < chars.len() && chars[i + n] == ' ' {
                    n += 1;
                }
                let at_edge = i == 0 || i + n == chars.len();
                let (literal, encoded) = if at_edge { (0, n) } else { (1, n - 1) };
                if literal == 1 {
                    out.push(' ');
                }
                match encoded {
                    0 => {}
                    1 => out.push_str("<text:s/>"),
                    k => {
                        let _ = write!(out, r#"<text:s text:c="{}"/>"#, k);
                    }
                }
                i += n;
                continue;
            }
            '\t' => out.push_str("<text:tab/>"),
            '\n' | '\u{000B}' => out.push_str("<text:line-break/>"),
            c => out.push_str(&esc(c.encode_utf8(&mut [0; 4]))),
        }
        i += 1;
    }
}

// ======================================================================================
// Reader
// ======================================================================================

#[derive(Clone, Default, Debug)]
struct OdfStyle {
    family: String,
    parent: Option<String>,
    display: Option<String>,
    para: ParaProps,
    chars: CharProps,
    list_style: Option<String>,
    next: Option<String>,
    col_width: Option<f32>,
    min_height: Option<f32>,
    align: Option<Align>,
    margin_left: Option<f32>,
    cell_borders: Borders,
    cell_bg: Option<Color>,
    valign: Option<VAlign>,
    padding_left: Option<f32>,
    break_column: bool,
    page_number: Option<u32>,
}

struct OdtReader<'a> {
    pkg: &'a Package,
    doc: Document,
    styles: HashMap<String, OdfStyle>,
    /// ODF name → model id for named paragraph/text styles.
    named: HashMap<String, String>,
    list_defs: HashMap<String, u32>,
    list_elems: HashMap<String, Element>,
    layouts: HashMap<String, Element>,
    font_faces: HashMap<String, String>,
    images: HashMap<String, Arc<ImageData>>,
    comments_active: Vec<(String, u32)>,
    pending: Vec<Paragraph>,
}

/// Read an .odt package.
pub fn read(bytes: &[u8]) -> Result<Document, String> {
    let pkg = Package::open(bytes)?;
    let content = xml::parse_bytes(pkg.get("content.xml").ok_or("Not an OpenDocument text file (content.xml missing)")?)?;
    let styles_root = pkg.get("styles.xml").and_then(|b| xml::parse_bytes(b).ok());
    let mut doc = Document::new();
    doc.body.clear();
    doc.default_para.space_after = Some(0.0);
    doc.default_para.line_spacing = Some(LineSpacing::Multiple(1.0));
    doc.default_chars.font = Some("Liberation Serif".into());
    doc.default_chars.size = Some(12.0);
    let mut r = OdtReader {
        pkg: &pkg,
        doc,
        styles: HashMap::new(),
        named: HashMap::new(),
        list_defs: HashMap::new(),
        list_elems: HashMap::new(),
        layouts: HashMap::new(),
        font_faces: HashMap::new(),
        images: HashMap::new(),
        comments_active: Vec::new(),
        pending: Vec::new(),
    };
    let mut tabs_relative = true;
    if let Some(settings) = pkg.get("settings.xml").and_then(|b| xml::parse_bytes(b).ok()) {
        let mut items = Vec::new();
        settings.find_all("config:config-item", &mut items);
        if let Some(i) = items.iter().find(|i| i.attr("config:name") == Some("TabsRelativeToIndent")) {
            tabs_relative = i.text().trim() != "false";
        }
    }
    for root in styles_root.iter().chain(std::iter::once(&content)) {
        if let Some(ff) = root.child("office:font-face-decls") {
            for f in ff.children_named("style:font-face") {
                if let Some(n) = f.attr("style:name") {
                    let fam = f.attr("svg:font-family").unwrap_or(n).trim_matches(|c| c == '\'' || c == '"').to_string();
                    r.font_faces.insert(n.to_string(), fam);
                }
            }
        }
    }
    if let Some(sr) = &styles_root {
        if let Some(st) = sr.child("office:styles") {
            r.read_style_container(st, true);
        }
        if let Some(st) = sr.child("office:automatic-styles") {
            r.read_style_container(st, false);
        }
    }
    r.build_named_styles();
    if let Some(meta) = pkg.get("meta.xml").and_then(|b| xml::parse_bytes(b).ok()) {
        r.read_meta(&meta);
    }
    if let Some(sr) = &styles_root {
        r.read_master(sr);
    }
    // Content automatic styles may reuse names of styles.xml automatic styles; they win inside the body.
    if let Some(st) = content.child("office:automatic-styles") {
        r.read_style_container(st, false);
    }
    let text = content.child("office:body").and_then(|b| b.child("office:text")).ok_or("content.xml has no office:text body")?;
    let mut blocks = Vec::new();
    r.blocks(text, &mut blocks, None);
    if blocks.is_empty() {
        blocks.push(Block::Para(Paragraph::new()));
    }
    r.doc.body = blocks;
    if tabs_relative {
        r.absolutise_tabs();
    }
    Ok(r.doc)
}

fn attr_len(e: &Element, a: &str) -> Option<f32> {
    e.attr(a).and_then(parse_length)
}

fn paras_only(blocks: Vec<Block>) -> Vec<Paragraph> {
    let mut out = Vec::new();
    for b in blocks {
        match b {
            Block::Para(p) => out.push(p),
            Block::Table(t) => {
                for r in t.rows {
                    let mut p = Paragraph::new();
                    for (i, c) in r.cells.into_iter().enumerate() {
                        if i > 0 {
                            p.runs.push(Run { content: Inline::Tab, props: CharProps::default() });
                        }
                        for cp in c.paragraphs {
                            p.runs.extend(cp.runs);
                        }
                    }
                    out.push(p);
                }
            }
        }
    }
    out
}

impl<'a> OdtReader<'a> {
    fn read_style_container(&mut self, el: &Element, named: bool) {
        for s in el.elems() {
            match s.name.as_str() {
                "style:style" => {
                    let Some(name) = s.attr("style:name") else { continue };
                    let st = self.parse_style(s);
                    if named {
                        self.named.insert(name.to_string(), String::new());
                    }
                    self.styles.insert(name.to_string(), st);
                }
                "style:default-style" if s.attr("style:family") == Some("paragraph") => {
                    let st = self.parse_style(s);
                    self.doc.default_chars.merge(&st.chars);
                    self.doc.default_para.merge(&st.para);
                    if let Some(d) = s.child("style:paragraph-properties").and_then(|p| attr_len(p, "style:tab-stop-distance")) {
                        if d > 0.0 {
                            self.doc.settings.default_tab = d;
                        }
                    }
                }
                "text:list-style" => {
                    if let Some(n) = s.attr("style:name") {
                        self.list_elems.insert(n.to_string(), s.clone());
                        self.list_defs.remove(n);
                    }
                }
                "style:page-layout" => {
                    if let Some(n) = s.attr("style:name") {
                        self.layouts.insert(n.to_string(), s.clone());
                    }
                }
                _ => {}
            }
        }
    }

    fn parse_style(&self, s: &Element) -> OdfStyle {
        let mut st = OdfStyle {
            family: s.attr("style:family").unwrap_or("paragraph").to_string(),
            parent: s.attr("style:parent-style-name").map(|x| x.to_string()),
            display: s.attr("style:display-name").map(|x| x.to_string()),
            list_style: s.attr("style:list-style-name").filter(|x| !x.is_empty()).map(|x| x.to_string()),
            next: s.attr("style:next-style-name").map(|x| x.to_string()),
            ..Default::default()
        };
        if let Some(tp) = s.child("style:text-properties") {
            st.chars = self.text_props(tp);
        }
        if let Some(pp) = s.child("style:paragraph-properties") {
            st.para = para_props(pp);
            st.break_column = pp.attr("fo:break-before") == Some("column");
            st.page_number = pp.attr("style:page-number").and_then(|v| v.parse().ok());
        }
        st.para.outline_level = s.attr("style:default-outline-level").and_then(|v| v.parse::<u8>().ok()).filter(|v| *v >= 1).map(|v| v - 1);
        if let Some(tp) = s.child("style:table-properties") {
            st.align = match tp.attr("table:align") {
                Some("center") => Some(Align::Center),
                Some("right") => Some(Align::Right),
                _ => Some(Align::Left),
            };
            st.margin_left = attr_len(tp, "fo:margin-left");
        }
        if let Some(cp) = s.child("style:table-column-properties") {
            st.col_width = attr_len(cp, "style:column-width");
        }
        if let Some(rp) = s.child("style:table-row-properties") {
            st.min_height = attr_len(rp, "style:min-row-height").or_else(|| attr_len(rp, "style:row-height"));
        }
        if let Some(cp) = s.child("style:table-cell-properties") {
            st.cell_borders = borders_of(cp);
            st.cell_bg = cp.attr("fo:background-color").and_then(css_color);
            st.valign = match cp.attr("style:vertical-align") {
                Some("middle") => Some(VAlign::Center),
                Some("bottom") => Some(VAlign::Bottom),
                Some(_) => Some(VAlign::Top),
                None => None,
            };
            st.padding_left = attr_len(cp, "fo:padding-left").or_else(|| attr_len(cp, "fo:padding"));
        }
        st
    }

    fn text_props(&self, tp: &Element) -> CharProps {
        let mut c = CharProps::default();
        if let Some(f) = tp.attr("style:font-name") {
            c.font = Some(self.font_faces.get(f).cloned().unwrap_or_else(|| f.to_string()));
        } else if let Some(f) = tp.attr("fo:font-family") {
            c.font = Some(f.trim_matches(|c| c == '\'' || c == '"').to_string());
        }
        if let Some(v) = tp.attr("fo:font-size") {
            if !v.ends_with('%') {
                c.size = parse_length(v);
            }
        }
        if let Some(v) = tp.attr("fo:font-weight") {
            c.bold = Some(v == "bold" || v.parse::<u32>().map(|w| w >= 600).unwrap_or(false));
        }
        if let Some(v) = tp.attr("fo:font-style") {
            c.italic = Some(v == "italic" || v == "oblique");
        }
        if let Some(v) = tp.attr("style:text-underline-style") {
            c.underline = Some(if v == "none" {
                Underline::None
            } else if tp.attr("style:text-underline-type") == Some("double") {
                Underline::Double
            } else if tp.attr("style:text-underline-mode") == Some("skip-white-space") {
                Underline::Words
            } else {
                match v {
                    "dotted" => Underline::Dotted,
                    "dash" | "long-dash" | "dot-dash" | "dot-dot-dash" => Underline::Dashed,
                    "wave" => Underline::Wavy,
                    _ if matches!(tp.attr("style:text-underline-width"), Some("bold" | "thick")) => Underline::Thick,
                    _ => Underline::Single,
                }
            });
        }
        if let Some(v) = tp.attr("style:text-line-through-style") {
            if v == "none" {
                c.strike = Some(false);
            } else if tp.attr("style:text-line-through-type") == Some("double") {
                c.double_strike = Some(true);
            } else {
                c.strike = Some(true);
            }
        }
        if let Some(v) = tp.attr("fo:color").and_then(css_color) {
            c.color = Some(v);
        }
        if let Some(v) = tp.attr("fo:background-color") {
            c.highlight = css_color(v);
        }
        if let Some(v) = tp.attr("style:text-position") {
            let first = v.split_whitespace().next().unwrap_or("0%");
            c.vert_align = Some(match first {
                "super" => VertAlign::Superscript,
                "sub" => VertAlign::Subscript,
                p => match p.trim_end_matches('%').parse::<f32>() {
                    Ok(x) if x > 0.0 => VertAlign::Superscript,
                    Ok(x) if x < 0.0 => VertAlign::Subscript,
                    _ => VertAlign::Baseline,
                },
            });
        }
        if let Some(v) = tp.attr("fo:text-transform") {
            c.caps = Some(v == "uppercase");
        }
        if let Some(v) = tp.attr("fo:font-variant") {
            c.small_caps = Some(v == "small-caps");
        }
        if let Some(v) = tp.attr("text:display") {
            c.hidden = Some(v == "none");
        }
        if let Some(v) = tp.attr("fo:letter-spacing") {
            c.spacing = if v == "normal" { Some(0.0) } else { parse_length(v) };
        }
        c
    }

    fn build_named_styles(&mut self) {
        let mut names: Vec<String> = self.named.keys().cloned().collect();
        names.sort();
        for n in &names {
            self.named.insert(n.clone(), model_id(n));
        }
        for n in names {
            let st = self.styles[&n].clone();
            let kind = match st.family.as_str() {
                "paragraph" => StyleKind::Paragraph,
                "text" => StyleKind::Character,
                _ => continue,
            };
            let id = self.named[&n].clone();
            let mut para = st.para.clone();
            if let Some(ls) = &st.list_style {
                if let Some(lid) = self.list_id(ls) {
                    para.list = Some(Some(ListRef { list: lid, level: 0 }));
                }
            }
            let existing = self.doc.styles.iter().position(|s| s.id == id);
            let name = match existing {
                Some(i) => self.doc.styles[i].name.clone(),
                None => st.display.clone().unwrap_or_else(|| n.replace("_20_", " ")),
            };
            let based_on = st.parent.as_ref().map(|p| self.named.get(p).cloned().unwrap_or_else(|| model_id(p)));
            let next = st.next.as_ref().map(|p| self.named.get(p).cloned().unwrap_or_else(|| model_id(p)));
            let quick = existing.map(|i| self.doc.styles[i].quick).unwrap_or(false);
            let style = Style { id: id.clone(), name, kind, based_on, next, para, chars: st.chars.clone(), quick };
            match existing {
                Some(i) => self.doc.styles[i] = style,
                None => self.doc.styles.push(style),
            }
        }
    }

    fn list_id(&mut self, name: &str) -> Option<u32> {
        if let Some(id) = self.list_defs.get(name) {
            return Some(*id);
        }
        let el = self.list_elems.get(name)?.clone();
        let mut def = ListDef::numbered(0);
        for lv in el.elems() {
            let Some(level) = lv.attr("text:level").and_then(|v| v.parse::<usize>().ok()).filter(|l| (1..=9).contains(l)) else { continue };
            let l = &mut def.levels[level - 1];
            if let Some(pr) = lv.child("style:list-level-properties") {
                if let Some(la) = pr.child("style:list-level-label-alignment") {
                    if let Some(m) = attr_len(la, "fo:margin-left") {
                        l.indent = m;
                    }
                    if let Some(t) = attr_len(la, "fo:text-indent") {
                        l.hanging = -t;
                    }
                } else {
                    let before = attr_len(pr, "text:space-before").unwrap_or(0.0);
                    let min = attr_len(pr, "text:min-label-width").unwrap_or(18.0);
                    l.indent = before + min;
                    l.hanging = min;
                }
            }
            l.font = lv
                .child("style:text-properties")
                .and_then(|t| t.attr("style:font-name").or_else(|| t.attr("fo:font-family")))
                .map(|f| self.font_faces.get(f).cloned().unwrap_or_else(|| f.to_string()));
            match lv.name.as_str() {
                "text:list-level-style-bullet" => {
                    l.format = NumFormat::Bullet;
                    l.text = lv.attr("text:bullet-char").unwrap_or("•").to_string();
                    if let Some(m) = map_symbol_bullet(&l.text, l.font.as_deref()) {
                        l.text = m;
                        l.font = None;
                    }
                }
                "text:list-level-style-number" => {
                    l.format = match lv.attr("style:num-format").unwrap_or("1") {
                        "a" => NumFormat::LowerLetter,
                        "A" => NumFormat::UpperLetter,
                        "i" => NumFormat::LowerRoman,
                        "I" => NumFormat::UpperRoman,
                        "" => NumFormat::None,
                        f if f.starts_with("01") => NumFormat::DecimalZero,
                        _ => NumFormat::Decimal,
                    };
                    let display = lv.attr("text:display-levels").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).clamp(1, level);
                    let mut t = lv.attr("style:num-prefix").unwrap_or("").to_string();
                    for k in (level + 1 - display)..=level {
                        let _ = write!(t, "%{}", k);
                        if k < level {
                            t.push('.');
                        }
                    }
                    t.push_str(lv.attr("style:num-suffix").unwrap_or(""));
                    l.text = t;
                    l.start = lv.attr("text:start-value").and_then(|v| v.parse().ok()).unwrap_or(1);
                }
                _ => {
                    l.format = NumFormat::None;
                    l.text.clear();
                }
            }
        }
        let id = self.doc.lists.iter().map(|l| l.id).max().unwrap_or(0) + 1;
        def.id = id;
        self.doc.lists.push(def);
        self.list_defs.insert(name.to_string(), id);
        Some(id)
    }

    fn read_meta(&mut self, meta: &Element) {
        let Some(m) = meta.child("office:meta") else { return };
        let p = &mut self.doc.properties;
        let date = |t: &str| {
            if t.len() >= 19 && !t.ends_with('Z') && !t[19..].contains(['+', '-']) {
                format!("{}Z", &t[..19])
            } else {
                t.to_string()
            }
        };
        for e in m.elems() {
            let t = e.text().trim().to_string();
            match e.name.as_str() {
                "dc:title" => p.title = t,
                "dc:subject" => p.subject = t,
                "meta:initial-creator" => p.author = t,
                "meta:keyword" => {
                    p.keywords = if p.keywords.is_empty() { t } else { format!("{}, {}", p.keywords, t) };
                }
                "dc:description" => p.description = t,
                "dc:creator" => p.last_modified_by = t,
                "meta:creation-date" => p.created = date(&t),
                "dc:date" => p.modified = date(&t),
                _ => {}
            }
        }
    }

    fn read_master(&mut self, styles_root: &Element) {
        let Some(ms) = styles_root.child("office:master-styles") else { return };
        let master = ms
            .children_named("style:master-page")
            .find(|m| m.attr("style:name") == Some("Standard"))
            .or_else(|| ms.child("style:master-page"));
        let Some(master) = master else { return };
        let layout_name = master.attr("style:page-layout-name").unwrap_or("");
        if let Some(layout) = self.layouts.get(layout_name).cloned() {
            let page = &mut self.doc.page;
            if let Some(pp) = layout.child("style:page-layout-properties") {
                page.width = attr_len(pp, "fo:page-width").unwrap_or(page.width);
                page.height = attr_len(pp, "fo:page-height").unwrap_or(page.height);
                page.margin_top = attr_len(pp, "fo:margin-top").unwrap_or(page.margin_top);
                page.margin_bottom = attr_len(pp, "fo:margin-bottom").unwrap_or(page.margin_bottom);
                page.margin_left = attr_len(pp, "fo:margin-left").unwrap_or(page.margin_left);
                page.margin_right = attr_len(pp, "fo:margin-right").unwrap_or(page.margin_right);
                page.page_color = pp.attr("fo:background-color").and_then(css_color);
                let b = borders_of(pp);
                if !b.is_empty() {
                    page.page_borders = Some(b);
                }
                if let Some(c) = pp.child("style:columns") {
                    page.columns = c.attr("fo:column-count").and_then(|v| v.parse().ok()).unwrap_or(1u32).max(1);
                    if let Some(g) = attr_len(c, "fo:column-gap") {
                        page.column_gap = g;
                    }
                }
            }
            let extra = |tag: &str, margin: &str| {
                layout.child(tag).and_then(|h| h.child("style:header-footer-properties")).map(|h| {
                    attr_len(h, "fo:min-height").or_else(|| attr_len(h, "svg:height")).unwrap_or(0.0) + attr_len(h, margin).unwrap_or(0.0)
                })
            };
            let header_extra = extra("style:header-style", "fo:margin-bottom");
            let footer_extra = extra("style:footer-style", "fo:margin-top");
            let has_header = master.child("style:header").is_some() || master.child("style:header-first").is_some();
            let has_footer = master.child("style:footer").is_some() || master.child("style:footer-first").is_some();
            if let (true, Some(x)) = (has_header, header_extra) {
                page.header_distance = page.margin_top;
                page.margin_top += x;
            }
            if let (true, Some(x)) = (has_footer, footer_extra) {
                page.footer_distance = page.margin_bottom;
                page.margin_bottom += x;
            }
        }
        for (tag, slot) in [("style:header", 0), ("style:footer", 1), ("style:header-first", 2), ("style:footer-first", 3)] {
            if let Some(h) = master.child(tag) {
                if h.attr("style:display") == Some("false") {
                    continue;
                }
                let mut blocks = Vec::new();
                self.blocks(h, &mut blocks, None);
                let hf = &mut self.doc.headers;
                match slot {
                    0 => hf.header = blocks,
                    1 => hf.footer = blocks,
                    2 => hf.first_header = blocks,
                    _ => hf.first_footer = blocks,
                }
                if slot >= 2 {
                    self.doc.page.title_page = true;
                }
            }
        }
    }

    /// Resolve a paragraph style name: (model style id, direct props, paragraph-level char props, list style, style).
    fn para_style(&self, name: Option<&str>) -> (String, ParaProps, CharProps, Option<String>, Option<OdfStyle>) {
        let normal = || ("Normal".to_string(), ParaProps::default(), CharProps::default(), None, None);
        let Some(name) = name else { return normal() };
        if let Some(id) = self.named.get(name) {
            let ls = self.styles.get(name).and_then(|s| s.list_style.clone());
            return (id.clone(), ParaProps::default(), CharProps::default(), ls, None);
        }
        let Some(st) = self.styles.get(name) else { return normal() };
        let parent = st.parent.as_ref().and_then(|p| self.named.get(p).cloned()).unwrap_or_else(|| "Normal".into());
        let mut para = st.para.clone();
        para.outline_level = None;
        let ls = st.list_style.clone().or_else(|| st.parent.as_ref().and_then(|p| self.styles.get(p)).and_then(|s| s.list_style.clone()));
        (parent, para, st.chars.clone(), ls, Some(st.clone()))
    }

    fn blocks(&mut self, el: &Element, out: &mut Vec<Block>, list: Option<(String, u8)>) {
        for c in el.elems() {
            match c.name.as_str() {
                "text:p" | "text:h" => {
                    let p = self.paragraph(c, list.clone());
                    out.push(Block::Para(p));
                    for p in self.pending.drain(..) {
                        out.push(Block::Para(p));
                    }
                }
                "text:list" => {
                    let style = c.attr("text:style-name").map(|s| s.to_string());
                    let next = match (&list, style) {
                        (None, s) => Some((s.unwrap_or_default(), 0u8)),
                        (Some((s, l)), ns) => Some((if s.is_empty() { ns.unwrap_or_default() } else { s.clone() }, l + 1)),
                    };
                    for item in c.elems() {
                        match item.name.as_str() {
                            "text:list-item" => self.blocks(item, out, next.clone()),
                            "text:list-header" => self.blocks(item, out, None),
                            _ => {}
                        }
                    }
                }
                "table:table" => {
                    let t = self.table(c);
                    out.push(Block::Table(t));
                }
                "text:section" | "text:index-body" | "text:table-of-content" | "text:alphabetical-index" | "text:illustration-index"
                | "text:table-index" | "text:user-index" | "text:bibliography" | "text:index-title" => self.blocks(c, out, list.clone()),
                _ => {}
            }
        }
    }

    fn paragraph(&mut self, el: &Element, list: Option<(String, u8)>) -> Paragraph {
        let (mut style, direct, base, list_style, st) = self.para_style(el.attr("text:style-name"));
        if let Some(n) = st.as_ref().and_then(|s| s.page_number) {
            if self.doc.body.is_empty() {
                self.doc.page.page_number_start = n;
            }
        }
        let mut p = Paragraph { props: direct, mark: base.clone(), ..Default::default() };
        if el.name == "text:h" {
            let lvl = el.attr("text:outline-level").and_then(|v| v.parse::<u8>().ok()).unwrap_or(1).clamp(1, 9);
            let style_outline = crate::styles::style_para_props(&self.doc, &style).outline_level;
            if style_outline != Some(lvl - 1) {
                let h = format!("Heading{}", lvl);
                if self.doc.style(&h).is_some() && !style.starts_with("Heading") {
                    style = h;
                } else {
                    p.props.outline_level = Some(lvl - 1);
                }
            }
        }
        p.props.style = if style == "Normal" { None } else { Some(style) };
        match list {
            Some((ls, level)) => {
                let name = if ls.is_empty() { list_style.unwrap_or_default() } else { ls };
                if let Some(id) = self.list_id(&name) {
                    p.props.list = Some(Some(ListRef { list: id, level: level.min(8) }));
                }
            }
            None => {
                if let Some(id) = list_style.and_then(|ls| self.list_id(&ls)) {
                    p.props.list = Some(Some(ListRef { list: id, level: 0 }));
                }
            }
        }
        let mut runs = Vec::new();
        let mut ws = WsState { at_start: true, prev_space: false };
        self.inline(el, &base, &mut runs, &mut ws);
        // Drop a trailing collapsible space.
        if ws.prev_space {
            if let Some(Run { content: Inline::Text(t), .. }) = runs.last_mut() {
                if t.ends_with(' ') {
                    t.pop();
                }
            }
            if matches!(runs.last(), Some(Run { content: Inline::Text(t), .. }) if t.is_empty()) {
                runs.pop();
            }
        }
        for r in runs.iter_mut() {
            if let Inline::Text(t) = &mut r.content {
                if t.contains('\u{0}') {
                    *t = t.replace('\u{0}', " ");
                }
            }
        }
        p.runs = runs;
        crate::io::docx::merge_runs(&mut p.runs);
        if st.map(|s| s.break_column).unwrap_or(false) {
            p.runs.insert(0, Run { content: Inline::ColumnBreak, props: CharProps::default() });
        }
        p
    }

    fn inline(&mut self, el: &Element, props: &CharProps, out: &mut Vec<Run>, ws: &mut WsState) {
        for n in &el.children {
            match n {
                Node::Text(t) => {
                    let mut s = String::new();
                    for c in t.chars() {
                        let c = if matches!(c, '\n' | '\r' | '\t') { ' ' } else { c };
                        if c == ' ' {
                            if ws.at_start || ws.prev_space {
                                continue;
                            }
                            ws.prev_space = true;
                        } else {
                            ws.prev_space = false;
                            ws.at_start = false;
                        }
                        s.push(c);
                    }
                    if !s.is_empty() {
                        self.push(out, Inline::Text(s), props);
                    }
                }
                Node::Element(e) => self.inline_elem(e, props, out, ws),
            }
        }
    }

    fn inline_elem(&mut self, e: &Element, props: &CharProps, out: &mut Vec<Run>, ws: &mut WsState) {
        let solid = |ws: &mut WsState| {
            ws.prev_space = false;
            ws.at_start = false;
        };
        match e.name.as_str() {
            "text:span" => {
                let mut p = props.clone();
                if let Some(sn) = e.attr("text:style-name") {
                    if let Some(id) = self.named.get(sn).cloned() {
                        p.style = Some(id);
                    } else if let Some(st) = self.styles.get(sn) {
                        if let Some(par) = st.parent.as_ref().and_then(|x| self.named.get(x)).cloned() {
                            p.style = Some(par);
                        }
                        p.merge(&st.chars.clone());
                    }
                }
                self.inline(e, &p, out, ws);
            }
            "text:a" => {
                let mut p = props.clone();
                if let Some(h) = e.attr("xlink:href") {
                    p.link = Some(h.to_string());
                }
                self.inline(e, &p, out, ws);
            }
            "text:s" => {
                let n = e.attr("text:c").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(1000);
                self.push(out, Inline::Text("\u{0}".repeat(n)), props);
                solid(ws);
            }
            "text:tab" => {
                self.push(out, Inline::Tab, props);
                solid(ws);
            }
            "text:line-break" => {
                self.push(out, Inline::LineBreak, props);
                ws.prev_space = false;
                ws.at_start = true;
            }
            "text:bookmark" | "text:bookmark-start" => {
                if let Some(n) = e.attr("text:name") {
                    self.push(out, Inline::Bookmark(n.to_string()), props);
                }
            }
            "text:page-number" => {
                self.push(out, Inline::Field(Field::PageNumber), props);
                solid(ws);
            }
            "text:page-count" => {
                self.push(out, Inline::Field(Field::PageCount), props);
                solid(ws);
            }
            "text:date" | "text:time" => {
                let pic = if e.name == "text:time" { "h:mm am/pm" } else { "M/d/yyyy" };
                self.push(out, Inline::Field(Field::Date(pic.into())), props);
                solid(ws);
            }
            "text:note" => {
                let foot = e.attr("text:note-class") != Some("endnote");
                let mut blocks = Vec::new();
                if let Some(b) = e.child("text:note-body") {
                    let saved = std::mem::take(&mut self.pending);
                    self.blocks(b, &mut blocks, None);
                    self.pending = saved;
                }
                let id = self.doc.next_note_id();
                let note = Note { id, paragraphs: paras_only(blocks) };
                if foot {
                    self.doc.footnotes.push(note);
                    self.push(out, Inline::FootnoteRef(id), props);
                } else {
                    self.doc.endnotes.push(note);
                    self.push(out, Inline::EndnoteRef(id), props);
                }
                solid(ws);
            }
            "office:annotation" => {
                let id = self.doc.next_comment_id();
                let mut blocks = Vec::new();
                self.blocks(e, &mut blocks, None);
                let author = e.child("dc:creator").map(|c| c.text()).unwrap_or_default();
                let date = e.child("dc:date").map(|c| c.text()).unwrap_or_default();
                let date = if date.len() >= 19 { format!("{}Z", &date[..19]) } else { date };
                let initials: String = author.split_whitespace().filter_map(|w| w.chars().next()).collect();
                self.doc.comments.push(Comment { id, author, initials, date, paragraphs: paras_only(blocks), resolved: false });
                if let Some(name) = e.attr("office:name") {
                    self.comments_active.push((name.to_string(), id));
                }
            }
            "office:annotation-end" => {
                if let Some(name) = e.attr("office:name") {
                    self.comments_active.retain(|(n, _)| n != name);
                }
            }
            "draw:frame" => {
                if let Some(img) = e.child("draw:image") {
                    if let Some(data) = self.load_image(img) {
                        let (nw, nh) = natural_size(&data, 468.0);
                        let w = attr_len(e, "svg:width").unwrap_or(nw);
                        let h = attr_len(e, "svg:height").unwrap_or(nh);
                        let alt = e.child("svg:desc").or_else(|| e.child("svg:title")).map(|d| d.text()).unwrap_or_default();
                        self.push(out, Inline::Image(Image { data, width: w, height: h, alt }), props);
                        solid(ws);
                    }
                } else if let Some(tb) = e.child("draw:text-box") {
                    let mut blocks = Vec::new();
                    let saved = std::mem::take(&mut self.pending);
                    self.blocks(tb, &mut blocks, None);
                    let mut pend = saved;
                    pend.extend(paras_only(blocks));
                    self.pending = pend;
                }
            }
            "text:soft-page-break" | "text:bookmark-end" | "text:reference-mark-end" | "text:change" | "text:change-start"
            | "text:change-end" | "text:toc-mark" | "text:alphabetical-index-mark" | "office:annotation-end-x" => {}
            _ => self.inline(e, props, out, ws),
        }
    }

    fn push(&self, out: &mut Vec<Run>, content: Inline, props: &CharProps) {
        let mut p = props.clone();
        if !self.comments_active.is_empty() {
            let mut ids: Vec<u32> = self.comments_active.iter().map(|(_, i)| *i).collect();
            ids.sort_unstable();
            p.comments = ids;
        }
        out.push(Run { content, props: p });
    }

    fn load_image(&mut self, img: &Element) -> Option<Arc<ImageData>> {
        let href = img.attr("xlink:href").unwrap_or("");
        if let Some(d) = self.images.get(href) {
            return Some(d.clone());
        }
        let bytes = if let Some(bin) = img.child("office:binary-data") {
            base64_decode(&bin.text())?
        } else if href.is_empty() || href.contains("://") {
            return None;
        } else {
            self.pkg.get(href.trim_start_matches("./"))?.to_vec()
        };
        let d = image_data(bytes);
        self.images.insert(href.to_string(), d.clone());
        Some(d)
    }

    fn table(&mut self, el: &Element) -> Table {
        let mut t = Table { borders: Borders::default(), style: None, ..Default::default() };
        if let Some(st) = el.attr("table:style-name").and_then(|n| self.styles.get(n)) {
            t.align = st.align.unwrap_or(Align::Left);
            t.indent = if t.align == Align::Left { st.margin_left.unwrap_or(0.0) } else { 0.0 };
        }
        let mut cols = Vec::new();
        let mut col_elems = Vec::new();
        collect(el, "table:table-column", &["table:table-columns", "table:table-header-columns", "table:table-column-group"], &mut col_elems);
        for c in col_elems {
            let n = c.attr("table:number-columns-repeated").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(64);
            let w = c.attr("table:style-name").and_then(|s| self.styles.get(s)).and_then(|s| s.col_width).unwrap_or(72.0);
            cols.extend(std::iter::repeat_n(w, n));
        }
        let mut rows: Vec<(&Element, bool)> = Vec::new();
        for c in el.elems() {
            match c.name.as_str() {
                "table:table-row" => rows.push((c, false)),
                "table:table-header-rows" => c.children_named("table:table-row").for_each(|r| rows.push((r, true))),
                "table:table-rows" | "table:table-row-group" => c.children_named("table:table-row").for_each(|r| rows.push((r, false))),
                _ => {}
            }
        }
        let mut cell_borders: Vec<Vec<Borders>> = Vec::new();
        let mut rowspan_left: Vec<usize> = Vec::new();
        for (re, header) in rows {
            let repeat = re.attr("table:number-rows-repeated").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(32);
            for _ in 0..repeat {
                let mut row = Row { header, ..Default::default() };
                row.height = re.attr("table:style-name").and_then(|s| self.styles.get(s)).and_then(|s| s.min_height);
                let mut rb = Vec::new();
                let mut g = 0usize;
                // Covered cells that are part of the previous cell's column span (already counted in `g`).
                let mut skip_covered = 0u32;
                for ce in re.elems() {
                    let reps = ce.attr("table:number-columns-repeated").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(64);
                    for _ in 0..reps {
                        match ce.name.as_str() {
                            "table:table-cell" => {
                                let span = ce.attr("table:number-columns-spanned").and_then(|v| v.parse::<u32>().ok()).unwrap_or(1).max(1);
                                let rspan = ce.attr("table:number-rows-spanned").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).max(1);
                                let mut cell = Cell { span, paragraphs: Vec::new(), ..Default::default() };
                                let mut b = Borders::default();
                                if let Some(st) = ce.attr("table:style-name").and_then(|s| self.styles.get(s)) {
                                    cell.shading = st.cell_bg;
                                    cell.valign = st.valign.unwrap_or_default();
                                    b = st.cell_borders.clone();
                                    if let Some(p) = st.padding_left {
                                        t.cell_margin = p;
                                    }
                                }
                                if rowspan_left.len() < g + span as usize {
                                    rowspan_left.resize(g + span as usize, 0);
                                }
                                if rspan > 1 {
                                    cell.vmerge = VMerge::Restart;
                                    for k in 0..span as usize {
                                        rowspan_left[g + k] = rspan - 1;
                                    }
                                }
                                let mut blocks = Vec::new();
                                self.blocks(ce, &mut blocks, None);
                                cell.paragraphs = paras_only(blocks);
                                if cell.paragraphs.is_empty() {
                                    cell.paragraphs.push(Paragraph::new());
                                }
                                row.cells.push(cell);
                                rb.push(b);
                                g += span as usize;
                                skip_covered = span - 1;
                            }
                            "table:covered-table-cell" => {
                                if skip_covered > 0 {
                                    skip_covered -= 1;
                                    continue;
                                }
                                if rowspan_left.len() <= g {
                                    rowspan_left.resize(g + 1, 0);
                                }
                                if rowspan_left[g] > 0 {
                                    rowspan_left[g] -= 1;
                                    row.cells.push(Cell { vmerge: VMerge::Continue, ..Default::default() });
                                    rb.push(Borders::default());
                                }
                                g += 1;
                            }
                            _ => {}
                        }
                    }
                }
                cell_borders.push(rb);
                t.rows.push(row);
            }
        }
        // Continue cells: merge adjacent ones to match the span of the cell above.
        for ri in 1..t.rows.len() {
            let above: Vec<(usize, u32)> = {
                let mut g = 0;
                t.rows[ri - 1].cells.iter().map(|c| {
                    let r = (g, c.span);
                    g += c.span as usize;
                    r
                }).collect()
            };
            let row = &mut t.rows[ri];
            let mut out: Vec<Cell> = Vec::new();
            let mut g = 0usize;
            let mut i = 0;
            while i < row.cells.len() {
                let c = row.cells[i].clone();
                if c.vmerge == VMerge::Continue {
                    let want = above.iter().find(|(ag, _)| *ag == g).map(|(_, s)| *s).unwrap_or(1);
                    let mut total = c.span;
                    let mut j = i + 1;
                    while total < want && j < row.cells.len() && row.cells[j].vmerge == VMerge::Continue {
                        total += row.cells[j].span;
                        j += 1;
                    }
                    g += total as usize;
                    out.push(Cell { span: total, ..c });
                    i = j;
                } else {
                    g += c.span as usize;
                    out.push(c);
                    i += 1;
                }
            }
            row.cells = out;
        }
        // Table borders from the corner cells.
        if let Some(first) = cell_borders.first().and_then(|r| r.first()) {
            t.borders.top = first.top;
            t.borders.left = first.left;
            t.borders.between = cell_borders.get(1).and_then(|r| r.first()).and_then(|b| b.top).or(first.bottom);
            t.borders.inside_v = cell_borders[0].get(1).and_then(|b| b.left).or(first.right);
            t.borders.bottom = cell_borders.last().and_then(|r| r.first()).and_then(|b| b.bottom);
            t.borders.right = cell_borders[0].last().and_then(|b| b.right);
        }
        let ncols = t.rows.iter().map(|r| r.cells.iter().map(|c| c.span as usize).sum::<usize>()).max().unwrap_or(1).max(1);
        if cols.len() < ncols {
            cols.resize(ncols, 72.0);
        }
        cols.truncate(ncols);
        t.columns = cols;
        if t.rows.is_empty() {
            t.rows.push(Row { cells: vec![Cell::default()], ..Default::default() });
        }
        t
    }

    /// ODF tab positions are relative to the paragraph indent by default; the model's are absolute.
    fn absolutise_tabs(&mut self) {
        let snapshot = self.doc.clone();
        let mut fix = |p: &mut Paragraph| {
            if p.props.tabs.is_empty() {
                return;
            }
            let ind = resolve_para(&snapshot, &p.props).indent_left;
            for t in p.props.tabs.iter_mut() {
                t.pos += ind;
            }
        };
        for b in self.doc.body.iter_mut() {
            match b {
                Block::Para(p) => fix(p),
                Block::Table(t) => t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()).flat_map(|c| c.paragraphs.iter_mut()).for_each(&mut fix),
            }
        }
        for st in self.doc.styles.iter_mut() {
            if !st.para.tabs.is_empty() {
                let ind = crate::styles::style_para_props(&snapshot, &st.id).indent_left.unwrap_or(0.0);
                for t in st.para.tabs.iter_mut() {
                    t.pos += ind;
                }
            }
        }
    }
}

struct WsState {
    at_start: bool,
    prev_space: bool,
}

fn collect<'e>(el: &'e Element, name: &str, wrappers: &[&str], out: &mut Vec<&'e Element>) {
    for c in el.elems() {
        if c.name == name {
            out.push(c);
        } else if wrappers.contains(&c.name.as_str()) {
            collect(c, name, wrappers, out);
        }
    }
}

fn borders_of(e: &Element) -> Borders {
    let mut b = Borders::default();
    let all = e.attr("fo:border");
    let pad_all = attr_len(e, "fo:padding");
    for side in ["top", "bottom", "left", "right"] {
        let Some(v) = e.attr(&format!("fo:border-{}", side)).or(all) else { continue };
        let v = parse_border_str(v).map(|mut br| {
            br.space = attr_len(e, &format!("fo:padding-{}", side)).or(pad_all).unwrap_or(0.0);
            br
        });
        match side {
            "top" => b.top = v,
            "bottom" => b.bottom = v,
            "left" => b.left = v,
            _ => b.right = v,
        }
    }
    b
}

fn para_props(pp: &Element) -> ParaProps {
    let mut p = ParaProps {
        align: match pp.attr("fo:text-align") {
            Some("center") => Some(Align::Center),
            Some("end" | "right") => Some(Align::Right),
            Some("justify") => Some(Align::Justify),
            Some(_) => Some(Align::Left),
            None => None,
        },
        indent_left: attr_len(pp, "fo:margin-left"),
        indent_right: attr_len(pp, "fo:margin-right"),
        indent_first: attr_len(pp, "fo:text-indent"),
        space_before: attr_len(pp, "fo:margin-top"),
        space_after: attr_len(pp, "fo:margin-bottom"),
        ..Default::default()
    };
    if let Some(lh) = pp.attr("fo:line-height") {
        if let Some(pc) = lh.strip_suffix('%') {
            if let Ok(v) = pc.parse::<f32>() {
                p.line_spacing = Some(LineSpacing::Multiple((v / 100.0 * 1000.0).round() / 1000.0));
            }
        } else if lh == "normal" {
            p.line_spacing = Some(LineSpacing::Multiple(1.0));
        } else if let Some(v) = parse_length(lh) {
            p.line_spacing = Some(LineSpacing::Exactly(v));
        }
    } else if let Some(v) = attr_len(pp, "style:line-height-at-least") {
        p.line_spacing = Some(LineSpacing::AtLeast(v));
    }
    if let Some(v) = pp.attr("loext:contextual-spacing").or_else(|| pp.attr("style:contextual-spacing")) {
        p.contextual_spacing = Some(v == "true");
    }
    if let Some(v) = pp.attr("fo:keep-with-next") {
        p.keep_next = Some(v == "always");
    }
    if let Some(v) = pp.attr("fo:keep-together") {
        p.keep_lines = Some(v == "always");
    }
    match pp.attr("fo:break-before") {
        Some("page") => p.page_break_before = Some(true),
        Some("auto") => p.page_break_before = Some(false),
        _ => {}
    }
    if let Some(v) = pp.attr("fo:widows").or_else(|| pp.attr("fo:orphans")) {
        p.widow_control = Some(v.parse::<u32>().map(|n| n > 0).unwrap_or(true));
    }
    p.shading = pp.attr("fo:background-color").and_then(css_color);
    let b = borders_of(pp);
    if b != Borders::default() {
        p.borders = Some(b);
    }
    if let Some(ts) = pp.child("style:tab-stops") {
        for t in ts.children_named("style:tab-stop") {
            let Some(pos) = attr_len(t, "style:position") else { continue };
            let align = match t.attr("style:type") {
                Some("center") => TabAlign::Center,
                Some("right") => TabAlign::Right,
                Some("char") => TabAlign::Decimal,
                _ => TabAlign::Left,
            };
            let leader = match (t.attr("style:leader-text"), t.attr("style:leader-style")) {
                (Some("."), _) | (None, Some("dotted")) => TabLeader::Dot,
                (Some("-"), _) | (None, Some("dash")) => TabLeader::Hyphen,
                (Some("_"), _) | (None, Some("solid")) => TabLeader::Underscore,
                _ => TabLeader::None,
            };
            p.tabs.push(TabStop { pos, align, leader });
        }
    }
    p
}
