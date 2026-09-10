# Changelog

## 0.8.0 — Interaction

- Render editable native inputs, checkboxes, radio groups, text areas, selects,
  submit/reset buttons, and retain form state independently in each tab.
- Submit bounded UTF-8 URL-encoded GET/POST forms with successful-control
  filtering, form ownership, required fields, button overrides and redirect handling.
- Load submissions in fresh workers; retain edits after failures, preserve the
  web scripting opt-in boundary, and never replay POST bodies on reload/history.
- Add per-tab find-in-page with Unicode lowercase matching, match counts,
  highlighting, wrapped next/previous navigation and scrolling in normal/Focus views.
- Correct HTTP localhost addresses being mistaken for file paths.
- Add the local interaction demo and regressions for native editing, form encoding,
  HTTP methods/redirects, worker submission, find shortcuts and rendered-text search.

## 0.7.0

- Add a scrolling tab strip with new, switch and close controls, keyboard
  shortcuts, and middle/Cmd/Ctrl-click links to open a new tab (up to 32 tabs).
- Keep addresses, navigation stacks, JavaScript realms, alerts, Focus preferences,
  images and scroll positions independent while sharing saved browsing history.
- Move networking, parsing, CSS, JavaScript and image decoding into separate
  child processes; a document crash or hang does not take down other workers.
- Exchange bounded presentation messages over private pipes, with asynchronous
  I/O, validated layout indices, and no page code executing in the UI process.
- Terminate stalled loads after 60 seconds and click handlers after 10 seconds;
  provide Stop and Reload recovery and reap workers on navigation, close and exit.
- Add real-process crash, suspension, watchdog, independent-realm, image transfer
  and parent-exit tests, plus tab routing, state and narrow-window regressions.
- Preserve remote JavaScript opt-in. Process isolation is fault containment,
  not an OS security sandbox or a hard memory limit.

## 0.6.0

- Add a Focus toolbar button and Cmd/Ctrl+Shift+F shortcut for a distraction-free
  reading view, with Exit Focus and Escape to return to the original page.
- Extract article/main content with a plain-page fallback; remove navigation,
  sidebars, forms, hidden content, and common advertising, sharing and signup blocks.
- Add a collapsible Appearance panel with 14–32 px font sizing and Light/Dark
  themes. Keep these preferences for the running session and reflow instantly.
- Preserve headings, lists, emphasis, code, loaded article images, selectable text,
  links and anchors. Keep reading links inert to page click handlers and change
  appearance without fetching resources, rerunning scripts or adding history visits.
- Keep separate scroll positions for normal and Focus views. New documents return
  to normal view; same-page anchors and failed navigations retain Focus mode.
- Add a Focus demo and regression coverage for extraction, appearance, navigation,
  image retention, script isolation and bounded rendering.

## 0.5.0

- Add matching vector outline icons to browser buttons, including navigation,
  history actions, JavaScript controls, diagnostics and alert confirmation.
- Keep icon-only controls accessible by name and keyboard, with a compact
  toolbar and wrapping status controls for narrow windows.

## 0.4.0

- Add saved browsing history for the latest 1,000 distinct URLs, with page titles,
  last-visited timestamps and visit counts retained across launches.
- Add a searchable History window, available from the toolbar or
  Cmd/Ctrl+Shift+H, with reopen, remove and confirmed clear-all controls.
- Record committed navigations, reloads and Back/Forward visits, including local
  files and same-page anchors; use final redirect URLs and exclude failed loads.
- Save history with atomic replacement in the platform's application data
  directory, support `OLIVE_HISTORY_FILE` for isolated profiles, and keep browsing
  available on storage errors. Preserve unreadable history until explicit clearing.
- Bound and validate saved data, keep history files private to their owner on
  Unix, and cover persistence, searching, deletion and navigation integration.

## 0.3.0

- Add bounded PNG/JPEG image loading for `<img src>` from local files and HTTP(S),
  including relative URLs, `<base href>`, HTTPS/origin checks, and resource
  diagnostics.
- Decode images with dimension, allocation, encoded-byte, and per-page pixel
  limits; retain alt-text fallback for missing, invalid, or unsupported images.
- Render intrinsic and numeric HTML/CSS image sizes, transparent pixels,
  backgrounds, borders, rounded corners, and clickable images inside links.

## 0.2.0

- Accommodate multi-megabyte site bundles with an 8 MiB resource ceiling, separate
  8 MiB CSS / 32 MiB JavaScript page budgets, an aligned 8 MiB CSS parser budget
  and 16,384-rule cap; distinguish resource-size errors from page-budget errors.
- Add document-local, quota-bound storage methods, basic navigator metadata,
  snapshot tag queries and insertBefore; identify script sources in diagnostics.
- Allow independent statements and literal data without a whole-file punctuation
  cap, while retaining expression/depth checks and conservative fallback checks
  for ambiguous syntax. The viewer accepts up to 256 scripts / 32 MiB of source;
  execution and DOM budgets remain unchanged.
- Add a resource-report example that fetches resources without executing scripts.

- Load remote and local linked stylesheets, preserving document-order cascade
  with embedded CSS and inline styles.
- Load external classic JavaScript alongside inline scripts; add per-page
  Enable/Disable JavaScript controls for web documents, disabled by default.
- Keep one JavaScript realm on the document worker, executing page-load scripts
  once and retaining variables/functions for subsequent inline click handlers.
- Resolve resources using redirects and base URLs; validate MIME/status, reject
  mixed-content and web-to-file resource loads, and report failures without
  discarding readable pages.
- Bound resource count, load time and retained bytes; retain existing shared
  CSS/JavaScript execution budgets and skip unsupported integrity-tagged sources.
- Add a linked CSS/JS demo and transport, cascade, script-order, session and
  resource-policy regression coverage. Imports, modules and dynamic resource
  fetching remain unsupported.

## 0.1.0

- Bundle Noto Sans Hebrew as a fallback for page text and browser controls,
  fixing missing-glyph rectangles on Yahoo's Hebrew privacy page; test regular,
  bold, mixed Latin/Hebrew and monospace coverage with the actual GUI fonts.
- Add HTTP/HTTPS document loading, verified TLS, redirects, gzip/deflate,
  declared HTTP character encodings, plain text, and readable HTTP error pages.
- Add an address bar, URL/path launch arguments, clickable links, relative and
  base URL resolution, fragment scrolling, back/forward history, and reload.
- Add URL loading to the DOM inspector with the optional `net` feature, included
  in GUI builds; preserve the default offline parser and local-file support.
- Bound requests to 20 seconds, ten redirects, 8 KiB addresses, and 1 MiB
  decompressed/decoded documents; retain the previous page on load failure.
- Disable remote JavaScript, display noscript fallbacks, reject unsupported
  schemes/URL credentials, and prevent web pages from opening local files.
- Hit-test individual links and click handlers inside wrapped paragraphs.
- Add URL, transport, history, renderer and CLI regression tests and update the
  documented browsing scope. External resources and forms remain unsupported.

## 0.0.4

- Add optional JavaScript parsing and interpretation with Boa 0.22.0, a reusable
  runtime/program API, and the `olive-js` execution and syntax-checking CLI.
- Execute inline classic scripts after HTML parsing in the GUI, with a shared
  realm and Rust DOM bindings for text, attributes and element creation/movement.
- Dispatch rendered inline `onclick` handlers, capture `alert()` in a native
  dialog, and keep the document preview in sync after a click.
- Apply script changes before CSS computation, update document titles, and show
  captured console output and script diagnostics in the status menu.
- Accept the common button-preview CSS forms `inline-block`, `:hover`, and
  `box-shadow` without raising the unsupported-CSS notice.
- Bound source complexity, total VM instructions, loops, recursion, DOM work,
  mutations, node allocations and retained output; keep external resources inert.
- Preserve default inert HTML parsing and Rust 1.85 HTML/CSS builds. JavaScript
  requires Rust 1.91; the GUI continues to require Rust 1.95.
- Add JavaScript examples, language/DOM/CLI/GUI regression tests, and JavaScript CI.

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
