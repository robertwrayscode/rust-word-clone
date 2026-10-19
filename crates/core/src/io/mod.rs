//! File formats: DOCX, ODT, RTF, HTML, plain text and Markdown.
//!
//! Use [`open`] / [`save`] for files (format chosen by extension, with content sniffing as a fallback
//! when reading), or [`read_bytes`] / [`write_bytes`] for in-memory conversion.

pub mod docx;
pub mod html;
pub mod markdown;
pub mod odt;
pub mod rtf;
pub mod txt;

pub mod package;
pub mod util;
pub mod xml;

use crate::model::Document;
use std::path::Path;

/// A supported file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Docx,
    Odt,
    Rtf,
    Html,
    Txt,
    Markdown,
}

impl Format {
    pub const ALL: [Format; 6] = [Format::Docx, Format::Odt, Format::Rtf, Format::Html, Format::Txt, Format::Markdown];

    /// Format implied by a file's extension.
    pub fn from_path(path: &Path) -> Option<Format> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Format::from_extension(&ext)
    }

    pub fn from_extension(ext: &str) -> Option<Format> {
        Some(match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "docx" | "docm" | "dotx" | "dotm" => Format::Docx,
            "odt" | "ott" => Format::Odt,
            "rtf" => Format::Rtf,
            "html" | "htm" | "xhtml" => Format::Html,
            "txt" | "text" => Format::Txt,
            "md" | "markdown" => Format::Markdown,
            _ => return None,
        })
    }

    /// Canonical file extension (without dot).
    pub fn extension(self) -> &'static str {
        match self {
            Format::Docx => "docx",
            Format::Odt => "odt",
            Format::Rtf => "rtf",
            Format::Html => "html",
            Format::Txt => "txt",
            Format::Markdown => "md",
        }
    }

    /// Human-readable name for file dialogs.
    pub fn description(self) -> &'static str {
        match self {
            Format::Docx => "Word Document",
            Format::Odt => "OpenDocument Text",
            Format::Rtf => "Rich Text Format",
            Format::Html => "Web Page",
            Format::Txt => "Plain Text",
            Format::Markdown => "Markdown",
        }
    }

    /// Guess the format from file contents.
    pub fn sniff(bytes: &[u8]) -> Format {
        if bytes.starts_with(b"PK") {
            if package::zip_has_entry(bytes, "word/document.xml") {
                return Format::Docx;
            }
            if package::zip_has_entry(bytes, "content.xml") {
                return Format::Odt;
            }
        }
        let head = &bytes[..bytes.len().min(1024)];
        let head_str = String::from_utf8_lossy(head);
        let trimmed = head_str.trim_start_matches('\u{FEFF}').trim_start();
        if trimmed.starts_with("{\\rtf") {
            return Format::Rtf;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("<!doctype html") || lower.starts_with("<html") || lower.contains("<html") {
            return Format::Html;
        }
        Format::Txt
    }
}

/// Read a document from bytes in the given format.
pub fn read_bytes(bytes: &[u8], format: Format) -> Result<Document, String> {
    match format {
        Format::Docx => docx::read(bytes),
        Format::Odt => odt::read(bytes),
        Format::Rtf => rtf::read(bytes),
        Format::Html => Ok(html::read(&txt::decode(bytes))),
        Format::Txt => Ok(txt::read(bytes)),
        Format::Markdown => Ok(markdown::read(&txt::decode(bytes))),
    }
}

/// Serialise a document in the given format.
pub fn write_bytes(doc: &Document, format: Format) -> Result<Vec<u8>, String> {
    match format {
        Format::Docx => docx::write(doc),
        Format::Odt => odt::write(doc),
        Format::Rtf => Ok(rtf::write(doc).into_bytes()),
        Format::Html => Ok(html::write(doc).into_bytes()),
        Format::Txt => Ok(txt::write(doc).into_bytes()),
        Format::Markdown => Ok(markdown::write(doc).into_bytes()),
    }
}

/// Open a file, choosing the format by extension and falling back to sniffing the contents.
pub fn open(path: &Path) -> Result<Document, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
    let sniffed = Format::sniff(&bytes);
    let fmt = match Format::from_path(path) {
        // Trust the contents when they clearly disagree with the extension (e.g. an RTF saved as .doc).
        Some(f @ (Format::Docx | Format::Odt)) if sniffed == f => f,
        Some(Format::Docx | Format::Odt) => sniffed,
        Some(f @ (Format::Txt | Format::Markdown)) if sniffed == Format::Txt => f,
        Some(f) if sniffed == Format::Txt || sniffed == f => f,
        _ => sniffed,
    };
    let mut doc = read_bytes(&bytes, fmt)?;
    if doc.properties.title.is_empty() && fmt != Format::Docx && fmt != Format::Odt {
        // Leave the title alone; the app shows the file name.
    }
    if doc.body.is_empty() {
        doc.body.push(crate::model::Block::Para(crate::model::Paragraph::new()));
    }
    Ok(doc)
}

/// Save a document; the format is chosen by the file extension (DOCX when unknown).
pub fn save(doc: &Document, path: &Path) -> Result<(), String> {
    let fmt = Format::from_path(path).unwrap_or(Format::Docx);
    let bytes = write_bytes(doc, fmt)?;
    // Write to a temporary file first so a failed save never truncates the original.
    let tmp = path.with_extension(format!("{}.rword-tmp", fmt.extension()));
    std::fs::write(&tmp, &bytes).map_err(|e| format!("Cannot write {}: {}", tmp.display(), e))?;
    std::fs::rename(&tmp, path).or_else(|_| {
        let r = std::fs::write(path, &bytes).map_err(|e| format!("Cannot write {}: {}", path.display(), e));
        let _ = std::fs::remove_file(&tmp);
        r
    })
}
