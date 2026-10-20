//! Site icons. Tab processes fetch and decode them; the browser keeps them as
//! textures for the session and as small PNG files in the profile, so
//! bookmarks, history and restored tabs show them after a restart. Icons are
//! kept per host, and clearing browsing history removes them.
use crate::gui::{
    document::{FAVICON_PIXELS, Favicon},
    storage,
};
use crate::net::Url;
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use image::ImageEncoder;
use std::{collections::HashMap, path::PathBuf, sync::Arc};

const DIRECTORY: &str = "favicons";
const MAX_FILE_BYTES: u64 = 64 * 1024;
/// Textures held at once; the cache starts over when it is full.
const MAX_TEXTURES: usize = 512;

#[derive(Default)]
pub struct Favicons {
    dir: Option<PathBuf>,
    /// Textures by host. `None` remembers that a host has no stored icon.
    textures: HashMap<String, Option<TextureHandle>>,
}

impl Favicons {
    pub fn load_default() -> Self {
        Self {
            dir: storage::profile_dir().map(|dir| dir.join(DIRECTORY)),
            textures: HashMap::new(),
        }
    }

    /// The host whose icon a web address shows. Local files have none.
    fn host(url: &str) -> Option<String> {
        let url = Url::parse(url).ok()?;
        matches!(url.scheme(), "http" | "https")
            .then(|| url.host_str().map(str::to_ascii_lowercase))
            .flatten()
    }

    /// Keep the icon of the page at `url`, replacing the host's earlier one.
    pub fn store(&mut self, ctx: &egui::Context, url: &str, icon: &Favicon) {
        let Some(host) = Self::host(url) else {
            return;
        };
        if let Some(path) = self.path(&host) {
            if let Some(png) = encode(&icon.0) {
                // A missing icon file only costs the icon after a restart.
                let _ = storage::write_state(&path, &png);
            }
        }
        let texture = texture(ctx, &host, icon.0.clone());
        self.insert(host, Some(texture));
    }

    /// The icon last seen for the host of `url`, if any. A site that
    /// redirects between `www.` and its bare domain shares one icon.
    pub fn get(&mut self, ctx: &egui::Context, url: &str) -> Option<TextureHandle> {
        let host = Self::host(url)?;
        if let Some(entry) = self.textures.get(&host) {
            return entry.clone();
        }
        let sibling = match host.strip_prefix("www.") {
            Some(bare) => bare.to_owned(),
            None => format!("www.{host}"),
        };
        let texture = [&host, &sibling].into_iter().find_map(|name| {
            if let Some(Some(texture)) = self.textures.get(name) {
                return Some(texture.clone());
            }
            let png = storage::read_bounded(&self.path(name)?, MAX_FILE_BYTES, "Site icon")
                .ok()
                .flatten()?;
            Some(texture(ctx, name, decode(&png)?))
        });
        self.insert(host, texture.clone());
        texture
    }

    /// Forget every icon, as when browsing history is cleared.
    pub fn clear(&mut self) {
        self.textures.clear();
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// Delete stored icons of hosts that no address in `keep` belongs to.
    pub fn prune<'a>(&self, keep: impl IntoIterator<Item = &'a str>) {
        let Some(dir) = &self.dir else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let keep: std::collections::HashSet<_> = keep
            .into_iter()
            .filter_map(Self::host)
            .map(|host| file_name(&host))
            .collect();
        for entry in entries.flatten() {
            let name = entry.file_name();
            if !keep.contains(name.to_string_lossy().as_ref()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    fn path(&self, host: &str) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join(file_name(host)))
    }

    fn insert(&mut self, host: String, texture: Option<TextureHandle>) {
        if self.textures.len() >= MAX_TEXTURES {
            self.textures.clear();
        }
        self.textures.insert(host, texture);
    }
}

/// A file name for a host. Hosts are ASCII after IDNA; anything outside
/// letters, digits, dots and hyphens (an IPv6 address) is hex-encoded.
fn file_name(host: &str) -> String {
    let safe = host
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
        && !host.starts_with('.');
    let stem = if safe {
        host.to_owned()
    } else {
        host.bytes().map(|byte| format!("{byte:02x}")).collect()
    };
    format!("{stem}.png")
}

fn texture(ctx: &egui::Context, host: &str, image: Arc<ColorImage>) -> TextureHandle {
    ctx.load_texture(format!("favicon-{host}"), image, TextureOptions::LINEAR)
}

fn encode(image: &ColorImage) -> Option<Vec<u8>> {
    let pixels: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|pixel| pixel.to_srgba_unmultiplied())
        .collect();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &pixels,
            image.size[0] as u32,
            image.size[1] as u32,
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some(png)
}

fn decode(png: &[u8]) -> Option<Arc<ColorImage>> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    if image.width().max(image.height()) > FAVICON_PIXELS || image.width().min(image.height()) == 0
    {
        return None;
    }
    Some(Arc::new(ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_kept_per_web_host_and_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut favicons = Favicons {
            dir: Some(dir.path().join(DIRECTORY)),
            ..Default::default()
        };
        let icon = Favicon(Arc::new(ColorImage::new(
            [16, 16],
            vec![egui::Color32::from_rgb(200, 30, 30); 256],
        )));
        favicons.store(&ctx, "https://www.Example.com/a?b", &icon);
        favicons.store(&ctx, "file:///tmp/page.html", &icon);
        assert!(favicons.get(&ctx, "http://www.example.com/other").is_some());
        // A bare domain borrows its www. sibling's icon, but not other subdomains.
        assert!(favicons.get(&ctx, "https://example.com/").is_some());
        assert!(favicons.get(&ctx, "https://docs.example.com/").is_none());
        assert!(favicons.get(&ctx, "file:///tmp/page.html").is_none());

        let mut restarted = Favicons {
            dir: Some(dir.path().join(DIRECTORY)),
            ..Default::default()
        };
        let texture = restarted.get(&ctx, "https://www.example.com/").unwrap();
        assert_eq!(texture.size(), [16, 16]);

        restarted.prune(["https://elsewhere.org/"]);
        let mut pruned = Favicons {
            dir: Some(dir.path().join(DIRECTORY)),
            ..Default::default()
        };
        assert!(pruned.get(&ctx, "https://www.example.com/").is_none());
        favicons.clear();
        assert!(!dir.path().join(DIRECTORY).exists());
    }

    #[test]
    fn unusual_hosts_get_safe_file_names() {
        assert_eq!(file_name("www.example.com"), "www.example.com.png");
        assert_eq!(file_name("[::1]"), "5b3a3a315d.png");
        assert_eq!(file_name("..x"), "2e2e78.png");
    }
}
