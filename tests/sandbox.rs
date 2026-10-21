#![cfg(feature = "sandbox-probe")]
// Ask real tab processes, started in their OS sandbox exactly as the browser
// starts them, to attempt what the sandbox forbids.
use olive_html::gui::worker::{self, Probe};
use std::path::Path;

fn attempt(probe: Probe) -> Result<(), String> {
    worker::probe_at(Path::new(env!("CARGO_BIN_EXE_olive-gui")), probe.clone())
        .unwrap_or_else(|error| panic!("no answer to {probe:?}: {error}"))
}

#[test]
fn tab_processes_cannot_write_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("escaped.txt");
    assert!(attempt(Probe::Write(path.clone())).is_err());
    assert!(!path.exists());
}

#[test]
fn tab_processes_cannot_start_programs() {
    let error = attempt(Probe::Execute).unwrap_err();
    assert!(!error.is_empty());
}

#[test]
fn tab_processes_still_read_files_and_reach_websites() {
    // Page resources still load inside the tab process.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    assert_eq!(attempt(Probe::Read(manifest)), Ok(()));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    assert_eq!(attempt(Probe::Connect(address)), Ok(()));
}

#[cfg(unix)]
#[test]
fn tab_processes_cannot_reach_local_sockets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    // The socket accepts connections from outside the sandbox.
    assert!(std::os::unix::net::UnixStream::connect(&path).is_ok());
    assert!(attempt(Probe::ConnectLocal(path)).is_err());
}

#[cfg(unix)]
#[test]
fn tab_processes_cannot_signal_other_processes() {
    // The test process starts the tab process, so it is the tab's parent.
    assert!(attempt(Probe::Signal(std::process::id())).is_err());
}
