use cindermark::{
    ast::{InlineKind, ParseMode},
    parser::{self, ParseOptions},
    CindermarkParser,
};

fn kinds(text: &str) -> Vec<InlineKind> {
    parser::parse_with_options(
        text,
        ParseMode::Editable,
        &ParseOptions {
            semantic_tokens: true,
            ..Default::default()
        },
    )
    .blocks
    .into_iter()
    .flat_map(|b| {
        b.inline_spans
            .into_iter()
            .chain(b.table_cells.into_iter().flat_map(|c| c.inline_spans))
    })
    .map(|s| s.kind)
    .collect()
}
fn names(text: &str) -> Vec<String> {
    kinds(text)
        .into_iter()
        .filter_map(|k| match k {
            InlineKind::Tag { name } => Some(format!("#{name}")),
            InlineKind::Mention { name } => Some(format!("@{name}")),
            _ => None,
        })
        .collect()
}
#[test]
fn words_nested_closed_and_unicode() {
    assert_eq!(names("@Mike #marketing #product-design #work/project #meeting notes# #学校/生物 101# (@Élodie) #cafe\u{301}"),
        ["@Mike", "#marketing", "#product-design", "#work/project", "#meeting notes", "#学校/生物 101", "@Élodie", "#cafe\u{301}"]);
}
#[test]
fn protected_contexts_and_escapes() {
    assert!(names("`@Mike #tag` [#label](https://x.org/#tag) ![@image](x) [[@Note]] user@example.com https://example.org/#tag <https://x.org/@a> %%#draft%% $@math$ \\#literal \\@literal <i title='#attr'> x </i>").is_empty());
    assert!(names("```\n@Mike #tag\n```\n\n    #code\n").is_empty());
}
#[test]
fn precedence_and_formatting() {
    let k = kinds("#fff #abcd #FF5733 #12345678 #fff-launch **@Mike** *#topic* # Heading");
    assert_eq!(
        k.iter()
            .filter(|k| matches!(k, InlineKind::HexColor { .. }))
            .count(),
        4
    );
    assert!(k.iter().any(|k| matches!(k, InlineKind::Bold)));
    assert!(k.iter().any(|k| matches!(k, InlineKind::Italic)));
    assert_eq!(
        names("#fff-launch **@Mike** *#topic*"),
        ["#fff-launch", "@Mike", "#topic"]
    );
    assert!(names("word#tag word@entity ##tag @@entity # Heading").is_empty());
}
#[test]
fn incomplete_tags_do_not_capture_next_opener() {
    assert_eq!(
        names("#one sentence #next #two words#"),
        ["#one", "#next", "#two words"]
    );
    assert_eq!(names("#unfinished phrase\n#next"), ["#unfinished", "#next"]);
}
#[test]
fn tables_and_lists() {
    assert_eq!(
        names("| A | B |\n| --- | --- |\n| #topic | @Mike |\n\n- #work\n> @Jane"),
        ["#topic", "@Mike", "#work", "@Jane"]
    );
}
#[test]
fn default_behavior_is_unchanged() {
    let doc = parser::parse("@Mike #tag #fff-launch", ParseMode::Editable);
    assert!(!doc.blocks[0]
        .inline_spans
        .iter()
        .any(|s| matches!(s.kind, InlineKind::Tag { .. } | InlineKind::Mention { .. })));
    assert!(doc.blocks[0]
        .inline_spans
        .iter()
        .any(|s| matches!(s.kind, InlineKind::HexColor { .. })));
}

#[test]
fn existing_color_boundaries_survive_semantic_annotations() {
    for source in ["accent:#fff", "color=#FF5733;", "#fff, #abcd", "**#fff**"] {
        let colors = |spans: Vec<InlineKind>| {
            spans
                .into_iter()
                .filter(|kind| matches!(kind, InlineKind::HexColor { .. }))
                .collect::<Vec<_>>()
        };
        let original = parser::parse(source, ParseMode::Editable)
            .blocks
            .into_iter()
            .flat_map(|block| block.inline_spans.into_iter().map(|span| span.kind))
            .collect();
        assert_eq!(colors(kinds(source)), colors(original), "{source}");
    }
    assert_eq!(
        names("#fff-launch #fff/project"),
        ["#fff-launch", "#fff/project"]
    );
    assert!(!kinds("#fff-launch #fff/project `color:#fff`")
        .iter()
        .any(|kind| matches!(kind, InlineKind::HexColor { .. })));
}
#[test]
fn completion_respects_context_and_utf16() {
    let p = CindermarkParser::with_semantic_tokens(None);
    for text in ["@", "Hello (@Mi", "😀 #mar", "#meeting no"] {
        assert!(
            p.semantic_completion(text.into(), text.encode_utf16().count() as u32)
                .is_some(),
            "{text}"
        );
    }
    for text in [
        "`@Mi`",
        "```\n@Mi",
        "https://example.org/#tag",
        "user@mi",
        "#fff",
        "\\@Mi",
        "# ",
        "#first `literal`",
        "#first [label](url)",
        "#first 😀",
        "#first  second",
        "`code`@Mike",
        "[link](url)#topic",
    ] {
        assert!(
            p.semantic_completion(text.into(), text.encode_utf16().count() as u32)
                .is_none(),
            "{text}"
        );
    }
}
#[test]
fn incremental_matches_full() {
    let p = CindermarkParser::with_semantic_tokens(None);
    let mut old = "😀 @Mike\n\n#fff\n\n#work/project".to_owned();
    p.parse_editable(old.clone());
    for new in [
        "😀 @Jane\n\n#fff\n\n#work/project",
        "😀 @Jane\n\n#fff-launch\n\n#work/project",
        "😀 @Jane\n\n#meeting notes#\n\n#work/project",
        "```\n😀 @Jane\n\n#meeting notes#\n\n#work/project",
    ] {
        let a: Vec<u16> = old.encode_utf16().collect();
        let b: Vec<u16> = new.encode_utf16().collect();
        let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let result = p.parse_editable_incremental(
            new.into(),
            prefix as u32,
            (a.len() - prefix) as u32,
            (b.len() - prefix) as u32,
        );
        let full = CindermarkParser::with_semantic_tokens(None).parse_editable(new.into());
        assert_eq!(result.blocks, full.blocks);
        old = new.to_owned();
    }
}

#[test]
fn token_ranges_slice_original_source() {
    let source = "😀 **@Élodie** #meeting notes# #fff-launch";
    let p = CindermarkParser::with_semantic_tokens(None);
    let doc = p.parse_editable(source.into());
    let units: Vec<u16> = source.encode_utf16().collect();
    for span in doc.blocks.iter().flat_map(|b| &b.inline_spans) {
        if let cindermark::FfiInlineType::Tag { name }
        | cindermark::FfiInlineType::Mention { name } = &span.inline_type
        {
            assert_eq!(
                String::from_utf16(
                    &units[span.content_utf16_start as usize..span.content_utf16_end as usize]
                )
                .unwrap(),
                *name
            );
        }
    }
}

#[test]
fn grouped_lists_and_preview_keep_semantics_and_sigils() {
    let p = CindermarkParser::with_semantic_tokens(None);
    let source = "- **@Mike** #work\n- `#literal` #meeting notes#\n\n#fff-launch";
    let doc = p.parse(source.into());
    let names: Vec<_> = doc
        .blocks
        .iter()
        .flat_map(|b| &b.inline_spans)
        .filter_map(|s| match &s.inline_type {
            cindermark::FfiInlineType::Tag { name }
            | cindermark::FfiInlineType::Mention { name } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert!(names.contains(&"Mike") && names.contains(&"work") && names.contains(&"meeting notes"));
    assert!(!names.contains(&"literal"));
    let preview = p.render_preview(source.into(), 1000);
    assert!(preview.plain_text.contains("@Mike"));
    assert!(preview.plain_text.contains("#meeting notes#"));
    assert!(preview.plain_text.contains("#fff-launch"));
}

#[test]
fn small_edits_shift_untouched_semantic_ranges() {
    let p = CindermarkParser::with_semantic_tokens(None);
    let mut source = "first\n\n😀 @Mike #work/project\n\nlast".to_owned();
    p.parse_editable(source.clone());
    for (offset, removed, inserted) in [(0, 0, "😀 "), (2, 1, ""), (0, 2, "new")] {
        let mut units: Vec<u16> = source.encode_utf16().collect();
        let replacement: Vec<u16> = inserted.encode_utf16().collect();
        units.splice(offset..offset + removed, replacement.iter().copied());
        source = String::from_utf16(&units).unwrap();
        let result = p.parse_editable_incremental(
            source.clone(),
            offset as u32,
            removed as u32,
            replacement.len() as u32,
        );
        let expected = CindermarkParser::with_semantic_tokens(None).parse_editable(source.clone());
        assert_eq!(result.blocks, expected.blocks);
    }
}

#[test]
fn completion_does_not_duplicate_a_suffix_or_closing_sigil() {
    let p = CindermarkParser::with_semantic_tokens(None);
    assert!(p.semantic_completion("@Michael".into(), 3).is_none());
    let closed = p.semantic_completion("#meeting notes#".into(), 14).unwrap();
    assert_eq!(closed.utf16_end, 15);
}
