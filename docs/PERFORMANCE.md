# Cindermark Performance

## Methodology

All numbers come from `cargo bench` (criterion 0.5) with the release profile
(`lto = "fat"`, `codegen-units = 1`). Benchmarks generate synthetic notes with
a realistic block mix (headings, paragraphs, lists, checkboxes, code blocks).

Two families of benchmarks matter, and they answer different questions:

- **Full parse** (`parse_500_lines`, `parse_2500_lines`,
  `parse_editable_500_lines`) — the cost of opening a document or of the
  incremental path's full-reparse fallback.
- **Incremental keystroke** (`incremental_keystroke_*`,
  `incremental_with_stats_2500`) — the cost the editor actually pays per
  debounced edit while typing. This is the hot path.

Run them yourself:

```bash
cargo bench
```

Numbers below are from x86_64 Linux. Apple Silicon is typically faster;
relative deltas are what matter.

## Current numbers

<!-- Updated by perf commits; see git history for per-change deltas. -->

| Benchmark | Time |
|---|---|
| parse_500_lines | ~1.3 ms |
| parse_2500_lines | ~6.6 ms |
| parse_editable_500_lines | ~1.5 ms |
| incremental_keystroke_500 | (see latest bench run) |
| incremental_keystroke_2500 | (see latest bench run) |
| incremental_keystroke_10k | (see latest bench run) |
| incremental_with_stats_2500 | (see latest bench run) |

## What the FFI boundary costs

UniFFI serializes every returned value into a `RustBuffer` that Swift then
lifts into native types. That means:

1. Every `String` in the result is copied at the boundary regardless of how
   the Rust side produced it.
2. The dominant *avoidable* costs on the Rust side are extra full passes over
   the block list before conversion.

The incremental FFI paths used to make up to three such passes per call
(a snapshot clone, a second clone into a temporary document for stats, and
the FFI conversion pass). The conversion pass is required; the clones were
not, and have been eliminated — stats and conversion now borrow the block
list, which is then *moved* (not copied) into the parser's snapshot state.

## Deliberately deferred optimizations

### Borrowed / interned AST strings

`BlockKind` variants own their `String` data (heading text, paragraph text,
table cells, list-marker metadata). A borrowed-AST refactor (`&str` +
lifetimes, or `Arc<str>` interning) would eliminate those allocations during
parsing.

Deferred because:

- UniFFI copies every string into a `RustBuffer` at the FFI boundary anyway,
  so for FFI consumers the win is only the *intermediate* copy, not the
  boundary copy.
- A lifetime-carrying AST ripples through all ~9,400 LOC including the
  incremental engine, which stores blocks in a long-lived snapshot — that
  snapshot would still need owned data, forcing either `Arc<str>` everywhere
  or a copy at snapshot time (which is what happens today, once, via move).
- Benchmarks should drive it: revisit if profiling shows allocation in
  `parser::parse` dominating over lexing + inline scanning.

### Incremental stats

`compute_stats` is O(document) per call (grapheme iteration + byte scan). It
is already skipped on the hot styling path (`*_style_only`); the full variant
runs on the host app's slower content-sync cadence (~1.2 s in Ember Notes).
Making stats incremental (per-block deltas) is possible but adds bookkeeping
complexity for a path that is not latency-critical. Revisit if a host app
needs per-keystroke stats.

### Viewport-aware restyling

Beyond the parser: the biggest remaining editor-side win for very large
documents is restyling only the visible viewport instead of the full dirty
range. That work lives in the host app's text-view layer, not in Cindermark —
the parser already returns the dirty block range needed to drive it.
