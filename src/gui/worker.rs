//! Private, framed pipe protocol and independently supervised document processes.
//! Pipe I/O and deserialization never block the UI. No listener or shared temp files.
//!
//! The browser fetches each document through its network process, keeping the
//! cookie jar to itself, and a fresh tab process renders it. Tab processes
//! have no network or file access: they ask the browser for each resource,
//! and the browser checks the request against the document it fetched. What a
//! tab process reports about its page's address, base and cookies is held to
//! that document too.
use crate::gui::document::{
    ClickRequest, LoadedPage, PageUpdate, PreparedPage, prepare_page_preview,
    prepare_page_with_parsed,
};
use crate::gui::network::{Fetch, NETWORK_ARG, Network};
use crate::net::{
    CookieJar, DocumentLoader, FormRequest, LoadedDocument, Location, MAX_RESOURCE_BYTES,
    RemoteFetch, ResourceRequest,
};
use crate::resources::{MAX_RESOURCES, RESOURCE_TIMEOUT};
use eframe::egui;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    io::{self, Read, Write},
    path::Path,
    process::{Child, Command as ProcessCommand, Stdio},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
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
/// Resources a tab process may request: its page's and its site icons.
const MAX_TAB_REQUESTS: usize = MAX_RESOURCES + 8;
/// Resource requests of one tab in flight at once.
const MAX_TAB_REQUESTS_IN_FLIGHT: usize = 32;
/// How long a tab process waits past a request's own deadline for the browser.
const FETCH_GRACE: Duration = Duration::from_secs(10);
/// The environment a sandboxed process keeps, compared without regard to
/// case: proxy and certificate settings for the network process, and what
/// the OS needs to start it. Everything else, such as tokens, stays in the
/// browser.
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

// Apple deprecates sandbox-exec. Both policies deny everything a process does
// not need on top of Apple's baseline for system processes: no writes,
// programs, other processes or system services beyond the baseline's.
#[cfg(target_os = "macos")]
macro_rules! macos_profile {
    ($($rules:literal)*) => {
        concat!(
            "(version 1)\n(deny default)\n(import \"system.sb\")\n",
            "(allow process-exec (literal (param \"WORKER\")))\n",
            "(allow signal (target self))\n",
            $($rules, "\n",)*
        )
    };
}
/// Tab processes read no files and open no connections.
#[cfg(target_os = "macos")]
const MACOS_TAB_SANDBOX_PROFILE: &str = macos_profile!();
/// The network process reaches websites, DNS, and the system keychain and
/// keychain service for root certificates, but no user files.
#[cfg(target_os = "macos")]
const MACOS_NETWORK_SANDBOX_PROFILE: &str = macos_profile!(
    "(allow system-socket (socket-domain AF_INET) (socket-domain AF_INET6) (socket-domain AF_UNIX))"
    "(allow network-outbound (remote ip) (literal \"/private/var/run/mDNSResponder\"))"
    "(allow mach-lookup (global-name \"com.apple.SecurityServer\"))"
    "(allow file-read* (subpath \"/Library/Keychains\"))"
);

/// What a sandboxed child process of the browser does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Renders one document.
    Tab,
    /// Makes the browser's HTTP(S) requests.
    Network,
}

impl Role {
    pub fn argument(self) -> &'static str {
        match self {
            Self::Tab => WORKER_ARG,
            Self::Network => NETWORK_ARG,
        }
    }

    pub fn from_argument(argument: &OsStr) -> Option<Self> {
        [Self::Tab, Self::Network]
            .into_iter()
            .find(|role| OsStr::new(role.argument()) == argument)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Tab => "tab worker",
            Self::Network => "network process",
        }
    }

    #[cfg(target_os = "macos")]
    fn macos_profile(self) -> &'static str {
        match self {
            Self::Tab => MACOS_TAB_SANDBOX_PROFILE,
            Self::Network => MACOS_NETWORK_SANDBOX_PROFILE,
        }
    }
}

/// What the browser shares with its tab processes: the cookie jar, which
/// stays in the browser, and the network process that fetches for them.
#[derive(Clone)]
pub struct Services {
    pub cookies: Arc<Mutex<CookieJar>>,
    pub network: Arc<Network>,
}

impl Services {
    pub fn new(network: Network) -> Self {
        Self {
            cookies: Arc::default(),
            network: Arc::new(network),
        }
    }

    pub fn jar(&self) -> MutexGuard<'_, CookieJar> {
        self.cookies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

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

/// A resource the browser fetched for a tab process.
#[derive(Serialize, Deserialize)]
pub struct FetchedResource {
    pub location: Location,
    #[serde(with = "crate::gui::network::base64_bytes")]
    pub bytes: Vec<u8>,
}

/// Browser-to-worker messages. A worker renders exactly one document, sent
/// first; later commands act on that document.
#[derive(Serialize, Deserialize)]
pub enum Command {
    /// The document the browser fetched, with the cookies its scripts may read.
    Render {
        document: LoadedDocument,
        scripting: bool,
        cookies: CookieJar,
        viewport: Option<(f32, f32)>,
    },
    /// `Worker::send` fills in the cookies the page's scripts may read,
    /// refreshed before the handler runs.
    Click {
        click: ClickRequest,
        cookies: CookieJar,
    },
    Resize {
        width: f32,
        height: f32,
    },
    /// The answer to the worker's resource request `id`.
    Fetched {
        id: u64,
        result: Result<FetchedResource, String>,
    },
    /// Make one sandbox test attempt instead of rendering a document.
    #[cfg(feature = "sandbox-probe")]
    Probe(Probe),
}
#[derive(Serialize, Deserialize)]
pub enum Event {
    Loaded(Box<LoadedPage>),
    Updated(Box<PageUpdate>),
    ResourcesUpdated(Box<PageUpdate>),
    Error(String),
    /// A page resource the worker asks the browser for.
    Fetch {
        id: u64,
        request: ResourceRequest,
    },
    /// How a sandbox test attempt went.
    #[cfg(feature = "sandbox-probe")]
    Probed(Result<(), String>),
}

/// Something sandbox tests ask a tab or network process to attempt. Each
/// must fail unless the sandbox deliberately allows it.
#[cfg(feature = "sandbox-probe")]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Probe {
    Read(std::path::PathBuf),
    Write(std::path::PathBuf),
    /// Start a shell.
    Execute,
    Connect(std::net::SocketAddr),
    /// Connect to a Unix socket, such as an SSH agent's.
    #[cfg(unix)]
    ConnectLocal(std::path::PathBuf),
    /// Check whether another process could be sent a signal.
    #[cfg(unix)]
    Signal(u32),
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
    /// Stops the browser's fetches for the tab when the tab stops.
    stopped: Arc<AtomicBool>,
    services: Services,
}

/// What the browser knows of the document a worker renders, independently of
/// what the worker reports.
struct Fetched {
    location: Location,
    status: Option<u16>,
    scripting: bool,
}

impl Worker {
    /// Fetch the document of `request` and render it in a fresh tab process.
    pub fn spawn(
        request: Request,
        services: &Services,
        ctx: &egui::Context,
    ) -> Result<Self, String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        Self::spawn_at(&executable, request, services, ctx)
    }

    pub fn spawn_at(
        executable: &Path,
        request: Request,
        services: &Services,
        ctx: &egui::Context,
    ) -> Result<Self, String> {
        let mut child = start_process(executable, Role::Tab)?;
        let mut input = child.stdin.take().ok_or("Tab process has no input pipe")?;
        let mut output = child
            .stdout
            .take()
            .ok_or("Tab process has no output pipe")?;
        // Answers to resource requests share the channel with commands.
        let (sender, commands) = mpsc::sync_channel(MAX_TAB_REQUESTS_IN_FLIGHT);
        let (events_tx, events) = mpsc::sync_channel(1);
        let (fetched_tx, fetched) = mpsc::sync_channel(1);
        let render = sender.clone();
        let worker = Self {
            child: Some(child),
            sender: Some(sender.clone()),
            events,
            deadline: Some(Instant::now() + LOAD_TIMEOUT),
            fetched,
            document: None,
            started: SystemTime::now(),
            stopped: Arc::new(AtomicBool::new(false)),
            services: services.clone(),
        };
        let resources = Arc::new(TabResources {
            services: services.clone(),
            document: OnceLock::new(),
            answers: sender,
            stopped: worker.stopped.clone(),
            requested: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
        });
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
        let serving = resources.clone();
        std::thread::Builder::new()
            .name("olive-tab-output".into())
            .spawn(move || {
                loop {
                    let event = read_event(&mut output)
                        .map_err(|e| format!("Tab process stopped or sent invalid data: {e}"));
                    // Resource requests are the browser's to answer, not the UI's.
                    let event = match event {
                        Ok(Event::Fetch { id, request }) => {
                            serving.serve(id, request);
                            continue;
                        }
                        event => event,
                    };
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
        let (services, stopped) = (services.clone(), worker.stopped.clone());
        std::thread::Builder::new()
            .name("olive-tab-fetch".into())
            .spawn(move || match fetch(request, &services, &stopped) {
                Ok((document, command)) => {
                    // The browser's record is in place before the worker
                    // can answer or ask for resources.
                    let _ = resources.document.set(document.location.clone());
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
            Command::Render { .. } | Command::Fetched { .. } => {
                return Err("The browser sends a tab its document and resources itself.".into());
            }
            Command::Click { cookies, .. } => {
                *cookies = self
                    .document
                    .as_ref()
                    .map(|document| self.services.jar().for_script(&document.location))
                    .unwrap_or_default();
                CLICK_TIMEOUT
            }
            Command::Resize { .. } => CLICK_TIMEOUT,
            #[cfg(feature = "sandbox-probe")]
            Command::Probe(_) => return Err("Probes run in a tab process of their own.".into()),
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
    /// base stays on the web, and its scripts' cookie changes, applied here,
    /// stay on the page's site.
    fn confine(&mut self, mut event: Event) -> Result<Event, String> {
        let (base, updates) = match &mut event {
            Event::Loaded(page) => (&mut page.base, &mut page.cookie_updates),
            Event::Updated(update) | Event::ResourcesUpdated(update) => {
                (&mut update.base, &mut update.cookie_updates)
            }
            Event::Error(_) => return Ok(event),
            Event::Fetch { .. } => return Err("The tab process sent an unexpected request.".into()),
            #[cfg(feature = "sandbox-probe")]
            Event::Probed(_) => return Ok(event),
        };
        // The browser records its fetch before sending the document, so the
        // record is in by the time the worker's first page arrives.
        if self.document.is_none() {
            self.document = self.fetched.try_recv().ok();
        }
        let document = self
            .document
            .as_ref()
            .ok_or("The tab process sent a page before its document was fetched.")?;
        if document.location.is_remote() && !base.is_remote() {
            *base = document.location.clone();
        }
        let confined: Vec<_> = std::mem::take(updates)
            .into_iter()
            .filter_map(|update| update.confine(&document.location, self.started))
            .collect();
        self.services.jar().apply_updates(&confined);
        if let Event::Loaded(page) = &mut event {
            page.location = document.location.clone();
            page.status = document.status;
            page.scripting_enabled = document.scripting;
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
                services: Services::new(Network::new("olive-network-mock")),
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
        });
        (worker, sender)
    }
}

/// Fetch a navigation's document, through the network process with the whole
/// cookie jar or from a local file, and prepare the worker's command to
/// render it.
fn fetch(
    request: Request,
    services: &Services,
    stopped: &Arc<AtomicBool>,
) -> Result<(Fetched, Command), String> {
    let Request {
        target,
        scripting,
        initiator,
        viewport,
    } = request;
    let (requested, fetch) = match target {
        Target::Load(location) if !location.is_remote() => {
            let document = DocumentLoader::local(LOAD_TIMEOUT).load(location.clone())?;
            (location, Err(document))
        }
        Target::Load(location) => (
            location.clone(),
            Ok(Fetch::Document {
                location,
                initiator,
            }),
        ),
        Target::Submit(request) => (
            request.location.clone(),
            Ok(Fetch::Form { request, initiator }),
        ),
    };
    let document = match fetch {
        Ok(fetch) => {
            let jar = services.jar().clone();
            let response = services.network.fetch(fetch, jar, LOAD_TIMEOUT, stopped)?;
            services.jar().apply_updates(&response.cookie_updates);
            response.into_document()
        }
        Err(local) => local,
    };
    let scripting = scripting.enabled_for(&requested, &document.location);
    let fetched = Fetched {
        location: document.location.clone(),
        status: document.status,
        scripting,
    };
    let command = Command::Render {
        cookies: services.jar().for_script(&document.location),
        document,
        scripting,
        viewport,
    };
    Ok((fetched, command))
}

/// Read and check one message from a tab process: the browser's whole
/// handling of untrusted tab output before it reaches the UI.
pub fn read_event(reader: &mut impl Read) -> io::Result<Event> {
    let event = read_frame::<Event>(reader, MAX_EVENT_BYTES)?;
    let pages = match &event {
        Event::Loaded(page) => Some((&page.page, &page.reading)),
        Event::Updated(update) | Event::ResourcesUpdated(update) => {
            Some((&update.page, &update.reading))
        }
        Event::Error(_) | Event::Fetch { .. } => None,
        #[cfg(feature = "sandbox-probe")]
        Event::Probed(_) => None,
    };
    if let Some((page, reading)) = pages {
        page.validate()
            .and_then(|()| reading.validate())
            .map_err(io::Error::other)?;
    }
    Ok(event)
}

/// Answers a tab process's resource requests in the browser.
struct TabResources {
    services: Services,
    /// The document the browser fetched for the tab, once it has.
    document: OnceLock<Location>,
    answers: SyncSender<Command>,
    stopped: Arc<AtomicBool>,
    requested: AtomicUsize,
    in_flight: AtomicUsize,
}

impl TabResources {
    fn serve(self: &Arc<Self>, id: u64, request: ResourceRequest) {
        // The input thread drains answers independently, so waiting here for
        // room cannot stall it.
        let refuse = |error: &str| {
            let _ = self.answers.send(Command::Fetched {
                id,
                result: Err(error.into()),
            });
        };
        let Some(document) = self.document.get().cloned() else {
            return refuse("The tab has no document to load resources for.");
        };
        if self.requested.fetch_add(1, Ordering::Relaxed) >= MAX_TAB_REQUESTS {
            return refuse("The page requested too many resources.");
        }
        if self.in_flight.fetch_add(1, Ordering::Relaxed) >= MAX_TAB_REQUESTS_IN_FLIGHT {
            self.in_flight.fetch_sub(1, Ordering::Relaxed);
            return refuse("The page requested too many resources at once.");
        }
        let this = self.clone();
        let spawned = std::thread::Builder::new()
            .name("olive-tab-resource".into())
            .spawn(move || {
                let result = fetch_resource(&this.services, &document, request, &this.stopped);
                this.in_flight.fetch_sub(1, Ordering::Relaxed);
                let _ = this.answers.send(Command::Fetched { id, result });
            });
        if spawned.is_err() {
            self.in_flight.fetch_sub(1, Ordering::Relaxed);
            refuse("Could not start loading the resource.");
        }
    }
}

/// Load a tab's resource with the browser's record of its document: through
/// the network process with the document site's cookies, or from a local file
/// for a local document. The tab's word for its document is not taken.
fn fetch_resource(
    services: &Services,
    document: &Location,
    request: ResourceRequest,
    stopped: &AtomicBool,
) -> Result<FetchedResource, String> {
    let timeout = request.timeout.min(RESOURCE_TIMEOUT);
    let max_bytes = request.max_bytes.min(MAX_RESOURCE_BYTES);
    if request.location.is_remote() {
        let cookies = services.jar().for_document(document);
        let request = ResourceRequest {
            document: document.clone(),
            timeout,
            max_bytes,
            ..request
        };
        let response =
            services
                .network
                .fetch(Fetch::Resource(request), cookies, timeout, stopped)?;
        services.jar().apply_updates(&response.cookie_updates);
        Ok(FetchedResource {
            location: response.location,
            bytes: response.bytes,
        })
    } else {
        DocumentLoader::local(timeout)
            .load_resource(document, request.location, request.kind, timeout, max_bytes)
            .map(|resource| FetchedResource {
                location: resource.location,
                bytes: resource.bytes,
            })
    }
}

/// Start a sandboxed `role` process from `executable`, talking over pipes.
pub(crate) fn start_process(executable: &Path, role: Role) -> Result<Child, String> {
    let mut command = sandboxed_command(executable, role);
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    #[cfg(all(windows, test))]
    command.stderr(Stdio::inherit());
    #[cfg(not(all(windows, test)))]
    command.stderr(Stdio::null());
    command.arg(role.argument());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW: workers have no console/window.
    }
    command.spawn().map_err(|e| {
        #[cfg(target_os = "macos")]
        {
            format!("Could not start {} in the macOS sandbox: {e}", role.name())
        }
        #[cfg(target_os = "windows")]
        {
            format!(
                "Could not start {} in the Windows sandbox: {e}",
                role.name()
            )
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            format!("Could not start {}: {e}", role.name())
        }
    })
}

/// Ask a fresh tab process from `executable` to make one `probe` attempt in
/// its sandbox, and return how the attempt went.
#[cfg(feature = "sandbox-probe")]
pub fn probe_at(executable: &Path, probe: Probe) -> Result<Result<(), String>, String> {
    let mut child = start_process(executable, Role::Tab)?;
    let mut input = child.stdin.take().ok_or("Tab process has no input pipe")?;
    let mut output = child
        .stdout
        .take()
        .ok_or("Tab process has no output pipe")?;
    let reply = write_frame(&mut input, &Command::Probe(probe), MAX_COMMAND_BYTES)
        .and_then(|()| read_frame::<Event>(&mut output, MAX_EVENT_BYTES))
        .map_err(|e| e.to_string());
    let _ = child.kill();
    let _ = child.wait();
    match reply? {
        Event::Probed(result) => Ok(result),
        Event::Error(error) => Err(error),
        _ => Err("The tab process sent an unexpected reply.".into()),
    }
}

#[cfg(feature = "sandbox-probe")]
pub(crate) fn attempt_probe(probe: Probe) -> Result<(), String> {
    probe::attempt(probe)
}

fn sandboxed_command(executable: &Path, role: Role) -> ProcessCommand {
    #[cfg(target_os = "macos")]
    let mut command = {
        // The profile allows starting exactly this file, by its real path.
        let executable = executable
            .canonicalize()
            .unwrap_or_else(|_| executable.to_path_buf());
        let mut parameter = OsString::from("WORKER=");
        parameter.push(&executable);
        let mut command = ProcessCommand::new("/usr/bin/sandbox-exec");
        command
            .arg("-p")
            .arg(role.macos_profile())
            .arg("-D")
            .arg(parameter)
            .arg(&executable);
        command
    };
    #[cfg(windows)]
    let mut command = {
        let _ = role;
        let mut command = ProcessCommand::new(executable);
        command.arg(WINDOWS_LAUNCHER_ARG);
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = {
        let _ = role;
        ProcessCommand::new(executable)
    };
    command
        .env_clear()
        .envs(worker_environment(std::env::vars_os()));
    if let Some(directory) = executable.parent().filter(|path| path.is_dir()) {
        command.current_dir(directory);
    }
    command
}

/// The variables of `environment` that a sandboxed process keeps.
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
            eprintln!("Could not start a restricted process in the Windows sandbox: {error}");
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

/// Limit this process's counted heap to `bytes`; false when the executable
/// does not count its heap.
pub(crate) fn limit_heap(bytes: usize) -> bool {
    heap::limit(bytes)
}

/// Apply the Linux sandbox for `role` to this process.
#[cfg(target_os = "linux")]
pub(crate) fn restrict_linux(role: Role) -> Result<(), String> {
    linux_sandbox::restrict(role)
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.terminate();
    }
}

/// The worker's output pipe, shared by its main thread and its resource
/// requests.
#[derive(Clone)]
struct Output(Arc<Mutex<io::Stdout>>);

impl Output {
    fn send(&self, message: &impl Serialize) -> io::Result<()> {
        let mut output = self
            .0
            .lock()
            .map_err(|_| io::Error::other("tab output failed"))?;
        write_frame(&mut *output, message, MAX_EVENT_BYTES)
    }
}

/// Where answers to the worker's resource requests go, by request.
type Waiting = Arc<Mutex<HashMap<u64, SyncSender<Result<FetchedResource, String>>>>>;

/// Ask the browser for page resources over the worker's pipes.
fn remote_fetch(output: Output, waiting: Waiting) -> Arc<RemoteFetch> {
    let next = AtomicU64::new(1);
    Arc::new(move |request: ResourceRequest| {
        let id = next.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = mpsc::sync_channel(1);
        waiting
            .lock()
            .map_err(|_| "Resource requests failed.")?
            .insert(id, answer);
        let wait = request.timeout + FETCH_GRACE;
        let sent = output.send(&Event::Fetch { id, request });
        let result = sent.map_err(|error| error.to_string()).and_then(|()| {
            answered
                .recv_timeout(wait)
                .map_err(|_| "The browser did not answer in time.".to_string())
        });
        if let Ok(mut waiting) = waiting.lock() {
            waiting.remove(&id);
        }
        result?.map(|resource| LoadedDocument {
            location: resource.location,
            bytes: resource.bytes,
            status: None,
            plain_text: false,
        })
    })
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
    if let Err(error) = linux_sandbox::restrict(Role::Tab) {
        report_start_error(format!(
            "Could not start tab worker in the Linux sandbox: {error}"
        ));
        return Err(io::Error::other(error));
    }

    let waiting: Waiting = Arc::default();
    let (sender, commands) = mpsc::sync_channel(1);
    let answers = waiting.clone();
    std::thread::Builder::new()
        .name("olive-parent-watch".into())
        .spawn(move || {
            let mut input = io::stdin().lock();
            loop {
                match read_frame::<Command>(&mut input, MAX_COMMAND_BYTES) {
                    Ok(Command::Fetched { id, result }) => {
                        let answer = answers
                            .lock()
                            .ok()
                            .and_then(|mut waiting| waiting.remove(&id));
                        // A request that gave up waiting has no answer route.
                        if let Some(answer) = answer {
                            let _ = answer.send(result);
                        }
                    }
                    Ok(command) => {
                        if sender.try_send(command).is_err() {
                            std::process::exit(1);
                        }
                    }
                    Err(_) => std::process::exit(0),
                }
            }
        })?;
    let output = Output(Arc::new(Mutex::new(io::stdout())));
    let mut prepared: Option<PreparedPage> = None;
    for command in commands {
        let event = match command {
            Command::Render {
                document,
                scripting,
                cookies,
                viewport,
            } => render(
                &output,
                &mut prepared,
                document,
                Rendering {
                    scripting,
                    loader: DocumentLoader::remote(
                        remote_fetch(output.clone(), waiting.clone()),
                        LOAD_TIMEOUT,
                        cookies,
                    ),
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
            Command::Fetched { .. } => continue,
            #[cfg(feature = "sandbox-probe")]
            Command::Probe(probe) => Event::Probed(probe::attempt(probe)),
        };
        output.send(&event)?;
    }
    Ok(())
}

/// How a worker renders its document.
struct Rendering {
    scripting: bool,
    loader: DocumentLoader,
    viewport: Option<(f32, f32)>,
}

/// Stream the document's preview and, once its stylesheets are in, its styled
/// page; then load the remaining resources and run scripts. Every full
/// navigation gets a fresh process and document realm.
fn render(
    output: &Output,
    prepared: &mut Option<PreparedPage>,
    source: LoadedDocument,
    rendering: Rendering,
) -> io::Result<Event> {
    if prepared.is_some() {
        return Err(io::Error::other("Unexpected second document"));
    }
    let Rendering {
        scripting,
        loader,
        viewport,
    } = rendering;
    let viewport = viewport.map_or(crate::css::DEFAULT_VIEWPORT, |(width, height)| {
        (width.clamp(1.0, 10_000.0), height.clamp(1.0, 10_000.0))
    });
    let mut preview = None;
    let result = (|| {
        let (preview_page, parsed) = prepare_page_preview(&source, scripting, viewport)?;
        output
            .send(&InitialEvent::Loaded(&preview_page))
            .map_err(|error| error.to_string())?;
        preview = Some(preview_page);
        let mut progress_error = None;
        let mut progress = |update: PageUpdate| {
            if progress_error.is_none() {
                progress_error = output
                    .send(&Event::ResourcesUpdated(Box::new(update)))
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

#[allow(unsafe_code)]
#[path = "worker/heap.rs"]
mod heap;

// Sending a test signal on Unix goes through libc.
#[cfg(feature = "sandbox-probe")]
#[allow(unsafe_code)]
#[path = "worker/probe.rs"]
mod probe;

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
    // Grow with what arrives, so a sender cannot reserve the whole limit
    // with a four-byte prefix.
    let mut bytes = Vec::with_capacity(size.min(1024 * 1024));
    reader.take(size as u64).read_to_end(&mut bytes)?;
    if bytes.len() < size {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
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
        use crate::net::CookieUpdate;
        let fetched = page("https://www.example.com/page", b"<p>Fetched</p>");
        let bank = Location::from_input("https://bank.example/").unwrap();
        let cdn = Location::from_input("https://cdn.example.com/").unwrap();
        let local = Location::from_path(std::env::current_dir().unwrap().join("secret/")).unwrap();
        let (mut worker, events) = Worker::mock_rendering(&fetched);

        let mut spoofed = page("https://bank.example/account", b"<p>Spoofed</p>");
        spoofed.base = local.clone();
        spoofed.status = Some(500);
        spoofed.scripting_enabled = true;
        let mut response = CookieUpdate::from_script(cdn.clone(), "forged=1; HttpOnly".into());
        response.from_script = false;
        spoofed.cookie_updates = vec![
            CookieUpdate::from_script(bank.clone(), "stolen=1; Path=/".into()),
            CookieUpdate::from_script(cdn.clone(), "own=1".into()),
            response,
        ];
        events.send(Ok(Event::Loaded(Box::new(spoofed)))).unwrap();
        let Some(Ok(Event::Loaded(loaded))) = worker.poll() else {
            panic!("expected the page");
        };
        assert_eq!(loaded.location, fetched.location);
        assert_eq!(loaded.base, fetched.location);
        assert_eq!(loaded.status, Some(200));
        assert!(!loaded.scripting_enabled);
        // The browser applies the page's own script cookies itself. Tab
        // processes see no responses, so they cannot set HttpOnly cookies.
        assert!(loaded.cookie_updates.is_empty());
        let mut jar = worker.services.jar().clone();
        assert_eq!(jar.cookie_header(&cdn).as_deref(), Some("own=1"));
        assert_eq!(jar.cookie_header(&bank), None);

        // Later updates are held to the same document, so a web page's links
        // still cannot reach local files.
        let mut update = page("https://bank.example/", b"").into_update();
        update.base = local;
        update.cookie_updates = vec![CookieUpdate::from_script(bank.clone(), "later=1".into())];
        events.send(Ok(Event::Updated(Box::new(update)))).unwrap();
        let Some(Ok(Event::Updated(update))) = worker.poll() else {
            panic!("expected an update");
        };
        assert_eq!(update.base, fetched.location);
        assert_eq!(worker.services.jar().cookie_header(&bank), None);
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
    fn clicks_carry_only_the_cookies_the_pages_scripts_may_read() {
        use crate::net::CookieUpdate;
        let fetched = page("https://www.example.com/page", b"<p>Fetched</p>");
        let other = Location::from_input("https://bank.example/").unwrap();
        let (mut worker, _events) = Worker::mock_rendering(&fetched);
        let (sender, commands) = mpsc::sync_channel(1);
        worker.sender = Some(sender);
        let mut session =
            CookieUpdate::from_script(fetched.location.clone(), "session=1; HttpOnly".into());
        session.from_script = false;
        worker.services.jar().apply_updates(&[
            CookieUpdate::from_script(fetched.location.clone(), "own=1".into()),
            CookieUpdate::from_script(other.clone(), "bank=1".into()),
            session,
        ]);
        let target = crate::parse("<p>").unwrap().document.root();
        let click = Command::Click {
            click: ClickRequest { target, href: None },
            cookies: CookieJar::default(),
        };
        worker.send(click).unwrap();
        let Ok(Command::Click { mut cookies, .. }) = commands.try_recv() else {
            panic!("expected a click");
        };
        assert_eq!(
            cookies.cookie_header(&fetched.location).as_deref(),
            Some("own=1")
        );
        assert_eq!(cookies.cookie_header(&other), None);
        // The browser alone sends a tab its document and resources.
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
        let fetched = Command::Fetched {
            id: 1,
            result: Err("forged".into()),
        };
        assert!(worker.send(fetched).is_err());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn a_tab_process_refuses_to_start_without_the_counted_heap() {
        // Unit tests run on the system allocator, so no heap limit could apply.
        assert!(!heap::limit(usize::MAX));
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
