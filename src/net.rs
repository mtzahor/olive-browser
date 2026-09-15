//! Explicit, bounded document loading and URL resolution. Parsing never fetches
//! subresources or executes scripts. Local files and HTTP(S) are the only schemes.

use crate::{Document, ParseOptions, ParseOutput, parse_utf8};
use encoding_rs::{Encoding, UTF_8};
use reqwest::{blocking::Client, header, redirect};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
pub use url::Url;

pub mod download;

pub const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
pub const MAX_URL_BYTES: usize = 8192;
pub const MAX_RESOURCE_BYTES: usize = 8 * 1024 * 1024;
const MAX_COOKIES: usize = 256;
const MAX_COOKIE_BYTES: usize = 4 * 1024;
const MAX_COOKIE_HEADER_BYTES: usize = 64 * 1024;
const MAX_COOKIE_JAR_BYTES: usize = 64 * 1024;
const MAX_COOKIE_UPDATES: usize = 256;

/// A bounded, in-memory HTTP cookie jar with expiry. The browser never writes
/// cookies to disk, including cookies with `Max-Age` or `Expires` attributes.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub struct CookieJar {
    cookies: Vec<StoredCookie>,
    next_creation: u64,
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
struct StoredCookie {
    name: String,
    value: String,
    domain: String,
    path: String,
    host_only: bool,
    secure: bool,
    http_only: bool,
    same_site: SameSite,
    expires_at: Option<i64>,
    creation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
enum SameSite {
    Strict,
    Lax,
    None,
}

/// A cookie mutation produced by a document or one of its resources. The UI
/// applies these mutations to its shared jar after a worker completes.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
pub struct CookieUpdate {
    pub location: Location,
    pub set_cookie: String,
    pub from_script: bool,
    received_at: i64,
}

impl CookieUpdate {
    pub fn from_script(location: Location, set_cookie: String) -> Self {
        Self {
            location,
            set_cookie,
            from_script: true,
            received_at: now_seconds(),
        }
    }

    /// Bound mutations crossing a worker boundary; repeated writes to a cookie
    /// replace its pending update while retaining the final expiry timestamp.
    pub fn record(self, updates: &mut Vec<Self>) {
        let identity = |update: &Self| {
            let parsed = cookie::Cookie::parse(update.set_cookie.as_str()).ok()?;
            Some((
                parsed.name().to_owned(),
                parsed
                    .domain()
                    .unwrap_or_else(|| update.location.url().host_str().unwrap_or_default())
                    .to_ascii_lowercase(),
                parsed
                    .path()
                    .filter(|path| path.starts_with('/'))
                    .map(str::to_owned)
                    .unwrap_or_else(|| default_cookie_path(update.location.url())),
            ))
        };
        let key = identity(&self);
        if let Some(index) = updates.iter().position(|update| identity(update) == key) {
            updates.remove(index);
        }
        if updates.len() == MAX_COOKIE_UPDATES {
            updates.remove(0);
        }
        updates.push(self);
    }
}

impl CookieJar {
    pub fn cookie_header(&mut self, location: &Location) -> Option<String> {
        self.cookie_header_for(location, true, true, true)
    }

    fn cookie_header_for(
        &mut self,
        location: &Location,
        same_site: bool,
        top_level: bool,
        safe: bool,
    ) -> Option<String> {
        self.remove_expired();
        let url = location.url();
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        let host = url.host_str()?.to_ascii_lowercase();
        let path = request_path(url);
        let https = url.scheme() == "https";
        let mut cookies: Vec<_> = self
            .cookies
            .iter()
            .filter(|cookie| {
                (!cookie.host_only && domain_matches(&host, &cookie.domain)
                    || cookie.host_only && host == cookie.domain)
                    && path_matches(&path, &cookie.path)
                    && (!cookie.secure || https)
                    && match cookie.same_site {
                        SameSite::Strict => same_site,
                        SameSite::Lax => same_site || (top_level && safe),
                        SameSite::None => true,
                    }
            })
            .collect();
        cookies.sort_by_key(|cookie| (std::cmp::Reverse(cookie.path.len()), cookie.creation));
        if cookies.is_empty() {
            return None;
        }
        let mut result = String::new();
        for cookie in cookies {
            let piece = format!("{}={}", cookie.name, cookie.value);
            let separator = usize::from(!result.is_empty()) * 2;
            if result
                .len()
                .saturating_add(separator)
                .saturating_add(piece.len())
                > MAX_COOKIE_HEADER_BYTES
            {
                break;
            }
            if !result.is_empty() {
                result.push_str("; ");
            }
            result.push_str(&piece);
        }
        (!result.is_empty()).then_some(result)
    }

    pub fn document_cookie(&mut self, location: &Location) -> String {
        self.remove_expired();
        let url = location.url();
        if !matches!(url.scheme(), "http" | "https") {
            return String::new();
        }
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let path = request_path(url);
        let https = url.scheme() == "https";
        let mut cookies: Vec<_> = self
            .cookies
            .iter()
            .filter(|cookie| {
                !cookie.http_only
                    && (!cookie.host_only && domain_matches(&host, &cookie.domain)
                        || cookie.host_only && host == cookie.domain)
                    && path_matches(&path, &cookie.path)
                    && (!cookie.secure || https)
            })
            .collect();
        cookies.sort_by_key(|cookie| (std::cmp::Reverse(cookie.path.len()), cookie.creation));
        cookies
            .into_iter()
            .map(|cookie| format!("{}={}", cookie.name, cookie.value))
            .collect::<Vec<_>>()
            .join("; ")
    }

    pub fn set_document_cookie(&mut self, location: &Location, value: &str) -> bool {
        self.store(location, value, false)
    }

    fn absorb(&mut self, location: &Location, value: &str) -> bool {
        self.store(location, value, true)
    }

    pub fn apply_updates(&mut self, updates: &[CookieUpdate]) {
        for update in updates.iter().take(MAX_COOKIE_UPDATES) {
            self.store_at(
                &update.location,
                &update.set_cookie,
                !update.from_script,
                update.received_at,
            );
        }
    }

    fn store(&mut self, location: &Location, header: &str, from_response: bool) -> bool {
        self.store_at(location, header, from_response, now_seconds())
    }

    fn store_at(
        &mut self,
        location: &Location,
        header: &str,
        from_response: bool,
        received_at: i64,
    ) -> bool {
        self.remove_expired();
        let url = location.url();
        if url.scheme() != "http" && url.scheme() != "https" {
            return false;
        }
        if header.len() > MAX_COOKIE_BYTES || header.bytes().any(|byte| byte.is_ascii_control()) {
            return false;
        }
        let mut parts = header.split(';');
        let Some(pair) = parts.next().map(str::trim) else {
            return false;
        };
        let Some((name, raw_value)) = pair.split_once('=') else {
            return false;
        };
        let name = name.trim();
        let value = raw_value.trim();
        let value = if let Some(value) = value.strip_prefix('"') {
            let Some(value) = value.strip_suffix('"') else {
                return false;
            };
            value
        } else {
            value
        };
        if !valid_cookie_name(name)
            || value.contains(';')
            || value
                .bytes()
                .any(|byte| !byte.is_ascii() || byte.is_ascii_control())
        {
            return false;
        }
        let Some(host) = url.host_str().map(|host| host.to_ascii_lowercase()) else {
            return false;
        };
        let default_path = default_cookie_path(url);
        let mut domain = host.clone();
        let mut host_only = true;
        let mut path = default_path;
        let mut secure = false;
        let mut http_only = false;
        let mut max_age = None;
        let mut same_site = SameSite::Lax;
        let parsed = cookie::Cookie::parse(header).ok();
        let expires = parsed
            .as_ref()
            .and_then(|cookie| cookie.expires_datetime())
            .map(|date| date.unix_timestamp());
        for attribute in parts {
            let attribute = attribute.trim();
            let (key, value) = attribute
                .split_once('=')
                .map_or((attribute, None), |(key, value)| {
                    (key.trim(), Some(value.trim()))
                });
            if key.eq_ignore_ascii_case("domain") {
                let Some(value) = value else { continue };
                let candidate = value.trim_start_matches('.').to_ascii_lowercase();
                if candidate.is_empty()
                    || !domain_matches(&host, &candidate)
                    || (host.parse::<std::net::IpAddr>().is_ok() && candidate != host)
                {
                    return false;
                }
                if psl::suffix_str(&candidate) == Some(candidate.as_str()) && candidate != host {
                    return false;
                }
                domain = candidate;
                host_only = psl::suffix_str(&domain) == Some(domain.as_str());
            } else if key.eq_ignore_ascii_case("path") {
                if let Some(value) = value.filter(|value| value.starts_with('/')) {
                    path = value.to_owned();
                }
            } else if key.eq_ignore_ascii_case("secure") {
                secure = true;
            } else if key.eq_ignore_ascii_case("httponly") {
                if !from_response {
                    return false;
                }
                http_only = true;
            } else if key.eq_ignore_ascii_case("max-age") {
                if let Some(value) = value {
                    let digits = value.strip_prefix('-').unwrap_or(value);
                    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                        max_age = value.parse::<i64>().ok().or_else(|| {
                            Some(if value.starts_with('-') {
                                i64::MIN
                            } else {
                                i64::MAX
                            })
                        });
                    }
                }
            } else if key.eq_ignore_ascii_case("samesite") {
                same_site = match value.map(str::to_ascii_lowercase).as_deref() {
                    Some("strict") => SameSite::Strict,
                    Some("none") => SameSite::None,
                    _ => SameSite::Lax,
                };
            }
        }
        if secure && url.scheme() != "https" {
            return false;
        }
        if same_site == SameSite::None && !secure
            || name.starts_with("__Secure-") && !secure
            || name.starts_with("__Host-")
                && (!secure
                    || !host_only
                    || path != "/"
                    || parsed.as_ref().is_none_or(|cookie| {
                        cookie.path() != Some("/") || cookie.domain().is_some()
                    }))
        {
            return false;
        }
        let key = |cookie: &StoredCookie| {
            cookie.name == name && cookie.domain == domain && cookie.path == path
        };
        if self.cookies.iter().any(|cookie| {
            (!from_response && key(cookie) && cookie.http_only)
                || (url.scheme() != "https"
                    && cookie.secure
                    && cookie.name == name
                    && (domain_matches(&domain, &cookie.domain)
                        || domain_matches(&cookie.domain, &domain))
                    && path_matches(&path, &cookie.path))
        }) {
            return false;
        }
        let creation = self
            .cookies
            .iter()
            .find(|cookie| key(cookie))
            .map(|cookie| cookie.creation);
        self.cookies.retain(|cookie| !key(cookie));
        let expires_at = max_age
            .map(|age| received_at.saturating_add(age.min(31_536_000_i64 * 10)))
            .or(expires);
        if max_age.is_some_and(|age| age <= 0)
            || expires_at.is_some_and(|date| date <= now_seconds())
        {
            return true;
        }
        self.next_creation = self.next_creation.wrapping_add(1);
        self.cookies.push(StoredCookie {
            name: name.to_owned(),
            value: value.to_owned(),
            domain,
            path,
            host_only,
            secure,
            http_only,
            same_site,
            expires_at,
            creation: creation.unwrap_or(self.next_creation),
        });
        if self.cookies.len() > MAX_COOKIES {
            self.cookies.sort_by_key(|cookie| cookie.creation);
            self.cookies.remove(0);
        }
        while self.cookies.iter().map(stored_cookie_bytes).sum::<usize>() > MAX_COOKIE_JAR_BYTES {
            let Some(oldest) = self
                .cookies
                .iter()
                .enumerate()
                .min_by_key(|(_, cookie)| cookie.creation)
                .map(|(index, _)| index)
            else {
                break;
            };
            self.cookies.remove(oldest);
        }
        true
    }

    fn remove_expired(&mut self) {
        let now = now_seconds();
        self.cookies
            .retain(|cookie| cookie.expires_at.is_none_or(|expires| expires > now));
    }
}

fn stored_cookie_bytes(cookie: &StoredCookie) -> usize {
    cookie
        .name
        .len()
        .saturating_add(cookie.value.len())
        .saturating_add(cookie.domain.len())
        .saturating_add(cookie.path.len())
        .saturating_add(64)
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs().min(i64::MAX as u64) as i64)
}

fn valid_cookie_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn same_cookie_site(left: &Location, right: &Location) -> bool {
    let (left, right) = (left.url(), right.url());
    match (left.host(), right.host()) {
        (Some(url::Host::Domain(a)), Some(url::Host::Domain(b))) => {
            left.scheme() == right.scheme()
                && psl::domain_str(a).unwrap_or(a) == psl::domain_str(b).unwrap_or(b)
        }
        (Some(a), Some(b)) => left.scheme() == right.scheme() && a == b,
        _ => false,
    }
}

fn request_path(url: &Url) -> String {
    let path = url.path();
    if path.is_empty() {
        "/".into()
    } else {
        path.to_owned()
    }
}

fn default_cookie_path(url: &Url) -> String {
    let path = request_path(url);
    if path == "/" {
        return path;
    }
    path.rfind('/').map_or_else(
        || "/".into(),
        |index| {
            if index == 0 {
                "/".into()
            } else {
                path[..index].to_owned()
            }
        },
    )
}

fn path_matches(request: &str, cookie: &str) -> bool {
    request == cookie
        || request
            .strip_prefix(cookie)
            .is_some_and(|rest| cookie.ends_with('/') || rest.starts_with('/'))
}

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

/// A validated address. Credentials and non-local file authorities are rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location(Url);

// Deserialize through the same validation used by normal navigation.
#[cfg(feature = "gui")]
impl serde::Serialize for Location {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
#[cfg(feature = "gui")]
impl<'de> serde::Deserialize<'de> for Location {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = <String as serde::Deserialize>::deserialize(deserializer)?;
        check_input(&value).map_err(serde::de::Error::custom)?;
        let url = Url::parse(&value).map_err(serde::de::Error::custom)?;
        Self::from_url(url).map_err(serde::de::Error::custom)
    }
}

impl Location {
    /// Address-bar input: explicit URLs, file paths, or a host (HTTPS by default).
    /// Loopback addresses without a scheme use HTTP for local development.
    pub fn from_input(input: &str) -> Result<Self, String> {
        let input = input.trim();
        if input.is_empty() {
            return Err("Enter a website address or an HTML file path.".into());
        }
        check_input(input)?;
        let path = Path::new(input);
        if (path.is_absolute() && !input.starts_with("//"))
            || input.starts_with("./")
            || input.starts_with("../")
            || path.exists()
        {
            return Self::from_path(path);
        }
        let authority = input.split(['/', '?', '#']).next().unwrap_or(input);
        let host_port = authority.rsplit_once(':').is_some_and(|(host, port)| {
            (host.contains('.') || host.eq_ignore_ascii_case("localhost") || host.starts_with('['))
                && !port.is_empty()
                && port.bytes().all(|c| c.is_ascii_digit())
        });
        if input.contains(':') && !host_port && !input.starts_with('[') {
            return Self::from_url(Url::parse(input).map_err(|e| format!("Invalid URL: {e}"))?);
        }
        let address = input.strip_prefix("//").unwrap_or(input);
        let mut url = Url::parse(&format!("https://{address}"))
            .map_err(|e| format!("Invalid address: {e}"))?;
        let loopback = match url.host() {
            Some(url::Host::Domain(host)) => host == "localhost" || host.ends_with(".localhost"),
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        };
        if loopback {
            url.set_scheme("http").map_err(|_| "Invalid address")?;
        }
        Self::from_url(url)
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(path)
        };
        Self::from_url(Url::from_file_path(absolute).map_err(|_| "Invalid local file path")?)
    }

    pub fn from_url(url: Url) -> Result<Self, String> {
        check_input(url.as_str())?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err("URLs containing a username or password are not supported.".into());
        }
        match url.scheme() {
            "http" | "https" if url.has_host() => {}
            // `url` reports the empty authority as either `None` or `Some("")`
            // depending on platform and URL crate version. Both represent a
            // local file URL; a non-empty host remains a remote file authority.
            "file"
                if url.host_str().is_none_or(|host| host.is_empty())
                    && url.to_file_path().is_ok() => {}
            "file" => return Err("Only local file URLs are supported.".into()),
            _ => return Err("Supported address schemes are http://, https:// and file://.".into()),
        }
        Ok(Self(url))
    }

    pub fn url(&self) -> &Url {
        &self.0
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
    pub fn is_remote(&self) -> bool {
        self.0.scheme() != "file"
    }
    pub fn file_path(&self) -> Option<PathBuf> {
        (self.0.scheme() == "file")
            .then(|| self.0.to_file_path().ok())
            .flatten()
    }

    /// Resolve a page link, preventing a web page from navigating into local files.
    pub fn resolve(&self, href: &str) -> Result<Self, String> {
        check_input(href)?;
        let target = Self::from_url(
            self.0
                .join(href)
                .map_err(|e| format!("Invalid link: {e}"))?,
        )?;
        if self.is_remote() && !target.is_remote() {
            return Err("Web pages cannot open local files.".into());
        }
        Ok(target)
    }

    pub fn same_document(&self, other: &Self) -> bool {
        let mut a = self.0.clone();
        let mut b = other.0.clone();
        a.set_fragment(None);
        b.set_fragment(None);
        a == b
    }

    pub fn fragment(&self) -> Option<String> {
        self.0.fragment().map(|value| {
            percent_encoding::percent_decode_str(value)
                .decode_utf8_lossy()
                .into_owned()
        })
    }

    /// Use the first HTML base href, falling back to the final response URL.
    pub fn document_base(&self, document: &Document) -> Self {
        document
            .descendants(document.root())
            .find_map(|id| {
                let element = document.node(id)?.as_element()?;
                if element.name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                    && element.name.local.as_ref() == "base"
                {
                    element.attribute("href")
                } else {
                    None
                }
            })
            .and_then(|href| self.resolve(href).ok())
            .unwrap_or_else(|| self.clone())
    }
}

fn check_input(input: &str) -> Result<(), String> {
    if input.len() > MAX_URL_BYTES {
        return Err("This address is too long (maximum 8192 bytes).".into());
    }
    if input.chars().any(char::is_control) {
        return Err("Addresses cannot contain control characters.".into());
    }
    Ok(())
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
#[cfg_attr(feature = "gui", derive(serde::Serialize, serde::Deserialize))]
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
                        Some(ResourceKind::Image) => "image/png, image/jpeg, image/webp",
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
            Some(ResourceKind::Image) => {
                matches!(mime.as_str(), "image/jpeg" | "image/png" | "image/webp")
            }
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

fn limit_error(max_bytes: usize) -> String {
    format!("Resource exceeds the {max_bytes}-byte limit.")
}

#[cfg(test)]
fn read_bounded(reader: impl Read) -> Result<Vec<u8>, String> {
    read_limit(reader, MAX_DOCUMENT_BYTES)
}
fn read_limit(reader: impl Read, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Could not read document: {e}"))?;
    if bytes.len() > max_bytes {
        return Err(limit_error(max_bytes));
    }
    Ok(bytes)
}

#[cfg(test)]
fn decode(bytes: &[u8], content_type: &str) -> Result<Vec<u8>, String> {
    decode_limit(bytes, content_type, MAX_DOCUMENT_BYTES)
}
fn decode_limit(bytes: &[u8], content_type: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    decode_with_encoding(bytes, declared_encoding(content_type), max_bytes)
}

fn decode_html_limit(
    bytes: &[u8],
    content_type: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let declared = declared_encoding(content_type);
    let bom = Encoding::for_bom(bytes);
    let encoding = bom.map(|(encoding, _)| encoding).or(declared).or_else(|| {
        sniff_html_encoding(bytes).and_then(|label| {
            let encoding = Encoding::for_label(label.as_bytes())?;
            Some(
                if encoding == encoding_rs::UTF_16LE || encoding == encoding_rs::UTF_16BE {
                    UTF_8
                } else if encoding == encoding_rs::X_USER_DEFINED {
                    encoding_rs::WINDOWS_1252
                } else {
                    encoding
                },
            )
        })
    });
    decode_with_encoding(bytes, encoding, max_bytes)
}

fn declared_encoding(content_type: &str) -> Option<&'static Encoding> {
    content_type
        .split(';')
        .skip(1)
        .find_map(|part| {
            let (name, value) = part.trim().split_once('=')?;
            name.trim()
                .eq_ignore_ascii_case("charset")
                .then(|| value.trim().trim_matches(['\'', '"']))
        })
        .and_then(|label| Encoding::for_label(label.as_bytes()))
}

fn decode_with_encoding(
    bytes: &[u8],
    declared: Option<&'static Encoding>,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let (encoding, skip) = Encoding::for_bom(bytes).unwrap_or((declared.unwrap_or(UTF_8), 0));
    let (decoded, _) = encoding.decode_without_bom_handling(&bytes[skip..]);
    if decoded.len() > max_bytes {
        return Err(limit_error(max_bytes));
    }
    Ok(decoded.into_owned().into_bytes())
}

/// Prescan the first 1024 bytes for the HTML encoding declarations accepted by
/// browsers. It deliberately only recognizes `<meta>` attributes and never
/// treats visible text as a declaration.
fn sniff_html_encoding(bytes: &[u8]) -> Option<String> {
    let end = bytes.len().min(1024);
    let bytes = &bytes[..end];
    let mut cursor = 0;
    while let Some(relative) = bytes[cursor..].iter().position(|byte| *byte == b'<') {
        let start = cursor + relative;
        cursor = start + 1;
        if bytes
            .get(cursor..)
            .is_some_and(|rest| rest.starts_with(b"!--"))
        {
            cursor = bytes[cursor + 3..]
                .windows(3)
                .position(|window| window == b"-->")
                .map_or(end, |offset| cursor + 3 + offset + 3);
            continue;
        }
        let mut quote = None;
        let Some(tag_end) = bytes[start..].iter().position(|&byte| {
            if let Some(delimiter) = quote {
                if byte == delimiter {
                    quote = None;
                }
            } else if matches!(byte, b'\'' | b'"') {
                quote = Some(byte);
            } else if byte == b'>' {
                return true;
            }
            false
        }) else {
            break;
        };
        let tag = &bytes[start + 1..start + tag_end];
        let mut name_end = 0;
        while name_end < tag.len() && tag[name_end].is_ascii_alphabetic() {
            name_end += 1;
        }
        if !tag[..name_end].eq_ignore_ascii_case(b"meta")
            || tag
                .get(name_end)
                .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'/')
        {
            cursor = start + tag_end + 1;
            continue;
        }
        let attributes = meta_attributes(&tag[name_end..]);
        if let Some(value) = attributes
            .iter()
            .find(|(name, _)| name == "charset")
            .map(|(_, value)| value.clone())
            .or_else(|| {
                if !attributes.iter().any(|(name, value)| {
                    name == "http-equiv" && value.eq_ignore_ascii_case("content-type")
                }) {
                    return None;
                }
                attributes
                    .iter()
                    .find(|(name, _)| name == "content")
                    .and_then(|(_, value)| charset_parameter(value))
            })
        {
            if Encoding::for_label(value.trim().as_bytes()).is_some() {
                return Some(value.trim().to_owned());
            }
        }
        cursor = start + tag_end + 1;
    }
    None
}

fn meta_attributes(bytes: &[u8]) -> Vec<(String, String)> {
    let mut attributes = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        while cursor < bytes.len() && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b'/')
        {
            cursor += 1;
        }
        let start = cursor;
        while cursor < bytes.len()
            && !bytes[cursor].is_ascii_whitespace()
            && !matches!(bytes[cursor], b'=' | b'/')
        {
            cursor += 1;
        }
        if start == cursor {
            cursor += 1;
            continue;
        }
        let name = String::from_utf8_lossy(&bytes[start..cursor]).to_ascii_lowercase();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let mut value = String::new();
        if bytes.get(cursor) == Some(&b'=') {
            cursor += 1;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if let Some(quote @ (b'"' | b'\'')) = bytes.get(cursor).copied() {
                cursor += 1;
                let value_start = cursor;
                while cursor < bytes.len() && bytes[cursor] != quote {
                    cursor += 1;
                }
                value = String::from_utf8_lossy(&bytes[value_start..cursor]).into_owned();
                cursor += usize::from(cursor < bytes.len());
            } else {
                let value_start = cursor;
                while cursor < bytes.len() && !bytes[cursor].is_ascii_whitespace() {
                    cursor += 1;
                }
                value = String::from_utf8_lossy(&bytes[value_start..cursor]).into_owned();
            }
        }
        if !attributes.iter().any(|(existing, _)| existing == &name) {
            attributes.push((name, value));
        }
    }
    attributes
}

fn charset_parameter(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let start = lower.find("charset")? + "charset".len();
    let rest = &value[start..];
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest
        .strip_prefix('\'')
        .or_else(|| rest.strip_prefix('"'))
        .unwrap_or(rest);
    let end = rest
        .find(|character: char| {
            character.is_ascii_whitespace()
                || character == ';'
                || character == '\''
                || character == '"'
        })
        .unwrap_or(rest.len());
    (end > 0).then(|| rest[..end].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_and_relative_links() {
        let site = Location::from_input(" example.com/docs/index.html ").unwrap();
        assert_eq!(site.as_str(), "https://example.com/docs/index.html");
        assert_eq!(
            site.resolve("../about?q=olive tree#one").unwrap().as_str(),
            "https://example.com/about?q=olive%20tree#one"
        );
        assert_eq!(
            site.resolve("//other.example/path").unwrap().as_str(),
            "https://other.example/path"
        );
        for address in ["localhost:8000/test", "127.0.0.1:8080", "[::1]:3000"] {
            assert!(Location::from_input(address).unwrap().file_path().is_none());
            assert_eq!(
                Location::from_input(address).unwrap().url().scheme(),
                "http"
            );
        }
        assert_eq!(
            Location::from_input("example.com:8080").unwrap().as_str(),
            "https://example.com:8080/"
        );
        assert_eq!(
            Location::from_input("//example.com/path").unwrap().as_str(),
            "https://example.com/path"
        );
        let path = std::env::current_dir().unwrap().join("a b.html");
        let local = Location::from_path(&path).unwrap();
        assert_eq!(local.file_path().unwrap(), path);
        assert_eq!(
            local.resolve("next.html").unwrap().file_path().unwrap(),
            path.with_file_name("next.html")
        );
        assert_eq!(
            site.resolve("#caf%C3%A9").unwrap().fragment().unwrap(),
            "café"
        );
    }

    #[test]
    fn reject_unsafe_or_invalid_addresses() {
        for address in [
            "",
            "javascript:alert(1)",
            "data:text/html,hi",
            "ftp://example.com",
            "https://u:p@example.com",
            "file://server/secret",
            "https://example.com/\nnext",
            "http://",
        ] {
            assert!(Location::from_input(address).is_err(), "{address}");
        }
        let site = Location::from_input("https://example.com").unwrap();
        assert!(site.resolve("file:///secret.html").is_err());
        assert!(site.resolve("javascript:alert(1)").is_err());
        assert!(site.resolve(&"x".repeat(MAX_URL_BYTES + 1)).is_err());

        // Windows file URLs require a drive-qualified path; Unix file URLs do
        // not. Both forms have a local (empty) authority.
        let local_file = if cfg!(windows) {
            "file:///C:/secret.js"
        } else {
            "file:///secret.js"
        };
        assert!(Location::from_input(local_file).is_ok());
    }

    #[test]
    fn base_resolution_preserves_web_file_boundary() {
        let site = Location::from_input("https://example.com/dir/page").unwrap();
        let doc = crate::parse("<base href='../assets/'><base href='https://ignored.example'>")
            .unwrap()
            .document;
        assert_eq!(
            site.document_base(&doc).resolve("next").unwrap().as_str(),
            "https://example.com/assets/next"
        );
        let doc = crate::parse("<base href='file:///secret/'>")
            .unwrap()
            .document;
        assert_eq!(site.document_base(&doc), site);
    }

    #[test]
    fn bounded_decoding_handles_header_and_bom() {
        assert_eq!(
            decode(b"caf\xe9", "text/html; charset=windows-1252").unwrap(),
            "café".as_bytes()
        );
        assert_eq!(
            decode(
                b"\xef\xbb\xbfcaf\xc3\xa9",
                "text/html; charset=windows-1252"
            )
            .unwrap(),
            "café".as_bytes()
        );
        assert!(read_bounded(&vec![b'x'; MAX_DOCUMENT_BYTES + 1][..]).is_err());
        assert!(
            decode(
                &vec![0xe9; MAX_DOCUMENT_BYTES],
                "text/html;charset=windows-1252"
            )
            .is_err()
        );
    }

    #[test]
    fn html_meta_charset_is_sniffed_before_utf8_fallback() {
        let html = b"<html><head><meta charset='windows-1252'></head><body>caf\xe9</body>";
        let decoded = decode_html_limit(html, "text/html", MAX_DOCUMENT_BYTES).unwrap();
        assert_eq!(
            String::from_utf8(decoded).unwrap(),
            "<html><head><meta charset='windows-1252'></head><body>café</body>"
        );
        assert_eq!(
            sniff_html_encoding(
                b"<meta http-equiv=Content-Type content='text/html; charset=iso-8859-1'>"
            )
            .as_deref(),
            Some("iso-8859-1")
        );
        assert_eq!(sniff_html_encoding(b"<p>charset=windows-1252</p>"), None);
    }

    #[test]
    fn cookies_obey_domain_path_secure_and_deletion_rules() {
        let site = Location::from_input("http://example.com/docs/page").unwrap();
        let other = Location::from_input("http://other.example.com/docs/next").unwrap();
        let secure = Location::from_input("https://example.com/docs/next").unwrap();
        let mut jar = CookieJar::default();
        assert!(jar.absorb(&site, "session=one; Path=/docs; HttpOnly"));
        assert!(jar.absorb(&site, "wide=two; Domain=example.com; Max-Age=60"));
        assert!(!jar.absorb(&site, "bad=x; Domain=attacker.example"));
        assert_eq!(
            jar.cookie_header(&site).as_deref(),
            Some("session=one; wide=two")
        );
        assert_eq!(jar.cookie_header(&other).as_deref(), Some("wide=two"));
        assert_eq!(jar.document_cookie(&site), "wide=two");
        assert!(!jar.set_document_cookie(&site, "http-only=x; HttpOnly"));
        assert!(!jar.absorb(&site, "secure=x; Secure"));
        assert!(jar.absorb(&secure, "secure=x; Secure"));
        assert_eq!(
            jar.cookie_header(&site).as_deref(),
            Some("session=one; wide=two")
        );
        assert!(jar.cookie_header(&secure).unwrap().contains("secure=x"));
        assert!(jar.absorb(&site, "session=gone; Path=/docs; Max-Age=0"));
        assert_eq!(jar.document_cookie(&site), "wide=two");
    }

    #[test]
    fn cookies_protect_http_only_secure_prefixes_public_suffixes_and_expiry() {
        let https = Location::from_input("https://www.example.co.uk/docs/page").unwrap();
        let http = Location::from_input("http://www.example.co.uk/docs/page").unwrap();
        let mut jar = CookieJar::default();
        assert!(jar.absorb(&https, "session=secret; HttpOnly; Secure; Path=/"));
        assert!(!jar.set_document_cookie(&https, "session=replace; Path=/"));
        assert!(!jar.set_document_cookie(&https, "session=; Path=/; Max-Age=0"));
        assert!(!jar.absorb(&http, "session=insecure; Path=/"));
        assert!(!jar.absorb(&https, "bad=1; Domain=co.uk"));
        assert!(!jar.absorb(&https, "__Secure-invalid=1"));
        assert!(!jar.absorb(
            &https,
            "__Host-invalid=1; Secure; Domain=example.co.uk; Path=/"
        ));
        assert!(!jar.absorb(&https, "__Host-invalid=1; Secure"));
        assert!(jar.absorb(&https, "__Host-valid=1; Secure; Path=/"));
        assert!(jar.absorb(&https, "expired=1; Expires=Wed, 09 Jun 2021 10:18:14 GMT"));
        assert!(!jar.cookie_header(&https).unwrap().contains("expired="));
        assert!(jar.absorb(
            &https,
            "alive=1; Max-Age=60; Expires=Wed, 09 Jun 2021 10:18:14 GMT"
        ));
        assert!(jar.document_cookie(&https).contains("alive=1"));
        assert!(jar.absorb(&https, "alive=; Expires=Wed, 09 Jun 2021 10:18:14 GMT"));
        assert!(!jar.document_cookie(&https).contains("alive="));
        let update = CookieUpdate {
            location: https.clone(),
            set_cookie: "delayed=1; Max-Age=1".into(),
            from_script: false,
            received_at: now_seconds() - 10,
        };
        jar.apply_updates(&[update]);
        assert!(!jar.document_cookie(&https).contains("delayed="));
        jar.apply_updates(&[CookieUpdate::from_script(
            https.clone(),
            "session=replace; Path=/".into(),
        )]);
        assert!(
            jar.cookie_header(&https)
                .unwrap()
                .contains("session=secret")
        );
    }

    #[test]
    fn same_site_cookie_filtering_distinguishes_navigation_method_and_subresources() {
        let site = Location::from_input("https://www.example.com/").unwrap();
        let sibling = Location::from_input("https://cdn.example.com/").unwrap();
        let other = Location::from_input("https://example.net/").unwrap();
        let downgrade = Location::from_input("http://www.example.com/").unwrap();
        assert!(same_cookie_site(&site, &sibling));
        assert!(!same_cookie_site(&site, &other));
        assert!(!same_cookie_site(&site, &downgrade));
        let mut jar = CookieJar::default();
        assert!(jar.absorb(&site, "strict=1; SameSite=Strict"));
        assert!(jar.absorb(&site, "lax=1"));
        assert!(jar.absorb(&site, "none=1; SameSite=None; Secure"));
        assert!(!jar.absorb(&site, "bad=1; SameSite=None"));
        assert_eq!(
            jar.cookie_header_for(&site, false, false, true).as_deref(),
            Some("none=1")
        );
        assert_eq!(
            jar.cookie_header_for(&site, false, true, false).as_deref(),
            Some("none=1")
        );
        assert_eq!(
            jar.cookie_header_for(&site, false, true, true).as_deref(),
            Some("lax=1; none=1")
        );
        assert_eq!(
            jar.cookie_header(&site).as_deref(),
            Some("strict=1; lax=1; none=1")
        );
    }

    #[test]
    fn meta_charset_obeys_precedence_pragma_quotes_and_prescan_limit() {
        assert_eq!(sniff_html_encoding(b"<!-- <meta charset=windows-1252> --><meta content='text/html; charset=windows-1252'>"), None);
        assert_eq!(
            sniff_html_encoding(
                b"<meta2 charset=windows-1252><p data-x='<meta charset=windows-1252>'>"
            ),
            None
        );
        assert_eq!(
            sniff_html_encoding(b"<META data-x='>' CHARSET=windows-1252 charset=utf-8>").as_deref(),
            Some("windows-1252")
        );
        let mut html = vec![b' '; 1024];
        html.extend_from_slice(b"<meta charset=windows-1252>");
        assert_eq!(sniff_html_encoding(&html), None);
        let html = b"<meta charset=windows-1252>caf\xc3\xa9";
        assert_eq!(
            decode_html_limit(html, "text/html; charset=utf-8", MAX_DOCUMENT_BYTES).unwrap(),
            html
        );
        let mut bom = b"\xef\xbb\xbf".to_vec();
        bom.extend_from_slice(html);
        assert_eq!(
            decode_html_limit(&bom, "text/html; charset=windows-1252", MAX_DOCUMENT_BYTES).unwrap(),
            html
        );
        let html = b"<meta charset=utf-16>caf\xc3\xa9";
        assert_eq!(
            decode_html_limit(html, "text/html", MAX_DOCUMENT_BYTES).unwrap(),
            html
        );
        let html = b"<meta charset=x-user-defined>\x80";
        assert!(
            String::from_utf8(decode_html_limit(html, "text/html", MAX_DOCUMENT_BYTES).unwrap())
                .unwrap()
                .ends_with('€')
        );
        assert!(
            decode_html_limit(b"<meta charset=windows-1252>\xe9\xe9", "text/html", 29).is_err()
        );
    }
}
