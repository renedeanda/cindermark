use cindermark::ast::ParseMode;
use cindermark::parser::parse;

fn main() {
    let cases = [
        ("escape", r"text \*not italic\*"),
        ("hex 6", "color #0E7BFE here"),
        ("hex 3", "short #f0a!"),
        ("callout", "> [!tip] Nice\n> body"),
        ("comment", "visible %%hidden%% more"),
        ("wiki alias", "see [[Target|Display]] here"),
    ];
    for (name, input) in cases {
        let doc = parse(input, ParseMode::Editable);
        println!("--- {} ({:?}) ---", name, input);
        for b in &doc.blocks {
            println!("  block: {:?}", std::mem::discriminant(&b.kind));
            for s in &b.inline_spans {
                println!(
                    "    span: {:?} utf16[{}..{}]",
                    s.kind, s.utf16_start, s.utf16_end
                );
            }
        }
    }
}
