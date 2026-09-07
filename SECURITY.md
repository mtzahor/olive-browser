# Security boundary in 0.0.4

Olive's HTML parser accepts local or stdin UTF-8 HTML and produces an inert DOM.
The optional GUI renders text and embedded/inline CSS, and now explicitly runs
inline classic JavaScript before rendering. Enabling the `js` feature alone never
executes HTML. The `olive` DOM inspector remains inert; `olive-js` explicitly runs
standalone JavaScript. Olive does not load external resources or enforce CSP.
**The parser is not an HTML sanitizer, and script execution is not sandboxed.**

The GUI and HTML CLI accept at most 1 MiB. Library callers can configure this limit.
Recovery diagnostics are capped at 64 by default and 512 bytes per message.
Tree output escapes terminal control characters and limits indentation.
Olive's flat arena avoids ownership cycles and recursive DOM drop, and all
first-party Rust targets forbid unsafe code.

The GUI adds a 200,000-character / 5,000-text-block / 10,000-box display limit,
reads files on one
background worker, and retains the previous page if a new file cannot be read.
Its CSS parser, JavaScript engine/garbage collector, font and native UI dependencies
expand the trust boundary beyond the HTML parser.
Links do not navigate; page-controlled resources are never opened.

CSS is data only: linked stylesheets, imports, all at-rules, URL backgrounds and
page fonts are ignored. A combined 256 KiB CSS input budget, 2,048-rule limit,
128-declaration limit per block, selector size limits and a per-document
2,000,000-step matching budget bound stored styles and matching work. Exceeded
limits show a notice; unsupported syntax is skipped. DOM conversion and nested
box layout are iterative. Font sizes, geometry and line heights are clamped as
documented in README.md. CSS can change the document area but not browser
controls or the file picker.

JavaScript uses Boa 0.22.0 in-process on the file-loading worker, with a fresh
realm per document. No network/filesystem APIs, timers, module loader, dynamic
code compilation (`eval`/function constructors), or asynchronous job execution
are enabled. The host dispatches only inline `onclick` handlers on rendered
elements; page code can only mutate the current document via
the supported DOM API and append to bounded console output. Mutating an attribute
never opens a URL. New script elements, templates, foreign trees and embedded
documents are not executed. Scripts run after full HTML parsing, not interleaved
with it; this is not browser lifecycle or CSP conformance.

JavaScript source defaults to 256 KiB combined and 64 attempted inline scripts.
A conservative preflight counts raw punctuation/delimiters, including those in
strings/comments, rejecting sources over 512 marks or 32 nested levels before
Boa's recursive parser/compiler. Prefix/control keywords and `!~?:` also share
a 32-marker cap. This intentionally rejects some valid programs.
The runtime shares one million VM instructions across all scripts (using Boa's
`fuzz` instruction-accounting feature), including callbacks from built-ins.
Loops also cap at 100,000 iterations; function recursion at 128 and VM stack size
at 16,384. DOM operations/traversal share one million steps. Cumulative text/
attribute writes cap at 256 KiB, and added arena nodes at 5,000, including detached
nodes. Detaching/replacing nodes preserves identities and cannot create cycles.
Console output and errors retain at most 64 messages each, bounded to 1,024 bytes.
Host arguments avoid invoking user-defined coercion and output previews do not
invoke getters. An execution or DOM limit stops subsequent scripts; mutations
already performed remain visible. These limits are separate from CSS/display limits.

VM instructions do not meter parsing, compilation, allocations or time spent
inside native built-ins such as regular expressions and large string/array
operations. Scripts can still consume excessive memory/CPU or trigger dependency
defects, including process-aborting failures. Use local documents you trust;
process isolation with enforceable memory/time limits is required before exposing
this viewer to arbitrary active web content. The UI worker is not a security boundary.

These controls do not provide a hard memory ceiling or processing deadline.
DOM construction expands input, parser recovery can be expensive, allocations
can fail, and dependencies can contain defects and unsafe code. The parser has
not undergone an independent security audit. A network-facing browser will
need process isolation, enforceable resource budgets, and a separate design
for navigation and active content before it can safely browse arbitrary sites.

## Reporting

Please use GitHub's private vulnerability reporting feature if it is available
on this repository. Otherwise, contact the repository owner to arrange private
disclosure before posting sensitive reproduction details publicly.
