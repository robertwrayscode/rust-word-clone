//! RWord: a desktop word processor in the style of Microsoft Word.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod canvas;
mod clipboard;
mod demo;
mod dialogs;
mod fileio;
mod fonts;
mod images;
mod macos_open;
mod objects;
mod panels;
mod png;
mod print;
mod proof;
mod ribbon;
mod theme;

use std::path::PathBuf;

/// Command-line options.
#[derive(Default, Clone, Debug)]
pub struct Args {
    pub file: Option<PathBuf>,
    pub demo: bool,
    pub screenshot: Option<PathBuf>,
    /// Scripted UI actions run before taking a screenshot (testing aid).
    pub actions: Vec<String>,
}

fn parse_args() -> Args {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(s) = it.next() {
        match s.as_str() {
            "--demo" => a.demo = true,
            "--screenshot" => a.screenshot = it.next().map(PathBuf::from),
            "--action" => {
                if let Some(x) = it.next() {
                    a.actions.push(x);
                }
            }
            "--help" | "-h" => {
                println!("Usage: rword [FILE] [--demo] [--screenshot OUT.png] [--action NAME]...");
                std::process::exit(0);
            }
            // macOS passes -psn_... when launched from Finder.
            _ if s.starts_with("-psn") => {}
            _ if !s.starts_with("--") && a.file.is_none() => a.file = Some(PathBuf::from(s)),
            _ => eprintln!("rword: ignoring unknown argument {}", s),
        }
    }
    a
}

fn main() -> eframe::Result {
    let args = parse_args();
    macos_open::install();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("Document1 - RWord {}", env!("CARGO_PKG_VERSION")))
            .with_inner_size([1360.0, 880.0])
            .with_min_inner_size([700.0, 420.0])
            .with_drag_and_drop(true)
            .with_icon(std::sync::Arc::new(egui::IconData {
                rgba: include_bytes!("../assets/icon/rword-128.rgba").to_vec(),
                width: 128,
                height: 128,
            })),
        ..Default::default()
    };
    eframe::run_native(
        "RWord",
        options,
        Box::new(move |cc| {
            macos_open::set_context(&cc.egui_ctx);
            let db = wordcore::fonts::FontDb::global();
            fonts::install(&cc.egui_ctx, db);
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(app::App::new(args)))
        }),
    )
}
