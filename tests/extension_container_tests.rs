use cindermark::{CindermarkParser, FfiInlineType};

#[test]
fn extensions_share_absolute_ranges_across_inline_containers() {
    let content = "++café 👩‍💻++ $α+β$";
    for source in [
        format!("🍃 {content}"),
        format!("### {content}"),
        format!("- {content}"),
        format!("003) {content}"),
        format!("- [x] {content}"),
        format!("> {content}"),
        format!("> [!NOTE]\n> {content}"),
        format!("[^ref]: {content}"),
        format!("[{content}](https://example.org)"),
        format!("| {content} |\n| --- |\n| {content} |"),
    ] {
        for grouped in [false, true] {
            let parser = CindermarkParser::new(None);
            let doc = if grouped {
                parser.parse(source.clone())
            } else {
                parser.parse_editable(source.clone())
            };
            let spans: Vec<_> = doc
                .blocks
                .iter()
                .flat_map(|block| {
                    block
                        .inline_spans
                        .iter()
                        .chain(
                            block
                                .list_items
                                .iter()
                                .flat_map(|item| item.inline_spans.iter()),
                        )
                        .chain(
                            block
                                .table_cells
                                .iter()
                                .flat_map(|cell| cell.inline_spans.iter()),
                        )
                })
                .collect();
            let utf16: Vec<_> = source.encode_utf16().collect();
            for (kind, expected) in [
                (FfiInlineType::UnderlinePlus, "++café 👩‍💻++"),
                (
                    FfiInlineType::Math {
                        expression: "α+β".into(),
                    },
                    "$α+β$",
                ),
            ] {
                let matching: Vec<_> = spans
                    .iter()
                    .filter(|span| span.inline_type == kind)
                    .collect();
                assert!(
                    !matching.is_empty(),
                    "{source}, grouped={grouped}, {kind:?}"
                );
                for span in matching {
                    assert_eq!(
                        String::from_utf16(
                            &utf16[span.utf16_start as usize..span.utf16_end as usize]
                        )
                        .unwrap(),
                        expected
                    );
                    assert!(span.content_utf16_start >= span.utf16_start);
                    assert!(span.content_utf16_end <= span.utf16_end);
                }
            }
        }
    }
}

#[test]
fn extension_source_is_unchanged_after_incremental_container_edits() {
    for prefix in ["### ", "- [ ] ", "> ", "[^ref]: "] {
        let source = format!("{prefix}++café++ $α+β$\r\n");
        let parser = CindermarkParser::new(None);
        let original = parser.parse_editable(source.clone());
        let byte = source.find("café").unwrap();
        let offset = source[..byte].encode_utf16().count() as u32;
        let mut edited = source.clone();
        edited.insert(byte, '🍃');
        let incremental =
            parser.parse_editable_incremental_style_only(edited.clone(), offset, 0, 2);
        assert_eq!(
            incremental.blocks,
            CindermarkParser::new(None).parse_editable(edited).blocks
        );
        let restored = parser.parse_editable_incremental_style_only(source, offset, 2, 0);
        assert_eq!(restored.blocks, original.blocks);
    }
}
