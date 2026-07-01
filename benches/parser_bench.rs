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
    let parser = CindermarkParser::new();
    c.bench_function("parse_500_lines", |b| {
        b.iter(|| parser.parse(black_box(note.clone())))
    });
}

fn bench_parse_2500(c: &mut Criterion) {
    let note = generate_note(2500);
    let parser = CindermarkParser::new();
    c.bench_function("parse_2500_lines", |b| {
        b.iter(|| parser.parse(black_box(note.clone())))
    });
}

fn bench_parse_editable_500(c: &mut Criterion) {
    let note = generate_note(500);
    let parser = CindermarkParser::new();
    c.bench_function("parse_editable_500_lines", |b| {
        b.iter(|| parser.parse_editable(black_box(note.clone())))
    });
}

criterion_group!(
    benches,
    bench_parse_500,
    bench_parse_2500,
    bench_parse_editable_500
);
criterion_main!(benches);
