use cindermark::CindermarkParser;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn delimiter_edits_match_full_parse(
        parts in prop::collection::vec(prop::sample::select(vec![
            "$", "$$", "++", "+++", "\\", " ", "\n", "\r\n", "🍃", "e\u{301}",
            "x", "`", "**", "[[x]]", "|", "\0", "\t", "{", "}",
            "<!--", "-->", "<?", "?>", "<![CDATA[", "]]>", "<!DOC", ">",
            "<script>", "</script>", "<div>", "<custom-tag>",
        ]), 0..80),
        inserted in prop::sample::select(vec!["$x$", "++x++", "🍃", "\n\n", "\\"]),
        position in any::<usize>(),
    ) {
        let source = parts.concat();
        let boundaries: Vec<_> = source.char_indices().map(|(i, _)| i).chain([source.len()]).collect();
        let offset = boundaries[position % boundaries.len()];
        let utf16_offset = source[..offset].encode_utf16().count() as u32;
        let mut edited = source.clone();
        edited.insert_str(offset, inserted);
        let parser = CindermarkParser::new(None);
        parser.parse_editable(source);
        let incremental = parser.parse_editable_incremental_style_only(
            edited.clone(), utf16_offset, 0, inserted.encode_utf16().count() as u32,
        );
        let full = CindermarkParser::new(None).parse_editable(edited);
        prop_assert_eq!(incremental.blocks, full.blocks);
    }
}

#[test]
fn edit_hint_inside_surrogate_pair_remains_safe() {
    let parser = CindermarkParser::new(None);
    parser.parse_editable("🍃 $x$\n\n| ++a++ |\n| --- |".into());
    let edited = "🌱 $y$\n\n| ++a++ |\n| --- |";
    let incremental = parser.parse_editable_incremental_style_only(edited.into(), 1, 1, 1);
    let full = CindermarkParser::new(None).parse_editable(edited.into());
    assert_eq!(incremental.blocks, full.blocks);
}
