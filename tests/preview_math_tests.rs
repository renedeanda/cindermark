use cindermark::{CindermarkParser, FfiInlineType};

#[test]
fn preview_does_not_pair_underline_across_blocks() {
    let preview = CindermarkParser::new(None).render_preview("++first\n\nsecond++".into(), 200);
    assert_eq!(preview.plain_text, "++first\nsecond++");
    assert!(preview.spans.is_empty());
}

#[test]
fn preview_preserves_readable_math_fallback_and_opaque_content() {
    let parser = CindermarkParser::new(None);
    for source in [
        "$x + **y**$",
        "$$x + **y**$$",
        "~~~math\nx + **y**\n~~~",
        "> $$x + **y**$$",
    ] {
        let preview = parser.render_preview(source.into(), 200);
        assert!(
            preview.plain_text.contains("x + **y**"),
            "{source}: {preview:?}"
        );
        assert_eq!(preview.spans.len(), 1);
        assert!(matches!(
            preview.spans[0].span_type,
            FfiInlineType::Math { .. }
        ));
        assert!(preview.plain_text.contains('$'));
    }
}

#[test]
fn preview_spans_never_extend_past_surrogate_safe_truncation() {
    let preview = CindermarkParser::new(None).render_preview("++a🍃++".into(), 2);
    assert_eq!(preview.plain_text, "a");
    assert!(preview.spans.iter().all(|span| span.end <= 1));
}

#[test]
fn truncated_math_is_literal_not_a_full_formula_attachment() {
    let preview = CindermarkParser::new(None).render_preview("$abcdef$".into(), 4);
    assert_eq!(preview.plain_text, "$abc");
    assert!(preview.spans.is_empty());
}
