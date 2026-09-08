//! An inert CSS subset for documents. Tokenization and error recovery use
//! Servo's `cssparser`; Olive implements selectors, property values and cascade.
//! No CSS construct fetches a resource. Unsupported rules/declarations are skipped.
mod selectors;
mod values;

use crate::{Document, Element, ExternalSource, NodeId, NodeKind};
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, Delimiter, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser,
};
use selectors::Selector;
use std::collections::HashMap;
pub use values::{Color, ComputedStyle, Display, Length, LineHeight, TextAlign, WhiteSpace};
use values::{Declaration, PROPERTIES, Value};

pub const MAX_CSS_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RULES: usize = 16_384;
const MAX_DECLARATIONS: usize = 128;
const MAX_SELECTORS: usize = 64;
const MATCH_BUDGET: usize = 2_000_000;

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
        let mut input = ParserInput::new(source);
        let mut input = Parser::new(&mut input);
        let mut parser = Rules {
            diagnostics: &mut self.diagnostics,
        };
        let mut ignored = 0;
        let mut limited = false;
        for result in StyleSheetParser::new(&mut input, &mut parser) {
            if self.rules.len() == MAX_RULES {
                limited = true;
                break;
            }
            match result {
                Ok(rule) => self.rules.push(rule),
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
        for rule in &self.rules {
            if budget.matches == 0 {
                budget.limited = true;
                break;
            }
            let specificity = rule
                .selectors
                .iter()
                .filter_map(|s| {
                    s.matches(doc, id, &mut budget.matches)
                        .then_some(s.specificity)
                })
                .max();
            if let Some(specificity) = specificity {
                consider(&rule.declarations, false, specificity);
            }
        }
        if let Some(inline) = element.attribute("style") {
            if self
                .bytes
                .saturating_add(budget.inline_bytes)
                .saturating_add(inline.len())
                <= MAX_CSS_BYTES
            {
                budget.inline_bytes += inline.len();
                let mut input = ParserInput::new(inline);
                let mut diagnostics = Diagnostics::default();
                let declarations = declarations(&mut Parser::new(&mut input), &mut diagnostics);
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
        || element.attribute("media").is_some_and(|s| {
            !s.trim().is_empty()
                && !s
                    .split(',')
                    .any(|s| matches!(s.trim().to_ascii_lowercase().as_str(), "screen" | "all"))
        })
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
}
impl<'i> AtRuleParser<'i> for Rules<'_> {
    type Prelude = ();
    type AtRule = Rule;
    type Error = ();
}
impl<'i> QualifiedRuleParser<'i> for Rules<'_> {
    type Prelude = Vec<Selector>;
    type QualifiedRule = Rule;
    type Error = ();
    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, ()>> {
        let mut count = 0;
        input.parse_comma_separated(|input| {
            count += 1;
            if count > MAX_SELECTORS {
                self.diagnostics.limited = true;
                return Err(input.new_custom_error(()));
            }
            Selector::parse(input)
        })
    }
    fn parse_block<'t>(
        &mut self,
        selectors: Self::Prelude,
        _: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Rule, cssparser::ParseError<'i, ()>> {
        Ok(Rule {
            selectors,
            declarations: declarations(input, self.diagnostics),
        })
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
    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _: &ParserState,
    ) -> Result<Self::Declaration, cssparser::ParseError<'i, ()>> {
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
fn declarations(input: &mut Parser<'_, '_>, diagnostics: &mut Diagnostics) -> Vec<Declaration> {
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
