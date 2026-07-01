//! UTF-8 to UTF-16 offset mapping.
//!
//! NSTextStorage uses UTF-16 internally. This module provides efficient
//! conversion between byte offsets (used during parsing) and UTF-16 offsets
//! (used by Swift for NSAttributedString attribute ranges).
//!
//! Strategy: Build a per-line cumulative offset table during lexing.
//! ASCII-only documents (the common case) get an O(1) fast path where
//! byte offset == UTF-16 offset.

/// Maps between UTF-8 byte offsets and UTF-16 code unit offsets.
#[derive(Debug, Clone)]
pub struct Utf16Map {
    /// Cumulative UTF-16 offset at the start of each line.
    /// `line_starts[i]` = UTF-16 offset of the first character on line `i`.
    line_utf16_starts: Vec<u32>,
    /// Cumulative byte offset at the start of each line.
    line_byte_starts: Vec<u32>,
    /// True if the entire document is ASCII (byte offset == UTF-16 offset).
    is_ascii: bool,
    /// Total UTF-16 length of the document.
    pub total_utf16_len: u32,
}

impl Utf16Map {
    /// Build the offset map from source bytes.
    pub fn build(source: &[u8]) -> Self {
        let mut line_utf16_starts = Vec::with_capacity(source.len() / 40 + 1);
        let mut line_byte_starts = Vec::with_capacity(source.len() / 40 + 1);
        let mut is_ascii = true;

        line_utf16_starts.push(0);
        line_byte_starts.push(0);

        let mut utf16_pos: u32 = 0;
        let mut i = 0;

        while i < source.len() {
            let b = source[i];
            if b == b'\n' {
                utf16_pos += 1; // newline is 1 UTF-16 code unit
                i += 1;
                line_utf16_starts.push(utf16_pos);
                line_byte_starts.push(i as u32);
            } else if b < 0x80 {
                // ASCII: 1 byte, 1 UTF-16 code unit
                utf16_pos += 1;
                i += 1;
            } else if b < 0xC0 {
                // Continuation byte (shouldn't appear as lead) — treat as 1
                utf16_pos += 1;
                i += 1;
                is_ascii = false;
            } else if b < 0xE0 {
                // 2-byte sequence: 1 UTF-16 code unit
                utf16_pos += 1;
                i += 2.min(source.len());
                is_ascii = false;
            } else if b < 0xF0 {
                // 3-byte sequence: 1 UTF-16 code unit
                utf16_pos += 1;
                i += 3.min(source.len());
                is_ascii = false;
            } else {
                // 4-byte sequence: 2 UTF-16 code units (surrogate pair)
                utf16_pos += 2;
                i += 4.min(source.len());
                is_ascii = false;
            }
        }

        // If the source doesn't end with a newline, the last line's data
        // is still valid — the line_starts arrays already contain its start.
        // However, ensure line_count reflects the actual number of lines.
        // A source like "abc" (no trailing newline) has 1 line: line_starts = [0].
        // A source like "abc\n" has 1 line + empty trailing: line_starts = [0, 3].
        // Both cases are already handled correctly by the loop above.

        Utf16Map {
            line_utf16_starts,
            line_byte_starts,
            is_ascii,
            total_utf16_len: utf16_pos,
        }
    }

    /// Convert a byte offset to a UTF-16 offset. O(1) for ASCII, O(log n) + O(line_len) otherwise.
    /// Byte offsets past the end of the source are clamped to `total_utf16_len`.
    #[allow(clippy::if_same_then_else)]
    pub fn byte_to_utf16(&self, byte_offset: u32, source: &[u8]) -> u32 {
        // Clamp to document bounds
        if byte_offset >= source.len() as u32 {
            return self.total_utf16_len;
        }

        if self.is_ascii {
            return byte_offset;
        }

        // Binary search for the line containing this byte offset
        let line = match self.line_byte_starts.binary_search(&byte_offset) {
            Ok(exact) => exact,
            Err(insert) => insert.saturating_sub(1),
        };

        let line_byte_start = self.line_byte_starts[line] as usize;
        let line_utf16_start = self.line_utf16_starts[line];
        let target = byte_offset as usize;

        // Walk from line start to target byte, counting UTF-16 units
        let mut utf16_offset = line_utf16_start;
        let mut pos = line_byte_start;

        while pos < target && pos < source.len() {
            let b = source[pos];
            if b < 0x80 {
                utf16_offset += 1;
                pos += 1;
            } else if b < 0xC0 {
                utf16_offset += 1;
                pos += 1;
            } else if b < 0xE0 {
                utf16_offset += 1;
                pos += 2;
            } else if b < 0xF0 {
                utf16_offset += 1;
                pos += 3;
            } else {
                utf16_offset += 2;
                pos += 4;
            }
        }

        utf16_offset
    }

    /// Convert a UTF-16 offset to a byte offset. O(1) for ASCII, O(log n) + O(line_len) otherwise.
    #[allow(clippy::if_same_then_else)]
    pub fn utf16_to_byte(&self, utf16_offset: u32, source: &[u8]) -> u32 {
        if self.is_ascii {
            return utf16_offset;
        }

        // Binary search for the line containing this UTF-16 offset
        let line = match self.line_utf16_starts.binary_search(&utf16_offset) {
            Ok(exact) => exact,
            Err(insert) => insert.saturating_sub(1),
        };

        let line_byte_start = self.line_byte_starts[line] as usize;
        let line_utf16_start = self.line_utf16_starts[line];
        let target = utf16_offset;

        let mut current_utf16 = line_utf16_start;
        let mut pos = line_byte_start;

        while current_utf16 < target && pos < source.len() {
            let b = source[pos];
            if b < 0x80 {
                current_utf16 += 1;
                pos += 1;
            } else if b < 0xC0 {
                current_utf16 += 1;
                pos += 1;
            } else if b < 0xE0 {
                current_utf16 += 1;
                pos += 2;
            } else if b < 0xF0 {
                current_utf16 += 1;
                pos += 3;
            } else {
                current_utf16 += 2;
                pos += 4;
            }
        }

        pos as u32
    }

    /// Get the line index for a byte offset.
    pub fn line_at_byte(&self, byte_offset: u32) -> u32 {
        match self.line_byte_starts.binary_search(&byte_offset) {
            Ok(exact) => exact as u32,
            Err(insert) => insert.saturating_sub(1) as u32,
        }
    }

    /// Get the byte offset of a line start.
    pub fn line_byte_start(&self, line: u32) -> u32 {
        self.line_byte_starts
            .get(line as usize)
            .copied()
            .unwrap_or(0)
    }

    /// Get the UTF-16 offset of a line start.
    pub fn line_utf16_start(&self, line: u32) -> u32 {
        self.line_utf16_starts
            .get(line as usize)
            .copied()
            .unwrap_or(0)
    }

    /// Total number of lines.
    pub fn line_count(&self) -> u32 {
        self.line_byte_starts.len() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_identity() {
        let src = b"hello\nworld\n";
        let map = Utf16Map::build(src);
        assert!(map.is_ascii);
        assert_eq!(map.byte_to_utf16(0, src), 0);
        assert_eq!(map.byte_to_utf16(5, src), 5);
        assert_eq!(map.byte_to_utf16(6, src), 6);
        assert_eq!(map.total_utf16_len, 12);
    }

    #[test]
    fn two_byte_utf8() {
        // "ñ" = 0xC3 0xB1 (2 bytes UTF-8, 1 UTF-16 code unit)
        let src = "señor\n".as_bytes();
        let map = Utf16Map::build(src);
        assert!(!map.is_ascii);
        // 's' at byte 0 -> utf16 0
        assert_eq!(map.byte_to_utf16(0, src), 0);
        // 'e' at byte 1 -> utf16 1
        assert_eq!(map.byte_to_utf16(1, src), 1);
        // 'ñ' starts at byte 2 -> utf16 2
        assert_eq!(map.byte_to_utf16(2, src), 2);
        // 'o' at byte 4 -> utf16 3
        assert_eq!(map.byte_to_utf16(4, src), 3);
        // Total: "señor\n" = 6 chars in UTF-16
        assert_eq!(map.total_utf16_len, 6);
    }

    #[test]
    fn four_byte_emoji() {
        // "🟢" = U+1F7E2 = 4 bytes UTF-8, 2 UTF-16 code units (surrogate pair)
        let src = "a🟢b\n".as_bytes();
        let map = Utf16Map::build(src);
        assert!(!map.is_ascii);
        assert_eq!(map.byte_to_utf16(0, src), 0); // 'a'
        assert_eq!(map.byte_to_utf16(1, src), 1); // start of 🟢
        assert_eq!(map.byte_to_utf16(5, src), 3); // 'b' (after 4-byte emoji = +2 utf16)
        assert_eq!(map.total_utf16_len, 5); // a(1) + 🟢(2) + b(1) + \n(1) = 5
    }

    #[test]
    fn multiline_offsets() {
        let src = "line1\nline2\nline3\n".as_bytes();
        let map = Utf16Map::build(src);
        assert_eq!(map.line_count(), 4); // 3 lines + empty after trailing \n
        assert_eq!(map.line_byte_start(0), 0);
        assert_eq!(map.line_byte_start(1), 6);
        assert_eq!(map.line_byte_start(2), 12);
        assert_eq!(map.line_at_byte(7), 1);
    }

    #[test]
    fn roundtrip() {
        let src = "hello 🌍 world\nañ\n".as_bytes();
        let map = Utf16Map::build(src);
        for byte_off in 0..src.len() as u32 {
            // Only test at character boundaries
            if byte_off == 0
                || (byte_off > 0 && src[byte_off as usize] < 0x80)
                || (byte_off > 0 && src[byte_off as usize] >= 0xC0)
            {
                let utf16 = map.byte_to_utf16(byte_off, src);
                let back = map.utf16_to_byte(utf16, src);
                assert_eq!(back, byte_off, "roundtrip failed for byte_off={byte_off}");
            }
        }
    }
}
