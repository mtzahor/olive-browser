//! Marks a downloaded file as coming from the internet, so the operating
//! system checks it before opening: Gatekeeper's quarantine attribute on
//! macOS and the Mark of the Web on Windows. Other systems have no such mark.

use std::{fs::File, io, path::Path};

/// Mark `file`, saved at `path`, as downloaded from `url`. Filesystems without
/// extended attributes or alternate data streams cannot hold the mark.
#[cfg(target_os = "macos")]
pub(super) fn mark(file: &File, _path: &Path, _url: &str) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_secs());
    // Flags 0x0081 mark a download, as other browsers' downloads are marked.
    let value = format!("0081;{seconds:08x};Olive Browser;");
    // SAFETY: the descriptor belongs to the open `file`; the name is a
    // NUL-terminated literal and the value pointer and length describe one
    // live buffer. No position or options are passed.
    let result = unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            c"com.apple.quarantine".as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
            0,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
pub(super) fn mark(_file: &File, path: &Path, url: &str) -> io::Result<()> {
    // Zone 3 is the Internet zone. Olive rejects addresses with credentials,
    // so the address holds none.
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    std::fs::write(
        stream,
        format!("[ZoneTransfer]\r\nZoneId=3\r\nHostUrl={url}\r\n"),
    )
}

#[cfg(not(any(target_os = "macos", windows)))]
pub(super) fn mark(_file: &File, _path: &Path, _url: &str) -> io::Result<()> {
    Ok(())
}
