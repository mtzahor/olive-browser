# Olive Browser 🫒

An open, lightweight browser project written in Rust, with security as a design
goal. **Version 0.0.1 is an HTML parser library and command-line DOM inspector.**

## Try it

Install [Rust](https://www.rust-lang.org/tools/install) 1.85 or newer, then:

```sh
cargo run --locked -- examples/hello.html
printf '<!doctype html><p>Hello &amp; welcome!' | cargo run --locked -- -
cargo run --locked -- --help
```

Build a standalone executable with `cargo build --release --locked`. It is
written to `target/release/olive` (`olive.exe` on Windows). Running `olive` with
no arguments reads stdin. Tree output goes to stdout; HTML recovery diagnostics
go to stderr. Recovered HTML exits successfully. Unreadable or oversized input
and invalid CLI arguments exit unsuccessfully.

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

## HTML standard and 0.0.1 scope

The target is the [WHATWG HTML Living Standard](https://html.spec.whatwg.org/),
particularly [parsing HTML documents](https://html.spec.whatwg.org/multipage/parsing.html),
checked on **2026-09-06** (standard last updated 2026-09-04). HTML is a living
standard; there is no fixed “latest HTML version” number to implement.

Tokenization and tree construction use
[Servo's html5ever](https://github.com/servo/html5ever), **0.39.0**, the latest
stable crate checked for this milestone. Olive supplies its own arena DOM.
Reusing this Rust parser gives the project mature HTML error-recovery rules
without introducing a renderer, JavaScript runtime, UI framework, or garbage
collector. `Cargo.lock` records the exact dependency graph.

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
parsing as a public API, and legacy character-encoding detection. A `<meta charset>` does not change the UTF-8 input contract. Scripting is always disabled,
so `<noscript>` is parsed accordingly.

CSS, layout, painting, networking, JavaScript, browser UI, and process isolation
belong to future milestones.

## Design

- All Olive implementation code is Rust, with `unsafe_code = "forbid"`.
- One direct runtime dependency: `html5ever`. Transitive dependencies are locked;
  the unsafe-code prohibition applies to Olive, not its dependencies.
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
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
cargo doc --locked --no-deps
```

Tests cover parser behavior, CLI input/error handling, resource limits,
8,000-level nesting on a small stack, and 200 deterministic malformed documents.
The conformance harness compares all **112 document trees** in a pinned html5lib
fixture; [fixture provenance and license](tests/fixtures/html5lib/README.md) are
included. This is a subset of conformance tests, not the full WPT suite.

CI runs tests on Linux, macOS, and Windows, checks formatting and Clippy, and
checks the Rust 1.85 minimum version. Dependency updates should refresh the
lockfile and re-run parser and conformance tests before merging.

## License

Olive Browser is [Apache-2.0](LICENSE). Vendored html5lib test fixtures are MIT
licensed, with their license alongside the fixture. Dependencies retain their
respective licenses.
