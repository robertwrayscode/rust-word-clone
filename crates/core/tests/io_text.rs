mod common;

use common::*;
use wordcore::io::{self, markdown, txt};
use wordcore::*;

#[test]
fn txt_decoding() {
    assert_eq!(txt::decode(b"\xEF\xBB\xBFhello"), "hello");
    assert_eq!(txt::decode(&[0xFF, 0xFE, b'h', 0, 0xE9, 0]), "hé");
    assert_eq!(txt::decode(&[0xFE, 0xFF, 0, b'h', 0, 0xE9]), "hé");
    assert_eq!(txt::decode(b"caf\xE9 \x93q\x94"), "café “q”");
    assert_eq!(txt::decode("naïve".as_bytes()), "naïve");
    let d = txt::read(b"one\r\ntwo\n\tthree\n");
    let paras = body_paras(&d);
    assert_eq!(paras.len(), 3);
    assert_eq!(paras[2].text(), "\tthree");
    let out = txt::write(&kitchen_sink());
    assert!(out.contains("Fancy paragraph"));
    assert!(!out.contains("deleted"));
    assert!(!out.contains('\r'));
}

#[test]
fn markdown_export() {
    let md = markdown::write(&kitchen_sink());
    assert!(md.contains("# The Title\n"), "{md}");
    assert!(md.contains("# Chapter One\n"));
    assert!(md.contains("## Section 1.1\n"));
    assert!(md.contains("**bold**"));
    assert!(md.contains("*italic*"));
    assert!(md.contains("~~struck dstruck~~"));
    assert!(md.contains("E=mc<sup>2</sup>") && md.contains("<u>dunder</u>"), "{md}");
    assert!(md.contains("- Bullet one\n    - Bullet two\n"), "{md}");
    assert!(md.contains("1. Number one\n2. Number two\n"));
    assert!(md.contains("[example](https://example.com/a?b=1&c=2)"));
    assert!(md.contains("](data:image/png;base64,"));
    assert!(md.contains("| H1 | H2 | H3 |\n| --- | --- | --- |\n"), "{md}");
    assert!(md.contains("[^1]: The footnote text."), "{md}");
    assert!(md.contains("Ünïcödé — “quotes” & \\<angle> 日本語 😀"));
}

#[test]
fn markdown_roundtrip() {
    let back = markdown::read(&markdown::write(&kitchen_sink()));
    assert_eq!(find_para(&back, "The Title").props.style.as_deref(), Some("Heading1"));
    assert_eq!(find_para(&back, "Section 1.1").props.style.as_deref(), Some("Heading2"));
    assert_eq!(find_run(&back, "bold").props.bold, Some(true));
    assert_eq!(find_run(&back, "italic").props.italic, Some(true));
    assert_eq!(find_run(&back, "struck").props.strike, Some(true));
    assert_eq!(find_run(&back, "dunder").props.underline, Some(Underline::Single));
    assert!(find_para(&back, "E=mc").runs.iter().any(|r| r.props.vert_align == Some(VertAlign::Superscript)));
    assert_eq!(find_para(&back, "Ünïcödé").text(), "Ünïcödé — “quotes” & <angle> 日本語 😀");
    let b1 = find_para(&back, "Bullet one").props.list.flatten().unwrap();
    let b2 = find_para(&back, "Bullet two").props.list.flatten().unwrap();
    assert_eq!((b1.list, b1.level, b2.level), (b2.list, 0, 1));
    let n1 = find_para(&back, "Number one").props.list.flatten().unwrap();
    assert_ne!(n1.list, b1.list);
    assert_eq!(back.list(n1.list).unwrap().levels[0].format, NumFormat::Decimal);
    assert_eq!(find_run(&back, "example").props.link.as_deref(), Some("https://example.com/a?b=1&c=2"));
    let p = find_para(&back, "Image:");
    assert!(p.runs.iter().any(|r| matches!(&r.content, Inline::Image(i) if i.alt == "A pixel" && i.data.pixel_size == Some((1, 1)))));
    let t = back.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.rows.len(), 3);
    assert_eq!(t.rows[0].cells[0].text(), "H1");
    assert_eq!(t.rows[1].cells[0].text(), "Span two");
    let p = find_para(&back, "Visit");
    let fid = p.runs.iter().find_map(|r| if let Inline::FootnoteRef(i) = r.content { Some(i) } else { None }).unwrap();
    assert!(back.footnotes.iter().find(|n| n.id == fid).unwrap().paragraphs[0].text().contains("The footnote text."));
}

#[test]
fn markdown_read_commonmark() {
    let src = "Title\n=====\n\nSub\n---\n\n### Third ###\n\nSome *em* and **strong** and ***both*** and ~~del~~ and `co*de*` \\*lit\\* snake_case_word.\nSoft break\nhard break  \nnext\\\nlast &amp; &copy;\n\n\
* a\n* b\n  * b1\n    1. deep\n* c\n\n3. three\n4. four\n\n> quoted **text**\n> more\n\n```rust\nfn main() {\n\tprintln!();\n}\n```\n\n    indented code\n\n---\n\n\
| Left | Center | Right |\n|:-----|:------:|------:|\n| `a|b` | x \\| y | **z** |\n| 1 | 2 |\n\n\
[link](https://x.org/a_(b) \"title\") [ref][r] <https://auto.link> www.example.com/path. ![img](nope.png)\n\n\
Note[^n].\n\n[r]: https://ref.example\n[^n]: The note.\n\n<div align=\"center\"><b>html</b> block</div>\n";
    let d = markdown::read(src);
    assert_eq!(find_para(&d, "Title").props.style.as_deref(), Some("Heading1"));
    assert_eq!(find_para(&d, "Sub").props.style.as_deref(), Some("Heading2"));
    assert_eq!(find_para(&d, "Third").props.style.as_deref(), Some("Heading3"));
    assert_eq!(find_para(&d, "Third").text(), "Third");
    assert_eq!(find_run(&d, "em").props.italic, Some(true));
    assert_eq!(find_run(&d, "strong").props.bold, Some(true));
    let both = find_run(&d, "both");
    assert_eq!((both.props.bold, both.props.italic), (Some(true), Some(true)));
    assert_eq!(find_run(&d, "del").props.strike, Some(true));
    assert_eq!(find_run(&d, "co*de*").props.font.as_deref(), Some("Courier New"));
    let p = find_para(&d, "Some");
    assert!(p.text().contains("*lit* snake_case_word. Soft break hard break\nnext\nlast & ©"), "{:?}", p.text());
    let a = exact(&d, "a").props.list.flatten().unwrap();
    let b1 = exact(&d, "b1").props.list.flatten().unwrap();
    let deep = find_para(&d, "deep").props.list.flatten().unwrap();
    let c = exact(&d, "c").props.list.flatten().unwrap();
    assert_eq!((a.level, b1.level, deep.level, c.level), (0, 1, 2, 0));
    assert_eq!(a.list, deep.list);
    assert_eq!(d.list(a.list).unwrap().levels[2].format, NumFormat::Decimal);
    let three = find_para(&d, "three").props.list.flatten().unwrap();
    assert_ne!(three.list, a.list);
    assert_eq!(d.list(three.list).unwrap().levels[0].start, 3);
    let q = find_para(&d, "quoted");
    assert_eq!(q.props.style.as_deref(), Some("Quote"));
    assert_eq!(q.text(), "quoted text more");
    assert_eq!(find_run(&d, "text").props.bold, Some(true));
    assert_eq!(find_para(&d, "fn main").runs[0].props.font.as_deref(), Some("Courier New"));
    assert_eq!(find_para(&d, "println").text(), "\tprintln!();");
    assert_eq!(find_para(&d, "indented code").runs[0].props.font.as_deref(), Some("Courier New"));
    let t = d.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.rows.len(), 3);
    assert!(t.rows[0].header);
    assert_eq!(t.rows[1].cells[0].text(), "a|b");
    assert_eq!(t.rows[1].cells[1].text(), "x | y");
    assert_eq!(t.rows[1].cells[1].paragraphs[0].props.align, Some(Align::Center));
    assert_eq!(t.rows[1].cells[2].paragraphs[0].props.align, Some(Align::Right));
    assert_eq!(t.rows[2].cells.len(), 3);
    assert_eq!(find_run(&d, "link").props.link.as_deref(), Some("https://x.org/a_(b)"));
    assert_eq!(find_run(&d, "ref").props.link.as_deref(), Some("https://ref.example"));
    assert_eq!(find_run(&d, "https://auto.link").props.link.as_deref(), Some("https://auto.link"));
    assert_eq!(find_run(&d, "www.example.com/path").props.link.as_deref(), Some("http://www.example.com/path"));
    assert!(find_para(&d, "link").text().ends_with("[img]"));
    let p = find_para(&d, "Note");
    let id = p.runs.iter().find_map(|r| if let Inline::FootnoteRef(i) = r.content { Some(i) } else { None }).unwrap();
    assert!(d.footnotes.iter().find(|n| n.id == id).unwrap().paragraphs[0].text().contains("The note."));
    assert!(!d.plain_text().contains("ref.example"));
    let h = find_para(&d, "html");
    assert_eq!(h.props.align, Some(Align::Center));
    assert_eq!(find_run(&d, "html").props.bold, Some(true));
    // A rule.
    assert!(body_paras(&d).iter().any(|p| p.runs.is_empty() && p.props.borders.is_some()));
}

#[test]
fn text_formats_via_dispatch() {
    let dir = scratch_dir().join("text_dispatch");
    std::fs::create_dir_all(&dir).unwrap();
    let doc = kitchen_sink();
    for (name, check) in [("a.md", "Chapter One"), ("a.txt", "Chapter One"), ("a.html", "Chapter One"), ("a.rtf", "Chapter One")] {
        let path = dir.join(name);
        io::save(&doc, &path).unwrap();
        let back = io::open(&path).unwrap();
        assert!(back.plain_text().contains(check), "{name}");
        assert!(back.plain_text().contains("Fancy paragraph"), "{name}");
    }
    // Content sniffing beats a wrong extension.
    let path = dir.join("really_html.txt");
    std::fs::write(&path, io::html::write(&doc)).unwrap();
    let back = io::open(&path).unwrap();
    assert_eq!(find_para(&back, "Chapter One").props.style.as_deref(), Some("Heading1"));
}

#[test]
#[ignore]
fn dump_md() {
    std::fs::create_dir_all("/tmp/claude-io-scratch").unwrap();
    std::fs::write("/tmp/claude-io-scratch/kitchen.md", markdown::write(&kitchen_sink())).unwrap();
}

fn exact<'a>(d: &'a Document, s: &str) -> &'a Paragraph {
    body_paras(d).into_iter().find(|p| p.text() == s).unwrap_or_else(|| panic!("no paragraph {s:?}"))
}

#[test]
fn readers_survive_truncated_input() {
    let doc = kitchen_sink();
    let html = io::html::write(&doc);
    let md = markdown::write(&doc);
    let rtf = io::rtf::write(&doc);
    for src in [&html, &md, &rtf] {
        let len = src.len();
        let step = (len / 300).max(1);
        let mut cut = 0;
        while cut < len {
            let mut c = cut;
            while !src.is_char_boundary(c) {
                c += 1;
            }
            let s = &src[..c];
            let _ = io::html::read(s);
            let _ = io::html::read_fragment(s);
            let _ = markdown::read(s);
            let _ = io::rtf::read(s.as_bytes());
            cut += step;
        }
    }
    for junk in ["<", "<<>>", "<a href=", "&#", "&#x110000;", "[", "![", "**", "`", "|", "| a |\n|", "{\\rtf1", "{\\rtf1\\u", "{\\rtf1{\\*", "{\\rtf1\\'", "> ", "1.", "- ", "```"] {
        let _ = io::html::read(junk);
        let _ = markdown::read(junk);
        let _ = io::rtf::read(junk.as_bytes());
    }
}

#[test]
fn text_box_paragraphs_in_plain_formats() {
    let doc = sections_doc();
    let t = wordcore::io::txt::write(&doc);
    let lines: Vec<&str> = t.lines().collect();
    let i = lines.iter().position(|l| *l == "Anchor of the box").unwrap();
    assert_eq!(&lines[i + 1..i + 3], &["Box line one", "Box line two"]);
    let md = wordcore::io::markdown::write(&doc);
    assert!(md.contains("Anchor of the box\n\n> Box line one\n>\n> Box line two\n"), "{md}");
}
