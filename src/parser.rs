#![allow(clippy::manual_strip)]
//! Block-level parser producing a `Document` from source text.
//!
//! Matches the exact semantics of `MarkdownParser.swift`:
//! - Same block detection priority order
//! - Same line grouping rules for lists, blockquotes, paragraphs
//! - Two parse modes: Grouped (for rendering) and Editable (for block editor)
//!
//! Operates on raw source bytes with line-oriented scanning. The inline parser
//! (`inline.rs`) runs on each block's text content after block parsing.

use crate::ast::*;
use crate::inline;
use crate::utf16::Utf16Map;

/// Maximum columns in a table before it's treated as a paragraph.
/// Even on a 27" display, tables beyond ~20 columns are unusable.
const MAX_TABLE_COLUMNS: usize = 20;

/// Maximum data rows in a table.
const MAX_TABLE_ROWS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParsedListKind {
    Bullet,
    Ordered,
    Checkbox(bool),
}

#[derive(Debug, Clone)]
struct ParsedListMarker<'a> {
    kind: ParsedListKind,
    indent: usize,
    marker_start: usize,
    marker_end: usize,
    content_start: usize,
    marker_source: &'a str,
    unordered_marker: Option<char>,
    ordered_delimiter: Option<char>,
    ordered_raw_number: &'a str,
    ordered_number: u32,
}

/// Parse source text into a `Document` with the given mode.
pub fn parse(source: &str, mode: ParseMode) -> Document {
    let bytes = source.as_bytes();
    let utf16_map = Utf16Map::build(bytes);
    let lines = split_lines(source);
    let mut blocks = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = &lines[i];
        let trimmed = line.text.trim();

        // Empty line
        if trimmed.is_empty() {
            if mode == ParseMode::Grouped {
                // Collapse multiple empty lines
                if let Some(last) = blocks.last() {
                    if matches!(block_kind_tag(last), BlockKindTag::Empty) {
                        i += 1;
                        continue;
                    }
                }
            }
            blocks.push(make_block(
                BlockKind::Empty,
                &lines,
                i,
                i + 1,
                bytes,
                &utf16_map,
            ));
            i += 1;
            continue;
        }

        // Image / sketch marker: `![](ember:<UUID>)` on a line by itself. The
        // editor injects these as `ImageTextAttachment` (U+FFFC) so the user
        // sees an inline image, not raw markdown. Must run before paragraph
        // collection — once a marker rolls up into a paragraph block the
        // injector can't get at it. Lexer already recognises the same
        // pattern for token output; we mirror the rule here at the block
        // level so editor surfaces (which consume blocks, not tokens) get
        // the dedicated `ImageMarker` kind.
        if let Some(uuid) = parse_image_marker_line(trimmed) {
            blocks.push(make_block(
                BlockKind::ImageMarker { uuid },
                &lines,
                i,
                i + 1,
                bytes,
                &utf16_map,
            ));
            i += 1;
            continue;
        }

        // Indented code block (CommonMark §4.4): 4+ leading spaces (or tab).
        // Must come before fenced code so `    ```` is treated as code, not a fence.
        // Cannot interrupt a paragraph — that constraint is naturally enforced
        // because paragraph collection runs entirely within a single outer-loop
        // iteration and emits its block before we return here.
        if is_indented_code_line(line.text) {
            let start_line = i;
            let mut code_lines: Vec<String> = Vec::new();
            let mut last_content_offset = 0; // index in code_lines past the last non-blank line
            while i < lines.len() {
                let cl = lines[i].text;
                if is_indented_code_line(cl) {
                    code_lines.push(strip_indented_code_indent(cl).to_string());
                    i += 1;
                    last_content_offset = code_lines.len();
                } else if cl.trim().is_empty() {
                    code_lines.push(String::new());
                    i += 1;
                } else {
                    break;
                }
            }
            // Drop trailing blank lines per CommonMark §4.4.
            code_lines.truncate(last_content_offset);
            // Rewind `i` past any consumed-but-discarded trailing blank lines so
            // they get re-emitted as Empty blocks on the next outer iteration.
            i = start_line + last_content_offset;
            blocks.push(make_block(
                BlockKind::CodeBlock {
                    language: None,
                    code: code_lines.join("\n"),
                },
                &lines,
                start_line,
                i,
                bytes,
                &utf16_map,
            ));
            continue;
        }

        // Fenced code block
        if trimmed.starts_with("```") {
            let start_line = i;
            let language = {
                let after_fence = trimmed[3..].trim();
                if after_fence.is_empty() {
                    None
                } else {
                    // Take first word only (CommonMark: info string's first word is the language)
                    Some(
                        after_fence
                            .split_whitespace()
                            .next()
                            .unwrap_or(after_fence)
                            .to_string(),
                    )
                }
            };
            let mut code_lines: Vec<&str> = Vec::new();
            i += 1;
            while i < lines.len() {
                let cl = lines[i].text.trim();
                if cl.starts_with("```") {
                    i += 1;
                    break;
                }
                code_lines.push(lines[i].text);
                i += 1;
            }
            let code = code_lines.join("\n");
            // Route ```mermaid fences into a dedicated block kind so Swift
            // can render the diagram instead of a code tile. Detection is
            // case-insensitive to tolerate `Mermaid` / `MERMAID`.
            let kind = if language.as_deref().is_some_and(is_mermaid_info_string) {
                BlockKind::MermaidDiagram {
                    diagram_type: MermaidDiagramType::from_source(&code),
                    source: code,
                }
            } else {
                BlockKind::CodeBlock { language, code }
            };
            blocks.push(make_block(kind, &lines, start_line, i, bytes, &utf16_map));
            continue;
        }

        // Table: current line has pipes AND next line is a separator
        if is_table_row(trimmed)
            && i + 1 < lines.len()
            && is_table_separator(lines[i + 1].text.trim())
        {
            let headers = parse_table_row(trimmed);
            // Column limit: tables beyond MAX_TABLE_COLUMNS fall through to paragraph
            if headers.len() <= MAX_TABLE_COLUMNS {
                let start_line = i;
                let separator_line = lines[i + 1].text.trim();
                let alignments = parse_alignments(separator_line);
                i += 2; // skip header + separator
                let mut rows: Vec<Vec<String>> = Vec::new();
                while i < lines.len() && rows.len() < MAX_TABLE_ROWS {
                    let rt = lines[i].text.trim();
                    if !is_table_row(rt) {
                        break;
                    }
                    rows.push(parse_table_row(rt));
                    i += 1;
                }
                // Skip remaining rows beyond the limit
                while i < lines.len() && is_table_row(lines[i].text.trim()) {
                    i += 1;
                }
                blocks.push(make_block(
                    BlockKind::Table {
                        headers,
                        rows,
                        alignments,
                    },
                    &lines,
                    start_line,
                    i,
                    bytes,
                    &utf16_map,
                ));
                continue;
            }
            // else: too many columns, fall through to paragraph
        }

        // Horizontal rule
        if is_horizontal_rule(trimmed) {
            blocks.push(make_block(
                BlockKind::HorizontalRule,
                &lines,
                i,
                i + 1,
                bytes,
                &utf16_map,
            ));
            i += 1;
            continue;
        }

        // Heading
        if let Some(kind) = parse_heading(trimmed) {
            blocks.push(make_block(kind, &lines, i, i + 1, bytes, &utf16_map));
            i += 1;
            continue;
        }

        // Blockquote (with Obsidian callout detection on first line)
        if trimmed.starts_with("> ") || trimmed == ">" {
            let start_line = i;
            let mut quote_lines: Vec<&str> = Vec::new();
            while i < lines.len() {
                let ql = lines[i].text.trim();
                if ql.starts_with("> ") {
                    quote_lines.push(&ql[2..]);
                } else if ql == ">" {
                    quote_lines.push("");
                } else {
                    break;
                }
                i += 1;
            }
            // Callout: first line begins with `[!<kind>]` (optionally followed by a title).
            // Remaining lines form the body. Unknown kind names degrade to a plain blockquote.
            if let Some(first) = quote_lines.first() {
                if let Some((kind, title)) = parse_callout_header(first) {
                    let body = if quote_lines.len() > 1 {
                        quote_lines[1..].join("\n")
                    } else {
                        String::new()
                    };
                    blocks.push(make_block(
                        BlockKind::Callout {
                            kind,
                            title,
                            text: body,
                        },
                        &lines,
                        start_line,
                        i,
                        bytes,
                        &utf16_map,
                    ));
                    continue;
                }
            }
            blocks.push(make_block(
                BlockKind::Blockquote {
                    text: quote_lines.join("\n"),
                },
                &lines,
                start_line,
                i,
                bytes,
                &utf16_map,
            ));
            continue;
        }

        let list_marker = parse_list_marker(line.text);

        // Checkbox (GFM-style extension layered on valid bullet markers).
        if let Some(marker) = list_marker
            .as_ref()
            .filter(|m| matches!(m.kind, ParsedListKind::Checkbox(_)))
        {
            let checked = matches!(marker.kind, ParsedListKind::Checkbox(true));
            let text = line.text[marker.content_start..].to_string();
            blocks.push(make_block_with_marker(
                BlockKind::Checkbox { checked, text },
                marker_to_meta(marker, line, bytes, &utf16_map),
                &lines,
                i,
                i + 1,
                bytes,
                &utf16_map,
            ));
            i += 1;
            continue;
        }

        // Unordered list
        if let Some(marker) = list_marker
            .as_ref()
            .filter(|m| m.kind == ParsedListKind::Bullet)
        {
            match mode {
                ParseMode::Grouped => {
                    let start_line = i;
                    let first_marker = marker.clone();
                    let mut items: Vec<String> = Vec::new();
                    while i < lines.len() {
                        let Some(ul_marker) = parse_list_marker(lines[i].text) else {
                            let ul = lines[i].text.trim();
                            if ul.is_empty() {
                                break;
                            }
                            if let Some(last) = items.last_mut() {
                                last.push(' ');
                                last.push_str(ul);
                            }
                            i += 1;
                            continue;
                        };

                        if matches!(ul_marker.kind, ParsedListKind::Checkbox(_)) {
                            break; // hand off to checkbox parser
                        } else if ul_marker.kind == ParsedListKind::Bullet
                            && ul_marker.unordered_marker == first_marker.unordered_marker
                        {
                            items.push(lines[i].text[ul_marker.content_start..].to_string());
                        } else {
                            break;
                        }
                        i += 1;
                    }
                    let list_items = items
                        .into_iter()
                        .map(|text| ListItem {
                            text,
                            inline_spans: Vec::new(),
                        })
                        .collect();
                    blocks.push(make_block_with_marker(
                        BlockKind::BulletList { items: list_items },
                        marker_to_meta(&first_marker, &lines[start_line], bytes, &utf16_map),
                        &lines,
                        start_line,
                        i,
                        bytes,
                        &utf16_map,
                    ));
                }
                ParseMode::Editable => {
                    let text = line.text[marker.content_start..].to_string();
                    blocks.push(make_block_with_marker(
                        BlockKind::BulletItem { text },
                        marker_to_meta(marker, line, bytes, &utf16_map),
                        &lines,
                        i,
                        i + 1,
                        bytes,
                        &utf16_map,
                    ));
                    i += 1;
                }
            }
            continue;
        }

        // Ordered list
        if let Some(marker) = list_marker
            .as_ref()
            .filter(|m| m.kind == ParsedListKind::Ordered)
        {
            match mode {
                ParseMode::Grouped => {
                    let start_line = i;
                    let first_marker = marker.clone();
                    let mut items: Vec<String> = Vec::new();
                    while i < lines.len() {
                        let ol = lines[i].text.trim();
                        if let Some(ol_marker) = parse_list_marker(lines[i].text) {
                            if ol_marker.kind == ParsedListKind::Ordered
                                && ol_marker.ordered_delimiter == first_marker.ordered_delimiter
                            {
                                items.push(lines[i].text[ol_marker.content_start..].to_string());
                            } else {
                                break;
                            }
                        } else if ol.is_empty() {
                            break;
                        } else {
                            // Continuation of previous item
                            if let Some(last) = items.last_mut() {
                                last.push(' ');
                                last.push_str(ol);
                            }
                        }
                        i += 1;
                    }
                    let list_items = items
                        .into_iter()
                        .map(|text| ListItem {
                            text,
                            inline_spans: Vec::new(),
                        })
                        .collect();
                    blocks.push(make_block_with_marker(
                        BlockKind::OrderedList {
                            start: first_marker.ordered_number,
                            items: list_items,
                        },
                        marker_to_meta(&first_marker, &lines[start_line], bytes, &utf16_map),
                        &lines,
                        start_line,
                        i,
                        bytes,
                        &utf16_map,
                    ));
                }
                ParseMode::Editable => {
                    let text = line.text[marker.content_start..].to_string();
                    let number = marker.ordered_number;
                    blocks.push(make_block_with_marker(
                        BlockKind::NumberedItem { number, text },
                        marker_to_meta(marker, line, bytes, &utf16_map),
                        &lines,
                        i,
                        i + 1,
                        bytes,
                        &utf16_map,
                    ));
                    i += 1;
                }
            }
            continue;
        }

        // Footnote definition
        if let Some(kind) = parse_footnote_def(trimmed) {
            blocks.push(make_block(kind, &lines, i, i + 1, bytes, &utf16_map));
            i += 1;
            continue;
        }

        // Paragraph — collect consecutive non-special lines
        let start_line = i;
        let mut para_lines: Vec<&str> = Vec::new();
        while i < lines.len() {
            let pl = lines[i].text;
            let pt = pl.trim();
            if pt.is_empty()
                || pt.starts_with("```")
                || is_heading_line(pt)
                || pt.starts_with("> ")
                || is_horizontal_rule(pt)
                || parse_list_marker(pl).is_some()
                || parse_footnote_def(pt).is_some()
                || (is_table_row(pt)
                    && i + 1 < lines.len()
                    && is_table_separator(lines[i + 1].text.trim()))
                || (!para_lines.is_empty() && parse_setext_underline(pl).is_some())
            {
                break;
            }
            para_lines.push(pl);
            i += 1;
        }

        // Setext heading: paragraph followed by =/- underline line.
        // CommonMark resolves the ambiguity in favor of setext over thematic break.
        if !para_lines.is_empty() && i < lines.len() {
            if let Some(level) = parse_setext_underline(lines[i].text) {
                let heading_text = para_lines.join("\n").trim().to_string();
                blocks.push(make_block(
                    BlockKind::Heading {
                        level,
                        text: heading_text,
                    },
                    &lines,
                    start_line,
                    i + 1,
                    bytes,
                    &utf16_map,
                ));
                i += 1;
                continue;
            }
        }

        if !para_lines.is_empty() {
            blocks.push(make_block(
                BlockKind::Paragraph {
                    text: para_lines.join("\n"),
                },
                &lines,
                start_line,
                i,
                bytes,
                &utf16_map,
            ));
        }
    }

    // Run inline parsing on all blocks
    inline::parse_inline_spans(&mut blocks, bytes, &utf16_map);

    Document {
        line_count: lines.len() as u32,
        blocks,
    }
}

// MARK: - Line splitting

/// A line with its byte range in the source.
struct Line<'a> {
    text: &'a str,
    byte_start: usize,
    byte_end: usize, // exclusive, includes the newline if present
}

fn split_lines(source: &str) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    let mut start = 0;
    let bytes = source.as_bytes();

    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            // Strip trailing \r for Windows line endings
            let text_end = if i > start && bytes[i - 1] == b'\r' {
                i - 1
            } else {
                i
            };
            lines.push(Line {
                text: &source[start..text_end],
                byte_start: start,
                byte_end: i + 1,
            });
            start = i + 1;
        }
    }
    // Trailing content after last newline
    if start < source.len() {
        let end = if source.as_bytes().last() == Some(&b'\r') {
            source.len() - 1
        } else {
            source.len()
        };
        lines.push(Line {
            text: &source[start..end],
            byte_start: start,
            byte_end: source.len(),
        });
    }

    lines
}

// MARK: - Block construction

fn make_block(
    kind: BlockKind,
    lines: &[Line],
    line_start: usize,
    line_end: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
) -> BlockNode {
    make_block_with_optional_marker(kind, None, lines, line_start, line_end, source, utf16_map)
}

fn make_block_with_marker(
    kind: BlockKind,
    list_marker: ListMarkerMeta,
    lines: &[Line],
    line_start: usize,
    line_end: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
) -> BlockNode {
    make_block_with_optional_marker(
        kind,
        Some(list_marker),
        lines,
        line_start,
        line_end,
        source,
        utf16_map,
    )
}

fn make_block_with_optional_marker(
    kind: BlockKind,
    list_marker: Option<ListMarkerMeta>,
    lines: &[Line],
    line_start: usize,
    line_end: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
) -> BlockNode {
    let byte_start = lines[line_start].byte_start as u32;
    let byte_end = if line_end > 0 && line_end <= lines.len() {
        lines[line_end - 1].byte_end as u32
    } else {
        byte_start
    };
    let utf16_start = utf16_map.byte_to_utf16(byte_start, source);
    let utf16_end = utf16_map.byte_to_utf16(byte_end, source);

    BlockNode {
        kind,
        line_start: line_start as u32,
        line_end: line_end as u32,
        utf16_start,
        utf16_end,
        byte_start,
        byte_end,
        list_marker,
        inline_spans: Vec::new(),
    }
}

fn marker_to_meta(
    marker: &ParsedListMarker,
    line: &Line,
    source: &[u8],
    utf16_map: &Utf16Map,
) -> ListMarkerMeta {
    let marker_byte_start = (line.byte_start + marker.marker_start) as u32;
    let marker_byte_end = (line.byte_start + marker.marker_end) as u32;
    let content_byte_start = (line.byte_start + marker.content_start) as u32;

    ListMarkerMeta {
        indent: marker.indent as u32,
        marker_utf16_start: utf16_map.byte_to_utf16(marker_byte_start, source),
        marker_utf16_end: utf16_map.byte_to_utf16(marker_byte_end, source),
        marker_byte_start,
        marker_byte_end,
        content_byte_start,
        marker_source: marker.marker_source.to_string(),
        unordered_marker: marker
            .unordered_marker
            .map(|c| c.to_string())
            .unwrap_or_default(),
        ordered_delimiter: marker
            .ordered_delimiter
            .map(|c| c.to_string())
            .unwrap_or_default(),
        ordered_raw_number: marker.ordered_raw_number.to_string(),
    }
}

#[derive(PartialEq)]
enum BlockKindTag {
    Empty,
    Other,
}

fn block_kind_tag(block: &BlockNode) -> BlockKindTag {
    match block.kind {
        BlockKind::Empty => BlockKindTag::Empty,
        _ => BlockKindTag::Other,
    }
}

// MARK: - Block detection helpers

fn parse_heading(line: &str) -> Option<BlockKind> {
    for level in (1..=6u8).rev() {
        let prefix = "#".repeat(level as usize);
        let marker = format!("{} ", prefix);
        if line.starts_with(&marker) {
            return Some(BlockKind::Heading {
                level,
                text: line[marker.len()..].to_string(),
            });
        }
    }
    None
}

fn is_heading_line(line: &str) -> bool {
    if !line.starts_with('#') {
        return false;
    }
    for level in 1..=6 {
        let marker = format!("{} ", "#".repeat(level));
        if line.starts_with(&marker) {
            return true;
        }
    }
    false
}

/// CommonMark §4.4: a line begins an indented code block when it is indented
/// by ≥4 spaces (or a leading tab), and is not blank, and isn't already part
/// of another block. Tabs count as advancing to the next 4-column stop, but
/// for our byte scanner a leading tab is treated as ≥4 columns.
fn is_indented_code_line(line: &str) -> bool {
    let bytes = line.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    if bytes[0] == b'\t' {
        return !line[1..].trim().is_empty();
    }
    if bytes.len() >= 4 && &bytes[..4] == b"    " {
        return !line[4..].trim().is_empty() || bytes.len() > 4;
    }
    false
}

/// Strip the 4-space (or 1-tab) indentation from an indented-code line.
fn strip_indented_code_indent(line: &str) -> &str {
    let bytes = line.as_bytes();
    if !bytes.is_empty() && bytes[0] == b'\t' {
        &line[1..]
    } else if bytes.len() >= 4 && &bytes[..4] == b"    " {
        &line[4..]
    } else {
        line
    }
}

/// Detect an Obsidian callout header `[!<kind>]` with optional trailing title.
/// Returns the kind and (if present) the trimmed custom title.
///
/// Format (all parts after `>` stripping done by caller):
///   `[!note]`            → (Note, None)
///   `[!tip] Friendly`    → (Tip, Some("Friendly"))
///   `[!Warning]`         → case-insensitive kind
///   `[!unknown]`         → None  (caller falls back to plain blockquote)
///
/// Foldable markers (`[!note]-` / `[!note]+`) are currently ignored — the `-`
/// or `+` is stripped so unknown-kind fallback doesn't fire. We skip the fold
/// state for launch and can surface it later without an ABI change.
pub(crate) fn parse_callout_header(
    line: &str,
) -> Option<(crate::ast::CalloutKind, Option<String>)> {
    let trimmed = line.trim_start();
    let after_open = trimmed.strip_prefix("[!")?;
    let close_idx = after_open.find(']')?;
    let kind_name = &after_open[..close_idx];
    let kind = crate::ast::CalloutKind::from_name(kind_name)?;
    let mut rest = &after_open[close_idx + 1..];
    // Strip optional Obsidian fold marker (+ open-by-default, - closed-by-default)
    if rest.starts_with('+') || rest.starts_with('-') {
        rest = &rest[1..];
    }
    let title = rest.trim();
    let title = if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    };
    Some((kind, title))
}

/// Detect a CommonMark setext heading underline: `=+` (level 1) or `-+` (level 2),
/// with optional 0-3 leading spaces and trailing whitespace, nothing else.
/// Returns the heading level if matched.
pub(crate) fn parse_setext_underline(line: &str) -> Option<u8> {
    let bytes = line.as_bytes();
    let mut i = 0;
    // 0-3 leading spaces (4+ would be an indented code line, not setext).
    let mut leading_spaces = 0;
    while i < bytes.len() && bytes[i] == b' ' {
        leading_spaces += 1;
        i += 1;
    }
    if leading_spaces > 3 {
        return None;
    }
    if i >= bytes.len() {
        return None;
    }
    let underline_char = bytes[i];
    if underline_char != b'=' && underline_char != b'-' {
        return None;
    }
    while i < bytes.len() && bytes[i] == underline_char {
        i += 1;
    }
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i != bytes.len() {
        return None;
    }
    Some(if underline_char == b'=' { 1 } else { 2 })
}

fn is_horizontal_rule(line: &str) -> bool {
    let stripped: String = line.chars().filter(|c| *c != ' ').collect();
    if stripped.len() < 3 {
        return false;
    }
    (stripped.chars().all(|c| c == '-') && stripped.len() >= 3)
        || (stripped.chars().all(|c| c == '*') && stripped.len() == 3)
        || (stripped.chars().all(|c| c == '_') && stripped.len() >= 3)
}

fn parse_list_marker(line: &str) -> Option<ParsedListMarker<'_>> {
    let bytes = line.as_bytes();
    let mut indent = 0;
    while indent < bytes.len() && bytes[indent] == b' ' {
        indent += 1;
    }
    if indent > 3 || indent >= bytes.len() {
        return None;
    }

    let marker_start = indent;
    let marker_byte = bytes[marker_start];

    if matches!(marker_byte, b'-' | b'+' | b'*') {
        let after_marker = marker_start + 1;
        if after_marker >= bytes.len() || !is_marker_space(bytes[after_marker]) {
            return None;
        }

        let mut bullet_marker_end = after_marker + 1;
        while bullet_marker_end < bytes.len() && is_marker_space(bytes[bullet_marker_end]) {
            bullet_marker_end += 1;
        }

        if let Some((checked, checkbox_end)) = parse_checkbox_tail(bytes, bullet_marker_end) {
            let marker_source = &line[marker_start..checkbox_end];
            return Some(ParsedListMarker {
                kind: ParsedListKind::Checkbox(checked),
                indent,
                marker_start,
                marker_end: checkbox_end,
                content_start: checkbox_end,
                marker_source,
                unordered_marker: Some(marker_byte as char),
                ordered_delimiter: None,
                ordered_raw_number: "",
                ordered_number: 0,
            });
        }

        let marker_source = &line[marker_start..bullet_marker_end];
        return Some(ParsedListMarker {
            kind: ParsedListKind::Bullet,
            indent,
            marker_start,
            marker_end: bullet_marker_end,
            content_start: bullet_marker_end,
            marker_source,
            unordered_marker: Some(marker_byte as char),
            ordered_delimiter: None,
            ordered_raw_number: "",
            ordered_number: 0,
        });
    }

    if marker_byte.is_ascii_digit() {
        let number_start = marker_start;
        let mut number_end = number_start;
        while number_end < bytes.len() && bytes[number_end].is_ascii_digit() {
            number_end += 1;
        }
        let digit_count = number_end - number_start;
        if digit_count == 0 || digit_count > 9 || number_end >= bytes.len() {
            return None;
        }

        let delimiter = bytes[number_end];
        if delimiter != b'.' && delimiter != b')' {
            return None;
        }

        let after_delimiter = number_end + 1;
        if after_delimiter >= bytes.len() || !is_marker_space(bytes[after_delimiter]) {
            return None;
        }

        let mut marker_end = after_delimiter + 1;
        while marker_end < bytes.len() && is_marker_space(bytes[marker_end]) {
            marker_end += 1;
        }

        let raw_number = &line[number_start..number_end];
        let ordered_number = raw_number.parse::<u32>().unwrap_or(1);
        let marker_source = &line[marker_start..marker_end];

        return Some(ParsedListMarker {
            kind: ParsedListKind::Ordered,
            indent,
            marker_start,
            marker_end,
            content_start: marker_end,
            marker_source,
            unordered_marker: None,
            ordered_delimiter: Some(delimiter as char),
            ordered_raw_number: raw_number,
            ordered_number,
        });
    }

    None
}

fn is_marker_space(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

fn parse_checkbox_tail(bytes: &[u8], marker_content_start: usize) -> Option<(bool, usize)> {
    if marker_content_start + 3 > bytes.len() || bytes[marker_content_start] != b'[' {
        return None;
    }

    let state = bytes[marker_content_start + 1];
    let checked = match state {
        b' ' => false,
        b'x' | b'X' => true,
        _ => return None,
    };

    if bytes[marker_content_start + 2] != b']' {
        return None;
    }

    let after_checkbox = marker_content_start + 3;
    if after_checkbox == bytes.len() {
        return Some((checked, after_checkbox));
    }
    if !is_marker_space(bytes[after_checkbox]) {
        return None;
    }

    let mut end = after_checkbox + 1;
    while end < bytes.len() && is_marker_space(bytes[end]) {
        end += 1;
    }
    Some((checked, end))
}

fn parse_checkbox_any_indent(trimmed_line: &str) -> Option<(&str, &str, bool)> {
    let bytes = trimmed_line.as_bytes();
    let bullet = *bytes.first()?;
    if !matches!(bullet, b'-' | b'+' | b'*') || bytes.get(1) != Some(&b' ') {
        return None;
    }

    let (checked, end) = parse_checkbox_tail(bytes, 2)?;
    Some((&trimmed_line[..end], &trimmed_line[end..], checked))
}

/// Parses `![](ember:<UUID>)` block markers (image / sketch attachments).
///
/// Returns the UUID string (preserving original case) when `line` is *exactly*
/// the marker — leading / trailing whitespace is the caller's job to strip
/// (we receive the already-trimmed line from the parser loop). Anything that
/// isn't a single marker — extra text on the same line, surrounding inline
/// markdown, or a malformed UUID — falls through to the regular paragraph
/// path so the user sees raw markdown instead of a silently-injected blank.
///
/// UUID validation is deliberately permissive: any 36-char `8-4-4-4-12`
/// hex sequence (case-insensitive) qualifies. Stricter version-bit checks
/// would reject UUIDs the app itself produced via `UUID()` (which returns
/// v4) without round-trip-safe value across SwiftData migrations.
pub(crate) fn parse_image_marker_line(line: &str) -> Option<String> {
    const PREFIX: &str = "![](ember:";
    const SUFFIX: &str = ")";
    let stripped = line.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    if !is_uuid_format(stripped) {
        return None;
    }
    Some(stripped.to_string())
}

fn is_uuid_format(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (i, b) in bytes.iter().enumerate() {
        let expect_hyphen = matches!(i, 8 | 13 | 18 | 23);
        if expect_hyphen {
            if *b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

fn parse_footnote_def(line: &str) -> Option<BlockKind> {
    if !line.starts_with("[^") {
        return None;
    }
    let close_bracket = line.find(']')?;
    if close_bracket <= 2 {
        return None;
    }
    let after_close = close_bracket + 1;
    if after_close >= line.len() || line.as_bytes()[after_close] != b':' {
        return None;
    }
    let label = line[2..close_bracket].to_string();
    let text_start = after_close + 1;
    let text = if text_start < line.len() {
        line[text_start..].trim().to_string()
    } else {
        String::new()
    };
    Some(BlockKind::FootnoteDefinition { label, text })
}

// MARK: - Table helpers

fn is_table_row(line: &str) -> bool {
    line.contains('|') && !line.trim_start().starts_with("|--")
}

fn is_table_separator(line: &str) -> bool {
    let stripped = line.replace([' ', '|', '-', ':'], "");
    stripped.is_empty() && line.contains('-') && line.contains('|')
}

fn parse_table_row(line: &str) -> Vec<String> {
    let mut cells: Vec<String> = line.split('|').map(|s| s.trim().to_string()).collect();
    if cells.first().is_some_and(|s| s.is_empty()) {
        cells.remove(0);
    }
    if cells.last().is_some_and(|s| s.is_empty()) {
        cells.pop();
    }
    cells
}

fn parse_alignments(separator: &str) -> Vec<ColumnAlignment> {
    let mut cells: Vec<&str> = separator.split('|').map(|s| s.trim()).collect();
    if cells.first().is_some_and(|s| s.is_empty()) {
        cells.remove(0);
    }
    if cells.last().is_some_and(|s| s.is_empty()) {
        cells.pop();
    }

    cells
        .iter()
        .map(|cell| {
            let has_leading = cell.starts_with(':');
            let has_trailing = cell.ends_with(':');
            if has_leading && has_trailing {
                ColumnAlignment::Center
            } else if has_trailing {
                ColumnAlignment::Right
            } else if has_leading {
                ColumnAlignment::Left
            } else {
                ColumnAlignment::Default
            }
        })
        .collect()
}

// MARK: - Public utilities (matching Swift API)

/// Extract wiki link titles from content (skipping code blocks).
pub fn extract_wiki_links(content: &str) -> Vec<String> {
    let without_code = strip_code_blocks(content);
    parse_inline_segments(&without_code)
        .into_iter()
        .filter_map(|seg| match seg {
            InlineSegment::WikiLink(title) => Some(title),
            _ => None,
        })
        .collect()
}

/// Toggle checkbox at a line index, returning the new content.
pub fn toggle_checkbox(content: &str, line_index: u32) -> String {
    let lines: Vec<&str> = content.split('\n').collect();
    let idx = line_index as usize;
    if idx >= lines.len() {
        return content.to_string();
    }
    let line = lines[idx];
    let trimmed = line.trim_start();
    let indent: &str = &line[..line.len() - trimmed.len()];

    let (marker_source, item_text, checked) = if let Some(marker) = parse_list_marker(line) {
        let ParsedListKind::Checkbox(checked) = marker.kind else {
            return content.to_string();
        };
        (
            marker.marker_source.to_string(),
            line[marker.content_start..].to_string(),
            checked,
        )
    } else if let Some((marker_source, item_text, checked)) = parse_checkbox_any_indent(trimmed) {
        (marker_source.to_string(), item_text.to_string(), checked)
    } else {
        return content.to_string();
    };

    let old_state = if checked {
        if marker_source.contains("[X]") {
            "[X]"
        } else {
            "[x]"
        }
    } else {
        "[ ]"
    };
    let new_state = if checked { "[ ]" } else { "[x]" };
    let new_marker_source = marker_source.replacen(old_state, new_state, 1);
    let new_line = format!("{indent}{new_marker_source}{item_text}");

    let mut result: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    result[idx] = new_line;
    result.join("\n")
}

// MARK: - Inline segments (for wiki link extraction)

enum InlineSegment {
    #[allow(dead_code)]
    Text(String),
    WikiLink(String),
}

fn parse_inline_segments(text: &str) -> Vec<InlineSegment> {
    let mut segments = Vec::new();
    let mut remaining = text;

    while let Some(open_pos) = remaining.find("[[") {
        let before = &remaining[..open_pos];
        if !before.is_empty() {
            segments.push(InlineSegment::Text(before.to_string()));
        }
        let after_open = &remaining[open_pos + 2..];
        if let Some(close_pos) = after_open.find("]]") {
            let body = &after_open[..close_pos];
            // Aliased form `[[target|Display]]` — the target for backlinks is
            // the pre-pipe portion; the display text is rendered inline only.
            let target = body.split('|').next().unwrap_or(body).trim().to_string();
            if !target.is_empty() {
                segments.push(InlineSegment::WikiLink(target));
            } else {
                segments.push(InlineSegment::Text("[[]]".to_string()));
            }
            remaining = &after_open[close_pos + 2..];
        } else {
            segments.push(InlineSegment::Text(remaining[open_pos..].to_string()));
            remaining = "";
        }
    }

    if !remaining.is_empty() {
        segments.push(InlineSegment::Text(remaining.to_string()));
    }

    segments
}

fn strip_code_blocks(text: &str) -> String {
    let mut result = String::new();
    let mut in_code_block = false;

    for line in text.lines() {
        if line.trim().starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if !in_code_block {
            // Strip inline code spans
            let mut cleaned = String::new();
            let mut in_inline_code = false;
            for ch in line.chars() {
                if ch == '`' {
                    in_inline_code = !in_inline_code;
                } else if !in_inline_code {
                    cleaned.push(ch);
                }
            }
            result.push_str(&cleaned);
            result.push('\n');
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to parse in grouped mode
    fn parse_grouped(input: &str) -> Vec<BlockNode> {
        parse(input, ParseMode::Grouped).blocks
    }

    fn parse_editable(input: &str) -> Vec<BlockNode> {
        parse(input, ParseMode::Editable).blocks
    }

    // MARK: - Headings

    #[test]
    fn h1_heading() {
        let blocks = parse_grouped("# Hello World");
        assert_eq!(blocks.len(), 1);
        assert!(
            matches!(&blocks[0].kind, BlockKind::Heading { level: 1, text } if text == "Hello World")
        );
    }

    #[test]
    fn h2_heading() {
        let blocks = parse_grouped("## Sub Heading");
        assert_eq!(blocks.len(), 1);
        assert!(
            matches!(&blocks[0].kind, BlockKind::Heading { level: 2, text } if text == "Sub Heading")
        );
    }

    #[test]
    fn h3_heading() {
        let blocks = parse_grouped("### Small Heading");
        assert_eq!(blocks.len(), 1);
        assert!(
            matches!(&blocks[0].kind, BlockKind::Heading { level: 3, text } if text == "Small Heading")
        );
    }

    #[test]
    fn heading_requires_space() {
        let blocks = parse_grouped("#NoSpace");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0].kind, BlockKind::Paragraph { .. }));
    }

    #[test]
    fn all_heading_levels() {
        for level in 1..=6u8 {
            let input = format!("{} Heading {}", "#".repeat(level as usize), level);
            let blocks = parse_grouped(&input);
            assert_eq!(blocks.len(), 1);
            if let BlockKind::Heading { level: l, text: _ } = &blocks[0].kind {
                assert_eq!(*l, level);
            } else {
                panic!("Expected heading for level {}", level);
            }
        }
    }

    // MARK: - Code blocks

    #[test]
    fn code_block_with_language() {
        let blocks = parse_grouped("```swift\nlet x = 1\n```");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::CodeBlock { language, code } = &blocks[0].kind {
            assert_eq!(language.as_deref(), Some("swift"));
            assert_eq!(code, "let x = 1");
        } else {
            panic!("Expected code block");
        }
    }

    #[test]
    fn code_block_no_language() {
        let blocks = parse_grouped("```\nsome code\n```");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::CodeBlock { language, code } = &blocks[0].kind {
            assert!(language.is_none());
            assert_eq!(code, "some code");
        } else {
            panic!("Expected code block");
        }
    }

    #[test]
    fn code_block_language_first_word_only() {
        let blocks = parse_grouped("```python3 interactive\nprint('hi')\n```");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::CodeBlock { language, .. } = &blocks[0].kind {
            assert_eq!(language.as_deref(), Some("python3"));
        } else {
            panic!("Expected code block");
        }
    }

    #[test]
    fn unclosed_code_block() {
        let blocks = parse_grouped("```\nno closing fence");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::CodeBlock { code, .. } = &blocks[0].kind {
            assert_eq!(code, "no closing fence");
        } else {
            panic!("Expected code block");
        }
    }

    // MARK: - Tables

    #[test]
    fn simple_table() {
        let input = "| A | B |\n| --- | --- |\n| 1 | 2 |";
        let blocks = parse_grouped(input);
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Table { headers, rows, .. } = &blocks[0].kind {
            assert_eq!(headers, &["A", "B"]);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0], &["1", "2"]);
        } else {
            panic!("Expected table");
        }
    }

    #[test]
    fn table_with_alignments() {
        let input = "| Left | Center | Right |\n| :--- | :---: | ---: |\n| a | b | c |";
        let blocks = parse_grouped(input);
        if let BlockKind::Table { alignments, .. } = &blocks[0].kind {
            assert_eq!(
                alignments,
                &[
                    ColumnAlignment::Left,
                    ColumnAlignment::Center,
                    ColumnAlignment::Right
                ]
            );
        } else {
            panic!("Expected table");
        }
    }

    #[test]
    fn table_column_limit_enforced() {
        // A 3-column table should parse normally
        let small = "| A | B | C |\n| --- | --- | --- |\n| 1 | 2 | 3 |";
        let blocks = parse_grouped(small);
        assert!(matches!(&blocks[0].kind, BlockKind::Table { .. }));

        // Verify the constant is 20
        assert_eq!(super::MAX_TABLE_COLUMNS, 20);
    }

    // MARK: - Horizontal rule

    #[test]
    fn hr_dashes() {
        let blocks = parse_grouped("---");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0].kind, BlockKind::HorizontalRule));
    }

    #[test]
    fn hr_stars() {
        let blocks = parse_grouped("***");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0].kind, BlockKind::HorizontalRule));
    }

    #[test]
    fn hr_underscores() {
        let blocks = parse_grouped("___");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0].kind, BlockKind::HorizontalRule));
    }

    // MARK: - Blockquote

    #[test]
    fn blockquote_single_line() {
        let blocks = parse_grouped("> Hello");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Blockquote { text } = &blocks[0].kind {
            assert_eq!(text, "Hello");
        } else {
            panic!("Expected blockquote");
        }
    }

    #[test]
    fn blockquote_multiline() {
        let blocks = parse_grouped("> line 1\n> line 2");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Blockquote { text } = &blocks[0].kind {
            assert_eq!(text, "line 1\nline 2");
        } else {
            panic!("Expected blockquote");
        }
    }

    // MARK: - Checkbox

    #[test]
    fn checkbox_unchecked() {
        let blocks = parse_grouped("- [ ] task");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Checkbox { checked, text } = &blocks[0].kind {
            assert!(!checked);
            assert_eq!(text, "task");
        } else {
            panic!("Expected checkbox");
        }
    }

    #[test]
    fn checkbox_checked() {
        let blocks = parse_grouped("- [x] done");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Checkbox { checked, text } = &blocks[0].kind {
            assert!(checked);
            assert_eq!(text, "done");
        } else {
            panic!("Expected checkbox");
        }
    }

    // MARK: - Lists

    #[test]
    fn unordered_list() {
        let blocks = parse_grouped("- first\n- second\n- third");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::BulletList { items } = &blocks[0].kind {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0].text, "first");
            assert_eq!(items[1].text, "second");
            assert_eq!(items[2].text, "third");
        } else {
            panic!("Expected bullet list");
        }
    }

    #[test]
    fn ordered_list() {
        let blocks = parse_grouped("1. first\n2. second");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::OrderedList { items, .. } = &blocks[0].kind {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0].text, "first");
            assert_eq!(items[1].text, "second");
        } else {
            panic!("Expected ordered list");
        }
    }

    // MARK: - Editable mode

    #[test]
    fn editable_bullet_items() {
        let blocks = parse_editable("- first\n- second");
        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[0].kind, BlockKind::BulletItem { text } if text == "first"));
        assert!(matches!(&blocks[1].kind, BlockKind::BulletItem { text } if text == "second"));
    }

    #[test]
    fn editable_numbered_items() {
        let blocks = parse_editable("1. first\n2. second");
        assert_eq!(blocks.len(), 2);
        assert!(
            matches!(&blocks[0].kind, BlockKind::NumberedItem { number: 1, text } if text == "first")
        );
        assert!(
            matches!(&blocks[1].kind, BlockKind::NumberedItem { number: 2, text } if text == "second")
        );
    }

    // MARK: - Paragraph

    #[test]
    fn simple_paragraph() {
        let blocks = parse_grouped("Hello world");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0].kind, BlockKind::Paragraph { text } if text == "Hello world"));
    }

    #[test]
    fn multiline_paragraph() {
        let blocks = parse_grouped("Line one\nLine two");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Paragraph { text } = &blocks[0].kind {
            assert_eq!(text, "Line one\nLine two");
        } else {
            panic!("Expected paragraph");
        }
    }

    // MARK: - Empty

    #[test]
    fn empty_line() {
        let blocks = parse_grouped("before\n\nafter");
        assert_eq!(blocks.len(), 3);
        assert!(matches!(blocks[1].kind, BlockKind::Empty));
    }

    // MARK: - Footnote definition

    #[test]
    fn footnote_def() {
        let blocks = parse_grouped("[^1]: Some footnote text");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::FootnoteDefinition { label, text } = &blocks[0].kind {
            assert_eq!(label, "1");
            assert_eq!(text, "Some footnote text");
        } else {
            panic!("Expected footnote definition");
        }
    }

    // MARK: - UTF-16 offsets

    #[test]
    fn utf16_offsets_ascii() {
        let blocks = parse_grouped("# Hello\nworld");
        assert_eq!(blocks[0].utf16_start, 0);
        assert_eq!(blocks[0].utf16_end, 8); // "# Hello\n" = 8 UTF-16 units
        assert_eq!(blocks[1].utf16_start, 8);
    }

    // MARK: - Mixed content

    #[test]
    fn mixed_document() {
        let input = "# Title\n\nSome text\n\n- item 1\n- item 2\n\n> quote\n\n---";
        let blocks = parse_grouped(input);
        assert!(matches!(
            &blocks[0].kind,
            BlockKind::Heading { level: 1, .. }
        ));
        assert!(matches!(blocks[1].kind, BlockKind::Empty));
        assert!(matches!(&blocks[2].kind, BlockKind::Paragraph { .. }));
        assert!(matches!(blocks[3].kind, BlockKind::Empty));
        assert!(matches!(&blocks[4].kind, BlockKind::BulletList { .. }));
        assert!(matches!(blocks[5].kind, BlockKind::Empty));
        assert!(matches!(&blocks[6].kind, BlockKind::Blockquote { .. }));
        assert!(matches!(blocks[7].kind, BlockKind::Empty));
        assert!(matches!(blocks[8].kind, BlockKind::HorizontalRule));
    }

    // MARK: - Wiki links

    #[test]
    fn extract_wiki_links_basic() {
        let links = extract_wiki_links("see [[Note One]] and [[Note Two]]");
        assert_eq!(links, vec!["Note One", "Note Two"]);
    }

    #[test]
    fn extract_wiki_links_skips_code() {
        let links = extract_wiki_links("text `[[not a link]]` and [[real link]]");
        assert_eq!(links, vec!["real link"]);
    }

    #[test]
    fn extract_wiki_links_skips_code_block() {
        let links = extract_wiki_links("text\n```\n[[not a link]]\n```\n[[real]]");
        assert_eq!(links, vec!["real"]);
    }

    #[test]
    fn extract_wiki_link_alias_uses_target() {
        // `[[Project Apollo|the moon shot]]` — the target (pre-pipe) is the
        // backlink anchor; the post-pipe text is just display.
        let links = extract_wiki_links("see [[Project Apollo|the moon shot]] today");
        assert_eq!(links, vec!["Project Apollo"]);
    }

    #[test]
    fn extract_wiki_link_alias_mixed_with_bare() {
        let links = extract_wiki_links("[[Foo|first alias]] and [[Bar]] and [[Foo|second alias]]");
        // Each `[[...]]` yields its target; dedup happens at the document level
        // (see `extract_wiki_links_from_doc`), not in this per-segment helper.
        assert_eq!(links, vec!["Foo", "Bar", "Foo"]);
    }

    #[test]
    fn extract_wiki_link_empty_target_with_alias_ignored() {
        // `[[|alias]]` has empty target and must not produce a backlink.
        let links = extract_wiki_links("garbage [[|only alias]] end");
        assert!(
            links.is_empty(),
            "Empty target should produce no backlink, got {:?}",
            links
        );
    }

    // MARK: - Toggle checkbox

    #[test]
    fn toggle_checkbox_check() {
        let result = toggle_checkbox("- [ ] task", 0);
        assert_eq!(result, "- [x] task");
    }

    #[test]
    fn toggle_checkbox_uncheck() {
        let result = toggle_checkbox("- [x] task", 0);
        assert_eq!(result, "- [ ] task");
    }

    // MARK: - Setext headings (CommonMark §4.3)

    #[test]
    fn setext_h1_equals() {
        let blocks = parse_grouped("Title\n=====");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Heading { level, text } = &blocks[0].kind {
            assert_eq!(*level, 1);
            assert_eq!(text, "Title");
        } else {
            panic!("Expected setext H1, got {:?}", blocks[0].kind);
        }
    }

    #[test]
    fn setext_h2_dashes() {
        let blocks = parse_grouped("Subtitle\n---");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Heading { level, text } = &blocks[0].kind {
            assert_eq!(*level, 2);
            assert_eq!(text, "Subtitle");
        } else {
            panic!("Expected setext H2, got {:?}", blocks[0].kind);
        }
    }

    #[test]
    fn setext_single_equals_is_h1() {
        // CommonMark: any number of = (≥1) makes a level-1 setext heading.
        let blocks = parse_grouped("Hi\n=");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(
            &blocks[0].kind,
            BlockKind::Heading { level: 1, .. }
        ));
    }

    #[test]
    fn setext_with_leading_spaces_in_underline() {
        // 0-3 leading spaces on the underline are allowed.
        let blocks = parse_grouped("Title\n   ====");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(
            &blocks[0].kind,
            BlockKind::Heading { level: 1, .. }
        ));
    }

    #[test]
    fn setext_underline_with_trailing_whitespace() {
        let blocks = parse_grouped("Title\n===   ");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(
            &blocks[0].kind,
            BlockKind::Heading { level: 1, .. }
        ));
    }

    #[test]
    fn setext_h2_wins_over_thematic_break() {
        // `Foo\n---` — the `---` looks like a thematic break, but a preceding
        // paragraph means it's a setext H2 (CommonMark spec resolution).
        let blocks = parse_grouped("Foo\n---");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(
            &blocks[0].kind,
            BlockKind::Heading { level: 2, .. }
        ));
    }

    #[test]
    fn setext_heading_followed_by_paragraph() {
        let blocks = parse_grouped("Foo\n---\nbar");
        assert_eq!(blocks.len(), 2);
        assert!(matches!(
            &blocks[0].kind,
            BlockKind::Heading { level: 2, .. }
        ));
        assert!(matches!(&blocks[1].kind, BlockKind::Paragraph { .. }));
    }

    #[test]
    fn setext_after_blank_line_is_thematic_break() {
        // Blank line breaks the paragraph; `---` then becomes a thematic break.
        let blocks = parse_grouped("Foo\n\n---");
        // [Paragraph "Foo", Empty, HorizontalRule]
        assert_eq!(blocks.len(), 3);
        assert!(matches!(&blocks[0].kind, BlockKind::Paragraph { .. }));
        assert!(matches!(blocks[2].kind, BlockKind::HorizontalRule));
    }

    #[test]
    fn setext_no_match_when_no_paragraph_above() {
        // Just `===` alone has no preceding paragraph; treated as paragraph.
        let blocks = parse_grouped("===");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0].kind, BlockKind::Paragraph { .. }));
    }

    #[test]
    fn setext_multiline_paragraph_heading() {
        let blocks = parse_grouped("Foo\nBar\n===");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Heading { level, text } = &blocks[0].kind {
            assert_eq!(*level, 1);
            assert_eq!(text, "Foo\nBar");
        } else {
            panic!(
                "Expected multi-line setext heading, got {:?}",
                blocks[0].kind
            );
        }
    }

    // MARK: - Indented code blocks (CommonMark §4.4)

    #[test]
    fn indented_code_block_basic() {
        let blocks = parse_grouped("    let x = 1");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::CodeBlock { language, code } = &blocks[0].kind {
            assert!(language.is_none());
            assert_eq!(code, "let x = 1");
        } else {
            panic!("Expected indented code block, got {:?}", blocks[0].kind);
        }
    }

    #[test]
    fn indented_code_with_tab() {
        let blocks = parse_grouped("\tlet x = 1");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0].kind, BlockKind::CodeBlock { .. }));
    }

    #[test]
    fn indented_code_multi_line() {
        let blocks = parse_grouped("    line one\n    line two\n    line three");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::CodeBlock { code, .. } = &blocks[0].kind {
            assert_eq!(code, "line one\nline two\nline three");
        } else {
            panic!("Expected indented code block");
        }
    }

    #[test]
    fn indented_code_includes_internal_blank_line() {
        let blocks = parse_grouped("    line one\n\n    line three");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::CodeBlock { code, .. } = &blocks[0].kind {
            assert_eq!(code, "line one\n\nline three");
        } else {
            panic!("Expected single indented code block, got {:?}", blocks);
        }
    }

    #[test]
    fn indented_code_strips_trailing_blanks() {
        let blocks = parse_grouped("    code\n\n\nparagraph");
        // [CodeBlock "code", Empty, Empty, Paragraph]
        assert!(blocks.len() >= 2);
        if let BlockKind::CodeBlock { code, .. } = &blocks[0].kind {
            assert_eq!(code, "code");
        } else {
            panic!("Expected indented code block, got {:?}", blocks[0].kind);
        }
    }

    #[test]
    fn three_space_indent_is_paragraph_not_code() {
        // Only 4+ spaces qualify; 3 spaces is paragraph text.
        let blocks = parse_grouped("   not code");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0].kind, BlockKind::Paragraph { .. }));
    }

    #[test]
    fn indented_code_cannot_interrupt_paragraph() {
        // The indented line is a continuation of the paragraph, not a new code block.
        let blocks = parse_grouped("paragraph\n    not code");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Paragraph { text } = &blocks[0].kind {
            assert!(text.contains("not code"));
        } else {
            panic!("Expected paragraph, got {:?}", blocks[0].kind);
        }
    }

    #[test]
    fn indented_code_preserves_extra_indent() {
        // Spaces beyond the first 4 belong to the code content.
        let blocks = parse_grouped("        deep indent");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::CodeBlock { code, .. } = &blocks[0].kind {
            assert_eq!(code, "    deep indent");
        } else {
            panic!("Expected indented code block");
        }
    }

    // MARK: - Source preservation regressions
    //
    // The editor renders source text verbatim; the SwiftUI preview defers to
    // `AttributedString(markdown:)` (Apple's CommonMark parser). For both
    // paths to render correctly, the parser must preserve the raw bytes that
    // CommonMark assigns special meaning to — trailing-space hard breaks,
    // backslash hard breaks, and HTML entity references.

    #[test]
    fn paragraph_preserves_two_space_hard_break() {
        // Two trailing spaces + newline — `AttributedString(markdown:)` treats
        // this as a hard line break. We must keep both spaces in the joined text.
        let input = "line one  \nline two";
        let blocks = parse_grouped(input);
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Paragraph { text } = &blocks[0].kind {
            assert_eq!(
                text, "line one  \nline two",
                "Trailing spaces must survive line join"
            );
        } else {
            panic!("Expected paragraph, got {:?}", blocks[0].kind);
        }
    }

    #[test]
    fn paragraph_preserves_backslash_hard_break() {
        // Trailing backslash + newline — also a CommonMark hard line break.
        let input = "line one\\\nline two";
        let blocks = parse_grouped(input);
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Paragraph { text } = &blocks[0].kind {
            assert_eq!(text, "line one\\\nline two");
        } else {
            panic!("Expected paragraph, got {:?}", blocks[0].kind);
        }
    }

    #[test]
    fn paragraph_preserves_html_entities() {
        // Entities pass through unchanged so the preview path can decode them.
        let input = "5 &amp; 6 &lt; 10 &#x2603;";
        let blocks = parse_grouped(input);
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Paragraph { text } = &blocks[0].kind {
            assert_eq!(text, "5 &amp; 6 &lt; 10 &#x2603;");
        } else {
            panic!("Expected paragraph, got {:?}", blocks[0].kind);
        }
    }

    #[test]
    fn setext_underline_strips_inline_scan() {
        // The underline must not be inline-scanned (otherwise `==` triggers highlight).
        let doc = parse("Title\n===", ParseMode::Grouped);
        assert_eq!(doc.blocks.len(), 1);
        // No inline spans should be created from the `===` underline.
        let highlights: Vec<_> = doc.blocks[0]
            .inline_spans
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::Highlight))
            .collect();
        assert!(
            highlights.is_empty(),
            "Setext underline must be excluded from inline scan"
        );
    }

    // MARK: - Callouts (Obsidian-style)

    #[test]
    fn callout_note_no_title_no_body() {
        let blocks = parse_grouped("> [!note]");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Callout { kind, title, text } = &blocks[0].kind {
            assert_eq!(*kind, CalloutKind::Note);
            assert!(title.is_none());
            assert_eq!(text, "");
        } else {
            panic!("Expected Callout, got {:?}", blocks[0].kind);
        }
    }

    #[test]
    fn callout_tip_with_title() {
        let blocks = parse_grouped("> [!tip] Pro tip for you");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Callout { kind, title, .. } = &blocks[0].kind {
            assert_eq!(*kind, CalloutKind::Tip);
            assert_eq!(title.as_deref(), Some("Pro tip for you"));
        } else {
            panic!("Expected Callout");
        }
    }

    #[test]
    fn callout_warning_with_body() {
        let blocks = parse_grouped("> [!warning]\n> Watch out\n> for bears");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Callout { kind, title, text } = &blocks[0].kind {
            assert_eq!(*kind, CalloutKind::Warning);
            assert!(title.is_none());
            assert_eq!(text, "Watch out\nfor bears");
        } else {
            panic!("Expected Callout");
        }
    }

    #[test]
    fn callout_important_with_title_and_body() {
        let blocks = parse_grouped("> [!important] Read carefully\n> This matters");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Callout { kind, title, text } = &blocks[0].kind {
            assert_eq!(*kind, CalloutKind::Important);
            assert_eq!(title.as_deref(), Some("Read carefully"));
            assert_eq!(text, "This matters");
        } else {
            panic!("Expected Callout");
        }
    }

    #[test]
    fn callout_caution_case_insensitive_kind() {
        let blocks = parse_grouped("> [!CAUTION] Big deal");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(
            &blocks[0].kind,
            BlockKind::Callout {
                kind: CalloutKind::Caution,
                ..
            }
        ));
    }

    #[test]
    fn callout_all_five_kinds() {
        for (name, expected) in [
            ("note", CalloutKind::Note),
            ("tip", CalloutKind::Tip),
            ("warning", CalloutKind::Warning),
            ("important", CalloutKind::Important),
            ("caution", CalloutKind::Caution),
        ] {
            let input = format!("> [!{}]", name);
            let blocks = parse_grouped(&input);
            assert_eq!(blocks.len(), 1, "kind={}", name);
            if let BlockKind::Callout { kind, .. } = &blocks[0].kind {
                assert_eq!(*kind, expected, "kind={}", name);
            } else {
                panic!("Expected Callout for {}", name);
            }
        }
    }

    #[test]
    fn unknown_callout_kind_degrades_to_blockquote() {
        let blocks = parse_grouped("> [!nonsense] Not a callout\n> just a quote");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0].kind, BlockKind::Blockquote { .. }));
    }

    #[test]
    fn regular_blockquote_still_works() {
        let blocks = parse_grouped("> Just a quote\n> with two lines");
        assert_eq!(blocks.len(), 1);
        if let BlockKind::Blockquote { text } = &blocks[0].kind {
            assert_eq!(text, "Just a quote\nwith two lines");
        } else {
            panic!("Expected Blockquote, got {:?}", blocks[0].kind);
        }
    }

    #[test]
    fn three_dashes_at_bof_is_thematic_break() {
        // Previously the parser carved a YAML frontmatter block out of `---`
        // fences at BOF. That support has been removed; a leading `---` now
        // behaves as a normal thematic break, and a second `---` on a later
        // line is either a thematic break or a setext underline per spec.
        let blocks = parse_grouped("---\ntitle: example\n---\nbody");
        assert!(matches!(&blocks[0].kind, BlockKind::HorizontalRule));
        // `title: example` on line 2 followed by `---` on line 3 still forms
        // a setext H2 (the ambiguity resolved in setext's favor).
        assert!(blocks
            .iter()
            .any(|b| matches!(&b.kind, BlockKind::Heading { level: 2, .. })));
    }

    #[test]
    fn callout_fold_markers_stripped() {
        // `[!note]+` and `[!note]-` both parse as Note; the +/- is consumed.
        let open = parse_grouped("> [!note]+ Expanded");
        assert!(matches!(
            &open[0].kind,
            BlockKind::Callout {
                kind: CalloutKind::Note,
                ..
            }
        ));
        if let BlockKind::Callout { title, .. } = &open[0].kind {
            assert_eq!(title.as_deref(), Some("Expanded"));
        }
        let closed = parse_grouped("> [!note]- Collapsed");
        assert!(matches!(
            &closed[0].kind,
            BlockKind::Callout {
                kind: CalloutKind::Note,
                ..
            }
        ));
    }

    // MARK: - Regression checks across the five delight features

    #[test]
    fn callout_works_in_editable_mode() {
        let blocks = parse_editable("> [!tip] Hey\n> body line");
        assert!(blocks
            .iter()
            .any(|b| matches!(&b.kind, BlockKind::Callout { .. })));
    }

    #[test]
    fn double_percent_in_text_without_closer_is_not_comment() {
        // `50%%` alone is a single unmatched `%%` — no comment should form.
        let blocks = parse_grouped("reached 50%% utilization today");
        assert!(
            blocks[0]
                .inline_spans
                .iter()
                .all(|s| !matches!(s.kind, crate::ast::InlineKind::Comment)),
            "Unmatched %% must not create a Comment span"
        );
    }

    #[test]
    fn hex_color_does_not_eat_trailing_markdown() {
        // `#ff0000 **bold**` — hex span claims only the hex token; bold still parses.
        let blocks = parse_grouped("#ff0000 **bold**");
        let spans = &blocks[0].inline_spans;
        assert!(spans
            .iter()
            .any(|s| matches!(&s.kind, crate::ast::InlineKind::HexColor { .. })));
        assert!(spans
            .iter()
            .any(|s| matches!(&s.kind, crate::ast::InlineKind::Bold)));
    }

    #[test]
    fn wiki_alias_inside_blockquote_works() {
        let blocks = parse_grouped("> see [[Target|Display]] today");
        if let BlockKind::Blockquote { .. } = &blocks[0].kind {
            assert!(blocks[0]
                .inline_spans
                .iter()
                .any(|s| matches!(s.kind, crate::ast::InlineKind::WikiLink)));
        } else {
            panic!("Expected Blockquote");
        }
    }

    // MARK: - Image marker

    #[test]
    fn image_marker_uppercase_uuid_recognised() {
        let blocks = parse_editable("![](ember:DEBD1746-CBBB-4A33-9CB0-4B1A5D956200)\n");
        assert_eq!(
            blocks.len(),
            1,
            "marker should not be merged with empty trailing"
        );
        match &blocks[0].kind {
            BlockKind::ImageMarker { uuid } => {
                assert_eq!(uuid, "DEBD1746-CBBB-4A33-9CB0-4B1A5D956200");
            }
            other => panic!("expected ImageMarker, got {:?}", other),
        }
    }

    #[test]
    fn image_marker_lowercase_uuid_recognised() {
        let blocks = parse_editable("![](ember:debd1746-cbbb-4a33-9cb0-4b1a5d956200)\n");
        assert!(matches!(&blocks[0].kind, BlockKind::ImageMarker { .. }));
    }

    #[test]
    fn image_marker_grouped_mode_recognised() {
        // Grouped mode is what preview surfaces consume; same dispatch rule.
        let blocks = parse_grouped("![](ember:DEBD1746-CBBB-4A33-9CB0-4B1A5D956200)");
        assert!(matches!(&blocks[0].kind, BlockKind::ImageMarker { .. }));
    }

    #[test]
    fn image_marker_inside_paragraph_falls_through() {
        // Inline form (text on same line) is not a block marker — must
        // remain a Paragraph so editor surfaces don't accidentally tear
        // out a chunk of the user's prose.
        let blocks =
            parse_editable("look at this ![](ember:DEBD1746-CBBB-4A33-9CB0-4B1A5D956200) inline\n");
        assert!(matches!(&blocks[0].kind, BlockKind::Paragraph { .. }));
    }

    #[test]
    fn image_marker_malformed_uuid_falls_through() {
        let blocks = parse_editable("![](ember:not-a-uuid)\n");
        assert!(matches!(&blocks[0].kind, BlockKind::Paragraph { .. }));
    }

    #[test]
    fn image_marker_wrong_scheme_falls_through() {
        let blocks = parse_editable("![](other:DEBD1746-CBBB-4A33-9CB0-4B1A5D956200)\n");
        assert!(matches!(&blocks[0].kind, BlockKind::Paragraph { .. }));
    }

    #[test]
    fn image_marker_in_document_alongside_other_blocks() {
        let src = "# Title\n\n![](ember:DEBD1746-CBBB-4A33-9CB0-4B1A5D956200)\n\nNext paragraph\n";
        let blocks = parse_editable(src);
        let kinds: Vec<&BlockKind> = blocks.iter().map(|b| &b.kind).collect();
        assert!(matches!(kinds[0], BlockKind::Heading { .. }));
        // Empty line, then marker, then empty line, then paragraph — exact
        // count varies by mode but the marker must be present.
        assert!(blocks
            .iter()
            .any(|b| matches!(b.kind, BlockKind::ImageMarker { .. })));
        assert!(blocks
            .iter()
            .any(|b| matches!(b.kind, BlockKind::Paragraph { .. })));
    }
}
