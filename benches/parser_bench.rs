use cindermark::CindermarkParser;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn generate_note(lines: usize) -> String {
    let mut note = String::new();
    note.push_str("# Sample Note\n\n");
    for i in 0..lines {
        match i % 10 {
            0 => note.push_str(&format!("## Section {}\n\n", i / 10 + 1)),
            1 => note.push_str("Some **bold** and *italic* text with a [[wiki link]].\n"),
            2 => note.push_str("- Bullet item one\n- Bullet item two\n- Bullet item three\n"),
            3 => note.push_str("1. First ordered\n2. Second ordered\n"),
            4 => note.push_str("- [ ] Unchecked task\n- [x] Completed task\n"),
            5 => note.push_str("> A blockquote with some ==highlighted== text.\n"),
            6 => note.push_str("```swift\nlet x = 42\nprint(x)\n```\n"),
            7 => note.push_str("| Header A | Header B |\n| --- | --- |\n| Cell 1 | Cell 2 |\n"),
            8 => note.push_str("---\n"),
            9 => note.push_str("Regular paragraph with ~~strikethrough~~ and `inline code`.\n\n"),
            _ => {}
        }
    }
    note
}

fn bench_parse_500(c: &mut Criterion) {
    let note = generate_note(500);
    let parser = CindermarkParser::new(None);
    c.bench_function("parse_500_lines", |b| {
        b.iter(|| parser.parse(black_box(note.clone())))
    });
}

fn bench_parse_2500(c: &mut Criterion) {
    let note = generate_note(2500);
    let parser = CindermarkParser::new(None);
    c.bench_function("parse_2500_lines", |b| {
        b.iter(|| parser.parse(black_box(note.clone())))
    });
}

fn bench_parse_editable_500(c: &mut Criterion) {
    let note = generate_note(500);
    let parser = CindermarkParser::new(None);
    c.bench_function("parse_editable_500_lines", |b| {
        b.iter(|| parser.parse_editable(black_box(note.clone())))
    });
}

/// Steady-state incremental keystroke benchmark.
///
/// Seeds the parser snapshot with a full editable parse, then alternates
/// inserting and deleting one character mid-document. Each call's text is
/// consistent with the previous snapshot plus the claimed edit, so the
/// parser stays on the incremental path (no re-seeding needed) and the
/// measurement reflects what an editor pays per debounced keystroke.
/// The text clone inside the loop is deliberate: the real FFI call passes
/// an owned String across the boundary too.
fn bench_incremental_keystroke(c: &mut Criterion, name: &str, lines: usize, with_stats: bool) {
    let base = generate_note(lines);
    // Edit inside a plain paragraph in the middle of the note — the common
    // case, which must not fall back to a full re-parse.
    let mid = base.len() / 2;
    let pos = mid
        + base[mid..]
            .find("Regular paragraph")
            .expect("generated note contains paragraph lines")
        + "Regular ".len();
    let mut edited = base.clone();
    edited.insert(pos, 'x');

    let parser = CindermarkParser::new(None);
    parser.parse_editable(base.clone());

    // The generated note is pure ASCII, so byte offset == UTF-16 offset.
    let utf16_pos = pos as u32;
    let mut inserted = false;
    c.bench_function(name, |b| {
        b.iter(|| {
            let (text, old_len, new_len) = if inserted {
                (base.clone(), 1, 0)
            } else {
                (edited.clone(), 0, 1)
            };
            inserted = !inserted;
            if with_stats {
                black_box(parser.parse_editable_incremental(
                    black_box(text),
                    utf16_pos,
                    old_len,
                    new_len,
                ));
            } else {
                black_box(parser.parse_editable_incremental_style_only(
                    black_box(text),
                    utf16_pos,
                    old_len,
                    new_len,
                ));
            }
        })
    });
}

fn bench_incremental_500(c: &mut Criterion) {
    bench_incremental_keystroke(c, "incremental_keystroke_500", 500, false);
}

fn bench_incremental_2500(c: &mut Criterion) {
    bench_incremental_keystroke(c, "incremental_keystroke_2500", 2500, false);
}

fn bench_incremental_10k(c: &mut Criterion) {
    bench_incremental_keystroke(c, "incremental_keystroke_10k", 10_000, false);
}

fn bench_incremental_with_stats_2500(c: &mut Criterion) {
    bench_incremental_keystroke(c, "incremental_with_stats_2500", 2500, true);
}

fn bench_extended_syntax(c: &mut Criterion) {
    for (name, source) in [
        (
            "math_1000_lines",
            "$\\frac{α}{β} + x^2$ and ++text++\n\n".repeat(1000),
        ),
        ("unmatched_delimiters_1mb", "\\++ $ +++ ".repeat(100_000)),
        (
            "raw_html_1mb",
            format!(
                "<script>\n{}\n</script>",
                "$$x$$ ++hidden++ ".repeat(65_000)
            ),
        ),
        (
            "unclosed_inline_html_1mb",
            format!(
                "prefix {} ++shown++ $y$",
                "<!--<?<!DOCUMENT<![CDATA[".repeat(40_000)
            ),
        ),
    ] {
        let parser = CindermarkParser::new(None);
        c.bench_function(name, |b| {
            b.iter(|| parser.parse_editable(black_box(source.clone())))
        });
    }
}

fn bench_container_math(c: &mut Criterion) {
    let source = "- > $$\n  > \\frac{α}{β}\n  > $$\n\n".repeat(1000);
    let parser = CindermarkParser::new(None);
    let parsed = parser.parse_editable(source.clone());
    assert_eq!(
        parsed
            .blocks
            .iter()
            .filter(|block| matches!(block.block_type, cindermark::FfiBlockType::Math { .. }))
            .count(),
        1000
    );
    c.bench_function("quoted_list_math_1000", |b| {
        b.iter(|| parser.parse_editable(black_box(source.clone())))
    });
}

fn bench_container_html(c: &mut Criterion) {
    let source = "- > <script>\n  > $$x$$ ++hidden++\n  > </script>\n\n".repeat(1000);
    let parser = CindermarkParser::new(None);
    let parsed = parser.parse_editable(source.clone());
    assert_eq!(
        parsed
            .blocks
            .iter()
            .filter(|block| matches!(block.block_type, cindermark::FfiBlockType::RawHtml))
            .count(),
        1000
    );
    c.bench_function("quoted_list_html_1000", |b| {
        b.iter(|| parser.parse_editable(black_box(source.clone())))
    });
}

fn bench_compound_containers(c: &mut Criterion) {
    for (name, unit, math) in [
        (
            "alternating_list_math_1000",
            "- > - > $$\n  >   > \\frac{α}{β}\n  >   > $$\n\n",
            true,
        ),
        (
            "alternating_list_html_1000",
            "- > - > <!--\n  >   > $$x$$ ++hidden++\n  >   > -->\n\n",
            false,
        ),
    ] {
        let source = unit.repeat(1000);
        let parser = CindermarkParser::new(None);
        let parsed = parser.parse_editable(source.clone());
        assert_eq!(
            parsed
                .blocks
                .iter()
                .filter(|block| {
                    if math {
                        matches!(block.block_type, cindermark::FfiBlockType::Math { .. })
                    } else {
                        matches!(block.block_type, cindermark::FfiBlockType::RawHtml)
                    }
                })
                .count(),
            1000
        );
        c.bench_function(name, |b| {
            b.iter(|| parser.parse_editable(black_box(source.clone())))
        });
    }
}

criterion_group!(
    benches,
    bench_parse_500,
    bench_parse_2500,
    bench_parse_editable_500,
    bench_incremental_500,
    bench_incremental_2500,
    bench_incremental_10k,
    bench_incremental_with_stats_2500,
    bench_extended_syntax,
    bench_container_math,
    bench_container_html,
    bench_compound_containers,
    bench_list_ranges,
    bench_resource_references
);
criterion_main!(benches);

fn bench_list_ranges(c: &mut Criterion) {
    for count in [500, 2500] {
        let source = (0..count)
            .map(|i| format!("- [ ] Task {i}\n  - [x] Child\n  continuation\n"))
            .collect::<String>();
        c.bench_function(&format!("list_ranges_{count}"), |b| {
            b.iter(|| cindermark::parser::list_item_ranges(black_box(&source), &Default::default()))
        });
    }
}

fn bench_resource_references(c: &mut Criterion) {
    for count in [500, 2500] {
        let source = (0..count)
            .map(|i| format!("- ![Photo {i}](Attachments/photo-{i}.png)\r\n"))
            .collect::<String>();
        c.bench_function(&format!("resource_references_{count}"), |b| {
            b.iter(|| {
                cindermark::resources::resource_references(black_box(&source), &Default::default())
            })
        });
    }
}
