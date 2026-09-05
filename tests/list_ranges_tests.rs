use cindermark::{parser, CindermarkParser};

fn ranges(source: &str) -> Vec<cindermark::ast::ListItemRange> {
    parser::list_item_ranges(source, &parser::ParseOptions::default())
}

#[test]
fn nested_tasks_own_continuations_and_attachments() {
    let source = "# Tasks\n\n- [ ] Parent\n  - [x] Child 😀\n    details\n  ![](asset:one)\n- [ ] Sibling\n\nOutside\n";
    let items = ranges(source);
    assert_eq!(items.len(), 3);
    assert_eq!(items[1].parent_index, Some(0));
    assert_eq!(items[2].parent_index, None);
    assert_eq!(items[0].sibling_group, items[2].sibling_group);
    assert_eq!(items[1].checked, Some(true));
    assert_eq!(
        &source[items[0].byte_start as usize..items[0].byte_end as usize],
        "- [ ] Parent\n  - [x] Child 😀\n    details\n  ![](asset:one)\n"
    );
    assert_eq!(
        &source[items[1].byte_start as usize..items[1].byte_end as usize],
        "  - [x] Child 😀\n    details\n"
    );
    assert!(!source[items[2].byte_start as usize..items[2].byte_end as usize].contains("Outside"));
}

#[test]
fn blank_lines_and_eof_are_not_normalized() {
    let source = "- [ ] α\r\n\r\n  continuation\r\n\r\n- [x] e\u{301}😀";
    let items = ranges(source);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].sibling_group, items[1].sibling_group);
    assert_eq!(
        &source[items[0].byte_start as usize..items[0].byte_end as usize],
        "- [ ] α\r\n\r\n  continuation\r\n"
    );
    let utf16: Vec<u16> = source.encode_utf16().collect();
    for item in &items {
        assert_eq!(
            String::from_utf16(&utf16[item.utf16_start as usize..item.utf16_end as usize]).unwrap(),
            &source[item.byte_start as usize..item.byte_end as usize]
        );
    }
    assert_eq!(items[1].byte_end as usize, source.len());
}

#[test]
fn sibling_groups_do_not_cross_body_paragraphs() {
    let source = "- [ ] Parent\n  - [ ] First child\n  Body text\n  - [ ] Separate child list\n\nOutside\n\n- [ ] Separate root list\n";
    let items = ranges(source);
    assert_eq!(items.len(), 4);
    assert_eq!(items[1].parent_index, Some(0));
    assert_eq!(items[2].parent_index, Some(0));
    assert_ne!(items[1].sibling_group, items[2].sibling_group);
    assert_ne!(items[0].sibling_group, items[3].sibling_group);
}

#[test]
fn tabs_use_columns_and_opaque_blocks_do_not_create_tasks() {
    let source = "- [ ] Parent\n\t- [ ] Child\n    $$\n    - [x] opaque\n    $$\n- [x] Next\n\n```\n- [ ] code\n```\n";
    let items = ranges(source);
    assert_eq!(items.len(), 3);
    assert_eq!(items[1].indent_columns, 4);
    assert_eq!(items[1].parent_index, Some(0));
    assert!(source[items[0].byte_start as usize..items[0].byte_end as usize].contains("opaque"));
    assert!(!source[items[2].byte_start as usize..items[2].byte_end as usize].contains("code"));
}

#[test]
fn mixed_markers_and_attachment_extension_preserve_source() {
    let source = "* [ ] Parent\n  003) Ordered\n  + [X] Task\n  ![](asset:550e8400-e29b-41d4-a716-446655440000)\n* [ ] Next\n";
    let parser = CindermarkParser::new(Some("asset:".into()));
    let items = parser.list_item_ranges(source.into());
    assert_eq!(items.len(), 4);
    assert_eq!(items[1].checked, None);
    assert_eq!(items[2].checked, Some(true));
    assert_ne!(items[1].sibling_group, items[2].sibling_group);
    assert!(source[items[0].byte_start as usize..items[0].byte_end as usize].contains("550e8400"));
}

#[test]
fn query_does_not_replace_incremental_snapshot() {
    let parser = CindermarkParser::new(None);
    parser.parse_editable("- [ ] original\n".into());
    parser.list_item_ranges("- [x] unrelated\n".into());
    let edited = "- [ ] originals\n";
    let incremental = parser.parse_editable_incremental(edited.into(), 14, 0, 1);
    let full = CindermarkParser::new(None).parse_editable(edited.into());
    assert_eq!(incremental.blocks, full.blocks);
}

#[test]
fn large_list_ranges_remain_nested_and_complete() {
    let source = (0..10_000)
        .map(|i| format!("- [ ] Parent {i}\n  - [x] Child {i}\n"))
        .collect::<String>();
    let items = ranges(&source);
    assert_eq!(items.len(), 20_000);
    for pair in items.chunks_exact(2) {
        assert_eq!(pair[0].byte_end, pair[1].byte_end);
        assert_eq!(pair[0].sibling_group, 0);
    }
}

proptest::proptest! {
    #[test]
    fn generated_tasks_have_nonoverlapping_sibling_envelopes(
        rows in proptest::collection::vec((0usize..9, proptest::bool::ANY, "[^\\r\\n]{0,20}"), 1..100)
    ) {
        let source = rows.iter().map(|(indent, checked, text)| {
            format!("{}- [{}] {}\r\n", " ".repeat(indent * 2), if *checked { "x" } else { " " }, text)
        }).collect::<String>();
        let items = ranges(&source);
        for (index, item) in items.iter().enumerate() {
            if let Some(parent) = item.parent_index {
                proptest::prop_assert!((parent as usize) < index);
                proptest::prop_assert!(items[parent as usize].byte_end >= item.byte_end);
            }
            if let Some(previous) = items[..index].iter().rev().find(|previous| previous.sibling_group == item.sibling_group) {
                proptest::prop_assert!(previous.byte_end <= item.byte_start);
            }
        }
    }

    #[test]
    fn source_ranges_are_valid_and_nested(source in "(?s).{0,1500}") {
        let items = ranges(&source);
        let utf16: Vec<u16> = source.encode_utf16().collect();
        for (index, item) in items.iter().enumerate() {
            proptest::prop_assert!(item.byte_start < item.byte_end);
            proptest::prop_assert!(item.byte_end as usize <= source.len());
            proptest::prop_assert_eq!(String::from_utf16(&utf16[item.utf16_start as usize..item.utf16_end as usize]).unwrap(),
                &source[item.byte_start as usize..item.byte_end as usize]);
            if let Some(parent) = item.parent_index {
                proptest::prop_assert!((parent as usize) < index);
                proptest::prop_assert!(items[parent as usize].byte_start < item.byte_start);
                proptest::prop_assert!(items[parent as usize].byte_end >= item.byte_end);
            }
        }
    }
}
