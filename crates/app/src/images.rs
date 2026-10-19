//! Decode document images into egui textures (cached by image identity).

use std::collections::HashMap;
use std::sync::Arc;

use wordcore::ImageData;

pub fn texture(cache: &mut HashMap<usize, egui::TextureHandle>, ctx: &egui::Context, img: &Arc<ImageData>) -> Option<egui::TextureId> {
    let key = Arc::as_ptr(img) as usize;
    if let Some(t) = cache.get(&key) {
        return Some(t.id());
    }
    let decoded = image::load_from_memory(&img.bytes).ok()?.to_rgba8();
    let (w, h) = decoded.dimensions();
    let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], decoded.as_raw());
    let t = ctx.load_texture(format!("img-{}", key), ci, egui::TextureOptions::LINEAR);
    let id = t.id();
    cache.insert(key, t);
    Some(id)
}

/// Read an image file into ImageData (with pixel size).
pub fn load_file(path: &std::path::Path) -> Result<ImageData, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    from_bytes(bytes)
}

pub fn from_bytes(bytes: Vec<u8>) -> Result<ImageData, String> {
    let fmt = image::guess_format(&bytes).map_err(|e| e.to_string())?;
    let (format, bytes) = match fmt {
        image::ImageFormat::Png => ("png", bytes),
        image::ImageFormat::Jpeg => ("jpeg", bytes),
        image::ImageFormat::Gif => ("gif", bytes),
        image::ImageFormat::Bmp => ("bmp", bytes),
        _ => return Err("Unsupported picture format. Use PNG, JPEG, GIF or BMP.".into()),
    };
    let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    Ok(ImageData { bytes, format: format.into(), pixel_size: Some((img.width(), img.height())) })
}

/// Encode RGBA pixels (e.g. from the clipboard) as PNG image data.
pub fn from_rgba(w: usize, h: usize, rgba: &[u8]) -> Option<ImageData> {
    let bytes = crate::png::encode_rgba(w as u32, h as u32, rgba);
    Some(ImageData { bytes, format: "png".into(), pixel_size: Some((w as u32, h as u32)) })
}
