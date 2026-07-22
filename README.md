# Cindermark

[![crates.io](https://img.shields.io/crates/v/cindermark.svg)](https://crates.io/crates/cindermark)
[![docs.rs](https://img.shields.io/docsrs/cindermark)](https://docs.rs/cindermark)
[![CI](https://github.com/renedeanda/cindermark/actions/workflows/ci.yml/badge.svg)](https://github.com/renedeanda/cindermark/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/renedeanda/cindermark)](https://github.com/renedeanda/cindermark/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platforms](https://img.shields.io/badge/platforms-iOS%20%7C%20macOS%20%7C%20Rust-lightgrey)

**A high-performance incremental Markdown parser for native text editors, written in Rust.**

Cindermark is the engine that powers the live Markdown editor in [Ember Notes](https://embernotes.app). It was built for one job and does it well: parsing Markdown *while the user types*, fast enough that a native iOS/macOS editor never waits on it.

Most Markdown parsers are built for rendering documents. Cindermark is built for **editing** them:

- **UTF-16 offsets, natively.** Every block and inline span carries UTF-16 ranges — the coordinate system of `NSTextStorage`, `NSAttributedString`, and TextKit. No conversion layer, no off-by-one emoji bugs in Swift.
- **Incremental re-parsing.** After an edit, Cindermark re-parses only the dirty blocks and shifts the rest, then tells you exactly which block range changed so you can restyle just that region.
- **Single-pass architecture.** One pass over the source produces the full block + inline AST, document stats (word counts, reading time, checkbox progress), wiki links, and headings. It replaced 8–9 full-text regex passes in the app it came from.
- **First-class Swift bindings.** Generated with [UniFFI](https://mozilla.github.io/uniffi-rs/) — a real Swift API, not a C header. Rust panics surface as catchable Swift errors, never app crashes.
- **Tiny dependency tree.** `memchr`, `rustc-hash`, `unicode-segmentation`, `uniffi`. That's it.

## Syntax support

CommonMark core plus the extensions a notes app actually needs:

| Category | Supported |
|---|---|
| Blocks | Headings, paragraphs, fenced code blocks (with language), blockquotes, bullet/ordered lists (nested), task lists / checkboxes, tables (with alignment), horizontal rules, footnote definitions, callouts, Mermaid diagrams (typed) |
| Inline | Bold, italic, bold-italic (full CommonMark delimiter-run algorithm incl. Unicode flanking), strikethrough, inline code (multi-backtick), links, autolinks (bare URLs, domains, emails, subreddits), wiki links `[[...]]`, highlights `==...==` (plus colored/hex variants), underline (`<u>`/tilde), footnote refs, hex color literals, comments |
| Editor extras | Document stats as a parse byproduct, wiki-link extraction, heading outline extraction, checkbox toggling, plain-text preview rendering with span ranges, configurable image-marker URI scheme for attachment placeholders |

Everything is covered by **420+ tests**, including checks that every incremental parse result must equal the equivalent full parse.

## Using from Swift (iOS / macOS)

### Swift Package Manager

```swift
.package(url: "https://github.com/renedeanda/cindermark", from: "0.1.0")
```

```swift
import Cindermark

let parser = CindermarkParser()
// Or opt in to the attachment-marker extension with your own URI scheme,
// so `![](myapp:<UUID>)` lines parse as ImageMarker blocks:
// let parser = CindermarkParser(imageMarkerScheme: "myapp:")
let result = parser.parseEditable(text: markdown)

for block in result.blocks {
    // block.utf16Start / block.utf16End map directly onto NSTextStorage
    // block.inlineSpans carry per-span UTF-16 ranges for styling
}
```

For live editing, feed edits to the incremental API and restyle only the dirty range:

```swift
let update = parser.parseEditableIncrementalStyleOnly(
    text: newText,
    editUtf16Start: editStart,
    editOldUtf16Len: oldLen,
    editNewUtf16Len: newLen
)
// Restyle only blocks in update.dirtyStart..<update.dirtyEnd
```

> **Note:** the SwiftPM binary target resolves for **tagged releases**. If you're building from an untagged checkout, use `build-apple.sh` below instead.

### Advanced: build from source (vendored / submodule)

Most apps should use Swift Package Manager above. This path is for building
directly from source — first-party integrations, contributors, or building from
an untagged commit with custom flags. [Ember Notes](https://embernotes.app)
consumes Cindermark as a git submodule and links the static library directly:

```bash
git submodule add https://github.com/renedeanda/cindermark
cd cindermark
./build-apple.sh release --out-dir "$YOUR_PROJECT/Parser"
```

This drops `libcindermark.a` (per-SDK: device / simulator / macOS), the generated `CindermarkFFI.swift`, and the `CindermarkFFIFFI` module header into your integration directory. Point `LIBRARY_SEARCH_PATHS` at the per-SDK dirs, add `-lcindermark` to `OTHER_LDFLAGS`, and include the generated Swift file in your target.

## Using from Rust

```toml
[dependencies]
cindermark = "0.1"
```

```rust
use cindermark::CindermarkParser;

// Pass None for CommonMark-clean defaults, or Some("myapp:".into()) to
// enable the attachment-marker extension.
let parser = CindermarkParser::new(None);
let result = parser.parse("# Hello\n\nSome **bold** text.".to_string());
```

## Performance

Numbers from `cargo bench` (criterion); release profile with fat LTO. See [docs/PERFORMANCE.md](docs/PERFORMANCE.md) for methodology.

| Benchmark | Apple Silicon | x86_64 Linux |
|---|---|---|
| Incremental keystroke, 500-line note | ~117 µs | ~255 µs |
| Incremental keystroke, 2,500-line note | ~562 µs | ~1.3 ms |
| Incremental keystroke, 10,000-line note | ~2.3 ms | ~8.7 ms |
| Full parse, 500-line note | ~666 µs | ~1.3 ms |
| Full parse, 2,500-line note | ~3.2 ms | ~7.3 ms |

The design targets the editor's real budget: a debounced keystroke on a large document must cost single-digit milliseconds, and it does — even at 10,000 lines.

## Architecture

```
src/
├── lexer.rs        # UTF-8 byte scanner + block tokenizer (memchr-accelerated)
├── parser.rs       # Single-pass block parser (grouped + editable modes)
├── inline.rs       # CommonMark inline spans: delimiter-run emphasis, links,
│                   # autolinks, highlights, wiki links, code, footnotes…
├── incremental.rs  # Dirty-block detection + partial re-parse + offset shifting
├── ast.rs          # Block + inline AST node types
├── utf16.rs        # UTF-8 → UTF-16 offset mapping (O(1) ASCII fast path)
├── lib.rs          # UniFFI FFI layer: CindermarkParser object + Ffi* types
└── cindermark.udl  # UniFFI interface definition
```

Design notes:

- **Editable vs grouped mode.** Grouped mode merges list items into list blocks (for rendering); editable mode keeps every line's block separate (for per-line editor styling).
- **Incremental strategy.** Edits are located by binary search over block UTF-16 ranges, expanded ±1 block for boundary effects, and re-parsed as a substring. Code fences and tables have unbounded reach, so edits touching them fall back to a full parse — correctness first.
- **Panic safety.** The release profile keeps `panic = "unwind"` so UniFFI converts any parser panic into a Swift error instead of killing the host app.

## Building

```bash
cargo test          # full suite, any platform
cargo bench         # criterion benchmarks
./build-apple.sh    # Apple static libs + Swift bindings (requires macOS + rustup Apple targets)
```

## Known limitations

Nested lists are column-based, not CommonMark container-based:

- A list/checkbox marker may be indented up to 32 tab-expanded columns
  (tab = next multiple-of-4 column) and always parses as a list item —
  nesting depth is the leading-column count, not the CommonMark
  "marker width + 1 relative to the parent" rule. This keeps every
  line's classification local (required for incremental parity) at the
  cost of §4.4 fidelity: a 4-space-indented `- item` is a nested list
  item here, never an indented code block. Indented lines *without* a
  list marker still parse as indented code.
- Loose vs. tight lists are not distinguished; blank lines always
  terminate a list run.
- Continuation paragraphs inside a list item (a following line indented
  to the item's content column) are not supported — in grouped mode the
  line is appended to the previous item's text, in editable mode it
  parses as its own paragraph/code block.

Good first issues:

- `***text***` at line start is ambiguous with thematic breaks in some edit sequences.
- Nested inline spans (e.g. `` **`code`** ``) render the outer span only in editable mode.

## License

MIT © René DeAnda. If you ship Cindermark in something cool, [say hi](https://github.com/renedeanda)!
