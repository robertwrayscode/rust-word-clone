//! System clipboard: plain text plus HTML for rich copy/paste with other apps.

use wordcore::{Block, Document, ImageData};

pub fn set(text: &str, blocks: &[Block], doc: &Document) {
    let html = wordcore::io::html::write_fragment(blocks, doc);
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let t = if cfg!(windows) { text.replace('\n', "\r\n") } else { text.to_string() };
        if cb.set_html(html, Some(t.clone())).is_err() {
            let _ = cb.set_text(t);
        }
    }
}

pub fn get_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

/// Rich content from another application (HTML on the clipboard).
pub fn get_html_blocks() -> Option<Vec<Block>> {
    let html = arboard::Clipboard::new().ok()?.get().html().ok()?;
    let blocks = wordcore::io::html::read_fragment(&html);
    if blocks.is_empty() {
        None
    } else {
        Some(blocks)
    }
}

pub fn get_image() -> Option<ImageData> {
    let img = arboard::Clipboard::new().ok()?.get_image().ok()?;
    crate::images::from_rgba(img.width, img.height, &img.bytes)
}
