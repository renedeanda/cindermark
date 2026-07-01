//! SIMD-accelerated UTF-8 byte scanner producing a token stream.
//!
//! The lexer operates in two modes per line:
//! 1. **Line-start mode**: Attempts to match block-level markers (headings, lists,
//!    code fences, etc.). If matched, the rest of the line is scanned in inline mode.
//! 2. **Inline mode**: Emits delimiter tokens for inline formatting and text runs.
//!
//! Uses `memchr` for SIMD-accelerated scanning on Apple Silicon (NEON) and x86 (SSE2/AVX2).

// memchr is available for SIMD-accelerated scanning in future optimizations
#[allow(unused_imports)]
use memchr::memchr;

/// A token produced by the lexer.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    /// Byte offset in source where this token starts.
    pub byte_start: u32,
    /// Byte length of this token.
    pub byte_len: u32,
}

impl Token {
    fn new(kind: TokenKind, byte_start: u32, byte_len: u32) -> Self {
        Self {
            kind,
            byte_start,
            byte_len,
        }
    }
}

/// Token kinds produced by the lexer.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Block-level markers (valid at line start)
    HeadingMarker(u8),    // level 1-6, includes trailing space
    CodeFence(u8),        // backtick count (3+)
    CodeFenceLanguage,    // language hint after opening fence
    BlockquoteMarker,     // "> " or ">"
    BulletMarker,         // "- " or "* " or "+ "
    OrderedMarker(u32),   // "1. " etc — the number
    CheckboxMarker(bool), // "- [ ] " (false) or "- [x] " / "- [X] " (true)
    HorizontalRule,       // "---" / "***" / "___" (3+ chars, sole content)
    TablePipe,            // "|"
    FootnoteDefMarker,    // "[^label]:" — content is the label
    ImageMarker,          // "![](<scheme>...)" — full marker (opt-in extension)

    // Inline delimiters
    Asterisks(u8),      // 1-3 consecutive *
    Tildes(u8),         // 1-2 consecutive ~
    Backtick,           // single `
    DoubleEquals,       // ==
    OpenBracket,        // [
    CloseBracket,       // ]
    OpenParen,          // (
    CloseParen,         // )
    DoubleOpenBracket,  // [[
    DoubleCloseBracket, // ]]
    FootnoteRefOpen,    // [^
    HtmlUnderlineOpen,  // <u>
    HtmlUnderlineClose, // </u>
    /// Color emoji after ==: 0=red, 1=orange, 2=yellow, 3=green, 4=blue, 5=purple
    ColorEmojiPrefix(u8),

    // Content
    Text,    // plain text run
    Newline, // \n
    Eof,
}

/// The lexer state machine.
pub struct Lexer<'a> {
    source: &'a [u8],
    pos: usize,
    /// URI-scheme prefix for `![](<scheme>...)` image markers, or `None`
    /// when the extension is disabled. See `parser::ParseOptions`.
    image_marker_scheme: Option<&'a str>,
    /// Tokens produced by the lexer.
    pub tokens: Vec<Token>,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a [u8]) -> Self {
        Self::with_image_marker_scheme(source, None)
    }

    pub fn with_image_marker_scheme(source: &'a [u8], scheme: Option<&'a str>) -> Self {
        Self {
            source,
            pos: 0,
            image_marker_scheme: scheme,
            tokens: Vec::with_capacity(source.len() / 4 + 16),
        }
    }

    /// Tokenize the entire source.
    pub fn tokenize(&mut self) {
        while self.pos < self.source.len() {
            self.scan_line();
        }
        self.tokens
            .push(Token::new(TokenKind::Eof, self.pos as u32, 0));
    }

    /// Scan a single line, starting in line-start mode.
    fn scan_line(&mut self) {
        let _line_start = self.pos;

        // Skip leading whitespace (but track it)
        let indent = self.skip_spaces();

        if self.pos >= self.source.len() {
            return;
        }

        // Check for newline (empty line)
        if self.source[self.pos] == b'\n' {
            self.emit_newline();
            return;
        }

        // Try block-level markers at line start
        if indent < 4
            && (self.try_heading()
                || self.try_code_fence()
                || self.try_checkbox()
                || self.try_bullet()
                || self.try_ordered_list()
                || self.try_horizontal_rule()
                || self.try_blockquote()
                || self.try_footnote_def()
                || self.try_image_marker())
        {
            // Block marker consumed, now scan rest of line as inline content
            self.scan_inline_until_newline();
            return;
        }

        // No block marker matched — scan as inline content (paragraph, table row, etc.)
        // Check if the line contains pipes (potential table row)
        self.scan_inline_until_newline();
    }

    // MARK: - Block marker detection

    fn try_heading(&mut self) -> bool {
        let start = self.pos;
        if self.source[self.pos] != b'#' {
            return false;
        }
        let mut level = 0u8;
        let mut p = self.pos;
        while p < self.source.len() && self.source[p] == b'#' && level < 7 {
            level += 1;
            p += 1;
        }
        if level > 6 {
            return false;
        }
        // Must be followed by a space
        if p >= self.source.len() || self.source[p] != b' ' {
            return false;
        }
        p += 1; // consume the space
        self.tokens.push(Token::new(
            TokenKind::HeadingMarker(level),
            start as u32,
            (p - start) as u32,
        ));
        self.pos = p;
        true
    }

    fn try_code_fence(&mut self) -> bool {
        let start = self.pos;
        if self.source[self.pos] != b'`' {
            return false;
        }
        let mut count = 0u8;
        let mut p = self.pos;
        while p < self.source.len() && self.source[p] == b'`' {
            count += 1;
            p += 1;
        }
        if count < 3 {
            return false;
        }
        self.tokens.push(Token::new(
            TokenKind::CodeFence(count),
            start as u32,
            (p - start) as u32,
        ));
        self.pos = p;

        // Scan language hint (rest of line before newline)
        let lang_start = p;
        while p < self.source.len() && self.source[p] != b'\n' {
            p += 1;
        }
        if p > lang_start {
            let trimmed_end = self.rtrim_pos(lang_start, p);
            if trimmed_end > lang_start {
                self.tokens.push(Token::new(
                    TokenKind::CodeFenceLanguage,
                    lang_start as u32,
                    (trimmed_end - lang_start) as u32,
                ));
            }
        }
        self.pos = p;
        // Don't consume newline here — scan_line will handle it
        true
    }

    fn try_checkbox(&mut self) -> bool {
        let start = self.pos;
        let remaining = self.source.len() - self.pos;
        if remaining < 6 {
            return false;
        }
        let s = &self.source[self.pos..];
        if s[0] == b'-' && s[1] == b' ' && s[2] == b'[' {
            if s[3] == b' ' && s[4] == b']' && s[5] == b' ' {
                self.tokens.push(Token::new(
                    TokenKind::CheckboxMarker(false),
                    start as u32,
                    6,
                ));
                self.pos += 6;
                return true;
            }
            if (s[3] == b'x' || s[3] == b'X') && s[4] == b']' && s[5] == b' ' {
                self.tokens
                    .push(Token::new(TokenKind::CheckboxMarker(true), start as u32, 6));
                self.pos += 6;
                return true;
            }
        }
        false
    }

    fn try_bullet(&mut self) -> bool {
        let start = self.pos;
        if self.pos + 1 >= self.source.len() {
            return false;
        }
        let ch = self.source[self.pos];
        if (ch == b'-' || ch == b'*' || ch == b'+') && self.source[self.pos + 1] == b' ' {
            self.tokens
                .push(Token::new(TokenKind::BulletMarker, start as u32, 2));
            self.pos += 2;
            true
        } else {
            false
        }
    }

    fn try_ordered_list(&mut self) -> bool {
        let start = self.pos;
        let mut p = self.pos;
        // Scan digits
        while p < self.source.len() && self.source[p].is_ascii_digit() {
            p += 1;
        }
        if p == self.pos {
            return false;
        }
        // Must be followed by ". "
        if p + 1 >= self.source.len() || self.source[p] != b'.' || self.source[p + 1] != b' ' {
            return false;
        }
        let num_str = std::str::from_utf8(&self.source[self.pos..p]).unwrap_or("1");
        let num = num_str.parse::<u32>().unwrap_or(1);
        let len = p + 2 - start;
        self.tokens.push(Token::new(
            TokenKind::OrderedMarker(num),
            start as u32,
            len as u32,
        ));
        self.pos = p + 2;
        true
    }

    fn try_horizontal_rule(&mut self) -> bool {
        let start = self.pos;
        let ch = self.source[self.pos];
        if ch != b'-' && ch != b'*' && ch != b'_' {
            return false;
        }
        // Must be 3+ of the same character (with optional spaces), and nothing else on the line
        let mut p = self.pos;
        let mut count = 0u32;
        while p < self.source.len() && self.source[p] != b'\n' {
            if self.source[p] == ch {
                count += 1;
            } else if self.source[p] != b' ' {
                return false;
            }
            p += 1;
        }
        if count < 3 {
            return false;
        }
        // For asterisks, require exactly 3 to avoid conflict with bold (****)
        if ch == b'*' && count != 3 {
            return false;
        }
        self.tokens.push(Token::new(
            TokenKind::HorizontalRule,
            start as u32,
            (p - start) as u32,
        ));
        self.pos = p;
        true
    }

    fn try_blockquote(&mut self) -> bool {
        let start = self.pos;
        if self.source[self.pos] != b'>' {
            return false;
        }
        if self.pos + 1 < self.source.len() && self.source[self.pos + 1] == b' ' {
            self.tokens
                .push(Token::new(TokenKind::BlockquoteMarker, start as u32, 2));
            self.pos += 2;
            true
        } else if self.pos + 1 >= self.source.len() || self.source[self.pos + 1] == b'\n' {
            // Bare ">" at end of line
            self.tokens
                .push(Token::new(TokenKind::BlockquoteMarker, start as u32, 1));
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn try_footnote_def(&mut self) -> bool {
        let start = self.pos;
        let remaining = &self.source[self.pos..];
        if remaining.len() < 4 || remaining[0] != b'[' || remaining[1] != b'^' {
            return false;
        }
        // Find closing ]: pattern
        let mut p = self.pos + 2;
        while p < self.source.len() && self.source[p] != b']' && self.source[p] != b'\n' {
            p += 1;
        }
        if p >= self.source.len() || self.source[p] != b']' {
            return false;
        }
        if p + 1 >= self.source.len() || self.source[p + 1] != b':' {
            return false;
        }
        let label_end = p;
        p += 2; // skip "]:"
                // Skip optional space after colon
        if p < self.source.len() && self.source[p] == b' ' {
            p += 1;
        }
        // Ensure label is not empty
        if label_end <= self.pos + 2 {
            return false;
        }
        self.tokens.push(Token::new(
            TokenKind::FootnoteDefMarker,
            start as u32,
            (p - start) as u32,
        ));
        self.pos = p;
        true
    }

    fn try_image_marker(&mut self) -> bool {
        // Opt-in extension: no scheme configured, no marker tokens.
        let Some(scheme) = self.image_marker_scheme else {
            return false;
        };
        let scheme = scheme.as_bytes();
        let start = self.pos;
        let remaining = &self.source[self.pos..];
        // Match "![](<scheme>" pattern with at least one byte of payload
        // before the closing paren.
        let prefix_len = 4 + scheme.len();
        if remaining.len() <= prefix_len {
            return false;
        }
        if remaining[0] != b'!'
            || remaining[1] != b'['
            || remaining[2] != b']'
            || remaining[3] != b'('
        {
            return false;
        }
        if &remaining[4..prefix_len] != scheme {
            return false;
        }
        // Find closing paren
        let mut p = self.pos + prefix_len;
        while p < self.source.len() && self.source[p] != b')' && self.source[p] != b'\n' {
            p += 1;
        }
        if p >= self.source.len() || self.source[p] != b')' {
            return false;
        }
        p += 1; // include closing paren
        self.tokens.push(Token::new(
            TokenKind::ImageMarker,
            start as u32,
            (p - start) as u32,
        ));
        self.pos = p;
        true
    }

    // MARK: - Inline scanning

    /// Scan inline tokens until end of line (newline or EOF).
    fn scan_inline_until_newline(&mut self) {
        while self.pos < self.source.len() {
            if self.source[self.pos] == b'\n' {
                self.emit_newline();
                return;
            }
            self.scan_inline_token();
        }
    }

    /// Scan a single inline token at the current position.
    fn scan_inline_token(&mut self) {
        let start = self.pos;
        let b = self.source[self.pos];

        match b {
            b'*' => {
                let count = self.count_consecutive(b'*').min(3) as u8;
                self.tokens.push(Token::new(
                    TokenKind::Asterisks(count),
                    start as u32,
                    count as u32,
                ));
            }
            b'~' => {
                let count = self.count_consecutive(b'~').min(2) as u8;
                self.tokens.push(Token::new(
                    TokenKind::Tildes(count),
                    start as u32,
                    count as u32,
                ));
            }
            b'`' => {
                // Single backtick for inline code (don't confuse with code fence)
                self.pos += 1;
                self.tokens
                    .push(Token::new(TokenKind::Backtick, start as u32, 1));
            }
            b'=' if self.pos + 1 < self.source.len() && self.source[self.pos + 1] == b'=' => {
                self.pos += 2;
                // Check for color emoji immediately after ==
                if let Some(color_idx) = self.try_color_emoji() {
                    self.tokens
                        .push(Token::new(TokenKind::DoubleEquals, start as u32, 2));
                    let emoji_start = self.pos;
                    let emoji_len = self.color_emoji_byte_len();
                    self.pos += emoji_len;
                    self.tokens.push(Token::new(
                        TokenKind::ColorEmojiPrefix(color_idx),
                        emoji_start as u32,
                        emoji_len as u32,
                    ));
                } else {
                    self.tokens
                        .push(Token::new(TokenKind::DoubleEquals, start as u32, 2));
                }
            }
            b'[' => {
                if self.pos + 1 < self.source.len() {
                    if self.source[self.pos + 1] == b'[' {
                        self.pos += 2;
                        self.tokens
                            .push(Token::new(TokenKind::DoubleOpenBracket, start as u32, 2));
                    } else if self.source[self.pos + 1] == b'^' {
                        self.pos += 2;
                        self.tokens
                            .push(Token::new(TokenKind::FootnoteRefOpen, start as u32, 2));
                    } else {
                        self.pos += 1;
                        self.tokens
                            .push(Token::new(TokenKind::OpenBracket, start as u32, 1));
                    }
                } else {
                    self.pos += 1;
                    self.tokens
                        .push(Token::new(TokenKind::OpenBracket, start as u32, 1));
                }
            }
            b']' => {
                if self.pos + 1 < self.source.len() && self.source[self.pos + 1] == b']' {
                    self.pos += 2;
                    self.tokens
                        .push(Token::new(TokenKind::DoubleCloseBracket, start as u32, 2));
                } else {
                    self.pos += 1;
                    self.tokens
                        .push(Token::new(TokenKind::CloseBracket, start as u32, 1));
                }
            }
            b'(' => {
                self.pos += 1;
                self.tokens
                    .push(Token::new(TokenKind::OpenParen, start as u32, 1));
            }
            b')' => {
                self.pos += 1;
                self.tokens
                    .push(Token::new(TokenKind::CloseParen, start as u32, 1));
            }
            b'|' => {
                self.pos += 1;
                self.tokens
                    .push(Token::new(TokenKind::TablePipe, start as u32, 1));
            }
            // When the guard fails, `<` falls through to the text-run arm.
            b'<' if self.try_html_underline_open() || self.try_html_underline_close() => {
                // Token already emitted by the guard.
            }
            _ => {
                self.scan_text_run();
            }
        }
    }

    /// Scan a text run until the next interesting byte, using SIMD-accelerated memchr.
    fn scan_text_run(&mut self) {
        let start = self.pos;

        while self.pos < self.source.len() {
            let b = self.source[self.pos];
            // Stop at any character that could be an inline delimiter or newline
            if matches!(
                b,
                b'\n' | b'*' | b'~' | b'`' | b'=' | b'[' | b']' | b'(' | b')' | b'|' | b'<'
            ) {
                break;
            }
            self.pos += self.char_byte_len(self.pos);
        }

        if self.pos > start {
            self.tokens.push(Token::new(
                TokenKind::Text,
                start as u32,
                (self.pos - start) as u32,
            ));
        }
    }

    // MARK: - HTML tag detection

    fn try_html_underline_open(&mut self) -> bool {
        let remaining = &self.source[self.pos..];
        if remaining.len() >= 3
            && remaining[0] == b'<'
            && remaining[1] == b'u'
            && remaining[2] == b'>'
        {
            let start = self.pos;
            self.pos += 3;
            self.tokens
                .push(Token::new(TokenKind::HtmlUnderlineOpen, start as u32, 3));
            true
        } else {
            false
        }
    }

    fn try_html_underline_close(&mut self) -> bool {
        let remaining = &self.source[self.pos..];
        if remaining.len() >= 4
            && remaining[0] == b'<'
            && remaining[1] == b'/'
            && remaining[2] == b'u'
            && remaining[3] == b'>'
        {
            let start = self.pos;
            self.pos += 4;
            self.tokens
                .push(Token::new(TokenKind::HtmlUnderlineClose, start as u32, 4));
            true
        } else {
            false
        }
    }

    // MARK: - Color emoji detection

    /// Check if the current position has a color emoji. Returns color index if found.
    fn try_color_emoji(&self) -> Option<u8> {
        if self.pos + 3 >= self.source.len() {
            return None;
        }
        let s = &self.source[self.pos..];
        if s[0] != 0xF0 || s[1] != 0x9F {
            return None;
        }
        // Match specific color circle emojis
        match (s[2], s[3]) {
            (0x94, 0xB4) => Some(0), // 🔴 U+1F534
            (0x9F, 0xA0) => Some(1), // 🟠 U+1F7E0
            (0x9F, 0xA1) => Some(2), // 🟡 U+1F7E1
            (0x9F, 0xA2) => Some(3), // 🟢 U+1F7E2
            (0x94, 0xB5) => Some(4), // 🔵 U+1F535
            (0x9F, 0xA3) => Some(5), // 🟣 U+1F7E3
            _ => None,
        }
    }

    fn color_emoji_byte_len(&self) -> usize {
        4 // All color emojis are 4-byte UTF-8 sequences
    }

    // MARK: - Utility

    fn emit_newline(&mut self) {
        self.tokens
            .push(Token::new(TokenKind::Newline, self.pos as u32, 1));
        self.pos += 1;
    }

    fn skip_spaces(&mut self) -> usize {
        let start = self.pos;
        while self.pos < self.source.len() && self.source[self.pos] == b' ' {
            self.pos += 1;
        }
        self.pos - start
    }

    /// Count consecutive occurrences of `byte` starting at current pos, advancing pos.
    fn count_consecutive(&mut self, byte: u8) -> usize {
        let start = self.pos;
        while self.pos < self.source.len() && self.source[self.pos] == byte {
            self.pos += 1;
        }
        self.pos - start
    }

    /// Get the byte length of the UTF-8 character at `pos`.
    fn char_byte_len(&self, pos: usize) -> usize {
        if pos >= self.source.len() {
            return 0;
        }
        let b = self.source[pos];
        let len = if b < 0x80 {
            1
        } else if b < 0xE0 {
            2
        } else if b < 0xF0 {
            3
        } else {
            4
        };
        // Cap to remaining bytes to prevent overrun on truncated UTF-8
        len.min(self.source.len() - pos)
    }

    /// Find the end of non-whitespace content (right-trim position).
    fn rtrim_pos(&self, start: usize, end: usize) -> usize {
        let mut p = end;
        while p > start && self.source[p - 1] == b' ' {
            p -= 1;
        }
        p
    }

    /// Get a slice of the source as a string.
    pub fn slice_str(&self, start: u32, len: u32) -> &'a str {
        let s = start as usize;
        let e = (s + len as usize).min(self.source.len());
        if s >= self.source.len() {
            return "";
        }
        std::str::from_utf8(&self.source[s..e]).unwrap_or("")
    }

    /// Get the source bytes.
    pub fn source(&self) -> &'a [u8] {
        self.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokenize(input: &str) -> Vec<Token> {
        let mut lexer = Lexer::new(input.as_bytes());
        lexer.tokenize();
        lexer.tokens
    }

    #[allow(dead_code)]
    fn kinds(tokens: &[Token]) -> Vec<&TokenKind> {
        tokens.iter().map(|t| &t.kind).collect()
    }

    #[test]
    fn heading_h1() {
        let tokens = tokenize("# Hello\n");
        assert!(matches!(tokens[0].kind, TokenKind::HeadingMarker(1)));
        assert!(matches!(tokens[1].kind, TokenKind::Text));
        assert!(matches!(tokens[2].kind, TokenKind::Newline));
    }

    #[test]
    fn heading_h3() {
        let tokens = tokenize("### Title\n");
        assert!(matches!(tokens[0].kind, TokenKind::HeadingMarker(3)));
    }

    #[test]
    fn heading_no_space() {
        let tokens = tokenize("#NoSpace\n");
        // Should NOT produce a heading marker
        assert!(!matches!(tokens[0].kind, TokenKind::HeadingMarker(_)));
    }

    #[test]
    fn code_fence() {
        let tokens = tokenize("```rust\ncode\n```\n");
        assert!(matches!(tokens[0].kind, TokenKind::CodeFence(3)));
        assert!(matches!(tokens[1].kind, TokenKind::CodeFenceLanguage));
    }

    #[test]
    fn checkbox_unchecked() {
        let tokens = tokenize("- [ ] task\n");
        assert!(matches!(tokens[0].kind, TokenKind::CheckboxMarker(false)));
    }

    #[test]
    fn checkbox_checked() {
        let tokens = tokenize("- [x] done\n");
        assert!(matches!(tokens[0].kind, TokenKind::CheckboxMarker(true)));
    }

    #[test]
    fn bullet_list() {
        let tokens = tokenize("- item\n");
        assert!(matches!(tokens[0].kind, TokenKind::BulletMarker));
    }

    #[test]
    fn ordered_list() {
        let tokens = tokenize("1. first\n");
        assert!(matches!(tokens[0].kind, TokenKind::OrderedMarker(1)));
    }

    #[test]
    fn ordered_list_42() {
        let tokens = tokenize("42. forty-two\n");
        assert!(matches!(tokens[0].kind, TokenKind::OrderedMarker(42)));
    }

    #[test]
    fn horizontal_rule() {
        let tokens = tokenize("---\n");
        assert!(matches!(tokens[0].kind, TokenKind::HorizontalRule));
    }

    #[test]
    fn horizontal_rule_stars() {
        let tokens = tokenize("***\n");
        assert!(matches!(tokens[0].kind, TokenKind::HorizontalRule));
    }

    #[test]
    fn blockquote() {
        let tokens = tokenize("> text\n");
        assert!(matches!(tokens[0].kind, TokenKind::BlockquoteMarker));
    }

    #[test]
    fn inline_bold() {
        let tokens = tokenize("hello **world**\n");
        // Text, Asterisks(2), Text, Asterisks(2), Newline
        assert!(matches!(tokens[1].kind, TokenKind::Asterisks(2)));
        assert!(matches!(tokens[3].kind, TokenKind::Asterisks(2)));
    }

    #[test]
    fn wiki_link() {
        let tokens = tokenize("see [[Note Title]] here\n");
        let has_double_open = tokens
            .iter()
            .any(|t| matches!(t.kind, TokenKind::DoubleOpenBracket));
        let has_double_close = tokens
            .iter()
            .any(|t| matches!(t.kind, TokenKind::DoubleCloseBracket));
        assert!(has_double_open);
        assert!(has_double_close);
    }

    #[test]
    fn table_pipes() {
        let tokens = tokenize("| a | b |\n");
        let pipe_count = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::TablePipe))
            .count();
        assert!(pipe_count >= 3); // at least 3 pipes in "| a | b |"
    }

    #[test]
    fn footnote_def() {
        let tokens = tokenize("[^1]: Some text\n");
        assert!(matches!(tokens[0].kind, TokenKind::FootnoteDefMarker));
    }

    fn tokenize_with_scheme(input: &str, scheme: &str) -> Vec<Token> {
        let mut lexer = Lexer::with_image_marker_scheme(input.as_bytes(), Some(scheme));
        lexer.tokenize();
        lexer.tokens
    }

    #[test]
    fn image_marker() {
        let tokens = tokenize_with_scheme("![](ember:abc-123)\n", "ember:");
        assert!(matches!(tokens[0].kind, TokenKind::ImageMarker));
    }

    #[test]
    fn image_marker_custom_scheme() {
        let tokens = tokenize_with_scheme("![](cinder:abc-123)\n", "cinder:");
        assert!(matches!(tokens[0].kind, TokenKind::ImageMarker));
    }

    #[test]
    fn image_marker_disabled_by_default() {
        let tokens = tokenize("![](ember:abc-123)\n");
        assert!(!tokens
            .iter()
            .any(|t| matches!(t.kind, TokenKind::ImageMarker)));
    }

    #[test]
    fn html_underline() {
        let tokens = tokenize("<u>text</u>\n");
        assert!(matches!(tokens[0].kind, TokenKind::HtmlUnderlineOpen));
        let has_close = tokens
            .iter()
            .any(|t| matches!(t.kind, TokenKind::HtmlUnderlineClose));
        assert!(has_close);
    }

    #[test]
    fn color_emoji_highlight() {
        let tokens = tokenize("==🟢green==\n");
        assert!(matches!(tokens[0].kind, TokenKind::DoubleEquals));
        assert!(matches!(tokens[1].kind, TokenKind::ColorEmojiPrefix(3))); // green = 3
    }
}
