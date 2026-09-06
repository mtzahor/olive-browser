//! Inert presentation of Olive's DOM. This module does not parse HTML or load URLs.
use eframe::egui::epaint::text::VariationCoords;
use eframe::egui::{
    self, Color32, FontFamily, FontId, Stroke,
    text::{LayoutJob, TextFormat},
};
use olive_html::{Document, NodeId, NodeKind};
use std::sync::Arc;

pub const INK: Color32 = Color32::from_rgb(38, 44, 32);
pub const OLIVE: Color32 = Color32::from_rgb(86, 105, 51);
const MAX_TEXT: usize = 200_000;
const MAX_BLOCKS: usize = 5_000;

#[derive(Clone, Copy)]
struct Style {
    size: f32,
    bold: bool,
    italic: bool,
    mono: bool,
    underline: bool,
    strike: bool,
    link: bool,
    pre: bool,
    indent: usize,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            size: 17.0,
            bold: false,
            italic: false,
            mono: false,
            underline: false,
            strike: false,
            link: false,
            pre: false,
            indent: 0,
        }
    }
}

impl Style {
    fn format(self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(
                self.size,
                if self.mono {
                    FontFamily::Monospace
                } else {
                    FontFamily::Proportional
                },
            ),
            coords: VariationCoords::new([("wght", if self.bold { 700.0 } else { 400.0 })]),
            color: if self.link { OLIVE } else { INK },
            italics: self.italic,
            underline: if self.underline || self.link {
                Stroke::new(1.0, OLIVE)
            } else {
                Stroke::NONE
            },
            strikethrough: if self.strike {
                Stroke::new(1.0, INK)
            } else {
                Stroke::NONE
            },
            line_height: Some(self.size * 1.45),
            background: if self.mono {
                Color32::from_rgb(239, 240, 233)
            } else {
                Color32::TRANSPARENT
            },
            ..Default::default()
        }
    }
}

#[derive(Default)]
struct Block {
    job: LayoutJob,
    indent: usize,
    rule: bool,
    // Reflow only when the available width changes. Repaints reuse the galley.
    layout: Option<(f32, f32, Arc<egui::Galley>)>,
}

#[derive(Default)]
pub struct Page {
    pub title: String,
    pub truncated: bool,
    blocks: Vec<Block>,
}

struct Builder {
    page: Page,
    current: Block,
    pending_space: bool,
    characters: usize,
}

enum Visit {
    Enter(NodeId, Style),
    Exit { block: bool, list: bool },
}

impl Page {
    pub fn from_document(doc: &Document) -> Self {
        let mut builder = Builder {
            page: Self::default(),
            current: Block::default(),
            pending_space: false,
            characters: 0,
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
                        builder.flush();
                    }
                    if list {
                        lists.pop();
                    }
                    continue;
                }
            };
            let Some(node) = doc.node(id) else { continue };
            match &node.kind {
                NodeKind::Text(text) => {
                    builder.text(text, style);
                    continue;
                }
                NodeKind::Element(element) => {
                    let tag = element.name.local.as_ref();
                    if element.attribute("hidden").is_some()
                        || element.name.ns.as_ref() != "http://www.w3.org/1999/xhtml"
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
                    {
                        continue;
                    }
                    let block = matches!(
                        tag,
                        "html"
                            | "body"
                            | "main"
                            | "article"
                            | "section"
                            | "header"
                            | "footer"
                            | "nav"
                            | "aside"
                            | "div"
                            | "p"
                            | "h1"
                            | "h2"
                            | "h3"
                            | "h4"
                            | "h5"
                            | "h6"
                            | "ul"
                            | "ol"
                            | "li"
                            | "pre"
                            | "blockquote"
                            | "hr"
                            | "table"
                            | "tr"
                            | "dl"
                            | "dt"
                            | "dd"
                            | "figure"
                            | "figcaption"
                            | "form"
                    );
                    if block {
                        builder.flush();
                    }
                    if builder.page.truncated {
                        break;
                    }
                    if matches!(tag, "td" | "th") {
                        builder.text(" ", style);
                    }
                    let list = matches!(tag, "ul" | "ol");
                    match tag {
                        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                            style.size = [34.0, 27.0, 23.0, 20.0, 18.0, 17.0]
                                [(tag.as_bytes()[1] - b'1') as usize];
                            style.bold = true;
                        }
                        "b" | "strong" | "th" | "dt" => style.bold = true,
                        "i" | "em" | "cite" | "dfn" => style.italic = true,
                        "u" => style.underline = true,
                        "s" | "del" => style.strike = true,
                        "a" => style.link = element.attribute("href").is_some(),
                        "small" => style.size = (style.size * 0.85).max(10.0),
                        "code" | "kbd" | "samp" => {
                            style.mono = true;
                            style.size = 15.0;
                        }
                        "pre" => {
                            style.pre = true;
                            style.mono = true;
                            style.size = 15.0;
                        }
                        "blockquote" | "dd" => style.indent += 1,
                        "ul" => lists.push(None),
                        "ol" => lists.push(Some(
                            element
                                .attribute("start")
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(1),
                        )),
                        "li" => {
                            // Inherit the parent item's indent, then add one level.
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
                        "br" => {
                            builder.literal("\n", style);
                            continue;
                        }
                        "hr" => {
                            builder.page.blocks.push(Block {
                                rule: true,
                                ..Default::default()
                            });
                            continue;
                        }
                        "img" => {
                            if let Some(alt) =
                                element.attribute("alt").filter(|alt| !alt.is_empty())
                            {
                                builder.text(&format!("[Image: {alt}]"), style);
                            }
                            continue;
                        }
                        "input" => continue,
                        _ => {}
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
        builder.page
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        for block in &mut self.blocks {
            if block.rule {
                ui.separator();
                ui.add_space(12.0);
                continue;
            }
            let indent = (block.indent.min(12) as f32 * 22.0).min(ui.available_width() * 0.4);
            let width = (ui.available_width() - indent).max(40.0);
            let scale = ui.ctx().pixels_per_point();
            if block
                .layout
                .as_ref()
                .is_none_or(|(previous, previous_scale, _)| {
                    (previous - width).abs() > 0.5 || *previous_scale != scale
                })
            {
                let mut job = block.job.clone();
                job.wrap.max_width = width;
                block.layout = Some((width, scale, ui.fonts_mut(|fonts| fonts.layout_job(job))));
            }
            let galley = block.layout.as_ref().unwrap().2.clone();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(indent);
                ui.add(egui::Label::new(galley).selectable(true));
            });
            ui.add_space(12.0);
        }
        if self.truncated {
            ui.separator();
            ui.label(
                "This preview has reached its display limit. The rest of the file is not shown.",
            );
        }
    }
}

impl Builder {
    fn flush(&mut self) {
        if !self.current.job.text.is_empty() {
            self.page.blocks.push(std::mem::take(&mut self.current));
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
        self.current.indent = style.indent;
        self.current.job.append(&accepted, 0.0, style.format());
        self.pending_space = false;
    }

    fn text(&mut self, text: &str, style: Style) {
        if style.pre {
            self.literal(text, style);
            return;
        }
        let mut normalized = String::new();
        for c in text.chars() {
            if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}') {
                self.pending_space = true;
            } else {
                let previous = normalized
                    .chars()
                    .next_back()
                    .or_else(|| self.current.job.text.chars().next_back());
                if self.pending_space && previous.is_some_and(|c| c != ' ' && c != '\n') {
                    normalized.push(' ');
                }
                self.pending_space = false;
                normalized.push(c);
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
            let mut output = ctx.run_ui(input, |ui| page.show(ui));
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
}
