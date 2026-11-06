// No console window on Windows: this is a GUI-only app.
#![windows_subsystem = "windows"]

mod gui;
mod scan;

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `rustdu <folder>` scans that folder at once.
    let start = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    gui::run(start)
}
