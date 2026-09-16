use crate::{
    downloads::{Downloads, fraction},
    profile::Profile,
    theme::Theme,
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
                .collapsible(false)
                .default_width(560.0)
                .max_height((ctx.content_rect().height() - 80.0).max(160.0))
                .vscroll(true)
                .show(ctx, |ui| {
                    if profile.data.bookmarks.is_empty() {
                        ui.label(
                            "No bookmarks yet. Use the star beside the address bar to save a page.",
                        );
                    }
                    for bookmark in profile.data.bookmarks.clone() {
                        ui.horizontal(|ui| {
                            let text_width = (ui.available_width() - 48.0).max(120.0);
                            ui.vertical(|ui| {
                                ui.set_width(text_width);
                                let mut button = IconButton::new(
                                    Icon::Page,
                                    RichText::new(if bookmark.title.is_empty() {
                                        &bookmark.url
                                    } else {
                                        &bookmark.title
                                    })
                                    .strong(),
                                )
                                .quiet();
                                button.button = button
                                    .button
                                    .truncate()
                                    .min_size(egui::vec2(text_width, 32.0));
                                if ui.add(button).clicked() {
                                    open_url = Some(bookmark.url.clone());
                                }
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(bookmark.url.clone()).small().weak(),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(bookmark.url.clone());
                            });
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
                .collapsible(false)
                .default_width(400.0)
                .default_height(500.0)
                .max_height((ctx.content_rect().height() - 80.0).max(160.0))
                .vscroll(true)
                .show(ctx, |ui| {
                    ui.heading("Appearance");
                    ui.label(RichText::new("Make Olive feel at home.").weak());
                    ui.horizontal(|ui| {
                        ui.label("Browser theme");
                        for (theme, icon, label) in [
                            (Theme::Light, Icon::Sun, "Light"),
                            (Theme::Dark, Icon::Moon, "Dark"),
                        ] {
                            let mut button = IconButton::new(icon, label);
                            button.button = button
                                .button
                                .selected(profile.data.settings.browser_theme == theme);
                            if ui.add(button).clicked() {
                                profile.data.settings.browser_theme = theme;
                            }
                        }
                    });
                    ui.label(
                        RichText::new("Applies to browser controls. Pages keep their own colors.")
                            .small()
                            .weak(),
                    );
                    ui.add_space(8.0);
                    ui.separator();
                    ui.heading("Browsing");
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
                    ui.add_space(8.0);
                    ui.separator();
                    ui.heading("Focus reading");
                    ui.label(
                        RichText::new(
                            "Defaults for new tabs; adjust each reading view in Appearance.",
                        )
                        .small()
                        .weak(),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Font size");
                        ui.add(
                            egui::Slider::new(
                                &mut profile.data.settings.focus.font_size,
                                crate::focus::MIN_FONT_SIZE..=crate::focus::MAX_FONT_SIZE,
                            )
                            .suffix(" px"),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Reading theme");
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
                    ui.add_space(8.0);
                    ui.separator();
                    if let Some(error) = &profile.error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                        if ui.button("Retry saving").clicked() {
                            profile.save();
                        }
                        if profile.load_failed && ui.button("Reset saved profile").clicked() {
                            profile.reset();
                        }
                    }
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!(
                                "Profile: {}",
                                profile
                                    .path()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_else(|| "session only".into())
                            ))
                            .small()
                            .weak(),
                        )
                        .wrap(),
                    );
                });
            self.settings = open;
        }
        if downloads.open {
            let mut open = true;
            egui::Window::new("Downloads")
                .open(&mut open)
                .collapsible(false)
                .default_width(600.0)
                .max_height((ctx.content_rect().height() - 80.0).max(160.0))
                .vscroll(true)
                .show(ctx, |ui| {
                    if downloads.items.is_empty() {
                        ui.label("No downloads yet.");
                    }
                    let mut cancel_id = None;
                    for item in &downloads.items {
                        ui.push_id(item.id, |ui| {
                            ui.add(egui::Label::new(RichText::new(&item.name).strong()).truncate())
                                .on_hover_text(&item.name);
                            ui.add(
                                egui::Label::new(RichText::new(&item.url).small().weak())
                                    .truncate(),
                            )
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
