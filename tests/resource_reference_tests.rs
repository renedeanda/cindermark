use cindermark::CindermarkParser;

#[test]
fn resource_query_preserves_incremental_snapshot() {
    let parser = CindermarkParser::new(None);
    parser.parse_editable("original\n".into());
    parser.resource_references("![unrelated](asset.png)".into());
    let edited = "originals\n";
    let incremental = parser.parse_editable_incremental(edited.into(), 8, 0, 1);
    let full = CindermarkParser::new(None).parse_editable(edited.into());
    assert_eq!(incremental.blocks, full.blocks);
}

#[test]
fn repeated_resources_and_backslashes_preserve_each_occurrence() {
    let source = (0..10_000)
        .map(|i| format!("{}![😀{i}](Attachments/x.png)\r\n", "\\".repeat(32)))
        .collect::<String>();
    let references = CindermarkParser::new(None).resource_references(source);
    assert_eq!(references.len(), 10_000);
    assert!(references.iter().all(|reference| reference.is_image));
    assert!(references
        .windows(2)
        .all(|pair| pair[0].utf16_end <= pair[1].utf16_start));
}

proptest::proptest! {
    #[test]
    fn generated_ranges_are_valid_utf16(source in "(?s).{0,1500}") {
        let units: Vec<u16> = source.encode_utf16().collect();
        for reference in CindermarkParser::new(None).resource_references(source) {
            proptest::prop_assert!(reference.utf16_start <= reference.label_utf16_start);
            proptest::prop_assert!(reference.label_utf16_start <= reference.label_utf16_end);
            proptest::prop_assert!(reference.label_utf16_end <= reference.utf16_end);
            proptest::prop_assert!(reference.utf16_end as usize <= units.len());
            proptest::prop_assert!(String::from_utf16(&units[reference.utf16_start as usize..reference.utf16_end as usize]).is_ok());
        }
    }
}
