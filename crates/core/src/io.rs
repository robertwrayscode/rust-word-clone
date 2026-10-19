//! TEMPORARY stub until the file-format module lands.
use crate::model::*;
use std::path::Path;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format { Docx, Odt, Rtf, Html, Txt, Markdown }
impl Format {
    pub fn from_path(path: &Path) -> Option<Format> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() { "txt" => Some(Format::Txt), "docx" => Some(Format::Docx), _ => None }
    }
}
pub fn open(path: &Path) -> Result<Document, String> {
    let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    Ok(Document::from_text(&s))
}
pub fn save(doc: &Document, path: &Path) -> Result<(), String> {
    std::fs::write(path, doc.plain_text()).map_err(|e| e.to_string())
}
pub mod html {
    use crate::model::*;
    pub fn write_fragment(_b: &[Block], _d: &Document) -> String { String::new() }
    pub fn read_fragment(_s: &str) -> Vec<Block> { Vec::new() }
}
