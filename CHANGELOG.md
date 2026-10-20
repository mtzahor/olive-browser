# Changelog

## 1.7.0 — Grove interface

- Restyle the browser chrome as an olive grove: a shaded tab strip with the
  active tab joined to the toolbar (in Focus, to the reading toolbar), dividers
  between background tabs, and a page, globe, leaf or warning icon on each tab.
  A loading tab shows a spinner, and a thin bar slides along the toolbar.
- Round the address field into a pill led by a lock, warning or page icon for
  HTTPS, unencrypted HTTP and local pages. The toolbar no longer leaves a gap at
  its right edge, and the bookmark star fills with olive-oil gold.
- Show bookmarks as tiles with site initials and the most recently visited pages
  on the new-tab page, beneath a drawn olive sprig. Middle-click or
  Cmd/Ctrl-click opens them in a new tab.
- Give the status bar a connection chip and one outlined zoom group, and show
  keyboard shortcuts in the browser menu.
- Restyle the find bar with a search field, match arrows and a close button; group
  Settings into cards; add site initials to Bookmarks; show download sizes in KB,
  MB and GB; and give the crash-recovery bar and page errors their own cards.
- Quiet buttons now show a soft background on hover, olive-tinted shadows sit
  under windows and menus, and Focus reading controls use the olive palette.
- Stop repeating "Olive Browser" in the window title when the page title already
  ends with it.

## 1.6.0 — Rendering polish

- Lay out HTML tables with columns shared across rows and sized from their
  content, `colspan` and `rowspan`, cell backgrounds that fill the row,
  right-to-left column order, CSS `border-collapse` and `border-spacing`, and the
  legacy `bgcolor`, `width`, `align`, `border`, `cellpadding` and `cellspacing`
  attributes. Quirks-mode tables no longer inherit alignment or fonts. Near the
  display limit, further cells fall back to one line per row.
- Soften box shadows with real blur, keep them outside boxes that are not opaque,
  and paint them under images too.
- Support per-corner and percentage `border-radius`, including the longhands, and
  clip images to rounded corners.
- Give unstyled pages browser-like defaults: an 8px page margin instead of the
  browser's extra top inset, headings sized relative to their text with more space
  above than below, tight list items with markers hanging outside the text
  (disc, circle and square by depth), padded code blocks, raised superscripts,
  softer rules, and `center`, `mark` and other common elements. Code inside
  headings now scales with them, and closed `dialog` elements are hidden.
- Keep inline elements with `position: relative` in their paragraph instead of
  splitting it, right-align narrow boxes and images in right-to-left flow, and
  accept `pt`, `pc`, `in`, `cm`, `mm` and `Q` lengths.
- Lay out CSS floats: shrink-to-fit floats line up beside each other from the
  left and right edges, following text, pictures and blocks take the room beside
  them, containers grow to hold them, and `clear` moves content below them. Runs
  of floats no longer become flex rows.
- Clip contents of `overflow: hidden` and `clip` boxes, so visually hidden skip
  links and fixed-height panels no longer spill out. Scrolling overflow stays
  visible.
- Keep selector lists that include pseudo-elements such as `*::before`, so the
  common `box-sizing: border-box` reset and similar rules apply.
- Load stylesheets and images inside `noscript` when scripting is off, as
  browsers do, so pages that insert their CSS with scripts keep their fallback.
- Size flex items, floats and table columns from the structure of their content,
  summing items laid out side by side and ignoring absolutely positioned boxes,
  and let an empty positioned frame take the height of its absolute picture.
- Measure flex and table content without disturbing cached right-to-left lines.
- Library: `ComputedStyle::border_radius` is now one length per corner, and
  `ComputedStyle` gains `vertical_align`, `column_span`, `row_span`,
  `border_collapse`, `clear` and `overflow_hidden`. `Display` adds `Table`,
  `TableRow` and `TableCell`, and `css::Clear` is new.

## 1.5.0 — Codebase cleanup

- Add a `serde` feature that derives Serde support for node IDs and the CSS,
  script, network and resource types without enabling the GUI.
- Add `Element::is_html` and `Element::is_html_tag` to the library.
- Default downloads on Windows now go to `%USERPROFILE%\Downloads` instead of a
  `downloads` folder relative to the working directory.
- Stop copying the page stylesheet on every page build, click and resize.
- Split the largest GUI, rendering and networking modules, remove duplicated
  and dead code, and stop running GUI unit tests twice.
- Document the maintenance scripts, how to regenerate the app icon, and the
  two-second per-resource request timeout.

## 1.4.2 — CSS navigation layout compatibility

- Render common inline-block and floated navigation items horizontally, with
  wrapping when the available page width is too narrow.
- Honor CSS `float` and `min-width`, and give standard pages the full viewport
  width so horizontal navigation can lay out as intended.

## 1.4.1 — Windows worker sandbox fixes

- Fix Windows GUI worker compilation and startup, and run page workers with a
  low-integrity token and inherited mandatory policy.
- Keep worker processes isolated on a private desktop and terminate descendants
  when a tab closes.

## 1.4.0 — Cross-platform worker containment and navigation fixes

- Add partial filesystem-write containment for GUI tab workers on macOS, Linux
  and Windows. Reads and network access remain available; Linux requires
  Landlock ABI v3 or later, and Windows enforcement follows file ACLs.
- Fix text-link hit testing at non-100% browser zoom levels.
- Explain the platform-specific sandbox limits in the browser and security docs.

## 1.3.6 — CSS and worker stability fixes

- Apply linked and inline stylesheets when their `media` attribute is omitted.
- Release completed page workers when neither scripts nor page resources remain.
- Resolve CI Clippy failures without changing the documented CSS support limits.

## 1.3.5 — Bounded CSS layout compatibility

- Add inherited custom properties with bounded `var()` expansion, linear `calc()`
  lengths, viewport units and width/height/orientation media queries that react to
  the page viewport and browser zoom.
- Improve common page layout with row/column flex sizing, wrapping and gaps, equal
  fractional grid columns, basic relative/absolute positioning, opacity and box
  shadows.
- Report unsupported CSS property names in the status bar and Developer tools, and
  document the supported subset, visual approximations and processing limits in
  `docs/css-compatibility.md`.

## 1.3.4 — Resource loading stability

- Respect the shared page resource deadline before scheduling additional requests.
- Keep resource reporting accurate when the page deadline expires.

## 1.3.3 — Faster page display

- Show parsed page content before external stylesheets and images finish loading,
  then apply those resources in a follow-up update.
- Keep pages responsive during resource loading and stop the resource worker cleanly.
- Load independent resources in one bounded worker pool and cap individual
  resource requests at two seconds.

## 1.3.2 — Page loading and rendering stability

- Load independent page resources concurrently while preserving stylesheet priority.
- Compress tab image messages to reduce the data exchanged with page workers.
- Fix crashes rendering right-to-left text at paragraph boundaries and isolate
  image interactions between tabs.
- Release page workers after loading pages that do not use JavaScript.

## 1.3.1 — CSS compatibility and menu layout

- Parse modern and legacy `hsl()` and `hsla()` color syntax, including hue units.
- Lay out basic flex rows and columns with gaps, and respect `list-style: none`.
- Skip empty generated text sections so list markers cannot invalidate a page.

## 1.3.0 — Developer tools

- Add a Developer tools window with page overview, captured JavaScript console
  output and errors, and resource loading diagnostics. Open it with F12 or from
  the Browser menu.

## 1.2.0 — Focus HTML export and compatibility regressions

- Add bounded, standalone HTML export from the Focus toolbar, preserving article
  structure, Unicode, resolved links and embedded loaded images while omitting
  active content and remote resources; atomically save the selected file.
- Update the public suffix list to psl 2.1.238 and verify cookie/SameSite isolation
  between shared-hosting tenants.
- Protect malformed charset metadata recovery with parser integration regressions.

## 1.1.1 — HTML parser dependency compatibility

- Upgrade html5ever to 0.40.1 and its updated atom and string-cache dependencies.
- Use explicit string conversions for HTML names and namespaces throughout the
  parser, CSS, JavaScript, network, GUI, tests and examples to support the new
  string-cache API.

## 1.1.0 — Website compatibility and CSS rendering

- Expand the ranking snapshot to the top 50 sites in Israel and worldwide
  (77 unique domains, Semrush August 2026), and add a structured live audit
  that records all failures, HTTP blocks, resources and CSS limits.
- Raise the browser document ceiling from 1 to 8 MiB after eight audited sites
  exceeded the former limit; keep the standalone parser default at 1 MiB.
- Allow 256 resource attempts within 30 seconds, cap individual requests at
  five seconds and prioritize stylesheets, images, then optional scripts.
  Keep byte, image decoding, JavaScript and worker limits bounded.
- Index CSS rules by their rightmost ID/class/attribute/tag. Support attribute and sibling
  selectors, structural and functional pseudo-classes, and nested static screen
  media groups. Inactive hover/focus/visited rules no longer style every element.
- Support border-box sizing and minimum heights for block boxes and images.
  Raise CSS ceilings to 32,768 rules, 512 declarations per block, 128 selectors
  per rule and eight million matching steps; retain the shared 8 MiB input cap.
- Add offline regressions for large documents, resource prioritization, CSS
  matching/cascade/limits and rendered box geometry. See the versioned audit
  for measured improvements and remaining website limitations.
- Replace the packaged macOS executable atomically so rebuilding an open app
  does not leave stale executable signing state that prevents workers starting.

## 1.0.0 — First official release

- Remove the release-candidate labels from the browser status bar and menu;
  both display the package version, 1.0.0.
- Fix a download data-loss bug: completed downloads now atomically claim an unused
  filename, preserving existing files and symlinks even when downloads finish
  concurrently. Exhausting the available filenames reports an error instead of
  overwriting the original file.
- Add regression coverage for colliding and concurrent downloads, dangling
  symlinks and exhausted filenames. No new browser features are included.

## 0.13.0-rc3 — JavaScript defaults and theme switching

- Add a persistent, opt-in setting to enable web JavaScript by default for new
  pages, reloads, redirects, form responses and normal session restores. Existing profiles
  keep the setting off; crash-recovery restores still disable web scripts.
- Keep per-page JavaScript controls; the saved opt-in applies again on the next
  ordinary page load or reload.
- Refresh cached text layouts in all tabs and Focus views when switching browser
  themes, keeping page text visible without reloading or losing document state.

## 0.12.0-rc2 — Browser appearance

- Unify the browser interface with coordinated olive palettes, rounded controls,
  integrated tab close buttons and a clearer active-tab indicator.
- Add Light and Dark browser themes with a toolbar toggle and appearance settings;
  persist the choice across launches while preserving older profiles.
- Theme the new-tab page, address bar, tabs, status and find bars, menus, settings,
  history, bookmarks, downloads, alerts and error messages consistently.
- Consolidate browser actions in an always-available menu and simplify the status
  bar, preserving access to settings and saved pages at the minimum window width.
- Keep document colors and native forms independent of browser appearance, and
  preserve the separate Focus reading theme without reloading pages.

## 0.11.0-rc1 — Hardening

- Add crash recovery with an explicit restore/start-fresh choice after an
  interrupted session; preserve pending and background tab addresses.
- Load restored background tabs on selection to keep startup responsive.
- Keep normal restored pages script-enabled for client-rendered sites while
  crash recovery remains safe to inspect with scripts disabled.
- Fix transformed page overflow so long pages scroll without losing their
  rendered body.
- Keep the transformed viewport clip aligned while scrolling so lower page
  content remains visible.
- Apply page zoom controls immediately when the status-bar buttons are clicked.
- Add a reproducible compatibility snapshot for the top ten sites in Israel and
  worldwide (fourteen unique domains), plus an opt-in live smoke test.
- Keep the macOS bundle versioned from Cargo metadata for release packaging.

## 0.10.0 — Daily-driver usability

- Add persistent bookmarks, settings, page zoom and session restore with bounded,
  versioned atomic profile storage.
- Add a streaming download manager with progress, cancellation, safe file names,
  redirect policy and a 100 MiB limit.
- Add toolbar windows for bookmarks, downloads and settings; restore tab addresses
  with web JavaScript disabled and preserve per-tab zoom and reading preferences.

## 0.9.0 — Web compatibility

- Retain bounded session cookies across redirects, subresources, reloads and tabs;
  expose filtered `document.cookie` and `navigator.cookieEnabled` to page scripts.
  Enforce expiry, public-suffix checks, secure prefixes, HttpOnly and SameSite.
- Decode and render WebP images alongside PNG and JPEG, retaining existing byte,
  dimension and pixel budgets with alt-text fallback on failure.
- Prescan the first 1,024 HTML bytes for `<meta charset>` and legacy
  `http-equiv=Content-Type` declarations after BOM and HTTP charset handling.
- Apply Unicode BiDi per wrapped line, including bracket mirroring and combining
  marks; preserve logical find/link mappings and support HTML `dir`, CSS `direction`
  and direction-aware `text-align: start/end`.

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
