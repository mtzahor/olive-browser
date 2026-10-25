//! Validated document addresses and URL resolution.
use super::{MAX_URL_BYTES, Url};
use crate::Document;
use std::path::{Path, PathBuf};

/// A validated address. Credentials and non-local file authorities are rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location(pub(super) Url);

// Deserialize through the same validation used by normal navigation.
#[cfg(feature = "serde")]
impl serde::Serialize for Location {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
#[cfg(feature = "serde")]
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
        if ends_in_drive_letter(&self.0) {
            return Err("Links from a file named like a drive letter are not supported.".into());
        }
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
                if element.is_html_tag("base") {
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

/// A file URL whose last segment is shaped like a Windows drive letter but
/// is not the first, as in `file:///notes/c:`. `url` keeps that segment when
/// resolving against it, so the result is wrong and debug builds panic
/// (servo/rust-url#1114).
fn ends_in_drive_letter(url: &Url) -> bool {
    url.scheme() == "file"
        && url.path().rsplit_once('/').is_some_and(|(parent, last)| {
            !parent.is_empty()
                && matches!(last.as_bytes(), [letter, b':'] if letter.is_ascii_alphabetic())
        })
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
    fn links_from_a_file_named_like_a_drive_letter() {
        let page = |address| Location(Url::parse(address).unwrap());
        for (address, href) in [
            ("file:///notes/c:", ".."),
            ("file:///notes/c:", "../next.html"),
            ("file:///p/a:", "..#ncoYl"),
            ("file:///=n:/t:", ".."),
        ] {
            assert!(page(address).resolve(href).is_err(), "{address} {href}");
        }
        // A drive letter in drive position is the root it names.
        assert_eq!(
            page("file:///c:/notes/a.html")
                .resolve("../b.html")
                .unwrap()
                .as_str(),
            "file:///c:/b.html"
        );
        assert_eq!(
            page("file:///c:").resolve("next.html").unwrap().as_str(),
            "file:///c:/next.html"
        );
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
}
