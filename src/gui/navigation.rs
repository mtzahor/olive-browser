use olive_html::net::Location;

#[derive(Clone, Copy, Debug)]
pub enum Navigation {
    New,
    Traverse(usize),
    Reload,
}

/// Only committed documents enter history. A failed or cancelled load leaves
/// both the current entry and the forward branch intact.
#[derive(Default)]
pub struct History {
    entries: Vec<Location>,
    cursor: Option<usize>,
}

impl History {
    pub fn back(&self) -> Option<(usize, Location)> {
        let index = self.cursor?.checked_sub(1)?;
        Some((index, self.entries[index].clone()))
    }
    pub fn forward(&self) -> Option<(usize, Location)> {
        let index = self.cursor? + 1;
        self.entries
            .get(index)
            .cloned()
            .map(|location| (index, location))
    }
    pub fn commit(&mut self, location: Location, navigation: Navigation) {
        match navigation {
            Navigation::New => {
                self.entries
                    .truncate(self.cursor.map_or(0, |index| index + 1));
                self.entries.push(location);
                if self.entries.len() > 256 {
                    self.entries.remove(0);
                }
                self.cursor = Some(self.entries.len() - 1);
            }
            Navigation::Traverse(index) => {
                if let Some(entry) = self.entries.get_mut(index) {
                    *entry = location;
                    self.cursor = Some(index);
                }
            }
            Navigation::Reload => {
                if let Some(index) = self.cursor {
                    self.entries[index] = location;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn location(path: &str) -> Location {
        Location::from_input(&format!("https://example.com/{path}")).unwrap()
    }

    #[test]
    fn commits_traversal_reload_and_new_branches() {
        let mut history = History::default();
        for path in ["a", "b", "c"] {
            history.commit(location(path), Navigation::New);
        }
        let (index, previous) = history.back().unwrap();
        assert_eq!(previous, location("b"));
        // Until a load succeeds, querying a target does not change history.
        assert!(history.forward().is_none());
        assert_eq!(history.cursor, Some(2));
        history.commit(previous, Navigation::Traverse(index));
        assert_eq!(history.forward().unwrap().1, location("c"));
        history.commit(location("redirected-b"), Navigation::Reload);
        assert_eq!(history.entries.len(), 3);
        history.commit(location("d"), Navigation::New);
        assert!(history.forward().is_none());
        assert_eq!(history.back().unwrap().1, location("redirected-b"));
    }

    #[test]
    fn history_is_bounded() {
        let mut history = History::default();
        for index in 0..300 {
            history.commit(location(&index.to_string()), Navigation::New);
        }
        assert_eq!(history.entries.len(), 256);
        assert_eq!(history.cursor, Some(255));
        assert_eq!(history.entries[0], location("44"));
    }
}
