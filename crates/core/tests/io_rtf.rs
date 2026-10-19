mod common;

use common::*;
use wordcore::io::{self, rtf};
use wordcore::*;

fn roundtrip(doc: &Document) -> Document {
    let text = rtf::write(doc);
    rtf::read(text.as_bytes()).expect("read")
}

#[test]
fn rtf_output_is_ascii_and_balanced() {
    let text = rtf::write(&kitchen_sink());
    assert!(text.starts_with("{\\rtf1"));
    assert!(text.is_ascii(), "RTF output must be 7-bit");
    let mut depth = 0i32;
    let mut esc = false;
    for c in text.chars() {
        if esc {
            esc = false;
            continue;
        }
        match c {
            '\\' => esc = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                assert!(depth >= 0);
            }
            _ => {}
        }
    }
    assert_eq!(depth, 0);
    assert_eq!(io::Format::sniff(text.as_bytes()), io::Format::Rtf);
}

#[test]
fn rtf_roundtrip_char_props() {
    let back = roundtrip(&kitchen_sink());
    assert_eq!(find_run(&back, "bold").props.bold, Some(true));
    assert_eq!(find_run(&back, "italic").props.italic, Some(true));
    assert_eq!(find_run(&back, "dunder").props.underline, Some(Underline::Double));
    assert_eq!(find_run(&back, "struck").props.strike, Some(true));
    assert_eq!(find_run(&back, "dstruck").props.double_strike, Some(true));
    assert_eq!(find_run(&back, "red").props.color, Some(Color::rgb(255, 0, 0)));
    assert_eq!(find_run(&back, "hilite").props.highlight, Some(Color::rgb(255, 255, 0)));
    assert_eq!(find_run(&back, "caps").props.caps, Some(true));
    assert_eq!(find_run(&back, "smallcaps").props.small_caps, Some(true));
    assert_eq!(find_run(&back, "hidden").props.hidden, Some(true));
    let g = find_run(&back, "georgia");
    assert_eq!(g.props.font.as_deref(), Some("Georgia"));
    assert_eq!(g.props.size, Some(14.5));
    let p = find_para(&back, "E=mc");
    assert!(p.runs.iter().any(|r| r.props.vert_align == Some(VertAlign::Superscript)));
    assert!(p.runs.iter().any(|r| r.props.vert_align == Some(VertAlign::Subscript)));
    assert_eq!(find_para(&back, "Ünïcödé").text(), "Ünïcödé — “quotes” & <angle> 日本語 😀");
    assert!(find_para(&back, "two  spaces").text().ends_with("  two  spaces "));
}

#[test]
fn rtf_roundtrip_paragraphs_styles_lists() {
    let doc = kitchen_sink();
    let back = roundtrip(&doc);
    let o = find_para(&doc, "Fancy paragraph");
    let p = find_para(&back, "Fancy paragraph");
    assert_eq!(p.props.align, o.props.align);
    assert_eq!(p.props.indent_left, o.props.indent_left);
    assert_eq!(p.props.indent_right, o.props.indent_right);
    assert_eq!(p.props.indent_first, o.props.indent_first);
    assert_eq!(p.props.space_before, o.props.space_before);
    assert_eq!(p.props.space_after, o.props.space_after);
    assert_eq!(p.props.line_spacing, o.props.line_spacing);
    assert_eq!(p.props.keep_next, Some(true));
    assert_eq!(p.props.keep_lines, Some(true));
    assert_eq!(p.props.shading, o.props.shading);
    assert_eq!(p.props.tabs, o.props.tabs);
    assert!(p.props.borders.as_ref().unwrap().top.is_some());
    assert_eq!(find_para(&back, "Line spacing 1.5").props.line_spacing, Some(LineSpacing::Multiple(1.5)));
    assert_eq!(find_para(&back, "At least 20pt").props.line_spacing, Some(LineSpacing::AtLeast(20.0)));
    assert_eq!(find_para(&back, "Starts on new page").props.page_break_before, Some(true));
    assert_eq!(find_para(&back, "Chapter One").props.style.as_deref(), Some("Heading1"));
    assert_eq!(find_para(&back, "The Title").props.style.as_deref(), Some("Title"));
    let s = back.styles.iter().find(|s| s.name == "My Style").expect("custom style");
    assert_eq!(s.para.align, Some(Align::Center));
    assert_eq!(s.chars.color, Some(Color::rgb(0x80, 0, 0)));
    assert_eq!(back.style("Heading1").unwrap().chars.size, Some(16.0));

    let b1 = find_para(&back, "Bullet one").props.list.flatten().unwrap();
    let b2 = find_para(&back, "Bullet two").props.list.flatten().unwrap();
    assert_eq!(b1.list, b2.list);
    assert_eq!((b1.level, b2.level), (0, 1));
    assert_eq!(back.list(b1.list).unwrap().levels[0].format, NumFormat::Bullet);
    let n1 = find_para(&back, "Number one").props.list.flatten().unwrap();
    assert_ne!(n1.list, b1.list);
    let nl = back.list(n1.list).unwrap();
    assert_eq!(nl.levels[0].format, NumFormat::Decimal);
    assert_eq!(nl.levels[0].text, "%1.");
}

#[test]
fn rtf_roundtrip_tables_images_fields_notes() {
    let doc = kitchen_sink();
    let back = roundtrip(&doc);
    let t = back.body.iter().find_map(|b| b.as_table()).expect("table");
    assert_eq!(t.rows.len(), 3);
    assert_eq!(t.columns.len(), 3);
    for (a, b) in t.columns.iter().zip([100.0, 150.0, 182.0]) {
        assert!((a - b).abs() < 0.1, "{:?}", t.columns);
    }
    assert!(t.rows[0].header);
    assert_eq!(t.rows[0].cells[0].shading, Some(Color::rgb(0xCC, 0xCC, 0xCC)));
    assert_eq!(t.rows[0].cells[0].valign, VAlign::Center);
    assert_eq!(t.rows[1].cells.len(), 2);
    assert_eq!(t.rows[1].cells[0].span, 2);
    assert_eq!(t.rows[1].cells[1].vmerge, VMerge::Restart);
    assert_eq!(t.rows[2].cells[2].vmerge, VMerge::Continue);
    assert!(back.plain_text().contains("Span two"));

    let p = find_para(&back, "Image:");
    let img = p.runs.iter().find_map(|r| if let Inline::Image(i) = &r.content { Some(i) } else { None }).expect("image");
    assert!((img.width - 72.0).abs() < 0.1 && (img.height - 36.0).abs() < 0.1, "{} x {}", img.width, img.height);
    assert_eq!(img.data.pixel_size, Some((1, 1)));
    assert!(p.runs.iter().any(|r| r.content == Inline::Field(Field::PageNumber)));
    assert!(p.runs.iter().any(|r| r.content == Inline::Field(Field::PageCount)));

    let p = find_para(&back, "Visit");
    assert!(p.runs.iter().any(|r| r.content == Inline::Bookmark("target_mark".into())));
    assert_eq!(find_run(&back, "example").props.link.as_deref(), Some("https://example.com/a?b=1&c=2"));
    assert_eq!(find_run(&back, "inside").props.link.as_deref(), Some("#target_mark"));
    let fid = p.runs.iter().find_map(|r| if let Inline::FootnoteRef(i) = r.content { Some(i) } else { None }).expect("fn ref");
    assert_eq!(back.footnotes.iter().find(|n| n.id == fid).unwrap().paragraphs[0].text(), "The footnote text.");
    let eid = p.runs.iter().find_map(|r| if let Inline::EndnoteRef(i) = r.content { Some(i) } else { None }).expect("en ref");
    assert_eq!(back.endnotes.iter().find(|n| n.id == eid).unwrap().paragraphs[0].text(), "The endnote text.");

    assert_eq!(back.comments.len(), 2);
    assert_eq!(back.comments[0].author, "Reviewer");
    assert_eq!(back.comments[0].paragraphs[0].text(), "First comment");
    let cid = back.comments[0].id;
    assert!(find_run(&back, "commented").props.comments.contains(&cid));
}

#[test]
fn rtf_roundtrip_page_and_headers() {
    let doc = kitchen_sink();
    let back = roundtrip(&doc);
    let (p, q) = (&doc.page, &back.page);
    for (a, b) in [
        (p.width, q.width),
        (p.height, q.height),
        (p.margin_top, q.margin_top),
        (p.margin_bottom, q.margin_bottom),
        (p.margin_left, q.margin_left),
        (p.margin_right, q.margin_right),
        (p.header_distance, q.header_distance),
        (p.footer_distance, q.footer_distance),
    ] {
        assert!((a - b).abs() < 0.06, "{a} vs {b}");
    }
    assert_eq!(q.columns, 2);
    assert!(q.title_page);
    assert_eq!(q.page_number_start, 3);
    assert_eq!(back.headers.header[0].as_para().unwrap().text(), "Header text\t");
    assert_eq!(back.headers.footer[0].as_para().unwrap().text(), "Footer text");
    assert_eq!(back.headers.first_header[0].as_para().unwrap().text(), "First header");
    assert_eq!(back.properties.title, doc.properties.title);
    assert_eq!(back.properties.author, doc.properties.author);
}

#[test]
fn rtf_reader_handwritten() {
    let src = br#"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\froman Times New Roman;}{\f1\fswiss\fcharset0 Arial;}}
{\colortbl;\red255\green0\blue0;\red0\green0\blue255;}
{\*\generator Some Writer 1.0;}{\*\unknowndest with {nested} junk}
{\info{\title My Title}{\author Jane}}
\paperw12240\paperh15840\margl1440\margr1440
\pard\plain\qc\sb240\sa120\f1\fs32\b Centered bold\b0  Arial 16\par
\pard\li720\ri360\fi-360\sl360\slmult1 Indented \i italic\i0  \ul under\ulnone  \strike gone\strike0  \cf1 scarlet\cf0  {\cf2\highlight1 blue}\par
\pard Caf\'e9 na\'efve \u8364?euro \uc2\u26085\'93\'fa done\uc1  \'93q\'94\par
\pard a\tab b\line c\page d\par
\trowd\trgaph108\cellx2000\cellx5000\pard\intbl A1\cell B1\cell\row
\trowd\trgaph108\cellx2000\cellx5000\pard\intbl A2\cell B2\cell\row
\pard after table\par
{\field{\*\fldinst{HYPERLINK "https://rust-lang.org"}}{\fldrslt{\ul\cf2 Rust}}} and {\field{\*\fldinst PAGE}{\fldrslt 1}}\par
}"#;
    let doc = rtf::read(src).unwrap();
    assert_eq!(doc.properties.title, "My Title");
    assert_eq!(doc.properties.author, "Jane");
    assert!((doc.page.width - 612.0).abs() < 0.01);
    let p = find_para(&doc, "Centered bold");
    assert_eq!(p.props.align, Some(Align::Center));
    assert_eq!(p.props.space_before, Some(12.0));
    assert_eq!(p.props.space_after, Some(6.0));
    let r = find_run(&doc, "Centered bold");
    assert_eq!(r.props.bold, Some(true));
    assert_eq!(r.props.size, Some(16.0));
    assert_eq!(r.props.font.as_deref(), Some("Arial"));
    assert_ne!(find_run(&doc, " Arial 16").props.bold, Some(true));
    let p = find_para(&doc, "Indented");
    assert_eq!(p.props.indent_left, Some(36.0));
    assert_eq!(p.props.indent_right, Some(18.0));
    assert_eq!(p.props.indent_first, Some(-18.0));
    assert_eq!(p.props.line_spacing, Some(LineSpacing::Multiple(1.5)));
    assert_eq!(find_run(&doc, "italic").props.italic, Some(true));
    assert_eq!(find_run(&doc, "under").props.underline, Some(Underline::Single));
    assert_eq!(find_run(&doc, "gone").props.strike, Some(true));
    assert_eq!(find_run(&doc, "scarlet").props.color, Some(Color::rgb(255, 0, 0)));
    let b = find_run(&doc, "blue");
    assert_eq!(b.props.color, Some(Color::rgb(0, 0, 255)));
    assert_eq!(b.props.highlight, Some(Color::rgb(255, 0, 0)));
    assert_eq!(find_para(&doc, "Caf").text(), "Café naïve €euro 日 done “q”");
    let p = find_para(&doc, "a\tb");
    assert_eq!(p.text(), "a\tb\ncd");
    assert!(p.runs.iter().any(|r| r.content == Inline::PageBreak));
    let t = doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.rows.len(), 2);
    assert_eq!(t.columns, vec![100.0, 150.0]);
    assert_eq!(t.rows[1].cells[1].paragraphs[0].text(), "B2");
    assert!(doc.plain_text().contains("after table"));
    assert!(!doc.plain_text().contains("junk"));
    assert!(!doc.plain_text().contains("Some Writer"));
    assert_eq!(find_run(&doc, "Rust").props.link.as_deref(), Some("https://rust-lang.org"));
    assert!(find_para(&doc, "Rust").runs.iter().any(|r| r.content == Inline::Field(Field::PageNumber)));
}

#[test]
fn rtf_rejects_non_rtf() {
    assert!(rtf::read(b"hello").is_err());
}

#[test]
fn rtf_libreoffice_interop() {
    if !soffice_available() {
        eprintln!("soffice not available; skipping");
        return;
    }
    let dir = scratch_dir().join("rtf_lo");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("kitchen.rtf");
    std::fs::write(&path, rtf::write(&kitchen_sink())).unwrap();
    let pdf = soffice_convert(&path, "pdf", &dir.join("pdf")).expect("LibreOffice could not convert the rtf");
    if let Some(text) = pdf_text(&pdf) {
        for needle in ["The Title", "Chapter One", "Fancy paragraph", "Span two", "Header text", "The footnote text", "Ünïcödé"] {
            assert!(text.contains(needle), "PDF lacks {needle:?}");
        }
    }
    // LibreOffice's own RTF output must read back.
    let docx_path = dir.join("src.docx");
    std::fs::write(&docx_path, io::docx::write(&kitchen_sink()).unwrap()).unwrap();
    let lo_rtf = soffice_convert(&docx_path, "rtf", &dir.join("lo")).expect("docx→rtf");
    let back = io::open(&lo_rtf).unwrap();
    assert!(back.plain_text().contains("Fancy paragraph"));
    assert_eq!(find_run(&back, "bold").props.bold, Some(true));
    assert_eq!(find_run(&back, "red").props.color, Some(Color::rgb(255, 0, 0)));
    assert_eq!(find_para(&back, "Chapter One").props.style.as_deref(), Some("Heading1"));
    assert_eq!(find_para(&back, "Ünïcödé").text(), "Ünïcödé — “quotes” & <angle> 日本語 😀");
    let t = back.body.iter().find_map(|b| b.as_table()).expect("table");
    assert_eq!(t.rows.len(), 3);
    assert!(back.plain_text().contains("Span two"));
    assert_eq!(back.footnotes.len(), 1);
    assert!(matches!(find_para(&back, "Bullet two").props.list, Some(Some(ListRef { level: 1, .. }))));
    let p = find_para(&back, "Image:");
    assert!(p.runs.iter().any(|r| matches!(r.content, Inline::Image(_))));
}

#[test]
#[ignore]
fn dump_kitchen_rtf() {
    std::fs::create_dir_all("/tmp/claude-io-scratch").unwrap();
    std::fs::write("/tmp/claude-io-scratch/kitchen.rtf", rtf::write(&kitchen_sink())).unwrap();
    let mut d = kitchen_sink();
    d.page.columns = 1;
    std::fs::write("/tmp/claude-io-scratch/kitchen1.rtf", rtf::write(&d)).unwrap();
    std::fs::write("/tmp/claude-io-scratch/kitchen1.docx", io::docx::write(&d).unwrap()).unwrap();
}
