//! Optional semantic annotations over Markdown's eligible inline source.
use crate::{ast::*, parser, utf16::Utf16Map};
use unicode_segmentation::UnicodeSegmentation;

const COMPLETION_LIMIT: usize = 160;

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticCompletion {
    pub is_tag: bool,
    pub query: String,
    pub utf16_start: u32,
    pub utf16_end: u32,
}

fn eligible(kind: &BlockKind) -> bool {
    !matches!(
        kind,
        BlockKind::CodeBlock { .. }
            | BlockKind::RawHtml { .. }
            | BlockKind::Math { .. }
            | BlockKind::MermaidDiagram { .. }
            | BlockKind::HorizontalRule
            | BlockKind::Empty
            | BlockKind::ImageMarker { .. }
    )
}

pub(crate) fn annotate(blocks: &mut [BlockNode], source: &str, map: &Utf16Map) {
    for block in blocks {
        if !eligible(&block.kind) {
            continue;
        }
        if let BlockKind::BulletList { items } | BlockKind::OrderedList { items, .. } =
            &mut block.kind
        {
            for item in items {
                annotate_range(&item.text, 0, item.text.len(), 0, &mut item.inline_spans);
            }
        }
        if matches!(block.kind, BlockKind::Table { .. }) {
            for cell in &mut block.table_cells {
                let start = map.utf16_to_byte(cell.utf16_start, source.as_bytes()) as usize;
                let end = map.utf16_to_byte(cell.utf16_end, source.as_bytes()) as usize;
                annotate_range(source, start, end, cell.utf16_start, &mut cell.inline_spans);
            }
        } else {
            annotate_range(
                source,
                block.byte_start as usize,
                block.byte_end as usize,
                block.utf16_start,
                &mut block.inline_spans,
            );
        }
    }
}

fn boundary(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '(' | '[' | '{' | '"' | '\'' | '“' | '‘' | '«' | '「' | '（'
        )
}

fn word(grapheme: &str) -> bool {
    grapheme.chars().next().is_some_and(char::is_alphanumeric)
}

fn protected_mask(text: &str, base: u32, spans: &[InlineSpan]) -> Vec<bool> {
    let mut units = vec![false; text.encode_utf16().count()];
    let mut mark = |start: u32, end: u32| {
        let start = start.saturating_sub(base) as usize;
        let end = (end.saturating_sub(base) as usize).min(units.len());
        if start < end {
            units[start..end].fill(true);
        }
    };
    for span in spans {
        match span.kind {
            InlineKind::HexColor { .. } | InlineKind::Tag { .. } | InlineKind::Mention { .. } => {}
            InlineKind::InlineCode
            | InlineKind::Link { .. }
            | InlineKind::AutoLink { .. }
            | InlineKind::WikiLink
            | InlineKind::FootnoteRef
            | InlineKind::Comment
            | InlineKind::Math { .. } => {
                mark(span.utf16_start, span.utf16_end);
            }
            _ => {
                mark(span.utf16_start, span.content_utf16_start);
                mark(span.content_utf16_end, span.utf16_end);
            }
        }
    }
    let mut mask = vec![false; text.len()];
    let mut unit = 0;
    for (byte, ch) in text.char_indices() {
        if units.get(unit).copied().unwrap_or(false) {
            mask[byte..byte + ch.len_utf8()].fill(true);
        }
        unit += ch.len_utf16();
    }
    crate::inline::protect_html(text.as_bytes(), &mut mask);
    mask
}

pub(crate) fn annotate_range(
    source: &str,
    start: usize,
    end: usize,
    base: u32,
    spans: &mut Vec<InlineSpan>,
) {
    let text = &source[start..end];
    let mask = protected_mask(text, base, spans);
    let mut positions = vec![base; text.len() + 1];
    let mut pos = base;
    for (i, ch) in text.char_indices() {
        positions[i] = pos;
        pos += ch.len_utf16() as u32;
    }
    positions[text.len()] = pos;
    let parts: Vec<(usize, &str)> = text.grapheme_indices(true).collect();
    // The opt-in grammar classifies complete candidates, never a color prefix.
    spans.retain(|s| !matches!(s.kind, InlineKind::HexColor { .. }));
    let mut i = 0;
    while i < parts.len() {
        let (begin, marker) = parts[i];
        if !matches!(marker, "#" | "@")
            || mask[begin]
            || (i > 0
                && !boundary(parts[i - 1].1.chars().last().unwrap_or(' '))
                && !spans.iter().any(|s| {
                    s.content_utf16_start == positions[begin]
                        && s.utf16_start < s.content_utf16_start
                        && !matches!(
                            s.kind,
                            InlineKind::InlineCode
                                | InlineKind::Link { .. }
                                | InlineKind::AutoLink { .. }
                                | InlineKind::WikiLink
                                | InlineKind::Math { .. }
                                | InlineKind::Comment
                                | InlineKind::FootnoteRef
                        )
                }))
        {
            i += 1;
            continue;
        }
        // Formatting delimiters may precede a token; escaped openers may not.
        let slashes = text[..begin]
            .bytes()
            .rev()
            .take_while(|b| *b == b'\\')
            .count();
        if slashes % 2 == 1 {
            i += 1;
            continue;
        }
        let is_tag = marker == "#";
        let mut j = i + 1;
        let mut last_word_end = j;
        let mut has_space = false;
        let mut closed = false;
        let mut initial_end = None;
        let mut needs_word = true;
        while j < parts.len() {
            let (offset, part) = parts[j];
            if mask[offset] {
                break;
            }
            if word(part) {
                needs_word = false;
                last_word_end = j + 1;
            } else if matches!(part, "-" | "_") || (is_tag && part == "/") {
                if needs_word {
                    break;
                }
                needs_word = true;
            } else if is_tag && part == " " && !needs_word {
                initial_end.get_or_insert(j);
                has_space = true;
                needs_word = true;
            } else if is_tag && part == "#" && !needs_word {
                let next = parts.get(j + 1).map(|p| p.1);
                if next.is_none_or(|g| !word(g) && !matches!(g, "-" | "_" | "/" | "#" | "@")) {
                    closed = true;
                }
                break;
            } else {
                break;
            }
            j += 1;
        }
        let name_end = if closed {
            j
        } else if has_space {
            initial_end.unwrap_or(last_word_end)
        } else {
            last_word_end
        };
        if name_end <= i + 1 {
            i += 1;
            continue;
        }
        let byte_end = parts.get(name_end).map_or(text.len(), |p| p.0);
        let name = &text[begin + 1..byte_end];
        let full_end = if closed { byte_end + 1 } else { byte_end };
        let kind = if is_tag
            && matches!(name.len(), 3 | 4 | 6 | 8)
            && name.bytes().all(|b| b.is_ascii_hexdigit())
        {
            InlineKind::HexColor {
                hex: crate::inline::normalize_hex(name, name.len()),
            }
        } else if is_tag {
            InlineKind::Tag {
                name: name.to_owned(),
            }
        } else {
            InlineKind::Mention {
                name: name.to_owned(),
            }
        };
        spans.push(InlineSpan {
            kind,
            utf16_start: positions[begin],
            utf16_end: positions[full_end],
            content_utf16_start: positions[begin + 1],
            content_utf16_end: positions[byte_end],
        });
        i = name_end + usize::from(closed);
    }
    spans.sort_by_key(|s| (s.utf16_start, std::cmp::Reverse(s.utf16_end)));
}

/// Completion is read-only and bounded to a short query. The parsed structure
/// excludes code, links and other opaque contexts even for incomplete tokens.
pub fn completion(
    source: &str,
    cursor: u32,
    options: &parser::ParseOptions,
) -> Option<SemanticCompletion> {
    if !options.semantic_tokens {
        return None;
    }
    let map = Utf16Map::build(source.as_bytes());
    if cursor > map.total_utf16_len {
        return None;
    }
    let byte = map.utf16_to_byte(cursor, source.as_bytes()) as usize;
    if map.byte_to_utf16(byte as u32, source.as_bytes()) != cursor {
        return None;
    }
    let before = &source[..byte];
    let mut begin = None;
    for (count, (i, ch)) in before.char_indices().rev().enumerate() {
        if count > COMPLETION_LIMIT || ch == '\n' || ch == '\r' {
            break;
        }
        if matches!(ch, '#' | '@') {
            begin = Some((i, ch));
            break;
        }
        // Validate complete graphemes below, including combining marks.
        if ch.is_control() {
            break;
        }
    }
    let (begin, marker) = begin?;
    let query = &source[begin + 1..byte];
    let mut needs_word = true;
    for part in query.graphemes(true) {
        if word(part) {
            needs_word = false;
        } else if !needs_word
            && (matches!(part, "-" | "_") || (marker == '#' && matches!(part, "/" | " ")))
        {
            needs_word = true;
        } else {
            return None;
        }
    }
    let next = source[byte..].graphemes(true).next();
    if next.is_some_and(|g| word(g) || matches!(g, "-" | "_" | "/")) {
        return None;
    }
    let replacement_end =
        cursor + u32::from(marker == '#' && !query.is_empty() && next == Some("#"));
    // A temporary word admits a bare opener without changing the document.
    let mut probe = source.to_owned();
    probe.insert_str(byte, if marker == '#' { "z# " } else { "z " });
    let doc = parser::parse_with_options(&probe, ParseMode::Editable, options);
    let start = map.byte_to_utf16(begin as u32, source.as_bytes());
    let found = doc
        .blocks
        .iter()
        .flat_map(|b| {
            b.inline_spans
                .iter()
                .chain(b.table_cells.iter().flat_map(|c| c.inline_spans.iter()))
        })
        .any(|s| {
            s.utf16_start == start
                && s.content_utf16_end > cursor
                && matches!(s.kind, InlineKind::Tag { .. } | InlineKind::Mention { .. })
        });
    if !found {
        return None;
    }
    if marker == '#'
        && matches!(query.len(), 3 | 4 | 6 | 8)
        && query.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    Some(SemanticCompletion {
        is_tag: marker == '#',
        query: query.to_owned(),
        utf16_start: start,
        utf16_end: replacement_end,
    })
}
