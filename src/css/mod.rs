//! An inert CSS subset for documents. Tokenization and error recovery use
//! Servo's `cssparser`; Olive implements selectors, property values and cascade.
//! No CSS construct fetches a resource. Unsupported rules/declarations are skipped.
mod selectors;
mod values;

use crate::{Document, Element, ExternalSource, NodeId, NodeKind};
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, Delimiter, Parser, ParserState, QualifiedRuleParser,
    RuleBodyItemParser, RuleBodyParser, StyleSheetParser,
};
use selectors::{AncestorFilter, Key, Selector};
use std::collections::HashMap;
pub use values::{
    BoxShadow, Clear, Color, ComputedStyle, Direction, Display, FlexDirection, FlexWrap, FloatSide,
    JustifyContent, Length, LineHeight, Position, TextAlign, TextTransform, VerticalAlign,
    WhiteSpace,
};
use values::{Declaration, PROPERTIES, Value};

pub const MAX_CSS_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RULES: usize = 32_768;
/// Viewport assumed until the host reports the real content size.
pub(crate) const DEFAULT_VIEWPORT: (f32, f32) = (1024.0, 768.0);
const MAX_DECLARATIONS: usize = 512;
const MAX_SELECTORS: usize = 128;
const MATCH_BUDGET: usize = 8_000_000;
const MAX_CUSTOM_PROPERTIES: usize = 256;
const MAX_CUSTOM_PROPERTY_BYTES: usize = 64 * 1024;
const MAX_CUSTOM_PROPERTY_STORAGE_BYTES: usize = 1024 * 1024;
const MAX_VAR_DEPTH: usize = 16;
const MAX_MEDIA_ALTERNATIVES: usize = 32;
const MAX_DEFERRED_DECLARATIONS: usize = 512;

#[derive(Clone, Debug)]
struct Rule {
    selectors: Vec<Selector>,
    declarations: Vec<Declaration>,
    media: MediaCondition,
}

#[derive(Clone, Copy, Debug)]
struct MediaBranch {
    valid: bool,
    screen: bool,
    min_width: Option<f32>,
    max_width: Option<f32>,
    min_height: Option<f32>,
    max_height: Option<f32>,
    landscape: Option<bool>,
}
impl Default for MediaBranch {
    fn default() -> Self {
        Self {
            valid: true,
            screen: true,
            min_width: None,
            max_width: None,
            min_height: None,
            max_height: None,
            landscape: None,
        }
    }
}
impl MediaBranch {
    fn combine(self, child: Self) -> Self {
        Self {
            valid: self.valid
                && child.valid
                && !matches!((self.landscape, child.landscape), (Some(a), Some(b)) if a != b),
            screen: self.screen && child.screen,
            min_width: match (self.min_width, child.min_width) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            },
            max_width: match (self.max_width, child.max_width) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            },
            min_height: match (self.min_height, child.min_height) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            },
            max_height: match (self.max_height, child.max_height) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            },
            landscape: child.landscape.or(self.landscape),
        }
    }
    fn matches(self, width: f32, height: f32) -> bool {
        self.valid
            && self.screen
            && self.min_width.is_none_or(|value| width >= value)
            && self.max_width.is_none_or(|value| width <= value)
            && self.min_height.is_none_or(|value| height >= value)
            && self.max_height.is_none_or(|value| height <= value)
            && self
                .landscape
                .is_none_or(|value| (width >= height) == value)
    }
}

#[derive(Clone, Debug)]
struct MediaCondition {
    alternatives: Vec<MediaBranch>,
}
impl Default for MediaCondition {
    fn default() -> Self {
        Self {
            alternatives: vec![MediaBranch::default()],
        }
    }
}
impl MediaCondition {
    fn never() -> Self {
        Self {
            alternatives: vec![MediaBranch {
                valid: false,
                ..MediaBranch::default()
            }],
        }
    }

    fn combine(&self, child: &Self) -> Self {
        let mut alternatives = Vec::new();
        for parent in &self.alternatives {
            for nested in &child.alternatives {
                if alternatives.len() == MAX_MEDIA_ALTERNATIVES {
                    return Self {
                        alternatives: vec![MediaBranch {
                            valid: false,
                            ..MediaBranch::default()
                        }],
                    };
                }
                alternatives.push((*parent).combine(*nested));
            }
        }
        Self { alternatives }
    }
    fn matches(&self, width: f32, height: f32) -> bool {
        self.alternatives
            .iter()
            .any(|condition| condition.matches(width, height))
    }
    fn has_screen(&self) -> bool {
        self.alternatives.iter().any(|condition| condition.screen)
    }
    fn needs_viewport_reflow(&self) -> bool {
        self.alternatives.iter().any(|condition| {
            condition.min_width.is_some()
                || condition.max_width.is_some()
                || condition.min_height.is_some()
                || condition.max_height.is_some()
                || condition.landscape.is_some()
        })
    }
}

fn has_viewport_units(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    lower
        .as_bytes()
        .windows(2)
        .enumerate()
        .any(|(index, unit)| {
            if unit != b"vw" && unit != b"vh" {
                return false;
            }
            let bytes = lower.as_bytes();
            index > 0
                && bytes[index - 1].is_ascii_digit()
                && bytes
                    .get(index + 2)
                    .is_none_or(|next| !next.is_ascii_alphanumeric() && *next != b'-')
        })
}

fn parse_media_query(source: &str) -> Option<MediaCondition> {
    let source = source.split_whitespace().collect::<Vec<_>>().join(" ");
    let source = source.trim().to_ascii_lowercase();
    if source.is_empty() {
        return Some(MediaCondition::default());
    }
    if source.contains(',') {
        let alternatives: Vec<_> = source
            .split(',')
            .filter_map(parse_media_branch)
            .take(MAX_MEDIA_ALTERNATIVES + 1)
            .collect();
        if alternatives.is_empty() || alternatives.len() > MAX_MEDIA_ALTERNATIVES {
            return None;
        }
        return Some(MediaCondition { alternatives });
    }
    Some(MediaCondition {
        alternatives: vec![parse_media_branch(&source)?],
    })
}

fn parse_media_branch(source: &str) -> Option<MediaBranch> {
    let mut condition = MediaBranch::default();
    for part in source.split(" and ").map(str::trim) {
        match part {
            "screen" | "all" | "only screen" | "only all" => {}
            "not print" => {}
            "print" | "speech" => condition.screen = false,
            _ => {
                let feature = part.strip_prefix('(')?.strip_suffix(')')?.trim();
                let (name, value) = feature.split_once(':')?;
                let value = value.trim();
                match name.trim() {
                    "min-width" | "max-width" | "min-height" | "max-height" => {
                        let number_end = value
                            .find(|c: char| {
                                !(c.is_ascii_digit() || c == '.' || c == '+' || c == '-')
                            })
                            .unwrap_or(value.len());
                        let number = value[..number_end].parse::<f32>().ok()?;
                        if !number.is_finite() || number < 0.0 {
                            return None;
                        }
                        let unit = value[number_end..].trim();
                        let px = match unit {
                            "px" | "" => number,
                            "em" | "rem" => number * 16.0,
                            _ => return None,
                        };
                        match name.trim() {
                            "min-width" => condition.min_width = Some(px),
                            "max-width" => condition.max_width = Some(px),
                            "min-height" => condition.min_height = Some(px),
                            _ => condition.max_height = Some(px),
                        }
                    }
                    "orientation" => {
                        condition.landscape = Some(match value {
                            "landscape" => true,
                            "portrait" => false,
                            _ => return None,
                        });
                    }
                    _ => return None,
                }
            }
        }
    }
    Some(condition)
}

fn media_attribute(source: Option<&str>) -> MediaCondition {
    match source {
        None => MediaCondition::default(),
        Some(source) => parse_media_query(source).unwrap_or_else(MediaCondition::never),
    }
}

/// Counts rejected syntax/features and reports an exceeded processing limit.
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub ignored: usize,
    pub limited: bool,
    pub unsupported_properties: HashMap<String, usize>,
}
/// Parsed author styles, ordered as they appeared in the document.
#[derive(Clone, Default, Debug)]
pub struct Stylesheet {
    rules: Vec<Rule>,
    index: RuleIndex,
    unindexed: Vec<(usize, usize)>,
    pub diagnostics: Diagnostics,
    bytes: usize,
    viewport_dependent: bool,
}
/// Selectors by the ID, class, attribute or tag their subject needs, each
/// folded to ASCII lowercase, as `(rule, selector)` positions.
#[derive(Clone, Default, Debug)]
struct RuleIndex {
    ids: HashMap<String, Vec<(usize, usize)>>,
    classes: HashMap<String, Vec<(usize, usize)>>,
    attributes: HashMap<String, Vec<(usize, usize)>>,
    tags: HashMap<String, Vec<(usize, usize)>>,
}

impl RuleIndex {
    fn insert(&mut self, key: Key, entry: (usize, usize)) {
        let (map, name) = match key {
            Key::Id(name) => (&mut self.ids, name),
            Key::Class(name) => (&mut self.classes, name),
            Key::Attribute(name) => (&mut self.attributes, name),
            Key::Tag(name) => (&mut self.tags, name),
        };
        map.entry(name).or_default().push(entry);
    }
}

/// `value` folded to ASCII lowercase, without copying when it already is.
fn ascii_lowercase(value: &str) -> std::borrow::Cow<'_, str> {
    if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        value.to_ascii_lowercase().into()
    } else {
        value.into()
    }
}

/// A per-document work budget. Reuse it across the whole style traversal of
/// one document.
#[derive(Debug)]
pub struct StyleBudget {
    matches: usize,
    inline_bytes: usize,
    custom_properties: HashMap<NodeId, std::sync::Arc<HashMap<String, String>>>,
    /// The ancestor filter for the children of each parent seen so far.
    ancestor_filters: HashMap<NodeId, AncestorFilter>,
    /// Reused storage for the rules an element may match.
    candidates: Vec<(usize, usize)>,
    custom_property_bytes_used: usize,
    pub limited: bool,
    pub ignored: usize,
    pub unsupported_properties: HashMap<String, usize>,
    viewport_width: f32,
    viewport_height: f32,
}
impl Default for StyleBudget {
    fn default() -> Self {
        Self {
            matches: MATCH_BUDGET,
            inline_bytes: 0,
            custom_properties: HashMap::new(),
            ancestor_filters: HashMap::new(),
            candidates: Vec::new(),
            custom_property_bytes_used: 0,
            limited: false,
            ignored: 0,
            unsupported_properties: HashMap::new(),
            viewport_width: DEFAULT_VIEWPORT.0,
            viewport_height: DEFAULT_VIEWPORT.1,
        }
    }
}
impl StyleBudget {
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport_width = width.clamp(1.0, 10_000.0);
        self.viewport_height = height.clamp(1.0, 10_000.0);
    }

    /// The filter of the names of `id`'s ancestors, built once per parent.
    fn ancestor_filter(&mut self, doc: &Document, id: NodeId) -> AncestorFilter {
        let mut filter = AncestorFilter::default();
        let mut uncached = Vec::new();
        let mut ancestor = doc.node(id).and_then(|node| node.parent());
        while let Some(parent) = ancestor {
            if let Some(cached) = self.ancestor_filters.get(&parent) {
                filter = *cached;
                break;
            }
            uncached.push(parent);
            ancestor = doc.node(parent).and_then(|node| node.parent());
        }
        for parent in uncached.into_iter().rev() {
            if let Some(element) = doc.node(parent).and_then(|node| node.as_element()) {
                filter = filter.with_element(element);
            }
            self.ancestor_filters.insert(parent, filter);
        }
        filter
    }
}
impl Stylesheet {
    /// Parse one stylesheet, recovering at rule/declaration boundaries.
    pub fn parse(source: &str) -> Self {
        let mut sheet = Self::default();
        sheet.append(source);
        sheet
    }
    /// Collect applicable HTML `<style>` elements in document order. Template
    /// fragments and non-CSS types are ignored; supported media conditions are
    /// evaluated later against the requested viewport.
    pub fn from_document(doc: &Document) -> Self {
        Self::from_document_with_sources(doc, &HashMap::new())
    }

    /// Merge loaded links and embedded styles in DOM order, sharing one CSS budget.
    pub fn from_document_with_sources(
        doc: &Document,
        sources: &HashMap<NodeId, ExternalSource>,
    ) -> Self {
        let mut sheet = Self::default();
        for id in doc.descendants(doc.root()) {
            let Some(element) = doc.node(id).and_then(|n| n.as_element()) else {
                continue;
            };
            sheet.viewport_dependent |= element.attribute("style").is_some_and(has_viewport_units);
            if !applicable_style(element) {
                continue;
            }
            if element.name.local.as_str() == "link" {
                if let Some(source) = sources
                    .get(&id)
                    .filter(|source| element.attribute("href") == Some(source.reference.as_str()))
                {
                    let media = media_attribute(element.attribute("media"));
                    sheet.append_with_media(&source.source, media);
                }
                continue;
            }
            let source: String = doc
                .children(id)
                .filter_map(|id| match &doc.node(id)?.kind {
                    NodeKind::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            let media = media_attribute(element.attribute("media"));
            sheet.append_with_media(&source, media);
        }
        sheet
    }
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }
    pub fn needs_viewport_reflow(&self) -> bool {
        self.viewport_dependent
    }
    fn append(&mut self, source: &str) {
        self.append_with_media(source, MediaCondition::default());
    }
    fn append_with_media(&mut self, source: &str, media: MediaCondition) {
        if self.bytes.saturating_add(source.len()) > MAX_CSS_BYTES {
            self.diagnostics.limited = true;
            return;
        }
        self.viewport_dependent |= media.needs_viewport_reflow() || has_viewport_units(source);
        self.bytes += source.len();
        let mut input = Parser::new(source);
        let mut remaining = MAX_RULES - self.rules.len();
        let mut parser = Rules {
            diagnostics: &mut self.diagnostics,
            remaining: &mut remaining,
            depth: 0,
            media,
            viewport_dependent: &mut self.viewport_dependent,
        };
        let mut ignored = 0;
        let mut limited = false;
        for result in StyleSheetParser::new(&mut input, &mut parser) {
            if self.rules.len() == MAX_RULES {
                limited = true;
                break;
            }
            match result {
                Ok(rules) => {
                    for rule in rules {
                        let index = self.rules.len();
                        for (selector_index, selector) in rule.selectors.iter().enumerate() {
                            let entry = (index, selector_index);
                            if let Some(key) = selector.key() {
                                self.index.insert(key, entry);
                            } else {
                                self.unindexed.push(entry);
                            }
                        }
                        self.rules.push(rule);
                    }
                }
                Err(_) => ignored += 1,
            }
        }
        self.diagnostics.ignored += ignored;
        self.diagnostics.limited |= limited;
    }
    /// Compute an HTML element's style after its parent. Pass the computed root
    /// font size for `rem` (17 for the root itself) and one shared work budget.
    pub fn compute(
        &self,
        doc: &Document,
        id: NodeId,
        parent: ComputedStyle,
        root_font: f32,
        budget: &mut StyleBudget,
    ) -> ComputedStyle {
        let mut style = ComputedStyle::inherited(parent);
        let Some(element) = doc.node(id).and_then(|n| n.as_element()) else {
            return style;
        };
        let parent_id = doc.node(id).and_then(|node| node.parent());
        let parent_element = parent_id.and_then(|parent| doc.node(parent)?.as_element());
        // Cells take `cellpadding` and `border` from their table: td, tr, tbody, table.
        let table = if matches!(element.name.local.as_str(), "td" | "th") {
            std::iter::successors(parent_id, |&ancestor| doc.node(ancestor)?.parent())
                .take(3)
                .filter_map(|ancestor| doc.node(ancestor)?.as_element())
                .find(|ancestor| ancestor.is_html_tag("table"))
        } else {
            None
        };
        let quirks = doc.quirks_mode() == crate::QuirksMode::Quirks;
        let border_color = user_agent(element, parent_element, table, quirks, &mut style);
        // Importance, inline, specificity, then source order: the later rule wins
        // ties, so rules can be considered in any order.
        type Priority = (bool, bool, (u16, u16, u16), usize);
        let mut winners: [Option<(Priority, Value)>; PROPERTIES.len()] = [None; PROPERTIES.len()];
        let mut custom_winners: HashMap<String, (Priority, String)> = HashMap::new();
        let mut deferred: Vec<(Priority, Declaration)> = Vec::new();
        let mut custom_limited = false;
        let mut deferred_limited = false;
        let mut consider = |declaration: &Declaration, inline, specificity, order| {
            let priority = (declaration.important, inline, specificity, order);
            if declaration.name.starts_with("--") {
                let eligible = custom_winners.contains_key(&declaration.name)
                    || custom_winners.len() < MAX_CUSTOM_PROPERTIES;
                if eligible
                    && custom_winners
                        .get(&declaration.name)
                        .is_none_or(|(previous, _)| priority >= *previous)
                {
                    custom_winners.insert(
                        declaration.name.clone(),
                        (priority, declaration.raw.clone()),
                    );
                } else if !eligible {
                    custom_limited = true;
                }
            } else if declaration.values.is_empty() {
                if deferred.len() < MAX_DEFERRED_DECLARATIONS {
                    deferred.push((priority, declaration.clone()));
                } else {
                    deferred_limited = true;
                }
            } else {
                for (property, value) in &declaration.values {
                    let winner = &mut winners[*property as usize];
                    if winner.is_none_or(|(previous, _)| priority >= previous) {
                        *winner = Some((priority, *value));
                    }
                }
            }
        };
        // Each selector sits in one index list, so no candidate repeats.
        let mut candidates = std::mem::take(&mut budget.candidates);
        candidates.clear();
        if budget.matches > 0 {
            candidates.extend_from_slice(&self.unindexed);
            let mut collect = |map: &HashMap<String, Vec<(usize, usize)>>, name: &str| {
                if let Some(entries) = map.get(name) {
                    candidates.extend_from_slice(entries);
                }
            };
            collect(
                &self.index.tags,
                &ascii_lowercase(element.name.local.as_str()),
            );
            if let Some(id) = element.attribute("id") {
                collect(&self.index.ids, &ascii_lowercase(id));
            }
            if let Some(classes) = element.attribute("class") {
                let mut classes: Vec<_> = classes
                    .split_ascii_whitespace()
                    .map(ascii_lowercase)
                    .collect();
                classes.sort_unstable();
                classes.dedup();
                for class in classes {
                    collect(&self.index.classes, &class);
                }
            }
            for attribute in &element.attributes {
                if attribute.name.ns.is_empty() {
                    collect(&self.index.attributes, &attribute.name.local);
                }
            }
        }
        let ancestors = if candidates.is_empty() {
            AncestorFilter::default()
        } else {
            budget.ancestor_filter(doc, id)
        };
        for &(rule_index, selector_index) in &candidates {
            if budget.matches == 0 {
                budget.limited = true;
                break;
            }
            let rule = &self.rules[rule_index];
            if !rule
                .media
                .matches(budget.viewport_width, budget.viewport_height)
            {
                continue;
            }
            let selector = &rule.selectors[selector_index];
            if selector.matches_with(doc, id, &mut budget.matches, Some(&ancestors)) {
                for declaration in &rule.declarations {
                    consider(declaration, false, selector.specificity, rule_index);
                }
            }
        }
        budget.candidates = candidates;
        budget.limited |= budget.matches == 0;
        if let Some(inline) = element.attribute("style") {
            if self
                .bytes
                .saturating_add(budget.inline_bytes)
                .saturating_add(inline.len())
                <= MAX_CSS_BYTES
            {
                budget.inline_bytes += inline.len();
                let mut input = Parser::new(inline);
                let mut diagnostics = Diagnostics::default();
                let declarations = declarations(&mut input, &mut diagnostics);
                budget.limited |= diagnostics.limited;
                budget.ignored += diagnostics.ignored;
                for (property, count) in diagnostics.unsupported_properties {
                    record_unsupported(&mut budget.unsupported_properties, &property, count);
                }
                for declaration in &declarations {
                    consider(declaration, true, (0, 0, 0), usize::MAX);
                }
            } else {
                budget.limited = true;
            }
        }
        drop(consider);
        budget.limited |= custom_limited || deferred_limited;
        let parent_variables = doc
            .node(id)
            .and_then(|node| node.parent())
            .and_then(|parent| budget.custom_properties.get(&parent))
            .cloned()
            .unwrap_or_else(|| std::sync::Arc::new(HashMap::new()));
        let mut variables = parent_variables.clone();
        for (name, (_, raw)) in custom_winners {
            let variables_mut = std::sync::Arc::make_mut(&mut variables);
            match raw.trim().to_ascii_lowercase().as_str() {
                "initial" => {
                    variables_mut.remove(&name);
                }
                "inherit" | "unset" => {
                    if let Some(value) = parent_variables.get(&name) {
                        variables_mut.insert(name, value.clone());
                    } else {
                        variables_mut.remove(&name);
                    }
                }
                _ => {
                    if variables_mut.len() >= MAX_CUSTOM_PROPERTIES
                        && !variables_mut.contains_key(&name)
                    {
                        budget.limited = true;
                        continue;
                    }
                    if raw.len() > MAX_CUSTOM_PROPERTY_BYTES {
                        budget.limited = true;
                        continue;
                    }
                    variables_mut.insert(name, raw);
                }
            }
        }
        let variable_bytes = variables
            .iter()
            .map(|(name, value)| name.len().saturating_add(value.len()))
            .sum::<usize>();
        if variable_bytes > MAX_CUSTOM_PROPERTY_BYTES {
            budget.limited = true;
            variables = parent_variables.clone();
        }
        if !std::sync::Arc::ptr_eq(&variables, &parent_variables) {
            if budget
                .custom_property_bytes_used
                .saturating_add(variable_bytes)
                > MAX_CUSTOM_PROPERTY_STORAGE_BYTES
            {
                budget.limited = true;
                variables = parent_variables.clone();
            } else {
                budget.custom_property_bytes_used += variable_bytes;
            }
        }
        budget.custom_properties.insert(id, variables.clone());
        for (priority, declaration) in deferred {
            let Some(expanded) = substitute_variables(&declaration.raw, &variables) else {
                budget.ignored += 1;
                continue;
            };
            let parsed = parse_deferred(&declaration.name, &expanded);
            match parsed {
                Some(values) => {
                    for (property, value) in values {
                        let winner = &mut winners[property as usize];
                        if winner.is_none_or(|(previous, _)| priority >= previous) {
                            *winner = Some((priority, value));
                        }
                    }
                }
                None => budget.ignored += 1,
            }
        }
        // Font size precedes dependent lengths; color precedes currentColor.
        let mut root_font = root_font;
        for property in PROPERTIES {
            if let Some((_, value)) = winners[property as usize] {
                values::apply(
                    &mut style,
                    parent,
                    root_font,
                    (budget.viewport_width, budget.viewport_height),
                    property,
                    value,
                );
            }
            if property == values::Property::FontSize && element.name.local.as_str() == "html" {
                root_font = style.font_size;
            }
            if property == values::Property::Color {
                style.border_color = [border_color.unwrap_or(style.color); 4];
            }
        }
        // Resolve em/rem now so explicit inheritance preserves computed lengths.
        let font = style.font_size;
        let computed_length = |v| match v {
            Length::Em(_) | Length::Rem(_) | Length::Vw(_) | Length::Vh(_) => {
                Length::Px(v.resolve_with_viewport(
                    0.0,
                    font,
                    root_font,
                    budget.viewport_width,
                    budget.viewport_height,
                ))
            }
            Length::Calc {
                px,
                em,
                rem,
                percent,
                vw,
                vh,
            } => Length::Calc {
                px: px
                    + em * font
                    + rem * root_font
                    + vw * budget.viewport_width / 100.0
                    + vh * budget.viewport_height / 100.0,
                em: 0.0,
                rem: 0.0,
                percent,
                vw: 0.0,
                vh: 0.0,
            },
            _ => v,
        };
        style.margin = style.margin.map(computed_length);
        style.padding = style.padding.map(computed_length);
        style.width = computed_length(style.width);
        style.min_width = computed_length(style.min_width);
        style.max_width = computed_length(style.max_width);
        style.height = computed_length(style.height);
        style.min_height = computed_length(style.min_height);
        style.border_width = style.border_width.map(computed_length);
        style.border_radius = style.border_radius.map(computed_length);
        style.flex_basis = computed_length(style.flex_basis);
        style.row_gap = computed_length(style.row_gap);
        style.column_gap = computed_length(style.column_gap);
        style.inset = style.inset.map(computed_length);
        style.box_shadow = style.box_shadow.map(|mut shadow| {
            shadow.offset_x = computed_length(shadow.offset_x);
            shadow.offset_y = computed_length(shadow.offset_y);
            shadow.blur = computed_length(shadow.blur);
            shadow.spread = computed_length(shadow.spread);
            shadow
        });
        if element.attribute("hidden").is_some() {
            style.display = Display::None;
        }
        style
    }
}

/// Whether this HTML style/link applies to Olive's screen media subset.
pub fn applicable_style(element: &Element) -> bool {
    if !element.is_html()
        || !matches!(element.name.local.as_str(), "style" | "link")
        || element.attribute("disabled").is_some()
        || element
            .attribute("type")
            .is_some_and(|s| !s.trim().is_empty() && !s.trim().eq_ignore_ascii_case("text/css"))
        || element.attribute("media").is_some_and(|s| !screen_media(s))
    {
        return false;
    }
    if element.name.local.as_str() == "link" {
        let rel: Vec<_> = element
            .attribute("rel")
            .unwrap_or("")
            .split_ascii_whitespace()
            .collect();
        return element.attribute("href").is_some()
            && rel.iter().any(|s| s.eq_ignore_ascii_case("stylesheet"))
            && !rel.iter().any(|s| s.eq_ignore_ascii_case("alternate"));
    }
    true
}

struct Rules<'a> {
    diagnostics: &'a mut Diagnostics,
    remaining: &'a mut usize,
    depth: usize,
    media: MediaCondition,
    viewport_dependent: &'a mut bool,
}
fn screen_media(source: &str) -> bool {
    source.trim().is_empty()
        || source
            .split(',')
            .any(|query| parse_media_query(query).is_some_and(|condition| condition.has_screen()))
}
impl<'i> AtRuleParser<'i> for Rules<'_> {
    type Prelude = MediaCondition;
    type AtRule = Vec<Rule>;
    type Error = ();
    fn parse_prelude(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<MediaCondition, cssparser::ParseError<()>> {
        if !name.eq_ignore_ascii_case("media") {
            return Err(cssparser::ParseError::custom(()));
        }
        let start = input.position();
        while input.next().is_ok() {}
        let source = input.slice_from(start);
        parse_media_query(source).ok_or_else(|| cssparser::ParseError::custom(()))
    }
    fn parse_block(
        &mut self,
        condition: MediaCondition,
        _: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Vec<Rule>, cssparser::ParseError<()>> {
        *self.viewport_dependent |= condition.needs_viewport_reflow();
        if self.depth >= 8 {
            self.diagnostics.limited = true;
            while input.next().is_ok() {}
            return Ok(vec![]);
        }
        let mut result = vec![];
        let limit = *self.remaining;
        let mut ignored = 0;
        let mut limited = false;
        let mut parser = Rules {
            diagnostics: self.diagnostics,
            remaining: self.remaining,
            depth: self.depth + 1,
            media: self.media.combine(&condition),
            viewport_dependent: self.viewport_dependent,
        };
        for item in StyleSheetParser::new(input, &mut parser) {
            match item {
                Ok(rules) => result.extend(rules),
                Err(_) => ignored += 1,
            }
            if result.len() >= limit {
                limited = true;
                break;
            }
        }
        self.diagnostics.ignored += ignored;
        self.diagnostics.limited |= limited;
        // A nested parser requires its input to be exhausted even when the
        // retained-rule budget stops collection partway through the block.
        while input.next().is_ok() {}
        Ok(result)
    }
}
impl<'i> QualifiedRuleParser<'i> for Rules<'_> {
    type Prelude = Vec<Selector>;
    type QualifiedRule = Vec<Rule>;
    type Error = ();
    fn parse_prelude(
        &mut self,
        input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, cssparser::ParseError<()>> {
        let mut count = 0;
        let selectors = input.parse_comma_separated(|input| {
            count += 1;
            if count > MAX_SELECTORS {
                self.diagnostics.limited = true;
                return Err(cssparser::ParseError::custom(()));
            }
            Selector::parse(input)
        })?;
        // Generated-content selectors are valid but never match an element.
        Ok(selectors
            .into_iter()
            .filter(|selector| !selector.generated)
            .collect())
    }
    fn parse_block(
        &mut self,
        selectors: Self::Prelude,
        _: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Vec<Rule>, cssparser::ParseError<()>> {
        // Rules that only style generated content have nothing to match.
        if selectors.is_empty() {
            while input.next().is_ok() {}
            return Ok(vec![]);
        }
        if *self.remaining == 0 {
            self.diagnostics.limited = true;
            return Ok(vec![]);
        }
        *self.remaining -= 1;
        Ok(vec![Rule {
            selectors,
            declarations: declarations(input, self.diagnostics),
            media: self.media.clone(),
        }])
    }
}
#[derive(Default)]
struct Declarations {
    unsupported_properties: HashMap<String, usize>,
}
impl<'i> AtRuleParser<'i> for Declarations {
    type Prelude = ();
    type AtRule = Vec<Declaration>;
    type Error = ();
}
impl<'i> QualifiedRuleParser<'i> for Declarations {
    type Prelude = ();
    type QualifiedRule = Vec<Declaration>;
    type Error = ();
}
impl<'i> RuleBodyItemParser<'i, Vec<Declaration>, ()> for Declarations {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}
impl<'i> DeclarationParser<'i> for Declarations {
    type Declaration = Vec<Declaration>;
    type Error = ();
    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _: &ParserState,
    ) -> Result<Self::Declaration, cssparser::ParseError<()>> {
        let name = if name.starts_with("--") {
            name.to_string()
        } else {
            name.to_ascii_lowercase()
        };
        if !name.starts_with("--") && !values::supports_property(&name) {
            let bounded_name: String = name.chars().take(128).collect();
            if self.unsupported_properties.len() < 512
                || self.unsupported_properties.contains_key(&bounded_name)
            {
                *self.unsupported_properties.entry(bounded_name).or_default() += 1;
            }
            return Err(cssparser::ParseError::custom(()));
        }
        let (raw, parsed) = input.parse_until_before(Delimiter::Bang, |p| {
            let start = p.position();
            let raw = p.slice_from(start).trim().to_owned();
            let parsed = if name.starts_with("--") || raw.to_ascii_lowercase().contains("var(") {
                while p.next().is_ok() {}
                Vec::new()
            } else {
                let parsed = values::parse_values(&name, p)?;
                p.expect_exhausted()?;
                parsed
            };
            Ok((raw, parsed))
        })?;
        let important = input.try_parse(cssparser::parse_important).is_ok();
        input.expect_exhausted()?;
        Ok(vec![Declaration {
            name,
            raw,
            values: parsed,
            important,
        }])
    }
}
fn parse_deferred(name: &str, source: &str) -> Option<Vec<(values::Property, Value)>> {
    let mut input = Parser::new(source);
    let parsed = values::parse_values(name, &mut input).ok()?;
    input.expect_exhausted().ok()?;
    Some(parsed)
}

fn substitute_variables(source: &str, variables: &HashMap<String, String>) -> Option<String> {
    fn resolve(
        source: &str,
        variables: &HashMap<String, String>,
        stack: &mut std::collections::HashSet<String>,
        depth: usize,
        expanded: &mut usize,
    ) -> Option<String> {
        if depth > MAX_VAR_DEPTH || source.len() > MAX_CUSTOM_PROPERTY_BYTES {
            return None;
        }
        let bytes = source.as_bytes();
        let mut output = String::with_capacity(source.len());
        let mut cursor = 0;
        let mut i = 0;
        let mut quote = None;
        while i < bytes.len() {
            let byte = bytes[i];
            if let Some(end_quote) = quote {
                if byte == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                if byte == end_quote {
                    quote = None;
                }
                i += 1;
                continue;
            }
            if byte == b'\'' || byte == b'"' {
                quote = Some(byte);
                i += 1;
                continue;
            }
            if byte == b'/' && bytes.get(i + 1) == Some(&b'*') {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
                continue;
            }
            let is_var = i + 4 <= bytes.len()
                && source
                    .get(i..i + 4)
                    .is_some_and(|s| s.eq_ignore_ascii_case("var("))
                && (i == 0
                    || !(bytes[i - 1].is_ascii_alphanumeric()
                        || matches!(bytes[i - 1], b'_' | b'-')));
            if !is_var {
                i += source[i..].chars().next()?.len_utf8();
                continue;
            }
            let open = i + 3;
            let close = matching_parenthesis(source, open)?;
            let (name, fallback) = split_var_arguments(source.get(open + 1..close)?)?;
            let name = name.trim();
            if !name.starts_with("--") || name.len() <= 2 {
                return None;
            }
            output.push_str(source.get(cursor..i)?);
            let replacement = if let Some(value) = variables.get(name) {
                if stack.insert(name.to_owned()) {
                    let resolved = resolve(value, variables, stack, depth + 1, expanded);
                    stack.remove(name);
                    resolved.or_else(|| {
                        fallback.and_then(|fallback| {
                            resolve(fallback, variables, stack, depth + 1, expanded)
                        })
                    })
                } else {
                    fallback.and_then(|fallback| {
                        resolve(fallback, variables, stack, depth + 1, expanded)
                    })
                }
            } else {
                fallback
                    .and_then(|fallback| resolve(fallback, variables, stack, depth + 1, expanded))
            }?;
            *expanded = expanded.saturating_add(replacement.len());
            if *expanded > MAX_CUSTOM_PROPERTY_BYTES {
                return None;
            }
            output.push_str(&replacement);
            i = close + 1;
            cursor = i;
        }
        output.push_str(source.get(cursor..)?);
        *expanded = expanded.saturating_add(output.len());
        (*expanded <= MAX_CUSTOM_PROPERTY_BYTES).then_some(output)
    }

    let mut expanded = 0;
    resolve(
        source,
        variables,
        &mut std::collections::HashSet::new(),
        0,
        &mut expanded,
    )
}

fn matching_parenthesis(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    let mut quote = None;
    while i < bytes.len() {
        let byte = bytes[i];
        if let Some(end_quote) = quote {
            if byte == b'\\' {
                i = (i + 2).min(bytes.len());
                continue;
            }
            if byte == end_quote {
                quote = None;
            }
            i += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'(' => depth += 1,
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i);
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn split_var_arguments(source: &str) -> Option<(&str, Option<&str>)> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    let mut quote = None;
    while i < bytes.len() {
        let byte = bytes[i];
        if let Some(end_quote) = quote {
            if byte == b'\\' {
                i = (i + 2).min(bytes.len());
                continue;
            }
            if byte == end_quote {
                quote = None;
            }
            i += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'(' => depth += 1,
            b')' => depth = depth.checked_sub(1)?,
            b',' if depth == 0 => return Some((&source[..i], Some(&source[i + 1..]))),
            _ => {}
        }
        i += 1;
    }
    (depth == 0).then_some((source, None))
}

fn declarations(input: &mut Parser<'_>, diagnostics: &mut Diagnostics) -> Vec<Declaration> {
    let mut result = vec![];
    let mut parser = Declarations::default();
    for (count, item) in RuleBodyParser::new(input, &mut parser).enumerate() {
        if count == MAX_DECLARATIONS {
            diagnostics.limited = true;
            break;
        }
        match item {
            Ok(declarations) => result.extend(declarations),
            Err(_) => diagnostics.ignored += 1,
        }
    }
    for (property, count) in parser.unsupported_properties {
        record_unsupported(&mut diagnostics.unsupported_properties, &property, count);
    }
    result
}

fn record_unsupported(target: &mut HashMap<String, usize>, name: &str, count: usize) {
    let bounded_name: String = name.chars().take(128).collect();
    if target.len() < 512 || target.contains_key(&bounded_name) {
        *target.entry(bounded_name).or_default() += count;
    }
}
/// A legacy HTML length attribute such as `width="85%"` or `cellpadding="4"`.
fn attribute_length(source: &str) -> Option<Length> {
    let source = source.trim();
    let (number, percent) = match source.strip_suffix('%') {
        Some(number) => (number, true),
        None => (source.strip_suffix("px").unwrap_or(source), false),
    };
    let value = number
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)?;
    Some(if percent {
        Length::Percent(value.min(100.0) / 100.0)
    } else {
        Length::Px(value.min(10_000.0))
    })
}

/// The built-in presentation of HTML elements, before author styles. `parent`
/// is the parent element, `table` a cell's table, and `quirks` whether the
/// document lacks a standards doctype. Returns the default border color when
/// it differs from the text color.
fn user_agent(
    element: &Element,
    parent: Option<&Element>,
    table: Option<&Element>,
    quirks: bool,
    style: &mut ComputedStyle,
) -> Option<Color> {
    let tag = element.name.local.as_str();
    let mut border_color = None;
    if let Some(direction) = element.attribute("dir") {
        style.direction = match direction.trim().to_ascii_lowercase().as_str() {
            "rtl" => Direction::Rtl,
            "auto" => Direction::Auto,
            "ltr" => Direction::Ltr,
            _ => style.direction,
        };
    } else if let Some(direction) = element.attribute("lang").and_then(language_direction) {
        // Many real-world pages, including Hebrew news sites, declare the
        // document language but omit dir on the content subtree. Treat the
        // language as a UA-level direction hint; author CSS still overrides it
        // below, and an explicit dir always wins this fallback.
        style.direction = direction;
    }
    // Indentation that belongs at the start of the line in the element's direction.
    let start = if style.direction == Direction::Rtl {
        1
    } else {
        3
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
            | "dl"
            | "dt"
            | "dd"
            | "figure"
            | "figcaption"
            | "form"
            | "center"
            | "address"
            | "caption"
            | "details"
            | "summary"
            | "fieldset"
            | "hgroup"
            | "menu"
    ) {
        style.display = Display::Block;
    }
    if matches!(
        tag,
        "p" | "pre" | "blockquote" | "hr" | "dl" | "figure" | "figcaption" | "address"
    ) {
        style.margin[2] = Length::Px(12.0);
    }
    let centered = |element: &Element| {
        element
            .attribute("align")
            .is_some_and(|align| align.trim().eq_ignore_ascii_case("center"))
    };
    // Legacy presentational attributes; author styles still override them.
    if matches!(tag, "th" | "caption") {
        style.text_align = TextAlign::Center;
    }
    if let Some(align) = element.attribute("align").filter(|_| {
        matches!(
            tag,
            "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "td" | "th" | "tr" | "caption"
        )
    }) {
        match align.trim().to_ascii_lowercase().as_str() {
            "left" => style.text_align = TextAlign::Left,
            "right" => style.text_align = TextAlign::Right,
            "center" | "middle" => style.text_align = TextAlign::Center,
            _ => {}
        }
    }
    if let Some(background) = element
        .attribute("bgcolor")
        .filter(|_| matches!(tag, "body" | "table" | "tr" | "td" | "th"))
        .and_then(values::attribute_color)
    {
        style.background = background;
    }
    if let Some(ink) = match tag {
        "body" => element.attribute("text"),
        "font" => element.attribute("color"),
        _ => None,
    }
    .and_then(values::attribute_color)
    {
        style.color = ink;
    }
    if let Some(width) = element
        .attribute("width")
        .filter(|_| matches!(tag, "table" | "td" | "th"))
        .and_then(attribute_length)
    {
        style.width = width;
    }
    let table_border = |table: &Element| {
        table
            .attribute("border")
            .and_then(attribute_length)
            .filter(|width| matches!(width, Length::Px(width) if *width > 0.0))
    };
    match tag {
        "body" => style.margin = [Length::Px(8.0); 4],
        // A quiet neutral rule that reads on light and dark pages alike.
        "hr" => style.color = Color(150, 156, 140, 255),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            // Sizes follow the surrounding text, so a heading inside small
            // print stays in proportion. More space above than below keeps
            // each heading with the section it introduces.
            let level = (tag.as_bytes()[1] - b'1') as usize;
            style.font_size *= [2.0, 1.6, 1.35, 1.18, 1.06, 1.0][level];
            style.font_weight = 700.0;
            style.margin[0] = Length::Em(0.6);
            style.margin[2] = Length::Em(0.4);
        }
        "ul" | "ol" | "menu" => {
            // Markers hang in this padding, outside the item text.
            style.padding[start] = Length::Px(40.0);
            if !parent.is_some_and(|parent| {
                matches!(parent.name.local.as_str(), "li" | "ul" | "ol" | "menu")
            }) {
                style.margin[2] = Length::Px(12.0);
            }
        }
        "dd" => style.margin[start] = Length::Px(40.0),
        "blockquote" | "figure" => {
            style.margin[1] = Length::Px(40.0);
            style.margin[3] = Length::Px(40.0);
        }
        "center" => style.text_align = TextAlign::Center,
        "table" => {
            style.display = Display::Table;
            if quirks {
                // Legacy pages expect tables not to inherit text alignment or
                // fonts, as the HTML quirks-mode stylesheet specifies.
                let initial = ComputedStyle::default();
                style.text_align = initial.text_align;
                style.font_size = initial.font_size;
                style.font_weight = initial.font_weight;
                style.italic = initial.italic;
                style.line_height = initial.line_height;
                style.white_space = initial.white_space;
            }
            // Spacing between and around cells, as HTML `cellspacing` sets it.
            let spacing = element
                .attribute("cellspacing")
                .and_then(attribute_length)
                .filter(|spacing| matches!(spacing, Length::Px(_)))
                .unwrap_or(Length::Px(2.0));
            style.row_gap = spacing;
            style.column_gap = spacing;
            if let Some(width) = table_border(element) {
                style.set_border(width, None);
                border_color = Some(Color(128, 128, 128, 255));
            }
            // Separate a table from what follows, as paragraphs are, except
            // for layout tables nested in a cell.
            if !parent.is_some_and(|p| matches!(p.name.local.as_str(), "td" | "th")) {
                style.margin[2] = Length::Px(12.0);
            }
            // Browsers center a table inside `<center>` or an `align=center` parent.
            if centered(element) || parent.is_some_and(|p| p.is_html_tag("center") || centered(p)) {
                style.margin[1] = Length::Auto;
                style.margin[3] = Length::Auto;
            }
        }
        "tr" => style.display = Display::TableRow,
        "td" | "th" => {
            style.display = Display::TableCell;
            let span = |name| {
                element
                    .attribute(name)
                    .and_then(|value| value.trim().parse::<u16>().ok())
                    .map_or(1, |value| value.clamp(1, 64) as u8)
            };
            style.column_span = span("colspan");
            style.row_span = span("rowspan");
            style.padding = match table
                .and_then(|table| table.attribute("cellpadding"))
                .and_then(attribute_length)
            {
                Some(Length::Px(padding)) => [Length::Px(padding); 4],
                _ => [
                    Length::Em(0.2),
                    Length::Em(0.4),
                    Length::Em(0.2),
                    Length::Em(0.4),
                ],
            };
            if table.and_then(table_border).is_some() {
                style.set_border(Length::Px(1.0), None);
                border_color = Some(Color(128, 128, 128, 255));
            }
            if element.attribute("nowrap").is_some() {
                style.white_space = WhiteSpace::NoWrap;
            }
            if tag == "th" {
                style.font_weight = 700.0;
            }
        }
        "col" | "colgroup" => style.display = Display::None,
        "b" | "strong" | "dt" => style.font_weight = 700.0,
        "i" | "em" | "cite" | "dfn" | "var" | "address" => style.italic = true,
        "u" | "ins" => style.underline = true,
        "s" | "del" | "strike" => style.strike = true,
        "a" if element.attribute("href").is_some() => {
            style.color = Color(86, 105, 51, 255);
            style.underline = true;
        }
        "small" => style.font_size = (style.font_size * 0.85).max(1.0),
        "sup" | "sub" => {
            style.font_size = (style.font_size * 0.75).max(1.0);
            style.vertical_align = if tag == "sup" {
                VerticalAlign::Super
            } else {
                VerticalAlign::Sub
            };
        }
        "mark" => {
            style.background = Color(255, 232, 128, 255);
            style.color = Color(38, 44, 32, 255);
        }
        "code" | "kbd" | "samp" | "tt" | "pre" => {
            // Code inside a code block keeps the block's size.
            if !style.monospace {
                style.font_size = (style.font_size * 15.0 / 17.0).max(1.0);
            }
            style.monospace = true;
            style.background = Color(239, 240, 233, 255);
            if tag == "pre" {
                style.white_space = WhiteSpace::Pre;
                style.padding = [
                    Length::Em(0.7),
                    Length::Em(0.9),
                    Length::Em(0.7),
                    Length::Em(0.9),
                ];
                style.border_radius = [Length::Px(6.0); 4];
            }
        }
        "dialog" if element.attribute("open").is_none() => style.display = Display::None,
        "datalist" => style.display = Display::None,
        _ => {}
    }
    border_color
}

fn language_direction(language: &str) -> Option<Direction> {
    let primary = language
        .trim()
        .split(['-', '_'])
        .next()
        .filter(|language| !language.is_empty())?
        .to_ascii_lowercase();
    if matches!(
        primary.as_str(),
        "ar" | "ckb"
            | "dv"
            | "fa"
            | "he"
            | "iw"
            | "ku"
            | "nqo"
            | "ps"
            | "sd"
            | "syr"
            | "ug"
            | "ur"
            | "yi"
    ) {
        Some(Direction::Rtl)
    } else if matches!(
        primary.as_str(),
        "af" | "am"
            | "az"
            | "bg"
            | "bn"
            | "ca"
            | "cs"
            | "da"
            | "de"
            | "el"
            | "en"
            | "es"
            | "et"
            | "eu"
            | "fi"
            | "fr"
            | "gl"
            | "gu"
            | "hi"
            | "hr"
            | "hu"
            | "hy"
            | "id"
            | "is"
            | "it"
            | "ja"
            | "ka"
            | "kk"
            | "km"
            | "kn"
            | "ko"
            | "lo"
            | "lt"
            | "lv"
            | "mk"
            | "ml"
            | "mn"
            | "mr"
            | "ms"
            | "my"
            | "ne"
            | "nl"
            | "no"
            | "pa"
            | "pl"
            | "pt"
            | "ro"
            | "ru"
            | "sk"
            | "sl"
            | "sq"
            | "sr"
            | "sv"
            | "sw"
            | "ta"
            | "te"
            | "th"
            | "tr"
            | "uk"
            | "vi"
            | "zh"
    ) {
        Some(Direction::Ltr)
    } else {
        None
    }
}
