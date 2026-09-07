use super::{ScriptOptions, preview};
use crate::{Document, Element, NodeId, NodeKind};
use boa_engine::{
    Context, JsData, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction,
    js_string,
    object::{FunctionObjectBuilder, ObjectInitializer},
    property::Attribute,
};
use boa_gc::{Finalize, Trace};
use std::{cell::RefCell, collections::HashMap};

const HTML: &str = "http://www.w3.org/1999/xhtml";
struct Host {
    document: Option<Document>,
    wrappers: HashMap<NodeId, JsObject>,
    options: ScriptOptions,
    operations: usize,
    bytes: usize,
    nodes: usize,
    limited: bool,
    console: Vec<String>,
    omitted: usize,
    alerts: Vec<String>,
    alert_count: usize,
    omitted_alerts: usize,
}
impl Host {
    fn spend(&mut self, operations: usize, bytes: usize, nodes: usize) -> JsResult<()> {
        if self.limited || operations > self.operations || bytes > self.bytes || nodes > self.nodes
        {
            self.limited = true;
            return Err(JsNativeError::range()
                .with_message("Olive DOM resource limit exceeded")
                .into());
        }
        self.operations -= operations;
        self.bytes -= bytes;
        self.nodes -= nodes;
        Ok(())
    }
    fn doc(&self) -> &Document {
        self.document.as_ref().expect("DOM installed")
    }
    fn doc_mut(&mut self) -> &mut Document {
        self.document.as_mut().expect("DOM installed")
    }
    fn find(&mut self, predicate: impl Fn(&Element) -> bool) -> JsResult<Option<NodeId>> {
        let mut spent = 0;
        let found = self.doc().descendants(self.doc().root()).find(|&id| {
            spent += 1;
            spent > self.operations
                || self
                    .doc()
                    .node(id)
                    .and_then(|n| n.as_element())
                    .is_some_and(|e| e.name.ns.as_ref() == HTML && predicate(e))
        });
        self.spend(spent, 0, 0)?;
        Ok(found)
    }
    fn text(&mut self, id: NodeId) -> JsResult<String> {
        self.spend(1, 0, 0)?;
        if let NodeKind::Text(s) | NodeKind::Comment(s) = &self.doc().node(id).unwrap().kind {
            return Ok(s.clone());
        }
        let mut value = String::new();
        let mut visited = 0;
        for child in self.doc().descendants(id) {
            visited += 1;
            if visited > self.operations {
                break;
            }
            if let NodeKind::Text(s) = &self.doc().node(child).unwrap().kind {
                value.push_str(s);
            }
        }
        self.spend(visited, 0, 0)?;
        Ok(value)
    }
    fn set_text(&mut self, id: NodeId, text: String) -> JsResult<()> {
        let is_text = matches!(
            self.doc().node(id).unwrap().kind,
            NodeKind::Text(_) | NodeKind::Comment(_)
        );
        if is_text {
            self.spend(1, text.len(), 0)?;
            match &mut self.doc_mut().nodes[id.index()].kind {
                NodeKind::Text(s) | NodeKind::Comment(s) => *s = text,
                _ => unreachable!(),
            }
        } else {
            let children: Vec<_> = self.doc().children(id).collect();
            self.spend(
                children.len().saturating_add(1),
                text.len(),
                usize::from(!text.is_empty()),
            )?;
            for child in children {
                self.doc_mut().detach(child);
            }
            if !text.is_empty() {
                let child = self.doc_mut().allocate(NodeKind::Text(text));
                self.doc_mut().insert(id, None, child);
            }
        }
        Ok(())
    }
}
fn host(context: &Context) -> &RefCell<Host> {
    context.get_data().expect("JavaScript host installed")
}

pub(super) fn limited(context: &Context) -> bool {
    host(context).borrow().limited
}
pub(super) fn console(context: &Context) -> Vec<String> {
    host(context).borrow().console.clone()
}
pub(super) fn omitted_console(context: &Context) -> usize {
    host(context).borrow().omitted
}
pub(super) fn take_document(context: &Context) -> Document {
    host(context)
        .borrow_mut()
        .document
        .take()
        .expect("DOM installed")
}

pub(super) fn install_console(context: &mut Context, options: ScriptOptions) -> JsResult<()> {
    context.insert_data(RefCell::new(Host {
        document: None,
        wrappers: HashMap::new(),
        options,
        operations: options.max_dom_operations,
        bytes: options.max_dom_bytes,
        nodes: options.max_new_nodes,
        limited: false,
        console: Vec::new(),
        omitted: 0,
        alerts: Vec::new(),
        alert_count: 0,
        omitted_alerts: 0,
    }));
    let console = ObjectInitializer::new(context)
        .function(NativeFunction::from_fn_ptr(log), js_string!("log"), 0)
        .function(NativeFunction::from_fn_ptr(log), js_string!("info"), 0)
        .function(NativeFunction::from_fn_ptr(log), js_string!("warn"), 0)
        .function(NativeFunction::from_fn_ptr(log), js_string!("error"), 0)
        .build();
    context.register_global_property(js_string!("console"), console, Attribute::all())
}
fn log(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let mut host = host(context).borrow_mut();
    if host.console.len() >= host.options.max_messages {
        host.omitted = host.omitted.saturating_add(1);
        return Ok(JsValue::undefined());
    }
    let mut message = String::new();
    for (index, value) in args.iter().enumerate() {
        let remaining = host.options.max_output_bytes.saturating_sub(message.len());
        if remaining == 0 {
            break;
        }
        if index > 0 {
            message.push(' ');
        }
        message.push_str(&preview(
            value,
            host.options.max_output_bytes.saturating_sub(message.len()),
        ));
    }
    host.console.push(message);
    Ok(JsValue::undefined())
}

type Callback = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;
fn accessor(builder: &mut ObjectInitializer<'_>, name: &str, get: Callback, set: Option<Callback>) {
    let getter =
        FunctionObjectBuilder::new(builder.context().realm(), NativeFunction::from_fn_ptr(get))
            .build();
    let setter = set.map(|f| {
        FunctionObjectBuilder::new(builder.context().realm(), NativeFunction::from_fn_ptr(f))
            .build()
    });
    builder.accessor(
        JsString::from(name),
        Some(getter),
        setter,
        Attribute::CONFIGURABLE,
    );
}

pub(super) fn install_document(context: &mut Context, document: Document) -> JsResult<()> {
    host(context).borrow_mut().document = Some(document);
    context.register_global_builtin_callable(
        js_string!("alert"),
        1,
        NativeFunction::from_fn_ptr(alert),
    )?;
    let mut builder = ObjectInitializer::new(context);
    builder
        .function(
            NativeFunction::from_fn_ptr(get_element_by_id),
            js_string!("getElementById"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(create_element),
            js_string!("createElement"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(create_text_node),
            js_string!("createTextNode"),
            1,
        );
    accessor(&mut builder, "body", body, None);
    accessor(&mut builder, "documentElement", document_element, None);
    accessor(&mut builder, "title", title, Some(set_title));
    let document = builder.build();
    context.register_global_property(js_string!("document"), document, Attribute::all())?;
    let global = context.global_object();
    context.register_global_property(js_string!("window"), global.clone(), Attribute::all())?;
    context.register_global_property(js_string!("self"), global, Attribute::all())
}

#[derive(Debug, Trace, Finalize, JsData)]
struct NodeHandle {
    #[unsafe_ignore_trace]
    id: NodeId,
}
fn node_id(this: &JsValue) -> JsResult<NodeId> {
    this.as_object()
        .and_then(|o| o.downcast_ref::<NodeHandle>().map(|n| n.id))
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("Expected an Olive DOM node")
                .into()
        })
}
pub(super) fn wrap(context: &mut Context, id: Option<NodeId>) -> JsResult<JsValue> {
    let Some(id) = id else {
        return Ok(JsValue::null());
    };
    if let Some(object) = host(context).borrow().wrappers.get(&id) {
        return Ok(object.clone().into());
    }
    let is_element = host(context)
        .borrow()
        .doc()
        .node(id)
        .unwrap()
        .as_element()
        .is_some();
    let mut builder = ObjectInitializer::with_native_data(NodeHandle { id }, context);
    accessor(
        &mut builder,
        "textContent",
        text_content,
        Some(set_text_content),
    );
    accessor(&mut builder, "parentNode", parent_node, None);
    builder.function(NativeFunction::from_fn_ptr(remove), js_string!("remove"), 0);
    if is_element {
        builder
            .function(
                NativeFunction::from_fn_ptr(get_attribute),
                js_string!("getAttribute"),
                1,
            )
            .function(
                NativeFunction::from_fn_ptr(set_attribute),
                js_string!("setAttribute"),
                2,
            )
            .function(
                NativeFunction::from_fn_ptr(remove_attribute),
                js_string!("removeAttribute"),
                1,
            )
            .function(
                NativeFunction::from_fn_ptr(append_child),
                js_string!("appendChild"),
                1,
            );
        accessor(&mut builder, "id", get_id, Some(set_id));
        accessor(&mut builder, "className", get_class, Some(set_class));
        accessor(&mut builder, "tagName", tag_name, None);
    }
    let object = builder.build();
    host(context)
        .borrow_mut()
        .wrappers
        .insert(id, object.clone());
    Ok(object.into())
}

// Host arguments accept primitives. Avoid invoking page code from native callbacks,
// and keep argument conversion predictable.
fn string_arg(args: &[JsValue], index: usize, context: &Context) -> JsResult<String> {
    let value = args.get(index).cloned().unwrap_or_default();
    if value.is_object() || value.is_symbol() || value.is_bigint() {
        return Err(JsNativeError::typ()
            .with_message("DOM arguments must be strings or simple primitives")
            .into());
    }
    let mut host = host(context).borrow_mut();
    if let Some(s) = value.as_string() {
        if s.len() > host.options.max_dom_bytes {
            host.spend(0, usize::MAX, 0)?;
        }
        return Ok(s.to_std_string_lossy());
    }
    Ok(value.display().to_string())
}
fn get_element_by_id(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let key = string_arg(args, 0, context)?;
    let id = if key.is_empty() {
        None
    } else {
        host(context)
            .borrow_mut()
            .find(|e| e.attribute("id") == Some(key.as_str()))?
    };
    wrap(context, id)
}
fn find_tag(context: &mut Context, tag: &str) -> JsResult<JsValue> {
    let id = host(context)
        .borrow_mut()
        .find(|e| e.name.local.as_ref() == tag)?;
    wrap(context, id)
}
fn body(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    find_tag(context, "body")
}
fn document_element(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    find_tag(context, "html")
}
fn title(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let mut host = host(context).borrow_mut();
    let id = host.find(|e| e.name.local.as_ref() == "title")?;
    let text = match id {
        Some(id) => host.text(id)?,
        None => String::new(),
    };
    Ok(JsString::from(text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")).into())
}
fn set_title(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let text = string_arg(args, 0, context)?;
    let mut host = host(context).borrow_mut();
    let id = match host.find(|e| e.name.local.as_ref() == "title")? {
        Some(id) => id,
        None => {
            let Some(head) = host.find(|e| e.name.local.as_ref() == "head")? else {
                return Ok(JsValue::undefined());
            };
            host.spend(1, 0, 1)?;
            let id = host.doc_mut().allocate(element("title"));
            host.doc_mut().insert(head, None, id);
            id
        }
    };
    host.set_text(id, text)?;
    Ok(JsValue::undefined())
}
fn element(name: &str) -> NodeKind {
    NodeKind::Element(Element {
        name: html5ever::QualName::new(None, HTML.into(), name.into()),
        attributes: Vec::new(),
        template_contents: None,
        had_duplicate_attributes: false,
        mathml_integration_point: false,
    })
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().enumerate().all(|(i, b)| {
            b.is_ascii_alphabetic() || (i > 0 && (b.is_ascii_digit() || b"-_:".contains(&b)))
        })
}
fn create_element(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let name = string_arg(args, 0, context)?.to_ascii_lowercase();
    if !valid_name(&name) || name == "template" {
        return Err(JsNativeError::typ()
            .with_message("Unsupported element name")
            .into());
    }
    let id = {
        let mut host = host(context).borrow_mut();
        host.spend(1, name.len(), 1)?;
        host.doc_mut().allocate(element(&name))
    };
    wrap(context, Some(id))
}
fn create_text_node(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let text = string_arg(args, 0, context)?;
    let id = {
        let mut host = host(context).borrow_mut();
        host.spend(1, text.len(), 1)?;
        host.doc_mut().allocate(NodeKind::Text(text))
    };
    wrap(context, Some(id))
}
fn text_content(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let text = host(context).borrow_mut().text(node_id(this)?)?;
    Ok(JsString::from(text).into())
}
fn set_text_content(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let text = if args.first().is_some_and(|v| v.is_null_or_undefined()) {
        String::new()
    } else {
        string_arg(args, 0, context)?
    };
    host(context).borrow_mut().set_text(node_id(this)?, text)?;
    Ok(JsValue::undefined())
}
fn parent_node(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let id = node_id(this)?;
    let parent = {
        let mut host = host(context).borrow_mut();
        host.spend(1, 0, 0)?;
        // The Document global is not an Element wrapper.
        host.doc()
            .node(id)
            .unwrap()
            .parent()
            .filter(|&p| p != host.doc().root())
    };
    wrap(context, parent)
}
fn append_child(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let parent = node_id(this)?;
    let child = node_id(args.first().unwrap_or(&JsValue::undefined()))?;
    let mut host = host(context).borrow_mut();
    // Validate before changing links, including a self/ancestor insertion.
    let mut ancestor = Some(parent);
    while let Some(id) = ancestor {
        host.spend(1, 0, 0)?;
        if id == child {
            return Err(JsNativeError::typ()
                .with_message("Cannot create a DOM cycle")
                .into());
        }
        ancestor = host.doc().node(id).unwrap().parent();
    }
    if host.doc().node(parent).unwrap().as_element().is_none() {
        return Err(JsNativeError::typ()
            .with_message("Parent must be an element")
            .into());
    }
    host.doc_mut().insert(parent, None, child);
    Ok(args[0].clone())
}
fn remove(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let mut host = host(context).borrow_mut();
    host.spend(1, 0, 0)?;
    host.doc_mut().detach(node_id(this)?);
    Ok(JsValue::undefined())
}
fn read_attribute(this: &JsValue, name: &str, context: &Context) -> JsResult<JsValue> {
    let mut host = host(context).borrow_mut();
    host.spend(1, 0, 0)?;
    let element = host
        .doc()
        .node(node_id(this)?)
        .unwrap()
        .as_element()
        .ok_or_else(|| JsNativeError::typ().with_message("Expected an element"))?;
    Ok(element
        .attribute(name)
        .map(|s| JsString::from(s).into())
        .unwrap_or_else(JsValue::null))
}
fn write_attribute(
    this: &JsValue,
    name: &str,
    value: Option<String>,
    context: &Context,
) -> JsResult<JsValue> {
    if !valid_name(name) {
        return Err(JsNativeError::typ()
            .with_message("Unsupported attribute name")
            .into());
    }
    let id = node_id(this)?;
    let mut host = host(context).borrow_mut();
    host.spend(
        1,
        name.len()
            .saturating_add(value.as_ref().map_or(0, String::len)),
        0,
    )?;
    let NodeKind::Element(element) = &mut host.doc_mut().nodes[id.index()].kind else {
        return Err(JsNativeError::typ()
            .with_message("Expected an element")
            .into());
    };
    let existing = element
        .attributes
        .iter()
        .position(|a| a.name.ns.is_empty() && a.name.local.as_ref() == name);
    match (existing, value) {
        (Some(index), Some(value)) => element.attributes[index].value = value.into(),
        (Some(index), None) => {
            element.attributes.remove(index);
        }
        (None, Some(value)) => element.attributes.push(html5ever::Attribute {
            name: html5ever::QualName::new(None, "".into(), name.into()),
            value: value.into(),
        }),
        (None, None) => {}
    }
    Ok(JsValue::undefined())
}
fn get_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let name = string_arg(args, 0, context)?.to_ascii_lowercase();
    read_attribute(this, &name, context)
}
fn set_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let name = string_arg(args, 0, context)?.to_ascii_lowercase();
    let value = string_arg(args, 1, context)?;
    write_attribute(this, &name, Some(value), context)
}
fn remove_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let name = string_arg(args, 0, context)?.to_ascii_lowercase();
    write_attribute(this, &name, None, context)
}
fn reflected(this: &JsValue, name: &str, context: &Context) -> JsResult<JsValue> {
    let value = read_attribute(this, name, context)?;
    Ok(if value.is_null() {
        js_string!("").into()
    } else {
        value
    })
}
fn get_id(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    reflected(this, "id", context)
}
fn get_class(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    reflected(this, "class", context)
}
fn set_id(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let value = string_arg(args, 0, context)?;
    write_attribute(this, "id", Some(value), context)
}
fn set_class(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let value = string_arg(args, 0, context)?;
    write_attribute(this, "class", Some(value), context)
}
fn tag_name(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let mut host = host(context).borrow_mut();
    host.spend(1, 0, 0)?;
    let name = host
        .doc()
        .node(node_id(this)?)
        .unwrap()
        .as_element()
        .ok_or_else(|| JsNativeError::typ().with_message("Expected an element"))?
        .name
        .local
        .to_ascii_uppercase();
    Ok(JsString::from(name.as_ref()).into())
}

pub(super) fn restore_document(context: &Context, document: Document) {
    host(context).borrow_mut().document = Some(document);
}

pub(super) fn with_document<T>(context: &Context, read: impl FnOnce(&Document) -> T) -> T {
    read(host(context).borrow().doc())
}
pub(super) fn take_alerts(context: &Context) -> Vec<String> {
    std::mem::take(&mut host(context).borrow_mut().alerts)
}
pub(super) fn omitted_alerts(context: &Context) -> usize {
    host(context).borrow().omitted_alerts
}
fn alert(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let mut host = host(context).borrow_mut();
    if host.alert_count >= host.options.max_messages {
        host.omitted_alerts = host.omitted_alerts.saturating_add(1);
    } else {
        let message = args
            .first()
            .map(|v| preview(v, host.options.max_output_bytes))
            .unwrap_or_default();
        host.alerts.push(message);
        host.alert_count += 1;
    }
    Ok(JsValue::undefined())
}
