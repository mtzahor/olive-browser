use crate::{
    history::{BrowsingHistory, MAX_ENTRIES},
    ui_icons::{Icon, IconButton},
};
use eframe::egui::{self, RichText};
use olive_html::net::Location;

#[derive(Default)]
pub struct HistoryWindow {
    open: bool,
    query: String,
    confirm_clear: bool,
    focus_search: bool,
}

impl HistoryWindow {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.focus_search = self.open;
        self.confirm_clear = false;
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        history: &mut BrowsingHistory,
        busy: bool,
    ) -> Option<Location> {
        if !self.open {
            return None;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            if self.confirm_clear {
                self.confirm_clear = false;
            } else {
                self.open = false;
                return None;
            }
        }
        let mut selected = None;
        let mut remove = None;
        let mut open = self.open;
        egui::Window::new("Browsing history")
            .open(&mut open)
            .collapsible(false)
            .default_width(620.0)
            .default_height(440.0)
            .min_width(300.0)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Search titles and addresses")
                        .char_limit(256)
                        .desired_width(f32::INFINITY),
                );
                if self.focus_search {
                    search.request_focus();
                    self.focus_search = false;
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} saved {} · newest first",
                            history.entries().len(),
                            if history.entries().len() == 1 {
                                "page"
                            } else {
                                "pages"
                            }
                        ))
                        .weak(),
                    );
                    if ui
                        .add_enabled(
                            history.can_clear(),
                            IconButton::new(Icon::Trash, "Clear all…"),
                        )
                        .clicked()
                    {
                        self.confirm_clear = true;
                    }
                });
                if self.confirm_clear {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.label("Clear all saved browsing history? This cannot be undone.");
                        ui.label(
                            RichText::new("The current page and Back/Forward remain available.")
                                .small()
                                .weak(),
                        );
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add(IconButton::new(Icon::Trash, "Clear all history"))
                                .clicked()
                            {
                                history.clear();
                                self.confirm_clear = false;
                            }
                            if ui.add(IconButton::new(Icon::Close, "Cancel")).clicked() {
                                self.confirm_clear = false;
                            }
                        });
                    });
                }
                if let Some(error) = history.error() {
                    ui.colored_label(egui::Color32::from_rgb(160, 55, 35), error);
                    if history.can_retry()
                        && ui
                            .add(IconButton::new(Icon::Reload, "Retry saving"))
                            .clicked()
                    {
                        history.save();
                    }
                }
                let matches = history.search(&self.query);
                if history.entries().is_empty() {
                    ui.add_space(22.0);
                    ui.label("No browsing history yet.");
                    ui.label(RichText::new("Pages you open will appear here.").weak());
                } else if matches.is_empty() {
                    ui.add_space(22.0);
                    ui.label("No pages match your search.");
                } else {
                    if !self.query.trim().is_empty() {
                        ui.label(
                            RichText::new(format!(
                                "{} matching {}",
                                matches.len(),
                                if matches.len() == 1 { "page" } else { "pages" }
                            ))
                            .small()
                            .weak(),
                        );
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("saved-history")
                        .auto_shrink([false, false])
                        .max_height((ctx.content_rect().height() - 300.0).max(100.0))
                        .show_rows(ui, 82.0, matches.len(), |ui, range| {
                            for index in range {
                                let entry = matches[index];
                                ui.push_id(&entry.url, |ui| {
                                    ui.set_min_height(82.0);
                                    ui.spacing_mut().item_spacing.y = 4.0;
                                    ui.spacing_mut().button_padding.y = 4.0;
                                    ui.horizontal(|ui| {
                                        let text_width = (ui.available_width() - 56.0).max(140.0);
                                        ui.vertical(|ui| {
                                            ui.set_width(text_width);
                                            let mut open_button = IconButton::new(
                                                Icon::Page,
                                                RichText::new(entry.display_title()).strong(),
                                            );
                                            open_button.button = open_button
                                                .button
                                                .frame(false)
                                                .truncate()
                                                .min_size(egui::vec2(text_width, 22.0));
                                            if ui
                                                .add_enabled(!busy, open_button)
                                                .on_hover_text(format!("Open {}", entry.url))
                                                .clicked()
                                            {
                                                selected = Location::from_input(&entry.url).ok();
                                            }
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&entry.url).small().weak(),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(&entry.url);
                                            let visits = if entry.visits == 1 {
                                                "1 visit".into()
                                            } else {
                                                format!("{} visits", entry.visits)
                                            };
                                            let metadata =
                                                format!("{} · {visits}", entry.visited_label());
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&metadata).small().weak(),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(metadata);
                                        });
                                        if ui
                                            .add(IconButton::icon_only(
                                                Icon::Trash,
                                                "Remove from history",
                                            ))
                                            .on_hover_text("Remove this address from saved history")
                                            .clicked()
                                        {
                                            remove = Some(entry.url.clone());
                                        }
                                    });
                                    ui.separator();
                                });
                            }
                        });
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!(
                        "Keeps up to {MAX_ENTRIES} addresses on this computer."
                    ))
                    .small()
                    .weak(),
                );
                if let Some(path) = history.path() {
                    ui.add(
                        egui::Label::new(RichText::new(path.display().to_string()).small().weak())
                            .truncate(),
                    )
                    .on_hover_text(path.display().to_string());
                }
            });
        if let Some(url) = remove {
            history.remove(&url);
        }
        self.open = open && selected.is_none();
        if !self.open {
            self.confirm_clear = false;
        }
        selected
    }
}
