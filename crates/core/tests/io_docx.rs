mod common;

use common::*;
use std::io::Write;
use wordcore::io::{self, docx, Format};
use wordcore::*;

fn roundtrip(doc: &Document) -> Document {
    let bytes = docx::write(doc).expect("write");
    docx::read(&bytes).expect("read")
}

#[test]
fn docx_roundtrip_properties_settings_page() {
    let doc = kitchen_sink();
    let back = roundtrip(&doc);
    assert_eq!(back.properties, doc.properties);
    assert!(back.settings.track_changes);
    assert_eq!(back.settings.default_tab, 30.0);
    let (p, q) = (&doc.page, &back.page);
    assert!((p.width - q.width).abs() < 0.1 && (p.height - q.height).abs() < 0.1);
    assert_eq!(q.orientation(), Orientation::Landscape);
    for (a, b) in [
        (p.margin_top, q.margin_top),
        (p.margin_bottom, q.margin_bottom),
        (p.margin_left, q.margin_left),
        (p.margin_right, q.margin_right),
        (p.header_distance, q.header_distance),
        (p.footer_distance, q.footer_distance),
        (p.column_gap, q.column_gap),
    ] {
        assert!((a - b).abs() < 0.06, "{a} vs {b}");
    }
    assert_eq!(q.columns, 2);
    assert!(q.title_page);
    assert_eq!(q.page_number_start, 3);
    assert_eq!(q.page_color, p.page_color);
    assert_eq!(q.watermark.as_deref(), Some("DRAFT"));
    let pb = q.page_borders.as_ref().unwrap();
    let top = pb.top.unwrap();
    assert_eq!(top.style, BorderStyle::Double);
    assert_eq!(top.width, 1.5);
    assert_eq!(top.color, Color::rgb(0x11, 0x22, 0x33));
    assert_eq!(top.space, 24.0);
}

#[test]
fn docx_roundtrip_char_props() {
    let back = roundtrip(&kitchen_sink());
    assert_eq!(find_run(&back, "bold").props.bold, Some(true));
    assert_eq!(find_run(&back, "italic").props.italic, Some(true));
    assert_eq!(find_run(&back, "dunder").props.underline, Some(Underline::Double));
    assert_eq!(find_run(&back, "struck").props.strike, Some(true));
    assert_eq!(find_run(&back, "dstruck").props.double_strike, Some(true));
    assert_eq!(find_run(&back, "red").props.color, Some(Color::rgb(255, 0, 0)));
    assert_eq!(find_run(&back, "hilite").props.highlight, Some(Color::rgb(255, 255, 0)));
    assert_eq!(find_run(&back, "shaded").props.shading, Some(Color::rgb(0xDD, 0xEE, 0xFF)));
    assert_eq!(find_run(&back, "caps").props.caps, Some(true));
    assert_eq!(find_run(&back, "smallcaps").props.small_caps, Some(true));
    assert_eq!(find_run(&back, "hidden").props.hidden, Some(true));
    assert_eq!(find_run(&back, "spaced").props.spacing, Some(1.5));
    let g = find_run(&back, "georgia");
    assert_eq!(g.props.font.as_deref(), Some("Georgia"));
    assert_eq!(g.props.size, Some(14.5));
    assert_eq!(find_run(&back, "emph").props.style.as_deref(), Some("Emphasis"));
    assert!(find_para(&back, "two  spaces").text().ends_with("  two  spaces "));
    let p = find_para(&back, "E=mc");
    let sup = p.runs.iter().find(|r| r.props.vert_align == Some(VertAlign::Superscript)).unwrap();
    assert_eq!(sup.content, Inline::Text("2".into()));
    assert!(p.runs.iter().any(|r| r.props.vert_align == Some(VertAlign::Subscript)));
}

#[test]
fn docx_roundtrip_para_props() {
    let doc = kitchen_sink();
    let back = roundtrip(&doc);
    let orig = find_para(&doc, "Fancy paragraph");
    let p = find_para(&back, "Fancy paragraph");
    assert_eq!(p.props, orig.props);
    assert_eq!(p.runs.iter().filter(|r| r.content == Inline::Tab).count(), 1);
    assert_eq!(p.runs.iter().filter(|r| r.content == Inline::LineBreak).count(), 1);
    assert_eq!(find_para(&back, "Line spacing 1.5").props.line_spacing, Some(LineSpacing::Multiple(1.5)));
    assert_eq!(find_para(&back, "Line spacing 1.5").props.align, Some(Align::Right));
    let al = find_para(&back, "At least 20pt");
    assert_eq!(al.props.line_spacing, Some(LineSpacing::AtLeast(20.0)));
    assert_eq!(al.props.indent_first, Some(24.0));
    assert_eq!(find_para(&back, "Starts on new page").props.page_break_before, Some(true));
    assert_eq!(find_para(&back, "Chapter One").props.style.as_deref(), Some("Heading1"));
    assert_eq!(find_para(&back, "The Title").props.style.as_deref(), Some("Title"));
}

#[test]
fn docx_roundtrip_styles() {
    let doc = kitchen_sink();
    let back = roundtrip(&doc);
    let s = back.style("MyStyle").expect("custom style");
    assert_eq!(s.name, "My Style");
    assert_eq!(s.based_on.as_deref(), Some("Normal"));
    assert_eq!(s.next.as_deref(), Some("Normal"));
    assert_eq!(s.para.align, Some(Align::Center));
    assert_eq!(s.para.space_before, Some(6.0));
    assert_eq!(s.chars.color, Some(Color::rgb(0x80, 0, 0)));
    assert!(s.quick);
    // Built-in styles survive with their display names and formatting.
    let h1 = back.style("Heading1").unwrap();
    assert_eq!(h1.name, "Heading 1");
    assert_eq!(h1, doc.style("Heading1").unwrap());
    for st in &doc.styles {
        let b = back.style(&st.id).unwrap_or_else(|| panic!("missing style {}", st.id));
        assert_eq!(b, st, "style {}", st.id);
    }
    assert_eq!(back.default_chars, doc.default_chars);
    assert_eq!(back.default_para, doc.default_para);
}

#[test]
fn docx_roundtrip_lists() {
    let doc = kitchen_sink();
    let back = roundtrip(&doc);
    assert_eq!(back.lists, doc.lists);
    let b2 = find_para(&back, "Bullet two");
    assert_eq!(b2.props.list, Some(Some(ListRef { list: 1, level: 1 })));
    let n1 = find_para(&back, "Number one");
    assert_eq!(n1.props.list, Some(Some(ListRef { list: 2, level: 0 })));
}

#[test]
fn docx_roundtrip_inlines() {
    let back = roundtrip(&kitchen_sink());
    let p = find_para(&back, "Image:");
    let img = p.runs.iter().find_map(|r| if let Inline::Image(i) = &r.content { Some(i) } else { None }).unwrap();
    assert_eq!((img.width, img.height), (72.0, 36.0));
    assert_eq!(img.alt, "A pixel");
    assert_eq!(img.data.format, "png");
    assert_eq!(img.data.pixel_size, Some((1, 1)));
    let fields: Vec<&Field> = p.runs.iter().filter_map(|r| if let Inline::Field(f) = &r.content { Some(f) } else { None }).collect();
    assert_eq!(fields[0], &Field::PageNumber);
    assert_eq!(fields[1], &Field::PageCount);
    assert_eq!(fields[2], &Field::Date("d MMMM yyyy".into()));
    assert_eq!(fields[3], &Field::Other { instr: "AUTHOR".into(), result: "Ada".into() });

    let p = find_para(&back, "Visit");
    assert_eq!(p.runs[0].content, Inline::Bookmark("target_mark".into()));
    assert_eq!(find_run(&back, "example").props.link.as_deref(), Some("https://example.com/a?b=1&c=2"));
    assert_eq!(find_run(&back, "inside").props.link.as_deref(), Some("#target_mark"));
    let foot = p.runs.iter().find_map(|r| if let Inline::FootnoteRef(i) = r.content { Some(i) } else { None }).unwrap();
    let end = p.runs.iter().find_map(|r| if let Inline::EndnoteRef(i) = r.content { Some(i) } else { None }).unwrap();
    assert_eq!(back.footnotes.len(), 1);
    assert_eq!(back.footnotes[0].id, foot);
    assert_eq!(back.footnotes[0].paragraphs[0].text(), "The footnote text.");
    assert_eq!(back.endnotes[0].id, end);
    assert_eq!(back.endnotes[0].paragraphs[0].text(), "The endnote text.");

    let p = find_para(&back, "Before page break");
    assert!(p.runs.iter().any(|r| r.content == Inline::PageBreak));
    assert!(find_para(&back, "Col").runs.iter().any(|r| r.content == Inline::ColumnBreak));
    assert_eq!(find_para(&back, "Ünïcödé").text(), "Ünïcödé — “quotes” & <angle> 日本語 😀");
}

#[test]
fn docx_roundtrip_revisions_comments() {
    let back = roundtrip(&kitchen_sink());
    let ins = find_run(&back, "inserted").props.revision.clone().unwrap();
    assert_eq!(ins.kind, RevisionKind::Insert);
    assert_eq!(ins.author, "Ada");
    assert_eq!(ins.date, "2026-03-01T10:00:00Z");
    let del = find_run(&back, "deleted").props.revision.clone().unwrap();
    assert_eq!(del.kind, RevisionKind::Delete);
    assert_eq!(del.author, "Bob");
    assert_eq!(find_run(&back, "commented").props.comments, vec![1]);
    assert_eq!(find_run(&back, " both").props.comments, vec![1, 2]);
    assert!(find_run(&back, " tail").props.comments.is_empty());
    assert_eq!(back.comments.len(), 2);
    let c1 = &back.comments[0];
    assert_eq!((c1.id, c1.author.as_str(), c1.initials.as_str()), (1, "Reviewer", "RV"));
    assert_eq!(c1.date, "2026-04-01T09:00:00Z");
    assert_eq!(c1.paragraphs[0].text(), "First comment");
    assert!(!c1.resolved);
    assert!(back.comments[1].resolved);
}

#[test]
fn docx_roundtrip_table() {
    let doc = kitchen_sink();
    let back = roundtrip(&doc);
    let t = back.body.iter().find_map(|b| b.as_table()).unwrap();
    let o = doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.columns, o.columns);
    assert_eq!(t.align, Align::Center);
    assert_eq!(t.cell_margin, 6.0);
    assert_eq!(t.borders.top, o.borders.top);
    assert_eq!(t.borders.inside_v, o.borders.inside_v);
    assert!(t.rows[0].header);
    assert_eq!(t.rows[0].height, Some(24.0));
    assert_eq!(t.rows[0].cells[1].text(), "H2");
    assert_eq!(t.rows[0].cells[0].shading, Some(Color::rgb(0xCC, 0xCC, 0xCC)));
    assert_eq!(t.rows[0].cells[0].valign, VAlign::Center);
    assert_eq!(t.rows[1].cells[0].span, 2);
    assert_eq!(t.rows[1].cells[1].vmerge, VMerge::Restart);
    assert_eq!(t.rows[1].cells[1].valign, VAlign::Bottom);
    assert_eq!(t.rows[2].cells[2].vmerge, VMerge::Continue);
    let b = t.rows[2].cells[1].borders.as_ref().unwrap().bottom.unwrap();
    assert_eq!((b.style, b.width, b.color), (BorderStyle::Double, 1.5, Color::RED));
    // Paragraph after table is preserved.
    let idx = back.body.iter().position(|b| b.as_table().is_some()).unwrap();
    assert!(back.body[idx + 1].as_para().unwrap().text().starts_with("Before page break"));
}

#[test]
fn docx_roundtrip_headers_footers() {
    let back = roundtrip(&kitchen_sink());
    let h = &back.headers;
    let htext: Vec<String> = h.header.iter().filter_map(|b| b.as_para()).map(|p| p.text()).collect();
    assert!(htext.iter().any(|t| t.starts_with("Header text\t")), "{htext:?}");
    let hp = h.header.iter().filter_map(|b| b.as_para()).find(|p| p.text().starts_with("Header")).unwrap();
    assert!(hp.runs.iter().any(|r| r.content == Inline::Field(Field::PageNumber)));
    assert_eq!(h.footer[0].as_para().unwrap().text(), "Footer text");
    assert!(h.first_header.iter().filter_map(|b| b.as_para()).any(|p| p.text() == "First header"));
    assert_eq!(h.first_footer[0].as_para().unwrap().text(), "First footer");
}

#[test]
fn docx_roundtrip_is_stable() {
    // A second round trip must not change anything.
    let once = roundtrip(&kitchen_sink());
    let twice = roundtrip(&once);
    assert_eq!(once.body, twice.body);
    assert_eq!(once.styles, twice.styles);
    assert_eq!(once.lists, twice.lists);
    assert_eq!(once.headers, twice.headers);
    assert_eq!(once.footnotes, twice.footnotes);
    assert_eq!(once.comments, twice.comments);
    assert_eq!(once.page, twice.page);
}

#[test]
fn docx_package_is_complete() {
    let bytes = docx::write(&kitchen_sink()).unwrap();
    let pkg = io::package::Package::open(&bytes).unwrap();
    for part in [
        "[Content_Types].xml",
        "_rels/.rels",
        "word/document.xml",
        "word/_rels/document.xml.rels",
        "word/styles.xml",
        "word/numbering.xml",
        "word/settings.xml",
        "word/footnotes.xml",
        "word/endnotes.xml",
        "word/comments.xml",
        "word/header1.xml",
        "word/footer1.xml",
        "docProps/core.xml",
        "docProps/app.xml",
        "word/media/image1.png",
    ] {
        assert!(pkg.has(part), "missing {part}");
    }
    let foot = String::from_utf8(pkg.get("word/footnotes.xml").unwrap().to_vec()).unwrap();
    assert!(foot.contains(r#"w:type="separator" w:id="-1""#));
    assert!(foot.contains(r#"w:type="continuationSeparator" w:id="0""#));
    assert!(foot.contains(r#"<w:footnote w:id="1">"#));
    // Every XML part must be well-formed.
    for name in pkg.names().filter(|n| n.ends_with(".xml") || n.ends_with(".rels")) {
        let s = String::from_utf8(pkg.get(name).unwrap().to_vec()).unwrap();
        assert!(wordcore::io::xml::parse(&s).is_ok(), "{name}");
        assert_balanced(&s, name);
    }
}

/// Check tags are properly nested (our parser is lenient, so verify strictly here).
fn assert_balanced(s: &str, name: &str) {
    let mut stack: Vec<String> = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        let end = rest.find('>').unwrap();
        let tag = &rest[..end];
        rest = &rest[end + 1..];
        if tag.starts_with('?') || tag.starts_with('!') || tag.ends_with('/') {
            continue;
        }
        if let Some(close) = tag.strip_prefix('/') {
            let open = stack.pop().unwrap_or_else(|| panic!("{name}: unexpected </{close}>"));
            assert_eq!(open, close, "{name}: mismatched tags");
        } else {
            stack.push(tag.split_whitespace().next().unwrap().to_string());
        }
    }
    assert!(stack.is_empty(), "{name}: unclosed {stack:?}");
}

// ---------- reading real-world-shaped packages ----------

fn make_docx(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (n, c) in parts {
        z.start_file(*n, opts).unwrap();
        z.write_all(c.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:v="urn:schemas-microsoft-com:vml""#;

fn minimal(body: &str, extra: &[(&str, &str)], rels: &str) -> Vec<u8> {
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8"?><w:document {W}><w:body>{body}</w:body></w:document>"#);
    let doc_rels = format!(r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#);
    let mut parts = vec![
        ("[Content_Types].xml", r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#.to_string()),
        (
            "_rels/.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string(),
        ),
        ("word/document.xml", doc),
        ("word/_rels/document.xml.rels", doc_rels),
    ];
    for (n, c) in extra {
        parts.push((n, c.to_string()));
    }
    let refs: Vec<(&str, &str)> = parts.iter().map(|(n, c)| (*n, c.as_str())).collect();
    make_docx(&refs)
}

#[test]
fn read_complex_fields_and_toc() {
    let body = r#"
<w:p><w:pPr><w:pStyle w:val="TOC1"/><w:tabs><w:tab w:val="right" w:leader="dot" w:pos="9350"/></w:tabs></w:pPr>
  <w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \h \z \u </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>
  <w:hyperlink w:anchor="_Toc1" w:history="1"><w:r><w:t>Intro</w:t></w:r><w:r><w:tab/></w:r>
    <w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGEREF _Toc1 \h </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>2</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>
  </w:hyperlink></w:p>
<w:p><w:r><w:t>Second entry</w:t></w:r></w:p>
<w:p><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Page </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>PAGE</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>4</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>
<w:fldSimple w:instr=" NUMPAGES "><w:r><w:t>9</w:t></w:r></w:fldSimple>
<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> HYPERLINK "http://rust-lang.org" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>rust</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>
<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TIME </w:instrText></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>
</w:p>"#;
    let doc = docx::read(&minimal(body, &[], "")).unwrap();
    let paras = body_paras(&doc);
    assert_eq!(paras[0].text(), "Intro\t");
    assert_eq!(paras[0].runs[0].props.link.as_deref(), Some("#_Toc1"));
    let pageref = paras[0].runs.iter().find_map(|r| if let Inline::Field(f) = &r.content { Some(f.clone()) } else { None }).unwrap();
    assert_eq!(pageref, Field::Other { instr: r"PAGEREF _Toc1 \h".into(), result: "2".into() });
    assert_eq!(paras[0].props.tabs[0].leader, TabLeader::Dot);
    assert_eq!(paras[0].props.tabs[0].pos, 467.5);
    assert_eq!(paras[1].text(), "Second entry");
    let p = paras[3];
    assert_eq!(p.text(), "Page rust");
    let fields: Vec<&Inline> = p.runs.iter().map(|r| &r.content).filter(|c| matches!(c, Inline::Field(_))).collect();
    assert_eq!(fields, vec![&Inline::Field(Field::PageNumber), &Inline::Field(Field::PageCount), &Inline::Field(Field::Date("h:mm am/pm".into()))]);
    assert_eq!(find_run(&doc, "rust").props.link.as_deref(), Some("http://rust-lang.org"));
}

#[test]
fn read_styles_numbering_theme_and_defaults() {
    let styles = r#"<?xml version="1.0"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:asciiTheme="minorHAnsi" w:hAnsiTheme="minorHAnsi"/><w:sz w:val="24"/></w:rPr></w:rPrDefault>
<w:pPrDefault><w:pPr><w:spacing w:after="120" w:line="276" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>
<w:style w:type="paragraph" w:styleId="berschrift1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:rFonts w:asciiTheme="majorHAnsi"/><w:b/><w:sz w:val="40"/></w:rPr></w:style>
<w:style w:type="table" w:styleId="Gitter"><w:name w:val="Table Grid"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:color="auto"/><w:insideV w:val="single" w:sz="4"/></w:tblBorders></w:tblPr></w:style>
<w:style w:type="numbering" w:styleId="NoList"><w:name w:val="No List"/></w:style>
</w:styles>"#;
    let numbering = r#"<?xml version="1.0"?><w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">
<w:abstractNum w:abstractNumId="3"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="&#xF0B7;"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr><w:rPr><w:rFonts w:ascii="Symbol" w:hAnsi="Symbol" w:hint="default"/></w:rPr></w:lvl>
<w:lvl w:ilvl="1"><w:start w:val="1"/><mc:AlternateContent><mc:Choice Requires="w14"><w:numFmt w:val="custom" w:format="01, 02, 03, ..."/></mc:Choice><mc:Fallback><w:numFmt w:val="decimalZero"/></mc:Fallback></mc:AlternateContent><w:lvlText w:val="%2)"/><w:pPr><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>
<w:num w:numId="5"><w:abstractNumId w:val="3"/><w:lvlOverride w:ilvl="1"><w:startOverride w:val="4"/></w:lvlOverride></w:num></w:numbering>"#;
    let theme = r#"<?xml version="1.0"?><a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:fontScheme name="x"><a:majorFont><a:latin typeface="Aptos Display"/></a:majorFont><a:minorFont><a:latin typeface="Aptos"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#;
    let rels = r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/>"#;
    let body = r#"<w:p><w:pPr><w:pStyle w:val="berschrift1"/></w:pPr><w:r><w:t>Titel</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="5"/></w:numPr><w:ind w:start="100" w:hanging="50"/><w:jc w:val="both"/></w:pPr><w:r><w:t>Item</w:t></w:r></w:p>
<w:tbl><w:tblPr><w:tblStyle w:val="Gitter"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="3000"/></w:tblGrid>
<w:tr><w:tc><w:p><w:r><w:t>a</w:t></w:r></w:p></w:tc><w:sdt><w:sdtContent><w:tc><w:tcPr><w:shd w:val="clear" w:fill="FF0000"/></w:tcPr><w:p><w:r><w:t>b</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>n1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>n2</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:tc></w:sdtContent></w:sdt></w:tr></w:tbl>
<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1417" w:right="1417" w:bottom="1134" w:left="1417" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>"#;
    let bytes = minimal(body, &[("word/styles.xml", styles), ("word/numbering.xml", numbering), ("word/theme/theme1.xml", theme)], rels);
    let doc = docx::read(&bytes).unwrap();
    assert_eq!(doc.default_chars.font.as_deref(), Some("Aptos"));
    assert_eq!(doc.default_chars.size, Some(12.0));
    assert_eq!(doc.default_para.space_after, Some(6.0));
    assert_eq!(doc.default_para.line_spacing, Some(LineSpacing::Multiple(1.15)));
    // Localised style id maps to the built-in Heading1.
    let p0 = doc.body[0].as_para().unwrap();
    assert_eq!(p0.props.style.as_deref(), Some("Heading1"));
    let h1 = doc.style("Heading1").unwrap();
    assert_eq!(h1.name, "Heading 1");
    assert_eq!(h1.chars.font.as_deref(), Some("Aptos Display"));
    assert_eq!(h1.chars.size, Some(20.0));
    assert_eq!(h1.chars.bold, Some(true));
    assert!(doc.style("berschrift1").is_none());
    // Numbering.
    let l = doc.list(5).unwrap();
    assert_eq!(l.levels[0].format, NumFormat::Bullet);
    assert_eq!(l.levels[0].text, "•");
    assert_eq!(l.levels[0].font, None);
    assert_eq!(l.levels[0].indent, 36.0);
    assert_eq!(l.levels[0].hanging, 18.0);
    assert_eq!(l.levels[1].format, NumFormat::DecimalZero);
    assert_eq!(l.levels[1].text, "%2)");
    assert_eq!(l.levels[1].start, 4);
    let p1 = doc.body[1].as_para().unwrap();
    assert_eq!(p1.props.list, Some(Some(ListRef { list: 5, level: 1 })));
    assert_eq!(p1.props.indent_left, Some(5.0));
    assert_eq!(p1.props.indent_first, Some(-2.5));
    assert_eq!(p1.props.align, Some(Align::Justify));
    // Table borders come from the (aliased) table style; nested table flattened; sdt-wrapped cell kept.
    let t = doc.body[2].as_table().unwrap();
    assert_eq!(t.style.as_deref(), Some("TableGrid"));
    assert_eq!(t.columns, vec![100.0, 150.0]);
    assert!(t.borders.top.unwrap().is_visible());
    assert!(t.borders.inside_v.is_some());
    assert_eq!(t.rows[0].cells.len(), 2);
    assert_eq!(t.rows[0].cells[1].shading, Some(Color::rgb(255, 0, 0)));
    assert_eq!(t.rows[0].cells[1].text(), "b\nn1\tn2");
    // A4 page.
    assert!((doc.page.width - 595.3).abs() < 0.1);
    assert!((doc.page.margin_bottom - 56.7).abs() < 0.1);
}

#[test]
fn read_drawings_alternate_content_and_vml() {
    let png = wordcore::io::util::base64_decode(PNG_1X1).unwrap();
    let body = r#"<w:p><w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><wp:anchor><wp:extent cx="1270000" cy="635000"/><wp:docPr id="1" name="x" descr="anchored"/><a:graphic><a:graphicData><pic:pic><pic:blipFill><a:blip r:embed="rIdImg"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:anchor></w:drawing></mc:Choice><mc:Fallback><w:pict/></mc:Fallback></mc:AlternateContent></w:r>
<w:r><w:pict><v:shape style="width:36pt;height:0.5in"><v:imagedata r:id="rIdImg" o:title="vml"/></v:shape></w:pict></w:r>
<w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><wp:anchor><wp:extent cx="1" cy="1"/><a:graphic><a:graphicData><wps:wsp><wps:txbx><w:txbxContent><w:p><w:r><w:t>In a text box</w:t></w:r></w:p></w:txbxContent></wps:txbx></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></mc:Choice><mc:Fallback><w:pict><v:shape><v:textbox><w:txbxContent><w:p><w:r><w:t>In a text box</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></mc:Fallback></mc:AlternateContent></w:r>
<w:smartTag><w:r><w:t>smart</w:t></w:r></w:smartTag><w:sdt><w:sdtContent><w:r><w:t>sdt</w:t></w:r></w:sdtContent></w:sdt>
<w:r><w:sym w:font="Wingdings" w:char="F0FC"/></w:r></w:p>"#;
    let rels = r#"<Relationship Id="rIdImg" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/pic.png"/>"#;
    let mut parts: Vec<(&str, String)> = vec![];
    let doc_bytes = minimal(body, &[], rels);
    // add the image to the zip
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut src = zip::ZipArchive::new(std::io::Cursor::new(doc_bytes)).unwrap();
    for i in 0..src.len() {
        let mut f = src.by_index(i).unwrap();
        let mut s = String::new();
        std::io::Read::read_to_string(&mut f, &mut s).unwrap();
        parts.push((Box::leak(f.name().unwrap().to_string().into_boxed_str()), s));
    }
    for (n, c) in &parts {
        z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(c.as_bytes()).unwrap();
    }
    z.start_file("word/media/pic.png", zip::write::SimpleFileOptions::default()).unwrap();
    z.write_all(&png).unwrap();
    let bytes = z.finish().unwrap().into_inner();
    let doc = docx::read(&bytes).unwrap();
    let p = doc.body[0].as_para().unwrap();
    let imgs: Vec<&Image> = p.runs.iter().filter_map(|r| if let Inline::Image(i) = &r.content { Some(i) } else { None }).collect();
    assert_eq!(imgs.len(), 2);
    assert_eq!((imgs[0].width, imgs[0].height, imgs[0].alt.as_str()), (100.0, 50.0, "anchored"));
    assert_eq!((imgs[1].width, imgs[1].height), (36.0, 36.0));
    // Both images share the same decoded data.
    assert!(std::sync::Arc::ptr_eq(&imgs[0].data, &imgs[1].data));
    assert!(p.text().contains("smartsdt✓"));
    // Text box content follows the paragraph.
    assert_eq!(doc.body[1].as_para().unwrap().text(), "In a text box");
    assert_eq!(doc.body.len(), 2);
}

#[test]
fn read_notes_comments_revisions_and_sections() {
    let body = r#"<w:p><w:pPr><w:sectPr><w:type w:val="continuous"/></w:sectPr></w:pPr><w:r><w:t>One</w:t></w:r><w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr><w:footnoteReference w:id="2"/></w:r></w:p>
<w:p><w:commentRangeStart w:id="0"/><w:r><w:t>Two</w:t></w:r><w:ins w:id="1" w:author="A" w:date="2024-01-01T00:00:00Z"><w:r><w:t>new</w:t></w:r></w:ins><w:del w:id="2" w:author="B"><w:r><w:delText>old</w:delText></w:r></w:del><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r></w:p>
<w:p><w:pPr><w:sectPr><w:headerReference w:type="default" r:id="rH1"/><w:pgSz w:w="12240" w:h="15840"/></w:sectPr></w:pPr><w:r><w:t>Three</w:t></w:r></w:p>
<w:p><w:r><w:t>Four</w:t></w:r></w:p>
<w:sectPr><w:type w:val="nextPage"/><w:pgSz w:w="15840" w:h="12240" w:orient="landscape"/><w:cols w:num="3" w:space="360"/><w:titlePg/></w:sectPr>"#;
    let footnotes = r#"<?xml version="1.0"?><w:footnotes xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:footnote w:type="separator" w:id="0"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:type="continuationSeparator" w:id="1"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote><w:footnote w:id="2"><w:p><w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr><w:footnoteRef/></w:r><w:r><w:t xml:space="preserve"> Note two.</w:t></w:r></w:p></w:footnote></w:footnotes>"#;
    let comments = r#"<?xml version="1.0"?><w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:comment w:id="0" w:author="Zed" w:initials="Z"><w:p><w:r><w:annotationRef/></w:r><w:r><w:t>Hmm</w:t></w:r></w:p></w:comment></w:comments>"#;
    let header = r#"<?xml version="1.0"?><w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>Running head</w:t></w:r></w:p></w:hdr>"#;
    let rels = r#"<Relationship Id="rF" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/><Relationship Id="rC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/><Relationship Id="rH1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header7.xml"/>"#;
    let bytes = minimal(body, &[("word/footnotes.xml", footnotes), ("word/comments.xml", comments), ("word/header7.xml", header)], rels);
    let doc = docx::read(&bytes).unwrap();
    assert_eq!(doc.footnotes.len(), 1);
    assert_eq!(doc.footnotes[0].id, 2);
    assert_eq!(doc.footnotes[0].paragraphs[0].text(), "Note two.");
    let p0 = doc.body[0].as_para().unwrap();
    assert!(p0.runs.iter().any(|r| r.content == Inline::FootnoteRef(2)));
    assert_eq!(doc.comments[0].author, "Zed");
    assert_eq!(doc.comments[0].paragraphs[0].text(), "Hmm");
    assert_eq!(find_run(&doc, "Two").props.comments, vec![0]);
    assert_eq!(find_run(&doc, "new").props.revision.as_ref().unwrap().kind, RevisionKind::Insert);
    assert_eq!(find_run(&doc, "new").props.comments, vec![0]);
    assert_eq!(find_run(&doc, "old").props.revision.as_ref().unwrap().kind, RevisionKind::Delete);
    // Section 2 ("Three") starts continuous? No: section 2's type is absent → nextPage; section 3 is nextPage.
    assert_eq!(doc.body[1].as_para().unwrap().props.page_break_before, Some(true));
    assert_eq!(doc.body[3].as_para().unwrap().props.page_break_before, Some(true));
    assert_eq!(doc.body[2].as_para().unwrap().props.page_break_before, None);
    // Final section's page setup, header inherited from the earlier section.
    assert_eq!(doc.page.orientation(), Orientation::Landscape);
    assert_eq!(doc.page.columns, 3);
    assert_eq!(doc.page.column_gap, 18.0);
    assert!(doc.page.title_page);
    assert_eq!(doc.headers.header[0].as_para().unwrap().text(), "Running head");
}

#[test]
fn read_rejects_garbage() {
    assert!(docx::read(b"not a zip").is_err());
    let bytes = make_docx(&[("foo.txt", "hi")]);
    assert!(docx::read(&bytes).is_err());
}

#[test]
fn format_dispatch_and_sniffing() {
    use std::path::Path;
    assert_eq!(Format::from_path(Path::new("a.DOCX")), Some(Format::Docx));
    assert_eq!(Format::from_path(Path::new("a.dotx")), Some(Format::Docx));
    assert_eq!(Format::from_path(Path::new("a.htm")), Some(Format::Html));
    assert_eq!(Format::from_path(Path::new("a.markdown")), Some(Format::Markdown));
    assert_eq!(Format::from_path(Path::new("a.xyz")), None);
    let bytes = docx::write(&Document::from_text("hello")).unwrap();
    assert_eq!(Format::sniff(&bytes), Format::Docx);
    assert_eq!(Format::sniff(b"{\\rtf1\\ansi hi}"), Format::Rtf);
    assert_eq!(Format::sniff(b"\n<!DOCTYPE html><p>x"), Format::Html);
    assert_eq!(Format::sniff(b"plain"), Format::Txt);
    // open() falls back to sniffing when the extension lies.
    let dir = scratch_dir().join("dispatch");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("really_docx.txt");
    std::fs::write(&p, &bytes).unwrap();
    // .txt with zip content → sniffed as docx
    let d = io::open(&p).unwrap();
    assert_eq!(d.plain_text(), "hello");
    let p2 = dir.join("x.docx");
    io::save(&d, &p2).unwrap();
    assert_eq!(io::open(&p2).unwrap().plain_text(), "hello");
}

/// Validate with LibreOffice when available: the written file must load, render the text, and convert back.
#[test]
fn docx_loads_in_libreoffice() {
    if !soffice_available() {
        eprintln!("soffice not available; skipping");
        return;
    }
    let dir = scratch_dir().join("docx_lo");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("kitchen.docx");
    std::fs::write(&path, docx::write(&kitchen_sink()).unwrap()).unwrap();
    let pdf = soffice_convert(&path, "pdf", &dir.join("pdf")).expect("LibreOffice could not convert the docx");
    if let Some(text) = pdf_text(&pdf) {
        for needle in ["The Title", "Chapter One", "Fancy paragraph", "Span two", "Header text", "The footnote text", "Ünïcödé"] {
            assert!(text.contains(needle), "PDF lacks {needle:?}");
        }
    }
    // LibreOffice re-saves as docx; our reader must handle its output.
    let lo_docx = soffice_convert(&path, "docx:MS Word 2007 XML", &dir.join("resaved")).expect("re-save");
    let back = io::open(&lo_docx).unwrap();
    assert!(back.plain_text().contains("Fancy paragraph"));
    assert!(back.plain_text().contains("Span two"));
    assert_eq!(find_run(&back, "bold").props.bold, Some(true));
    assert_eq!(find_para(&back, "Chapter One").props.style.as_deref(), Some("Heading1"));
    assert!(back.body.iter().any(|b| b.as_table().is_some()));
    assert_eq!(back.footnotes.len(), 1);
    assert_eq!(back.comments.len(), 2);
    assert!(body_paras(&back).iter().any(|p| p.runs.iter().any(|r| matches!(r.content, Inline::Image(_)))));
    let b2 = find_para(&back, "Bullet two");
    assert!(matches!(b2.props.list, Some(Some(_))));
    assert_eq!(back.page.orientation(), Orientation::Landscape);
}
