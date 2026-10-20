//! DOCX reader.

use super::builtin_id_for_name;
use crate::io::package::Package;
use crate::io::util::*;
use crate::io::xml::{self, Element};
use crate::model::*;
use std::collections::HashMap;
use std::sync::Arc;

const REL_OFFICE_DOC: &str = "/officeDocument";

#[derive(Clone, Debug)]
struct Rel {
    kind: String,
    target: String,
    external: bool,
}

#[derive(Default, Clone)]
struct Rels {
    map: HashMap<String, Rel>,
}

impl Rels {
    fn by_type(&self, suffix: &str) -> Option<&Rel> {
        let mut v: Vec<(&String, &Rel)> = self.map.iter().filter(|(_, r)| r.kind.ends_with(suffix)).collect();
        v.sort_by(|a, b| a.0.cmp(b.0));
        v.first().map(|(_, r)| *r)
    }
}

/// Resolve a relationship target relative to the directory of `base_part`.
fn resolve_target(base_part: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = base_part.split('/').collect();
    parts.pop();
    for seg in target.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn rels_path(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{}/_rels/{}.rels", dir, file),
        None => format!("_rels/{}.rels", part),
    }
}

fn load_rels(pkg: &Package, part: &str) -> Rels {
    let mut rels = Rels::default();
    let Some(bytes) = pkg.get(&rels_path(part)) else { return rels };
    let Ok(root) = xml::parse_bytes(bytes) else { return rels };
    for r in root.elems().filter(|e| e.local() == "Relationship") {
        let (Some(id), Some(target)) = (r.attr("Id"), r.attr("Target")) else { continue };
        let external = r.attr("TargetMode").map(|m| m.eq_ignore_ascii_case("External")).unwrap_or(false);
        let target = if external { target.to_string() } else { resolve_target(part, target) };
        rels.map.insert(id.to_string(), Rel { kind: r.attr("Type").unwrap_or("").to_string(), target, external });
    }
    rels
}

/// "on/off" property value: absent `w:val` means on.
fn on(e: &Element) -> bool {
    match e.attr("w:val") {
        None => true,
        Some(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "none"),
    }
}

fn num_attr(e: &Element, a: &str) -> Option<f32> {
    e.attr(a).and_then(|v| v.trim().parse::<f32>().ok())
}

/// Twips attribute → points.
fn tw(e: &Element, a: &str) -> Option<f32> {
    num_attr(e, a).map(|v| v / 20.0)
}

fn val(e: &Element) -> Option<&str> {
    e.attr("w:val")
}

fn hex_color(v: &str) -> Option<Color> {
    if v.eq_ignore_ascii_case("auto") {
        return None;
    }
    Color::from_hex(v)
}

fn parse_border(e: &Element) -> Option<Border> {
    let v = val(e).unwrap_or("single");
    let style = match v {
        "nil" | "none" => return Some(Border { style: BorderStyle::None, width: 0.0, color: Color::BLACK, space: 0.0 }),
        "double" | "triple" | "thinThickSmallGap" | "thickThinSmallGap" | "thinThickMediumGap" | "thickThinMediumGap"
        | "thinThickLargeGap" | "thickThinLargeGap" => BorderStyle::Double,
        "dotted" | "dotDash" | "dotDotDash" => BorderStyle::Dotted,
        "dashed" | "dashSmallGap" | "dashDotStroked" => BorderStyle::Dashed,
        "thick" => BorderStyle::Thick,
        _ => BorderStyle::Single,
    };
    let width = num_attr(e, "w:sz").map(|s| s / 8.0).unwrap_or(0.5);
    let color = e.attr("w:color").and_then(hex_color).unwrap_or(Color::BLACK);
    let space = num_attr(e, "w:space").unwrap_or(0.0);
    Some(Border { style, width, color, space })
}

fn parse_borders(e: &Element) -> Borders {
    let mut b = Borders::default();
    for c in e.elems() {
        let br = parse_border(c);
        match c.local() {
            "top" => b.top = br,
            "bottom" => b.bottom = br,
            "left" | "start" => b.left = br,
            "right" | "end" => b.right = br,
            "between" | "insideH" => b.between = br,
            "insideV" => b.inside_v = br,
            _ => {}
        }
    }
    b
}

fn shading_fill(e: &Element) -> Option<Color> {
    let fill = e.attr("w:fill").and_then(hex_color);
    if fill.is_some() {
        return fill;
    }
    // A solid pattern uses the pattern colour.
    if val(e) == Some("solid") {
        return e.attr("w:color").and_then(hex_color);
    }
    None
}

#[derive(PartialEq, Clone, Copy)]
enum FieldMode {
    Swallow,
    Pass,
}

struct FieldState {
    instr: String,
    separated: bool,
    mode: FieldMode,
    result: String,
    props: Option<CharProps>,
    begin_props: CharProps,
    link: Option<String>,
}

#[derive(Clone, Default)]
struct RunCtx {
    link: Option<String>,
    revision: Option<Revision>,
}

#[derive(Clone, Copy, PartialEq)]
enum Part {
    Body,
    Header,
    Note,
    Comment,
}

struct Reader<'a> {
    pkg: &'a Package,
    doc: Document,
    part: String,
    rels: Rels,
    style_alias: HashMap<String, String>,
    table_style_borders: HashMap<String, Borders>,
    table_style_based: HashMap<String, String>,
    major_font: String,
    minor_font: String,
    comments_active: Vec<u32>,
    fields: Vec<FieldState>,
    images: HashMap<String, Arc<ImageData>>,
    /// `w:pPr/w:sectPr` elements of section-ending body paragraphs, in document order.
    para_sects: Vec<Element>,
    /// The body-level `w:sectPr` (the last section).
    body_sect: Option<Element>,
    /// Nesting depth below the body (tables, text boxes): section breaks only count at depth 0.
    nest: u32,
    strip_space: bool,
    saw_watermark: bool,
    /// Watermark text found while reading a header part.
    found_watermark: Option<String>,
    /// Parsed header/footer parts by target: (blocks, watermark).
    hf_cache: HashMap<String, (Vec<Block>, Option<String>)>,
    /// Theme colour scheme (dk1, lt1, accent1, …) → colour.
    theme_colors: HashMap<String, Color>,
    kind: Part,
}

/// Read a .docx package.
pub fn read(bytes: &[u8]) -> Result<Document, String> {
    let pkg = Package::open(bytes)?;
    let root_rels = load_rels(&pkg, "");
    let main = root_rels
        .map
        .values()
        .find(|r| r.kind.ends_with(REL_OFFICE_DOC) && !r.external)
        .map(|r| r.target.clone())
        .filter(|t| pkg.has(t))
        .unwrap_or_else(|| "word/document.xml".to_string());
    let main_bytes = pkg.get(&main).ok_or("The file is not a Word document (word/document.xml is missing)")?;
    let root = xml::parse_bytes(main_bytes)?;

    let mut doc = Document::new();
    doc.body.clear();
    // Defaults when styles.xml has no docDefaults (ECMA-376 / Word behaviour).
    doc.default_chars.font = Some("Times New Roman".into());
    doc.default_chars.size = Some(10.0);
    doc.default_para.space_after = Some(0.0);
    doc.default_para.line_spacing = Some(LineSpacing::Multiple(1.0));

    let mut r = Reader {
        pkg: &pkg,
        doc,
        part: main.clone(),
        rels: load_rels(&pkg, &main),
        style_alias: HashMap::new(),
        table_style_borders: HashMap::new(),
        table_style_based: HashMap::new(),
        major_font: "Calibri Light".into(),
        minor_font: "Calibri".into(),
        comments_active: Vec::new(),
        fields: Vec::new(),
        images: HashMap::new(),
        para_sects: Vec::new(),
        body_sect: None,
        nest: 0,
        strip_space: false,
        saw_watermark: false,
        found_watermark: None,
        hf_cache: HashMap::new(),
        theme_colors: HashMap::new(),
        kind: Part::Body,
    };

    r.read_theme();
    r.read_styles();
    r.read_numbering();
    r.read_settings();
    r.read_core(&root_rels);
    r.read_footnotes(true);
    r.read_footnotes(false);
    r.read_comments();

    // Body.
    if let Some(bg) = root.child("w:background").and_then(|b| b.attr("w:color")).and_then(hex_color) {
        r.doc.page.page_color = Some(bg);
    }
    if let Some(body) = root.child("w:body") {
        let mut blocks = Vec::new();
        r.kind = Part::Body;
        r.blocks(body, &mut blocks);
        r.doc.body = blocks;
    }
    if r.doc.body.is_empty() {
        r.doc.body.push(Block::Para(Paragraph::new()));
    }
    r.apply_sections();
    Ok(r.doc)
}

impl<'a> Reader<'a> {
    fn part_xml(&self, rel_suffix: &str) -> Option<(String, Element)> {
        let main_rels = load_rels(self.pkg, &self.main_part());
        let rel = main_rels.by_type(rel_suffix)?;
        let bytes = self.pkg.get(&rel.target)?;
        Some((rel.target.clone(), xml::parse_bytes(bytes).ok()?))
    }

    fn main_part(&self) -> String {
        let root_rels = load_rels(self.pkg, "");
        root_rels
            .map
            .values()
            .find(|r| r.kind.ends_with(REL_OFFICE_DOC))
            .map(|r| r.target.clone())
            .filter(|t| self.pkg.has(t))
            .unwrap_or_else(|| "word/document.xml".into())
    }

    fn read_theme(&mut self) {
        for (k, v) in [
            ("dk1", "000000"),
            ("lt1", "FFFFFF"),
            ("dk2", "44546A"),
            ("lt2", "E7E6E6"),
            ("accent1", "4472C4"),
            ("accent2", "ED7D31"),
            ("accent3", "A5A5A5"),
            ("accent4", "FFC000"),
            ("accent5", "5B9BD5"),
            ("accent6", "70AD47"),
            ("hlink", "0563C1"),
            ("folHlink", "954F72"),
        ] {
            self.theme_colors.insert(k.to_string(), Color::from_hex(v).unwrap_or_default());
        }
        let Some((_, root)) = self.part_xml("/theme") else { return };
        if let Some(cs) = root.find("a:clrScheme") {
            for c in cs.elems() {
                if let Some(col) = c.elems().find_map(|x| x.attr("val").filter(|_| x.local() == "srgbClr").or_else(|| x.attr("lastClr"))).and_then(Color::from_hex) {
                    self.theme_colors.insert(c.local().to_string(), col);
                }
            }
        }
        if let Some(f) = root.find("a:majorFont").and_then(|m| m.child("a:latin")).and_then(|l| l.attr("typeface")) {
            if !f.is_empty() {
                self.major_font = f.to_string();
            }
        }
        if let Some(f) = root.find("a:minorFont").and_then(|m| m.child("a:latin")).and_then(|l| l.attr("typeface")) {
            if !f.is_empty() {
                self.minor_font = f.to_string();
            }
        }
    }

    fn style_id(&self, id: &str) -> String {
        self.style_alias.get(id).cloned().unwrap_or_else(|| id.to_string())
    }

    // ----- styles -----

    fn read_styles(&mut self) {
        let Some((_, root)) = self.part_xml("/styles") else { return };
        // Pass 1: alias map for localised / differently-named built-in styles.
        for s in root.children_named("w:style") {
            let (Some(id), Some(name)) = (s.attr("w:styleId"), s.child_attr("w:name", "w:val")) else { continue };
            if let Some(bid) = builtin_id_for_name(name) {
                if bid != id {
                    self.style_alias.insert(id.to_string(), bid.to_string());
                }
            }
        }
        if let Some(dd) = root.child("w:docDefaults") {
            if let Some(rpr) = dd.child("w:rPrDefault").and_then(|x| x.child("w:rPr")) {
                let c = self.char_props(rpr);
                self.doc.default_chars.merge(&c);
            }
            if let Some(ppr) = dd.child("w:pPrDefault").and_then(|x| x.child("w:pPr")) {
                let (p, _) = self.para_props(ppr);
                self.doc.default_para.merge(&p);
            }
        }
        for s in root.children_named("w:style") {
            let Some(raw_id) = s.attr("w:styleId") else { continue };
            let kind = match s.attr("w:type").unwrap_or("paragraph") {
                "paragraph" => StyleKind::Paragraph,
                "character" => StyleKind::Character,
                "table" => StyleKind::Table,
                _ => continue,
            };
            let id = self.style_id(raw_id);
            let raw_name = s.child_attr("w:name", "w:val").unwrap_or(raw_id).to_string();
            let existing = self.doc.styles.iter().position(|x| x.id == id);
            let name = match existing {
                Some(i) if builtin_id_for_name(&raw_name).is_some() => self.doc.styles[i].name.clone(),
                _ => {
                    let mut c = raw_name.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        None => id.clone(),
                    }
                }
            };
            let based_on = s.child_attr("w:basedOn", "w:val").map(|b| self.style_id(b));
            let next = s.child_attr("w:next", "w:val").map(|b| self.style_id(b));
            let (para, _) = s.child("w:pPr").map(|p| self.para_props(p)).unwrap_or_default();
            let chars = s.child("w:rPr").map(|r| self.char_props(r)).unwrap_or_default();
            if kind == StyleKind::Table {
                if let Some(b) = s.child("w:tblPr").and_then(|t| t.child("w:tblBorders")) {
                    self.table_style_borders.insert(id.clone(), parse_borders(b));
                }
                if let Some(b) = &based_on {
                    self.table_style_based.insert(id.clone(), b.clone());
                }
            }
            let quick = s.child("w:qFormat").map(on).unwrap_or(false);
            let style = Style { id: id.clone(), name, kind, based_on, next, para, chars, quick };
            match existing {
                Some(i) => self.doc.styles[i] = style,
                None => self.doc.styles.push(style),
            }
        }
    }

    fn table_borders_from_style(&self, id: &str) -> Option<Borders> {
        let mut cur = Some(id.to_string());
        let mut n = 0;
        while let Some(c) = cur {
            if let Some(b) = self.table_style_borders.get(&c) {
                return Some(b.clone());
            }
            if c == "TableGrid" {
                return Some(Borders::all(Border::single()));
            }
            cur = self.table_style_based.get(&c).cloned();
            n += 1;
            if n > 16 {
                break;
            }
        }
        None
    }

    // ----- numbering -----

    fn read_numbering(&mut self) {
        let Some((_, root)) = self.part_xml("/numbering") else { return };
        let mut abstracts: HashMap<String, Vec<ListLevel>> = HashMap::new();
        for an in root.children_named("w:abstractNum") {
            let Some(aid) = an.attr("w:abstractNumId") else { continue };
            let mut levels = ListDef::numbered(0).levels;
            for lvl in an.children_named("w:lvl") {
                let i = lvl.attr("w:ilvl").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
                if i < 9 {
                    levels[i] = self.list_level(lvl, &levels[i]);
                }
            }
            abstracts.insert(aid.to_string(), levels);
        }
        for num in root.children_named("w:num") {
            let Some(id) = num.attr("w:numId").and_then(|v| v.parse::<u32>().ok()) else { continue };
            let Some(aid) = num.child_attr("w:abstractNumId", "w:val") else { continue };
            let Some(mut levels) = abstracts.get(aid).cloned() else { continue };
            for ov in num.children_named("w:lvlOverride") {
                let i = ov.attr("w:ilvl").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
                if i >= 9 {
                    continue;
                }
                if let Some(lvl) = ov.child("w:lvl") {
                    levels[i] = self.list_level(lvl, &levels[i]);
                }
                if let Some(s) = ov.child("w:startOverride").and_then(|s| num_attr(s, "w:val")) {
                    levels[i].start = s as u32;
                }
            }
            self.doc.lists.retain(|l| l.id != id);
            self.doc.lists.push(ListDef { id, levels });
        }
        self.doc.lists.sort_by_key(|l| l.id);
    }

    fn list_level(&self, lvl: &Element, base: &ListLevel) -> ListLevel {
        let mut l = base.clone();
        if let Some(s) = lvl.child("w:start").and_then(|s| num_attr(s, "w:val")) {
            l.start = s.max(0.0) as u32;
        }
        let mut fmt_el = lvl.child("w:numFmt");
        if fmt_el.is_none() {
            if let Some(ac) = lvl.child("mc:AlternateContent") {
                fmt_el = ac.child("mc:Fallback").and_then(|f| f.child("w:numFmt"));
            }
        }
        if let Some(f) = fmt_el.and_then(val) {
            l.format = match f {
                "decimal" => NumFormat::Decimal,
                "lowerLetter" => NumFormat::LowerLetter,
                "upperLetter" => NumFormat::UpperLetter,
                "lowerRoman" => NumFormat::LowerRoman,
                "upperRoman" => NumFormat::UpperRoman,
                "bullet" => NumFormat::Bullet,
                "decimalZero" => NumFormat::DecimalZero,
                "none" => NumFormat::None,
                _ => NumFormat::Decimal,
            };
        }
        if let Some(t) = lvl.child("w:lvlText") {
            l.text = val(t).unwrap_or("").to_string();
        }
        if let Some(ind) = lvl.child("w:pPr").and_then(|p| p.child("w:ind")) {
            if let Some(v) = tw(ind, "w:left").or_else(|| tw(ind, "w:start")) {
                l.indent = v;
            }
            if let Some(h) = tw(ind, "w:hanging") {
                l.hanging = h;
            } else if let Some(f) = tw(ind, "w:firstLine") {
                l.hanging = -f;
            }
        }
        l.font = lvl.child("w:rPr").and_then(|r| r.child("w:rFonts")).and_then(|f| self.font_of(f));
        if l.format == NumFormat::Bullet {
            if let Some(m) = map_symbol_bullet(&l.text, l.font.as_deref()) {
                l.text = m;
                l.font = None;
            } else if l.font.as_deref().map(|f| f.eq_ignore_ascii_case("symbol")).unwrap_or(false) {
                // Symbol-font PUA chars: shift down to the ASCII-compatible range.
                l.text = l.text.chars().map(|c| if (0xF020..0xF100).contains(&(c as u32)) { char::from_u32(c as u32 - 0xF000).unwrap_or(c) } else { c }).collect();
            }
        }
        l
    }

    fn read_settings(&mut self) {
        let Some((_, root)) = self.part_xml("/settings") else { return };
        if let Some(t) = root.child("w:trackRevisions") {
            self.doc.settings.track_changes = on(t);
        }
        if let Some(t) = root.child("w:defaultTabStop").and_then(|t| tw(t, "w:val")) {
            if t > 0.0 {
                self.doc.settings.default_tab = t;
            }
        }
    }

    fn read_core(&mut self, root_rels: &Rels) {
        let path = root_rels
            .map
            .values()
            .find(|r| r.kind.ends_with("/metadata/core-properties"))
            .map(|r| r.target.clone())
            .unwrap_or_else(|| "docProps/core.xml".into());
        let Some(bytes) = self.pkg.get(&path) else { return };
        let Ok(root) = xml::parse_bytes(bytes) else { return };
        let p = &mut self.doc.properties;
        for e in root.elems() {
            let t = e.text().trim().to_string();
            match e.local() {
                "title" => p.title = t,
                "subject" => p.subject = t,
                "creator" => p.author = t,
                "keywords" => p.keywords = t,
                "description" => p.description = t,
                "lastModifiedBy" => p.last_modified_by = t,
                "created" => p.created = t,
                "modified" => p.modified = t,
                _ => {}
            }
        }
    }

    fn with_part<T>(&mut self, part: &str, kind: Part, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved_part = std::mem::replace(&mut self.part, part.to_string());
        let saved_rels = std::mem::replace(&mut self.rels, load_rels(self.pkg, part));
        let saved_kind = std::mem::replace(&mut self.kind, kind);
        let saved_fields = std::mem::take(&mut self.fields);
        let saved_comments = std::mem::take(&mut self.comments_active);
        let r = f(self);
        self.part = saved_part;
        self.rels = saved_rels;
        self.kind = saved_kind;
        self.fields = saved_fields;
        self.comments_active = saved_comments;
        r
    }

    fn read_footnotes(&mut self, foot: bool) {
        let (suffix, tag) = if foot { ("/footnotes", "w:footnote") } else { ("/endnotes", "w:endnote") };
        let Some((path, root)) = self.part_xml(suffix) else { return };
        let notes = self.with_part(&path, Part::Note, |r| {
            let mut notes = Vec::new();
            for n in root.children_named(tag) {
                if matches!(n.attr("w:type"), Some("separator" | "continuationSeparator" | "continuationNotice")) {
                    continue;
                }
                let Some(id) = n.attr("w:id").and_then(|v| v.parse::<i64>().ok()) else { continue };
                if id < 0 {
                    continue;
                }
                let mut blocks = Vec::new();
                r.blocks(n, &mut blocks);
                notes.push(Note { id: id as u32, paragraphs: flatten_blocks(blocks) });
            }
            notes
        });
        if foot {
            self.doc.footnotes = notes;
        } else {
            self.doc.endnotes = notes;
        }
    }

    fn read_comments(&mut self) {
        let Some((path, root)) = self.part_xml("/comments") else { return };
        // Resolved state lives in commentsExtended.xml, keyed by the last paragraph's w14:paraId.
        let mut done: HashMap<String, bool> = HashMap::new();
        if let Some((_, ext)) = self.part_xml("/commentsExtended") {
            for c in ext.elems() {
                if let Some(pid) = c.attr("w15:paraId") {
                    done.insert(pid.to_string(), c.attr("w15:done").map(|d| d == "1" || d == "true").unwrap_or(false));
                }
            }
        }
        let comments = self.with_part(&path, Part::Comment, |r| {
            let mut out = Vec::new();
            for c in root.children_named("w:comment") {
                let Some(id) = c.attr("w:id").and_then(|v| v.parse::<u32>().ok()) else { continue };
                let mut blocks = Vec::new();
                r.blocks(c, &mut blocks);
                let last_pid = c.children_named("w:p").last().and_then(|p| p.attr("w14:paraId")).map(|s| s.to_string());
                let resolved = last_pid.and_then(|p| done.get(&p).copied()).unwrap_or(false);
                out.push(Comment {
                    id,
                    author: c.attr("w:author").unwrap_or("").to_string(),
                    initials: c.attr("w:initials").unwrap_or("").to_string(),
                    date: c.attr("w:date").unwrap_or("").to_string(),
                    paragraphs: flatten_blocks(blocks),
                    resolved,
                });
            }
            out
        });
        self.doc.comments = comments;
    }

    fn apply_sections(&mut self) {
        let para_sects = std::mem::take(&mut self.para_sects);
        let body_sect = self.body_sect.take();
        let bg = self.doc.page.page_color;
        let all: Vec<Option<&Element>> = para_sects.iter().map(Some).chain(std::iter::once(body_sect.as_ref())).collect();
        const KINDS: [(&str, &str, HfKind); 4] = [
            ("w:headerReference", "default", HfKind::Header),
            ("w:footerReference", "default", HfKind::Footer),
            ("w:headerReference", "first", HfKind::FirstHeader),
            ("w:footerReference", "first", HfKind::FirstFooter),
        ];
        // Word semantics: a section without a reference of some kind inherits that kind from the previous section.
        let mut eff: [Option<String>; 4] = Default::default();
        let mut sections: Vec<(PageSetup, HeadersFooters, bool)> = Vec::new();
        for s in &all {
            let mut page = s.map(|s| self.parse_page(s)).unwrap_or_default();
            page.page_color = bg;
            let mut has_own = false;
            if let Some(s) = s {
                for (k, (tag, ty, _)) in KINDS.iter().enumerate() {
                    for r in s.children_named(tag) {
                        if r.attr("w:type").unwrap_or("default") != *ty {
                            continue;
                        }
                        if let Some(rel) = r.attr("r:id").and_then(|id| self.rels.map.get(id)) {
                            eff[k] = Some(rel.target.clone());
                            has_own = true;
                        }
                    }
                }
            }
            let mut hf = HeadersFooters::default();
            let mut wm = None;
            for (k, (_, _, kind)) in KINDS.iter().enumerate() {
                if let Some(t) = eff[k].clone() {
                    let (blocks, w) = self.header_part(&t);
                    *hf.part_mut(*kind) = blocks;
                    if matches!(kind, HfKind::Header | HfKind::FirstHeader) && wm.is_none() {
                        wm = w;
                    }
                }
            }
            if wm.is_some() {
                page.watermark = wm;
            }
            sections.push((page, hf, has_own));
        }
        let (last_page, last_hf, _) = sections.pop().unwrap_or_default();
        // Earlier sections: keep their own headers when they had references or when the model's
        // "linked to previous" resolution would display something else.
        let mut decided: Vec<Option<HeadersFooters>> = Vec::new();
        let mut out: Vec<Section> = Vec::new();
        for (page, hf, has_own) in sections {
            let resolved = decided.iter().rev().find_map(|h| h.as_ref()).unwrap_or(&last_hf);
            let headers = if has_own || *resolved != hf { Some(hf) } else { None };
            decided.push(headers.clone());
            out.push(Section { page, headers });
        }
        let mut it = out.into_iter();
        for b in self.doc.body.iter_mut() {
            if let Block::Para(p) = b {
                if p.section.is_some() {
                    match it.next() {
                        Some(s) => p.section = Some(Box::new(s)),
                        None => p.section = None,
                    }
                }
            }
        }
        self.doc.page = last_page;
        self.doc.headers = last_hf;
    }

    /// Read (once) a header/footer part: its blocks and any watermark it carries.
    fn header_part(&mut self, target: &str) -> (Vec<Block>, Option<String>) {
        if let Some(c) = self.hf_cache.get(target) {
            return c.clone();
        }
        let res = match self.pkg.get(target).and_then(|b| xml::parse_bytes(b).ok()) {
            Some(root) => {
                self.found_watermark = None;
                let blocks = self.with_part(target, Part::Header, |r| {
                    let mut b = Vec::new();
                    r.blocks(&root, &mut b);
                    b
                });
                (blocks, self.found_watermark.take())
            }
            None => (Vec::new(), None),
        };
        self.hf_cache.insert(target.to_string(), res.clone());
        res
    }

    /// Page setup of one `w:sectPr`.
    fn parse_page(&self, sect: &Element) -> PageSetup {
        let mut page = PageSetup::default();
        if let Some(sz) = sect.child("w:pgSz") {
            if let (Some(w), Some(h)) = (tw(sz, "w:w"), tw(sz, "w:h")) {
                page.width = w;
                page.height = h;
                let landscape = sz.attr("w:orient") == Some("landscape");
                if landscape && page.width < page.height || !landscape && sz.attr("w:orient").is_some() && page.width > page.height {
                    std::mem::swap(&mut page.width, &mut page.height);
                }
            }
        }
        if let Some(m) = sect.child("w:pgMar") {
            if let Some(v) = tw(m, "w:top") {
                page.margin_top = v.abs();
            }
            if let Some(v) = tw(m, "w:bottom") {
                page.margin_bottom = v.abs();
            }
            let gutter = tw(m, "w:gutter").unwrap_or(0.0);
            if let Some(v) = tw(m, "w:left").or_else(|| tw(m, "w:start")) {
                page.margin_left = v + gutter;
            }
            if let Some(v) = tw(m, "w:right").or_else(|| tw(m, "w:end")) {
                page.margin_right = v;
            }
            if let Some(v) = tw(m, "w:header") {
                page.header_distance = v;
            }
            if let Some(v) = tw(m, "w:footer") {
                page.footer_distance = v;
            }
        }
        if let Some(c) = sect.child("w:cols") {
            page.columns = num_attr(c, "w:num").map(|n| n.max(1.0) as u32).unwrap_or(1);
            if let Some(s) = tw(c, "w:space") {
                page.column_gap = s;
            }
        }
        page.title_page = sect.child("w:titlePg").map(on).unwrap_or(false);
        if let Some(s) = sect.child("w:pgNumType").and_then(|p| num_attr(p, "w:start")) {
            page.page_number_start = s.max(0.0) as u32;
        }
        if let Some(b) = sect.child("w:pgBorders") {
            let borders = parse_borders(b);
            if !borders.is_empty() {
                page.page_borders = Some(borders);
            }
        }
        page.start = match sect.child_attr("w:type", "w:val") {
            Some("continuous") => SectionStart::Continuous,
            Some("evenPage") => SectionStart::EvenPage,
            Some("oddPage") => SectionStart::OddPage,
            _ => SectionStart::NextPage,
        };
        page
    }

    // ----- block content -----

    fn blocks(&mut self, el: &Element, out: &mut Vec<Block>) {
        for c in el.elems() {
            match c.name.as_str() {
                "w:p" => {
                    self.saw_watermark = false;
                    let p = self.paragraph(c);
                    // A paragraph that only carried the watermark shape is dropped (the writer recreates it).
                    if self.saw_watermark && p.runs.is_empty() && p.section.is_none() {
                        self.saw_watermark = false;
                        continue;
                    }
                    out.push(Block::Para(p));
                }
                "w:tbl" => {
                    let t = self.table(c);
                    out.push(Block::Table(t));
                }
                "w:sdt" => {
                    if let Some(sc) = c.child("w:sdtContent") {
                        self.blocks(sc, out);
                    }
                }
                "w:customXml" | "w:ins" | "w:moveTo" | "w:smartTag" => self.blocks(c, out),
                "mc:AlternateContent" => {
                    if let Some(ch) = choose_alternate(c) {
                        self.blocks(ch, out);
                    }
                }
                "w:sectPr" if self.kind == Part::Body && self.nest == 0 => self.body_sect = Some(c.clone()),
                "w:commentRangeStart" => self.comment_start(c),
                "w:commentRangeEnd" => self.comment_end(c),
                _ => {}
            }
        }
    }

    fn comment_start(&mut self, c: &Element) {
        if let Some(id) = c.attr("w:id").and_then(|v| v.parse::<u32>().ok()) {
            if !self.comments_active.contains(&id) {
                self.comments_active.push(id);
                self.comments_active.sort_unstable();
            }
        }
    }

    fn comment_end(&mut self, c: &Element) {
        if let Some(id) = c.attr("w:id").and_then(|v| v.parse::<u32>().ok()) {
            self.comments_active.retain(|x| *x != id);
        }
    }

    fn paragraph(&mut self, el: &Element) -> Paragraph {
        let mut p = Paragraph::new();
        if let Some(ppr) = el.child("w:pPr") {
            let (props, mark) = self.para_props(ppr);
            p.props = props;
            p.mark = mark;
            if let Some(s) = ppr.child("w:sectPr") {
                if self.kind == Part::Body && self.nest == 0 {
                    // Filled in by `apply_sections` once all sections are known.
                    self.para_sects.push(s.clone());
                    p.section = Some(Box::default());
                }
            }
        }
        let mut runs = Vec::new();
        self.para_content(el, &RunCtx::default(), &mut runs);
        p.runs = runs;
        self.finish_paragraph(&mut p);
        p
    }

    fn finish_paragraph(&mut self, p: &mut Paragraph) {
        // Fields still swallowing their result at a paragraph end span paragraphs: emit what we have and
        // let the rest of the result through as ordinary text.
        let mut emitted = Vec::new();
        for f in self.fields.iter_mut() {
            if f.mode == FieldMode::Swallow && f.separated {
                f.mode = FieldMode::Pass;
                let field = field_from_instr(&f.instr, &f.result);
                emitted.push(Run { content: Inline::Field(field), props: f.props.clone().unwrap_or_else(|| f.begin_props.clone()) });
            }
        }
        p.runs.extend(emitted);
        merge_runs(&mut p.runs);
        self.strip_space = false;
    }

    fn para_content(&mut self, el: &Element, ctx: &RunCtx, out: &mut Vec<Run>) {
        for c in el.elems() {
            match c.name.as_str() {
                "w:r" => self.run(c, ctx, out),
                "w:hyperlink" => {
                    let mut target = c.attr("r:id").and_then(|id| self.rels.map.get(id)).map(|r| r.target.clone());
                    if let Some(a) = c.attr("w:anchor") {
                        target = Some(match target {
                            Some(t) => format!("{}#{}", t, a),
                            None => format!("#{}", a),
                        });
                    }
                    let mut ctx2 = ctx.clone();
                    if target.is_some() {
                        ctx2.link = target;
                    }
                    self.para_content(c, &ctx2, out);
                }
                "w:ins" | "w:moveTo" | "w:del" | "w:moveFrom" => {
                    let kind = if matches!(c.name.as_str(), "w:ins" | "w:moveTo") { RevisionKind::Insert } else { RevisionKind::Delete };
                    let mut ctx2 = ctx.clone();
                    ctx2.revision = Some(Revision {
                        kind,
                        author: c.attr("w:author").unwrap_or("").to_string(),
                        date: c.attr("w:date").unwrap_or("").to_string(),
                    });
                    self.para_content(c, &ctx2, out);
                }
                "w:smartTag" | "w:customXml" | "w:dir" | "w:bdo" | "w:sdtContent" => self.para_content(c, ctx, out),
                "w:sdt" => {
                    if let Some(sc) = c.child("w:sdtContent") {
                        self.para_content(sc, ctx, out);
                    }
                }
                "w:fldSimple" => {
                    let instr = c.attr("w:instr").unwrap_or("").to_string();
                    let kw = instr_keyword(&instr);
                    if matches!(kw.as_str(), "HYPERLINK" | "TOC" | "INDEX") {
                        let mut ctx2 = ctx.clone();
                        if kw == "HYPERLINK" {
                            ctx2.link = hyperlink_target(&instr).or(ctx2.link);
                        }
                        self.para_content(c, &ctx2, out);
                    } else {
                        let mut inner = Vec::new();
                        self.para_content(c, ctx, &mut inner);
                        let result: String = inner.iter().map(|r| r.content.plain()).collect();
                        let props = inner.first().map(|r| r.props.clone()).unwrap_or_else(|| self.ctx_props(ctx));
                        self.emit(Run { content: Inline::Field(field_from_instr(&instr, &result)), props }, out);
                    }
                }
                "w:bookmarkStart" => {
                    if let Some(n) = c.attr("w:name") {
                        if n != "_GoBack" && !n.is_empty() {
                            let props = self.ctx_props(ctx);
                            self.emit(Run { content: Inline::Bookmark(n.to_string()), props }, out);
                        }
                    }
                }
                "w:commentRangeStart" => self.comment_start(c),
                "w:commentRangeEnd" => self.comment_end(c),
                "mc:AlternateContent" => {
                    if let Some(ch) = choose_alternate(c) {
                        self.para_content(ch, ctx, out);
                    }
                }
                "m:oMathPara" | "m:oMath" => {
                    let mut ts = Vec::new();
                    c.find_all("m:t", &mut ts);
                    let s: String = ts.iter().map(|t| t.text()).collect();
                    if !s.is_empty() {
                        let props = self.ctx_props(ctx);
                        self.emit(Run { content: Inline::Text(s), props }, out);
                    }
                }
                _ => {}
            }
        }
    }

    fn ctx_props(&self, ctx: &RunCtx) -> CharProps {
        CharProps { link: ctx.link.clone(), revision: ctx.revision.clone(), comments: self.comments_active.clone(), ..Default::default() }
    }

    fn run(&mut self, r: &Element, ctx: &RunCtx, out: &mut Vec<Run>) {
        let mut props = r.child("w:rPr").map(|p| self.char_props(p)).unwrap_or_default();
        if ctx.link.is_some() {
            props.link = ctx.link.clone();
        }
        if ctx.revision.is_some() {
            props.revision = ctx.revision.clone();
        }
        props.comments = self.comments_active.clone();
        self.run_children(r, &props, out);
    }

    fn run_children(&mut self, r: &Element, props: &CharProps, out: &mut Vec<Run>) {
        for c in r.elems() {
            let content = match c.name.as_str() {
                "w:t" | "w:delText" => {
                    let mut t = c.text();
                    if self.strip_space {
                        self.strip_space = false;
                        if let Some(s) = t.strip_prefix(' ') {
                            t = s.to_string();
                        }
                    }
                    if t.is_empty() {
                        continue;
                    }
                    Inline::Text(t)
                }
                "w:tab" | "w:ptab" => Inline::Tab,
                "w:br" => match c.attr("w:type") {
                    Some("page") => Inline::PageBreak,
                    Some("column") => Inline::ColumnBreak,
                    _ => Inline::LineBreak,
                },
                "w:cr" => Inline::LineBreak,
                "w:noBreakHyphen" => Inline::Text("\u{2011}".into()),
                "w:softHyphen" => Inline::Text("\u{00AD}".into()),
                "w:sym" => {
                    let code = c.attr("w:char").and_then(|h| u32::from_str_radix(h, 16).ok()).unwrap_or(0x20);
                    let font = c.attr("w:font").map(|s| s.to_string());
                    let s = char::from_u32(code).map(|ch| ch.to_string()).unwrap_or_default();
                    let mapped = map_symbol_bullet(&s, font.as_deref());
                    let mut p = props.clone();
                    let text = match mapped {
                        Some(m) => m,
                        None => {
                            p.font = font;
                            if (0xF020..0xF100).contains(&code) {
                                char::from_u32(code - 0xF000).map(|c| c.to_string()).unwrap_or(s)
                            } else {
                                s
                            }
                        }
                    };
                    self.emit(Run { content: Inline::Text(text), props: p }, out);
                    continue;
                }
                "w:drawing" => {
                    for i in self.drawing(c) {
                        self.strip_space = false;
                        self.emit(Run { content: i, props: props.clone() }, out);
                    }
                    continue;
                }
                "w:pict" | "w:object" => match self.vml(c) {
                    Some(i) => i,
                    None => continue,
                },
                "mc:AlternateContent" => {
                    if let Some(ch) = choose_alternate(c) {
                        self.run_children(ch, props, out);
                    }
                    continue;
                }
                "w:fldChar" => {
                    match c.attr("w:fldCharType") {
                        Some("begin") => self.fields.push(FieldState {
                            instr: String::new(),
                            separated: false,
                            mode: FieldMode::Swallow,
                            result: String::new(),
                            props: None,
                            begin_props: props.clone(),
                            link: None,
                        }),
                        Some("separate") => {
                            if let Some(f) = self.fields.last_mut() {
                                f.separated = true;
                                let kw = instr_keyword(&f.instr);
                                if matches!(kw.as_str(), "HYPERLINK" | "TOC" | "INDEX" | "BIBLIOGRAPHY" | "TOA") {
                                    f.mode = FieldMode::Pass;
                                    if kw == "HYPERLINK" {
                                        f.link = hyperlink_target(&f.instr);
                                    }
                                }
                            }
                        }
                        Some("end") => {
                            if let Some(f) = self.fields.pop() {
                                if f.mode == FieldMode::Swallow {
                                    let field = field_from_instr(&f.instr, &f.result);
                                    let p = f.props.unwrap_or(f.begin_props);
                                    self.emit(Run { content: Inline::Field(field), props: p }, out);
                                }
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                "w:instrText" | "w:delInstrText" => {
                    if let Some(f) = self.fields.last_mut() {
                        if !f.separated {
                            f.instr.push_str(&c.text());
                        }
                    }
                    continue;
                }
                "w:footnoteReference" => match c.attr("w:id").and_then(|v| v.parse::<u32>().ok()) {
                    Some(id) => Inline::FootnoteRef(id),
                    None => continue,
                },
                "w:endnoteReference" => match c.attr("w:id").and_then(|v| v.parse::<u32>().ok()) {
                    Some(id) => Inline::EndnoteRef(id),
                    None => continue,
                },
                "w:footnoteRef" | "w:endnoteRef" => {
                    self.strip_space = true;
                    continue;
                }
                _ => continue,
            };
            if !matches!(content, Inline::Text(_)) {
                self.strip_space = false;
            }
            self.emit(Run { content, props: props.clone() }, out);
        }
    }

    /// Route a run through any open complex fields.
    fn emit(&mut self, mut run: Run, out: &mut Vec<Run>) {
        if let Some(f) = self.fields.last_mut() {
            if !f.separated {
                if let Inline::Text(t) = &run.content {
                    f.instr.push_str(t);
                }
                return;
            }
            if f.mode == FieldMode::Swallow {
                f.result.push_str(&run.content.plain());
                if f.props.is_none() {
                    f.props = Some(run.props.clone());
                }
                return;
            }
        }
        if run.props.link.is_none() {
            if let Some(l) = self.fields.iter().rev().find_map(|f| f.link.clone()) {
                run.props.link = Some(l);
            }
        }
        out.push(run);
    }

    fn load_image(&mut self, rel_id: &str) -> Option<Arc<ImageData>> {
        let rel = self.rels.map.get(rel_id)?;
        if rel.external {
            return None;
        }
        if let Some(d) = self.images.get(&rel.target) {
            return Some(d.clone());
        }
        let bytes = self.pkg.get(&rel.target)?.to_vec();
        let d = image_data(bytes);
        self.images.insert(rel.target.clone(), d.clone());
        Some(d)
    }

    fn drawing(&mut self, d: &Element) -> Vec<Inline> {
        let Some(cont) = d.child("wp:inline").or_else(|| d.child("wp:anchor")) else { return Vec::new() };
        let float = if cont.name == "wp:anchor" { Some(anchor_float(cont)) } else { None };
        let ext = cont.child("wp:extent");
        let cx = ext.and_then(|e| num_attr(e, "cx")).unwrap_or(0.0) / EMU_PER_PT;
        let cy = ext.and_then(|e| num_attr(e, "cy")).unwrap_or(0.0) / EMU_PER_PT;
        let doc_pr = cont.child("wp:docPr");
        let alt = doc_pr.and_then(|p| p.attr("descr").filter(|s| !s.is_empty()).or_else(|| p.attr("title"))).unwrap_or("").to_string();
        let gd = cont.child("a:graphic").and_then(|g| g.child("a:graphicData"));
        let mut out = Vec::new();
        if let Some(gd) = gd {
            if let Some(grp) = gd.child("wpg:wgp") {
                self.group(grp, (0.0, 0.0, cx, cy), float, &mut out);
                if !out.is_empty() {
                    return out;
                }
            }
            if let Some(wsp) = gd.child("wps:wsp") {
                if let Some(tb) = self.wsp_textbox(wsp, cx, cy, float) {
                    return vec![Inline::TextBox(Box::new(tb))];
                }
            }
        }
        if let Some(blip) = cont.find("a:blip") {
            if let Some(id) = blip.attr("r:embed") {
                if let Some(data) = self.load_image(id) {
                    let (w, h) = if cx > 0.0 && cy > 0.0 { (cx, cy) } else { natural_size(&data, 468.0) };
                    out.push(Inline::Image(Image { data, width: w, height: h, alt, float }));
                    return out;
                }
            }
        }
        if let Some(tb) = cont.find("w:txbxContent") {
            let paragraphs = self.textbox_paras(tb);
            let tb = TextBox { paragraphs, width: cx.max(1.0), height: cy.max(1.0), float, fill: None, border: None, ..Default::default() };
            out.push(Inline::TextBox(Box::new(tb)));
        }
        out
    }

    /// Pictures and text boxes of a shape group. `frame`: the group's placement (x, y, w, h) in points,
    /// relative to the anchor position.
    fn group(&mut self, grp: &Element, frame: (f32, f32, f32, f32), float: Option<Float>, out: &mut Vec<Inline>) {
        let xf = grp.child("wpg:grpSpPr").and_then(|p| p.child("a:xfrm"));
        let get = |n: &str, a: &str| xf.and_then(|x| x.child(n)).and_then(|e| num_attr(e, a));
        let (chx, chy) = (get("a:chOff", "x").unwrap_or(0.0), get("a:chOff", "y").unwrap_or(0.0));
        let chw = get("a:chExt", "cx").filter(|v| *v > 0.0);
        let chh = get("a:chExt", "cy").filter(|v| *v > 0.0);
        let sx = chw.map(|w| frame.2 / w).unwrap_or(1.0 / EMU_PER_PT);
        let sy = chh.map(|h| frame.3 / h).unwrap_or(1.0 / EMU_PER_PT);
        for c in grp.elems() {
            let sppr = c.child("wps:spPr").or_else(|| c.child("pic:spPr")).or_else(|| c.child("wpg:grpSpPr"));
            let cxf = sppr.and_then(|p| p.child("a:xfrm"));
            let off = cxf.and_then(|x| x.child("a:off"));
            let ext = cxf.and_then(|x| x.child("a:ext"));
            let x = frame.0 + (off.and_then(|o| num_attr(o, "x")).unwrap_or(chx) - chx) * sx;
            let y = frame.1 + (off.and_then(|o| num_attr(o, "y")).unwrap_or(chy) - chy) * sy;
            let w = ext.and_then(|e| num_attr(e, "cx")).unwrap_or(0.0) * sx;
            let h = ext.and_then(|e| num_attr(e, "cy")).unwrap_or(0.0) * sy;
            let child_float = float.map(|mut f| {
                if f.h_align.is_none() {
                    f.x += x;
                }
                if f.v_align.is_none() {
                    f.y += y;
                }
                f
            });
            match c.name.as_str() {
                "wpg:grpSp" => self.group(c, (x, y, w, h), float, out),
                "wps:wsp" => {
                    if let Some(tb) = self.wsp_textbox(c, w, h, child_float) {
                        out.push(Inline::TextBox(Box::new(tb)));
                    }
                }
                "pic:pic" => {
                    let Some(id) = c.find("a:blip").and_then(|b| b.attr("r:embed")) else { continue };
                    let Some(data) = self.load_image(id) else { continue };
                    let alt = c.find("pic:cNvPr").and_then(|p| p.attr("descr")).unwrap_or("").to_string();
                    let (w, h) = if w > 0.0 && h > 0.0 { (w, h) } else { natural_size(&data, 468.0) };
                    out.push(Inline::Image(Image { data, width: w, height: h, alt, float: child_float }));
                }
                _ => {}
            }
        }
    }

    /// A `wps:wsp` shape with text → text box (shapes without text are skipped).
    fn wsp_textbox(&mut self, wsp: &Element, w: f32, h: f32, float: Option<Float>) -> Option<TextBox> {
        let content = wsp.child("wps:txbx").and_then(|t| t.child("w:txbxContent"))?;
        let paragraphs = self.textbox_paras(content);
        let sppr = wsp.child("wps:spPr");
        let style = wsp.child("wps:style");
        let style_ref = |name: &str| style.and_then(|s| s.child(name)).filter(|r| num_attr(r, "idx").unwrap_or(0.0) > 0.0);
        let fill = match sppr.and_then(|p| p.elems().find(|e| matches!(e.name.as_str(), "a:noFill" | "a:solidFill" | "a:gradFill" | "a:pattFill"))) {
            Some(f) if f.name == "a:noFill" => None,
            Some(f) if f.name == "a:solidFill" => self.drawing_color(f),
            Some(f) => f.find("a:gs").and_then(|g| self.drawing_color(g)).or_else(|| f.find("a:fgClr").and_then(|g| self.drawing_color(g))),
            None => style_ref("a:fillRef").and_then(|r| self.drawing_color(r)),
        };
        let border = match sppr.and_then(|p| p.child("a:ln")) {
            Some(ln) if ln.child("a:noFill").is_some() => None,
            Some(ln) => {
                let color = ln
                    .child("a:solidFill")
                    .and_then(|f| self.drawing_color(f))
                    .or_else(|| style_ref("a:lnRef").and_then(|r| self.drawing_color(r)));
                // A line without a fill of its own uses the style's line colour; no style line → none.
                color.map(|color| Border { style: line_dash(ln), width: num_attr(ln, "w").map(|v| v / EMU_PER_PT).unwrap_or(0.75), color, space: 0.0 })
            }
            None => style_ref("a:lnRef").and_then(|r| self.drawing_color(r)).map(|color| Border { style: BorderStyle::Single, width: 0.75, color, space: 0.0 }),
        };
        let body = wsp.child("wps:bodyPr");
        let padding = body.and_then(|b| num_attr(b, "lIns")).unwrap_or(91440.0) / EMU_PER_PT;
        let auto_height = body.map(|b| b.child("a:spAutoFit").is_some()).unwrap_or(false);
        Some(TextBox { paragraphs, width: w.max(1.0), height: h.max(1.0), float, fill, border, padding, auto_height })
    }

    /// Colour of a DrawingML fill/colour container (`a:srgbClr`, `a:schemeClr`, `a:prstClr`, `a:sysClr`).
    fn drawing_color(&self, e: &Element) -> Option<Color> {
        let c = e.elems().find(|c| matches!(c.name.as_str(), "a:srgbClr" | "a:schemeClr" | "a:prstClr" | "a:sysClr" | "a:scrgbClr"))?;
        let base = match c.name.as_str() {
            "a:srgbClr" => c.attr("val").and_then(Color::from_hex)?,
            "a:sysClr" => c.attr("lastClr").and_then(Color::from_hex).unwrap_or(if c.attr("val") == Some("window") { Color::WHITE } else { Color::BLACK }),
            "a:prstClr" => c.attr("val").and_then(named_color).unwrap_or(Color::BLACK),
            "a:scrgbClr" => {
                let ch = |a: &str| (num_attr(c, a).unwrap_or(0.0) / 100000.0 * 255.0).round().clamp(0.0, 255.0) as u8;
                Color::rgb(ch("r"), ch("g"), ch("b"))
            }
            _ => {
                let v = c.attr("val").unwrap_or("dk1");
                let key = match v {
                    "tx1" => "dk1",
                    "bg1" => "lt1",
                    "tx2" => "dk2",
                    "bg2" => "lt2",
                    "phClr" => "accent1",
                    k => k,
                };
                self.theme_colors.get(key).copied().unwrap_or(Color::BLACK)
            }
        };
        let m = |n: &str| c.child(n).and_then(|e| num_attr(e, "val")).map(|v| v / 100000.0);
        Some(adjust_color(base, m("a:lumMod"), m("a:lumOff"), m("a:shade"), m("a:tint")))
    }

    fn textbox_paras(&mut self, tb: &Element) -> Vec<Paragraph> {
        let saved_fields = std::mem::take(&mut self.fields);
        let saved_strip = self.strip_space;
        self.nest += 1;
        let mut blocks = Vec::new();
        self.blocks(tb, &mut blocks);
        self.nest -= 1;
        self.fields = saved_fields;
        self.strip_space = saved_strip;
        let mut paras = flatten_blocks(blocks);
        for p in paras.iter_mut() {
            p.section = None;
        }
        if paras.is_empty() {
            paras.push(Paragraph::new());
        }
        paras
    }

    fn vml(&mut self, el: &Element) -> Option<Inline> {
        let mut tps = Vec::new();
        el.find_all("v:textpath", &mut tps);
        if let Some(s) = tps.iter().find_map(|t| t.attr("string")) {
            if self.kind == Part::Header {
                if !s.trim().is_empty() {
                    self.found_watermark = Some(s.to_string());
                }
                self.saw_watermark = true;
                return None;
            }
        }
        let shape = el.elems().find(|e| e.name.starts_with("v:") && e.name != "v:shapetype").or_else(|| el.find("v:shape")).or_else(|| el.find("v:rect"));
        let style = shape.and_then(|s| s.attr("style")).unwrap_or("");
        let css = parse_vml_style(style);
        let w = css.get("width").and_then(|v| parse_length(v));
        let h = css.get("height").and_then(|v| parse_length(v));
        let float = shape.and_then(|s| vml_float(s, &css));
        if let Some(img) = el.find("v:imagedata") {
            let id = img.attr("r:id").or_else(|| img.attr("r:pict"))?;
            let data = self.load_image(id)?;
            let (nw, nh) = natural_size(&data, 468.0);
            let alt = shape.and_then(|s| s.attr("alt")).or_else(|| img.attr("o:title")).unwrap_or("").to_string();
            return Some(Inline::Image(Image { data, width: w.unwrap_or(nw), height: h.unwrap_or(nh), alt, float }));
        }
        if let Some(tb) = el.find("w:txbxContent") {
            let paragraphs = self.textbox_paras(tb);
            let shape = shape?;
            let fill = if shape.attr("filled").map(vml_false).unwrap_or(false) {
                None
            } else {
                Some(shape.attr("fillcolor").and_then(vml_color).unwrap_or(Color::WHITE))
            };
            let border = if shape.attr("stroked").map(vml_false).unwrap_or(false) {
                None
            } else {
                Some(Border {
                    style: BorderStyle::Single,
                    width: shape.attr("strokeweight").and_then(parse_length).unwrap_or(0.75),
                    color: shape.attr("strokecolor").and_then(vml_color).unwrap_or(Color::BLACK),
                    space: 0.0,
                })
            };
            let vtb = shape.child("v:textbox");
            let padding = vtb.and_then(|t| t.attr("inset")).and_then(|i| i.split(',').next().and_then(|v| parse_length(v.trim()))).unwrap_or(7.2);
            let auto_height = vtb.and_then(|t| t.attr("style")).map(|s| s.replace(' ', "").contains("mso-fit-shape-to-text:t")).unwrap_or(false);
            return Some(Inline::TextBox(Box::new(TextBox {
                paragraphs,
                width: w.unwrap_or(144.0),
                height: h.unwrap_or(72.0),
                float,
                fill,
                border,
                padding,
                auto_height,
            })));
        }
        None
    }

    // ----- properties -----

    fn font_of(&self, f: &Element) -> Option<String> {
        for a in ["w:ascii", "w:hAnsi"] {
            if let Some(v) = f.attr(a) {
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
        for a in ["w:asciiTheme", "w:hAnsiTheme"] {
            if let Some(v) = f.attr(a) {
                return Some(if v.starts_with("major") { self.major_font.clone() } else { self.minor_font.clone() });
            }
        }
        for a in ["w:eastAsia", "w:cs"] {
            if let Some(v) = f.attr(a) {
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
        None
    }

    fn char_props(&self, rpr: &Element) -> CharProps {
        let mut c = CharProps::default();
        for e in rpr.elems() {
            match e.name.as_str() {
                "w:rStyle" => c.style = val(e).map(|v| self.style_id(v)),
                "w:rFonts" => {
                    if let Some(f) = self.font_of(e) {
                        c.font = Some(f);
                    }
                }
                "w:b" => c.bold = Some(on(e)),
                "w:i" => c.italic = Some(on(e)),
                "w:u" => {
                    c.underline = Some(match val(e).unwrap_or("single") {
                        "none" => Underline::None,
                        "double" => Underline::Double,
                        "thick" => Underline::Thick,
                        "dotted" | "dottedHeavy" => Underline::Dotted,
                        "dash" | "dashedHeavy" | "dashLong" | "dashLongHeavy" | "dotDash" | "dashDotHeavy" | "dotDotDash"
                        | "dashDotDotHeavy" => Underline::Dashed,
                        "wave" | "wavyHeavy" | "wavyDouble" => Underline::Wavy,
                        "words" => Underline::Words,
                        _ => Underline::Single,
                    })
                }
                "w:strike" => c.strike = Some(on(e)),
                "w:dstrike" => c.double_strike = Some(on(e)),
                "w:color" => {
                    if let Some(v) = val(e) {
                        c.color = Some(hex_color(v).unwrap_or(Color::BLACK));
                    }
                }
                "w:highlight" => c.highlight = val(e).and_then(highlight_color),
                "w:shd" => c.shading = shading_fill(e),
                "w:vertAlign" => {
                    c.vert_align = Some(match val(e) {
                        Some("superscript") => VertAlign::Superscript,
                        Some("subscript") => VertAlign::Subscript,
                        _ => VertAlign::Baseline,
                    })
                }
                "w:caps" => c.caps = Some(on(e)),
                "w:smallCaps" => c.small_caps = Some(on(e)),
                "w:vanish" => c.hidden = Some(on(e)),
                "w:spacing" => c.spacing = tw(e, "w:val"),
                "w:sz" => c.size = num_attr(e, "w:val").map(|v| v / 2.0),
                _ => {}
            }
        }
        c
    }

    fn para_props(&self, ppr: &Element) -> (ParaProps, CharProps) {
        let mut p = ParaProps::default();
        let mut mark = CharProps::default();
        for e in ppr.elems() {
            match e.name.as_str() {
                "w:pStyle" => p.style = val(e).map(|v| self.style_id(v)),
                "w:jc" => {
                    p.align = match val(e) {
                        Some("center") => Some(Align::Center),
                        Some("right" | "end") => Some(Align::Right),
                        Some("both" | "distribute" | "lowKashida" | "mediumKashida" | "highKashida" | "thaiDistribute") => {
                            Some(Align::Justify)
                        }
                        Some(_) => Some(Align::Left),
                        None => None,
                    }
                }
                "w:ind" => {
                    if let Some(v) = tw(e, "w:left").or_else(|| tw(e, "w:start")) {
                        p.indent_left = Some(v);
                    }
                    if let Some(v) = tw(e, "w:right").or_else(|| tw(e, "w:end")) {
                        p.indent_right = Some(v);
                    }
                    if let Some(v) = tw(e, "w:hanging") {
                        p.indent_first = Some(-v);
                    } else if let Some(v) = tw(e, "w:firstLine") {
                        p.indent_first = Some(v);
                    }
                }
                "w:spacing" => {
                    if let Some(v) = tw(e, "w:before") {
                        p.space_before = Some(v);
                    } else if e.attr("w:beforeAutospacing").map(|v| v == "1" || v == "true").unwrap_or(false) {
                        p.space_before = Some(14.0);
                    }
                    if let Some(v) = tw(e, "w:after") {
                        p.space_after = Some(v);
                    } else if e.attr("w:afterAutospacing").map(|v| v == "1" || v == "true").unwrap_or(false) {
                        p.space_after = Some(14.0);
                    }
                    if let Some(line) = num_attr(e, "w:line") {
                        p.line_spacing = Some(match e.attr("w:lineRule").unwrap_or("auto") {
                            "exact" => LineSpacing::Exactly(line.abs() / 20.0),
                            "atLeast" => LineSpacing::AtLeast(line / 20.0),
                            _ => LineSpacing::Multiple((line / 240.0 * 100.0).round() / 100.0),
                        });
                    }
                }
                "w:contextualSpacing" => p.contextual_spacing = Some(on(e)),
                "w:keepNext" => p.keep_next = Some(on(e)),
                "w:keepLines" => p.keep_lines = Some(on(e)),
                "w:pageBreakBefore" => p.page_break_before = Some(on(e)),
                "w:widowControl" => p.widow_control = Some(on(e)),
                "w:numPr" => {
                    let id = e.child("w:numId").and_then(|n| num_attr(n, "w:val")).map(|v| v as u32);
                    let lvl = e.child("w:ilvl").and_then(|n| num_attr(n, "w:val")).unwrap_or(0.0) as u8;
                    match id {
                        Some(0) => p.list = Some(None),
                        Some(id) if self.doc.list(id).is_some() => p.list = Some(Some(ListRef { list: id, level: lvl.min(8) })),
                        Some(_) => p.list = Some(None),
                        None => {}
                    }
                }
                "w:shd" => p.shading = shading_fill(e),
                "w:pBdr" => p.borders = Some(parse_borders(e)),
                "w:tabs" => {
                    for t in e.children_named("w:tab") {
                        let Some(pos) = tw(t, "w:pos") else { continue };
                        let align = match val(t) {
                            Some("center") => TabAlign::Center,
                            Some("right" | "end") => TabAlign::Right,
                            Some("decimal") => TabAlign::Decimal,
                            Some("clear") => TabAlign::Clear,
                            Some("bar") => continue,
                            _ => TabAlign::Left,
                        };
                        let leader = match t.attr("w:leader") {
                            Some("dot" | "middleDot") => TabLeader::Dot,
                            Some("hyphen") => TabLeader::Hyphen,
                            Some("underscore" | "heavy") => TabLeader::Underscore,
                            _ => TabLeader::None,
                        };
                        p.tabs.push(TabStop { pos, align, leader });
                    }
                }
                "w:outlineLvl" => {
                    if let Some(v) = num_attr(e, "w:val") {
                        p.outline_level = if v < 9.0 { Some(v as u8) } else { None };
                    }
                }
                "w:rPr" => mark = self.char_props(e),
                _ => {}
            }
        }
        (p, mark)
    }

    // ----- tables -----

    fn table(&mut self, el: &Element) -> Table {
        self.nest += 1;
        let t = self.table_inner(el);
        self.nest -= 1;
        t
    }

    fn table_inner(&mut self, el: &Element) -> Table {
        let mut t = Table { style: None, borders: Borders::default(), ..Default::default() };
        let mut has_borders = false;
        if let Some(pr) = el.child("w:tblPr") {
            t.style = pr.child_attr("w:tblStyle", "w:val").map(|v| self.style_id(v));
            t.align = match pr.child_attr("w:jc", "w:val") {
                Some("center") => Align::Center,
                Some("right" | "end") => Align::Right,
                _ => Align::Left,
            };
            if let Some(ind) = pr.child("w:tblInd") {
                if ind.attr("w:type").map(|t| t == "dxa").unwrap_or(true) {
                    t.indent = tw(ind, "w:w").unwrap_or(0.0);
                }
            }
            if let Some(b) = pr.child("w:tblBorders") {
                t.borders = parse_borders(b);
                has_borders = true;
                // Missing edges fall back to the style's.
                if let Some(sb) = t.style.as_deref().and_then(|s| self.table_borders_from_style(s)) {
                    let tb = &mut t.borders;
                    tb.top = tb.top.or(sb.top);
                    tb.bottom = tb.bottom.or(sb.bottom);
                    tb.left = tb.left.or(sb.left);
                    tb.right = tb.right.or(sb.right);
                    tb.between = tb.between.or(sb.between);
                    tb.inside_v = tb.inside_v.or(sb.inside_v);
                }
            }
            if let Some(m) = pr.child("w:tblCellMar") {
                if let Some(l) = m.child("w:left").or_else(|| m.child("w:start")).and_then(|l| tw(l, "w:w")) {
                    t.cell_margin = l;
                }
            }
        }
        if !has_borders {
            if let Some(b) = t.style.as_deref().and_then(|s| self.table_borders_from_style(s)) {
                t.borders = b;
            }
        }
        if let Some(g) = el.child("w:tblGrid") {
            t.columns = g.children_named("w:gridCol").map(|c| tw(c, "w:w").unwrap_or(0.0)).collect();
        }
        let mut rows = Vec::new();
        collect_named(el, "w:tr", &mut rows);
        for tr in rows {
            let mut row = Row::default();
            if let Some(pr) = tr.child("w:trPr") {
                if let Some(h) = pr.child("w:trHeight").and_then(|h| tw(h, "w:val")) {
                    if h > 0.0 {
                        row.height = Some(h);
                    }
                }
                row.header = pr.child("w:tblHeader").map(on).unwrap_or(false);
                if let Some(gb) = pr.child("w:gridBefore").and_then(|g| num_attr(g, "w:val")) {
                    if gb >= 1.0 {
                        row.cells.push(Cell { span: gb as u32, borders: Some(Borders::all(Border { style: BorderStyle::None, width: 0.0, ..Border::single() })), ..Default::default() });
                    }
                }
            }
            let mut cells = Vec::new();
            collect_named(tr, "w:tc", &mut cells);
            for tc in cells {
                let mut cell = Cell { paragraphs: Vec::new(), ..Default::default() };
                let mut hmerge_continue = false;
                if let Some(pr) = tc.child("w:tcPr") {
                    if let Some(w) = pr.child("w:tcW") {
                        if w.attr("w:type").map(|t| t == "dxa").unwrap_or(true) {
                            cell.width = tw(w, "w:w").filter(|w| *w > 0.0);
                        }
                    }
                    if let Some(s) = pr.child("w:gridSpan").and_then(|g| num_attr(g, "w:val")) {
                        cell.span = (s as u32).max(1);
                    }
                    if let Some(v) = pr.child("w:vMerge") {
                        cell.vmerge = if val(v) == Some("restart") { VMerge::Restart } else { VMerge::Continue };
                    }
                    if let Some(h) = pr.child("w:hMerge") {
                        hmerge_continue = val(h) != Some("restart");
                    }
                    if let Some(b) = pr.child("w:tcBorders") {
                        cell.borders = Some(parse_borders(b));
                    }
                    if let Some(s) = pr.child("w:shd") {
                        cell.shading = shading_fill(s);
                    }
                    cell.valign = match pr.child_attr("w:vAlign", "w:val") {
                        Some("center") => VAlign::Center,
                        Some("bottom") => VAlign::Bottom,
                        _ => VAlign::Top,
                    };
                }
                let mut blocks = Vec::new();
                self.blocks(tc, &mut blocks);
                cell.paragraphs = flatten_blocks(blocks);
                if cell.paragraphs.is_empty() {
                    cell.paragraphs.push(Paragraph::new());
                }
                if hmerge_continue {
                    if let Some(prev) = row.cells.last_mut() {
                        prev.span += cell.span;
                        continue;
                    }
                }
                row.cells.push(cell);
            }
            if !row.cells.is_empty() {
                t.rows.push(row);
            }
        }
        // Make sure the grid covers every row.
        let ncols = t.rows.iter().map(|r| r.cells.iter().map(|c| c.span.max(1) as usize).sum::<usize>()).max().unwrap_or(1).max(1);
        if t.columns.len() < ncols || t.columns.iter().all(|w| *w <= 0.0) {
            let widths_from_cells: Option<Vec<f32>> = t.rows.iter().find(|r| r.cells.iter().all(|c| c.span == 1 && c.width.is_some()) && r.cells.len() == ncols).map(|r| r.cells.iter().map(|c| c.width.unwrap()).collect());
            t.columns = widths_from_cells.unwrap_or_else(|| vec![468.0 / ncols as f32; ncols]);
        }
        for w in t.columns.iter_mut() {
            if *w <= 0.0 {
                *w = 1.0;
            }
        }
        if t.rows.is_empty() {
            t.rows.push(Row { cells: vec![Cell::default()], height: None, header: false });
        }
        t
    }
}

/// Collect `name` children, descending through sdt/customXml wrappers.
fn collect_named<'e>(el: &'e Element, name: &str, out: &mut Vec<&'e Element>) {
    for c in el.elems() {
        if c.name == name {
            out.push(c);
        } else if matches!(c.name.as_str(), "w:sdt" | "w:sdtContent" | "w:customXml" | "w:ins" | "w:smartTag") {
            collect_named(c, name, out);
        } else if c.name == "mc:AlternateContent" {
            if let Some(ch) = choose_alternate(c) {
                collect_named(ch, name, out);
            }
        }
    }
}

/// Pick the branch of an `mc:AlternateContent` to read: a Choice with a picture, shape or group we
/// understand, else the Fallback, else the first Choice.
fn choose_alternate(ac: &Element) -> Option<&Element> {
    let choices: Vec<&Element> = ac.children_named("mc:Choice").collect();
    if let Some(c) = choices.iter().find(|c| c.find("a:blip").is_some() || c.find("wps:wsp").is_some() || c.find("wpg:wgp").is_some()) {
        return Some(c);
    }
    if let Some(f) = ac.child("mc:Fallback") {
        return Some(f);
    }
    choices.first().copied()
}

const EMU_PER_PT: f32 = 12700.0;

/// Floating placement of a `wp:anchor`.
fn anchor_float(a: &Element) -> Float {
    let emu = |e: &Element, n: &str| num_attr(e, n).map(|v| v / EMU_PER_PT);
    let mut f = Float { dist: ["distL", "distR", "distT", "distB"].iter().filter_map(|n| emu(a, n)).fold(0.0, f32::max), ..Default::default() };
    let behind = a.attr("behindDoc").map(|v| v == "1" || v == "true").unwrap_or(false);
    for c in a.elems() {
        match c.name.as_str() {
            "wp:wrapSquare" => f.wrap = WrapMode::Square,
            "wp:wrapTight" | "wp:wrapThrough" => f.wrap = WrapMode::Tight,
            "wp:wrapTopAndBottom" => f.wrap = WrapMode::TopBottom,
            "wp:wrapNone" => f.wrap = if behind { WrapMode::Behind } else { WrapMode::InFront },
            _ => {}
        }
    }
    let text_of = |e: &Element, n: &str| e.child(n).map(|x| x.text().trim().to_string());
    if a.attr("simplePos").map(|v| v == "1" || v == "true").unwrap_or(false) {
        if let Some(sp) = a.child("wp:simplePos") {
            f.h_rel = HRel::Page;
            f.v_rel = VRel::Page;
            f.x = emu(sp, "x").unwrap_or(0.0);
            f.y = emu(sp, "y").unwrap_or(0.0);
            return f;
        }
    }
    if let Some(ph) = a.child("wp:positionH") {
        f.h_rel = match ph.attr("relativeFrom").unwrap_or("column") {
            "page" | "leftMargin" | "rightMargin" | "outsideMargin" => HRel::Page,
            "margin" | "insideMargin" => HRel::Margin,
            "character" => HRel::Character,
            _ => HRel::Column,
        };
        if let Some(v) = text_of(ph, "wp:posOffset").and_then(|t| t.parse::<f32>().ok()) {
            f.x = v / EMU_PER_PT;
        }
        f.h_align = text_of(ph, "wp:align").map(|t| match t.as_str() {
            "center" => FloatAlign::Center,
            "right" | "outside" => FloatAlign::End,
            _ => FloatAlign::Start,
        });
        if ph.attr("relativeFrom") == Some("rightMargin") && f.h_align.is_none() {
            // Offsets from the right margin area cannot be expressed; keep the object at the right edge.
            f.h_rel = HRel::Margin;
            f.h_align = Some(FloatAlign::End);
        }
    }
    if let Some(pv) = a.child("wp:positionV") {
        f.v_rel = match pv.attr("relativeFrom").unwrap_or("paragraph") {
            "page" | "topMargin" | "bottomMargin" | "outsideMargin" => VRel::Page,
            "margin" | "insideMargin" => VRel::Margin,
            "line" => VRel::Line,
            _ => VRel::Paragraph,
        };
        if let Some(v) = text_of(pv, "wp:posOffset").and_then(|t| t.parse::<f32>().ok()) {
            f.y = v / EMU_PER_PT;
        }
        f.v_align = text_of(pv, "wp:align").map(|t| match t.as_str() {
            "center" => FloatAlign::Center,
            "bottom" | "outside" => FloatAlign::End,
            _ => FloatAlign::Start,
        });
        if pv.attr("relativeFrom") == Some("bottomMargin") && f.v_align.is_none() {
            f.v_rel = VRel::Margin;
            f.v_align = Some(FloatAlign::End);
        }
    }
    f
}

fn line_dash(ln: &Element) -> BorderStyle {
    if ln.attr("cmpd").map(|c| c != "sng").unwrap_or(false) {
        return BorderStyle::Double;
    }
    match ln.child("a:prstDash").and_then(|d| d.attr("val")) {
        Some("dot" | "sysDot") => BorderStyle::Dotted,
        Some("solid") | None => BorderStyle::Single,
        Some(_) => BorderStyle::Dashed,
    }
}

/// Apply DrawingML luminance / shade / tint modifiers.
fn adjust_color(c: Color, lum_mod: Option<f32>, lum_off: Option<f32>, shade: Option<f32>, tint: Option<f32>) -> Color {
    let (mut r, mut g, mut b) = (c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0);
    if lum_mod.is_some() || lum_off.is_some() {
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let mut l = (max + min) / 2.0;
        let d = max - min;
        let (h, s) = if d == 0.0 {
            (0.0, 0.0)
        } else {
            let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
            let h = if max == r {
                (g - b) / d + if g < b { 6.0 } else { 0.0 }
            } else if max == g {
                (b - r) / d + 2.0
            } else {
                (r - g) / d + 4.0
            } / 6.0;
            (h, s)
        };
        l = (l * lum_mod.unwrap_or(1.0) + lum_off.unwrap_or(0.0)).clamp(0.0, 1.0);
        let hue = |p: f32, q: f32, mut t: f32| {
            if t < 0.0 {
                t += 1.0;
            }
            if t > 1.0 {
                t -= 1.0;
            }
            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        };
        if s == 0.0 {
            r = l;
            g = l;
            b = l;
        } else {
            let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
            let p = 2.0 * l - q;
            r = hue(p, q, h + 1.0 / 3.0);
            g = hue(p, q, h);
            b = hue(p, q, h - 1.0 / 3.0);
        }
    }
    if let Some(sh) = shade {
        r *= sh;
        g *= sh;
        b *= sh;
    }
    if let Some(t) = tint {
        r = 1.0 - (1.0 - r) * t;
        g = 1.0 - (1.0 - g) * t;
        b = 1.0 - (1.0 - b) * t;
    }
    let ch = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    Color::rgb(ch(r), ch(g), ch(b))
}

/// Split a VML `style` attribute into lower-case property → value.
fn parse_vml_style(style: &str) -> HashMap<String, String> {
    style
        .split(';')
        .filter_map(|d| d.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect()
}

fn vml_false(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "f" | "false" | "0")
}

fn vml_color(v: &str) -> Option<Color> {
    let v = v.split_whitespace().next()?;
    css_color(v)
}

/// Floating placement of an absolutely positioned VML shape (None when it is inline).
fn vml_float(shape: &Element, css: &HashMap<String, String>) -> Option<Float> {
    if css.get("position").map(|p| p != "absolute").unwrap_or(true) {
        return None;
    }
    let mut f = Float::default();
    let len = |k: &str| css.get(k).and_then(|v| parse_length(v));
    f.x = len("margin-left").or_else(|| len("left")).unwrap_or(0.0);
    f.y = len("margin-top").or_else(|| len("top")).unwrap_or(0.0);
    f.h_rel = match css.get("mso-position-horizontal-relative").map(|s| s.as_str()) {
        Some("page" | "left-margin-area" | "right-margin-area" | "outer-margin-area") => HRel::Page,
        Some("margin" | "inner-margin-area") => HRel::Margin,
        Some("char") => HRel::Character,
        _ => HRel::Column,
    };
    f.v_rel = match css.get("mso-position-vertical-relative").map(|s| s.as_str()) {
        Some("page" | "top-margin-area" | "bottom-margin-area" | "outer-margin-area") => VRel::Page,
        Some("margin" | "inner-margin-area") => VRel::Margin,
        Some("line") => VRel::Line,
        _ => VRel::Paragraph,
    };
    f.h_align = match css.get("mso-position-horizontal").map(|s| s.as_str()) {
        Some("left" | "inside") => Some(FloatAlign::Start),
        Some("center") => Some(FloatAlign::Center),
        Some("right" | "outside") => Some(FloatAlign::End),
        _ => None,
    };
    f.v_align = match css.get("mso-position-vertical").map(|s| s.as_str()) {
        Some("top" | "inside") => Some(FloatAlign::Start),
        Some("center") => Some(FloatAlign::Center),
        Some("bottom" | "outside") => Some(FloatAlign::End),
        _ => None,
    };
    let behind = css.get("z-index").and_then(|z| z.parse::<i64>().ok()).map(|z| z < 0).unwrap_or(false);
    let wrap = shape.child("w10:wrap").and_then(|w| w.attr("type"));
    f.wrap = match wrap {
        Some("square") => WrapMode::Square,
        Some("tight" | "through") => WrapMode::Tight,
        Some("topAndBottom") => WrapMode::TopBottom,
        _ if behind => WrapMode::Behind,
        _ => WrapMode::InFront,
    };
    f.dist = ["mso-wrap-distance-left", "mso-wrap-distance-right", "mso-wrap-distance-top", "mso-wrap-distance-bottom"]
        .iter()
        .filter_map(|k| len(k))
        .fold(0.0, f32::max);
    if f.dist == 0.0 && matches!(f.wrap, WrapMode::Square | WrapMode::Tight) {
        f.dist = 9.0;
    }
    Some(f)
}

/// Flatten blocks to paragraphs (tables become one paragraph per row, cells separated by tabs).
fn flatten_blocks(blocks: Vec<Block>) -> Vec<Paragraph> {
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
                        for (j, cp) in c.paragraphs.into_iter().enumerate() {
                            if j > 0 {
                                p.runs.push(Run { content: Inline::Text(" ".into()), props: CharProps::default() });
                            }
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

/// Merge adjacent text runs with identical formatting.
pub(crate) fn merge_runs(runs: &mut Vec<Run>) {
    let mut out: Vec<Run> = Vec::with_capacity(runs.len());
    for r in runs.drain(..) {
        if let (Some(last), Inline::Text(t)) = (out.last_mut(), &r.content) {
            if last.props == r.props {
                if let Inline::Text(lt) = &mut last.content {
                    lt.push_str(t);
                    continue;
                }
            }
        }
        out.push(r);
    }
    *runs = out;
}

fn instr_keyword(instr: &str) -> String {
    instr.split_whitespace().next().unwrap_or("").trim_start_matches('\\').to_ascii_uppercase()
}

/// Extract the quoted target of a HYPERLINK field.
fn hyperlink_target(instr: &str) -> Option<String> {
    let rest = instr.trim();
    let rest = rest.get(9..)?.trim();
    let mut url = None;
    let mut anchor = None;
    let mut it = rest.chars().peekable();
    let mut tokens = Vec::new();
    while let Some(&c) = it.peek() {
        if c.is_whitespace() {
            it.next();
        } else if c == '"' {
            it.next();
            let s: String = it.by_ref().take_while(|&c| c != '"').collect();
            tokens.push(s);
        } else {
            let s: String = std::iter::from_fn(|| it.next_if(|c| !c.is_whitespace())).collect();
            tokens.push(s);
        }
    }
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "\\l" => {
                anchor = tokens.get(i + 1).cloned();
                i += 2;
            }
            t if t.starts_with('\\') => {
                i += if matches!(t, "\\o" | "\\t") { 2 } else { 1 };
            }
            t => {
                if url.is_none() {
                    url = Some(t.to_string());
                }
                i += 1;
            }
        }
    }
    match (url, anchor) {
        (Some(u), Some(a)) => Some(format!("{}#{}", u, a)),
        (Some(u), None) => Some(u),
        (None, Some(a)) => Some(format!("#{}", a)),
        _ => None,
    }
}

/// Map a field instruction to a model field.
pub(crate) fn field_from_instr(instr: &str, result: &str) -> Field {
    let kw = instr_keyword(instr);
    match kw.as_str() {
        "PAGE" => Field::PageNumber,
        "NUMPAGES" => Field::PageCount,
        "DATE" | "TIME" => {
            let pic = instr.find("\\@").map(|i| {
                let rest = instr[i + 2..].trim_start();
                if let Some(r) = rest.strip_prefix('"') {
                    r.split('"').next().unwrap_or("").to_string()
                } else {
                    rest.split_whitespace().next().unwrap_or("").to_string()
                }
            });
            Field::Date(pic.unwrap_or_else(|| if kw == "TIME" { "h:mm am/pm".into() } else { "M/d/yyyy".into() }))
        }
        _ => Field::Other { instr: instr.trim().to_string(), result: result.to_string() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets() {
        assert_eq!(resolve_target("word/document.xml", "media/image1.png"), "word/media/image1.png");
        assert_eq!(resolve_target("word/document.xml", "../customXml/item.xml"), "customXml/item.xml");
        assert_eq!(resolve_target("word/document.xml", "/word/media/a.png"), "word/media/a.png");
        assert_eq!(rels_path("word/document.xml"), "word/_rels/document.xml.rels");
        assert_eq!(rels_path(""), "_rels/.rels");
    }

    #[test]
    fn hyperlink_fields() {
        assert_eq!(hyperlink_target(r#"HYPERLINK "http://x.com/" \o "tip""#).as_deref(), Some("http://x.com/"));
        assert_eq!(hyperlink_target(r#" HYPERLINK \l "_Toc123" \h "#).as_deref(), Some("#_Toc123"));
        assert_eq!(field_from_instr(" PAGE \\* MERGEFORMAT ", "3"), Field::PageNumber);
        assert_eq!(field_from_instr(r#"DATE \@ "d MMMM yyyy""#, ""), Field::Date("d MMMM yyyy".into()));
    }
}
