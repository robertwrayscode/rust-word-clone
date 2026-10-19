//! PDF export (also used for printing: the app exports a PDF and hands it to the OS).
//!
//! The PDF is written by hand (PDF 1.7). Fonts are embedded as CID-keyed Type0
//! fonts (Identity-H, glyph ids as character codes) with a subsetted TrueType
//! program, a `/W` widths array and a `/ToUnicode` CMap so the text can be
//! selected and searched. Content streams, fonts and decoded images are
//! Flate-compressed; JPEGs are passed through unchanged.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::sync::Arc;

use crate::fonts::{FaceId, FontDb};
use crate::layout::{DocLayout, LineStyle, Page, Shape};
use crate::model::{Color, DocProperties, ImageData};

/// Export every page of `layout`.
pub fn export(layout: &DocLayout, db: &FontDb, props: &DocProperties) -> Vec<u8> {
    let all: Vec<usize> = (0..layout.pages.len()).collect();
    export_pages(layout, db, props, &all)
}

/// Export the given pages (0-based indexes into `layout.pages`, in the order given).
/// Out-of-range indexes are ignored; an empty selection produces one blank page.
pub fn export_pages(layout: &DocLayout, db: &FontDb, props: &DocProperties, pages: &[usize]) -> Vec<u8> {
    let pages: Vec<&Page> = pages.iter().filter_map(|&i| layout.pages.get(i)).collect();
    Exporter { db, w: Writer::default(), fonts: Vec::new(), font_of_face: HashMap::new(), images: Vec::new(), image_of: HashMap::new() }
        .run(&pages, props)
}

// ---------------------------------------------------------------------------
// Low-level object writer
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Writer {
    objs: Vec<Option<Vec<u8>>>,
}

impl Writer {
    /// Reserve an object number (1-based).
    fn alloc(&mut self) -> usize {
        self.objs.push(None);
        self.objs.len()
    }

    fn set(&mut self, id: usize, body: impl Into<Vec<u8>>) {
        self.objs[id - 1] = Some(body.into());
    }

    fn add(&mut self, body: impl Into<Vec<u8>>) -> usize {
        let id = self.alloc();
        self.set(id, body);
        id
    }

    /// A stream object; `dict` holds the extra dictionary entries (without << >>).
    fn add_stream(&mut self, dict: &str, data: &[u8], compress: bool) -> usize {
        let id = self.alloc();
        self.set_stream(id, dict, data, compress);
        id
    }

    fn set_stream(&mut self, id: usize, dict: &str, data: &[u8], compress: bool) {
        let packed;
        let (bytes, filter) = if compress {
            packed = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
            (&packed[..], " /Filter /FlateDecode")
        } else {
            (data, "")
        };
        let mut body = format!("<< {dict}{filter} /Length {} >>\nstream\n", bytes.len()).into_bytes();
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\nendstream");
        self.set(id, body);
    }

    fn finish(self, root: usize, info: usize) -> Vec<u8> {
        let mut out: Vec<u8> = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::with_capacity(self.objs.len());
        for (i, o) in self.objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(o.as_deref().unwrap_or(b"null"));
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        let mut x = format!("xref\n0 {}\n0000000000 65535 f\r\n", self.objs.len() + 1);
        for off in &offsets {
            let _ = write!(x, "{off:010} 00000 n\r\n");
        }
        let id = file_id(&out);
        let _ = write!(
            x,
            "trailer\n<< /Size {} /Root {root} 0 R /Info {info} 0 R /ID [<{id}> <{id}>] >>\nstartxref\n{xref}\n%%EOF\n",
            self.objs.len() + 1
        );
        out.extend_from_slice(x.as_bytes());
        out
    }
}

/// A 128-bit content hash (two FNV-1a variants) as hex, for the trailer /ID.
fn file_id(data: &[u8]) -> String {
    let mut a: u64 = 0xcbf2_9ce4_8422_2325;
    let mut b: u64 = 0x8422_2325_cbf2_9ce4;
    for &x in data {
        a = (a ^ x as u64).wrapping_mul(0x0000_0100_0000_01B3);
        b = (b ^ x as u64).wrapping_mul(0x0000_0100_0000_01B3).rotate_left(5);
    }
    format!("{a:016X}{b:016X}")
}

fn hash64(data: &[u8]) -> u64 {
    let mut a: u64 = 0xcbf2_9ce4_8422_2325;
    for &x in data {
        a = (a ^ x as u64).wrapping_mul(0x0000_0100_0000_01B3);
    }
    a
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/// Append a number with at most 3 decimals and no trailing zeros.
fn num(out: &mut String, v: f32) {
    let v = if v.is_finite() { v as f64 } else { 0.0 };
    let r = (v * 1000.0).round() / 1000.0;
    if r == r.trunc() {
        let _ = write!(out, "{}", r as i64);
    } else {
        let s = format!("{r:.3}");
        out.push_str(s.trim_end_matches('0').trim_end_matches('.'));
    }
}

/// Space-separated numbers followed by a space.
fn nums(out: &mut String, vs: &[f32]) {
    for v in vs {
        num(out, *v);
        out.push(' ');
    }
}

fn color(out: &mut String, c: Color, op: &str) {
    nums(out, &[c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0]);
    out.push_str(op);
    out.push('\n');
}

/// A PDF text string: literal for printable ASCII, UTF-16BE with BOM otherwise.
fn text_string(s: &str) -> String {
    if s.chars().all(|c| (' '..='~').contains(&c)) {
        let mut out = String::from("(");
        for c in s.chars() {
            if matches!(c, '(' | ')' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push(')');
        out
    } else {
        let mut out = String::from("<FEFF");
        for u in s.encode_utf16() {
            let _ = write!(out, "{u:04X}");
        }
        out.push('>');
        out
    }
}

/// A 7-bit literal string for URIs (non-ASCII is percent-encoded as UTF-8).
fn uri_string(s: &str) -> String {
    let mut out = String::from("(");
    for b in s.bytes() {
        match b {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(b as char);
            }
            0x21..=0x7E => out.push(b as char),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out.push(')');
    out
}

/// A PDF name token from arbitrary text (drops delimiters and spaces).
fn name_token(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_graphic() && !"()<>[]{}/%#".contains(*c)).take(100).collect()
}

/// Convert an ISO 8601 timestamp to a PDF date; falls back to now.
fn pdf_date(iso: &str) -> String {
    parse_iso_date(iso).unwrap_or_else(now_date)
}

fn parse_iso_date(s: &str) -> Option<String> {
    let s = s.trim();
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| -> Option<&str> {
        let t = s.get(r)?;
        t.bytes().all(|c| c.is_ascii_digit()).then_some(t)
    };
    let (y, mo, d) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    if b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return None;
    }
    let mut out = format!("D:{y}{mo}{d}");
    if matches!(b.get(10), Some(b'T') | Some(b' ')) {
        let (h, mi) = (digits(11..13)?, digits(14..16)?);
        let sec = digits(17..19).unwrap_or("00");
        let _ = write!(out, "{h}{mi}{sec}");
        // Time zone after optional seconds / fraction.
        let mut i = if digits(17..19).is_some() { 19 } else { 16 };
        if b.get(i) == Some(&b'.') {
            i += 1;
            while b.get(i).is_some_and(|c| c.is_ascii_digit()) {
                i += 1;
            }
        }
        match b.get(i) {
            Some(b'Z') => out.push('Z'),
            Some(&c) if c == b'+' || c == b'-' => {
                let th = digits(i + 1..i + 3)?;
                let tm = digits(i + 4..i + 6).or_else(|| digits(i + 3..i + 5)).unwrap_or("00");
                let _ = write!(out, "{}{th}'{tm}'", c as char);
            }
            _ => {}
        }
    }
    Some(out)
}

fn now_date() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("D:{y:04}{m:02}{d:02}{:02}{:02}{:02}Z", rem / 3600, rem / 60 % 60, rem % 60)
}

// ---------------------------------------------------------------------------
// Exporter
// ---------------------------------------------------------------------------

/// One embedded font program (faces sharing a file share it).
struct FontUse {
    face: FaceId,
    /// glyph id → char it was used for (for ToUnicode).
    glyphs: BTreeMap<u16, char>,
    obj: usize,
}

struct ImageUse {
    image: Arc<ImageData>,
    /// XObject number, or None when the format can't be decoded (placeholder drawn).
    obj: Option<usize>,
}

struct Exporter<'a> {
    db: &'a FontDb,
    w: Writer,
    fonts: Vec<FontUse>,
    font_of_face: HashMap<FaceId, usize>,
    images: Vec<ImageUse>,
    image_of: HashMap<usize, usize>,
}

impl Exporter<'_> {
    fn run(mut self, pages: &[&Page], props: &DocProperties) -> Vec<u8> {
        let catalog = self.w.alloc();
        let pages_obj = self.w.alloc();
        let resources = self.w.alloc();

        // Collect fonts and images.
        for page in pages {
            for s in &page.shapes {
                match s {
                    Shape::Text { text, face, .. } | Shape::Watermark { text, face, .. } => self.use_text(*face, text),
                    Shape::Image { image, .. } => {
                        let key = Arc::as_ptr(image) as usize;
                        if !self.image_of.contains_key(&key) {
                            self.image_of.insert(key, self.images.len());
                            self.images.push(ImageUse { image: image.clone(), obj: None });
                        }
                    }
                    _ => {}
                }
            }
        }
        for i in 0..self.fonts.len() {
            self.fonts[i].obj = self.w.alloc();
        }

        // Pages.
        let mut kids = Vec::new();
        let blank;
        let blank_ref;
        let pages: &[&Page] = if pages.is_empty() {
            blank = Page {
                index: 0,
                number: 1,
                width: 612.0,
                height: 792.0,
                shapes: Vec::new(),
                lines: Vec::new(),
                body_top: 72.0,
                body_bottom: 720.0,
                comment_marks: Vec::new(),
            };
            blank_ref = &blank;
            std::slice::from_ref(&blank_ref)
        } else {
            pages
        };
        // Images are written before page contents so placeholders are known.
        for i in 0..self.images.len() {
            self.images[i].obj = self.write_image(&self.images[i].image.clone());
        }
        for page in pages {
            let (content, annots) = self.page_content(page);
            let cid = self.w.add_stream("", content.as_bytes(), true);
            let mut annot_ids = Vec::new();
            for a in annots {
                annot_ids.push(self.w.add(a));
            }
            let mut d = format!("<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 ");
            nums(&mut d, &[page.width, page.height]);
            let _ = write!(d, "] /Resources {resources} 0 R /Contents {cid} 0 R");
            if !annot_ids.is_empty() {
                d.push_str(" /Annots [");
                for a in annot_ids {
                    let _ = write!(d, "{a} 0 R ");
                }
                d.push(']');
            }
            d.push_str(" >>");
            kids.push(self.w.add(d));
        }
        let mut d = String::from("<< /Type /Pages /Kids [");
        for k in &kids {
            let _ = write!(d, "{k} 0 R ");
        }
        let _ = write!(d, "] /Count {} >>", kids.len());
        self.w.set(pages_obj, d);

        // Fonts.
        for i in 0..self.fonts.len() {
            self.write_font(i);
        }
        let mut r = String::from("<< /ProcSet [/PDF /Text /ImageB /ImageC]");
        if !self.fonts.is_empty() {
            r.push_str(" /Font <<");
            for (i, f) in self.fonts.iter().enumerate() {
                let _ = write!(r, " /F{} {} 0 R", i + 1, f.obj);
            }
            r.push_str(" >>");
        }
        if self.images.iter().any(|i| i.obj.is_some()) {
            r.push_str(" /XObject <<");
            for (i, im) in self.images.iter().enumerate() {
                if let Some(o) = im.obj {
                    let _ = write!(r, " /Im{} {o} 0 R", i + 1);
                }
            }
            r.push_str(" >>");
        }
        r.push_str(" >>");
        self.w.set(resources, r);

        let mut c = format!("<< /Type /Catalog /Pages {pages_obj} 0 R");
        if !props.title.trim().is_empty() {
            c.push_str(" /ViewerPreferences << /DisplayDocTitle true >>");
        }
        c.push_str(" >>");
        self.w.set(catalog, c);

        // Document information.
        let mut info = String::from("<<");
        for (k, v) in [("Title", &props.title), ("Author", &props.author), ("Subject", &props.subject), ("Keywords", &props.keywords)] {
            if !v.trim().is_empty() {
                let _ = write!(info, " /{k} {}", text_string(v));
            }
        }
        let created = pdf_date(&props.created);
        let modified = if props.modified.trim().is_empty() { created.clone() } else { pdf_date(&props.modified) };
        let _ = write!(
            info,
            " /Creator (RWord) /Producer (RWord {}) /CreationDate {} /ModDate {} >>",
            env!("CARGO_PKG_VERSION"),
            text_string(&created),
            text_string(&modified)
        );
        let info = self.w.add(info);
        self.w.finish(catalog, info)
    }

    fn use_text(&mut self, face: FaceId, text: &str) {
        if (face as usize) >= self.db.faces.len() {
            return;
        }
        let fi = match self.font_of_face.get(&face) {
            Some(i) => *i,
            None => {
                let f = self.db.face(face);
                let key = (Arc::as_ptr(&f.data) as usize, f.index);
                let existing = self.fonts.iter().position(|u| {
                    let g = self.db.face(u.face);
                    (Arc::as_ptr(&g.data) as usize, g.index) == key
                });
                let i = existing.unwrap_or_else(|| {
                    self.fonts.push(FontUse { face, glyphs: BTreeMap::new(), obj: 0 });
                    self.fonts.len() - 1
                });
                self.font_of_face.insert(face, i);
                i
            }
        };
        let f = self.db.face(face);
        for c in text.chars().filter(|c| !c.is_control()) {
            self.fonts[fi].glyphs.entry(f.glyph(c)).or_insert(c);
        }
    }

    /// Hex-encoded glyph ids for `text` in face `face`.
    fn encode(&self, face: FaceId, text: &str) -> String {
        let f = self.db.face(face);
        let mut s = String::with_capacity(text.len() * 4 + 2);
        s.push('<');
        for c in text.chars().filter(|c| !c.is_control()) {
            let _ = write!(s, "{:04X}", f.glyph(c));
        }
        s.push('>');
        s
    }

    fn text_width(&self, face: FaceId, text: &str, size: f32, spacing: f32) -> f32 {
        let f = self.db.face(face);
        text.chars().filter(|c| !c.is_control()).map(|c| f.advance(c) * size + spacing).sum()
    }

    fn page_content(&self, page: &Page) -> (String, Vec<String>) {
        let h = page.height;
        let mut out = String::new();
        let mut links: Vec<(String, [f32; 4])> = Vec::new();
        for s in &page.shapes {
            match s {
                Shape::Rect { x, y, w, h: rh, color: c } => {
                    if *w > 0.0 && *rh > 0.0 {
                        color(&mut out, *c, "rg");
                        nums(&mut out, &[*x, h - y - rh, *w, *rh]);
                        out.push_str("re f\n");
                    }
                }
                Shape::Line { x1, y1, x2, y2, width, color: c, style } => {
                    line(&mut out, *x1, h - y1, *x2, h - y2, *width, *c, *style);
                }
                Shape::Text { x, y, text, face, size, color: c, spacing, fake_bold, fake_italic, link } => {
                    if text.is_empty() || *size <= 0.0 || (*face as usize) >= self.db.faces.len() {
                        continue;
                    }
                    let fi = self.font_of_face[face];
                    out.push_str("BT\n");
                    let _ = write!(out, "/F{} ", fi + 1);
                    num(&mut out, *size);
                    out.push_str(" Tf\n");
                    color(&mut out, *c, "rg");
                    num(&mut out, *spacing);
                    out.push_str(" Tc\n");
                    if *fake_bold {
                        color(&mut out, *c, "RG");
                        num(&mut out, size * 0.025);
                        out.push_str(" w 2 Tr\n");
                    } else {
                        out.push_str("0 Tr\n");
                    }
                    let skew = if *fake_italic { 0.2 } else { 0.0 };
                    nums(&mut out, &[1.0, 0.0, skew, 1.0, *x, h - y]);
                    out.push_str("Tm\n");
                    out.push_str(&self.encode(*face, text));
                    out.push_str(" Tj\nET\n");
                    if let Some(url) = link {
                        let l = url.to_ascii_lowercase();
                        if l.starts_with("http://") || l.starts_with("https://") || l.starts_with("mailto:") {
                            let f = self.db.face(*face);
                            let wd = self.text_width(*face, text, *size, *spacing);
                            let r = [*x, h - (y + f.descent * size), x + wd, h - (y - f.ascent * size)];
                            match links.last_mut() {
                                Some((u, lr)) if u == url && (lr[1] - r[1]).abs() < 0.5 && r[0] >= lr[0] && r[0] - lr[2] <= *size => {
                                    lr[2] = r[2];
                                    lr[3] = lr[3].max(r[3]);
                                }
                                _ => links.push((url.clone(), r)),
                            }
                        }
                    }
                }
                Shape::Watermark { cx, cy, text, face, size, color: c, angle } => {
                    if text.is_empty() || *size <= 0.0 || (*face as usize) >= self.db.faces.len() {
                        continue;
                    }
                    let fi = self.font_of_face[face];
                    let f = self.db.face(*face);
                    let wd = self.text_width(*face, text, *size, 0.0);
                    let (sn, cs) = angle.to_radians().sin_cos();
                    // Local origin: left end of the baseline, text centred on (cx, cy).
                    let (lx, ly) = (-wd / 2.0, -(f.ascent - f.descent) * size / 2.0);
                    let tx = cx + lx * cs - ly * sn;
                    let ty = (h - cy) + lx * sn + ly * cs;
                    let _ = write!(out, "BT\n/F{} ", fi + 1);
                    num(&mut out, *size);
                    out.push_str(" Tf\n");
                    color(&mut out, *c, "rg");
                    out.push_str("0 Tc 0 Tr\n");
                    nums(&mut out, &[cs, sn, -sn, cs, tx, ty]);
                    out.push_str("Tm\n");
                    out.push_str(&self.encode(*face, text));
                    out.push_str(" Tj\nET\n");
                }
                Shape::Image { x, y, w, h: ih, image } => {
                    let idx = self.image_of[&(Arc::as_ptr(image) as usize)];
                    match self.images[idx].obj {
                        Some(_) => {
                            out.push_str("q\n");
                            nums(&mut out, &[*w, 0.0, 0.0, *ih, *x, h - y - ih]);
                            let _ = writeln!(out, "cm /Im{} Do\nQ", idx + 1);
                        }
                        None => {
                            // Undecodable image: light grey placeholder with a border.
                            out.push_str("0.85 g 0.6 G 0.5 w [] 0 d\n");
                            nums(&mut out, &[*x, h - y - ih, *w, *ih]);
                            out.push_str("re B\n");
                        }
                    }
                }
            }
        }
        let annots = links
            .into_iter()
            .map(|(url, r)| {
                let mut a = String::from("<< /Type /Annot /Subtype /Link /Rect [");
                nums(&mut a, &r);
                let _ = write!(a, "] /Border [0 0 0] /F 4 /A << /Type /Action /S /URI /URI {} >> >>", uri_string(&url));
                a
            })
            .collect();
        (out, annots)
    }

    // ---- fonts -------------------------------------------------------------

    fn write_font(&mut self, i: usize) {
        let fu = &self.fonts[i];
        let face = self.db.face(fu.face);
        let data: &[u8] = &face.data;
        let parsed = ttf_parser::Face::parse(data, face.index).ok();
        let upem = face.units_per_em.max(1.0);
        let scale = 1000.0 / upem;

        // Glyph widths from the font's hmtx.
        let widths: Vec<(u16, f32)> = fu
            .glyphs
            .keys()
            .map(|&g| {
                let adv = parsed.as_ref().and_then(|p| p.glyph_hor_advance(ttf_parser::GlyphId(g))).map(|a| a as f32);
                let adv = adv.unwrap_or_else(|| fu.glyphs.get(&g).map(|c| face.advance(*c) * upem).unwrap_or(0.0));
                (g, adv * scale)
            })
            .collect();

        let mut ps_name = String::new();
        let (mut bbox, mut italic_angle, mut cap, mut mono) = ([0.0f32, -200.0, 1000.0, 900.0], 0.0f32, face.ascent * 700.0, false);
        if let Some(p) = &parsed {
            for n in p.names() {
                if n.name_id == ttf_parser::name_id::POST_SCRIPT_NAME {
                    if let Some(s) = n.to_string() {
                        ps_name = name_token(&s);
                        if !ps_name.is_empty() {
                            break;
                        }
                    }
                }
            }
            let b = p.global_bounding_box();
            bbox = [b.x_min as f32 * scale, b.y_min as f32 * scale, b.x_max as f32 * scale, b.y_max as f32 * scale];
            italic_angle = p.italic_angle();
            if let Some(c) = p.capital_height() {
                cap = c as f32 * scale;
            }
            mono = p.is_monospaced();
        }
        if ps_name.is_empty() {
            ps_name = name_token(&face.family);
        }
        if ps_name.is_empty() {
            ps_name = format!("Font{}", i + 1);
        }

        let used: BTreeSet<u16> = fu.glyphs.keys().copied().collect();
        let program = font_program(data, face.index, &used);
        let (program, subset, cff) = match program {
            Some(p) => p,
            None => (data.to_vec(), false, false),
        };
        let base = if subset {
            let mut h = hash64(&program) ^ (i as u64);
            let tag: String = (0..6)
                .map(|_| {
                    let c = (b'A' + (h % 26) as u8) as char;
                    h /= 26;
                    c
                })
                .collect();
            format!("{tag}+{ps_name}")
        } else {
            ps_name
        };

        let file = if cff {
            self.w.add_stream("/Subtype /OpenType", &program, true)
        } else {
            self.w.add_stream(&format!("/Length1 {}", program.len()), &program, true)
        };
        let mut flags = 32u32;
        if mono {
            flags |= 1;
        }
        if face.italic || italic_angle != 0.0 {
            flags |= 64;
        }
        let mut fd = format!("<< /Type /FontDescriptor /FontName /{base} /Flags {flags} /FontBBox [");
        nums(&mut fd, &bbox);
        fd.push_str("] /ItalicAngle ");
        num(&mut fd, italic_angle);
        fd.push_str(" /Ascent ");
        num(&mut fd, face.ascent * 1000.0);
        fd.push_str(" /Descent ");
        num(&mut fd, -face.descent * 1000.0);
        fd.push_str(" /CapHeight ");
        num(&mut fd, cap);
        let _ = write!(fd, " /StemV {} /{} {file} 0 R >>", if face.bold { 120 } else { 80 }, if cff { "FontFile3" } else { "FontFile2" });
        let fd = self.w.add(fd);

        // /W array: runs of consecutive glyph ids.
        let mut wa = String::from("[");
        let mut k = 0;
        while k < widths.len() {
            let start = widths[k].0;
            let _ = write!(wa, "{start} [");
            let mut j = k;
            while j < widths.len() && widths[j].0 as usize == start as usize + (j - k) {
                num(&mut wa, widths[j].1);
                wa.push(' ');
                j += 1;
            }
            wa.push_str("] ");
            k = j;
        }
        wa.push(']');

        let cid = self.w.add(format!(
            "<< /Type /Font /Subtype /{} /BaseFont /{base} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {fd} 0 R /DW 1000 /W {wa}{} >>",
            if cff { "CIDFontType0" } else { "CIDFontType2" },
            if cff { "" } else { " /CIDToGIDMap /Identity" },
        ));
        let tu = self.w.add_stream("", to_unicode(&self.fonts[i].glyphs).as_bytes(), true);
        let obj = self.fonts[i].obj;
        self.w.set(
            obj,
            format!("<< /Type /Font /Subtype /Type0 /BaseFont /{base} /Encoding /Identity-H /DescendantFonts [{cid} 0 R] /ToUnicode {tu} 0 R >>"),
        );
    }

    // ---- images ------------------------------------------------------------

    fn write_image(&mut self, img: &ImageData) -> Option<usize> {
        let b = &img.bytes;
        if b.starts_with(&[0xFF, 0xD8]) {
            let (w, h, comps, adobe) = jpeg_info(b)?;
            let cs = match comps {
                1 => "/DeviceGray",
                4 => "/DeviceCMYK",
                _ => "/DeviceRGB",
            };
            let decode = if comps == 4 && adobe { " /Decode [1 0 1 0 1 0 1 0]" } else { "" };
            let mut body = format!(
                "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace {cs} /BitsPerComponent 8{decode} /Interpolate true /Filter /DCTDecode /Length {} >>\nstream\n",
                b.len()
            )
            .into_bytes();
            body.extend_from_slice(b);
            body.extend_from_slice(b"\nendstream");
            return Some(self.w.add(body));
        }
        let r = if b.starts_with(b"\x89PNG\r\n\x1a\n") {
            decode_png(b)?
        } else if b.starts_with(b"BM") {
            decode_bmp(b)?
        } else if b.starts_with(b"GIF8") {
            decode_gif(b)?
        } else {
            return None;
        };
        let smask = r.alpha.as_ref().map(|a| {
            self.w.add_stream(
                &format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8", r.w, r.h),
                a,
                true,
            )
        });
        let mut d = format!(
            "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {} /BitsPerComponent 8 /Interpolate true",
            r.w,
            r.h,
            if r.gray { "/DeviceGray" } else { "/DeviceRGB" }
        );
        if let Some(s) = smask {
            let _ = write!(d, " /SMask {s} 0 R");
        }
        Some(self.w.add_stream(&d, &r.data, true))
    }
}

/// Draw a line in PDF coordinates.
#[allow(clippy::too_many_arguments)]
fn line(out: &mut String, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, c: Color, style: LineStyle) {
    let width = width.max(0.1);
    let (dx, dy) = (x2 - x1, y2 - y1);
    let len = (dx * dx + dy * dy).sqrt();
    if len <= 0.0 {
        return;
    }
    color(out, c, "RG");
    match style {
        LineStyle::Solid | LineStyle::Dotted | LineStyle::Dashed => {
            num(out, width);
            match style {
                LineStyle::Dotted => {
                    let d = width.max(1.0);
                    out.push_str(" w 0 J [");
                    nums(out, &[d, d]);
                    out.push_str("] 0 d\n");
                }
                LineStyle::Dashed => {
                    out.push_str(" w 0 J [");
                    nums(out, &[(width * 3.0).max(3.0), (width * 2.0).max(2.0)]);
                    out.push_str("] 0 d\n");
                }
                _ => out.push_str(" w 0 J [] 0 d\n"),
            }
            nums(out, &[x1, y1]);
            out.push_str("m ");
            nums(out, &[x2, y2]);
            out.push_str("l S\n");
        }
        LineStyle::Double => {
            let lw = (width * 0.6).max(0.25);
            let off = (width * 0.9).max(lw * 1.5);
            let (nx, ny) = (-dy / len * off, dx / len * off);
            num(out, lw);
            out.push_str(" w 0 J [] 0 d\n");
            for s in [-1.0, 1.0] {
                nums(out, &[x1 + nx * s, y1 + ny * s]);
                out.push_str("m ");
                nums(out, &[x2 + nx * s, y2 + ny * s]);
                out.push_str("l ");
            }
            out.push_str("S\n");
        }
        LineStyle::Wavy => {
            // Draw in a local frame along the line: x along, y across.
            let amp = (width * 1.2).max(0.8);
            let half = amp * 2.0;
            let (ux, uy) = (dx / len, dy / len);
            out.push_str("q\n");
            nums(out, &[ux, uy, -uy, ux, x1, y1]);
            out.push_str("cm ");
            num(out, width);
            out.push_str(" w 1 J 1 j [] 0 d\n0 0 m\n");
            let mut x = 0.0;
            let mut up = true;
            while x < len - 1e-3 {
                let step = half.min(len - x);
                let a = if up { amp } else { -amp } * (step / half);
                nums(out, &[x + step * 0.35, a * 1.33, x + step * 0.65, a * 1.33, x + step, 0.0]);
                out.push_str("c\n");
                x += step;
                up = !up;
            }
            out.push_str("S\nQ\n");
        }
    }
}

fn to_unicode(glyphs: &BTreeMap<u16, char>) -> String {
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries: Vec<(&u16, &char)> = glyphs.iter().collect();
    for chunk in entries.chunks(100) {
        let _ = writeln!(s, "{} beginbfchar", chunk.len());
        for (g, c) in chunk {
            let _ = write!(s, "<{g:04X}> <");
            let mut buf = [0u16; 2];
            for u in c.encode_utf16(&mut buf) {
                let _ = write!(s, "{u:04X}");
            }
            s.push_str(">\n");
        }
        s.push_str("endbfchar\n");
    }
    s.push_str("endcmap\nCMapName currentdict /CMapResource defineresource pop\nend\nend\n");
    s
}

// ---------------------------------------------------------------------------
// sfnt handling: collection extraction and TrueType subsetting
// ---------------------------------------------------------------------------

fn be16(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(o)?, *d.get(o + 1)?]))
}

fn be32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*d.get(o)?, *d.get(o + 1)?, *d.get(o + 2)?, *d.get(o + 3)?]))
}

type Tables = Vec<([u8; 4], Vec<u8>)>;

/// The tables of face `index` of an sfnt / collection, with the sfnt version.
fn sfnt_tables(data: &[u8], index: u32) -> Option<(u32, Tables)> {
    let mut off = 0usize;
    if data.get(0..4)? == b"ttcf" {
        let n = be32(data, 8)?;
        if index >= n {
            return None;
        }
        off = be32(data, 12 + 4 * index as usize)? as usize;
    }
    let version = be32(data, off)?;
    let num = be16(data, off + 4)? as usize;
    let mut tables = Vec::with_capacity(num);
    for i in 0..num {
        let rec = off + 12 + 16 * i;
        let tag: [u8; 4] = data.get(rec..rec + 4)?.try_into().ok()?;
        let start = be32(data, rec + 8)? as usize;
        let len = be32(data, rec + 12)? as usize;
        tables.push((tag, data.get(start..start.checked_add(len)?)?.to_vec()));
    }
    Some((version, tables))
}

fn checksum(d: &[u8]) -> u32 {
    let mut sum = 0u32;
    for ch in d.chunks(4) {
        let mut b = [0u8; 4];
        b[..ch.len()].copy_from_slice(ch);
        sum = sum.wrapping_add(u32::from_be_bytes(b));
    }
    sum
}

/// Serialize tables into a standalone sfnt with correct checksums.
fn build_sfnt(version: u32, mut tables: Tables) -> Vec<u8> {
    tables.sort_by_key(|t| t.0);
    for (tag, d) in tables.iter_mut() {
        if tag == b"head" && d.len() >= 12 {
            d[8..12].copy_from_slice(&[0; 4]);
        }
    }
    let n = tables.len() as u16;
    let mut pow = 1u16;
    let mut log = 0u16;
    while pow * 2 <= n {
        pow *= 2;
        log += 1;
    }
    let mut out = Vec::new();
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&(pow * 16).to_be_bytes());
    out.extend_from_slice(&log.to_be_bytes());
    out.extend_from_slice(&(n * 16 - pow * 16).to_be_bytes());
    let mut offset = 12 + 16 * tables.len();
    for (tag, d) in &tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum(d).to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(d.len() as u32).to_be_bytes());
        offset += (d.len() + 3) & !3;
    }
    let mut head_at = None;
    for (tag, d) in &tables {
        if tag == b"head" {
            head_at = Some(out.len());
        }
        out.extend_from_slice(d);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    if let Some(h) = head_at {
        if out.len() >= h + 12 {
            let adj = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
            out[h + 8..h + 12].copy_from_slice(&adj.to_be_bytes());
        }
    }
    out
}

/// The font program to embed: (bytes, subsetted, is CFF/OpenType).
/// Returns None when the data can't be parsed (embed the raw file then).
fn font_program(data: &[u8], index: u32, used: &BTreeSet<u16>) -> Option<(Vec<u8>, bool, bool)> {
    let (version, tables) = sfnt_tables(data, index)?;
    if version == u32::from_be_bytes(*b"OTTO") {
        let keep: Tables = tables.into_iter().filter(|(t, _)| !matches!(t, b"DSIG")).collect();
        return Some((build_sfnt(version, keep), false, true));
    }
    match subset_truetype(&tables, used) {
        Some(t) => Some((build_sfnt(0x0001_0000, t), true, false)),
        None => {
            let keep: Tables = tables.into_iter().filter(|(t, _)| !matches!(t, b"DSIG")).collect();
            Some((build_sfnt(version, keep), false, false))
        }
    }
}

/// Keep only the outlines of `used` glyphs (plus composite components and
/// .notdef); glyph ids stay unchanged, unused glyphs become empty.
fn subset_truetype(tables: &Tables, used: &BTreeSet<u16>) -> Option<Tables> {
    let get = |tag: &[u8; 4]| tables.iter().find(|(t, _)| t == tag).map(|(_, d)| d.as_slice());
    let head = get(b"head")?;
    let maxp = get(b"maxp")?;
    let loca = get(b"loca")?;
    let glyf = get(b"glyf")?;
    get(b"hhea")?;
    get(b"hmtx")?;
    let long = be16(head, 50)? != 0;
    let n = be16(maxp, 4)? as usize;
    let offset = |g: usize| -> Option<usize> {
        if long {
            be32(loca, g * 4).map(|v| v as usize)
        } else {
            be16(loca, g * 2).map(|v| v as usize * 2)
        }
    };
    let glyph = |g: usize| -> Option<&[u8]> {
        let (a, b) = (offset(g)?, offset(g + 1)?);
        if a >= b {
            return Some(&[]);
        }
        glyf.get(a..b)
    };
    // Closure over composite components.
    let mut keep = vec![false; n];
    let mut stack: Vec<usize> = std::iter::once(0).chain(used.iter().map(|g| *g as usize)).filter(|g| *g < n).collect();
    while let Some(g) = stack.pop() {
        if keep[g] {
            continue;
        }
        keep[g] = true;
        let d = glyph(g)?;
        if d.len() >= 10 && (be16(d, 0)? as i16) < 0 {
            let mut p = 10;
            loop {
                let flags = be16(d, p)?;
                let comp = be16(d, p + 2)? as usize;
                if comp < n && !keep[comp] {
                    stack.push(comp);
                }
                p += 4 + if flags & 1 != 0 { 4 } else { 2 };
                p += if flags & 8 != 0 {
                    2
                } else if flags & 0x40 != 0 {
                    4
                } else if flags & 0x80 != 0 {
                    8
                } else {
                    0
                };
                if flags & 0x20 == 0 {
                    break;
                }
            }
        }
    }
    let mut new_glyf = Vec::new();
    let mut new_loca = Vec::with_capacity((n + 1) * 4);
    for (g, k) in keep.iter().enumerate() {
        new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
        if *k {
            new_glyf.extend_from_slice(glyph(g)?);
            while new_glyf.len() % 4 != 0 {
                new_glyf.push(0);
            }
        }
    }
    new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
    let mut new_head = head.to_vec();
    new_head.get_mut(50..52)?.copy_from_slice(&1u16.to_be_bytes());

    let mut out: Tables = vec![(*b"head", new_head), (*b"loca", new_loca), (*b"glyf", new_glyf)];
    // Hinting programs are kept: glyph instructions may call into fpgm.
    for tag in [b"hhea", b"maxp", b"hmtx", b"cvt ", b"fpgm", b"prep", b"OS/2"] {
        if let Some(d) = get(tag) {
            out.push((*tag, d.to_vec()));
        }
    }
    if let Some(post) = get(b"post") {
        if post.len() >= 32 {
            let mut p = post[..32].to_vec();
            p[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
            out.push((*b"post", p));
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Image decoding
// ---------------------------------------------------------------------------

/// Decoded 8-bit raster: 1 (gray) or 3 (RGB) channels, plus optional alpha.
struct Raster {
    w: u32,
    h: u32,
    gray: bool,
    data: Vec<u8>,
    alpha: Option<Vec<u8>>,
}

impl Raster {
    fn finish(mut self) -> Raster {
        if self.alpha.as_ref().is_some_and(|a| a.iter().all(|v| *v == 255)) {
            self.alpha = None;
        }
        self
    }
}

const MAX_PIXELS: usize = 100_000_000;

fn pixel_count(w: u32, h: u32) -> Option<usize> {
    let n = (w as usize).checked_mul(h as usize)?;
    (n > 0 && n <= MAX_PIXELS).then_some(n)
}

/// (width, height, components, Adobe APP14 present) of a JPEG.
fn jpeg_info(d: &[u8]) -> Option<(u16, u16, u8, bool)> {
    let mut i = 2;
    let mut adobe = false;
    while i + 4 <= d.len() {
        if d[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = d[i + 1];
        if m == 0xFF {
            i += 1;
            continue;
        }
        if (0xD0..=0xD9).contains(&m) || m == 0x01 {
            i += 2;
            continue;
        }
        let len = be16(d, i + 2)? as usize;
        if m == 0xEE && d.get(i + 4..i + 9) == Some(b"Adobe") {
            adobe = true;
        }
        if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
            let h = be16(d, i + 5)?;
            let w = be16(d, i + 7)?;
            let c = *d.get(i + 9)?;
            return (w > 0 && h > 0 && matches!(c, 1 | 3 | 4)).then_some((w, h, c, adobe));
        }
        i += 2 + len;
    }
    None
}

fn decode_png(d: &[u8]) -> Option<Raster> {
    let mut p = 8;
    let (mut w, mut h, mut depth, mut ctype, mut interlace) = (0u32, 0u32, 0u8, 0u8, 0u8);
    let mut palette: &[u8] = &[];
    let mut trns: &[u8] = &[];
    let mut idat = Vec::new();
    while p + 8 <= d.len() {
        let len = be32(d, p)? as usize;
        let ty = d.get(p + 4..p + 8)?;
        let body = d.get(p + 8..p + 8 + len)?;
        match ty {
            b"IHDR" => {
                w = be32(body, 0)?;
                h = be32(body, 4)?;
                depth = *body.get(8)?;
                ctype = *body.get(9)?;
                interlace = *body.get(12)?;
            }
            b"PLTE" => palette = body,
            b"tRNS" => trns = body,
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        p += 12 + len;
    }
    let npix = pixel_count(w, h)?;
    let spp: usize = match (ctype, depth) {
        (0, 1 | 2 | 4 | 8 | 16) => 1,
        (2, 8 | 16) => 3,
        (3, 1 | 2 | 4 | 8) => 1,
        (4, 8 | 16) => 2,
        (6, 8 | 16) => 4,
        _ => return None,
    };
    let depth = depth as usize;
    let (w, h) = (w as usize, h as usize);
    let bits = spp * depth;
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&idat).ok()?;
    let gray = matches!(ctype, 0 | 4);
    let chans = if gray { 1 } else { 3 };
    let mut data = vec![0u8; npix * chans];
    let mut alpha = vec![255u8; npix];
    // Passes: (x0, y0, dx, dy); a non-interlaced image is one full pass.
    let passes: &[(usize, usize, usize, usize)] = match interlace {
        0 => &[(0, 0, 1, 1)],
        1 => &[(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)],
        _ => return None,
    };
    let to8 = |v: u16| -> u8 {
        match depth {
            16 => (v >> 8) as u8,
            8 => v as u8,
            _ => (v as u32 * 255 / ((1u32 << depth) - 1)) as u8,
        }
    };
    let key = |i: usize| be16(trns, i * 2);
    let mut pos = 0;
    for &(x0, y0, dx, dy) in passes {
        let pw = if w > x0 { (w - x0).div_ceil(dx) } else { 0 };
        let ph = if h > y0 { (h - y0).div_ceil(dy) } else { 0 };
        if pw == 0 || ph == 0 {
            continue;
        }
        let stride = (pw * bits).div_ceil(8);
        let img = png_unfilter(raw.get(pos..pos + ph * (stride + 1))?, stride, bits.div_ceil(8).max(1))?;
        pos += ph * (stride + 1);
        for py in 0..ph {
            let row = &img[py * stride..(py + 1) * stride];
            let sample = |i: usize| -> u16 {
                match depth {
                    8 => row[i] as u16,
                    16 => u16::from_be_bytes([row[2 * i], row[2 * i + 1]]),
                    _ => {
                        let bit = i * depth;
                        ((row[bit / 8] >> (8 - depth - bit % 8)) & ((1u8 << depth) - 1)) as u16
                    }
                }
            };
            for px in 0..pw {
                let o = (y0 + py * dy) * w + x0 + px * dx;
                let out = &mut data[o * chans..(o + 1) * chans];
                match ctype {
                    0 => {
                        let v = sample(px);
                        out[0] = to8(v);
                        if key(0) == Some(v) {
                            alpha[o] = 0;
                        }
                    }
                    2 => {
                        let (r, g, b) = (sample(3 * px), sample(3 * px + 1), sample(3 * px + 2));
                        out.copy_from_slice(&[to8(r), to8(g), to8(b)]);
                        if key(0) == Some(r) && key(1) == Some(g) && key(2) == Some(b) {
                            alpha[o] = 0;
                        }
                    }
                    3 => {
                        let i = sample(px) as usize;
                        out.copy_from_slice(palette.get(i * 3..i * 3 + 3).unwrap_or(&[0, 0, 0]));
                        alpha[o] = trns.get(i).copied().unwrap_or(255);
                    }
                    4 => {
                        out[0] = to8(sample(2 * px));
                        alpha[o] = to8(sample(2 * px + 1));
                    }
                    _ => {
                        for (k, v) in out.iter_mut().enumerate() {
                            *v = to8(sample(4 * px + k));
                        }
                        alpha[o] = to8(sample(4 * px + 3));
                    }
                }
            }
        }
    }
    Some(Raster { w: w as u32, h: h as u32, gray, data, alpha: Some(alpha) }.finish())
}

/// Undo PNG row filters; `raw` holds rows of (filter byte + `stride` bytes).
fn png_unfilter(raw: &[u8], stride: usize, bpp: usize) -> Option<Vec<u8>> {
    let h = raw.len() / (stride + 1);
    let mut img = vec![0u8; h * stride];
    for y in 0..h {
        let ft = raw[y * (stride + 1)];
        let src = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        for x in 0..stride {
            let a = if x >= bpp { img[y * stride + x - bpp] } else { 0 };
            let b = if y > 0 { img[(y - 1) * stride + x] } else { 0 };
            let c = if y > 0 && x >= bpp { img[(y - 1) * stride + x - bpp] } else { 0 };
            let pred = match ft {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((a as u16 + b as u16) / 2) as u8,
                4 => {
                    let pa = (b as i16 - c as i16).abs();
                    let pb = (a as i16 - c as i16).abs();
                    let pc = (a as i16 + b as i16 - 2 * c as i16).abs();
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => return None,
            };
            img[y * stride + x] = src[x].wrapping_add(pred);
        }
    }
    Some(img)
}

fn decode_bmp(d: &[u8]) -> Option<Raster> {
    let le16 = |o: usize| Some(u16::from_le_bytes([*d.get(o)?, *d.get(o + 1)?]));
    let le32 = |o: usize| Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?));
    let data_off = le32(10)? as usize;
    let dib = le32(14)? as usize;
    if dib < 40 {
        return None;
    }
    let w = le32(18)? as i32;
    let hraw = le32(22)? as i32;
    let bpp = le16(28)?;
    let comp = le32(30)?;
    let (w, top_down) = (w.unsigned_abs(), hraw < 0);
    let h = hraw.unsigned_abs();
    let npix = pixel_count(w, h)?;
    if !(comp == 0 || (comp == 3 && bpp == 32)) {
        return None;
    }
    let stride = ((w as usize * bpp as usize).div_ceil(32)) * 4;
    let palette: Vec<[u8; 3]> = if bpp <= 8 {
        let n = match le32(46)? {
            0 => 1usize << bpp,
            n => n as usize,
        };
        (0..n.min(256)).map(|i| d.get(14 + dib + i * 4..14 + dib + i * 4 + 3).map(|c| [c[2], c[1], c[0]])).collect::<Option<_>>()?
    } else {
        Vec::new()
    };
    let mut data = Vec::with_capacity(npix * 3);
    for y in 0..h as usize {
        let sy = if top_down { y } else { h as usize - 1 - y };
        let row = d.get(data_off + sy * stride..data_off + (sy + 1) * stride)?;
        for x in 0..w as usize {
            match bpp {
                24 | 32 => {
                    let p = x * (bpp as usize / 8);
                    data.extend_from_slice(&[row[p + 2], row[p + 1], row[p]]);
                }
                1 | 4 | 8 => {
                    let bit = x * bpp as usize;
                    let i = (row[bit / 8] >> (8 - bpp as usize - bit % 8)) & ((1u16 << bpp) - 1) as u8;
                    data.extend_from_slice(palette.get(i as usize).unwrap_or(&[0, 0, 0]));
                }
                _ => return None,
            }
        }
    }
    Some(Raster { w, h, gray: false, data, alpha: None })
}

/// First frame of a GIF.
fn decode_gif(d: &[u8]) -> Option<Raster> {
    let le16 = |o: usize| Some(u16::from_le_bytes([*d.get(o)?, *d.get(o + 1)?]));
    let (sw, sh) = (le16(6)? as usize, le16(8)? as usize);
    let flags = *d.get(10)?;
    let mut p = 13;
    let mut global: &[u8] = &[];
    if flags & 0x80 != 0 {
        let n = 3 << ((flags & 7) + 1);
        global = d.get(p..p + n)?;
        p += n;
    }
    let mut transparent: Option<u8> = None;
    loop {
        match *d.get(p)? {
            0x21 => {
                let label = *d.get(p + 1)?;
                p += 2;
                if label == 0xF9 && *d.get(p)? >= 4 && d.get(p + 1)? & 1 != 0 {
                    transparent = Some(*d.get(p + 4)?);
                }
                loop {
                    let n = *d.get(p)? as usize;
                    p += 1 + n;
                    if n == 0 {
                        break;
                    }
                }
            }
            0x2C => break,
            _ => return None,
        }
    }
    let (fx, fy, fw, fh) = (le16(p + 1)? as usize, le16(p + 3)? as usize, le16(p + 5)? as usize, le16(p + 7)? as usize);
    let iflags = *d.get(p + 9)?;
    p += 10;
    let mut table = global;
    if iflags & 0x80 != 0 {
        let n = 3 << ((iflags & 7) + 1);
        table = d.get(p..p + n)?;
        p += n;
    }
    let (sw, sh) = if sw == 0 || sh == 0 { (fx + fw, fy + fh) } else { (sw, sh) };
    let npix = pixel_count(sw as u32, sh as u32)?;
    let min = *d.get(p)?;
    p += 1;
    let mut lzw = Vec::new();
    loop {
        let n = *d.get(p)? as usize;
        if n == 0 {
            break;
        }
        lzw.extend_from_slice(d.get(p + 1..p + 1 + n)?);
        p += 1 + n;
    }
    let idx = lzw_decode(&lzw, min, fw * fh)?;
    // Interlaced row order.
    let rows: Vec<usize> = if iflags & 0x40 != 0 {
        [(0, 8), (4, 8), (2, 4), (1, 2)].iter().flat_map(|&(s, st)| (s..fh).step_by(st)).collect()
    } else {
        (0..fh).collect()
    };
    let mut data = vec![255u8; npix * 3];
    let mut alpha = vec![0u8; npix];
    for (k, &ry) in rows.iter().enumerate() {
        for x in 0..fw {
            let Some(&i) = idx.get(k * fw + x) else { break };
            let (tx, ty) = (fx + x, fy + ry);
            if tx >= sw || ty >= sh || Some(i) == transparent {
                continue;
            }
            let o = ty * sw + tx;
            if let Some(c) = table.get(i as usize * 3..i as usize * 3 + 3) {
                data[o * 3..o * 3 + 3].copy_from_slice(c);
                alpha[o] = 255;
            }
        }
    }
    Some(Raster { w: sw as u32, h: sh as u32, gray: false, data, alpha: Some(alpha) }.finish())
}

fn lzw_decode(data: &[u8], min: u8, max_out: usize) -> Option<Vec<u8>> {
    if !(1..=11).contains(&min) {
        return None;
    }
    let clear = 1usize << min;
    let end = clear + 1;
    let mut prefix = vec![0u16; 4096];
    let mut suffix = vec![0u8; 4096];
    let mut first = vec![0u8; 4096];
    for i in 0..clear {
        suffix[i] = i as u8;
        first[i] = i as u8;
    }
    let mut size = min as u32 + 1;
    let mut next = clear + 2;
    let mut prev: Option<usize> = None;
    let mut out = Vec::with_capacity(max_out);
    let mut stack = Vec::new();
    let (mut acc, mut nbits) = (0u32, 0u32);
    for &byte in data {
        acc |= (byte as u32) << nbits;
        nbits += 8;
        while nbits >= size {
            let code = (acc & ((1 << size) - 1)) as usize;
            acc >>= size;
            nbits -= size;
            if code == clear {
                size = min as u32 + 1;
                next = clear + 2;
                prev = None;
                continue;
            }
            if code == end || out.len() >= max_out {
                return Some(out);
            }
            if let Some(p) = prev {
                if code > next {
                    return Some(out);
                }
                let k = if code < next { first[code] } else { first[p] };
                if next < 4096 {
                    prefix[next] = p as u16;
                    suffix[next] = k;
                    first[next] = first[p];
                    next += 1;
                    if next == (1 << size) && size < 12 {
                        size += 1;
                    }
                }
            } else if code >= clear {
                return Some(out);
            }
            stack.clear();
            let mut c = code;
            loop {
                stack.push(suffix[c]);
                if c < clear {
                    break;
                }
                c = prefix[c] as usize;
            }
            out.extend(stack.iter().rev());
            prev = Some(code);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_strings() {
        assert_eq!(parse_iso_date("2024-03-05T10:20:30Z").unwrap(), "D:20240305102030Z");
        assert_eq!(parse_iso_date("2024-03-05T10:20:30.123+02:00").unwrap(), "D:20240305102030+02'00'");
        assert_eq!(parse_iso_date("2024-03-05").unwrap(), "D:20240305");
        assert!(parse_iso_date("garbage").is_none());
        assert!(now_date().starts_with("D:20"));
        assert_eq!(text_string("a(b)\\"), "(a\\(b\\)\\\\)");
        assert_eq!(text_string("é"), "<FEFF00E9>");
        let mut s = String::new();
        num(&mut s, 1.5);
        s.push(' ');
        num(&mut s, 2.0);
        s.push(' ');
        num(&mut s, -0.0004);
        assert_eq!(s, "1.5 2 0");
    }

    fn outline_len(face: &ttf_parser::Face, g: u16) -> usize {
        struct Count(usize);
        impl ttf_parser::OutlineBuilder for Count {
            fn move_to(&mut self, _: f32, _: f32) {
                self.0 += 1;
            }
            fn line_to(&mut self, _: f32, _: f32) {
                self.0 += 1;
            }
            fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
                self.0 += 1;
            }
            fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
                self.0 += 1;
            }
            fn close(&mut self) {}
        }
        let mut c = Count(0);
        face.outline_glyph(ttf_parser::GlyphId(g), &mut c);
        c.0
    }

    fn check_subset(data: &[u8], index: u32) {
        let orig = ttf_parser::Face::parse(data, index).unwrap();
        let a = orig.glyph_index('A').unwrap().0;
        let b = orig.glyph_index('B').unwrap().0;
        let used: BTreeSet<u16> = [a].into_iter().collect();
        let (prog, subset, cff) = font_program(data, index, &used).unwrap();
        assert!(subset && !cff);
        let sub = ttf_parser::Face::parse(&prog, 0).unwrap();
        assert_eq!(sub.number_of_glyphs(), orig.number_of_glyphs());
        assert_eq!(outline_len(&sub, a), outline_len(&orig, a));
        assert!(outline_len(&orig, b) > 0);
        assert_eq!(outline_len(&sub, b), 0);
        assert_eq!(sub.glyph_hor_advance(ttf_parser::GlyphId(a)), orig.glyph_hor_advance(ttf_parser::GlyphId(a)));
        // Whole-file checksum must be 0xB1B0AFBA.
        assert_eq!(checksum(&prog), 0xB1B0_AFBA);
        assert!(prog.len() < data.len() / 2);
    }

    #[test]
    fn subset_truetype_font() {
        let data = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/Carlito-Regular.ttf"));
        check_subset(data, 0);
    }

    #[test]
    fn subset_collection_face() {
        // Uses a system .ttc when one is installed.
        for path in ["/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc", "C:\\Windows\\Fonts\\cambria.ttc", "/System/Library/Fonts/Helvetica.ttc"] {
            if let Ok(data) = std::fs::read(path) {
                let n = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
                check_subset(&data, n - 1);
                return;
            }
        }
    }

    #[test]
    fn opentype_cff_passthrough() {
        let dir = "/usr/share/fonts/opentype/inter";
        let Some(path) = std::fs::read_dir(dir).ok().and_then(|mut d| d.find_map(|e| e.ok().map(|e| e.path()))) else { return };
        let data = std::fs::read(path).unwrap();
        if !data.starts_with(b"OTTO") {
            return;
        }
        let (prog, subset, cff) = font_program(&data, 0, &BTreeSet::new()).unwrap();
        assert!(cff && !subset);
        let f = ttf_parser::Face::parse(&prog, 0).unwrap();
        assert!(f.tables().cff.is_some());
    }

    #[test]
    fn decode_bmp_and_gif() {
        // 2x2 24-bit bottom-up BMP: bottom row blue, red; top row green, white.
        let mut bmp = b"BM".to_vec();
        let px: [[u8; 3]; 4] = [[255, 0, 0], [0, 0, 255], [0, 255, 0], [255, 255, 255]]; // BGR
        let mut pix = Vec::new();
        for row in px.chunks(2) {
            for p in row {
                pix.extend_from_slice(p);
            }
            pix.extend_from_slice(&[0, 0]);
        }
        bmp.extend_from_slice(&((54 + pix.len()) as u32).to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&2i32.to_le_bytes());
        bmp.extend_from_slice(&2i32.to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        bmp.extend_from_slice(&pix);
        let r = decode_bmp(&bmp).unwrap();
        assert_eq!((r.w, r.h, r.gray), (2, 2, false));
        assert_eq!(r.data, vec![0, 255, 0, 255, 255, 255, 0, 0, 255, 255, 0, 0]);

        // 2x2 GIF, global palette [black, red], pixels 1 1 1 1, index 0 transparent.
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[2, 0, 2, 0, 0x80, 0, 0]);
        gif.extend_from_slice(&[0, 0, 0, 255, 0, 0]);
        gif.extend_from_slice(&[0x21, 0xF9, 4, 1, 0, 0, 0, 0]);
        gif.extend_from_slice(&[0x2C, 0, 0, 0, 0, 2, 0, 2, 0, 0]);
        // LZW (min size 2): clear(4) 1 6 1 end(5) with 3,3,3,3,4-bit codes.
        gif.extend_from_slice(&[2, 2, 0x8C, 0x53, 0, 0x3B]);
        let r = decode_gif(&gif).unwrap();
        assert_eq!((r.w, r.h), (2, 2));
        assert_eq!(r.data, vec![255, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0]);
        assert!(r.alpha.is_none());
    }

    #[test]
    fn lzw_roundtrip_small() {
        // "GIF" LZW stream for 4 pixels of index 1, min code size 2:
        // codes: clear(4) 1 6 1 end(5)  → 1, 1 1, 1
        let codes = [4u16, 1, 6, 1, 5];
        let sizes = [3u32, 3, 3, 3, 4];
        let (mut acc, mut n, mut bytes) = (0u32, 0u32, Vec::new());
        for (c, s) in codes.iter().zip(sizes) {
            acc |= (*c as u32) << n;
            n += s;
            while n >= 8 {
                bytes.push(acc as u8);
                acc >>= 8;
                n -= 8;
            }
        }
        if n > 0 {
            bytes.push(acc as u8);
        }
        assert_eq!(lzw_decode(&bytes, 2, 100).unwrap(), vec![1, 1, 1, 1]);
    }
}
