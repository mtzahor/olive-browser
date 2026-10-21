#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use eframe::egui;
use olive_html::gui::{app, icon, network, worker};

// Counts allocations so tab processes, which run this executable, can limit their heap.
#[global_allocator]
static HEAP: worker::CountedHeap = worker::CountedHeap;

fn main() -> eframe::Result {
    #[cfg(windows)]
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new(worker::WINDOWS_LAUNCHER_ARG))
    {
        std::process::exit(worker::run_windows_launcher() as i32);
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new(worker::WORKER_ARG)) {
        if let Err(error) = worker::run() {
            eprintln!("Tab worker failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new(network::NETWORK_ARG)) {
        if let Err(error) = network::run() {
            eprintln!("Network process failed: {error}");
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
