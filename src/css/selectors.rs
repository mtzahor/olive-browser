use crate::{Document, Element, NodeId, NodeKind};
use cssparser::{Parser, Token};

type Error = cssparser::ParseError<()>;
const MAX_NESTING: usize = 8;

#[derive(Clone, Debug)]
enum Simple {
    Tag(String),
    Id(String),
    Class(String),
    Attribute(Attribute),
    Pseudo(String),
    Nth(i32, i32, bool, bool), // a, b, from end, of type
    Any(Vec<Selector>, bool),  // selector list, negated
    Universal,
}
#[derive(Clone, Debug)]
struct Attribute {
    name: String,
    operator: Option<char>,
    value: String,
    insensitive: bool,
}
impl Attribute {
    fn parse(input: &mut Parser<'_>) -> Result<Self, Error> {
        let name = input.expect_ident()?.to_ascii_lowercase();
        if input.is_exhausted() {
            return Ok(Self {
                name,
                operator: None,
                value: String::new(),
                insensitive: false,
            });
        }
        let operator = match input.next()? {
            Token::Delim('=') => '=',
            Token::IncludeMatch => '~',
            Token::DashMatch => '|',
            Token::PrefixMatch => '^',
            Token::SuffixMatch => '$',
            Token::SubstringMatch => '*',
            _ => return Err(Error::custom(())),
        };
        let value = input.expect_ident_or_string()?.to_string();
        let insensitive = if input.is_exhausted() {
            // HTML enumerated attributes are ASCII case insensitive by default.
            matches!(name.as_str(), "type" | "dir" | "rel" | "method" | "enctype")
        } else {
            match input.expect_ident()?.to_ascii_lowercase().as_str() {
                "i" => true,
                "s" => false,
                _ => return Err(Error::custom(())),
            }
        };
        input.expect_exhausted()?;
        Ok(Self {
            name,
            operator: Some(operator),
            value,
            insensitive,
        })
    }
    fn matches(&self, element: &Element) -> bool {
        let Some(actual) = element.attribute(&self.name) else {
            return false;
        };
        let (actual, expected) = if self.insensitive {
            (actual.to_ascii_lowercase(), self.value.to_ascii_lowercase())
        } else {
            (actual.to_owned(), self.value.clone())
        };
        match self.operator {
            None => true,
            Some('=') => actual == expected,
            Some('~') => {
                !expected.is_empty() && actual.split_ascii_whitespace().any(|s| s == expected)
            }
            Some('|') => {
                actual == expected
                    || actual
                        .strip_prefix(&expected)
                        .is_some_and(|s| s.starts_with('-'))
            }
            Some('^') => !expected.is_empty() && actual.starts_with(&expected),
            Some('$') => !expected.is_empty() && actual.ends_with(&expected),
            Some('*') => !expected.is_empty() && actual.contains(&expected),
            _ => false,
        }
    }
}
#[derive(Clone, Copy, Debug)]
enum Relation {
    Descendant,
    Child,
    Adjacent,
    Sibling,
}
#[derive(Clone, Debug)]
struct Compound {
    simple: Vec<Simple>,
    relation: Relation,
}
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(super) enum Key {
    Id(String),
    Class(String),
    Attribute(String),
    Tag(String),
}
#[derive(Clone, Debug)]
pub(super) struct Selector {
    parts: Vec<Compound>,
    pub specificity: (u16, u16, u16),
}
impl Selector {
    pub fn parse(input: &mut Parser<'_>) -> Result<Self, Error> {
        Self::parse_nested(input, 0)
    }
    fn parse_nested(input: &mut Parser<'_>, depth: usize) -> Result<Self, Error> {
        if depth > MAX_NESTING {
            return Err(Error::custom(()));
        }
        let mut result = Self {
            parts: vec![],
            specificity: (0, 0, 0),
        };
        let mut simple = vec![];
        let mut relation = Relation::Descendant;
        let mut whitespace = false;
        while let Ok(token) = input.next_including_whitespace().cloned() {
            if matches!(token, Token::WhiteSpace(_)) {
                whitespace = !simple.is_empty();
                continue;
            }
            if let Token::Delim(c @ ('>' | '+' | '~')) = token {
                if simple.is_empty() || result.parts.len() >= 31 {
                    return Err(Error::custom(()));
                }
                result.parts.push(Compound {
                    simple: std::mem::take(&mut simple),
                    relation,
                });
                relation = match c {
                    '>' => Relation::Child,
                    '+' => Relation::Adjacent,
                    _ => Relation::Sibling,
                };
                whitespace = false;
                continue;
            }
            if whitespace {
                result.parts.push(Compound {
                    simple: std::mem::take(&mut simple),
                    relation,
                });
                relation = Relation::Descendant;
                whitespace = false;
            }
            let value = match token {
                Token::Ident(v) if simple.is_empty() => {
                    result.specificity.2 = result.specificity.2.saturating_add(1);
                    Simple::Tag(v.to_ascii_lowercase())
                }
                Token::Delim('*') if simple.is_empty() => Simple::Universal,
                Token::IDHash(v) => {
                    result.specificity.0 = result.specificity.0.saturating_add(1);
                    Simple::Id(v.to_string())
                }
                Token::Delim('.') => {
                    result.specificity.1 = result.specificity.1.saturating_add(1);
                    Simple::Class(match input.next_including_whitespace()? {
                        Token::Ident(name) => name.to_string(),
                        _ => return Err(Error::custom(())),
                    })
                }
                Token::SquareBracketBlock => {
                    result.specificity.1 = result.specificity.1.saturating_add(1);
                    Simple::Attribute(input.parse_nested_block(Attribute::parse)?)
                }
                Token::Colon => match input.next_including_whitespace()?.clone() {
                    Token::Ident(name) => {
                        let name = name.to_ascii_lowercase();
                        if !matches!(
                            name.as_str(),
                            "root"
                                | "empty"
                                | "first-child"
                                | "last-child"
                                | "only-child"
                                | "link"
                                | "any-link"
                                | "visited"
                                | "hover"
                                | "active"
                                | "focus"
                                | "focus-visible"
                                | "focus-within"
                                | "checked"
                        ) {
                            return Err(Error::custom(()));
                        }
                        result.specificity.1 = result.specificity.1.saturating_add(1);
                        Simple::Pseudo(name)
                    }
                    Token::Function(name) => {
                        let name = name.to_ascii_lowercase();
                        match name.as_str() {
                            "is" | "where" | "not" => {
                                let list = input.parse_nested_block(|p| {
                                    let mut count = 0;
                                    let mut parse_one = |p: &mut Parser<'_>| {
                                        count += 1;
                                        if count > 64 {
                                            return Err(Error::custom(()));
                                        }
                                        Self::parse_nested(p, depth + 1)
                                    };
                                    let list = if name == "not" {
                                        p.parse_comma_separated(&mut parse_one)?
                                    } else {
                                        p.parse_comma_separated_ignoring_errors(&mut parse_one)
                                    };
                                    if count > 64 {
                                        return Err(Error::custom(()));
                                    }
                                    Ok(list)
                                })?;
                                if name != "where" {
                                    let specificity = list
                                        .iter()
                                        .map(|s| s.specificity)
                                        .max()
                                        .unwrap_or_default();
                                    result.specificity.0 =
                                        result.specificity.0.saturating_add(specificity.0);
                                    result.specificity.1 =
                                        result.specificity.1.saturating_add(specificity.1);
                                    result.specificity.2 =
                                        result.specificity.2.saturating_add(specificity.2);
                                }
                                Simple::Any(list, name == "not")
                            }
                            "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type" => {
                                let (a, b) = input.parse_nested_block(|p| {
                                    cssparser::parse_nth(p).map_err(Into::into)
                                })?;
                                result.specificity.1 = result.specificity.1.saturating_add(1);
                                Simple::Nth(a, b, name.contains("last"), name.ends_with("of-type"))
                            }
                            _ => return Err(Error::custom(())),
                        }
                    }
                    _ => return Err(Error::custom(())),
                },
                _ => return Err(Error::custom(())),
            };
            simple.push(value);
            if result.parts.len() >= 32 || simple.len() > 64 {
                return Err(Error::custom(()));
            }
        }
        if simple.is_empty() {
            return Err(Error::custom(()));
        }
        result.parts.push(Compound { simple, relation });
        Ok(result)
    }

    /// A necessary rightmost condition, used to avoid testing unrelated rules.
    /// Fold index keys even in standards mode; matching still checks exact case.
    pub fn key(&self) -> Option<Key> {
        let simple = &self.parts.last()?.simple;
        simple
            .iter()
            .find_map(|s| match s {
                Simple::Id(v) => Some(Key::Id(v.to_ascii_lowercase())),
                _ => None,
            })
            .or_else(|| {
                simple.iter().find_map(|s| match s {
                    Simple::Class(v) => Some(Key::Class(v.to_ascii_lowercase())),
                    _ => None,
                })
            })
            .or_else(|| {
                simple.iter().find_map(|s| match s {
                    Simple::Attribute(attribute) => Some(Key::Attribute(attribute.name.clone())),
                    _ => None,
                })
            })
            .or_else(|| {
                simple.iter().find_map(|s| match s {
                    Simple::Tag(v) => Some(Key::Tag(v.clone())),
                    _ => None,
                })
            })
    }

    pub fn matches(&self, doc: &Document, id: NodeId, budget: &mut usize) -> bool {
        let mut pending = vec![(id, self.parts.len() - 1, None)];
        while let Some((id, index, search)) = pending.pop() {
            if !spend(budget) {
                return false;
            }
            let Some(node) = doc.node(id) else { continue };
            if let Some(relation) = search {
                if let Some(next) = related(doc, id, relation, budget) {
                    pending.push((next, index, search));
                }
            }
            let Some(element) = node.as_element() else {
                continue;
            };
            if element.name.ns.as_ref() != "http://www.w3.org/1999/xhtml" {
                continue;
            }
            let quirks = doc.quirks_mode() == crate::QuirksMode::Quirks;
            let equal = |a: &str, b: &str| {
                if quirks {
                    a.eq_ignore_ascii_case(b)
                } else {
                    a == b
                }
            };
            let matched = self.parts[index].simple.iter().all(|simple| {
                if !spend(budget) {
                    return false;
                }
                match simple {
                    Simple::Universal => true,
                    Simple::Tag(tag) => element.name.local.as_ref().eq_ignore_ascii_case(tag),
                    Simple::Id(id) => element.attribute("id").is_some_and(|v| equal(v, id)),
                    Simple::Class(class) => element
                        .attribute("class")
                        .is_some_and(|v| v.split_ascii_whitespace().any(|v| equal(v, class))),
                    Simple::Attribute(attribute) => attribute.matches(element),
                    Simple::Any(list, negated) => {
                        list.iter().any(|s| s.matches(doc, id, budget)) != *negated
                    }
                    Simple::Nth(a, b, end, typed) => nth(doc, id, *end, *typed, budget)
                        .is_some_and(|n| {
                            let delta = i64::from(n) - i64::from(*b);
                            if *a == 0 {
                                delta == 0
                            } else {
                                delta % i64::from(*a) == 0 && delta / i64::from(*a) >= 0
                            }
                        }),
                    Simple::Pseudo(name) => match name.as_str() {
                        "root" => node.parent().is_some_and(|p| {
                            matches!(doc.node(p).map(|n| &n.kind), Some(NodeKind::Document))
                        }),
                        "empty" => doc.children(id).all(|child| {
                            spend(budget)
                                && !matches!(
                                    &doc.node(child).unwrap().kind,
                                    NodeKind::Element(_) | NodeKind::Text(_)
                                )
                        }),
                        "first-child" => sibling(doc, id, false, budget).is_none(),
                        "last-child" => sibling(doc, id, true, budget).is_none(),
                        "only-child" => {
                            sibling(doc, id, false, budget).is_none()
                                && sibling(doc, id, true, budget).is_none()
                        }
                        "link" | "any-link" => {
                            matches!(element.name.local.as_ref(), "a" | "area")
                                && element.attribute("href").is_some()
                        }
                        "checked" => match element.name.local.as_ref() {
                            "option" => element.attribute("selected").is_some(),
                            "input" => {
                                element.attribute("type").is_some_and(|v| {
                                    v.eq_ignore_ascii_case("checkbox")
                                        || v.eq_ignore_ascii_case("radio")
                                }) && element.attribute("checked").is_some()
                            }
                            _ => false,
                        },
                        // A static presentation has no hover/focus or visited state.
                        _ => false,
                    },
                }
            });
            if *budget == 0 {
                return false;
            }
            if !matched {
                continue;
            }
            if index == 0 {
                return true;
            }
            let relation = self.parts[index].relation;
            if let Some(next) = related(doc, id, relation, budget) {
                let search = matches!(relation, Relation::Descendant | Relation::Sibling)
                    .then_some(relation);
                pending.push((next, index - 1, search));
            }
        }
        false
    }
}
fn spend(budget: &mut usize) -> bool {
    if *budget == 0 {
        false
    } else {
        *budget -= 1;
        true
    }
}
fn related(doc: &Document, id: NodeId, relation: Relation, budget: &mut usize) -> Option<NodeId> {
    match relation {
        Relation::Descendant | Relation::Child => doc.node(id)?.parent(),
        Relation::Adjacent | Relation::Sibling => sibling(doc, id, false, budget),
    }
}
fn sibling(doc: &Document, id: NodeId, next: bool, budget: &mut usize) -> Option<NodeId> {
    let mut node = doc.node(id)?;
    loop {
        let id = if next {
            node.next_sibling
        } else {
            node.previous_sibling
        }?;
        if !spend(budget) {
            return None;
        }
        node = doc.node(id)?;
        if node.as_element().is_some() {
            return Some(id);
        }
    }
}
fn nth(doc: &Document, id: NodeId, end: bool, typed: bool, budget: &mut usize) -> Option<i32> {
    let element = doc.node(id)?.as_element()?;
    let mut count = 1;
    let mut current = id;
    while let Some(previous) = sibling(doc, current, end, budget) {
        current = previous;
        let other = doc.node(current)?.as_element()?;
        if !typed || other.name == element.name {
            count += 1;
        }
    }
    Some(count)
}
