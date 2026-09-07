use crate::{Diagnostic, Document, Element, Node, NodeId, NodeKind, ParseOutput};
use html5ever::interface::{ElemName, ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::tendril::StrTendril;
use html5ever::{Attribute, LocalName, Namespace, QualName};
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
};

pub(crate) struct Sink {
    document: RefCell<Document>,
    line: Cell<u64>,
    diagnostics: RefCell<Vec<Diagnostic>>,
    max_diagnostics: usize,
    omitted: Cell<usize>,
}

impl Sink {
    pub(crate) fn new(max_diagnostics: usize) -> Self {
        Self {
            document: RefCell::new(Document {
                nodes: vec![Node::new(NodeKind::Document)],
                quirks_mode: QuirksMode::NoQuirks,
            }),
            line: Cell::new(1),
            diagnostics: RefCell::new(Vec::new()),
            max_diagnostics,
            omitted: Cell::new(0),
        }
    }
}

impl Document {
    pub(crate) fn allocate(&mut self, kind: NodeKind) -> NodeId {
        let id = NodeId::new(self.nodes.len());
        self.nodes.push(Node::new(kind));
        id
    }

    pub(crate) fn detach(&mut self, id: NodeId) {
        let node = &mut self.nodes[id.index()];
        let parent = node.parent.take();
        let previous = node.previous_sibling.take();
        let next = node.next_sibling.take();
        if let Some(previous) = previous {
            self.nodes[previous.index()].next_sibling = next;
        } else if let Some(parent) = parent {
            self.nodes[parent.index()].first_child = next;
        }
        if let Some(next) = next {
            self.nodes[next.index()].previous_sibling = previous;
        } else if let Some(parent) = parent {
            self.nodes[parent.index()].last_child = previous;
        }
    }

    /// Move a node into `parent`, before `before` or at the end.
    pub(crate) fn insert(&mut self, parent: NodeId, before: Option<NodeId>, id: NodeId) {
        if before == Some(id) {
            return;
        }
        self.detach(id);
        let previous = match before {
            Some(sibling) => self.nodes[sibling.index()].previous_sibling,
            None => self.nodes[parent.index()].last_child,
        };
        let node = &mut self.nodes[id.index()];
        node.parent = Some(parent);
        node.previous_sibling = previous;
        node.next_sibling = before;
        if let Some(previous) = previous {
            self.nodes[previous.index()].next_sibling = Some(id);
        } else {
            self.nodes[parent.index()].first_child = Some(id);
        }
        if let Some(before) = before {
            self.nodes[before.index()].previous_sibling = Some(id);
        } else {
            self.nodes[parent.index()].last_child = Some(id);
        }
    }

    fn insert_token(&mut self, parent: NodeId, before: Option<NodeId>, child: NodeOrText<NodeId>) {
        let id = match child {
            NodeOrText::AppendNode(id) => id,
            NodeOrText::AppendText(text) => {
                if text.is_empty() {
                    return;
                }
                let previous = match before {
                    Some(sibling) => self.nodes[sibling.index()].previous_sibling,
                    None => self.nodes[parent.index()].last_child,
                };
                if let Some(previous) = previous {
                    if let NodeKind::Text(contents) = &mut self.nodes[previous.index()].kind {
                        contents.push_str(&text);
                        return;
                    }
                }
                self.allocate(NodeKind::Text(text.to_string()))
            }
        };
        self.insert(parent, before, id);
    }
}

// Clone the interned name to release the arena borrow before other sink callbacks.
// Keeping a Ref<Vec<Node>> alive here would prevent the tree builder from mutating it.
#[derive(Debug)]
pub(crate) struct OwnedName(QualName);
impl ElemName for OwnedName {
    fn ns(&self) -> &Namespace {
        &self.0.ns
    }
    fn local_name(&self) -> &LocalName {
        &self.0.local
    }
}

impl TreeSink for Sink {
    type Handle = NodeId;
    type Output = ParseOutput;
    type ElemName<'a> = OwnedName;

    fn finish(self) -> ParseOutput {
        ParseOutput {
            document: self.document.into_inner(),
            diagnostics: self.diagnostics.into_inner(),
            omitted_diagnostics: self.omitted.get(),
        }
    }

    fn parse_error(&self, message: Cow<'static, str>) {
        let mut diagnostics = self.diagnostics.borrow_mut();
        if diagnostics.len() < self.max_diagnostics {
            let mut end = message.len().min(512);
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            diagnostics.push(Diagnostic {
                line: self.line.get(),
                message: message[..end].to_owned(),
            });
        } else {
            self.omitted.set(self.omitted.get().saturating_add(1));
        }
    }

    fn set_current_line(&self, line: u64) {
        self.line.set(line);
    }
    fn get_document(&self) -> NodeId {
        self.document.borrow().root()
    }
    fn same_node(&self, x: &NodeId, y: &NodeId) -> bool {
        x == y
    }
    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.document.borrow_mut().quirks_mode = mode;
    }

    fn elem_name<'a>(&'a self, target: &'a NodeId) -> OwnedName {
        let doc = self.document.borrow();
        OwnedName(
            doc.nodes[target.index()]
                .as_element()
                .expect("tree builder requested element name")
                .name
                .clone(),
        )
    }

    fn create_element(
        &self,
        name: QualName,
        attributes: Vec<Attribute>,
        flags: ElementFlags,
    ) -> NodeId {
        let mut doc = self.document.borrow_mut();
        let template_contents = flags
            .template
            .then(|| doc.allocate(NodeKind::DocumentFragment));
        doc.allocate(NodeKind::Element(Element {
            name,
            attributes,
            template_contents,
            had_duplicate_attributes: flags.had_duplicate_attributes,
            mathml_integration_point: flags.mathml_annotation_xml_integration_point,
        }))
    }

    fn create_comment(&self, text: StrTendril) -> NodeId {
        self.document
            .borrow_mut()
            .allocate(NodeKind::Comment(text.to_string()))
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> NodeId {
        self.document
            .borrow_mut()
            .allocate(NodeKind::ProcessingInstruction {
                target: target.to_string(),
                data: data.to_string(),
            })
    }

    fn append(&self, parent: &NodeId, child: NodeOrText<NodeId>) {
        self.document
            .borrow_mut()
            .insert_token(*parent, None, child);
    }

    fn append_before_sibling(&self, sibling: &NodeId, child: NodeOrText<NodeId>) {
        let mut doc = self.document.borrow_mut();
        let parent = doc.nodes[sibling.index()]
            .parent
            .expect("insertion sibling has a parent");
        doc.insert_token(parent, Some(*sibling), child);
    }

    fn append_based_on_parent_node(
        &self,
        element: &NodeId,
        previous: &NodeId,
        child: NodeOrText<NodeId>,
    ) {
        let mut doc = self.document.borrow_mut();
        if let Some(parent) = doc.nodes[element.index()].parent {
            doc.insert_token(parent, Some(*element), child);
        } else {
            doc.insert_token(*previous, None, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        let mut doc = self.document.borrow_mut();
        let id = doc.allocate(NodeKind::Doctype {
            name: name.to_string(),
            public_id: public_id.to_string(),
            system_id: system_id.to_string(),
        });
        let root = doc.root();
        doc.insert(root, None, id);
    }

    fn get_template_contents(&self, target: &NodeId) -> NodeId {
        self.document.borrow().nodes[target.index()]
            .as_element()
            .and_then(|e| e.template_contents)
            .expect("tree builder requested template contents")
    }

    fn add_attrs_if_missing(&self, target: &NodeId, attrs: Vec<Attribute>) {
        let mut doc = self.document.borrow_mut();
        let NodeKind::Element(element) = &mut doc.nodes[target.index()].kind else {
            unreachable!("tree builder adds attributes only to elements")
        };
        for attr in attrs {
            if !element
                .attributes
                .iter()
                .any(|existing| existing.name == attr.name)
            {
                element.attributes.push(attr);
            }
        }
    }

    fn remove_from_parent(&self, target: &NodeId) {
        self.document.borrow_mut().detach(*target);
    }

    fn reparent_children(&self, node: &NodeId, new_parent: &NodeId) {
        let mut doc = self.document.borrow_mut();
        while let Some(child) = doc.nodes[node.index()].first_child {
            doc.insert(*new_parent, None, child);
        }
    }

    fn is_mathml_annotation_xml_integration_point(&self, target: &NodeId) -> bool {
        self.document.borrow().nodes[target.index()]
            .as_element()
            .is_some_and(|e| e.mathml_integration_point)
    }

    fn allow_declarative_shadow_roots(&self, _parent: &NodeId) -> bool {
        false
    }
}
