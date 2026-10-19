//! Font loading and metrics.
//!
//! Real system fonts are used when present (Calibri, Arial, Times New Roman …);
//! otherwise metric-compatible open fonts bundled into the binary are used
//! (Carlito, Liberation Sans/Serif/Mono, Caladea), so documents lay out the same
//! way on every machine. The layout engine measures with these fonts, and the
//! GUI and PDF export draw with the very same font files.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// Index of a loaded face in a [`FontDb`].
pub type FaceId = u16;

/// Metrics for one face, in font units scaled to 1.0 = em.
#[derive(Debug)]
pub struct Face {
    pub id: FaceId,
    /// Family name as we expose it (e.g. "Calibri").
    pub family: String,
    pub bold: bool,
    pub italic: bool,
    /// Whether the face is synthesized-bold/italic (the face file is the regular one).
    pub fake_bold: bool,
    pub fake_italic: bool,
    pub data: Arc<Vec<u8>>,
    /// Face index in a collection (.ttc).
    pub index: u32,
    /// Ascent (above baseline), em.
    pub ascent: f32,
    /// Descent (below baseline, positive), em.
    pub descent: f32,
    /// Extra line gap, em.
    pub line_gap: f32,
    pub underline_pos: f32,
    pub underline_thickness: f32,
    pub strike_pos: f32,
    pub x_height: f32,
    pub units_per_em: f32,
    advances: HashMap<char, f32>,
    glyphs: HashMap<char, u16>,
    default_advance: f32,
}

impl Face {
    fn load(id: FaceId, family: &str, data: Arc<Vec<u8>>, index: u32, bold: bool, italic: bool) -> Option<Face> {
        let data2 = data.clone();
        let f = ttf_parser::Face::parse(&data2, index).ok()?;
        let upem = f.units_per_em() as f32;
        let (asc, desc) = match f.tables().os2 {
            Some(os2) if os2.windows_ascender() > 0 => (os2.windows_ascender() as f32, os2.windows_descender().unsigned_abs() as f32),
            _ => (f.ascender() as f32, f.descender().unsigned_abs() as f32),
        };
        let gap = if f.tables().os2.map(|o| o.windows_ascender() > 0).unwrap_or(false) {
            // Word uses win metrics for line height; add hhea line gap only if win metrics are tight.
            let hhea_total = (f.ascender() as f32 - f.descender() as f32 + f.line_gap() as f32).max(0.0);
            (hhea_total - asc - desc).max(0.0)
        } else {
            f.line_gap().max(0) as f32
        };
        let ul = f.underline_metrics();
        let st = f.strikeout_metrics();
        let mut advances = HashMap::new();
        let mut glyphs = HashMap::new();
        if let Some(cmap) = f.tables().cmap {
            for sub in cmap.subtables {
                if !sub.is_unicode() {
                    continue;
                }
                sub.codepoints(|cp| {
                    if let Some(c) = char::from_u32(cp) {
                        if let Some(g) = sub.glyph_index(cp) {
                            if g.0 != 0 && !glyphs.contains_key(&c) {
                                glyphs.insert(c, g.0);
                                advances.insert(c, f.glyph_hor_advance(g).unwrap_or(0) as f32 / upem);
                            }
                        }
                    }
                });
            }
        }
        let default_advance = advances.get(&'n').copied().unwrap_or(0.5);
        Some(Face {
            id,
            family: family.to_string(),
            bold,
            italic,
            fake_bold: false,
            fake_italic: false,
            data,
            index,
            ascent: asc / upem,
            descent: desc / upem,
            line_gap: gap / upem,
            underline_pos: ul.map(|m| -(m.position as f32) / upem).unwrap_or(0.1),
            underline_thickness: ul.map(|m| m.thickness as f32 / upem).unwrap_or(0.05).max(0.04),
            strike_pos: st.map(|m| m.position as f32 / upem).unwrap_or(0.26),
            x_height: f.tables().os2.and_then(|o| o.x_height()).map(|x| x as f32 / upem).unwrap_or(0.47),
            units_per_em: upem,
            advances,
            glyphs,
            default_advance,
        })
    }

    pub fn has_char(&self, c: char) -> bool {
        self.glyphs.contains_key(&c)
    }

    /// Glyph id for `c` (0 if missing).
    pub fn glyph(&self, c: char) -> u16 {
        self.glyphs.get(&c).copied().unwrap_or(0)
    }

    /// Advance width of `c` in em.
    pub fn advance(&self, c: char) -> f32 {
        match self.advances.get(&c) {
            Some(a) => *a,
            None => {
                if c.is_control() {
                    0.0
                } else {
                    self.default_advance
                }
            }
        }
    }

    /// Single line height in em (Word's "single" spacing).
    pub fn line_height(&self) -> f32 {
        self.ascent + self.descent + self.line_gap
    }

    /// Iterate over all (char, glyph) pairs (for PDF embedding).
    pub fn char_map(&self) -> impl Iterator<Item = (char, u16)> + '_ {
        self.glyphs.iter().map(|(c, g)| (*c, *g))
    }
}

struct Candidate {
    /// Names this entry answers to (first one is the canonical family name).
    names: &'static [&'static str],
    /// System font files: regular, bold, italic, bold italic (each a list of alternatives).
    files: [&'static [&'static str]; 4],
    /// Bundled fallback key (see `bundled`).
    bundled: &'static str,
}

macro_rules! bundle {
    ($name:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/", $name))
    };
}

fn bundled(key: &str) -> [&'static [u8]; 4] {
    match key {
        "serif" => [
            bundle!("LiberationSerif-Regular.ttf"),
            bundle!("LiberationSerif-Bold.ttf"),
            bundle!("LiberationSerif-Italic.ttf"),
            bundle!("LiberationSerif-BoldItalic.ttf"),
        ],
        "sans" => [
            bundle!("LiberationSans-Regular.ttf"),
            bundle!("LiberationSans-Bold.ttf"),
            bundle!("LiberationSans-Italic.ttf"),
            bundle!("LiberationSans-BoldItalic.ttf"),
        ],
        "mono" => [
            bundle!("LiberationMono-Regular.ttf"),
            bundle!("LiberationMono-Bold.ttf"),
            bundle!("LiberationMono-Italic.ttf"),
            bundle!("LiberationMono-BoldItalic.ttf"),
        ],
        "cambria" => [
            bundle!("Caladea-Regular.ttf"),
            bundle!("Caladea-Bold.ttf"),
            bundle!("Caladea-Italic.ttf"),
            bundle!("Caladea-BoldItalic.ttf"),
        ],
        _ => [
            bundle!("Carlito-Regular.ttf"),
            bundle!("Carlito-Bold.ttf"),
            bundle!("Carlito-Italic.ttf"),
            bundle!("Carlito-BoldItalic.ttf"),
        ],
    }
}

const WIN: &str = "C:\\Windows\\Fonts\\";

const CANDIDATES: &[Candidate] = &[
    Candidate {
        names: &["Calibri", "Calibri Light", "Aptos", "Aptos Display", "Carlito", "Segoe UI"],
        files: [
            &["C:\\Windows\\Fonts\\calibri.ttf", "/Library/Fonts/Microsoft/Calibri.ttf", "/Applications/Microsoft Word.app/Contents/Resources/DFonts/Calibri.ttf"],
            &["C:\\Windows\\Fonts\\calibrib.ttf", "/Library/Fonts/Microsoft/Calibri Bold.ttf", "/Applications/Microsoft Word.app/Contents/Resources/DFonts/Calibrib.ttf"],
            &["C:\\Windows\\Fonts\\calibrii.ttf", "/Library/Fonts/Microsoft/Calibri Italic.ttf", "/Applications/Microsoft Word.app/Contents/Resources/DFonts/Calibrii.ttf"],
            &["C:\\Windows\\Fonts\\calibriz.ttf", "/Library/Fonts/Microsoft/Calibri Bold Italic.ttf", "/Applications/Microsoft Word.app/Contents/Resources/DFonts/Calibriz.ttf"],
        ],
        bundled: "calibri",
    },
    Candidate {
        names: &["Arial", "Helvetica", "Liberation Sans", "Arial Nova", "Helvetica Neue"],
        files: [
            &["C:\\Windows\\Fonts\\arial.ttf", "/System/Library/Fonts/Supplemental/Arial.ttf", "/Library/Fonts/Arial.ttf"],
            &["C:\\Windows\\Fonts\\arialbd.ttf", "/System/Library/Fonts/Supplemental/Arial Bold.ttf", "/Library/Fonts/Arial Bold.ttf"],
            &["C:\\Windows\\Fonts\\ariali.ttf", "/System/Library/Fonts/Supplemental/Arial Italic.ttf", "/Library/Fonts/Arial Italic.ttf"],
            &["C:\\Windows\\Fonts\\arialbi.ttf", "/System/Library/Fonts/Supplemental/Arial Bold Italic.ttf", "/Library/Fonts/Arial Bold Italic.ttf"],
        ],
        bundled: "sans",
    },
    Candidate {
        names: &["Times New Roman", "Times", "Liberation Serif", "Book Antiqua", "Palatino Linotype", "Garamond"],
        files: [
            &["C:\\Windows\\Fonts\\times.ttf", "/System/Library/Fonts/Supplemental/Times New Roman.ttf", "/Library/Fonts/Times New Roman.ttf"],
            &["C:\\Windows\\Fonts\\timesbd.ttf", "/System/Library/Fonts/Supplemental/Times New Roman Bold.ttf", "/Library/Fonts/Times New Roman Bold.ttf"],
            &["C:\\Windows\\Fonts\\timesi.ttf", "/System/Library/Fonts/Supplemental/Times New Roman Italic.ttf", "/Library/Fonts/Times New Roman Italic.ttf"],
            &["C:\\Windows\\Fonts\\timesbi.ttf", "/System/Library/Fonts/Supplemental/Times New Roman Bold Italic.ttf", "/Library/Fonts/Times New Roman Bold Italic.ttf"],
        ],
        bundled: "serif",
    },
    Candidate {
        names: &["Courier New", "Courier", "Consolas", "Liberation Mono", "Lucida Console", "Menlo", "Cascadia Code"],
        files: [
            &["C:\\Windows\\Fonts\\cour.ttf", "/System/Library/Fonts/Supplemental/Courier New.ttf", "/Library/Fonts/Courier New.ttf"],
            &["C:\\Windows\\Fonts\\courbd.ttf", "/System/Library/Fonts/Supplemental/Courier New Bold.ttf", "/Library/Fonts/Courier New Bold.ttf"],
            &["C:\\Windows\\Fonts\\couri.ttf", "/System/Library/Fonts/Supplemental/Courier New Italic.ttf", "/Library/Fonts/Courier New Italic.ttf"],
            &["C:\\Windows\\Fonts\\courbi.ttf", "/System/Library/Fonts/Supplemental/Courier New Bold Italic.ttf", "/Library/Fonts/Courier New Bold Italic.ttf"],
        ],
        bundled: "mono",
    },
    Candidate {
        names: &["Cambria", "Caladea", "Cambria Math"],
        files: [
            &["C:\\Windows\\Fonts\\cambria.ttc", "/Library/Fonts/Microsoft/Cambria.ttf", "/Applications/Microsoft Word.app/Contents/Resources/DFonts/Cambria.ttc"],
            &["C:\\Windows\\Fonts\\cambriab.ttf", "/Library/Fonts/Microsoft/Cambria Bold.ttf", "/Applications/Microsoft Word.app/Contents/Resources/DFonts/Cambriab.ttf"],
            &["C:\\Windows\\Fonts\\cambriai.ttf", "/Library/Fonts/Microsoft/Cambria Italic.ttf", "/Applications/Microsoft Word.app/Contents/Resources/DFonts/Cambriai.ttf"],
            &["C:\\Windows\\Fonts\\cambriaz.ttf", "/Library/Fonts/Microsoft/Cambria Bold Italic.ttf", "/Applications/Microsoft Word.app/Contents/Resources/DFonts/Cambriaz.ttf"],
        ],
        bundled: "cambria",
    },
    Candidate {
        names: &["Georgia"],
        files: [
            &["C:\\Windows\\Fonts\\georgia.ttf", "/System/Library/Fonts/Supplemental/Georgia.ttf", "/Library/Fonts/Georgia.ttf"],
            &["C:\\Windows\\Fonts\\georgiab.ttf", "/System/Library/Fonts/Supplemental/Georgia Bold.ttf", "/Library/Fonts/Georgia Bold.ttf"],
            &["C:\\Windows\\Fonts\\georgiai.ttf", "/System/Library/Fonts/Supplemental/Georgia Italic.ttf", "/Library/Fonts/Georgia Italic.ttf"],
            &["C:\\Windows\\Fonts\\georgiaz.ttf", "/System/Library/Fonts/Supplemental/Georgia Bold Italic.ttf", "/Library/Fonts/Georgia Bold Italic.ttf"],
        ],
        bundled: "serif",
    },
    Candidate {
        names: &["Verdana"],
        files: [
            &["C:\\Windows\\Fonts\\verdana.ttf", "/System/Library/Fonts/Supplemental/Verdana.ttf", "/Library/Fonts/Verdana.ttf"],
            &["C:\\Windows\\Fonts\\verdanab.ttf", "/System/Library/Fonts/Supplemental/Verdana Bold.ttf", "/Library/Fonts/Verdana Bold.ttf"],
            &["C:\\Windows\\Fonts\\verdanai.ttf", "/System/Library/Fonts/Supplemental/Verdana Italic.ttf", "/Library/Fonts/Verdana Italic.ttf"],
            &["C:\\Windows\\Fonts\\verdanaz.ttf", "/System/Library/Fonts/Supplemental/Verdana Bold Italic.ttf", "/Library/Fonts/Verdana Bold Italic.ttf"],
        ],
        bundled: "sans",
    },
    Candidate {
        names: &["Tahoma"],
        files: [
            &["C:\\Windows\\Fonts\\tahoma.ttf", "/System/Library/Fonts/Supplemental/Tahoma.ttf", "/Library/Fonts/Tahoma.ttf"],
            &["C:\\Windows\\Fonts\\tahomabd.ttf", "/System/Library/Fonts/Supplemental/Tahoma Bold.ttf", "/Library/Fonts/Tahoma Bold.ttf"],
            &[],
            &[],
        ],
        bundled: "sans",
    },
    Candidate {
        names: &["Trebuchet MS"],
        files: [
            &["C:\\Windows\\Fonts\\trebuc.ttf", "/System/Library/Fonts/Supplemental/Trebuchet MS.ttf", "/Library/Fonts/Trebuchet MS.ttf"],
            &["C:\\Windows\\Fonts\\trebucbd.ttf", "/System/Library/Fonts/Supplemental/Trebuchet MS Bold.ttf", "/Library/Fonts/Trebuchet MS Bold.ttf"],
            &["C:\\Windows\\Fonts\\trebucit.ttf", "/System/Library/Fonts/Supplemental/Trebuchet MS Italic.ttf", "/Library/Fonts/Trebuchet MS Italic.ttf"],
            &["C:\\Windows\\Fonts\\trebucbi.ttf", "/System/Library/Fonts/Supplemental/Trebuchet MS Bold Italic.ttf", "/Library/Fonts/Trebuchet MS Bold Italic.ttf"],
        ],
        bundled: "sans",
    },
    Candidate {
        names: &["Comic Sans MS"],
        files: [
            &["C:\\Windows\\Fonts\\comic.ttf", "/System/Library/Fonts/Supplemental/Comic Sans MS.ttf", "/Library/Fonts/Comic Sans MS.ttf"],
            &["C:\\Windows\\Fonts\\comicbd.ttf", "/System/Library/Fonts/Supplemental/Comic Sans MS Bold.ttf", "/Library/Fonts/Comic Sans MS Bold.ttf"],
            &["C:\\Windows\\Fonts\\comici.ttf"],
            &["C:\\Windows\\Fonts\\comicz.ttf"],
        ],
        bundled: "sans",
    },
    Candidate {
        names: &["Impact"],
        files: [&["C:\\Windows\\Fonts\\impact.ttf", "/System/Library/Fonts/Supplemental/Impact.ttf", "/Library/Fonts/Impact.ttf"], &[], &[], &[]],
        bundled: "sans",
    },
    Candidate {
        names: &["Century Gothic"],
        files: [
            &["C:\\Windows\\Fonts\\GOTHIC.TTF", "/Library/Fonts/Microsoft/Century Gothic.ttf"],
            &["C:\\Windows\\Fonts\\GOTHICB.TTF", "/Library/Fonts/Microsoft/Century Gothic Bold.ttf"],
            &["C:\\Windows\\Fonts\\GOTHICI.TTF", "/Library/Fonts/Microsoft/Century Gothic Italic.ttf"],
            &["C:\\Windows\\Fonts\\GOTHICBI.TTF", "/Library/Fonts/Microsoft/Century Gothic Bold Italic.ttf"],
        ],
        bundled: "sans",
    },
    Candidate {
        names: &["Segoe Print"],
        files: [&["C:\\Windows\\Fonts\\segoepr.ttf"], &["C:\\Windows\\Fonts\\segoeprb.ttf"], &[], &[]],
        bundled: "sans",
    },
    Candidate {
        names: &["Franklin Gothic Medium"],
        files: [&["C:\\Windows\\Fonts\\framd.ttf"], &[], &["C:\\Windows\\Fonts\\framdit.ttf"], &[]],
        bundled: "sans",
    },
    Candidate {
        names: &["Lucida Sans Unicode", "Lucida Grande"],
        files: [&["C:\\Windows\\Fonts\\l_10646.ttf", "/System/Library/Fonts/LucidaGrande.ttc"], &[], &[], &[]],
        bundled: "sans",
    },
    Candidate {
        names: &["Symbol", "Wingdings"],
        files: [&["C:\\Windows\\Fonts\\symbol.ttf", "/System/Library/Fonts/Supplemental/Symbol.ttf"], &[], &[], &[]],
        bundled: "sans",
    },
    Candidate {
        names: &["Segoe UI Emoji", "Apple Color Emoji"],
        files: [&["C:\\Windows\\Fonts\\seguiemj.ttf"], &[], &[], &[]],
        bundled: "sans",
    },
];

/// Font families offered in the font picker, in display order.
pub fn picker_families() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = CANDIDATES
        .iter()
        .filter(|c| c.names[0] != "Symbol" && c.names[0] != "Segoe UI Emoji")
        .map(|c| c.names[0])
        .collect();
    v.extend(["Calibri Light", "Aptos", "Helvetica", "Consolas", "Garamond", "Segoe UI"]);
    v.sort_unstable();
    v.dedup();
    v
}

/// All loaded faces, plus lookup by family name.
pub struct FontDb {
    pub faces: Vec<Face>,
    /// lower-case family name → [regular, bold, italic, bolditalic] face ids
    families: HashMap<String, [FaceId; 4]>,
    /// fallback families for missing glyphs (face ids of regular style)
    fallback: Vec<[FaceId; 4]>,
    default_family: [FaceId; 4],
}

fn read_first(paths: &[&str]) -> Option<(Vec<u8>, String)> {
    for p in paths {
        if let Ok(b) = std::fs::read(p) {
            return Some((b, p.to_string()));
        }
    }
    None
}

impl FontDb {
    /// The shared font database (loaded once).
    pub fn global() -> &'static FontDb {
        static DB: OnceLock<FontDb> = OnceLock::new();
        DB.get_or_init(|| FontDb::load(true))
    }

    /// Load fonts. With `use_system` false, only bundled fonts are used (deterministic, for tests).
    pub fn load(use_system: bool) -> FontDb {
        let mut db = FontDb { faces: Vec::new(), families: HashMap::new(), fallback: Vec::new(), default_family: [0; 4] };
        let mut bundled_loaded: HashMap<&'static str, [FaceId; 4]> = HashMap::new();
        let _ = WIN;
        for cand in CANDIDATES {
            let mut ids: [Option<FaceId>; 4] = [None; 4];
            if use_system {
                for (style, files) in cand.files.iter().enumerate() {
                    if let Some((bytes, path)) = read_first(files) {
                        let data = Arc::new(bytes);
                        let index = if path.to_lowercase().ends_with(".ttc") { 0 } else { 0 };
                        let id = db.faces.len() as FaceId;
                        if let Some(f) = Face::load(id, cand.names[0], data, index, style & 1 == 1, style & 2 == 2) {
                            db.faces.push(f);
                            ids[style] = Some(id);
                        }
                    }
                }
            }
            let fam: [FaceId; 4] = if ids[0].is_none() {
                // Fall back to the bundled font for the whole family.
                *bundled_loaded.entry(cand.bundled).or_insert_with(|| {
                    let files = bundled(cand.bundled);
                    let mut out = [0; 4];
                    for (style, bytes) in files.iter().enumerate() {
                        let id = db.faces.len() as FaceId;
                        let f = Face::load(id, cand.names[0], Arc::new(bytes.to_vec()), 0, style & 1 == 1, style & 2 == 2)
                            .expect("bundled font");
                        db.faces.push(f);
                        out[style] = id;
                    }
                    out
                })
            } else {
                let reg = ids[0].unwrap();
                let mut out = [reg; 4];
                for s in 1..4 {
                    out[s] = match ids[s] {
                        Some(i) => i,
                        None => {
                            // Synthesize from the closest available face.
                            let base = if s == 3 { ids[1].or(ids[2]).unwrap_or(reg) } else { reg };
                            let src = &db.faces[base as usize];
                            let id = db.faces.len() as FaceId;
                            let mut f = Face::load(id, cand.names[0], src.data.clone(), src.index, s & 1 == 1, s & 2 == 2).unwrap();
                            f.fake_bold = s & 1 == 1 && !src.bold;
                            f.fake_italic = s & 2 == 2 && !src.italic;
                            db.faces.push(f);
                            id
                        }
                    };
                }
                out
            };
            for n in cand.names {
                db.families.entry(n.to_lowercase()).or_insert(fam);
            }
        }
        db.default_family = db.families["calibri"];
        // Bundled families double as glyph fallbacks.
        for key in ["calibri", "liberation sans", "times new roman"] {
            if let Some(f) = db.families.get(key) {
                db.fallback.push(*f);
            }
        }
        if use_system {
            // Wide-coverage system fonts for scripts our bundled fonts lack.
            let extra: &[&str] = &[
                "C:\\Windows\\Fonts\\seguisym.ttf",
                "C:\\Windows\\Fonts\\msyh.ttc",
                "C:\\Windows\\Fonts\\msgothic.ttc",
                "C:\\Windows\\Fonts\\malgun.ttf",
                "C:\\Windows\\Fonts\\arialuni.ttf",
                "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
                "/Library/Fonts/Arial Unicode.ttf",
                "/System/Library/Fonts/Hiragino Sans GB.ttc",
                "/System/Library/Fonts/PingFang.ttc",
                "/System/Library/Fonts/Apple Symbols.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
            ];
            for p in extra {
                if let Ok(b) = std::fs::read(p) {
                    let id = db.faces.len() as FaceId;
                    if let Some(f) = Face::load(id, "Fallback", Arc::new(b), 0, false, false) {
                        if f.glyphs.len() > 500 {
                            db.faces.push(f);
                            db.fallback.push([id; 4]);
                        }
                    }
                }
            }
        }
        db
    }

    /// Face for a family and style (unknown families map to Calibri).
    pub fn face_for(&self, family: &str, bold: bool, italic: bool) -> FaceId {
        let fam = self.families.get(&family.to_lowercase()).copied().unwrap_or_else(|| self.guess(family));
        fam[(bold as usize) | ((italic as usize) << 1)]
    }

    fn guess(&self, family: &str) -> [FaceId; 4] {
        let l = family.to_lowercase();
        let key = if l.contains("mono") || l.contains("courier") || l.contains("code") || l.contains("console") {
            "courier new"
        } else if l.contains("serif") && !l.contains("sans") || l.contains("times") || l.contains("roman") || l.contains("garamond") || l.contains("book") {
            "times new roman"
        } else if l.contains("arial") || l.contains("helvet") || l.contains("sans") {
            "arial"
        } else {
            "calibri"
        };
        self.families.get(key).copied().unwrap_or(self.default_family)
    }

    pub fn face(&self, id: FaceId) -> &Face {
        &self.faces[id as usize]
    }

    /// The face to draw `c` with: `id` if it has the glyph, else a fallback with the same style.
    pub fn resolve_char(&self, id: FaceId, c: char) -> FaceId {
        let f = self.face(id);
        if f.has_char(c) || c.is_control() || c == ' ' {
            return id;
        }
        let style = (f.bold as usize) | ((f.italic as usize) << 1);
        for fb in &self.fallback {
            let fid = fb[style];
            if self.face(fid).has_char(c) {
                return fid;
            }
        }
        id
    }

    /// Whether the family name maps to a real installed font (not a substitute).
    pub fn is_known_family(&self, family: &str) -> bool {
        self.families.contains_key(&family.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_metrics() {
        let db = FontDb::load(false);
        let f = db.face(db.face_for("Calibri", false, false));
        // Carlito is metric-compatible with Calibri: 'a' advance = 979/2048
        assert!((f.advance('a') - 979.0 / 2048.0).abs() < 1e-3, "{}", f.advance('a'));
        assert!((f.line_height() - 1.2207).abs() < 0.01, "{}", f.line_height());
        let t = db.face(db.face_for("Times New Roman", true, false));
        assert!(t.bold);
        assert_eq!(db.face_for("Unknown Font", false, false), db.face_for("Calibri", false, false));
        assert_eq!(db.face_for("Courier", false, true), db.face_for("Courier New", false, true));
    }
}
