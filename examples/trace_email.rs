use cindermark::ast::ParseMode;
use cindermark::parser::parse;

fn main() {
    let cases = [
        ("bare email .com", "hello foo@bar.com world"),
        ("bare email .org", "contact rene@emberful.org today"),
        ("bare email .net", "email test@example.net end"),
        ("bare url .com", "visit google.com today"),
        ("bare email no space", "foo@bar.com"),
        ("email in list", "- email rene@embernotes.app"),
    ];
    for (name, input) in cases {
        let doc = parse(input, ParseMode::Editable);
        println!("--- {} ({:?}) ---", name, input);
        for b in &doc.blocks {
            for s in &b.inline_spans {
                println!(
                    "    span: {:?} utf16[{}..{}]",
                    s.kind, s.utf16_start, s.utf16_end
                );
            }
        }
    }
}
