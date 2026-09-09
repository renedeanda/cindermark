# Migrating from 0.2 to 0.3

0.3 adds source-ranged math, plus underline, table-cell spans, and on-demand
queries for list subtrees and resource references. It is a breaking source
release, not a claim of complete CommonMark or LaTeX support.

## Rust

Update exhaustive matches for `BlockKind::Math`, `BlockKind::RawHtml`,
`InlineKind::UnderlinePlus`, and `InlineKind::Math`. The public FFI-facing
enums also gain corresponding variants. Struct literals constructing
`BlockNode` need `table_cells`; consumers constructing FFI records must account
for their added table-cell fields. Consult generated API documentation for
the complete record definitions.

Handle math as opaque expression content. Its recognition does not imply that
a renderer supports or validates it. Raw HTML is source, not executable output.
Map plus underline to the same presentation as the existing underline forms.

Block byte ranges address UTF-8 source. Inline and table-cell ranges are absolute
UTF-16 offsets; FFI ranges are UTF-16. Do not use them directly as Rust string
indices. Keep the original source for editing and saving: derived expression
strings can omit container prefixes and normalize line separators.

List subtree and resource-reference queries do not replace the incremental
snapshot. Their ranges and indices belong to the queried source only; recompute
them after edits. Resource destinations are untrusted strings, not permission
to read files or load URLs.

The default feature set excludes UniFFI. Enable `ffi` for Apple scaffolding,
`bindgen` for the binding generator, or `wasm` for the browser surface.

## Swift and other UniFFI consumers

Regenerate bindings and native libraries from the same revision. Existing wire
variant order is retained and new variants are appended, but mixing 0.2 bindings
with 0.3 libraries is unsupported. Update exhaustive switches and initializers.
Table cells now carry inline spans and source ranges in addition to cell text.

## WASM

Full-parse JSON uses `schema_version: 2`. Existing keys remain; consumers must
handle math, raw HTML, table-cell spans and associated syntax/range metadata.
Unknown kinds should retain visible source rather than disappear. Serve the
generated JavaScript and WASM from the same build.

## Compatibility and performance

See [the compatibility profile](compatibility.md) for delimiter boundaries,
container limits and literal fallback. Math and raw HTML can require conservative
full reparsing after earlier edits; benchmark representative documents before
assuming the plain-note incremental cost applies to them.
