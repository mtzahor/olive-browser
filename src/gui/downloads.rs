use crate::gui::network::Network;
use crate::net::download::{MAX_DOWNLOAD_BYTES, suggested_name};
// The macOS quarantine attribute is set through libc.
#[allow(unsafe_code)]
mod origin;
use crate::net::{DocumentLoader, Location};
use eframe::egui;
use std::{
    fmt, fs,
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

/// How long copying a local file may take, as for website downloads.
const LOCAL_COPY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

enum Message {
    Progress(u64, Option<u64>),
    Finished(PathBuf, u64),
    Failed(String),
}
pub enum DownloadState {
    Starting,
    Downloading,
    Saved(PathBuf),
    /// Includes cancellation, reported with the reason.
    Failed(String),
}
impl DownloadState {
    pub fn active(&self) -> bool {
        matches!(self, Self::Starting | Self::Downloading)
    }
}
impl fmt::Display for DownloadState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Starting => f.write_str("Starting…"),
            Self::Downloading => f.write_str("Downloading…"),
            Self::Saved(path) => write!(f, "Saved to {}", path.display()),
            Self::Failed(error) => f.write_str(error),
        }
    }
}
pub struct DownloadItem {
    pub id: u64,
    pub name: String,
    pub url: String,
    pub received: u64,
    pub total: Option<u64>,
    pub state: DownloadState,
    cancel: Arc<AtomicBool>,
}
pub struct Downloads {
    next: u64,
    pub items: Vec<DownloadItem>,
    rx: Receiver<(u64, Message)>,
    tx: Sender<(u64, Message)>,
    pub open: bool,
}

impl Default for Downloads {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            next: 1,
            items: Vec::new(),
            rx,
            tx,
            open: false,
        }
    }
}
impl Downloads {
    /// Download `location` into `directory` through `network`; local files
    /// are copied directly.
    pub fn start(&mut self, location: Location, directory: Option<PathBuf>, network: Arc<Network>) {
        let id = self.next;
        self.next += 1;
        let name = suggested_name(&location);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let tx = self.tx.clone();
        let url = location.as_str().to_owned();
        let remote = location.is_remote();
        let dir = directory.unwrap_or_else(default_directory);
        self.items.push(DownloadItem {
            id,
            name: name.clone(),
            url: url.clone(),
            received: 0,
            total: None,
            state: DownloadState::Starting,
            cancel,
        });
        thread::spawn(move || {
            let result = (|| -> Result<(PathBuf, u64), String> {
                fs::create_dir_all(&dir)
                    .map_err(|e| format!("Could not create download folder: {e}"))?;
                let destination = dir.join(&name);
                let temp = tempfile::NamedTempFile::new_in(&dir)
                    .map_err(|e| format!("Could not create download file: {e}"))?;
                let mut file = temp.as_file().try_clone().map_err(|e| e.to_string())?;
                let progress = |received, total| {
                    if flag.load(Ordering::Relaxed) {
                        return Err("Download cancelled.".to_string());
                    }
                    let _ = tx.send((id, Message::Progress(received, total)));
                    Ok(())
                };
                let bytes = if remote {
                    let mut received = 0;
                    network.download(location, |bytes, total| {
                        progress(received, total)?;
                        if received + bytes.len() as u64 > MAX_DOWNLOAD_BYTES {
                            return Err("Download exceeds the 100 MiB limit.".into());
                        }
                        file.write_all(bytes).map_err(|e| e.to_string())?;
                        received += bytes.len() as u64;
                        Ok(())
                    })?
                } else {
                    DocumentLoader::local(LOCAL_COPY_TIMEOUT)
                        .download(location, &mut file, progress)?
                };
                file.sync_all().map_err(|e| e.to_string())?;
                drop(file);
                // Mark the file before it takes its name. Best effort: some
                // filesystems cannot hold the mark.
                if remote {
                    let _ = origin::mark(temp.as_file(), temp.path(), &url);
                }
                let destination = publish_download(temp, &destination)?;
                Ok((destination, bytes))
            })();
            let message = match result {
                Ok((path, bytes)) => Message::Finished(path, bytes),
                Err(error) => Message::Failed(error),
            };
            let _ = tx.send((id, message));
        });
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((id, message)) = self.rx.try_recv() {
            if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
                match message {
                    Message::Progress(received, total) => {
                        item.received = received;
                        item.total = total;
                        item.state = DownloadState::Downloading;
                    }
                    Message::Finished(path, bytes) => {
                        item.received = bytes;
                        item.total = Some(bytes);
                        item.state = DownloadState::Saved(path);
                    }
                    Message::Failed(error) => item.state = DownloadState::Failed(error),
                }
            }
        }
        if self.items.iter().any(|item| item.state.active()) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
    pub fn cancel(&mut self, id: u64) {
        if let Some(item) = self.items.iter().find(|item| item.id == id) {
            item.cancel.store(true, Ordering::Relaxed);
        }
    }
}
fn default_directory() -> PathBuf {
    crate::gui::storage::home_dir()
        .map(|path| path.join("Downloads"))
        .unwrap_or_else(|| PathBuf::from("downloads"))
}
fn publish_download(
    mut temp: tempfile::NamedTempFile,
    destination: &std::path::Path,
) -> Result<PathBuf, String> {
    let stem = destination
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let ext = destination
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| format!(".{s}"))
        .unwrap_or_default();
    // Claim the name atomically after the transfer. An existence check followed
    // by `persist` can overwrite another download (or a dangling symlink).
    for index in 0..10_000 {
        let candidate = if index == 0 {
            destination.to_path_buf()
        } else {
            destination.with_file_name(format!("{stem} ({index}){ext}"))
        };
        match temp.persist_noclobber(&candidate) {
            Ok(_) => return Ok(candidate),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                temp = error.file;
            }
            Err(error) => return Err(format!("Could not save download: {error}")),
        }
    }
    Err("Could not save download: no unused filename is available.".into())
}
/// A byte count in the largest whole unit, such as "512 bytes" or "3.4 MB".
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1_000 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1_000.0;
    let mut unit = 0;
    while value >= 999.95 && unit + 1 < UNITS.len() {
        value /= 1_000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

pub fn fraction(received: u64, total: Option<u64>) -> f32 {
    total
        .map(|n| {
            if n == 0 {
                0.0
            } else {
                (received as f64 / n as f64).min(1.0) as f32
            }
        })
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn byte_counts_use_readable_units() {
        assert_eq!(format_bytes(0), "0 bytes");
        assert_eq!(format_bytes(999), "999 bytes");
        assert_eq!(format_bytes(1_000), "1.0 KB");
        assert_eq!(format_bytes(3_400_000), "3.4 MB");
        assert_eq!(format_bytes(999_960), "1.0 MB");
        assert_eq!(format_bytes(u64::MAX), "18446744.1 TB");
    }

    fn completed_file(dir: &std::path::Path, bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new_in(dir).unwrap();
        file.write_all(bytes).unwrap();
        file
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn downloaded_files_keep_their_mark_of_the_internet_when_published() {
        let dir = tempfile::tempdir().unwrap();
        let file = completed_file(dir.path(), b"<p>Page</p>");
        origin::mark(file.as_file(), file.path(), "https://example.com/page.html").unwrap();
        let published = publish_download(file, &dir.path().join("page.html")).unwrap();
        #[cfg(target_os = "macos")]
        {
            let output = std::process::Command::new("/usr/bin/xattr")
                .args(["-p", "com.apple.quarantine"])
                .arg(&published)
                .output()
                .unwrap();
            let value = String::from_utf8(output.stdout).unwrap();
            assert!(value.starts_with("0081;"), "{value}");
            assert!(value.trim_end().ends_with(";Olive Browser;"), "{value}");
        }
        #[cfg(windows)]
        {
            let mut stream = published.into_os_string();
            stream.push(":Zone.Identifier");
            let zone = fs::read_to_string(stream).unwrap();
            assert!(zone.contains("ZoneId=3"), "{zone}");
            assert!(
                zone.contains("HostUrl=https://example.com/page.html"),
                "{zone}"
            );
        }
    }

    #[test]
    fn downloads_with_the_same_destination_preserve_both_files() {
        let dir = tempfile::tempdir().unwrap();
        // Both transfers selected this name before either finished downloading.
        let destination = dir.path().join("report.pdf");
        let first = publish_download(completed_file(dir.path(), b"first"), &destination).unwrap();
        let second = publish_download(completed_file(dir.path(), b"second"), &destination).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"first");
        assert_eq!(fs::read(&second).unwrap(), b"second");
        assert_ne!(first, second);
    }

    #[test]
    fn concurrent_downloads_claim_distinct_filenames() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("report.pdf");
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8u8)
                .map(|index| {
                    let barrier = &barrier;
                    let destination = &destination;
                    let temp = completed_file(dir.path(), &[index]);
                    scope.spawn(move || {
                        barrier.wait();
                        (publish_download(temp, destination).unwrap(), index)
                    })
                })
                .collect();
            for handle in handles {
                let (path, index) = handle.join().unwrap();
                assert_eq!(fs::read(path).unwrap(), [index]);
            }
        });
    }

    #[test]
    fn exhausted_download_names_do_not_overwrite_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("report.pdf");
        fs::write(&destination, b"original").unwrap();
        for index in 1..10_000 {
            fs::write(
                dir.path().join(format!("report ({index}).pdf")),
                b"existing",
            )
            .unwrap();
        }
        let error = publish_download(completed_file(dir.path(), b"new"), &destination).unwrap_err();
        assert!(error.contains("no unused filename"));
        assert_eq!(fs::read(destination).unwrap(), b"original");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 10_000);
    }

    #[cfg(unix)]
    #[test]
    fn downloads_preserve_dangling_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("report.pdf");
        let target = dir.path().join("missing.pdf");
        std::os::unix::fs::symlink(&target, &destination).unwrap();
        let saved =
            publish_download(completed_file(dir.path(), b"download"), &destination).unwrap();
        assert_eq!(fs::read_link(&destination).unwrap(), target);
        assert!(!target.exists());
        assert_eq!(fs::read(saved).unwrap(), b"download");
    }
}
