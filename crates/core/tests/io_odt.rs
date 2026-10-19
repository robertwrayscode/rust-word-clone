mod common;

use common::*;
use wordcore::io::{self, odt};
use wordcore::*;

fn roundtrip(doc: &Document) -> Document {
    let bytes = odt::write(doc).expect("write");
    odt::read(&bytes).expect("read")
}

#[test]
fn odt_roundtrip_text_and_char_props() {
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
    assert_eq!(find_run(&back, "spaced").props.spacing, Some(1.5));
    let g = find_run(&back, "georgia");
    assert_eq!(g.props.font.as_deref(), Some("Georgia"));
    assert_eq!(g.props.size, Some(14.5));
    assert_eq!(find_run(&back, "emph").props.style.as_deref(), Some("Emphasis"));
    // Whitespace is preserved exactly.
    assert!(find_para(&back, "two  spaces").text().ends_with("  two  spaces "), "{:?}", find_para(&back, "two  spaces").text());
    let p = find_para(&back, "E=mc");
    assert!(p.runs.iter().any(|r| r.props.vert_align == Some(VertAlign::Superscript)));
    assert!(p.runs.iter().any(|r| r.props.vert_align == Some(VertAlign::Subscript)));
    assert_eq!(find_para(&back, "Ünïcödé").text(), "Ünïcödé — “quotes” & <angle> 日本語 😀");
}

#[test]
fn odt_roundtrip_para_props_and_styles() {
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
    assert_eq!(p.props.widow_control, Some(false));
    assert_eq!(p.props.contextual_spacing, Some(true));
    assert_eq!(p.props.shading, o.props.shading);
    assert_eq!(p.props.tabs, o.props.tabs);
    let b = p.props.borders.as_ref().unwrap();
    assert_eq!(b.top.unwrap().color, Color::rgb(0, 0, 255));
    assert_eq!(b.top.unwrap().width, 1.0);
    assert_eq!(b.top.unwrap().space, 4.0);
    assert_eq!(b.bottom.unwrap().style, BorderStyle::Dashed);
    assert_eq!(find_para(&back, "Line spacing 1.5").props.line_spacing, Some(LineSpacing::Multiple(1.5)));
    assert_eq!(find_para(&back, "At least 20pt").props.line_spacing, Some(LineSpacing::AtLeast(20.0)));
    assert_eq!(find_para(&back, "Starts on new page").props.page_break_before, Some(true));
    assert_eq!(find_para(&back, "Chapter One").props.style.as_deref(), Some("Heading1"));
    assert_eq!(find_para(&back, "Section 1.1").props.style.as_deref(), Some("Heading2"));
    assert_eq!(find_para(&back, "The Title").props.style.as_deref(), Some("Title"));
    let s = back.style("MyStyle").expect("custom style");
    assert_eq!(s.name, "My Style");
    assert_eq!(s.para.align, Some(Align::Center));
    assert_eq!(s.chars.color, Some(Color::rgb(0x80, 0, 0)));
    assert_eq!(s.based_on.as_deref(), Some("Normal"));
    let h1 = back.style("Heading1").unwrap();
    assert_eq!(h1.chars.size, Some(16.0));
    assert_eq!(h1.para.outline_level, Some(0));
    assert_eq!(back.default_chars.font.as_deref(), Some("Calibri"));
    assert_eq!(back.default_para.space_after, Some(8.0));
    assert_eq!(back.settings.default_tab, 30.0);
    // Page break inside a paragraph splits it.
    let i = body_paras(&back).iter().position(|p| p.text() == "Before page break").unwrap();
    let next = body_paras(&back)[i + 1];
    assert_eq!(next.text(), "After page break");
    assert_eq!(next.props.page_break_before, Some(true));
}

#[test]
fn odt_roundtrip_lists_tables_images_notes() {
    let doc = kitchen_sink();
    let back = roundtrip(&doc);
    let b1 = find_para(&back, "Bullet one").props.list.flatten().unwrap();
    let b2 = find_para(&back, "Bullet two").props.list.flatten().unwrap();
    assert_eq!(b1.list, b2.list);
    assert_eq!((b1.level, b2.level), (0, 1));
    let l = back.list(b1.list).unwrap();
    assert_eq!(l.levels[0].format, NumFormat::Bullet);
    assert_eq!(l.levels[0].text, "•");
    assert_eq!(l.levels[0].indent, 36.0);
    assert_eq!(l.levels[0].hanging, 18.0);
    let n1 = find_para(&back, "Number one").props.list.flatten().unwrap();
    let n2 = find_para(&back, "Number two").props.list.flatten().unwrap();
    assert_eq!(n1, n2);
    assert_ne!(n1.list, b1.list);
    let nl = back.list(n1.list).unwrap();
    assert_eq!(nl.levels[0].format, NumFormat::Decimal);
    assert_eq!(nl.levels[0].text, "%1.");
    assert_eq!(nl.levels[1].format, NumFormat::LowerLetter);

    let t = back.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.columns, vec![100.0, 150.0, 182.0]);
    assert_eq!(t.align, Align::Center);
    assert!(t.rows[0].header);
    assert_eq!(t.rows[0].height, Some(24.0));
    assert_eq!(t.rows[0].cells[0].shading, Some(Color::rgb(0xCC, 0xCC, 0xCC)));
    assert_eq!(t.rows[0].cells[0].valign, VAlign::Center);
    assert_eq!(t.rows[1].cells.len(), 2);
    assert_eq!(t.rows[1].cells[0].span, 2);
    assert_eq!(t.rows[1].cells[1].vmerge, VMerge::Restart);
    assert_eq!(t.rows[2].cells.len(), 3);
    assert_eq!(t.rows[2].cells[2].vmerge, VMerge::Continue);
    assert_eq!(t.borders.top.unwrap().color, Color::rgb(0x44, 0x44, 0x44));
    assert!(t.borders.inside_v.is_some());
    assert!(t.borders.between.is_some());

    let p = find_para(&back, "Image:");
    let img = p.runs.iter().find_map(|r| if let Inline::Image(i) = &r.content { Some(i) } else { None }).unwrap();
    assert_eq!((img.width, img.height, img.alt.as_str()), (72.0, 36.0, "A pixel"));
    assert_eq!(img.data.pixel_size, Some((1, 1)));
    assert!(p.runs.iter().any(|r| r.content == Inline::Field(Field::PageNumber)));
    assert!(p.runs.iter().any(|r| r.content == Inline::Field(Field::PageCount)));
    assert!(p.runs.iter().any(|r| matches!(r.content, Inline::Field(Field::Date(_)))));
    assert!(p.text().ends_with("author Ada"));

    let p = find_para(&back, "Visit");
    assert_eq!(p.runs[0].content, Inline::Bookmark("target_mark".into()));
    assert_eq!(find_run(&back, "example").props.link.as_deref(), Some("https://example.com/a?b=1&c=2"));
    assert_eq!(find_run(&back, "inside").props.link.as_deref(), Some("#target_mark"));
    let fid = p.runs.iter().find_map(|r| if let Inline::FootnoteRef(i) = r.content { Some(i) } else { None }).unwrap();
    assert_eq!(back.footnotes.iter().find(|n| n.id == fid).unwrap().paragraphs[0].text(), "The footnote text.");
    let eid = p.runs.iter().find_map(|r| if let Inline::EndnoteRef(i) = r.content { Some(i) } else { None }).unwrap();
    assert_eq!(back.endnotes.iter().find(|n| n.id == eid).unwrap().paragraphs[0].text(), "The endnote text.");

    // Comments survive; tracked deletions are exported as accepted.
    assert_eq!(back.comments.len(), 2);
    assert_eq!(back.comments[0].author, "Reviewer");
    assert_eq!(back.comments[0].paragraphs[0].text(), "First comment");
    let cid = back.comments[0].id;
    assert!(find_run(&back, "commented").props.comments.contains(&cid));
    assert!(find_run(&back, " both").props.comments.len() == 2);
    assert_eq!(find_para(&back, "Tracked:").text(), "Tracked: inserted  commented both tail");
}

#[test]
fn odt_roundtrip_page_and_headers() {
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
        (p.column_gap, q.column_gap),
    ] {
        assert!((a - b).abs() < 0.01, "{a} vs {b}");
    }
    assert_eq!(q.columns, 2);
    assert_eq!(q.page_color, p.page_color);
    assert!(q.title_page);
    assert_eq!(q.page_number_start, 3);
    let pb = q.page_borders.as_ref().unwrap();
    assert_eq!(pb.top.unwrap().style, BorderStyle::Double);
    assert_eq!(pb.top.unwrap().width, 1.5);
    assert_eq!(back.headers.header[0].as_para().unwrap().text(), "Header text\t");
    assert!(back.headers.header[0].as_para().unwrap().runs.iter().any(|r| r.content == Inline::Field(Field::PageNumber)));
    assert_eq!(back.headers.footer[0].as_para().unwrap().text(), "Footer text");
    assert_eq!(back.headers.first_header[0].as_para().unwrap().text(), "First header");
    assert_eq!(back.headers.first_footer[0].as_para().unwrap().text(), "First footer");
    assert_eq!(back.properties, doc.properties);
}

#[test]
fn odt_package_layout() {
    let bytes = odt::write(&kitchen_sink()).unwrap();
    // `mimetype` must be the first entry, stored uncompressed.
    assert_eq!(&bytes[0..4], b"PK\x03\x04");
    assert_eq!(u16::from_le_bytes([bytes[8], bytes[9]]), 0, "mimetype must be stored");
    let name_len = u16::from_le_bytes([bytes[26], bytes[27]]) as usize;
    assert_eq!(&bytes[30..30 + name_len], b"mimetype");
    assert_eq!(&bytes[30 + name_len..30 + name_len + 39], b"application/vnd.oasis.opendocument.text");
    let pkg = io::package::Package::open(&bytes).unwrap();
    for f in ["content.xml", "styles.xml", "meta.xml", "META-INF/manifest.xml", "Pictures/image1.png"] {
        assert!(pkg.has(f), "{f}");
    }
    assert_eq!(io::Format::sniff(&bytes), io::Format::Odt);
}

#[test]
fn odt_loads_in_libreoffice() {
    if !soffice_available() {
        eprintln!("soffice not available; skipping");
        return;
    }
    let dir = scratch_dir().join("odt_lo");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("kitchen.odt");
    std::fs::write(&path, odt::write(&kitchen_sink()).unwrap()).unwrap();
    let pdf = soffice_convert(&path, "pdf", &dir.join("pdf")).expect("LibreOffice could not convert the odt");
    if let Some(text) = pdf_text(&pdf) {
        for needle in ["The Title", "Chapter One", "Fancy paragraph", "Span two", "Header text", "The footnote text", "Ünïcödé"] {
            assert!(text.contains(needle), "PDF lacks {needle:?}");
        }
    }
    // LibreOffice's own ODT output (re-saved) must read back.
    let resaved = soffice_convert(&path, "odt", &dir.join("resaved")).expect("re-save");
    let back = io::open(&resaved).unwrap();
    assert!(back.plain_text().contains("Fancy paragraph"));
    assert_eq!(find_run(&back, "bold").props.bold, Some(true));
    assert_eq!(find_para(&back, "Chapter One").props.style.as_deref(), Some("Heading1"));
    assert!(back.body.iter().any(|b| b.as_table().is_some()));
    assert_eq!(back.footnotes.len(), 1);
    assert!(matches!(find_para(&back, "Bullet two").props.list, Some(Some(ListRef { level: 1, .. }))));
    let t = back.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.rows[1].cells[0].span, 2);
    assert_eq!(t.rows[1].cells[1].vmerge, VMerge::Restart);
    // And a LibreOffice-produced ODT converted from our DOCX.
    let docx_path = dir.join("fromdocx.docx");
    std::fs::write(&docx_path, io::docx::write(&kitchen_sink()).unwrap()).unwrap();
    let odt2 = soffice_convert(&docx_path, "odt", &dir.join("fromdocx")).expect("docx→odt");
    let d2 = io::open(&odt2).unwrap();
    assert!(d2.plain_text().contains("Span two"));
    assert_eq!(find_run(&d2, "red").props.color, Some(Color::rgb(255, 0, 0)));
}
