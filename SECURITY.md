# Security boundary in 1.11.0

Olive's HTML parser accepts local or stdin UTF-8 HTML and produces an inert DOM.
The optional GUI renders text, bounded PNG/JPEG/WebP images, and linked/embedded/inline CSS.
Local documents run
inline and external classic JavaScript automatically. Web documents start with
JavaScript disabled: no script sources are fetched and no handlers run until the
user chooses **Enable JavaScript** for that page or opts into **Enable JavaScript
by default on page loads** in Settings. The saved setting starts off, including
for existing profiles, and applies to new pages, reloads, redirects, form responses and
normal session restores. Crash-recovery restores keep web scripting disabled.
Same-page navigation preserves the current page's choice. Without the saved opt-in,
reload also preserves it, while other document navigation and changed redirect
destinations reset it. **Disable JavaScript** reloads without fetching or running
scripts; the saved opt-in applies again on the next ordinary load or reload.
Enabling the `js` feature alone never
executes HTML. The `olive` DOM inspector remains inert; `olive-js` explicitly runs
standalone JavaScript. The optional `net` feature explicitly loads HTTP(S)
documents, and is included in the GUI. Parsing itself never performs networking.
The viewer explicitly fetches supported CSS/JS/image resources; Olive does not enforce CSP.
**The parser is not an HTML sanitizer. GUI tab workers have partial OS
containment on macOS, Linux, and Windows. Standalone JavaScript execution is not
OS-sandboxed.**

The GUI and CLI URL loader accept at most 8 MiB per document. The parser-only
library default and CLI file/stdin paths remain at 1 MiB. Library callers can configure this limit.
Recovery diagnostics are capped at 64 by default and 512 bytes per message.
Tree output escapes terminal control characters and limits indentation.
Olive's flat arena avoids ownership cycles and recursive DOM drop. Unsafe code is
denied projectwide, with narrow Linux and Windows exceptions for Landlock and
Win32 worker setup, and one for the OpenGL calls that draw the browser frame's
glass. Those copy the browser's own rendered window into a texture and draw it
blurred; they run in the browser process and read no tab data.

The GUI adds a 200,000-character / 5,000-text-block / 2,048-image / 10,000-box
display limit. Table cells are boxes, so once 70% of the text-block or box limit
is used, further cells render inline, one line per row, instead of exhausting it.
Flex, float and table content measurement share one 250,000-step budget per
rendered frame, tables lay out at most 64 columns, and each container tracks at
most 64 active floats. The browser fetches each document and a dedicated child
process parses, styles and executes it.
On macOS, the GUI launches that process through `/usr/bin/sandbox-exec` with a
Seatbelt profile that denies everything by default, on top of Apple's
`system.sb` baseline for system processes. It allows starting only the
browser's own executable, signals only to itself, file reads, socket creation,
outbound IP connections and the DNS daemon's socket, and the keychain service
that supplies root certificates. Writes, other programs and processes, signals
to other processes, local Unix sockets (such as SSH agents or Docker) and
system services beyond the baseline's, including Launch Services, Apple Events
and the pasteboard, are denied. `sandbox-exec` is deprecated by Apple, so this
is a containment layer rather than an App Sandbox integration.

Linux workers apply a Landlock ruleset and a seccomp filter before starting any
threads. Landlock denies filesystem writes and requires ABI v3 so truncate
operations are covered; on kernels that support them it also denies TCP
listening (ABI v4), device ioctls (v5), and abstract Unix sockets and signals
reaching outside the sandbox (v6). The seccomp filter, for x86_64 and aarch64,
denies starting programs and processes (only threads may be created), debugging
or reading other processes, signals to other processes, namespaces and mounts,
Unix sockets, terminal input injection (`TIOCSTI` and `TIOCLINUX`), io_uring,
BPF, perf events, userfaultfd, keyrings and kernel modules. Other architectures
fail closed.

Windows launches a second copy of the browser with a low-integrity LUA token in
a job that allows one process, limits its committed memory to 2 GiB, stops it on
an unhandled exception or when the tab closes, and denies the clipboard, the
desktop, global atoms, display and system settings, and other processes' user
handles. The worker inherits only its three pipe handles, cannot create child
processes, and starts with strict handle checks and without legacy extension
points such as AppInit DLLs, non-system fonts, or images from remote shares or
with a low integrity label. Windows Mandatory Integrity Control blocks writes to
medium- and higher-integrity objects; low-integrity targets can remain writable.

All three policies preserve local file reads and outbound network connections,
which tab processes still need for page resources. Worker startup fails closed
on every platform if its restriction cannot be established. Sandbox tests, built
with the `sandbox-probe` feature, ask real tab processes started like the
browser's to write files and start programs, and on Unix to connect to a local
socket and signal their parent, and expect each attempt to fail.
The previous page and its process remain available if a new document cannot be read.
Up to 32 tabs are open at once; a tab can temporarily own an old document process
and a pending replacement. Empty tabs do not start a process.
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
redirect limit. Document responses are bounded to 8 MiB after decompression and
again after decoding; document types other than HTML/XHTML/plain text are rejected. Plain text
is escaped before display. URL inputs are capped at 8 KiB and the session's
Back/Forward stack at 256 entries.
One navigation load runs per tab. Starting another navigation cancels that tab's
pending load. Other tabs continue loading and responding independently.
Cookies are shared in memory between tabs, with at most 256 cookies and 64 KiB of
stored data, 4 KiB per cookie, and 256 pending mutations per worker event. The
browser keeps the jar; each tab process receives a copy of only its document's
site cookies (the registrable domain, or the IP address), without those marked
Secure on HTTP pages. Local documents receive none. Cross-site resources neither
send nor store cookies. Domain/path,
Secure, HttpOnly, SameSite, Max-Age and Expires are enforced. Public-suffix domains
and invalid secure prefixes are rejected; scripts cannot read or overwrite HttpOnly
cookies, and insecure responses cannot overwrite overlapping secure cookies.
Cookies are not persisted to disk. HTTP authentication,
automatic Referer headers, persistent network cache and automatic document
navigation are not enabled. User-initiated downloads are capped at 100 MiB and
published from a temporary file without replacing existing files or symlinks,
including when concurrent downloads use the same filename. Before publishing, a
download from a website receives the system's internet mark: the
`com.apple.quarantine` attribute on macOS, so Gatekeeper checks it, and a
`Zone.Identifier` stream with zone 3 (Internet) and the address on Windows.
Marking is best effort; filesystems that cannot hold the mark save the file
without it, and Linux has no such mark.
The client honors HTTP(S) proxy environment variables. Loopback and private-network
addresses are allowed, including for subresources; this is not an SSRF-filtering API.

Native forms submit only after user activation. The browser sends the
submission and a fresh document worker renders the response.
Only UTF-8 URL-encoded HTTP(S) GET/POST is supported, with a 64 KiB encoded-body
limit and the existing 8 KiB URL limit. HTTPS form targets and redirects must stay
HTTPS. The loader bounds POST responses exactly like GET responses. Redirects
301/302/303 change POST to GET; 307/308 retain its body. Cookie selection accounts
for the initiating site, method and redirects. No HTTP authentication or Referer
is added. POST data lives only in browser memory for the request and is not
saved to history. Reload and history traversal fetch URLs with GET, without replaying
POST. A stopped or failed request might already have reached the server.
Native password widgets mask their display; entered values are not encrypted in memory.
Controls are capped at 512 and select options at 2,048 per page, with 16,384 characters
per text field. Oversized initial form state disables submission. Unsupported file or
specialized input controls and unsupported encodings/targets produce a submission error.
No form event or value API exposes native edits to page scripts. Find searches only
bounded rendered text, with a 256-character query; hidden text and input values are excluded.

The GUI saves the latest 1,000 distinct visited URLs (including query strings,
fragments and local file paths), titles, timestamps and visit counts in a local,
unencrypted history file. Page scripts cannot access this history. Titles are
bounded to 1 KiB and file reads to 32 MiB; restored URLs pass the normal scheme,
credential and local-file validation. Saves atomically replace the file and use
owner-only file permissions on Unix. Failed writes are reported and preserve
the previous saved file; unreadable or unsupported files are not overwritten
until the user explicitly clears history. Remove/Clear affect saved history,
not the current document or in-memory Back/Forward stack. There is no private
browsing mode. Storage locations and the `OLIVE_HISTORY_FILE` override are
documented in README.md; multiple instances sharing a file use last-writer wins.

Site icons are fetched by the tab process with the page's resource rules and
cookies: at most two requests (the declared raster icon, then `/favicon.ico` for
websites), each capped at 1 MiB and the icon deadline of 1.5 seconds after the page
starts loading its resources. Icons decode in the tab process with the image limits
below and are scaled to at most 32×32 pixels before reaching the browser. The
browser keeps one icon per host as a PNG file of at most 64 KiB in the profile's
`favicons` folder, which reveals visited hosts like the history file does. Icons
of hosts no longer in history or bookmarks are deleted at startup, and clearing
history deletes them all.

Page resources resolve against the final document URL and first base href, with
at most 256 attempts and a shared 30-second network deadline after document loading.
Each resource request is capped at two seconds and the remaining shared time, with
at most 16 requests in flight. Elements that use the same address for the same
kind of resource share one request; each still counts toward the attempt limit
and the page budgets.
Stylesheets precede images and optional scripts, with document order preserved
within each kind. Resource discovery uses a linear ancestry pass and bounded queues.
Each resource is capped at 8 MiB after decompression, with at most 8 MiB of
external CSS, 32 MiB of external JavaScript, and 32 MiB of encoded PNG/JPEG/WebP/GIF/ICO/BMP images
retained per page. Image decoding additionally caps each dimension at 4,096,
decoder allocation at 32 MiB, and the page at 8,388,608 decoded pixels.
A response that does not fit the remaining page budget is discarded; its error
reports the page budget separately from the individual resource ceiling.
Rejected responses also have bounded reads; count and time limits bound repeated
failures. Raising the byte ceilings does not relax VM or DOM execution limits. HTTP errors and missing or
incorrect CSS/JavaScript MIME types are rejected. HTTPS pages cannot load HTTP
resources, and web pages cannot load files, including via redirects or base href.
Local documents can load local and remote sources. Cross-origin classic scripts
and stylesheets are allowed. Cookies follow domain/path, Secure and SameSite rules;
cross-site subresources send and store no cookies.
No HTTP authentication or Referer is sent. Nonempty integrity
attributes are rejected because integrity verification is not implemented. HTML
encoding prescans examine only the first 1,024 bytes for `<meta charset>` declarations.
Failures leave other resources and the document available; up to 256 resource
errors plus one budget notice are retained, at most 1,024 bytes per message.
Resource collection is a single initial snapshot; CSS imports and dynamically
inserted or changed resource URLs never fetch. Unsupported modules, alternate,
disabled and non-screen stylesheets are skipped. A resource may be fetched before
an earlier script removes its element; removed scripts do not execute. Unsupported
image formats, CSS background images, and image data URLs remain inert.

CSS is data only: imports, URL backgrounds and page fonts are ignored. Only
static screen/all media groups are supported; viewport/feature queries and other
at-rules remain inert. Media and functional-selector nesting are capped at eight.
A combined 8 MiB linked/embedded/inline CSS input budget, 32,768-rule limit,
512-declaration limit per block, 128 selectors per rule, selector size limits and a per-document
8,000,000-step matching budget bound stored styles and matching work. Exceeded
limits show a notice; unsupported syntax is skipped. DOM conversion and nested
box layout are iterative. Font sizes, geometry and line heights are clamped as
documented in README.md. CSS can change the document area but not browser
controls or the file picker.

GUI JavaScript uses Boa 0.22.0 inside the document child process, with one fresh
realm per document retained for initial scripts and subsequent clicks. The
standalone JavaScript CLI continues to execute in its own process. Initial scripts execute
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
an explicit experimental opt-in, not safe execution of untrusted code. The worker
policies do not restrict reads or outbound network access. Tab processes
cannot create processes on any platform. Each tab process counts its heap and limits it to 1 GiB: past the
limit an allocation fails, which the JavaScript engine reports as a RangeError
for buffers and which otherwise stops the process. The limit does not cover
memory that system libraries map themselves. Stronger OS sandboxing remains
required before automatic execution of untrusted remote JavaScript. Tab
processes provide fault isolation and partial containment, not a complete
security boundary against compromised native code.

The browser supervises child processes with 60-second load and 10-second click
wall-clock deadlines, checked while polling all tabs every 100 ms. Stop, closing
a tab, replacing a load and browser shutdown terminate the affected workers;
process reaping happens outside the UI thread. A worker watches the private
stdin pipe on a separate thread and exits on parent disconnect even if its main
thread is stuck. The same executable enters worker mode before creating any
window or reading browser history.

IPC uses inherited stdin/stdout pipes, no network listener or shared temporary
files. Messages have a four-byte length prefix: commands are limited to 64 MiB,
enough for an 8 MiB document as escaped JSON text, and presentation responses
to 128 MiB. A message is JSON followed by the raw RGBA
pixels of the pictures it refers to, each picture once per message. The browser
checks each picture's dimensions, pixel budget and length and runs no image
decoder on tab data. Pipe reads, writes, JSON decoding, image validation and
presentation-index checks happen off the UI thread. Focus exports carry
placeholders for their pictures; the browser encodes the validated pixels as PNG
only when an export is saved. Only inert
text/style/image presentation data and bounded reports cross into the browser;
DOMs and JavaScript realms stay in their document process. The UI retains shared
text layout, GPU uploads and window composition, so faults in that common UI
code can still affect the application. The protocol assumes the bundled worker
executable; it is not a hardened interface to compromised native code.

A tab process that runs page content is treated as untrusted. The browser does
not take its word for security-relevant state: the page address, HTTP status
and scripting state come from the browser's own fetch; a web page's reported
base address that points at a local file is replaced with the page address, so
links from it still cannot open files; cookie changes are applied only for the
document's own site, with receipt times limited to the process's lifetime; and
click commands carry only the site's cookies. A compromised tab process still
holds its own site's cookies, including HttpOnly ones, and can set HttpOnly
cookies for that site. Tab processes start in the executable's directory with
an empty environment apart from proxy, certificate, `HOME`, `PATH` and Windows
system-directory variables, so tokens and other secrets in the browser's
environment do not reach them. Stopping a tab or replacing its navigation stops
the browser's fetch before its next request or body read; a request already
waiting for its response may continue until the response starts or times out.

Beyond the tab heap limit, these controls do not provide a hard memory ceiling
for the browser process or a per-process CPU quota.
DOM construction expands input, parser recovery can be expensive, allocations
can fail, and dependencies can contain defects and unsafe code. OS-wide memory
exhaustion can still affect the browser and other applications. The parser has
not undergone an independent security audit. Browsing remains experimental;
stronger OS sandboxing and enforceable CPU budgets remain necessary hardening
work.

## Reporting

Please use GitHub's private vulnerability reporting feature if it is available
on this repository. Otherwise, contact the repository owner to arrange private
disclosure before posting sensitive reproduction details publicly.
