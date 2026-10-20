//! Plain text import / export.

use crate::io::util::cp1252_decode;
use crate::model::*;

/// Decode text bytes: UTF-8 (with or without BOM), UTF-16 LE/BE with BOM, else Windows-1252.
pub fn decode(b: &[u8]) -> String {
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF][..]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if b.len() >= 2 && (b[0] == 0xFF && b[1] == 0xFE || b[0] == 0xFE && b[1] == 0xFF) {
        let le = b[0] == 0xFF;
        let units: Vec<u16> = b[2..]
            .chunks_exact(2)
            .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
            .collect();
        return String::from_utf16_lossy(&units);
    }
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => cp1252_decode(b),
    }
}

/// Read plain text: one paragraph per line, tabs preserved, form feeds become page breaks.
pub fn read(bytes: &[u8]) -> Document {
    read_str(&decode(bytes))
}

pub fn read_str(text: &str) -> Document {
    let mut doc = Document::new();
    // A final line terminator does not start another paragraph.
    let text = text.strip_suffix("\r\n").or_else(|| text.strip_suffix('\n')).or_else(|| text.strip_suffix('\r')).unwrap_or(text);
    doc.body = text_to_blocks(text);
    doc
}

/// Convert text to paragraphs (used for plain-text paste too).
pub fn text_to_blocks(text: &str) -> Vec<Block> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::new();
    for line in text.split('\n') {
        out.push(Block::Para(line_to_para(line)));
    }
    if out.is_empty() {
        out.push(Block::Para(Paragraph::new()));
    }
    out
}

fn line_to_para(line: &str) -> Paragraph {
    let mut p = Paragraph::new();
    let mut cur = String::new();
    let flush = |p: &mut Paragraph, cur: &mut String| {
        if !cur.is_empty() {
            p.runs.push(Run::text(std::mem::take(cur)));
        }
    };
    for c in line.chars() {
        let inline = match c {
            '\t' => Some(Inline::Tab),
            '\u{000C}' => Some(Inline::PageBreak),
            '\u{000B}' | '\u{2028}' => Some(Inline::LineBreak),
            '\u{FEFF}' => {
                continue;
            }
            c if (c as u32) < 0x20 => {
                continue;
            }
            _ => None,
        };
        match inline {
            Some(i) => {
                flush(&mut p, &mut cur);
                p.runs.push(Run { content: i, props: CharProps::default() });
            }
            None => cur.push(c),
        }
    }
    flush(&mut p, &mut cur);
    p
}

fn para_text(p: &Paragraph, out: &mut String) {
    for r in &p.runs {
        if r.props.revision.as_ref().map(|v| v.kind == RevisionKind::Delete).unwrap_or(false) {
            continue;
        }
        match &r.content {
            Inline::Text(t) => out.push_str(t),
            Inline::Tab => out.push('\t'),
            Inline::LineBreak => out.push('\n'),
            Inline::PageBreak => out.push('\u{000C}'),
            Inline::ColumnBreak => out.push('\n'),
            Inline::Field(f) => match f {
                Field::PageNumber => out.push('1'),
                Field::PageCount => out.push('1'),
                Field::Date(pic) => out.push_str(&crate::io::util::format_date_now(pic)),
                Field::Other { result, .. } => out.push_str(result),
            },
            Inline::Image(_) | Inline::TextBox(_) | Inline::Bookmark(_) | Inline::FootnoteRef(_) | Inline::EndnoteRef(_) => {}
        }
    }
}

/// Plain text of blocks: paragraphs separated by '\n', table cells by '\t'.
pub fn blocks_to_text(blocks: &[Block]) -> String {
    let mut lines = Vec::new();
    for b in blocks {
        match b {
            Block::Para(p) => {
                let mut s = String::new();
                para_text(p, &mut s);
                lines.push(s);
                // Text box contents follow their anchor paragraph.
                for r in &p.runs {
                    if let Inline::TextBox(tb) = &r.content {
                        for q in &tb.paragraphs {
                            let mut s = String::new();
                            para_text(q, &mut s);
                            lines.push(s);
                        }
                    }
                }
            }
            Block::Table(t) => {
                for r in &t.rows {
                    let cells: Vec<String> = r
                        .cells
                        .iter()
                        .map(|c| {
                            let parts: Vec<String> = c
                                .paragraphs
                                .iter()
                                .map(|p| {
                                    let mut s = String::new();
                                    para_text(p, &mut s);
                                    s
                                })
                                .collect();
                            parts.join(" ")
                        })
                        .collect();
                    lines.push(cells.join("\t"));
                }
            }
        }
    }
    lines.join("\n")
}

/// Write plain text (UTF-8, '\n' line endings, trailing newline).
pub fn write(doc: &Document) -> String {
    let mut s = blocks_to_text(&doc.body);
    s.push('\n');
    s
}
