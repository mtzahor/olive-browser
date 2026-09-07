use crate::render::{INK, OLIVE, Page};
use eframe::egui::{self, Color32, FontFamily, RichText};
use olive_html::{ParseOptions, parse_reader};
use std::{
    fs::File,
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
};

const PAPER: Color32 = Color32::from_rgb(250, 250, 246);
const CHROME: Color32 = Color32::from_rgb(239, 242, 231);

struct LoadedPage {
    path: PathBuf,
    page: Page,
    corrections: usize,
}

pub struct OliveApp {
    loaded: Option<LoadedPage>,
    pending: Option<Receiver<Result<LoadedPage, String>>>,
    error: Option<String>,
    // Each successful open gets a new scroll ID, including re-opening the same file.
    generation: u64,
}

impl OliveApp {
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        let ctx = &cc.egui_ctx;
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "Inter".into(),
            egui::FontData::from_static(include_bytes!("../../assets/fonts/InterVariable.ttf"))
                .into(),
        );
        fonts
            .families
            .get_mut(&FontFamily::Proportional)
            .unwrap()
            .insert(0, "Inter".into());
        ctx.set_fonts(fonts);
        let mut style = egui::Style {
            visuals: egui::Visuals::light(),
            ..Default::default()
        };
        style.visuals.override_text_color = Some(INK);
        style.visuals.panel_fill = PAPER;
        style.visuals.selection.bg_fill = Color32::from_rgb(209, 223, 182);
        style.spacing.button_padding = egui::vec2(16.0, 10.0);
        style.spacing.item_spacing = egui::vec2(12.0, 8.0);
        ctx.set_global_style(style);
        ctx.set_theme(egui::Theme::Light);
        let mut app = Self {
            loaded: None,
            pending: None,
            error: None,
            generation: 0,
        };
        if let Some(path) = path {
            app.open(path, ctx);
        }
        app
    }

    fn choose_file(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        let mut dialog = rfd::FileDialog::new().add_filter("HTML documents", &["html", "htm"]);
        if let Some(parent) = self.loaded.as_ref().and_then(|loaded| loaded.path.parent()) {
            dialog = dialog.set_directory(parent);
        }
        if let Some(path) = dialog.pick_file() {
            self.open(path, ctx);
        }
    }

    fn open(&mut self, path: PathBuf, ctx: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        self.error = None;
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        // The parser's DOM stays on this worker; only the owned presentation crosses threads.
        match std::thread::Builder::new()
            .name("olive-file-loader".into())
            .spawn(move || {
                let result = load_file(path);
                let _ = sender.send(result);
                ctx.request_repaint();
            }) {
            Ok(_) => self.pending = Some(receiver),
            Err(error) => self.error = Some(format!("Could not start the file loader: {error}")),
        }
    }

    fn receive(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.pending else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(loaded)) => {
                let title = if loaded.page.title.trim().is_empty() {
                    loaded
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                } else {
                    loaded.page.title.clone()
                };
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                    "{title} — Olive Browser"
                )));
                self.loaded = Some(loaded);
                self.generation = self.generation.wrapping_add(1);
                self.pending = None;
            }
            Ok(Err(error)) => {
                self.error = Some(error);
                self.pending = None;
            }
            Err(TryRecvError::Disconnected) => {
                self.error =
                    Some("The file loader stopped unexpectedly. You can open another file.".into());
                self.pending = None;
            }
            Err(TryRecvError::Empty) => {}
        }
    }
}

fn load_file(path: PathBuf) -> Result<LoadedPage, String> {
    if !path
        .metadata()
        .map_err(|e| format!("Could not open {}: {e}", path.display()))?
        .is_file()
    {
        return Err("Choose a regular HTML file.".into());
    }
    let file = File::open(&path).map_err(|e| format!("Could not open {}: {e}", path.display()))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Choose a regular HTML file.".into());
    }
    let parsed = parse_reader(file, ParseOptions::default()).map_err(|e| e.to_string())?;
    let corrections = parsed
        .diagnostics
        .len()
        .saturating_add(parsed.omitted_diagnostics);
    Ok(LoadedPage {
        path,
        page: Page::from_document(&parsed.document),
        corrections,
    })
}

fn open_button(ui: &mut egui::Ui, enabled: bool) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new("Open HTML…").color(Color32::WHITE))
            .fill(OLIVE)
            .min_size(egui::vec2(126.0, 38.0))
            .corner_radius(7),
    )
    .clicked()
}

impl eframe::App for OliveApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        PAPER.to_normalized_gamma_f32()
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Panels share the root UI; leave no unpainted gap between them.
        ui.spacing_mut().item_spacing.y = 0.0;
        let ctx = ui.ctx().clone();
        if ctx.input_mut(|input| {
            input.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::O,
            ))
        }) {
            self.choose_file(&ctx);
        }
        if let Some(path) = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .first()
                .map(|file| file.path().to_path_buf())
        }) {
            self.open(path, &ctx);
        }
        let mut choose = false;
        egui::Panel::top("toolbar")
            .exact_size(64.0)
            .frame(
                egui::Frame::new()
                    .fill(CHROME)
                    .inner_margin(egui::Margin::symmetric(20, 12)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Olive")
                            .size(25.0)
                            .color(OLIVE)
                            .variations([("wght", 650.0)]),
                    );
                    ui.label(RichText::new("Browser").size(14.0).weak());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        choose |= open_button(ui, self.pending.is_none());
                        if self.pending.is_some() {
                            ui.label("Opening…");
                        }
                    });
                });
            });
        egui::Panel::bottom("status")
            .exact_size(36.0)
            .frame(
                egui::Frame::new()
                    .fill(CHROME)
                    .inner_margin(egui::Margin::symmetric(20, 9)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let label = self
                        .loaded
                        .as_ref()
                        .map(|loaded| {
                            loaded
                                .path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned()
                        })
                        .unwrap_or_else(|| "Local HTML viewer".into());
                    ui.add(egui::Label::new(RichText::new(label).size(12.0)).truncate());
                    if let Some(loaded) = &self.loaded {
                        if loaded.page.css_ignored > 0 {
                            ui.label(RichText::new("Some CSS is unsupported").size(11.0).weak())
                                .on_hover_text(format!(
                                    "{} CSS rules or declarations were ignored.",
                                    loaded.page.css_ignored
                                ));
                        }
                        if loaded.corrections > 0 {
                            ui.label(
                                RichText::new("Opened with HTML corrections")
                                    .size(11.0)
                                    .weak(),
                            );
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                                .size(12.0)
                                .weak(),
                        );
                    });
                });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new().fill(
                    self.loaded
                        .as_ref()
                        .and_then(|loaded| loaded.page.background)
                        .unwrap_or(PAPER),
                ),
            )
            .show(ui, |ui| {
                if let Some(error) = &self.error {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(255, 235, 228))
                        .inner_margin(16)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("Could not open this file")
                                    .variations([("wght", 650.0)]),
                            );
                            ui.label(error);
                        });
                }
                if let Some(loaded) = &mut self.loaded {
                    egui::ScrollArea::both()
                        .id_salt(("document", self.generation))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let margin = ((ui.available_width() - 820.0) / 2.0).max(24.0);
                            egui::Frame::new()
                                .inner_margin(egui::Margin {
                                    left: margin.min(100.0) as i8,
                                    right: margin.min(100.0) as i8,
                                    top: 34,
                                    bottom: 40,
                                })
                                .show(ui, |ui| {
                                    if loaded.page.is_empty() {
                                        ui.label("This document has no visible content.");
                                    }
                                    loaded.page.show(ui);
                                });
                        });
                } else {
                    ui.vertical_centered(|ui| {
                        ui.add_space(((ui.available_height() - 240.0) * 0.4).max(24.0));
                        ui.label(
                            RichText::new("A fresh page.")
                                .size(38.0)
                                .color(OLIVE)
                                .variations([("wght", 600.0)]),
                        );
                        ui.add_space(10.0);
                        ui.label(RichText::new("Open an HTML file to begin.").size(18.0));
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("Choose a local file, or drop one into this window.")
                                .size(14.0)
                                .weak(),
                        );
                        ui.add_space(22.0);
                        choose |= open_button(ui, self.pending.is_none());
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(if cfg!(target_os = "macos") {
                                "⌘O to open a file"
                            } else {
                                "Ctrl+O to open a file"
                            })
                            .size(12.0)
                            .weak(),
                        );
                    });
                }
            });
        if choose {
            self.choose_file(&ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_accepts_an_html_file_and_reports_read_errors() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let loaded = load_file(root.join("examples/hello.html")).unwrap();
        assert_eq!(loaded.page.title, "Olive Browser");
        assert!(!loaded.page.is_empty());
        assert!(load_file(root.clone()).is_err());
        assert!(load_file(root.join("missing-olive-example.html")).is_err());
    }
}
