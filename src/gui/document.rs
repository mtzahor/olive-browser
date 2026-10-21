//! Document work runs in a tab child process, never in the browser UI.
use crate::gui::render::{ImageAsset, Page};
use crate::js::{DocumentSession, ScriptOptions, ScriptReport};
use crate::{
    ExternalSource, NodeId, ParseOutput,
    css::Stylesheet,
    net::{CookieJar, CookieUpdate},
    net::{DocumentLoader, LoadedDocument, Location},
    resources::{PageResources, RESOURCE_TIMEOUT, ResourceReport},
};
use eframe::egui;
use image::{ImageReader, Limits};
use std::{collections::HashMap, io::Cursor};
pub(crate) const MAX_IMAGE_DIMENSION: u32 = 4_096;
const MAX_IMAGE_DECODE_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_PAGE_IMAGE_PIXELS: u64 = 8 * 1024 * 1024;
/// The largest side of a decoded site icon, enough for a 16-point tab icon on
/// a 2× display.
pub const FAVICON_PIXELS: u32 = 32;
/// How long a page waits for its icon after its other resources are in.
const FAVICON_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

/// A site icon, decoded in the tab process and scaled down to fit
/// `FAVICON_PIXELS`.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Favicon(
    #[serde(with = "crate::gui::worker::wire_image")] pub std::sync::Arc<egui::ColorImage>,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub struct LoadedPage {
    pub location: Location,
    pub base: Location,
    pub status: Option<u16>,
    pub page: Page,
    pub reading: Page,
    pub corrections: usize,
    pub scripts: ScriptReport,
    pub scripting_enabled: bool,
    pub resources: ResourceReport,
    #[serde(default)]
    pub cookie_updates: Vec<CookieUpdate>,
    #[serde(default)]
    pub favicon: Option<Favicon>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ClickRequest {
    pub target: NodeId,
    pub href: Option<String>,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PageUpdate {
    pub default_allowed: bool,
    pub page: Page,
    pub reading: Page,
    pub base: Location,
    pub scripts: ScriptReport,
    #[serde(default)]
    pub resources: ResourceReport,
    #[serde(default)]
    pub session_available: bool,
    #[serde(default)]
    pub presentation_available: bool,
    pub alert: Option<String>,
    pub link: Option<String>,
    #[serde(default)]
    pub cookie_updates: Vec<CookieUpdate>,
    /// A newly loaded site icon; updates without one keep the current icon.
    #[serde(default)]
    pub favicon: Option<Favicon>,
}
impl LoadedPage {
    /// Swap in a worker's newer presentation, keeping the user's form edits.
    pub fn apply_update(&mut self, update: &mut PageUpdate) {
        update.page.forms.preserve_edits(&self.page.forms);
        update.page.reuse_text_layout(&mut self.page);
        update.reading.reuse_text_layout(&mut self.reading);
        self.page = std::mem::take(&mut update.page);
        self.reading = std::mem::take(&mut update.reading);
        self.base = update.base.clone();
        self.scripts = std::mem::take(&mut update.scripts);
        if let Some(favicon) = update.favicon.take() {
            self.favicon = Some(favicon);
        }
    }

    /// Hand the whole page to the browser as a completed resource update.
    pub fn into_update(self) -> PageUpdate {
        PageUpdate {
            default_allowed: false,
            page: self.page,
            reading: self.reading,
            base: self.base,
            scripts: self.scripts,
            resources: self.resources,
            session_available: false,
            presentation_available: false,
            alert: None,
            link: None,
            cookie_updates: self.cookie_updates,
            favicon: self.favicon,
        }
    }
}

pub struct PreparedPage {
    pub loaded: LoadedPage,
    pub session: Option<DocumentSession>,
    document: Option<crate::Document>,
    pub styles: HashMap<NodeId, ExternalSource>,
    pub images: HashMap<NodeId, ImageAsset>,
    stylesheet: Stylesheet,
    viewport: (f32, f32),
}

/// The normal and reading views of one document state.
struct Presentation {
    page: Page,
    reading: Page,
    base: Location,
    sheet: Stylesheet,
}

fn present(
    document: &crate::Document,
    location: &Location,
    styles: &HashMap<NodeId, ExternalSource>,
    images: &HashMap<NodeId, ImageAsset>,
    scripting_enabled: bool,
    viewport: (f32, f32),
) -> Presentation {
    let sheet = Stylesheet::from_document_with_sources(document, styles);
    let base = location.document_base(document);
    let reading = Page::reading(
        document,
        scripting_enabled,
        &sheet,
        images,
        Some(&base),
        viewport,
    );
    let page = Page::new(document, scripting_enabled, &sheet, images, viewport);
    Presentation {
        page,
        reading,
        base,
        sheet,
    }
}

fn corrections(parsed: &ParseOutput) -> usize {
    parsed
        .diagnostics
        .len()
        .saturating_add(parsed.omitted_diagnostics)
}

/// Build the first visible page from the parsed document before waiting on linked resources.
pub fn prepare_page_preview(
    source: &LoadedDocument,
    scripting_enabled: bool,
    viewport: (f32, f32),
) -> Result<(LoadedPage, ParseOutput), String> {
    let parsed = source.parse(scripting_enabled)?;
    let view = present(
        &parsed.document,
        &source.location,
        &HashMap::new(),
        &HashMap::new(),
        scripting_enabled,
        viewport,
    );
    Ok((
        LoadedPage {
            location: source.location.clone(),
            base: view.base,
            status: source.status,
            page: view.page,
            reading: view.reading,
            corrections: corrections(&parsed),
            scripts: ScriptReport::default(),
            scripting_enabled,
            resources: ResourceReport {
                pending: true,
                ..ResourceReport::default()
            },
            cookie_updates: Vec::new(),
            favicon: None,
        },
        parsed,
    ))
}

#[cfg(test)]
pub fn prepare_page_with_loader(
    source: LoadedDocument,
    loader: &DocumentLoader,
    scripting_enabled: bool,
) -> Result<PreparedPage, String> {
    let parsed = source.parse(scripting_enabled)?;
    prepare_page_with_parsed(
        source,
        loader,
        scripting_enabled,
        parsed,
        false,
        crate::css::DEFAULT_VIEWPORT,
        None,
    )
}

/// Load the page's resources and build its presentation for a CSS viewport
/// of `viewport` pixels. With `favicon`, the site icon is fetched alongside
/// the other resources. `progress` receives the page styled by its
/// stylesheets while images and scripts are still loading.
pub fn prepare_page_with_parsed(
    source: LoadedDocument,
    loader: &DocumentLoader,
    scripting_enabled: bool,
    parsed: ParseOutput,
    favicon: bool,
    viewport: (f32, f32),
    mut progress: Option<&mut dyn FnMut(PageUpdate)>,
) -> Result<PreparedPage, String> {
    let corrections = corrections(&parsed);
    let icon_addresses = if favicon {
        PageResources::favicon_addresses(&source.location, &parsed.document)
    } else {
        Vec::new()
    };
    let (resources, favicon) = std::thread::scope(|scope| {
        let icon = (!icon_addresses.is_empty()).then(|| {
            scope.spawn(|| {
                PageResources::load_favicon(
                    loader,
                    &source.location,
                    &icon_addresses,
                    FAVICON_TIMEOUT,
                )
            })
        });
        let resources = PageResources::load_with_progress(
            loader,
            &source.location,
            &parsed.document,
            scripting_enabled,
            RESOURCE_TIMEOUT,
            |styles| {
                let Some(progress) = progress.as_mut() else {
                    return;
                };
                let view = present(
                    &parsed.document,
                    &source.location,
                    styles,
                    &HashMap::new(),
                    scripting_enabled,
                    viewport,
                );
                progress(PageUpdate {
                    default_allowed: false,
                    page: view.page,
                    reading: view.reading,
                    base: view.base,
                    scripts: ScriptReport::default(),
                    resources: ResourceReport {
                        pending: true,
                        ..ResourceReport::default()
                    },
                    session_available: false,
                    presentation_available: false,
                    alert: None,
                    link: None,
                    cookie_updates: Vec::new(),
                    favicon: None,
                });
            },
        );
        let icon = icon
            .and_then(|icon| icon.join().ok().flatten())
            .and_then(|icon| decode_favicon(&icon.bytes));
        (resources, icon)
    });
    let mut resource_report = resources.report;
    let images = decode_images(&resources.images, &mut resource_report);
    let render = |document: &crate::Document| {
        present(
            document,
            &source.location,
            &resources.styles,
            &images,
            scripting_enabled,
            viewport,
        )
    };
    let (view, session, document) = if scripting_enabled {
        let session = DocumentSession::with_sources_and_cookies(
            parsed.document,
            ScriptOptions::browser(),
            &resources.scripts,
            loader.cookies(),
            source.location.clone(),
        );
        (session.with_document(render), Some(session), None)
    } else {
        (render(&parsed.document), None, Some(parsed.document))
    };
    let scripts = session
        .as_ref()
        .map(|session| session.report().clone())
        .unwrap_or_default();
    let mut cookie_updates = loader.take_cookie_updates();
    if let Some(session) = &session {
        for update in session.take_cookie_updates() {
            update.record(&mut cookie_updates);
        }
    }
    Ok(PreparedPage {
        loaded: LoadedPage {
            location: source.location,
            base: view.base,
            status: source.status,
            page: view.page,
            reading: view.reading,
            corrections,
            scripts,
            scripting_enabled,
            resources: resource_report,
            cookie_updates,
            favicon,
        },
        session,
        document,
        styles: resources.styles,
        images,
        stylesheet: view.sheet,
        viewport,
    })
}

/// Decode the page's pictures. Each picture's header reserves its share of the
/// page's pixel budget in document order, so the same pictures show on every
/// load, and the reserved pictures then decode in parallel.
fn decode_images(
    sources: &HashMap<NodeId, crate::resources::ExternalImage>,
    report: &mut ResourceReport,
) -> HashMap<NodeId, ImageAsset> {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    let mut ordered: Vec<_> = sources.iter().collect();
    ordered.sort_unstable_by_key(|(id, _)| id.index());
    let mut pixels = 0_u64;
    let reserved: Vec<_> = ordered
        .into_iter()
        .map(|(&id, source)| {
            let result = image_pixels(&source.bytes).and_then(|image_pixels| {
                if pixels.saturating_add(image_pixels) > MAX_PAGE_IMAGE_PIXELS {
                    Err(format!(
                        "Page image pixel budget exhausted ({} pixels total).",
                        MAX_PAGE_IMAGE_PIXELS
                    ))
                } else {
                    pixels += image_pixels;
                    Ok(image_pixels)
                }
            });
            (id, source, result)
        })
        .collect();
    let decoded = Mutex::new(
        std::iter::repeat_with(|| None)
            .take(reserved.len())
            .collect::<Vec<_>>(),
    );
    // Workers take one picture at a time, so a few large ones do not hold up the rest.
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .clamp(1, 8)
        .min(reserved.len());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some((_, source, reserved)) = reserved.get(index) else {
                        break;
                    };
                    let result = reserved.clone().and_then(|reserved| {
                        let (image, pixels) = decode_image(&source.bytes)?;
                        if pixels > reserved {
                            return Err("image is larger than its header states".to_owned());
                        }
                        Ok(image)
                    });
                    if let Ok(mut decoded) = decoded.lock() {
                        decoded[index] = Some(result);
                    }
                }
            });
        }
    });
    let mut images = HashMap::new();
    let decoded = decoded.into_inner().unwrap_or_default();
    for ((id, source, _), result) in reserved.iter().zip(decoded) {
        match result.unwrap_or_else(|| Err("image decoding stopped".into())) {
            Ok(image) => {
                images.insert(
                    *id,
                    ImageAsset {
                        reference: source.reference.clone(),
                        image,
                    },
                );
            }
            Err(error) => {
                let message = format!("{}: {error}", source.reference);
                report
                    .diagnostics
                    .push(crate::text::truncate(&message, 1024).to_owned());
            }
        }
    }
    images
}

/// The pixel count a picture's header states, within the per-image limits.
fn image_pixels(bytes: &[u8]) -> Result<u64, String> {
    let (width, height) = if bytes.starts_with(&[0xff, 0xd8]) {
        let mut decoder = zune_jpeg::JpegDecoder::new(Cursor::new(bytes));
        decoder
            .decode_headers()
            .map_err(|error| format!("could not read image dimensions: {error}"))?;
        let info = decoder
            .info()
            .ok_or_else(|| "could not read image dimensions".to_owned())?;
        (u32::from(info.width), u32::from(info.height))
    } else {
        ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|error| format!("could not identify image format: {error}"))?
            .into_dimensions()
            .map_err(|error| format!("could not decode image: {error}"))?
    };
    checked_image_pixels(width, height)
}

fn decode_image(bytes: &[u8]) -> Result<(std::sync::Arc<egui::ColorImage>, u64), String> {
    if bytes.starts_with(&[0xff, 0xd8]) {
        return decode_jpeg(bytes);
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| format!("could not identify image format: {error}"))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_IMAGE_DECODE_BYTES);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| format!("could not decode image: {error}"))?;
    let width = image.width();
    let height = image.height();
    let pixels = checked_image_pixels(width, height)?;
    let rgba = image.to_rgba8();
    Ok((color_image(width, height, rgba.as_raw()), pixels))
}

/// Decode a site icon and scale it down to fit `FAVICON_PIXELS`. Icon files
/// hold several sizes; the decoder picks the largest.
fn decode_favicon(bytes: &[u8]) -> Option<Favicon> {
    let (image, _) = decode_image(bytes).ok()?;
    let [width, height] = image.size.map(|side| side as u32);
    let pixels = image
        .pixels
        .iter()
        .flat_map(|pixel| pixel.to_srgba_unmultiplied())
        .collect();
    let rgba = image::RgbaImage::from_raw(width, height, pixels)?;
    let scale = (FAVICON_PIXELS as f32 / width.max(height) as f32).min(1.0);
    let fitted = image::imageops::thumbnail(
        &rgba,
        ((width as f32 * scale).round() as u32).max(1),
        ((height as f32 * scale).round() as u32).max(1),
    );
    Some(Favicon(color_image(
        fitted.width(),
        fitted.height(),
        fitted.as_raw(),
    )))
}

fn decode_jpeg(bytes: &[u8]) -> Result<(std::sync::Arc<egui::ColorImage>, u64), String> {
    use zune_jpeg::{JpegDecoder, zune_core::colorspace::ColorSpace};

    let options = zune_jpeg::zune_core::options::DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_use_unsafe(false);
    let mut decoder = JpegDecoder::new_with_options(Cursor::new(bytes), options);
    decoder
        .decode_headers()
        .map_err(|error| format!("could not read image dimensions: {error}"))?;
    let info = decoder
        .info()
        .ok_or_else(|| "could not read image dimensions".to_owned())?;
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    let pixels = checked_image_pixels(width, height)?;
    let rgba = decoder
        .decode()
        .map_err(|error| format!("could not decode image: {error}"))?;
    let expected = usize::try_from(pixels)
        .ok()
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "decoded image is too large".to_owned())?;
    if rgba.len() != expected {
        return Err("JPEG decoder returned an unexpected pixel buffer".into());
    }
    Ok((color_image(width, height, &rgba), pixels))
}

fn checked_image_pixels(width: u32, height: u32) -> Result<u64, String> {
    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(format!(
            "image dimensions exceed the {MAX_IMAGE_DIMENSION}px limit"
        ));
    }
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if pixels > MAX_PAGE_IMAGE_PIXELS {
        return Err(format!(
            "image has too many pixels (maximum {MAX_PAGE_IMAGE_PIXELS})"
        ));
    }
    Ok(pixels)
}

fn color_image(width: u32, height: u32, rgba: &[u8]) -> std::sync::Arc<egui::ColorImage> {
    std::sync::Arc::new(egui::ColorImage::from_rgba_unmultiplied(
        [width as usize, height as usize],
        rgba,
    ))
}

impl PreparedPage {
    fn presentation_available(&self) -> bool {
        self.session.is_some() || self.stylesheet.needs_viewport_reflow()
    }

    /// Move the prepared presentation into the update that completes resource
    /// loading. The document and its metadata stay for later clicks and reflows.
    pub fn take_update(&mut self) -> PageUpdate {
        PageUpdate {
            favicon: self.loaded.favicon.clone(),
            default_allowed: false,
            page: std::mem::take(&mut self.loaded.page),
            reading: std::mem::take(&mut self.loaded.reading),
            base: self.loaded.base.clone(),
            scripts: self.loaded.scripts.clone(),
            resources: self.loaded.resources.clone(),
            session_available: self.session.is_some(),
            presentation_available: self.presentation_available(),
            alert: None,
            link: None,
            cookie_updates: std::mem::take(&mut self.loaded.cookie_updates),
        }
    }

    /// Run a click handler after refreshing the session with the browser's cookies.
    pub fn click(&mut self, click: ClickRequest, cookies: CookieJar) -> Option<PageUpdate> {
        let session = self.session.as_mut()?;
        session.replace_cookies(cookies);
        let allowed = session.click(click.target);
        let view = session.with_document(|document| {
            present(
                document,
                &self.loaded.location,
                &self.styles,
                &self.images,
                true,
                self.viewport,
            )
        });
        Some(PageUpdate {
            default_allowed: allowed,
            page: view.page,
            reading: view.reading,
            base: view.base,
            scripts: session.report().clone(),
            resources: self.loaded.resources.clone(),
            session_available: true,
            presentation_available: true,
            alert: session.take_alerts().into_iter().next(),
            link: if allowed { click.href } else { None },
            cookie_updates: session.take_cookie_updates(),
            favicon: None,
        })
    }

    pub fn resize(&mut self, viewport: (f32, f32)) -> Option<PageUpdate> {
        let viewport = (
            viewport.0.clamp(1.0, 10_000.0),
            viewport.1.clamp(1.0, 10_000.0),
        );
        let render = |document: &crate::Document| {
            present(
                document,
                &self.loaded.location,
                &self.styles,
                &self.images,
                self.loaded.scripting_enabled,
                viewport,
            )
        };
        let view = match &self.session {
            Some(session) => session.with_document(render),
            None => render(self.document.as_ref()?),
        };
        self.viewport = viewport;
        self.stylesheet = view.sheet;
        self.loaded.base = view.base.clone();
        Some(PageUpdate {
            default_allowed: false,
            page: view.page,
            reading: view.reading,
            base: view.base,
            scripts: self.loaded.scripts.clone(),
            resources: self.loaded.resources.clone(),
            session_available: self.session.is_some(),
            presentation_available: true,
            alert: None,
            link: None,
            cookie_updates: Vec::new(),
            favicon: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_are_laid_out_for_the_requested_viewport() {
        let source = LoadedDocument {
            location: Location::from_input("https://example.com/").unwrap(),
            bytes:
                b"<style>@media (max-width: 600px) { p { color: rgb(255, 0, 0) } }</style><p>Narrow"
                    .to_vec(),
            status: Some(200),
            plain_text: false,
        };
        let red = |viewport| {
            let (preview, _) = prepare_page_preview(&source, false, viewport).unwrap();
            serde_json::to_string(&preview.page)
                .unwrap()
                .contains("[255,0,0,255]")
        };
        assert!(red((500.0, 700.0)));
        assert!(!red((1024.0, 768.0)));
    }

    #[test]
    fn the_pixel_budget_goes_to_pictures_in_document_order() {
        // A BMP header states the size; the missing pixels fail to decode later.
        let header = |side: i32| {
            let mut bmp = b"BM".to_vec();
            for value in [54_u32, 0, 54, 40] {
                bmp.extend_from_slice(&value.to_le_bytes());
            }
            bmp.extend_from_slice(&side.to_le_bytes());
            bmp.extend_from_slice(&side.to_le_bytes());
            bmp.extend_from_slice(&1_u16.to_le_bytes());
            bmp.extend_from_slice(&24_u16.to_le_bytes());
            bmp.extend(vec![0; 24]);
            bmp
        };
        let doc = crate::parse("<img src=z.bmp><img src=y.bmp><img src=x.bmp>")
            .unwrap()
            .document;
        let sources: HashMap<_, _> = doc
            .descendants(doc.root())
            .filter_map(|id| {
                let reference = doc.node(id)?.as_element()?.attribute("src")?.to_owned();
                Some((
                    id,
                    crate::resources::ExternalImage {
                        reference,
                        bytes: header(2048),
                    },
                ))
            })
            .collect();
        for _ in 0..3 {
            let mut report = ResourceReport::default();
            assert!(decode_images(&sources, &mut report).is_empty());
            let exhausted: Vec<_> = report
                .diagnostics
                .iter()
                .filter(|message| message.contains("budget exhausted"))
                .collect();
            assert_eq!(exhausted.len(), 1);
            assert!(exhausted[0].starts_with("x.bmp:"), "{exhausted:?}");
        }
    }
    #[test]
    fn decodes_gif_and_icon_files_and_scales_site_icons_down() {
        use image::ImageEncoder;
        let mut gif = Vec::new();
        image::codecs::gif::GifEncoder::new(&mut gif)
            .encode(
                &[0, 0, 255, 255].repeat(6),
                3,
                2,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        let (decoded, pixels) = decode_image(&gif).unwrap();
        assert_eq!((decoded.size, pixels), ([3, 2], 6));
        assert_eq!(decoded.pixels[0], egui::Color32::BLUE);

        // An icon file holding a 64-pixel icon becomes a 32-pixel site icon.
        let mut ico = Vec::new();
        image::codecs::ico::IcoEncoder::new(&mut ico)
            .write_image(
                &[255, 0, 0, 255].repeat(64 * 64),
                64,
                64,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        let icon = decode_favicon(&ico).unwrap();
        assert_eq!(icon.0.size, [FAVICON_PIXELS as usize; 2]);
        assert_eq!(icon.0.pixels[0], egui::Color32::RED);
        // Small and wide icons keep their size and shape.
        let mut wide = Vec::new();
        image::codecs::png::PngEncoder::new(&mut wide)
            .write_image(
                &[0, 255, 0, 255].repeat(16 * 8),
                16,
                8,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        assert_eq!(decode_favicon(&wide).unwrap().0.size, [16, 8]);
        assert!(decode_favicon(b"<svg/>").is_none());
    }
    #[test]
    fn decodes_webp_alpha_and_rejects_excessive_dimensions_and_truncation() {
        let rgba = [255, 0, 0, 255, 0, 255, 0, 128];
        let mut encoded = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut encoded)
            .encode(&rgba, 2, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        let (decoded, pixels) = decode_image(&encoded).unwrap();
        assert_eq!(pixels, 2);
        assert_eq!(decoded.size, [2, 1]);
        assert_eq!(decoded.pixels[0], egui::Color32::RED);
        assert_eq!(
            decoded.pixels[1],
            egui::Color32::from_rgba_unmultiplied(0, 255, 0, 128)
        );
        assert!(decode_image(&encoded[..16]).is_err());
        encoded.clear();
        image::codecs::webp::WebPEncoder::new_lossless(&mut encoded)
            .encode(&vec![0; 4097 * 4], 4097, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert!(decode_image(&encoded).is_err());
    }
    #[test]
    fn decodes_jpeg_rgba_and_rejects_excessive_dimensions_and_truncation() {
        let encoded = include_bytes!("../../tests/fixtures/images/red-2x1.jpg");
        let (decoded, pixels) = decode_image(encoded).unwrap();
        assert_eq!(pixels, 2);
        assert_eq!(decoded.size, [2, 1]);
        for pixel in &decoded.pixels {
            let [r, g, b, a] = pixel.to_array();
            // JPEG is lossy, so allow small rounding differences in the red pixels.
            assert!(r >= 250 && g <= 5 && b <= 5);
            assert_eq!(a, 255);
        }
        assert!(decode_image(&encoded[..16]).is_err());

        // Increase the baseline frame width without allocating a large image.
        let mut oversized = encoded.to_vec();
        let frame = oversized
            .windows(2)
            .position(|marker| marker == [0xff, 0xc0])
            .unwrap();
        oversized[frame + 7..frame + 9]
            .copy_from_slice(&((MAX_IMAGE_DIMENSION + 1) as u16).to_be_bytes());
        let error = decode_image(&oversized).unwrap_err();
        assert!(error.contains("image dimensions exceed"), "{error}");
    }
    #[test]
    fn decodes_png_images_with_bounded_dimensions() {
        let (image, pixels) =
            decode_image(include_bytes!("../../assets/olive-browser.png")).unwrap();
        assert_eq!(image.size, [1024, 1024]);
        assert_eq!(pixels, 1024 * 1024);
        assert!(decode_image(b"not an image").is_err());
    }
}
