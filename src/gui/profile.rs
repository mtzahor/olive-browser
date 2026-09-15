//! Versioned, bounded daily-driver state. Page content, credentials and form bodies
//! never belong in this file. A damaged file is preserved until an explicit reset.
use crate::focus::{MAX_FONT_SIZE, MIN_FONT_SIZE, Settings as FocusSettings};
use olive_html::net::{Location, Url};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const MAX_BOOKMARKS: usize = 1_000;
pub const MAX_TABS: usize = 32;
pub const MIN_ZOOM: u16 = 50;
pub const MAX_ZOOM: u16 = 200;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub restore_session: bool,
    pub default_zoom: u16,
    pub focus: FocusSettings,
    pub download_directory: Option<PathBuf>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            restore_session: true,
            default_zoom: 100,
            focus: FocusSettings::default(),
            download_directory: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    pub url: Option<String>,
    pub zoom: u16,
    pub focus: FocusSettings,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub tabs: Vec<SavedTab>,
    pub active: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Data {
    version: u32,
    pub settings: Settings,
    pub bookmarks: Vec<Bookmark>,
    pub session: Session,
    #[serde(default = "clean_shutdown_default")]
    pub clean_shutdown: bool,
}

fn clean_shutdown_default() -> bool {
    true // Profiles written before v0.11 have no crash marker.
}
impl Default for Data {
    fn default() -> Self {
        Self {
            version: 1,
            settings: Settings::default(),
            bookmarks: Vec::new(),
            session: Session::default(),
            clean_shutdown: true,
        }
    }
}

#[derive(Default)]
pub struct Profile {
    pub data: Data,
    pub error: Option<String>,
    path: Option<PathBuf>,
    saved: Data,
    pub load_failed: bool,
}

impl Profile {
    pub fn load_default() -> Self {
        match crate::history::default_path() {
            Ok(path) => Self::load(path.with_file_name("profile.json")),
            Err(error) => Self {
                error: Some(error),
                ..Self::default()
            },
        }
    }

    pub fn load(path: PathBuf) -> Self {
        let mut profile = Self {
            path: Some(path.clone()),
            ..Self::default()
        };
        match read(&path) {
            Ok(data) => {
                profile.saved = data.clone();
                profile.data = data;
            }
            Err(error) => {
                profile.load_failed = true;
                profile.error = Some(format!(
                    "Could not read profile: {error}. Changes are kept in memory. Reset saved profile to replace this file."
                ));
            }
        }
        profile
    }

    /// Persist the marker before starting any document workers.
    pub fn begin_session(&mut self) -> bool {
        let interrupted = !self.data.clean_shutdown;
        self.data.clean_shutdown = false;
        self.save_if_changed();
        interrupted
    }

    pub fn finish_session(&mut self) {
        self.data.clean_shutdown = true;
        self.save_if_changed();
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.data.bookmarks.iter().any(|entry| entry.url == url)
    }

    pub fn toggle_bookmark(&mut self, location: &Location, title: &str) -> Result<(), String> {
        let url = location.as_str();
        if self.is_bookmarked(url) {
            self.data.bookmarks.retain(|entry| entry.url != url);
        } else {
            if self.data.bookmarks.len() >= MAX_BOOKMARKS {
                return Err(
                    "Remove a bookmark before adding another (1,000 bookmark limit).".into(),
                );
            }
            self.data.bookmarks.insert(
                0,
                Bookmark {
                    url: url.into(),
                    title: crate::history::bounded_title(title),
                },
            );
        }
        Ok(())
    }

    /// Failed writes are retried explicitly, not on every animation frame.
    pub fn save_if_changed(&mut self) {
        if self.saved != self.data && self.error.is_none() {
            self.save();
        }
    }

    pub fn save(&mut self) {
        if self.load_failed {
            return;
        }
        let Some(path) = &self.path else { return };
        match write(path, &self.data) {
            Ok(()) => {
                self.saved = self.data.clone();
                self.error = None;
            }
            Err(error) => {
                self.error = Some(format!(
                    "Profile changes could not be saved: {error}. Retry saving when storage is available."
                ))
            }
        }
    }

    pub fn reset(&mut self) {
        self.data = Data::default();
        self.data.clean_shutdown = false;
        self.load_failed = false;
        self.save();
    }
}

fn validate_focus(focus: &FocusSettings) -> Result<(), String> {
    if !(MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&focus.font_size) {
        return Err("invalid reading font size".into());
    }
    Ok(())
}
fn validate_zoom(zoom: u16) -> Result<(), String> {
    if !(MIN_ZOOM..=MAX_ZOOM).contains(&zoom) {
        return Err("invalid page zoom".into());
    }
    Ok(())
}
fn canonical_url(url: &str) -> Result<String, String> {
    Ok(
        Location::from_url(Url::parse(url).map_err(|e| e.to_string())?)?
            .as_str()
            .into(),
    )
}
fn read(path: &Path) -> Result<Data, String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Data::default()),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("profile exceeds the 16 MiB limit".into());
    }
    let mut data: Data = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if data.version != 1 {
        return Err("unsupported profile version".into());
    }
    if data.bookmarks.len() > MAX_BOOKMARKS || data.session.tabs.len() > MAX_TABS {
        return Err("too many saved bookmarks or tabs".into());
    }
    if data.session.active >= data.session.tabs.len().max(1) {
        return Err("invalid active tab".into());
    }
    validate_zoom(data.settings.default_zoom)?;
    validate_focus(&data.settings.focus)?;
    if data
        .settings
        .download_directory
        .as_ref()
        .is_some_and(|path| !path.is_absolute() || path.as_os_str().len() > 8192)
    {
        return Err("invalid download directory".into());
    }
    let mut seen = HashSet::new();
    for bookmark in &mut data.bookmarks {
        bookmark.url = canonical_url(&bookmark.url)?;
        bookmark.title = crate::history::bounded_title(&bookmark.title);
        if !seen.insert(bookmark.url.clone()) {
            return Err("duplicate bookmark".into());
        }
    }
    for tab in &mut data.session.tabs {
        if let Some(url) = &mut tab.url {
            *url = canonical_url(url)?;
        }
        validate_zoom(tab.zoom)?;
        validate_focus(&tab.focus)?;
    }
    Ok(data)
}
fn write(path: &Path, data: &Data) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut file, data).map_err(|e| e.to_string())?;
    file.write_all(b"\n").map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bookmarks_settings_and_session_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.json");
        let mut profile = Profile::load(path.clone());
        let location = Location::from_input("https://example.com/#one").unwrap();
        profile
            .toggle_bookmark(&location, " A\n title 🫒 ")
            .unwrap();
        profile.data.settings.default_zoom = 125;
        profile.data.session = Session {
            active: 1,
            tabs: vec![
                SavedTab {
                    url: Some(location.as_str().into()),
                    zoom: 150,
                    focus: FocusSettings::default(),
                },
                SavedTab {
                    url: None,
                    zoom: 100,
                    focus: FocusSettings::default(),
                },
            ],
        };
        profile.save_if_changed();
        assert!(profile.error.is_none());
        let mut restored = Profile::load(path.clone());
        assert_eq!(restored.data, profile.data);
        assert_eq!(restored.data.bookmarks[0].title, "A title 🫒");
        restored.toggle_bookmark(&location, "ignored").unwrap();
        restored.save_if_changed();
        assert!(Profile::load(path).data.bookmarks.is_empty());
    }
    #[test]
    fn damaged_profiles_are_preserved_and_invalid_state_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.json");
        let mut values = vec!["broken JSON".into(), "{\"version\":99}".into()];
        for url in [
            "javascript:alert(1)",
            "https://user:password@example.com",
            "relative.html",
            "file://remote/share",
        ] {
            let mut data = Data::default();
            data.bookmarks.push(Bookmark {
                url: url.into(),
                title: String::new(),
            });
            values.push(serde_json::to_string(&data).unwrap());
        }
        let mut invalid = Data::default();
        invalid.settings.default_zoom = 0;
        values.push(serde_json::to_string(&invalid).unwrap());
        for bytes in values {
            fs::write(&path, &bytes).unwrap();
            let mut profile = Profile::load(path.clone());
            assert!(profile.load_failed);
            profile.data.settings.restore_session = false;
            profile.save();
            assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
            profile.reset();
            assert!(Profile::load(path.clone()).error.is_none());
        }
    }
    #[test]
    fn failed_write_is_retryable_without_losing_memory() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("blocked");
        let path = parent.join("profile.json");
        let mut profile = Profile::load(path.clone());
        fs::write(&parent, "file").unwrap();
        profile.data.settings.restore_session = false;
        profile.save_if_changed();
        assert!(profile.error.is_some());
        fs::remove_file(parent).unwrap();
        profile.save();
        assert!(!Profile::load(path).data.settings.restore_session);
    }

    #[test]
    fn session_marker_detects_interrupted_launches_and_closes_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.json");
        let mut first = Profile::load(path.clone());
        assert!(!first.begin_session());
        assert!(!Profile::load(path.clone()).data.clean_shutdown);

        // A second launch sees the marker left by the interrupted first launch.
        let mut recovered = Profile::load(path.clone());
        assert!(recovered.begin_session());
        recovered.finish_session();
        assert!(Profile::load(path).data.clean_shutdown);
    }
}
