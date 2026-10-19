# CSS compatibility map

Olive uses a bounded CSS presentation engine. This page is the feature reference
for the current renderer; it describes the behavior users see, including its
fallbacks. It is not a claim of full browser CSS conformance.

## Sources, parsing and cascade

| Area | Implemented behavior | Gaps and fallback |
| --- | --- | --- |
| Stylesheet sources | Linked stylesheets, `<style>` elements and inline `style` attributes. Linked and embedded sources are applied in document order. | `@import`, CSS modules, constructed/adopted stylesheets and downloaded fonts are not loaded. |
| Declaration parsing | Recovery at rule and declaration boundaries; case-insensitive standard property names; `!important`. | Invalid values are skipped. Custom properties preserve their case. |
| Cascade | Author cascade, selector specificity, inheritance, inline style precedence, and `inherit`, `initial` and `unset`. | There is no user stylesheet or browser extension stylesheet. The built-in HTML defaults are described under [HTML defaults](#html-defaults-and-presentational-attributes). |
| Custom properties | Cascaded and inherited `--name` values, `var()` substitution, nested fallbacks and cycle fallback. | Substitution is limited to depth 16 and 64 KiB per computed environment; aggregate stored values are capped at 1 MiB. `env()` and registered properties are unsupported. |

## Selectors and rules

| Feature | Implemented subset |
| --- | --- |
| Selectors | Type, universal, class, ID, compound, descendant, child, adjacent/general sibling and attribute selectors. Attribute operators include exact, token, language-prefix, prefix, suffix and substring matching, with `i` and `s` flags. |
| Pseudo-classes | `:root`, `:empty`, child-position and type-position forms, `:nth-child()` / `:nth-last-child()`, `:is()`, `:where()`, `:not()`, `:link`, `:any-link`, and initial checked-state matching. |
| Pseudo-elements | `::before`, `::after`, `::selection` and other pseudo-elements, including the legacy `:before`, `:after`, `:first-line` and `:first-letter`, are valid but match nothing, because generated content is not rendered. The rest of their selector list still applies, so `*, *::before, *::after { box-sizing: border-box }` styles every element. Rules that only target pseudo-elements are skipped. Pseudo-elements prefixed for other engines (`-moz-`, `-ms-`, `-o-`) invalidate their list, as in Chromium, and pseudo-elements inside `:is()`, `:where()` and `:not()` are invalid. |
| Dynamic state | Hover, focus, active, visited, target, validity and other interaction/state selectors do not match. The page is a static presentation, not an interactive CSS state engine. |
| `@media` | Nested `screen`, `all`, `only screen`, `only all` and `not print`; comma alternatives; `min-width`, `max-width`, `min-height`, `max-height` in px/em/rem; and portrait/landscape orientation. Media-query em/rem thresholds use a fixed 16px. Stylesheet `media` attributes use the same subset; unsupported media conditions do not apply. |
| At-rules | `@media` only. Other at-rules, including `@supports`, `@container`, `@font-face`, `@keyframes` and `@page`, are ignored. |

Media conditions are evaluated against the page content viewport in CSS pixels.
Olive recomputes styles and layout when that viewport changes, including window
resizing and browser zoom. The content area is used, so the measured width can
differ from the outer window width. `vw` and `vh` lengths, including terms inside
`calc()`, are resolved against that same viewport each time styles are recomputed.

## Values and properties

The following property names are recognized. Shorthands expand to their listed
longhands. A recognized name can still reject an unsupported value.

| Group | Properties and values |
| --- | --- |
| Text | `color`, `font-size`, `font-weight`, `font-style`, `font-family`, `line-height`, `white-space`, `text-align`, `text-decoration`, `text-decoration-line`, `direction`. Font weight accepts `normal`, `bold` and numbers 1–1000; style accepts `normal`, `italic` and `oblique`; line height accepts `normal`, nonnegative numbers and lengths. White space accepts `normal`, `pre`, `pre-wrap` and `nowrap`; alignment accepts `start`, `end`, `left`, `center` and `right`; decoration accepts `none`, `underline` and `line-through`. CSS direction accepts `ltr`/`rtl`; HTML `dir` also accepts `auto`. Font family selection is reduced to the bundled proportional or monospace face; website font files are not fetched. |
| Color and paint | `background` and `background-color` as solid colors; `opacity`; `border`, `border-width`, `border-style`, `border-color`, `border-radius`; and one `box-shadow`. Colors accept hex, CSS named colors, `transparent`, `currentColor`, RGB/RGBA and HSL/HSLA forms. Border style accepts `solid`, `none` and `hidden`; widths accept lengths and `thin`/`medium`/`thick`. `border-radius` takes one to four lengths or percentages and the four `border-*-*-radius` longhands; corners are circular, so percentages use the box's shorter side and the vertical radii after `/` are accepted but ignored. Radii shrink together when adjacent corners would overlap, and images clip to them. Box shadows accept two to four lengths (offsets, then optional blur and spread) and an optional color, on boxes and images. Blur softens the edge with a linear falloff that approximates the CSS Gaussian, spread grows rounded corners, and the shadow stays outside boxes that are not opaque (clipped to the box's rectangle, so a rounded see-through box keeps square inner corners). `inset` and comma-separated multiple shadows are unsupported. |
| Display and flow | `display: none`, `inline`, `block`, `list-item`, `inline-block`, `flex`, `inline-flex`, `grid` and `inline-grid`. `inline-block`, `inline-flex` and `inline-grid` use block/container layout. HTML `table`, `tr`, `td` and `th` use table layout; the CSS `table*` display values are not parsed, so other elements cannot opt in. In right-to-left flow, boxes narrower than their container start at its right edge. |
| Tables | `border-collapse: separate/collapse` and `border-spacing` with one or two lengths. Columns are shared by every row and sized like CSS automatic table layout: each column gets at least its widest word, image or fixed-width box, and remaining room is shared by how much more its unwrapped content needs. Cell widths are preferences that content can exceed; tables without a width shrink to their columns. `colspan` and `rowspan` (1–64) place cells around covered slots, cells stretch to their row's height and spanning cells to their last row, and right-to-left tables run their columns from the right. Collapsed borders overlap neighboring cells and the table edge so shared borders draw once. Cell content is top-aligned (no `vertical-align`), `table-layout: fixed`, `caption-side`, column groups and `empty-cells` are unsupported, and only rows that are direct children of the table (through implicit `tbody`) take part. |
| Flexbox | `flex-direction: row/column`, row `flex-wrap: nowrap/wrap`, `justify-content: flex-start/start, flex-end/end, center, space-between, space-around, space-evenly`, `flex`, `flex-grow`, `flex-shrink`, `flex-basis`, `gap`, `row-gap` and `column-gap`. The shorthand accepts `none`, `auto`, a grow number with optional shrink and basis, or a basis. Row flex distributes available space, wraps rows and supports the main-axis spacing modes. Column flex uses normal block sizing with row gaps. |
| Grid | `grid-template-columns` with up to 12 equal `1fr` tracks, including `repeat(n, 1fr)`, plus `gap`, `row-gap` and `column-gap`. Grid items fill equal-width columns and rows grow to their content. Containers with direct anonymous text use normal block flow. |
| Position | `position: static/relative/absolute` and `top`, `right`, `bottom`, `left` with `auto` or lengths. Relative offsets retain the original flow slot; on inline elements they are ignored, and the text stays in its line. Absolute boxes leave normal flow and are anchored to their immediate rendered parent. |
| Box sizing and spacing | `width`, `max-width`, `height`, `min-height`, `box-sizing: content-box/border-box`, `margin`, the four margin sides, `padding` and the four padding sides. Width and max-width accept percentages; height and min-height do not. Percent margins, padding and position insets resolve against containing width (including top/bottom insets). A fixed `height` grows to fit its content unless vertical overflow is hidden, so differing fonts do not make text overlap. |
| Floats | `float: left/right` (and `inline-start`/`inline-end`) and `clear: none/left/right/both`. Floats shrink to fit their content when their width is `auto` and line up beside earlier floats, left floats from the left edge and right floats from the right in source order, moving below them when a line is full. Text, pictures and blocks that follow take the room left beside the floats, and a container grows to hold its floats, as with a clearfix. Floats only shape siblings in the same container; blocks beside floats narrow as a whole instead of wrapping around the float's bottom edge. Rows of inline blocks still lay out as a wrapping horizontal run. |
| Overflow | `overflow`, `overflow-x` and `overflow-y`. `hidden` and `clip` clip the box's contents (including absolutely positioned descendants) to its border box horizontally, and vertically when its height is fixed; the box's own background and border are unclipped. `visible`, `auto`, `scroll` and `overlay` leave contents visible, because Olive has no inner scrolling. Clipping is rectangular, even inside rounded corners. |
| Lists | `list-style` and `list-style-type` accept `none`, `disc`, `circle`, `square` and `decimal`; `none` suppresses markers. Markers hang outside the item text in the list's start padding, level with the first line. Other accepted values retain the marker supplied by the HTML list (`ol` numbering, or `ul` disc, circle and square by nesting depth), rather than changing its shape. Items displayed inline, as inline blocks, or as flex or grid containers show no marker. |
| Lengths | Unitless zero, `px`, `em`, `rem`, the absolute units `pt`, `pc`, `in`, `cm`, `mm` and `Q` (at 96 px per inch), percentages where accepted, `vw`, `vh`, `auto` where accepted, and linear `calc()` sums/subtractions of those units. Font size also accepts the standard size keywords from `xx-small` through `xx-large`, plus `smaller` and `larger`. Values are clamped to bounded layout ranges. |

## HTML defaults and presentational attributes

Before author styles, Olive applies built-in HTML defaults. They follow browser
conventions closely, with Olive's own spacing: blocks are separated by bottom
margins rather than collapsed top and bottom margins.

| Element or attribute | Default |
| --- | --- |
| `body` | 8px margin on every side. |
| `h1`–`h6` | 2, 1.6, 1.35, 1.18, 1.06 and 1 times the surrounding font size, bold, with 0.6em above and 0.4em below. |
| `p`, `pre`, `blockquote`, `hr`, `dl`, `figure`, `figcaption`, `address`, outer lists, tables outside cells | 12px below. |
| `ul`, `ol`, `menu` | 40px start padding (the right side in right-to-left text); nested lists add no margin. List items have no margin. |
| `blockquote`, `figure`, `dd` | 40px side margins; `dd` indents at the start side only. |
| `code`, `kbd`, `samp`, `tt`, `pre` | Monospace at 15/17 of the surrounding size (code inside code keeps its size) on a light background. `pre` adds padding and 6px rounded corners. |
| `sup`, `sub` | 75% size; superscripts are raised to the top of the line. The CSS `vertical-align` property is not supported. |
| `mark`, `ins`, `strike`, `var`, `address`, `center` | Highlighted, underlined, struck, italic, italic and centered respectively. |
| `hr` | A 1px neutral rule; a rule with an authored border or background paints only its box. |
| `table`, `td`, `th` | 2px spacing, cells padded 0.2em by 0.4em, bold centered `th`. In quirks-mode documents, tables do not inherit text alignment or font properties. |
| `dialog` without `open`, `datalist`, `col`, `colgroup` | Hidden. |
| `bgcolor` | Background of `body`, `table`, `tr`, `td` and `th`; also accepts hex digits without `#`. |
| `text`, `color` | Text color of `body` and `font`. |
| `width` | Width of `table`, `td` and `th`, in pixels or percent. |
| `align` | Text alignment of `div`, `p`, headings, `tr`, `td`, `th` and `caption`; `align=center` on a table, or a `center`/`align=center` parent, centers the table. |
| `cellpadding`, `cellspacing`, `border`, `colspan`, `rowspan`, `nowrap` | Cell padding, cell spacing, a gray table border with 1px cell borders, cell spans and unwrapped cell text. |

`background` accepts a solid color only; background images, gradients and multiple
background layers are unsupported. Only one box shadow is retained. Inline
backgrounds can color glyph runs, but
inline border geometry and inline box sizing are not laid out. A row flex container
with direct anonymous text uses normal column flow; flex and grid do not position
anonymous text as CSS items. `align-items`, `align-self`, flex column wrapping and
vertical flex growth are not implemented.
Grid placement, named lines, spans, auto-fit/auto-fill, `minmax()`, implicit track
sizing and masonry are not implemented. Absolute positioning does not implement
fixed/sticky positioning, stacking contexts, `z-index`, or containing-block search
past the immediate rendered parent. `bottom` placement in an auto-height parent is
approximate. Opacity multiplies descendant paint alpha; it does not composite an
entire subtree into an offscreen group.

Intrinsic sizes, used for shrink-to-fit floats, flex items, absolutely positioned
boxes and table columns, follow the box structure: flex rows, table rows and runs
of floats add up their items, nested boxes add their padding, borders and margins,
fixed widths and `max-width` apply, and absolutely positioned descendants do not
count. Generated content cannot size boxes, so the common padding-based
aspect-ratio idiom has one approximation: a `position: relative` box with no
content of its own and an automatic height takes the height of an absolutely
positioned child that holds a picture.

Other common unsupported layout and visual properties include `transform`,
`filter`, `clip-path`, `visibility`, `z-index`, `max-height`, `aspect-ratio`,
`vertical-align`, logical box properties, transitions, animations, counters and
SVG presentation. They are ignored and appear in unsupported-property diagnostics
when their declaration name is encountered.

## Processing limits and diagnostics

| Limit | Value |
| --- | ---: |
| Combined linked, embedded and inline CSS input | 8 MiB per page |
| Retained rules | 32,768 |
| Declarations in one block | 512 |
| Selectors in one rule | 128 |
| Nested media / functional selector levels | 8 |
| Selector matching work | 8 million steps per style pass |
| Custom properties in one environment | 256 |
| Custom-property environment value bytes | 64 KiB |
| Stored custom-property environment bytes across changed nodes | 1 MiB |
| `var()` expansion depth | 16 |
| Deferred declarations containing `var()` per element | 512 |
| Distinct unsupported property names retained for reporting | 512 |
| Media query alternatives | 32 |
| Flex, float and table intrinsic measurement work per rendered page pass | 250,000 bounded work units |
| Active floats shaping one container's content | 64 |
| Table columns | 64 |
| Cell `colspan` / `rowspan` | 1–64 |

Unknown property names are counted and shown in the status-bar CSS tooltip and
Developer tools. The report keeps at most 512 distinct names (up to 128 characters
each), ordered by frequency. Invalid values and unsupported syntax increment the
general ignored-declaration count instead of the property-name list. Existing
selector, DOM, image and worker limits continue to cap total presentation work.
Rows of a table whose measurement exhausts the work budget stack their cells.
Once 70% of the display's text-block or box limit is used, further table cells
render inline, one line per row, so long tables keep their text visible.

The [compatibility fixture](../examples/compatibility.html) exercises the supported
responsive layout and paint subset. The [1.1.0 compatibility audit](compatibility-1.1.0.md)
is a historical snapshot; its renderer feature list predates this map.
