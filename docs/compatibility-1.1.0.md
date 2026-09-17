# Olive 1.1.0 compatibility and resource review

## Scope and method

The August 2026 Semrush all-category rankings supply 50 Israel and 50 global targets:
77 unique domains, with 23 in both lists. The snapshot was retrieved September 17, 2026
from [Israel](https://www.semrush.com/trending-websites/il/all) and
[worldwide](https://www.semrush.com/trending-websites/global/all). Public ranking-page
application data supplies the complete ordered lists; the first screen shows only 20.
See the [pinned TSV](../tests/fixtures/compatibility/sites.tsv).

Baseline audit: `2026-09-17T13:18:56.600868+00:00`. Release audit: `2026-09-17T13:37:01.839803+00:00`.
The same runner and target list were used in both runs. Each domain receives a fresh
Olive document loader in an isolated process, with no saved user cookies or profile.
Web JavaScript stays disabled. The runner allows four concurrent probes, a 65-second
wall-clock timeout per probe, and records failures without aborting subsequent sites.

This measures document fetching, resource loading and static CSS computation. It does
not verify login, video, checkout, client-side navigation, rendering fidelity or full
web application support. HTTP 200 can be a challenge or consent page. Network results
vary by time, location, provider blocking and rate limits; compare the controlled
regressions alongside the live numbers. The audit computes styles for all elements,
including DOM that the GUI may skip, so its matching limits are conservative.

## Results

| Measurement | 1.0.0 | 1.1.0 |
| --- | ---: | ---: |
| Unique domains attempted | 77 | 77 |
| Documents fetched and parsed (including non-200 responses) | 66 | 75 |
| HTTP 200 responses | 58 | 67 |
| Rejected by the former 1 MiB document cap | 8 | 0 |
| Pages reaching resource count/time/byte limits | 12 | 1 |
| Pages reaching CSS matching limits | 19 | 4 |
| Pages reaching CSS parsing/storage limits | 17 | 11 |

Eight domains rejected by the old document cap loaded in the release audit: Amazon.de,
Globo, Haaretz, Israel Hayom, Mako, Netflix, Pinterest and Super-Pharm. Response sizes
change between requests; Israel Hayom was about 6.5 MiB in the release sample.
The remaining transport failures in this run were Adobe (timeout) and gov.il (TLS handshake).
HTTP 202, 403 and 503 responses remain recorded separately below.

## Resource decisions

| Limit | Previous | 1.1.0 | Reason |
| --- | --- | --- | --- |
| Browser document bytes | 1 MiB | 8 MiB | Eight measured rejections; largest successful release response about 6.5 MiB. |
| Resource attempts | 64 | 256 | News and commerce pages regularly contain more than 64 assets. |
| Shared resource deadline | 20 s | 30 s | More assets can load while leaving room within the 60 s worker deadline. |
| Individual resource timeout | Remaining page time | At most 5 s | One slow host should not consume the full page deadline. |
| CSS retained rules | 16,384 | 32,768 | Large Facebook and YouTube stylesheets reached the old ceiling. |
| CSS declarations per block | 128 | 512 | Modern generated style blocks exceed the old limit. |
| Selectors per rule | 64 | 128 | Larger resets and generated selector groups. |
| CSS match steps per document | 2 million | 8 million | Index ID/class/attribute/tag rules first, then permit more necessary work. |

External CSS remains capped at 8 MiB per page; CSS processing shares an 8 MiB
linked/embedded/inline input budget. External JavaScript and encoded images remain
at 32 MiB each, and individual resources at 8 MiB. Decoded images remain limited to
8,388,608 pixels (32 MiB of RGBA), consistent with bounded worker messages and GUI
memory. JavaScript execution limits, the 60-second worker watchdog and default-disabled
web scripting remain unchanged. Parser-only file/stdin input stays at 1 MiB.

CSS now loads before images and optional scripts, preserving source order within
each kind. A linear ancestry scan and bounded discovery queues avoid quadratic
resource discovery on deeply nested documents. Byte and time caps still apply.

The remaining CSS matching limits in this run affect Clalit, Instagram, Sport5 and
Ynet. Further selector optimization is preferable to removing the work budget.
Unsupported CSS is still reported. Flex/grid, positioning, SVG, downloaded fonts,
CSS variables and viewport/feature-dependent media queries are still outside the
renderer subset. Modules, event loops and modern framework DOM APIs remain limited.

## Rendering and verification

- Added attribute and sibling selectors; structural and functional pseudo-classes;
  bounded nested static screen/all media groups; correct non-matching static
  hover/focus/active/visited states; border-box dimensions and minimum heights.
- 247 automated tests pass with all features, including loopback HTTP, resource
  bounds, cascade/specificity, nested parsing limits, box geometry, IPC and tab isolation.
- Formatting, Clippy with warnings denied, rustdoc with warnings denied and the
  optimized macOS ARM64 build pass. The archive is checked for integrity and version.
- The packaged app renders the bilingual [compatibility fixture](../examples/compatibility.html)
  with correct Hebrew ordering and box widths. A live Israel Hayom page opens with
  scripting disabled; its unsupported layouts still fall back to normal block flow.
- The packaged document worker renders the fixture without CSS or preview-limit
  diagnostics, returns its presentation through IPC and exits after its pipe closes.
  Packaging now atomically replaces the executable to avoid stale macOS signing
  state when rebuilding an open app.

## Per-domain results

Ranks are from the snapshot, not current live traffic. “CSS limit” identifies the
bounded stage reached, not a website pass/fail. Asset failures are retained in the
[full release JSON](compatibility-1.1.0.json); the [baseline JSON](compatibility-1.0.0.json)
contains the corresponding pre-change results.

| Domain | Israel | Global | HTTP/result | HTML KiB | Assets loaded/attempted | CSS rules | CSS limit |
| --- | ---: | ---: | --- | ---: | ---: | ---: | --- |
| adobe.com | — | 49 | timeout | — | — | — | — |
| aliexpress.com | 16 | 38 | 200 | 1.8 | 0/0 | 0 | — |
| amazon.com | 36 | 13 | 200 | 835.9 | 116/118 | 8279 | — |
| amazon.de | — | 50 | 200 | 3.7 | 1/1 | 1335 | — |
| apple.com | — | 36 | 200 | 253.1 | 60/61 | 1528 | — |
| bing.com | — | 16 | 200 | 228.7 | 0/2 | 496 | — |
| bizportal.co.il | 45 | — | 200 | 302.7 | 28/73 | 973 | — |
| booking.com | 44 | 43 | 202 | 3.9 | 0/0 | 1 | — |
| btl.gov.il | 46 | — | 200 | 160.7 | 16/24 | 4021 | — |
| c14.co.il | 18 | — | 403 | 5.2 | 0/0 | 5 | — |
| calcalist.co.il | 34 | — | 403 | 0.4 | 0/0 | 0 | — |
| canva.com | — | 34 | 200 | 10.1 | 5/5 | 14 | — |
| chatgpt.com | 7 | 4 | 200 | 481.4 | 2/2 | 1083 | — |
| clalit.co.il | 25 | — | 200 | 218.2 | 13/54 | 5480 | match |
| claude.ai | 35 | 28 | 200 | 119.0 | 3/3 | 604 | — |
| dailymotion.com | — | 45 | 200 | 53.3 | 1/1 | 1648 | — |
| discord.com | — | 31 | 200 | 166.1 | 60/173 | 391 | parse |
| duckduckgo.com | 39 | 15 | 200 | 200.9 | 0/0 | 199 | — |
| ebay.com | — | 46 | 403 | 1.8 | 0/0 | 11 | — |
| facebook.com | 3 | 3 | 200 | 453.7 | 1/2 | 24295 | parse |
| github.com | 42 | 26 | 200 | 563.0 | 45/50 | 5850 | parse |
| globes.co.il | 33 | — | 200 | 183.5 | 58/63 | 808 | — |
| globo.com | — | 40 | 200 | 2349.8 | 36/55 | 1415 | — |
| google.co.il | 43 | — | 200 | 84.3 | 3/3 | 196 | — |
| google.com | 1 | 1 | 200 | 84.2 | 3/3 | 196 | — |
| gov.il | 22 | — | transport failure | — | — | — | — |
| haaretz.co.il | 30 | — | 200 | 1509.9 | 34/34 | 612 | — |
| imdb.com | — | 35 | 202 | 2.0 | 0/0 | 1 | — |
| inn.co.il | 26 | — | 200 | 285.9 | 134/135 | 531 | — |
| instagram.com | 8 | 5 | 200 | 487.2 | 2/4 | 11813 | parse, match |
| israelhayom.co.il | 23 | — | 200 | 6673.9 | 37/55 | 3256 | — |
| israelpost.co.il | 49 | — | 200 | 14.7 | 2/2 | 14 | — |
| kan.org.il | 38 | — | 200 | 495.1 | 166/256 | 2508 | — |
| ksp.co.il | 28 | — | 403 | 18.8 | 0/1 | 14 | — |
| linkedin.com | 31 | 18 | 200 | 136.8 | 1/2 | 1538 | parse |
| live.com | — | 29 | 200 | 13.9 | 0/1 | 17 | — |
| login.gov.il | 48 | — | 200 | 1.5 | 0/0 | 0 | — |
| maariv.co.il | 13 | — | 200 | 458.2 | 85/87 | 710 | — |
| maccabi4u.co.il | 50 | — | 200 | 175.9 | 114/158 | 3462 | — |
| mako.co.il | 6 | — | 200 | 15.3 | 2/2 | 14 | — |
| microsoft.com | — | 25 | 200 | 196.5 | 2/5 | 466 | — |
| msn.com | — | 21 | 200 | 46.4 | 0/0 | 0 | — |
| n12.co.il | 14 | — | 200 | 639.7 | 160/165 | 3049 | — |
| naver.com | — | 32 | 200 | 252.9 | 2/2 | 2281 | parse |
| netflix.com | 37 | 22 | 200 | 3091.9 | 2/3 | 506 | — |
| one.co.il | 21 | — | 200 | 481.5 | 1/1 | 135 | — |
| openai.com | — | 42 | 200 | 441.4 | 51/51 | 2038 | — |
| paypal.com | — | 37 | 200 | 203.5 | 8/8 | 1244 | — |
| pinterest.com | — | 23 | 200 | 1298.0 | 0/0 | 2161 | parse |
| pornhub.com | 12 | 9 | 503 | 3.5 | 0/2 | 20 | — |
| reddit.com | 20 | 6 | 200 | 8.2 | 0/0 | 15 | — |
| roblox.com | — | 39 | 200 | 62.4 | 26/26 | 7856 | parse |
| rotter.net | 29 | — | 200 | 112.8 | 42/74 | 257 | — |
| sport5.co.il | 10 | — | 200 | 516.8 | 46/127 | 4029 | match |
| spotify.com | — | 33 | 200 | 299.8 | 85/85 | 2294 | — |
| super-pharm.co.il | 47 | — | 200 | 2079.1 | 40/41 | 1119 | — |
| telegram.org | — | 41 | 200 | 19.5 | 14/14 | 1281 | — |
| temu.com | 24 | 27 | 200 | 2.8 | 0/0 | 0 | — |
| tiktok.com | 15 | 11 | 200 | 1.4 | 0/0 | 0 | — |
| twitch.tv | — | 24 | 200 | 195.8 | 1/1 | 591 | — |
| twitter.com | — | 44 | 200 | 32.0 | 1/1 | 2874 | — |
| vk.ru | — | 47 | 200 | 107.6 | 4/4 | 2222 | — |
| walla.co.il | 5 | — | 200 | 630.4 | 4/59 | 614 | — |
| walmart.com | — | 48 | 200 | 387.9 | 18/20 | 3276 | parse |
| weather.com | 41 | 19 | 200 | 360.4 | 4/8 | 31 | — |
| whatsapp.com | 11 | 10 | 200 | 271.7 | 21/23 | 1465 | — |
| wikipedia.org | 9 | 7 | 200 | 116.8 | 6/7 | 261 | — |
| x.com | 17 | 8 | 200 | 32.0 | 1/1 | 2874 | — |
| xhamster.com | 27 | 20 | 503 | 3.5 | 0/2 | 20 | — |
| xnxx.com | 40 | — | 200 | 116.6 | 2/2 | 6467 | parse |
| xvideos.com | 32 | 12 | 200 | 196.6 | 1/50 | 9326 | parse |
| yad2.co.il | 19 | — | 200 | 15.0 | 2/4 | 18 | — |
| yahoo.co.jp | — | 17 | 200 | 32.6 | 2/2 | 804 | — |
| yahoo.com | — | 14 | 200 | 109.2 | 3/6 | 1445 | — |
| yandex.ru | — | 30 | 200 | 2.8 | 0/0 | 0 | — |
| ynet.co.il | 4 | — | 200 | 519.7 | 160/161 | 9182 | match |
| youtube.com | 2 | 2 | 200 | 864.2 | 5/5 | 17076 | — |

Reproduce with `./scripts/run-compatibility.sh`; offline checks use
`cargo test --locked --all-features`. Live audits are deliberately separate from CI.
