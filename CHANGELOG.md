# Changelog

## 1.13.0 — Fuzzing and dependency checks

- Fuzz the code that handles untrusted input: the browser's reading of tab and
  network process messages, picture decoding, HTML and CSS parsing, cookies and
  addresses. Every build runs each target over its seeds and thousands of
  mutations, and a nightly workflow fuzzes each for ten minutes with
  cargo-fuzz and AddressSanitizer.
- Read process messages into a buffer that grows with the bytes that arrive,
  so a message's length prefix alone cannot make the browser reserve 128 MiB.
- Check dependencies with `cargo deny` on every build and nightly: security
  advisories, licenses, crate sources and version wildcards.
- Update rustls to 0.23.46, rustls-webpki to 0.103.16 and webpki-roots to
  1.0.10 for nine advisories, among them TLS 1.3 handshake messages accepted
  across encryption levels, a downgrade to classical key exchange, and trust in
  distrusted intermediates and roots.
- Keep arithmetic overflow checks in release builds: an overflow stops the
  process instead of wrapping.
- Sign the macOS app with the hardened runtime, ad hoc or with the Developer ID
  in `OLIVE_SIGNING_IDENTITY`. CI checks the signature.

## 1.12.0 — A separate network process

- Make every HTTP(S) request in a sandboxed network process, a child of the
  browser started on first use and restarted if it stops: documents, form
  submissions, page resources, site icons and downloads. The browser no longer
  parses responses from websites, and tabs share one connection pool, so
  repeat visits load faster (0.4–1.0 s instead of 1.2–2.6 s on ynet, Walla,
  BBC News and Wikipedia).
- Take network and file access away from tab processes. They ask the browser
  for each resource; the browser checks the request against its own record of
  the document and has the network process fetch it, or reads a local
  document's local file itself. On macOS and Linux a tab process can now
  neither read files nor open connections, so a compromised one cannot read
  the user's files or send them anywhere.
- Keep HttpOnly cookies out of tab processes. A tab receives only the cookies
  its page's scripts may read, and every cookie change it reports counts as a
  script's. The browser keeps the jar and hands the network process only the
  cookies each request needs.
- Sandbox the network process: on macOS and Linux it reaches websites, DNS and
  root certificates but cannot read the user's files, write files, start
  programs, or reach local sockets. On Windows it runs in the same restricted
  job as tabs. Its heap is limited to 2 GiB.
- On Windows, a low-integrity token cannot deny file reads or connections, so
  tab and network processes keep both at the OS level; that needs an
  AppContainer.
- Library: `DocumentLoader::with_client` shares an `HttpClient`,
  `DocumentLoader::local` reads files without network access, and
  `DocumentLoader::remote` loads resources through a `RemoteFetch` callback.
  `CookieJar::for_script` copies the cookies scripts may read. The GUI's
  `worker::Services` holds the cookie jar and `network::Network`; `Request` no
  longer carries cookies and `Worker::spawn_at` takes the services.

## 1.11.0 — Tighter tab sandboxes

- macOS: tab processes now run under a Seatbelt profile that denies everything
  by default on top of Apple's baseline for system processes, instead of
  allowing everything but writes. They can no longer start programs or other
  processes, signal other processes, connect to local Unix sockets such as SSH
  agents or Docker, or reach Launch Services, Apple Events, the pasteboard and
  other system services. File reads, outbound connections, DNS and the keychain
  service for root certificates remain for page resources.
- Linux: a seccomp filter joins Landlock. Tab processes can create threads but
  not processes, and cannot start programs, debug or signal other processes,
  enter namespaces, mount filesystems, open Unix sockets, inject terminal
  input, or use io_uring, BPF, perf events, userfaultfd, keyrings or kernel
  modules. On kernels that support them, Landlock also denies TCP listening,
  device ioctls, and abstract Unix sockets and signals outside the sandbox.
- Windows: the tab job allows one process, caps committed memory at 2 GiB, and
  denies the clipboard, desktop, global atoms and other processes' windows. The
  worker inherits only its pipes, cannot create child processes, and starts
  with strict handle checks and without extension-point DLLs, non-system fonts
  or remote and low-integrity images.
- Sandbox tests: with the new `sandbox-probe` feature, tests ask real tab
  processes to write files, start programs, connect to local sockets and signal
  their parent, and expect each attempt to fail on every platform. A filter
  test runs the Linux seccomp program against each rule.

## 1.10.0 — A stricter tab boundary

- Fetch each document in the browser, then hand it to a fresh tab process to
  render. The browser follows redirects with the whole cookie jar, so the tab
  process never needs other sites' cookies, and the address bar, history and
  bookmarks show the address the browser fetched rather than one the tab
  process reports.
- Give each tab process only the cookies of the site it shows, without the
  site's Secure cookies on HTTP pages and with none for local files. Clicks
  refresh only those cookies.
- Apply a tab process's cookie changes only for its own site, with receipt
  times limited to the process's lifetime. A web page's reported base address
  can no longer point at local files.
- Stop sending and storing cookies for cross-site images, stylesheets, scripts
  and site icons.
- Start tab processes without the browser's environment variables, keeping only
  proxy, certificate, `HOME`, `PATH` and Windows system-directory settings, so
  tokens in the environment stay in the browser.
- Limit each tab process's heap to 1 GiB. Past it, a JavaScript buffer
  allocation fails with a RangeError and other allocations stop the tab, instead
  of one page exhausting the computer's memory. Heavy news sites with scripts
  peaked at about 190 MiB.
- Mark downloads from websites as coming from the internet: the quarantine
  attribute on macOS and the Mark of the Web on Windows, so the system checks
  them before they open.
- Stopping a tab or starting another navigation also stops the browser's fetch
  of the replaced document.
- Library: `LoadedDocument` serializes with the `serde` feature,
  `DocumentLoader::with_stop` stops loads, `CookieJar::for_document` copies one
  site's cookies and `CookieUpdate::confine` holds an update to a document's
  site. The GUI's `worker::Request` replaces the `Load` and `Submit` worker
  commands, and `Worker::spawn_command_at` is now `Worker::spawn_at`.

## 1.9.0 — Faster loading and real liquid glass

- Make the liquid glass real. Pages now fill the window and scroll beneath the
  tabs, toolbar, find bar and status bar instead of sitting on a separate card
  between them. Each glass pane shows the page behind it blurred and bent at its
  rim like a lens, and the page softens beneath the frame toward the window
  edges so the controls read over any page. The page's first screen still starts
  below the toolbar, and its scroll bar stays between the frame's bands. The blur
  is drawn with OpenGL; without OpenGL 3 or OpenGL ES 3 shaders, panes keep a
  plain tint.
- Show pages styled as soon as their stylesheets arrive, while images and
  scripts are still loading, instead of waiting for every resource.
- Load pages laid out for the tab's viewport. Pages with media queries or
  viewport units no longer reload their whole presentation after loading.
- Fetch up to 16 resources at once instead of 8, and request an address that
  several elements use only once.
- Decode a page's images in parallel. The page's pixel budget now goes to
  images in document order, so the same images show on every load.
- Send pictures from tab processes to the browser as raw pixels after each
  message, once per message, instead of as PNG text in each view. The browser
  no longer decodes PNG data from tab processes.
- Encode Focus export images only when an export is saved, rather than on
  every page update.
- Match CSS selectors faster: a filter of each element's ancestor tags, IDs and
  classes rules out most descendant selectors without walking the tree, and
  candidate rules are gathered without sorting or copying names. Building a
  page's presentation takes a third of the time on large sites.
- Lay out the visible text of long pages first and the rest over the following
  frames, keeping the browser responsive when pages arrive and windows resize.
  Updates reuse the layout of unchanged text, and images upload without a copy,
  once per address.
- Measured on the same network, complete loads at the window's viewport took
  1.6–3.0 s instead of 5.5–6.7 s on ynet.co.il, 1.6–1.9 s instead of
  7.6–8.5 s on walla.co.il, about 1.0 s instead of 2.5–2.8 s on BBC News, and
  1.7 s instead of 2.6 s on Wikipedia.
- Library: `PageResources::load_with_progress` hands over the page's
  stylesheets while other resources load. `LoadedResource` is `Clone` and
  `ResourceKind` is `Hash`.

## 1.8.0 — Glass, icons and GSMArena

- Restyle the browser frame as liquid glass. Tabs, the toolbar's navigation,
  address and action capsules, the find bar and the status bar float as
  translucent panes over a soft olive-grove backdrop, lit by a sheen and a bright
  rim. The active tab is the brightest pane, pages sit on a rounded card, menus
  and windows are frosted glass, and icon buttons are round. A glint slides along
  the address field while a tab loads, and new-tab bookmarks are glass tiles.
- Show site icons on tabs, bookmarks, recently visited pages and in History. Tab
  processes fetch the declared raster icon nearest 32 pixels, or `/favicon.ico`,
  alongside the page's resources. Icons are kept per host in the profile, so they
  appear after a restart; clearing history removes them.
- Decode GIF (first frame), ICO and BMP images, and show `<object>` fallback
  content.
- Improve compatibility with GSMArena and similar sites:
  - Support `position: fixed` (placed against the first screen and painted on
    top) and `sticky` (kept in flow), so fixed headers no longer push the page
    off screen.
  - Place absolutely positioned boxes in their nearest positioned ancestor, give
    them the room their insets leave, stretch them between opposite insets, and
    place boxes anchored to `bottom`.
  - Paint positioned boxes above in-flow content, ordered by `z-index` in
    stacking contexts.
  - Parse the `font` shorthand, `text-transform`, per-side borders
    (`border-top` and friends), `flex-flow`, `row-reverse`, and percentage
    heights against definite containers.
  - Fill fixed-height `column wrap` flex containers column by column, keep
    `nowrap` runs of inline blocks on one line, and treat flex children as blocks.
  - Leave out icon-font glyphs from the Unicode private use areas, which showed
    as empty boxes.
- Library: `ComputedStyle` gains `text_transform` and `z_index`, and its
  `border_width`, `border_color` and `border_solid` are now per side, with
  `border_widths`, `has_border` and `set_border` helpers. `Position` gains `Fixed`,
  `Sticky` and `out_of_flow`; `FlexDirection` gains `RowReverse`, `ColumnReverse`
  and `is_row`; `Length` gains `has_percent`; `css::TextTransform` is new.
  `PageResources::favicon_addresses` and `PageResources::load_favicon` find and
  fetch a page's icon.

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
