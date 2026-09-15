use eframe::egui;
use olive_html::net::download::{MAX_DOWNLOAD_BYTES, suggested_name};
use olive_html::net::{DocumentLoader, Location};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

enum Message {
    Progress(u64, Option<u64>),
    Finished(PathBuf, u64),
    Failed(String),
}
pub struct DownloadItem {
    pub id: u64,
    pub name: String,
    pub url: String,
    pub received: u64,
    pub total: Option<u64>,
    pub state: String,
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
    pub fn start(&mut self, location: Location, directory: Option<PathBuf>) {
        let id = self.next;
        self.next += 1;
        let name = suggested_name(&location);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let tx = self.tx.clone();
        let url = location.as_str().to_owned();
        let dir = directory.unwrap_or_else(default_directory);
        self.items.push(DownloadItem {
            id,
            name: name.clone(),
            url: url.clone(),
            received: 0,
            total: None,
            state: "Starting…".into(),
            cancel,
        });
        thread::spawn(move || {
            let result = (|| -> Result<(PathBuf, u64), String> {
                fs::create_dir_all(&dir)
                    .map_err(|e| format!("Could not create download folder: {e}"))?;
                let destination = unique_path(&dir, &name);
                let temp = tempfile::NamedTempFile::new_in(&dir)
                    .map_err(|e| format!("Could not create download file: {e}"))?;
                let mut file = temp.as_file().try_clone().map_err(|e| e.to_string())?;
                let loader = DocumentLoader::new()?;
                let bytes = loader.download(location, &mut file, |received, total| {
                    if flag.load(Ordering::Relaxed) {
                        return Err("Download cancelled.".into());
                    }
                    let _ = tx.send((id, Message::Progress(received, total)));
                    Ok(())
                })?;
                file.sync_all().map_err(|e| e.to_string())?;
                temp.persist(&destination).map_err(|e| e.to_string())?;
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
                        item.state = "Downloading…".into();
                    }
                    Message::Finished(path, bytes) => {
                        item.received = bytes;
                        item.total = Some(bytes);
                        item.state = format!("Saved to {}", path.display());
                    }
                    Message::Failed(error) => item.state = error,
                }
            }
        }
        if self
            .items
            .iter()
            .any(|item| item.state == "Starting…" || item.state == "Downloading…")
        {
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
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join("Downloads"))
        .unwrap_or_else(|| PathBuf::from("downloads"))
}
fn unique_path(dir: &std::path::Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| format!(".{s}"))
        .unwrap_or_default();
    (1..10_000)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|path| !path.exists())
        .unwrap_or(candidate)
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
#[allow(dead_code)]
const _: u64 = MAX_DOWNLOAD_BYTES;
