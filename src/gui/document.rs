//! Document work runs in a tab child process, never in the browser UI.
use crate::render::{ImageAsset, Page};
use eframe::egui;
use image::{ImageReader, Limits};
use olive_html::js::{DocumentSession, ScriptOptions, ScriptReport};
use olive_html::{
    ExternalSource, NodeId,
    css::Stylesheet,
    net::{DocumentLoader, LoadedDocument, Location},
    resources::{PageResources, ResourceReport},
};
use std::{collections::HashMap, io::Cursor};
const MAX_IMAGE_DIMENSION: u32 = 4_096;
const MAX_IMAGE_DECODE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PAGE_IMAGE_PIXELS: u64 = 8 * 1024 * 1024;
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
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ClickRequest {
    pub target: NodeId,
    pub href: Option<String>,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PageUpdate {
    pub page: Page,
    pub reading: Page,
    pub base: Location,
    pub scripts: ScriptReport,
    pub alert: Option<String>,
    pub link: Option<String>,
}
pub struct PreparedPage {
    pub loaded: LoadedPage,
    pub session: Option<DocumentSession>,
    pub styles: HashMap<NodeId, ExternalSource>,
    pub images: HashMap<NodeId, ImageAsset>,
}

pub fn prepare_page_with_loader(
    source: LoadedDocument,
    loader: &DocumentLoader,
    scripting_enabled: bool,
) -> Result<PreparedPage, String> {
    let parsed = source.parse(scripting_enabled)?;
    let corrections = parsed
        .diagnostics
        .len()
        .saturating_add(parsed.omitted_diagnostics);
    let resources = PageResources::load(
        loader,
        &source.location,
        &parsed.document,
        scripting_enabled,
    );
    let mut resource_report = resources.report;
    let images = decode_images(&resources.images, &mut resource_report);
    let (page, reading, base, scripts, session) = if scripting_enabled {
        let session = DocumentSession::with_sources(
            parsed.document,
            ScriptOptions::browser(),
            &resources.scripts,
        );
        let (page, reading, base) = session.with_document(|document| {
            let sheet = Stylesheet::from_document_with_sources(document, &resources.styles);
            let reading = Page::reading(document, true, &sheet, &images);
            (
                Page::with_stylesheet_and_images(document, true, sheet, &images),
                reading,
                source.location.document_base(document),
            )
        });
        (page, reading, base, session.report().clone(), Some(session))
    } else {
        let document = parsed.document;
        let sheet = Stylesheet::from_document_with_sources(&document, &resources.styles);
        let reading = Page::reading(&document, false, &sheet, &images);
        let page = Page::with_stylesheet_and_images(&document, false, sheet, &images);
        (
            page,
            reading,
            source.location.document_base(&document),
            ScriptReport::default(),
            None,
        )
    };
    Ok(PreparedPage {
        loaded: LoadedPage {
            location: source.location,
            base,
            status: source.status,
            page,
            reading,
            corrections,
            scripts,
            scripting_enabled,
            resources: resource_report,
        },
        session,
        styles: resources.styles,
        images,
    })
}

fn decode_images(
    sources: &HashMap<NodeId, olive_html::resources::ExternalImage>,
    report: &mut ResourceReport,
) -> HashMap<NodeId, ImageAsset> {
    let mut decoded = HashMap::new();
    let mut pixels = 0_u64;
    for (&id, source) in sources {
        let result = decode_image(&source.bytes).and_then(|(image, image_pixels)| {
            if pixels.saturating_add(image_pixels) > MAX_PAGE_IMAGE_PIXELS {
                Err(format!(
                    "Page image pixel budget exhausted ({} pixels total).",
                    MAX_PAGE_IMAGE_PIXELS
                ))
            } else {
                pixels += image_pixels;
                Ok(image)
            }
        });
        match result {
            Ok(image) => {
                decoded.insert(
                    id,
                    ImageAsset {
                        reference: source.reference.clone(),
                        image,
                    },
                );
            }
            Err(error) => {
                let message = format!("{}: {error}", source.reference);
                let end = message
                    .char_indices()
                    .nth(1024)
                    .map_or(message.len(), |(index, _)| index);
                report.diagnostics.push(message[..end].to_owned());
            }
        }
    }
    decoded
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

fn decode_jpeg(bytes: &[u8]) -> Result<(std::sync::Arc<egui::ColorImage>, u64), String> {
    use zune_jpeg::{JpegDecoder, zune_core::colorspace::ColorSpace};

    let options = zune_jpeg::zune_core::options::DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_use_unsafe(false);
    let mut decoder = JpegDecoder::new_with_options(bytes, options);
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
    pub fn click(&mut self, click: ClickRequest) -> Option<PageUpdate> {
        let session = self.session.as_mut()?;
        let allowed = session.click(click.target);
        let (page, reading, base) = session.with_document(|document| {
            let sheet = Stylesheet::from_document_with_sources(document, &self.styles);
            let reading = Page::reading(document, true, &sheet, &self.images);
            (
                Page::with_stylesheet_and_images(document, true, sheet, &self.images),
                reading,
                self.loaded.location.document_base(document),
            )
        });
        Some(PageUpdate {
            page,
            reading,
            base,
            scripts: session.report().clone(),
            alert: session.take_alerts().into_iter().next(),
            link: if allowed { click.href } else { None },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_png_images_with_bounded_dimensions() {
        let (image, pixels) =
            decode_image(include_bytes!("../../assets/olive-browser.png")).unwrap();
        assert_eq!(image.size, [1024, 1024]);
        assert_eq!(pixels, 1024 * 1024);
        assert!(decode_image(b"not an image").is_err());
    }
}
