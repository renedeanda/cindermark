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
            ..
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
        "$$\nx\n    $$",
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

#[test]
fn quoted_math_preserves_depth_expression_and_source_envelope() {
    for (source, depth) in [
        ("> $$\n> α\n> + β\n> $$\nplain", 1),
        ("> > ~~~~TeX label\n> > α\n> > + β\n> > ~~~~\nplain", 2),
    ] {
        let parsed = CindermarkParser::new(None).parse_editable(source.into());
        assert_eq!(parsed.blocks.len(), 2);
        let block = &parsed.blocks[0];
        let FfiBlockType::Math {
            quote_depth,
            content_utf16_start,
            content_utf16_end,
            ..
        } = block.block_type
        else {
            panic!("{source}")
        };
        assert_eq!(quote_depth, depth);
        assert_eq!(block.text, "α\n+ β");
        let utf16: Vec<_> = source.encode_utf16().collect();
        let envelope =
            String::from_utf16(&utf16[content_utf16_start as usize..content_utf16_end as usize])
                .unwrap();
        assert!(envelope.starts_with('α') && envelope.ends_with('β'));
        assert!(envelope.contains('>'));
        assert_eq!(parsed.blocks[1].text, "plain");
    }
}

#[test]
fn quoted_math_is_bounded_and_can_follow_ordinary_quote_text() {
    let parser = CindermarkParser::new(None);
    let parsed = parser.parse_editable("> Introduction\n> $$x$$\nplain".into());
    assert_eq!(parsed.blocks.len(), 3);
    assert!(matches!(
        parsed.blocks[1].block_type,
        FfiBlockType::Math { quote_depth: 1, .. }
    ));
    let parsed = parser.parse_editable("> ~~~math\n> x\nplain\n~~~".into());
    assert_eq!(parsed.blocks[0].text, "x");
    assert_eq!(parsed.blocks[1].text, "plain");
    let parsed = parser.parse_editable("> $$\n> x\nplain\n$$".into());
    assert!(!parsed
        .blocks
        .iter()
        .any(|block| matches!(block.block_type, FfiBlockType::Math { .. })));
}

#[test]
fn quoted_math_incremental_edits_match_full_parse() {
    for source in [
        "> $$x$$\n\nlast",
        "> ~~~math\n> α\n> ~~~\n\nlast",
        "> first\n> $$\n> α\n> $$",
    ] {
        for offset in source.char_indices().map(|(index, _)| index) {
            let parser = CindermarkParser::new(None);
            parser.parse_editable(source.into());
            let mut edited = source.to_owned();
            edited.insert(offset, '🍃');
            let start = source[..offset].encode_utf16().count() as u32;
            let incremental =
                parser.parse_editable_incremental_style_only(edited.clone(), start, 0, 2);
            let full = CindermarkParser::new(None).parse_editable(edited);
            assert_eq!(incremental.blocks, full.blocks, "{source} @ {offset}");
        }
    }
}
