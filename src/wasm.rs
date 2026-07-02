//! Browser bindings (`--features wasm`, target `wasm32-unknown-unknown`).
//!
//! A thin `wasm-bindgen` surface over [`CindermarkParser`] for interactive
//! demos — notably the live playground at <https://embernotes.app/cindermark>.
//! Results cross the JS boundary as compact JSON strings: the payloads are
//! small (block/span metadata, never the document text), so hand-rolled JSON
//! keeps the `.wasm` binary free of a serde dependency.
//!
//! Timing note: measure parse cost in JS around these calls
//! (`performance.now()`); `std::time::Instant` is unavailable on
//! `wasm32-unknown-unknown`.

use wasm_bindgen::prelude::*;

use crate::{CindermarkParser, FfiBlock, FfiBlockType, FfiInlineSpan, FfiInlineType};

#[wasm_bindgen]
pub struct WasmParser {
    inner: CindermarkParser,
}

#[wasm_bindgen]
impl WasmParser {
    /// A parser with no image-marker scheme — the demo has no attachments.
    #[wasm_bindgen(constructor)]
    pub fn new() -> WasmParser {
        WasmParser {
            inner: CindermarkParser::new(None),
        }
    }

    /// Full editable parse. JSON: `{"blocks":[…],"lineCount":n,"stats":{…}}`.
    /// Also primes the incremental snapshot, so `keystroke` calls that follow
    /// re-parse only the dirty block window.
    #[wasm_bindgen(js_name = parseJson)]
    pub fn parse_json(&self, text: String) -> String {
        let result = self.inner.parse_editable(text);
        let mut out = String::with_capacity(result.blocks.len() * 96 + 256);
        out.push_str("{\"blocks\":[");
        push_blocks(&mut out, &result.blocks);
        out.push_str("],\"lineCount\":");
        push_u32(&mut out, result.line_count);
        out.push_str(",\"stats\":{\"words\":");
        push_u32(&mut out, result.stats.word_count);
        out.push_str(",\"readingSeconds\":");
        push_u32(&mut out, result.stats.reading_time_seconds);
        out.push_str(",\"checkboxTotal\":");
        push_u32(&mut out, result.stats.checkbox_total);
        out.push_str(",\"checkboxDone\":");
        push_u32(&mut out, result.stats.checkbox_completed);
        out.push_str(",\"headings\":");
        push_u32(&mut out, result.stats.heading_count);
        out.push_str(",\"links\":");
        push_u32(&mut out, result.stats.link_count);
        out.push_str("}}");
        out
    }

    /// Incremental re-parse after one edit. Returns only the summary —
    /// `{"dirtyStart":n,"dirtyEnd":n,"blockCount":n,"lineCount":n}` — so the
    /// measured round-trip is parse cost, not JSON transport.
    #[wasm_bindgen(js_name = keystroke)]
    pub fn keystroke(
        &self,
        text: String,
        edit_utf16_start: u32,
        edit_old_utf16_len: u32,
        edit_new_utf16_len: u32,
    ) -> String {
        let update = self.inner.parse_editable_incremental_style_only(
            text,
            edit_utf16_start,
            edit_old_utf16_len,
            edit_new_utf16_len,
        );
        let mut out = String::with_capacity(96);
        out.push_str("{\"dirtyStart\":");
        push_u32(&mut out, update.dirty_start);
        out.push_str(",\"dirtyEnd\":");
        push_u32(&mut out, update.dirty_end);
        out.push_str(",\"blockCount\":");
        push_u32(&mut out, update.blocks.len() as u32);
        out.push_str(",\"lineCount\":");
        push_u32(&mut out, update.line_count);
        out.push('}');
        out
    }

    /// Drop the incremental snapshot (e.g. when the demo swaps documents).
    #[wasm_bindgen(js_name = resetState)]
    pub fn reset_state(&self) {
        self.inner.reset_state();
    }

    /// Toggle the checkbox on `line_index`, returning the updated document.
    /// Same behavior the native editors use for tap-to-toggle.
    #[wasm_bindgen(js_name = toggleCheckbox)]
    pub fn toggle_checkbox(&self, text: String, line_index: u32) -> String {
        self.inner.toggle_checkbox(text, line_index)
    }
}

impl Default for WasmParser {
    fn default() -> Self {
        Self::new()
    }
}

fn push_blocks(out: &mut String, blocks: &[FfiBlock]) {
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"type\":\"");
        out.push_str(block_type_name(&block.block_type));
        out.push('"');
        match &block.block_type {
            FfiBlockType::Heading => {
                out.push_str(",\"level\":");
                push_u32(out, u32::from(block.heading_level));
            }
            FfiBlockType::Checkbox => {
                out.push_str(",\"checked\":");
                out.push_str(if block.is_checked { "true" } else { "false" });
            }
            FfiBlockType::Callout { kind } => {
                out.push_str(",\"kind\":");
                push_u32(out, u32::from(*kind));
            }
            FfiBlockType::MermaidDiagram { diagram_type } => {
                out.push_str(",\"diagram\":");
                push_u32(out, u32::from(*diagram_type));
            }
            FfiBlockType::NumberedItem => {
                out.push_str(",\"number\":");
                push_u32(out, block.number);
            }
            _ => {}
        }
        if let Some(language) = &block.language {
            out.push_str(",\"language\":");
            push_json_string(out, language);
        }
        out.push_str(",\"lineStart\":");
        push_u32(out, block.line_start);
        out.push_str(",\"lineEnd\":");
        push_u32(out, block.line_end);
        out.push_str(",\"start\":");
        push_u32(out, block.utf16_start);
        out.push_str(",\"end\":");
        push_u32(out, block.utf16_end);
        out.push_str(",\"markerEnd\":");
        push_u32(out, block.marker_utf16_end);
        out.push_str(",\"indent\":");
        push_u32(out, block.list_indent);
        out.push_str(",\"spans\":[");
        push_spans(out, &block.inline_spans);
        out.push_str("]}");
    }
}

fn push_spans(out: &mut String, spans: &[FfiInlineSpan]) {
    for (i, span) in spans.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"type\":\"");
        out.push_str(inline_type_name(&span.inline_type));
        out.push('"');
        match &span.inline_type {
            FfiInlineType::Link { url } | FfiInlineType::AutoLink { url } => {
                out.push_str(",\"url\":");
                push_json_string(out, url);
            }
            FfiInlineType::HighlightHex { hex } | FfiInlineType::HexColor { hex } => {
                out.push_str(",\"hex\":");
                push_json_string(out, hex);
            }
            FfiInlineType::HighlightColor { color_index } => {
                out.push_str(",\"colorIndex\":");
                push_u32(out, u32::from(*color_index));
            }
            _ => {}
        }
        out.push_str(",\"start\":");
        push_u32(out, span.utf16_start);
        out.push_str(",\"end\":");
        push_u32(out, span.utf16_end);
        out.push_str(",\"contentStart\":");
        push_u32(out, span.content_utf16_start);
        out.push_str(",\"contentEnd\":");
        push_u32(out, span.content_utf16_end);
        out.push('}');
    }
}

fn block_type_name(block_type: &FfiBlockType) -> &'static str {
    match block_type {
        FfiBlockType::Heading => "heading",
        FfiBlockType::Paragraph => "paragraph",
        FfiBlockType::CodeBlock => "code",
        FfiBlockType::Blockquote => "blockquote",
        FfiBlockType::BulletList => "bulletList",
        FfiBlockType::OrderedList => "orderedList",
        FfiBlockType::Checkbox => "checkbox",
        FfiBlockType::Table => "table",
        FfiBlockType::HorizontalRule => "hr",
        FfiBlockType::Empty => "empty",
        FfiBlockType::FootnoteDefinition => "footnote",
        FfiBlockType::ImageMarker => "imageMarker",
        FfiBlockType::BulletItem => "bulletItem",
        FfiBlockType::NumberedItem => "numberedItem",
        FfiBlockType::Callout { .. } => "callout",
        FfiBlockType::MermaidDiagram { .. } => "mermaid",
    }
}

fn inline_type_name(inline_type: &FfiInlineType) -> &'static str {
    match inline_type {
        FfiInlineType::Bold => "bold",
        FfiInlineType::Italic => "italic",
        FfiInlineType::BoldItalic => "boldItalic",
        FfiInlineType::Strikethrough => "strike",
        FfiInlineType::UnderlineTilde | FfiInlineType::UnderlineHtml => "underline",
        FfiInlineType::InlineCode => "code",
        FfiInlineType::Highlight
        | FfiInlineType::HighlightColor { .. }
        | FfiInlineType::HighlightHex { .. } => "highlight",
        FfiInlineType::Link { .. } => "link",
        FfiInlineType::AutoLink { .. } => "autolink",
        FfiInlineType::WikiLink => "wikiLink",
        FfiInlineType::FootnoteRef => "footnoteRef",
        FfiInlineType::Comment => "comment",
        FfiInlineType::HexColor { .. } => "hexColor",
    }
}

fn push_u32(out: &mut String, value: u32) {
    let mut buffer = itoa_buffer();
    let mut i = buffer.len();
    let mut v = value;
    loop {
        i -= 1;
        buffer[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    // Digits are ASCII by construction.
    out.push_str(core::str::from_utf8(&buffer[i..]).unwrap());
}

fn itoa_buffer() -> [u8; 10] {
    [0; 10]
}

fn push_json_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_shape_is_valid_and_typed() {
        let parser = WasmParser::new();
        let json = parser.parse_json("# Hi\n\nSome **bold** text.\n".to_string());
        assert!(json.starts_with("{\"blocks\":[{\"type\":\"heading\",\"level\":1"));
        assert!(json.contains("\"type\":\"bold\""));
        assert!(json.contains("\"stats\":{\"words\":"));
    }

    #[test]
    fn keystroke_reports_dirty_window() {
        let parser = WasmParser::new();
        let text = "# Title\n\nline one\n\nline two\n";
        parser.parse_json(text.to_string());
        let edited = "# Title\n\nline one!\n\nline two\n";
        let json = parser.keystroke(edited.to_string(), 17, 0, 1);
        assert!(json.contains("\"dirtyStart\":"));
        assert!(json.contains("\"blockCount\":5"));
    }

    #[test]
    fn json_string_escaping() {
        let mut out = String::new();
        push_json_string(&mut out, "a\"b\\c\nd");
        assert_eq!(out, "\"a\\\"b\\\\c\\nd\"");
    }
}
