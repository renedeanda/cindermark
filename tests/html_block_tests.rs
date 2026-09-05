use cindermark::{CindermarkParser, FfiBlockType};

#[test]
fn delimited_html_blocks_preserve_source_and_hide_markdown() {
    // Original fixtures for CommonMark §4.6, examples 171–182.
    for (open, close) in [
        ("<ScRiPt>", "</PRE>"),
        ("<!--", "-->"),
        ("<?target", "?>"),
        ("<!DOC", ">"),
        ("<![CDATA[", "]]>"),
    ] {
        let raw =
            format!("   {open}\r\n\r\n$$x$$\r\n++hidden++ [[Hidden]]\r\n{close} $also_hidden$\r\n");
        let source = format!("{raw}$shown$");
        for grouped in [false, true] {
            let parser = CindermarkParser::new(None);
            let doc = if grouped {
                parser.parse(source.clone())
            } else {
                parser.parse_editable(source.clone())
            };
            assert_eq!(doc.blocks[0].block_type, FfiBlockType::RawHtml, "{source}");
            assert_eq!(doc.blocks[0].text, raw);
            assert_eq!(doc.blocks[0].utf16_end, raw.encode_utf16().count() as u32);
            assert!(doc.blocks[0].inline_spans.is_empty());
            assert!(doc.wiki_links.is_empty());
            assert_eq!(doc.blocks.len(), 2);
            assert_eq!(doc.blocks[1].inline_spans.len(), 1);
        }
    }
}

#[test]
fn blank_terminated_html_does_not_change_termination_on_nested_tags() {
    for open in [
        "<div>",
        "</TABLE>",
        "<custom-tag _flag data-x='🍃' />",
        "</custom-tag>",
    ] {
        let source = format!("{open}\n<pre>\n$$x$$ ++hidden++\n\n$shown$");
        let doc = CindermarkParser::new(None).parse_editable(source);
        assert_eq!(doc.blocks[0].block_type, FfiBlockType::RawHtml);
        assert!(doc.blocks[0].inline_spans.is_empty());
        assert_eq!(doc.blocks.last().unwrap().inline_spans.len(), 1);
    }
}

#[test]
fn only_types_one_through_six_interrupt_paragraphs() {
    let parser = CindermarkParser::new(None);
    let raw = parser.parse_editable("before\n<script>\n$x$\n</script>".into());
    assert_eq!(raw.blocks[1].block_type, FfiBlockType::RawHtml);
    let inline = parser.parse_editable("before\n<custom-tag>\n$x$\n</custom-tag>".into());
    assert_eq!(inline.blocks.len(), 1);
    assert_eq!(inline.blocks[0].block_type, FfiBlockType::Paragraph);
    assert_eq!(inline.blocks[0].inline_spans.len(), 1);
}

#[test]
fn malformed_tags_and_indented_code_do_not_start_html_blocks() {
    for source in [
        "<3tag>",
        "<custom a='bad'next=x>",
        "<custom /bad>",
        "<custom / >",
        "<custom a=>",
        "</custom attr>",
        "<script/>",
        "    <script>\n    $x$",
    ] {
        let doc = CindermarkParser::new(None).parse_editable(source.into());
        assert!(
            doc.blocks
                .iter()
                .all(|block| block.block_type != FfiBlockType::RawHtml),
            "{source}"
        );
    }
}

#[test]
fn html_open_and_close_edits_match_full_parse() {
    let source = "🍃\n\nscript>\n$$x$$\n\n++hidden++\n</script>\n\n$shown$";
    let parser = CindermarkParser::new(None);
    parser.parse_editable(source.into());
    let offset = "🍃\n\n".encode_utf16().count() as u32;
    let edited = source.replacen("script>", "<script>", 1);
    let inc = parser.parse_editable_incremental_style_only(edited.clone(), offset, 0, 1);
    let full = CindermarkParser::new(None).parse_editable(edited.clone());
    assert_eq!(inc.blocks, full.blocks);
    let close_byte = edited.find("</script>").unwrap();
    let close_offset = edited[..close_byte].encode_utf16().count() as u32;
    let deleted = edited.replace("</script>", "");
    let inc = parser.parse_editable_incremental_style_only(deleted.clone(), close_offset, 9, 0);
    assert_eq!(
        inc.blocks,
        CindermarkParser::new(None).parse_editable(deleted).blocks
    );
}

#[test]
fn html_previews_keep_raw_source_without_interpreting_extensions() {
    let source = "<script>\n$$x$$\n++hidden++ **literal** [[Hidden]]\n</script>";
    let parser = CindermarkParser::new(None);
    let preview = parser.render_preview(source.into(), 500);
    assert_eq!(preview.plain_text, source);
    assert!(preview.spans.is_empty());
    assert!(parser.extract_wiki_links(source.into()).is_empty());
}

#[test]
fn same_line_html_terminator_keeps_the_entire_line_opaque() {
    let source = "<script> 🍃 $$x$$ </script> ++hidden++\n$shown$";
    let doc = CindermarkParser::new(None).parse_editable(source.into());
    assert_eq!(doc.blocks.len(), 2);
    assert_eq!(doc.blocks[0].block_type, FfiBlockType::RawHtml);
    assert_eq!(
        doc.blocks[0].text,
        "<script> 🍃 $$x$$ </script> ++hidden++\n"
    );
    assert!(doc.blocks[0].inline_spans.is_empty());
    assert_eq!(doc.blocks[1].inline_spans.len(), 1);
}

#[test]
fn container_html_is_opaque_and_stops_at_its_boundary() {
    for raw in [
        "> <script>\n> $$x$$\n> ++hidden++ [[Hidden]]\n",
        "> > <!--\n> > $$x$$\n> > ++hidden++\n",
        "- <script>\n  $$x$$\n  ++hidden++ [[Hidden]]\n",
        "003) <script>\n     $$x$$\n     ++hidden++\n",
        "- [x] <script>\n  $$x$$\n  ++hidden++\n",
        "- > <script>\n  > $$x$$\n  > ++hidden++\n",
        "> - <script>\n>   $$x$$\n>   ++hidden++\n",
        "> - > <script>\n>   > $$x$$\n>   > ++hidden++\n",
        "-\t<script>\r\n\t$$x$$\r\n\t++hidden++\r\n",
    ] {
        let source = format!("{raw}outside $shown$");
        for grouped in [false, true] {
            let parser = CindermarkParser::new(None);
            let doc = if grouped {
                parser.parse(source.clone())
            } else {
                parser.parse_editable(source.clone())
            };
            assert_eq!(doc.blocks[0].block_type, FfiBlockType::RawHtml, "{source}");
            assert_eq!(doc.blocks[0].text, raw);
            assert!(doc.blocks[0].inline_spans.is_empty());
            assert!(doc.wiki_links.is_empty());
            assert_eq!(doc.blocks.len(), 2, "{source}");
            assert_eq!(doc.blocks[1].inline_spans.len(), 1);
        }
    }
}

#[test]
fn container_html_interrupts_existing_content_without_swallowing_siblings() {
    for source in [
        "- first\n- <script>\n  $$hidden$$\n- sibling $shown$",
        "1. first\n2. <script>\n   $$hidden$$\n3. sibling $shown$",
        "- first\n  <script>\n  $$hidden$$\n- sibling $shown$",
        "> first\n> <script>\n> $$hidden$$\noutside $shown$",
        "- first\n  > <script>\n  > $$hidden$$\n- sibling $shown$",
    ] {
        let parser = CindermarkParser::new(None);
        for doc in [
            parser.parse_editable(source.into()),
            parser.parse(source.into()),
        ] {
            let raw = doc
                .blocks
                .iter()
                .find(|block| block.block_type == FfiBlockType::RawHtml)
                .expect(source);
            assert!(raw.text.contains("$$hidden$$"));
            assert!(!raw.text.contains("shown"));
            assert!(raw.inline_spans.is_empty());
        }
    }
}

#[test]
fn type_seven_does_not_interrupt_container_paragraphs() {
    for source in [
        "> before\n> <custom-tag>\n> $shown$",
        "- before\n  <custom-tag>\n  $shown$",
    ] {
        let parser = CindermarkParser::new(None);
        for doc in [
            parser.parse(source.into()),
            parser.parse_editable(source.into()),
        ] {
            assert!(
                doc.blocks
                    .iter()
                    .all(|block| block.block_type != FfiBlockType::RawHtml),
                "{source}"
            );
        }
    }
    let doc = CindermarkParser::new(None)
        .parse("> before\n>\n> <custom-tag>\n> $$hidden$$\n\noutside".into());
    assert!(doc
        .blocks
        .iter()
        .any(|block| block.block_type == FfiBlockType::RawHtml));
}

#[test]
fn container_prefix_edits_recompute_distant_html_boundaries() {
    let source = "- parent\n\n  <script>\n  $$hidden$$\n  </script>\n\n$shown$";
    let parser = CindermarkParser::new(None);
    parser.parse_editable(source.into());
    let edited = source.replacen("- parent", "parent", 1);
    let incremental = parser.parse_editable_incremental_style_only(edited.clone(), 0, 2, 0);
    assert_eq!(
        incremental.blocks,
        CindermarkParser::new(None).parse_editable(edited).blocks
    );
}

#[test]
fn editable_custom_tag_continuation_retains_paragraph_context_after_edits() {
    let source = "- before\n  <custom-tag>\n  ordinary body\n\noutside";
    let parser = CindermarkParser::new(None);
    parser.parse_editable(source.into());
    let offset = source.find("body").unwrap();
    let edited = source.replace("body", "bodies");
    let incremental =
        parser.parse_editable_incremental_style_only(edited.clone(), offset as u32, 4, 6);
    assert_eq!(
        incremental.blocks,
        CindermarkParser::new(None).parse_editable(edited).blocks
    );
}

#[test]
fn quoted_html_retains_list_marker_ranges_and_parent_quote_boundary() {
    let source = "> 003) <script>\r\n>      $$hidden$$\r\n> sibling $shown$";
    let doc = CindermarkParser::new(None).parse_editable(source.into());
    let raw = &doc.blocks[0];
    assert_eq!(raw.block_type, FfiBlockType::RawHtml);
    assert_eq!(raw.marker_source, "003) ");
    assert_eq!(raw.ordered_raw_number, "003");
    assert_eq!((raw.marker_utf16_start, raw.marker_utf16_end), (2, 7));
    assert!(!raw.text.contains("sibling"));
    assert_eq!(doc.blocks[1].inline_spans.len(), 1);
}
