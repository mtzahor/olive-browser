//! The network process: one sandboxed child of the browser that makes every
//! HTTP(S) request. Tab processes have no network or file access; they ask
//! the browser for each page resource, and the browser checks the request
//! against the document it fetched for the tab before passing it here. The
//! browser keeps the cookie jar and sends each request only the cookies it
//! needs; it reads local files itself.
use crate::gui::worker::{self, Role, read_frame, write_frame};
use crate::net::{
    CookieJar, CookieUpdate, DocumentLoader, FormRequest, HttpClient, LoadedDocument, Location,
    ResourceRequest,
};
use serde::{Deserialize, Serialize};
use std::{
    cell::Cell,
    collections::HashMap,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Child,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    time::{Duration, Instant},
};

pub const NETWORK_ARG: &str = "--olive-network";
const MAX_COMMAND_BYTES: usize = 1024 * 1024;
// A response of up to 8 MiB crosses as base64 text.
const MAX_EVENT_BYTES: usize = 32 * 1024 * 1024;
/// Requests the network process runs at once; more wait for a free slot.
const MAX_REQUESTS: usize = 64;
/// The most heap the network process may use.
pub const MAX_NETWORK_HEAP_BYTES: usize = 2 * 1024 * 1024 * 1024;
/// Downloads have the loader's default deadline.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(20);
/// How long the browser waits past a request's own deadline.
const ANSWER_GRACE: Duration = Duration::from_secs(5);
/// How long a download may go without news before the browser stops it.
const DOWNLOAD_STALL: Duration = Duration::from_secs(60);
const STOPPED: &str = "Loading stopped.";

/// What the browser asks the network process to fetch.
#[derive(Serialize, Deserialize)]
pub enum Fetch {
    /// A navigation's document. `initiator` is the page that started it, for
    /// SameSite cookie rules.
    Document {
        location: Location,
        initiator: Option<Location>,
    },
    Form {
        request: FormRequest,
        initiator: Option<Location>,
    },
    /// A page resource, with the browser's record of the document.
    Resource(ResourceRequest),
}

/// What a fetch returned: the final address, and the bytes decoded to UTF-8
/// text for documents, stylesheets and scripts or as sent for images.
#[derive(Serialize, Deserialize)]
pub struct Response {
    pub location: Location,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
    pub status: Option<u16>,
    pub plain_text: bool,
    /// Cookie changes from the responses, for the browser's jar.
    pub cookie_updates: Vec<CookieUpdate>,
}

impl Response {
    pub fn into_document(self) -> LoadedDocument {
        LoadedDocument {
            location: self.location,
            bytes: self.bytes,
            status: self.status,
            plain_text: self.plain_text,
        }
    }
}

#[derive(Serialize, Deserialize)]
enum NetCommand {
    Fetch {
        id: u64,
        fetch: Fetch,
        cookies: CookieJar,
        timeout: Duration,
    },
    Download {
        id: u64,
        location: Location,
    },
    Cancel {
        id: u64,
    },
    #[cfg(feature = "sandbox-probe")]
    Probe {
        id: u64,
        probe: worker::Probe,
    },
}

#[derive(Serialize, Deserialize)]
enum NetEvent {
    Fetched {
        id: u64,
        result: Result<Response, String>,
    },
    Chunk {
        id: u64,
        #[serde(with = "base64_bytes")]
        bytes: Vec<u8>,
        total: Option<u64>,
    },
    Downloaded {
        id: u64,
        result: Result<u64, String>,
    },
    #[cfg(feature = "sandbox-probe")]
    Probed { id: u64, result: Result<(), String> },
}

impl NetEvent {
    fn id(&self) -> u64 {
        match self {
            Self::Fetched { id, .. } | Self::Chunk { id, .. } | Self::Downloaded { id, .. } => *id,
            #[cfg(feature = "sandbox-probe")]
            Self::Probed { id, .. } => *id,
        }
    }

    /// Whether the request has its last answer.
    fn finishes(&self) -> bool {
        !matches!(self, Self::Chunk { .. })
    }
}

/// Bytes as base64 text inside a JSON message.
pub(crate) mod base64_bytes {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        STANDARD.decode(text).map_err(D::Error::custom)
    }
}

/// The browser's connection to its network process, started on first use
/// and again after the process stops.
pub struct Network {
    executable: PathBuf,
    connection: Mutex<Option<Arc<Connection>>>,
    next_id: AtomicU64,
}

struct Connection {
    child: Mutex<Option<Child>>,
    commands: SyncSender<NetCommand>,
    /// Where each request's answers go, by request.
    pending: Mutex<HashMap<u64, SyncSender<NetEvent>>>,
    closed: AtomicBool,
}

impl Connection {
    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        // Waiting callers see their answers disconnect.
        if let Ok(mut pending) = self.pending.lock() {
            pending.clear();
        }
        if let Some(mut child) = self.child.lock().ok().and_then(|mut child| child.take()) {
            let _ = child.kill();
            let _ = std::thread::Builder::new()
                .name("olive-network-reaper".into())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
    }

    fn cancel(&self, id: u64) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&id);
        }
        let _ = self.commands.try_send(NetCommand::Cancel { id });
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.close();
    }
}

impl Network {
    /// A connection to network processes started from `executable`.
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            connection: Mutex::new(None),
            next_id: AtomicU64::new(1),
        }
    }

    /// A connection to network processes started from this executable.
    pub fn for_current_executable() -> Result<Self, String> {
        Ok(Self::new(
            std::env::current_exe().map_err(|e| e.to_string())?,
        ))
    }

    fn connection(&self) -> Result<Arc<Connection>, String> {
        let mut current = self
            .connection
            .lock()
            .map_err(|_| "The network connection failed.")?;
        if let Some(connection) = current
            .as_ref()
            .filter(|connection| !connection.closed.load(Ordering::Relaxed))
        {
            return Ok(connection.clone());
        }
        let connection = connect(&self.executable)?;
        *current = Some(connection.clone());
        Ok(connection)
    }

    /// Send the command `request` builds for a new request id, and return
    /// where its answers arrive.
    fn start(
        &self,
        capacity: usize,
        request: impl FnOnce(u64) -> NetCommand,
    ) -> Result<(Arc<Connection>, u64, Receiver<NetEvent>), String> {
        let connection = self.connection()?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, answers) = mpsc::sync_channel(capacity);
        connection
            .pending
            .lock()
            .map_err(|_| "The network connection failed.")?
            .insert(id, sender);
        if connection.commands.send(request(id)).is_err() {
            connection.close();
            return Err("The network process stopped.".into());
        }
        Ok((connection, id, answers))
    }

    /// Fetch through the network process with `cookies`, waiting at most
    /// `timeout` and a grace period, or until `stopped` is set.
    pub fn fetch(
        &self,
        fetch: Fetch,
        cookies: CookieJar,
        timeout: Duration,
        stopped: &AtomicBool,
    ) -> Result<Response, String> {
        let (connection, id, answers) = self.start(1, |id| NetCommand::Fetch {
            id,
            fetch,
            cookies,
            timeout,
        })?;
        let deadline = Instant::now() + timeout + ANSWER_GRACE;
        loop {
            match answers.recv_timeout(Duration::from_millis(50)) {
                Ok(NetEvent::Fetched { result, .. }) => return result,
                Ok(_) => return Err("The network process sent an unexpected reply.".into()),
                Err(RecvTimeoutError::Timeout) => {
                    if stopped.load(Ordering::Relaxed) {
                        connection.cancel(id);
                        return Err(STOPPED.into());
                    }
                    if Instant::now() >= deadline {
                        connection.cancel(id);
                        return Err("The network process did not answer in time.".into());
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("The network process stopped.".into());
                }
            }
        }
    }

    /// Download `location`, handing each piece and the expected total size
    /// to `received`. An error from `received` cancels the download.
    pub fn download(
        &self,
        location: Location,
        mut received: impl FnMut(&[u8], Option<u64>) -> Result<(), String>,
    ) -> Result<u64, String> {
        let (connection, id, answers) =
            self.start(8, |id| NetCommand::Download { id, location })?;
        loop {
            match answers.recv_timeout(DOWNLOAD_STALL) {
                Ok(NetEvent::Chunk { bytes, total, .. }) => {
                    if let Err(error) = received(&bytes, total) {
                        connection.cancel(id);
                        return Err(error);
                    }
                }
                Ok(NetEvent::Downloaded { result, .. }) => return result,
                Ok(_) => return Err("The network process sent an unexpected reply.".into()),
                Err(RecvTimeoutError::Timeout) => {
                    connection.cancel(id);
                    return Err("The download stopped making progress.".into());
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("The network process stopped.".into());
                }
            }
        }
    }
}

/// Start a network process from `executable` in its sandbox.
fn connect(executable: &Path) -> Result<Arc<Connection>, String> {
    let mut child = worker::start_process(executable, Role::Network)?;
    let mut input = child
        .stdin
        .take()
        .ok_or("The network process has no input pipe")?;
    let mut output = child
        .stdout
        .take()
        .ok_or("The network process has no output pipe")?;
    let (commands, queued) = mpsc::sync_channel::<NetCommand>(64);
    let connection = Arc::new(Connection {
        child: Mutex::new(Some(child)),
        commands,
        pending: Mutex::new(HashMap::new()),
        closed: AtomicBool::new(false),
    });
    let writer = Arc::downgrade(&connection);
    std::thread::Builder::new()
        .name("olive-network-input".into())
        .spawn(move || {
            for command in queued {
                if write_frame(&mut input, &command, MAX_COMMAND_BYTES).is_err() {
                    if let Some(connection) = writer.upgrade() {
                        connection.close();
                    }
                    break;
                }
            }
        })
        .map_err(|e| format!("Could not start network input: {e}"))?;
    let reader = Arc::downgrade(&connection);
    std::thread::Builder::new()
        .name("olive-network-output".into())
        .spawn(move || {
            while let Ok(event) = read_frame::<NetEvent>(&mut output, MAX_EVENT_BYTES) {
                let Some(connection) = reader.upgrade() else {
                    return;
                };
                let id = event.id();
                let finishes = event.finishes();
                let route = connection.pending.lock().ok().and_then(|mut pending| {
                    if finishes {
                        pending.remove(&id)
                    } else {
                        pending.get(&id).cloned()
                    }
                });
                // A request that stopped waiting no longer has a route.
                if let Some(route) = route {
                    let _ = route.send(event);
                }
            }
            if let Some(connection) = reader.upgrade() {
                connection.close();
            }
        })
        .map_err(|e| format!("Could not start network output: {e}"))?;
    Ok(connection)
}

/// Ask a fresh network process from `executable` to make one `probe`
/// attempt in its sandbox, and return how the attempt went.
#[cfg(feature = "sandbox-probe")]
pub fn probe_at(executable: &Path, probe: worker::Probe) -> Result<Result<(), String>, String> {
    let network = Network::new(executable);
    let (_connection, _id, answers) = network.start(1, |id| NetCommand::Probe { id, probe })?;
    match answers.recv_timeout(Duration::from_secs(30)) {
        Ok(NetEvent::Probed { result, .. }) => Ok(result),
        Ok(_) => Err("The network process sent an unexpected reply.".into()),
        Err(error) => Err(format!("The network process did not answer: {error}")),
    }
}

/// The network process: answer the browser's requests until it disconnects.
pub fn run() -> io::Result<()> {
    if !worker::limit_heap(MAX_NETWORK_HEAP_BYTES) {
        let error = "this executable does not count its heap, so no memory limit can apply";
        return Err(io::Error::other(error));
    }
    // Root certificates load here, before Linux restricts file reads.
    let client = HttpClient::new().map_err(io::Error::other)?;
    #[cfg(target_os = "linux")]
    worker::restrict_linux(Role::Network).map_err(io::Error::other)?;
    let output = Arc::new(Mutex::new(io::stdout()));
    let stops: Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>> = Arc::default();
    let active = Arc::new(AtomicUsize::new(0));
    let mut input = io::stdin().lock();
    // The browser closing its pipe ends the process, even mid-request.
    while let Ok(command) = read_frame::<NetCommand>(&mut input, MAX_COMMAND_BYTES) {
        type Work = Box<dyn FnOnce(Arc<AtomicBool>) -> NetEvent + Send>;
        let (id, work): (u64, Work) = match command {
            NetCommand::Cancel { id } => {
                if let Some(stopped) = stops.lock().ok().and_then(|stops| stops.get(&id).cloned()) {
                    stopped.store(true, Ordering::Relaxed);
                }
                continue;
            }
            #[cfg(feature = "sandbox-probe")]
            NetCommand::Probe { id, probe } => {
                let result = worker::attempt_probe(probe);
                send(&output, &NetEvent::Probed { id, result })?;
                continue;
            }
            NetCommand::Fetch {
                id,
                fetch,
                cookies,
                timeout,
            } => {
                let client = client.clone();
                (
                    id,
                    Box::new(move |stopped| NetEvent::Fetched {
                        id,
                        result: answer(&client, fetch, cookies, timeout, stopped),
                    }),
                )
            }
            NetCommand::Download { id, location } => {
                let client = client.clone();
                let output = output.clone();
                (
                    id,
                    Box::new(move |stopped| NetEvent::Downloaded {
                        id,
                        result: download(&client, id, location, &output, stopped),
                    }),
                )
            }
        };
        let stopped = Arc::new(AtomicBool::new(false));
        if let Ok(mut stops) = stops.lock() {
            stops.insert(id, stopped.clone());
        }
        let (output, stops, active) = (output.clone(), stops.clone(), active.clone());
        // Requests wait for a free slot in their own thread, so the browser's
        // cancellations are read while they wait.
        std::thread::Builder::new()
            .name("olive-network-request".into())
            .spawn(move || {
                let mut running = active.load(Ordering::Relaxed);
                loop {
                    if running < MAX_REQUESTS {
                        match active.compare_exchange_weak(
                            running,
                            running + 1,
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                        ) {
                            Ok(_) => break,
                            Err(now) => running = now,
                        }
                    } else {
                        std::thread::sleep(Duration::from_millis(5));
                        running = active.load(Ordering::Relaxed);
                    }
                }
                let event = work(stopped);
                if let Ok(mut stops) = stops.lock() {
                    stops.remove(&id);
                }
                active.fetch_sub(1, Ordering::Relaxed);
                if send(&output, &event).is_err() {
                    std::process::exit(0);
                }
            })?;
    }
    Ok(())
}

fn send(output: &Mutex<io::Stdout>, event: &NetEvent) -> io::Result<()> {
    let mut output = output
        .lock()
        .map_err(|_| io::Error::other("network output failed"))?;
    write_frame(&mut *output, event, MAX_EVENT_BYTES)
}

fn answer(
    client: &HttpClient,
    fetch: Fetch,
    cookies: CookieJar,
    timeout: Duration,
    stopped: Arc<AtomicBool>,
) -> Result<Response, String> {
    let loader = |initiator| {
        DocumentLoader::with_client(client, timeout, cookies)
            .with_cookie_initiator(initiator)
            .with_stop(stopped)
    };
    let (loader, result) = match fetch {
        Fetch::Document {
            location,
            initiator,
        } => {
            let loader = loader(initiator);
            let result = loader.load(location);
            (loader, result)
        }
        Fetch::Form { request, initiator } => {
            let loader = loader(initiator);
            let result = loader.submit(request);
            (loader, result)
        }
        Fetch::Resource(request) => {
            let loader = loader(None);
            let result = loader
                .load_resource(
                    &request.document,
                    request.location,
                    request.kind,
                    request.timeout,
                    request.max_bytes,
                )
                .map(|resource| LoadedDocument {
                    location: resource.location,
                    bytes: resource.bytes,
                    status: None,
                    plain_text: false,
                });
            (loader, result)
        }
    };
    result.map(|document| Response {
        location: document.location,
        bytes: document.bytes,
        status: document.status,
        plain_text: document.plain_text,
        cookie_updates: loader.take_cookie_updates(),
    })
}

fn download(
    client: &HttpClient,
    id: u64,
    location: Location,
    output: &Mutex<io::Stdout>,
    stopped: Arc<AtomicBool>,
) -> Result<u64, String> {
    let loader = DocumentLoader::with_client(client, DOWNLOAD_TIMEOUT, CookieJar::default())
        .with_stop(stopped.clone());
    let total = Cell::new(None);
    let mut chunks = Chunks {
        id,
        output,
        total: &total,
    };
    loader.download(location, &mut chunks, |_, size| {
        total.set(size);
        if stopped.load(Ordering::Relaxed) {
            Err("Download cancelled.".into())
        } else {
            Ok(())
        }
    })
}

/// Sends a download's bytes to the browser as they arrive.
struct Chunks<'a> {
    id: u64,
    output: &'a Mutex<io::Stdout>,
    total: &'a Cell<Option<u64>>,
}

impl Write for Chunks<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        send(
            self.output,
            &NetEvent::Chunk {
                id: self.id,
                bytes: bytes.to_vec(),
                total: self.total.get(),
            },
        )?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
