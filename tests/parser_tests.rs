//! Comprehensive test suite ported from MarkdownParserTests.swift (433 lines).
//!
//! Every test from the Swift suite has a corresponding Rust test here,
//! ensuring the Rust parser produces identical block structure.

use cindermark::CindermarkParser;

// Helper: parse in grouped mode via the FFI layer
fn parse_ffi(input: &str) -> cindermark::FfiParseResult {
    let parser = CindermarkParser::new(None);
    parser.parse(input.to_string())
}

fn parse_ffi_editable(input: &str) -> cindermark::FfiParseResult {
    let parser = CindermarkParser::new(None);
    parser.parse_editable(input.to_string())
}

// MARK: - Headings

#[test]
fn h1_heading() {
    let result = parse_ffi("# Hello World");
    assert_eq!(result.blocks.len(), 1);
    assert_eq!(
        result.blocks[0].block_type,
        cindermark::FfiBlockType::Heading
    );
    assert_eq!(result.blocks[0].heading_level, 1);
    assert_eq!(result.blocks[0].text, "Hello World");
}

#[test]
fn h2_heading() {
    let result = parse_ffi("## Sub Heading");
    assert_eq!(result.blocks.len(), 1);
    assert_eq!(result.blocks[0].heading_level, 2);
    assert_eq!(result.blocks[0].text, "Sub Heading");
}

#[test]
fn h3_heading() {
    let result = parse_ffi("### Small Heading");
    assert_eq!(result.blocks.len(), 1);
    assert_eq!(result.blocks[0].heading_level, 3);
    assert_eq!(result.blocks[0].text, "Small Heading");
}

#[test]
fn heading_requires_space() {
    let result = parse_ffi("#NoSpace");
    assert_eq!(result.blocks.len(), 1);
    assert_eq!(
        result.blocks[0].block_type,
        cindermark::FfiBlockType::Paragraph
    );
}

#[test]
fn all_heading_levels() {
    for level in 1..=6u8 {
        let input = format!("{} Heading {}", "#".repeat(level as usize), level);
        let result = parse_ffi(&input);
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].heading_level, level);
    }
}

// MARK: - Paragraphs

#[test]
fn simple_paragraph() {
    let result = parse_ffi("This is a paragraph.");
    assert_eq!(result.blocks.len(), 1);
    assert_eq!(result.blocks[0].text, "This is a paragraph.");
}

#[test]
fn multiline_paragraph() {
    let result = parse_ffi("Line one\nLine two");
    assert_eq!(result.blocks.len(), 1);
    assert_eq!(result.blocks[0].text, "Line one\nLine two");
}

// MARK: - Code Blocks

#[test]
fn code_block_with_language() {
    let result = parse_ffi("```swift\nlet x = 1\n```");
    let code_blocks: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::CodeBlock)
        .collect();
    assert_eq!(code_blocks.len(), 1);
    assert_eq!(code_blocks[0].language.as_deref(), Some("swift"));
    assert_eq!(code_blocks[0].text, "let x = 1");
}

#[test]
fn code_block_no_language() {
    let result = parse_ffi("```\nhello\n```");
    let code_blocks: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::CodeBlock)
        .collect();
    assert_eq!(code_blocks.len(), 1);
    assert!(code_blocks[0].language.is_none());
    assert_eq!(code_blocks[0].text, "hello");
}

#[test]
fn unclosed_code_block() {
    let result = parse_ffi("```python\nprint(\"hello\")\nno closing fence");
    let code_blocks: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::CodeBlock)
        .collect();
    assert_eq!(code_blocks.len(), 1);
    assert_eq!(code_blocks[0].language.as_deref(), Some("python"));
    assert!(code_blocks[0].text.contains("print"));
}

// MARK: - Mermaid Diagrams

/// Helper: find the single mermaid block in a parse result, with its
/// u8 diagram type discriminant for terse assertions.
fn mermaid_blocks(result: &cindermark::FfiParseResult) -> Vec<(u8, String)> {
    result
        .blocks
        .iter()
        .filter_map(|b| match b.block_type {
            cindermark::FfiBlockType::MermaidDiagram { diagram_type } => {
                Some((diagram_type, b.text.clone()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn mermaid_flowchart_top_down() {
    let result = parse_ffi("```mermaid\nflowchart TD\n  A --> B\n```");
    let blocks = mermaid_blocks(&result);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].0, 1); // Flowchart
    assert_eq!(blocks[0].1, "flowchart TD\n  A --> B");
}

#[test]
fn mermaid_graph_legacy_alias() {
    let result = parse_ffi("```mermaid\ngraph LR\n  A --> B\n```");
    let blocks = mermaid_blocks(&result);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].0, 2); // Graph
}

#[test]
fn mermaid_sequence_diagram() {
    let result =
        parse_ffi("```mermaid\nsequenceDiagram\n  Alice->>Bob: hi\n  Bob-->>Alice: hello\n```");
    let blocks = mermaid_blocks(&result);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].0, 3); // SequenceDiagram
}

#[test]
fn mermaid_class_diagram_v2() {
    let result = parse_ffi("```mermaid\nclassDiagram-v2\n  class Animal\n```");
    let blocks = mermaid_blocks(&result);
    assert_eq!(blocks[0].0, 4); // ClassDiagram
}

#[test]
fn mermaid_state_diagram() {
    let result = parse_ffi("```mermaid\nstateDiagram-v2\n  [*] --> Idle\n```");
    let blocks = mermaid_blocks(&result);
    assert_eq!(blocks[0].0, 5); // StateDiagram
}

#[test]
fn mermaid_er_diagram() {
    let result = parse_ffi("```mermaid\nerDiagram\n  USER ||--o{ ORDER : places\n```");
    assert_eq!(mermaid_blocks(&result)[0].0, 6);
}

#[test]
fn mermaid_gantt() {
    let result = parse_ffi("```mermaid\ngantt\n  title A Gantt\n```");
    assert_eq!(mermaid_blocks(&result)[0].0, 7);
}

#[test]
fn mermaid_mindmap() {
    let result = parse_ffi("```mermaid\nmindmap\n  root((origin))\n```");
    assert_eq!(mermaid_blocks(&result)[0].0, 8);
}

#[test]
fn mermaid_pie_with_title() {
    let result = parse_ffi("```mermaid\npie title Pets\n  \"Dogs\" : 60\n  \"Cats\" : 40\n```");
    assert_eq!(mermaid_blocks(&result)[0].0, 9);
}

#[test]
fn mermaid_journey() {
    let result = parse_ffi("```mermaid\njourney\n  title User day\n```");
    assert_eq!(mermaid_blocks(&result)[0].0, 10);
}

#[test]
fn mermaid_timeline() {
    let result = parse_ffi("```mermaid\ntimeline\n  title History\n```");
    assert_eq!(mermaid_blocks(&result)[0].0, 11);
}

#[test]
fn mermaid_gitgraph() {
    let result = parse_ffi("```mermaid\ngitGraph\n  commit\n  commit\n```");
    assert_eq!(mermaid_blocks(&result)[0].0, 12);
}

#[test]
fn mermaid_c4_context() {
    let result = parse_ffi("```mermaid\nC4Context\n  Person(u, \"User\")\n```");
    assert_eq!(mermaid_blocks(&result)[0].0, 13);
}

#[test]
fn mermaid_c4_component_maps_to_same_variant() {
    // All C4 subtypes classify under the single `C4` umbrella.
    let result = parse_ffi("```mermaid\nC4Component\n  Component(c, \"X\")\n```");
    assert_eq!(mermaid_blocks(&result)[0].0, 13);
}

#[test]
fn mermaid_quadrant_chart() {
    let result = parse_ffi("```mermaid\nquadrantChart\n  title Reach\n```");
    assert_eq!(mermaid_blocks(&result)[0].0, 15);
}

#[test]
fn mermaid_sankey_beta() {
    let result = parse_ffi("```mermaid\nsankey-beta\n  A,B,10\n```");
    assert_eq!(mermaid_blocks(&result)[0].0, 16);
}

#[test]
fn mermaid_unknown_diagram_type() {
    // Classifier doesn't know `bogustype` — still parses as mermaid block,
    // just with Unknown discriminant so the renderer can show a generic label.
    let result = parse_ffi("```mermaid\nbogustype\n  whatever\n```");
    let blocks = mermaid_blocks(&result);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].0, 0); // Unknown
}

#[test]
fn mermaid_case_insensitive_info_string() {
    // `MERMAID` and `Mermaid` both route to mermaid diagram.
    let upper = parse_ffi("```MERMAID\nflowchart TD\n  A --> B\n```");
    let mixed = parse_ffi("```Mermaid\nflowchart TD\n  A --> B\n```");
    assert_eq!(mermaid_blocks(&upper).len(), 1);
    assert_eq!(mermaid_blocks(&mixed).len(), 1);
}

#[test]
fn mermaid_case_insensitive_diagram_keyword() {
    // Diagram keyword classifier is case-insensitive.
    let result = parse_ffi("```mermaid\nSEQUENCEDIAGRAM\n  Alice->>Bob: hi\n```");
    assert_eq!(mermaid_blocks(&result)[0].0, 3); // SequenceDiagram
}

#[test]
fn mermaid_frontmatter_is_skipped_for_classification() {
    // YAML frontmatter before the diagram keyword shouldn't confuse the classifier.
    let source = "```mermaid\n---\ntitle: My Flow\nconfig:\n  theme: dark\n---\nflowchart TD\n  A --> B\n```";
    let result = parse_ffi(source);
    let blocks = mermaid_blocks(&result);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].0, 1); // Flowchart — frontmatter skipped
                                // Source round-trips verbatim including the frontmatter
    assert!(blocks[0].1.contains("title: My Flow"));
    assert!(blocks[0].1.contains("flowchart TD"));
}

#[test]
fn mermaid_roundtrips_source_verbatim() {
    let source = "```mermaid\nflowchart TD\n  A --> B\n  B --> C\n```";
    let result = parse_ffi(source);
    let blocks = mermaid_blocks(&result);
    assert_eq!(blocks[0].1, "flowchart TD\n  A --> B\n  B --> C");
}

#[test]
fn mermaid_not_counted_as_code_block() {
    // stats.code_block_count counts true code blocks only.
    // stats.mermaid_diagram_count counts mermaid diagrams separately.
    let result = parse_ffi("```swift\nlet x = 1\n```\n\n```mermaid\nflowchart TD\n  A --> B\n```");
    assert_eq!(result.stats.code_block_count, 1);
    assert_eq!(result.stats.mermaid_diagram_count, 1);
}

#[test]
fn mermaid_adjacent_to_code_block() {
    // Two fenced blocks back-to-back, one mermaid one code — neither
    // should swallow the other.
    let source = "```mermaid\nflowchart TD\n  A --> B\n```\n\n```python\nprint(\"hi\")\n```";
    let result = parse_ffi(source);
    let mermaid = mermaid_blocks(&result);
    let code: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::CodeBlock)
        .collect();
    assert_eq!(mermaid.len(), 1);
    assert_eq!(code.len(), 1);
    assert_eq!(code[0].language.as_deref(), Some("python"));
}

#[test]
fn mermaid_wiki_links_in_source_are_not_extracted() {
    // Looking-like-wiki-link text inside a mermaid block is diagram content,
    // not a real link.
    let result = parse_ffi("```mermaid\nflowchart TD\n  A[\"[[Not A Link]]\"]\n```");
    assert!(
        result.wiki_links.is_empty(),
        "wiki links leaked from mermaid source: {:?}",
        result.wiki_links
    );
}

#[test]
fn mermaid_editable_mode_parses_identically() {
    // parse_editable should treat mermaid blocks the same as parse() —
    // they're atomic, not split into items.
    let result = parse_ffi_editable("```mermaid\nflowchart TD\n  A --> B\n```");
    let blocks = mermaid_blocks(&result);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].0, 1);
}

#[test]
fn mermaid_after_heading_preserves_both() {
    // Regression guard for the class of block-boundary bugs that trapped
    // code blocks before — ensure a mermaid fence immediately after a
    // heading parses both blocks cleanly.
    let result = parse_ffi("# Title\n\n```mermaid\nflowchart TD\n  A --> B\n```");
    assert!(matches!(
        result.blocks[0].block_type,
        cindermark::FfiBlockType::Heading
    ));
    assert_eq!(mermaid_blocks(&result).len(), 1);
}

// MARK: - Blockquotes

#[test]
fn single_blockquote() {
    let result = parse_ffi("> This is a quote");
    let quotes: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::Blockquote)
        .collect();
    assert_eq!(quotes.len(), 1);
    assert_eq!(quotes[0].text, "This is a quote");
}

#[test]
fn multiline_blockquote() {
    let result = parse_ffi("> Line one\n> Line two");
    let quotes: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::Blockquote)
        .collect();
    assert_eq!(quotes.len(), 1);
    assert_eq!(quotes[0].text, "Line one\nLine two");
}

// MARK: - Unordered Lists

#[test]
fn unordered_list_dash() {
    let result = parse_ffi("- First\n- Second\n- Third");
    let lists: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::BulletList)
        .collect();
    assert_eq!(lists.len(), 1);
    let items: Vec<&str> = lists[0]
        .list_items
        .iter()
        .map(|i| i.text.as_str())
        .collect();
    assert_eq!(items, vec!["First", "Second", "Third"]);
}

#[test]
fn unordered_list_asterisk() {
    let result = parse_ffi("* Alpha\n* Beta");
    let lists: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::BulletList)
        .collect();
    assert_eq!(lists.len(), 1);
    let items: Vec<&str> = lists[0]
        .list_items
        .iter()
        .map(|i| i.text.as_str())
        .collect();
    assert_eq!(items, vec!["Alpha", "Beta"]);
}

#[test]
fn commonmark_bullet_requires_space() {
    let result = parse_ffi("-one");
    assert_eq!(
        result.blocks[0].block_type,
        cindermark::FfiBlockType::Paragraph
    );
}

#[test]
fn unordered_marker_source_splits_lists() {
    let result = parse_ffi("- Dash\n* Star\n+ Plus");
    let lists: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::BulletList)
        .collect();
    assert_eq!(lists.len(), 3);
    assert_eq!(lists[0].unordered_marker, "-");
    assert_eq!(lists[1].unordered_marker, "*");
    assert_eq!(lists[2].unordered_marker, "+");
}

// MARK: - Ordered Lists

#[test]
fn ordered_list() {
    let result = parse_ffi("1. First\n2. Second\n3. Third");
    let lists: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::OrderedList)
        .collect();
    assert_eq!(lists.len(), 1);
    let items: Vec<&str> = lists[0]
        .list_items
        .iter()
        .map(|i| i.text.as_str())
        .collect();
    assert_eq!(items, vec!["First", "Second", "Third"]);
}

#[test]
fn commonmark_ordered_requires_space() {
    let result = parse_ffi("2.two");
    assert_eq!(
        result.blocks[0].block_type,
        cindermark::FfiBlockType::Paragraph
    );
}

#[test]
fn commonmark_ordered_rejects_ten_digits() {
    let result = parse_ffi("1234567890. too much");
    assert_eq!(
        result.blocks[0].block_type,
        cindermark::FfiBlockType::Paragraph
    );
}

#[test]
fn ordered_start_zero_and_leading_zeroes_are_preserved() {
    let result = parse_ffi("0. Zero\n003. Three");
    let lists: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::OrderedList)
        .collect();
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0].number, 0);
    assert_eq!(lists[0].marker_source, "0. ");

    let editable = cindermark::CindermarkParser::new(None).parse_editable("003. Three".to_string());
    assert_eq!(
        editable.blocks[0].block_type,
        cindermark::FfiBlockType::NumberedItem
    );
    assert_eq!(editable.blocks[0].number, 3);
    assert_eq!(editable.blocks[0].ordered_raw_number, "003");
    assert_eq!(editable.blocks[0].marker_source, "003. ");
}

#[test]
fn ordered_parenthesis_delimiter_works_and_splits_from_dot() {
    let result = parse_ffi("1) One\n2) Two\n3. Three");
    let lists: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::OrderedList)
        .collect();
    assert_eq!(lists.len(), 2);
    assert_eq!(lists[0].ordered_delimiter, ")");
    assert_eq!(lists[1].ordered_delimiter, ".");
}

// MARK: - Checkboxes

#[test]
fn unchecked_checkbox() {
    let result = parse_ffi("- [ ] Todo item");
    let cbs: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::Checkbox)
        .collect();
    assert_eq!(cbs.len(), 1);
    assert!(!cbs[0].is_checked);
    assert_eq!(cbs[0].text, "Todo item");
}

#[test]
fn checked_checkbox() {
    let result = parse_ffi("- [x] Done item");
    let cbs: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::Checkbox)
        .collect();
    assert_eq!(cbs.len(), 1);
    assert!(cbs[0].is_checked);
    assert_eq!(cbs[0].text, "Done item");
}

#[test]
fn checkbox_extension_accepts_all_bullet_sources() {
    let result = parse_ffi("* [ ] Star task\n+ [x] Plus task");
    let cbs: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::Checkbox)
        .collect();
    assert_eq!(cbs.len(), 2);
    assert_eq!(cbs[0].unordered_marker, "*");
    assert_eq!(cbs[0].marker_source, "* [ ] ");
    assert_eq!(cbs[1].unordered_marker, "+");
    assert_eq!(cbs[1].marker_source, "+ [x] ");
}

#[test]
fn editable_list_metadata_preserves_indentation_and_marker_range() {
    let parser = cindermark::CindermarkParser::new(None);
    let result = parser.parse_editable("  + item\n003) value".to_string());
    assert_eq!(
        result.blocks[0].block_type,
        cindermark::FfiBlockType::BulletItem
    );
    assert_eq!(result.blocks[0].list_indent, 2);
    assert_eq!(result.blocks[0].marker_utf16_start, 2);
    assert_eq!(result.blocks[0].marker_utf16_end, 4);
    assert_eq!(result.blocks[0].marker_source, "+ ");

    assert_eq!(
        result.blocks[1].block_type,
        cindermark::FfiBlockType::NumberedItem
    );
    assert_eq!(result.blocks[1].marker_source, "003) ");
    assert_eq!(result.blocks[1].ordered_delimiter, ")");
    assert_eq!(result.blocks[1].ordered_raw_number, "003");
}

#[test]
fn uppercase_x_checkbox() {
    let result = parse_ffi("- [X] Also done");
    let cbs: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::Checkbox)
        .collect();
    assert_eq!(cbs.len(), 1);
    assert!(cbs[0].is_checked);
}

#[test]
fn checkboxes_not_swallowed_by_list() {
    let result = parse_ffi("- [ ] First task\n- [ ] Second task\n- [x] Third done");
    let cbs: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::Checkbox)
        .collect();
    let lists: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::BulletList)
        .collect();
    assert_eq!(cbs.len(), 3);
    assert!(
        lists.is_empty(),
        "Checkboxes should NOT be parsed as list items"
    );
}

#[test]
fn mixed_list_and_checkboxes() {
    let result = parse_ffi("- Regular item\n- Another item\n- [ ] Task item");
    let lists: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::BulletList)
        .collect();
    let cbs: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::Checkbox)
        .collect();
    assert_eq!(lists.len(), 1);
    let items: Vec<&str> = lists[0]
        .list_items
        .iter()
        .map(|i| i.text.as_str())
        .collect();
    assert_eq!(items, vec!["Regular item", "Another item"]);
    assert_eq!(cbs.len(), 1);
    assert_eq!(cbs[0].text, "Task item");
}

// MARK: - Nested Lists (deep indentation)

#[test]
fn editable_nested_list_indent_metadata() {
    let result = parse_ffi_editable("- a\n    - b\n        - [ ] c");
    assert_eq!(result.blocks.len(), 3);

    assert_eq!(
        result.blocks[0].block_type,
        cindermark::FfiBlockType::BulletItem
    );
    assert_eq!(result.blocks[0].list_indent, 0);

    assert_eq!(
        result.blocks[1].block_type,
        cindermark::FfiBlockType::BulletItem
    );
    assert_eq!(result.blocks[1].list_indent, 4);
    // Marker range excludes indentation: "    - b" → marker at [8..10)
    // in document coords (line starts at UTF-16 4).
    assert_eq!(result.blocks[1].marker_utf16_start, 8);
    assert_eq!(result.blocks[1].marker_utf16_end, 10);
    assert_eq!(result.blocks[1].marker_source, "- ");
    assert_eq!(result.blocks[1].text, "b");

    assert_eq!(
        result.blocks[2].block_type,
        cindermark::FfiBlockType::Checkbox
    );
    assert_eq!(result.blocks[2].list_indent, 8);
    assert_eq!(result.blocks[2].marker_source, "- [ ] ");
    assert_eq!(result.blocks[2].text, "c");
    assert!(!result.blocks[2].is_checked);
}

#[test]
fn nested_preview_items_on_own_lines() {
    // Grouped mode must not flatten the nested item into the previous
    // item's text — each item gets its own line in previews.
    let parser = CindermarkParser::new(None);
    let result = parser.render_preview("- a\n    - b\n- c".to_string(), 200);
    assert_eq!(result.plain_text, "a\nb\nc");
}

#[test]
fn nested_checkbox_preview_keeps_glyph_line() {
    let parser = CindermarkParser::new(None);
    let result = parser.render_preview("- a\n    - [x] done".to_string(), 200);
    assert_eq!(result.plain_text, "a\n✓ done");
}

#[test]
fn toggle_nested_checkbox_ffi() {
    let parser = CindermarkParser::new(None);
    let toggled = parser.toggle_checkbox("- a\n    - [ ] nested task".to_string(), 1);
    assert_eq!(toggled, "- a\n    - [x] nested task");
    let back = parser.toggle_checkbox(toggled, 1);
    assert_eq!(back, "- a\n    - [ ] nested task");
}

#[test]
fn incremental_edit_in_nested_list_matches_full_parse() {
    let parser = CindermarkParser::new(None);
    let old = "- a\n    - b\n        - c\n- d";
    parser.parse_editable(old.to_string());

    // Append "XY" to nested "b" ('b' is at UTF-16 10, edit after it).
    let new = "- a\n    - bXY\n        - c\n- d";
    let incremental = parser.parse_editable_incremental(new.to_string(), 11, 0, 2);

    let full = parse_ffi_editable(new);

    assert_eq!(incremental.blocks.len(), full.blocks.len());
    for (inc, ful) in incremental.blocks.iter().zip(full.blocks.iter()) {
        assert_eq!(inc.block_type, ful.block_type);
        assert_eq!(inc.utf16_start, ful.utf16_start);
        assert_eq!(inc.utf16_end, ful.utf16_end);
        assert_eq!(inc.list_indent, ful.list_indent);
        assert_eq!(inc.marker_utf16_start, ful.marker_utf16_start);
        assert_eq!(inc.marker_utf16_end, ful.marker_utf16_end);
        assert_eq!(inc.text, ful.text);
    }
}

#[test]
fn incremental_indent_reclassification_matches_full_parse() {
    let parser = CindermarkParser::new(None);
    let old = "- a\n- b\n- c";
    parser.parse_editable(old.to_string());

    // Indent "- b" by 4 spaces (insert at line start, UTF-16 4).
    let new = "- a\n    - b\n- c";
    let incremental = parser.parse_editable_incremental(new.to_string(), 4, 0, 4);

    let full = parse_ffi_editable(new);

    assert_eq!(incremental.blocks.len(), full.blocks.len());
    for (inc, ful) in incremental.blocks.iter().zip(full.blocks.iter()) {
        assert_eq!(inc.block_type, ful.block_type);
        assert_eq!(inc.list_indent, ful.list_indent);
        assert_eq!(inc.utf16_start, ful.utf16_start);
        assert_eq!(inc.utf16_end, ful.utf16_end);
    }
    assert_eq!(
        incremental.blocks[1].block_type,
        cindermark::FfiBlockType::BulletItem
    );
    assert_eq!(incremental.blocks[1].list_indent, 4);
}

#[test]
fn incremental_style_only_nested_parity() {
    let parser = CindermarkParser::new(None);
    let old = "- a\n    - [ ] task";
    parser.parse_editable(old.to_string());

    // Type "s" at the end of "task" (UTF-16 end = 18).
    let new = "- a\n    - [ ] tasks";
    let style = parser.parse_editable_incremental_style_only(new.to_string(), 18, 0, 1);

    let full = parse_ffi_editable(new);

    assert_eq!(style.blocks.len(), full.blocks.len());
    for (inc, ful) in style.blocks.iter().zip(full.blocks.iter()) {
        assert_eq!(inc.block_type, ful.block_type);
        assert_eq!(inc.list_indent, ful.list_indent);
        assert_eq!(inc.utf16_start, ful.utf16_start);
        assert_eq!(inc.utf16_end, ful.utf16_end);
    }
    assert_eq!(
        style.blocks[1].block_type,
        cindermark::FfiBlockType::Checkbox
    );
}

// MARK: - Horizontal Rule

#[test]
fn hr_dashes() {
    let result = parse_ffi("---");
    let rules: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::HorizontalRule)
        .collect();
    assert_eq!(rules.len(), 1);
}

#[test]
fn hr_asterisks() {
    let result = parse_ffi("***");
    let rules: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::HorizontalRule)
        .collect();
    assert_eq!(rules.len(), 1);
}

#[test]
fn hr_underscores() {
    let result = parse_ffi("___");
    let rules: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::HorizontalRule)
        .collect();
    assert_eq!(rules.len(), 1);
}

#[test]
fn hr_four_asterisks_is_not_hr() {
    // **** (bold button inserts this) must NOT be treated as a thematic break
    let result = parse_ffi("****");
    let rules: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::HorizontalRule)
        .collect();
    assert_eq!(
        rules.len(),
        0,
        "**** should be a paragraph, not a horizontal rule"
    );
}

#[test]
fn hr_three_asterisks_still_works() {
    // *** must still be a thematic break
    let result = parse_ffi("***");
    let rules: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::HorizontalRule)
        .collect();
    assert_eq!(rules.len(), 1, "*** should still be a horizontal rule");
}

#[test]
fn hr_four_asterisks_with_text_is_paragraph() {
    // ****text**** should be a paragraph with emphasis, not a horizontal rule
    let result = parse_ffi("****text****");
    let rules: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::HorizontalRule)
        .collect();
    assert_eq!(
        rules.len(),
        0,
        "****text**** should be a paragraph, not a horizontal rule"
    );
    let paras: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::Paragraph)
        .collect();
    assert_eq!(paras.len(), 1);
}

// MARK: - Empty Lines

#[test]
fn collapse_empty_lines() {
    let result = parse_ffi("Para one\n\n\n\nPara two");
    let empties: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::Empty)
        .collect();
    assert_eq!(
        empties.len(),
        1,
        "Multiple blank lines should collapse to one .empty block"
    );
}

// MARK: - Checkbox Toggle

#[test]
fn toggle_unchecked() {
    let parser = CindermarkParser::new(None);
    let result = parser.toggle_checkbox("- [ ] Task one\n- [ ] Task two".to_string(), 0);
    assert!(result.starts_with("- [x] Task one"));
}

#[test]
fn toggle_checked() {
    let parser = CindermarkParser::new(None);
    let result = parser.toggle_checkbox("- [x] Task one\n- [ ] Task two".to_string(), 0);
    assert!(result.starts_with("- [ ] Task one"));
}

#[test]
fn toggle_preserves_indent() {
    let parser = CindermarkParser::new(None);
    let result = parser.toggle_checkbox("    - [ ] Indented task".to_string(), 0);
    assert_eq!(result, "    - [x] Indented task");
}

#[test]
fn toggle_out_of_bounds() {
    let parser = CindermarkParser::new(None);
    let input = "- [ ] Only line".to_string();
    let result = parser.toggle_checkbox(input.clone(), 99);
    assert_eq!(result, input);
}

#[test]
fn toggle_non_checkbox() {
    let parser = CindermarkParser::new(None);
    let input = "Regular text".to_string();
    let result = parser.toggle_checkbox(input.clone(), 0);
    assert_eq!(result, input);
}

// MARK: - Complex Document

#[test]
fn full_document() {
    let input = "# Title\n\nA paragraph with **bold** text.\n\n```swift\nlet x = 42\n```\n\n> A quote\n\n- Item one\n- Item two\n\n1. First\n2. Second\n\n- [ ] Todo\n- [x] Done\n\n---\n\nEnd.";
    let result = parse_ffi(input);

    let mut headings = 0;
    let mut paragraphs = 0;
    let mut code_blocks = 0;
    let mut quotes = 0;
    let mut ulists = 0;
    let mut olists = 0;
    let mut checkboxes = 0;
    let mut rules = 0;

    for block in &result.blocks {
        match block.block_type {
            cindermark::FfiBlockType::Heading => headings += 1,
            cindermark::FfiBlockType::Paragraph => paragraphs += 1,
            cindermark::FfiBlockType::CodeBlock => code_blocks += 1,
            cindermark::FfiBlockType::Blockquote => quotes += 1,
            cindermark::FfiBlockType::BulletList => ulists += 1,
            cindermark::FfiBlockType::OrderedList => olists += 1,
            cindermark::FfiBlockType::Checkbox => checkboxes += 1,
            cindermark::FfiBlockType::HorizontalRule => rules += 1,
            _ => {}
        }
    }

    assert_eq!(headings, 1);
    assert_eq!(paragraphs, 2); // "A paragraph..." and "End."
    assert_eq!(code_blocks, 1);
    assert_eq!(quotes, 1);
    assert_eq!(ulists, 1);
    assert_eq!(olists, 1);
    assert_eq!(checkboxes, 2);
    assert_eq!(rules, 1);
}

// MARK: - Edge Cases

#[test]
fn empty_string() {
    let result = parse_ffi("");
    let non_empty: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type != cindermark::FfiBlockType::Empty)
        .collect();
    assert!(non_empty.is_empty());
}

#[test]
fn whitespace_only() {
    let result = parse_ffi("   \n   \n   ");
    let non_empty: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type != cindermark::FfiBlockType::Empty)
        .collect();
    assert!(non_empty.is_empty());
}

#[test]
fn checkbox_line_index() {
    let result = parse_ffi("# Title\n\n- [ ] First task\n- [x] Second task");
    let cbs: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::Checkbox)
        .collect();
    assert_eq!(cbs.len(), 2);
    assert_eq!(cbs[0].line_start, 2); // line index 2 (0-based)
    assert_eq!(cbs[1].line_start, 3); // line index 3
}

// MARK: - Tables

#[test]
fn simple_table() {
    let result = parse_ffi("| A | B |\n| --- | --- |\n| 1 | 2 |");
    let tables: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::Table)
        .collect();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].table_headers, vec!["A", "B"]);
    assert_eq!(tables[0].table_rows.len(), 1);
    assert_eq!(tables[0].table_rows[0], vec!["1", "2"]);
}

#[test]
fn table_with_alignments() {
    let result = parse_ffi("| Left | Center | Right |\n| :--- | :---: | ---: |\n| a | b | c |");
    let tables: Vec<_> = result
        .blocks
        .iter()
        .filter(|b| b.block_type == cindermark::FfiBlockType::Table)
        .collect();
    assert_eq!(tables[0].table_alignments, vec![1, 2, 3]); // Left=1, Center=2, Right=3
}

// MARK: - Footnote Definition

#[test]
fn footnote_definition() {
    let result = parse_ffi("[^1]: Some footnote text");
    assert_eq!(result.blocks.len(), 1);
    assert_eq!(
        result.blocks[0].block_type,
        cindermark::FfiBlockType::FootnoteDefinition
    );
}

// MARK: - Wiki Links

#[test]
fn extract_wiki_links() {
    let parser = CindermarkParser::new(None);
    let links = parser.extract_wiki_links("see [[Note One]] and [[Note Two]]".to_string());
    assert_eq!(links, vec!["Note One", "Note Two"]);
}

#[test]
fn wiki_links_skip_code() {
    let parser = CindermarkParser::new(None);
    let links = parser.extract_wiki_links("text `[[not a link]]` and [[real link]]".to_string());
    assert_eq!(links, vec!["real link"]);
}

#[test]
fn wiki_links_skip_code_block() {
    let parser = CindermarkParser::new(None);
    let links = parser.extract_wiki_links("text\n```\n[[not a link]]\n```\n[[real]]".to_string());
    assert_eq!(links, vec!["real"]);
}

// MARK: - Inline Spans via FFI

#[test]
fn inline_bold_via_ffi() {
    let result = parse_ffi("**bold text**");
    assert!(!result.blocks[0].inline_spans.is_empty());
    assert_eq!(
        result.blocks[0].inline_spans[0].inline_type,
        cindermark::FfiInlineType::Bold
    );
}

#[test]
fn inline_italic_via_ffi() {
    let result = parse_ffi("*italic text*");
    assert!(!result.blocks[0].inline_spans.is_empty());
    assert_eq!(
        result.blocks[0].inline_spans[0].inline_type,
        cindermark::FfiInlineType::Italic
    );
}

#[test]
fn inline_code_via_ffi() {
    let result = parse_ffi("`code`");
    assert!(!result.blocks[0].inline_spans.is_empty());
    assert_eq!(
        result.blocks[0].inline_spans[0].inline_type,
        cindermark::FfiInlineType::InlineCode
    );
}

#[test]
fn inline_wiki_link_via_ffi() {
    let result = parse_ffi("see [[Note]]");
    assert!(!result.blocks[0].inline_spans.is_empty());
    assert_eq!(
        result.blocks[0].inline_spans[0].inline_type,
        cindermark::FfiInlineType::WikiLink
    );
}

#[test]
fn inline_highlight_via_ffi() {
    let result = parse_ffi("==highlighted==");
    assert!(!result.blocks[0].inline_spans.is_empty());
    assert_eq!(
        result.blocks[0].inline_spans[0].inline_type,
        cindermark::FfiInlineType::Highlight
    );
}

#[test]
fn inline_subreddit_autolinks_in_editable_list_items() {
    let result = parse_ffi_editable("- r/apple\n1. r/SwiftUI\n- [ ] r/iOS");
    let urls: Vec<_> = result
        .blocks
        .iter()
        .flat_map(|block| block.inline_spans.iter())
        .filter_map(|span| {
            if let cindermark::FfiInlineType::AutoLink { url } = &span.inline_type {
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
            "https://www.reddit.com/r/iOS",
        ]
    );
}

#[test]
fn inline_subreddit_autolinks_after_marker_attachment() {
    let result = parse_ffi_editable("\u{FFFC}r/apple\n\u{FFFC}r/SwiftUI");
    let urls: Vec<_> = result
        .blocks
        .iter()
        .flat_map(|block| block.inline_spans.iter())
        .filter_map(|span| {
            if let cindermark::FfiInlineType::AutoLink { url } = &span.inline_type {
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
fn no_inline_in_code_block() {
    let result = parse_ffi("```\n**not bold**\n```");
    for block in &result.blocks {
        assert!(
            block.inline_spans.is_empty(),
            "Code blocks should have no inline spans"
        );
    }
}

// MARK: - Regression tests

#[test]
fn crlf_line_endings() {
    let result = parse_ffi("# Title\r\n\r\nParagraph\r\n");
    assert_eq!(
        result.blocks[0].block_type,
        cindermark::FfiBlockType::Heading
    );
    assert_eq!(result.blocks[0].text, "Title");
    // Paragraph text should NOT contain \r
    let para = result
        .blocks
        .iter()
        .find(|b| b.block_type == cindermark::FfiBlockType::Paragraph);
    if let Some(p) = para {
        assert!(!p.text.contains('\r'), "Paragraph should not contain \\r");
    }
}

#[test]
fn numbered_item_large_number_preserved() {
    let result = parse_ffi_editable("300. Item");
    assert_eq!(
        result.blocks[0].block_type,
        cindermark::FfiBlockType::NumberedItem
    );
    assert_eq!(result.blocks[0].number, 300);
}

#[test]
fn ordered_list_number_field_in_ffi() {
    // Grouped mode currently hardcodes start to 1
    let result = parse_ffi("1. First\n2. Second");
    let list = result
        .blocks
        .iter()
        .find(|b| b.block_type == cindermark::FfiBlockType::OrderedList);
    assert!(list.is_some());
    assert_eq!(list.unwrap().number, 1);
}

#[test]
fn tilde_underline_adjacent_strikethrough() {
    // ~under~ and ~~strike~~ should both work when separated by space
    let result = parse_ffi("~under~ ~~strike~~");
    let spans = &result.blocks[0].inline_spans;
    assert_eq!(
        spans.len(),
        2,
        "Should have underline and strikethrough spans"
    );
    assert_eq!(
        spans[0].inline_type,
        cindermark::FfiInlineType::UnderlineTilde
    );
    assert_eq!(
        spans[1].inline_type,
        cindermark::FfiInlineType::Strikethrough
    );
}

#[test]
fn tilde_underline_not_confused_by_adjacent_double_tilde() {
    // ~foo~~bar~ should parse as tilde underline wrapping "foo~~bar"
    // (strikethrough ~~ has no match so leaves unclaimed, tilde finds single ~ pair)
    let result = parse_ffi("~foo~~bar~");
    let spans = &result.blocks[0].inline_spans;
    // The tilde underline opener at 0 and closer at 9 are both single tildes
    // with a run of 2 tildes in between that don't match strikethrough
    assert!(!spans.is_empty(), "Should have at least one span");
}

// MARK: - Incremental Parsing (Phase 5)

#[test]
fn incremental_parse_basic() {
    let parser = cindermark::CindermarkParser::new(None);

    // Initial full parse establishes snapshot.
    let initial = parser.parse_editable("# Title\n\nHello world".to_string());
    assert_eq!(initial.blocks.len(), 3);

    // Incremental: insert "X" at start of "Hello world" (offset 9).
    let result = parser.parse_editable_incremental("# Title\n\nXHello world".to_string(), 9, 0, 1);
    assert_eq!(result.blocks.len(), 3);
    assert_eq!(
        result.blocks[0].block_type,
        cindermark::FfiBlockType::Heading
    );
    assert_eq!(
        result.blocks[2].block_type,
        cindermark::FfiBlockType::Paragraph
    );
    // Dirty range should be small (not the full document).
    assert!(result.dirty_end - result.dirty_start <= 3);
}

#[test]
fn incremental_parse_chained_edits() {
    let parser = cindermark::CindermarkParser::new(None);

    // Initial parse.
    parser.parse_editable("Hello".to_string());

    // Chain of incremental edits: "Hello" → "Hello " → "Hello W" → "Hello World"
    let r1 = parser.parse_editable_incremental("Hello ".to_string(), 5, 0, 1);
    assert_eq!(r1.blocks.len(), 1);

    let r2 = parser.parse_editable_incremental("Hello W".to_string(), 6, 0, 1);
    assert_eq!(r2.blocks.len(), 1);

    let r3 = parser.parse_editable_incremental("Hello World".to_string(), 7, 0, 4);
    assert_eq!(r3.blocks.len(), 1);
    assert_eq!(r3.blocks[0].text, "Hello World");
}

#[test]
fn incremental_parse_code_fence_fallback() {
    let parser = cindermark::CindermarkParser::new(None);

    // Initial parse.
    parser.parse_editable("# Title\n\nSome text".to_string());

    // Insert ``` — should fall back to full re-parse.
    let result =
        parser.parse_editable_incremental("# Title\n\n```\nSome text".to_string(), 9, 0, 4);
    // Full re-parse: dirty_start=0 means all blocks are dirty.
    assert_eq!(result.dirty_start, 0);
    assert!(result.blocks.len() >= 2);
}

#[test]
fn incremental_parse_reset_state() {
    let parser = cindermark::CindermarkParser::new(None);

    // Initial parse.
    parser.parse_editable("Hello".to_string());

    // Reset state.
    parser.reset_state();

    // Incremental without prior state → should still work (full re-parse).
    let result = parser.parse_editable_incremental("Hello World".to_string(), 5, 0, 6);
    assert_eq!(result.blocks.len(), 1);
    assert_eq!(result.dirty_start, 0); // full re-parse since no snapshot
}

#[test]
fn incremental_dirty_range_reported_correctly() {
    let parser = cindermark::CindermarkParser::new(None);

    // Document with many blocks.
    parser.parse_editable("# H1\n\n## H2\n\n### H3\n\nPara\n\n---".to_string());

    // Edit "Para" → "ParaX" (in block 6 or so).
    let result = parser.parse_editable_incremental(
        "# H1\n\n## H2\n\n### H3\n\nParaX\n\n---".to_string(),
        25,
        0,
        1,
    );

    // Dirty range should be small — not the whole document.
    let dirty_count = result.dirty_end - result.dirty_start;
    assert!(
        dirty_count <= 4,
        "Expected ≤4 dirty blocks, got {} ({}..{})",
        dirty_count,
        result.dirty_start,
        result.dirty_end
    );

    // Total blocks should match full parse.
    let full = parser.parse_editable("# H1\n\n## H2\n\n### H3\n\nParaX\n\n---".to_string());
    assert_eq!(result.blocks.len(), full.blocks.len());
}

#[test]
fn incremental_style_only_matches_full_variant() {
    let parser = cindermark::CindermarkParser::new(None);

    // Initial full parse to establish snapshot.
    parser.parse_editable("# Title\n\nHello world\n\n- item".to_string());

    // Use style-only variant for an edit.
    let style_result = parser.parse_editable_incremental_style_only(
        "# Title\n\nHello world!\n\n- item".to_string(),
        20,
        0,
        1, // insert "!" at offset 20
    );

    // Re-establish snapshot and use the full variant for the same edit.
    parser.parse_editable("# Title\n\nHello world\n\n- item".to_string());
    let full_result = parser.parse_editable_incremental(
        "# Title\n\nHello world!\n\n- item".to_string(),
        20,
        0,
        1,
    );

    // Blocks and dirty range must match.
    assert_eq!(style_result.blocks.len(), full_result.blocks.len());
    assert_eq!(style_result.dirty_start, full_result.dirty_start);
    assert_eq!(style_result.dirty_end, full_result.dirty_end);
    assert_eq!(style_result.line_count, full_result.line_count);

    // Block content must match exactly.
    for (s, f) in style_result.blocks.iter().zip(full_result.blocks.iter()) {
        assert_eq!(s.block_type, f.block_type);
        assert_eq!(s.utf16_start, f.utf16_start);
        assert_eq!(s.utf16_end, f.utf16_end);
    }
}

#[test]
fn incremental_style_only_no_snapshot_fallback() {
    let parser = cindermark::CindermarkParser::new(None);

    // Call style-only without prior snapshot — should fall back to full parse.
    let result = parser.parse_editable_incremental_style_only("Hello World".to_string(), 0, 0, 11);
    assert_eq!(result.blocks.len(), 1);
    assert_eq!(
        result.blocks[0].block_type,
        cindermark::FfiBlockType::Paragraph
    );
    assert_eq!(result.dirty_start, 0);
}
