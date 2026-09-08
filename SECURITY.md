# Security boundary in 0.2.0

Olive's HTML parser accepts local or stdin UTF-8 HTML and produces an inert DOM.
The optional GUI renders text and linked/embedded/inline CSS. Local documents run
inline and external classic JavaScript automatically. Web documents start with
JavaScript disabled: no script sources are fetched and no handlers run until the
user chooses **Enable JavaScript** for that page. Reload and same-page navigation
preserve this choice; other document navigation and changed redirect destinations
reset it. **Disable JavaScript** reloads without fetching or running scripts.
Enabling the `js` feature alone never
executes HTML. The `olive` DOM inspector remains inert; `olive-js` explicitly runs
standalone JavaScript. The optional `net` feature explicitly loads HTTP(S)
documents, and is included in the GUI. Parsing itself never performs networking.
The viewer explicitly fetches supported CSS/JS resources; Olive does not enforce CSP.
**The parser is not an HTML sanitizer, and script execution is not sandboxed.**

The GUI and HTML CLI accept at most 1 MiB. Library callers can configure this limit.
Recovery diagnostics are capped at 64 by default and 512 bytes per message.
Tree output escapes terminal control characters and limits indentation.
Olive's flat arena avoids ownership cycles and recursive DOM drop, and all
first-party Rust targets forbid unsafe code.

The GUI adds a 200,000-character / 5,000-text-block / 10,000-box display limit,
reads files on one
background worker, and retains the previous page if a new document cannot be read.
Its CSS parser, JavaScript engine/garbage collector, font and native UI dependencies
expand the trust boundary beyond the HTML parser.
Links navigate only after a user click. HTTP(S) pages cannot navigate to `file:`;
`javascript:`, `data:`, `ftp:`, custom protocols, remote file authorities, and
URLs containing credentials are rejected. Redirects obey the same scheme and
credential rules. Local file links can navigate to other local files or websites.

HTTP uses reqwest with Rustls, WebPKI roots and the operating system's trusted
certificates, with normal hostname/certificate
verification and no certificate-bypass option. Explicit HTTP and HTTPS-to-HTTP
redirects are allowed; the final address and transport are shown in browser chrome.
Requests have a 10-second connection timeout, 20-second total timeout and ten-hop
redirect limit. Document responses are bounded to 1 MiB after decompression and
again after decoding; document types other than HTML/XHTML/plain text are rejected. Plain text
is escaped before display. URL inputs are capped at 8 KiB and history at 256 entries.
Only one navigation load runs at a time. Navigation waits for that load to finish.
No cookies, HTTP authentication, automatic Referer headers, persistent network
cache, forms, downloads or automatic document navigation are enabled.
The client honors HTTP(S) proxy environment variables. Loopback and private-network
addresses are allowed, including for subresources; this is not an SSRF-filtering API.

Page resources resolve against the final document URL and first base href, with
at most 64 attempts and a shared 20-second network deadline after document loading.
Each resource is capped at 8 MiB after decompression and UTF-8 decoding, with
at most 8 MiB of external CSS and 32 MiB of external JavaScript retained per page.
A response that does not fit the remaining page budget is discarded; its error
reports the page budget separately from the individual resource ceiling.
Rejected responses also have bounded reads; count and time limits bound repeated
failures. Raising the byte ceilings does not relax VM or DOM execution limits. HTTP errors and missing or
incorrect CSS/JavaScript MIME types are rejected. HTTPS pages cannot load HTTP
resources, and web pages cannot load files, including via redirects or base href.
Local documents can load local and remote sources. Cross-origin classic scripts
and stylesheets are allowed; no credentials or Referer are sent. Nonempty integrity
attributes are rejected because integrity verification is not implemented.
Failures leave other resources and the document available; up to 64 resource
errors plus one budget notice are retained, at most 1,024 bytes per message.
Resource collection is a single initial snapshot; CSS imports and dynamically
inserted or changed resource URLs never fetch. Unsupported modules, alternate,
disabled and non-screen stylesheets are skipped. A resource may be fetched before
an earlier script removes its element; removed scripts do not execute.

CSS is data only: imports, all at-rules, URL backgrounds and page fonts are ignored.
A combined 8 MiB linked/embedded/inline CSS input budget, 16,384-rule limit,
128-declaration limit per block, selector size limits and a per-document
2,000,000-step matching budget bound stored styles and matching work. Exceeded
limits show a notice; unsupported syntax is skipped. DOM conversion and nested
box layout are iterative. Font sizes, geometry and line heights are clamped as
documented in README.md. CSS can change the document area but not browser
controls or the file picker.

JavaScript uses Boa 0.22.0 in-process, with one fresh realm per document retained
on its worker for initial scripts and subsequent clicks. Initial scripts execute
once; the UI receives only the presentation and bounded reports. No network/filesystem APIs, timers, module loader, dynamic
code compilation (`eval`/function constructors), or asynchronous job execution
are enabled. The host dispatches only inline `onclick` handlers on rendered
elements; page code can only mutate the current document via
the supported DOM API and append to bounded console output. Mutating an attribute
never automatically opens a URL; a later user click can follow a changed link.
New script elements, templates, foreign trees and embedded
documents are not executed. Scripts run after full HTML parsing, not interleaved
with it, in document order regardless of async/defer; this is not browser lifecycle
or CSP conformance. Script source mutation does not trigger another fetch.

Standalone JavaScript source defaults to 256 KiB combined inline/external and
64 attempted scripts. The viewer uses 32 MiB / 256 attempts for web bundles.
A conservative preflight bounds active expression paths to 512 punctuation marks,
32 delimiter levels and 32 prefix/control markers before Boa's recursive
parser/compiler. Statement boundaries reset punctuation counts within their own
block; strings and comments are skipped. Control counts remain active to reject
unbraced if/else chains. Ambiguous slash goals, template substitutions and HTML
comments use the earlier whole-source guard, counting raw punctuation including
literal data. This intentionally still rejects some valid large programs.

Document-only navigator properties provide fixed basic metadata, not access to
hardware or user accounts. Local/session storage implement bounded methods, with
separate in-memory stores of 64 KiB / 128 keys for each document realm. No storage
persists or is shared between pages. Storage operations share the DOM operation
and cumulative write budgets. Get-by-tag queries return snapshots of at most
5,000 HTML elements and charge traversal to the DOM work budget. Unsupported
lifecycle/timer APIs are not replaced with successful no-op implementations.
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
defects, including process-aborting failures. Use local documents and enable web
JavaScript only for pages and their script providers you trust. Web scripting is
an explicit experimental opt-in, not safe execution of untrusted code. Process
isolation with enforceable memory/time limits remains required before automatic
execution of untrusted remote JavaScript. The UI worker is not a security boundary.

These controls do not provide a hard memory ceiling or processing deadline.
DOM construction expands input, parser recovery can be expensive, allocations
can fail, and dependencies can contain defects and unsafe code. The parser has
not undergone an independent security audit. HTTP fetching is bounded, but HTML
and CSS processing remain in-process. Browsing is experimental; process isolation
and enforceable CPU/memory budgets remain necessary hardening work.

## Reporting

Please use GitHub's private vulnerability reporting feature if it is available
on this repository. Otherwise, contact the repository owner to arrange private
disclosure before posting sensitive reproduction details publicly.
