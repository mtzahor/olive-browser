//! The desktop viewer behind the `olive-gui` binary. This is not a stable
//! library API: it lives in the library so the binary and the process
//! integration tests share one build of the tab controller.

pub mod app;
mod daily_ui;
pub mod document;
mod downloads;
mod find;
mod focus;
mod focus_export;
mod fonts;
mod forms;
mod history;
mod history_ui;
pub mod icon;
mod navigation;
mod profile;
mod render;
mod theme;
mod ui_icons;
pub mod worker;
mod zoom;
