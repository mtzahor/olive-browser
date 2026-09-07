# Changelog

## 0.0.3

- Add an optional Rust CSS engine using cssparser 0.37.0 for embedded and inline
  styles, selectors, author cascade, inheritance and recoverable syntax errors.
- Render CSS text styles, nested block backgrounds, spacing, widths, solid
  borders, rounded corners and hidden subtrees in the native GUI.
- Preserve text selection and cached reflow; add horizontal scrolling for wide
  content and notices for unsupported CSS and processing limits.
- Bound CSS input, rule/declaration/selector counts, matching work and box output.
- Add a styled demo, CSS and GUI geometry regression tests, and Rust 1.85 CSS CI.
- Document the supported CSS subset and the unchanged no-resource-loading boundary.

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
