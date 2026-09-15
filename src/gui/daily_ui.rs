use crate::{
    downloads::{Downloads, fraction},
    profile::Profile,
    ui_icons::{Icon, IconButton},
};
use eframe::egui::{self, RichText};

#[derive(Default)]
pub struct DailyWindows {
    pub bookmarks: bool,
    pub settings: bool,
}
impl DailyWindows {
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        profile: &mut Profile,
        downloads: &mut Downloads,
    ) -> Option<String> {
        let mut open_url = None;
        if self.bookmarks {
            let mut open = true;
            egui::Window::new("Bookmarks")
                .open(&mut open)
                .default_width(560.0)
                .show(ctx, |ui| {
                    if profile.data.bookmarks.is_empty() {
                        ui.label(
                            "No bookmarks yet. Use the star beside the address bar to save a page.",
                        );
                    }
                    for bookmark in profile.data.bookmarks.clone() {
                        ui.horizontal(|ui| {
                            if ui
                                .add(IconButton::new(
                                    Icon::Page,
                                    RichText::new(if bookmark.title.is_empty() {
                                        &bookmark.url
                                    } else {
                                        &bookmark.title
                                    })
                                    .strong(),
                                ))
                                .clicked()
                            {
                                open_url = Some(bookmark.url.clone());
                            }
                            ui.label(RichText::new(bookmark.url.clone()).small().weak())
                                .on_hover_text(bookmark.url.clone());
                            if ui
                                .add(IconButton::icon_only(Icon::Trash, "Remove bookmark"))
                                .clicked()
                            {
                                profile
                                    .data
                                    .bookmarks
                                    .retain(|entry| entry.url != bookmark.url);
                            }
                        });
                        ui.separator();
                    }
                });
            self.bookmarks = open && open_url.is_none();
        }
        if self.settings {
            let mut open = true;
            egui::Window::new("Settings")
                .open(&mut open)
                .default_width(520.0)
                .show(ctx, |ui| {
                    ui.heading("Daily-driver settings");
                    ui.checkbox(
                        &mut profile.data.settings.restore_session,
                        "Restore tabs from the previous session",
                    );
                    ui.add(
                        egui::Slider::new(
                            &mut profile.data.settings.default_zoom,
                            crate::profile::MIN_ZOOM..=crate::profile::MAX_ZOOM,
                        )
                        .suffix("%")
                        .text("Default page zoom"),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Focus font size");
                        ui.add(
                            egui::Slider::new(
                                &mut profile.data.settings.focus.font_size,
                                crate::focus::MIN_FONT_SIZE..=crate::focus::MAX_FONT_SIZE,
                            )
                            .suffix(" px"),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Focus theme");
                        ui.selectable_value(
                            &mut profile.data.settings.focus.theme,
                            crate::focus::Theme::Light,
                            "Light",
                        );
                        ui.selectable_value(
                            &mut profile.data.settings.focus.theme,
                            crate::focus::Theme::Dark,
                            "Dark",
                        );
                    });
                    if let Some(error) = &profile.error {
                        ui.colored_label(egui::Color32::from_rgb(160, 55, 35), error);
                        if ui.button("Retry saving").clicked() {
                            profile.save();
                        }
                        if profile.load_failed && ui.button("Reset saved profile").clicked() {
                            profile.reset();
                        }
                    }
                    ui.small(format!(
                        "Profile: {}",
                        profile
                            .path()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "session only".into())
                    ));
                });
            self.settings = open;
        }
        if downloads.open {
            let mut open = true;
            egui::Window::new("Downloads")
                .open(&mut open)
                .default_width(600.0)
                .show(ctx, |ui| {
                    if downloads.items.is_empty() {
                        ui.label("No downloads yet.");
                    }
                    let mut cancel_id = None;
                    for item in &downloads.items {
                        ui.push_id(item.id, |ui| {
                            ui.label(RichText::new(&item.name).strong());
                            ui.label(RichText::new(&item.url).small().weak())
                                .on_hover_text(&item.url);
                            ui.add(
                                egui::ProgressBar::new(fraction(item.received, item.total)).text(
                                    format!(
                                        "{} / {}",
                                        item.received,
                                        item.total
                                            .map(|n| n.to_string())
                                            .unwrap_or_else(|| "?".into())
                                    ),
                                ),
                            );
                            ui.horizontal(|ui| {
                                ui.label(&item.state);
                                if (item.state == "Downloading…" || item.state == "Starting…")
                                    && ui.button("Cancel").clicked()
                                {
                                    cancel_id = Some(item.id);
                                }
                            });
                            ui.separator();
                        });
                    }
                    if let Some(id) = cancel_id {
                        downloads.cancel(id);
                    }
                });
            downloads.open = open;
        }
        open_url
    }
}
