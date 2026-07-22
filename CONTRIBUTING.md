# Contributing to Cindermark

Thanks for your interest! Cindermark is a small, focused project: a fast
incremental Markdown parser built for native text editors. Contributions that
sharpen that focus are very welcome.

## Getting started

```bash
git clone https://github.com/renedeanda/cindermark
cd cindermark
cargo test
```

That's it — the core crate builds and tests on any platform (Linux, macOS,
Windows). Apple static libraries and Swift bindings are only needed if you're
working on the FFI surface.

## Before you open a PR

All three gates run in CI on every push and must pass:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

If you changed the UniFFI interface (`src/cindermark.udl` or exported types in
`src/lib.rs`), regenerate the committed Swift bindings so the drift check
passes:

```bash
cargo run --features bindgen --bin uniffi-bindgen generate src/cindermark.udl \
    --language swift --out-dir /tmp/cindermark-bindings
cp /tmp/cindermark-bindings/CindermarkFFI.swift swift/Sources/Cindermark/
```

## Guidelines

- **Tests come with the change.** New syntax support, bug fixes, and perf
  work all need test coverage. The suite is fast — run it often.
- **Perf claims need benchmark numbers.** Use `cargo bench` (criterion) and
  include before/after numbers in the PR description. See
  [docs/PERFORMANCE.md](docs/PERFORMANCE.md) for methodology.
- **UTF-16 offsets are the contract.** Every span the parser emits is
  consumed by `NSTextStorage`-based editors. Changes to offset math need
  tests covering multi-byte characters (emoji, CJK).
- **Panics must not cross the FFI.** The release profile deliberately keeps
  `panic = "unwind"` so UniFFI converts Rust panics into Swift errors. Never
  set `panic = "abort"`, and prefer returning fallbacks over panicking.
- **No new dependencies without discussion.** The crate deliberately has a
  tiny dependency tree (`memchr`, `rustc-hash`, `unicode-segmentation`,
  `uniffi`). Open an issue first if you think one is needed.

## CI notes

- `ci.yml` runs on Linux only and is free to run on every push.
- `release.yml` builds the Apple XCFramework on a macOS runner and is
  **manual-dispatch only** (it costs money). Maintainers run it to cut
  releases; contributors never need it.

## Reporting bugs

Open an issue with a minimal Markdown input, the expected parse, and the
actual parse. Parser bugs with a failing test attached get fixed fastest.
