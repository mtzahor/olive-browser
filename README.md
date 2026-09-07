# Olive Browser 🫒

Olive is a small local HTML viewer and parser written in Rust. It parses HTML
into an owned DOM, applies a bounded CSS subset, and can run a bounded
JavaScript subset in the native viewer. It never loads network resources.

## Run the viewer

The GUI requires Rust 1.95 or newer:

```sh
cargo run --locked --features gui --bin olive-gui
cargo run --locked --features gui --bin olive-gui -- examples/scripted.html
```

Open a local `.html` or `.htm` file with **Open HTML…**, `Cmd+O`/`Ctrl+O`, or
drag and drop. The viewer renders text, basic boxes, embedded styles, and local
JavaScript. Links, forms, images, and external resources remain inert.

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
CSS comes from `<style>` elements and inline `style` attributes. The supported
subset includes:

- type, class, ID, compound, descendant, child, and static `:hover` selectors;
- the author cascade, specificity, inheritance, inline precedence, and CSS-wide keywords;
- text styles, whitespace modes, colors, borders, rounded corners, spacing,
  widths, and `display: block`, `inline`, `inline-block`, or `none`.

Unsupported CSS is skipped and reported in the status bar. See the [styled
example](examples/styled.html) for a working sample.

JavaScript uses [Boa](https://docs.rs/boa_engine/0.22.0/boa_engine/) and runs
inline classic scripts after the document is parsed. The host provides a small
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
viewer from oversized or pathological local files but are not a process
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
layout, JavaScript execution and DOM limits, CLI behavior, and GUI rendering.

## License

Olive Browser is [Apache-2.0](LICENSE). Vendored html5lib fixtures are MIT
licensed; dependencies retain their respective licenses. The GUI embeds the
[Inter font](assets/fonts/README.md) under the SIL Open Font License.
