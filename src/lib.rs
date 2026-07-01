//! Cindermark — High-Performance Incremental Markdown Parser
//!
//! A Rust-native markdown parser designed for real-time editing in native
//! iOS/macOS text editors. Cindermark is the engine that powers the editor
//! in Ember Notes. Features:
//!
//! - Single-pass lexer + parser producing block + inline AST
//! - SIMD-accelerated byte scanning via `memchr`
//! - UTF-8 to UTF-16 offset mapping for NSTextStorage compatibility
//! - UniFFI-generated Swift bindings (zero manual C bridging)
//! - Incremental dirty-region tracking (partial re-parse of edited blocks)

#![allow(clippy::empty_line_after_doc_comments)] // UniFFI 0.28 generated scaffolding.

pub mod ast;
pub mod incremental;
pub mod inline;
pub mod lexer;
pub mod parser;
pub mod utf16;

use ast::*;
use std::sync::Mutex;

// MARK: - FFI Types

/// Block type enum for FFI.
#[derive(Debug, Clone, PartialEq)]
pub enum FfiBlockType {
    Heading,
    Paragraph,
    CodeBlock,
    Blockquote,
    BulletList,
    OrderedList,
    Checkbox,
    Table,
    HorizontalRule,
    Empty,
    FootnoteDefinition,
    ImageMarker,
    BulletItem,
    NumberedItem,
    Callout {
        kind: u8,
    },
    /// Mermaid diagram fenced block. `diagram_type` matches
    /// `MermaidDiagramType::as_u8()` — 0 for Unknown, 1 Flowchart, …
    MermaidDiagram {
        diagram_type: u8,
    },
}

/// Inline type enum for FFI.
#[derive(Debug, Clone, PartialEq)]
pub enum FfiInlineType {
    Bold,
    Italic,
    BoldItalic,
    Strikethrough,
    UnderlineTilde,
    UnderlineHtml,
    InlineCode,
    Highlight,
    HighlightColor { color_index: u8 },
    HighlightHex { hex: String },
    Link { url: String },
    AutoLink { url: String },
    WikiLink,
    FootnoteRef,
    Comment,
    HexColor { hex: String },
}

/// An inline span for FFI transport.
#[derive(Debug, Clone, PartialEq)]
pub struct FfiInlineSpan {
    pub inline_type: FfiInlineType,
    pub utf16_start: u32,
    pub utf16_end: u32,
    pub content_utf16_start: u32,
    pub content_utf16_end: u32,
}

/// A list item for FFI transport.
#[derive(Debug, Clone, PartialEq)]
pub struct FfiListItem {
    pub text: String,
    pub inline_spans: Vec<FfiInlineSpan>,
}

/// A block for FFI transport.
#[derive(Debug, Clone, PartialEq)]
pub struct FfiBlock {
    pub block_type: FfiBlockType,
    pub line_start: u32,
    pub line_end: u32,
    pub utf16_start: u32,
    pub utf16_end: u32,
    pub list_indent: u32,
    pub marker_utf16_start: u32,
    pub marker_utf16_end: u32,
    pub marker_source: String,
    pub unordered_marker: String,
    pub ordered_delimiter: String,
    pub ordered_raw_number: String,
    pub heading_level: u8,
    pub number: u32,
    pub is_checked: bool,
    pub language: Option<String>,
    pub text: String,
    pub inline_spans: Vec<FfiInlineSpan>,
    pub list_items: Vec<FfiListItem>,
    pub table_headers: Vec<String>,
    pub table_rows: Vec<Vec<String>>,
    pub table_alignments: Vec<u8>,
}

/// Document-level statistics computed as a byproduct of parsing — zero extra cost.
/// Eliminates ~6 separate Swift string-scanning passes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FfiDocumentStats {
    pub word_count: u32,
    pub character_count: u32,
    pub character_count_no_spaces: u32,
    pub paragraph_count: u32,
    pub sentence_count: u32,
    pub checkbox_total: u32,
    pub checkbox_completed: u32,
    pub reading_time_seconds: u32,
    pub heading_count: u32,
    pub link_count: u32,
    pub code_block_count: u32,
    pub mermaid_diagram_count: u32,
}

/// A heading extracted from the document (level + text).
#[derive(Debug, Clone, PartialEq)]
pub struct FfiHeading {
    pub level: u8,
    pub text: String,
}

/// Parse result for FFI transport.
#[derive(Debug, Clone, PartialEq)]
pub struct FfiParseResult {
    pub blocks: Vec<FfiBlock>,
    pub line_count: u32,
    pub stats: FfiDocumentStats,
    /// Wiki link titles extracted from the document (deduped, code blocks excluded).
    pub wiki_links: Vec<String>,
    /// Headings extracted from the document in order.
    pub headings: Vec<FfiHeading>,
}

/// A single inline span in a rendered preview.
#[derive(Debug, Clone, PartialEq)]
pub struct FfiPreviewSpan {
    pub span_type: FfiInlineType,
    /// UTF-16 offset in the preview plain text.
    pub start: u32,
    /// UTF-16 end offset (exclusive) in the preview plain text.
    pub end: u32,
}

/// Rendered preview result: plain text with markdown syntax stripped,
/// plus inline span ranges for rich formatting.
#[derive(Debug, Clone, PartialEq)]
pub struct FfiRenderedPreview {
    pub plain_text: String,
    pub spans: Vec<FfiPreviewSpan>,
}

/// Combined parse + preview result. Single parse pass produces everything
/// needed for note save: AST, stats, wiki links, headings, and rich previews.
#[derive(Debug, Clone, PartialEq)]
pub struct FfiSaveParseResult {
    pub blocks: Vec<FfiBlock>,
    pub line_count: u32,
    pub stats: FfiDocumentStats,
    pub wiki_links: Vec<String>,
    pub headings: Vec<FfiHeading>,
    pub short_preview: FfiRenderedPreview,
    pub long_preview: FfiRenderedPreview,
}

/// Incremental parse result with dirty block range.
#[derive(Debug, Clone, PartialEq)]
pub struct FfiIncrementalResult {
    pub blocks: Vec<FfiBlock>,
    pub line_count: u32,
    /// Index of the first changed block (inclusive).
    pub dirty_start: u32,
    /// Index past the last changed block (exclusive).
    pub dirty_end: u32,
    pub stats: FfiDocumentStats,
}

/// Lightweight incremental result for the styling-only path.
/// Identical to `FfiIncrementalResult` but omits stats, avoiding the O(n)
/// grapheme iteration + byte scan that `compute_stats` performs on every call.
/// Used by the editor's 400ms restyle timer where stats aren't needed.
#[derive(Debug, Clone, PartialEq)]
pub struct FfiIncrementalStyleResult {
    pub blocks: Vec<FfiBlock>,
    pub line_count: u32,
    pub dirty_start: u32,
    pub dirty_end: u32,
}

// MARK: - Conversion from internal AST to FFI types

fn convert_inline_span(span: &InlineSpan) -> FfiInlineSpan {
    let inline_type = match &span.kind {
        InlineKind::Bold => FfiInlineType::Bold,
        InlineKind::Italic => FfiInlineType::Italic,
        InlineKind::BoldItalic => FfiInlineType::BoldItalic,
        InlineKind::Strikethrough => FfiInlineType::Strikethrough,
        InlineKind::UnderlineTilde => FfiInlineType::UnderlineTilde,
        InlineKind::UnderlineHtml => FfiInlineType::UnderlineHtml,
        InlineKind::InlineCode => FfiInlineType::InlineCode,
        InlineKind::Highlight => FfiInlineType::Highlight,
        InlineKind::HighlightColor(idx) => FfiInlineType::HighlightColor { color_index: *idx },
        InlineKind::HighlightHex { hex } => FfiInlineType::HighlightHex { hex: hex.clone() },
        InlineKind::Link { url } => FfiInlineType::Link { url: url.clone() },
        InlineKind::AutoLink { url } => FfiInlineType::AutoLink { url: url.clone() },
        InlineKind::WikiLink => FfiInlineType::WikiLink,
        InlineKind::FootnoteRef => FfiInlineType::FootnoteRef,
        InlineKind::Comment => FfiInlineType::Comment,
        InlineKind::HexColor { hex } => FfiInlineType::HexColor { hex: hex.clone() },
    };
    FfiInlineSpan {
        inline_type,
        utf16_start: span.utf16_start,
        utf16_end: span.utf16_end,
        content_utf16_start: span.content_utf16_start,
        content_utf16_end: span.content_utf16_end,
    }
}

fn convert_block(block: &BlockNode) -> FfiBlock {
    let (
        block_type,
        heading_level,
        number,
        is_checked,
        language,
        text,
        list_items,
        table_headers,
        table_rows,
        table_alignments,
    ) = match &block.kind {
        BlockKind::Heading { level, text } => (
            FfiBlockType::Heading,
            *level,
            0u32,
            false,
            None,
            text.clone(),
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        BlockKind::Paragraph { text } => (
            FfiBlockType::Paragraph,
            0,
            0,
            false,
            None,
            text.clone(),
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        BlockKind::CodeBlock { language, code } => (
            FfiBlockType::CodeBlock,
            0,
            0,
            false,
            language.clone(),
            code.clone(),
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        BlockKind::Blockquote { text } => (
            FfiBlockType::Blockquote,
            0,
            0,
            false,
            None,
            text.clone(),
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        BlockKind::BulletList { items } => (
            FfiBlockType::BulletList,
            0,
            0,
            false,
            None,
            String::new(),
            items
                .iter()
                .map(|item| FfiListItem {
                    text: item.text.clone(),
                    inline_spans: item.inline_spans.iter().map(convert_inline_span).collect(),
                })
                .collect(),
            vec![],
            vec![],
            vec![],
        ),
        BlockKind::OrderedList { start, items } => (
            FfiBlockType::OrderedList,
            0,
            *start,
            false,
            None,
            String::new(),
            items
                .iter()
                .map(|item| FfiListItem {
                    text: item.text.clone(),
                    inline_spans: item.inline_spans.iter().map(convert_inline_span).collect(),
                })
                .collect(),
            vec![],
            vec![],
            vec![],
        ),
        BlockKind::Checkbox { checked, text } => (
            FfiBlockType::Checkbox,
            0,
            0,
            *checked,
            None,
            text.clone(),
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        BlockKind::Table {
            headers,
            rows,
            alignments,
        } => (
            FfiBlockType::Table,
            0,
            0,
            false,
            None,
            String::new(),
            vec![],
            headers.clone(),
            rows.clone(),
            alignments
                .iter()
                .map(|a| match a {
                    ColumnAlignment::Default => 0u8,
                    ColumnAlignment::Left => 1,
                    ColumnAlignment::Center => 2,
                    ColumnAlignment::Right => 3,
                })
                .collect(),
        ),
        BlockKind::HorizontalRule => (
            FfiBlockType::HorizontalRule,
            0,
            0,
            false,
            None,
            String::new(),
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        BlockKind::Empty => (
            FfiBlockType::Empty,
            0,
            0,
            false,
            None,
            String::new(),
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        BlockKind::FootnoteDefinition { label, text } => (
            FfiBlockType::FootnoteDefinition,
            0,
            0,
            false,
            None,
            format!("{}: {}", label, text),
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        BlockKind::ImageMarker { uuid } => (
            FfiBlockType::ImageMarker,
            0,
            0,
            false,
            None,
            uuid.clone(),
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        BlockKind::BulletItem { text } => (
            FfiBlockType::BulletItem,
            0,
            0,
            false,
            None,
            text.clone(),
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        BlockKind::NumberedItem { number, text } => (
            FfiBlockType::NumberedItem,
            0,
            *number,
            false,
            None,
            text.clone(),
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        BlockKind::Callout { kind, title, text } => (
            FfiBlockType::Callout { kind: kind.as_u8() },
            0,
            0,
            false,
            title.clone(), // title shares the `language` field to avoid a dedicated FFI field
            text.clone(),
            vec![],
            vec![],
            vec![],
            vec![],
        ),
        BlockKind::MermaidDiagram {
            diagram_type,
            source,
        } => (
            FfiBlockType::MermaidDiagram {
                diagram_type: diagram_type.as_u8(),
            },
            0,
            0,
            false,
            // Populate `language` with the canonical `mermaid` info string so
            // existing code paths that inspect `block.language` (tile chip
            // labels, highlight routing) keep working. The `diagram_type`
            // carries the actual classification.
            Some("mermaid".to_string()),
            source.clone(),
            vec![],
            vec![],
            vec![],
            vec![],
        ),
    };

    FfiBlock {
        block_type,
        line_start: block.line_start,
        line_end: block.line_end,
        utf16_start: block.utf16_start,
        utf16_end: block.utf16_end,
        list_indent: block.list_marker.as_ref().map(|m| m.indent).unwrap_or(0),
        marker_utf16_start: block
            .list_marker
            .as_ref()
            .map(|m| m.marker_utf16_start)
            .unwrap_or(0),
        marker_utf16_end: block
            .list_marker
            .as_ref()
            .map(|m| m.marker_utf16_end)
            .unwrap_or(0),
        marker_source: block
            .list_marker
            .as_ref()
            .map(|m| m.marker_source.clone())
            .unwrap_or_default(),
        unordered_marker: block
            .list_marker
            .as_ref()
            .map(|m| m.unordered_marker.clone())
            .unwrap_or_default(),
        ordered_delimiter: block
            .list_marker
            .as_ref()
            .map(|m| m.ordered_delimiter.clone())
            .unwrap_or_default(),
        ordered_raw_number: block
            .list_marker
            .as_ref()
            .map(|m| m.ordered_raw_number.clone())
            .unwrap_or_default(),
        heading_level,
        number,
        is_checked,
        language,
        text,
        inline_spans: block.inline_spans.iter().map(convert_inline_span).collect(),
        list_items,
        table_headers,
        table_rows,
        table_alignments,
    }
}

/// Compute document statistics from the source text and parsed AST.
/// Character counts use extended grapheme clusters (matching Swift's `String.count`)
/// so emoji like 👨‍👩‍👧‍👦 count as 1 character, not 7 code points.
fn compute_stats(source: &str, blocks: &[BlockNode]) -> FfiDocumentStats {
    use unicode_segmentation::UnicodeSegmentation;

    let bytes = source.as_bytes();

    // Character counts use grapheme clusters to match Swift's String.count
    let mut character_count: u32 = 0;
    let mut character_count_no_spaces: u32 = 0;
    for grapheme in source.graphemes(true) {
        character_count += 1;
        // A grapheme is whitespace if it's a single whitespace char
        let is_ws = grapheme.len() == 1 && grapheme.as_bytes()[0].is_ascii_whitespace();
        if !is_ws {
            character_count_no_spaces += 1;
        }
    }

    // Word count + sentence detection via byte scan (fast, ASCII-safe)
    let mut word_count: u32 = 0;
    let mut sentence_count: u32 = 0;
    let mut in_word = false;
    let mut prev_is_terminator = false;

    for &b in bytes {
        let is_whitespace = b == b' ' || b == b'\t' || b == b'\n' || b == b'\r';
        if is_whitespace {
            if in_word {
                word_count += 1;
                in_word = false;
            }
        } else {
            // Only start a word on non-continuation bytes to avoid counting
            // multi-byte UTF-8 sequences as multiple word starts
            if b & 0xC0 != 0x80 || !in_word {
                in_word = true;
            }
        }

        // Sentence detection: count runs of terminators (.!?) as one sentence ending.
        // E.g., "..." = 1 sentence, "Wait... really?" = 2 sentences.
        let is_term = b == b'.' || b == b'!' || b == b'?';
        if is_term {
            prev_is_terminator = true;
        } else if prev_is_terminator {
            sentence_count += 1;
            prev_is_terminator = false;
        }
    }
    // Count final word if text doesn't end with whitespace
    if in_word {
        word_count += 1;
    }
    // Handle text ending with a terminator
    if prev_is_terminator {
        sentence_count += 1;
    }

    // Block-level stats from AST (already parsed, just iterate)
    let mut paragraph_count: u32 = 0;
    let mut checkbox_total: u32 = 0;
    let mut checkbox_completed: u32 = 0;
    let mut heading_count: u32 = 0;
    let mut link_count: u32 = 0;
    let mut code_block_count: u32 = 0;
    let mut mermaid_diagram_count: u32 = 0;

    for block in blocks {
        match &block.kind {
            BlockKind::Paragraph { .. } | BlockKind::Blockquote { .. } => {
                paragraph_count += 1;
            }
            BlockKind::Heading { .. } => {
                heading_count += 1;
            }
            BlockKind::Checkbox { checked, .. } => {
                checkbox_total += 1;
                if *checked {
                    checkbox_completed += 1;
                }
            }
            BlockKind::CodeBlock { .. } => {
                code_block_count += 1;
            }
            BlockKind::MermaidDiagram { .. } => {
                mermaid_diagram_count += 1;
            }
            _ => {}
        }
        // Count links from inline spans
        for span in &block.inline_spans {
            if matches!(
                span.kind,
                InlineKind::Link { .. } | InlineKind::AutoLink { .. } | InlineKind::WikiLink
            ) {
                link_count += 1;
            }
        }
    }

    let reading_time_seconds = if word_count > 0 {
        // 200 wpm average reading speed
        (word_count as f64 / 200.0 * 60.0).ceil() as u32
    } else {
        0
    };

    FfiDocumentStats {
        word_count,
        character_count,
        character_count_no_spaces,
        paragraph_count,
        sentence_count,
        checkbox_total,
        checkbox_completed,
        reading_time_seconds,
        heading_count,
        link_count,
        code_block_count,
        mermaid_diagram_count,
    }
}

/// Extract wiki link titles from the parsed AST (deduped, preserving first occurrence order).
/// Skips wiki links inside code blocks.
fn extract_wiki_links_from_doc(doc: &Document, source: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    // Pre-compute UTF-16 encoding once for the entire document
    let utf16_chars: Vec<u16> = source.encode_utf16().collect();

    let extract_title = |span: &InlineSpan,
                         seen: &mut std::collections::HashSet<String>,
                         result: &mut Vec<String>| {
        if !matches!(span.kind, InlineKind::WikiLink) {
            return;
        }
        // Target is the raw body between `[[` and `]]`, before any `|` alias.
        // The span's content range may already point at the alias, so we re-read
        // from the full wiki range to recover the target unambiguously.
        let body_start = span.utf16_start as usize + 2;
        let body_end = span.utf16_end.saturating_sub(2) as usize;
        if body_end <= utf16_chars.len() && body_start <= body_end {
            let body = String::from_utf16_lossy(&utf16_chars[body_start..body_end]);
            let target = body.split('|').next().unwrap_or(&body).trim().to_string();
            if !target.is_empty() && seen.insert(target.clone()) {
                result.push(target);
            }
        }
    };

    // Extract [[title]] from raw cell text (tables skip inline parsing)
    let extract_from_text =
        |text: &str, seen: &mut std::collections::HashSet<String>, result: &mut Vec<String>| {
            let bytes = text.as_bytes();
            let mut i = 0;
            while i + 1 < bytes.len() {
                if bytes[i] == b'[' && bytes[i + 1] == b'[' {
                    let start = i + 2;
                    if let Some(end) = text[start..].find("]]") {
                        let body = &text[start..start + end];
                        let target = body.split('|').next().unwrap_or(body).trim().to_string();
                        if !target.is_empty() && seen.insert(target.clone()) {
                            result.push(target);
                        }
                        i = start + end + 2;
                        continue;
                    }
                }
                i += 1;
            }
        };

    for block in &doc.blocks {
        // Skip code blocks and mermaid diagrams — wiki-link-looking text
        // in either is source content, not a real link.
        if matches!(
            block.kind,
            BlockKind::CodeBlock { .. } | BlockKind::MermaidDiagram { .. }
        ) {
            continue;
        }

        // Check inline spans for wiki links
        for span in &block.inline_spans {
            extract_title(span, &mut seen, &mut result);
        }

        // Also check list items' inline spans (grouped mode)
        // and table cells (which don't get inline spans from the parser)
        match &block.kind {
            BlockKind::BulletList { items } | BlockKind::OrderedList { items, .. } => {
                for item in items {
                    for span in &item.inline_spans {
                        extract_title(span, &mut seen, &mut result);
                    }
                }
            }
            BlockKind::Table { headers, rows, .. } => {
                for cell in headers {
                    extract_from_text(cell, &mut seen, &mut result);
                }
                for row in rows {
                    for cell in row {
                        extract_from_text(cell, &mut seen, &mut result);
                    }
                }
            }
            _ => {}
        }
    }

    result
}

/// Extract headings from the parsed AST in document order.
fn extract_headings_from_doc(doc: &Document) -> Vec<FfiHeading> {
    doc.blocks
        .iter()
        .filter_map(|block| {
            if let BlockKind::Heading { level, text } = &block.kind {
                Some(FfiHeading {
                    level: *level,
                    text: text.clone(),
                })
            } else {
                None
            }
        })
        .collect()
}

fn convert_document(doc: &Document, source: &str) -> FfiParseResult {
    let stats = compute_stats(source, &doc.blocks);
    let wiki_links = extract_wiki_links_from_doc(doc, source);
    let headings = extract_headings_from_doc(doc);
    FfiParseResult {
        blocks: doc.blocks.iter().map(convert_block).collect(),
        line_count: doc.line_count,
        stats,
        wiki_links,
        headings,
    }
}

// MARK: - CindermarkParser (UniFFI-exported object)

/// Validate an image-marker scheme received across the FFI boundary.
/// Returns `None` (extension off) for anything that could not work as a
/// literal `![](<scheme><UUID>)` prefix, instead of panicking.
fn sanitize_scheme(scheme: Option<String>) -> Option<String> {
    let s = scheme?;
    if s.is_empty()
        || !s.is_ascii()
        || s.bytes()
            .any(|b| b == b'(' || b == b')' || b.is_ascii_whitespace())
    {
        return None;
    }
    Some(s)
}

/// The main parser object exposed to Swift via UniFFI.
///
/// Thread-safe: uses a Mutex internally. Each editor instance should
/// create its own `CindermarkParser` for incremental update support.
pub struct CindermarkParser {
    state: Mutex<Option<incremental::ParseSnapshot>>,
    options: parser::ParseOptions,
}

impl CindermarkParser {
    /// Create a parser.
    ///
    /// `image_marker_scheme` opts in to the block-level image / attachment
    /// marker extension: `![](<scheme><UUID>)` on a line by itself parses as
    /// an `ImageMarker` block instead of a paragraph. The scheme is the
    /// literal prefix including any trailing colon (Ember Notes passes
    /// `"ember:"`). Pass `None` for CommonMark-clean default behavior.
    ///
    /// Invalid schemes (empty, non-ASCII, or containing whitespace,
    /// parentheses, or newlines) are treated as `None` rather than
    /// panicking — this constructor is called across the FFI boundary.
    pub fn new(image_marker_scheme: Option<String>) -> Self {
        Self {
            state: Mutex::new(None),
            options: parser::ParseOptions {
                image_marker_scheme: sanitize_scheme(image_marker_scheme),
            },
        }
    }

    /// Full parse in grouped mode (for rendering). Does not affect incremental state.
    pub fn parse(&self, text: String) -> FfiParseResult {
        let doc = parser::parse_with_options(&text, ParseMode::Grouped, &self.options);
        convert_document(&doc, &text)
    }

    /// Full parse in editable mode (for block editor).
    /// Stores a snapshot for subsequent incremental updates.
    pub fn parse_editable(&self, text: String) -> FfiParseResult {
        let doc = parser::parse_with_options(&text, ParseMode::Editable, &self.options);
        let utf16_map = utf16::Utf16Map::build(text.as_bytes());

        let result = convert_document(&doc, &text);

        // Store snapshot for future incremental updates. The conversion
        // above only borrowed the AST, so the block list is moved into the
        // snapshot instead of cloned.
        let mut state = self.state.lock().unwrap();
        *state = Some(incremental::ParseSnapshot::new(
            doc.blocks,
            doc.line_count,
            text.len(),
            utf16_map.total_utf16_len,
        ));

        result
    }

    /// Incremental parse after a text edit. Re-parses only the dirty blocks.
    ///
    /// Falls back to a full re-parse when:
    /// - No previous snapshot exists (first call should use `parse_editable`)
    /// - The edit involves code fences (unbounded reach)
    ///
    /// Returns the full block list plus dirty range indices so Swift can
    /// restyle only the changed region.
    pub fn parse_editable_incremental(
        &self,
        text: String,
        edit_utf16_start: u32,
        edit_old_utf16_len: u32,
        edit_new_utf16_len: u32,
    ) -> FfiIncrementalResult {
        let mut state = self.state.lock().unwrap();

        let result = if let Some(prev) = state.as_ref() {
            incremental::incremental_update(
                prev,
                &text,
                edit_utf16_start,
                edit_old_utf16_len,
                edit_new_utf16_len,
                &self.options,
            )
        } else {
            // No previous state — do full parse.
            let doc = parser::parse_with_options(&text, ParseMode::Editable, &self.options);
            let block_count = doc.blocks.len() as u32;
            let utf16_map = utf16::Utf16Map::build(text.as_bytes());
            incremental::IncrementalResult {
                blocks: doc.blocks,
                line_count: doc.line_count,
                dirty_start: 0,
                dirty_end: block_count,
                total_utf16_len: utf16_map.total_utf16_len,
            }
        };

        // Stats and FFI conversion only borrow the block list...
        let stats = compute_stats(&text, &result.blocks);
        let blocks: Vec<FfiBlock> = result.blocks.iter().map(convert_block).collect();

        // ...so it is moved (not cloned) into the snapshot for the next
        // incremental call. This path runs per keystroke; the two clones it
        // used to make were full O(document) passes.
        *state = Some(incremental::ParseSnapshot::new(
            result.blocks,
            result.line_count,
            text.len(),
            result.total_utf16_len,
        ));

        FfiIncrementalResult {
            blocks,
            line_count: result.line_count,
            dirty_start: result.dirty_start,
            dirty_end: result.dirty_end,
            stats,
        }
    }

    /// Style-only incremental parse: identical to `parse_editable_incremental`
    /// but skips the O(n) `compute_stats` call and the redundant `Utf16Map::build`.
    /// Used by the editor's 400ms restyle timer where only blocks + dirty range
    /// are needed. Stats are computed separately on the content-sync timer.
    pub fn parse_editable_incremental_style_only(
        &self,
        text: String,
        edit_utf16_start: u32,
        edit_old_utf16_len: u32,
        edit_new_utf16_len: u32,
    ) -> FfiIncrementalStyleResult {
        let mut state = self.state.lock().unwrap();

        let result = if let Some(prev) = state.as_ref() {
            incremental::incremental_update(
                prev,
                &text,
                edit_utf16_start,
                edit_old_utf16_len,
                edit_new_utf16_len,
                &self.options,
            )
        } else {
            // No previous state — do full parse.
            let doc = parser::parse_with_options(&text, ParseMode::Editable, &self.options);
            let block_count = doc.blocks.len() as u32;
            let utf16_map = utf16::Utf16Map::build(text.as_bytes());
            incremental::IncrementalResult {
                blocks: doc.blocks,
                line_count: doc.line_count,
                dirty_start: 0,
                dirty_end: block_count,
                total_utf16_len: utf16_map.total_utf16_len,
            }
        };

        // FFI conversion only borrows the block list...
        let blocks: Vec<FfiBlock> = result.blocks.iter().map(convert_block).collect();

        // ...so it is moved (not cloned) into the snapshot. Uses
        // total_utf16_len from the result — no extra Utf16Map::build.
        *state = Some(incremental::ParseSnapshot::new(
            result.blocks,
            result.line_count,
            text.len(),
            result.total_utf16_len,
        ));

        // No compute_stats — that's the whole point of this variant.
        FfiIncrementalStyleResult {
            blocks,
            line_count: result.line_count,
            dirty_start: result.dirty_start,
            dirty_end: result.dirty_end,
        }
    }

    /// Reset incremental state. Call when switching notes.
    pub fn reset_state(&self) {
        let mut state = self.state.lock().unwrap();
        *state = None;
    }

    /// Extract wiki link titles from content (skipping code blocks).
    pub fn extract_wiki_links(&self, text: String) -> Vec<String> {
        parser::extract_wiki_links(&text)
    }

    /// Toggle checkbox at a line index, returning new content.
    pub fn toggle_checkbox(&self, text: String, line_index: u32) -> String {
        parser::toggle_checkbox(&text, line_index)
    }

    /// Render a rich preview for note list display.
    ///
    /// Strips all markdown syntax (block markers, inline formatting markers) and
    /// returns plain text plus inline span ranges for rich AttributedString rendering.
    /// Checkboxes become ✓ (checked) / ☐ (unchecked).
    ///
    /// `max_chars` limits the preview length in UTF-16 code units.
    pub fn render_preview(&self, text: String, max_chars: u32) -> FfiRenderedPreview {
        render_preview_impl(&text, max_chars, &self.options)
    }

    /// Render two previews (short + long) from a single parse pass.
    /// Avoids double-parsing the same document when both card and peek previews are needed.
    pub fn render_previews(
        &self,
        text: String,
        short_max: u32,
        long_max: u32,
    ) -> Vec<FfiRenderedPreview> {
        render_previews_impl(&text, short_max, long_max, &self.options)
    }

    /// Combined parse + preview in a single pass. Returns everything needed for note save:
    /// blocks, stats, wiki links, headings, and two rich previews (short + long).
    ///
    /// Eliminates 3 separate parse calls that `performSave` previously required.
    pub fn parse_for_save(
        &self,
        text: String,
        short_preview_max: u32,
        long_preview_max: u32,
    ) -> FfiSaveParseResult {
        let doc = parser::parse_with_options(&text, ParseMode::Grouped, &self.options);
        let stats = compute_stats(&text, &doc.blocks);
        let wiki_links = extract_wiki_links_from_doc(&doc, &text);
        let headings = extract_headings_from_doc(&doc);
        let blocks = doc.blocks.iter().map(convert_block).collect();
        let line_count = doc.line_count;

        // Generate previews from the same AST — no re-parse
        let limit = std::cmp::max(short_preview_max, long_preview_max) as usize;
        let (short_preview, long_preview) = match build_clean_preview_from_doc(&doc, limit) {
            Some(preview) => {
                let short = truncate_preview(&preview, short_preview_max);
                let long = truncate_preview(&preview, long_preview_max);
                (short, long)
            }
            None => {
                let empty = FfiRenderedPreview {
                    plain_text: String::new(),
                    spans: Vec::new(),
                };
                (empty.clone(), empty)
            }
        };

        FfiSaveParseResult {
            blocks,
            line_count,
            stats,
            wiki_links,
            headings,
            short_preview,
            long_preview,
        }
    }
}

/// Convert internal InlineKind to FFI InlineType.
fn convert_inline_kind(kind: &InlineKind) -> FfiInlineType {
    match kind {
        InlineKind::Bold => FfiInlineType::Bold,
        InlineKind::Italic => FfiInlineType::Italic,
        InlineKind::BoldItalic => FfiInlineType::BoldItalic,
        InlineKind::Strikethrough => FfiInlineType::Strikethrough,
        InlineKind::UnderlineTilde => FfiInlineType::UnderlineTilde,
        InlineKind::UnderlineHtml => FfiInlineType::UnderlineHtml,
        InlineKind::InlineCode => FfiInlineType::InlineCode,
        InlineKind::Highlight => FfiInlineType::Highlight,
        InlineKind::HighlightColor(idx) => FfiInlineType::HighlightColor { color_index: *idx },
        InlineKind::HighlightHex { hex } => FfiInlineType::HighlightHex { hex: hex.clone() },
        InlineKind::Link { url } => FfiInlineType::Link { url: url.clone() },
        InlineKind::AutoLink { url } => FfiInlineType::AutoLink { url: url.clone() },
        InlineKind::WikiLink => FfiInlineType::WikiLink,
        InlineKind::FootnoteRef => FfiInlineType::FootnoteRef,
        InlineKind::Comment => FfiInlineType::Comment,
        InlineKind::HexColor { hex } => FfiInlineType::HexColor { hex: hex.clone() },
    }
}

/// Intermediate result from parsing and stripping markdown — shared across multiple truncation levels.
struct CleanPreview {
    clean_text: String,
    clean_utf16_len: u32,
    clean_spans: Vec<FfiPreviewSpan>,
}

/// Build a rich preview from markdown text (the expensive shared work):
///
/// 1. Parse the document to get block structure
/// 2. Build raw preview text from block contents (block markers stripped, inline markers kept)
/// 3. Re-parse inline spans on the preview text
/// 4. Strip inline markers and remap span positions
///
/// Truncation to specific lengths is cheap on top of this result.
fn build_clean_preview(
    text: &str,
    approx_limit: usize,
    options: &parser::ParseOptions,
) -> Option<CleanPreview> {
    let doc = parser::parse_with_options(text, ParseMode::Grouped, options);
    build_clean_preview_from_doc(&doc, approx_limit)
}

fn build_clean_preview_from_doc(doc: &Document, approx_limit: usize) -> Option<CleanPreview> {
    // Step 1: Build raw preview from block texts (inline markers kept, block markers stripped)
    let mut raw_parts: Vec<String> = Vec::new();
    let mut approx_len: usize = 0;

    for block in &doc.blocks {
        if approx_len >= approx_limit {
            break;
        }

        match &block.kind {
            BlockKind::Heading { text, .. }
            | BlockKind::Paragraph { text }
            | BlockKind::Blockquote { text } => {
                if !text.is_empty() {
                    raw_parts.push(text.clone());
                    approx_len += text.len();
                }
            }
            BlockKind::Checkbox { checked, text } => {
                let prefix = if *checked { "✓ " } else { "☐ " };
                raw_parts.push(format!("{}{}", prefix, text));
                approx_len += text.len() + 2;
            }
            BlockKind::BulletList { items } => {
                for item in items {
                    if approx_len >= approx_limit {
                        break;
                    }
                    if !item.text.is_empty() {
                        raw_parts.push(item.text.clone());
                        approx_len += item.text.len();
                    }
                }
            }
            BlockKind::OrderedList { items, .. } => {
                for item in items {
                    if approx_len >= approx_limit {
                        break;
                    }
                    if !item.text.is_empty() {
                        raw_parts.push(item.text.clone());
                        approx_len += item.text.len();
                    }
                }
            }
            BlockKind::BulletItem { text } | BlockKind::NumberedItem { text, .. }
                if !text.is_empty() =>
            {
                raw_parts.push(text.clone());
                approx_len += text.len();
            }
            BlockKind::Callout { text, .. } if !text.is_empty() => {
                raw_parts.push(text.clone());
                approx_len += text.len();
            }
            // Skip code blocks, tables, rules, empty lines, image markers,
            // and footnote definitions
            _ => {}
        }
    }

    if raw_parts.is_empty() {
        return None;
    }

    // Step 2: Join parts with newline separators so blocks appear on separate lines in previews
    let raw_preview = raw_parts.join("\n");

    // Step 3: Parse inline spans on the raw preview text
    let preview_bytes = raw_preview.as_bytes();
    let preview_utf16_map = utf16::Utf16Map::build(preview_bytes);
    let inline_spans = inline::parse_spans(preview_bytes, 0, preview_bytes, &preview_utf16_map);

    // Step 4: Strip inline markers
    let preview_utf16_len = preview_utf16_map.total_utf16_len as usize;
    let mut is_marker = vec![false; preview_utf16_len];

    for span in &inline_spans {
        // Hidden comments are stripped in their entirety from preview — mark
        // the full span so neither the `%%` fences nor the body surface.
        if matches!(span.kind, InlineKind::Comment) {
            for i in span.utf16_start..span.utf16_end {
                if (i as usize) < is_marker.len() {
                    is_marker[i as usize] = true;
                }
            }
            continue;
        }
        for i in span.utf16_start..span.content_utf16_start {
            if (i as usize) < is_marker.len() {
                is_marker[i as usize] = true;
            }
        }
        for i in span.content_utf16_end..span.utf16_end {
            if (i as usize) < is_marker.len() {
                is_marker[i as usize] = true;
            }
        }
    }

    // Build clean text with position mapping
    let mut clean_text = String::new();
    let mut clean_utf16_pos: u32 = 0;
    let mut input_utf16_pos: u32 = 0;
    let mut position_map = vec![0u32; preview_utf16_len + 1];

    for ch in raw_preview.chars() {
        let ch_utf16_len = ch.len_utf16() as u32;
        let idx = input_utf16_pos as usize;
        let is_marked = idx < is_marker.len() && is_marker[idx];

        for i in 0..ch_utf16_len {
            let map_idx = (input_utf16_pos + i) as usize;
            if map_idx < position_map.len() {
                position_map[map_idx] = clean_utf16_pos;
            }
        }

        if !is_marked {
            clean_text.push(ch);
            clean_utf16_pos += ch_utf16_len;
        }

        input_utf16_pos += ch_utf16_len;
    }
    let end_idx = input_utf16_pos as usize;
    if end_idx < position_map.len() {
        position_map[end_idx] = clean_utf16_pos;
    }

    // Step 5: Remap inline spans to clean text positions
    let mut clean_spans: Vec<FfiPreviewSpan> = Vec::new();
    for span in &inline_spans {
        let cs = span.content_utf16_start as usize;
        let ce = span.content_utf16_end as usize;

        if cs < position_map.len() && ce <= position_map.len() {
            let mapped_start = position_map[cs];
            let mapped_end = if ce < position_map.len() {
                position_map[ce]
            } else {
                clean_utf16_pos
            };

            if mapped_start < mapped_end {
                clean_spans.push(FfiPreviewSpan {
                    span_type: convert_inline_kind(&span.kind),
                    start: mapped_start,
                    end: mapped_end,
                });
            }
        }
    }

    Some(CleanPreview {
        clean_text,
        clean_utf16_len: clean_utf16_pos,
        clean_spans,
    })
}

/// Truncate a clean preview to a specific UTF-16 character limit.
fn truncate_preview(preview: &CleanPreview, max_chars: u32) -> FfiRenderedPreview {
    if preview.clean_utf16_len <= max_chars {
        return FfiRenderedPreview {
            plain_text: preview.clean_text.clone(),
            spans: preview.clean_spans.clone(),
        };
    }

    let mut count: u32 = 0;
    let mut byte_end = 0;
    for (i, ch) in preview.clean_text.char_indices() {
        count += ch.len_utf16() as u32;
        if count > max_chars {
            break;
        }
        byte_end = i + ch.len_utf8();
    }
    let truncated_text = preview.clean_text[..byte_end].to_string();

    let mut truncated_spans: Vec<FfiPreviewSpan> = preview
        .clean_spans
        .iter()
        .filter(|s| s.start < max_chars)
        .cloned()
        .collect();
    for span in &mut truncated_spans {
        if span.end > max_chars {
            span.end = max_chars;
        }
    }

    FfiRenderedPreview {
        plain_text: truncated_text,
        spans: truncated_spans,
    }
}

fn render_preview_impl(
    text: &str,
    max_chars: u32,
    options: &parser::ParseOptions,
) -> FfiRenderedPreview {
    if text.is_empty() {
        return FfiRenderedPreview {
            plain_text: String::new(),
            spans: Vec::new(),
        };
    }
    match build_clean_preview(text, max_chars as usize, options) {
        Some(preview) => truncate_preview(&preview, max_chars),
        None => FfiRenderedPreview {
            plain_text: String::new(),
            spans: Vec::new(),
        },
    }
}

/// Render two previews (short + long) from a single parse pass.
fn render_previews_impl(
    text: &str,
    short_max: u32,
    long_max: u32,
    options: &parser::ParseOptions,
) -> Vec<FfiRenderedPreview> {
    if text.is_empty() {
        let empty = FfiRenderedPreview {
            plain_text: String::new(),
            spans: Vec::new(),
        };
        return vec![empty.clone(), empty];
    }
    // Use the larger limit for the shared parse so we collect enough content for both.
    let limit = std::cmp::max(short_max, long_max) as usize;
    match build_clean_preview(text, limit, options) {
        Some(preview) => {
            let short = truncate_preview(&preview, short_max);
            let long = truncate_preview(&preview, long_max);
            vec![short, long]
        }
        None => {
            let empty = FfiRenderedPreview {
                plain_text: String::new(),
                spans: Vec::new(),
            };
            vec![empty.clone(), empty]
        }
    }
}

impl Default for CindermarkParser {
    fn default() -> Self {
        Self::new(None)
    }
}

// UniFFI scaffolding
uniffi::include_scaffolding!("cindermark");

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - Image marker scheme (constructor option)

    const MARKER: &str = "![](ember:DEBD1746-CBBB-4A33-9CB0-4B1A5D956200)";

    #[test]
    fn scheme_constructor_enables_image_markers() {
        let parser = CindermarkParser::new(Some("ember:".to_string()));
        let result = parser.parse_editable(format!("{}\n", MARKER));
        assert!(matches!(
            result.blocks[0].block_type,
            FfiBlockType::ImageMarker
        ));
    }

    #[test]
    fn no_scheme_leaves_marker_lines_as_paragraphs() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse_editable(format!("{}\n", MARKER));
        assert!(matches!(
            result.blocks[0].block_type,
            FfiBlockType::Paragraph
        ));
    }

    #[test]
    fn invalid_scheme_is_sanitized_to_none() {
        for bad in ["", "has space:", "paren):", "émber:", "new\nline:"] {
            let parser = CindermarkParser::new(Some(bad.to_string()));
            let result = parser.parse_editable(format!("{}\n", MARKER));
            assert!(
                matches!(result.blocks[0].block_type, FfiBlockType::Paragraph),
                "scheme {:?} should be rejected, not panic or match",
                bad
            );
        }
    }

    #[test]
    fn incremental_parse_respects_scheme() {
        // Options must flow into the incremental engine: an edit elsewhere
        // in the document must not demote the marker block to a paragraph.
        let parser = CindermarkParser::new(Some("ember:".to_string()));
        let base = format!("# Title\n\n{}\n\nSome paragraph\n", MARKER);
        parser.parse_editable(base.clone());

        let edited = base.replace("Some paragraph", "Some paragraphX");
        let edit_start = base.find("Some paragraph").unwrap() as u32 + 14;
        let result = parser.parse_editable_incremental_style_only(edited, edit_start, 0, 1);

        assert!(
            result
                .blocks
                .iter()
                .any(|b| matches!(b.block_type, FfiBlockType::ImageMarker)),
            "image marker must survive incremental updates"
        );
    }

    #[test]
    fn stats_word_and_sentence_counts() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse("Hello world. This is a test!".to_string());
        assert_eq!(result.stats.word_count, 6);
        assert_eq!(result.stats.sentence_count, 2);
        assert_eq!(result.stats.character_count, 28);
        assert_eq!(result.stats.paragraph_count, 1);
    }

    #[test]
    fn stats_checkboxes() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse_editable("- [x] Done\n- [ ] Todo\n- [X] Also done".to_string());
        assert_eq!(result.stats.checkbox_total, 3);
        assert_eq!(result.stats.checkbox_completed, 2);
    }

    #[test]
    fn stats_headings_and_code_blocks() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse("# Title\n\n## Subtitle\n\n```rust\nlet x = 1;\n```".to_string());
        assert_eq!(result.stats.heading_count, 2);
        assert_eq!(result.stats.code_block_count, 1);
    }

    #[test]
    fn stats_ellipsis_not_multiple_sentences() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse("Wait... really?".to_string());
        // "..." should count as 1 sentence ending, "?" as another = 2
        assert_eq!(result.stats.sentence_count, 2);
    }

    #[test]
    fn stats_grapheme_clusters_match_swift() {
        let parser = CindermarkParser::new(None);
        // Family emoji: 1 grapheme cluster (matches Swift String.count)
        let result = parser.parse("👨‍👩‍👧‍👦".to_string());
        assert_eq!(result.stats.character_count, 1);
        assert_eq!(result.stats.character_count_no_spaces, 1);

        // Skin tone modifier: 1 grapheme cluster
        let result2 = parser.parse("👨🏻".to_string());
        assert_eq!(result2.stats.character_count, 1);

        // Combining diacritical: "Café" = 4 grapheme clusters
        let result3 = parser.parse("Cafe\u{0301}".to_string());
        assert_eq!(result3.stats.character_count, 4);

        // Flag emoji (regional indicators): 1 grapheme cluster
        let result4 = parser.parse("🇺🇸".to_string());
        assert_eq!(result4.stats.character_count, 1);
    }

    #[test]
    fn stats_empty_document() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse(String::new());
        assert_eq!(result.stats.word_count, 0);
        assert_eq!(result.stats.sentence_count, 0);
        assert_eq!(result.stats.character_count, 0);
        assert_eq!(result.stats.reading_time_seconds, 0);
    }

    #[test]
    fn stats_reading_time() {
        let parser = CindermarkParser::new(None);
        // 200 words → 60 seconds
        let words: String = (0..200)
            .map(|i| format!("word{}", i))
            .collect::<Vec<_>>()
            .join(" ");
        let result = parser.parse(words);
        assert_eq!(result.stats.reading_time_seconds, 60);
    }

    #[test]
    fn ffi_parse_basic() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse("# Hello\n\nWorld".to_string());
        assert_eq!(result.blocks.len(), 3); // heading, empty, paragraph
        assert_eq!(result.blocks[0].block_type, FfiBlockType::Heading);
        assert_eq!(result.blocks[0].heading_level, 1);
        assert_eq!(result.blocks[0].text, "Hello");
    }

    #[test]
    fn ffi_parse_editable() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse_editable("- item 1\n- item 2".to_string());
        assert_eq!(result.blocks.len(), 2);
        assert_eq!(result.blocks[0].block_type, FfiBlockType::BulletItem);
        assert_eq!(result.blocks[1].block_type, FfiBlockType::BulletItem);
    }

    #[test]
    fn ffi_extract_wiki_links() {
        let parser = CindermarkParser::new(None);
        let links = parser.extract_wiki_links("see [[Note One]] and [[Note Two]]".to_string());
        assert_eq!(links, vec!["Note One", "Note Two"]);
    }

    #[test]
    fn ffi_toggle_checkbox() {
        let parser = CindermarkParser::new(None);
        let result = parser.toggle_checkbox("- [ ] task".to_string(), 0);
        assert_eq!(result, "- [x] task");
    }

    #[test]
    fn ffi_inline_spans_present() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse("**bold** text".to_string());
        assert!(!result.blocks[0].inline_spans.is_empty());
        assert_eq!(
            result.blocks[0].inline_spans[0].inline_type,
            FfiInlineType::Bold
        );
    }

    #[test]
    fn preview_strips_heading_markers() {
        let parser = CindermarkParser::new(None);
        let result = parser.render_preview("# Hello World".to_string(), 200);
        assert_eq!(result.plain_text, "Hello World");
        assert!(result.spans.is_empty());
    }

    #[test]
    fn preview_strips_bold_markers() {
        let parser = CindermarkParser::new(None);
        let result = parser.render_preview("**bold** text".to_string(), 200);
        assert_eq!(result.plain_text, "bold text");
        assert_eq!(result.spans.len(), 1);
        assert_eq!(result.spans[0].span_type, FfiInlineType::Bold);
        assert_eq!(result.spans[0].start, 0);
        assert_eq!(result.spans[0].end, 4);
    }

    #[test]
    fn preview_checkbox_rendering() {
        let parser = CindermarkParser::new(None);
        let result = parser.render_preview("- [x] Done\n- [ ] Todo".to_string(), 200);
        assert!(result.plain_text.starts_with("✓ Done"));
        assert!(result.plain_text.contains("☐ Todo"));
    }

    #[test]
    fn preview_multiple_blocks() {
        let parser = CindermarkParser::new(None);
        let result = parser.render_preview("# Title\n\nSome **bold** paragraph".to_string(), 200);
        assert_eq!(result.plain_text, "Title\nSome bold paragraph");
        assert_eq!(result.spans.len(), 1);
        assert_eq!(result.spans[0].span_type, FfiInlineType::Bold);
        // "Title\nSome " = 11 chars, then "bold" at positions 11-14
        assert_eq!(result.spans[0].start, 11);
        assert_eq!(result.spans[0].end, 15);
    }

    #[test]
    fn preview_truncation() {
        let parser = CindermarkParser::new(None);
        let result = parser.render_preview("Hello World, this is a long text".to_string(), 10);
        assert_eq!(result.plain_text.encode_utf16().count(), 10);
    }

    #[test]
    fn preview_truncation_clamps_spans() {
        let parser = CindermarkParser::new(None);
        // "bold text here" = 14 clean chars, truncate to 8 → "bold tex"
        // Bold span covers [0,4), should survive. No span should exceed max_chars.
        let result = parser.render_preview("**bold** text here".to_string(), 8);
        assert!(result.plain_text.encode_utf16().count() <= 8);
        for span in &result.spans {
            assert!(
                span.start < 8,
                "span start {} exceeds max_chars",
                span.start
            );
            assert!(span.end <= 8, "span end {} exceeds max_chars", span.end);
        }
    }

    #[test]
    fn preview_empty_text() {
        let parser = CindermarkParser::new(None);
        let result = parser.render_preview(String::new(), 200);
        assert_eq!(result.plain_text, "");
        assert!(result.spans.is_empty());
    }

    #[test]
    fn preview_skips_code_blocks() {
        let parser = CindermarkParser::new(None);
        let result = parser.render_preview(
            "Hello\n\n```rust\nlet x = 1;\n```\n\nWorld".to_string(),
            200,
        );
        assert_eq!(result.plain_text, "Hello\nWorld");
    }

    #[test]
    fn preview_italic_and_strikethrough() {
        let parser = CindermarkParser::new(None);
        let result = parser.render_preview("*italic* and ~~struck~~".to_string(), 200);
        assert_eq!(result.plain_text, "italic and struck");
        assert_eq!(result.spans.len(), 2);
        assert_eq!(result.spans[0].span_type, FfiInlineType::Italic);
        assert_eq!(result.spans[0].start, 0);
        assert_eq!(result.spans[0].end, 6);
        assert_eq!(result.spans[1].span_type, FfiInlineType::Strikethrough);
        assert_eq!(result.spans[1].start, 11);
        assert_eq!(result.spans[1].end, 17);
    }

    #[test]
    fn preview_bullet_list() {
        let parser = CindermarkParser::new(None);
        let result = parser.render_preview("- item one\n- item two".to_string(), 200);
        assert_eq!(result.plain_text, "item one\nitem two");
    }

    #[test]
    fn preview_inline_code() {
        let parser = CindermarkParser::new(None);
        let result = parser.render_preview("Use `println` here".to_string(), 200);
        assert_eq!(result.plain_text, "Use println here");
        assert_eq!(result.spans.len(), 1);
        assert_eq!(result.spans[0].span_type, FfiInlineType::InlineCode);
        assert_eq!(result.spans[0].start, 4);
        assert_eq!(result.spans[0].end, 11);
    }

    #[test]
    fn preview_blockquote() {
        let parser = CindermarkParser::new(None);
        let result = parser.render_preview("> Quoted **text**".to_string(), 200);
        assert_eq!(result.plain_text, "Quoted text");
        assert_eq!(result.spans.len(), 1);
        assert_eq!(result.spans[0].span_type, FfiInlineType::Bold);
    }

    #[test]
    fn preview_wiki_link_stripped() {
        let parser = CindermarkParser::new(None);
        let result = parser.render_preview("See [[My Note]] here".to_string(), 200);
        assert_eq!(result.plain_text, "See My Note here");
    }

    #[test]
    fn preview_wiki_link_alias_uses_display_text() {
        let parser = CindermarkParser::new(None);
        let result = parser.render_preview("See [[Target|friendly name]] here".to_string(), 200);
        assert_eq!(result.plain_text, "See friendly name here");
    }

    #[test]
    fn preview_comment_stripped_entirely() {
        let parser = CindermarkParser::new(None);
        let result =
            parser.render_preview("visible %%hidden drafting note%% more".to_string(), 200);
        // Double space is acceptable — matches Obsidian's comment-stripping behavior.
        assert!(
            result.plain_text.starts_with("visible") && result.plain_text.ends_with("more"),
            "Comment body must not appear in preview, got {:?}",
            result.plain_text
        );
        assert!(
            !result.plain_text.contains("hidden drafting note"),
            "Comment body leaked into preview: {:?}",
            result.plain_text
        );
        assert!(
            !result.plain_text.contains("%%"),
            "Comment fences must not appear in preview: {:?}",
            result.plain_text
        );
    }

    #[test]
    fn preview_bold_italic_nested() {
        let parser = CindermarkParser::new(None);
        let result = parser.render_preview("***bold italic*** text".to_string(), 200);
        assert_eq!(result.plain_text, "bold italic text");
        assert_eq!(result.spans.len(), 1);
        assert_eq!(result.spans[0].span_type, FfiInlineType::BoldItalic);
        assert_eq!(result.spans[0].start, 0);
        assert_eq!(result.spans[0].end, 11); // "bold italic" = 11 chars
    }

    #[test]
    fn preview_mixed_formatting() {
        let parser = CindermarkParser::new(None);
        // Inline code has higher priority: backticks are claimed first,
        // which prevents bold ** from matching across the code span.
        // This matches CommonMark behavior.
        let result = parser.render_preview("**bold** and `code`".to_string(), 200);
        assert_eq!(result.plain_text, "bold and code");
        assert_eq!(result.spans.len(), 2);
        assert_eq!(result.spans[0].span_type, FfiInlineType::Bold);
        assert_eq!(result.spans[1].span_type, FfiInlineType::InlineCode);
    }

    #[test]
    fn preview_stacked_strikethrough_bold() {
        let parser = CindermarkParser::new(None);
        let result = parser.render_preview("~~**bold strike**~~".to_string(), 200);
        assert_eq!(result.plain_text, "bold strike");
        // Should have both strikethrough and bold spans
        assert!(result.spans.len() >= 2);
    }

    #[test]
    fn render_previews_single_parse() {
        let parser = CindermarkParser::new(None);
        let results = parser.render_previews(
            "# Title\n\n**bold** paragraph\n\nMore text here".to_string(),
            10,
            200,
        );
        assert_eq!(results.len(), 2);
        // Short preview truncated to ~10 UTF-16 code units
        assert!(results[0].plain_text.encode_utf16().count() <= 10);
        // Long preview contains full content
        assert_eq!(
            results[1].plain_text,
            "Title\nbold paragraph\nMore text here"
        );
        assert!(!results[1].spans.is_empty());
    }

    #[test]
    fn ffi_table_data() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse("| A | B |\n| --- | --- |\n| 1 | 2 |".to_string());
        assert_eq!(result.blocks[0].block_type, FfiBlockType::Table);
        assert_eq!(result.blocks[0].table_headers, vec!["A", "B"]);
        assert_eq!(
            result.blocks[0].table_rows,
            vec![vec!["1".to_string(), "2".to_string()]]
        );
    }

    // MARK: - Wiki links + headings in parse result

    #[test]
    fn parse_result_contains_wiki_links() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse("See [[My Note]] and [[Other Note]]".to_string());
        assert_eq!(result.wiki_links, vec!["My Note", "Other Note"]);
    }

    #[test]
    fn parse_result_wiki_links_deduped() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse("See [[Note]] and [[Note]] again".to_string());
        assert_eq!(result.wiki_links, vec!["Note"]);
    }

    #[test]
    fn parse_result_wiki_link_alias_uses_target_for_backlinks() {
        let parser = CindermarkParser::new(None);
        let result = parser
            .parse("See [[Project Apollo|the moon shot]] and [[Project Apollo]] again".to_string());
        // Backlinks use the target; the alias is display-only and bare + aliased
        // references to the same target dedupe to one backlink entry.
        assert_eq!(result.wiki_links, vec!["Project Apollo"]);
    }

    #[test]
    fn parse_result_wiki_links_skip_code_blocks() {
        let parser = CindermarkParser::new(None);
        let result =
            parser.parse("See [[Real]]\n\n```\n[[InCode]]\n```\n\n[[Also Real]]".to_string());
        assert_eq!(result.wiki_links, vec!["Real", "Also Real"]);
    }

    #[test]
    fn parse_result_contains_headings() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse("# Title\n\nParagraph\n\n## Subtitle\n\n### Deep".to_string());
        assert_eq!(result.headings.len(), 3);
        assert_eq!(result.headings[0].level, 1);
        assert_eq!(result.headings[0].text, "Title");
        assert_eq!(result.headings[1].level, 2);
        assert_eq!(result.headings[1].text, "Subtitle");
        assert_eq!(result.headings[2].level, 3);
        assert_eq!(result.headings[2].text, "Deep");
    }

    #[test]
    fn parse_result_no_headings_or_wiki_links() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse("Just a plain paragraph.".to_string());
        assert!(result.wiki_links.is_empty());
        assert!(result.headings.is_empty());
    }

    // MARK: - parse_for_save combined method

    #[test]
    fn parse_for_save_returns_everything() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse_for_save(
            "# Title\n\nSee [[My Note]]\n\n**bold** text".to_string(),
            10,
            200,
        );
        // Blocks
        assert!(!result.blocks.is_empty());
        // Stats
        assert!(result.stats.word_count > 0);
        assert_eq!(result.stats.heading_count, 1);
        // Wiki links
        assert_eq!(result.wiki_links, vec!["My Note"]);
        // Headings
        assert_eq!(result.headings.len(), 1);
        assert_eq!(result.headings[0].text, "Title");
        // Previews
        assert!(!result.long_preview.plain_text.is_empty());
        assert!(result.long_preview.plain_text.contains("bold text"));
        // Short preview is truncated
        assert!(result.short_preview.plain_text.encode_utf16().count() <= 10);
    }

    #[test]
    fn parse_for_save_empty_input() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse_for_save(String::new(), 200, 800);
        assert!(result.blocks.is_empty());
        assert!(result.wiki_links.is_empty());
        assert!(result.headings.is_empty());
        assert!(result.short_preview.plain_text.is_empty());
        assert!(result.long_preview.plain_text.is_empty());
    }

    #[test]
    fn parse_for_save_matches_separate_calls() {
        let parser = CindermarkParser::new(None);
        let text = "# Hello\n\nSee [[World]]\n\n- [x] Done\n- [ ] Todo".to_string();
        let combined = parser.parse_for_save(text.clone(), 200, 800);
        let separate_parse = parser.parse(text.clone());
        let separate_previews = parser.render_previews(text.clone(), 200, 800);
        let separate_wikilinks = parser.extract_wiki_links(text);

        // Wiki links should match
        assert_eq!(combined.wiki_links, separate_wikilinks);
        // Stats should match
        assert_eq!(combined.stats.word_count, separate_parse.stats.word_count);
        assert_eq!(
            combined.stats.checkbox_total,
            separate_parse.stats.checkbox_total
        );
        // Preview text should match
        assert_eq!(
            combined.short_preview.plain_text,
            separate_previews[0].plain_text
        );
        assert_eq!(
            combined.long_preview.plain_text,
            separate_previews[1].plain_text
        );
        // Heading count should match
        assert_eq!(combined.headings.len(), separate_parse.headings.len());
    }

    #[test]
    fn parse_for_save_wiki_links_in_lists() {
        let parser = CindermarkParser::new(None);
        let result =
            parser.parse_for_save("- See [[Note A]]\n- And [[Note B]]".to_string(), 200, 800);
        assert_eq!(result.wiki_links, vec!["Note A", "Note B"]);
    }

    #[test]
    fn parse_for_save_wiki_links_in_tables() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse_for_save(
            "| Header | Link |\n| --- | --- |\n| Cell | See [[Table Note]] |".to_string(),
            200,
            800,
        );
        assert_eq!(result.wiki_links, vec!["Table Note"]);
    }

    #[test]
    fn parse_result_wiki_links_in_tables() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse("| A | B |\n| - | - |\n| [[Note X]] | text |".to_string());
        assert_eq!(result.wiki_links, vec!["Note X"]);
    }

    #[test]
    fn wiki_links_in_tables_deduped_with_body() {
        let parser = CindermarkParser::new(None);
        let result = parser.parse_for_save(
            "See [[Shared]]\n\n| A |\n| - |\n| [[Shared]] |\n| [[Table Only]] |".to_string(),
            200,
            800,
        );
        // "Shared" appears in both body and table — should be deduped
        assert_eq!(result.wiki_links, vec!["Shared", "Table Only"]);
    }

    #[test]
    fn wiki_links_in_blockquotes() {
        let parser = CindermarkParser::new(None);
        let result =
            parser.parse_for_save("> See [[Quoted Note]] for context".to_string(), 200, 800);
        assert_eq!(result.wiki_links, vec!["Quoted Note"]);
    }
}
