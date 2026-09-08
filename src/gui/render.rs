//! Inert CSS presentation of Olive's DOM. No HTML reparsing or resource loading.
use eframe::egui::epaint::text::VariationCoords;
use eframe::egui::{
    self, Color32, FontFamily, FontId, Stroke,
    text::{LayoutJob, TextFormat},
};
use olive_html::{
    Document, NodeId, NodeKind,
    css::{Color, ComputedStyle, Display, Length, StyleBudget, Stylesheet, TextAlign, WhiteSpace},
};
use std::{collections::HashMap, ops::Range, sync::Arc};

pub const INK: Color32 = Color32::from_rgb(38, 44, 32);
pub const OLIVE: Color32 = Color32::from_rgb(86, 105, 51);
const MAX_TEXT: usize = 200_000;
const MAX_BLOCKS: usize = 5_000;
const MAX_BOXES: usize = 10_000;

fn color(c: Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.0, c.1, c.2, c.3)
}
#[derive(Clone, Copy, Default)]
struct Style {
    css: ComputedStyle,
    indent: usize,
    inline_background: Color32,
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
            color: color(css.color),
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
            background: self.inline_background,
            ..Default::default()
        }
    }
}
#[derive(Default)]
struct Block {
    job: LayoutJob,
    indent: usize,
    rule: bool,
    style: ComputedStyle,
    layout: Option<(f32, f32, Arc<egui::Galley>)>,
    actions: Vec<(Range<usize>, Action)>,
    characters: usize,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Action {
    target: Option<NodeId>,
    link: Option<NodeId>,
}
pub struct PageClick {
    pub target: Option<NodeId>,
    pub href: Option<String>,
}
#[derive(Clone, Copy)]
enum Command {
    Open(usize),
    Close,
    Text(usize),
    Anchor(usize),
}
#[derive(Default)]
pub struct Page {
    pub title: String,
    pub truncated: bool,
    pub css_limited: bool,
    pub css_ignored: usize,
    pub background: Option<Color32>,
    blocks: Vec<Block>,
    boxes: Vec<ComputedStyle>,
    commands: Vec<Command>,
    box_rects: Vec<egui::Rect>,
    root_font: f32,
    links: HashMap<NodeId, String>,
    anchors: Vec<String>,
    scroll_to: Option<String>,
}
struct Builder {
    page: Page,
    current: Block,
    pending_space: bool,
    characters: usize,
    open_boxes: usize,
}
enum Visit {
    Enter(NodeId, Style),
    Exit { block: bool, list: bool },
}
impl Page {
    pub fn from_document(doc: &Document) -> Self {
        Self::with_scripts(doc, true)
    }
    pub fn with_scripts(doc: &Document, scripting_enabled: bool) -> Self {
        let sheet = Stylesheet::from_document(doc);
        let mut budget = StyleBudget::default();
        let mut builder = Builder {
            page: Self {
                root_font: 17.0,
                ..Self::default()
            },
            current: Block::default(),
            pending_space: false,
            characters: 0,
            open_boxes: 0,
        };
        if let Some(title) = doc.descendants(doc.root()).find(|&id| {
            doc.node(id).and_then(|n| n.as_element()).is_some_and(|e| {
                e.name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                    && e.name.local.as_ref() == "title"
            })
        }) {
            builder.page.title = doc
                .descendants(title)
                .filter_map(|id| match &doc.node(id)?.kind {
                    NodeKind::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .flat_map(str::chars)
                .filter(|c| !c.is_control())
                .take(160)
                .collect();
        }
        let mut pending = vec![Visit::Enter(doc.root(), Style::default())];
        let mut lists: Vec<Option<i64>> = Vec::new();
        while let Some(visit) = pending.pop() {
            if builder.page.truncated {
                break;
            }
            let (id, mut style) = match visit {
                Visit::Enter(id, style) => (id, style),
                Visit::Exit { block, list } => {
                    if block {
                        builder.close();
                    }
                    if list {
                        lists.pop();
                    }
                    continue;
                }
            };
            let Some(node) = doc.node(id) else {
                continue;
            };
            match &node.kind {
                NodeKind::Text(text) => {
                    builder.text(text, style);
                    continue;
                }
                NodeKind::Element(element) => {
                    let tag = element.name.local.as_ref();
                    if element.name.ns.as_ref() != "http://www.w3.org/1999/xhtml"
                        || matches!(
                            tag,
                            "head"
                                | "script"
                                | "style"
                                | "template"
                                | "title"
                                | "iframe"
                                | "object"
                                | "embed"
                        )
                        || (tag == "noscript" && scripting_enabled)
                    {
                        continue;
                    }
                    style.css =
                        sheet.compute(doc, id, style.css, builder.page.root_font, &mut budget);
                    if scripting_enabled && element.attribute("onclick").is_some() {
                        style.target = Some(id);
                    }
                    if tag == "a" {
                        if let Some(href) = element.attribute("href") {
                            style.link = Some(id);
                            builder.page.links.insert(id, href.to_owned());
                        }
                    }
                    if style.css.display == Display::None {
                        continue;
                    }
                    if tag == "html" {
                        builder.page.root_font = style.css.font_size;
                    }
                    if (tag == "html" || (tag == "body" && builder.page.background.is_none()))
                        && style.css.background.3 > 0
                    {
                        builder.page.background = Some(color(style.css.background));
                    }
                    let block = style.css.display == Display::Block;
                    if block {
                        builder.open(style.css);
                        style.inline_background = Color32::TRANSPARENT;
                    } else if style.css.background.3 > 0 {
                        style.inline_background = color(style.css.background);
                    }
                    if let Some(anchor) = element
                        .attribute("id")
                        .or_else(|| (tag == "a").then(|| element.attribute("name")).flatten())
                    {
                        builder
                            .page
                            .commands
                            .push(Command::Anchor(builder.page.anchors.len()));
                        builder.page.anchors.push(anchor.to_owned());
                    }
                    if builder.page.truncated {
                        break;
                    }
                    if matches!(tag, "td" | "th") {
                        builder.text(" ", style);
                    }
                    let list = matches!(tag, "ul" | "ol");
                    match tag {
                        "blockquote" | "dd" => style.indent += 1,
                        "ul" => lists.push(None),
                        "ol" => lists.push(Some(
                            element
                                .attribute("start")
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(1),
                        )),
                        "li" => {
                            style.indent += usize::from(lists.len() > 1);
                            let marker = if let Some(Some(next)) = lists.last_mut() {
                                *next = element
                                    .attribute("value")
                                    .and_then(|v| v.parse().ok())
                                    .unwrap_or(*next);
                                let marker = format!("{next}. ");
                                *next = next.saturating_add(1);
                                marker
                            } else {
                                "• ".to_owned()
                            };
                            builder.literal(&marker, style);
                        }
                        "br" => builder.literal("\n", style),
                        "hr" => {
                            builder.flush();
                            builder.push(Block {
                                rule: true,
                                style: style.css,
                                ..Default::default()
                            });
                        }
                        "img" => {
                            if let Some(alt) =
                                element.attribute("alt").filter(|alt| !alt.is_empty())
                            {
                                builder.text(&format!("[Image: {alt}]"), style);
                            }
                        }
                        _ => {}
                    }
                    if matches!(tag, "br" | "hr" | "img" | "input") {
                        if block {
                            builder.close();
                        }
                        continue;
                    }
                    pending.push(Visit::Exit { block, list });
                }
                NodeKind::Document | NodeKind::DocumentFragment => {}
                _ => continue,
            }
            let children: Vec<_> = doc.children(id).collect();
            pending.extend(
                children
                    .into_iter()
                    .rev()
                    .map(|child| Visit::Enter(child, style)),
            );
        }
        builder.flush();
        // A display limit may stop inside a box. Balance the paint commands so
        // ancestor backgrounds still contain the displayed prefix.
        while builder.open_boxes > 0 {
            builder.close();
        }
        builder.page.css_limited = sheet.diagnostics.limited || budget.limited;
        builder.page.css_ignored = sheet.diagnostics.ignored + budget.ignored;
        builder.page
    }
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
            && !self
                .boxes
                .iter()
                .any(|s| s.background.3 > 0 || s.border_solid || s.height != Length::Auto)
    }
    pub fn scroll_to_fragment(&mut self, fragment: Option<String>) {
        self.scroll_to = Some(fragment.unwrap_or_default());
    }
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<PageClick> {
        let mut clicked = None;
        let mut scroll_to = self.scroll_to.take();
        let start = ui.cursor().min;
        if scroll_to.as_deref() == Some("") || scroll_to.as_deref() == Some("top") {
            ui.scroll_to_rect(
                egui::Rect::from_min_size(start, egui::vec2(1.0, 1.0)),
                Some(egui::Align::Min),
            );
            if scroll_to.as_deref() == Some("") {
                scroll_to = None;
            }
        }
        let width = ui.available_width().max(1.0);
        let mut stack = vec![BoxLayout::root(start, width)];
        self.box_rects.resize(self.boxes.len(), egui::Rect::NOTHING);
        let mut right = start.x + width;
        // This UI shares the page's painter and clip, but text placement must not
        // advance the caller's layout until the complete page size is known.
        let mut text_ui = ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()));
        for command in &self.commands {
            match *command {
                Command::Anchor(index) => {
                    if scroll_to.as_deref() == Some(self.anchors[index].as_str()) {
                        let top = egui::pos2(start.x, stack.last().unwrap().cursor);
                        ui.scroll_to_rect(
                            egui::Rect::from_min_size(top, egui::vec2(1.0, 1.0)),
                            Some(egui::Align::Min),
                        );
                        scroll_to = None;
                    }
                }
                Command::Open(index) => {
                    let layout = BoxLayout::open(
                        index,
                        self.boxes[index],
                        stack.last().unwrap(),
                        self.root_font,
                        ui,
                    );
                    right = right.max(layout.left + layout.outer_width);
                    stack.push(layout);
                }
                Command::Close => {
                    let layout = stack.pop().unwrap();
                    let rect = layout.finish(ui, self.root_font);
                    self.box_rects[layout.index] = rect;
                    stack.last_mut().unwrap().cursor = rect.bottom() + layout.margin_bottom;
                }
                Command::Text(index) => {
                    let parent = stack.last_mut().unwrap();
                    let block = &mut self.blocks[index];
                    let indent = (block.indent.min(12) as f32 * 22.0).min(parent.width * 0.4);
                    let width = (parent.width - indent).max(1.0);
                    let x = parent.content_left + indent;
                    if block.rule {
                        ui.painter().hline(
                            x..=x + width,
                            parent.cursor + 1.0,
                            Stroke::new(1.0, color(block.style.color)),
                        );
                        parent.cursor += 2.0;
                        continue;
                    }
                    let scale = ui.ctx().pixels_per_point();
                    if block
                        .layout
                        .as_ref()
                        .is_none_or(|(previous, previous_scale, _)| {
                            (previous - width).abs() > 0.5 || *previous_scale != scale
                        })
                    {
                        let mut job = block.job.clone();
                        job.wrap.max_width = if matches!(
                            block.style.white_space,
                            WhiteSpace::Pre | WhiteSpace::NoWrap
                        ) {
                            f32::INFINITY
                        } else {
                            width
                        };
                        block.layout =
                            Some((width, scale, ui.fonts_mut(|fonts| fonts.layout_job(job))));
                    }
                    let galley = block.layout.as_ref().unwrap().2.clone();
                    let offset = match block.style.text_align {
                        TextAlign::Left => 0.0,
                        TextAlign::Center => (width - galley.size().x).max(0.0) / 2.0,
                        TextAlign::Right => (width - galley.size().x).max(0.0),
                    };
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(x + offset, parent.cursor),
                        galley.size(),
                    );
                    let response = text_ui.put(
                        rect,
                        egui::Label::new(galley.clone()).selectable(true).sense(
                            if !block.actions.is_empty() {
                                egui::Sense::click()
                            } else {
                                egui::Sense::hover()
                            },
                        ),
                    );
                    if response.hovered() {
                        if let Some(action) = ui
                            .ctx()
                            .pointer_hover_pos()
                            .and_then(|pos| action_at(block, &galley, pos - rect.min))
                        {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            let href = action.link.and_then(|id| self.links.get(&id)).cloned();
                            if response.clicked() {
                                clicked = Some(PageClick {
                                    target: action.target,
                                    href: href.clone(),
                                });
                            }
                            if let Some(href) = href {
                                response.on_hover_text(href);
                            }
                        }
                    }
                    parent.cursor += rect.height();
                    right = right.max(rect.right());
                }
            }
        }
        let bottom = stack[0].cursor.max(start.y);
        ui.allocate_space(egui::vec2(right - start.x, bottom - start.y));
        if self.truncated || self.css_limited {
            ui.add_space(12.0);
            ui.separator();
            ui.label(if self.truncated {
                "This preview has reached its display limit. The rest of the file is not shown."
            } else {
                "This document reached the CSS processing limit. Some styles are not shown."
            });
        }
        clicked
    }
}

// Hit-test actual glyphs, so adjacent links and surrounding text retain their
// individual actions even when they share a wrapped, selectable paragraph.
fn action_at(block: &Block, galley: &egui::Galley, position: egui::Vec2) -> Option<Action> {
    let mut offset = 0;
    for row in &galley.rows {
        if row.rect().contains(position.to_pos2()) {
            let column = row.glyphs.iter().position(|glyph| {
                position.x >= row.pos.x + glyph.pos.x && position.x < row.pos.x + glyph.max_x()
            })?;
            return block
                .actions
                .iter()
                .find(|(range, _)| range.contains(&(offset + column)))
                .map(|(_, action)| *action);
        }
        offset += row.glyphs.len() + usize::from(row.ends_with_newline);
    }
    None
}

struct BoxLayout {
    index: usize,
    style: ComputedStyle,
    left: f32,
    top: f32,
    outer_width: f32,
    content_left: f32,
    width: f32,
    cursor: f32,
    content_top: f32,
    padding_bottom: f32,
    border: f32,
    margin_bottom: f32,
    paint: Option<egui::layers::ShapeIdx>,
}
impl BoxLayout {
    fn root(start: egui::Pos2, width: f32) -> Self {
        Self {
            index: 0,
            style: ComputedStyle::default(),
            left: start.x,
            top: start.y,
            outer_width: width,
            content_left: start.x,
            width,
            cursor: start.y,
            content_top: start.y,
            padding_bottom: 0.0,
            border: 0.0,
            margin_bottom: 0.0,
            paint: None,
        }
    }
    fn open(index: usize, style: ComputedStyle, parent: &Self, root: f32, ui: &egui::Ui) -> Self {
        let resolve = |v: Length| v.resolve(parent.width, style.font_size, root);
        let margin = style.margin.map(resolve);
        let padding = style.padding.map(resolve);
        let border = if style.border_solid {
            resolve(style.border_width)
        } else {
            0.0
        };
        let sides = padding[1] + padding[3] + border * 2.0;
        let available = (parent.width - margin[1] - margin[3] - sides).max(1.0);
        let width = if style.width == Length::Auto {
            available
        } else {
            resolve(style.width).max(1.0)
        };
        let width = if style.max_width == Length::Auto {
            width
        } else {
            width.min(resolve(style.max_width).max(1.0))
        };
        let spare = (available - width).max(0.0);
        let left_auto = style.margin[3] == Length::Auto;
        let right_auto = style.margin[1] == Length::Auto;
        let left_margin = margin[3]
            + if left_auto {
                spare / if right_auto { 2.0 } else { 1.0 }
            } else {
                0.0
            };
        let left = parent.content_left + left_margin;
        let top = parent.cursor + margin[0];
        let content_top = top + border + padding[0];
        Self {
            index,
            style,
            left,
            top,
            outer_width: width + sides,
            content_left: left + border + padding[3],
            width,
            cursor: content_top,
            content_top,
            padding_bottom: padding[2],
            border,
            margin_bottom: margin[2],
            paint: Some(ui.painter().add(egui::Shape::Noop)),
        }
    }
    fn finish(&self, ui: &egui::Ui, root: f32) -> egui::Rect {
        let min_height = self.style.height.resolve(0.0, self.style.font_size, root);
        let bottom =
            self.cursor.max(self.content_top + min_height) + self.padding_bottom + self.border;
        let rect = egui::Rect::from_min_max(
            egui::pos2(self.left, self.top),
            egui::pos2(self.left + self.outer_width, bottom),
        );
        if let Some(paint) = self.paint {
            let radius = self
                .style
                .border_radius
                .resolve(0.0, self.style.font_size, root)
                .clamp(0.0, 255.0) as u8;
            ui.painter().set(
                paint,
                egui::epaint::RectShape::new(
                    rect,
                    radius,
                    color(self.style.background),
                    Stroke::new(self.border, color(self.style.border_color)),
                    egui::StrokeKind::Inside,
                ),
            );
        }
        rect
    }
}
impl Builder {
    fn open(&mut self, style: ComputedStyle) {
        self.flush();
        if self.page.truncated || self.page.boxes.len() == MAX_BOXES {
            self.page.truncated = true;
            return;
        }
        self.page
            .commands
            .push(Command::Open(self.page.boxes.len()));
        self.page.boxes.push(style);
        self.open_boxes += 1;
    }
    fn close(&mut self) {
        self.flush();
        self.page.commands.push(Command::Close);
        self.open_boxes -= 1;
    }
    fn push(&mut self, block: Block) {
        if self.page.blocks.len() < MAX_BLOCKS {
            self.page
                .commands
                .push(Command::Text(self.page.blocks.len()));
            self.page.blocks.push(block);
        } else {
            self.page.truncated = true;
        }
    }
    fn flush(&mut self) {
        if !self.current.job.text.is_empty() {
            let block = std::mem::take(&mut self.current);
            self.push(block);
        }
        self.pending_space = false;
        if self.page.blocks.len() >= MAX_BLOCKS {
            self.page.truncated = true;
        }
    }
    fn literal(&mut self, text: &str, style: Style) {
        let remaining = MAX_TEXT.saturating_sub(self.characters);
        let accepted: String = text.chars().take(remaining).collect();
        self.characters += accepted.chars().count();
        if accepted.len() < text.len() {
            self.page.truncated = true;
        }
        if self.current.job.text.is_empty() {
            self.current.style = style.css;
            self.current.job.halign = match style.css.text_align {
                TextAlign::Left => egui::Align::Min,
                TextAlign::Center => egui::Align::Center,
                TextAlign::Right => egui::Align::Max,
            };
        }
        self.current.indent = style.indent;
        let characters = accepted.chars().count();
        let action = Action {
            target: style.target,
            link: style.link,
        };
        if action != Action::default() {
            self.current.actions.push((
                self.current.characters..self.current.characters + characters,
                action,
            ));
        }
        self.current.characters += characters;
        self.current.job.append(&accepted, 0.0, style.format());
        self.pending_space = false;
    }
    fn text(&mut self, text: &str, style: Style) {
        if matches!(style.css.white_space, WhiteSpace::Pre | WhiteSpace::PreWrap) {
            self.literal(text, style);
            return;
        }
        let mut normalized = String::new();
        let remaining = MAX_TEXT.saturating_sub(self.characters);
        let mut count = 0;
        for c in text.chars() {
            if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}') {
                self.pending_space = true;
            } else {
                if count > remaining {
                    self.page.truncated = true;
                    break;
                }
                let previous = normalized
                    .chars()
                    .next_back()
                    .or_else(|| self.current.job.text.chars().next_back());
                if self.pending_space && previous.is_some_and(|c| c != ' ' && c != '\n') {
                    normalized.push(' ');
                    count += 1;
                }
                self.pending_space = false;
                normalized.push(c);
                count += 1;
            }
        }
        let trailing_space = self.pending_space;
        if !normalized.is_empty() {
            self.literal(&normalized, style);
        }
        self.pending_space = trailing_space;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use olive_html::parse;

    fn page(html: &str) -> Page {
        Page::from_document(&parse(html).unwrap().document)
    }
    fn texts(page: &Page) -> Vec<&str> {
        page.blocks.iter().map(|b| b.job.text.as_str()).collect()
    }

    #[test]
    fn links_have_independent_hit_targets_inside_wrapped_unicode_text() {
        let mut page = page(
            "<p>Plain café <a href='/one'>first <strong>link</strong></a> between <a href='/two'>second link wraps across lines</a> end</p>",
        );
        let mut output = draw(&mut page, 180.0);
        let block = &page.blocks[0];
        let galley = &block.layout.as_ref().unwrap().2;
        assert!(galley.rows.len() > 1);
        let text = block.job.text.chars().collect::<Vec<_>>();
        let mut offset = 0;
        for row in &galley.rows {
            for (column, glyph) in row.glyphs.iter().enumerate() {
                let position = egui::vec2(
                    row.pos.x + glyph.pos.x + glyph.advance_width / 2.0,
                    row.rect().center().y,
                );
                let action = action_at(block, galley, position);
                let prefix: String = text[..offset + column].iter().collect();
                let expected = if prefix.len() >= block.job.text.find("first").unwrap()
                    && prefix.len() < block.job.text.find(" between").unwrap()
                {
                    Some("/one")
                } else if prefix.len() >= block.job.text.find("second").unwrap()
                    && prefix.len() < block.job.text.find(" end").unwrap()
                {
                    Some("/two")
                } else {
                    None
                };
                // Collapsed separator whitespace may inherit either adjacent style.
                if !glyph.chr.is_whitespace() {
                    assert_eq!(
                        action
                            .and_then(|a| a.link)
                            .and_then(|id| page.links.get(&id))
                            .map(String::as_str),
                        expected,
                        "{prefix}"
                    );
                }
            }
            offset += row.glyphs.len() + usize::from(row.ends_with_newline);
        }
        assert!(action_at(block, galley, egui::vec2(-10.0, -10.0)).is_none());
        output.textures_delta.clear();
    }

    #[test]
    fn remote_noscript_is_visible_and_onclick_is_inert() {
        let document = parse("<body><noscript><p>Fallback</p></noscript><p onclick='alert(1)'>Inert</p><a href='/next' onclick='alert(2)'>Next</a><h2 id='café'>Anchor</h2><a name='legacy'></a>").unwrap().document;
        let page = Page::with_scripts(&document, false);
        assert!(texts(&page).contains(&"Fallback"));
        assert!(
            page.blocks
                .iter()
                .flat_map(|b| &b.actions)
                .all(|(_, action)| action.target.is_none())
        );
        assert_eq!(page.links.len(), 1);
        assert_eq!(page.anchors, ["café", "legacy"]);
    }

    #[test]
    fn javascript_mutations_feed_text_and_css_rendering() {
        let doc = parse("<!doctype html><style>.done {color:red}</style><p id=p>before</p><noscript>fallback</noscript><script>const p=document.getElementById('p'); p.textContent='after'; p.className='done'; document.title='Changed';</script>").unwrap().document;
        let (doc, report) = olive_html::js::run_document(doc, Default::default());
        assert!(report.diagnostics.is_empty());
        let page = Page::from_document(&doc);
        assert_eq!(texts(&page), ["after"]);
        assert_eq!(page.title, "Changed");
        assert_eq!(page.blocks[0].job.sections[0].format.color, Color32::RED);
    }

    #[test]
    fn renders_dom_text_with_inline_styles_and_html_whitespace() {
        let page = page(
            "<!doctype html><title>Title</title><h1>Heading</h1><p>A <strong>bold</strong> and <em>italic</em>&nbsp;x<br>end</p><pre>  a\n b</pre>",
        );
        assert_eq!(page.title, "Title");
        assert_eq!(
            texts(&page),
            ["Heading", "A bold and italic\u{a0}x\nend", "  a\n b"]
        );
        assert_eq!(page.blocks[0].job.sections[0].format.font_id.size, 34.0);
        let sections = &page.blocks[1].job.sections;
        assert!(sections.iter().any(|s| s.format.italics));
        assert!(
            sections
                .iter()
                .any(|s| s.format.coords == VariationCoords::new([("wght", 700.0)]))
        );
    }

    #[test]
    fn escaped_markup_stays_text_while_real_elements_are_styled() {
        let page = page(
            "<!doctype html><p>&lt;h1&gt;Literal&lt;/h1&gt; &amp;lt;em&amp;gt;</p><h1>Heading</h1>",
        );
        assert_eq!(texts(&page), ["<h1>Literal</h1> &lt;em&gt;", "Heading"]);
        assert_eq!(page.blocks[0].job.sections[0].format.font_id.size, 17.0);
        assert_eq!(page.blocks[1].job.sections[0].format.font_id.size, 34.0);
    }

    #[test]
    fn skips_inert_and_hidden_subtrees_and_keeps_image_alt_text() {
        let page = page(
            "<!doctype html><style>hidden css</style><body><script>hidden js</script><template>hidden template</template><div hidden><p>secret</p></div><p>visible <img src='file:///secret' alt='an olive'></p><iframe src='https://example.com'>frame</iframe>",
        );
        assert_eq!(texts(&page), ["visible [Image: an olive]"]);
    }

    #[test]
    fn lists_keep_numbering_and_nested_order() {
        let page = page(
            "<!doctype html><ol start=3><li>  three<ul><li> nested<ul><li>deeper</li></ul></li></ul></li><li value=8>eight</li><li>nine</li></ol>",
        );
        assert_eq!(
            texts(&page),
            ["3. three", "• nested", "• deeper", "8. eight", "9. nine"]
        );
        assert_eq!(
            page.blocks
                .iter()
                .map(|block| block.indent)
                .collect::<Vec<_>>(),
            [0, 1, 2, 0, 0]
        );
    }

    #[test]
    fn reflows_on_resize_and_reuses_layout_when_width_is_unchanged() {
        let mut page = page(&format!(
            "<p>{}",
            "A paragraph that wraps at the window edge. ".repeat(12)
        ));
        let ctx = egui::Context::default();
        let mut layout = |width| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 600.0),
                )),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                page.show(ui);
            });
            output.textures_delta.clear();
            page.blocks[0].layout.as_ref().unwrap().2.clone()
        };
        let wide = layout(800.0);
        let narrow = layout(300.0);
        assert!(narrow.size().y > wide.size().y);
        assert!(Arc::ptr_eq(&narrow, &layout(300.0)));
    }

    #[test]
    fn preview_limits_and_deep_trees_are_bounded() {
        let text = page(&format!("<p>{}", "🫒".repeat(MAX_TEXT + 1)));
        assert!(text.truncated);
        assert_eq!(text.blocks[0].job.text.chars().count(), MAX_TEXT);
        let blocks = page(&"<p>x".repeat(MAX_BLOCKS + 1));
        assert!(blocks.truncated);
        assert_eq!(blocks.blocks.len(), MAX_BLOCKS);
        let deep = page(&format!("{}end", "<div>".repeat(8_000)));
        assert_eq!(texts(&deep), ["end"]);
    }

    fn draw(page: &mut Page, width: f32) -> egui::FullOutput {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::fonts::definitions());
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 900.0),
                )),
                ..Default::default()
            },
            |ui| {
                page.show(ui);
            },
        )
    }

    #[test]
    fn css_styles_text_hides_subtrees_and_changes_block_flow() {
        let page = page(
            r#"<!doctype html><style>
            .gone { display:none } span.block {display:block; color:#f00}
            p {display:inline; margin:0} strong {font-weight:400; color:blue}
            </style><div class=gone>secret<p>hidden</p></div>
            <p>A <strong>normal</strong></p><span class=block>B</span><p>C</p>"#,
        );
        assert_eq!(texts(&page), ["A normal", "B", "C"]);
        let normal = &page.blocks[0].job.sections[1].format;
        assert_eq!(normal.color, Color32::BLUE);
        assert_eq!(normal.coords, VariationCoords::new([("wght", 400.0)]));
        assert_eq!(page.blocks[1].job.sections[0].format.color, Color32::RED);
    }

    #[test]
    fn nested_box_geometry_accounts_for_padding_borders_and_auto_margins() {
        let mut page = page(
            r#"<!doctype html><body><div style="width:200px; margin:0 auto; padding:10px; border:2px solid red; background:#def">
            <p style="margin:0; padding:5px; background:blue">child</p></div>"#,
        );
        let mut output = draw(&mut page, 800.0);
        let parent = page
            .boxes
            .iter()
            .position(|s| s.width == Length::Px(200.0))
            .unwrap();
        let child = page
            .boxes
            .iter()
            .position(|s| s.background == Color(0, 0, 255, 255))
            .unwrap();
        let a = page.box_rects[parent];
        let b = page.box_rects[child];
        assert_eq!(a.width(), 224.0);
        assert_eq!(b.width(), 200.0);
        assert_eq!(b.left() - a.left(), 12.0);
        assert_eq!(b.top() - a.top(), 12.0);
        assert_eq!(a.bottom() - b.bottom(), 12.0);
        let body = page.box_rects[1];
        assert!((a.center().x - body.center().x).abs() < 0.1);
        let fills: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(rect) => Some(rect.fill),
                _ => None,
            })
            .collect();
        let outer_fill = fills
            .iter()
            .position(|c| *c == Color32::from_rgb(221, 238, 255))
            .unwrap();
        let inner_fill = fills.iter().position(|c| *c == Color32::BLUE).unwrap();
        assert!(
            outer_fill < inner_fill,
            "parent background must paint behind children"
        );
        output.textures_delta.clear();
    }

    #[test]
    fn whitespace_modes_wrap_and_preserve_as_requested() {
        let text = "A long line with words ".repeat(10);
        let mut page = page(&format!(
            "<p style='white-space:pre-wrap'>  {text}\n  end</p><p style='white-space:nowrap'>{text}</p>"
        ));
        let mut output = draw(&mut page, 260.0);
        assert!(page.blocks[0].job.text.starts_with("  "));
        assert!(page.blocks[0].job.text.contains("\n  end"));
        let wrap = &page.blocks[0].layout.as_ref().unwrap().2;
        let nowrap = &page.blocks[1].layout.as_ref().unwrap().2;
        assert!(wrap.rows.len() > 2);
        assert_eq!(nowrap.rows.len(), 1);
        assert!(nowrap.size().x > 260.0);
        output.textures_delta.clear();
    }

    #[test]
    fn empty_boxes_are_visible_and_limits_leave_balanced_commands() {
        let mut empty = page("<div style='height:40px; background:red; padding:5px'></div>");
        assert!(!empty.is_empty());
        let mut output = draw(&mut empty, 400.0);
        assert_eq!(empty.box_rects.last().unwrap().height(), 50.0);
        output.textures_delta.clear();
        let deep = page(&format!("{}end", "<div>".repeat(MAX_BOXES + 1)));
        assert!(deep.truncated);
        assert_eq!(deep.boxes.len(), MAX_BOXES);
        assert_eq!(
            deep.commands
                .iter()
                .filter(|c| matches!(c, Command::Open(_)))
                .count(),
            deep.commands
                .iter()
                .filter(|c| matches!(c, Command::Close))
                .count()
        );
    }
}
