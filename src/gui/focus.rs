//! Bounded, document-local reading extraction and presentation preferences.
use eframe::egui::{self, Color32};
use olive_html::{
    Document, Element, NodeId, NodeKind,
    css::{Color, ComputedStyle, Display, Length, LineHeight, StyleBudget, Stylesheet, WhiteSpace},
};
use std::collections::{HashMap, HashSet};

pub const DEFAULT_FONT_SIZE: f32 = 20.0;
pub const MIN_FONT_SIZE: f32 = 14.0;
pub const MAX_FONT_SIZE: f32 = 32.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

impl Theme {
    pub fn visuals(self) -> egui::Visuals {
        let mut visuals = match self {
            Self::Light => egui::Visuals::light(),
            Self::Dark => egui::Visuals::dark(),
        };
        visuals.selection.bg_fill = match self {
            Self::Light => Color32::from_rgb(209, 223, 182),
            Self::Dark => Color32::from_rgb(64, 82, 43),
        };
        visuals
    }
    pub fn paper(self) -> Color32 {
        match self {
            Self::Light => Color32::from_rgb(250, 248, 241),
            Self::Dark => Color32::from_rgb(27, 30, 26),
        }
    }

    pub fn ink(self) -> Color {
        match self {
            Self::Light => Color(42, 46, 37, 255),
            Self::Dark => Color(226, 229, 218, 255),
        }
    }

    pub fn link(self) -> Color {
        match self {
            Self::Light => Color(75, 99, 39, 255),
            Self::Dark => Color(179, 205, 137, 255),
        }
    }

    pub fn code(self) -> Color {
        match self {
            Self::Light => Color(236, 235, 225, 255),
            Self::Dark => Color(42, 46, 39, 255),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub font_size: f32,
    pub theme: Theme,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
            theme: Theme::Light,
        }
    }
}

impl Settings {
    pub fn normalized(self) -> Self {
        Self {
            font_size: if self.font_size.is_finite() {
                self.font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
            } else {
                DEFAULT_FONT_SIZE
            },
            ..self
        }
    }
}

pub struct Content {
    pub root: NodeId,
    pub excluded: HashSet<NodeId>,
    pub has_heading: bool,
    pub has_text: bool,
}

#[derive(Clone, Copy, Default)]
struct Score {
    text: usize,
    linked: usize,
}

impl Score {
    fn prose(self) -> usize {
        self.text.saturating_sub(self.linked.saturating_mul(2))
    }
}

impl Content {
    pub fn extract(doc: &Document, sheet: &Stylesheet, scripting: bool) -> Self {
        let mut excluded = HashSet::new();
        let mut scores: HashMap<NodeId, Score> = HashMap::new();
        let mut order = Vec::new();
        let mut headings = HashSet::new();
        let mut budget = StyleBudget::default();
        let mut root_font = 17.0;
        let mut pending = vec![(doc.root(), ComputedStyle::default(), false, false)];
        // Iterative passes avoid recursion and repeated subtree scans on deep HTML.
        while let Some((id, parent_style, in_content, in_link)) = pending.pop() {
            let Some(node) = doc.node(id) else { continue };
            let mut style = parent_style;
            let mut content = in_content;
            let mut linked = in_link;
            if let Some(element) = node.as_element() {
                let tag = element.name.local.as_ref();
                if excluded_element(element, in_content, scripting) {
                    excluded.insert(id);
                    continue;
                }
                style = sheet.compute(doc, id, parent_style, root_font, &mut budget);
                if style.display == Display::None {
                    excluded.insert(id);
                    continue;
                }
                if tag == "html" {
                    root_font = style.font_size;
                }
                content |=
                    matches!(tag, "main" | "article") || element.attribute("role") == Some("main");
                linked |= tag == "a" && element.attribute("href").is_some();
                if matches!(tag, "h1" | "h2") {
                    headings.insert(id);
                }
            }
            let text = match &node.kind {
                NodeKind::Text(text) => text.chars().filter(|c| !c.is_whitespace()).count(),
                _ => 0,
            };
            scores.insert(
                id,
                Score {
                    text,
                    linked: if linked { text } else { 0 },
                },
            );
            order.push(id);
            let children: Vec<_> = doc.children(id).collect();
            pending.extend(
                children
                    .into_iter()
                    .rev()
                    .map(|child| (child, style, content, linked)),
            );
        }
        for &id in order.iter().rev() {
            let score = scores[&id];
            if let Some(parent) = doc.node(id).and_then(|node| node.parent()) {
                if let Some(total) = scores.get_mut(&parent) {
                    total.text = total.text.saturating_add(score.text);
                    total.linked = total.linked.saturating_add(score.linked);
                }
            }
        }
        let mut semantic = None;
        let mut generic = None;
        for &id in &order {
            let Some(element) = doc.node(id).and_then(|node| node.as_element()) else {
                continue;
            };
            let score = scores[&id];
            if score.text == 0 {
                continue;
            }
            let tag = element.name.local.as_ref();
            let priority = match tag {
                "article" => 2,
                "main" => 1,
                _ if element.attribute("role") == Some("main") => 1,
                _ => 0,
            };
            if priority > 0 && score.prose() > 0 {
                let rank = (priority, score.prose());
                if semantic.is_none_or(|(_, previous)| rank > previous) {
                    semantic = Some((id, rank));
                }
            }
            if matches!(tag, "body" | "div" | "section") && score.prose() >= 160 {
                // Prefer an inner content wrapper on ties, leaving surrounding chrome behind.
                if generic.is_none_or(|(_, previous)| score.prose() >= previous) {
                    generic = Some((id, score.prose()));
                }
            }
        }
        let root = semantic
            .map(|(id, _)| id)
            .or_else(|| generic.map(|(id, _)| id))
            .unwrap_or(doc.root());
        let has_heading =
            headings.contains(&root) || doc.descendants(root).any(|id| headings.contains(&id));
        Self {
            root,
            excluded,
            has_heading,
            has_text: scores.get(&root).is_some_and(|score| score.text > 0),
        }
    }
}

fn excluded_element(element: &Element, in_content: bool, scripting: bool) -> bool {
    let tag = element.name.local.as_ref();
    if element.name.ns.as_ref() != "http://www.w3.org/1999/xhtml"
        || element.attribute("hidden").is_some()
        || element
            .attribute("aria-hidden")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        || (tag == "noscript" && scripting)
        || (!in_content && matches!(tag, "header" | "footer"))
        || matches!(
            tag,
            "head"
                | "script"
                | "style"
                | "template"
                | "nav"
                | "aside"
                | "form"
                | "button"
                | "input"
                | "select"
                | "textarea"
                | "iframe"
                | "object"
                | "embed"
                | "video"
                | "audio"
                | "canvas"
                | "dialog"
        )
    {
        return true;
    }
    if element.attribute("role").is_some_and(|role| {
        role.split_ascii_whitespace().any(|role| {
            matches!(
                role,
                "navigation"
                    | "banner"
                    | "contentinfo"
                    | "complementary"
                    | "search"
                    | "dialog"
                    | "alertdialog"
            )
        })
    }) {
        return true;
    }
    [element.attribute("id"), element.attribute("class")]
        .into_iter()
        .flatten()
        .any(|value| {
            value.split(|c: char| !c.is_alphanumeric()).any(|token| {
                matches!(
                    token.to_ascii_lowercase().as_str(),
                    "ad" | "ads"
                        | "advert"
                        | "advertisement"
                        | "advertising"
                        | "promo"
                        | "promotion"
                        | "sidebar"
                        | "cookie"
                        | "cookies"
                        | "newsletter"
                        | "subscribe"
                        | "subscription"
                        | "social"
                        | "share"
                        | "sharing"
                        | "related"
                        | "recommended"
                        | "recommendations"
                        | "comment"
                        | "comments"
                        | "menu"
                        | "navigation"
                        | "pagination"
                        | "popup"
                )
            })
        })
}

/// Reader styles deliberately ignore author typography and box decoration.
pub fn style(element: &Element, parent: ComputedStyle) -> ComputedStyle {
    let tag = element.name.local.as_ref();
    let mut style = ComputedStyle {
        color: parent.color,
        font_size: parent.font_size,
        font_weight: parent.font_weight,
        italic: parent.italic,
        monospace: parent.monospace,
        underline: parent.underline,
        strike: parent.strike,
        white_space: parent.white_space,
        line_height: parent.line_height,
        ..Default::default()
    };
    if matches!(
        tag,
        "html"
            | "body"
            | "main"
            | "article"
            | "section"
            | "header"
            | "footer"
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
    ) {
        style.display = Display::Block;
    }
    if matches!(
        tag,
        "p" | "ul" | "ol" | "pre" | "blockquote" | "hr" | "figure" | "tr" | "dd"
    ) {
        style.margin[2] = Length::Em(0.9);
    }
    match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            style.font_size = DEFAULT_FONT_SIZE
                * [1.9, 1.4, 1.2, 1.1, 1.0, 1.0][(tag.as_bytes()[1] - b'1') as usize];
            style.font_weight = 700.0;
            style.line_height = LineHeight::Number(1.25);
            style.margin = [
                Length::Em(0.6),
                Length::Px(0.0),
                Length::Em(0.5),
                Length::Px(0.0),
            ];
        }
        "b" | "strong" | "th" | "dt" => style.font_weight = 700.0,
        "i" | "em" | "cite" | "dfn" => style.italic = true,
        "u" => style.underline = true,
        "s" | "del" => style.strike = true,
        "a" if element.attribute("href").is_some() => {
            style.color = Theme::Light.link();
            style.underline = true;
        }
        "small" | "figcaption" => style.font_size = DEFAULT_FONT_SIZE * 0.85,
        "code" | "kbd" | "samp" | "pre" => {
            style.monospace = true;
            style.font_size = DEFAULT_FONT_SIZE * 0.85;
            style.background = Theme::Light.code();
            if tag == "pre" {
                style.white_space = WhiteSpace::PreWrap;
                style.padding = [Length::Em(0.7); 4];
            }
        }
        _ => {}
    }
    style
}
