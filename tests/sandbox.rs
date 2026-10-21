#![cfg(feature = "sandbox-probe")]
// Ask real tab and network processes, started in their OS sandboxes exactly
// as the browser starts them, to attempt what the sandboxes forbid.
use olive_html::gui::{
    network,
    worker::{self, Probe},
};
use std::path::Path;

const EXECUTABLE: &str = env!("CARGO_BIN_EXE_olive-gui");

fn in_tab(probe: Probe) -> Result<(), String> {
    worker::probe_at(Path::new(EXECUTABLE), probe.clone())
        .unwrap_or_else(|error| panic!("no answer to {probe:?} from a tab: {error}"))
}

fn in_network(probe: Probe) -> Result<(), String> {
    network::probe_at(Path::new(EXECUTABLE), probe.clone())
        .unwrap_or_else(|error| panic!("no answer to {probe:?} from the network: {error}"))
}

/// A file in the user's temporary directory, outside every sandbox's reach.
fn private_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("private.txt");
    std::fs::write(&path, b"private").unwrap();
    path
}

#[test]
fn sandboxed_processes_cannot_write_files_or_start_programs() {
    let dir = tempfile::tempdir().unwrap();
    for (process, attempt) in [("tab", in_tab as fn(Probe) -> _), ("network", in_network)] {
        let path = dir.path().join(format!("escaped-from-{process}.txt"));
        assert!(attempt(Probe::Write(path.clone())).is_err(), "{process}");
        assert!(!path.exists(), "{process}");
        assert!(attempt(Probe::Execute).is_err(), "{process}");
    }
}

#[test]
fn tab_processes_cannot_reach_websites() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    // Only the network process reaches websites.
    assert_eq!(in_network(Probe::Connect(address)), Ok(()));
    let tab = in_tab(Probe::Connect(address));
    // A low-integrity Windows token cannot deny connections; that needs an
    // AppContainer.
    if cfg!(windows) {
        assert_eq!(tab, Ok(()));
    } else {
        assert!(tab.is_err());
    }
}

#[test]
fn tab_and_network_processes_cannot_read_the_users_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = private_file(&dir);
    assert!(std::fs::read(&path).is_ok());
    for (process, attempt) in [("tab", in_tab as fn(Probe) -> _), ("network", in_network)] {
        let read = attempt(Probe::Read(path.clone()));
        // A low-integrity Windows token cannot deny reads either.
        if cfg!(windows) {
            assert_eq!(read, Ok(()), "{process}");
        } else {
            assert!(read.is_err(), "{process}");
        }
    }
}

#[cfg(unix)]
#[test]
fn sandboxed_processes_cannot_reach_local_sockets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    // The socket accepts connections from outside the sandbox.
    assert!(std::os::unix::net::UnixStream::connect(&path).is_ok());
    assert!(in_tab(Probe::ConnectLocal(path.clone())).is_err());
    assert!(in_network(Probe::ConnectLocal(path)).is_err());
}

#[cfg(unix)]
#[test]
fn sandboxed_processes_cannot_signal_other_processes() {
    // The test process starts each sandboxed process, so it is their parent.
    assert!(in_tab(Probe::Signal(std::process::id())).is_err());
    assert!(in_network(Probe::Signal(std::process::id())).is_err());
}
