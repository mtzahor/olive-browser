# Olive Browser 🫒

An open, lightweight browser project written in Rust, with security as a design
goal. **Version 0.0.3 adds a CSS parser and styled rendering to the native HTML viewer.**

## Try it

Install [Rust](https://www.rust-lang.org/tools/install) **1.95 or newer** for the
GUI, then:

```sh
cargo run --locked --features gui --bin olive-gui
# Optionally open a document at launch:
cargo run --locked --features gui --bin olive-gui -- examples/styled.html
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

## Rendering in 0.0.3

The GUI calls `olive_html::parse_reader`, walks the resulting DOM, and builds a
small text presentation. It does not delegate HTML to a webview or reparse the
source. Visible text supports headings, paragraphs, lists (including nested
items and ordered-list start/value numbering), bold/italic/underlined/struck
text, line breaks, preformatted/code text, block quotes, and horizontal rules.
Links are styled but do not navigate. Images show their alt text; tables flow
as text rows. Text can be selected and copied.

### CSS support

Embedded `<style>` elements and inline `style` attributes feed an optional Rust
CSS engine. Tokenization and declaration/rule recovery use
[Servo's cssparser 0.37.0](https://docs.rs/cssparser/0.37.0/cssparser/), which
implements [CSS Syntax Level 3](https://www.w3.org/TR/css-syntax-3/).
Olive implements a bounded subset of selectors, the
[author cascade and inheritance](https://www.w3.org/TR/css-cascade-5/), and block
layout. This milestone does not claim full CSS conformance.

| Area | Supported |
| --- | --- |
| Sources | HTML style elements, plain `screen`/`all` media types, inline styles |
| Selectors | Type, universal `*`, class, ID, compounds, comma groups, descendant and child `>` combinators; CSS identifier escapes |
| Cascade | HTML defaults, author specificity, source order, inline precedence, `!important`, `inherit`, `initial`, `unset` |
| Text | `color`, `font-size`, numeric/normal/bold `font-weight`, `font-style`, `font-family` fallback, `line-height`, underline/line-through `text-decoration`, `text-align` left/center/right |
| Whitespace | `normal`, `pre`, `pre-wrap`, `nowrap` (wrapping mode is chosen per text block) |
| Boxes | `display: block/inline/none`, `width`, `max-width`, minimum content `height`, margin/padding shorthands and sides, horizontal auto margins |
| Painting | `background-color` and color-only `background`, uniform solid `border`, `border-width/style/color`, single-value `border-radius` |
| Values | `px`, `em`, `rem`, percentages for widths/spacing/font sizes, unitless zero, named/hex sRGB colors, `rgb()`/`rgba()`, `transparent`, `currentColor` |

Nested block backgrounds and borders surround their children, and widths reflow
on resize. Text remains selectable. Long unwrapped lines and wide boxes scroll
horizontally. The example [styled.html](examples/styled.html) demonstrates the
supported features; [reading.html](examples/reading.html) exercises HTML defaults.
The HTML/body background also colors the document viewport.

The layout is deliberately limited: margins add without collapsing; boxes use
content-box sizing and grow to fit their content even with a declared height.
Inline elements support text formatting and text backgrounds, not inline box
padding/borders. Font names choose between the bundled proportional and
monospace fonts; arbitrary system/web fonts are not loaded. Font sizes are
clamped to 1–256 logical pixels, lengths to ±10,000, line height to 1–1,024, and
painted corner radii to 255. These are preview limits, not CSS conformance rules.

Linked stylesheets and all at-rules (including imports, media-query blocks and
font faces), pseudo/attribute/sibling selectors, CSS nesting, variables,
`calc()`, flex/grid, positioning, floats, images, gradients and animations are
not implemented. Unsupported selectors reject their whole selector group;
unsupported or malformed declarations are skipped without discarding valid
neighbors. The status bar reports skipped CSS. Scripts, templates, `hidden`
subtrees, embedded documents and foreign SVG/MathML trees stay undisplayed even
when author CSS tries to show them. Links and form controls remain inert.
No URLs or external resources are loaded.

File reading, HTML/CSS parsing, and DOM conversion run on one background worker.
The UI keeps only the presentation of the current file, reuses text layouts
between repaints, and sleeps when idle. The 1 MiB input limit remains in place.
Preview output is limited to 200,000 text characters, 5,000 text/rule blocks and
10,000 block boxes. CSS has a 256 KiB combined stylesheet/inline budget, 2,048
rules, 128 declarations per rule or style attribute, 64 selectors per group,
32 compounds per selector, and 64 simple selectors per compound. Matching uses
a shared two-million-step budget per document. A visible notice reports display
or CSS processing limits; excess content/styles may be omitted.

The library exposes `olive_html::css` with `--features css` independently of the
GUI (also tested on Rust 1.85):

```rust
# #[cfg(feature = "css")]
# {
use olive_html::css::Stylesheet;
let sheet = Stylesheet::parse("p { color: olive; padding: 1em; }");
assert_eq!(sheet.rule_count(), 1);
# }
```

Use `Stylesheet::from_document` to collect embedded styles and `compute` for
elements in parent-before-child order, sharing one `StyleBudget` per document.
The GUI enables this feature automatically. The default HTML-only build keeps
its existing dependency footprint and CLI behavior.

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

Full CSS layout, graphical resources, networking, JavaScript, and process
isolation belong to future milestones.

## Design

- All Olive implementation code is Rust, with `unsafe_code = "forbid"`.
- The default HTML parser depends on `html5ever`; the optional CSS engine adds
  `cssparser`. The optional GUI adds `eframe`/egui using
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
cargo +1.85.0 test --locked --features css
```

Tests cover parser behavior, CLI input/error handling, resource limits,
8,000-level nesting on a small stack, and 200 deterministic malformed documents.
The conformance harness compares all **112 document trees** in a pinned html5lib
fixture; [fixture provenance and license](tests/fixtures/html5lib/README.md) are
included. This is a subset of conformance tests, not the full WPT suite.

CSS tests cover selectors, cascade, inheritance, values, recovery and resource
limits. GUI tests cover styled DOM conversion, nested box geometry and paint
order, whitespace, reflow/layout reuse, display limits and file-load errors. CI builds the GUI and runs tests on Linux, macOS, and Windows, checks
formatting and Clippy, and checks the HTML/CSS parsers on Rust 1.85. Native interaction
(file selection, scrolling, resizing, and cancellation) also needs a manual
smoke test on a desktop. Dependency updates should refresh the
lockfile and re-run parser and conformance tests before merging.

## License

Olive Browser is [Apache-2.0](LICENSE). Vendored html5lib test fixtures are MIT
licensed, with their license alongside the fixture. Dependencies retain their
respective licenses. The GUI embeds the [Inter font](assets/fonts/README.md)
under the SIL Open Font License.
