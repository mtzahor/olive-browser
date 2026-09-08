//! Optional JavaScript parsing and interpretation using Boa.
//!
//! A runtime owns one realm. Parsing never executes code; programs can only run
//! in their originating runtime. HTML parsing stays inert until `run_document`
//! is called. This preview implements synchronous scripts, not a browser event loop.
//!
//! ```
//! use olive_html::js::{Runtime, ScriptOptions};
//! let mut runtime = Runtime::new(ScriptOptions::default())?;
//! let program = runtime.parse("const square = x => x * x; square(7)")?;
//! assert_eq!(runtime.execute(&program)?, "49");
//! # Ok::<(), olive_html::js::ScriptError>(())
//! ```

mod bindings;
mod complexity;
use complexity::check_complexity;
pub(crate) mod document;
pub use document::{DocumentSession, ScriptDiagnostic, ScriptReport, run_document};

use boa_engine::{
    Context, JsError, JsNativeError, JsResult, JsString, JsValue, Source, context::HostHooks,
    job::IdleJobExecutor, module::IdleModuleLoader, realm::Realm, script::Script,
};
use std::{fmt, io::Read, rc::Rc};

/// Preview limits, not a heap sandbox or a wall-clock deadline.
#[derive(Clone, Copy, Debug)]
pub struct ScriptOptions {
    /// Per-source and combined inline/external source budget; default 256 KiB.
    pub max_input_bytes: usize,
    /// Maximum scripts attempted per document; default 64.
    pub max_scripts: usize,
    /// Maximum retained diagnostics and console messages; default 64 each.
    pub max_messages: usize,
    /// Maximum bytes in a result or message; default 1,024.
    pub max_output_bytes: usize,
    /// Total VM instructions across all scripts in this runtime; default 1,000,000.
    pub max_instructions: usize,
    /// Boa loop iterations per loop; default 100,000.
    pub max_loop_iterations: u64,
    /// JavaScript function recursion depth; default 128.
    pub max_recursion: usize,
    /// Shared DOM traversal/operation budget; default 1,000,000.
    pub max_dom_operations: usize,
    /// Cumulative text and attribute writes; default 256 KiB.
    pub max_dom_bytes: usize,
    /// Additional DOM nodes, including detached ones; default 5,000.
    pub max_new_nodes: usize,
}
impl Default for ScriptOptions {
    fn default() -> Self {
        Self {
            max_input_bytes: 256 * 1024,
            max_scripts: 64,
            max_messages: 64,
            max_output_bytes: 1024,
            max_instructions: 1_000_000,
            max_loop_iterations: 100_000,
            max_recursion: 128,
            max_dom_operations: 1_000_000,
            max_dom_bytes: 256 * 1024,
            max_new_nodes: 5_000,
        }
    }
}

impl ScriptOptions {
    /// Viewer profile for downloaded bundles. Execution/DOM limits are unchanged.
    /// Standalone runtimes retain the smaller default source and script budgets.
    pub fn browser() -> Self {
        Self {
            max_input_bytes: 32 * 1024 * 1024,
            max_scripts: 256,
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Syntax,
    Runtime,
    Limit,
    Input,
    WrongRuntime,
}

/// A bounded diagnostic. Syntax errors reject the whole script.
#[derive(Debug)]
pub struct ScriptError {
    pub kind: ErrorKind,
    pub message: String,
}
impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ScriptError {}
impl ScriptError {
    fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: truncate(&message.into(), 1024),
        }
    }
    fn engine(error: JsError, kind: ErrorKind) -> Self {
        let kind = if error.as_engine().is_some() {
            ErrorKind::Limit
        } else {
            kind
        };
        // Display never invokes page-defined toString/getters. Stop formatting
        // once the message is full, even for a very large thrown string.
        let mut message = BoundedMessage(String::new());
        let _ = fmt::write(&mut message, format_args!("{error}"));
        Self {
            kind,
            message: message.0,
        }
    }
}

struct BoundedMessage(String);
impl fmt::Write for BoundedMessage {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let remaining = 1024usize.saturating_sub(self.0.len());
        self.0.push_str(&truncate(text, remaining));
        if text.len() > remaining {
            Err(fmt::Error)
        } else {
            Ok(())
        }
    }
}

/// Parsed code tied to its originating runtime. Contains no executed side effects.
#[derive(Debug)]
pub struct Program {
    script: Script,
    owner: Rc<()>,
}

/// One isolated JavaScript global environment, with captured console output.
/// After a resource limit it is unusable; create a fresh runtime to continue.
pub struct Runtime {
    context: Context,
    owner: Rc<()>,
    options: ScriptOptions,
    stopped: bool,
}

struct Hooks;
impl HostHooks for Hooks {
    fn ensure_can_compile_strings(
        &self,
        _: Realm,
        _: &[JsString],
        _: &JsString,
        _: bool,
        _: &mut Context,
    ) -> JsResult<()> {
        Err(JsNativeError::typ()
            .with_message("eval and Function constructors are unavailable in Olive")
            .into())
    }
}

impl Runtime {
    pub fn new(options: ScriptOptions) -> Result<Self, ScriptError> {
        let mut context = Context::builder()
            .host_hooks(Rc::new(Hooks))
            .module_loader(Rc::new(IdleModuleLoader))
            .job_executor(Rc::new(IdleJobExecutor))
            .can_block(false)
            .instructions_remaining(options.max_instructions)
            .build()
            .map_err(|e| ScriptError::engine(e, ErrorKind::Runtime))?;
        let limits = context.runtime_limits_mut();
        limits.set_loop_iteration_limit(options.max_loop_iterations);
        limits.set_recursion_limit(options.max_recursion);
        limits.set_stack_size_limit(16_384);
        limits.set_backtrace_limit(8);
        bindings::install_console(&mut context, options)
            .map_err(|e| ScriptError::engine(e, ErrorKind::Runtime))?;
        Ok(Self {
            context,
            owner: Rc::new(()),
            options,
            stopped: false,
        })
    }

    /// Parse a classic ECMAScript script without evaluating it.
    pub fn parse(&mut self, source: &str) -> Result<Program, ScriptError> {
        self.check_running()?;
        if source.len() > self.options.max_input_bytes {
            return Err(ScriptError::new(
                ErrorKind::Limit,
                format!(
                    "JavaScript input exceeds the {}-byte limit",
                    self.options.max_input_bytes
                ),
            ));
        }
        check_complexity(source)?;
        let script = Script::parse(Source::from_bytes(source), None, &mut self.context)
            .map_err(|e| ScriptError::engine(e, ErrorKind::Syntax))?;
        Ok(Program {
            script,
            owner: self.owner.clone(),
        })
    }

    /// Read at most the source limit plus one byte. Invalid UTF-8 is rejected.
    pub fn parse_reader(&mut self, reader: impl Read) -> Result<Program, ScriptError> {
        let mut bytes = Vec::new();
        reader
            .take((self.options.max_input_bytes as u64).saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| ScriptError::new(ErrorKind::Input, e.to_string()))?;
        if bytes.len() > self.options.max_input_bytes {
            return Err(ScriptError::new(
                ErrorKind::Limit,
                "JavaScript input exceeds the source limit",
            ));
        }
        let source = std::str::from_utf8(&bytes)
            .map_err(|_| ScriptError::new(ErrorKind::Input, "JavaScript input must be UTF-8"))?;
        self.parse(source)
    }

    /// Evaluate a parsed program, returning a bounded, side-effect-free preview.
    /// Objects are shown as `[object]`; their getters/toString are never called.
    pub fn execute(&mut self, program: &Program) -> Result<String, ScriptError> {
        self.execute_value(program)
            .map(|value| preview(&value, self.options.max_output_bytes))
    }

    fn execute_value(&mut self, program: &Program) -> Result<JsValue, ScriptError> {
        self.check_running()?;
        if !Rc::ptr_eq(&self.owner, &program.owner) {
            return Err(ScriptError::new(
                ErrorKind::WrongRuntime,
                "Program belongs to another JavaScript runtime",
            ));
        }
        let result = program.script.evaluate(&mut self.context);
        if bindings::limited(&self.context) {
            self.stopped = true;
            return Err(ScriptError::new(
                ErrorKind::Limit,
                "JavaScript DOM budget exhausted",
            ));
        }
        self.finish_execution(result)
    }

    fn finish_execution(&mut self, result: JsResult<JsValue>) -> Result<JsValue, ScriptError> {
        result.map_err(|error| {
            let error = ScriptError::engine(error, ErrorKind::Runtime);
            self.stopped |= error.kind == ErrorKind::Limit;
            error
        })
    }

    pub fn eval(&mut self, source: &str) -> Result<String, ScriptError> {
        let program = self.parse(source)?;
        self.execute(&program)
    }

    pub fn console(&self) -> Vec<String> {
        bindings::console(&self.context)
    }
    pub fn omitted_console_messages(&self) -> usize {
        bindings::omitted_console(&self.context)
    }

    fn check_running(&self) -> Result<(), ScriptError> {
        if self.stopped {
            Err(ScriptError::new(
                ErrorKind::Limit,
                "JavaScript runtime stopped after a resource limit",
            ))
        } else {
            Ok(())
        }
    }
}

pub(super) fn truncate(value: &str, limit: usize) -> String {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}
fn preview(value: &JsValue, limit: usize) -> String {
    if let Some(s) = value.as_string() {
        // Bound conversion before allocating a Rust copy of a potentially huge JS string.
        let units: Vec<u16> = s.iter().take(limit).collect();
        truncate(&String::from_utf16_lossy(&units), limit)
    } else if value.is_object() {
        truncate("[object]", limit)
    } else if value.is_symbol() {
        truncate("[symbol]", limit)
    } else if value.is_bigint() {
        truncate("[bigint]", limit)
    } else {
        truncate(&value.display().to_string(), limit)
    }
}
