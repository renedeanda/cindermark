# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-07-22

### Added
- **CommonMark-style nested lists.** Bullet, ordered-list, and checkbox
  markers can now be indented for nesting — up to 32 tab-expanded columns
  (tab = next multiple-of-4 column). Previously such lines degraded into
  indented code blocks (editable mode) or were flattened into the previous
  item's text (grouped mode). Nesting depth is the leading-column count;
  parsing at 0–3 space indents is byte-identical to 0.1.0.
- **WebAssembly build (`wasm` feature).** A `wasm-bindgen` surface
  (`WasmParser`: `parseJson`, `keystroke`, `resetState`) that powers the live
  browser playground at <https://embernotes.app/cindermark>. Payloads use a
  hand-rolled JSON encoder, so `serde` never enters the `.wasm`.
- **Cargo feature flags with a pure-Rust default.** `cargo add cindermark`
  now pulls only three crates — `memchr`, `rustc-hash`,
  `unicode-segmentation`. The UniFFI Swift bindings are behind the opt-in
  `ffi` feature (enabled automatically by `build-apple.sh` and the release
  workflow), the `uniffi-bindgen` CLI is behind `bindgen`, and the browser
  surface is behind `wasm`.
- 35 new nested-list tests (2/4/6-space nesting, ordered-under-ordered,
  mixed marker kinds, nested checkboxes, tab and space+tab indents, 7-level
  chains, the 32-column cap, indented-code non-regression, grouped-mode
  non-flattening, and incremental/full-parse parity). The suite is now 457
  tests.

### Changed
- Documentation overhaul for the public launch: the README leads with
  crates.io and Swift Package Manager, and the git-submodule +
  `build-apple.sh` path is reframed as an advanced "build from source"
  option. Added crates.io / docs.rs badges and a Feature flags reference.
- CI now lints and tests both the default (pure-Rust) and `ffi` / `bindgen`
  configurations on every change, so neither path can regress.

### Fixed
- Incremental parse: `shift_block` / `shift_suffix_block` never adjusted
  `ListMarkerMeta` offsets, leaving stale marker UTF-16 / byte ranges on
  re-parsed and suffix blocks after an edit. Incremental parity checks now
  compare list-marker metadata for every case.
- README Rust usage example: `CindermarkParser::new` takes an
  `Option<String>`; the previous zero-argument call did not compile.

## [0.1.0] - 2026-07-01

### Added
- Initial public release of Cindermark, extracted from the Ember Notes
  editor engine: single-pass block + inline Markdown parser with UTF-16
  offsets, incremental dirty-block re-parsing, and UniFFI Swift bindings.
- `build-apple.sh` for building Apple static libraries + Swift bindings.
- Swift Package (`Package.swift`) with a binary XCFramework target.
- Configurable image-marker URI scheme (off by default; host apps opt in,
  e.g. Ember Notes passes `"ember:"`).

### Changed
- Eliminated redundant full block-list clones from the incremental parse
  FFI paths (up to 3 extra full passes per keystroke removed).
