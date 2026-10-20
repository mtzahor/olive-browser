use crate::gui::{
    downloads::{Downloads, format_bytes, fraction},
    favicons::Favicons,
    new_tab::{host_label, monogram, site_icon_badge},
    profile::Profile,
    theme::{Palette, Theme},
    ui_icons::{Icon, IconButton, quiet_text_button},
};
use eframe::egui::{self, RichText};

#[derive(Default)]
pub struct DailyWindows {
    pub bookmarks: bool,
    pub settings: bool,
}

/// A titled group of related settings on a card.
fn section(
    ui: &mut egui::Ui,
    palette: Palette,
    icon: Icon,
    title: &str,
    add: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::new()
        .fill(palette.canvas)
        .stroke(egui::Stroke::new(1.0, palette.border))
        .corner_radius(10)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::Vec2::splat(18.0), egui::Sense::hover());
                icon.paint(ui.painter(), rect, palette.accent);
                ui.label(RichText::new(title).size(15.0).strong());
            });
            ui.add_space(2.0);
            add(ui);
        });
    ui.add_space(10.0);
}

/// Light and Dark as two joined toggle buttons.
fn theme_choice(ui: &mut egui::Ui, value: &mut Theme) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (theme, icon, label) in [
            (Theme::Light, Icon::Sun, "Light"),
            (Theme::Dark, Icon::Moon, "Dark"),
        ] {
            if ui
                .add(IconButton::new(icon, label).selected(*value == theme))
                .clicked()
            {
                *value = theme;
            }
        }
    });
}

impl DailyWindows {
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        profile: &mut Profile,
        downloads: &mut Downloads,
        favicons: &mut Favicons,
    ) -> Option<String> {
        let palette = profile.data.settings.browser_theme.palette();
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
                            let title = if bookmark.title.is_empty() {
                                &bookmark.url
                            } else {
                                &bookmark.title
                            };
                            let (disc, _) = ui
                                .allocate_exact_size(egui::vec2(32.0, 48.0), egui::Sense::hover());
                            match favicons.get(ctx, &bookmark.url) {
                                Some(icon) => site_icon_badge(ui, disc.center(), 14.0, icon.id()),
                                None => monogram(
                                    ui,
                                    disc.center(),
                                    14.0,
                                    &host_label(&bookmark.url),
                                    title,
                                ),
                            }
                            let text_width = (ui.available_width() - 48.0).max(120.0);
                            ui.vertical(|ui| {
                                ui.set_width(text_width);
                                ui.spacing_mut().item_spacing.y = 0.0;
                                if quiet_text_button(ui, RichText::new(title).strong(), text_width)
                                    .clicked()
                                {
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
                                .add(IconButton::icon_only(Icon::Trash, "Remove bookmark").quiet())
                                .on_hover_text("Remove bookmark")
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
                .default_width(440.0)
                .default_height(520.0)
                .max_height((ctx.content_rect().height() - 80.0).max(160.0))
                .vscroll(true)
                .show(ctx, |ui| {
                    section(ui, palette, Icon::Sun, "Appearance", |ui| {
                        ui.label(RichText::new("Make Olive feel at home.").weak());
                        ui.horizontal(|ui| {
                            ui.label("Browser theme");
                            theme_choice(ui, &mut profile.data.settings.browser_theme);
                        });
                        ui.label(
                            RichText::new(
                                "Applies to browser controls. Pages keep their own colors.",
                            )
                            .small()
                            .weak(),
                        );
                    });
                    section(ui, palette, Icon::Globe, "Browsing", |ui| {
                        ui.checkbox(
                            &mut profile.data.settings.javascript_by_default,
                            "Enable JavaScript by default on page loads",
                        );
                        ui.label(
                            RichText::new("Applies to page loads, reloads, redirects, form responses and restored tabs. Use only with sites you trust: workers can still read local files and use the network.")
                                .small()
                                .weak(),
                        );
                        ui.checkbox(
                            &mut profile.data.settings.restore_session,
                            "Restore tabs from the previous session",
                        );
                        ui.add(
                            egui::Slider::new(
                                &mut profile.data.settings.default_zoom,
                                crate::gui::profile::MIN_ZOOM..=crate::gui::profile::MAX_ZOOM,
                            )
                            .suffix("%")
                            .text("Default page zoom"),
                        );
                    });
                    section(ui, palette, Icon::Focus, "Focus reading", |ui| {
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
                                    crate::gui::focus::MIN_FONT_SIZE
                                        ..=crate::gui::focus::MAX_FONT_SIZE,
                                )
                                .suffix(" px"),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("Reading theme");
                            theme_choice(ui, &mut profile.data.settings.focus.theme);
                        });
                    });
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
                        ui.label(
                            RichText::new("Use Save page… in the browser menu to keep a copy.")
                                .small()
                                .weak(),
                        );
                    }
                    let mut cancel_id = None;
                    for item in &downloads.items {
                        ui.push_id(item.id, |ui| {
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::Vec2::splat(18.0),
                                    egui::Sense::hover(),
                                );
                                Icon::Download.paint(ui.painter(), rect, palette.accent);
                                ui.add(
                                    egui::Label::new(RichText::new(&item.name).strong()).truncate(),
                                )
                                .on_hover_text(&item.name);
                            });
                            ui.add(
                                egui::Label::new(RichText::new(&item.url).small().weak())
                                    .truncate(),
                            )
                            .on_hover_text(&item.url);
                            ui.add(
                                egui::ProgressBar::new(fraction(item.received, item.total))
                                    .corner_radius(6)
                                    .text(match item.total {
                                        Some(total) => format!(
                                            "{} of {}",
                                            format_bytes(item.received),
                                            format_bytes(total)
                                        ),
                                        None => format_bytes(item.received),
                                    }),
                            );
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(item.state.to_string()).small());
                                if item.state.active() && ui.button("Cancel").clicked() {
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
