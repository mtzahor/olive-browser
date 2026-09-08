# Olive Browser 🫒

Olive is a small browser and HTML parser written in Rust. Version **0.1.0** opens
HTTP/HTTPS websites and local HTML files. It parses HTML
into an owned DOM, applies a bounded CSS subset, and can run a bounded
JavaScript subset in local documents. Web pages render with JavaScript disabled.

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

Click links to navigate, including relative links, `<base href>` addresses, and
same-page `#id` / named anchors. Back and Forward retain up to 256 addresses;
Reload fetches the current page again. Use `Cmd+L`/`Ctrl+L` for the address bar,
`Alt+Left`/`Alt+Right` for history, and `Cmd+R`/`Ctrl+R` or F5 to reload.
History traversal reloads documents; it does not cache page state or scroll offsets.

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

This first browsing release renders text, basic boxes, and embedded/inline styles.
Linked stylesheets, images (except alt text), external scripts, forms, downloads,
cookies, authentication, tabs and JavaScript on web pages are not implemented.
Sites that require those features will have limited presentation or functionality.

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
text, block quotes, rules, text alternatives for images, and selectable text.
The GUI bundles Inter with Noto Sans Hebrew fallback for page text, code blocks,
and browser controls, so Hebrew letters and vowel marks do not become missing-glyph
rectangles. Full bidirectional paragraph layout and website font downloads are
not yet implemented.
CSS comes from `<style>` elements and inline `style` attributes. The supported
subset includes:

- type, class, ID, compound, descendant, child, and static `:hover` selectors;
- the author cascade, specificity, inheritance, inline precedence, and CSS-wide keywords;
- text styles, whitespace modes, colors, borders, rounded corners, spacing,
  widths, and `display: block`, `inline`, `inline-block`, or `none`.

Unsupported CSS is skipped and reported in the status bar. See the [styled
example](examples/styled.html) for a working sample.

JavaScript uses [Boa](https://docs.rs/boa_engine/0.22.0/boa_engine/) and runs
inline classic scripts in local files after the document is parsed. The host provides a small
DOM API: `document`, `window`, `getElementById`, `createElement`, text and
attribute access, node insertion/removal, and captured `console` methods.
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
and bounded HTTP(S)/file loading with `--features net`. It never fetches subresources.

`run_document` executes inline scripts in a fresh realm and returns the mutated
document plus a `ScriptReport`. `olive_html::css::Stylesheet` parses embedded
styles and computes styles for elements in parent-before-child order.

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
URL resolution, HTTP redirects/errors/timeouts/limits, and navigation history.

## License

Olive Browser is [Apache-2.0](LICENSE). Vendored html5lib fixtures are MIT
licensed; dependencies retain their respective licenses. The GUI embeds the
[Inter and Noto Sans Hebrew fonts](assets/fonts/README.md) under the SIL Open Font License.
