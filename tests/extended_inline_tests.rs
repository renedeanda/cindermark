use cindermark::{CindermarkParser, FfiInlineType};

fn kinds(source: &str) -> Vec<FfiInlineType> {
    CindermarkParser::new(None)
        .parse_editable(source.into())
        .blocks
        .into_iter()
        .flat_map(|block| block.inline_spans.into_iter().map(|span| span.inline_type))
        .collect()
}

#[test]
fn backlink_apis_do_not_extract_links_from_math_or_table_code() {
    let parser = CindermarkParser::new(None);
    let source = "[[Visible]] $[[Opaque]]$\n\n| Header |\n| --- |\n| $[[Formula]]$ `[[Code]]` [[Cell]] |\n\n$$[[Display]]$$\n\n~~~math\n[[Fence]]\n~~~";
    assert_eq!(
        parser.extract_wiki_links(source.into()),
        vec!["Visible", "Cell"]
    );
    assert_eq!(
        parser.parse_editable(source.into()).wiki_links,
        vec!["Visible", "Cell"]
    );
}

#[test]
fn plus_underline_preserves_utf16_content_ranges() {
    let source = "🍃 ++café 👩‍💻++";
    let result = CindermarkParser::new(None).parse_editable(source.into());
    let span = &result.blocks[0].inline_spans[0];
    assert_eq!(span.inline_type, FfiInlineType::UnderlinePlus);
    assert_eq!((span.utf16_start, span.content_utf16_start), (3, 5));
    assert_eq!(span.content_utf16_end, 15);
    assert_eq!(span.utf16_end, source.encode_utf16().count() as u32);
}

#[test]
fn plus_underline_rejects_literal_runs_and_intraword_pairs() {
    for source in [
        "C++ and C++",
        "a++b++c",
        "+++word+++",
        "++++",
        "++ word++",
        "++word ++",
        "++word",
        "++one\n\ntwo++",
        r"\++word++",
        r"++word\++",
        r"+\+word++",
    ] {
        assert!(
            !kinds(source).contains(&FfiInlineType::UnderlinePlus),
            "{source}"
        );
    }
}

#[test]
fn plus_underline_supports_nested_emphasis_and_escaped_backslashes() {
    for source in [
        "++**word**++",
        "**++word++**",
        r"\\++word++",
        "（++文字++）",
    ] {
        assert!(
            kinds(source).contains(&FfiInlineType::UnderlinePlus),
            "{source}"
        );
    }
    assert!(kinds("++**word**++").contains(&FfiInlineType::Bold));
}

#[test]
fn crossing_underline_and_emphasis_remain_literal() {
    for source in ["++one *two++ three*", "*one ++two* three++"] {
        assert!(kinds(source).is_empty(), "{source}");
    }
}

#[test]
fn opaque_inline_content_does_not_produce_underline() {
    for source in [
        "`++word++`",
        "%%++word++%%",
        "<https://example.com/++word++>",
        "[label](https://example.com/++word++)",
        "$++word++$",
        "<span title='++word++'>text</span>",
    ] {
        assert!(
            !kinds(source).contains(&FfiInlineType::UnderlinePlus),
            "{source}"
        );
    }
}

#[test]
fn inline_math_is_opaque_and_preserves_tex_escapes() {
    let expression = r"\sqrt{\$4} + **x** + ++y++";
    assert_eq!(
        kinds(&format!("${expression}$")),
        vec![FfiInlineType::Math {
            expression: expression.into()
        }]
    );
}

#[test]
fn math_does_not_emit_links_comments_or_code_from_its_expression() {
    for expression in [
        "https://example.com",
        "a %%comment%% b",
        "a `tick` b",
        "[label](https://example.com)",
    ] {
        assert_eq!(
            kinds(&format!("${expression}$")),
            vec![FfiInlineType::Math {
                expression: expression.into()
            }]
        );
    }
}

#[test]
fn inline_math_rejects_currency_and_unsupported_delimiters() {
    for source in [
        "$20,000 and $30,000",
        "$ x$",
        "$x $",
        "$x$2",
        "$x\ny$",
        "$$x$$",
        "$`x`$",
        r"\$x$",
        "$x",
        "$$$",
    ] {
        assert!(
            !kinds(source)
                .iter()
                .any(|kind| matches!(kind, FfiInlineType::Math { .. })),
            "{source}"
        );
    }
}

#[test]
fn inline_math_allows_adjacent_expressions_and_unicode() {
    assert_eq!(
        kinds("$α$ + $β$"),
        vec![
            FfiInlineType::Math {
                expression: "α".into()
            },
            FfiInlineType::Math {
                expression: "β".into()
            }
        ]
    );
}

#[test]
fn link_labels_support_extensions_without_parsing_destinations() {
    let result = kinds("[++label++ $x$](https://example.com/++path++/$y$)");
    assert!(result.contains(&FfiInlineType::UnderlinePlus));
    assert_eq!(
        result
            .iter()
            .filter(|kind| matches!(kind, FfiInlineType::Math { .. }))
            .count(),
        1
    );
    assert!(result.contains(&FfiInlineType::Math {
        expression: "x".into()
    }));
}

#[test]
fn table_cells_have_absolute_ranges_and_semantic_spans() {
    let source = "🍃\n\n| ++Header++ | Formula |\n| --- | --- |\n| text \\| text | $α$ |";
    let result = CindermarkParser::new(None).parse_editable(source.into());
    let table = result
        .blocks
        .iter()
        .find(|block| !block.table_cells.is_empty())
        .unwrap();
    assert_eq!(table.table_cells.len(), 4);
    assert_eq!(table.table_headers.len(), 2);
    assert_eq!(table.table_rows[0].len(), 2);
    assert_eq!(
        table.table_cells[0].inline_spans[0].inline_type,
        FfiInlineType::UnderlinePlus
    );
    let math = &table.table_cells[3].inline_spans[0];
    assert_eq!(
        math.inline_type,
        FfiInlineType::Math {
            expression: "α".into()
        }
    );
    let utf16: Vec<_> = source.encode_utf16().collect();
    assert_eq!(
        String::from_utf16(&utf16[math.utf16_start as usize..math.utf16_end as usize]).unwrap(),
        "$α$"
    );
}

#[test]
fn table_and_inline_ranges_survive_prefix_edits() {
    let original = "first\n\n| ++Title++ |\n| --- |\n| $x$ |\n\nlast";
    let parser = CindermarkParser::new(None);
    parser.parse_editable(original.into());
    let edited = format!("🍃{original}");
    let incremental = parser.parse_editable_incremental_style_only(edited.clone(), 0, 0, 2);
    let full = CindermarkParser::new(None).parse_editable(edited);
    assert_eq!(incremental.blocks, full.blocks);
}

#[test]
fn long_literal_runs_and_large_math_remain_bounded() {
    assert!(!kinds(&"+".repeat(1_000_000)).contains(&FfiInlineType::UnderlinePlus));
    assert!(!kinds(&"$".repeat(1_000_000))
        .iter()
        .any(|kind| matches!(kind, FfiInlineType::Math { .. })));
    let expression = "x+".repeat(500_000);
    assert_eq!(
        kinds(&format!("${expression}$")),
        vec![FfiInlineType::Math { expression }]
    );
}
