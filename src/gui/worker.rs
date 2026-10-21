//! Private, framed pipe protocol and independently supervised document processes.
//! Pipe I/O and deserialization never block the UI. No listener or shared temp files.
use crate::gui::document::{
    ClickRequest, LoadedPage, PageUpdate, PreparedPage, prepare_page_preview,
    prepare_page_with_parsed,
};
use crate::net::{CookieJar, DocumentLoader, FormRequest, Location};
use eframe::egui;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::{self, Read, Write},
    path::Path,
    process::{Child, Command as ProcessCommand, Stdio},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    time::{Duration, Instant},
};

pub const WORKER_ARG: &str = "--olive-tab-worker";
#[cfg(windows)]
pub const WINDOWS_LAUNCHER_ARG: &str = "--olive-launch-restricted-worker";
const MAX_COMMAND_BYTES: usize = 1024 * 1024;
const MAX_EVENT_BYTES: usize = 128 * 1024 * 1024;
const LOAD_TIMEOUT: Duration = Duration::from_secs(60);
const CLICK_TIMEOUT: Duration = Duration::from_secs(10);

// Apple deprecates sandbox-exec. Keep this initial macOS policy narrow: it
// blocks filesystem writes and leaves reads and network access available.
#[cfg(target_os = "macos")]
const MACOS_TAB_SANDBOX_PROFILE: &str = "(version 1)\n(allow default)\n(deny file-write*)\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptPolicy {
    Disabled,
    /// The per-page opt-in does not grant permission to a redirect destination.
    Page,
    /// The saved opt-in also applies to redirect destinations.
    AllPages,
}

impl ScriptPolicy {
    pub fn for_load(enabled: bool, by_default: bool) -> Self {
        match (enabled, by_default) {
            (false, _) => Self::Disabled,
            (true, false) => Self::Page,
            (true, true) => Self::AllPages,
        }
    }

    fn enabled_for(self, requested: &Location, destination: &Location) -> bool {
        !destination.is_remote()
            || self == Self::AllPages
            || (self == Self::Page && requested.same_document(destination))
    }
}

/// Parent-to-worker messages. A worker performs exactly one `Load` or `Submit`,
/// as its first command; later commands act on that document.
#[derive(Serialize, Deserialize)]
pub enum Command {
    /// `viewport` is the tab's CSS viewport, when known, so responsive pages
    /// arrive laid out for it rather than reflowing after they load.
    Load {
        location: Location,
        scripting: ScriptPolicy,
        cookies: CookieJar,
        initiator: Option<Location>,
        viewport: Option<(f32, f32)>,
    },
    Submit {
        request: FormRequest,
        scripting: ScriptPolicy,
        cookies: CookieJar,
        initiator: Option<Location>,
        viewport: Option<(f32, f32)>,
    },
    /// Carries the browser's current cookies, refreshed before the handler runs.
    Click {
        click: ClickRequest,
        cookies: CookieJar,
    },
    Resize {
        width: f32,
        height: f32,
    },
}
#[derive(Serialize, Deserialize)]
pub enum Event {
    Loaded(Box<LoadedPage>),
    Updated(Box<PageUpdate>),
    ResourcesUpdated(Box<PageUpdate>),
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
    /// Start a tab process from this executable with its initial `Load` or `Submit`.
    pub fn spawn(initial: Command, ctx: &egui::Context) -> Result<Self, String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        Self::spawn_command_at(&executable, initial, ctx)
    }

    pub fn spawn_command_at(
        executable: &Path,
        initial: Command,
        ctx: &egui::Context,
    ) -> Result<Self, String> {
        let mut command = tab_worker_command(executable);
        command.stdin(Stdio::piped()).stdout(Stdio::piped());
        #[cfg(all(windows, test))]
        command.stderr(Stdio::inherit());
        #[cfg(not(all(windows, test)))]
        command.stderr(Stdio::null());
        command.arg(WORKER_ARG);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW: workers have no console/window.
        }
        let mut child = command.spawn().map_err(|e| {
            #[cfg(target_os = "macos")]
            {
                format!("Could not start tab worker in the macOS sandbox: {e}")
            }
            #[cfg(target_os = "windows")]
            {
                format!("Could not start tab worker in the Windows sandbox: {e}")
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            {
                format!("Could not start tab process: {e}")
            }
        })?;
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
                                Event::ResourcesUpdated(update) => {
                                    Some((&update.page, &update.reading))
                                }
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
        worker.send(initial)?;
        Ok(worker)
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }
    pub fn busy(&self) -> bool {
        self.deadline.is_some()
    }

    pub fn await_resource_update(&mut self) {
        self.deadline = Some(Instant::now() + LOAD_TIMEOUT);
    }

    pub fn send(&mut self, command: Command) -> Result<(), String> {
        if self.busy() {
            return Err("The tab is still working. Stop or reload it to continue.".into());
        }
        let timeout = match command {
            Command::Load { .. } | Command::Submit { .. } => LOAD_TIMEOUT,
            Command::Click { .. } | Command::Resize { .. } => CLICK_TIMEOUT,
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
                // A styled page that is still loading resources keeps the deadline.
                if !matches!(&event, Ok(Event::ResourcesUpdated(update)) if update.resources.pending)
                {
                    self.deadline = None;
                }
                match event {
                    Ok(event) => return Some(Ok(event)),
                    Err(error) => {
                        if let Some(child) = &mut self.child {
                            if let Ok(Some(status)) = child.try_wait() {
                                return Some(Err(format!(
                                    "{error}; worker launcher exited ({status})"
                                )));
                            }
                        }
                        return Some(Err(error));
                    }
                }
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

fn tab_worker_command(executable: &Path) -> ProcessCommand {
    #[cfg(target_os = "macos")]
    {
        let mut command = ProcessCommand::new("/usr/bin/sandbox-exec");
        command
            .arg("-p")
            .arg(MACOS_TAB_SANDBOX_PROFILE)
            .arg(executable);
        command
    }
    #[cfg(windows)]
    {
        let mut command = ProcessCommand::new(executable);
        command.arg(WINDOWS_LAUNCHER_ARG);
        command
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        ProcessCommand::new(executable)
    }
}

#[cfg(windows)]
pub fn run_windows_launcher() -> u32 {
    match windows_sandbox::launch_worker() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("Could not start tab worker in the Windows sandbox: {error}");
            report_start_error(format!(
                "Could not start tab worker in the Windows sandbox: {error}"
            ));
            1
        }
    }
}

#[cfg(any(target_os = "linux", windows))]
fn report_start_error(message: String) {
    let _ = write_frame(
        &mut io::stdout().lock(),
        &Event::Error(message),
        MAX_EVENT_BYTES,
    );
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.terminate();
    }
}

// Workers read commands on a separate thread so EOF kills even a stuck parser/VM.
// Closing the parent process closes its pipes; no orphaned document work remains.
pub fn run() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    if let Err(error) = restrict_linux_worker_writes() {
        report_start_error(format!(
            "Could not start tab worker in the Linux filesystem sandbox: {error}"
        ));
        return Err(io::Error::other(error));
    }

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
    let mut prepared: Option<PreparedPage> = None;
    for command in commands {
        let event = match command {
            Command::Load {
                location,
                scripting,
                cookies,
                initiator,
                viewport,
            } => navigate(
                &mut output,
                &mut prepared,
                Navigation {
                    location,
                    form: None,
                    scripting,
                    cookies,
                    initiator,
                    viewport,
                },
            )?,
            Command::Submit {
                request,
                scripting,
                cookies,
                initiator,
                viewport,
            } => navigate(
                &mut output,
                &mut prepared,
                Navigation {
                    location: request.location.clone(),
                    form: Some(request),
                    scripting,
                    cookies,
                    initiator,
                    viewport,
                },
            )?,
            Command::Click { click, cookies } => {
                match prepared
                    .as_mut()
                    .and_then(|page| page.click(click, cookies))
                {
                    Some(update) => Event::Updated(Box::new(update)),
                    None => Event::Error("No live scripting session. Reload this tab.".into()),
                }
            }
            Command::Resize { width, height } => {
                match prepared
                    .as_mut()
                    .and_then(|page| page.resize((width, height)))
                {
                    Some(update) => Event::Updated(Box::new(update)),
                    None => {
                        Event::Error("The page is no longer available. Reload this tab.".into())
                    }
                }
            }
        };
        write_frame(&mut output, &event, MAX_EVENT_BYTES)?;
    }
    Ok(())
}

/// A tab's single full navigation: an address load or a form submission.
struct Navigation {
    location: Location,
    form: Option<FormRequest>,
    scripting: ScriptPolicy,
    cookies: CookieJar,
    initiator: Option<Location>,
    viewport: Option<(f32, f32)>,
}

/// Fetch the document and stream its preview and, once its stylesheets are
/// in, its styled page; then load the remaining resources and run scripts.
/// Every full navigation gets a fresh process and document realm.
fn navigate(
    output: &mut impl Write,
    prepared: &mut Option<PreparedPage>,
    navigation: Navigation,
) -> io::Result<Event> {
    if prepared.is_some() {
        return Err(io::Error::other("Unexpected second load"));
    }
    let Navigation {
        location,
        form,
        scripting,
        cookies,
        initiator,
        viewport,
    } = navigation;
    let viewport = viewport.map_or(crate::css::DEFAULT_VIEWPORT, |(width, height)| {
        (width.clamp(1.0, 10_000.0), height.clamp(1.0, 10_000.0))
    });
    let mut preview = None;
    let result = (|| {
        let loader = DocumentLoader::with_timeout_and_cookies(LOAD_TIMEOUT, cookies)?
            .with_cookie_initiator(initiator);
        let source = match form {
            Some(request) => loader.submit(request)?,
            None => loader.load(location.clone())?,
        };
        let scripting = scripting.enabled_for(&location, &source.location);
        let (preview_page, parsed) = prepare_page_preview(&source, scripting, viewport)?;
        write_frame(
            output,
            &InitialEvent::Loaded(&preview_page),
            MAX_EVENT_BYTES,
        )
        .map_err(|error| error.to_string())?;
        preview = Some(preview_page);
        let mut progress_error = None;
        let mut progress = |update: PageUpdate| {
            if progress_error.is_none() {
                progress_error = write_frame(
                    output,
                    &Event::ResourcesUpdated(Box::new(update)),
                    MAX_EVENT_BYTES,
                )
                .err();
            }
        };
        let page = prepare_page_with_parsed(
            source,
            &loader,
            scripting,
            parsed,
            true,
            viewport,
            Some(&mut progress),
        );
        match progress_error {
            Some(error) => Err(error.to_string()),
            None => page,
        }
    })();
    Ok(match result {
        Ok(mut page) => {
            if let Some(session) = &mut page.session {
                session.take_alerts();
            }
            let update = page.take_update();
            *prepared = Some(page);
            Event::ResourcesUpdated(Box::new(update))
        }
        Err(error) => match preview {
            Some(mut loaded) => {
                loaded.resources.pending = false;
                loaded.resources.diagnostics.push(error);
                Event::ResourcesUpdated(Box::new(loaded.into_update()))
            }
            None => Event::Error(error),
        },
    })
}

#[cfg(target_os = "linux")]
fn restrict_linux_worker_writes() -> Result<(), String> {
    linux_sandbox::restrict_writes()
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
#[path = "worker/linux_sandbox.rs"]
mod linux_sandbox;

#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "worker/windows_sandbox.rs"]
mod windows_sandbox;

/// Read one frame: a little-endian `u32` length, then a `u32` JSON length, the
/// JSON message, and the pictures the message refers to.
pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read, limit: usize) -> io::Result<T> {
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix)?;
    let size = u32::from_le_bytes(prefix) as usize;
    if size == 0 || size > limit {
        return Err(io::Error::other("Invalid tab message size"));
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    let invalid = || io::Error::other("Invalid tab message");
    let (length, rest) = bytes.split_first_chunk::<4>().ok_or_else(invalid)?;
    let length = u32::from_le_bytes(*length) as usize;
    if length > rest.len() {
        return Err(invalid());
    }
    let (json, pictures) = rest.split_at(length);
    let pictures = wire_image::read_pictures(pictures).map_err(io::Error::other)?;
    wire_image::in_frame(pictures, || serde_json::from_slice(json))
        .0
        .map_err(io::Error::other)
}

pub fn write_frame(
    writer: &mut impl Write,
    value: &impl Serialize,
    limit: usize,
) -> io::Result<()> {
    let mut buffer = wire_image::BoundedBuffer {
        // Room for the JSON length.
        bytes: vec![0; 4],
        limit,
    };
    let (written, pictures) =
        wire_image::in_frame(Vec::new(), || serde_json::to_writer(&mut buffer, value));
    written.map_err(io::Error::other)?;
    let length = (buffer.bytes.len() - 4) as u32;
    buffer.bytes[..4].copy_from_slice(&length.to_le_bytes());
    for picture in &pictures {
        wire_image::write_picture(&mut buffer, picture)?;
    }
    writer.write_all(&(buffer.bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&buffer.bytes)?;
    writer.flush()
}

/// Pictures travel after a frame's JSON as raw premultiplied RGBA, each once
/// per frame however often the message refers to it. They need no encoding,
/// and the browser process only checks their dimensions and length before
/// upload; it never runs an image decoder on tab data.
pub mod wire_image {
    use crate::gui::document::{MAX_IMAGE_DIMENSION, MAX_PAGE_IMAGE_PIXELS};
    use eframe::egui::{Color32, ColorImage};
    use serde::{Deserialize, Serialize};
    use std::{
        cell::RefCell,
        collections::HashMap,
        io::{self, Write},
        sync::Arc,
    };

    pub(super) struct BoundedBuffer {
        pub(super) bytes: Vec<u8>,
        pub(super) limit: usize,
    }
    impl Write for BoundedBuffer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.reserve(bytes.len())?;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl BoundedBuffer {
        fn reserve(&mut self, length: usize) -> io::Result<()> {
            if length > self.limit.saturating_sub(self.bytes.len()) {
                return Err(io::Error::other("Tab message is too large"));
            }
            self.bytes.reserve(length);
            Ok(())
        }
    }

    /// The pictures of the frame being written or read on this thread.
    #[derive(Default)]
    struct Frame {
        pictures: Vec<Arc<ColorImage>>,
        indices: HashMap<*const ColorImage, usize>,
    }
    thread_local! {
        static FRAME: RefCell<Option<Frame>> = const { RefCell::new(None) };
    }

    /// Run `work` with a frame holding `pictures`, returning its result and
    /// the pictures the frame holds afterwards.
    pub(super) fn in_frame<T>(
        pictures: Vec<Arc<ColorImage>>,
        work: impl FnOnce() -> T,
    ) -> (T, Vec<Arc<ColorImage>>) {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                FRAME.with(|frame| frame.borrow_mut().take());
            }
        }
        FRAME.with(|frame| {
            *frame.borrow_mut() = Some(Frame {
                pictures,
                indices: HashMap::new(),
            })
        });
        let reset = Reset;
        let result = work();
        let pictures = FRAME
            .with(|frame| frame.borrow_mut().take())
            .unwrap_or_default()
            .pictures;
        drop(reset);
        (result, pictures)
    }

    pub(super) fn write_picture(buffer: &mut BoundedBuffer, image: &ColorImage) -> io::Result<()> {
        let [width, height] = image.size;
        buffer.reserve(8 + image.pixels.len() * 4)?;
        buffer.write_all(&(width as u32).to_le_bytes())?;
        buffer.write_all(&(height as u32).to_le_bytes())?;
        let start = buffer.bytes.len();
        buffer.bytes.resize(start + image.pixels.len() * 4, 0);
        for (bytes, pixel) in buffer.bytes[start..].chunks_exact_mut(4).zip(&image.pixels) {
            bytes.copy_from_slice(&pixel.to_array());
        }
        Ok(())
    }

    pub(super) fn read_pictures(mut bytes: &[u8]) -> Result<Vec<Arc<ColorImage>>, String> {
        let mut pictures = Vec::new();
        while !bytes.is_empty() {
            let mut side = || -> Result<usize, String> {
                let (side, rest) = bytes
                    .split_first_chunk::<4>()
                    .ok_or("Truncated tab image")?;
                bytes = rest;
                Ok(u32::from_le_bytes(*side) as usize)
            };
            let size = [side()?, side()?];
            let exceeds = || Err("Tab image exceeds pixel budget".to_owned());
            if size.contains(&0) || size.iter().any(|&side| side > MAX_IMAGE_DIMENSION as usize) {
                return exceeds();
            }
            let pixels = size[0] * size[1];
            if pixels as u64 > MAX_PAGE_IMAGE_PIXELS || pixels * 4 > bytes.len() {
                return exceeds();
            }
            let (data, rest) = bytes.split_at(pixels * 4);
            bytes = rest;
            pictures.push(Arc::new(ColorImage::new(
                size,
                data.chunks_exact(4)
                    .map(|p| Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3]))
                    .collect(),
            )));
        }
        Ok(pictures)
    }

    /// Write the picture's place among the frame's pictures.
    pub fn serialize<S: serde::Serializer>(
        image: &Arc<ColorImage>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        FRAME
            .with(|frame| {
                let mut frame = frame.borrow_mut();
                let Frame { pictures, indices } = frame.as_mut()?;
                Some(*indices.entry(Arc::as_ptr(image)).or_insert_with(|| {
                    pictures.push(image.clone());
                    pictures.len() - 1
                }))
            })
            .ok_or_else(|| serde::ser::Error::custom("Pictures travel only in tab messages"))?
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Arc<ColorImage>, D::Error> {
        let index = usize::deserialize(deserializer)?;
        FRAME
            .with(|frame| frame.borrow().as_ref()?.pictures.get(index).cloned())
            .ok_or_else(|| serde::de::Error::custom("Unknown tab image"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::document::MAX_IMAGE_DIMENSION;
    #[test]
    fn script_policy_preserves_page_opt_in_and_local_documents() {
        let page = Location::from_input("https://example.com/page").unwrap();
        let fragment = Location::from_input("https://example.com/page#part").unwrap();
        let redirect = Location::from_input("https://other.example/destination").unwrap();
        let local =
            Location::from_path(std::env::current_dir().unwrap().join("example.html")).unwrap();
        for policy in [
            ScriptPolicy::Disabled,
            ScriptPolicy::Page,
            ScriptPolicy::AllPages,
        ] {
            assert!(policy.enabled_for(&local, &local));
            assert_eq!(
                policy.enabled_for(&page, &fragment),
                policy != ScriptPolicy::Disabled
            );
            assert_eq!(
                policy.enabled_for(&page, &redirect),
                policy == ScriptPolicy::AllPages
            );
        }
        assert_eq!(ScriptPolicy::for_load(false, true), ScriptPolicy::Disabled);
        assert_eq!(ScriptPolicy::for_load(true, false), ScriptPolicy::Page);
        assert_eq!(ScriptPolicy::for_load(true, true), ScriptPolicy::AllPages);
    }

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
    fn pictures_travel_once_per_frame_and_are_checked() {
        use crate::gui::document::Favicon;
        use eframe::egui::{Color32, ColorImage};
        use std::sync::Arc;
        let image = Arc::new(ColorImage::new(
            [2, 3],
            vec![Color32::from_rgba_premultiplied(10, 20, 30, 40); 6],
        ));
        let pair = (Favicon(image.clone()), Favicon(image.clone()));
        let mut frame = Vec::new();
        write_frame(&mut frame, &pair, MAX_EVENT_BYTES).unwrap();
        // The JSON names the picture twice; its pixels follow once.
        let json = u32::from_le_bytes(frame[4..8].try_into().unwrap()) as usize;
        assert_eq!(frame.len(), 8 + json + 8 + 6 * 4);
        let (first, second): (Favicon, Favicon) =
            read_frame(&mut &frame[..], MAX_EVENT_BYTES).unwrap();
        assert!(Arc::ptr_eq(&first.0, &second.0));
        assert_eq!(*first.0, *image);
        // Pictures exist only inside frames.
        assert!(serde_json::to_vec(&pair).is_err());

        let frame_with = |json: &[u8], pictures: &[u8]| {
            let mut body = (json.len() as u32).to_le_bytes().to_vec();
            body.extend_from_slice(json);
            body.extend_from_slice(pictures);
            let mut frame = (body.len() as u32).to_le_bytes().to_vec();
            frame.extend(body);
            frame
        };
        let picture = |width: u32, height: u32, bytes: usize| {
            let mut picture = width.to_le_bytes().to_vec();
            picture.extend_from_slice(&height.to_le_bytes());
            picture.extend(vec![0; bytes]);
            picture
        };
        assert!(read_frame::<Favicon>(&mut &frame_with(b"0", &picture(1, 1, 4))[..], 1024).is_ok());
        for invalid in [
            frame_with(b"1", &picture(1, 1, 4)),
            frame_with(b"0", &[]),
            frame_with(b"0", &picture(0, 1, 0)),
            frame_with(b"0", &picture(MAX_IMAGE_DIMENSION + 1, 1, 0)),
            frame_with(b"0", &picture(2, 2, 15)),
            frame_with(b"0", &picture(1, 1, 4)[..6]),
        ] {
            assert!(read_frame::<Favicon>(&mut &invalid[..], MAX_EVENT_BYTES).is_err());
        }
        // A JSON length beyond the frame is rejected too.
        let mut overlong = frame_with(b"0", &picture(1, 1, 4));
        overlong[4..8].copy_from_slice(&1000_u32.to_le_bytes());
        assert!(read_frame::<Favicon>(&mut &overlong[..], MAX_EVENT_BYTES).is_err());
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
