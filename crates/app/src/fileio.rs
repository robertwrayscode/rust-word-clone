//! File dialogs, recent files, printing helpers.

use std::path::{Path, PathBuf};

const FILTERS: &[(&str, &[&str])] = &[
    ("Word Document", &["docx"]),
    ("OpenDocument Text", &["odt"]),
    ("Rich Text Format", &["rtf"]),
    ("Web Page", &["html", "htm"]),
    ("Plain Text", &["txt"]),
    ("Markdown", &["md"]),
    ("PDF", &["pdf"]),
];

pub fn open_dialog() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("All Documents", &["docx", "docm", "dotx", "odt", "rtf", "html", "htm", "txt", "md", "markdown"])
        .add_filter("Word Documents", &["docx", "docm", "dotx"])
        .add_filter("OpenDocument Text", &["odt"])
        .add_filter("Rich Text Format", &["rtf"])
        .add_filter("Web Pages", &["html", "htm"])
        .add_filter("Text Files", &["txt", "md"])
        .add_filter("All Files", &["*"])
        .pick_file()
}

pub fn save_dialog(current: Option<&Path>) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new();
    for (name, exts) in FILTERS {
        d = d.add_filter(*name, exts);
    }
    let name = current
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Document1".into());
    d = d.set_file_name(format!("{}.docx", name));
    if let Some(dir) = current.and_then(|p| p.parent()) {
        d = d.set_directory(dir);
    }
    let p = d.save_file()?;
    Some(if p.extension().is_none() { p.with_extension("docx") } else { p })
}

pub fn pdf_dialog(default: Option<&Path>) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new().add_filter("PDF", &["pdf"]);
    let name = default.and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Document1.pdf".into());
    d = d.set_file_name(name);
    if let Some(dir) = default.and_then(|p| p.parent()) {
        d = d.set_directory(dir);
    }
    let p = d.save_file()?;
    Some(if p.extension().is_none() { p.with_extension("pdf") } else { p })
}

pub fn picture_dialog() -> Option<PathBuf> {
    rfd::FileDialog::new().add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "bmp"]).pick_file()
}

fn config_dir() -> Option<PathBuf> {
    let d = dirs::config_dir()?.join("RWord");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

pub fn load_recent() -> Vec<PathBuf> {
    config_dir()
        .and_then(|d| std::fs::read_to_string(d.join("recent.txt")).ok())
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).map(PathBuf::from).collect())
        .unwrap_or_default()
}

pub fn add_recent(list: &mut Vec<PathBuf>, p: &Path) {
    let p = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    list.retain(|x| *x != p);
    list.insert(0, p);
    list.truncate(10);
    if let Some(d) = config_dir() {
        let s: Vec<String> = list.iter().map(|p| p.display().to_string()).collect();
        let _ = std::fs::write(d.join("recent.txt"), s.join("\n"));
    }
}

pub fn settings_path(name: &str) -> Option<PathBuf> {
    config_dir().map(|d| d.join(name))
}

/// The user's name for comments and tracked changes.
pub fn user_name() -> String {
    if let Some(p) = settings_path("author.txt") {
        if let Ok(s) = std::fs::read_to_string(p) {
            if !s.trim().is_empty() {
                return s.trim().to_string();
            }
        }
    }
    std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_else(|_| "Author".into())
}

pub fn save_user_name(name: &str) {
    if let Some(p) = settings_path("author.txt") {
        let _ = std::fs::write(p, name);
    }
}

/// Open a PDF in the system viewer so the user can print it.
pub fn print_pdf(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // Try the shell "print" verb first, else just open the file.
        let ok = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &format!("Start-Process -FilePath '{}' -Verb Print", path.display())])
            .creation_flags(0x0800_0000)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            let _ = std::process::Command::new("cmd").args(["/C", "start", "", &path.display().to_string()]).creation_flags(0x0800_0000).spawn();
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}
