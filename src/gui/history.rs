//! Saved, searchable browsing history, separate from the back/forward stack.

use olive_html::net::{Location, Url};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use time::OffsetDateTime;

pub const MAX_ENTRIES: usize = 1_000;
const MAX_TITLE_BYTES: usize = 1_024;
// Also bounds deserialization of a damaged or externally edited file. Valid
// entries (8 KiB URLs plus 1 KiB titles, including JSON escaping) fit below this.
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    pub last_visited: i64,
    pub visits: u64,
}

impl HistoryEntry {
    pub fn display_title(&self) -> &str {
        if self.title.is_empty() {
            &self.url
        } else {
            &self.title
        }
    }

    pub fn visited_label(&self) -> String {
        let Ok(date) = OffsetDateTime::from_unix_timestamp(self.last_visited) else {
            return "Unknown visit time".into();
        };
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02} UTC",
            date.year(),
            u8::from(date.month()),
            date.day(),
            date.hour(),
            date.minute()
        )
    }
}

#[derive(Deserialize, Serialize)]
struct HistoryFile {
    version: u32,
    entries: Vec<HistoryEntry>,
}

#[derive(Default)]
pub struct BrowsingHistory {
    // Newest first; one entry per complete URL, including its fragment.
    entries: Vec<HistoryEntry>,
    path: Option<PathBuf>,
    error: Option<String>,
    // Never silently overwrite an unreadable file. Explicit Clear all permits
    // replacing it; successful visits can still be searched during this session.
    load_failed: bool,
}

impl BrowsingHistory {
    pub fn load_default() -> Self {
        match default_path() {
            Ok(path) => Self::load(path),
            Err(error) => Self {
                error: Some(error),
                ..Self::default()
            },
        }
    }

    pub fn load(path: PathBuf) -> Self {
        let mut history = Self {
            path: Some(path.clone()),
            ..Self::default()
        };
        match read_entries(&path) {
            Ok(entries) => history.entries = entries,
            Err(error) => {
                history.load_failed = true;
                history.error = Some(format!(
                    "Could not read saved history: {error}. Visits are kept for this session. \
                     Clear all history to replace the unreadable file."
                ));
            }
        }
        history
    }

    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    pub fn search(&self, query: &str) -> Vec<&HistoryEntry> {
        let query = query.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|entry| {
                query.is_empty()
                    || entry.title.to_lowercase().contains(&query)
                    || entry.url.to_lowercase().contains(&query)
            })
            .collect()
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn can_retry(&self) -> bool {
        self.error.is_some() && self.path.is_some() && !self.load_failed
    }

    pub fn can_clear(&self) -> bool {
        !self.entries.is_empty() || self.load_failed
    }

    /// Call only after committing a document or same-document navigation.
    /// Reloads and traversals update the existing URL's visit count and recency.
    pub fn record(&mut self, location: &Location, title: &str) {
        self.record_at(location, title, OffsetDateTime::now_utc().unix_timestamp());
        self.save();
    }

    fn record_at(&mut self, location: &Location, title: &str, timestamp: i64) {
        let visits = self
            .entries
            .iter()
            .position(|entry| entry.url == location.as_str())
            .map(|index| self.entries.remove(index).visits.saturating_add(1))
            .unwrap_or(1);
        self.entries.insert(
            0,
            HistoryEntry {
                url: location.as_str().into(),
                title: bounded_title(title),
                last_visited: timestamp,
                visits,
            },
        );
        self.entries.truncate(MAX_ENTRIES);
    }

    /// A script changing the current title is not a new visit. In particular,
    /// updating a page after its entry was deleted must not re-create history.
    pub fn update_title(&mut self, location: &Location, title: &str) {
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.url == location.as_str())
        else {
            return;
        };
        let title = bounded_title(title);
        if entry.title != title {
            entry.title = title;
            self.save();
        }
    }

    pub fn remove(&mut self, url: &str) {
        self.entries.retain(|entry| entry.url != url);
        self.save();
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.load_failed = false;
        self.save();
    }

    pub fn save(&mut self) {
        if self.load_failed {
            return;
        }
        let Some(path) = &self.path else {
            return;
        };
        self.error = write_entries(path, &self.entries).err().map(|error| {
            format!(
                "History changes could not be saved: {error}. \
                 The saved file may still contain older entries. Retry saving when storage is available."
            )
        });
    }
}

fn bounded_title(title: &str) -> String {
    let mut result = String::new();
    for word in title.split_whitespace() {
        if !result.is_empty() && result.len() < MAX_TITLE_BYTES {
            result.push(' ');
        }
        for ch in word.chars().filter(|ch| !ch.is_control()) {
            if result.len() + ch.len_utf8() > MAX_TITLE_BYTES {
                return result.trim_end().to_owned();
            }
            result.push(ch);
        }
    }
    result.trim().to_owned()
}

fn read_entries(path: &Path) -> Result<Vec<HistoryEntry>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_FILE_BYTES {
        return Err("history file exceeds the 32 MiB limit".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("history file exceeds the 32 MiB limit".into());
    }
    let mut saved: HistoryFile = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if saved.version != FORMAT_VERSION {
        return Err("unsupported history format version".into());
    }
    if saved.entries.len() > MAX_ENTRIES {
        return Err(format!("history exceeds the {MAX_ENTRIES}-page limit"));
    }
    let mut seen = HashSet::new();
    for entry in &mut saved.entries {
        // Parse stored URLs explicitly: an edited bare hostname or path must not
        // be interpreted as an address-bar input when restoring history.
        let location = Location::from_url(Url::parse(&entry.url).map_err(|e| e.to_string())?)?;
        entry.url = location.as_str().into();
        if !seen.insert(entry.url.clone()) {
            return Err("duplicate history URL".into());
        }
        if entry.visits == 0
            || entry.last_visited < 0
            || OffsetDateTime::from_unix_timestamp(entry.last_visited).is_err()
        {
            return Err("invalid history visit metadata".into());
        }
        entry.title = bounded_title(&entry.title);
    }
    saved
        .entries
        .sort_by_key(|entry| std::cmp::Reverse(entry.last_visited));
    Ok(saved.entries)
}

fn write_entries(path: &Path, entries: &[HistoryEntry]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    // A file in the same directory allows atomic replacement on all supported
    // platforms. tempfile creates it with owner-only permissions on Unix.
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    #[derive(Serialize)]
    struct Saved<'a> {
        version: u32,
        entries: &'a [HistoryEntry],
    }
    serde_json::to_writer(
        &mut file,
        &Saved {
            version: FORMAT_VERSION,
            entries,
        },
    )
    .map_err(|e| e.to_string())?;
    file.write_all(b"\n").map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

fn default_path() -> Result<PathBuf, String> {
    let env_path = |name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if let Some(path) = env_path("OLIVE_HISTORY_FILE") {
        return Ok(path);
    }
    #[cfg(target_os = "macos")]
    let directory =
        env_path("HOME").map(|home| home.join("Library/Application Support/Olive Browser"));
    #[cfg(target_os = "windows")]
    let directory = env_path("LOCALAPPDATA").map(|data| data.join("Olive Browser"));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let directory = env_path("XDG_DATA_HOME")
        .filter(|path| path.is_absolute())
        .or_else(|| env_path("HOME").map(|home| home.join(".local/share")))
        .map(|data| data.join("olive-browser"));
    directory
        .map(|directory| directory.join("history.json"))
        .ok_or_else(|| {
            "Could not find a history storage directory. History is kept for this session only."
                .into()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(path: &str) -> Location {
        Location::from_input(&format!("https://example.com/{path}")).unwrap()
    }

    #[test]
    fn revisits_update_recency_title_and_count_without_duplicate_rows() {
        let mut history = BrowsingHistory::default();
        history.record_at(&location("a"), "First", 10);
        history.record_at(&location("b"), "Second", 20);
        history.record_at(&location("a"), "Renamed", 30);
        assert_eq!(history.entries.len(), 2);
        let entry = &history.entries[0];
        assert_eq!(entry.url, location("a").as_str());
        assert_eq!(entry.title, "Renamed");
        assert_eq!(entry.last_visited, 30);
        assert_eq!(entry.visits, 2);
        assert_eq!(history.search("  RENAMED ").len(), 1);
        assert_eq!(history.search("EXAMPLE.COM").len(), 2);
        assert!(history.search("missing").is_empty());
        assert_eq!(entry.visited_label(), "1970-01-01 00:00 UTC");
    }

    #[test]
    fn bounds_entries_and_utf8_titles_and_keeps_fragments_distinct() {
        let mut history = BrowsingHistory::default();
        for index in 0..MAX_ENTRIES + 2 {
            history.record_at(&location(&index.to_string()), "", index as i64);
        }
        assert_eq!(history.entries.len(), MAX_ENTRIES);
        assert_eq!(history.entries.last().unwrap().url, location("2").as_str());
        history.record_at(&location("a#one"), &"שלום".repeat(500), 2000);
        history.record_at(&location("a#two"), " \n A\t title\0 ", 2001);
        assert!(history.entries[1].title.len() <= MAX_TITLE_BYTES);
        assert_eq!(history.entries[0].title, "A title");
        assert_eq!(history.search("a#").len(), 2);
    }

    #[test]
    fn persistence_survives_restart_and_removal_and_clear_do_not_restore_entries() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile/history.json");
        let mut history = BrowsingHistory::load(path.clone());
        assert!(history.error().is_none());
        history.record(&location("a"), "Hello 🫒");
        let local = Location::from_path(directory.path().join("local page.html")).unwrap();
        history.record(&local, "Local");
        history.record(&location("a"), "Updated");
        assert!(history.error().is_none());
        let mut restored = BrowsingHistory::load(path.clone());
        assert_eq!(restored.entries.len(), 2);
        assert_eq!(restored.entries[0].title, "Updated");
        assert_eq!(restored.entries[0].visits, 2);
        assert_eq!(restored.entries[1].url, local.as_str());
        restored.remove(location("a").as_str());
        restored.update_title(&location("a"), "Do not restore");
        assert_eq!(BrowsingHistory::load(path.clone()).entries.len(), 1);
        restored.clear();
        assert!(BrowsingHistory::load(path).entries.is_empty());
    }

    #[test]
    fn title_updates_do_not_change_visit_time_count_or_order() {
        let mut history = BrowsingHistory::default();
        history.record_at(&location("a"), "Original", 10);
        history.record_at(&location("b"), "", 20);
        history.update_title(&location("a"), "Script title");
        assert_eq!(history.entries[1].title, "Script title");
        assert_eq!(history.entries[1].last_visited, 10);
        assert_eq!(history.entries[1].visits, 1);
        assert_eq!(history.entries[0].display_title(), location("b").as_str());
    }

    #[test]
    fn unreadable_and_unsupported_files_are_preserved_until_explicit_clear() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.json");
        for bytes in ["broken JSON", r#"{"version":99,"entries":[]}"#] {
            fs::write(&path, bytes).unwrap();
            let mut history = BrowsingHistory::load(path.clone());
            assert!(history.error().is_some());
            assert!(!history.can_retry());
            history.record(&location("new"), "New visit");
            assert_eq!(history.entries.len(), 1);
            assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
            history.clear();
            assert!(history.error().is_none());
            assert!(BrowsingHistory::load(path.clone()).entries.is_empty());
        }
    }

    #[test]
    fn rejects_unsafe_urls_invalid_metadata_and_oversized_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.json");
        for url in [
            "javascript:alert(1)",
            "https://user:password@example.com",
            "relative.html",
            "file://remote/share",
        ] {
            fs::write(&path, serde_json::json!({
                "version": 1, "entries": [{"url": url, "title": "Bad", "last_visited": 1, "visits": 1}]
            }).to_string()).unwrap();
            assert!(
                BrowsingHistory::load(path.clone()).error().is_some(),
                "{url}"
            );
        }
        for (timestamp, visits) in [(-1, 1), (i64::MAX, 1), (1, 0)] {
            fs::write(&path, serde_json::json!({
                "version": 1, "entries": [{"url": "https://example.com", "title": "Bad", "last_visited": timestamp, "visits": visits}]
            }).to_string()).unwrap();
            assert!(BrowsingHistory::load(path.clone()).error().is_some());
        }
        File::create(&path)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        assert!(
            BrowsingHistory::load(path)
                .error()
                .unwrap()
                .contains("32 MiB")
        );
    }

    #[test]
    fn storage_failure_keeps_memory_and_can_be_retried() {
        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("blocked");
        let mut history = BrowsingHistory::load(parent.join("history.json"));
        fs::write(&parent, "not a directory").unwrap();
        history.record(&location("a"), "Still readable");
        assert_eq!(history.entries.len(), 1);
        assert!(history.can_retry());
        fs::remove_file(&parent).unwrap();
        history.save();
        assert!(history.error().is_none());
        assert_eq!(
            BrowsingHistory::load(parent.join("history.json"))
                .entries
                .len(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn history_file_is_private_to_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.json");
        let mut history = BrowsingHistory::load(path.clone());
        history.record(&location("private"), "Private");
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
