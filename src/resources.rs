use crate::ast::{BlockKind, InlineKind, InlineSpan, ParseMode};
use crate::parser::{parse_with_options, ParseOptions};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceReference {
    pub utf16_start: u32,
    pub utf16_end: u32,
    pub label_utf16_start: u32,
    pub label_utf16_end: u32,
    pub destination: String,
    pub is_image: bool,
}

/// Inline destination references in source order; does not modify parser snapshots.
pub fn resource_references(source: &str, options: &ParseOptions) -> Vec<ResourceReference> {
    let doc = parse_with_options(source, ParseMode::Editable, options);
    let units: Vec<u16> = source.encode_utf16().collect();
    let mut result = Vec::new();
    let mut collect = |span: &InlineSpan| {
        let InlineKind::Link { url } = &span.kind else {
            return;
        };
        let start = span.utf16_start as usize;
        let mut is_image = start > 0 && units.get(start - 1) == Some(&u16::from(b'!'));
        if is_image {
            let mut escapes = 0;
            let mut cursor = start - 1;
            while cursor > 0 && units[cursor - 1] == u16::from(b'\\') {
                escapes += 1;
                cursor -= 1;
            }
            is_image = escapes % 2 == 0;
        }
        result.push(ResourceReference {
            utf16_start: span.utf16_start - u32::from(is_image),
            utf16_end: span.utf16_end,
            label_utf16_start: span.content_utf16_start,
            label_utf16_end: span.content_utf16_end,
            destination: url.clone(),
            is_image,
        });
    };
    for block in &doc.blocks {
        for span in &block.inline_spans {
            collect(span);
        }
        if let BlockKind::BulletList { items } | BlockKind::OrderedList { items, .. } = &block.kind
        {
            for item in items {
                for span in &item.inline_spans {
                    collect(span);
                }
            }
        }
        for cell in &block.table_cells {
            for span in &cell.inline_spans {
                collect(span);
            }
        }
    }
    result.sort_by_key(|reference| reference.utf16_start);
    result.dedup_by_key(|reference| (reference.utf16_start, reference.utf16_end));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_link_and_escaped_bang_ranges() {
        let source =
            "😀 ![photo](Attachments/a%20b.png) [file](Attachments/a.pdf) \\![label](other)";
        let refs = resource_references(source, &ParseOptions::default());
        assert_eq!(refs.len(), 3);
        assert!(refs[0].is_image);
        assert_eq!(refs[0].utf16_start, 3);
        assert_eq!(refs[0].destination, "Attachments/a%20b.png");
        let units: Vec<u16> = source.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(&units[refs[0].utf16_start as usize..refs[0].utf16_end as usize])
                .unwrap(),
            "![photo](Attachments/a%20b.png)"
        );
        assert!(!refs[1].is_image);
        assert!(!refs[2].is_image);
    }

    #[test]
    fn containers_and_opaque_regions() {
        let source = "- [ ] ![task](Attachments/task.png)\n\n| Picture |\n| --- |\n| ![cell](Attachments/cell.png) |\n\n`![code](ignored)`\n\n```\n![fence](ignored)\n```\n\n$![math](ignored)$\n\n<!-- ![html](ignored) -->";
        let refs = resource_references(source, &ParseOptions::default());
        assert_eq!(refs.len(), 2);
        assert!(refs.iter().all(|reference| reference.is_image));
        assert_eq!(refs[1].destination, "Attachments/cell.png");
    }
}
