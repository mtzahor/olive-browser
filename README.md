# Olive Browser 🫒

An open, lightweight browser project written in Rust, with security as a design
goal. **Version 0.0.2 adds a simple native GUI for opening and reading local HTML files.**

## Try it

Install [Rust](https://www.rust-lang.org/tools/install) **1.95 or newer** for the
GUI, then:

```sh
cargo run --locked --features gui --bin olive-gui
# Optionally open a document at launch:
cargo run --locked --features gui --bin olive-gui -- examples/reading.html
```

Click **Open HTML…**, press **Cmd+O** on macOS / **Ctrl+O** elsewhere, or drop a
local file into the window. The native picker accepts `.html` and `.htm` files.
Text wraps when the window resizes and long pages scroll. Canceling the picker
keeps the current page. An unreadable or oversized file shows an error and
preserves the last successfully opened page.

Save HTML source as **plain text**. Rich-text editors can export the source as
escaped text inside another HTML document; `&lt;h1&gt;` correctly displays as
literal markup, while `<h1>` creates a heading. In TextEdit, use **Format → Make
Plain Text** before saving source with an `.html` extension.

Build with `cargo build --release --locked --features gui --bin olive-gui`.
The executable is `target/release/olive-gui` (`olive-gui.exe` on Windows).
On macOS, `./scripts/build-macos-app.sh` creates `target/Olive Browser.app` for
launching from Finder. It is a local development bundle, not a signed/notarized
installer. On Linux, the GUI needs a display with OpenGL support; the file picker
uses the desktop's XDG portal (with Zenity as a fallback).

### Parser and CLI

The parser-only build still supports **Rust 1.85** and does not compile the GUI
or embed its font. The original command-line DOM inspector remains available:

```sh
cargo run --locked --bin olive -- examples/hello.html
printf '<!doctype html><p>Hello &amp; welcome!' | cargo run --locked --bin olive -- -
cargo run --locked --bin olive -- --help
```

Running `olive` with no arguments reads stdin. Tree output goes to stdout;
recovery diagnostics go to stderr. Recovered HTML exits successfully. Unreadable
or oversized input and invalid CLI arguments exit unsuccessfully. Build the CLI
with `cargo build --release --locked --bin olive`.

Example tree output:

```text
#document (NoQuirks)
  <!DOCTYPE "html" "" "">
  <html>
    <head>
    <body>
      <p>
        "Hello & welcome!"
```

The output is a diagnostic tree, not serialized HTML.

## Library

The package exposes the `olive_html` library:

```rust
use olive_html::{parse, NodeKind};

fn main() -> Result<(), olive_html::ParseError> {
    let parsed = parse("<!doctype html><p>Hello, Olive!</p>")?;
    let document = &parsed.document;
    for id in document.descendants(document.root()) {
        if let Some(node) = document.node(id) {
            if let NodeKind::Element(element) = &node.kind {
                println!("{}", element.name.local);
            }
        }
    }
    Ok(())
}
```

Use `parse_utf8(bytes, options)` or `parse_reader(reader, options)` to customize
`ParseOptions`. The defaults are a **1 MiB input limit** and **64 retained
recovery diagnostics**, each at most 512 bytes. Input size includes the UTF-8
BOM. Readers consume at most the configured limit plus one byte before size
validation. Diagnostics include approximate parser lines, not exact source spans.

`Document::children` and `Document::descendants` return `NodeId`s. IDs belong to
the document that created them. Templates own separate `DocumentFragment` nodes
through `Element::template_contents`; ordinary descendant traversal excludes
those fragments. `Document::write_tree` includes them. `node_count()` counts all
allocated nodes, including nodes detached during HTML error recovery.

## Rendering in 0.0.2

The GUI calls `olive_html::parse_reader`, walks the resulting DOM, and builds a
small text presentation. It does not delegate HTML to a webview or reparse the
source. Visible text supports headings, paragraphs, lists (including nested
items and ordered-list start/value numbering), bold/italic/underlined/struck
text, line breaks, preformatted/code text, block quotes, and horizontal rules.
Links are styled but do not navigate. Images show their alt text; tables flow
as text rows. Text can be selected and copied.

This is a basic semantic viewer, not a CSS layout engine. Stylesheets and style
attributes are ignored. Scripts, templates, hidden subtrees, embedded documents,
and foreign SVG/MathML trees are not displayed. No URLs, images, fonts from the
page, or other external resources are loaded. Form controls are not interactive.

File reading, parsing, and DOM conversion run on one background worker. The UI
keeps only the presentation of the current file, reuses text layout between
repaints, and sleeps when idle. The existing 1 MiB input limit remains in place.
Preview output is additionally limited to 200,000 text characters and 5,000
blocks, with a visible notice when truncated, to bound text-layout work.

## HTML parser standard and scope

The target is the [WHATWG HTML Living Standard](https://html.spec.whatwg.org/),
particularly [parsing HTML documents](https://html.spec.whatwg.org/multipage/parsing.html),
checked on **2026-09-06** (standard last updated 2026-09-04). HTML is a living
standard; there is no fixed “latest HTML version” number to implement.

Tokenization and tree construction use
[Servo's html5ever](https://github.com/servo/html5ever), **0.39.0**, the latest
stable crate checked for this milestone. Olive supplies its own arena DOM.
Reusing this Rust parser gives the project mature HTML error-recovery rules
with browser-style error recovery. The GUI is an optional consumer of this
parser; its dependencies are not enabled in parser-only builds. `Cargo.lock` records the exact dependency graph.

Supported in this milestone:

- Document structure, elements, namespaced attributes, text, comments, doctypes,
  and quirks modes.
- Named and numeric character references, raw text, RCDATA, and void elements.
- Implied elements/end tags, misnested formatting repair, and table foster parenting.
- Template content fragments and SVG/MathML integration points.
- UTF-8 input, optional leading BOM, newline normalization, and replacement of
  invalid UTF-8 with U+FFFD.

This milestone does **not** claim complete Living Standard conformance. It
inherits [html5ever's implementation gaps](https://github.com/servo/html5ever/issues).
In particular, newer processing-instruction tokenization is not present in
0.39.0. Olive also defers declarative shadow DOM attachment, selected-content
cloning for customizable selects, form-owner/runtime DOM behavior, fragment
parsing as a public API, and legacy character-encoding detection. A `<meta charset>` does not change the
UTF-8 input contract. Scripting is always disabled,
so `<noscript>` is parsed accordingly.

CSS layout, graphical resources, networking, JavaScript, and process isolation
belong to future milestones.

## Design

- All Olive implementation code is Rust, with `unsafe_code = "forbid"`.
- One parser dependency: `html5ever`. The optional GUI adds `eframe`/egui using
  its OpenGL backend and `rfd` for native file dialogs. A webview, WGPU backend,
  and link-opening integration are not enabled. Transitive dependencies are
  locked; the unsafe-code prohibition applies to Olive, not its dependencies.
- A `Vec<Node>` arena with integer links avoids per-node reference counting,
  ownership cycles, and recursive tree destruction. Names use html5ever's
  interned atoms; text is coalesced when parsing adjacent character tokens.
- Traversal is iterative. Tree-dump indentation is capped for deeply nested input.
- Parsing is inert: scripts, event handlers, URLs, and external resources are
  stored as data. No network requests or script execution occur.

Input and diagnostic limits reduce resource exposure; they are not strict CPU
or heap budgets. See [SECURITY.md](SECURITY.md) for the current trust boundary.

## Development

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo build --release --locked --all-features
cargo doc --locked --no-deps --all-features
# Parser-only compatibility check:
cargo +1.85.0 test --locked
```

Tests cover parser behavior, CLI input/error handling, resource limits,
8,000-level nesting on a small stack, and 200 deterministic malformed documents.
The conformance harness compares all **112 document trees** in a pinned html5lib
fixture; [fixture provenance and license](tests/fixtures/html5lib/README.md) are
included. This is a subset of conformance tests, not the full WPT suite.

GUI tests focus on DOM-to-presentation behavior, display limits, and file-load
errors. CI builds the GUI and runs tests on Linux, macOS, and Windows, checks
formatting and Clippy, and checks the parser on Rust 1.85. Native interaction
(file selection, scrolling, resizing, and cancellation) also needs a manual
smoke test on a desktop. Dependency updates should refresh the
lockfile and re-run parser and conformance tests before merging.

## License

Olive Browser is [Apache-2.0](LICENSE). Vendored html5lib test fixtures are MIT
licensed, with their license alongside the fixture. Dependencies retain their
respective licenses. The GUI embeds the [Inter font](assets/fonts/README.md)
under the SIL Open Font License.
