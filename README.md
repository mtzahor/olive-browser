# Olive Browser 🫒

Olive is a small browser and HTML parser written in Rust. Version **0.6.0** opens
HTTP/HTTPS websites and local HTML files. It parses HTML
into an owned DOM, renders bounded PNG and JPEG images, applies a bounded CSS subset, and
can run a bounded JavaScript subset, including external classic scripts. Linked
CSS and page images load automatically.
Web JavaScript starts disabled; click **Enable JavaScript** in the status bar to
reload the current page with scripting. Use this only for pages you trust: the
runtime runs inside Olive's process. **Disable JavaScript** reloads without scripts.
Reload and same-page anchors preserve the choice; new addresses, history traversal
to another document, and redirects to a different address start with web scripting
disabled. Local documents continue to run scripts automatically.

## Run the viewer

The GUI requires Rust 1.95 or newer:

```sh
cargo run --locked --features gui --bin olive-gui
cargo run --locked --features gui --bin olive-gui -- examples/scripted.html
cargo run --locked --features gui --bin olive-gui -- https://example.com
```

Enter an address and press Enter or **Go**. Bare hostnames use HTTPS;
`localhost`, loopback IP addresses and their optional ports use HTTP. Explicit
`http://`, `https://`, and local `file://` URLs are supported, along with file paths.
Open a local `.html` or `.htm` file with **Open…**, `Cmd+O`/`Ctrl+O`, or drag and drop.

Browser buttons use scalable outline icons, with tooltips and accessible names
for icon-only actions. **Go** is the olive-colored arrow beside the address field.
In narrow windows, **Open…** and **History** use folder and history icons, and
status controls wrap to keep every action reachable.

Click **Focus** or press `Cmd+Shift+F`/`Ctrl+Shift+F` for a distraction-free reading
view. Focus selects article/main content when available, falls back to plain page
text, and removes navigation, sidebars, forms, hidden content and common promotional
blocks. The browser toolbar and status bar give way to a quiet reading column.
Headings, lists, emphasis, code, loaded article images, text selection and links remain.
The button is disabled when no readable text is available.

While Focus is on, use **Appearance** to show or hide the reading settings panel.
Choose a **14–32 px font size** and **Light** or **Dark** appearance. Changes apply
immediately without reloading or running page scripts. Preferences last for the
current app session. Click **Exit Focus**, press **Escape**, or use the Focus shortcut
again to return to the original page and its scroll position. `Cmd+L`/`Ctrl+L` also
leaves Focus and selects the address bar. Following an anchor keeps Focus on;
successfully opening or reloading a document returns to the normal view. A failed
load keeps the current reading view. Reader links navigate without invoking page
click handlers. Entering and leaving Focus does not add history visits.

Content selection uses local heuristics, so unusual page layouts may omit useful
content or keep some surrounding text. Exit Focus to see the complete original page.
Try it with `cargo run --locked --features gui --bin olive-gui -- examples/focus.html`.

Click links to navigate, including relative links, `<base href>` addresses, and
same-page `#id` / named anchors. Back and Forward retain up to 256 addresses;
Reload fetches the current page again. Use `Cmd+L`/`Ctrl+L` for the address bar,
`Alt+Left`/`Alt+Right` for history, and `Cmd+R`/`Ctrl+R` or F5 to reload.
History traversal reloads documents; it does not cache page state or scroll offsets.

Open **History** or press `Cmd+Shift+H`/`Ctrl+Shift+H` to search saved page titles
and addresses, reopen a page, remove an address, or clear all saved history.
History keeps the latest 1,000 distinct URLs across launches, newest first, with
page titles, last-visited times (shown in UTC), and visit counts. Reloads and
Back/Forward visits update the existing entry; fragments have separate entries.
Only successfully opened documents are recorded, including readable HTTP error
pages and local files. Redirects record the final address, and failed loads do
not add visits. Script title changes update the entry without adding a visit.
Clearing saved history asks for confirmation and leaves the current page and
the session's Back/Forward stack available. Opening a page again records a new visit.

History is stored as a local, unencrypted `history.json` file:

- macOS: `~/Library/Application Support/Olive Browser/history.json`
- Windows: `%LOCALAPPDATA%\Olive Browser\history.json`
- Linux: `$XDG_DATA_HOME/olive-browser/history.json`, falling back to
  `~/.local/share/olive-browser/history.json`

Set `OLIVE_HISTORY_FILE` to use another file, for example an isolated test profile.
History saves after each change using atomic file replacement. Storage errors
appear as an amber badge on the **History** button and in the history window;
browsing continues with history in memory. An unreadable or unsupported file is
preserved until you explicitly clear history. History is intended for one running Olive instance
per file; simultaneous instances can overwrite each other's changes. History
does not restore tabs, page state, or the Back/Forward stack at startup.

Documents load on one background worker. Redirects update the displayed URL;
failed loads retain the previous page and history. HTTP error pages such as 404
remain readable and show their status. The status bar distinguishes HTTPS,
unencrypted HTTP, and local files. HTTPS verifies certificates using Rustls and
WebPKI roots plus the operating system's trusted certificates. Requests time out
after 20 seconds and follow at most 10 redirects.
Gzip/deflate and declared HTTP character encodings are supported. A BOM takes
precedence over the HTTP charset; otherwise UTF-8 is used. HTML meta charset
sniffing is not yet implemented. Responses are capped at 1 MiB after decompression
and again after decoding to UTF-8. HTML and plain-text responses are supported.

The viewer renders text, basic boxes, PNG and JPEG images, and linked/embedded/inline styles.
GIF, WebP, SVG and CSS background images, forms, downloads, cookies,
authentication and tabs are not implemented. Sites that require a full DOM, CSS
layout engine, or browser
JavaScript APIs will have limited presentation or functionality.

Linked `<link rel="stylesheet" href="…">`, classic `<script src="…">`, and
`<img src="…">` sources resolve against the final document URL and first
`<base href>`. Local relative CSS/JS/image files work too. Sources load once
before the script pass; changed URLs and dynamically inserted resources do not
fetch. Failed resources leave the document readable and appear under
**Resource errors** in the status bar. Disabled web JavaScript does not fetch
external scripts, but images still load.

Resource loading shares a 20-second deadline and 64-attempt limit per document,
with at most 8 MiB of retained external CSS, 32 MiB of external JavaScript, and
32 MiB of encoded page images. Each response is capped at 8 MiB after
decompression; text is capped again after character decoding. Resource
size errors and exhausted page budgets are reported separately. CSS processing
shares an 8 MiB input budget and retains at most 16,384 supported rules. HTTP errors, missing
or incorrect MIME types, HTTPS-to-HTTP resource loads, and web-to-file loads are
rejected. Stylesheets require `text/css`; scripts require a JavaScript MIME type.
Redirects, compression, BOMs and HTTP charsets use the document loader's rules.
Alternate, disabled and non-screen stylesheets are skipped. CSS imports, modules,
fonts, resource integrity verification and CSP are not implemented; resources
with a nonempty `integrity` attribute are skipped with a diagnostic.

Try the linked-resource demo locally or over HTTP:

```sh
cargo run --locked --features gui --bin olive-gui -- examples/remote.html
python3 -m http.server 8000 --bind 127.0.0.1
# In another terminal:
cargo run --locked --features gui --bin olive-gui -- http://localhost:8000/examples/remote.html
```

On the HTTP demo, CSS appears immediately; **Enable JavaScript** activates the
counter. Repeated clicks use the same page-load variables and functions.

To inspect downloads without executing page scripts:

```sh
cargo run --locked --all-features --example resource-report -- https://www.ynet.co.il/
```

Build a release binary with:

```sh
cargo build --release --locked --features gui --bin olive-gui
```

On macOS, `./scripts/build-macos-app.sh` creates a local `Olive Browser.app`.

## Parser and command line tools

The parser-only build supports Rust 1.85 and includes the `olive` DOM inspector:

```sh
cargo run --locked --bin olive -- examples/hello.html
printf '<!doctype html><p>Hello!' | cargo run --locked --bin olive -- -
```

`olive` reads stdin when no file is supplied, writes the tree to stdout, and
writes recovery diagnostics to stderr. The output is a diagnostic tree, not
serialized HTML.

Enable the optional `net` feature to inspect a URL from the command line:

```sh
cargo run --locked --features net --bin olive -- https://example.com
```

The default parser-only build remains offline and lightweight. Networking is
included automatically in GUI builds and does not change the inert parser API.

The optional JavaScript CLI requires Rust 1.91:

```sh
cargo run --locked --features js --bin olive-js -- examples/hello.js
cargo run --locked --features js --bin olive-js -- --check examples/hello.js
```

`--check` validates syntax without executing it. Console output goes to stderr;
the result preview goes to stdout.

## Supported rendering

The GUI renders headings, paragraphs, lists, emphasis, code and preformatted
text, block quotes, rules, PNG and JPEG images with alt-text fallbacks, and selectable text.
Images use their intrinsic dimensions unless numeric HTML `width`/`height` or
the supported CSS width/height properties specify another size; oversized images
scale down to fit the content column.
The GUI bundles Inter with Noto Sans Hebrew fallback for page text, code blocks,
and browser controls, so Hebrew letters and vowel marks do not become missing-glyph
rectangles. Full bidirectional paragraph layout and website font downloads are
not yet implemented.
CSS comes from stylesheet links, `<style>` elements and inline `style` attributes,
with linked and embedded rules merged in document order. The supported
subset includes:

- type, class, ID, compound, descendant, child, and static `:hover` selectors;
- the author cascade, specificity, inheritance, inline precedence, and CSS-wide keywords;
- text styles, whitespace modes, colors, borders, rounded corners, spacing,
  widths, and `display: block`, `inline`, `inline-block`, or `none`.

Unsupported CSS is skipped and reported in the status bar. See the [styled
example](examples/styled.html) for a working sample.

JavaScript uses [Boa](https://docs.rs/boa_engine/0.22.0/boa_engine/) and runs
inline and preloaded external classic scripts after the document is parsed, in
document order in one persistent realm. `async` and `defer` attributes do not
change this synchronous post-parse ordering. The host provides a small
DOM API: `document`, `window`, `getElementById`, `getElementsByTagName`,
`createElement`, text and attribute access, node insertion/removal (including
`insertBefore`), and captured `console` methods. Tag collections are bounded
snapshot arrays, not live HTMLCollections.

Document sessions provide basic `navigator` metadata and method-based
`localStorage`/`sessionStorage` (`getItem`, `setItem`, `removeItem`, `clear`, `key`,
`length`). Each store holds up to 64 KiB and 128 keys in memory for that document
only; storage does not survive reload or cross into another page. Storage writes
also consume the shared DOM write budget. Navigator reports Olive's user agent,
`en-US` language, no cookies and no touch points.

The viewer allows 32 MiB of combined script source and 256 attempted scripts;
standalone runtimes retain the 256 KiB / 64-script defaults. The source preflight
bounds active expression complexity so independent statements and quoted data do
not consume one file-wide punctuation allowance. It retains conservative checks
for regex/division and template syntax. Complex bundles can still be rejected;
this preview does not provide a full browser event loop, lifecycle listeners,
timers, layout APIs or framework-compatible DOM. Script errors identify the
external source or inline script number.
Rendered inline `onclick` handlers can call `alert()`, which appears in the
viewer as a dialog. There is no network, filesystem, timer, module, dynamic
code, navigation, or asynchronous job API.

## Library

The package exposes the `olive_html` library:

```rust
use olive_html::{parse, NodeKind};

fn main() -> Result<(), olive_html::ParseError> {
    let parsed = parse("<!doctype html><p>Hello, Olive!</p>")?;
    for id in parsed.document.descendants(parsed.document.root()) {
        if let Some(node) = parsed.document.node(id) {
            if let NodeKind::Element(element) = &node.kind {
                println!("{}", element.name.local);
            }
        }
    }
    Ok(())
}
```

Use `parse_utf8` or `parse_reader` for custom `ParseOptions`. Enable optional
engines with `--features css` or `--features js`; the GUI enables both.
`olive_html::net::{Location, DocumentLoader}` provides explicit URL resolution
and bounded HTTP(S)/file loading with `--features net`. `load` fetches only the
requested document; `load_resource` explicitly requests a typed CSS, JS, or PNG/JPEG
image resource. With `net`, `css` and `js`, `resources::PageResources::load`
collects page resources.

`run_document` executes inline scripts in a fresh realm and returns the mutated
document plus a `ScriptReport`. `DocumentSession::with_sources` accepts preloaded
external scripts without networking. `Stylesheet::from_document_with_sources`
merges preloaded links with embedded styles and computes styles for elements in
parent-before-child order. Both engines retain their combined processing budgets.

## HTML scope and security

HTML parsing uses [html5ever](https://github.com/servo/html5ever) and follows
the [WHATWG HTML parsing model](https://html.spec.whatwg.org/multipage/parsing.html),
including error recovery, namespaces, templates, character references, and
quirks modes. Parsing is inert by default; the GUI performs a separate script
pass.

Input, script, CSS, DOM, and preview work are bounded. These limits protect the
viewer from oversized or pathological documents but are not a process
sandbox. See [SECURITY.md](SECURITY.md) for the trust boundary and current
limits.

## Development

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo build --release --locked --all-features
cargo doc --locked --no-deps --all-features
```

The test suite covers HTML recovery and conformance fixtures, CSS cascade and
layout, JavaScript execution and DOM limits, CLI behavior, GUI rendering,
URL resolution, HTTP redirects/errors/timeouts/limits, resource loading and ordering,
opt-in web scripting, persistent click state, navigation history, and saved
history search, restart persistence, deletion, storage failures and size limits.

## License

Olive Browser is [Apache-2.0](LICENSE). Vendored html5lib fixtures are MIT
licensed; dependencies retain their respective licenses. The GUI embeds the
[Inter and Noto Sans Hebrew fonts](assets/fonts/README.md) under the SIL Open Font License.
