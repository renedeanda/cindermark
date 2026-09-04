use cindermark::{CindermarkParser, FfiBlockType};

#[test]
fn display_math_preserves_expression_and_content_range() {
    let parser = CindermarkParser::new(None);
    for source in ["$$α + β$$", "$$\nα + β\n$$", "   $$α + β$$"] {
        let result = parser.parse_editable(source.into());
        let block = &result.blocks[0];
        let FfiBlockType::Math {
            syntax,
            content_utf16_start,
            content_utf16_end,
        } = block.block_type
        else {
            panic!("{source}");
        };
        assert_eq!(syntax, 0);
        assert_eq!(block.text, "α + β");
        let utf16: Vec<_> = source.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(&utf16[content_utf16_start as usize..content_utf16_end as usize])
                .unwrap(),
            block.text
        );
        assert!(block.inline_spans.is_empty());
    }
}

#[test]
fn fenced_math_respects_fence_kind_length_and_info() {
    for source in [
        "````math label\nα\n```\nβ\n````",
        "~~~~LaTeX label\nα\n~~~\nβ\n~~~~",
    ] {
        let result = CindermarkParser::new(None).parse_editable(source.into());
        assert_eq!(result.blocks.len(), 1);
        assert!(matches!(
            result.blocks[0].block_type,
            FfiBlockType::Math { syntax: 1, .. }
        ));
        assert!(result.blocks[0]
            .language
            .as_ref()
            .unwrap()
            .ends_with(" label"));
        assert!(result.blocks[0].text.contains("β"));
    }
}

#[test]
fn invalid_display_math_leaves_following_content_available() {
    for source in [
        "$$\nx\n\ny\n$$",
        "$$\nx",
        "$$$$",
        "$$\nx\n# Heading",
        "prefix $$x$$",
        "    $$x$$",
    ] {
        let result = CindermarkParser::new(None).parse_editable(source.into());
        assert!(
            !result
                .blocks
                .iter()
                .any(|block| matches!(block.block_type, FfiBlockType::Math { .. })),
            "{source}"
        );
    }
}

#[test]
fn math_suffix_ranges_follow_incremental_edits() {
    let parser = CindermarkParser::new(None);
    let source = "first\n\n$$α$$\n\nlast";
    parser.parse_editable(source.into());
    let edited = "🍃first\n\n$$α$$\n\nlast";
    let incremental = parser.parse_editable_incremental_style_only(edited.into(), 0, 0, 2);
    let full = CindermarkParser::new(None).parse_editable(edited.into());
    assert_eq!(incremental.blocks, full.blocks);
}
