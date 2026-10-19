//! Turn the DOM and computed styles into the display list a `Page` paints.
use super::bidi::text_alignment;
use super::*;

pub(super) struct Builder {
    page: Page,
    current: Block,
    pending_space: bool,
    characters: usize,
    open_boxes: usize,
}
pub(super) enum Visit {
    Enter(NodeId, Box<Style>),
    Exit { block: bool, list: bool },
}

impl Page {
    pub(super) fn build(
        doc: &Document,
        scripting_enabled: bool,
        sheet: &Stylesheet,
        images: &HashMap<NodeId, ImageAsset>,
        reading: Option<&Content>,
        viewport: (f32, f32),
    ) -> Self {
        let mut budget = StyleBudget::default();
        budget.set_viewport(viewport.0, viewport.1);
        let mut builder = Builder {
            page: Self {
                forms: if reading.is_none() {
                    Forms::from_document(doc, scripting_enabled)
                } else {
                    Forms::default()
                },
                root_font: if reading.is_some() {
                    focus::DEFAULT_FONT_SIZE
                } else {
                    17.0
                },
                reading_style: reading.map(|_| Settings::default()),
                ..Self::default()
            },
            current: Block::default(),
            pending_space: false,
            characters: 0,
            open_boxes: 0,
        };
        if let Some(title) = doc.descendants(doc.root()).find(|&id| {
            doc.node(id)
                .and_then(|n| n.as_element())
                .is_some_and(|e| e.is_html_tag("title"))
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
        let mut initial = Style::default();
        if let Some(content) = reading {
            initial.css.direction = content.direction;
            initial.css.font_size = focus::DEFAULT_FONT_SIZE;
            initial.css.color = Theme::Light.reading_ink();
            initial.css.line_height = crate::css::LineHeight::Number(1.65);
            if !content.has_heading && !builder.page.title.trim().is_empty() {
                let mut heading = initial;
                heading.css.font_size *= 1.9;
                heading.css.font_weight = 700.0;
                heading.css.margin[2] = Length::Em(0.5);
                heading.css.line_height = crate::css::LineHeight::Number(1.25);
                builder.open(heading.css);
                builder.text(&builder.page.title.clone(), heading);
                builder.close();
            }
        }
        let mut pending = vec![Visit::Enter(
            reading.map_or(doc.root(), |content| content.root),
            Box::new(initial),
        )];
        let controls: HashMap<_, _> = builder
            .page
            .forms
            .controls
            .iter()
            .enumerate()
            .map(|(i, c)| (c.node, i))
            .collect();
        let mut lists: Vec<Option<i64>> = Vec::new();
        let mut prefetched_child_styles = HashMap::new();
        while let Some(visit) = pending.pop() {
            if builder.page.truncated {
                break;
            }
            let (id, mut style) = match visit {
                Visit::Enter(id, style) => (id, *style),
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
            if reading.is_some_and(|content| content.excluded.contains(&id)) {
                continue;
            }
            match &node.kind {
                NodeKind::Text(text) => {
                    builder.text(text, style);
                    continue;
                }
                NodeKind::Element(element) => {
                    let tag = element.name.local.as_str();
                    if !element.is_html()
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
                    style.css = if let Some(css) = prefetched_child_styles.remove(&id) {
                        css
                    } else if reading.is_some() {
                        focus::style(element, style.css)
                    } else {
                        sheet.compute(doc, id, style.css, builder.page.root_font, &mut budget)
                    };
                    if reading.is_none()
                        && matches!(style.css.display, Display::Block | Display::InlineBlock)
                    {
                        let mut has_layout_children = false;
                        let mut all_horizontal_items = true;
                        let mut all_floats = true;
                        let mut has_direct_text = false;
                        for child in doc.children(id) {
                            let Some(child_node) = doc.node(child) else {
                                continue;
                            };
                            match &child_node.kind {
                                NodeKind::Text(text) => {
                                    has_direct_text |= !text.trim().is_empty();
                                }
                                NodeKind::Element(child_element) => {
                                    let css = sheet.compute(
                                        doc,
                                        child,
                                        style.css,
                                        builder.page.root_font,
                                        &mut budget,
                                    );
                                    if css.display != Display::None {
                                        has_layout_children = true;
                                        all_horizontal_items &= css.display == Display::InlineBlock
                                            || css.float_side != FloatSide::None;
                                        all_floats &= css.float_side != FloatSide::None;
                                    }
                                    if child_element.is_html() {
                                        prefetched_child_styles.insert(child, css);
                                    }
                                }
                                _ => {}
                            }
                        }
                        // Inline-block siblings share a horizontal run and wrap to
                        // the next row when needed. Floats alone lay out as floats.
                        if has_layout_children
                            && all_horizontal_items
                            && !all_floats
                            && !has_direct_text
                        {
                            style.css.display = Display::Flex;
                            style.css.flex_wrap = FlexWrap::Wrap;
                        }
                    }
                    if reading.is_none()
                        && scripting_enabled
                        && element.attribute("onclick").is_some()
                    {
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
                        builder.page.background =
                            Some(paint_color(style.css.background, style.css.opacity));
                    }
                    // Near the display limits, cells fall back to inline runs, one
                    // line per row, so large tables keep their text visible.
                    if style.css.display == Display::TableCell
                        && (builder.page.boxes.len() >= MAX_BOXES * 7 / 10
                            || builder.page.blocks.len() >= MAX_BLOCKS * 7 / 10)
                    {
                        style.css.display = Display::Inline;
                    }
                    // Relative offsets do not apply to inline runs, so only absolute
                    // positioning and floats turn an inline element into a box.
                    let block = style.flex_parent
                        || style.css.position == Position::Absolute
                        || style.css.float_side != FloatSide::None
                        || matches!(
                            style.css.display,
                            Display::Block
                                | Display::Flex
                                | Display::Grid
                                | Display::InlineBlock
                                | Display::Table
                                | Display::TableRow
                                | Display::TableCell
                        );
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
                    if reading.is_none()
                        && matches!(tag, "input" | "textarea" | "select" | "button")
                    {
                        builder.flush();
                        if let Some(&index) = controls.get(&id) {
                            if builder.page.forms.controls[index].kind != Kind::Hidden {
                                builder
                                    .page
                                    .commands
                                    .push(Command::Control(index, style.target));
                            }
                        }
                        if block {
                            builder.close();
                        }
                        continue;
                    }
                    // Inline cells, as in Focus, still need a gap between them.
                    if !block && matches!(tag, "td" | "th") {
                        builder.text(" ", style);
                    }
                    let list = matches!(tag, "ul" | "ol" | "menu");
                    match tag {
                        "ul" | "menu" => lists.push(None),
                        "ol" => lists.push(Some(
                            element
                                .attribute("start")
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(1),
                        )),
                        "li" => {
                            let number = match lists.last_mut() {
                                Some(Some(next)) => {
                                    *next = element
                                        .attribute("value")
                                        .and_then(|v| v.parse().ok())
                                        .unwrap_or(*next);
                                    let number = *next;
                                    *next = next.saturating_add(1);
                                    Some(number)
                                }
                                _ => None,
                            };
                            // Only list-item boxes show a marker, as in browsers;
                            // inline menu items keep their numbering silently.
                            if style.css.list_marker && style.css.display == Display::Block {
                                let marker = match number {
                                    Some(number) => format!("{number}."),
                                    // Bullets step through disc, circle and square with
                                    // the nesting depth of all enclosing lists.
                                    None => ["•", "◦", "▪"][lists.len().clamp(1, 3) - 1].to_owned(),
                                };
                                builder.marker(&marker, style);
                            }
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
                            let rendered = element.attribute("src").and_then(|reference| {
                                images.get(&id).filter(|asset| asset.reference == reference)
                            });
                            if let Some(asset) = rendered {
                                builder.image(
                                    id,
                                    style,
                                    asset,
                                    element.attribute("alt").filter(|alt| !alt.is_empty()),
                                    html_dimension(element.attribute("width")),
                                    html_dimension(element.attribute("height")),
                                );
                            } else if let Some(alt) =
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
            let mut child_style = style;
            child_style.flex_parent = style.css.display == Display::Grid
                || (style.css.display == Display::Flex
                    && style.css.flex_direction == crate::css::FlexDirection::Row);
            pending.extend(
                children
                    .into_iter()
                    .rev()
                    .map(|child| Visit::Enter(child, Box::new(child_style))),
            );
        }
        builder.flush();
        // A display limit may stop inside a box. Balance the paint commands so
        // ancestor backgrounds still contain the displayed prefix.
        while builder.open_boxes > 0 {
            builder.close();
        }
        if reading.is_none() {
            builder.page.css_limited = sheet.diagnostics.limited || budget.limited;
            builder.page.css_ignored = sheet.diagnostics.ignored + budget.ignored;
            let mut unsupported = sheet.diagnostics.unsupported_properties.clone();
            for (property, count) in budget.unsupported_properties {
                *unsupported.entry(property).or_default() += count;
            }
            builder.page.css_unsupported = unsupported.into_iter().collect();
            builder
                .page
                .css_unsupported
                .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            builder.page.css_unsupported.truncate(512);
        }
        builder.page
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
    fn image(
        &mut self,
        node: NodeId,
        style: Style,
        asset: &ImageAsset,
        alt: Option<&str>,
        html_width: Option<f32>,
        html_height: Option<f32>,
    ) {
        self.flush();
        if self.page.images.len() >= MAX_IMAGES {
            self.page.truncated = true;
            return;
        }
        let index = self.page.images.len();
        self.page.images.push(RenderedImage {
            node,
            reference: asset.reference.clone(),
            image: asset.image.clone(),
            alt: alt.map(str::to_owned),
            style: style.css,
            html_width,
            html_height,
            action: Action {
                target: style.target,
                link: style.link,
            },
        });
        self.page.commands.push(Command::Image(index));
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
    /// A list marker, painted outside the item's start edge on its first line.
    fn marker(&mut self, text: &str, style: Style) {
        self.flush();
        let mut marker = Block {
            style: style.css,
            marker: true,
            characters: text.chars().count(),
            ..Default::default()
        };
        let mut format = style.format();
        format.underline = Stroke::NONE;
        format.strikethrough = Stroke::NONE;
        format.background = Color32::TRANSPARENT;
        marker.job.append(text, 0.0, format);
        self.push(marker);
    }
    fn literal(&mut self, text: &str, style: Style) {
        let remaining = MAX_TEXT.saturating_sub(self.characters);
        let accepted: String = text.chars().take(remaining).collect();
        self.characters += accepted.chars().count();
        if accepted.len() < text.len() {
            self.page.truncated = true;
        }
        if accepted.is_empty() {
            self.pending_space = false;
            return;
        }
        if self.current.job.text.is_empty() {
            self.current.style = style.css;
            self.current.job.halign =
                text_alignment(style.css.text_align, style.css.direction == Direction::Rtl);
        }
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

pub(super) fn html_dimension(value: Option<&str>) -> Option<f32> {
    let value = value?.trim();
    let value = value.parse::<f32>().ok()?;
    value.is_finite().then_some(value.clamp(1.0, 10_000.0))
}
