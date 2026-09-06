use html5ever::{Attribute, QualName, interface::QuirksMode};
use std::io::{self, Write};
use std::num::NonZeroUsize;

/// An arena index. Only use an ID with the document that produced it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NodeId(NonZeroUsize);

impl NodeId {
    pub(crate) fn new(index: usize) -> Self {
        Self(NonZeroUsize::new(index + 1).expect("node index must fit in usize"))
    }

    pub(crate) fn index(self) -> usize {
        self.0.get() - 1
    }
}

/// Names and attributes preserve HTML, SVG, MathML, and attribute namespaces.
#[derive(Debug)]
pub struct Element {
    pub name: QualName,
    pub attributes: Vec<Attribute>,
    /// Template contents live in a separate fragment, not the element's children.
    pub template_contents: Option<NodeId>,
    /// Retained for future CSP enforcement; this parser does not enforce CSP.
    pub had_duplicate_attributes: bool,
    pub(crate) mathml_integration_point: bool,
}

impl Element {
    /// Find an unnamespaced attribute. HTML attribute names are ASCII-lowercased.
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attr| attr.name.ns.is_empty() && attr.name.local.as_ref() == name)
            .map(|attr| attr.value.as_ref())
    }
}

/// Data owned by a node. Relationships are stored as indices, never pointers.
#[derive(Debug)]
pub enum NodeKind {
    Document,
    DocumentFragment,
    Doctype {
        name: String,
        public_id: String,
        system_id: String,
    },
    Element(Element),
    Text(String),
    Comment(String),
    ProcessingInstruction {
        target: String,
        data: String,
    },
}

/// A node in the immutable result tree.
#[derive(Debug)]
pub struct Node {
    pub kind: NodeKind,
    pub(crate) parent: Option<NodeId>,
    pub(crate) first_child: Option<NodeId>,
    pub(crate) last_child: Option<NodeId>,
    pub(crate) previous_sibling: Option<NodeId>,
    pub(crate) next_sibling: Option<NodeId>,
}

impl Node {
    pub(crate) fn new(kind: NodeKind) -> Self {
        Self {
            kind,
            parent: None,
            first_child: None,
            last_child: None,
            previous_sibling: None,
            next_sibling: None,
        }
    }

    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    pub fn as_element(&self) -> Option<&Element> {
        match &self.kind {
            NodeKind::Element(element) => Some(element),
            _ => None,
        }
    }
}

/// A flat arena: tree traversal and destruction do not recurse with HTML depth.
#[derive(Debug)]
pub struct Document {
    pub(crate) nodes: Vec<Node>,
    pub(crate) quirks_mode: QuirksMode,
}

impl Document {
    pub fn root(&self) -> NodeId {
        NodeId::new(0)
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.index())
    }

    /// Allocated nodes, including template fragments and detached nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn quirks_mode(&self) -> QuirksMode {
        self.quirks_mode
    }

    pub fn children(&self, parent: NodeId) -> Children<'_> {
        Children {
            document: self,
            next: self.node(parent).and_then(|node| node.first_child),
        }
    }

    /// Descendants in tree order, excluding the start node and template contents.
    /// Traverse an element's `template_contents` separately when needed.
    pub fn descendants(&self, root: NodeId) -> Descendants<'_> {
        Descendants {
            document: self,
            root,
            next: self.node(root).and_then(|node| node.first_child),
        }
    }

    /// Write a diagnostic tree, including template fragments. This is not HTML
    /// serialization. Control characters in untrusted text are escaped.
    pub fn write_tree(&self, mut writer: impl Write) -> io::Result<()> {
        let mut pending = vec![(self.root(), 0_usize)];
        while let Some((id, depth)) = pending.pop() {
            let node = &self.nodes[id.index()];
            // Bound indentation so deep input cannot produce quadratic output.
            for _ in 0..depth.min(32) {
                write!(writer, "  ")?;
            }
            if depth > 32 {
                write!(writer, "[depth {depth}] ")?;
            }
            match &node.kind {
                NodeKind::Document => writeln!(writer, "#document ({:?})", self.quirks_mode)?,
                NodeKind::DocumentFragment => writeln!(writer, "#document-fragment")?,
                NodeKind::Doctype {
                    name,
                    public_id,
                    system_id,
                } => {
                    writeln!(writer, "<!DOCTYPE {name:?} {public_id:?} {system_id:?}>")?;
                }
                NodeKind::Element(element) => {
                    write!(writer, "<{}", element.name.local.escape_debug())?;
                    if element.name.ns.as_ref() != "http://www.w3.org/1999/xhtml" {
                        write!(writer, " namespace={:?}", element.name.ns.as_ref())?;
                    }
                    for attr in &element.attributes {
                        write!(writer, " ")?;
                        if let Some(prefix) = &attr.name.prefix {
                            write!(writer, "{}:", prefix.escape_debug())?;
                        }
                        write!(
                            writer,
                            "{}={:?}",
                            attr.name.local.escape_debug(),
                            attr.value.as_ref()
                        )?;
                    }
                    writeln!(writer, ">")?;
                }
                NodeKind::Text(text) => writeln!(writer, "{text:?}")?,
                NodeKind::Comment(text) => writeln!(writer, "<!-- {text:?} -->")?,
                NodeKind::ProcessingInstruction { target, data } => {
                    writeln!(writer, "<? {target:?} {data:?} ?>")?;
                }
            }
            let mut child = node.last_child;
            while let Some(id) = child {
                pending.push((id, depth + 1));
                child = self.nodes[id.index()].previous_sibling;
            }
            if let NodeKind::Element(element) = &node.kind {
                if let Some(fragment) = element.template_contents {
                    pending.push((fragment, depth + 1));
                }
            }
        }
        Ok(())
    }
}

pub struct Children<'a> {
    document: &'a Document,
    next: Option<NodeId>,
}

impl Iterator for Children<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next?;
        self.next = self.document.nodes[current.index()].next_sibling;
        Some(current)
    }
}

/// A stack-free traversal using parent and sibling indices.
pub struct Descendants<'a> {
    document: &'a Document,
    root: NodeId,
    next: Option<NodeId>,
}

impl Iterator for Descendants<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next?;
        self.next = self.document.nodes[current.index()].first_child;
        if self.next.is_none() {
            let mut cursor = current;
            while cursor != self.root {
                let node = &self.document.nodes[cursor.index()];
                if let Some(next) = node.next_sibling {
                    self.next = Some(next);
                    break;
                }
                match node.parent {
                    Some(parent) => cursor = parent,
                    None => break,
                }
            }
        }
        Some(current)
    }
}
