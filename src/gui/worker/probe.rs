//! Sandbox test attempts, made inside a tab process after it is restricted.

use super::Probe;
use std::{io, time::Duration};

pub(super) fn attempt(probe: Probe) -> Result<(), String> {
    let result = match probe {
        Probe::Read(path) => std::fs::read(path).map(drop),
        Probe::Write(path) => std::fs::write(path, b"escaped"),
        Probe::Execute => {
            let mut shell = if cfg!(windows) {
                let mut shell = std::process::Command::new("cmd.exe");
                shell.args(["/C", "exit 0"]);
                shell
            } else {
                let mut shell = std::process::Command::new("/bin/sh");
                shell.args(["-c", "exit 0"]);
                shell
            };
            shell.status().map(drop)
        }
        Probe::Connect(address) => {
            std::net::TcpStream::connect_timeout(&address, Duration::from_secs(5)).map(drop)
        }
        #[cfg(unix)]
        Probe::ConnectLocal(path) => std::os::unix::net::UnixStream::connect(path).map(drop),
        #[cfg(unix)]
        Probe::Signal(pid) => libc::pid_t::try_from(pid)
            .map_err(io::Error::other)
            .and_then(|pid| {
                // SAFETY: signal 0 delivers nothing; kill only checks whether
                // the process exists and may be signalled.
                if unsafe { libc::kill(pid, 0) } == 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                }
            }),
    };
    result.map_err(|error| error.to_string())
}
