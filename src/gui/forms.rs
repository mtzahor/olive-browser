//! Bounded native HTML form state. No networking or page code runs in widgets.
use eframe::egui;
use olive_html::{
    Document, Element, NodeId, NodeKind,
    net::{FormRequest, Location, MAX_FORM_BYTES},
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const MAX_CONTROLS: usize = 512;
const MAX_OPTIONS: usize = 2048;
const MAX_VALUE: usize = 16_384;
const MAX_STATE_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Text,
    Password,
    Textarea,
    Checkbox,
    Radio,
    Select,
    Hidden,
    Submit,
    Reset,
    Button,
    Unsupported,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct OptionItem {
    label: String,
    value: String,
    disabled: bool,
    selected: bool,
    initial: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Attributes {
    action: Option<String>,
    method: Option<String>,
    enctype: Option<String>,
    target: Option<String>,
    novalidate: bool,
}
impl Attributes {
    fn read(e: &Element, prefix: &str) -> Self {
        let get = |name: &str| e.attribute(&format!("{prefix}{name}")).map(str::to_owned);
        Self {
            action: get("action"),
            method: get("method"),
            enctype: get("enctype"),
            target: get("target"),
            novalidate: get("novalidate").is_some(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Control {
    pub node: NodeId,
    pub owner: Option<NodeId>,
    pub kind: Kind,
    name: String,
    label: String,
    placeholder: String,
    pub value: String,
    initial: String,
    pub checked: bool,
    initial_checked: bool,
    pub disabled: bool,
    readonly: bool,
    required: bool,
    maxlength: usize,
    multiple: bool,
    options: Vec<OptionItem>,
    overrides: Attributes,
    #[serde(skip)]
    dirty: bool,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Forms {
    pub controls: Vec<Control>,
    forms: HashMap<NodeId, Attributes>,
    pub limited: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Activation {
    pub control: NodeId,
}

#[derive(Clone, Copy, Default)]
struct Context {
    form: Option<NodeId>,
    disabled: bool,
    legend: Option<NodeId>,
    inert: bool,
    label: Option<NodeId>,
}
fn element(doc: &Document, id: NodeId) -> Option<&Element> {
    doc.node(id)?
        .as_element()
        .filter(|e| e.name.ns.as_ref() == "http://www.w3.org/1999/xhtml")
}
fn text(doc: &Document, id: NodeId, limit: usize) -> String {
    doc.descendants(id)
        .filter_map(|id| match &doc.node(id)?.kind {
            NodeKind::Text(s) => Some(s.as_str()),
            _ => None,
        })
        .flat_map(str::chars)
        .take(limit)
        .collect()
}
impl Forms {
    pub fn from_document(doc: &Document, scripting: bool) -> Self {
        let mut result = Self::default();
        let mut ids = HashMap::new();
        let mut labels = HashMap::new();
        for id in doc.descendants(doc.root()) {
            if let Some(e) = element(doc, id) {
                if let Some(key) = e.attribute("id") {
                    ids.entry(key).or_insert(id);
                }
                if e.name.local.as_ref() == "label" && labels.len() < MAX_CONTROLS {
                    if let Some(key) = e.attribute("for") {
                        labels.entry(key).or_insert(id);
                    }
                }
            }
        }
        let mut contexts = HashMap::<NodeId, Context>::new();
        let mut option_count = 0;
        let mut bytes = 0;
        for id in doc.descendants(doc.root()) {
            let Some(e) = element(doc, id) else { continue };
            let parent = doc.node(id).and_then(|n| n.parent());
            let inherited = parent
                .and_then(|p| contexts.get(&p))
                .copied()
                .unwrap_or_default();
            let mut context = inherited;
            // The first legend is exempt only from its own disabled fieldset.
            if inherited.legend == Some(id) {
                context.disabled = parent
                    .and_then(|p| doc.node(p)?.parent())
                    .and_then(|p| contexts.get(&p))
                    .is_some_and(|p| p.disabled);
            }
            context.legend = None;
            let tag = e.name.local.as_ref();
            context.inert |= matches!(tag, "template" | "datalist" | "script" | "style")
                || (tag == "noscript" && scripting);
            if tag == "form" && !context.inert {
                if result.forms.len() == MAX_CONTROLS {
                    result.limited = true;
                    break;
                }
                result.forms.insert(id, Attributes::read(e, ""));
                context.form = Some(id);
            }
            if tag == "label" {
                context.label = Some(id);
            }
            if tag == "fieldset" && e.attribute("disabled").is_some() {
                context.disabled = true;
                context.legend = doc.children(id).find(|&id| {
                    element(doc, id).is_some_and(|e| e.name.local.as_ref() == "legend")
                });
            }
            contexts.insert(id, context);
            if context.inert || !matches!(tag, "input" | "textarea" | "select" | "button") {
                continue;
            }
            if result.controls.len() == MAX_CONTROLS {
                result.limited = true;
                break;
            }
            let input_type = e
                .attribute("type")
                .unwrap_or(if tag == "button" { "submit" } else { "text" })
                .to_ascii_lowercase();
            let kind = match tag {
                "textarea" => Kind::Textarea,
                "select" => Kind::Select,
                "button" => match input_type.as_str() {
                    "reset" => Kind::Reset,
                    "button" => Kind::Button,
                    _ => Kind::Submit,
                },
                _ => match input_type.as_str() {
                    "password" => Kind::Password,
                    "checkbox" => Kind::Checkbox,
                    "radio" => Kind::Radio,
                    "hidden" => Kind::Hidden,
                    "submit" => Kind::Submit,
                    "reset" => Kind::Reset,
                    "button" => Kind::Button,
                    "file" | "image" | "range" | "color" | "date" | "datetime-local" | "month"
                    | "week" | "time" | "number" => Kind::Unsupported,
                    _ => Kind::Text,
                },
            };
            let owner = if let Some(name) = e.attribute("form") {
                ids.get(name)
                    .copied()
                    .filter(|&id| element(doc, id).is_some_and(|e| e.name.local.as_ref() == "form"))
            } else {
                context.form
            };
            let value = if kind == Kind::Textarea {
                text(doc, id, MAX_VALUE + 1)
            } else {
                e.attribute("value")
                    .unwrap_or(if matches!(kind, Kind::Checkbox | Kind::Radio) {
                        "on"
                    } else {
                        ""
                    })
                    .to_owned()
            };
            let label_node = e
                .attribute("id")
                .and_then(|key| labels.get(key).copied())
                .or(context.label);
            let label = if tag == "button" {
                text(doc, id, 256)
            } else if matches!(kind, Kind::Submit | Kind::Reset | Kind::Button) {
                e.attribute("value")
                    .unwrap_or(match kind {
                        Kind::Submit => "Submit",
                        Kind::Reset => "Reset",
                        _ => "Button",
                    })
                    .chars()
                    .take(256)
                    .collect()
            } else {
                e.attribute("aria-label")
                    .map(str::to_owned)
                    .or_else(|| label_node.map(|id| text(doc, id, 256)))
                    .unwrap_or_else(|| e.attribute("name").unwrap_or("Input").to_owned())
            };
            let multiple = e.attribute("multiple").is_some();
            let mut options = Vec::new();
            if kind == Kind::Select {
                for option in doc.descendants(id) {
                    let Some(oe) =
                        element(doc, option).filter(|e| e.name.local.as_ref() == "option")
                    else {
                        continue;
                    };
                    if option_count == MAX_OPTIONS {
                        result.limited = true;
                        break;
                    }
                    option_count += 1;
                    let label = oe.attribute("label").map(str::to_owned).unwrap_or_else(|| {
                        text(doc, option, MAX_VALUE + 1)
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                    });
                    let value = oe.attribute("value").unwrap_or(&label).to_owned();
                    bytes += label.len() + value.len();
                    let disabled = oe.attribute("disabled").is_some()
                        || doc
                            .node(option)
                            .and_then(|n| n.parent())
                            .and_then(|p| element(doc, p))
                            .is_some_and(|p| {
                                p.name.local.as_ref() == "optgroup"
                                    && p.attribute("disabled").is_some()
                            });
                    let selected = oe.attribute("selected").is_some();
                    options.push(OptionItem {
                        label,
                        value,
                        disabled,
                        selected,
                        initial: selected,
                    });
                    if bytes > MAX_STATE_BYTES {
                        result.limited = true;
                        break;
                    }
                }
                if !multiple {
                    let selected = options
                        .iter()
                        .rposition(|o| o.selected)
                        .or_else(|| options.iter().position(|o| !o.disabled));
                    for (i, o) in options.iter_mut().enumerate() {
                        o.selected = Some(i) == selected;
                        o.initial = o.selected;
                    }
                }
            }
            bytes += value.len() * 2 + label.len();
            if value.chars().count() > MAX_VALUE || bytes > MAX_STATE_BYTES {
                result.limited = true;
                break;
            }
            result.controls.push(Control {
                node: id,
                owner,
                kind,
                name: e.attribute("name").unwrap_or("").into(),
                label,
                placeholder: e
                    .attribute("placeholder")
                    .unwrap_or("")
                    .chars()
                    .take(256)
                    .collect(),
                initial: value.clone(),
                value,
                checked: e.attribute("checked").is_some(),
                initial_checked: e.attribute("checked").is_some(),
                disabled: context.disabled || e.attribute("disabled").is_some(),
                readonly: e.attribute("readonly").is_some(),
                required: e.attribute("required").is_some(),
                maxlength: e
                    .attribute("maxlength")
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(MAX_VALUE)
                    .min(MAX_VALUE),
                multiple,
                options,
                overrides: Attributes::read(e, "form"),
                dirty: false,
            });
        }
        // In a named radio group the last checked control wins, in DOM order.
        let mut groups = HashSet::new();
        for c in result.controls.iter_mut().rev() {
            if c.kind == Kind::Radio
                && c.checked
                && !c.name.is_empty()
                && !groups.insert((c.owner, c.name.clone()))
            {
                c.checked = false;
            }
            c.initial_checked = c.checked;
        }
        result
    }

    pub fn validate(&self) -> Result<(), String> {
        let bytes: usize = self
            .controls
            .iter()
            .map(|c| {
                c.value.len()
                    + c.initial.len()
                    + c.label.len()
                    + c.options
                        .iter()
                        .map(|o| o.label.len() + o.value.len())
                        .sum::<usize>()
            })
            .sum();
        let unique: HashSet<_> = self.controls.iter().map(|c| c.node).collect();
        if self.controls.len() > MAX_CONTROLS
            || self.forms.len() > MAX_CONTROLS
            || self.controls.iter().map(|c| c.options.len()).sum::<usize>() > MAX_OPTIONS
            || bytes > MAX_STATE_BYTES + MAX_VALUE * 4
            || unique.len() != self.controls.len()
        {
            return Err("Invalid tab form data".into());
        }
        Ok(())
    }

    pub fn preserve_edits(&mut self, previous: &Self) {
        for c in &mut self.controls {
            if let Some(old) = previous
                .controls
                .iter()
                .find(|old| old.node == c.node && old.kind == c.kind && old.dirty)
            {
                c.value.clone_from(&old.value);
                c.checked = old.checked;
                c.dirty = true;
                for (o, old) in c.options.iter_mut().zip(&old.options) {
                    if o.value == old.value {
                        o.selected = old.selected;
                    }
                }
            }
        }
    }

    pub fn reset(&mut self, control: NodeId) {
        let Some(owner) = self
            .controls
            .iter()
            .find(|c| c.node == control)
            .and_then(|c| c.owner)
        else {
            return;
        };
        for c in self.controls.iter_mut().filter(|c| c.owner == Some(owner)) {
            c.value.clone_from(&c.initial);
            c.checked = c.initial_checked;
            c.dirty = false;
            for o in &mut c.options {
                o.selected = o.initial;
            }
        }
    }

    pub fn implicit(&self, control: NodeId) -> Option<NodeId> {
        let owner = self.controls.iter().find(|c| c.node == control)?.owner?;
        if let Some(c) = self
            .controls
            .iter()
            .find(|c| c.owner == Some(owner) && c.kind == Kind::Submit)
        {
            return (!c.disabled).then_some(c.node);
        }
        (self
            .controls
            .iter()
            .filter(|c| c.owner == Some(owner) && matches!(c.kind, Kind::Text | Kind::Password))
            .count()
            == 1)
            .then_some(control)
    }

    pub fn submit(
        &self,
        control: NodeId,
        current: &Location,
        base: &Location,
    ) -> Result<FormRequest, String> {
        if self.limited {
            return Err("This form exceeds the interaction limits and cannot be submitted.".into());
        }
        let submitter = self
            .controls
            .iter()
            .find(|c| c.node == control)
            .ok_or("Form control no longer exists.")?;
        if submitter.disabled {
            return Err("This control is disabled.".into());
        }
        let owner = submitter
            .owner
            .ok_or("This control is not associated with a form.")?;
        let form = self.forms.get(&owner).ok_or("Form no longer exists.")?;
        let overrides = if submitter.kind == Kind::Submit {
            &submitter.overrides
        } else {
            form
        };
        let action = overrides
            .action
            .as_ref()
            .or(form.action.as_ref())
            .map(String::as_str)
            .unwrap_or("");
        let location = if action.is_empty() {
            current.clone()
        } else {
            base.resolve(action)?
        };
        if !location.is_remote() {
            return Err("Forms can only submit to HTTP or HTTPS addresses.".into());
        }
        if current.url().scheme() == "https" && location.url().scheme() != "https" {
            return Err("HTTPS forms cannot submit to an insecure address.".into());
        }
        let method = overrides
            .method
            .as_ref()
            .or(form.method.as_ref())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        if method == "dialog" {
            return Err("Dialog forms are not supported.".into());
        }
        let enctype = overrides.enctype.as_ref().or(form.enctype.as_ref());
        if method == "post"
            && enctype.is_some_and(|s| {
                !s.is_empty() && !s.eq_ignore_ascii_case("application/x-www-form-urlencoded")
            })
        {
            return Err("Only URL-encoded forms are supported; multipart and plain-text forms are not yet available.".into());
        }
        let target = overrides.target.as_ref().or(form.target.as_ref());
        if target.is_some_and(|s| !s.is_empty() && !s.eq_ignore_ascii_case("_self")) {
            return Err(
                "Forms currently open in this tab; this form requests another target.".into(),
            );
        }
        let mut encoded = url::form_urlencoded::Serializer::new(String::new());
        let mut encoded_bytes = 0;
        for c in self
            .controls
            .iter()
            .filter(|c| c.owner == Some(owner) && !c.disabled)
        {
            if c.kind == Kind::Unsupported {
                return Err("This form contains an unsupported input type.".into());
            }
            if !form.novalidate && !overrides.novalidate && !c.readonly && c.required {
                let missing = match c.kind {
                    Kind::Checkbox => !c.checked,
                    Kind::Radio if !c.name.is_empty() => !self.controls.iter().any(|other| {
                        other.owner == c.owner
                            && other.kind == Kind::Radio
                            && other.name == c.name
                            && !other.disabled
                            && other.checked
                    }),
                    Kind::Radio => !c.checked,
                    Kind::Select => !c
                        .options
                        .iter()
                        .any(|o| o.selected && !o.disabled && !o.value.is_empty()),
                    Kind::Text | Kind::Password | Kind::Textarea => c.value.is_empty(),
                    _ => false,
                };
                if missing {
                    return Err(format!("Please complete the required field: {}", c.label));
                }
            }
            if c.name.is_empty()
                || matches!(c.kind, Kind::Reset | Kind::Button)
                || (c.kind == Kind::Submit && c.node != control)
                || (matches!(c.kind, Kind::Checkbox | Kind::Radio) && !c.checked)
            {
                continue;
            }
            let values: Vec<&str> = if c.kind == Kind::Select {
                c.options
                    .iter()
                    .filter(|o| o.selected && !o.disabled)
                    .map(|o| o.value.as_str())
                    .collect()
            } else {
                vec![&c.value]
            };
            for value in values {
                let name = crlf(&c.name);
                let value = crlf(if c.kind == Kind::Hidden && c.name == "_charset_" {
                    "UTF-8"
                } else {
                    value
                });
                // Exact byte accounting avoids building an unbounded encoded body.
                let pair = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair(&name, &value)
                    .finish();
                encoded_bytes += pair.len() + usize::from(encoded_bytes != 0);
                if encoded_bytes > MAX_FORM_BYTES {
                    return Err("Form data exceeds the 64 KiB limit.".into());
                }
                encoded.append_pair(&name, &value);
            }
        }
        let encoded = encoded.finish();
        if method == "post" {
            Ok(FormRequest {
                location,
                body: Some(encoded),
            })
        } else {
            let mut url = location.url().clone();
            url.set_query(Some(&encoded));
            Ok(FormRequest {
                location: Location::from_url(url)?,
                body: None,
            })
        }
    }

    pub fn show(&mut self, index: usize, ui: &mut egui::Ui) -> Option<Activation> {
        let c = &mut self.controls[index];
        let mut activate = false;
        let mut radio = false;
        let mut enter = false;
        let node = c.node;
        ui.push_id(("form-control", node), |ui| {
            ui.add_enabled_ui(!c.disabled, |ui| {
                let response = match c.kind {
                    Kind::Text | Kind::Password | Kind::Textarea => {
                        let edit = if c.kind == Kind::Textarea {
                            egui::TextEdit::multiline(&mut c.value).desired_rows(4)
                        } else {
                            egui::TextEdit::singleline(&mut c.value)
                        };
                        let response = ui.add(
                            edit.id_salt("value")
                                .password(c.kind == Kind::Password)
                                .margin(egui::vec2(8.0, 6.0))
                                .interactive(!c.readonly)
                                .hint_text(&c.placeholder)
                                .char_limit(c.maxlength)
                                .desired_width(ui.available_width().min(360.0)),
                        );
                        enter = c.kind != Kind::Textarea
                            && response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        c.dirty |= response.changed();
                        response
                    }
                    Kind::Checkbox => {
                        let r = ui.checkbox(&mut c.checked, &c.label);
                        c.dirty |= r.changed();
                        r
                    }
                    Kind::Radio => {
                        let r = ui.radio(c.checked, &c.label);
                        radio = r.clicked();
                        r
                    }
                    Kind::Select => {
                        let selected = c
                            .options
                            .iter()
                            .filter(|o| o.selected)
                            .map(|o| o.label.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        let mut choice = None;
                        let r = egui::ComboBox::from_id_salt("select")
                            .width(ui.available_width().min(360.0))
                            .selected_text(if selected.is_empty() {
                                "Select…"
                            } else {
                                &selected
                            })
                            .show_ui(ui, |ui| {
                                ui.set_max_width(360.0);
                                for (i, o) in c.options.iter_mut().enumerate() {
                                    if ui
                                        .add_enabled(
                                            !o.disabled,
                                            egui::Button::selectable(o.selected, &o.label).wrap(),
                                        )
                                        .clicked()
                                    {
                                        c.dirty = true;
                                        if c.multiple {
                                            o.selected = !o.selected;
                                        } else {
                                            choice = Some(i);
                                        }
                                    }
                                }
                            })
                            .response;
                        if let Some(choice) = choice {
                            for (i, o) in c.options.iter_mut().enumerate() {
                                o.selected = i == choice;
                            }
                        }
                        r
                    }
                    Kind::Submit | Kind::Reset | Kind::Button => {
                        let r = ui.button(&c.label);
                        activate = r.clicked();
                        r
                    }
                    Kind::Unsupported => {
                        ui.add_enabled(false, egui::Button::new("Unsupported input"))
                    }
                    Kind::Hidden => return,
                };
                if matches!(c.kind, Kind::Text | Kind::Password | Kind::Textarea) {
                    ui.ctx().accesskit_node_builder(response.id, |node| {
                        node.set_label(c.label.clone());
                        if c.readonly {
                            node.set_read_only();
                        }
                    });
                }
                response.on_hover_text(&c.label);
            });
        });
        if radio {
            let owner = c.owner;
            let name = c.name.clone();
            for other in &mut self.controls {
                if other.node == node
                    || (!name.is_empty()
                        && other.kind == Kind::Radio
                        && other.owner == owner
                        && other.name == name)
                {
                    other.checked = other.node == node;
                    other.dirty = true;
                }
            }
        }
        if enter {
            return self.implicit(node).map(|control| Activation { control });
        }
        activate.then_some(Activation { control: node })
    }
}
fn crlf(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn forms(html: &str) -> Forms {
        Forms::from_document(&olive_html::parse(html).unwrap().document, false)
    }
    fn location() -> Location {
        Location::from_input("https://example.com/dir/page?old=1#part").unwrap()
    }
    fn submit(f: &Forms) -> Result<FormRequest, String> {
        let button = f
            .controls
            .iter()
            .find(|c| c.kind == Kind::Submit)
            .unwrap()
            .node;
        f.submit(button, &location(), &location().resolve("/base/").unwrap())
    }
    #[test]
    fn successful_controls_keep_order_duplicates_unicode_and_crlf() {
        let f = forms(
            "<form action='send?discard=1#result'><input name=q value='café & tea'><input name=tag value=a><input name=tag value=b><input name=no disabled value=no><input value=unnamed><input type=checkbox name=on checked><input type=checkbox name=off><input type=hidden name=secret value=yes><textarea name=message>one\ntwo</textarea><select name=s><option>First<option value=two selected>Second</select><select multiple name=m><option selected>A<option disabled selected>B<option selected>C</select><button name=button value=send>Send</button><button name=other value=no>Other</button></form>",
        );
        let request = submit(&f).unwrap();
        assert!(request.body.is_none());
        assert_eq!(
            request.location.as_str(),
            "https://example.com/base/send?q=caf%C3%A9+%26+tea&tag=a&tag=b&on=on&secret=yes&message=one%0D%0Atwo&s=two&m=A&m=C&button=send#result"
        );
    }
    #[test]
    fn post_defaults_overrides_and_current_action() {
        let f = forms("<form method=post><input name=x value=y><button>Send</button></form>");
        let request = submit(&f).unwrap();
        assert_eq!(request.location, location());
        assert_eq!(request.body.as_deref(), Some("x=y"));
        let f = forms(
            "<form action=ignored method=get><input name=x value=y><button formaction=/post formmethod=POST>Send</button></form>",
        );
        let request = submit(&f).unwrap();
        assert_eq!(request.location.url().path(), "/post");
        assert_eq!(request.body.as_deref(), Some("x=y"));
    }
    #[test]
    fn explicit_ownership_fieldsets_legends_and_inert_controls() {
        let f = forms(
            "<input form=f name=before value=1><form id=f><fieldset disabled><legend><input name=legend value=yes></legend><input name=no value=no><legend><input name=no2 value=no></legend></fieldset><fieldset disabled><legend><fieldset disabled><input name=nested value=no></fieldset></legend></fieldset><input form=missing name=orphan value=no><datalist><input name=inert value=no></datalist><button>Send</button></form><input form=f name=after value=2>",
        );
        assert_eq!(
            submit(&f).unwrap().location.url().query(),
            Some("before=1&legend=yes&after=2")
        );
    }
    #[test]
    fn radios_reset_and_edits_survive_presentation_updates() {
        let html = "<form><input name=text value=initial><input type=radio name=r value=a checked><input type=radio name=r value=b checked><select name=s><option selected>A<option>B</select><button>Send</button><button type=reset>Reset</button></form>";
        let mut f = forms(html);
        assert!(!f.controls[1].checked);
        assert!(f.controls[2].checked);
        f.controls[0].value = "edited".into();
        f.controls[0].dirty = true;
        f.controls[3].options[0].selected = false;
        f.controls[3].options[1].selected = true;
        f.controls[3].dirty = true;
        let mut update = forms(html);
        update.preserve_edits(&f);
        assert_eq!(
            submit(&update).unwrap().location.url().query(),
            Some("text=edited&r=b&s=B")
        );
        let reset = update.controls.last().unwrap().node;
        update.reset(reset);
        assert_eq!(
            submit(&update).unwrap().location.url().query(),
            Some("text=initial&r=b&s=A")
        );
    }
    #[test]
    fn required_fields_implicit_submission_and_no_validate() {
        let mut f = forms("<form><input name=q required><button>Send</button></form>");
        assert!(submit(&f).unwrap_err().contains("required"));
        assert_eq!(f.implicit(f.controls[0].node), Some(f.controls[1].node));
        f.controls[0].value = "ok".into();
        assert!(submit(&f).is_ok());
        let f = forms("<form novalidate><input required name=q><button>Send</button></form>");
        assert!(submit(&f).is_ok());
        let f = forms("<form><input><input></form>");
        assert!(f.implicit(f.controls[0].node).is_none());
        let f = forms("<form><input name=q></form>");
        assert_eq!(f.implicit(f.controls[0].node), Some(f.controls[0].node));
        let f = forms("<form><input><button disabled>Send</button></form>");
        assert!(f.implicit(f.controls[0].node).is_none());
        let mut f = forms(
            "<form><input type=radio name=r required><input type=radio name=r checked disabled><button>Send</button></form>",
        );
        assert!(submit(&f).unwrap_err().contains("required"));
        f.controls[0].checked = true;
        assert!(submit(&f).is_ok());
    }
    #[test]
    fn rejects_unsupported_targets_encodings_types_and_excessive_payloads() {
        for html in [
            "<form action='file:///tmp/page'><button>Send</button></form>",
            "<form action='http://example.com'><button>Send</button></form>",
            "<form action='javascript:alert(1)'><button>Send</button></form>",
            "<form method=post enctype=multipart/form-data><button>Send</button></form>",
            "<form target=_blank><button>Send</button></form>",
            "<form><input type=file name=file><button>Send</button></form>",
        ] {
            assert!(submit(&forms(html)).is_err(), "{html}");
        }
        let mut f =
            forms("<form method=post><textarea name=q></textarea><button>Send</button></form>");
        f.controls[0].value = "♥".repeat(16_384);
        assert!(submit(&f).unwrap_err().contains("64 KiB"));
        let f = forms(&format!(
            "<form>{}<button>Send</button></form>",
            "<input>".repeat(MAX_CONTROLS + 1)
        ));
        assert!(f.limited);
        assert_eq!(f.controls.len(), MAX_CONTROLS);
        f.validate().unwrap();
    }
}
