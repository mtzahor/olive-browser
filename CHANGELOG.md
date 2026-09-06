# Changelog

## 0.0.2

- Add an optional Rust GUI with a native HTML file picker, keyboard shortcut,
  drag-and-drop, and scrollable text rendered from Olive's DOM.
- Present basic HTML structure and text formatting without running scripts or
  loading external resources; retain the existing parser and CLI.
- Load files in a background worker, reuse text layouts, and cap large previews.
- Add a sample reading page, focused rendering tests, and a macOS app-bundle script.

## 0.0.1

- Add the `olive_html` Rust document parser and `olive` command-line DOM inspector.
- Integrate html5ever 0.39.0 with an Olive-owned arena DOM and scripting disabled.
- Support bounded UTF-8 input and diagnostics, recoverable HTML errors,
  namespaced elements/attributes, and template fragments.
- Add correctness, CLI, malformed-input, deep-tree, and upstream conformance tests.
- Add cross-platform CI, a sample page, and architecture/security documentation.
