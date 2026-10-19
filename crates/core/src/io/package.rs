//! Zip package helpers (OOXML / ODF containers).

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

/// An opened zip package: entry name → bytes.
pub struct Package {
    files: HashMap<String, Vec<u8>>,
}

impl Package {
    pub fn open(bytes: &[u8]) -> Result<Package, String> {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("Not a valid zip package: {e}"))?;
        let mut files = HashMap::new();
        for i in 0..zip.len() {
            let mut f = match zip.by_index(i) {
                Ok(f) => f,
                Err(_) => continue,
            };
            if f.is_dir() {
                continue;
            }
            let name = match f.name() {
                Ok(n) => n.trim_start_matches('/').to_string(),
                Err(_) => continue,
            };
            let mut data = Vec::new();
            if f.read_to_end(&mut data).is_ok() {
                files.insert(name, data);
            }
        }
        Ok(Package { files })
    }

    /// Get an entry (exact match first, then case-insensitive).
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        let name = name.trim_start_matches('/');
        if let Some(v) = self.files.get(name) {
            return Some(v);
        }
        self.files.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_slice())
    }

    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.files.keys()
    }
}

/// True if `bytes` is a zip containing an entry with this name.
pub fn zip_has_entry(bytes: &[u8], name: &str) -> bool {
    match zip::ZipArchive::new(Cursor::new(bytes)) {
        Ok(z) => z.file_names().any(|n| n.map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false)),
        Err(_) => false,
    }
}

/// Builds a zip package in memory.
pub struct ZipBuilder {
    w: zip::ZipWriter<Cursor<Vec<u8>>>,
}

impl Default for ZipBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ZipBuilder {
    pub fn new() -> Self {
        ZipBuilder { w: zip::ZipWriter::new(Cursor::new(Vec::new())) }
    }

    pub fn add(&mut self, name: &str, data: &[u8]) -> Result<(), String> {
        self.add_with(name, data, true)
    }

    /// Add an entry without compression (e.g. ODF `mimetype`).
    pub fn add_stored(&mut self, name: &str, data: &[u8]) -> Result<(), String> {
        self.add_with(name, data, false)
    }

    fn add_with(&mut self, name: &str, data: &[u8], deflate: bool) -> Result<(), String> {
        let method = if deflate { zip::CompressionMethod::Deflated } else { zip::CompressionMethod::Stored };
        let opts = zip::write::SimpleFileOptions::default().compression_method(method);
        self.w.start_file(name, opts).map_err(|e| e.to_string())?;
        self.w.write_all(data).map_err(|e| e.to_string())
    }

    pub fn finish(self) -> Result<Vec<u8>, String> {
        Ok(self.w.finish().map_err(|e| e.to_string())?.into_inner())
    }
}
