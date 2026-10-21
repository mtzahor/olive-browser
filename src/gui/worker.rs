//! Private, framed pipe protocol and independently supervised document processes.
//! Pipe I/O and deserialization never block the UI. No listener or shared temp files.
//!
//! The browser fetches each document itself, with the whole cookie jar, and a
//! fresh tab process renders it holding only that site's cookies. What a tab
//! process reports about its page's address, base and cookies is held to what
//! the browser fetched for it.
use crate::gui::document::{
    ClickRequest, LoadedPage, PageUpdate, PreparedPage, prepare_page_preview,
    prepare_page_with_parsed,
};
use crate::net::{CookieJar, CookieUpdate, DocumentLoader, FormRequest, LoadedDocument, Location};
use eframe::egui;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    ffi::OsString,
    io::{self, Read, Write},
    path::Path,
    process::{Child, Command as ProcessCommand, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    time::{Duration, Instant, SystemTime},
};

pub use heap::CountedHeap;

pub const WORKER_ARG: &str = "--olive-tab-worker";
#[cfg(windows)]
pub const WINDOWS_LAUNCHER_ARG: &str = "--olive-launch-restricted-worker";
// A document of up to 8 MiB crosses as JSON text, where escaping can expand
// control characters sixfold.
const MAX_COMMAND_BYTES: usize = 64 * 1024 * 1024;
const MAX_EVENT_BYTES: usize = 128 * 1024 * 1024;
const LOAD_TIMEOUT: Duration = Duration::from_secs(60);
const CLICK_TIMEOUT: Duration = Duration::from_secs(10);
/// The most heap a tab process may use. Past it, allocation fails and the
/// process stops, so one page cannot exhaust the computer's memory.
pub const MAX_WORKER_HEAP_BYTES: usize = 1024 * 1024 * 1024;
/// The environment a tab process keeps, compared without regard to case:
/// proxy and certificate settings for its resource requests, and what the OS
/// needs to start it. Everything else, such as tokens, stays in the browser.
const WORKER_ENVIRONMENT: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "HOME",
    "PATH",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
];

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

/// A tab's one full navigation. The browser fetches the document, then a
/// fresh tab process renders it.
pub struct Request {
    pub target: Target,
    pub scripting: ScriptPolicy,
    /// The browser's whole jar, for fetching the document. The tab process
    /// receives only the cookies of the site it renders.
    pub cookies: CookieJar,
    /// The page that started the navigation, for SameSite cookie rules.
    pub initiator: Option<Location>,
    /// The tab's CSS viewport, when known, so responsive pages arrive laid
    /// out for it rather than reflowing after they load.
    pub viewport: Option<(f32, f32)>,
}

/// What a navigation fetches: an address, or a form submission.
pub enum Target {
    Load(Location),
    Submit(FormRequest),
}

/// Browser-to-worker messages. A worker renders exactly one document, sent
/// first; later commands act on that document.
#[derive(Serialize, Deserialize)]
pub enum Command {
    /// The document the browser fetched, with the cookies of its site.
    Render {
        document: LoadedDocument,
        scripting: bool,
        cookies: CookieJar,
        viewport: Option<(f32, f32)>,
    },
    /// Carries the browser's current cookies for the page's site, refreshed
    /// before the handler runs.
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
    fetched: Receiver<Fetched>,
    /// The browser's record of the document this worker renders, once fetched.
    document: Option<Fetched>,
    started: SystemTime,
    /// Stops the browser's fetch of the document when the tab stops.
    stopped: Arc<AtomicBool>,
}

/// What the browser knows of the document a worker renders, independently of
/// what the worker reports.
struct Fetched {
    location: Location,
    status: Option<u16>,
    scripting: bool,
    /// Cookies set while fetching the document, delivered with the page.
    cookie_updates: Vec<CookieUpdate>,
}

impl Worker {
    /// Fetch the document of `request` and render it in a fresh tab process.
    pub fn spawn(request: Request, ctx: &egui::Context) -> Result<Self, String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        Self::spawn_at(&executable, request, ctx)
    }

    pub fn spawn_at(
        executable: &Path,
        request: Request,
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
        let (fetched_tx, fetched) = mpsc::sync_channel(1);
        let render = sender.clone();
        let worker = Self {
            child: Some(child),
            sender: Some(sender),
            events,
            deadline: Some(Instant::now() + LOAD_TIMEOUT),
            fetched,
            document: None,
            started: SystemTime::now(),
            stopped: Arc::new(AtomicBool::new(false)),
        };
        let stopped = worker.stopped.clone();
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
        let ctx_fetch = ctx.clone();
        let fetch_errors = events_tx.clone();
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
        // The tab process starts while the browser fetches its document.
        std::thread::Builder::new()
            .name("olive-tab-fetch".into())
            .spawn(move || match fetch(request, stopped) {
                Ok((document, command)) => {
                    // The browser's record arrives before the worker can answer.
                    if fetched_tx.send(document).is_ok() {
                        let _ = render.send(command);
                    }
                }
                Err(error) => {
                    let _ = fetch_errors.send(Ok(Event::Error(error)));
                    ctx_fetch.request_repaint();
                }
            })
            .map_err(|e| format!("Could not start loading: {e}"))?;
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

    pub fn send(&mut self, mut command: Command) -> Result<(), String> {
        if self.busy() {
            return Err("The tab is still working. Stop or reload it to continue.".into());
        }
        let timeout = match &mut command {
            Command::Render { .. } => {
                return Err("A tab renders only the document the browser fetched for it.".into());
            }
            Command::Click { cookies, .. } => {
                *cookies = self
                    .document
                    .as_ref()
                    .map(|document| cookies.for_document(&document.location))
                    .unwrap_or_default();
                CLICK_TIMEOUT
            }
            Command::Resize { .. } => CLICK_TIMEOUT,
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
                match event.and_then(|event| self.confine(event)) {
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

    /// Hold a worker's report to the document the browser fetched for it: the
    /// address, status and scripting state are the browser's own, a web page's
    /// base stays on the web, and cookie changes stay on the page's site.
    fn confine(&mut self, mut event: Event) -> Result<Event, String> {
        let (base, updates) = match &mut event {
            Event::Loaded(page) => (&mut page.base, &mut page.cookie_updates),
            Event::Updated(update) | Event::ResourcesUpdated(update) => {
                (&mut update.base, &mut update.cookie_updates)
            }
            Event::Error(_) => return Ok(event),
        };
        // The browser records its fetch before sending the document, so the
        // record is in by the time the worker's first page arrives.
        if self.document.is_none() {
            self.document = self.fetched.try_recv().ok();
        }
        let document = self
            .document
            .as_mut()
            .ok_or("The tab process sent a page before its document was fetched.")?;
        if document.location.is_remote() && !base.is_remote() {
            *base = document.location.clone();
        }
        *updates = std::mem::take(updates)
            .into_iter()
            .filter_map(|update| update.confine(&document.location, self.started))
            .collect();
        if let Event::Loaded(page) = &mut event {
            page.location = document.location.clone();
            page.status = document.status;
            page.scripting_enabled = document.scripting;
            let mut cookie_updates = std::mem::take(&mut document.cookie_updates);
            cookie_updates.append(&mut page.cookie_updates);
            page.cookie_updates = cookie_updates;
        }
        Ok(event)
    }

    fn terminate(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
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
        let (_, fetched) = mpsc::sync_channel(1);
        (
            Self {
                child: None,
                sender: None,
                events,
                deadline: None,
                fetched,
                document: None,
                started: SystemTime::now(),
                stopped: Arc::new(AtomicBool::new(false)),
            },
            sender,
        )
    }

    /// A mock worker that the browser trusts to render `page`.
    #[cfg(test)]
    pub fn mock_rendering(page: &LoadedPage) -> (Self, SyncSender<Result<Event, String>>) {
        let (mut worker, sender) = Self::mock();
        worker.document = Some(Fetched {
            location: page.location.clone(),
            status: page.status,
            scripting: page.scripting_enabled,
            cookie_updates: Vec::new(),
        });
        (worker, sender)
    }
}

/// Fetch a navigation's document in the browser, with the whole cookie jar,
/// and prepare the worker's command to render it.
fn fetch(request: Request, stopped: Arc<AtomicBool>) -> Result<(Fetched, Command), String> {
    let Request {
        target,
        scripting,
        cookies,
        initiator,
        viewport,
    } = request;
    let loader = DocumentLoader::with_timeout_and_cookies(LOAD_TIMEOUT, cookies)?
        .with_cookie_initiator(initiator)
        .with_stop(stopped);
    let (requested, document) = match target {
        Target::Load(location) => (location.clone(), loader.load(location)?),
        Target::Submit(form) => (form.location.clone(), loader.submit(form)?),
    };
    let scripting = scripting.enabled_for(&requested, &document.location);
    let fetched = Fetched {
        location: document.location.clone(),
        status: document.status,
        scripting,
        cookie_updates: loader.take_cookie_updates(),
    };
    let command = Command::Render {
        cookies: loader.cookies().for_document(&document.location),
        document,
        scripting,
        viewport,
    };
    Ok((fetched, command))
}

fn tab_worker_command(executable: &Path) -> ProcessCommand {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = ProcessCommand::new("/usr/bin/sandbox-exec");
        command
            .arg("-p")
            .arg(MACOS_TAB_SANDBOX_PROFILE)
            .arg(executable);
        command
    };
    #[cfg(windows)]
    let mut command = {
        let mut command = ProcessCommand::new(executable);
        command.arg(WINDOWS_LAUNCHER_ARG);
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = ProcessCommand::new(executable);
    command
        .env_clear()
        .envs(worker_environment(std::env::vars_os()));
    if let Some(directory) = executable.parent().filter(|path| path.is_dir()) {
        command.current_dir(directory);
    }
    command
}

/// The variables of `environment` that a tab process keeps.
fn worker_environment(
    environment: impl IntoIterator<Item = (OsString, OsString)>,
) -> impl Iterator<Item = (OsString, OsString)> {
    environment.into_iter().filter(|(name, _)| {
        name.to_str().is_some_and(|name| {
            WORKER_ENVIRONMENT
                .iter()
                .any(|kept| kept.eq_ignore_ascii_case(name))
        })
    })
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
    if !heap::limit(MAX_WORKER_HEAP_BYTES) {
        let error = "this executable does not count its heap, so no memory limit can apply";
        report_start_error(format!("Could not start tab worker: {error}"));
        return Err(io::Error::other(error));
    }
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
            Command::Render {
                document,
                scripting,
                cookies,
                viewport,
            } => render(
                &mut output,
                &mut prepared,
                document,
                Rendering {
                    scripting,
                    cookies,
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

/// How a worker renders its document.
struct Rendering {
    scripting: bool,
    cookies: CookieJar,
    viewport: Option<(f32, f32)>,
}

/// Stream the document's preview and, once its stylesheets are in, its styled
/// page; then load the remaining resources and run scripts. Every full
/// navigation gets a fresh process and document realm.
fn render(
    output: &mut impl Write,
    prepared: &mut Option<PreparedPage>,
    source: LoadedDocument,
    rendering: Rendering,
) -> io::Result<Event> {
    if prepared.is_some() {
        return Err(io::Error::other("Unexpected second document"));
    }
    let Rendering {
        scripting,
        cookies,
        viewport,
    } = rendering;
    let viewport = viewport.map_or(crate::css::DEFAULT_VIEWPORT, |(width, height)| {
        (width.clamp(1.0, 10_000.0), height.clamp(1.0, 10_000.0))
    });
    let mut preview = None;
    let result = (|| {
        let loader = DocumentLoader::with_timeout_and_cookies(LOAD_TIMEOUT, cookies)?;
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

#[allow(unsafe_code)]
#[path = "worker/heap.rs"]
mod heap;

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

    fn page(url: &str, html: &[u8]) -> LoadedPage {
        let source = LoadedDocument {
            location: Location::from_input(url).unwrap(),
            bytes: html.to_vec(),
            status: Some(200),
            plain_text: false,
        };
        crate::gui::document::prepare_page_with_loader(
            source,
            &DocumentLoader::new().unwrap(),
            false,
        )
        .unwrap()
        .loaded
    }

    #[test]
    fn reports_are_held_to_the_document_the_browser_fetched() {
        let fetched = page("https://www.example.com/page", b"<p>Fetched</p>");
        let bank = Location::from_input("https://bank.example/").unwrap();
        let local = Location::from_path(std::env::current_dir().unwrap().join("secret/")).unwrap();
        let (mut worker, events) = Worker::mock_rendering(&fetched);
        worker.document.as_mut().unwrap().cookie_updates = vec![CookieUpdate::from_script(
            fetched.location.clone(),
            "fetched=1".into(),
        )];

        let mut spoofed = page("https://bank.example/account", b"<p>Spoofed</p>");
        spoofed.base = local.clone();
        spoofed.status = Some(500);
        spoofed.scripting_enabled = true;
        spoofed.cookie_updates = vec![
            CookieUpdate::from_script(bank.clone(), "stolen=1; Path=/".into()),
            CookieUpdate::from_script(
                Location::from_input("https://cdn.example.com/").unwrap(),
                "own=1".into(),
            ),
        ];
        events.send(Ok(Event::Loaded(Box::new(spoofed)))).unwrap();
        let Some(Ok(Event::Loaded(loaded))) = worker.poll() else {
            panic!("expected the page");
        };
        assert_eq!(loaded.location, fetched.location);
        assert_eq!(loaded.base, fetched.location);
        assert_eq!(loaded.status, Some(200));
        assert!(!loaded.scripting_enabled);
        let cookies: Vec<_> = loaded
            .cookie_updates
            .iter()
            .map(|update| update.set_cookie.as_str())
            .collect();
        assert_eq!(cookies, ["fetched=1", "own=1"]);

        // Later updates are held to the same document, so a web page's links
        // still cannot reach local files.
        let mut update = page("https://bank.example/", b"").into_update();
        update.base = local;
        update.cookie_updates = vec![CookieUpdate::from_script(bank, "later=1".into())];
        events.send(Ok(Event::Updated(Box::new(update)))).unwrap();
        let Some(Ok(Event::Updated(update))) = worker.poll() else {
            panic!("expected an update");
        };
        assert_eq!(update.base, fetched.location);
        assert!(update.cookie_updates.is_empty());
        assert!(update.base.resolve("secret.html").unwrap().is_remote());
    }

    #[test]
    fn a_page_sent_before_its_document_was_fetched_is_rejected() {
        let (mut worker, events) = Worker::mock();
        let early = page("https://example.com/", b"<p>Early</p>");
        events.send(Ok(Event::Loaded(Box::new(early)))).unwrap();
        assert!(worker.poll().unwrap().is_err());
    }

    #[test]
    fn clicks_carry_only_the_page_sites_cookies() {
        let fetched = page("https://www.example.com/page", b"<p>Fetched</p>");
        let other = Location::from_input("https://bank.example/").unwrap();
        let (mut worker, _events) = Worker::mock_rendering(&fetched);
        let (sender, commands) = mpsc::sync_channel(1);
        worker.sender = Some(sender);
        let mut jar = CookieJar::default();
        assert!(jar.set_document_cookie(&fetched.location, "own=1"));
        assert!(jar.set_document_cookie(&other, "bank=1"));
        let target = crate::parse("<p>").unwrap().document.root();
        let click = |cookies| Command::Click {
            click: ClickRequest { target, href: None },
            cookies,
        };
        worker.send(click(jar)).unwrap();
        let Ok(Command::Click { mut cookies, .. }) = commands.try_recv() else {
            panic!("expected a click");
        };
        assert_eq!(cookies.document_cookie(&fetched.location), "own=1");
        assert_eq!(cookies.document_cookie(&other), "");
        // A worker renders only the document the browser fetched for it.
        worker.deadline = None;
        let document = LoadedDocument {
            location: other,
            bytes: Vec::new(),
            status: None,
            plain_text: false,
        };
        let render = Command::Render {
            document,
            scripting: false,
            cookies: CookieJar::default(),
            viewport: None,
        };
        assert!(worker.send(render).is_err());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn the_browser_fetches_the_document_and_hands_over_only_its_sites_cookies() {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let start = format!("http://{}/start", listener.local_addr().unwrap());
        let start = Location::from_input(&start).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut headers = String::new();
            while !headers.ends_with("\r\n\r\n") {
                reader.read_line(&mut headers).unwrap();
            }
            let body = "<title>Fetched</title>";
            write!(
                reader.get_mut(),
                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/html\r\nSet-Cookie: fresh=2; Path=/\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            headers.to_ascii_lowercase()
        });
        let other = Location::from_input("http://localhost:9/").unwrap();
        let mut jar = CookieJar::default();
        assert!(jar.set_document_cookie(&start, "own=1; Path=/"));
        assert!(jar.set_document_cookie(&other, "other=1"));
        let request = Request {
            target: Target::Load(start.clone()),
            scripting: ScriptPolicy::Page,
            cookies: jar,
            initiator: None,
            viewport: Some((800.0, 600.0)),
        };
        let (fetched, command) = fetch(request, Arc::default()).unwrap();
        assert!(server.join().unwrap().contains("cookie: own=1"));
        assert_eq!(fetched.location, start);
        assert!(fetched.scripting);
        assert_eq!(fetched.cookie_updates.len(), 1);
        let Command::Render {
            document,
            scripting,
            mut cookies,
            viewport,
        } = command
        else {
            panic!("expected a document");
        };
        assert!(scripting);
        assert_eq!(viewport, Some((800.0, 600.0)));
        assert_eq!(document.bytes, b"<title>Fetched</title>");
        assert_eq!(
            cookies.cookie_header(&start).as_deref(),
            Some("own=1; fresh=2")
        );
        assert_eq!(cookies.cookie_header(&other), None);
    }

    #[test]
    fn tab_processes_keep_only_proxy_certificate_and_system_variables() {
        let kept: Vec<_> = worker_environment(
            [
                ("https_proxy", "http://proxy.example:3128"),
                ("SSL_CERT_FILE", "/etc/ssl/cert.pem"),
                ("SystemRoot", "C:\\Windows"),
                ("PATH", "/usr/bin"),
                ("GITHUB_TOKEN", "secret"),
                ("AWS_SECRET_ACCESS_KEY", "secret"),
                ("OLIVE_PROFILE_DIR", "/tmp/profile"),
                ("LD_PRELOAD", "/tmp/library.so"),
                ("DYLD_INSERT_LIBRARIES", "/tmp/library.dylib"),
            ]
            .map(|(name, value)| (OsString::from(name), OsString::from(value))),
        )
        .map(|(name, _)| name.into_string().unwrap())
        .collect();
        assert_eq!(kept, ["https_proxy", "SSL_CERT_FILE", "SystemRoot", "PATH"]);
    }
}
