use cindermark::{CindermarkParser, FfiBlockType};

#[test]
fn tab_indentation_preserves_columns_beyond_the_list_prefix() {
    for (source, expected) in [
        ("- $$\n\tx\n  $$", "  x"),
        ("- ~~~math\n\tx\n  ~~~", "  x"),
        ("- item\n    ~~~math\n\tx\n    ~~~", "x"),
        ("1. $$\n\tα\n   $$", " α"),
        ("- item\n\t~~~math\n\tx\n\t~~~", "x"),
        ("- item\n\t$$\n\tα\n\t$$", "  α"),
        ("- $$\r\n\t🍃\r\n  $$", "  🍃"),
    ] {
        let parsed = CindermarkParser::new(None).parse_editable(source.into());
        let math = parsed
            .blocks
            .iter()
            .find(|block| matches!(block.block_type, FfiBlockType::Math { .. }))
            .expect(source);
        assert!(
            matches!(math.block_type, FfiBlockType::Math { .. }),
            "{source}"
        );
        assert_eq!(math.text, expected, "{source}");
        let FfiBlockType::Math {
            content_utf16_start,
            content_utf16_end,
            ..
        } = math.block_type
        else {
            unreachable!()
        };
        let utf16: Vec<_> = source.encode_utf16().collect();
        let content =
            String::from_utf16(&utf16[content_utf16_start as usize..content_utf16_end as usize])
                .unwrap();
        assert_eq!(content.trim(), expected.trim(), "{source}");
    }
}

#[test]
fn indented_code_cannot_introduce_math_list_containers() {
    for source in [
        "    - $$x$$",
        "\t- $$x$$",
        "    1. ~~~math\n       x\n       ~~~",
        "- parent\n      - $$x$$",
        ">     - $$x$$",
    ] {
        for grouped in [false, true] {
            let parser = CindermarkParser::new(None);
            let parsed = if grouped {
                parser.parse(source.into())
            } else {
                parser.parse_editable(source.into())
            };
            assert!(
                !parsed
                    .blocks
                    .iter()
                    .any(|block| matches!(block.block_type, FfiBlockType::Math { .. })),
                "{source}"
            );
        }
    }
}

#[test]
fn math_continues_a_list_item_without_consuming_siblings() {
    for source in [
        "- item\n  $$\n  x + y\n  $$\n- next",
        "- item\n\n  ~~~math\n  x + y\n  ~~~\n- next",
        "1. item\n   $$x + y$$\n2. next",
    ] {
        for grouped in [false, true] {
            let parser = CindermarkParser::new(None);
            let parsed = if grouped {
                parser.parse(source.into())
            } else {
                parser.parse_editable(source.into())
            };
            let math = parsed
                .blocks
                .iter()
                .find(|block| matches!(block.block_type, FfiBlockType::Math { .. }))
                .expect(source);
            assert_eq!(math.text, "x + y");
            assert!(
                parsed.blocks.last().unwrap().text == "next"
                    || !parsed.blocks.last().unwrap().list_items.is_empty()
            );
        }
    }
}

#[test]
fn math_can_start_on_a_list_marker_line() {
    for source in [
        "- $$x$$\n- next",
        "003) ~~~TeX\n     x\n     ~~~\n004) next",
        "- [x] $$x$$\n- [ ] next",
    ] {
        let parsed = CindermarkParser::new(None).parse_editable(source.into());
        assert!(
            matches!(parsed.blocks[0].block_type, FfiBlockType::Math { .. }),
            "{source}"
        );
        assert_eq!(parsed.blocks[0].text, "x");
        assert!(!parsed.blocks[0].marker_source.is_empty());
        assert_eq!(parsed.blocks.len(), 2);
    }
}

#[test]
fn unclosed_list_math_stays_inside_its_container() {
    let parsed = CindermarkParser::new(None).parse_editable("- ~~~math\n  x\nplain\n~~~".into());
    assert_eq!(parsed.blocks[0].text, "x");
    assert_eq!(parsed.blocks[1].text, "plain");
    let parsed = CindermarkParser::new(None).parse_editable("- $$\n  x\nplain\n$$".into());
    assert!(!parsed
        .blocks
        .iter()
        .any(|block| matches!(block.block_type, FfiBlockType::Math { .. })));
}

#[test]
fn list_math_exposes_task_and_ordered_identity() {
    for (source, kind, indent, number) in [
        ("- [x] item\n  $$x$$", 4, 2, 0),
        ("- [ ] $$x$$", 3, 2, 0),
        ("003) $$x$$", 2, 5, 3),
        ("> - $$x$$", 1, 2, 0),
    ] {
        let parsed = CindermarkParser::new(None).parse_editable(source.into());
        let math = parsed.blocks.last().unwrap();
        assert!(
            matches!(math.block_type, FfiBlockType::Math { list_kind, list_content_indent, .. } if list_kind == kind && list_content_indent == indent),
            "{source}"
        );
        assert_eq!(math.number, number);
        assert_eq!(math.is_checked, kind == 4);
    }
}

#[test]
fn list_fences_allow_blank_lines_and_nested_context_unwinds() {
    let source = "- parent\n  - child\n    $$child$$\n  ~~~math\n  parent\n\n  + x\n  ~~~\noutside";
    let parsed = CindermarkParser::new(None).parse_editable(source.into());
    let math: Vec<_> = parsed
        .blocks
        .iter()
        .filter(|block| matches!(block.block_type, FfiBlockType::Math { .. }))
        .collect();
    assert_eq!(math.len(), 2);
    assert_eq!(math[0].text, "child");
    assert_eq!(math[1].text, "parent\n\n+ x");
    assert!(matches!(
        math[1].block_type,
        FfiBlockType::Math {
            list_content_indent: 2,
            ..
        }
    ));
    assert_eq!(parsed.blocks.last().unwrap().text, "outside");
}

#[test]
fn list_math_incremental_edits_match_full_parse() {
    for source in [
        "- parent\n  $$x$$\n- next",
        "- parent\n\n  paragraph\n\n  $$x$$\n\nnext",
        "- [x] ~~~math\n  α\n  ~~~\nlast",
        "> - item\n>   $$x$$\nend",
        "- item\n\t$$\n\tα\n\t$$\nend",
        "- item\n\t~~~math\n\tx\n\t~~~\nend",
    ] {
        for offset in source.char_indices().map(|(index, _)| index) {
            let parser = CindermarkParser::new(None);
            parser.parse_editable(source.into());
            let mut edited = source.to_owned();
            edited.insert(offset, '🍃');
            let incremental = parser.parse_editable_incremental_style_only(
                edited.clone(),
                source[..offset].encode_utf16().count() as u32,
                0,
                2,
            );
            let full = CindermarkParser::new(None).parse_editable(edited);
            assert_eq!(incremental.blocks, full.blocks, "{source} @ {offset}");
        }
    }
}
