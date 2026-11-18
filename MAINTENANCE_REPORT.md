# Olive browser maintenance report — 2026-11

## Scope and dependency history

- Latest release tag: `v1.13.0` (`d618b41`). The maintenance branch starts from `main` at `708af88`.
- `main` did not change `Cargo.toml` since the release tag. This follow-up raises the declared Rust baseline from 1.85 to 1.95, which matches the minimum version declared by `eframe 0.37.0` and permits the fixed Linux accessibility dependency resolution described below.
- The `tempfile` package stayed at `3.27.0`; its selected `getrandom` edge changed from `0.3.4` to `0.4.3`. `getrandom 0.4.3` was already present through Boa, so this is graph resolution rather than a new package version.

| Package | Release tag | `main` before this pass | Notes |
| --- | ---: | ---: | --- |
| `psl` | 2.1.241 | 2.1.252 | Direct optional dependency; public suffix list updates, detailed below. |
| `flate2` | 1.1.10 | 1.1.11 | Direct dev dependency; upstream compression correctness fixes. |
| `libc` | 0.2.190 | 0.2.191 | Direct optional dependency for Linux/macOS platform code. |

The `psl`, `libc`, and `flate2` updates arrived in merged Dependabot PRs [#31](https://github.com/mtzahor/olive-browser/pull/31), [#33](https://github.com/mtzahor/olive-browser/pull/33), [#34](https://github.com/mtzahor/olive-browser/pull/34), and [#35](https://github.com/mtzahor/olive-browser/pull/35). The repository had no open pull requests, including no open Dependabot or major-version PRs, when checked.

This branch adds these lockfile updates:

| Package | `main` | This branch | Reason |
| --- | ---: | ---: | --- |
| `tokio` | 1.53.1 | 1.53.3 | Fix an I/O-safety soundness issue; see Security below. |
| `rustix` | 1.1.4 | 1.1.6 | Fix unsound zero-length xattr buffer handling. |
| `quinn-proto` | 0.11.17 | 0.11.19 | Fix QUIC remote denial-of-service cases. |
| `quinn-udp` | 0.5.15 | 0.5.16 | Fix a UDP receive-loop denial-of-service case. |
| `async-executor` | 1.14.0 | 1.14.1 | Move off a yanked release. |
| `clipboard-win` | 5.4.1 | 5.4.3 | Move off a yanked release. |
| `zbus_xml` | 5.1.0 | 5.2.0 | Removes the vulnerable `quick-xml 0.38.4` parser from Linux accessibility dependencies. |
| `zbus_names` | 4.3.1 | 4.3.5 | Updated with the `zbus_xml` resolution. |
| `zvariant`, `zvariant_derive` | 5.9.2 | 5.15.1 | Updated with the `zbus_xml` resolution. |

The same resolution adds `zcheapstr 1.2.0` and `zvariant_utils 4.2.0`; the older `zvariant_utils 3.3.0` remains for another dependency path.

The lockfile also consolidates some Windows-only transitive requirements onto the already-present `windows-sys 0.52.0`; the now-unused `windows-sys 0.59.0`, `0.60.2`, and `windows-targets 0.53.5` package entries (and their 0.53.1 architecture packages) drop out. No direct dependency requirement or feature changed.

## Important dependency review

### Core direct dependencies

- **Networking and TLS — `reqwest 0.12.28`, Rustls `0.23.46`:** unchanged since `v1.13.0`. Olive enables blocking HTTP with Rustls, native roots, gzip, and deflate. No direct upgrade or API migration was needed. The QUIC crates below are in the lockfile through Reqwest's optional HTTP/3 dependency, but `cargo tree --all-features -i quinn` reports no active Quinn dependency for this project's feature set; Olive does not enable HTTP/3.
- **HTML/CSS parsing — `html5ever 0.40.1`, `cssparser 0.38.0`:** unchanged. No deprecation, security fix, or replacement API came from a version change in this range.
- **UI/rendering — `eframe`, `egui`, and `epaint 0.37.0`:** unchanged. `eframe 0.37.0` declares Rust 1.95 as its minimum; its Linux accessibility dependency path was updated to remove the vulnerable parser. [eframe 0.37.0 manifest](https://crates.io/crates/eframe/0.37.0)
- **Cookie/site boundaries — `psl 2.1.241 → 2.1.252`:** the crate has no detailed changelog for these data-only releases; the upstream commits are each public-suffix-list updates. This library feeds `src/net/cookies.rs` domain validation and site-key calculation, so changed suffix rules can change which cookies a site may set or share.

| `psl` version | Public-suffix-list change |
| ---: | --- |
| 2.1.242 | Added 65 and removed 2 rules, including AWS Bedrock AgentCore, Naver Cloud object storage, AWS transfer-webapp/Airflow rules; removed `perspecta.cloud` and `knightpoint.systems`. [Commit](https://github.com/addr-rs/psl/commit/fe246923) |
| 2.1.243 | Removed `gitapp.si`. [Commit](https://github.com/addr-rs/psl/commit/b9de8719) |
| 2.1.244 | Removed `aliases121.com` and `oya.to`. [Commit](https://github.com/addr-rs/psl/commit/8514e1a2) |
| 2.1.245 | Added `*.ztna-dingtalk.com`. [Commit](https://github.com/addr-rs/psl/commit/979e5aa1) |
| 2.1.246 | Removed `helsinki`. [Commit](https://github.com/addr-rs/psl/commit/a3ccf68f) |
| 2.1.247 | Added `pagelemon.dev`. [Commit](https://github.com/addr-rs/psl/commit/bc5d3011) |
| 2.1.248 | Added `auth.lovable.app`, `sliplane.app`, `*.apps.grok.bot`, `*.pages.grok.bot`, `linkyhost.page`, `sanity.run`, `docsflowy.site`, `manus.space`, and `sanity.studio`; adjusted existing `*.cursorusercontent.com` and `grok.me` rules. [Commit](https://github.com/addr-rs/psl/commit/abf8a020) |
| 2.1.249 | Added `ad.vn`, `ap.vn`, and `on.vn`. [Commit](https://github.com/addr-rs/psl/commit/22da4af9) |
| 2.1.250 | Added `prod.page` and `ship.place`. [Commit](https://github.com/addr-rs/psl/commit/98b547f2) |
| 2.1.251 | Added `gov.de`, `cloudzyro.app`, and `abyme.dev`. [Commit](https://github.com/addr-rs/psl/commit/19cc3df1) |
| 2.1.252 | Added `vibekit.page`. [Commit](https://github.com/addr-rs/psl/commit/9b4221e0) |

These rules are security-relevant because suffix recognition defines cookie scope and same-site boundaries. The existing tests exercise generic public-suffix and shared-hosting cases, not each newly changed domain.

### Other changed direct dependencies

- **`flate2 1.1.10 → 1.1.11` (dev dependency):** release notes fix sync-flush behavior across output-buffer boundaries; separate gzip-header reads from compressed payload; retain progress across input errors; reject incomplete write-side streams; and harden reset/finish paths. These improve compression correctness and error handling. No new API clearly replaces Olive code, and no flate2-specific deprecation warning appeared. [Upstream 1.1.11 notes](https://github.com/rust-lang/flate2-rs/releases/tag/v1.1.11)
- **`libc 0.2.190 → 0.2.191`:** upstream adds platform constants and Emscripten support, fixes FreeBSD-aarch64 register ABI definitions, restores Apple Mach API availability, and deprecates FreeBSD `NOTE_PCTRLMASK`. Olive does not use that constant; the requested deprecation lint is clean. [Upstream changelog](https://github.com/rust-lang/libc/blob/0.2.191/CHANGELOG.md)

### Security-driven transitive updates

- **Tokio 1.53.1 → 1.53.3:** 1.53.2 includes runtime, I/O, synchronization, and timer fixes. 1.53.3 adds `AsyncFd::register` APIs and deprecates the safe `AsyncFd::{new, with_interest, try_new, try_with_interest}` constructors because they did not uphold I/O safety. Olive has no Tokio/`AsyncFd` call sites. [1.53.2 notes](https://github.com/tokio-rs/tokio/releases/tag/tokio-1.53.2), [1.53.3 notes](https://github.com/tokio-rs/tokio/releases/tag/tokio-1.53.3), [RustSec RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386)
- **Rustix 1.1.4 → 1.1.6:** the 1.1.5 release tag has no detailed notes; 1.1.6 prevents xattr size-query calls with zero-length buffers from marking unwritten memory initialized and adds explicit `*_query` APIs. Olive does not call Rustix directly. No migration to these new APIs was needed. [Rustix 1.1.5 tag](https://github.com/bytecodealliance/rustix/releases/tag/v1.1.5), [RustSec RUSTSEC-2026-0486](https://rustsec.org/advisories/RUSTSEC-2026-0486)
- **Quinn Proto 0.11.17 → 0.11.19:** 0.11.18 fixes unordered-read panics, outgoing datagram accounting panics, and an ACK-frequency panic; 0.11.19 validates peer stream IDs and bounds long Retry-token handling to prevent stream-amplification and client memory-exhaustion denial of service. Quinn is not an active feature in Olive today, but the lockfile contained these vulnerable versions. [Upstream releases](https://github.com/quinn-rs/quinn/releases), [RustSec package advisories](https://rustsec.org/packages/quinn-proto.html)
- **Quinn UDP 0.5.15 → 0.5.16:** fixes a stale `MSG_TRUNC` receive-loop state that could spin the task after a truncated datagram on Apple, OpenBSD, NetBSD, Solaris, and illumos. The fix is relevant if the optional HTTP/3 path is enabled in the future. [RustSec RUSTSEC-2026-0538](https://rustsec.org/advisories/RUSTSEC-2026-0538)

## Changes made in this pass

### Security

- Updated `tokio`, `rustix`, `quinn-proto`, and `quinn-udp` in `Cargo.lock` to versions containing the listed fixes.
- Updated the two yanked lockfile packages, `async-executor` and `clipboard-win`, to current releases.
- Raised `rust-version` from 1.85 to 1.95, matching `eframe 0.37.0`'s declared MSRV. This makes the existing manifest accurate and allows Cargo to select `zbus_xml 5.2.0` through the existing `zbus-lockstep 0.5.2` requirement.
- Updated the Linux accessibility path from `zbus_xml 5.1.0` to `5.2.0`. That release uses `winnow` instead of `quick-xml`, removing vulnerable `quick-xml 0.38.4`. [zbus_xml 5.2.0](https://crates.io/crates/zbus_xml/5.2.0)
- The Linux all-features graph now includes only `quick-xml 0.41.0`, through Wayland tooling. This version is outside the affected range in [RUSTSEC-2026-0194](https://rustsec.org/advisories/RUSTSEC-2026-0194.html) and [RUSTSEC-2026-0195](https://rustsec.org/advisories/RUSTSEC-2026-0195.html).
- Installed and ran `cargo-audit`. Its scan of 512 locked dependencies reports no known vulnerabilities and no yanked packages. It still reports `zbus_names 4.3.5`, `zvariant 5.15.1`, and `zvariant_derive 5.15.1` as unmaintained, with no fixed upgrades available. These are warnings, not known security advisories.

### Deprecations

- `cargo clippy --all-targets -- -W deprecated` and its all-features version pass without warnings on Rust 1.95 after applying the mechanical Clippy suggestions.
- Tokio's newly deprecated `AsyncFd` constructors are not used by Olive. The FreeBSD `NOTE_PCTRLMASK` deprecation in libc is also unused.

### Simplifications

- Clippy's let-chain and slice-chunk suggestions simplified existing condition checks and fixed-size pixel loops without changing their behavior. No dependency API clearly replaces other Olive code.

### Major bumps

- No open Dependabot PRs or pending major-version PRs were found. No dependency major-version bump was adopted.
- The baseline is now Rust 1.95, matching the already-selected `eframe 0.37.0`; this is a minimum-toolchain change for downstream builders.

## Validation and manual follow-up

Passed on macOS using Rust 1.95.0:

- `cargo build`
- `cargo clippy --all-targets -- -W deprecated`
- `cargo clippy --all-targets --all-features -- -W deprecated`
- `cargo test`
- `cargo test --features gui` (including localhost network tests; run with the required socket permission)
- `cargo fmt --check`

`cargo audit` reports no known vulnerabilities or yanked packages. It reports only the three unmaintained `zbus*` crates listed above.

Manual review/testing before a release:

1. The branch resolves the `quick-xml` advisory and aligns the manifest with `eframe`'s 1.95 MSRV. Confirm the new minimum toolchain is acceptable for downstream builders as part of review.
2. Exercise cookie setting and same-site behavior on representative newly added or removed PSL hosting suffixes. Existing tests do not cover every updated suffix rule.
3. The full GUI suite ran on macOS, but no Linux/Windows cross-build or browser-level interoperability run was performed. In particular, tests may not reveal all differences in cookie boundaries, network decompression, or platform-specific sandbox behavior.

## Draft patch changelog entry

### 1.13.1

- Updated public-suffix data used for cookie-domain and same-site checks.
- Updated compression handling and patched transitive Tokio, Rustix, and QUIC dependencies.
- Raised the minimum supported Rust version to 1.95 and updated Linux accessibility dependencies to remove the vulnerable `quick-xml 0.38.4` parser.
- Replaced yanked transitive dependency releases.

The manifest and lockfile now declare version `1.13.1`. No tag was created and no release was published.
