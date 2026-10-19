//! Developer tools: page, console and resource diagnostics for the active tab.
use crate::gui::document::LoadedPage;
use eframe::egui;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Overview,
    Console,
    Network,
}

#[derive(Default)]
pub struct DevTools {
    pub open: bool,
    tab: Tab,
}

impl DevTools {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        loaded: Option<&LoadedPage>,
        worker_pid: Option<u32>,
    ) {
        if !self.open {
            return;
        }
        egui::Window::new("Developer tools")
            .id(egui::Id::unique("developer-tools"))
            .default_size(egui::vec2(620.0, 420.0))
            .min_size(egui::vec2(420.0, 280.0))
            .open(&mut self.open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab, Tab::Overview, "Overview");
                    ui.selectable_value(&mut self.tab, Tab::Console, "Console");
                    ui.selectable_value(&mut self.tab, Tab::Network, "Network");
                });
                ui.separator();
                let Some(loaded) = loaded else {
                    ui.label("No page is loaded in this tab.");
                    return;
                };
                match self.tab {
                    Tab::Overview => {
                        egui::Grid::new("devtools-overview")
                            .num_columns(2)
                            .spacing(egui::vec2(16.0, 8.0))
                            .show(ui, |ui| {
                                ui.strong("Title");
                                ui.label(&loaded.page.title);
                                ui.end_row();
                                ui.strong("Address");
                                ui.add(egui::Label::new(loaded.location.as_str()).truncate());
                                ui.end_row();
                                ui.strong("HTTP status");
                                ui.label(loaded.status.map_or_else(
                                    || "Local file".into(),
                                    |status| status.to_string(),
                                ));
                                ui.end_row();
                                ui.strong("JavaScript");
                                ui.label(if loaded.scripting_enabled {
                                    "Enabled"
                                } else {
                                    "Disabled"
                                });
                                ui.end_row();
                                ui.strong("Worker process");
                                ui.label(
                                    worker_pid
                                        .map_or_else(|| "Stopped".into(), |pid| pid.to_string()),
                                );
                                ui.end_row();
                                ui.strong("HTML corrections");
                                ui.label(loaded.corrections.to_string());
                                ui.end_row();
                                ui.strong("Unsupported CSS");
                                ui.label(loaded.page.css_ignored.to_string());
                                ui.end_row();
                                if !loaded.page.css_unsupported.is_empty() {
                                    ui.strong("Unsupported properties");
                                    ui.vertical(|ui| {
                                        for (name, count) in
                                            loaded.page.css_unsupported.iter().take(32)
                                        {
                                            ui.monospace(format!("{name}: {count}"));
                                        }
                                    });
                                    ui.end_row();
                                }
                            });
                    }
                    Tab::Console => {
                        ui.label(format!(
                            "{} scripts executed · {} skipped",
                            loaded.scripts.executed, loaded.scripts.skipped
                        ));
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for error in &loaded.scripts.diagnostics {
                                    ui.colored_label(
                                        ui.visuals().error_fg_color,
                                        format!("{:?}: {}", error.kind, error.message),
                                    );
                                }
                                for message in &loaded.scripts.console {
                                    ui.monospace(message);
                                }
                                let omitted = loaded
                                    .scripts
                                    .omitted_diagnostics
                                    .saturating_add(loaded.scripts.omitted_console_messages);
                                if omitted > 0 {
                                    ui.weak(format!("{omitted} messages omitted"));
                                }
                                if loaded.scripts.diagnostics.is_empty()
                                    && loaded.scripts.console.is_empty()
                                {
                                    ui.weak("No console messages or JavaScript errors.");
                                }
                            });
                    }
                    Tab::Network => {
                        ui.label(format!(
                            "{} of {} resources loaded",
                            loaded.resources.loaded, loaded.resources.attempted
                        ));
                        if loaded.resources.limited {
                            ui.weak("Resource loading reached a browser limit.");
                        }
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for message in &loaded.resources.diagnostics {
                                    ui.colored_label(ui.visuals().error_fg_color, message);
                                }
                                if loaded.resources.diagnostics.is_empty()
                                    && loaded.resources.attempted == 0
                                {
                                    ui.weak("No external resources were requested.");
                                }
                            });
                    }
                }
            });
    }
}
