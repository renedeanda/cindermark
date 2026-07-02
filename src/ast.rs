//! AST node types for the Cindermark Markdown parser.
//!
//! Designed for cache-friendly traversal: `InlineSpan` is 28 bytes (fits 2 per
//! 64-byte L1 cache line), `BlockNode` fields are accessed sequentially during
//! style application. All string data is borrowed from the source text.

/// A parsed markdown document.
#[derive(Debug, Clone)]
pub struct Document {
    pub blocks: Vec<BlockNode>,
    pub line_count: u32,
}

/// A single block-level node in the AST.
#[derive(Debug, Clone)]
pub struct BlockNode {
    pub kind: BlockKind,
    /// First line index (0-based).
    pub line_start: u32,
    /// Last line index (exclusive).
    pub line_end: u32,
    /// UTF-16 offset of the block start in the document.
    pub utf16_start: u32,
    /// UTF-16 offset of the block end (exclusive).
    pub utf16_end: u32,
    /// Byte range in source (for incremental diffing).
    pub byte_start: u32,
    pub byte_end: u32,
    /// Source metadata for list-like blocks. Present for editable list items,
    /// checkbox items, and grouped list blocks where the first marker defines
    /// list identity and ordered-list start behavior.
    pub list_marker: Option<ListMarkerMeta>,
    /// Inline formatting spans within this block.
    pub inline_spans: Vec<InlineSpan>,
}

/// Source metadata for a CommonMark list marker or checklist marker extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListMarkerMeta {
    /// Leading whitespace characters (spaces/tabs) before the marker. Nested
    /// list markers may be indented up to 32 tab-expanded columns; hosts
    /// receive this value as the text length of the indentation run, so it
    /// counts characters, not expanded columns.
    pub indent: u32,
    /// UTF-16 range of the marker prefix, excluding indentation and including
    /// the following marker whitespace. For checklists this also includes
    /// `[ ]` / `[x]` and its following whitespace when present.
    pub marker_utf16_start: u32,
    pub marker_utf16_end: u32,
    /// Byte positions matching the marker/content boundaries. These are internal
    /// parser conveniences; Swift receives the UTF-16 range through FFI.
    pub marker_byte_start: u32,
    pub marker_byte_end: u32,
    pub content_byte_start: u32,
    /// Exact markdown marker prefix excluding indentation, e.g. `* `, `003)  `,
    /// `+ [x] `. Extraction uses this to avoid source normalization.
    pub marker_source: String,
    /// Bullet/checklist source marker (`-`, `*`, `+`) when applicable.
    pub unordered_marker: String,
    /// Ordered delimiter (`.` or `)`) when applicable.
    pub ordered_delimiter: String,
    /// Ordered number exactly as typed, preserving leading zeroes.
    pub ordered_raw_number: String,
}

/// Block-level node kind with associated metadata.
#[derive(Debug, Clone, PartialEq)]
pub enum BlockKind {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph {
        text: String,
    },
    CodeBlock {
        language: Option<String>,
        code: String,
    },
    /// Mermaid diagram block: a fenced code block whose info string is
    /// `mermaid` (case-insensitive). `diagram_type` is classified from the
    /// first non-blank content line so Swift can pick a themed chip label
    /// ("Flowchart", "Sequence Diagram", …) without re-parsing the source.
    /// `source` is the raw diagram source (fences stripped) and round-trips
    /// verbatim.
    MermaidDiagram {
        diagram_type: MermaidDiagramType,
        source: String,
    },
    Blockquote {
        text: String,
    },
    BulletList {
        items: Vec<ListItem>,
    },
    OrderedList {
        start: u32,
        items: Vec<ListItem>,
    },
    Checkbox {
        checked: bool,
        text: String,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
        alignments: Vec<ColumnAlignment>,
    },
    HorizontalRule,
    Empty,
    FootnoteDefinition {
        label: String,
        text: String,
    },
    ImageMarker {
        uuid: String,
    },
    /// Callout: a blockquote whose first line is `[!<kind>]`
    /// followed by an optional custom title. Body text comes from subsequent
    /// `> ...` continuation lines.
    Callout {
        kind: CalloutKind,
        title: Option<String>,
        text: String,
    },
    // Editable-mode-only variants (individual list items)
    BulletItem {
        text: String,
    },
    NumberedItem {
        number: u32,
        text: String,
    },
}

/// Mermaid diagram type, classified from the first non-blank content line of
/// a ```mermaid fenced block. Matching is case-insensitive and tolerant of
/// Mermaid's YAML frontmatter (`---\n…\n---\n`) and trailing direction
/// keywords (`flowchart TD`, `graph LR`). Unknown or unclassified diagrams
/// still parse and render — they just show a generic "Diagram" chip label.
///
/// Discriminant values are stable wire format: Swift reads them as `u8`
/// via FFI. New variants append at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MermaidDiagramType {
    Unknown = 0,
    Flowchart = 1,
    /// Legacy alias for flowchart; Mermaid still supports it.
    Graph = 2,
    SequenceDiagram = 3,
    ClassDiagram = 4,
    StateDiagram = 5,
    ErDiagram = 6,
    Gantt = 7,
    Mindmap = 8,
    Pie = 9,
    /// `journey` — user journey.
    Journey = 10,
    Timeline = 11,
    GitGraph = 12,
    /// Any of the C4 variants (Context, Container, Component, Dynamic, Deployment).
    C4 = 13,
    RequirementDiagram = 14,
    /// `quadrantChart`.
    Quadrant = 15,
    /// `sankey` / `sankey-beta`.
    Sankey = 16,
    /// `xychart` / `xychart-beta`.
    XYChart = 17,
    /// `block` / `block-beta`.
    Block = 18,
    /// `packet` / `packet-beta`.
    Packet = 19,
    /// `architecture` / `architecture-beta`.
    Architecture = 20,
    Kanban = 21,
    /// `radar` / `radar-beta`.
    Radar = 22,
}

impl MermaidDiagramType {
    /// Classify the diagram type from the raw source inside the fence.
    /// Skips an optional YAML frontmatter block (`---\n…\n---\n`) and then
    /// matches the first whitespace-separated token case-insensitively.
    pub fn from_source(source: &str) -> Self {
        let mut lines = source.lines().peekable();

        // Skip a leading YAML frontmatter block if present. Mermaid 10+
        // supports `---\nconfig:…\n---\n<diagram>` for per-diagram config.
        loop {
            match lines.peek().copied() {
                Some(line) if line.trim().is_empty() => {
                    lines.next();
                }
                Some(line) if line.trim() == "---" => {
                    lines.next();
                    for l in lines.by_ref() {
                        if l.trim() == "---" {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }

        let first_line = lines
            .find(|l| !l.trim().is_empty())
            .map(str::trim)
            .unwrap_or("");

        let keyword = first_line.split_whitespace().next().unwrap_or("");
        let k = keyword.to_ascii_lowercase();
        match k.as_str() {
            "flowchart" => Self::Flowchart,
            "graph" => Self::Graph,
            "sequencediagram" => Self::SequenceDiagram,
            "classdiagram" | "classdiagram-v2" => Self::ClassDiagram,
            "statediagram" | "statediagram-v2" => Self::StateDiagram,
            "erdiagram" => Self::ErDiagram,
            "gantt" => Self::Gantt,
            "mindmap" => Self::Mindmap,
            "pie" => Self::Pie,
            "journey" => Self::Journey,
            "timeline" => Self::Timeline,
            "gitgraph" => Self::GitGraph,
            "c4context" | "c4container" | "c4component" | "c4dynamic" | "c4deployment" => Self::C4,
            "requirementdiagram" => Self::RequirementDiagram,
            "quadrantchart" => Self::Quadrant,
            "sankey" | "sankey-beta" => Self::Sankey,
            "xychart" | "xychart-beta" => Self::XYChart,
            "block" | "block-beta" => Self::Block,
            "packet" | "packet-beta" => Self::Packet,
            "architecture" | "architecture-beta" => Self::Architecture,
            "kanban" => Self::Kanban,
            "radar" | "radar-beta" => Self::Radar,
            _ => Self::Unknown,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Case-insensitive check for the `mermaid` info string on a fenced code
/// block. Mermaid itself is case-sensitive in practice, but users copy-paste
/// from all over, so we're forgiving here.
pub fn is_mermaid_info_string(info: &str) -> bool {
    info.eq_ignore_ascii_case("mermaid")
}

/// Callout kind (`[!note]`, `[!tip]`, …). The five most common variants; additional
/// aliases can be added post-launch without an ABI break (just map more
/// strings to the same enum values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalloutKind {
    Note = 0,
    Tip = 1,
    Warning = 2,
    Important = 3,
    Caution = 4,
}

impl CalloutKind {
    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "note" => Some(Self::Note),
            "tip" => Some(Self::Tip),
            "warning" => Some(Self::Warning),
            "important" => Some(Self::Important),
            "caution" => Some(Self::Caution),
            _ => None,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// A single list item (used inside BulletList/OrderedList).
#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    pub text: String,
    /// Inline spans within this list item (offsets relative to document).
    pub inline_spans: Vec<InlineSpan>,
}

/// An inline formatting span — 28 bytes for cache-friendly traversal.
#[derive(Debug, Clone, PartialEq)]
pub struct InlineSpan {
    pub kind: InlineKind,
    /// UTF-16 offset of the full span (including markers).
    pub utf16_start: u32,
    /// UTF-16 end offset (exclusive).
    pub utf16_end: u32,
    /// UTF-16 offset of inner content (after opening marker).
    pub content_utf16_start: u32,
    /// UTF-16 end of inner content (before closing marker).
    pub content_utf16_end: u32,
}

/// Inline formatting kind.
#[derive(Debug, Clone, PartialEq)]
pub enum InlineKind {
    Bold,
    Italic,
    BoldItalic,
    Strikethrough,
    UnderlineTilde,
    UnderlineHtml,
    InlineCode,
    Highlight,
    /// Colored highlight with color index:
    /// 0=red (🔴), 1=orange (🟠), 2=yellow (🟡), 3=green (🟢), 4=blue (🔵), 5=purple (🟣)
    HighlightColor(u8),
    /// Highlight backed by an arbitrary hex color: `=={#RRGGBB}text==`.
    /// `hex` is normalized to 6 lowercase hex digits (alpha dropped; Swift
    /// derives the pill tint from the RGB and can re-read the source for alpha).
    HighlightHex {
        hex: String,
    },
    Link {
        url: String,
    },
    /// Bare URL detected in text (e.g., `https://example.com`).
    /// No markdown syntax — the URL itself is the display text.
    AutoLink {
        url: String,
    },
    WikiLink,
    FootnoteRef,
    /// `%%hidden%%` drafting comment. Rendered faded in the editor and
    /// stripped from preview output entirely.
    Comment,
    /// `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA` inline color literal. `hex`
    /// is normalized to 6 lowercase hex digits (alpha dropped for rendering;
    /// Swift re-reads the source to recover the alpha channel if needed).
    HexColor {
        hex: String,
    },
}

/// Column alignment for markdown tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnAlignment {
    Default,
    Left,
    Center,
    Right,
}

/// Parse mode controls whether list items are grouped or individual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseMode {
    /// Groups consecutive list items into BulletList/OrderedList (for rendering).
    Grouped,
    /// Each list item is its own block (for the block editor).
    Editable,
}
