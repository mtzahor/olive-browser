//! The desktop viewer behind the `olive-gui` binary. This is not a stable
//! library API: it lives in the library so the binary and the process
//! integration tests share one build of the tab controller.

pub mod app;
mod daily_ui;
mod devtools;
pub mod document;
mod downloads;
mod favicons;
mod find;
mod focus;
mod focus_export;
mod fonts;
mod forms;
mod glass;
mod history;
mod history_ui;
pub mod icon;
mod navigation;
mod new_tab;
mod profile;
mod render;
mod storage;
mod theme;
mod ui_icons;
pub mod worker;
mod zoom;
