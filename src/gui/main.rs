#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod document;
mod focus;
mod fonts;
mod history;
mod history_ui;
mod icon;
mod navigation;
mod render;
mod ui_icons;
mod worker;

use eframe::egui;

fn main() -> eframe::Result {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new(worker::WORKER_ARG)) {
        if worker::run().is_err() {
            std::process::exit(1);
        }
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 760.0])
            .with_min_inner_size([480.0, 360.0])
            .with_icon(icon::data())
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    let source = std::env::args_os().nth(1);
    eframe::run_native(
        "Olive Browser",
        options,
        Box::new(move |cc| Ok(Box::new(app::OliveApp::new(cc, source)))),
    )
}
