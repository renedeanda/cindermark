//! Inline span parser using a delimiter-run algorithm.
//!
//! Processes the text content of each block to identify inline formatting:
//! bold, italic, strikethrough, highlight, underline, links, wiki links, etc.
//!
//! Priority order (matching `MarkdownTextStyling.swift`):
//! 1. Inline code (backticks) — highest, content is literal
//! 2. Wiki links [[...]]
//! 3. Markdown links [text](url)
//! 4. Bold-italic ***...***
//! 5. Bold **...**
//! 6. Italic *...*
//! 7. Strikethrough ~~...~~
//! 8. Colored highlight ==emoji...==, then default highlight ==...==
//! 9. HTML underline <u>...</u>
//! 10. Tilde underline ~...~
//! 11. Footnote refs [^label]

use crate::ast::*;
use crate::utf16::Utf16Map;

/// Run inline span parsing on all blocks in the document.
pub fn parse_inline_spans(blocks: &mut [BlockNode], source: &[u8], utf16_map: &Utf16Map) {
    for block in blocks.iter_mut() {
        match &block.kind {
            BlockKind::CodeBlock { .. }
            | BlockKind::HorizontalRule
            | BlockKind::Empty
            | BlockKind::Table { .. }
            | BlockKind::ImageMarker { .. } => continue,
            _ => {}
        }

        // Get the text content range for inline parsing
        let (text_byte_start, text_byte_end) = inline_text_range(block, source);
        if text_byte_start >= text_byte_end {
            continue;
        }

        let text_bytes = &source[text_byte_start..text_byte_end];
        let spans = parse_spans(text_bytes, text_byte_start, source, utf16_map);
        block.inline_spans = spans;
    }
}

/// Get the byte range of the text content that should be scanned for inline formatting.
/// For headings, this is after the "# " marker. For blockquotes, after "> ". Etc.
fn inline_text_range(block: &BlockNode, source: &[u8]) -> (usize, usize) {
    let start = block.byte_start as usize;
    let end = block.byte_end as usize;

    // Strip trailing newline
    let mut end = if end > start && end <= source.len() && source[end - 1] == b'\n' {
        end - 1
    } else {
        end
    };

    // Setext heading: the trailing `===` / `---` underline is structural and must
    // not be inline-scanned (otherwise `==` triggers highlight, `--` is harmless
    // but consistent stripping is cleaner).
    if matches!(block.kind, BlockKind::Heading { .. }) && start <= end && end <= source.len() {
        let region = &source[start..end];
        if let Some(last_nl) = region.iter().rposition(|&b| b == b'\n') {
            let last_line_start = start + last_nl + 1;
            if last_line_start < end {
                let last_line = std::str::from_utf8(&source[last_line_start..end]).unwrap_or("");
                if crate::parser::parse_setext_underline(last_line).is_some() {
                    end = last_line_start.saturating_sub(1);
                    if end < start {
                        end = start;
                    }
                }
            }
        }
    }

    if let Some(marker) = &block.list_marker {
        return (marker.content_byte_start as usize, end);
    }

    (start, end)
}

/// A claimed range that higher-priority spans have taken.
struct ClaimedRanges {
    ranges: Vec<(usize, usize)>, // byte ranges
}

impl ClaimedRanges {
    fn new() -> Self {
        Self { ranges: Vec::new() }
    }

    fn claim(&mut self, start: usize, end: usize) {
        self.ranges.push((start, end));
    }

    fn overlaps(&self, start: usize, end: usize) -> bool {
        self.ranges.iter().any(|&(rs, re)| start < re && end > rs)
    }
}

/// Parse all inline spans from a text byte range.
pub(crate) fn parse_spans(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
) -> Vec<InlineSpan> {
    let mut spans = Vec::new();
    let mut claimed = ClaimedRanges::new();

    // 1. Inline code (highest priority — exact-match backticks, immune to everything)
    parse_inline_code(
        text,
        byte_offset,
        source,
        utf16_map,
        &mut spans,
        &mut claimed,
    );

    // 1a. Hidden comments `%%text%%` (drafting notes that never render).
    //    Runs before wiki/markdown links so a comment containing `[[...]]` or
    //    `[label](url)` is claimed as a single Comment span. Still respects
    //    inline code: `` `%%literal%%` `` stays as code.
    parse_paired_marker(
        text,
        byte_offset,
        source,
        utf16_map,
        b"%%",
        b"%%",
        InlineKind::Comment,
        &mut spans,
        &mut claimed,
    );

    // 2. Wiki links [[...]]
    parse_wiki_links(
        text,
        byte_offset,
        source,
        utf16_map,
        &mut spans,
        &mut claimed,
    );

    // 3. Markdown links [text](url)
    parse_markdown_links(
        text,
        byte_offset,
        source,
        utf16_map,
        &mut spans,
        &mut claimed,
    );

    // 3a. Angle-bracket autolinks `<https://…>`, `<foo@bar.com>` (CommonMark §6.4)
    parse_angle_autolinks(
        text,
        byte_offset,
        source,
        utf16_map,
        &mut spans,
        &mut claimed,
    );

    // 3b. Bare URLs (https://..., http://...)
    parse_autolinks(
        text,
        byte_offset,
        source,
        utf16_map,
        &mut spans,
        &mut claimed,
    );

    // 3c. Hex color literals (`#FF0000`, `#RGB`, `#RGBA`, `#RRGGBBAA`).
    //     Runs after link scanners so `[anchor](#fragment)` stays a link.
    parse_hex_colors(
        text,
        byte_offset,
        source,
        utf16_map,
        &mut spans,
        &mut claimed,
    );

    // 4-6. Emphasis (bold-italic, bold, italic) — processed together
    parse_emphasis(
        text,
        byte_offset,
        source,
        utf16_map,
        &mut spans,
        &mut claimed,
    );

    // 7. Strikethrough ~~...~~
    parse_paired_marker(
        text,
        byte_offset,
        source,
        utf16_map,
        b"~~",
        b"~~",
        InlineKind::Strikethrough,
        &mut spans,
        &mut claimed,
    );

    // 8. Highlights ==...== (colored first, then default)
    parse_highlights(
        text,
        byte_offset,
        source,
        utf16_map,
        &mut spans,
        &mut claimed,
    );

    // 9. HTML underline <u>...</u>
    parse_html_underline(
        text,
        byte_offset,
        source,
        utf16_map,
        &mut spans,
        &mut claimed,
    );

    // 10. Tilde underline ~...~ (must not match ~~)
    parse_tilde_underline(
        text,
        byte_offset,
        source,
        utf16_map,
        &mut spans,
        &mut claimed,
    );

    // 11. Footnote refs [^label]
    parse_footnote_refs(
        text,
        byte_offset,
        source,
        utf16_map,
        &mut spans,
        &mut claimed,
    );

    // Sort by position for consistent output
    spans.sort_by_key(|s| s.utf16_start);
    spans
}

// MARK: - Inline code

/// Multi-backtick inline code per CommonMark: the opening backtick string must
/// be matched by a closing backtick string of the exact same length.
/// `code`, ``code with ` backtick``, ```code with `` inside```, etc.
fn parse_inline_code(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
    spans: &mut Vec<InlineSpan>,
    claimed: &mut ClaimedRanges,
) {
    let mut i = 0;
    while i < text.len() {
        if text[i] == b'`' {
            // CommonMark §6.1: an escaped backtick can't open an inline code span.
            if is_escaped(text, i) {
                i += 1;
                continue;
            }
            let open = i;
            // Count opening backtick run
            let mut open_count = 0;
            while i < text.len() && text[i] == b'`' {
                open_count += 1;
                i += 1;
            }
            // Search for a closing backtick run of exactly the same length
            let mut found = false;
            while i < text.len() {
                if text[i] == b'`' {
                    let close_start = i;
                    let mut close_count = 0;
                    while i < text.len() && text[i] == b'`' {
                        close_count += 1;
                        i += 1;
                    }
                    if close_count == open_count {
                        let abs_start = byte_offset + open;
                        let abs_end = byte_offset + close_start + close_count;
                        claimed.claim(abs_start, abs_end);
                        spans.push(make_span(
                            InlineKind::InlineCode,
                            abs_start,
                            abs_end,
                            abs_start + open_count,
                            byte_offset + close_start,
                            source,
                            utf16_map,
                        ));
                        found = true;
                        break;
                    }
                    // Wrong backtick count — keep searching
                } else {
                    i += 1;
                }
            }
            if !found {
                // No matching closer; the backticks are literal text
                // i is already past the opening run or at end
            }
        } else {
            i += 1;
        }
    }
}

// MARK: - Wiki links

fn parse_wiki_links(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
    spans: &mut Vec<InlineSpan>,
    claimed: &mut ClaimedRanges,
) {
    let mut i = 0;
    while i + 1 < text.len() {
        if text[i] == b'[' && text[i + 1] == b'[' {
            if is_escaped(text, i) {
                i += 1;
                continue;
            }
            let open = i;
            let content_start = i + 2;
            i += 2;
            // Scan content, tracking the first unescaped `|` (aliased display text).
            let mut pipe_pos: Option<usize> = None;
            while i + 1 < text.len() {
                if text[i] == b']' && text[i + 1] == b']' {
                    let abs_start = byte_offset + open;
                    let abs_end = byte_offset + i + 2;
                    if !claimed.overlaps(abs_start, abs_end) {
                        claimed.claim(abs_start, abs_end);
                        // Aliased: `[[target|alias]]` — content range covers `alias`
                        // only, so styling and cursor-span marker visibility treat
                        // `[[target|` as the opening marker.
                        let (content_byte_start, content_byte_end) = match pipe_pos {
                            Some(pipe) => (byte_offset + pipe + 1, byte_offset + i),
                            None => (byte_offset + content_start, byte_offset + i),
                        };
                        spans.push(make_span(
                            InlineKind::WikiLink,
                            abs_start,
                            abs_end,
                            content_byte_start,
                            content_byte_end,
                            source,
                            utf16_map,
                        ));
                    }
                    i += 2;
                    break;
                }
                if text[i] == b'|' && pipe_pos.is_none() && !is_escaped(text, i) {
                    pipe_pos = Some(i);
                }
                i += 1;
            }
        } else {
            i += 1;
        }
    }
}

// MARK: - Markdown links

fn parse_markdown_links(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
    spans: &mut Vec<InlineSpan>,
    claimed: &mut ClaimedRanges,
) {
    let mut i = 0;
    while i < text.len() {
        if text[i] == b'[' {
            // Don't match if this is [[ (wiki link) or [^ (footnote)
            if i + 1 < text.len() && (text[i + 1] == b'[' || text[i + 1] == b'^') {
                i += 1;
                continue;
            }
            if is_escaped(text, i) {
                i += 1;
                continue;
            }
            let open_bracket = i;
            i += 1;
            // Find closing ]
            let mut bracket_depth = 1;
            while i < text.len() && bracket_depth > 0 {
                if text[i] == b'[' {
                    bracket_depth += 1;
                } else if text[i] == b']' {
                    bracket_depth -= 1;
                }
                if bracket_depth > 0 {
                    i += 1;
                }
            }
            if bracket_depth != 0 {
                continue;
            }
            let close_bracket = i;
            i += 1;
            // Must be followed by (url)
            if i < text.len() && text[i] == b'(' {
                let url_open = i;
                i += 1;
                let mut paren_depth = 1;
                while i < text.len() && paren_depth > 0 {
                    if text[i] == b'(' {
                        paren_depth += 1;
                    } else if text[i] == b')' {
                        paren_depth -= 1;
                    }
                    if paren_depth > 0 {
                        i += 1;
                    }
                }
                if paren_depth == 0 {
                    let close_paren = i;
                    let abs_start = byte_offset + open_bracket;
                    let abs_end = byte_offset + close_paren + 1;
                    if !claimed.overlaps(abs_start, abs_end) {
                        let url_bytes = &text[url_open + 1..close_paren];
                        let url = std::str::from_utf8(url_bytes).unwrap_or("").to_string();
                        claimed.claim(abs_start, abs_end);
                        // Content range is the link text (between [ and ])
                        let content_start = byte_offset + open_bracket + 1;
                        let content_end = byte_offset + close_bracket;
                        spans.push(make_span(
                            InlineKind::Link { url },
                            abs_start,
                            abs_end,
                            content_start,
                            content_end,
                            source,
                            utf16_map,
                        ));
                    }
                    i += 1;
                }
            }
        } else {
            i += 1;
        }
    }
}

// MARK: - Angle-bracket autolinks (CommonMark §6.4)

/// Parses `<scheme:rest>` URI autolinks and `<local@domain>` email autolinks.
/// The `<` and `>` are part of the span; the URL stored on `InlineKind::AutoLink`
/// has them stripped (with `mailto:` prepended for emails).
fn parse_angle_autolinks(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
    spans: &mut Vec<InlineSpan>,
    claimed: &mut ClaimedRanges,
) {
    let mut i = 0;
    while i < text.len() {
        if text[i] != b'<' {
            i += 1;
            continue;
        }
        if is_escaped(text, i) {
            i += 1;
            continue;
        }
        // Scan content up to the closing `>`. CommonMark forbids whitespace,
        // newlines, and `<` inside an autolink; bail out and keep scanning.
        let content_start = i + 1;
        let mut j = content_start;
        let mut bad_char = false;
        while j < text.len() {
            let b = text[j];
            if b == b'>' {
                break;
            }
            if b.is_ascii_whitespace() || b == b'<' {
                bad_char = true;
                break;
            }
            j += 1;
        }
        if bad_char || j >= text.len() {
            i += 1;
            continue;
        }
        let content = &text[content_start..j];
        let url = match parse_uri_autolink_content(content) {
            Some(u) => Some(u),
            None => parse_email_autolink_content(content),
        };
        if let Some(url) = url {
            let abs_start = byte_offset + i;
            let abs_end = byte_offset + j + 1;
            if !claimed.overlaps(abs_start, abs_end) {
                claimed.claim(abs_start, abs_end);
                spans.push(make_span(
                    InlineKind::AutoLink { url },
                    abs_start,
                    abs_end,
                    abs_start + 1,
                    byte_offset + j,
                    source,
                    utf16_map,
                ));
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
}

/// CommonMark URI autolink: scheme (ASCII letter then 1-31 of letter/digit/+/-/.)
/// followed by `:` then any non-whitespace, non-`<>` characters.
fn parse_uri_autolink_content(content: &[u8]) -> Option<String> {
    if content.is_empty() || !content[0].is_ascii_alphabetic() {
        return None;
    }
    let mut i = 1;
    while i < content.len()
        && (content[i].is_ascii_alphanumeric() || matches!(content[i], b'+' | b'-' | b'.'))
    {
        i += 1;
    }
    // Scheme length must be 2–32, must be terminated by `:`.
    if !(2..=32).contains(&i) || i >= content.len() || content[i] != b':' {
        return None;
    }
    std::str::from_utf8(content).ok().map(|s| s.to_string())
}

/// CommonMark email autolink: a simplified RFC-5322 local-part `@` domain.
/// Returns the email prefixed with `mailto:` for the URL field.
fn parse_email_autolink_content(content: &[u8]) -> Option<String> {
    let at_pos = content.iter().position(|&b| b == b'@')?;
    if at_pos == 0 || at_pos + 1 >= content.len() {
        return None;
    }
    let local = &content[..at_pos];
    let domain = &content[at_pos + 1..];
    let local_ok = local.iter().all(|&b| {
        b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'.' | b'!'
                    | b'#'
                    | b'$'
                    | b'%'
                    | b'&'
                    | b'\''
                    | b'*'
                    | b'+'
                    | b'/'
                    | b'='
                    | b'?'
                    | b'^'
                    | b'_'
                    | b'`'
                    | b'{'
                    | b'|'
                    | b'}'
                    | b'~'
                    | b'-'
            )
    });
    let domain_ok = !domain.is_empty()
        && domain.contains(&b'.')
        && domain
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'));
    if !local_ok || !domain_ok {
        return None;
    }
    std::str::from_utf8(content)
        .ok()
        .map(|s| format!("mailto:{}", s))
}

// MARK: - Autolinks (bare URLs and domain.TLD patterns)

/// Common TLDs for schemeless URL detection (e.g., `rede.io`, `google.com`).
/// Must be lowercase. Ordered roughly by frequency to speed up early matches.
const COMMON_TLDS: &[&str] = &[
    // Generic
    "com", "org", "net", "io", "co", "dev", "app", "ai", "me", "info", "biz", "tv", "cc", "xyz",
    "tech", "today", "world", "life", "space", "site", "online", "store", "cloud", "design", "gg",
    "fm", "sh", "edu", "gov", "mil", // Country codes (common for web)
    "us", "uk", "de", "fr", "es", "it", "nl", "ru", "br", "ca", "au", "in", "jp", "cn", "kr",
];

/// Detects URLs in text:
/// 1. Scheme-based: `https://example.com`, `http://example.com`
/// 2. Domain-based: `example.com`, `rede.io`, `sub.domain.org/path`
/// 3. Bare-email: `rene@deanda.org` (no `<>` wrapping required)
/// 4. Subreddit shorthand: `r/apple` -> `https://www.reddit.com/r/apple`
///
/// Runs after markdown links so `[text](url)` URLs are already claimed.
/// For autolinks, the entire URL is both the full span and the content span.
///
/// URL boundary: stops at whitespace, `<`, `>`, `"`, or end of text.
/// Trailing punctuation (`.`, `,`, `;`, `:`, `!`, `?`, `)`, `]`) is stripped.
/// Balanced parentheses are preserved (Wikipedia URLs).
fn parse_autolinks(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
    spans: &mut Vec<InlineSpan>,
    claimed: &mut ClaimedRanges,
) {
    // Pass 1: scheme-based URLs (https://, http://)
    parse_scheme_urls(text, byte_offset, source, utf16_map, spans, claimed);

    // Pass 2: bare-email autolinks (rene@deanda.org). Runs BEFORE the
    // domain pass so it can claim the FULL email address — otherwise
    // `parse_domain_urls` would grab `deanda.org` first and the
    // bare-email pass would see an overlapping claim and bail.
    parse_bare_email_autolinks(text, byte_offset, source, utf16_map, spans, claimed);

    // Pass 3: schemeless domain.TLD patterns (rede.io, google.com/search)
    parse_domain_urls(text, byte_offset, source, utf16_map, spans, claimed);

    // Pass 4: Reddit subreddit shorthand (r/apple). Runs after full URLs so
    // reddit.com/r/apple is claimed by the domain scanner, not split at r/apple.
    parse_subreddit_autolinks(text, byte_offset, source, utf16_map, spans, claimed);
}

/// Detects URLs with explicit http:// or https:// scheme.
fn parse_scheme_urls(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
    spans: &mut Vec<InlineSpan>,
    claimed: &mut ClaimedRanges,
) {
    let prefixes: &[&[u8]] = &[b"https://", b"http://"];
    let mut i = 0;

    while i < text.len() {
        let mut matched_prefix_len = 0;
        for prefix in prefixes {
            if i + prefix.len() <= text.len() && text[i..].starts_with(prefix) {
                matched_prefix_len = prefix.len();
                break;
            }
        }

        if matched_prefix_len == 0 {
            i += 1;
            continue;
        }

        // Don't match if preceded by an alphanumeric char (part of another word/URL scheme).
        if i > 0 && text[i - 1].is_ascii_alphanumeric() {
            i += 1;
            continue;
        }

        if let Some(url_end) = scan_url_end(text, i, i + matched_prefix_len) {
            let abs_start = byte_offset + i;
            let abs_end = byte_offset + url_end;
            if !claimed.overlaps(abs_start, abs_end) {
                let url = std::str::from_utf8(&text[i..url_end])
                    .unwrap_or("")
                    .to_string();
                claimed.claim(abs_start, abs_end);
                spans.push(make_span(
                    InlineKind::AutoLink { url },
                    abs_start,
                    abs_end,
                    abs_start,
                    abs_end,
                    source,
                    utf16_map,
                ));
            }
            i = url_end;
        } else {
            i += 1;
        }
    }
}

/// Detects schemeless domain patterns: `word.TLD` optionally followed by `/path`.
/// Only matches when the TLD is in the curated COMMON_TLDS list to avoid
/// false positives (e.g., `hello.world` is NOT a link).
fn parse_domain_urls(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
    spans: &mut Vec<InlineSpan>,
    claimed: &mut ClaimedRanges,
) {
    let mut i = 0;

    while i < text.len() {
        // Look for a dot that could be domain.TLD boundary.
        if text[i] != b'.' {
            i += 1;
            continue;
        }

        // Find the start of the domain label before the dot.
        // Domain labels: [a-zA-Z0-9-] (simplified; we require at least one char).
        let dot_pos = i;
        let mut domain_start = dot_pos;
        while domain_start > 0 {
            let prev = text[domain_start - 1];
            if prev.is_ascii_alphanumeric() || prev == b'-' || prev == b'.' {
                domain_start -= 1;
            } else {
                break;
            }
        }

        // Must have at least one char before the dot.
        if domain_start == dot_pos {
            i += 1;
            continue;
        }

        // Don't match inside existing URLs (preceded by :// or already claimed).
        if domain_start >= 3 {
            let before = &text[domain_start.saturating_sub(3)..domain_start];
            if before.ends_with(b"://") {
                i += 1;
                continue;
            }
        }

        // Extract the TLD after the dot.
        let tld_start = dot_pos + 1;
        let mut tld_end = tld_start;
        while tld_end < text.len() && text[tld_end].is_ascii_alphanumeric() {
            tld_end += 1;
        }

        if tld_end == tld_start {
            i += 1;
            continue;
        }

        let tld = std::str::from_utf8(&text[tld_start..tld_end]).unwrap_or("");
        let tld_lower = tld.to_ascii_lowercase();

        if !COMMON_TLDS.contains(&tld_lower.as_str()) {
            i = tld_end;
            continue;
        }

        // Don't match if preceded by non-boundary chars (e.g., middle of a word
        // that happens to end with .com — `random.com` is fine, but we already
        // checked domain_start boundary above).
        if domain_start > 0 && !is_url_boundary(text[domain_start - 1]) {
            i = tld_end;
            continue;
        }

        // Scan forward for optional path/query/fragment after the TLD.
        let url_end = if tld_end < text.len() && (text[tld_end] == b'/' || text[tld_end] == b':') {
            // Has path or port — scan URL end from here.
            scan_url_end(text, domain_start, tld_end).unwrap_or(tld_end)
        } else {
            tld_end
        };

        let abs_start = byte_offset + domain_start;
        let abs_end = byte_offset + url_end;

        if !claimed.overlaps(abs_start, abs_end) {
            // Store URL with https:// prefix for the link callback.
            let raw = std::str::from_utf8(&text[domain_start..url_end])
                .unwrap_or("")
                .to_string();
            let url = format!("https://{}", raw);
            claimed.claim(abs_start, abs_end);
            spans.push(make_span(
                InlineKind::AutoLink { url },
                abs_start,
                abs_end,
                abs_start,
                abs_end,
                source,
                utf16_map,
            ));
        }
        i = url_end;
    }
}

/// Returns true if the byte is a valid URL boundary (whitespace, punctuation, start of text).
fn is_url_boundary(b: u8) -> bool {
    b.is_ascii_whitespace()
        || matches!(
            b,
            b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'<' | b'>' | b'"' | b'\'' | b',' | b';'
        )
}

fn parse_subreddit_autolinks(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
    spans: &mut Vec<InlineSpan>,
    claimed: &mut ClaimedRanges,
) {
    let mut i = 0;

    while i + 3 <= text.len() {
        if !(text[i] == b'r' || text[i] == b'R') || text[i + 1] != b'/' {
            i += 1;
            continue;
        }

        if !is_subreddit_start_boundary_at(text, i) {
            i += 1;
            continue;
        }

        let name_start = i + 2;
        let mut name_end = name_start;
        while name_end < text.len() && is_subreddit_name_char(text[name_end]) {
            name_end += 1;
        }

        let name_len = name_end - name_start;
        if !(2..=21).contains(&name_len) {
            i += 1;
            continue;
        }

        if !is_subreddit_end_boundary_at(text, name_end) {
            i += 1;
            continue;
        }

        let abs_start = byte_offset + i;
        let abs_end = byte_offset + name_end;
        if !claimed.overlaps(abs_start, abs_end) {
            let name = std::str::from_utf8(&text[name_start..name_end]).unwrap_or("");
            let url = format!("https://www.reddit.com/r/{}", name);
            claimed.claim(abs_start, abs_end);
            spans.push(make_span(
                InlineKind::AutoLink { url },
                abs_start,
                abs_end,
                abs_start,
                abs_end,
                source,
                utf16_map,
            ));
        }

        i = name_end;
    }
}

fn is_subreddit_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_subreddit_start_boundary_at(text: &[u8], index: usize) -> bool {
    if index == 0 {
        return true;
    }
    // TextKit replaces list/checklist markers with U+FFFC attachment
    // characters before the styling pass. Treat that marker as a word
    // boundary so attachment-backed list items like "• r/apple" link too.
    if index >= 3 && &text[index - 3..index] == "\u{FFFC}".as_bytes() {
        return true;
    }
    is_subreddit_start_boundary(text[index - 1])
}

fn is_subreddit_end_boundary_at(text: &[u8], index: usize) -> bool {
    if index >= text.len() {
        return true;
    }
    if text[index..].starts_with("\u{FFFC}".as_bytes()) {
        return true;
    }
    is_subreddit_end_boundary(text[index])
}

fn is_subreddit_start_boundary(b: u8) -> bool {
    b.is_ascii_whitespace()
        || matches!(
            b,
            b'(' | b'[' | b'{' | b'<' | b'"' | b'\'' | b',' | b';' | b':' | b'!' | b'?'
        )
}

fn is_subreddit_end_boundary(b: u8) -> bool {
    b.is_ascii_whitespace()
        || matches!(
            b,
            b'/' | b'.'
                | b','
                | b';'
                | b':'
                | b'!'
                | b'?'
                | b')'
                | b']'
                | b'}'
                | b'>'
                | b'"'
                | b'\''
        )
}

/// Detects bare email addresses (`rene@embernotes.app`) and emits
/// `mailto:` autolinks. Ported from a prior implementation
/// (commit b505126 on branch `claude/prioritize-backlog-code-blocks-ZWMWh`)
/// that never made it to main but had been on-device-validated and is
/// substantially more thorough than a TLD-list approach:
///
/// - Honors `\@` escape so `\rene\@deanda.org` stays plain.
/// - Allows `%` in the local-part (CommonMark email autolinks do too).
/// - TLD: any 2+ ASCII letters — no curated list to maintain. Real-
///   world TLDs grow constantly (`.app`, `.dev`, `.ai`); a closed list
///   silently drops valid emails.
/// - Trims trailing `.` AND `-` so sentence punctuation doesn't get
///   absorbed into the address.
/// - Rejects consecutive dots in the domain.
/// - Strips leading dots from the local-part so context like
///   `...rene@deanda.org` doesn't break.
///
/// Runs BEFORE `parse_domain_urls` so the email claims the full
/// address before the domain scanner would otherwise grab the bare
/// `domain.tld` portion alone.
fn parse_bare_email_autolinks(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
    spans: &mut Vec<InlineSpan>,
    claimed: &mut ClaimedRanges,
) {
    let mut i = 0;
    while i < text.len() {
        if text[i] != b'@' {
            i += 1;
            continue;
        }
        if is_escaped(text, i) {
            i += 1;
            continue;
        }

        // Scan backward to find the start of the local part.
        let mut local_start = i;
        while local_start > 0 {
            let prev = text[local_start - 1];
            if prev.is_ascii_alphanumeric() || matches!(prev, b'.' | b'_' | b'%' | b'+' | b'-') {
                local_start -= 1;
            } else {
                break;
            }
        }

        // Trim leading dots — "...rene@deanda.org" should still match
        // "rene@deanda.org". The dots are context, not part of the address.
        while local_start < i && text[local_start] == b'.' {
            local_start += 1;
        }

        // Must have at least one local char; local must not end with a dot.
        if local_start == i || text[i - 1] == b'.' {
            i += 1;
            continue;
        }

        // Require a real boundary before the local part. Dots count as a
        // boundary-equivalent here because we already trimmed any leading
        // dots off the matched range.
        if local_start > 0 {
            let before = text[local_start - 1];
            if !is_url_boundary(before) && before != b'.' {
                i += 1;
                continue;
            }
        }

        // Scan forward for the domain part (alnum, `.`, `-`).
        let mut domain_end = i + 1;
        while domain_end < text.len() {
            let b = text[domain_end];
            if b.is_ascii_alphanumeric() || b == b'.' || b == b'-' {
                domain_end += 1;
            } else {
                break;
            }
        }

        // Strip trailing dots / hyphens (e.g., sentence-ending period).
        while domain_end > i + 1 && matches!(text[domain_end - 1], b'.' | b'-') {
            domain_end -= 1;
        }

        if domain_end <= i + 1 {
            i += 1;
            continue;
        }
        let domain = &text[i + 1..domain_end];

        // Domain must contain a dot and not begin with `.` or `-`.
        if domain[0] == b'-' || domain[0] == b'.' || !domain.contains(&b'.') {
            i += 1;
            continue;
        }

        // Reject consecutive dots in the domain.
        if domain.windows(2).any(|w| w == b"..") {
            i += 1;
            continue;
        }

        // TLD must be at least 2 ASCII letters. Any letters — real-world
        // TLDs grow constantly (`.app`, `.dev`, `.ai`, `.zone`); a
        // curated allowlist silently drops valid addresses.
        let last_dot = domain
            .iter()
            .rposition(|&b| b == b'.')
            .expect("domain contains dot (checked above)");
        let tld = &domain[last_dot + 1..];
        if tld.len() < 2 || !tld.iter().all(|&b| b.is_ascii_alphabetic()) {
            i += 1;
            continue;
        }

        let abs_start = byte_offset + local_start;
        let abs_end = byte_offset + domain_end;
        if !claimed.overlaps(abs_start, abs_end) {
            let email = std::str::from_utf8(&text[local_start..domain_end])
                .unwrap_or("")
                .to_string();
            let url = format!("mailto:{}", email);
            claimed.claim(abs_start, abs_end);
            spans.push(make_span(
                InlineKind::AutoLink { url },
                abs_start,
                abs_end,
                abs_start,
                abs_end,
                source,
                utf16_map,
            ));
        }
        i = domain_end;
    }
}

/// Scans forward from `scan_start` to find the end of a URL.
/// Returns None if the URL is too short (no content after scheme/domain).
fn scan_url_end(text: &[u8], _url_start: usize, scan_start: usize) -> Option<usize> {
    let mut url_end = scan_start;

    // Must have at least one character to scan.
    if url_end >= text.len() || text[url_end].is_ascii_whitespace() {
        return None;
    }

    let mut paren_depth: i32 = 0;
    while url_end < text.len() {
        let b = text[url_end];
        if b.is_ascii_whitespace() || b == b'<' || b == b'>' || b == b'"' {
            break;
        }
        if b == b'(' {
            paren_depth += 1;
        } else if b == b')' {
            if paren_depth <= 0 {
                break;
            }
            paren_depth -= 1;
        }
        url_end += 1;
    }

    // Strip trailing punctuation.
    while url_end > scan_start {
        let last = text[url_end - 1];
        if matches!(last, b'.' | b',' | b';' | b':' | b'!' | b'?' | b'\'' | b']') {
            url_end -= 1;
        } else {
            break;
        }
    }

    if url_end <= scan_start {
        return None;
    }

    Some(url_end)
}

// MARK: - Hex color literals
//
// Detects `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA` (case-insensitive) at word
// boundaries so we can render a color swatch alongside the text in the editor.
//
// Word-boundary rules:
//   - `#` must be at the start of the input, preceded by whitespace, or
//     preceded by non-alphanumeric punctuation. `a#FF0000` does NOT match.
//   - The character immediately after the hex digits must NOT be alphanumeric
//     or `_` — otherwise `#abcgh` would false-match a 3-digit `#abc`.
//
// Normalization: `hex` on the span is always 6 lowercase hex digits. Three-digit
// `#RGB` expands via duplication (`#a1b` → `aa11bb`); 4- and 8-digit forms drop
// the alpha channel. The Swift side reads the raw source if it needs the alpha.

fn parse_hex_colors(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
    spans: &mut Vec<InlineSpan>,
    claimed: &mut ClaimedRanges,
) {
    let mut i = 0;
    while i < text.len() {
        if text[i] != b'#' {
            i += 1;
            continue;
        }
        if is_escaped(text, i) {
            i += 1;
            continue;
        }
        // Preceding-char must be a boundary (start, whitespace, or non-alnum).
        let before_ok = i == 0 || !text[i - 1].is_ascii_alphanumeric();
        if !before_ok {
            i += 1;
            continue;
        }
        // A `#` that begins a highlight color marker (`=={#RRGGBB}…`) belongs
        // to the highlight pass, not a standalone hex literal. Skip it here so
        // `parse_highlights` can claim the full `==…==` span and emit a
        // `HighlightHex`. (`parse_highlights` runs after this pass.)
        if i >= 3 && text[i - 1] == b'{' && text[i - 2] == b'=' && text[i - 3] == b'=' {
            i += 1;
            continue;
        }
        // Count hex digits immediately after the `#`.
        let digits_start = i + 1;
        let mut j = digits_start;
        while j < text.len() && text[j].is_ascii_hexdigit() {
            j += 1;
        }
        let hex_len = j - digits_start;
        if !matches!(hex_len, 3 | 4 | 6 | 8) {
            i += 1;
            continue;
        }
        // Next char must be a boundary (not alnum or `_`).
        let after_ok = j >= text.len() || {
            let b = text[j];
            !(b.is_ascii_alphanumeric() || b == b'_')
        };
        if !after_ok {
            i += 1;
            continue;
        }
        let raw = match std::str::from_utf8(&text[digits_start..j]) {
            Ok(s) => s,
            Err(_) => {
                i += 1;
                continue;
            }
        };
        let normalized = normalize_hex(raw, hex_len);
        let abs_start = byte_offset + i;
        let abs_end = byte_offset + j;
        if !claimed.overlaps(abs_start, abs_end) {
            claimed.claim(abs_start, abs_end);
            // Content starts after the `#` so Swift can treat the hash as an
            // optional marker. `abs_end` is both the span end and content end.
            spans.push(make_span(
                InlineKind::HexColor { hex: normalized },
                abs_start,
                abs_end,
                abs_start + 1,
                abs_end,
                source,
                utf16_map,
            ));
        }
        i = j;
    }
}

/// Expand 3/4-digit hex to 6, drop alpha from 8-digit, lowercase everything.
fn normalize_hex(raw: &str, hex_len: usize) -> String {
    match hex_len {
        3 => raw
            .chars()
            .flat_map(|c| {
                let lc = c.to_ascii_lowercase();
                [lc, lc]
            })
            .collect(),
        4 => raw
            .chars()
            .take(3)
            .flat_map(|c| {
                let lc = c.to_ascii_lowercase();
                [lc, lc]
            })
            .collect(),
        6 => raw.to_ascii_lowercase(),
        8 => raw[..6].to_ascii_lowercase(),
        _ => String::new(),
    }
}

// MARK: - Emphasis (bold, italic, bold-italic) — CommonMark delimiter-run algorithm
//
// Supports nested emphasis: **bold *italic* bold** produces both Bold and Italic spans.
// Uses a stack-based approach with left/right flanking rules per CommonMark spec.
// A run can be both opener and closer (e.g., a middle `*` in `*a *b* c*`).
// Delimiter consumption: match min(opener_remaining, closer_remaining), using:
//   3 → BoldItalic, 2 → Bold, 1 → Italic.

/// A potential opening delimiter on the stack.
#[derive(Debug)]
struct EmphasisOpener {
    /// Byte position in `text` where this opener's unconsumed asterisks start.
    pos: usize,
    /// How many asterisks remain unconsumed from this run.
    remaining: usize,
    /// Original run length (needed for the "multiple of 3" rule).
    original_len: usize,
    /// Whether this opener's original run could also close.
    can_close: bool,
}

// MARK: - Unicode character classification for CommonMark flanking rules

/// Returns true if the character is Unicode whitespace (CommonMark definition).
/// Includes ASCII whitespace plus Unicode Zs category.
fn is_unicode_whitespace(c: char) -> bool {
    // ASCII whitespace
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C')
    // Unicode Zs (space separators) + line/paragraph separators
    || matches!(c,
        '\u{00A0}'  // no-break space
        | '\u{1680}' // ogham space mark
        | '\u{2000}'..='\u{200A}' // en quad through hair space
        | '\u{202F}' // narrow no-break space
        | '\u{205F}' // medium mathematical space
        | '\u{3000}' // ideographic space
        | '\u{2028}' // line separator
        | '\u{2029}' // paragraph separator
    )
}

/// Returns true if the character is Unicode punctuation (CommonMark definition).
/// Includes ASCII punctuation plus Unicode categories Pc, Pd, Pe, Pf, Pi, Po, Ps, Sc, Sk, Sm, So.
fn is_unicode_punctuation(c: char) -> bool {
    // ASCII punctuation (CommonMark explicit set)
    if c.is_ascii() {
        return matches!(
            c,
            '!' | '"'
                | '#'
                | '$'
                | '%'
                | '&'
                | '\''
                | '('
                | ')'
                | '*'
                | '+'
                | ','
                | '-'
                | '.'
                | '/'
                | ':'
                | ';'
                | '<'
                | '='
                | '>'
                | '?'
                | '@'
                | '['
                | '\\'
                | ']'
                | '^'
                | '_'
                | '`'
                | '{'
                | '|'
                | '}'
                | '~'
        );
    }
    // Unicode general categories: P* (punctuation) and S* (symbols)
    // Use Unicode category lookup via char methods + explicit ranges for common cases
    let cat = unicode_general_category(c);
    matches!(
        cat,
        UnicodeCategory::Pc
            | UnicodeCategory::Pd
            | UnicodeCategory::Pe
            | UnicodeCategory::Pf
            | UnicodeCategory::Pi
            | UnicodeCategory::Po
            | UnicodeCategory::Ps
            | UnicodeCategory::Sc
            | UnicodeCategory::Sk
            | UnicodeCategory::Sm
            | UnicodeCategory::So
    )
}

/// Lightweight Unicode general category classification.
/// Covers the categories needed for CommonMark punctuation detection.
/// For characters outside the common ranges, falls back to heuristic classification.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)] // All variants are valid Unicode categories; some aren't produced by our classifier yet
enum UnicodeCategory {
    Pc,    // Connector punctuation (_)
    Pd,    // Dash punctuation (–, —, ‐)
    Pe,    // Close punctuation (), ], })
    Pf,    // Final punctuation (», ', ")
    Pi,    // Initial punctuation («, ', ")
    Po,    // Other punctuation (!, ?, etc.)
    Ps,    // Open punctuation ((, [, {)
    Sc,    // Currency symbol ($, €, £, ¥)
    Sk,    // Modifier symbol (^, `, ´)
    Sm,    // Math symbol (+, =, <, >, |, ~)
    So,    // Other symbol (©, ®, °, emoji modifiers)
    Other, // Not punctuation/symbol
}

fn unicode_general_category(c: char) -> UnicodeCategory {
    // Common Unicode punctuation ranges used in real-world text
    match c {
        // General punctuation block (U+2000–U+206F) — punctuation subset
        '\u{2010}'..='\u{2015}' => UnicodeCategory::Pd, // hyphens, dashes
        '\u{2016}'..='\u{2017}' => UnicodeCategory::Po,
        '\u{2018}' | '\u{201B}' | '\u{201C}' | '\u{201F}' | '\u{2039}' => UnicodeCategory::Pi, // left quotes
        '\u{2019}' | '\u{201D}' | '\u{203A}' => UnicodeCategory::Pf, // right quotes
        '\u{201A}' | '\u{201E}' => UnicodeCategory::Ps,              // low quotes (open)
        '\u{2020}'..='\u{2027}' => UnicodeCategory::Po,              // daggers, bullets, etc.
        '\u{2030}'..='\u{2038}' => UnicodeCategory::Po,              // per mille, prime, etc.
        '\u{203B}'..='\u{203E}' => UnicodeCategory::Po,
        '\u{2041}'..='\u{2043}' => UnicodeCategory::Po,
        '\u{2044}' => UnicodeCategory::Sm, // fraction slash
        '\u{2045}' => UnicodeCategory::Ps, // left square bracket with quill
        '\u{2046}' => UnicodeCategory::Pe, // right square bracket with quill
        '\u{2047}'..='\u{2051}' => UnicodeCategory::Po,
        '\u{2052}' => UnicodeCategory::Sm,
        '\u{2053}' => UnicodeCategory::Po,
        '\u{2055}'..='\u{205E}' => UnicodeCategory::Po,

        // CJK punctuation (U+3000–U+303F)
        '\u{3001}'..='\u{3003}' => UnicodeCategory::Po, // ideographic comma, period, ditto
        '\u{3008}' | '\u{300A}' | '\u{300C}' | '\u{300E}' | '\u{3010}' | '\u{3014}'
        | '\u{3016}' | '\u{3018}' | '\u{301A}' => UnicodeCategory::Ps, // CJK open brackets
        '\u{3009}' | '\u{300B}' | '\u{300D}' | '\u{300F}' | '\u{3011}' | '\u{3015}'
        | '\u{3017}' | '\u{3019}' | '\u{301B}' => UnicodeCategory::Pe, // CJK close brackets
        '\u{301C}'..='\u{301F}' => UnicodeCategory::Pd, // wave dashes, quotation marks

        // Fullwidth punctuation (U+FF00–U+FF60)
        '\u{FF08}' => UnicodeCategory::Ps, // （
        '\u{FF09}' => UnicodeCategory::Pe, // ）
        '\u{FF01}'..='\u{FF07}' | '\u{FF0A}'..='\u{FF0F}' => UnicodeCategory::Po, // ！ to ／ (excl. parens)
        '\u{FF1A}'..='\u{FF1B}' => UnicodeCategory::Po,                           // ： ；
        '\u{FF1F}'..='\u{FF20}' => UnicodeCategory::Po,                           // ？ ＠
        '\u{FF3B}' => UnicodeCategory::Ps,                                        // ［
        '\u{FF3D}' => UnicodeCategory::Pe,                                        // ］
        '\u{FF5B}' => UnicodeCategory::Ps,                                        // ｛
        '\u{FF5D}' => UnicodeCategory::Pe,                                        // ｝
        '\u{FF5F}' => UnicodeCategory::Ps,                                        // ｟
        '\u{FF60}' => UnicodeCategory::Pe,                                        // ｠

        // Latin-1 supplement punctuation
        '\u{00A1}' => UnicodeCategory::Po,              // ¡
        '\u{00A7}' => UnicodeCategory::Po,              // §
        '\u{00AB}' => UnicodeCategory::Pi,              // «
        '\u{00B6}'..='\u{00B7}' => UnicodeCategory::Po, // ¶ ·
        '\u{00BB}' => UnicodeCategory::Pf,              // »
        '\u{00BF}' => UnicodeCategory::Po,              // ¿

        // Currency symbols
        '\u{00A2}'..='\u{00A5}' => UnicodeCategory::Sc, // ¢ £ ¤ ¥
        '\u{20A0}'..='\u{20CF}' => UnicodeCategory::Sc, // Currency symbols block (€, ₹, etc.)

        // Math symbols
        '\u{00AC}' | '\u{00B1}' | '\u{00D7}' | '\u{00F7}' => UnicodeCategory::Sm,
        '\u{2190}'..='\u{21FF}' => UnicodeCategory::Sm, // Arrows
        '\u{2200}'..='\u{22FF}' => UnicodeCategory::Sm, // Mathematical operators
        '\u{2300}'..='\u{23FF}' => UnicodeCategory::So, // Miscellaneous technical
        '\u{2500}'..='\u{257F}' => UnicodeCategory::So, // Box drawing
        '\u{25A0}'..='\u{25FF}' => UnicodeCategory::So, // Geometric shapes
        '\u{2600}'..='\u{26FF}' => UnicodeCategory::So, // Miscellaneous symbols
        '\u{2700}'..='\u{27BF}' => UnicodeCategory::So, // Dingbats

        // Other common punctuation
        '\u{00A9}' | '\u{00AE}' => UnicodeCategory::So, // © ®
        '\u{00B0}' => UnicodeCategory::So,              // °
        '\u{2116}' => UnicodeCategory::So,              // №
        '\u{2122}' => UnicodeCategory::So,              // ™

        // Fallback: use char properties for broad classification
        _ => {
            // Rust's char::is_ascii_punctuation only covers ASCII, but we already handle that.
            // For non-ASCII, non-alphanumeric, non-whitespace chars in symbol-heavy blocks,
            // treat as punctuation/symbol. This is conservative (may over-classify) which is
            // the safe direction — it only relaxes flanking rules, never tightens them.
            if !c.is_alphanumeric() && !c.is_whitespace() && !c.is_control() {
                UnicodeCategory::Po // conservative: treat unknown symbols as punctuation
            } else {
                UnicodeCategory::Other
            }
        }
    }
}

/// Returns true if the byte at `pos` is preceded by an unescaped backslash —
/// i.e. it's the second byte of a CommonMark backslash escape (`\*`, `\[`, …).
///
/// Counts the backslash run immediately before `pos`: an odd count means the
/// byte is escaped; even means the backslashes paired up as literal escapes
/// (`\\`) and left the byte at `pos` alone. Per CommonMark §6.1, only ASCII
/// punctuation is escapable; callers invoke this guard at positions they
/// already know hold ASCII punctuation (`*`, `` ` ``, `[`, `~`, `=`, `<`).
#[inline]
fn is_escaped(text: &[u8], pos: usize) -> bool {
    let mut count = 0usize;
    let mut i = pos;
    while i > 0 && text[i - 1] == b'\\' {
        count += 1;
        i -= 1;
    }
    count % 2 == 1
}

/// Decode the UTF-8 character at the given byte position and return it.
/// Returns None if the position is out of bounds.
fn char_at(text: &[u8], pos: usize) -> Option<char> {
    if pos >= text.len() {
        return None;
    }
    let b = text[pos];
    if b < 0x80 {
        return Some(b as char);
    }
    // Multi-byte UTF-8: decode by hand for speed (no allocation)
    let width = match b {
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => return None, // continuation byte or invalid
    };
    if pos + width > text.len() {
        return None;
    }
    std::str::from_utf8(&text[pos..pos + width])
        .ok()
        .and_then(|s| s.chars().next())
}

/// Decode the UTF-8 character ending at (or before) the given byte position.
/// Returns the character and its starting byte index.
fn char_before(text: &[u8], pos: usize) -> Option<char> {
    if pos == 0 || pos > text.len() {
        return None;
    }
    // Walk backwards to find the start of the UTF-8 sequence
    let mut start = pos - 1;
    while start > 0 && (text[start] & 0xC0) == 0x80 {
        start -= 1;
    }
    // Verify this is a valid start byte
    let b = text[start];
    let width = if b < 0x80 {
        1
    } else if (0xC0..=0xDF).contains(&b) {
        2
    } else if (0xE0..=0xEF).contains(&b) {
        3
    } else if (0xF0..=0xF7).contains(&b) {
        4
    } else {
        return None;
    };
    if start + width != pos {
        // The byte before `pos` wasn't the last byte of a character ending at `pos`
        // This happens with ASCII: width=1, start=pos-1, start+1 == pos ✓
        // Or multi-byte: start + width should equal pos
        // If not, we might have a partial sequence
        if start + width > pos {
            return None;
        }
        // Actually for the common case, we want the char immediately before pos.
        // start + width might be < pos if there are multiple characters. We want
        // the one right before pos.
        // Recalculate: the char before pos must end at pos.
        let try_start = pos - 1;
        if text[try_start] < 0x80 {
            return Some(text[try_start] as char);
        }
        // Multi-byte: find the start
        let mut s = try_start;
        while s > 0 && (text[s] & 0xC0) == 0x80 {
            s -= 1;
        }
        return std::str::from_utf8(&text[s..pos])
            .ok()
            .and_then(|str| str.chars().last());
    }
    std::str::from_utf8(&text[start..pos])
        .ok()
        .and_then(|s| s.chars().next())
}

/// Determine if an asterisk run is left-flanking (can open emphasis).
///
/// CommonMark spec §6.2: A left-flanking delimiter run is one that is:
/// (1) not followed by Unicode whitespace, AND
/// (2) not followed by a Unicode punctuation character,
///     OR preceded by Unicode whitespace or a Unicode punctuation character.
fn is_left_flanking(text: &[u8], run_end: usize) -> bool {
    let after = match char_at(text, run_end) {
        Some(c) => c,
        None => return false, // nothing after → can't open
    };

    // Condition 1: not followed by Unicode whitespace
    if is_unicode_whitespace(after) {
        return false;
    }

    // Condition 2: not followed by punctuation, OR preceded by whitespace/punctuation
    if is_unicode_punctuation(after) {
        // Must check what precedes the run
        // run_end is the first byte after the run; the run started at some earlier position.
        // We need the character before the run. The caller passes run_end = start + len,
        // so the run starts at run_end - len. We need the char before run_start.
        // However, we don't have run_start here. We need to look at what's before the run.
        // Since the run is all asterisks, the byte before the run is at (run_end - len - 1)
        // which we can approximate by walking back from run_end past the asterisks.
        let mut run_start = run_end;
        while run_start > 0 && text[run_start - 1] == b'*' {
            run_start -= 1;
        }
        match char_before(text, run_start) {
            None => true, // beginning of string counts as whitespace-preceded
            Some(c) => is_unicode_whitespace(c) || is_unicode_punctuation(c),
        }
    } else {
        true // not followed by punctuation → left-flanking
    }
}

/// Determine if an asterisk run is right-flanking (can close emphasis).
///
/// CommonMark spec §6.2: A right-flanking delimiter run is one that is:
/// (1) not preceded by Unicode whitespace, AND
/// (2) not preceded by a Unicode punctuation character,
///     OR followed by Unicode whitespace or a Unicode punctuation character.
fn is_right_flanking(text: &[u8], run_start: usize) -> bool {
    let before = match char_before(text, run_start) {
        Some(c) => c,
        None => return false, // nothing before → can't close
    };

    // Condition 1: not preceded by Unicode whitespace
    if is_unicode_whitespace(before) {
        return false;
    }

    // Condition 2: not preceded by punctuation, OR followed by whitespace/punctuation
    if is_unicode_punctuation(before) {
        // Must check what follows the run
        let mut run_end = run_start;
        while run_end < text.len() && text[run_end] == b'*' {
            run_end += 1;
        }
        match char_at(text, run_end) {
            None => true, // end of string counts as whitespace-followed
            Some(c) => is_unicode_whitespace(c) || is_unicode_punctuation(c),
        }
    } else {
        true // not preceded by punctuation → right-flanking
    }
}

fn parse_emphasis(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
    spans: &mut Vec<InlineSpan>,
    claimed: &mut ClaimedRanges,
) {
    // Collect all asterisk runs with their positions and flanking info
    struct AsteriskRun {
        pos: usize,
        len: usize,
        can_open: bool,
        can_close: bool,
    }

    let mut runs: Vec<AsteriskRun> = Vec::new();
    {
        let mut i = 0;
        while i < text.len() {
            if text[i] == b'*' {
                // Skip an escaped asterisk: it can't be part of a delimiter run.
                if is_escaped(text, i) {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < text.len() && text[i] == b'*' && !is_escaped(text, i) {
                    i += 1;
                }
                let len = i - start;
                runs.push(AsteriskRun {
                    pos: start,
                    len,
                    can_open: is_left_flanking(text, start + len),
                    can_close: is_right_flanking(text, start),
                });
            } else {
                i += 1;
            }
        }
    }

    if runs.is_empty() {
        return;
    }

    // Stack of potential openers
    let mut opener_stack: Vec<EmphasisOpener> = Vec::new();
    // Collected spans (before claimed-range filtering)
    let mut emphasis_spans: Vec<(usize, usize, InlineKind)> = Vec::new();

    for run in &runs {
        let mut remaining = run.len;

        // Phase 1: If this run can close, scan opener stack from BOTTOM (oldest)
        // to find the closest matching opener (CommonMark spec).
        if run.can_close {
            // Scan from bottom to find innermost matching opener
            let mut stack_idx = opener_stack.len();
            while remaining > 0 && stack_idx > 0 {
                stack_idx -= 1;

                let opener = &opener_stack[stack_idx];

                // CommonMark "multiple of 3" rule (spec rule 17):
                // If either the closer or opener can both open and close,
                // and the sum of their original run lengths is a multiple of 3,
                // AND neither is individually a multiple of 3, skip the match.
                if run.can_open || opener.can_close {
                    let sum = run.len + opener.original_len;
                    if sum.is_multiple_of(3)
                        && !run.len.is_multiple_of(3)
                        && !opener.original_len.is_multiple_of(3)
                    {
                        continue;
                    }
                }

                let consume = remaining.min(opener.remaining);

                // Determine kind from consumed count
                let (kind, used) = if consume >= 3 {
                    (InlineKind::BoldItalic, 3)
                } else if consume == 2 {
                    (InlineKind::Bold, 2)
                } else {
                    (InlineKind::Italic, 1)
                };

                // Opener asterisks consumed from the RIGHT (inner edge)
                let open_byte_start = opener.pos + opener.remaining - used;
                // Closer asterisks consumed from the LEFT (inner edge)
                let close_byte_start = run.pos + (run.len - remaining);

                let abs_start = byte_offset + open_byte_start;
                let abs_end = byte_offset + close_byte_start + used;

                emphasis_spans.push((abs_start, abs_end, kind));

                // Mutate the opener
                let opener = &mut opener_stack[stack_idx];
                opener.remaining -= used;
                remaining -= used;

                // Remove exhausted opener and all openers above it
                // (openers between matched pair are abandoned per CommonMark)
                if opener.remaining == 0 {
                    opener_stack.drain(stack_idx..);
                } else {
                    // Remove everything above the partially consumed opener
                    opener_stack.truncate(stack_idx + 1);
                }
                // Reset stack_idx for next iteration
                stack_idx = opener_stack.len();
            }
        }

        // Phase 2: If this run can open (and has remaining asterisks), push as opener
        if remaining > 0 && run.can_open {
            opener_stack.push(EmphasisOpener {
                pos: run.pos + (run.len - remaining),
                remaining,
                original_len: run.len,
                can_close: run.can_close,
            });
        }
    }

    // Emit spans. Check only against pre-existing claimed ranges (from code, links, etc.)
    // Emphasis spans are allowed to nest within each other, so we don't check emphasis
    // spans against other emphasis spans. We DO claim them for lower-priority parsers.
    emphasis_spans.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    for (abs_start, abs_end, kind) in emphasis_spans {
        if abs_start >= abs_end {
            continue;
        }
        let delim_len = match kind {
            InlineKind::BoldItalic => 3,
            InlineKind::Bold => 2,
            InlineKind::Italic => 1,
            _ => unreachable!(),
        };
        let content_start = abs_start + delim_len;
        let content_end = abs_end - delim_len;
        if content_start > content_end {
            continue;
        }
        // Only check against pre-existing claims (code, links) — not other emphasis spans
        if !claimed.overlaps(abs_start, abs_end) {
            spans.push(make_span(
                kind,
                abs_start,
                abs_end,
                content_start,
                content_end,
                source,
                utf16_map,
            ));
        }
    }

    // Note: emphasis markers (*) don't conflict with any other inline pattern's markers
    // (~, =, `, [, <u>), so we don't claim emphasis ranges. This allows other inline
    // styles to nest freely inside emphasis content (e.g., **bold ~~strike~~ bold**).
}

// MARK: - Highlights

fn parse_highlights(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
    spans: &mut Vec<InlineSpan>,
    claimed: &mut ClaimedRanges,
) {
    let mut i = 0;
    while i + 1 < text.len() {
        if text[i] == b'=' && text[i + 1] == b'=' {
            if is_escaped(text, i) {
                i += 1;
                continue;
            }
            let open = i;
            i += 2;

            // Check for color emoji prefix
            let (kind, content_start_offset) =
                if i + 3 < text.len() && text[i] == 0xF0 && text[i + 1] == 0x9F {
                    let color_idx = match (text[i + 2], text[i + 3]) {
                        (0x94, 0xB4) => Some(0u8), // 🔴
                        (0x9F, 0xA0) => Some(1),   // 🟠
                        (0x9F, 0xA1) => Some(2),   // 🟡
                        (0x9F, 0xA2) => Some(3),   // 🟢
                        (0x94, 0xB5) => Some(4),   // 🔵
                        (0x9F, 0xA3) => Some(5),   // 🟣
                        _ => None,
                    };
                    if let Some(idx) = color_idx {
                        (InlineKind::HighlightColor(idx), 4) // skip emoji bytes
                    } else {
                        (InlineKind::Highlight, 0)
                    }
                } else if text[i..].starts_with(b"{#") {
                    // Hex-colored highlight: =={#RRGGBB}text==
                    // Accepts 3/4/6/8 hex digits (same forms as inline hex literals).
                    let hex_start = i + 2; // skip "{#"
                    let mut k = hex_start;
                    while k < text.len() && text[k].is_ascii_hexdigit() {
                        k += 1;
                    }
                    let hex_len = k - hex_start;
                    if k < text.len() && text[k] == b'}' && matches!(hex_len, 3 | 4 | 6 | 8) {
                        match std::str::from_utf8(&text[hex_start..k]) {
                            Ok(raw) => (
                                InlineKind::HighlightHex {
                                    hex: normalize_hex(raw, hex_len),
                                },
                                k + 1 - i, // skip past the closing brace
                            ),
                            Err(_) => (InlineKind::Highlight, 0),
                        }
                    } else {
                        // Malformed brace — treat as a plain highlight; the literal
                        // `{#…` stays in the content.
                        (InlineKind::Highlight, 0)
                    }
                } else if text[i..].starts_with(b"{color:") {
                    // Legacy colored highlight: =={color:name}text==
                    // Find the closing }
                    let color_name_start = i + 7; // skip "{color:"
                    let mut end_brace = color_name_start;
                    while end_brace < text.len() && text[end_brace] != b'}' {
                        end_brace += 1;
                    }
                    if end_brace < text.len() {
                        let name = &text[color_name_start..end_brace];
                        let idx = match name {
                            b"red" => Some(0u8),
                            b"orange" => Some(1),
                            b"yellow" => Some(2),
                            b"green" => Some(3),
                            b"blue" => Some(4),
                            b"purple" => Some(5),
                            _ => None,
                        };
                        if let Some(color_idx) = idx {
                            (InlineKind::HighlightColor(color_idx), end_brace + 1 - i)
                        // skip past }
                        } else {
                            (InlineKind::Highlight, 0)
                        }
                    } else {
                        (InlineKind::Highlight, 0)
                    }
                } else {
                    (InlineKind::Highlight, 0)
                };

            let content_byte_start = i + content_start_offset;

            // Find closing ==
            let mut j = content_byte_start;
            let mut found = false;
            while j + 1 < text.len() {
                if text[j] == b'=' && text[j + 1] == b'=' {
                    let abs_start = byte_offset + open;
                    let abs_end = byte_offset + j + 2;
                    let abs_content_start = byte_offset + content_byte_start;
                    let abs_content_end = byte_offset + j;
                    if !claimed.overlaps(abs_start, abs_end) {
                        claimed.claim(abs_start, abs_end);
                        spans.push(make_span(
                            kind.clone(),
                            abs_start,
                            abs_end,
                            abs_content_start,
                            abs_content_end,
                            source,
                            utf16_map,
                        ));
                    }
                    i = j + 2;
                    found = true;
                    break;
                }
                j += 1;
            }
            if !found {
                i = content_byte_start.max(i);
            }
        } else {
            i += 1;
        }
    }
}

// MARK: - HTML underline

fn parse_html_underline(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
    spans: &mut Vec<InlineSpan>,
    claimed: &mut ClaimedRanges,
) {
    let open_tag = b"<u>";
    let close_tag = b"</u>";
    let mut i = 0;
    while i + 3 <= text.len() {
        if text[i..].starts_with(open_tag) {
            if is_escaped(text, i) {
                i += 1;
                continue;
            }
            let open = i;
            i += 3;
            // Find </u>
            while i + 4 <= text.len() {
                if text[i..].starts_with(close_tag) {
                    let abs_start = byte_offset + open;
                    let abs_end = byte_offset + i + 4;
                    if !claimed.overlaps(abs_start, abs_end) {
                        claimed.claim(abs_start, abs_end);
                        spans.push(make_span(
                            InlineKind::UnderlineHtml,
                            abs_start,
                            abs_end,
                            abs_start + 3,
                            byte_offset + i,
                            source,
                            utf16_map,
                        ));
                    }
                    i += 4;
                    break;
                }
                i += 1;
            }
        } else {
            i += 1;
        }
    }
}

// MARK: - Tilde underline

fn parse_tilde_underline(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
    spans: &mut Vec<InlineSpan>,
    claimed: &mut ClaimedRanges,
) {
    let mut i = 0;
    while i < text.len() {
        if text[i] == b'~' {
            if is_escaped(text, i) {
                i += 1;
                continue;
            }
            // Count the tilde run length
            let run_start = i;
            while i < text.len() && text[i] == b'~' && !is_escaped(text, i) {
                i += 1;
            }
            let run_len = i - run_start;

            // Only single tildes are underline delimiters; skip ~~ runs
            if run_len != 1 {
                continue;
            }

            let open = run_start;
            let abs_open = byte_offset + open;
            if claimed.overlaps(abs_open, abs_open + 1) {
                continue;
            }

            // Find matching single ~
            let mut j = i;
            while j < text.len() {
                if text[j] == b'~' {
                    if is_escaped(text, j) {
                        j += 1;
                        continue;
                    }
                    // Count closing tilde run
                    let close_start = j;
                    while j < text.len() && text[j] == b'~' && !is_escaped(text, j) {
                        j += 1;
                    }
                    if j - close_start != 1 {
                        continue; // Skip ~~ runs
                    }

                    let abs_start = byte_offset + open;
                    let abs_end = byte_offset + close_start + 1;
                    if !claimed.overlaps(abs_start, abs_end) {
                        claimed.claim(abs_start, abs_end);
                        spans.push(make_span(
                            InlineKind::UnderlineTilde,
                            abs_start,
                            abs_end,
                            abs_start + 1,
                            abs_end - 1,
                            source,
                            utf16_map,
                        ));
                    }
                    i = j;
                    break;
                }
                j += 1;
            }
            if j >= text.len() {
                i = j; // No closer found
            }
        } else {
            i += 1;
        }
    }
}

// MARK: - Footnote refs

fn parse_footnote_refs(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
    spans: &mut Vec<InlineSpan>,
    claimed: &mut ClaimedRanges,
) {
    let mut i = 0;
    while i + 2 < text.len() {
        if text[i] == b'[' && text[i + 1] == b'^' {
            if is_escaped(text, i) {
                i += 1;
                continue;
            }
            let open = i;
            i += 2;
            // Find closing ]
            let label_start = i;
            while i < text.len() && text[i] != b']' && text[i] != b'\n' {
                i += 1;
            }
            if i < text.len() && text[i] == b']' {
                // Must NOT be followed by : (that's a definition, not a reference)
                if i + 1 < text.len() && text[i + 1] == b':' {
                    i += 1;
                    continue;
                }
                // Must have non-empty label
                if i > label_start {
                    let abs_start = byte_offset + open;
                    let abs_end = byte_offset + i + 1;
                    if !claimed.overlaps(abs_start, abs_end) {
                        claimed.claim(abs_start, abs_end);
                        spans.push(make_span(
                            InlineKind::FootnoteRef,
                            abs_start,
                            abs_end,
                            byte_offset + label_start,
                            byte_offset + i,
                            source,
                            utf16_map,
                        ));
                    }
                }
                i += 1;
            }
        } else {
            i += 1;
        }
    }
}

// MARK: - Generic paired marker

#[allow(clippy::too_many_arguments)]
fn parse_paired_marker(
    text: &[u8],
    byte_offset: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
    open_marker: &[u8],
    close_marker: &[u8],
    kind: InlineKind,
    spans: &mut Vec<InlineSpan>,
    claimed: &mut ClaimedRanges,
) {
    let open_len = open_marker.len();
    let close_len = close_marker.len();
    let mut i = 0;

    while i + open_len <= text.len() {
        if text[i..].starts_with(open_marker) {
            if is_escaped(text, i) {
                i += 1;
                continue;
            }
            let open = i;
            i += open_len;
            // Find closing marker
            while i + close_len <= text.len() {
                if text[i..].starts_with(close_marker) && !is_escaped(text, i) {
                    let abs_start = byte_offset + open;
                    let abs_end = byte_offset + i + close_len;
                    if !claimed.overlaps(abs_start, abs_end) {
                        claimed.claim(abs_start, abs_end);
                        spans.push(make_span(
                            kind.clone(),
                            abs_start,
                            abs_end,
                            abs_start + open_len,
                            byte_offset + i,
                            source,
                            utf16_map,
                        ));
                    }
                    i += close_len;
                    break;
                }
                i += 1;
            }
        } else {
            i += 1;
        }
    }
}

// MARK: - Span construction helper

fn make_span(
    kind: InlineKind,
    abs_byte_start: usize,
    abs_byte_end: usize,
    abs_content_byte_start: usize,
    abs_content_byte_end: usize,
    source: &[u8],
    utf16_map: &Utf16Map,
) -> InlineSpan {
    InlineSpan {
        kind,
        utf16_start: utf16_map.byte_to_utf16(abs_byte_start as u32, source),
        utf16_end: utf16_map.byte_to_utf16(abs_byte_end as u32, source),
        content_utf16_start: utf16_map.byte_to_utf16(abs_content_byte_start as u32, source),
        content_utf16_end: utf16_map.byte_to_utf16(abs_content_byte_end as u32, source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn get_spans(input: &str) -> Vec<InlineSpan> {
        let doc = parse(input, ParseMode::Grouped);
        doc.blocks
            .into_iter()
            .flat_map(|b| b.inline_spans)
            .collect()
    }

    #[test]
    fn inline_code() {
        let spans = get_spans("`code`");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::InlineCode));
    }

    #[test]
    fn bold() {
        let spans = get_spans("**bold**");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::Bold));
    }

    #[test]
    fn italic() {
        let spans = get_spans("*italic*");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::Italic));
    }

    #[test]
    fn bold_italic() {
        let spans = get_spans("***bold italic***");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::BoldItalic));
    }

    #[test]
    fn strikethrough() {
        let spans = get_spans("~~strikethrough~~");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::Strikethrough));
    }

    #[test]
    fn highlight() {
        let spans = get_spans("==highlighted==");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::Highlight));
    }

    #[test]
    fn colored_highlight_green() {
        let spans = get_spans("==\u{1F7E2}green text==");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::HighlightColor(3)));
    }

    #[test]
    fn colored_highlight_red() {
        let spans = get_spans("==\u{1F534}red text==");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::HighlightColor(0)));
    }

    #[test]
    fn legacy_colored_highlight_red() {
        let spans = get_spans("=={color:red}red text==");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::HighlightColor(0)));
    }

    #[test]
    fn legacy_colored_highlight_green() {
        let spans = get_spans("=={color:green}green text==");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::HighlightColor(3)));
    }

    #[test]
    fn legacy_colored_highlight_blue() {
        let spans = get_spans("=={color:blue}blue text==");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::HighlightColor(4)));
    }

    #[test]
    fn legacy_colored_highlight_unknown_falls_back() {
        let spans = get_spans("=={color:magenta}text==");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::Highlight));
    }

    #[test]
    fn hex_highlight_six_digit() {
        let spans = get_spans("=={#ff5733}text==");
        assert_eq!(spans.len(), 1);
        match &spans[0].kind {
            InlineKind::HighlightHex { hex } => assert_eq!(hex, "ff5733"),
            other => panic!("expected HighlightHex, got {other:?}"),
        }
        // Content `text` follows `==` (2) + `{#ff5733}` (9) = offset 11..15.
        assert_eq!(spans[0].utf16_start, 0);
        assert_eq!(spans[0].utf16_end, 17);
        assert_eq!(spans[0].content_utf16_start, 11);
        assert_eq!(spans[0].content_utf16_end, 15);
    }

    #[test]
    fn hex_highlight_three_digit_expands() {
        let spans = get_spans("=={#f00}x==");
        assert_eq!(spans.len(), 1);
        match &spans[0].kind {
            InlineKind::HighlightHex { hex } => assert_eq!(hex, "ff0000"),
            other => panic!("expected HighlightHex, got {other:?}"),
        }
    }

    #[test]
    fn hex_highlight_uppercase_normalized() {
        let spans = get_spans("=={#AABBCC}x==");
        assert_eq!(spans.len(), 1);
        match &spans[0].kind {
            InlineKind::HighlightHex { hex } => assert_eq!(hex, "aabbcc"),
            other => panic!("expected HighlightHex, got {other:?}"),
        }
    }

    #[test]
    fn hex_highlight_eight_digit_drops_alpha() {
        let spans = get_spans("=={#11223344}x==");
        assert_eq!(spans.len(), 1);
        match &spans[0].kind {
            InlineKind::HighlightHex { hex } => assert_eq!(hex, "112233"),
            other => panic!("expected HighlightHex, got {other:?}"),
        }
    }

    #[test]
    fn hex_highlight_invalid_length_falls_back() {
        // 2 hex digits is not a valid color length → plain highlight, `{#ff}`
        // stays in the content.
        let spans = get_spans("=={#ff}text==");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::Highlight));
    }

    #[test]
    fn hex_highlight_non_hex_falls_back() {
        let spans = get_spans("=={#xyz}text==");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::Highlight));
    }

    #[test]
    fn hex_highlight_missing_brace_falls_back() {
        let spans = get_spans("=={#ff5733 text==");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::Highlight));
    }

    #[test]
    fn legacy_colored_highlight_content_range() {
        let spans = get_spans("=={color:red}hello==");
        assert_eq!(spans.len(), 1);
        // Content should be "hello" — after the } and before the closing ==
        // "=={color:red}" = 13 bytes, "hello" = 5 bytes, "==" = 2 bytes
        assert_eq!(spans[0].content_utf16_start, 13);
        assert_eq!(spans[0].content_utf16_end, 18);
    }

    #[test]
    fn wiki_link() {
        let spans = get_spans("see [[Note Title]] here");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::WikiLink));
    }

    #[test]
    fn wiki_link_with_alias_content_range_covers_alias_only() {
        // `[[Target|Alias]]` — content range points at `Alias`, making the
        // entire `[[Target|` run the opening marker for cursor-span hiding.
        let input = "see [[Target|Alias]] here";
        let spans = get_spans(input);
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::WikiLink));
        // Full span spans `[[Target|Alias]]` — 16 UTF-16 code units from offset 4.
        assert_eq!(spans[0].utf16_start, 4);
        assert_eq!(spans[0].utf16_end, 20);
        // Content range covers `Alias` — 5 units starting after `[[Target|`.
        // `[[` + `Target|` = 2 + 7 = 9, so content starts at 4 + 9 = 13.
        assert_eq!(spans[0].content_utf16_start, 13);
        assert_eq!(spans[0].content_utf16_end, 18);
    }

    #[test]
    fn wiki_link_without_alias_content_range_covers_target() {
        let input = "see [[Target]] here";
        let spans = get_spans(input);
        assert_eq!(spans.len(), 1);
        // Content = target since no alias: offset 6..12 (the `Target` text).
        assert_eq!(spans[0].content_utf16_start, 6);
        assert_eq!(spans[0].content_utf16_end, 12);
    }

    #[test]
    fn wiki_link_alias_with_spaces() {
        let spans = get_spans("[[Long Target Name|Short Name]]");
        assert_eq!(spans.len(), 1);
        // `Short Name` begins right after `[[Long Target Name|`.
        assert_eq!(spans[0].content_utf16_start, 19);
        assert_eq!(spans[0].content_utf16_end, 29);
    }

    // MARK: - Hex color literals

    fn get_hex_span(input: &str) -> Option<(String, u32, u32)> {
        get_spans(input).into_iter().find_map(|s| {
            if let InlineKind::HexColor { hex } = &s.kind {
                Some((hex.clone(), s.utf16_start, s.utf16_end))
            } else {
                None
            }
        })
    }

    #[test]
    fn hex_color_six_digit_uppercase() {
        let (hex, _, _) = get_hex_span("the color #0E7BFE today").expect("hex span");
        assert_eq!(hex, "0e7bfe");
    }

    #[test]
    fn hex_color_six_digit_lowercase() {
        let (hex, _, _) = get_hex_span("color #e34bfe here").expect("hex span");
        assert_eq!(hex, "e34bfe");
    }

    #[test]
    fn hex_color_three_digit_expands() {
        let (hex, _, _) = get_hex_span("short #f0a!").expect("hex span");
        assert_eq!(hex, "ff00aa");
    }

    #[test]
    fn hex_color_four_digit_drops_alpha() {
        let (hex, _, _) = get_hex_span("with alpha #f0a8 here").expect("hex span");
        assert_eq!(hex, "ff00aa");
    }

    #[test]
    fn hex_color_eight_digit_drops_alpha() {
        let (hex, _, _) = get_hex_span("long alpha #0e7bfe80 here").expect("hex span");
        assert_eq!(hex, "0e7bfe");
    }

    #[test]
    fn hex_color_at_start_of_text() {
        let (hex, start, _) = get_hex_span("#ff0000 leads").expect("hex span");
        assert_eq!(hex, "ff0000");
        assert_eq!(start, 0);
    }

    #[test]
    fn hex_color_rejects_mid_word() {
        // `a#ff0000` — preceded by a letter, no swatch.
        let span = get_hex_span("a#ff0000");
        assert!(span.is_none(), "Mid-word # must not match, got {:?}", span);
    }

    #[test]
    fn hex_color_rejects_trailing_alpha_beyond_8() {
        // `#ff0000ghi` — 6 hex digits followed by a letter → not a valid boundary.
        let span = get_hex_span("color #ff0000ghi here");
        assert!(
            span.is_none(),
            "Hex followed by letter must not match, got {:?}",
            span
        );
    }

    #[test]
    fn hex_color_rejects_non_color_length() {
        // `#ab` (2 digits) and `#abcde` (5 digits) — not a valid hex color length.
        assert!(get_hex_span("is #ab here").is_none());
        assert!(get_hex_span("is #abcde here").is_none());
        assert!(
            get_hex_span("is #abcdefg here").is_none(),
            "7 digits invalid"
        );
    }

    #[test]
    fn hex_color_rejects_hashtag_with_letters() {
        // `#general` — contains `g`, not a hex digit → no match.
        assert!(get_hex_span("see #general channel").is_none());
    }

    #[test]
    fn hex_color_escaped_hash_is_literal() {
        let span = get_hex_span(r"literal \#ff0000 please");
        assert!(
            span.is_none(),
            "Escaped # must not start a hex, got {:?}",
            span
        );
    }

    #[test]
    fn hex_color_inside_code_span_ignored() {
        let span = get_hex_span("`#ff0000`");
        assert!(span.is_none(), "Hex inside inline code must not match");
    }

    #[test]
    fn hex_color_inside_comment_ignored() {
        let span = get_hex_span("%%#ff0000 draft%%");
        assert!(span.is_none(), "Hex inside comment must not match");
    }

    #[test]
    fn hex_color_link_fragment_not_hex() {
        // `[home](#top)` — the `#top` is a URL fragment; the link claims it first,
        // and `#top` wouldn't be a hex anyway (t is not a hex digit).
        let spans = get_spans("[home](#top)");
        assert!(spans
            .iter()
            .any(|s| matches!(s.kind, InlineKind::Link { .. })));
        assert!(!spans
            .iter()
            .any(|s| matches!(s.kind, InlineKind::HexColor { .. })));
    }

    #[test]
    fn hex_color_two_in_sequence() {
        let spans = get_spans("compare #0e7bfe and #e34bfe");
        let hexes: Vec<_> = spans
            .iter()
            .filter_map(|s| {
                if let InlineKind::HexColor { hex } = &s.kind {
                    Some(hex.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(hexes, vec!["0e7bfe", "e34bfe"]);
    }

    // MARK: - Hidden comments %%...%%

    #[test]
    fn comment_basic() {
        let spans = get_spans("keep %%hidden note%% visible");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::Comment));
    }

    #[test]
    fn comment_suppresses_inner_formatting() {
        // `%%**bold**%%` — inner bold is inside a claimed comment range, so no bold span.
        let spans = get_spans("%%**not bold**%%");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::Comment));
    }

    #[test]
    fn comment_does_not_claim_inside_code() {
        // `` `%%literal%%` `` — inline code wins, no Comment span.
        let spans = get_spans("`%%literal%%`");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::InlineCode));
    }

    #[test]
    fn comment_claims_wiki_link_inside() {
        // `%%[[foo]]%%` — the comment claims the full range, so no wiki span emitted.
        let spans = get_spans("%%[[foo]]%%");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::Comment));
    }

    #[test]
    fn comment_unmatched_percent_does_nothing() {
        // A stray `%%` with no close is plain text.
        let spans = get_spans("only one %% fence here");
        assert!(
            spans.iter().all(|s| !matches!(s.kind, InlineKind::Comment)),
            "Unmatched %% should not emit a Comment, got: {:?}",
            spans
        );
    }

    #[test]
    fn comment_escaped_open_not_a_fence() {
        let spans = get_spans(r"text \%%not a comment%% end");
        let comments: Vec<_> = spans
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::Comment))
            .collect();
        assert!(
            comments.is_empty(),
            "Escaped %% should not open a comment, got: {:?}",
            spans
        );
    }

    #[test]
    fn wiki_link_escaped_pipe_not_alias_separator() {
        // `[[Foo\|Bar]]` — the `\|` is an escaped pipe, so the target is
        // literal `Foo\|Bar` and there's no alias. Content covers the full body.
        let spans = get_spans(r"[[Foo\|Bar]]");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content_utf16_start, 2);
        // Full body length: `Foo\|Bar` = 8 chars, so content ends at 2 + 8 = 10.
        assert_eq!(spans[0].content_utf16_end, 10);
    }

    #[test]
    fn markdown_link() {
        let spans = get_spans("[click here](https://example.com)");
        assert_eq!(spans.len(), 1);
        if let InlineKind::Link { url } = &spans[0].kind {
            assert_eq!(url, "https://example.com");
        } else {
            panic!("Expected Link");
        }
    }

    #[test]
    fn html_underline() {
        let spans = get_spans("<u>underlined</u>");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::UnderlineHtml));
    }

    #[test]
    fn tilde_underline() {
        let spans = get_spans("~underlined~");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::UnderlineTilde));
    }

    #[test]
    fn footnote_ref() {
        let spans = get_spans("text[^1] here");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::FootnoteRef));
    }

    #[test]
    fn code_prevents_emphasis() {
        // Inline code should prevent * inside from being emphasis
        let spans = get_spans("`*not italic*`");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::InlineCode));
    }

    #[test]
    fn multiple_inline_spans() {
        let spans = get_spans("**bold** and *italic*");
        assert_eq!(spans.len(), 2);
        assert!(matches!(spans[0].kind, InlineKind::Bold));
        assert!(matches!(spans[1].kind, InlineKind::Italic));
    }

    #[test]
    fn no_inline_in_code_block() {
        let doc = parse("```\n**not bold**\n```", ParseMode::Grouped);
        let all_spans: Vec<_> = doc
            .blocks
            .iter()
            .flat_map(|b| b.inline_spans.iter())
            .collect();
        assert!(all_spans.is_empty());
    }

    #[test]
    fn utf16_offsets_for_inline() {
        // "**hi**" = 8 bytes, 8 UTF-16
        let spans = get_spans("**hi**");
        assert_eq!(spans[0].utf16_start, 0);
        assert_eq!(spans[0].utf16_end, 6);
        assert_eq!(spans[0].content_utf16_start, 2);
        assert_eq!(spans[0].content_utf16_end, 4);
    }

    // MARK: - Nested emphasis tests

    #[test]
    fn nested_italic_inside_bold() {
        // **bold *italic* bold** → Bold wrapping everything, Italic inside
        let spans = get_spans("**bold *italic* bold**");
        assert_eq!(spans.len(), 2, "Expected Bold + Italic, got: {:?}", spans);
        // Outer Bold span
        let bold = spans
            .iter()
            .find(|s| matches!(s.kind, InlineKind::Bold))
            .unwrap();
        // Inner Italic span
        let italic = spans
            .iter()
            .find(|s| matches!(s.kind, InlineKind::Italic))
            .unwrap();
        // Bold should contain Italic
        assert!(bold.utf16_start <= italic.utf16_start);
        assert!(bold.utf16_end >= italic.utf16_end);
    }

    #[test]
    fn nested_bold_inside_italic() {
        // *italic **bold** italic* → Italic wrapping, Bold inside
        let spans = get_spans("*italic **bold** italic*");
        assert_eq!(spans.len(), 2, "Expected Italic + Bold, got: {:?}", spans);
        let italic = spans
            .iter()
            .find(|s| matches!(s.kind, InlineKind::Italic))
            .unwrap();
        let bold = spans
            .iter()
            .find(|s| matches!(s.kind, InlineKind::Bold))
            .unwrap();
        assert!(italic.utf16_start <= bold.utf16_start);
        assert!(italic.utf16_end >= bold.utf16_end);
    }

    #[test]
    fn bold_italic_triple_asterisk() {
        let spans = get_spans("***bold italic***");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::BoldItalic));
    }

    #[test]
    fn empty_delimiters_produce_no_spans() {
        // **** with no content should not produce spans
        let spans = get_spans("****");
        assert!(
            spans.is_empty(),
            "Empty delimiters should produce no spans, got: {:?}",
            spans
        );
    }

    #[test]
    fn adjacent_bold_then_italic() {
        // **bold** *italic* — two separate non-overlapping spans
        let spans = get_spans("**bold** *italic*");
        assert_eq!(spans.len(), 2);
        assert!(matches!(spans[0].kind, InlineKind::Bold));
        assert!(matches!(spans[1].kind, InlineKind::Italic));
    }

    #[test]
    fn five_asterisks_produces_bold_italic() {
        // *****text***** → should produce BoldItalic (3) at minimum
        let spans = get_spans("*****text*****");
        assert!(!spans.is_empty(), "Five asterisks should produce emphasis");
        // Should have BoldItalic at least
        assert!(spans
            .iter()
            .any(|s| matches!(s.kind, InlineKind::BoldItalic)));
    }

    // MARK: - Multi-backtick inline code tests

    #[test]
    fn double_backtick_code() {
        // ``code with ` backtick`` — double-backtick delimiters
        let spans = get_spans("``code with ` backtick``");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::InlineCode));
    }

    #[test]
    fn triple_backtick_inline_code() {
        // ``` code with `` inside ``` — triple backtick delimiters
        let spans = get_spans("text ```code with `` inside``` more");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::InlineCode));
    }

    #[test]
    fn mismatched_backtick_counts_no_match() {
        // `code`` — single backtick opener, double closer → no match
        // Per CommonMark, a backtick run of N can only be closed by a run of exactly N.
        // The `` is a 2-backtick run, not two separate single backticks.
        let spans = get_spans("text `code`` more");
        assert!(
            spans.is_empty(),
            "Mismatched backtick runs should not match, got: {:?}",
            spans
        );
    }

    #[test]
    fn backtick_run_requires_exact_match() {
        // `` code ` more `` — the single ` inside doesn't close the ``
        let spans = get_spans("``code ` more``");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::InlineCode));
        // Content should include the inner backtick
        // Content range: after `` to before `` = "code ` more"
    }

    // MARK: - CommonMark Unicode flanking tests

    #[test]
    fn emphasis_after_open_paren() {
        // "(*foo*)" — asterisks adjacent to punctuation should still work
        // The opening * is preceded by ( (punctuation) → left-flanking condition 2 satisfied
        // The closing * is followed by ) (punctuation) → right-flanking condition 2 satisfied
        let spans = get_spans("(*foo*)");
        assert_eq!(
            spans.len(),
            1,
            "Emphasis inside parens should work, got: {:?}",
            spans
        );
        assert!(matches!(spans[0].kind, InlineKind::Italic));
    }

    #[test]
    fn emphasis_inside_cjk_brackets() {
        // 「*foo*」— CJK brackets are Unicode punctuation
        let spans = get_spans("\u{300C}*foo*\u{300D}");
        assert_eq!(
            spans.len(),
            1,
            "Emphasis inside CJK brackets should work, got: {:?}",
            spans
        );
        assert!(matches!(spans[0].kind, InlineKind::Italic));
    }

    #[test]
    fn emphasis_inside_smart_quotes() {
        // "*foo*" — smart quotes are Unicode punctuation (Pi/Pf)
        let spans = get_spans("\u{201C}*foo*\u{201D}");
        assert_eq!(
            spans.len(),
            1,
            "Emphasis inside smart quotes should work, got: {:?}",
            spans
        );
        assert!(matches!(spans[0].kind, InlineKind::Italic));
    }

    #[test]
    fn emphasis_with_fullwidth_punctuation() {
        // ！*foo*！ — fullwidth punctuation
        let spans = get_spans("\u{FF01}*foo*\u{FF01}");
        assert_eq!(
            spans.len(),
            1,
            "Emphasis with fullwidth punctuation, got: {:?}",
            spans
        );
        assert!(matches!(spans[0].kind, InlineKind::Italic));
    }

    #[test]
    fn no_emphasis_when_preceded_by_word_and_followed_by_punctuation() {
        // CommonMark example 386: foo*bar* — intraword emphasis with * DOES work
        let spans = get_spans("foo*bar*");
        assert_eq!(
            spans.len(),
            1,
            "Intraword emphasis should work with *, got: {:?}",
            spans
        );
        assert!(matches!(spans[0].kind, InlineKind::Italic));
    }

    #[test]
    fn emphasis_preceded_by_no_break_space() {
        // \u{00A0}*foo* — no-break space is Unicode whitespace, so * at position after it
        // is left-flanking (not preceded by word char — the space counts)
        let spans = get_spans("\u{00A0}*foo*");
        assert_eq!(
            spans.len(),
            1,
            "Emphasis after no-break space should work, got: {:?}",
            spans
        );
        assert!(matches!(spans[0].kind, InlineKind::Italic));
    }

    // MARK: - AutoLink tests

    #[test]
    fn autolink_https() {
        let spans = get_spans("Visit https://example.com today");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "https://example.com");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn autolink_http() {
        let spans = get_spans("See http://example.org/path");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "http://example.org/path");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn autolink_with_path_and_query() {
        let spans = get_spans("https://example.com/path?q=test&lang=en#section");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "https://example.com/path?q=test&lang=en#section");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn autolink_strips_trailing_period() {
        let spans = get_spans("Visit https://example.com.");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "https://example.com");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn autolink_strips_trailing_comma() {
        let spans = get_spans("See https://a.com, https://b.com");
        assert_eq!(spans.len(), 2);
    }

    #[test]
    fn autolink_preserves_balanced_parens() {
        // Wikipedia-style URLs with balanced parentheses.
        let spans = get_spans("https://en.wikipedia.org/wiki/Rust_(programming_language)");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(
                url,
                "https://en.wikipedia.org/wiki/Rust_(programming_language)"
            );
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn autolink_unmatched_closing_paren() {
        // URL followed by sentence-level closing paren.
        let spans = get_spans("(see https://example.com)");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "https://example.com");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn autolink_not_inside_markdown_link() {
        // URL inside [text](url) should NOT produce an AutoLink — it's already a Link.
        let spans = get_spans("[click](https://example.com)");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::Link { .. }));
    }

    #[test]
    fn autolink_not_inside_inline_code() {
        let spans = get_spans("`https://example.com`");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::InlineCode));
    }

    #[test]
    fn autolink_multiple_on_line() {
        let spans = get_spans("Visit https://a.com and https://b.com today");
        let autolinks: Vec<_> = spans
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::AutoLink { .. }))
            .collect();
        assert_eq!(autolinks.len(), 2);
    }

    #[test]
    fn autolink_at_line_start() {
        let spans = get_spans("https://example.com is great");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::AutoLink { .. }));
        assert_eq!(spans[0].utf16_start, 0);
    }

    #[test]
    fn autolink_at_line_end() {
        let spans = get_spans("Visit https://example.com");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "https://example.com");
        } else {
            panic!("Expected AutoLink");
        }
    }

    #[test]
    fn autolink_no_scheme_only() {
        // Just "https://" with nothing after should not match.
        let spans = get_spans("See https:// here");
        let autolinks: Vec<_> = spans
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::AutoLink { .. }))
            .collect();
        assert_eq!(autolinks.len(), 0);
    }

    #[test]
    fn autolink_with_bold() {
        let spans = get_spans("**bold** and https://example.com");
        assert_eq!(spans.len(), 2);
        assert!(matches!(spans[0].kind, InlineKind::Bold));
        assert!(matches!(spans[1].kind, InlineKind::AutoLink { .. }));
    }

    #[test]
    fn autolink_content_equals_full_range() {
        // AutoLinks have no delimiters — content range equals full range.
        let spans = get_spans("https://example.com");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].utf16_start, spans[0].content_utf16_start);
        assert_eq!(spans[0].utf16_end, spans[0].content_utf16_end);
    }

    // MARK: - Domain-based autolink tests

    #[test]
    fn autolink_domain_com() {
        let spans = get_spans("Visit google.com today");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "https://google.com");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn autolink_domain_io() {
        let spans = get_spans("Check out rede.io for notes");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "https://rede.io");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn autolink_domain_with_path() {
        let spans = get_spans("See docs.example.org/guide");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "https://docs.example.org/guide");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn autolink_domain_with_subdomain() {
        let spans = get_spans("Visit www.example.com");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "https://www.example.com");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn autolink_domain_world_tld() {
        // .world is in COMMON_TLDS — should be detected.
        let spans = get_spans("Visit hello.world today");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "https://hello.world");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn autolink_domain_unknown_tld_ignored() {
        // .foobar is not in COMMON_TLDS — should NOT be detected.
        let spans = get_spans("hello.foobar is not a link");
        let autolinks: Vec<_> = spans
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::AutoLink { .. }))
            .collect();
        assert_eq!(autolinks.len(), 0);
    }

    #[test]
    fn autolink_domain_strips_trailing_period() {
        let spans = get_spans("Visit google.com.");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "https://google.com");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn autolink_domain_in_parens() {
        let spans = get_spans("(see google.com)");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "https://google.com");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn autolink_domain_not_inside_scheme_url() {
        // google.com inside https://google.com should NOT produce TWO autolinks.
        let spans = get_spans("Visit https://google.com today");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "https://google.com");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn autolink_domain_multiple() {
        let spans = get_spans("See google.com and rede.io");
        let autolinks: Vec<_> = spans
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::AutoLink { .. }))
            .collect();
        assert_eq!(autolinks.len(), 2);
    }

    #[test]
    fn autolink_domain_dev_tld() {
        let spans = get_spans("Check example.dev");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "https://example.dev");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    // MARK: - Subreddit autolinks

    #[test]
    fn autolink_subreddit_simple() {
        let spans = get_spans("Post this to r/apple");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "https://www.reddit.com/r/apple");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn autolink_subreddit_trailing_punctuation() {
        let spans = get_spans("Try r/SwiftUI, then r/iOS.");
        let urls: Vec<_> = spans
            .iter()
            .filter_map(|span| {
                if let InlineKind::AutoLink { url } = &span.kind {
                    Some(url.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            urls,
            vec![
                "https://www.reddit.com/r/SwiftUI",
                "https://www.reddit.com/r/iOS",
            ]
        );
    }

    #[test]
    fn autolink_subreddit_not_inside_existing_url_or_code() {
        let spans = get_spans("https://www.reddit.com/r/apple and `r/swift`");
        assert_eq!(spans.len(), 2);
        assert!(matches!(spans[0].kind, InlineKind::AutoLink { .. }));
        assert!(matches!(spans[1].kind, InlineKind::InlineCode));
    }

    #[test]
    fn autolink_subreddit_after_textkit_attachment_marker() {
        let spans = get_spans("\u{FFFC}r/apple\n\u{FFFC}r/SwiftUI");
        let urls: Vec<_> = spans
            .iter()
            .filter_map(|span| {
                if let InlineKind::AutoLink { url } = &span.kind {
                    Some(url.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            urls,
            vec![
                "https://www.reddit.com/r/apple",
                "https://www.reddit.com/r/SwiftUI",
            ]
        );
    }

    #[test]
    fn autolink_subreddit_requires_boundary() {
        let spans = get_spans("cr/apple and r/a are not subreddit shorthand");
        let autolinks: Vec<_> = spans
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::AutoLink { .. }))
            .collect();
        assert_eq!(autolinks.len(), 0);
    }

    // MARK: - Angle-bracket autolinks (CommonMark §6.4)

    #[test]
    fn angle_autolink_https() {
        let spans = get_spans("Visit <https://example.com> today");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "https://example.com");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn angle_autolink_http() {
        let spans = get_spans("<http://example.org/path>");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "http://example.org/path");
        } else {
            panic!("Expected AutoLink");
        }
    }

    #[test]
    fn angle_autolink_custom_scheme() {
        // CommonMark allows any 2-32 char scheme.
        let spans = get_spans("<ftp://files.example.com>");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "ftp://files.example.com");
        } else {
            panic!("Expected AutoLink");
        }
    }

    #[test]
    fn angle_autolink_email() {
        let spans = get_spans("Email <foo@bar.com> for help");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "mailto:foo@bar.com");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn angle_autolink_email_with_plus_alias() {
        let spans = get_spans("<foo+filter@example.com>");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "mailto:foo+filter@example.com");
        } else {
            panic!("Expected AutoLink");
        }
    }

    #[test]
    fn angle_autolink_rejects_whitespace_inside() {
        // `<foo bar>` is not a valid autolink — embedded space disqualifies it.
        let spans = get_spans("<foo bar>");
        let auto: Vec<_> = spans
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::AutoLink { .. }))
            .collect();
        assert!(
            auto.is_empty(),
            "Whitespace inside <…> disqualifies autolink"
        );
    }

    #[test]
    fn angle_autolink_rejects_bare_word() {
        // `<foo>` is not a URI (no scheme `:`) and not an email (no `@`).
        let spans = get_spans("see <foo> for details");
        let auto: Vec<_> = spans
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::AutoLink { .. }))
            .collect();
        assert!(auto.is_empty(), "<foo> alone is not an autolink");
    }

    #[test]
    fn angle_autolink_escaped_open_falls_back_to_bare_url() {
        // `\<` skips the angle scanner, but the bare URL scanner still detects
        // `https://example.com`. We assert the autolink span doesn't include
        // the `<` / `>` (i.e. it's the bare URL form, not the angle form).
        let spans = get_spans(r"\<https://example.com>");
        let auto: Vec<_> = spans
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::AutoLink { .. }))
            .collect();
        assert_eq!(auto.len(), 1);
        if let InlineKind::AutoLink { url } = &auto[0].kind {
            assert_eq!(url, "https://example.com");
        }
    }

    #[test]
    fn angle_autolink_does_not_collide_with_html_underline() {
        // `<u>text</u>` is HTML underline, not an autolink (`u` isn't a valid scheme).
        let spans = get_spans("<u>underlined</u>");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::UnderlineHtml));
    }

    // MARK: - Bare email autolinks (GFM-style, ported from b505126)

    #[test]
    fn bare_email_simple() {
        let spans = get_spans("Email rene@embernotes.app for support");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "mailto:rene@embernotes.app");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn bare_email_with_dots_and_subdomain() {
        let spans = get_spans("first.last@mail.example.co.uk");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "mailto:first.last@mail.example.co.uk");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn bare_email_with_plus_tag() {
        let spans = get_spans("user+tag@domain.com");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "mailto:user+tag@domain.com");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn bare_email_strips_trailing_period() {
        let spans = get_spans("email me at rene@embernotes.app.");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "mailto:rene@embernotes.app");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn bare_email_trims_leading_dots() {
        let spans = get_spans(".foo@example.com");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "mailto:foo@example.com");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn bare_email_rejects_trailing_dot_in_local() {
        let spans = get_spans("foo.@example.com");
        assert!(spans.is_empty(), "Expected no spans, got {:?}", spans);
    }

    #[test]
    fn bare_email_rejects_one_letter_tld() {
        let spans = get_spans("user@domain.x");
        assert!(spans.is_empty(), "Single-letter TLD should not match");
    }

    #[test]
    fn bare_email_rejects_empty_local() {
        let spans = get_spans("@example.com is not an email");
        assert!(spans.is_empty(), "Empty local part should not match");
    }

    #[test]
    fn bare_email_in_parens() {
        let spans = get_spans("(contact rene@embernotes.app)");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "mailto:rene@embernotes.app");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn bare_email_at_line_start() {
        let spans = get_spans("rene@embernotes.app is the best email");
        assert_eq!(spans.len(), 1);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "mailto:rene@embernotes.app");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn bare_email_claims_domain_before_scheme_url_scan() {
        // Regression: `user@example.com` must be claimed as a single email
        // range so `example.com` is not ALSO matched as a bare domain URL.
        let spans = get_spans("Reach rene@embernotes.app now");
        assert_eq!(spans.len(), 1, "Expected exactly 1 span, got {:?}", spans);
        if let InlineKind::AutoLink { url } = &spans[0].kind {
            assert_eq!(url, "mailto:rene@embernotes.app");
        } else {
            panic!("Expected AutoLink, got {:?}", spans[0].kind);
        }
    }

    #[test]
    fn bare_email_alongside_bare_url() {
        let spans = get_spans("Visit https://embernotes.app or email rene@embernotes.app today");
        assert_eq!(spans.len(), 2, "Expected 2 spans, got {:?}", spans);
    }

    #[test]
    fn bare_email_escaped_at_sign_ignored() {
        let spans = get_spans("use foo\\@bar.com literally");
        assert!(spans.is_empty(), "Escaped @ should not match");
    }

    #[test]
    fn bare_email_inside_markdown_link_ignored() {
        let spans = get_spans("[write me](mailto:rene@embernotes.app)");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::Link { .. }));
    }

    // MARK: - Backslash escapes (CommonMark §6.1)

    #[test]
    fn escaped_asterisk_no_emphasis() {
        // \*not italic\* — escaped asterisks must not open/close emphasis.
        let spans = get_spans(r"\*not italic\*");
        assert!(
            spans.is_empty(),
            "Escaped asterisks should produce no emphasis span, got: {:?}",
            spans
        );
    }

    #[test]
    fn escaped_first_asterisk_then_unescaped_pair_emphasizes() {
        // `\**foo\**` — first `*` escaped, second unescaped opens italic.
        // Closer pair at end: `\` escapes its `*`, but the following `*` opens/closes
        // unescaped. This matches CommonMark — only the immediately-preceded char is escaped.
        let spans = get_spans(r"\**foo\**");
        let emph: Vec<_> = spans
            .iter()
            .filter(|s| {
                matches!(
                    s.kind,
                    InlineKind::Bold | InlineKind::Italic | InlineKind::BoldItalic
                )
            })
            .collect();
        assert_eq!(
            emph.len(),
            1,
            "Unescaped *s should still pair, got: {:?}",
            spans
        );
        assert!(matches!(emph[0].kind, InlineKind::Italic));
    }

    #[test]
    fn fully_escaped_double_asterisk_no_bold() {
        // `\*\*not bold\*\*` — every asterisk individually escaped → no emphasis at all.
        let spans = get_spans(r"\*\*not bold\*\*");
        let emph: Vec<_> = spans
            .iter()
            .filter(|s| {
                matches!(
                    s.kind,
                    InlineKind::Bold | InlineKind::Italic | InlineKind::BoldItalic
                )
            })
            .collect();
        assert!(
            emph.is_empty(),
            "All-escaped *s should not bold, got: {:?}",
            spans
        );
    }

    #[test]
    fn double_backslash_then_asterisk_still_emphasizes() {
        // `\\*foo*` — `\\` is literal backslash, then `*foo*` is normal italic.
        let spans = get_spans(r"\\*foo*");
        assert_eq!(
            spans.len(),
            1,
            "Even backslash run leaves * unescaped, got: {:?}",
            spans
        );
        assert!(matches!(spans[0].kind, InlineKind::Italic));
    }

    #[test]
    fn escaped_backtick_no_code() {
        let spans = get_spans(r"text \`code\` more");
        assert!(
            spans.is_empty(),
            "Escaped backticks should not create code, got: {:?}",
            spans
        );
    }

    #[test]
    fn escaped_bracket_no_link() {
        let spans = get_spans(r"text \[click](url) more");
        assert!(
            spans.is_empty(),
            "Escaped [ should not start a link, got: {:?}",
            spans
        );
    }

    #[test]
    fn escaped_double_bracket_no_wiki_link() {
        let spans = get_spans(r"text \[[Note]] more");
        let wiki: Vec<_> = spans
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::WikiLink))
            .collect();
        assert!(
            wiki.is_empty(),
            "Escaped [[ should not create wiki link, got: {:?}",
            spans
        );
    }

    #[test]
    fn escaped_tilde_no_strikethrough() {
        let spans = get_spans(r"text \~~not strike\~~ more");
        let strike: Vec<_> = spans
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::Strikethrough))
            .collect();
        assert!(
            strike.is_empty(),
            "Escaped ~~ should not strike, got: {:?}",
            spans
        );
    }

    #[test]
    fn escaped_single_tilde_no_underline() {
        let spans = get_spans(r"text \~not underlined\~ more");
        let und: Vec<_> = spans
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::UnderlineTilde))
            .collect();
        assert!(
            und.is_empty(),
            "Escaped ~ should not underline, got: {:?}",
            spans
        );
    }

    #[test]
    fn escaped_equals_no_highlight() {
        let spans = get_spans(r"text \==not highlight\== more");
        let hi: Vec<_> = spans
            .iter()
            .filter(|s| {
                matches!(
                    s.kind,
                    InlineKind::Highlight | InlineKind::HighlightColor(_)
                )
            })
            .collect();
        assert!(
            hi.is_empty(),
            "Escaped == should not highlight, got: {:?}",
            spans
        );
    }

    #[test]
    fn escaped_html_underline_open() {
        let spans = get_spans(r"text \<u>nope</u> more");
        let und: Vec<_> = spans
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::UnderlineHtml))
            .collect();
        assert!(
            und.is_empty(),
            "Escaped <u> should not underline, got: {:?}",
            spans
        );
    }

    #[test]
    fn escaped_footnote_ref() {
        let spans = get_spans(r"text \[^1] more");
        let fn_refs: Vec<_> = spans
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::FootnoteRef))
            .collect();
        assert!(
            fn_refs.is_empty(),
            "Escaped [^ should not create footnote ref, got: {:?}",
            spans
        );
    }

    #[test]
    fn unescaped_emphasis_still_works_after_escape() {
        // \* literal then *real* italic
        let spans = get_spans(r"\* literal then *italic*");
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].kind, InlineKind::Italic));
    }

    #[test]
    fn escaped_asterisk_inside_unescaped_emphasis() {
        // *foo \* bar* — the inner \* should be literal but the outer * pair forms italic.
        let spans = get_spans(r"*foo \* bar*");
        // The opening * (pos 0) is unescaped; the * at pos 5 is escaped (literal);
        // the closing * (pos 11) is unescaped → italic spans 0..12.
        let italics: Vec<_> = spans
            .iter()
            .filter(|s| matches!(s.kind, InlineKind::Italic))
            .collect();
        assert_eq!(
            italics.len(),
            1,
            "Outer italic should still match, got: {:?}",
            spans
        );
    }

    #[test]
    fn three_backslashes_then_asterisk_escapes() {
        // `\\\*` — \\ is literal `\`, then \* is escaped `*`. Total: literal `\*`.
        let spans = get_spans(r"\\\*not italic\\\*");
        assert!(
            spans.is_empty(),
            "Odd backslash run escapes the *, got: {:?}",
            spans
        );
    }
}
