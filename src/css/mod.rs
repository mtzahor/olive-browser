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
use selectors::{Key, Selector};
use std::collections::HashMap;
pub use values::{
    Color, ComputedStyle, Direction, Display, Length, LineHeight, TextAlign, WhiteSpace,
};
use values::{Declaration, PROPERTIES, Value};

pub const MAX_CSS_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RULES: usize = 32_768;
const MAX_DECLARATIONS: usize = 512;
const MAX_SELECTORS: usize = 128;
const MATCH_BUDGET: usize = 8_000_000;

#[derive(Clone, Debug)]
struct Rule {
    selectors: Vec<Selector>,
    declarations: Vec<Declaration>,
}

/// Counts rejected syntax/features and reports an exceeded processing limit.
#[derive(Clone, Copy, Debug, Default)]
pub struct Diagnostics {
    pub ignored: usize,
    pub limited: bool,
}
/// Parsed author styles, ordered as they appeared in the document.
#[derive(Default, Debug)]
pub struct Stylesheet {
    rules: Vec<Rule>,
    index: HashMap<Key, Vec<(usize, usize)>>,
    unindexed: Vec<(usize, usize)>,
    pub diagnostics: Diagnostics,
    bytes: usize,
}
/// A per-document work budget. Reuse it across the whole style traversal.
#[derive(Debug)]
pub struct StyleBudget {
    matches: usize,
    inline_bytes: usize,
    pub limited: bool,
    pub ignored: usize,
}
impl Default for StyleBudget {
    fn default() -> Self {
        Self {
            matches: MATCH_BUDGET,
            inline_bytes: 0,
            limited: false,
            ignored: 0,
        }
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
    /// fragments, non-CSS types and media other than plain screen/all are ignored.
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
            if !applicable_style(element) {
                continue;
            }
            if element.name.local.as_ref() == "link" {
                if let Some(source) = sources
                    .get(&id)
                    .filter(|source| element.attribute("href") == Some(source.reference.as_str()))
                {
                    sheet.append(&source.source);
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
            sheet.append(&source);
        }
        sheet
    }
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }
    fn append(&mut self, source: &str) {
        if self.bytes.saturating_add(source.len()) > MAX_CSS_BYTES {
            self.diagnostics.limited = true;
            return;
        }
        self.bytes += source.len();
        let mut input = Parser::new(source);
        let mut remaining = MAX_RULES - self.rules.len();
        let mut parser = Rules {
            diagnostics: &mut self.diagnostics,
            remaining: &mut remaining,
            depth: 0,
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
                                self.index.entry(key).or_default().push(entry);
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
        user_agent(element, &mut style);
        type Priority = (bool, bool, (u16, u16, u16));
        let mut winners: [Option<(Priority, Value)>; PROPERTIES.len()] = [None; PROPERTIES.len()];
        let mut consider = |declarations: &[Declaration], inline, specificity| {
            for declaration in declarations {
                let priority = (declaration.important, inline, specificity);
                let winner = &mut winners[declaration.property as usize];
                if winner.is_none_or(|(previous, _)| priority >= previous) {
                    *winner = Some((priority, declaration.value));
                }
            }
        };
        let mut candidates = Vec::new();
        if budget.matches > 0 {
            candidates.extend_from_slice(&self.unindexed);
            let mut collect = |key| {
                if let Some(entries) = self.index.get(&key) {
                    candidates.extend_from_slice(entries);
                }
            };
            collect(Key::Tag(element.name.local.as_ref().to_ascii_lowercase()));
            if let Some(id) = element.attribute("id") {
                collect(Key::Id(id.to_ascii_lowercase()));
            }
            if let Some(classes) = element.attribute("class") {
                let classes: std::collections::HashSet<_> = classes
                    .split_ascii_whitespace()
                    .map(str::to_ascii_lowercase)
                    .collect();
                for class in classes {
                    collect(Key::Class(class));
                }
            }
            for attribute in &element.attributes {
                if attribute.name.ns.is_empty() {
                    collect(Key::Attribute(attribute.name.local.to_string()));
                }
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        for (rule_index, selector_index) in candidates {
            if budget.matches == 0 {
                budget.limited = true;
                break;
            }
            let rule = &self.rules[rule_index];
            let selector = &rule.selectors[selector_index];
            if selector.matches(doc, id, &mut budget.matches) {
                consider(&rule.declarations, false, selector.specificity);
            }
        }
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
                consider(&declarations, true, (0, 0, 0));
            } else {
                budget.limited = true;
            }
        }
        // Font size precedes dependent lengths; color precedes currentColor.
        let mut root_font = root_font;
        for property in PROPERTIES {
            if let Some((_, value)) = winners[property as usize] {
                values::apply(&mut style, parent, root_font, property, value);
            }
            if property == values::Property::FontSize && element.name.local.as_ref() == "html" {
                root_font = style.font_size;
            }
            if property == values::Property::Color {
                style.border_color = style.color;
            }
        }
        // Resolve em/rem now so explicit inheritance preserves computed lengths.
        let font = style.font_size;
        let computed_length = |v| match v {
            Length::Em(_) | Length::Rem(_) => Length::Px(v.resolve(0.0, font, root_font)),
            _ => v,
        };
        style.margin = style.margin.map(computed_length);
        style.padding = style.padding.map(computed_length);
        style.width = computed_length(style.width);
        style.max_width = computed_length(style.max_width);
        style.height = computed_length(style.height);
        style.min_height = computed_length(style.min_height);
        style.border_width = computed_length(style.border_width);
        style.border_radius = computed_length(style.border_radius);
        if element.attribute("hidden").is_some() {
            style.display = Display::None;
        }
        style
    }
}

/// Whether this HTML style/link applies to Olive's screen media subset.
pub fn applicable_style(element: &Element) -> bool {
    if element.name.ns.as_ref() != "http://www.w3.org/1999/xhtml"
        || !matches!(element.name.local.as_ref(), "style" | "link")
        || element.attribute("disabled").is_some()
        || element
            .attribute("type")
            .is_some_and(|s| !s.trim().is_empty() && !s.trim().eq_ignore_ascii_case("text/css"))
        || element.attribute("media").is_some_and(|s| !screen_media(s))
    {
        return false;
    }
    if element.name.local.as_ref() == "link" {
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
}
// Viewport-dependent features remain unsupported, rather than assuming a
// desktop width and applying the wrong rules when the user resizes the window.
fn screen_media(source: &str) -> bool {
    source.trim().is_empty()
        || source.split(',').any(|query| {
            let words = query
                .split_ascii_whitespace()
                .map(str::to_ascii_lowercase)
                .collect::<Vec<_>>();
            matches!(
                words
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .as_slice(),
                ["screen"] | ["all"] | ["only", "screen"] | ["only", "all"] | ["not", "print"]
            )
        })
}
impl<'i> AtRuleParser<'i> for Rules<'_> {
    type Prelude = bool;
    type AtRule = Vec<Rule>;
    type Error = ();
    fn parse_prelude(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<bool, cssparser::ParseError<()>> {
        if !name.eq_ignore_ascii_case("media") {
            return Err(cssparser::ParseError::custom(()));
        }
        let start = input.position();
        while input.next().is_ok() {}
        let source = input.slice_from(start);
        Ok(!source.trim().is_empty() && screen_media(source))
    }
    fn parse_block(
        &mut self,
        applies: bool,
        _: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Vec<Rule>, cssparser::ParseError<()>> {
        if !applies {
            while input.next().is_ok() {}
            return Ok(vec![]);
        }
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
        input.parse_comma_separated(|input| {
            count += 1;
            if count > MAX_SELECTORS {
                self.diagnostics.limited = true;
                return Err(cssparser::ParseError::custom(()));
            }
            Selector::parse(input)
        })
    }
    fn parse_block(
        &mut self,
        selectors: Self::Prelude,
        _: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Vec<Rule>, cssparser::ParseError<()>> {
        if *self.remaining == 0 {
            self.diagnostics.limited = true;
            return Ok(vec![]);
        }
        *self.remaining -= 1;
        Ok(vec![Rule {
            selectors,
            declarations: declarations(input, self.diagnostics),
        }])
    }
}
struct Declarations;
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
        let values = input.parse_until_before(Delimiter::Bang, |p| {
            let values = values::parse_values(&name.to_ascii_lowercase(), p)?;
            p.expect_exhausted()?;
            Ok(values)
        })?;
        let important = input.try_parse(cssparser::parse_important).is_ok();
        input.expect_exhausted()?;
        Ok(values
            .into_iter()
            .map(|(property, value)| Declaration {
                property,
                value,
                important,
            })
            .collect())
    }
}
fn declarations(input: &mut Parser<'_>, diagnostics: &mut Diagnostics) -> Vec<Declaration> {
    let mut result = vec![];
    for (count, item) in RuleBodyParser::new(input, &mut Declarations).enumerate() {
        if count == MAX_DECLARATIONS {
            diagnostics.limited = true;
            break;
        }
        match item {
            Ok(declarations) => result.extend(declarations),
            Err(_) => diagnostics.ignored += 1,
        }
    }
    result
}
fn user_agent(element: &Element, style: &mut ComputedStyle) {
    let tag = element.name.local.as_ref();
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
            | "table"
            | "tr"
            | "dl"
            | "dt"
            | "dd"
            | "figure"
            | "figcaption"
            | "form"
    ) {
        style.display = Display::Block;
    }
    if matches!(
        tag,
        "p" | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "li"
            | "pre"
            | "blockquote"
            | "hr"
            | "tr"
            | "dt"
            | "dd"
            | "figcaption"
    ) {
        style.margin[2] = Length::Px(12.0);
    }
    match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            style.font_size =
                [34.0, 27.0, 23.0, 20.0, 18.0, 17.0][(tag.as_bytes()[1] - b'1') as usize];
            style.font_weight = 700.0;
        }
        "b" | "strong" | "th" | "dt" => style.font_weight = 700.0,
        "i" | "em" | "cite" | "dfn" => style.italic = true,
        "u" => style.underline = true,
        "s" | "del" => style.strike = true,
        "a" if element.attribute("href").is_some() => {
            style.color = Color(86, 105, 51, 255);
            style.underline = true;
        }
        "small" => style.font_size = (style.font_size * 0.85).max(1.0),
        "code" | "kbd" | "samp" | "pre" => {
            style.monospace = true;
            style.font_size = 15.0;
            style.background = Color(239, 240, 233, 255);
            if tag == "pre" {
                style.white_space = WhiteSpace::Pre;
            }
        }
        _ => {}
    }
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
