# Compatibility profile (0.3.0 development)

Cindermark is oriented toward [CommonMark 0.31.2](https://spec.commonmark.org/0.31.2/),
with GFM-oriented tables, task lists, strikethrough and extended autolinks.
Wiki links, highlights, comments, callouts, Mermaid, underline and math are
Cindermark extensions, not CommonMark syntax. No complete upstream conformance
result is currently published.

## Source and ranges

The caller retains the authoritative source. Parsing does not serialize or
normalize it. Rust block offsets are byte-based; inline, table-cell and FFI
ranges are absolute UTF-16 offsets with exclusive ends. Content ranges exclude
delimiters; full ranges retain them. No Unicode normalization is performed.
Block expression strings omit structural fences and ordinary fence indentation;
their line separators are LF. Use original source ranges, not these derived
strings, for lossless copy, edits and export, including CRLF preservation.
Quoted math reports `quote_depth` (`quoteDepth` in WASM). Its content range is
the source envelope, including intervening quote prefixes; its expression
omits those prefixes. Full block ranges retain every source character.
List math additionally reports the list's content indentation and kind
(0 none, 1 bullet, 2 ordered, 3 unchecked task, 4 checked task). The block's
number and checked fields retain the containing item's identity. A marker on
the math opening line retains its exact marker metadata; a continuation does
not invent a duplicate marker. Task continuation indentation belongs to the
bullet marker, not the visual width of `[ ]`.
When a tab crosses a list indentation boundary, remaining columns become spaces
in the derived math expression; source ranges still address the original tab.

Table cells retain the existing header/row strings and additionally expose
inline spans and source ranges. Row zero is the header; row one is the first
data row. The separator row is structural and has no cells. Escaped pipes do
not split cells. The existing 20-header-column and 500-data-row limits remain.

## Underline

`++text++` produces `UnderlinePlus`, semantically equivalent to the existing
single-tilde and `<u>` underline forms. Exactly two plus signs delimit nonempty
content. Longer runs and intraword pairs remain literal. Delimiters use Unicode
flanking with underscore-like intraword restrictions. An odd preceding
backslash run escapes a delimiter; even runs leave it available.

Code, comments, autolinks, link destinations and HTML attributes are opaque.
Delimited inline HTML comments, processing instructions, declarations and CDATA
also protect their contents from plus underline and math. Unclosed inline forms
do not hide subsequent extensions. This is distinct from HTML block handling,
which remains an outstanding conformance gate below.
Link labels and table cells expose the new syntax. Math expressions do not
expose underline or other Markdown spans. Nested emphasis is supported.

## Math

The parser identifies opaque TeX source; it does not validate expressions,
expand macros, execute code or load resources.

- Inline `$…$` stays on one logical line. The opener must precede non-whitespace;
  the closer must follow non-whitespace and must not precede an ASCII digit.
  Escaped dollars remain source text. Empty and unmatched pairs remain literal.
  These boundaries follow [Pandoc dollar math](https://pandoc.org/MANUAL.html#extension-tex_math_dollars).
- A standalone `$$…$$` line or matching `$$` delimiter lines produce display
  math. Blank lines terminate recognition; unclosed dollar blocks remain raw.
  Ordinary opening indentation is limited to three spaces.
- Explicitly prefixed blockquotes support dollar and fenced math, including
  nested quote depth. Recognition stops at the enclosing quote boundary;
  unclosed dollar forms remain literal, while fences end at that boundary.
- List items support math on their marker line or in an indented continuation,
  including nested lists, lists inside explicitly prefixed blockquotes and
  explicitly prefixed quotes inside lists. Quote depth counts both enclosing
  and inner quote prefixes; the list indentation is relative to outer quotes.
  Fences permit blank content lines and stop when the list container ends.
- Fences classify the first info-string word, case-insensitively: `math`,
  `latex` or `tex`. The full info string is retained. Backtick and tilde fences
  require at least three identical characters; closing fences use the same
  character and at least the opening length. Unclosed fences follow code-fence
  behavior. The `math` fence is also documented by
  [GitHub](https://docs.github.com/en/get-started/writing-on-github/working-with-advanced-formatting/writing-mathematical-expressions).
- GitHub's dollar/backtick inline form and TeX `\(...\)` / `\[...\]` delimiters
  are deliberately unsupported and remain literal.

## Consumer responsibilities

Renderers decide which TeX commands they support and must preserve accessible
raw-source fallback for unsupported or excessive expressions. Parsing is not
HTML sanitization: HTML exporters must escape text and attributes, validate
link schemes and never evaluate source as HTML, JavaScript or TeX macros.

0.3.0 adds exhaustive Rust/UniFFI enum variants and table-cell fields, requiring
consumer source updates and regenerated bindings from the same revision.
Existing UniFFI variant order is retained and new variants are appended.
WASM full-parse JSON declares `schema_version: 2`, retaining existing keys and
adding math syntax/content metadata and `tableCells`.

## Known deviations and outstanding development gates

- Lists remain column-based rather than a complete CommonMark container tree.
  Arbitrarily alternating list/quote containers and lazy quote continuations
  are not covered by a full conformance result.
- Incremental edits before math conservatively reparse the document because
  a changed ancestor list marker can alter a distant math container. Plain-note
  incremental parsing remains available; container checkpoints are not yet cached.
- Bare CR line splitting, NUL replacement and general HTML-block semantics do
  not yet constitute CommonMark conformance.
- Previews preserve readable math delimiters and opaque expressions; display
  forms use a dollar-block fallback. Preview text is derived, not lossless
  source serialization. Formatting does not pair across separate block parts.
- Current compatibility fixtures are original test inputs, not an authenticated
  Apple Notes export bundle. Real export-package compatibility remains a gate.

Upstream fixture text is not vendored by this change. A future conformance
harness must preserve the upstream specification's attribution and license.
