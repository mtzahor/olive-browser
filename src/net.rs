//! Explicit, bounded document loading and URL resolution. Parsing never fetches
//! subresources or executes scripts. Local files and HTTP(S) are the only schemes.

use crate::{ParseOptions, ParseOutput, parse_utf8};
use reqwest::{blocking::Client, header, redirect};
use std::{
    fs::File,
    io::Read,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
pub use url::Url;

mod body;
mod cookies;
pub mod download;
mod location;

use body::{decode_html_limit, decode_limit, limit_error, read_limit};
pub use cookies::{CookieJar, CookieUpdate};
use cookies::{now_seconds, same_cookie_site};
pub use location::Location;

pub const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_URL_BYTES: usize = 8192;
pub const MAX_RESOURCE_BYTES: usize = 8 * 1024 * 1024;
/// Resource types the viewer can explicitly request. No recursive fetching occurs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    Stylesheet,
    Script,
    Image,
}

pub struct LoadedResource {
    pub location: Location,
    /// Decoded text for stylesheets and scripts, or the original bytes for images.
    pub source: String,
    pub bytes: Vec<u8>,
}

pub struct LoadedDocument {
    /// Final address after redirects, including the navigation fragment.
    pub location: Location,
    pub bytes: Vec<u8>,
    pub status: Option<u16>,
    pub plain_text: bool,
}

impl LoadedDocument {
    pub fn parse(&self, scripting_enabled: bool) -> Result<ParseOutput, String> {
        let source;
        let bytes = if self.plain_text {
            let text = String::from_utf8_lossy(&self.bytes);
            source = format!(
                "<!doctype html><meta charset=utf-8><pre>{}</pre>",
                text.replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            );
            source.as_bytes()
        } else {
            &self.bytes
        };
        parse_utf8(
            bytes,
            ParseOptions {
                scripting_enabled,
                // Escaping plain text can expand each byte to six bytes.
                max_input_bytes: if self.plain_text {
                    MAX_DOCUMENT_BYTES * 6 + 128
                } else {
                    MAX_DOCUMENT_BYTES
                },
                ..ParseOptions::default()
            },
        )
        .map_err(|e| e.to_string())
    }
}

/// Reusable HTTP client with verified TLS, limited redirects and bounded reads.
pub struct DocumentLoader {
    client: Client,
    timeout: Duration,
    cookies: Arc<Mutex<CookieJar>>,
    cookie_updates: Arc<Mutex<Vec<CookieUpdate>>>,
    cookie_initiator: Option<Location>,
}

pub const MAX_FORM_BYTES: usize = 64 * 1024;

/// A user-initiated, UTF-8 URL-encoded form navigation. Bodies are never history data.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FormRequest {
    pub location: Location,
    pub body: Option<String>,
}

impl DocumentLoader {
    pub fn new() -> Result<Self, String> {
        Self::with_timeout(Duration::from_secs(20))
    }

    pub fn with_timeout(timeout: Duration) -> Result<Self, String> {
        Self::with_timeout_and_cookies(timeout, CookieJar::default())
    }

    pub fn with_timeout_and_cookies(timeout: Duration, cookies: CookieJar) -> Result<Self, String> {
        Client::builder()
            .user_agent(concat!("OliveBrowser/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(timeout.min(Duration::from_secs(10)))
            .timeout(timeout)
            .referer(false)
            // Resolve redirects ourselves before sending them. This preserves
            // URL fragments and validates credentials/schemes before HTTP URI
            // conversion, which can discard parts of a Location header.
            .redirect(redirect::Policy::none())
            .build()
            .map(|client| Self {
                client,
                timeout,
                cookies: Arc::new(Mutex::new(cookies)),
                cookie_updates: Arc::new(Mutex::new(Vec::new())),
                cookie_initiator: None,
            })
            .map_err(|e| format!("Could not initialize HTTP: {e}"))
    }

    pub fn cookies(&self) -> CookieJar {
        self.cookies
            .lock()
            .map_or_else(|_| CookieJar::default(), |cookies| cookies.clone())
    }

    /// The document initiating a top-level navigation, for SameSite filtering.
    pub fn with_cookie_initiator(mut self, initiator: Option<Location>) -> Self {
        self.cookie_initiator = initiator;
        self
    }

    pub fn take_cookie_updates(&self) -> Vec<CookieUpdate> {
        self.cookie_updates
            .lock()
            .map_or_else(|_| Vec::new(), |mut updates| std::mem::take(&mut *updates))
    }

    pub fn load(&self, location: Location) -> Result<LoadedDocument, String> {
        self.load_kind(location, None, None, self.timeout, MAX_DOCUMENT_BYTES, None)
    }

    pub fn submit(&self, request: FormRequest) -> Result<LoadedDocument, String> {
        if !request.location.is_remote() {
            return Err("Forms can only submit to HTTP or HTTPS addresses.".into());
        }
        if request
            .body
            .as_ref()
            .is_some_and(|body| body.len() > MAX_FORM_BYTES)
        {
            return Err("Form data exceeds the 64 KiB limit.".into());
        }
        self.load_kind(
            request.location.clone(),
            None,
            Some(&request.location),
            self.timeout,
            MAX_DOCUMENT_BYTES,
            request.body,
        )
    }

    /// Fetch a stylesheet, classic script, or raster image with the document's origin policy.
    /// Web resources cannot read files; HTTPS resources cannot downgrade to HTTP,
    /// including on redirects. HTTP errors and incorrect MIME types are rejected.
    pub fn load_resource(
        &self,
        document: &Location,
        location: Location,
        kind: ResourceKind,
        timeout: Duration,
        max_bytes: usize,
    ) -> Result<LoadedResource, String> {
        let loaded = self.load_kind(
            location,
            Some(kind),
            Some(document),
            timeout.min(self.timeout),
            max_bytes.min(MAX_RESOURCE_BYTES),
            None,
        )?;
        let bytes = loaded.bytes;
        let source = if kind == ResourceKind::Image {
            String::new()
        } else {
            String::from_utf8(bytes.clone()).map_err(|e| e.to_string())?
        };
        Ok(LoadedResource {
            location: loaded.location,
            source,
            bytes,
        })
    }

    fn load_kind(
        &self,
        location: Location,
        kind: Option<ResourceKind>,
        document: Option<&Location>,
        timeout: Duration,
        max_bytes: usize,
        mut body: Option<String>,
    ) -> Result<LoadedDocument, String> {
        if let Some(document) = document {
            check_resource_target(document, &location)?;
        }
        if timeout.is_zero() {
            return Err("Resource loading deadline reached.".into());
        }
        if let Some(path) = location.file_path() {
            if !path
                .metadata()
                .map_err(|e| format!("Could not open {}: {e}", path.display()))?
                .is_file()
            {
                return Err("Choose a regular HTML file.".into());
            }
            let file =
                File::open(&path).map_err(|e| format!("Could not open {}: {e}", path.display()))?;
            if !file.metadata().map_err(|e| e.to_string())?.is_file() {
                return Err("Choose a regular HTML file.".into());
            }
            return Ok(LoadedDocument {
                location,
                bytes: match kind {
                    Some(ResourceKind::Image) => read_limit(file, max_bytes)?,
                    Some(ResourceKind::Stylesheet | ResourceKind::Script) => {
                        decode_limit(&read_limit(file, max_bytes)?, "", max_bytes)?
                    }
                    None => decode_html_limit(&read_limit(file, max_bytes)?, "", max_bytes)?,
                },
                status: None,
                plain_text: false,
            });
        }
        let initiator = if kind.is_some() {
            document
        } else {
            self.cookie_initiator.as_ref()
        };
        let mut same_site =
            initiator.is_none_or(|initiator| same_cookie_site(initiator, &location));
        let mut location = location;
        let started = Instant::now();
        let mut redirects = 0;
        let response = loop {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err("The website took too long to respond. Try again.".into());
            }
            let request = if let Some(body) = &body {
                self.client
                    .post(location.url().clone())
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(body.clone())
            } else {
                self.client.get(location.url().clone())
            };
            let request = if let Some(cookie) = self.cookies.lock().ok().and_then(|mut cookies| {
                cookies.cookie_header_for(&location, same_site, kind.is_none(), body.is_none())
            }) {
                request.header(header::COOKIE, cookie)
            } else {
                request
            };
            let response = request
                .timeout(remaining)
                .header(
                    header::ACCEPT,
                    match kind {
                        Some(ResourceKind::Stylesheet) => "text/css",
                        Some(ResourceKind::Script) => "text/javascript, application/javascript",
                        Some(ResourceKind::Image) => {
                            "image/png, image/jpeg, image/webp, image/gif, image/x-icon, image/bmp"
                        }
                        None => "text/html, application/xhtml+xml, text/plain;q=0.8",
                    },
                )
                .send()
                .map_err(request_error)?;
            for value in response.headers().get_all(header::SET_COOKIE) {
                let Ok(value) = value.to_str() else { continue };
                if kind.is_some()
                    && !same_site
                    && cookie::Cookie::parse(value)
                        .ok()
                        .and_then(|cookie| cookie.same_site())
                        != Some(cookie::SameSite::None)
                {
                    continue;
                }
                if self
                    .cookies
                    .lock()
                    .is_ok_and(|mut cookies| cookies.absorb(&location, value))
                {
                    if let Ok(mut updates) = self.cookie_updates.lock() {
                        CookieUpdate {
                            location: location.clone(),
                            set_cookie: value.to_owned(),
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
                        return Err("Too many redirects (maximum 10).".into());
                    }
                    let href = target.to_str().map_err(|_| "Invalid redirect address")?;
                    let mut target = location.resolve(href)?;
                    if body.is_some()
                        && location.url().scheme() == "https"
                        && target.url().scheme() != "https"
                    {
                        return Err("HTTPS forms cannot redirect to an insecure address.".into());
                    }
                    // Historical POST redirects become GET; 307/308 retain the body.
                    if matches!(response.status().as_u16(), 301..=303) {
                        body = None;
                    }
                    if target.0.fragment().is_none() {
                        target.0.set_fragment(location.0.fragment());
                    }
                    if let Some(document) = document {
                        check_resource_target(document, &target)?;
                    }
                    same_site &= same_cookie_site(&location, &target);
                    location = target;
                    redirects += 1;
                    continue;
                }
            }
            break response;
        };
        let status = Some(response.status().as_u16());
        if kind.is_some() && !response.status().is_success() {
            return Err(format!(
                "Resource returned HTTP {}.",
                response.status().as_u16()
            ));
        }
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or(if kind.is_some() { "" } else { "text/html" })
            .to_owned();
        let mime = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let supported = match kind {
            None => matches!(
                mime.as_str(),
                "text/html" | "application/xhtml+xml" | "text/plain"
            ),
            Some(ResourceKind::Stylesheet) => mime == "text/css",
            Some(ResourceKind::Script) => matches!(
                mime.as_str(),
                "text/javascript"
                    | "application/javascript"
                    | "text/ecmascript"
                    | "application/ecmascript"
                    | "application/x-javascript"
                    | "application/x-ecmascript"
                    | "text/x-javascript"
                    | "text/x-ecmascript"
                    | "text/javascript1.0"
                    | "text/javascript1.1"
                    | "text/javascript1.2"
                    | "text/javascript1.3"
                    | "text/javascript1.4"
                    | "text/javascript1.5"
                    | "text/jscript"
                    | "text/livescript"
            ),
            Some(ResourceKind::Image) => matches!(
                mime.as_str(),
                "image/jpeg"
                    | "image/png"
                    | "image/webp"
                    | "image/gif"
                    | "image/x-icon"
                    | "image/vnd.microsoft.icon"
                    | "image/bmp"
            ),
        };
        if !supported {
            return Err(format!(
                "Unsupported content type: {}.",
                if mime.is_empty() { "missing" } else { &mime }
            ));
        }
        if response
            .content_length()
            .is_some_and(|n| n > max_bytes as u64)
        {
            return Err(limit_error(max_bytes));
        }
        let raw = read_limit(response, max_bytes)?;
        let bytes = match kind {
            Some(ResourceKind::Image) => raw,
            Some(ResourceKind::Stylesheet | ResourceKind::Script) => {
                decode_limit(&raw, &content_type, max_bytes)?
            }
            None if mime == "text/html" || mime.is_empty() => {
                decode_html_limit(&raw, &content_type, max_bytes)?
            }
            None => decode_limit(&raw, &content_type, max_bytes)?,
        };
        Ok(LoadedDocument {
            location,
            bytes,
            status,
            plain_text: mime == "text/plain",
        })
    }
}

fn request_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        return "The website took too long to respond. Try again.".into();
    }
    let mut message = "Could not load the website".to_owned();
    let mut cause: Option<&dyn std::error::Error> = Some(&error);
    for _ in 0..8 {
        let Some(error) = cause else {
            break;
        };
        message.push_str(": ");
        message.push_str(&error.to_string());
        cause = error.source();
    }
    message
}

fn check_resource_target(document: &Location, target: &Location) -> Result<(), String> {
    if document.is_remote() && !target.is_remote() {
        return Err("Web resources cannot open local files.".into());
    }
    if document.url().scheme() == "https" && target.url().scheme() != "https" {
        return Err("HTTPS pages cannot load insecure resources.".into());
    }
    Ok(())
}
