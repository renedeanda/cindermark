# Cindermark

**A high-performance incremental Markdown parser for native text editors, written in Rust.**

Cindermark is the engine that powers the live Markdown editor in [Ember Notes](https://github.com/renedeanda). It was built for one job and does it well: parsing Markdown *while the user types*, fast enough that a native iOS/macOS editor never waits on it.

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

### Vendored / submodule

Ember Notes consumes Cindermark as a git submodule and links the static library directly:

```bash
git submodule add https://github.com/renedeanda/cindermark
cd cindermark
./build-apple.sh release --out-dir "$YOUR_PROJECT/Parser"
```

This drops `libcindermark.a` (per-SDK: device / simulator / macOS), the generated `CindermarkFFI.swift`, and the `CindermarkFFIFFI` module header into your integration directory. Point `LIBRARY_SEARCH_PATHS` at the per-SDK dirs, add `-lcindermark` to `OTHER_LDFLAGS`, and include the generated Swift file in your target.

## Using from Rust

```toml
[dependencies]
cindermark = { git = "https://github.com/renedeanda/cindermark" }
```

```rust
use cindermark::CindermarkParser;

let parser = CindermarkParser::new();
let result = parser.parse("# Hello\n\nSome **bold** text.".to_string());
```

## Performance

Numbers from `cargo bench` (criterion) on x86_64 Linux; release profile with fat LTO. See [docs/PERFORMANCE.md](docs/PERFORMANCE.md) for methodology.

| Benchmark | Time |
|---|---|
| Incremental keystroke, 500-line note | ~255 µs |
| Incremental keystroke, 2,500-line note | ~1.3 ms |
| Incremental keystroke, 10,000-line note | ~8.7 ms |
| Full parse, 500-line note | ~1.3 ms |
| Full parse, 2,500-line note | ~7.3 ms |

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

Good first issues:

- `***text***` at line start is ambiguous with thematic breaks in some edit sequences.
- Nested inline spans (e.g. `` **`code`** ``) render the outer span only in editable mode.

## License

MIT © René DeAnda. If you ship Cindermark in something cool, [say hi](https://github.com/renedeanda)!
