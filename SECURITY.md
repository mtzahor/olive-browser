# Security boundary in 0.0.3

Olive 0.0.3 parses local or stdin UTF-8 HTML into an inert DOM. The optional GUI
renders text and a bounded subset of embedded/inline CSS from that DOM. It
does not load external resources, resolve URLs, execute JavaScript, or enforce CSP.
Script content and event-handler attributes remain in the DOM as data;
**the parser is not an HTML sanitizer**.

The GUI and CLI accept at most 1 MiB. Library callers can configure this limit.
Recovery diagnostics are capped at 64 by default and 512 bytes per message.
Tree output escapes terminal control characters and limits indentation.
Olive's flat arena avoids ownership cycles and recursive DOM drop, and all
first-party Rust targets forbid unsafe code.

The GUI adds a 200,000-character / 5,000-text-block / 10,000-box display limit,
reads files on one
background worker, and retains the previous page if a new file cannot be read.
Its CSS parser, font and native UI dependencies expand the trust boundary beyond the parser.
Links do not navigate; page-controlled resources are never opened.

CSS is data only: linked stylesheets, imports, all at-rules, URL backgrounds and
page fonts are ignored. A combined 256 KiB CSS input budget, 2,048-rule limit,
128-declaration limit per block, selector size limits and a per-document
2,000,000-step matching budget bound stored styles and matching work. Exceeded
limits show a notice; unsupported syntax is skipped. DOM conversion and nested
box layout are iterative. Font sizes, geometry and line heights are clamped as
documented in README.md. CSS can change the document area but not browser
controls or the file picker.

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
