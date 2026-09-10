//! Explicit, bounded document loading and URL resolution. Parsing never fetches
//! subresources or executes scripts. Local files and HTTP(S) are the only schemes.

use crate::{Document, ParseOptions, ParseOutput, parse_utf8};
use encoding_rs::{Encoding, UTF_8};
use reqwest::{blocking::Client, header, redirect};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub use url::Url;

pub const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
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
            .map(|client| Self { client, timeout })
            .map_err(|e| format!("Could not initialize HTTP: {e}"))
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
                bytes: if kind.is_some_and(|kind| kind != ResourceKind::Image) {
                    decode_limit(&read_limit(file, max_bytes)?, "", max_bytes)?
                } else {
                    read_limit(file, max_bytes)?
                },
                status: None,
                plain_text: false,
            });
        }
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
            let response = request
                .timeout(remaining)
                .header(
                    header::ACCEPT,
                    match kind {
                        Some(ResourceKind::Stylesheet) => "text/css",
                        Some(ResourceKind::Script) => "text/javascript, application/javascript",
                        Some(ResourceKind::Image) => "image/png, image/jpeg",
                        None => "text/html, application/xhtml+xml, text/plain;q=0.8",
                    },
                )
                .send()
                .map_err(request_error)?;
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
            Some(ResourceKind::Image) => matches!(mime.as_str(), "image/jpeg" | "image/png"),
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
        let bytes = if kind == Some(ResourceKind::Image) {
            read_limit(response, max_bytes)?
        } else {
            decode_limit(&read_limit(response, max_bytes)?, &content_type, max_bytes)?
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
    let declared = content_type
        .split(';')
        .skip(1)
        .find_map(|part| {
            let (name, value) = part.trim().split_once('=')?;
            name.trim()
                .eq_ignore_ascii_case("charset")
                .then(|| value.trim().trim_matches(['\'', '"']))
        })
        .and_then(|label| Encoding::for_label(label.as_bytes()));
    let (encoding, skip) = Encoding::for_bom(bytes).unwrap_or((declared.unwrap_or(UTF_8), 0));
    let (decoded, _) = encoding.decode_without_bom_handling(&bytes[skip..]);
    if decoded.len() > max_bytes {
        return Err(limit_error(max_bytes));
    }
    Ok(decoded.into_owned().into_bytes())
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
}
