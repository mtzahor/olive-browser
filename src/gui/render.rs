//! Inert CSS presentation of Olive's DOM. No HTML reparsing or resource loading.
use crate::gui::focus::{self, Content, Settings};
use crate::gui::theme::Theme;
use crate::gui::{
    find::Matches,
    forms::{Activation, Forms, Kind},
};
use crate::{
    Document, NodeId, NodeKind,
    css::Direction,
    css::{
        Clear, Color, ComputedStyle, Display, FlexWrap, FloatSide, JustifyContent, Length,
        Position, StyleBudget, Stylesheet, TextAlign, TextTransform, VerticalAlign, WhiteSpace,
    },
};
use eframe::egui::epaint::text::VariationCoords;
use eframe::egui::{
    self, Color32, ColorImage, FontFamily, FontId, Role, Stroke, TextureHandle, TextureOptions,
    WidgetInfo,
    text::{LayoutJob, TextFormat},
};
use std::{collections::HashMap, ops::Range, sync::Arc};

mod bidi;
mod build;
mod layout;
mod paint;
mod stacking;
#[cfg(test)]
mod tests;

const MAX_TEXT: usize = 200_000;
const MAX_BLOCKS: usize = 5_000;
const MAX_BOXES: usize = 10_000;
const MAX_IMAGES: usize = 2_048;
const MAX_LAYOUT_MEASURE_STEPS: usize = 250_000;

fn color(c: Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.0, c.1, c.2, c.3)
}
fn paint_color(c: Color, opacity: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.0, c.1, c.2, (c.3 as f32 * opacity).round() as u8)
}
#[derive(Clone, Copy, Default)]
struct Style {
    css: ComputedStyle,
    inline_background: Color32,
    flex_parent: bool,
    target: Option<NodeId>,
    link: Option<NodeId>,
}
impl Style {
    fn format(self) -> TextFormat {
        let css = self.css;
        TextFormat {
            font_id: FontId::new(
                css.font_size,
                if css.monospace {
                    FontFamily::Monospace
                } else {
                    FontFamily::Proportional
                },
            ),
            coords: VariationCoords::new([("wght", css.font_weight)]),
            color: paint_color(css.color, css.opacity),
            italics: css.italic,
            underline: if css.underline {
                Stroke::new(1.0, color(css.color))
            } else {
                Stroke::NONE
            },
            strikethrough: if css.strike {
                Stroke::new(1.0, color(css.color))
            } else {
                Stroke::NONE
            },
            line_height: Some(css.line_height_px()),
            background: self.inline_background.gamma_multiply(css.opacity),
            valign: match css.vertical_align {
                VerticalAlign::Super => egui::Align::TOP,
                VerticalAlign::Sub | VerticalAlign::Baseline => egui::Align::BOTTOM,
            },
            ..Default::default()
        }
    }
}
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Block {
    #[serde(with = "wire_text")]
    job: LayoutJob,
    #[serde(skip)]
    visual_job: LayoutJob,
    rule: bool,
    /// A list marker, painted beside the item rather than in the text flow.
    #[serde(default)]
    marker: bool,
    style: ComputedStyle,
    #[serde(skip)]
    layout: Option<(f32, f32, Arc<egui::Galley>)>,
    actions: Vec<(Range<usize>, Action)>,
    #[serde(skip)]
    visual_actions: Vec<(Range<usize>, Action)>,
    #[serde(skip)]
    visual_to_logical: Vec<usize>,
    characters: usize,
}

/// A decoded image retained by the GUI presentation layer. The encoded bytes
/// stay in the resource collector so the parser and network layers do not need
/// to depend on a particular image decoder.
#[derive(Clone)]
pub struct ImageAsset {
    pub reference: String,
    pub image: Arc<ColorImage>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RenderedImage {
    node: NodeId,
    reference: String,
    #[serde(with = "crate::gui::worker::wire_image")]
    image: Arc<ColorImage>,
    alt: Option<String>,
    style: ComputedStyle,
    html_width: Option<f32>,
    html_height: Option<f32>,
    action: Action,
}

/// Textures of one page's pictures by address. A page fetches each address
/// once, so pictures sharing one also share a texture, in both views.
#[derive(Default)]
pub struct ImageTextureCache {
    textures: HashMap<String, TextureHandle>,
}

impl ImageTextureCache {
    pub fn clear(&mut self) {
        self.textures.clear();
    }

    fn texture(&mut self, ctx: &egui::Context, image: &RenderedImage) -> TextureHandle {
        if let Some(texture) = self.textures.get(&image.reference) {
            return texture.clone();
        }
        let texture = ctx.load_texture(
            format!("olive-image-{}", image.reference),
            image.image.clone(),
            TextureOptions::LINEAR,
        );
        self.textures
            .insert(image.reference.clone(), texture.clone());
        texture
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Action {
    target: Option<NodeId>,
    link: Option<NodeId>,
}
pub struct PageClick {
    pub form: Option<Activation>,
    pub new_tab: bool,
    pub target: Option<NodeId>,
    pub href: Option<String>,
}
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
enum Command {
    Open(usize),
    Close,
    Text(usize),
    Image(usize),
    Control(usize, Option<NodeId>),
    Anchor(usize),
}
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct Page {
    pub title: String,
    #[serde(default)]
    pub export_html: Option<String>,
    #[serde(default)]
    pub export_error: Option<String>,
    pub truncated: bool,
    pub css_limited: bool,
    pub css_ignored: usize,
    #[serde(default)]
    pub css_unsupported: Vec<(String, usize)>,
    pub background: Option<Color32>,
    pub forms: Forms,
    #[serde(skip)]
    pub find: Matches,
    #[serde(skip)]
    pub find_current: usize,
    #[serde(skip)]
    pub find_scroll: bool,
    blocks: Vec<Block>,
    boxes: Vec<ComputedStyle>,
    images: Vec<RenderedImage>,
    commands: Vec<Command>,
    #[serde(skip)]
    box_rects: Vec<egui::Rect>,
    /// Consecutive frames repainted to settle bottom-anchored boxes.
    #[serde(skip)]
    settle_frames: u8,
    root_font: f32,
    links: HashMap<NodeId, String>,
    anchors: Vec<String>,
    scroll_to: Option<String>,
    reading_style: Option<Settings>,
}
// Wrapping and layout caches belong to the local viewport. Transmit the text,
// runs and alignment only; egui's default infinite wrap width is not JSON data.
mod wire_text {
    use super::*;
    use serde::{Deserialize, Serialize};
    pub fn serialize<S: serde::Serializer>(
        job: &LayoutJob,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        (&job.text, &job.sections, job.halign).serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<LayoutJob, D::Error> {
        let (text, sections, halign) = Deserialize::deserialize(deserializer)?;
        Ok(LayoutJob {
            text,
            sections,
            halign,
            ..Default::default()
        })
    }
}

impl Page {
    /// The normal view of `doc`, styled for a CSS viewport of `viewport` pixels.
    pub fn new(
        doc: &Document,
        scripting_enabled: bool,
        sheet: &Stylesheet,
        images: &HashMap<NodeId, ImageAsset>,
        viewport: (f32, f32),
    ) -> Self {
        Self::build(doc, scripting_enabled, sheet, images, None, viewport)
    }

    /// The Focus reading view, or an empty page when `doc` has no readable text.
    /// With a `base` address the view also carries a standalone HTML export.
    pub fn reading(
        doc: &Document,
        scripting_enabled: bool,
        sheet: &Stylesheet,
        images: &HashMap<NodeId, ImageAsset>,
        base: Option<&crate::net::Location>,
        viewport: (f32, f32),
    ) -> Self {
        let content = Content::extract(doc, sheet, scripting_enabled);
        if !content.has_text {
            return Self::default();
        }
        let mut page = Self::build(
            doc,
            scripting_enabled,
            sheet,
            images,
            Some(&content),
            viewport,
        );
        if let Some(base) = base {
            match crate::gui::focus_export::export(doc, &content, &page.title, base, images) {
                Ok(html) => page.export_html = Some(html),
                Err(error) => page.export_error = Some(error),
            }
        }
        page
    }

    /// Reject malformed indices and excessive presentation work before the UI sees IPC data.
    pub fn validate(&self) -> Result<(), String> {
        self.forms.validate()?;
        if self
            .export_html
            .as_ref()
            .is_some_and(|html| html.len() > crate::gui::focus_export::MAX_EXPORT_BYTES)
            || self
                .export_error
                .as_ref()
                .is_some_and(|error| error.len() > 1024)
        {
            return Err("Invalid Focus export".into());
        }
        let invalid = || "Invalid tab presentation".to_owned();
        if self.blocks.len() > MAX_BLOCKS
            || self.boxes.len() > MAX_BOXES
            || self.images.len() > MAX_IMAGES
            || self.commands.len() > 100_000
            || self.anchors.len() > 100_000
            || self.css_unsupported.len() > 512
            || self
                .css_unsupported
                .iter()
                .any(|(name, count)| name.len() > 128 || *count == 0)
        {
            return Err(invalid());
        }
        let mut depth = 0usize;
        for command in &self.commands {
            match *command {
                Command::Open(index) if index < self.boxes.len() => depth += 1,
                Command::Close if depth > 0 => depth -= 1,
                Command::Text(index) if index < self.blocks.len() => {}
                Command::Image(index) if index < self.images.len() => {}
                Command::Control(index, _) if index < self.forms.controls.len() => {}
                Command::Anchor(index) if index < self.anchors.len() => {}
                _ => return Err(invalid()),
            }
        }
        if depth != 0 {
            return Err(invalid());
        }
        let mut characters = 0;
        for block in &self.blocks {
            let count = block.job.text.chars().count();
            characters += count;
            if characters > MAX_TEXT
                || block.job.sections.len() > count
                || block.actions.len() > count
            {
                return Err(invalid());
            }
            let mut end = 0;
            for section in &block.job.sections {
                let range = section.byte_range.start.0..section.byte_range.end.0;
                if range.start != end
                    || range.is_empty()
                    || block.job.text.get(range.clone()).is_none()
                {
                    return Err(invalid());
                }
                end = range.end;
            }
            if end != block.job.text.len()
                || block
                    .actions
                    .iter()
                    .any(|(range, _)| range.start > range.end || range.end > count)
            {
                return Err(invalid());
            }
            if !block.visual_job.text.is_empty()
                || !block.visual_to_logical.is_empty()
                || !block.visual_actions.is_empty()
            {
                if block.visual_job.text.chars().count() != count
                    || block.visual_to_logical.len() != count
                    || block.visual_actions.len() > count
                    || block
                        .visual_to_logical
                        .iter()
                        .any(|&source| source >= count)
                    || block
                        .visual_actions
                        .iter()
                        .any(|(range, _)| range.start > range.end || range.end > count)
                {
                    return Err(invalid());
                }
                let mut visual_end = 0;
                for section in &block.visual_job.sections {
                    let range = section.byte_range.start.0..section.byte_range.end.0;
                    if range.start != visual_end
                        || range.is_empty()
                        || block.visual_job.text.get(range.clone()).is_none()
                    {
                        return Err(invalid());
                    }
                    visual_end = range.end;
                }
                if visual_end != block.visual_job.text.len() {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }

    /// The Focus export with its pictures embedded, when the view has one.
    pub fn export_document(&self) -> Option<Result<String, String>> {
        let html = self.export_html.as_deref()?;
        let pictures: HashMap<usize, &ColorImage> = self
            .images
            .iter()
            .map(|image| (image.node.index(), &*image.image))
            .collect();
        Some(crate::gui::focus_export::embed_pictures(html, |node| {
            pictures.get(&node).copied()
        }))
    }

    /// Keep the shaped lines of text that `previous` already showed, so an
    /// updated presentation of the same document appears without shaping its
    /// unchanged text again.
    pub fn reuse_text_layout(&mut self, previous: &mut Page) {
        use std::hash::BuildHasher;
        let hasher = std::collections::hash_map::RandomState::new();
        let mut shaped: HashMap<u64, Vec<usize>> = HashMap::new();
        for (index, block) in previous.blocks.iter().enumerate() {
            if block.layout.is_some() {
                shaped
                    .entry(hasher.hash_one(&block.job))
                    .or_default()
                    .push(index);
            }
        }
        if shaped.is_empty() {
            return;
        }
        for block in &mut self.blocks {
            let Some(candidates) = shaped.get_mut(&hasher.hash_one(&block.job)) else {
                continue;
            };
            // Bidi layout also reads the direction, alignment, wrapping and links.
            let Some(position) = candidates.iter().position(|&index| {
                let old = &previous.blocks[index];
                old.job == block.job
                    && old.actions == block.actions
                    && old.style.direction == block.style.direction
                    && old.style.text_align == block.style.text_align
                    && old.style.white_space == block.style.white_space
            }) else {
                continue;
            };
            let old = &mut previous.blocks[candidates.swap_remove(position)];
            block.layout = old.layout.take();
            block.visual_job = std::mem::take(&mut old.visual_job);
            block.visual_to_logical = std::mem::take(&mut old.visual_to_logical);
            block.visual_actions = std::mem::take(&mut old.visual_actions);
        }
    }

    /// Cached galleys refer to the font atlas, which egui rebuilds on theme changes.
    pub fn invalidate_text_layout(&mut self) {
        for block in &mut self.blocks {
            block.layout = None;
        }
    }

    /// Reflow the retained reader presentation without reloading or running scripts.
    pub fn set_reading_style(&mut self, settings: Settings) {
        let Some(previous) = self.reading_style else {
            return;
        };
        let settings = settings.normalized();
        if previous == settings {
            return;
        }
        let scale = settings.font_size / previous.font_size;
        let recolor = |value: Color| {
            if value == previous.theme.reading_link() {
                settings.theme.reading_link()
            } else if value == previous.theme.reading_code() {
                settings.theme.reading_code()
            } else if value.3 > 0 {
                settings.theme.reading_ink()
            } else {
                value
            }
        };
        let restyle = |style: &mut ComputedStyle| {
            style.font_size *= scale;
            style.color = recolor(style.color);
            style.background = recolor(style.background);
        };
        for block in &mut self.blocks {
            restyle(&mut block.style);
            for job in [&mut block.job, &mut block.visual_job] {
                for section in &mut job.sections {
                    let format = &mut section.format;
                    format.font_id.size *= scale;
                    if let Some(height) = &mut format.line_height {
                        *height *= scale;
                    }
                    let update_color =
                        |c: Color32| color(recolor(Color(c.r(), c.g(), c.b(), c.a())));
                    format.color = update_color(format.color);
                    format.background = update_color(format.background);
                    format.underline.color = update_color(format.underline.color);
                    format.strikethrough.color = update_color(format.strikethrough.color);
                }
            }
            block.layout = None;
        }
        for style in &mut self.boxes {
            restyle(style);
        }
        for image in &mut self.images {
            restyle(&mut image.style);
        }
        self.root_font = settings.font_size;
        self.reading_style = Some(settings);
    }
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
            && !self
                .commands
                .iter()
                .any(|c| matches!(c, Command::Control(..)))
            && self.images.is_empty()
            && !self
                .boxes
                .iter()
                .any(|s| s.background.3 > 0 || s.has_border() || s.height != Length::Auto)
    }
    pub fn image_count(&self) -> usize {
        self.images.len()
    }
    pub fn find_query(&mut self, query: &str) -> usize {
        // List markers are decoration, so search skips them as browsers do.
        self.find.update(
            query,
            self.blocks
                .iter()
                .map(|b| if b.marker { "" } else { b.job.text.as_str() }),
        );
        self.find.count
    }
    pub fn scroll_to_fragment(&mut self, fragment: Option<String>) {
        self.scroll_to = Some(fragment.unwrap_or_default());
    }
}
