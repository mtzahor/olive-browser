//! The browser's bounded, in-memory cookie jar.
use super::{Location, Url};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_COOKIES: usize = 256;
const MAX_COOKIE_BYTES: usize = 4 * 1024;
const MAX_COOKIE_HEADER_BYTES: usize = 64 * 1024;
const MAX_COOKIE_JAR_BYTES: usize = 64 * 1024;
const MAX_COOKIE_UPDATES: usize = 256;

/// A bounded, in-memory HTTP cookie jar with expiry. The browser never writes
/// cookies to disk, including cookies with `Max-Age` or `Expires` attributes.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CookieJar {
    cookies: Vec<StoredCookie>,
    next_creation: u64,
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
enum SameSite {
    Strict,
    Lax,
    None,
}

/// A cookie mutation produced by a document or one of its resources. The UI
/// applies these mutations to its shared jar after a worker completes.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CookieUpdate {
    pub location: Location,
    pub set_cookie: String,
    pub from_script: bool,
    pub(super) received_at: i64,
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

    /// Keep an update from the tab process rendering `document` only when it
    /// is for that document's own site, received no earlier than `since` (when
    /// the process started) and no later than now. Tab processes see no
    /// responses, so the update is a script's and cannot set HttpOnly cookies.
    pub fn confine(mut self, document: &Location, since: SystemTime) -> Option<Self> {
        if !same_cookie_site(document, &self.location) {
            return None;
        }
        self.from_script = true;
        let now = now_seconds();
        let since = since
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs().min(i64::MAX as u64) as i64)
            .min(now);
        self.received_at = self.received_at.clamp(since, now);
        Some(self)
    }
}

impl CookieJar {
    /// The cookies a tab process rendering `document` may hold: those of the
    /// document's own site, and on an HTTP page only those without `Secure`.
    /// Other sites' cookies, and every cookie for a local document, stay in
    /// the browser.
    pub fn for_document(&self, document: &Location) -> Self {
        self.site_cookies(document, true)
    }

    /// The cookies a script in `document` may read: those of
    /// [`Self::for_document`] without HttpOnly cookies.
    pub fn for_script(&self, document: &Location) -> Self {
        self.site_cookies(document, false)
    }

    fn site_cookies(&self, document: &Location, http_only: bool) -> Self {
        let url = document.url();
        let site = match url.scheme() {
            "http" | "https" => url.host_str().map(cookie_site),
            _ => None,
        };
        let https = url.scheme() == "https";
        Self {
            cookies: self
                .cookies
                .iter()
                .filter(|cookie| {
                    site == Some(cookie_site(&cookie.domain))
                        && (https || !cookie.secure)
                        && (http_only || !cookie.http_only)
                })
                .cloned()
                .collect(),
            next_creation: self.next_creation,
        }
    }

    pub fn cookie_header(&mut self, location: &Location) -> Option<String> {
        self.cookie_header_for(location, true, true, true)
    }

    pub(super) fn cookie_header_for(
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

    pub(super) fn absorb(&mut self, location: &Location, value: &str) -> bool {
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

pub(super) fn now_seconds() -> i64 {
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

/// The registrable domain whose cookies `host` shares, or an IP address itself.
fn cookie_site(host: &str) -> &str {
    if host.starts_with('[') || host.parse::<std::net::Ipv4Addr>().is_ok() {
        host
    } else {
        psl::domain_str(host).unwrap_or(host)
    }
}

pub(super) fn same_cookie_site(left: &Location, right: &Location) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn shared_hosting_tenants_have_separate_cookie_sites() {
        for suffix in [
            "surge.sh",
            "aws.databricksapps.com",
            "gcp.databricksapps.com",
            "tenant.azure.databricksapps.com",
            "region.compute.herokuapp.com",
        ] {
            let first = Location::from_input(&format!("https://alice.{suffix}/")).unwrap();
            let second = Location::from_input(&format!("https://bob.{suffix}/")).unwrap();
            let child = Location::from_input(&format!("https://cdn.alice.{suffix}/")).unwrap();
            assert!(!same_cookie_site(&first, &second), "{suffix}");
            assert!(same_cookie_site(&first, &child), "{suffix}");
            let mut jar = CookieJar::default();
            assert!(
                !jar.absorb(&first, &format!("shared=1; Domain={suffix}")),
                "{suffix}"
            );
            assert!(
                !jar.set_document_cookie(&first, &format!("shared=1; Domain={suffix}")),
                "{suffix}"
            );
            assert!(jar.absorb(&first, "host=1; Secure; SameSite=Strict"));
            assert!(jar.absorb(
                &first,
                &format!("tenant=1; Domain=alice.{suffix}; Secure; SameSite=Strict")
            ));
            assert_eq!(jar.cookie_header(&second), None);
            assert_eq!(jar.cookie_header(&child).as_deref(), Some("tenant=1"));
            assert_eq!(
                jar.cookie_header_for(&first, same_cookie_site(&first, &second), false, true),
                None
            );
        }
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
    fn tab_processes_hold_only_their_own_sites_cookies() {
        let page = Location::from_input("https://www.example.com/page").unwrap();
        let sibling = Location::from_input("https://cdn.example.com/").unwrap();
        let insecure = Location::from_input("http://www.example.com/").unwrap();
        let other = Location::from_input("https://example.net/").unwrap();
        let tenant = Location::from_input("https://alice.surge.sh/").unwrap();
        let loopback = Location::from_input("http://127.0.0.1:8000/").unwrap();
        let mut jar = CookieJar::default();
        assert!(jar.absorb(&page, "session=1; Secure; HttpOnly; SameSite=None"));
        assert!(jar.absorb(&sibling, "wide=2; Domain=example.com"));
        assert!(jar.absorb(&other, "tracker=3; Secure; SameSite=None"));
        assert!(jar.absorb(&tenant, "tenant=4"));
        assert!(jar.absorb(&loopback, "local=5"));

        let names = |jar: &mut CookieJar, location: &Location| {
            let mut header = jar.cookie_header(location).unwrap_or_default();
            header.retain(|c| c != ' ');
            header
        };
        let mut scoped = jar.for_document(&page);
        assert_eq!(names(&mut scoped, &page), "session=1;wide=2");
        assert_eq!(names(&mut scoped, &sibling), "wide=2");
        assert_eq!(scoped.cookie_header(&other), None);
        // HTTP pages never receive the site's Secure cookies.
        let mut scoped = jar.for_document(&insecure);
        assert_eq!(names(&mut scoped, &page), "wide=2");
        let mut scoped = jar.for_document(&other);
        assert_eq!(scoped.cookie_header(&page), None);
        assert_eq!(names(&mut scoped, &other), "tracker=3");
        let mut scoped = jar.for_document(&loopback);
        assert_eq!(names(&mut scoped, &loopback), "local=5");
        assert_eq!(
            jar.for_document(&Location::from_input("https://bob.surge.sh/").unwrap())
                .cookies
                .len(),
            0
        );
        let local = Location::from_path(std::env::current_dir().unwrap().join("a.html")).unwrap();
        assert!(jar.for_document(&local).cookies.is_empty());
        // Scripts never receive HttpOnly cookies.
        let mut scoped = jar.for_script(&page);
        assert_eq!(names(&mut scoped, &page), "wide=2");
    }

    #[test]
    fn tab_process_updates_are_confined_to_their_site_and_lifetime() {
        let page = Location::from_input("https://www.example.com/page").unwrap();
        let since = SystemTime::now() - std::time::Duration::from_secs(60);
        let started = now_seconds() - 60;
        let update = |location: &str, received_at| CookieUpdate {
            location: Location::from_input(location).unwrap(),
            set_cookie: "a=1".into(),
            from_script: false,
            received_at,
        };
        let kept = update("https://cdn.example.com/", started + 1)
            .confine(&page, since)
            .unwrap();
        assert_eq!(kept.received_at, started + 1);
        // A tab process cannot claim a response's right to set HttpOnly.
        assert!(kept.from_script);
        for rejected in [
            "https://example.net/",
            "http://www.example.com/",
            "https://example.com.evil/",
        ] {
            assert!(update(rejected, started).confine(&page, since).is_none());
        }
        let early = update("https://www.example.com/", 0).confine(&page, since);
        assert!(early.unwrap().received_at >= started);
        let late = update("https://www.example.com/", i64::MAX).confine(&page, since);
        assert!(late.unwrap().received_at <= now_seconds());
    }
}
