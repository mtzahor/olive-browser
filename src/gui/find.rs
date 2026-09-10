//! Case-insensitive search over rendered text, retaining original character offsets.
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
    pub fn show(&mut self, ui: &mut egui::Ui, count: usize) {
        egui::Panel::top("find-bar").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Find");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .id_salt("find-query")
                        .hint_text("Find in page")
                        .char_limit(256)
                        .desired_width((ui.available_width() - 280.0).clamp(60.0, 300.0)),
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
                ui.label(if self.query.is_empty() {
                    "Type to search".into()
                } else if count == 0 {
                    "No matches".into()
                } else {
                    format!("{} of {}", self.current.min(count - 1) + 1, count)
                });
                if ui
                    .add_enabled(count > 0, egui::Button::new("Previous"))
                    .clicked()
                {
                    self.step(count, true);
                }
                if ui
                    .add_enabled(count > 0, egui::Button::new("Next"))
                    .clicked()
                {
                    self.step(count, false);
                }
                if ui.button("Close").clicked() {
                    self.open = false;
                }
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
