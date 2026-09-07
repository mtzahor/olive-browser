use super::{ErrorKind, Runtime, ScriptError, ScriptOptions, bindings, truncate};
use crate::{Document, NodeId, NodeKind};

#[derive(Clone, Debug)]
pub struct ScriptDiagnostic {
    /// Script element, or `None` for runtime setup errors.
    pub node: Option<NodeId>,
    pub kind: ErrorKind,
    pub message: String,
}
#[derive(Clone, Debug, Default)]
pub struct ScriptReport {
    pub attempted: usize,
    pub executed: usize,
    /// External scripts, modules and unsupported script types.
    pub skipped: usize,
    pub diagnostics: Vec<ScriptDiagnostic>,
    pub omitted_diagnostics: usize,
    pub console: Vec<String>,
    pub omitted_console_messages: usize,
    pub limited: bool,
    pub clicks: usize,
    pub omitted_alerts: usize,
}
impl ScriptReport {
    fn error(&mut self, node: Option<NodeId>, error: ScriptError, options: ScriptOptions) {
        self.limited |= error.kind == ErrorKind::Limit;
        if self.diagnostics.len() < options.max_messages {
            self.diagnostics.push(ScriptDiagnostic {
                node,
                kind: error.kind,
                message: truncate(&error.message, options.max_output_bytes),
            });
        } else {
            self.omitted_diagnostics = self.omitted_diagnostics.saturating_add(1);
        }
    }
}

/// Execute connected, inline classic HTML scripts once in tree order, after
/// parsing the full document. Each document gets a fresh realm. This is not
/// parser-blocking script execution: every script sees the complete initial DOM.
/// Templates and foreign/embedded document subtrees are inert. Inline `onclick`
/// handlers run only through an explicit [`DocumentSession::click`] call; no
/// other event handlers, modules, jobs, timers or external sources are executed.
/// A fatal execution limit stops the remaining scripts; ordinary exceptions
/// allow the next script.
/// Mutations made before an error are retained, like JavaScript side effects.
pub fn run_document(document: Document, options: ScriptOptions) -> (Document, ScriptReport) {
    DocumentSession::new(document, options).into_parts()
}

/// A document and its persistent JavaScript realm. Keep this on its owning thread.
/// Inline `onclick` handlers run only when the host explicitly calls `click`.
pub struct DocumentSession {
    runtime: Option<Runtime>,
    fallback: Option<Document>,
    report: ScriptReport,
}
impl DocumentSession {
    /// Attach a fresh realm to a document whose initial inline scripts have
    /// already run. This lets the GUI keep event-handler state alive without
    /// executing page-load scripts a second time.
    pub fn attach(document: Document, options: ScriptOptions, report: ScriptReport) -> Self {
        let mut runtime = match Runtime::new(options) {
            Ok(runtime) => runtime,
            Err(error) => {
                let mut report = report;
                report.error(None, error, options);
                return Self {
                    runtime: None,
                    fallback: Some(document),
                    report,
                };
            }
        };
        if let Err(error) = bindings::install_document(&mut runtime.context, document) {
            let mut report = report;
            report.error(
                None,
                ScriptError::engine(error, ErrorKind::Runtime),
                options,
            );
            let document = bindings::take_document(&runtime.context);
            return Self {
                runtime: None,
                fallback: Some(document),
                report,
            };
        }
        Self {
            runtime: Some(runtime),
            fallback: None,
            report,
        }
    }

    pub fn new(document: Document, options: ScriptOptions) -> Self {
        let mut report = ScriptReport::default();
        // Snapshot only IDs, never sources. Later scripts can change or remove an
        // earlier-discovered element; newly-created scripts never execute this pass.
        let scripts: Vec<_> = document
            .descendants(document.root())
            .filter(|&id| {
                document
                    .node(id)
                    .and_then(|n| n.as_element())
                    .is_some_and(|e| {
                        e.name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                            && e.name.local.as_ref() == "script"
                    })
                    && eligible_ancestry(&document, id)
            })
            .collect();
        let mut runtime = match Runtime::new(options) {
            Ok(runtime) => runtime,
            Err(error) => {
                report.error(None, error, options);
                return Self {
                    runtime: None,
                    fallback: Some(document),
                    report,
                };
            }
        };
        if let Err(error) = bindings::install_document(&mut runtime.context, document) {
            report.error(
                None,
                ScriptError::engine(error, ErrorKind::Runtime),
                options,
            );
            let document = bindings::take_document(&runtime.context);
            return Self {
                runtime: None,
                fallback: Some(document),
                report,
            };
        }
        let mut remaining = options.max_input_bytes;
        for id in scripts {
            let document = bindings::take_document(&runtime.context);
            let script = document.node(id).unwrap().as_element().unwrap();
            if !eligible_ancestry(&document, id) {
                bindings::restore_document(&runtime.context, document);
                continue;
            }
            if script.attribute("src").is_some()
                || !classic_type(script.attribute("type"), script.attribute("language"))
            {
                report.skipped += 1;
                bindings::restore_document(&runtime.context, document);
                continue;
            }
            if report.attempted >= options.max_scripts {
                report.error(
                    Some(id),
                    ScriptError::new(ErrorKind::Limit, "Inline script count limit exceeded"),
                    options,
                );
                bindings::restore_document(&runtime.context, document);
                break;
            }
            let mut source = String::new();
            let mut oversized = false;
            for child in document.children(id) {
                if let NodeKind::Text(text) = &document.node(child).unwrap().kind {
                    if text.len() > remaining.saturating_sub(source.len()) {
                        oversized = true;
                        break;
                    }
                    source.push_str(text);
                }
            }
            bindings::restore_document(&runtime.context, document);
            if oversized {
                report.error(
                    Some(id),
                    ScriptError::new(
                        ErrorKind::Limit,
                        "Combined inline JavaScript source limit exceeded",
                    ),
                    options,
                );
                break;
            }
            remaining -= source.len();
            report.attempted += 1;
            match runtime.eval(&source) {
                Ok(_) => report.executed += 1,
                Err(error) => report.error(Some(id), error, options),
            }
            if runtime.stopped {
                break;
            }
        }
        let mut session = Self {
            runtime: Some(runtime),
            fallback: None,
            report,
        };
        session.refresh_report();
        session
    }

    pub fn with_document<T>(&self, read: impl FnOnce(&Document) -> T) -> T {
        match &self.runtime {
            Some(runtime) => bindings::with_document(&runtime.context, read),
            None => read(self.fallback.as_ref().expect("fallback document")),
        }
    }
    pub fn report(&self) -> &ScriptReport {
        &self.report
    }
    pub fn take_alerts(&mut self) -> Vec<String> {
        self.runtime
            .as_ref()
            .map(|r| bindings::take_alerts(&r.context))
            .unwrap_or_default()
    }
    pub fn into_parts(mut self) -> (Document, ScriptReport) {
        let document = match self.runtime.take() {
            Some(runtime) => bindings::take_document(&runtime.context),
            None => self.fallback.take().expect("fallback document"),
        };
        (document, self.report)
    }
    fn refresh_report(&mut self) {
        if let Some(runtime) = &self.runtime {
            self.report.console = runtime.console();
            self.report.omitted_console_messages = runtime.omitted_console_messages();
            self.report.omitted_alerts = bindings::omitted_alerts(&runtime.context);
        }
    }

    /// Invoke the current `onclick` attribute with this element as `this` and
    /// `event.target/currentTarget`. `false` is returned for `return false`.
    /// This small host does not navigate, submit forms or bubble events.
    pub fn click(&mut self, target: NodeId) -> bool {
        let source = self.with_document(|doc| {
            let e = doc.node(target)?.as_element()?;
            if !eligible_ancestry(doc, target)
                || e.name.ns.as_ref() != "http://www.w3.org/1999/xhtml"
                || e.attribute("hidden").is_some()
                || e.attribute("disabled").is_some()
            {
                return None;
            }
            e.attribute("onclick").map(str::to_owned)
        });
        let Some(source) = source else {
            return true;
        };
        let Some(runtime) = &mut self.runtime else {
            return true;
        };
        self.report.clicks = self.report.clicks.saturating_add(1);
        let result = (|| {
            runtime.check_running()?;
            if source.len() > runtime.options.max_input_bytes {
                return Err(ScriptError::new(
                    ErrorKind::Limit,
                    "Click handler exceeds the JavaScript source limit",
                ));
            }
            super::check_complexity(&source)?;
            // Validate a complete function body before wrapping it. A stray `}`
            // must not escape the wrapper and run as a separate script.
            boa_parser::Parser::new(boa_engine::Source::from_bytes(&source))
                .parse_function_body(runtime.context.interner_mut(), false, false)
                .map_err(|e| ScriptError::new(ErrorKind::Syntax, e.to_string()))?;
            let program = runtime.parse(&format!("(function(event) {{\n{source}\n}})"))?;
            let function = runtime
                .execute_value(&program)?
                .as_callable()
                .ok_or_else(|| ScriptError::new(ErrorKind::Runtime, "Invalid click handler"))?;
            let this = bindings::wrap(&mut runtime.context, Some(target))
                .map_err(|e| ScriptError::engine(e, ErrorKind::Runtime))?;
            let event = boa_engine::object::ObjectInitializer::new(&mut runtime.context)
                .property(
                    boa_engine::js_string!("type"),
                    boa_engine::js_string!("click"),
                    boa_engine::property::Attribute::READONLY,
                )
                .property(
                    boa_engine::js_string!("target"),
                    this.clone(),
                    boa_engine::property::Attribute::READONLY,
                )
                .property(
                    boa_engine::js_string!("currentTarget"),
                    this.clone(),
                    boa_engine::property::Attribute::READONLY,
                )
                .build();
            let result = function.call(&this, &[event.into()], &mut runtime.context);
            if bindings::limited(&runtime.context) {
                runtime.stopped = true;
                return Err(ScriptError::new(
                    ErrorKind::Limit,
                    "JavaScript DOM budget exhausted",
                ));
            }
            runtime.finish_execution(result)
        })();
        let allowed = match result {
            Ok(value) => value.as_boolean() != Some(false),
            Err(error) => {
                self.report.error(Some(target), error, runtime.options);
                true
            }
        };
        self.refresh_report();
        allowed
    }
}

fn eligible_ancestry(doc: &Document, id: NodeId) -> bool {
    let mut next = doc.node(id).and_then(|n| n.parent());
    while let Some(parent) = next {
        if parent == doc.root() {
            return true;
        }
        let node = doc.node(parent).unwrap();
        if let Some(e) = node.as_element() {
            if e.name.ns.as_ref() != "http://www.w3.org/1999/xhtml"
                || matches!(
                    e.name.local.as_ref(),
                    "template" | "noscript" | "iframe" | "object" | "embed"
                )
            {
                return false;
            }
        }
        next = node.parent();
    }
    false
}
fn classic_type(kind: Option<&str>, language: Option<&str>) -> bool {
    let kind = match kind {
        Some(kind) => kind.trim().to_ascii_lowercase(),
        None => match language {
            Some(language) if !language.is_empty() => {
                format!("text/{}", language.to_ascii_lowercase())
            }
            _ => String::new(),
        },
    };
    matches!(
        kind.as_str(),
        "" | "text/javascript"
            | "application/javascript"
            | "text/ecmascript"
            | "application/ecmascript"
            | "application/x-ecmascript"
            | "application/x-javascript"
            | "text/javascript1.0"
            | "text/javascript1.1"
            | "text/javascript1.2"
            | "text/javascript1.3"
            | "text/javascript1.4"
            | "text/javascript1.5"
            | "text/jscript"
            | "text/livescript"
            | "text/x-ecmascript"
            | "text/x-javascript"
    )
}
