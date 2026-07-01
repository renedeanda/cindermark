//! Incremental parse: dirty-block detection and partial re-parse.
//!
//! On each text edit, instead of re-parsing the entire document, we:
//! 1. Identify which blocks overlap the edit region (dirty blocks)
//! 2. Re-parse only those blocks from the new source text
//! 3. Splice the re-parsed blocks into the unchanged prefix/suffix
//! 4. Adjust byte/UTF-16/line offsets for suffix blocks
//!
//! Typical keystroke edits touch 1-3 blocks. A 10,000-line note that
//! would take ~5ms for a full parse now takes <0.5ms for the dirty region.
//!
//! Fallback: If the edit creates or destroys a code fence, we do a full
//! re-parse (still fast) because code fences have unbounded reach —
//! a single ``` can change the type of every subsequent block.

use crate::ast::*;
use crate::parser;
use crate::utf16::Utf16Map;

/// Snapshot of a previous parse, stored for incremental updates.
#[derive(Debug, Clone)]
pub struct ParseSnapshot {
    /// The block AST from the last parse.
    pub blocks: Vec<BlockNode>,
    /// Total line count.
    pub line_count: u32,
    /// Source text byte length.
    pub source_byte_len: usize,
    /// Source text UTF-16 length.
    pub source_utf16_len: u32,
}

/// Result of an incremental parse update.
#[derive(Debug)]
pub struct IncrementalResult {
    /// The full updated block list.
    pub blocks: Vec<BlockNode>,
    /// Updated line count.
    pub line_count: u32,
    /// Index of the first changed block (inclusive).
    pub dirty_start: u32,
    /// Index past the last changed block (exclusive).
    pub dirty_end: u32,
    /// Total UTF-16 length of the new source text. Avoids a redundant
    /// `Utf16Map::build` in the FFI layer for snapshot updates.
    pub total_utf16_len: u32,
}

impl ParseSnapshot {
    /// Create a snapshot from a full parse result.
    pub fn new(
        blocks: Vec<BlockNode>,
        line_count: u32,
        source_byte_len: usize,
        source_utf16_len: u32,
    ) -> Self {
        Self {
            blocks,
            line_count,
            source_byte_len,
            source_utf16_len,
        }
    }
}

/// Perform an incremental re-parse after a text edit.
///
/// # Parameters
/// - `prev`: Snapshot from the previous parse
/// - `new_source`: The full source text after the edit
/// - `edit_utf16_start`: UTF-16 offset where the edit began
/// - `edit_old_utf16_len`: Number of UTF-16 code units replaced (from old text)
/// - `edit_new_utf16_len`: Number of UTF-16 code units inserted (in new text)
/// - `options`: Parse options — must match the options used for the parse
///   that produced `prev`, or incremental results will diverge from a full
///   parse.
///
/// # Returns
/// An `IncrementalResult` with the updated blocks and dirty range.
pub fn incremental_update(
    prev: &ParseSnapshot,
    new_source: &str,
    edit_utf16_start: u32,
    edit_old_utf16_len: u32,
    edit_new_utf16_len: u32,
    options: &parser::ParseOptions,
) -> IncrementalResult {
    // Empty previous state → full parse
    if prev.blocks.is_empty() {
        return full_parse_result(new_source, options);
    }

    let edit_utf16_end_old = edit_utf16_start.saturating_add(edit_old_utf16_len);
    let byte_delta = new_source.len() as i64 - prev.source_byte_len as i64;

    // 1. Find which old blocks overlap the edit region (by UTF-16 ranges).
    let (first_overlap, last_overlap) =
        find_overlap_range(&prev.blocks, edit_utf16_start, edit_utf16_end_old);

    // Expand by 1 block on each side — block boundary detection (e.g., a new
    // heading marker typed at the start of a paragraph changes the previous
    // block's boundary).
    let first_dirty = first_overlap.saturating_sub(1);
    let last_dirty = (last_overlap + 1).min(prev.blocks.len());

    // 2. Build UTF-16 map for the new source (used by fence check and offset computation).
    let new_utf16_map = Utf16Map::build(new_source.as_bytes());

    // 3. Code fence / table check: if the dirty region involves code blocks or tables,
    //    or the edited text might contain a fence, fall back to full re-parse.
    //    Code fences have unbounded reach — one ``` changes everything below.
    //    Tables have multi-line structure that's fragile to incremental edits.
    if needs_full_reparse(
        prev,
        first_dirty,
        last_dirty,
        new_source,
        &new_utf16_map,
        edit_utf16_start,
        edit_new_utf16_len,
    ) {
        return full_parse_result(new_source, options);
    }

    // 4. Determine the byte range to re-parse in the new source.
    //    - Start: first dirty block's byte_start (same in old and new — prefix is unchanged)
    //    - End: last dirty block's byte_end adjusted by byte_delta
    let reparse_byte_start = prev.blocks[first_dirty].byte_start as usize;
    let old_dirty_byte_end = prev.blocks[last_dirty - 1].byte_end as usize;
    let reparse_byte_end_raw = (old_dirty_byte_end as i64 + byte_delta).max(0) as usize;
    let reparse_byte_end =
        extend_to_line_end(new_source, reparse_byte_end_raw.min(new_source.len()));

    // Sanity check: if the reparse region is empty or invalid, fall back.
    if reparse_byte_start >= reparse_byte_end || reparse_byte_start > new_source.len() {
        return full_parse_result(new_source, options);
    }

    // 5. Parse the dirty region as a standalone substring.
    let dirty_text = &new_source[reparse_byte_start..reparse_byte_end];
    let dirty_doc = parser::parse_with_options(dirty_text, ParseMode::Editable, options);

    // 6. Compute base offsets for shifting dirty blocks to absolute positions.
    let base_utf16 = new_utf16_map.byte_to_utf16(reparse_byte_start as u32, new_source.as_bytes());
    let base_line = count_newlines_before(new_source.as_bytes(), reparse_byte_start);

    // 7. Build result: prefix + adjusted dirty blocks + adjusted suffix.
    let prefix_count = first_dirty;
    let suffix_start = last_dirty;
    let dirty_block_count = dirty_doc.blocks.len();
    let total_blocks = prefix_count + dirty_block_count + (prev.blocks.len() - suffix_start);
    let mut result_blocks = Vec::with_capacity(total_blocks);

    // Prefix: unchanged blocks (before dirty region).
    result_blocks.extend_from_slice(&prev.blocks[..first_dirty]);

    // Dirty: re-parsed blocks shifted to absolute offsets.
    for mut block in dirty_doc.blocks {
        shift_block(
            &mut block,
            reparse_byte_start as u32,
            base_utf16,
            base_line as u32,
        );
        result_blocks.push(block);
    }
    let dirty_end_idx = result_blocks.len();

    // Suffix: unchanged blocks with offset deltas applied.
    let suffix_byte_delta = reparse_byte_end as i64 - old_dirty_byte_end as i64;
    let old_dirty_utf16_end = prev.blocks[last_dirty - 1].utf16_end;
    let new_dirty_utf16_end =
        new_utf16_map.byte_to_utf16(reparse_byte_end as u32, new_source.as_bytes());
    let suffix_utf16_delta = new_dirty_utf16_end as i64 - old_dirty_utf16_end as i64;

    let old_dirty_line_end = prev.blocks[last_dirty - 1].line_end;
    let new_dirty_line_end = (base_line as u32).saturating_add(dirty_doc.line_count);
    let suffix_line_delta = new_dirty_line_end as i64 - old_dirty_line_end as i64;

    for block in &prev.blocks[suffix_start..] {
        let mut b = block.clone();
        if !shift_suffix_block(
            &mut b,
            suffix_byte_delta,
            suffix_utf16_delta,
            suffix_line_delta,
        ) {
            // Offset underflow detected — fall back to full re-parse.
            return full_parse_result(new_source, options);
        }
        result_blocks.push(b);
    }

    let new_line_count = match safe_add_delta(prev.line_count, suffix_line_delta) {
        Some(lc) => lc,
        None => return full_parse_result(new_source, options),
    };

    IncrementalResult {
        blocks: result_blocks,
        line_count: new_line_count,
        dirty_start: first_dirty as u32,
        dirty_end: dirty_end_idx as u32,
        total_utf16_len: new_utf16_map.total_utf16_len,
    }
}

// MARK: - Dirty range detection

/// Find the range of old blocks whose UTF-16 spans overlap [edit_start, edit_end).
/// Returns (first_overlapping_index, last_exclusive_index).
fn find_overlap_range(
    blocks: &[BlockNode],
    edit_utf16_start: u32,
    edit_utf16_end: u32,
) -> (usize, usize) {
    // A block overlaps if: block.utf16_start < edit_end AND block.utf16_end > edit_start
    //
    // Special case: insertion at a block boundary (edit_start == edit_end == block.utf16_end)
    // should include that block. Use >= for the end comparison.

    // First block that isn't entirely before the edit.
    let first = blocks.partition_point(|b| b.utf16_end <= edit_utf16_start);

    // First block that is entirely after the edit.
    let last = blocks.partition_point(|b| b.utf16_start < edit_utf16_end);

    // Ensure at least one block is included.
    let first = first.min(blocks.len().saturating_sub(1));
    let last = last.max(first + 1).min(blocks.len());

    (first, last)
}

/// Determine if a full re-parse is needed.
///
/// Returns true if:
/// 1. Any old dirty block is a CodeBlock or Table (multi-line structures fragile to edits)
/// 2. The new text in the edit region contains a code fence (```)
fn needs_full_reparse(
    prev: &ParseSnapshot,
    first_dirty: usize,
    last_dirty: usize,
    new_source: &str,
    new_utf16_map: &Utf16Map,
    edit_utf16_start: u32,
    edit_new_utf16_len: u32,
) -> bool {
    // Check if any old dirty block is a code block, mermaid diagram, or
    // table. All three are multi-line fenced/structured blocks whose reach
    // is unbounded from any single-line edit.
    for block in &prev.blocks[first_dirty..last_dirty] {
        if matches!(
            block.kind,
            BlockKind::CodeBlock { .. }
                | BlockKind::MermaidDiagram { .. }
                | BlockKind::Table { .. }
        ) {
            return true;
        }
    }

    // Check if the edited region in the new text contains a code fence.
    // We scan a generous region around the edit to catch fences created by the edit.
    let scan_utf16_start = edit_utf16_start.saturating_sub(3); // ``` is 3 chars
    let scan_utf16_end = edit_utf16_start
        .saturating_add(edit_new_utf16_len)
        .saturating_add(3);
    let scan_byte_start =
        new_utf16_map.utf16_to_byte(scan_utf16_start, new_source.as_bytes()) as usize;
    let scan_byte_end = (new_utf16_map.utf16_to_byte(scan_utf16_end, new_source.as_bytes())
        as usize)
        .min(new_source.len());

    // Extend to line boundaries for proper fence detection.
    let scan_start = find_line_start(new_source, scan_byte_start);
    let scan_end = extend_to_line_end(new_source, scan_byte_end);
    let scan_region = &new_source[scan_start..scan_end];

    for line in scan_region.lines() {
        if line.trim().starts_with("```") {
            return true;
        }
    }

    false
}

// MARK: - Full parse fallback

/// Fall back to a full re-parse, marking all blocks as dirty.
fn full_parse_result(source: &str, options: &parser::ParseOptions) -> IncrementalResult {
    let doc = parser::parse_with_options(source, ParseMode::Editable, options);
    let block_count = doc.blocks.len() as u32;
    let utf16_map = Utf16Map::build(source.as_bytes());
    IncrementalResult {
        blocks: doc.blocks,
        line_count: doc.line_count,
        dirty_start: 0,
        dirty_end: block_count,
        total_utf16_len: utf16_map.total_utf16_len,
    }
}

// MARK: - Offset adjustment

/// Shift a re-parsed block's offsets from substring-relative to document-absolute.
fn shift_block(block: &mut BlockNode, byte_base: u32, utf16_base: u32, line_base: u32) {
    block.byte_start += byte_base;
    block.byte_end += byte_base;
    block.utf16_start += utf16_base;
    block.utf16_end += utf16_base;
    block.line_start += line_base;
    block.line_end += line_base;

    for span in &mut block.inline_spans {
        span.utf16_start += utf16_base;
        span.utf16_end += utf16_base;
        span.content_utf16_start += utf16_base;
        span.content_utf16_end += utf16_base;
    }

    // Shift inline spans inside list items (BulletList/OrderedList).
    if let BlockKind::BulletList { items } | BlockKind::OrderedList { items, .. } = &mut block.kind
    {
        for item in items {
            for span in &mut item.inline_spans {
                span.utf16_start += utf16_base;
                span.utf16_end += utf16_base;
                span.content_utf16_start += utf16_base;
                span.content_utf16_end += utf16_base;
            }
        }
    }
}

/// Safely add an i64 delta to a u32 value. Returns None if the result would underflow or overflow u32.
fn safe_add_delta(base: u32, delta: i64) -> Option<u32> {
    let result = base as i64 + delta;
    if result < 0 || result > u32::MAX as i64 {
        None
    } else {
        Some(result as u32)
    }
}

/// Adjust a suffix block's offsets by the computed deltas.
/// Returns false if any offset would underflow (caller should fall back to full reparse).
fn shift_suffix_block(
    block: &mut BlockNode,
    byte_delta: i64,
    utf16_delta: i64,
    line_delta: i64,
) -> bool {
    let Some(bs) = safe_add_delta(block.byte_start, byte_delta) else {
        return false;
    };
    let Some(be) = safe_add_delta(block.byte_end, byte_delta) else {
        return false;
    };
    let Some(us) = safe_add_delta(block.utf16_start, utf16_delta) else {
        return false;
    };
    let Some(ue) = safe_add_delta(block.utf16_end, utf16_delta) else {
        return false;
    };
    let Some(ls) = safe_add_delta(block.line_start, line_delta) else {
        return false;
    };
    let Some(le) = safe_add_delta(block.line_end, line_delta) else {
        return false;
    };

    block.byte_start = bs;
    block.byte_end = be;
    block.utf16_start = us;
    block.utf16_end = ue;
    block.line_start = ls;
    block.line_end = le;

    for span in &mut block.inline_spans {
        let Some(ss) = safe_add_delta(span.utf16_start, utf16_delta) else {
            return false;
        };
        let Some(se) = safe_add_delta(span.utf16_end, utf16_delta) else {
            return false;
        };
        let Some(cs) = safe_add_delta(span.content_utf16_start, utf16_delta) else {
            return false;
        };
        let Some(ce) = safe_add_delta(span.content_utf16_end, utf16_delta) else {
            return false;
        };
        span.utf16_start = ss;
        span.utf16_end = se;
        span.content_utf16_start = cs;
        span.content_utf16_end = ce;
    }

    if let BlockKind::BulletList { items } | BlockKind::OrderedList { items, .. } = &mut block.kind
    {
        for item in items {
            for span in &mut item.inline_spans {
                let Some(ss) = safe_add_delta(span.utf16_start, utf16_delta) else {
                    return false;
                };
                let Some(se) = safe_add_delta(span.utf16_end, utf16_delta) else {
                    return false;
                };
                let Some(cs) = safe_add_delta(span.content_utf16_start, utf16_delta) else {
                    return false;
                };
                let Some(ce) = safe_add_delta(span.content_utf16_end, utf16_delta) else {
                    return false;
                };
                span.utf16_start = ss;
                span.utf16_end = se;
                span.content_utf16_start = cs;
                span.content_utf16_end = ce;
            }
        }
    }

    true
}

// MARK: - Line boundary helpers

/// Count newlines before the given byte offset.
fn count_newlines_before(source: &[u8], byte_offset: usize) -> usize {
    source[..byte_offset.min(source.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
}

/// Find the start of the line containing `byte_offset` (scan backward to \n or start).
fn find_line_start(source: &str, byte_offset: usize) -> usize {
    let offset = byte_offset.min(source.len());
    if offset == 0 {
        return 0;
    }
    let bytes = source.as_bytes();
    // Scan backward from offset-1 to find the previous newline.
    for i in (0..offset).rev() {
        if bytes[i] == b'\n' {
            return i + 1;
        }
    }
    0
}

/// Extend `byte_offset` forward to the end of its line (past \n or to EOF).
/// If the offset is already at a line boundary (right after \n or at the
/// start/end of the source), returns the offset unchanged — no extension needed.
fn extend_to_line_end(source: &str, byte_offset: usize) -> usize {
    let offset = byte_offset.min(source.len());
    if offset == source.len() {
        return offset;
    }
    // Already at a line boundary? (After a \n or at document start.)
    if offset == 0 || source.as_bytes()[offset - 1] == b'\n' {
        return offset;
    }
    // Mid-line: scan forward to include the rest of this line.
    let bytes = source.as_bytes();
    for (i, &b) in bytes.iter().enumerate().skip(offset) {
        if b == b'\n' {
            return i + 1;
        }
    }
    source.len()
}

// MARK: - Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    /// Helper: full parse → snapshot.
    fn snapshot(source: &str) -> ParseSnapshot {
        let doc = parser::parse(source, ParseMode::Editable);
        let utf16_map = Utf16Map::build(source.as_bytes());
        ParseSnapshot::new(
            doc.blocks,
            doc.line_count,
            source.len(),
            utf16_map.total_utf16_len,
        )
    }

    /// Helper: verify incremental result matches full parse.
    fn assert_matches_full_parse(result: &IncrementalResult, new_source: &str) {
        let full = parser::parse(new_source, ParseMode::Editable);
        assert_eq!(
            result.blocks.len(),
            full.blocks.len(),
            "Block count mismatch: incremental={}, full={}",
            result.blocks.len(),
            full.blocks.len()
        );
        for (i, (inc, ful)) in result.blocks.iter().zip(full.blocks.iter()).enumerate() {
            assert_eq!(
                inc.kind, ful.kind,
                "Block {} kind mismatch:\n  incremental: {:?}\n  full: {:?}",
                i, inc.kind, ful.kind
            );
            assert_eq!(
                inc.byte_start, ful.byte_start,
                "Block {} byte_start: inc={}, full={}",
                i, inc.byte_start, ful.byte_start
            );
            assert_eq!(
                inc.byte_end, ful.byte_end,
                "Block {} byte_end: inc={}, full={}",
                i, inc.byte_end, ful.byte_end
            );
            assert_eq!(
                inc.utf16_start, ful.utf16_start,
                "Block {} utf16_start: inc={}, full={}",
                i, inc.utf16_start, ful.utf16_start
            );
            assert_eq!(
                inc.utf16_end, ful.utf16_end,
                "Block {} utf16_end: inc={}, full={}",
                i, inc.utf16_end, ful.utf16_end
            );
            assert_eq!(
                inc.line_start, ful.line_start,
                "Block {} line_start: inc={}, full={}",
                i, inc.line_start, ful.line_start
            );
            assert_eq!(
                inc.line_end, ful.line_end,
                "Block {} line_end: inc={}, full={}",
                i, inc.line_end, ful.line_end
            );
            assert_eq!(
                inc.inline_spans.len(),
                ful.inline_spans.len(),
                "Block {} inline_span count mismatch: inc={}, full={}",
                i,
                inc.inline_spans.len(),
                ful.inline_spans.len()
            );
            for (j, (is_span, f_span)) in inc
                .inline_spans
                .iter()
                .zip(ful.inline_spans.iter())
                .enumerate()
            {
                assert_eq!(
                    is_span, f_span,
                    "Block {} span {} mismatch:\n  incremental: {:?}\n  full: {:?}",
                    i, j, is_span, f_span
                );
            }
        }
        assert_eq!(result.line_count, full.line_count, "Line count mismatch");
    }

    // MARK: - Core incremental tests

    #[test]
    fn single_char_insert_in_paragraph() {
        let old = "# Title\n\nHello world\n\n---";
        let snap = snapshot(old);
        // Insert "X" at the start of "Hello world" → "XHello world"
        let new = "# Title\n\nXHello world\n\n---";
        // Edit at UTF-16 offset 9 (after "# Title\n\n"), old len 0, new len 1
        let result = incremental_update(&snap, new, 9, 0, 1, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
        // Should NOT re-parse the entire document — dirty range should be small.
        assert!(
            result.dirty_end - result.dirty_start <= 3,
            "Dirty range too large: {}..{}",
            result.dirty_start,
            result.dirty_end
        );
    }

    #[test]
    fn single_char_insert_in_heading() {
        let old = "# Title\n\nSome text";
        let snap = snapshot(old);
        let new = "# Titles\n\nSome text";
        // Insert "s" at offset 7 (after "Title")
        let result = incremental_update(&snap, new, 7, 0, 1, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn delete_characters() {
        let old = "# Title\n\nHello world\n\n---";
        let snap = snapshot(old);
        // Delete "Hello" (5 chars at offset 9)
        let new = "# Title\n\n world\n\n---";
        let result = incremental_update(&snap, new, 9, 5, 0, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn replace_text() {
        let old = "# Title\n\nHello\n\nWorld";
        let snap = snapshot(old);
        // Replace "Hello" (5 chars at offset 9) with "Goodbye" (7 chars)
        let new = "# Title\n\nGoodbye\n\nWorld";
        let result = incremental_update(&snap, new, 9, 5, 7, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn insert_newline_splits_paragraph() {
        let old = "# Title\n\nHello world";
        let snap = snapshot(old);
        // Insert newline in "Hello world" → "Hello\nworld"
        let new = "# Title\n\nHello\nworld";
        let result = incremental_update(&snap, new, 14, 1, 1, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn edit_at_document_end() {
        let old = "# Title\n\nText";
        let snap = snapshot(old);
        // Append " more" at end
        let new = "# Title\n\nText more";
        let result = incremental_update(&snap, new, 13, 0, 5, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn edit_at_document_start() {
        let old = "Hello\n\nWorld";
        let snap = snapshot(old);
        // Insert "# " at start → makes it a heading
        let new = "# Hello\n\nWorld";
        let result = incremental_update(&snap, new, 0, 0, 2, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn edit_in_checkbox() {
        let old = "- [ ] Task one\n- [x] Task two";
        let snap = snapshot(old);
        // Append " done" to first task
        let new = "- [ ] Task one done\n- [x] Task two";
        let result = incremental_update(&snap, new, 14, 0, 5, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn edit_in_bullet_list() {
        let old = "- item 1\n- item 2\n- item 3";
        let snap = snapshot(old);
        // Edit "item 2" → "item TWO"
        let new = "- item 1\n- item TWO\n- item 3";
        let result = incremental_update(&snap, new, 11, 6, 8, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn code_fence_triggers_full_reparse() {
        let old = "# Title\n\nSome text\n\nMore text";
        let snap = snapshot(old);
        // Type "```" on the "Some text" line → code fence
        let new = "# Title\n\n```\nSome text\n\nMore text";
        let result = incremental_update(&snap, new, 9, 0, 4, &parser::ParseOptions::default());
        // Should fall back to full re-parse
        assert_eq!(result.dirty_start, 0);
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn edit_inside_code_block_triggers_full_reparse() {
        let old = "# Title\n\n```\ncode\n```\n\nText";
        let snap = snapshot(old);
        // Edit inside code block
        let new = "# Title\n\n```\ncode here\n```\n\nText";
        let result = incremental_update(&snap, new, 13, 4, 9, &parser::ParseOptions::default());
        // Code block → full reparse
        assert_eq!(result.dirty_start, 0);
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn empty_document_insert() {
        let old = "";
        let snap = snapshot(old);
        let new = "Hello";
        let result = incremental_update(&snap, new, 0, 0, 5, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn single_block_document() {
        let old = "Hello";
        let snap = snapshot(old);
        let new = "Hello world";
        let result = incremental_update(&snap, new, 5, 0, 6, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn delete_entire_line() {
        let old = "# Title\n\nMiddle\n\nEnd";
        let snap = snapshot(old);
        // Delete "Middle\n" line
        let new = "# Title\n\n\nEnd";
        let result = incremental_update(&snap, new, 9, 7, 0, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn insert_new_line_between_blocks() {
        let old = "# Title\n\nEnd";
        let snap = snapshot(old);
        // Insert "Middle\n\n" between heading and End
        let new = "# Title\n\nMiddle\n\nEnd";
        let result = incremental_update(&snap, new, 9, 0, 8, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn edit_with_inline_formatting() {
        let old = "Hello **world**";
        let snap = snapshot(old);
        // Change "world" → "earth" inside bold
        let new = "Hello **earth**";
        let result = incremental_update(&snap, new, 8, 5, 5, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn multiple_blocks_edit() {
        let old = "# One\n\n## Two\n\n### Three\n\nParagraph";
        let snap = snapshot(old);
        // Edit "Two" → "TWO"
        let new = "# One\n\n## TWO\n\n### Three\n\nParagraph";
        let result = incremental_update(&snap, new, 10, 3, 3, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
        // Should not dirty the entire document
        assert!(result.dirty_end - result.dirty_start <= 4);
    }

    #[test]
    fn edit_horizontal_rule() {
        let old = "Before\n\n---\n\nAfter";
        let snap = snapshot(old);
        // Change --- to --
        let new = "Before\n\n--\n\nAfter";
        let result = incremental_update(&snap, new, 10, 1, 0, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn unicode_edit() {
        let old = "Hello 🌍\n\nWorld";
        let snap = snapshot(old);
        // Insert after emoji (🌍 is 4 bytes / 2 UTF-16 units)
        // "Hello 🌍" = H(1) e(1) l(1) l(1) o(1) ' '(1) 🌍(2) = 8 UTF-16 units
        let new = "Hello 🌍!\n\nWorld";
        let result = incremental_update(&snap, new, 8, 0, 1, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn blockquote_edit() {
        let old = "# Title\n\n> Quote text\n\nEnd";
        let snap = snapshot(old);
        let new = "# Title\n\n> Quote text more\n\nEnd";
        let result = incremental_update(&snap, new, 20, 0, 5, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn dirty_range_is_minimal() {
        // 10 blocks, edit in the middle — dirty range should be ~3 blocks
        let old = "# H1\n\n## H2\n\n### H3\n\nPara\n\n- B1\n\n- B2\n\n> Quote\n\n---\n\nEnd";
        let snap = snapshot(old);
        // Edit "Para" → "ParaX"
        let new = "# H1\n\n## H2\n\n### H3\n\nParaX\n\n- B1\n\n- B2\n\n> Quote\n\n---\n\nEnd";
        // "ParaX" starts at UTF-16 offset 21 in the text
        let result = incremental_update(&snap, new, 25, 0, 1, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
        // Dirty range should be small (not the full document).
        let dirty_count = result.dirty_end - result.dirty_start;
        assert!(
            dirty_count <= 4,
            "Expected ≤4 dirty blocks, got {} ({}..{})",
            dirty_count,
            result.dirty_start,
            result.dirty_end
        );
    }

    // MARK: - Helper tests

    #[test]
    fn test_find_line_start() {
        assert_eq!(find_line_start("hello\nworld", 0), 0);
        assert_eq!(find_line_start("hello\nworld", 6), 6);
        assert_eq!(find_line_start("hello\nworld", 8), 6);
        assert_eq!(find_line_start("hello\nworld\nfoo", 12), 12);
    }

    #[test]
    fn test_extend_to_line_end() {
        // Mid-line: extends to end of "hello\n"
        assert_eq!(extend_to_line_end("hello\nworld", 3), 6);
        // At line boundary (after \n): stays put
        assert_eq!(extend_to_line_end("hello\nworld", 6), 6);
        // Mid-line in second line: extends to end
        assert_eq!(extend_to_line_end("hello\nworld", 8), 11);
        // At EOF: stays
        assert_eq!(extend_to_line_end("hello\nworld", 11), 11);
        // At document start (line boundary): stays
        assert_eq!(extend_to_line_end("hello\nworld", 0), 0);
        // After trailing \n: stays
        assert_eq!(extend_to_line_end("hello\nworld\n", 12), 12);
    }

    #[test]
    fn test_count_newlines_before() {
        assert_eq!(count_newlines_before(b"hello\nworld\nfoo", 0), 0);
        assert_eq!(count_newlines_before(b"hello\nworld\nfoo", 6), 1);
        assert_eq!(count_newlines_before(b"hello\nworld\nfoo", 12), 2);
    }

    #[test]
    fn test_find_overlap_range() {
        // Create 3 blocks: [0..5), [5..10), [10..15)
        let blocks = vec![
            make_test_block(0, 5),
            make_test_block(5, 10),
            make_test_block(10, 15),
        ];

        // Edit in middle block
        assert_eq!(find_overlap_range(&blocks, 6, 8), (1, 2));
        // Edit at start
        assert_eq!(find_overlap_range(&blocks, 0, 3), (0, 1));
        // Edit spanning blocks 1-2
        assert_eq!(find_overlap_range(&blocks, 4, 11), (0, 3));
        // Insertion at block boundary (offset 5, len 0)
        let (f, l) = find_overlap_range(&blocks, 5, 5);
        assert!(
            f <= 1 && l >= 2,
            "Boundary insertion should include adjacent block: {f}..{l}"
        );
    }

    #[test]
    fn table_edit_triggers_full_reparse() {
        let old = "# Title\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n\nEnd";
        let snap = snapshot(old);
        // Edit inside table cell: "1" → "X"
        let new = "# Title\n\n| A | B |\n| --- | --- |\n| X | 2 |\n\nEnd";
        let result = incremental_update(&snap, new, 34, 1, 1, &parser::ParseOptions::default());
        // Table block → full reparse
        assert_eq!(result.dirty_start, 0);
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn ordered_list_edit() {
        let old = "1. first\n2. second\n3. third";
        let snap = snapshot(old);
        // Edit "second" → "SECOND"
        let new = "1. first\n2. SECOND\n3. third";
        let result = incremental_update(&snap, new, 12, 6, 6, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn large_deletion_removes_multiple_blocks() {
        let old = "# H1\n\n## H2\n\n### H3\n\nParagraph\n\nEnd";
        let snap = snapshot(old);
        // Delete everything between H1 and End: "## H2\n\n### H3\n\nParagraph\n\n"
        let new = "# H1\n\nEnd";
        // Edit at UTF-16 offset 6 (after "# H1\n\n"), old len 26, new len 0
        let result = incremental_update(&snap, new, 6, 26, 0, &parser::ParseOptions::default());
        assert_matches_full_parse(&result, new);
    }

    #[test]
    fn chained_incremental_edits() {
        // Simulate typing "abc" one character at a time.
        let old = "# Title\n\nHello";
        let snap = snapshot(old);

        // Type 'a' at end
        let new1 = "# Title\n\nHelloa";
        let r1 = incremental_update(&snap, new1, 14, 0, 1, &parser::ParseOptions::default());
        assert_matches_full_parse(&r1, new1);

        // Build snapshot from r1 for next edit
        let utf16_map = Utf16Map::build(new1.as_bytes());
        let snap1 = ParseSnapshot::new(
            r1.blocks,
            r1.line_count,
            new1.len(),
            utf16_map.total_utf16_len,
        );

        // Type 'b' at end
        let new2 = "# Title\n\nHelloab";
        let r2 = incremental_update(&snap1, new2, 15, 0, 1, &parser::ParseOptions::default());
        assert_matches_full_parse(&r2, new2);

        // Build snapshot from r2
        let utf16_map2 = Utf16Map::build(new2.as_bytes());
        let snap2 = ParseSnapshot::new(
            r2.blocks,
            r2.line_count,
            new2.len(),
            utf16_map2.total_utf16_len,
        );

        // Type 'c' at end
        let new3 = "# Title\n\nHelloabc";
        let r3 = incremental_update(&snap2, new3, 16, 0, 1, &parser::ParseOptions::default());
        assert_matches_full_parse(&r3, new3);
    }

    fn make_test_block(utf16_start: u32, utf16_end: u32) -> BlockNode {
        BlockNode {
            kind: BlockKind::Paragraph {
                text: String::new(),
            },
            line_start: 0,
            line_end: 1,
            utf16_start,
            utf16_end,
            byte_start: utf16_start,
            byte_end: utf16_end,
            list_marker: None,
            inline_spans: vec![],
        }
    }
}
