//! Raw, streaming user-initiated downloads. Nothing here parses or executes a file.
use super::*;
use std::io::Write;

pub const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;

impl DocumentLoader {
    /// Stream bytes to a caller-owned temporary file. `progress` can cancel before
    /// each read and after EOF. The caller alone decides when to publish the file.
    pub fn download(
        &self,
        mut location: Location,
        destination: &mut impl Write,
        mut progress: impl FnMut(u64, Option<u64>) -> Result<(), String>,
    ) -> Result<u64, String> {
        if let Some(initiator) = &self.cookie_initiator {
            check_resource_target(initiator, &location)?;
        }
        progress(0, None)?;
        if let Some(path) = location.file_path() {
            if !path.metadata().map_err(|e| e.to_string())?.is_file() {
                return Err("Choose a regular file to download.".into());
            }
            let file = File::open(path).map_err(|e| e.to_string())?;
            let metadata = file.metadata().map_err(|e| e.to_string())?;
            if !metadata.is_file() {
                return Err("Choose a regular file to download.".into());
            }
            return transfer(file, destination, Some(metadata.len()), progress);
        }
        let initial = location.clone();
        let mut same_site = self
            .cookie_initiator
            .as_ref()
            .is_none_or(|from| same_cookie_site(from, &location));
        let started = Instant::now();
        for redirects in 0..=10 {
            progress(0, None)?;
            let remaining = self.timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err("Download timed out.".into());
            }
            let mut request = self
                .client
                .get(location.url().clone())
                .timeout(remaining)
                .header(header::ACCEPT, "*/*");
            if let Some(cookie) = self
                .cookies
                .lock()
                .ok()
                .and_then(|mut jar| jar.cookie_header_for(&location, same_site, true, true))
            {
                request = request.header(header::COOKIE, cookie);
            }
            let response = request.send().map_err(request_error)?;
            for value in response.headers().get_all(header::SET_COOKIE) {
                let Ok(value) = value.to_str() else { continue };
                if self
                    .cookies
                    .lock()
                    .is_ok_and(|mut jar| jar.absorb(&location, value))
                {
                    if let Ok(mut updates) = self.cookie_updates.lock() {
                        CookieUpdate {
                            location: location.clone(),
                            set_cookie: value.into(),
                            from_script: false,
                            received_at: now_seconds(),
                        }
                        .record(&mut updates);
                    }
                }
            }
            if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                if let Some(target) = response.headers().get(header::LOCATION) {
                    if redirects == 10 {
                        return Err("Too many download redirects (maximum 10).".into());
                    }
                    let target = location
                        .resolve(target.to_str().map_err(|_| "Invalid redirect address")?)?;
                    // Validate every hop, including HTTP-to-HTTPS-to-HTTP chains.
                    check_resource_target(&initial, &target)?;
                    check_resource_target(&location, &target)?;
                    if let Some(initiator) = &self.cookie_initiator {
                        check_resource_target(initiator, &target)?;
                    }
                    same_site &= same_cookie_site(&location, &target);
                    location = target;
                    continue;
                }
            }
            if !response.status().is_success() {
                return Err(format!(
                    "Download returned HTTP {}.",
                    response.status().as_u16()
                ));
            }
            let total = response.content_length();
            return transfer(response, destination, total, progress);
        }
        Err("Too many download redirects.".into())
    }
}

fn transfer(
    mut source: impl Read,
    destination: &mut impl Write,
    total: Option<u64>,
    mut progress: impl FnMut(u64, Option<u64>) -> Result<(), String>,
) -> Result<u64, String> {
    if total.is_some_and(|total| total > MAX_DOWNLOAD_BYTES) {
        return Err("Download exceeds the 100 MiB limit.".into());
    }
    let mut received = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        progress(received, total)?;
        let count = source
            .read(&mut buffer)
            .map_err(|e| format!("Download failed: {e}"))?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if received > MAX_DOWNLOAD_BYTES {
            return Err("Download exceeds the 100 MiB limit.".into());
        }
        destination
            .write_all(&buffer[..count])
            .map_err(|e| format!("Could not save download: {e}"))?;
    }
    progress(received, total)?;
    if total.is_some_and(|total| total != received) {
        return Err("Download ended before the complete file arrived.".into());
    }
    Ok(received)
}

/// URL-derived suggestion only. Server-controlled names never select directories.
pub fn suggested_name(location: &Location) -> String {
    let name = location
        .url()
        .path_segments()
        .and_then(|mut parts| parts.next_back())
        .unwrap_or("");
    let decoded = percent_encoding::percent_decode_str(name).decode_utf8_lossy();
    let mut name: String = decoded
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .take(100)
        .collect();
    name = name.trim_matches([' ', '.']).into();
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if name.is_empty()
        || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (1..=9).any(|n| stem == format!("COM{n}") || stem == format!("LPT{n}"))
    {
        return "download".into();
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streams_binary_bytes_and_rejects_limits_truncation_and_cancellation() {
        let bytes = [0, 255, 13, 10, 128];
        let mut saved = Vec::new();
        assert_eq!(
            transfer(&bytes[..], &mut saved, Some(5), |_, _| Ok(())).unwrap(),
            5
        );
        assert_eq!(saved, bytes);
        assert!(
            transfer(
                &bytes[..],
                &mut Vec::new(),
                Some(MAX_DOWNLOAD_BYTES + 1),
                |_, _| Ok(())
            )
            .is_err()
        );
        assert!(transfer(&bytes[..], &mut Vec::new(), Some(6), |_, _| Ok(())).is_err());
        let mut saved = Vec::new();
        assert!(transfer(&bytes[..], &mut saved, None, |_, _| Err("Cancelled".into())).is_err());
        assert!(saved.is_empty());
    }
    #[test]
    fn filename_suggestions_cannot_escape_a_directory() {
        for (url, expected) in [
            ("https://example.com/", "download"),
            ("https://example.com/%2Ftmp%5Cfile.pdf", "_tmp_file.pdf"),
            ("https://example.com/CON.txt", "download"),
            ("https://example.com/report.pdf?token=secret", "report.pdf"),
        ] {
            assert_eq!(
                suggested_name(&Location::from_input(url).unwrap()),
                expected
            );
        }
    }
}
