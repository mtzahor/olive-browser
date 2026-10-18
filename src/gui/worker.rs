//! Private, framed pipe protocol and independently supervised document processes.
//! Pipe I/O and deserialization never block the UI. No listener or shared temp files.
use crate::document::{
    ClickRequest, LoadedPage, PageUpdate, prepare_page_preview, prepare_page_with_parsed,
};
use eframe::egui;
use olive_html::net::{CookieJar, DocumentLoader, FormRequest, Location};
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

#[derive(Serialize, Deserialize)]
pub enum Command {
    Load {
        location: Location,
        scripting: ScriptPolicy,
    },
    LoadWithCookies {
        location: Location,
        scripting: ScriptPolicy,
        cookies: CookieJar,
        initiator: Option<Location>,
    },
    Click(ClickRequest),
    ClickWithCookies {
        click: ClickRequest,
        cookies: CookieJar,
    },
    Submit(FormRequest),
    SubmitWithCookies {
        request: FormRequest,
        scripting: ScriptPolicy,
        cookies: CookieJar,
        initiator: Option<Location>,
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
    #[allow(dead_code)]
    pub fn spawn(location: Location, scripting: bool, ctx: &egui::Context) -> Result<Self, String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        Self::spawn_at(&executable, location, scripting, ctx)
    }

    #[allow(dead_code)]
    pub fn spawn_at(
        executable: &Path,
        location: Location,
        scripting: bool,
        ctx: &egui::Context,
    ) -> Result<Self, String> {
        Self::spawn_command_at(
            executable,
            Command::Load {
                location,
                scripting: ScriptPolicy::for_load(scripting, false),
            },
            ctx,
        )
    }

    pub fn spawn_with_cookies(
        location: Location,
        scripting: ScriptPolicy,
        cookies: CookieJar,
        initiator: Option<Location>,
        ctx: &egui::Context,
    ) -> Result<Self, String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        Self::spawn_command_at(
            &executable,
            Command::LoadWithCookies {
                location,
                scripting,
                cookies,
                initiator,
            },
            ctx,
        )
    }

    #[allow(dead_code)]
    pub fn spawn_form(request: FormRequest, ctx: &egui::Context) -> Result<Self, String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        Self::spawn_command_at(&executable, Command::Submit(request), ctx)
    }

    pub fn spawn_form_with_cookies(
        request: FormRequest,
        scripting: ScriptPolicy,
        cookies: CookieJar,
        initiator: Option<Location>,
        ctx: &egui::Context,
    ) -> Result<Self, String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        Self::spawn_command_at(
            &executable,
            Command::SubmitWithCookies {
                request,
                scripting,
                cookies,
                initiator,
            },
            ctx,
        )
    }

    pub fn spawn_command_at(
        executable: &Path,
        initial: Command,
        ctx: &egui::Context,
    ) -> Result<Self, String> {
        let mut command = tab_worker_command(executable);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
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
            Command::Load { .. }
            | Command::LoadWithCookies { .. }
            | Command::Submit(_)
            | Command::SubmitWithCookies { .. } => LOAD_TIMEOUT,
            Command::Click(_) | Command::ClickWithCookies { .. } | Command::Resize { .. } => {
                CLICK_TIMEOUT
            }
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
    let mut prepared: Option<crate::document::PreparedPage> = None;
    for command in commands {
        let (command, form, cookies, initiator) = match command {
            Command::ClickWithCookies { click, cookies } => {
                if let Some(session) = prepared.as_mut().and_then(|page| page.session.as_mut()) {
                    session.replace_cookies(cookies);
                }
                (Command::Click(click), None, CookieJar::default(), None)
            }
            Command::Submit(request) => (
                Command::Load {
                    location: request.location.clone(),
                    scripting: ScriptPolicy::Disabled,
                },
                Some(request),
                CookieJar::default(),
                None,
            ),
            Command::SubmitWithCookies {
                request,
                scripting,
                cookies,
                initiator,
            } => (
                Command::Load {
                    location: request.location.clone(),
                    scripting,
                },
                Some(request),
                cookies,
                initiator,
            ),
            Command::LoadWithCookies {
                location,
                scripting,
                cookies,
                initiator,
            } => (
                Command::Load {
                    location,
                    scripting,
                },
                None,
                cookies,
                initiator,
            ),
            command => (command, None, CookieJar::default(), None),
        };
        let event = match command {
            Command::Load {
                location,
                scripting,
            } => {
                // Every full navigation gets a fresh process and document realm.
                if prepared.is_some() {
                    return Err(io::Error::other("Unexpected second load"));
                }
                let mut preview = None;
                let result = (|| {
                    let loader = DocumentLoader::with_timeout_and_cookies(LOAD_TIMEOUT, cookies)?
                        .with_cookie_initiator(initiator);
                    let source = if let Some(request) = form {
                        loader.submit(request)?
                    } else {
                        loader.load(location.clone())?
                    };
                    let scripting = scripting.enabled_for(&location, &source.location);
                    let (preview_page, parsed) = prepare_page_preview(&source, scripting)?;
                    write_frame(
                        &mut output,
                        &InitialEvent::Loaded(&preview_page),
                        MAX_EVENT_BYTES,
                    )
                    .map_err(|error| error.to_string())?;
                    preview = Some(preview_page);
                    prepare_page_with_parsed(source, &loader, scripting, parsed)
                })();
                match result {
                    Ok(mut page) => {
                        if let Some(session) = &mut page.session {
                            session.take_alerts();
                        }
                        let presentation_available = page.presentation_available();
                        let update = PageUpdate {
                            default_allowed: false,
                            page: std::mem::take(&mut page.loaded.page),
                            reading: std::mem::take(&mut page.loaded.reading),
                            base: page.loaded.base.clone(),
                            scripts: page.loaded.scripts.clone(),
                            resources: page.loaded.resources.clone(),
                            session_available: page.session.is_some(),
                            presentation_available,
                            alert: None,
                            link: None,
                            cookie_updates: std::mem::take(&mut page.loaded.cookie_updates),
                        };
                        prepared = Some(page);
                        Event::ResourcesUpdated(Box::new(update))
                    }
                    Err(error) => {
                        if let Some(mut loaded) = preview {
                            loaded.resources.pending = false;
                            loaded.resources.diagnostics.push(error);
                            Event::ResourcesUpdated(Box::new(PageUpdate {
                                default_allowed: false,
                                page: loaded.page,
                                reading: loaded.reading,
                                base: loaded.base,
                                scripts: loaded.scripts,
                                resources: loaded.resources,
                                session_available: false,
                                presentation_available: false,
                                alert: None,
                                link: None,
                                cookie_updates: loaded.cookie_updates,
                            }))
                        } else {
                            Event::Error(error)
                        }
                    }
                }
            }
            Command::LoadWithCookies { .. } | Command::ClickWithCookies { .. } => unreachable!(),
            Command::Submit(_) | Command::SubmitWithCookies { .. } => {
                return Err(io::Error::other("Unexpected form command"));
            }
            Command::Click(click) => match prepared.as_mut().and_then(|page| page.click(click)) {
                Some(update) => Event::Updated(Box::new(update)),
                None => Event::Error("No live scripting session. Reload this tab.".into()),
            },
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
    use image::ImageEncoder;
    use serde::{Deserialize, Serialize};
    use std::sync::Arc;

    const MAX_ENCODED_IMAGE_BYTES: usize = 48 * 1024 * 1024;

    pub fn serialize<S: serde::Serializer>(
        image: &Arc<ColorImage>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let bytes: Vec<u8> = image
            .pixels
            .iter()
            .flat_map(|pixel| pixel.to_array())
            .collect();
        let mut compressed = Vec::new();
        image::codecs::png::PngEncoder::new(&mut compressed)
            .write_image(
                &bytes,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(serde::ser::Error::custom)?;
        (image.size, STANDARD.encode(compressed)).serialize(serializer)
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
            || encoded.len() > MAX_ENCODED_IMAGE_BYTES
        {
            return Err(serde::de::Error::custom("Tab image exceeds pixel budget"));
        }
        let encoded = STANDARD.decode(encoded).map_err(serde::de::Error::custom)?;
        let decoded = image::load_from_memory_with_format(&encoded, image::ImageFormat::Png)
            .map_err(serde::de::Error::custom)?
            .to_rgba8();
        if decoded.width() as usize != size[0] || decoded.height() as usize != size[1] {
            return Err(serde::de::Error::custom("Invalid image dimensions"));
        }
        let bytes = decoded.into_raw();
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
