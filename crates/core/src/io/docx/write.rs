//! DOCX writer: produces a complete WordprocessingML package.

use super::docx_style_name;
use crate::io::package::ZipBuilder;
use crate::io::util::*;
use crate::io::xml::{esc, esc_attr};
use crate::model::*;
use std::collections::HashMap;
use std::fmt::Write as _;

const NS_W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const CT_WML: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml";

fn root_ns() -> String {
    format!(
        concat!(
            r#"xmlns:wpc="http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas" "#,
            r#"xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" "#,
            r#"xmlns:o="urn:schemas-microsoft-com:office:office" "#,
            r#"xmlns:r="{r}" "#,
            r#"xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" "#,
            r#"xmlns:v="urn:schemas-microsoft-com:vml" "#,
            r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" "#,
            r#"xmlns:w10="urn:schemas-microsoft-com:office:word" "#,
            r#"xmlns:w="{w}" "#,
            r#"xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" "#,
            r#"xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" "#,
            r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
            r#"xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" "#,
            r#"mc:Ignorable="w14 w15""#
        ),
        r = NS_R,
        w = NS_W
    )
}

const XML_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

struct RelEntry {
    id: String,
    kind: String,
    target: String,
    external: bool,
}

#[derive(Default)]
struct PartCtx {
    rels: Vec<RelEntry>,
    /// Comment ranges are tracked in the main document only.
    body: bool,
}

impl PartCtx {
    fn add(&mut self, kind: &str, target: &str, external: bool) -> String {
        if let Some(r) = self.rels.iter().find(|r| r.kind == kind && r.target == target && r.external == external) {
            return r.id.clone();
        }
        let id = format!("rId{}", self.rels.len() + 1);
        self.rels.push(RelEntry { id: id.clone(), kind: kind.to_string(), target: target.to_string(), external });
        id
    }

    fn xml(&self) -> String {
        let mut s = String::from(XML_DECL);
        s.push_str(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
        for r in &self.rels {
            let _ = write!(
                s,
                r#"<Relationship Id="{}" Type="{}" Target="{}"{}/>"#,
                r.id,
                r.kind,
                esc_attr(&r.target),
                if r.external { r#" TargetMode="External""# } else { "" }
            );
        }
        s.push_str("</Relationships>");
        s
    }
}

struct Writer<'a> {
    doc: &'a Document,
    media: HashMap<usize, String>,
    media_files: Vec<(String, Vec<u8>)>,
    docpr_id: u32,
    bookmark_id: u32,
    rev_id: u32,
    footnote_ids: HashMap<u32, u32>,
    endnote_ids: HashMap<u32, u32>,
    list_ids: HashMap<u32, u32>,
    /// comment id → (first run index, last run index) in body traversal order.
    comment_ranges: HashMap<u32, (usize, usize)>,
    orphan_comments: Vec<u32>,
    run_counter: usize,
    para_id: u32,
}

/// Serialise a document as .docx bytes.
pub fn write(doc: &Document) -> Result<Vec<u8>, String> {
    let mut w = Writer {
        doc,
        media: HashMap::new(),
        media_files: Vec::new(),
        docpr_id: 0,
        bookmark_id: 0,
        rev_id: 0,
        footnote_ids: HashMap::new(),
        endnote_ids: HashMap::new(),
        list_ids: HashMap::new(),
        comment_ranges: HashMap::new(),
        orphan_comments: Vec::new(),
        run_counter: 0,
        para_id: 0x1000_0000,
    };
    for (i, n) in doc.footnotes.iter().enumerate() {
        w.footnote_ids.insert(n.id, i as u32 + 1);
    }
    for (i, n) in doc.endnotes.iter().enumerate() {
        w.endnote_ids.insert(n.id, i as u32 + 1);
    }
    let max_list = doc.lists.iter().map(|l| l.id).max().unwrap_or(0);
    let mut next_free = max_list + 1;
    for l in &doc.lists {
        let nid = if l.id == 0 {
            next_free += 1;
            next_free - 1
        } else {
            l.id
        };
        w.list_ids.insert(l.id, nid);
    }
    w.compute_comment_ranges();

    let mut zip = ZipBuilder::new();
    let mut overrides: Vec<(String, String)> = Vec::new();
    let mut doc_rels = PartCtx::default();

    // Headers and footers.
    let mut hf_refs = String::new();
    let watermark = doc.page.watermark.as_deref().filter(|s| !s.trim().is_empty());
    let hf: [(&str, &str, &Vec<Block>, bool); 4] = [
        ("header", "default", &doc.headers.header, true),
        ("header", "first", &doc.headers.first_header, doc.page.title_page),
        ("footer", "default", &doc.headers.footer, true),
        ("footer", "first", &doc.headers.first_footer, doc.page.title_page),
    ];
    let mut counters = HashMap::new();
    for (kind, ty, blocks, allowed) in hf {
        let has_content = !blocks.is_empty() || (kind == "header" && watermark.is_some());
        // With a different first page, always write the first-page parts so the first page is blank.
        if !allowed || (!has_content && ty == "default") {
            continue;
        }
        let n = counters.entry(kind).or_insert(0);
        *n += 1;
        let name = format!("{}{}.xml", kind, n);
        let mut pc = PartCtx::default();
        let mut body = String::new();
        if let (true, Some(wm)) = (kind == "header", watermark) {
            body.push_str(&watermark_para(wm));
        }
        w.blocks(&mut body, blocks, &mut pc);
        if body.is_empty() || matches!(blocks.last(), Some(Block::Table(_))) {
            body.push_str("<w:p/>");
        }
        let tag = if kind == "header" { "w:hdr" } else { "w:ftr" };
        let xml = format!("{}<{} {}>{}</{}>", XML_DECL, tag, root_ns(), body, tag);
        zip.add(&format!("word/{}", name), xml.as_bytes())?;
        if !pc.rels.is_empty() {
            zip.add(&format!("word/_rels/{}.rels", name), pc.xml().as_bytes())?;
        }
        overrides.push((format!("/word/{}", name), format!("{}.{}+xml", CT_WML, kind)));
        let rid = doc_rels.add(&format!("{}/{}", REL, kind), &name, false);
        let _ = write!(hf_refs, r#"<w:{}Reference w:type="{}" r:id="{}"/>"#, kind, ty, rid);
    }
    // Header references must precede footer references.
    let hf_refs = {
        let mut parts: Vec<&str> = hf_refs.split_inclusive("/>").collect();
        parts.sort_by_key(|p| !p.starts_with("<w:headerReference"));
        parts.concat()
    };

    // Main document.
    let mut body = String::new();
    doc_rels.body = true;
    w.blocks(&mut body, &doc.body, &mut doc_rels);
    doc_rels.body = false;
    if doc.body.is_empty() || matches!(doc.body.last(), Some(Block::Table(_))) {
        body.push_str("<w:p/>");
    }
    body.push_str(&sect_pr(doc, &hf_refs));
    let background = doc.page.page_color.map(|c| format!(r#"<w:background w:color="{}"/>"#, c.to_hex())).unwrap_or_default();
    let document_xml = format!("{}<w:document {}>{}<w:body>{}</w:body></w:document>", XML_DECL, root_ns(), background, body);

    // Notes and comments.
    if !doc.footnotes.is_empty() {
        let (xml, rels) = w.notes(&doc.footnotes, true);
        zip.add("word/footnotes.xml", xml.as_bytes())?;
        if !rels.rels.is_empty() {
            zip.add("word/_rels/footnotes.xml.rels", rels.xml().as_bytes())?;
        }
        overrides.push(("/word/footnotes.xml".into(), format!("{}.footnotes+xml", CT_WML)));
        doc_rels.add(&format!("{}/footnotes", REL), "footnotes.xml", false);
    }
    if !doc.endnotes.is_empty() {
        let (xml, rels) = w.notes(&doc.endnotes, false);
        zip.add("word/endnotes.xml", xml.as_bytes())?;
        if !rels.rels.is_empty() {
            zip.add("word/_rels/endnotes.xml.rels", rels.xml().as_bytes())?;
        }
        overrides.push(("/word/endnotes.xml".into(), format!("{}.endnotes+xml", CT_WML)));
        doc_rels.add(&format!("{}/endnotes", REL), "endnotes.xml", false);
    }
    if !doc.comments.is_empty() {
        let (xml, ext, rels) = w.comments();
        zip.add("word/comments.xml", xml.as_bytes())?;
        if !rels.rels.is_empty() {
            zip.add("word/_rels/comments.xml.rels", rels.xml().as_bytes())?;
        }
        zip.add("word/commentsExtended.xml", ext.as_bytes())?;
        overrides.push(("/word/comments.xml".into(), format!("{}.comments+xml", CT_WML)));
        overrides.push(("/word/commentsExtended.xml".into(), format!("{}.commentsExtended+xml", CT_WML)));
        doc_rels.add(&format!("{}/comments", REL), "comments.xml", false);
        doc_rels.add("http://schemas.microsoft.com/office/2011/relationships/commentsExtended", "commentsExtended.xml", false);
    }

    // Styles, numbering, settings, fonts.
    zip.add("word/styles.xml", styles_xml(doc, &w.list_ids).as_bytes())?;
    overrides.push(("/word/styles.xml".into(), format!("{}.styles+xml", CT_WML)));
    doc_rels.add(&format!("{}/styles", REL), "styles.xml", false);
    if !doc.lists.is_empty() {
        zip.add("word/numbering.xml", numbering_xml(doc, &w.list_ids).as_bytes())?;
        overrides.push(("/word/numbering.xml".into(), format!("{}.numbering+xml", CT_WML)));
        doc_rels.add(&format!("{}/numbering", REL), "numbering.xml", false);
    }
    zip.add("word/settings.xml", settings_xml(doc).as_bytes())?;
    overrides.push(("/word/settings.xml".into(), format!("{}.settings+xml", CT_WML)));
    doc_rels.add(&format!("{}/settings", REL), "settings.xml", false);
    zip.add("word/fontTable.xml", font_table_xml(doc).as_bytes())?;
    overrides.push(("/word/fontTable.xml".into(), format!("{}.fontTable+xml", CT_WML)));
    doc_rels.add(&format!("{}/fontTable", REL), "fontTable.xml", false);

    zip.add("word/document.xml", document_xml.as_bytes())?;
    overrides.push(("/word/document.xml".into(), format!("{}.document.main+xml", CT_WML)));
    zip.add("word/_rels/document.xml.rels", doc_rels.xml().as_bytes())?;

    for (name, data) in &w.media_files {
        zip.add(&format!("word/{}", name), data)?;
    }

    // Package-level parts.
    zip.add("docProps/core.xml", core_xml(doc).as_bytes())?;
    overrides.push(("/docProps/core.xml".into(), "application/vnd.openxmlformats-package.core-properties+xml".into()));
    zip.add("docProps/app.xml", app_xml().as_bytes())?;
    overrides.push(("/docProps/app.xml".into(), "application/vnd.openxmlformats-officedocument.extended-properties+xml".into()));
    let mut root_rels = PartCtx::default();
    root_rels.add(&format!("{}/officeDocument", REL), "word/document.xml", false);
    root_rels.add("http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties", "docProps/core.xml", false);
    root_rels.add(&format!("{}/extended-properties", REL), "docProps/app.xml", false);
    zip.add("_rels/.rels", root_rels.xml().as_bytes())?;

    let mut ct = String::from(XML_DECL);
    ct.push_str(r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#);
    ct.push_str(r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#);
    ct.push_str(r#"<Default Extension="xml" ContentType="application/xml"/>"#);
    let mut exts: Vec<String> = w.media_files.iter().filter_map(|(n, _)| n.rsplit('.').next().map(|e| e.to_ascii_lowercase())).collect();
    exts.sort();
    exts.dedup();
    for e in exts {
        if e == "xml" || e == "rels" {
            continue;
        }
        let _ = write!(ct, r#"<Default Extension="{}" ContentType="{}"/>"#, e, image_mime(&e));
    }
    for (part, t) in &overrides {
        let _ = write!(ct, r#"<Override PartName="{}" ContentType="{}"/>"#, part, t);
    }
    ct.push_str("</Types>");
    zip.add("[Content_Types].xml", ct.as_bytes())?;
    zip.finish()
}

fn on_off(out: &mut String, tag: &str, v: Option<bool>) {
    match v {
        Some(true) => {
            let _ = write!(out, "<w:{}/>", tag);
        }
        Some(false) => {
            let _ = write!(out, r#"<w:{} w:val="0"/>"#, tag);
        }
        None => {}
    }
}

fn border_xml(tag: &str, b: &Border) -> String {
    let v = if !b.is_visible() {
        return format!(r#"<w:{} w:val="nil"/>"#, tag);
    } else {
        match b.style {
            BorderStyle::Single | BorderStyle::None => "single",
            BorderStyle::Double => "double",
            BorderStyle::Dotted => "dotted",
            BorderStyle::Dashed => "dashed",
            BorderStyle::Thick => "thick",
        }
    };
    format!(
        r#"<w:{} w:val="{}" w:sz="{}" w:space="{}" w:color="{}"/>"#,
        tag,
        v,
        ((b.width * 8.0).round() as i64).clamp(2, 96),
        (b.space.round() as i64).clamp(0, 31),
        b.color.to_hex()
    )
}

/// `order`: element names for top, left, bottom, right, between, insideV.
fn borders_xml(b: &Borders, wrapper: &str, between_tag: &str, explicit_none: bool) -> String {
    let mut s = String::new();
    let items = [
        ("top", b.top),
        ("left", b.left),
        ("bottom", b.bottom),
        ("right", b.right),
        (between_tag, b.between),
        ("insideV", b.inside_v),
    ];
    for (tag, br) in items {
        if tag.is_empty() || (tag == "insideV" && wrapper == "pBdr") {
            continue;
        }
        match br {
            Some(br) => s.push_str(&border_xml(tag, &br)),
            None if explicit_none => {
                let _ = write!(s, r#"<w:{} w:val="nil"/>"#, tag);
            }
            None => {}
        }
    }
    if s.is_empty() {
        return String::new();
    }
    format!("<w:{}>{}</w:{}>", wrapper, s, wrapper)
}

fn shd(c: Color) -> String {
    format!(r#"<w:shd w:val="clear" w:color="auto" w:fill="{}"/>"#, c.to_hex())
}

pub(crate) fn rpr_inner(c: &CharProps) -> String {
    let mut s = String::new();
    if let Some(st) = &c.style {
        let _ = write!(s, r#"<w:rStyle w:val="{}"/>"#, esc_attr(st));
    }
    if let Some(f) = &c.font {
        let f = esc_attr(f);
        let _ = write!(s, r#"<w:rFonts w:ascii="{f}" w:hAnsi="{f}" w:cs="{f}"/>"#);
    }
    on_off(&mut s, "b", c.bold);
    on_off(&mut s, "i", c.italic);
    on_off(&mut s, "caps", c.caps);
    on_off(&mut s, "smallCaps", c.small_caps);
    on_off(&mut s, "strike", c.strike);
    on_off(&mut s, "dstrike", c.double_strike);
    on_off(&mut s, "vanish", c.hidden);
    if let Some(col) = c.color {
        let _ = write!(s, r#"<w:color w:val="{}"/>"#, col.to_hex());
    }
    if let Some(sp) = c.spacing {
        let _ = write!(s, r#"<w:spacing w:val="{}"/>"#, twips(sp));
    }
    if let Some(sz) = c.size {
        let hp = (sz * 2.0).round().max(1.0) as i64;
        let _ = write!(s, r#"<w:sz w:val="{hp}"/><w:szCs w:val="{hp}"/>"#);
    }
    if let Some(h) = c.highlight {
        let _ = write!(s, r#"<w:highlight w:val="{}"/>"#, highlight_name(h));
    }
    if let Some(u) = c.underline {
        let v = match u {
            Underline::None => "none",
            Underline::Single => "single",
            Underline::Double => "double",
            Underline::Thick => "thick",
            Underline::Dotted => "dotted",
            Underline::Dashed => "dash",
            Underline::Wavy => "wave",
            Underline::Words => "words",
        };
        let _ = write!(s, r#"<w:u w:val="{}"/>"#, v);
    }
    if let Some(sh) = c.shading {
        s.push_str(&shd(sh));
    }
    if let Some(v) = c.vert_align {
        let v = match v {
            VertAlign::Baseline => "baseline",
            VertAlign::Superscript => "superscript",
            VertAlign::Subscript => "subscript",
        };
        let _ = write!(s, r#"<w:vertAlign w:val="{}"/>"#, v);
    }
    s
}

fn rpr(c: &CharProps) -> String {
    let inner = rpr_inner(c);
    if inner.is_empty() {
        String::new()
    } else {
        format!("<w:rPr>{}</w:rPr>", inner)
    }
}

fn ppr_inner(p: &ParaProps, list_ids: &HashMap<u32, u32>) -> String {
    let mut s = String::new();
    if let Some(st) = &p.style {
        let _ = write!(s, r#"<w:pStyle w:val="{}"/>"#, esc_attr(st));
    }
    on_off(&mut s, "keepNext", p.keep_next);
    on_off(&mut s, "keepLines", p.keep_lines);
    on_off(&mut s, "pageBreakBefore", p.page_break_before);
    on_off(&mut s, "widowControl", p.widow_control);
    match p.list {
        Some(Some(l)) => {
            let id = list_ids.get(&l.list).copied().unwrap_or(0);
            let _ = write!(s, r#"<w:numPr><w:ilvl w:val="{}"/><w:numId w:val="{}"/></w:numPr>"#, l.level.min(8), id);
        }
        Some(None) => s.push_str(r#"<w:numPr><w:ilvl w:val="0"/><w:numId w:val="0"/></w:numPr>"#),
        None => {}
    }
    if let Some(b) = &p.borders {
        s.push_str(&borders_xml(b, "pBdr", "between", false));
    }
    if let Some(c) = p.shading {
        s.push_str(&shd(c));
    }
    if !p.tabs.is_empty() {
        s.push_str("<w:tabs>");
        for t in &p.tabs {
            let a = match t.align {
                TabAlign::Left => "left",
                TabAlign::Center => "center",
                TabAlign::Right => "right",
                TabAlign::Decimal => "decimal",
                TabAlign::Clear => "clear",
            };
            let leader = match t.leader {
                TabLeader::None => "",
                TabLeader::Dot => r#" w:leader="dot""#,
                TabLeader::Hyphen => r#" w:leader="hyphen""#,
                TabLeader::Underscore => r#" w:leader="underscore""#,
            };
            let _ = write!(s, r#"<w:tab w:val="{}"{} w:pos="{}"/>"#, a, leader, twips(t.pos));
        }
        s.push_str("</w:tabs>");
    }
    if p.space_before.is_some() || p.space_after.is_some() || p.line_spacing.is_some() {
        s.push_str("<w:spacing");
        if let Some(v) = p.space_before {
            let _ = write!(s, r#" w:before="{}""#, twips(v).max(0));
        }
        if let Some(v) = p.space_after {
            let _ = write!(s, r#" w:after="{}""#, twips(v).max(0));
        }
        match p.line_spacing {
            Some(LineSpacing::Multiple(m)) => {
                let _ = write!(s, r#" w:line="{}" w:lineRule="auto""#, (m * 240.0).round() as i64);
            }
            Some(LineSpacing::AtLeast(v)) => {
                let _ = write!(s, r#" w:line="{}" w:lineRule="atLeast""#, twips(v));
            }
            Some(LineSpacing::Exactly(v)) => {
                let _ = write!(s, r#" w:line="{}" w:lineRule="exact""#, twips(v));
            }
            None => {}
        }
        s.push_str("/>");
    }
    if p.indent_left.is_some() || p.indent_right.is_some() || p.indent_first.is_some() {
        s.push_str("<w:ind");
        if let Some(v) = p.indent_left {
            let _ = write!(s, r#" w:left="{}""#, twips(v));
        }
        if let Some(v) = p.indent_right {
            let _ = write!(s, r#" w:right="{}""#, twips(v));
        }
        if let Some(v) = p.indent_first {
            if v < 0.0 {
                let _ = write!(s, r#" w:hanging="{}""#, twips(-v));
            } else {
                let _ = write!(s, r#" w:firstLine="{}""#, twips(v));
            }
        }
        s.push_str("/>");
    }
    on_off(&mut s, "contextualSpacing", p.contextual_spacing);
    if let Some(a) = p.align {
        let v = match a {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
            Align::Justify => "both",
        };
        let _ = write!(s, r#"<w:jc w:val="{}"/>"#, v);
    }
    if let Some(l) = p.outline_level {
        let _ = write!(s, r#"<w:outlineLvl w:val="{}"/>"#, l.min(9));
    }
    s
}

/// Text with tabs / newlines turned into the proper elements (inside a w:r).
fn text_xml(out: &mut String, s: &str, deleted: bool) {
    let tag = if deleted { "w:delText" } else { "w:t" };
    let mut seg = String::new();
    let flush = |out: &mut String, seg: &mut String| {
        if seg.is_empty() {
            return;
        }
        let preserve = seg.starts_with(' ') || seg.ends_with(' ') || seg.contains("  ") || seg.starts_with('\u{a0}');
        let _ = write!(out, "<{}{}>{}</{}>", tag, if preserve { r#" xml:space="preserve""# } else { "" }, esc(seg), tag);
        seg.clear();
    };
    for c in s.chars() {
        match c {
            '\t' => {
                flush(out, &mut seg);
                out.push_str("<w:tab/>");
            }
            '\n' | '\u{000B}' | '\u{2028}' => {
                flush(out, &mut seg);
                out.push_str("<w:br/>");
            }
            '\u{000C}' => {
                flush(out, &mut seg);
                out.push_str(r#"<w:br w:type="page"/>"#);
            }
            '\r' => {}
            c => seg.push(c),
        }
    }
    flush(out, &mut seg);
}

fn sect_pr(doc: &Document, hf_refs: &str) -> String {
    let p = &doc.page;
    let mut s = String::from("<w:sectPr>");
    s.push_str(hf_refs);
    let orient = if p.width > p.height { r#" w:orient="landscape""# } else { "" };
    let _ = write!(s, r#"<w:pgSz w:w="{}" w:h="{}"{}/>"#, twips(p.width), twips(p.height), orient);
    let _ = write!(
        s,
        r#"<w:pgMar w:top="{}" w:right="{}" w:bottom="{}" w:left="{}" w:header="{}" w:footer="{}" w:gutter="0"/>"#,
        twips(p.margin_top),
        twips(p.margin_right),
        twips(p.margin_bottom),
        twips(p.margin_left),
        twips(p.header_distance),
        twips(p.footer_distance)
    );
    if let Some(b) = &p.page_borders {
        let inner = borders_xml(b, "pgBorders", "", false);
        if !inner.is_empty() {
            s.push_str(&inner.replacen("<w:pgBorders>", r#"<w:pgBorders w:offsetFrom="text">"#, 1));
        }
    }
    if p.page_number_start != 1 {
        let _ = write!(s, r#"<w:pgNumType w:start="{}"/>"#, p.page_number_start);
    }
    let _ = write!(s, r#"<w:cols w:num="{}" w:space="{}"/>"#, p.columns.max(1), twips(p.column_gap));
    if p.title_page {
        s.push_str("<w:titlePg/>");
    }
    s.push_str(r#"<w:docGrid w:linePitch="360"/>"#);
    s.push_str("</w:sectPr>");
    s
}

fn watermark_para(text: &str) -> String {
    let formulas = [
        "sum #0 0 10800",
        "prod #0 2 1",
        "sum 21600 0 @1",
        "sum 0 0 @2",
        "sum 21600 0 @3",
        "if @0 @3 0",
        "if @0 21600 @1",
        "if @0 0 @2",
        "if @0 @4 21600",
        "mid @5 @6",
        "mid @8 @5",
        "mid @7 @8",
        "mid @6 @7",
        "sum @6 0 @5",
    ];
    let f: String = formulas.iter().map(|e| format!(r#"<v:f eqn="{}"/>"#, e)).collect();
    let len = text.chars().count().max(1) as f32;
    let width = (len * 60.0).min(460.0);
    let height = (width / len * 1.4).clamp(40.0, 160.0);
    format!(
        concat!(
            r##"<w:p><w:pPr><w:pStyle w:val="Header"/></w:pPr><w:r><w:rPr><w:noProof/></w:rPr><w:pict>"##,
            r##"<v:shapetype id="_x0000_t136" coordsize="21600,21600" o:spt="136" adj="10800" path="m@7,l@8,m@5,21600l@6,21600e">"##,
            r##"<v:formulas>{f}</v:formulas><v:path textpathok="t" o:connecttype="custom" o:connectlocs="@9,0;@10,10800;@11,21600;@12,10800" o:connectangles="270,180,90,0"/>"##,
            r##"<v:textpath on="t" fitshape="t"/><v:handles><v:h position="#0,bottomRight" xrange="6629,14971"/></v:handles><o:lock v:ext="edit" text="t" shapetype="t"/></v:shapetype>"##,
            r##"<v:shape id="PowerPlusWaterMarkObject1" o:spid="_x0000_s2049" type="#_x0000_t136" style="position:absolute;margin-left:0;margin-top:0;width:{w}pt;height:{h}pt;rotation:315;z-index:-251657216;mso-position-horizontal:center;mso-position-horizontal-relative:margin;mso-position-vertical:center;mso-position-vertical-relative:margin" o:allowincell="f" fillcolor="silver" stroked="f">"##,
            r##"<v:fill opacity=".5"/><v:textpath style="font-family:&quot;Calibri&quot;;font-size:1pt" string="{t}"/><w10:wrap anchorx="margin" anchory="margin"/></v:shape>"##,
            r#"</w:pict></w:r></w:p>"#
        ),
        f = f,
        w = fmt_num(width),
        h = fmt_num(height),
        t = esc_attr(text)
    )
}

impl<'a> Writer<'a> {
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
        let known: Vec<u32> = self.doc.comments.iter().map(|c| c.id).collect();
        ranges.retain(|k, _| known.contains(k));
        self.orphan_comments = known.into_iter().filter(|k| !ranges.contains_key(k)).collect();
        self.comment_ranges = ranges;
    }

    fn blocks(&mut self, out: &mut String, blocks: &[Block], pc: &mut PartCtx) {
        for b in blocks {
            match b {
                Block::Para(p) => self.paragraph(out, p, pc, None),
                Block::Table(t) => self.table(out, t, pc),
            }
        }
    }

    fn table(&mut self, out: &mut String, t: &Table, pc: &mut PartCtx) {
        out.push_str("<w:tbl><w:tblPr>");
        if let Some(st) = &t.style {
            if self.doc.style(st).map(|s| s.kind == StyleKind::Table).unwrap_or(false) {
                let _ = write!(out, r#"<w:tblStyle w:val="{}"/>"#, esc_attr(st));
            }
        }
        out.push_str(r#"<w:tblW w:w="0" w:type="auto"/>"#);
        match t.align {
            Align::Center => out.push_str(r#"<w:jc w:val="center"/>"#),
            Align::Right => out.push_str(r#"<w:jc w:val="right"/>"#),
            _ => {}
        }
        if t.indent != 0.0 {
            let _ = write!(out, r#"<w:tblInd w:w="{}" w:type="dxa"/>"#, twips(t.indent));
        }
        out.push_str(&borders_xml(&t.borders, "tblBorders", "insideH", true));
        out.push_str(r#"<w:tblLayout w:type="fixed"/>"#);
        let m = twips(t.cell_margin);
        let _ = write!(out, r#"<w:tblCellMar><w:left w:w="{m}" w:type="dxa"/><w:right w:w="{m}" w:type="dxa"/></w:tblCellMar>"#);
        out.push_str(r#"<w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="1" w:lastColumn="0" w:noHBand="0" w:noVBand="1"/>"#);
        out.push_str("</w:tblPr><w:tblGrid>");
        let ncols = t.rows.iter().map(|r| r.cells.iter().map(|c| c.span.max(1) as usize).sum::<usize>()).max().unwrap_or(0);
        let mut cols = t.columns.clone();
        if cols.len() < ncols {
            let avg = if cols.is_empty() { 468.0 / ncols as f32 } else { cols.iter().sum::<f32>() / cols.len() as f32 };
            cols.resize(ncols, avg);
        }
        for c in &cols {
            let _ = write!(out, r#"<w:gridCol w:w="{}"/>"#, twips(*c).max(1));
        }
        out.push_str("</w:tblGrid>");
        for (ri, row) in t.rows.iter().enumerate() {
            out.push_str("<w:tr>");
            if row.height.is_some() || row.header {
                out.push_str("<w:trPr>");
                if let Some(h) = row.height {
                    let _ = write!(out, r#"<w:trHeight w:val="{}" w:hRule="atLeast"/>"#, twips(h));
                }
                if row.header {
                    out.push_str("<w:tblHeader/>");
                }
                out.push_str("</w:trPr>");
            }
            let mut gc = 0usize;
            for cell in &row.cells {
                let span = cell.span.max(1) as usize;
                let width: f32 = cols.iter().skip(gc).take(span).sum();
                gc += span;
                out.push_str("<w:tc><w:tcPr>");
                let _ = write!(out, r#"<w:tcW w:w="{}" w:type="dxa"/>"#, twips(if width > 0.0 { width } else { cell.width.unwrap_or(72.0) }));
                if span > 1 {
                    let _ = write!(out, r#"<w:gridSpan w:val="{}"/>"#, span);
                }
                match cell.vmerge {
                    VMerge::Restart => out.push_str(r#"<w:vMerge w:val="restart"/>"#),
                    VMerge::Continue => out.push_str("<w:vMerge/>"),
                    VMerge::None => {}
                }
                if let Some(b) = &cell.borders {
                    out.push_str(&borders_xml(b, "tcBorders", "insideH", false));
                }
                let shading = cell.shading.or(if ri == 0 { t.header_shading } else { None }).or(if ri % 2 == 1 { t.band_shading } else { None });
                if let Some(c) = shading {
                    out.push_str(&shd(c));
                }
                match cell.valign {
                    VAlign::Center => out.push_str(r#"<w:vAlign w:val="center"/>"#),
                    VAlign::Bottom => out.push_str(r#"<w:vAlign w:val="bottom"/>"#),
                    VAlign::Top => {}
                }
                out.push_str("</w:tcPr>");
                if cell.paragraphs.is_empty() {
                    out.push_str("<w:p/>");
                }
                for p in &cell.paragraphs {
                    self.paragraph(out, p, pc, None);
                }
                out.push_str("</w:tc>");
            }
            out.push_str("</w:tr>");
        }
        out.push_str("</w:tbl>");
    }

    /// `note_ref`: Some(true/false) for the first paragraph of a footnote/endnote (adds the reference mark).
    fn paragraph(&mut self, out: &mut String, p: &Paragraph, pc: &mut PartCtx, note_ref: Option<bool>) {
        out.push_str("<w:p>");
        let mut ppr = ppr_inner(&p.props, &self.list_ids);
        let mark = CharProps { link: None, revision: None, comments: Vec::new(), ..p.mark.clone() };
        let mrpr = rpr_inner(&mark);
        if !mrpr.is_empty() {
            let _ = write!(ppr, "<w:rPr>{}</w:rPr>", mrpr);
        }
        if !ppr.is_empty() {
            let _ = write!(out, "<w:pPr>{}</w:pPr>", ppr);
        }
        if let Some(foot) = note_ref {
            let (st, tag) = if foot { ("FootnoteReference", "footnoteRef") } else { ("EndnoteReference", "endnoteRef") };
            let _ = write!(out, r#"<w:r><w:rPr><w:rStyle w:val="{}"/></w:rPr><w:{}/></w:r><w:r><w:t xml:space="preserve"> </w:t></w:r>"#, st, tag);
        }
        if pc.body && !self.orphan_comments.is_empty() {
            for id in std::mem::take(&mut self.orphan_comments) {
                let _ = write!(out, r#"<w:commentRangeStart w:id="{id}"/><w:commentRangeEnd w:id="{id}"/><w:r><w:commentReference w:id="{id}"/></w:r>"#);
            }
        }
        let mut open_link: Option<&str> = None;
        let mut open_rev: Option<&Revision> = None;
        for run in &p.runs {
            let link = run.props.link.as_deref();
            let rev = run.props.revision.as_ref();
            if open_link != link || open_rev != rev {
                if let Some(r) = open_rev.take() {
                    out.push_str(if r.kind == RevisionKind::Insert { "</w:ins>" } else { "</w:del>" });
                }
                if open_link != link {
                    if open_link.is_some() {
                        out.push_str("</w:hyperlink>");
                    }
                    if let Some(l) = link {
                        if let Some(anchor) = l.strip_prefix('#') {
                            let _ = write!(out, r#"<w:hyperlink w:anchor="{}" w:history="1">"#, esc_attr(anchor));
                        } else {
                            let id = pc.add(&format!("{}/hyperlink", REL), l, true);
                            let _ = write!(out, r#"<w:hyperlink r:id="{}" w:history="1">"#, id);
                        }
                    }
                    open_link = link;
                }
                if let Some(r) = rev {
                    self.rev_id += 1;
                    let tag = if r.kind == RevisionKind::Insert { "w:ins" } else { "w:del" };
                    let _ = write!(out, r#"<{} w:id="{}" w:author="{}""#, tag, self.rev_id, esc_attr(if r.author.is_empty() { "Author" } else { &r.author }));
                    if valid_date(&r.date) {
                        let _ = write!(out, r#" w:date="{}""#, esc_attr(&r.date));
                    }
                    out.push('>');
                }
                open_rev = rev;
            }
            let idx = self.run_counter;
            if pc.body {
                self.run_counter += 1;
                let mut starts: Vec<u32> = self.comment_ranges.iter().filter(|(_, r)| r.0 == idx).map(|(k, _)| *k).collect();
                starts.sort_unstable();
                for c in starts {
                    let _ = write!(out, r#"<w:commentRangeStart w:id="{}"/>"#, c);
                }
            }
            self.run(out, run, pc);
            if pc.body {
                let mut ends: Vec<u32> = self.comment_ranges.iter().filter(|(_, r)| r.1 == idx).map(|(k, _)| *k).collect();
                ends.sort_unstable();
                for c in ends {
                    let _ = write!(
                        out,
                        r#"<w:commentRangeEnd w:id="{c}"/><w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:commentReference w:id="{c}"/></w:r>"#
                    );
                }
            }
        }
        if let Some(r) = open_rev {
            out.push_str(if r.kind == RevisionKind::Insert { "</w:ins>" } else { "</w:del>" });
        }
        if open_link.is_some() {
            out.push_str("</w:hyperlink>");
        }
        out.push_str("</w:p>");
    }

    fn run(&mut self, out: &mut String, run: &Run, pc: &mut PartCtx) {
        let deleted = run.props.revision.as_ref().map(|r| r.kind == RevisionKind::Delete).unwrap_or(false);
        let props = CharProps { link: None, revision: None, comments: Vec::new(), ..run.props.clone() };
        let rp = rpr(&props);
        match &run.content {
            Inline::Bookmark(name) => {
                self.bookmark_id += 1;
                let _ = write!(
                    out,
                    r#"<w:bookmarkStart w:id="{id}" w:name="{n}"/><w:bookmarkEnd w:id="{id}"/>"#,
                    id = self.bookmark_id,
                    n = esc_attr(name)
                );
            }
            Inline::Text(t) => {
                let _ = write!(out, "<w:r>{}", rp);
                text_xml(out, t, deleted);
                out.push_str("</w:r>");
            }
            Inline::Tab => {
                let _ = write!(out, "<w:r>{}<w:tab/></w:r>", rp);
            }
            Inline::LineBreak => {
                let _ = write!(out, "<w:r>{}<w:br/></w:r>", rp);
            }
            Inline::PageBreak => {
                let _ = write!(out, r#"<w:r>{}<w:br w:type="page"/></w:r>"#, rp);
            }
            Inline::ColumnBreak => {
                let _ = write!(out, r#"<w:r>{}<w:br w:type="column"/></w:r>"#, rp);
            }
            Inline::Image(img) => {
                let xml = self.image_xml(img, pc);
                let _ = write!(out, "<w:r>{}{}</w:r>", rp, xml);
            }
            Inline::Field(f) => {
                let (instr, result) = match f {
                    Field::PageNumber => (" PAGE ".to_string(), "1".to_string()),
                    Field::PageCount => (" NUMPAGES ".to_string(), "1".to_string()),
                    Field::Date(pic) => (format!(r#" DATE \@ "{}" "#, pic), format_date_now(pic)),
                    Field::Other { instr, result } => (format!(" {} ", instr.trim()), result.clone()),
                };
                let it = if deleted { "w:delInstrText" } else { "w:instrText" };
                let _ = write!(
                    out,
                    r#"<w:r>{rp}<w:fldChar w:fldCharType="begin"/></w:r><w:r>{rp}<{it} xml:space="preserve">{}</{it}></w:r><w:r>{rp}<w:fldChar w:fldCharType="separate"/></w:r>"#,
                    esc(&instr)
                );
                if !result.is_empty() {
                    let _ = write!(out, "<w:r>{}", rp);
                    text_xml(out, &result, deleted);
                    out.push_str("</w:r>");
                }
                let _ = write!(out, r#"<w:r>{rp}<w:fldChar w:fldCharType="end"/></w:r>"#);
            }
            Inline::FootnoteRef(id) | Inline::EndnoteRef(id) => {
                let foot = matches!(run.content, Inline::FootnoteRef(_));
                let map = if foot { &self.footnote_ids } else { &self.endnote_ids };
                let Some(nid) = map.get(id) else { return };
                let mut props = props.clone();
                if props.style.is_none() {
                    props.style = Some(if foot { "FootnoteReference" } else { "EndnoteReference" }.into());
                }
                let tag = if foot { "footnoteReference" } else { "endnoteReference" };
                let _ = write!(out, r#"<w:r>{}<w:{} w:id="{}"/></w:r>"#, rpr(&props), tag, nid);
            }
        }
    }

    fn image_xml(&mut self, img: &Image, pc: &mut PartCtx) -> String {
        let key = std::sync::Arc::as_ptr(&img.data) as usize;
        let name = match self.media.get(&key) {
            Some(n) => n.clone(),
            None => {
                let n = format!("media/image{}.{}", self.media_files.len() + 1, image_ext(&img.data.format));
                self.media_files.push((n.clone(), img.data.bytes.clone()));
                self.media.insert(key, n.clone());
                n
            }
        };
        let rid = pc.add(&format!("{}/image", REL), &name, false);
        self.docpr_id += 1;
        let (cx, cy) = (emu(img.width.max(1.0)), emu(img.height.max(1.0)));
        let id = self.docpr_id;
        format!(
            concat!(
                r#"<w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/>"#,
                r#"<wp:effectExtent l="0" t="0" r="0" b="0"/><wp:docPr id="{id}" name="Picture {id}" descr="{alt}"/>"#,
                r#"<wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="1"/></wp:cNvGraphicFramePr>"#,
                r#"<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">"#,
                r#"<pic:pic><pic:nvPicPr><pic:cNvPr id="{id}" name="{file}"/><pic:cNvPicPr/></pic:nvPicPr>"#,
                r#"<pic:blipFill><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>"#,
                r#"<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr>"#,
                r#"</pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing>"#
            ),
            cx = cx,
            cy = cy,
            id = id,
            alt = esc_attr(&img.alt),
            file = esc_attr(name.rsplit('/').next().unwrap_or("image")),
            rid = rid
        )
    }

    fn notes(&mut self, notes: &[Note], foot: bool) -> (String, PartCtx) {
        let (root, tag, style) = if foot { ("w:footnotes", "w:footnote", "FootnoteText") } else { ("w:endnotes", "w:endnote", "EndnoteText") };
        let mut s = format!("{}<{} {}>", XML_DECL, root, root_ns());
        let sep_p = r#"<w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:r>"#;
        let _ = write!(s, r#"<{tag} w:type="separator" w:id="-1">{sep_p}<w:separator/></w:r></w:p></{tag}>"#);
        let _ = write!(s, r#"<{tag} w:type="continuationSeparator" w:id="0">{sep_p}<w:continuationSeparator/></w:r></w:p></{tag}>"#);
        let mut pc = PartCtx::default();
        for (i, n) in notes.iter().enumerate() {
            let _ = write!(s, r#"<{} w:id="{}">"#, tag, i + 1);
            let mut paras = n.paragraphs.clone();
            if paras.is_empty() {
                paras.push(Paragraph::new());
            }
            for (j, p) in paras.iter().enumerate() {
                let mut p = p.clone();
                if p.props.style.is_none() {
                    p.props.style = Some(style.into());
                }
                self.paragraph(&mut s, &p, &mut pc, if j == 0 { Some(foot) } else { None });
            }
            let _ = write!(s, "</{}>", tag);
        }
        let _ = write!(s, "</{}>", root);
        (s, pc)
    }

    fn comments(&mut self) -> (String, String, PartCtx) {
        let mut s = format!("{}<w:comments {}>", XML_DECL, root_ns());
        let mut ext = format!(
            r#"{}<w15:commentsEx xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" mc:Ignorable="w15">"#,
            XML_DECL
        );
        let mut pc = PartCtx::default();
        for c in &self.doc.comments {
            let _ = write!(s, r#"<w:comment w:id="{}" w:author="{}""#, c.id, esc_attr(if c.author.is_empty() { "Author" } else { &c.author }));
            if valid_date(&c.date) {
                let _ = write!(s, r#" w:date="{}""#, esc_attr(&c.date));
            }
            let _ = write!(s, r#" w:initials="{}">"#, esc_attr(&c.initials));
            let mut paras = c.paragraphs.clone();
            if paras.is_empty() {
                paras.push(Paragraph::new());
            }
            let mut last_pid = 0;
            for (j, p) in paras.iter().enumerate() {
                self.para_id += 1;
                last_pid = self.para_id;
                let mut body = String::new();
                let mut p = p.clone();
                if p.props.style.is_none() {
                    p.props.style = Some("CommentText".into());
                }
                self.paragraph(&mut body, &p, &mut pc, None);
                // Add the paraId and (first paragraph) the annotation reference mark.
                let mut body = body.replacen("<w:p>", &format!(r#"<w:p w14:paraId="{:08X}" w14:textId="77777777">"#, last_pid), 1);
                if j == 0 {
                    let insert_at = body.find("</w:pPr>").map(|i| i + 8).unwrap_or_else(|| body.find('>').unwrap() + 1);
                    body.insert_str(insert_at, r#"<w:r><w:annotationRef/></w:r>"#);
                }
                s.push_str(&body);
            }
            s.push_str("</w:comment>");
            let _ = write!(ext, r#"<w15:commentEx w15:paraId="{:08X}" w15:done="{}"/>"#, last_pid, if c.resolved { 1 } else { 0 });
        }
        s.push_str("</w:comments>");
        ext.push_str("</w15:commentsEx>");
        (s, ext, pc)
    }
}

/// Visit every paragraph (including those in table cells) in document order.
pub(crate) fn for_each_para(blocks: &[Block], f: &mut dyn FnMut(&Paragraph)) {
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

fn valid_date(d: &str) -> bool {
    let b = d.as_bytes();
    b.len() >= 10 && b[..4].iter().all(|c| c.is_ascii_digit()) && b[4] == b'-' && b[7] == b'-'
}

fn styles_xml(doc: &Document, list_ids: &HashMap<u32, u32>) -> String {
    let mut s = format!("{}<w:styles {}>", XML_DECL, root_ns());
    s.push_str("<w:docDefaults><w:rPrDefault><w:rPr>");
    let mut dc = doc.default_chars.clone();
    dc.style = None;
    dc.link = None;
    dc.revision = None;
    dc.comments.clear();
    s.push_str(&rpr_inner(&dc));
    s.push_str(r#"<w:lang w:val="en-US" w:eastAsia="en-US" w:bidi="ar-SA"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr>"#);
    let mut dp = doc.default_para.clone();
    dp.style = None;
    s.push_str(&ppr_inner(&dp, list_ids));
    s.push_str("</w:pPr></w:pPrDefault></w:docDefaults>");
    let has = |id: &str| doc.styles.iter().any(|st| st.id == id);
    for st in &doc.styles {
        let (ty, default) = match st.kind {
            StyleKind::Paragraph => ("paragraph", st.id == "Normal"),
            StyleKind::Character => ("character", st.id == "DefaultParagraphFont"),
            StyleKind::Table => ("table", st.id == "TableNormal"),
        };
        let _ = write!(
            s,
            r#"<w:style w:type="{}"{} w:styleId="{}"><w:name w:val="{}"/>"#,
            ty,
            if default { r#" w:default="1""# } else { "" },
            esc_attr(&st.id),
            esc_attr(&docx_style_name(&st.id, &st.name))
        );
        if let Some(b) = &st.based_on {
            if has(b) && b != &st.id {
                let _ = write!(s, r#"<w:basedOn w:val="{}"/>"#, esc_attr(b));
            }
        }
        if let Some(n) = &st.next {
            if has(n) {
                let _ = write!(s, r#"<w:next w:val="{}"/>"#, esc_attr(n));
            }
        }
        if st.id == "DefaultParagraphFont" || st.id == "TableNormal" {
            s.push_str(r#"<w:uiPriority w:val="1"/><w:semiHidden/><w:unhideWhenUsed/>"#);
        }
        if st.quick {
            s.push_str("<w:qFormat/>");
        }
        if st.kind != StyleKind::Character {
            let p = ppr_inner(&st.para, list_ids);
            if !p.is_empty() {
                let _ = write!(s, "<w:pPr>{}</w:pPr>", p);
            }
        }
        let c = rpr_inner(&CharProps { style: None, link: None, revision: None, comments: vec![], ..st.chars.clone() });
        if !c.is_empty() {
            let _ = write!(s, "<w:rPr>{}</w:rPr>", c);
        }
        if st.kind == StyleKind::Table {
            if st.id == "TableNormal" {
                s.push_str(r#"<w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr>"#);
            } else if st.id == "TableGrid" {
                s.push_str("<w:tblPr>");
                s.push_str(&borders_xml(&Borders::all(Border::single()), "tblBorders", "insideH", false));
                s.push_str("</w:tblPr>");
            }
        }
        s.push_str("</w:style>");
    }
    if !has("DefaultParagraphFont") {
        s.push_str(r#"<w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont"><w:name w:val="Default Paragraph Font"/><w:uiPriority w:val="1"/><w:semiHidden/><w:unhideWhenUsed/></w:style>"#);
    }
    if !has("TableNormal") {
        s.push_str(r#"<w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:uiPriority w:val="99"/><w:semiHidden/><w:unhideWhenUsed/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>"#);
    }
    if !has("CommentReference") && !doc.comments.is_empty() {
        s.push_str(r#"<w:style w:type="character" w:styleId="CommentReference"><w:name w:val="annotation reference"/><w:basedOn w:val="DefaultParagraphFont"/><w:uiPriority w:val="99"/><w:semiHidden/><w:unhideWhenUsed/><w:rPr><w:sz w:val="16"/><w:szCs w:val="16"/></w:rPr></w:style>"#);
    }
    s.push_str("</w:styles>");
    s
}

fn numbering_xml(doc: &Document, list_ids: &HashMap<u32, u32>) -> String {
    let mut s = format!("{}<w:numbering {}>", XML_DECL, root_ns());
    for (i, l) in doc.lists.iter().enumerate() {
        let multi = l.levels.iter().enumerate().any(|(li, lv)| {
            let mut it = lv.text.chars().peekable();
            let mut other = false;
            while let Some(c) = it.next() {
                if c == '%' {
                    if let Some(d) = it.peek().and_then(|d| d.to_digit(10)) {
                        if d as usize != li + 1 {
                            other = true;
                        }
                    }
                }
            }
            other
        });
        let _ = write!(
            s,
            r#"<w:abstractNum w:abstractNumId="{}"><w:nsid w:val="{:08X}"/><w:multiLevelType w:val="{}"/>"#,
            i,
            0x1A2B_0000u32 + i as u32,
            if multi { "multilevel" } else { "hybridMultilevel" }
        );
        for (li, lv) in l.levels.iter().enumerate().take(9) {
            let fmt = match lv.format {
                NumFormat::Decimal => "decimal",
                NumFormat::LowerLetter => "lowerLetter",
                NumFormat::UpperLetter => "upperLetter",
                NumFormat::LowerRoman => "lowerRoman",
                NumFormat::UpperRoman => "upperRoman",
                NumFormat::Bullet => "bullet",
                NumFormat::DecimalZero => "decimalZero",
                NumFormat::None => "none",
            };
            let _ = write!(
                s,
                r#"<w:lvl w:ilvl="{}"><w:start w:val="{}"/><w:numFmt w:val="{}"/><w:lvlText w:val="{}"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="{}" "#,
                li,
                lv.start,
                fmt,
                esc_attr(&lv.text),
                twips(lv.indent)
            );
            if lv.hanging >= 0.0 {
                let _ = write!(s, r#"w:hanging="{}"/>"#, twips(lv.hanging));
            } else {
                let _ = write!(s, r#"w:firstLine="{}"/>"#, twips(-lv.hanging));
            }
            s.push_str("</w:pPr>");
            if let Some(f) = &lv.font {
                let f = esc_attr(f);
                let _ = write!(s, r#"<w:rPr><w:rFonts w:ascii="{f}" w:hAnsi="{f}" w:hint="default"/></w:rPr>"#);
            }
            s.push_str("</w:lvl>");
        }
        s.push_str("</w:abstractNum>");
    }
    for (i, l) in doc.lists.iter().enumerate() {
        let _ = write!(s, r#"<w:num w:numId="{}"><w:abstractNumId w:val="{}"/></w:num>"#, list_ids.get(&l.id).copied().unwrap_or(l.id), i);
    }
    s.push_str("</w:numbering>");
    s
}

fn settings_xml(doc: &Document) -> String {
    let mut s = format!("{}<w:settings {}>", XML_DECL, root_ns());
    s.push_str(r#"<w:zoom w:percent="100"/>"#);
    if doc.page.page_color.is_some() {
        s.push_str("<w:displayBackgroundShape/>");
    }
    if doc.settings.track_changes {
        s.push_str("<w:trackRevisions/>");
    }
    let _ = write!(s, r#"<w:defaultTabStop w:val="{}"/>"#, twips(doc.settings.default_tab).max(1));
    s.push_str(r#"<w:characterSpacingControl w:val="doNotCompress"/>"#);
    if !doc.footnotes.is_empty() {
        s.push_str(r#"<w:footnotePr><w:footnote w:id="-1"/><w:footnote w:id="0"/></w:footnotePr>"#);
    }
    if !doc.endnotes.is_empty() {
        s.push_str(r#"<w:endnotePr><w:endnote w:id="-1"/><w:endnote w:id="0"/></w:endnotePr>"#);
    }
    s.push_str(r#"<w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat>"#);
    s.push_str("</w:settings>");
    s
}

fn font_table_xml(doc: &Document) -> String {
    let mut fonts: Vec<String> = Vec::new();
    let mut add = |f: &Option<String>| {
        if let Some(f) = f {
            if !fonts.contains(f) {
                fonts.push(f.clone());
            }
        }
    };
    add(&doc.default_chars.font);
    for st in &doc.styles {
        add(&st.chars.font);
    }
    for_each_para(&doc.body, &mut |p| p.runs.iter().for_each(|r| add(&r.props.font)));
    let mut s = format!("{}<w:fonts {}>", XML_DECL, root_ns());
    for f in fonts {
        let _ = write!(s, r#"<w:font w:name="{}"><w:family w:val="auto"/><w:pitch w:val="variable"/></w:font>"#, esc_attr(&f));
    }
    s.push_str("</w:fonts>");
    s
}

fn core_xml(doc: &Document) -> String {
    let p = &doc.properties;
    let now = now_iso8601();
    let created = if valid_date(&p.created) { p.created.clone() } else { now.clone() };
    let modified = if valid_date(&p.modified) { p.modified.clone() } else { now };
    let mut s = String::from(XML_DECL);
    s.push_str(r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#);
    let tags = [
        ("dc:title", &p.title),
        ("dc:subject", &p.subject),
        ("dc:creator", &p.author),
        ("cp:keywords", &p.keywords),
        ("dc:description", &p.description),
        ("cp:lastModifiedBy", &p.last_modified_by),
    ];
    for (t, v) in tags {
        if !v.is_empty() {
            let _ = write!(s, "<{t}>{}</{t}>", esc(v));
        }
    }
    let _ = write!(s, r#"<dcterms:created xsi:type="dcterms:W3CDTF">{}</dcterms:created>"#, esc(&created));
    let _ = write!(s, r#"<dcterms:modified xsi:type="dcterms:W3CDTF">{}</dcterms:modified>"#, esc(&modified));
    s.push_str("</cp:coreProperties>");
    s
}

fn app_xml() -> String {
    format!(
        r#"{}<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><Template>Normal.dotm</Template><Application>RWord</Application><DocSecurity>0</DocSecurity><AppVersion>16.0000</AppVersion></Properties>"#,
        XML_DECL
    )
}
