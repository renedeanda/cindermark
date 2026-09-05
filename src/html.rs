//! CommonMark 0.31.2 §4.6 leaf-block boundaries; source remains opaque.

#[derive(Clone, Copy)]
pub(crate) enum HtmlEnd {
    RawText,
    Delimiter(&'static str),
    Blank,
}

pub(crate) struct HtmlStart {
    pub end: HtmlEnd,
    pub interrupts_paragraph: bool,
}

impl HtmlEnd {
    pub fn closes(self, line: &str) -> bool {
        match self {
            Self::RawText => ["</pre>", "</script>", "</style>", "</textarea>"]
                .iter()
                .any(|tag| {
                    line.as_bytes()
                        .windows(tag.len())
                        .any(|part| part.eq_ignore_ascii_case(tag.as_bytes()))
                }),
            Self::Delimiter(end) => line.contains(end),
            Self::Blank => false,
        }
    }
}

pub(crate) fn block_start(line: &str) -> Option<HtmlStart> {
    let text = line.trim_start_matches(' ');
    if line.len() - text.len() > 3 || !text.starts_with('<') {
        return None;
    }
    let bytes = text.as_bytes();
    let end = if ["<pre", "<script", "<style", "<textarea"]
        .iter()
        .any(|tag| {
            bytes
                .get(..tag.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(tag.as_bytes()))
                && bytes
                    .get(tag.len())
                    .is_none_or(|b| matches!(b, b' ' | b'\t' | b'>'))
        }) {
        HtmlEnd::RawText
    } else if text.starts_with("<!--") {
        HtmlEnd::Delimiter("-->")
    } else if text.starts_with("<?") {
        HtmlEnd::Delimiter("?>")
    } else if text.starts_with("<![CDATA[") {
        HtmlEnd::Delimiter("]]>")
    } else if text.starts_with("<!") && bytes.get(2).is_some_and(u8::is_ascii_alphabetic) {
        HtmlEnd::Delimiter(">")
    } else {
        let start = if text.starts_with("</") { 2 } else { 1 };
        let mut i = start;
        while bytes
            .get(i)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'-')
        {
            i += 1;
        }
        let name = &text[start..i];
        const BLOCK_TAGS: &str = "address article aside base basefont blockquote body caption center col colgroup dd details dialog dir div dl dt fieldset figcaption figure footer form frame frameset h1 h2 h3 h4 h5 h6 head header hr html iframe legend li link main menu menuitem nav noframes ol optgroup option p param search section summary table tbody td tfoot th thead title tr track ul";
        if BLOCK_TAGS
            .split(' ')
            .any(|tag| name.eq_ignore_ascii_case(tag))
            && (bytes
                .get(i)
                .is_none_or(|b| matches!(b, b' ' | b'\t' | b'>'))
                || bytes.get(i..i + 2) == Some(b"/>"))
        {
            HtmlEnd::Blank
        } else {
            if start == 1
                && ["pre", "script", "style", "textarea"]
                    .iter()
                    .any(|tag| name.eq_ignore_ascii_case(tag))
            {
                return None;
            }
            if !complete_tag(bytes) {
                return None;
            }
            return Some(HtmlStart {
                end: HtmlEnd::Blank,
                interrupts_paragraph: false,
            });
        }
    };
    Some(HtmlStart {
        end,
        interrupts_paragraph: true,
    })
}

fn complete_tag(bytes: &[u8]) -> bool {
    let closing = bytes.get(1) == Some(&b'/');
    let mut i = if closing { 2 } else { 1 };
    if !bytes.get(i).is_some_and(u8::is_ascii_alphabetic) {
        return false;
    }
    i += 1;
    while bytes
        .get(i)
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'-')
    {
        i += 1;
    }
    loop {
        let before_space = i;
        while bytes.get(i).is_some_and(|b| matches!(b, b' ' | b'\t')) {
            i += 1;
        }
        if !closing && bytes.get(i) == Some(&b'/') {
            i += 1;
            return bytes.get(i) == Some(&b'>')
                && bytes[i + 1..].iter().all(|b| matches!(b, b' ' | b'\t'));
        }
        if bytes.get(i) == Some(&b'>') {
            return bytes[i + 1..].iter().all(|b| matches!(b, b' ' | b'\t'));
        }
        if closing
            || i == before_space
            || !bytes
                .get(i)
                .is_some_and(|b| b.is_ascii_alphabetic() || matches!(b, b'_' | b':'))
        {
            return false;
        }
        i += 1;
        while bytes
            .get(i)
            .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
        {
            i += 1;
        }
        let after_name = i;
        while bytes.get(i).is_some_and(|b| matches!(b, b' ' | b'\t')) {
            i += 1;
        }
        if bytes.get(i) != Some(&b'=') {
            i = after_name;
            continue;
        }
        i += 1;
        while bytes.get(i).is_some_and(|b| matches!(b, b' ' | b'\t')) {
            i += 1;
        }
        if let Some(&quote @ (b'\'' | b'"')) = bytes.get(i) {
            i += 1;
            while bytes.get(i).is_some_and(|b| *b != quote) {
                i += 1;
            }
            if bytes.get(i) != Some(&quote) {
                return false;
            }
            i += 1;
        } else {
            let start = i;
            while bytes.get(i).is_some_and(|b| {
                !matches!(
                    b,
                    b' ' | b'\t' | b'\r' | b'\n' | b'\'' | b'"' | b'=' | b'<' | b'>' | b'`'
                )
            }) {
                i += 1;
            }
            if i == start {
                return false;
            }
        }
    }
}
