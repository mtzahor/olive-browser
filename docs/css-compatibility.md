# CSS compatibility map

Olive uses a bounded CSS presentation engine. This page is the feature reference
for the current renderer; it describes the behavior users see, including its
fallbacks. It is not a claim of full browser CSS conformance.

## Sources, parsing and cascade

| Area | Implemented behavior | Gaps and fallback |
| --- | --- | --- |
| Stylesheet sources | Linked stylesheets, `<style>` elements and inline `style` attributes. Linked and embedded sources are applied in document order. | `@import`, CSS modules, constructed/adopted stylesheets and downloaded fonts are not loaded. |
| Declaration parsing | Recovery at rule and declaration boundaries; case-insensitive standard property names; `!important`. | Invalid values are skipped. Custom properties preserve their case. |
| Cascade | Author cascade, selector specificity, inheritance, inline style precedence, and `inherit`, `initial` and `unset`. | There is no user stylesheet, browser extension stylesheet or full user-agent stylesheet. |
| Custom properties | Cascaded and inherited `--name` values, `var()` substitution, nested fallbacks and cycle fallback. | Substitution is limited to depth 16 and 64 KiB per computed environment; aggregate stored values are capped at 1 MiB. `env()` and registered properties are unsupported. |

## Selectors and rules

| Feature | Implemented subset |
| --- | --- |
| Selectors | Type, universal, class, ID, compound, descendant, child, adjacent/general sibling and attribute selectors. Attribute operators include exact, token, language-prefix, prefix, suffix and substring matching, with `i` and `s` flags. |
| Pseudo-classes | `:root`, `:empty`, child-position and type-position forms, `:nth-child()` / `:nth-last-child()`, `:is()`, `:where()`, `:not()`, `:link`, `:any-link`, and initial checked-state matching. |
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
| Color and paint | `background` and `background-color` as solid colors; `opacity`; `border`, `border-width`, `border-style`, `border-color`, `border-radius`; and one `box-shadow`. Colors accept hex, CSS named colors, `transparent`, `currentColor`, RGB/RGBA and HSL/HSLA forms. Border style accepts `solid`, `none` and `hidden`; widths accept lengths and `thin`/`medium`/`thick`. Border radius is one length shared by all corners. Box shadows accept two to four lengths (offsets, then optional blur and spread) and an optional color, then render as a translucent expanded silhouette; blur is not physically softened. `inset` and comma-separated multiple shadows are unsupported. |
| Display and flow | `display: none`, `inline`, `block`, `list-item`, `inline-block`, `flex`, `inline-flex`, `grid` and `inline-grid`. `inline-block`, `inline-flex` and `inline-grid` use block/container layout. |
| Flexbox | `flex-direction: row/column`, row `flex-wrap: nowrap/wrap`, `justify-content: flex-start/start, flex-end/end, center, space-between, space-around, space-evenly`, `flex`, `flex-grow`, `flex-shrink`, `flex-basis`, `gap`, `row-gap` and `column-gap`. The shorthand accepts `none`, `auto`, a grow number with optional shrink and basis, or a basis. Row flex distributes available space, wraps rows and supports the main-axis spacing modes. Column flex uses normal block sizing with row gaps. |
| Grid | `grid-template-columns` with up to 12 equal `1fr` tracks, including `repeat(n, 1fr)`, plus `gap`, `row-gap` and `column-gap`. Grid items fill equal-width columns and rows grow to their content. Containers with direct anonymous text use normal block flow. |
| Position | `position: static/relative/absolute` and `top`, `right`, `bottom`, `left` with `auto` or lengths. Relative offsets retain the original flow slot. Absolute boxes leave normal flow and are anchored to their immediate rendered parent. |
| Box sizing and spacing | `width`, `max-width`, `height`, `min-height`, `box-sizing: content-box/border-box`, `margin`, the four margin sides, `padding` and the four padding sides. Width and max-width accept percentages; height and min-height do not. Percent margins, padding and position insets resolve against containing width (including top/bottom insets). |
| Lists | `list-style` and `list-style-type` accept `none`, `disc`, `circle`, `square` and `decimal`; `none` suppresses markers. Other accepted values retain the marker supplied by the HTML list (`ul` bullet or `ol` numbering), rather than changing its shape. |
| Lengths | Unitless zero, `px`, `em`, `rem`, percentages where accepted, `vw`, `vh`, `auto` where accepted, and linear `calc()` sums/subtractions of those units. Font size also accepts the standard size keywords from `xx-small` through `xx-large`, plus `smaller` and `larger`. Values are clamped to bounded layout ranges. |

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

Other common unsupported layout and visual properties include `float`, `clear`,
`overflow`, `transform`, `filter`, `clip-path`, `visibility`, `z-index`, `min-width`,
`max-height`, logical box properties, transitions, animations, counters and SVG
presentation. They are ignored and appear in unsupported-property diagnostics when
their declaration name is encountered.

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
| Flex intrinsic measurement work per rendered page pass | 250,000 bounded work units |

Unknown property names are counted and shown in the status-bar CSS tooltip and
Developer tools. The report keeps at most 512 distinct names (up to 128 characters
each), ordered by frequency. Invalid values and unsupported syntax increment the
general ignored-declaration count instead of the property-name list. Existing
selector, DOM, image and worker limits continue to cap total presentation work.

The [compatibility fixture](../examples/compatibility.html) exercises the supported
responsive layout and paint subset. The [1.1.0 compatibility audit](compatibility-1.1.0.md)
is a historical snapshot; its renderer feature list predates this map.
