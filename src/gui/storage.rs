//! Locations of local browser state and crash-safe file replacement.
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

/// A non-empty path from the environment.
pub(crate) fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env_path("USERPROFILE")
    }
    #[cfg(not(windows))]
    {
        env_path("HOME")
    }
}

/// The per-user directory for history and profile files. `OLIVE_PROFILE_DIR`
/// replaces the platform data directory.
pub(crate) fn profile_dir() -> Option<PathBuf> {
    if let Some(path) = env_path("OLIVE_PROFILE_DIR") {
        return Some(path);
    }
    #[cfg(target_os = "macos")]
    {
        home_dir().map(|home| home.join("Library/Application Support/Olive Browser"))
    }
    #[cfg(target_os = "windows")]
    {
        env_path("LOCALAPPDATA").map(|data| data.join("Olive Browser"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        env_path("XDG_DATA_HOME")
            .filter(|path| path.is_absolute())
            .or_else(|| home_dir().map(|home| home.join(".local/share")))
            .map(|data| data.join("olive-browser"))
    }
}

/// Read a whole file of at most `limit` bytes. A missing file is `Ok(None)`.
/// `what` names the file in the size error.
pub(crate) fn read_bounded(path: &Path, limit: u64, what: &str) -> Result<Option<Vec<u8>>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let too_large = || format!("{what} exceeds the {} MiB limit", limit / (1024 * 1024));
    if file.metadata().map_err(|e| e.to_string())?.len() > limit {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(too_large());
    }
    Ok(Some(bytes))
}

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Replace a browser state file atomically, creating its directory on first use.
pub(crate) fn write_state(path: &Path, contents: &[u8]) -> io::Result<()> {
    fs::create_dir_all(parent_dir(path))?;
    write_atomic(path, contents)
}

/// Replace `path` with `contents`, so readers see the old or the new file and
/// never a partial one. The parent directory must already exist.
pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    // A file in the same directory allows atomic replacement on all supported
    // platforms. tempfile creates it with owner-only permissions on Unix.
    let mut file = tempfile::NamedTempFile::new_in(parent_dir(path))?;
    file.write_all(contents)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}
