//! Case-insensitive search over rendered text, retaining original character offsets.
use crate::gui::{
    glass::{self, Band},
    theme::Theme,
    ui_icons::{self, Icon, IconButton},
};
use eframe::egui;
use std::ops::Range;

#[derive(Default)]
pub struct FindBar {
    pub open: bool,
    pub query: String,
    pub current: usize,
    pub focus: bool,
    pub scroll: bool,
}
impl FindBar {
    /// Show the bar with its query focused and the current match scrolled into view.
    pub fn activate(&mut self) {
        self.open = true;
        self.focus = true;
        self.scroll = true;
    }
    pub fn show(&mut self, ui: &mut egui::Ui, count: usize) {
        let palette = Theme::from_visuals(ui.visuals()).palette();
        egui::Panel::top("find-bar")
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 12,
                right: 12,
                top: 0,
                bottom: 8,
            }))
            .show(ui, |ui| {
                glass::in_band(ui, Band::Top, |ui| {
                    glass::capsule(ui, egui::Margin::symmetric(6, 4), 20, &palette, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            let response = ui_icons::search_field(
                                ui,
                                egui::TextEdit::singleline(&mut self.query)
                                    .id_salt("find-query")
                                    .hint_text("Find in page")
                                    .char_limit(256)
                                    .desired_width(
                                        (ui.available_width() - 240.0).clamp(120.0, 320.0),
                                    ),
                            );
                            if self.focus {
                                response.request_focus();
                                self.focus = false;
                            }
                            if (response.has_focus() || response.lost_focus())
                                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            {
                                let previous = ui.input(|i| i.modifiers.shift);
                                self.step(count, previous);
                                response.request_focus();
                            }
                            if response.changed() {
                                self.current = 0;
                                self.scroll = true;
                            }
                            ui.add_space(4.0);
                            let (status, color) = if self.query.is_empty() {
                                ("Type to search".into(), palette.muted)
                            } else if count == 0 {
                                ("No matches".into(), palette.warning)
                            } else {
                                (
                                    format!("{} of {}", self.current.min(count - 1) + 1, count),
                                    palette.ink,
                                )
                            };
                            ui.label(egui::RichText::new(status).size(13.0).color(color));
                            ui.add_space(4.0);
                            if ui
                                .add_enabled(
                                    count > 0,
                                    IconButton::icon_only(Icon::ChevronUp, "Previous").quiet(),
                                )
                                .on_hover_text("Previous match (Shift+Enter)")
                                .clicked()
                            {
                                self.step(count, true);
                            }
                            if ui
                                .add_enabled(
                                    count > 0,
                                    IconButton::icon_only(Icon::ChevronDown, "Next").quiet(),
                                )
                                .on_hover_text("Next match (Enter)")
                                .clicked()
                            {
                                self.step(count, false);
                            }
                            if ui
                                .add(IconButton::icon_only(Icon::Close, "Close").quiet())
                                .on_hover_text("Close find (Escape)")
                                .clicked()
                            {
                                self.open = false;
                            }
                        });
                    });
                });
            });
    }
    pub fn step(&mut self, count: usize, previous: bool) {
        if count > 0 {
            self.current = if previous {
                (self.current + count - 1) % count
            } else {
                (self.current + 1) % count
            };
            self.scroll = true;
        }
    }
}
#[derive(Default)]
pub struct Matches {
    query: String,
    pub blocks: Vec<Vec<Range<usize>>>,
    pub count: usize,
}
impl Matches {
    pub fn update<'a>(&mut self, query: &str, texts: impl Iterator<Item = &'a str>) {
        if self.query == query {
            return;
        }
        self.query = query.into();
        self.count = 0;
        self.blocks = texts
            .map(|text| {
                let matches = ranges(text, query);
                self.count += matches.len();
                matches
            })
            .collect();
    }
}
fn ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let query = query.to_lowercase();
    let mut folded = String::new();
    let mut offsets = Vec::new();
    for (index, ch) in text.chars().enumerate() {
        for lower in ch.to_lowercase() {
            for _ in 0..lower.len_utf8() {
                offsets.push(index);
            }
            folded.push(lower);
        }
    }
    let mut result = Vec::<Range<usize>>::new();
    for (start, _) in folded.match_indices(&query) {
        let range = offsets[start]..offsets[start + query.len() - 1] + 1;
        if result.last().is_none_or(|last| last.end <= range.start) {
            result.push(range);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_offsets_case_and_nonoverlapping_matches() {
        assert_eq!(ranges("é OLIVE olive שלום", "Olive"), vec![2..7, 8..13]);
        assert_eq!(ranges("é OLIVE olive שלום", "שלום"), vec![14..18]);
        assert_eq!(ranges("İstanbul", "i"), vec![0..1]);
        assert_eq!(ranges("aaaa", "aa"), vec![0..2, 2..4]);
        assert!(ranges("text", "").is_empty());
    }
    #[test]
    fn stepping_wraps_and_empty_results_are_safe() {
        let mut bar = FindBar::default();
        bar.step(3, true);
        assert_eq!(bar.current, 2);
        bar.step(3, false);
        assert_eq!(bar.current, 0);
        bar.step(0, false);
        assert_eq!(bar.current, 0);
    }
}
