# Security boundary in 0.0.1

Olive 0.0.1 parses local or stdin UTF-8 HTML into an inert DOM. It does not load
external resources, resolve URLs, execute JavaScript, enforce CSP, or render
HTML. Script content and event-handler attributes remain in the DOM as data;
**the parser is not an HTML sanitizer**.

The CLI accepts at most 1 MiB. Library callers can configure this limit.
Recovery diagnostics are capped at 64 by default and 512 bytes per message.
Tree output escapes terminal control characters and limits indentation.
Olive's flat arena avoids ownership cycles and recursive DOM drop, and all
first-party Rust targets forbid unsafe code.

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
