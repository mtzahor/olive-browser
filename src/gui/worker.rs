//! Private, framed pipe protocol and independently supervised document processes.
//! Pipe I/O and deserialization never block the UI. No listener or shared temp files.
use crate::document::{ClickRequest, LoadedPage, PageUpdate, prepare_page_with_loader};
use eframe::egui;
use olive_html::net::{DocumentLoader, Location};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::{self, Read, Write},
    path::Path,
    process::{Child, Command as ProcessCommand, Stdio},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    time::{Duration, Instant},
};

pub const WORKER_ARG: &str = "--olive-tab-worker";
const MAX_COMMAND_BYTES: usize = 32 * 1024;
const MAX_EVENT_BYTES: usize = 128 * 1024 * 1024;
const LOAD_TIMEOUT: Duration = Duration::from_secs(60);
const CLICK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Serialize, Deserialize)]
pub enum Command {
    Load { location: Location, scripting: bool },
    Click(ClickRequest),
}
#[derive(Serialize, Deserialize)]
pub enum Event {
    Loaded(Box<LoadedPage>),
    Updated(Box<PageUpdate>),
    Error(String),
}
#[derive(Serialize)]
enum InitialEvent<'a> {
    Loaded(&'a LoadedPage),
}

pub struct Worker {
    child: Option<Child>,
    sender: Option<SyncSender<Command>>,
    events: Receiver<Result<Event, String>>,
    deadline: Option<Instant>,
}

impl Worker {
    pub fn spawn(location: Location, scripting: bool, ctx: &egui::Context) -> Result<Self, String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        Self::spawn_at(&executable, location, scripting, ctx)
    }

    pub fn spawn_at(
        executable: &Path,
        location: Location,
        scripting: bool,
        ctx: &egui::Context,
    ) -> Result<Self, String> {
        let mut command = ProcessCommand::new(executable);
        command
            .arg(WORKER_ARG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW: workers have no console/window.
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("Could not start tab process: {e}"))?;
        let mut input = child.stdin.take().ok_or("Tab process has no input pipe")?;
        let mut output = child
            .stdout
            .take()
            .ok_or("Tab process has no output pipe")?;
        let (sender, commands) = mpsc::sync_channel(1);
        let (events_tx, events) = mpsc::sync_channel(1);
        let mut worker = Self {
            child: Some(child),
            sender: Some(sender),
            events,
            deadline: None,
        };
        let ctx_write = ctx.clone();
        let errors = events_tx.clone();
        std::thread::Builder::new()
            .name("olive-tab-input".into())
            .spawn(move || {
                for command in commands {
                    if let Err(error) = write_frame(&mut input, &command, MAX_COMMAND_BYTES) {
                        let _ = errors.send(Err(format!("Tab process disconnected: {error}")));
                        ctx_write.request_repaint();
                        break;
                    }
                }
            })
            .map_err(|e| format!("Could not start tab input: {e}"))?;
        let ctx_read = ctx.clone();
        std::thread::Builder::new()
            .name("olive-tab-output".into())
            .spawn(move || {
                loop {
                    let event = read_frame::<Event>(&mut output, MAX_EVENT_BYTES)
                        .and_then(|event| {
                            let pages = match &event {
                                Event::Loaded(page) => Some((&page.page, &page.reading)),
                                Event::Updated(update) => Some((&update.page, &update.reading)),
                                Event::Error(_) => None,
                            };
                            if let Some((page, reading)) = pages {
                                page.validate()
                                    .and_then(|()| reading.validate())
                                    .map_err(io::Error::other)?;
                            }
                            Ok(event)
                        })
                        .map_err(|e| format!("Tab process stopped or sent invalid data: {e}"));
                    let failed = event.is_err();
                    if events_tx.send(event).is_err() {
                        break;
                    }
                    ctx_read.request_repaint();
                    if failed {
                        break;
                    }
                }
            })
            .map_err(|e| format!("Could not start tab output: {e}"))?;
        worker.send(Command::Load {
            location,
            scripting,
        })?;
        Ok(worker)
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }
    pub fn busy(&self) -> bool {
        self.deadline.is_some()
    }

    pub fn send(&mut self, command: Command) -> Result<(), String> {
        if self.busy() {
            return Err("The tab is still working. Stop or reload it to continue.".into());
        }
        let timeout = match command {
            Command::Load { .. } => LOAD_TIMEOUT,
            Command::Click(_) => CLICK_TIMEOUT,
        };
        self.sender
            .as_ref()
            .ok_or("The tab process has stopped. Reload to restart it.")?
            .try_send(command)
            .map_err(|e| format!("Could not contact tab process: {e}"))?;
        self.deadline = Some(Instant::now() + timeout);
        Ok(())
    }

    pub fn poll(&mut self) -> Option<Result<Event, String>> {
        match self.events.try_recv() {
            Ok(event) => {
                self.deadline = None;
                return Some(event);
            }
            Err(TryRecvError::Disconnected) => {
                return Some(Err("The tab process stopped. Reload to restart it.".into()));
            }
            Err(TryRecvError::Empty) => {}
        }
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Some(Err(format!(
                        "The tab process exited ({status}). Reload to restart it."
                    )));
                }
                Err(error) => return Some(Err(format!("Could not check tab process: {error}"))),
                Ok(None) => {}
            }
        }
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.terminate();
            return Some(Err(
                "This tab stopped responding and was terminated. Reload to restart it.".into(),
            ));
        }
        None
    }

    fn terminate(&mut self) {
        self.sender = None;
        self.deadline = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            // Reap off the UI thread, including when closing an unresponsive tab.
            let _ = std::thread::Builder::new()
                .name("olive-tab-reaper".into())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
    }

    #[cfg(test)]
    pub fn mock() -> (Self, SyncSender<Result<Event, String>>) {
        let (sender, events) = mpsc::sync_channel(1);
        (
            Self {
                child: None,
                sender: None,
                events,
                deadline: None,
            },
            sender,
        )
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.terminate();
    }
}

// Workers read commands on a separate thread so EOF kills even a stuck parser/VM.
// Closing the parent process closes its pipes; no orphaned document work remains.
pub fn run() -> io::Result<()> {
    let (sender, commands) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("olive-parent-watch".into())
        .spawn(move || {
            let mut input = io::stdin().lock();
            loop {
                match read_frame::<Command>(&mut input, MAX_COMMAND_BYTES) {
                    Ok(command) => {
                        if sender.try_send(command).is_err() {
                            std::process::exit(1);
                        }
                    }
                    Err(_) => std::process::exit(0),
                }
            }
        })?;
    let mut output = io::stdout().lock();
    let mut prepared = None;
    for command in commands {
        let event = match command {
            Command::Load {
                location,
                scripting,
            } => {
                // Every full navigation gets a fresh process and document realm.
                if prepared.is_some() {
                    return Err(io::Error::other("Unexpected second load"));
                }
                let result = (|| {
                    let loader = DocumentLoader::new()?;
                    let source = loader.load(location.clone())?;
                    let scripting = !source.location.is_remote()
                        || (scripting && source.location.same_document(&location));
                    prepare_page_with_loader(source, &loader, scripting)
                })();
                match result {
                    Ok(mut page) => {
                        if let Some(session) = &mut page.session {
                            session.take_alerts();
                        }
                        write_frame(
                            &mut output,
                            &InitialEvent::Loaded(&page.loaded),
                            MAX_EVENT_BYTES,
                        )?;
                        // Only the DOM, realm, image assets and source styles need to survive.
                        page.loaded.page = Default::default();
                        page.loaded.reading = Default::default();
                        prepared = Some(page);
                        continue;
                    }
                    Err(error) => Event::Error(error),
                }
            }
            Command::Click(click) => match prepared.as_mut().and_then(|page| page.click(click)) {
                Some(update) => Event::Updated(Box::new(update)),
                None => Event::Error("No live scripting session. Reload this tab.".into()),
            },
        };
        write_frame(&mut output, &event, MAX_EVENT_BYTES)?;
    }
    Ok(())
}

pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read, limit: usize) -> io::Result<T> {
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix)?;
    let size = u32::from_le_bytes(prefix) as usize;
    if size == 0 || size > limit {
        return Err(io::Error::other("Invalid tab message size"));
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

pub fn write_frame(
    writer: &mut impl Write,
    value: &impl Serialize,
    limit: usize,
) -> io::Result<()> {
    struct BoundedBuffer {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl Write for BoundedBuffer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
                return Err(io::Error::other("Tab message is too large"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut buffer = BoundedBuffer {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut buffer, value).map_err(io::Error::other)?;
    writer.write_all(&(buffer.bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&buffer.bytes)?;
    writer.flush()
}

// Compact image transfer, with dimensions and byte lengths checked before upload.
pub mod wire_image {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use eframe::egui::{Color32, ColorImage};
    use serde::{Deserialize, Serialize};
    use std::sync::Arc;
    pub fn serialize<S: serde::Serializer>(
        image: &Arc<ColorImage>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let bytes: Vec<u8> = image
            .pixels
            .iter()
            .flat_map(|pixel| pixel.to_array())
            .collect();
        (image.size, STANDARD.encode(bytes)).serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Arc<ColorImage>, D::Error> {
        let (size, encoded): ([usize; 2], String) = Deserialize::deserialize(deserializer)?;
        let pixels = size[0]
            .checked_mul(size[1])
            .ok_or_else(|| serde::de::Error::custom("Invalid image dimensions"))?;
        if size.contains(&0)
            || size.iter().any(|&v| v > 4096)
            || pixels > 8 * 1024 * 1024
            || encoded.len() > (pixels * 4).div_ceil(3) * 4
        {
            return Err(serde::de::Error::custom("Tab image exceeds pixel budget"));
        }
        let bytes = STANDARD.decode(encoded).map_err(serde::de::Error::custom)?;
        if bytes.len() != pixels * 4 {
            return Err(serde::de::Error::custom("Invalid image byte length"));
        }
        Ok(Arc::new(ColorImage::new(
            size,
            bytes
                .chunks_exact(4)
                .map(|p| Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3]))
                .collect(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_oversized_truncated_and_invalid_frames() {
        for bytes in [
            0_u32.to_le_bytes().to_vec(),
            (MAX_COMMAND_BYTES as u32 + 1).to_le_bytes().to_vec(),
            vec![10, 0, 0, 0, b'{'],
            vec![1, 0, 0, 0, b'!'],
        ] {
            assert!(read_frame::<Command>(&mut &bytes[..], MAX_COMMAND_BYTES).is_err());
        }
        assert!(write_frame(&mut Vec::new(), &"too large", 2).is_err());
    }
    #[test]
    fn deadline_returns_a_recoverable_failure() {
        let (mut worker, _sender) = Worker::mock();
        worker.deadline = Some(Instant::now() - Duration::from_secs(1));
        assert!(
            worker
                .poll()
                .unwrap()
                .err()
                .unwrap()
                .contains("stopped responding")
        );
        assert!(!worker.busy());
    }
}
