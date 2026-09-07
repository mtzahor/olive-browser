use crate::{Document, NodeId};
use cssparser::{Parser, Token};

#[derive(Clone, Debug)]
enum Simple {
    Tag(String),
    Id(String),
    Class(String),
    Universal,
}
#[derive(Clone, Copy, Debug)]
enum Relation {
    Descendant,
    Child,
}
#[derive(Clone, Debug)]
struct Compound {
    simple: Vec<Simple>,
    relation: Relation,
}
#[derive(Clone, Debug)]
pub(super) struct Selector {
    parts: Vec<Compound>,
    pub specificity: (u16, u16, u16),
}
impl Selector {
    pub fn parse<'i>(input: &mut Parser<'i, '_>) -> Result<Self, cssparser::ParseError<'i, ()>> {
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
            if matches!(token, Token::Delim('>')) {
                if simple.is_empty() {
                    return Err(input.new_custom_error(()));
                }
                result.parts.push(Compound {
                    simple: std::mem::take(&mut simple),
                    relation,
                });
                relation = Relation::Child;
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
                    result.specificity.2 += 1;
                    Simple::Tag(v.to_ascii_lowercase())
                }
                Token::Delim('*') if simple.is_empty() => Simple::Universal,
                Token::IDHash(v) => {
                    result.specificity.0 += 1;
                    Simple::Id(v.to_string())
                }
                Token::Delim('.') => {
                    result.specificity.1 += 1;
                    Simple::Class(match input.next_including_whitespace()? {
                        Token::Ident(name) => name.to_string(),
                        _ => return Err(input.new_custom_error(())),
                    })
                }
                _ => return Err(input.new_custom_error(())),
            };
            simple.push(value);
            if result.parts.len() >= 32 || simple.len() > 64 {
                return Err(input.new_custom_error(()));
            }
        }
        if simple.is_empty() {
            return Err(input.new_custom_error(()));
        }
        result.parts.push(Compound { simple, relation });
        Ok(result)
    }
    pub fn matches(&self, doc: &Document, id: NodeId, budget: &mut usize) -> bool {
        // Keep alternatives for descendant combinators: the closest matching
        // ancestor need not satisfy the selectors to its left.
        let mut pending = vec![(id, self.parts.len() - 1, false)];
        while let Some((id, index, search_ancestors)) = pending.pop() {
            if *budget == 0 {
                return false;
            }
            *budget -= 1;
            let Some(node) = doc.node(id) else {
                continue;
            };
            let parent = node.parent();
            if search_ancestors {
                if let Some(parent) = parent {
                    pending.push((parent, index, true));
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
            if !self.parts[index].simple.iter().all(|simple| match simple {
                Simple::Universal => true,
                Simple::Tag(tag) => element.name.local.as_ref().eq_ignore_ascii_case(tag),
                Simple::Id(id) => element.attribute("id").is_some_and(|v| equal(v, id)),
                Simple::Class(class) => element
                    .attribute("class")
                    .is_some_and(|v| v.split_ascii_whitespace().any(|v| equal(v, class))),
            }) {
                continue;
            }
            if index == 0 {
                return true;
            }
            if let Some(parent) = parent {
                pending.push((
                    parent,
                    index - 1,
                    matches!(self.parts[index].relation, Relation::Descendant),
                ));
            }
        }
        false
    }
}
