//! Register the document fonts (from wordcore's FontDb) with egui and draw laid-out text.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Painter, Pos2};
use wordcore::fonts::{FaceId, FontDb};

fn family(id: FaceId) -> FontFamily {
    FontFamily::Name(format!("f{}", id).into())
}

/// Font id for drawing a document face at `size` screen points.
pub fn font_id(id: FaceId, size: f32) -> FontId {
    FontId::new(size, family(id))
}

/// Load fonts and register them with egui.
pub fn install(ctx: &egui::Context, db: &FontDb) {
    let mut defs = FontDefinitions::default();
    let fallback: Vec<String> = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    // Share font bytes between faces that use the same file (synthesized styles).
    let mut keys: HashMap<(usize, u32), String> = HashMap::new();
    for f in &db.faces {
        let ptr = Arc::as_ptr(&f.data) as usize;
        let key = keys
            .entry((ptr, f.index))
            .or_insert_with(|| {
                let k = format!("doc-{}", f.id);
                let blob: Arc<dyn AsRef<[u8]> + Send + Sync> = f.data.clone();
                defs.font_data.insert(k.clone(), Arc::new(FontData::from_blob(blob, f.index)));
                k
            })
            .clone();
        let mut fam = vec![key];
        fam.extend(fallback.iter().cloned());
        defs.families.insert(family(f.id), fam);
    }
    // Interface font: Segoe UI on Windows, the Calibri-compatible font elsewhere.
    let ui_face = ["C:\\Windows\\Fonts\\segoeui.ttf"].iter().find_map(|p| std::fs::read(p).ok());
    let ui_key = match ui_face {
        Some(bytes) => {
            defs.font_data.insert("ui".into(), Arc::new(FontData::from_owned(bytes)));
            Some("ui".to_string())
        }
        None => {
            let id = db.face_for("Calibri", false, false);
            keys.iter().find(|(_, v)| **v == format!("doc-{}", id)).map(|(_, v)| v.clone()).or_else(|| Some(format!("doc-{}", id)))
        }
    };
    if let Some(k) = ui_key {
        if defs.font_data.contains_key(&k) {
            if let Some(p) = defs.families.get_mut(&FontFamily::Proportional) {
                p.insert(0, k);
            }
        }
    }
    // Symbol fonts for ribbon icons.
    for (i, path) in [
        "C:\\Windows\\Fonts\\seguisym.ttf",
        "C:\\Windows\\Fonts\\seguiemj.ttf",
        "/System/Library/Fonts/Apple Symbols.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]
    .iter()
    .enumerate()
    {
        if let Ok(bytes) = std::fs::read(path) {
            let key = format!("sym-{}", i);
            defs.font_data.insert(key.clone(), Arc::new(FontData::from_owned(bytes)));
            for fam in [FontFamily::Proportional, FontFamily::Monospace] {
                if let Some(list) = defs.families.get_mut(&fam) {
                    list.push(key.clone());
                }
            }
        }
    }
    ctx.set_fonts(defs);
}

/// Baseline offset (from the top of a single-row galley) per (face, size in 1/8 px).
static BASELINES: OnceLock<Mutex<HashMap<(FaceId, u32), f32>>> = OnceLock::new();

/// Draw `text` with its baseline at `pos.y`, starting at `pos.x`. `size` is in screen points.
#[allow(clippy::too_many_arguments)]
pub fn draw_text(painter: &Painter, pos: Pos2, text: &str, face: FaceId, size: f32, color: Color32, fake_bold: bool, spacing: f32, db: &FontDb) {
    if text.is_empty() || size < 0.5 {
        return;
    }
    let fid = font_id(face, size);
    if spacing.abs() > 0.01 {
        // Character spacing: draw each glyph at its own position.
        let f = db.face(face);
        let mut x = pos.x;
        for c in text.chars() {
            let s = c.to_string();
            draw_text(painter, Pos2::new(x, pos.y), &s, face, size, color, fake_bold, 0.0, db);
            x += f.advance(c) * size + spacing;
        }
        return;
    }
    let galley = painter.layout_no_wrap(text.to_string(), fid, color);
    let key = (face, (size * 8.0).round() as u32);
    let base = {
        let map = BASELINES.get_or_init(Default::default);
        let mut m = map.lock().unwrap();
        *m.entry(key).or_insert_with(|| {
            galley.rows.first().and_then(|r| r.glyphs.first().map(|g| r.pos.y + g.pos.y)).unwrap_or(size * 0.8)
        })
    };
    let top = Pos2::new(pos.x, pos.y - base);
    if fake_bold {
        painter.galley(top + egui::vec2(size * 0.03, 0.0), galley.clone(), color);
    }
    painter.galley(top, galley, color);
}
