//! Shared fixtures for the io tests.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use wordcore::io::util::{base64_decode, image_data};
use wordcore::*;

/// A valid 1×1 PNG.
pub const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

pub fn png() -> Arc<ImageData> {
    image_data(base64_decode(PNG_1X1).unwrap())
}

pub fn run(text: &str, f: impl FnOnce(&mut CharProps)) -> Run {
    let mut p = CharProps::default();
    f(&mut p);
    Run::styled(text, p)
}

pub fn inline(content: Inline) -> Run {
    Run { content, props: CharProps::default() }
}

pub fn para(runs: Vec<Run>) -> Paragraph {
    Paragraph { runs, ..Default::default() }
}

pub fn scratch_dir() -> PathBuf {
    let d = PathBuf::from("/tmp/claude-io-scratch");
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub fn soffice_available() -> bool {
    std::process::Command::new("soffice").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// Convert `file` with LibreOffice; returns the output path on success.
pub fn soffice_convert(file: &std::path::Path, to: &str, outdir: &std::path::Path) -> Option<PathBuf> {
    std::fs::create_dir_all(outdir).ok()?;
    // A private profile directory avoids clashes between concurrently running tests.
    let profile = outdir.join("lo-profile");
    let status = std::process::Command::new("soffice")
        .arg(format!("-env:UserInstallation=file://{}", profile.display()))
        .args(["--headless", "--convert-to", to, "--outdir"])
        .arg(outdir)
        .arg(file)
        .output()
        .ok()?;
    let ext = to.split(':').next().unwrap();
    let out = outdir.join(file.file_stem()?).with_extension(ext);
    if status.status.success() && out.exists() && std::fs::metadata(&out).ok()?.len() > 0 {
        Some(out)
    } else {
        eprintln!("soffice failed: {}", String::from_utf8_lossy(&status.stderr));
        None
    }
}

/// Extract text from a PDF with pdftotext if available.
pub fn pdf_text(pdf: &std::path::Path) -> Option<String> {
    let out = std::process::Command::new("pdftotext").arg(pdf).arg("-").output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        None
    }
}

pub fn body_paras(doc: &Document) -> Vec<&Paragraph> {
    doc.body.iter().filter_map(|b| b.as_para()).collect()
}

/// Find the first body paragraph whose text contains `needle`.
pub fn find_para<'a>(doc: &'a Document, needle: &str) -> &'a Paragraph {
    body_paras(doc).into_iter().find(|p| p.text().contains(needle)).unwrap_or_else(|| panic!("no paragraph containing {needle:?}"))
}

/// Find the run whose text contains `needle` (searching body paragraphs).
pub fn find_run<'a>(doc: &'a Document, needle: &str) -> &'a Run {
    for p in body_paras(doc) {
        for r in &p.runs {
            if let Inline::Text(t) = &r.content {
                if t.contains(needle) {
                    return r;
                }
            }
        }
    }
    panic!("no run containing {needle:?}")
}

/// A document exercising (nearly) every model feature.
pub fn kitchen_sink() -> Document {
    let mut doc = Document::new();
    doc.properties = DocProperties {
        title: "Kitchen Sink".into(),
        subject: "Testing".into(),
        author: "Ada Lovelace".into(),
        keywords: "rust, word".into(),
        description: "All features".into(),
        last_modified_by: "Charles Babbage".into(),
        created: "2026-01-02T03:04:05Z".into(),
        modified: "2026-02-03T04:05:06Z".into(),
    };
    doc.settings.track_changes = true;
    doc.settings.default_tab = 30.0;
    doc.page = PageSetup {
        width: 841.9,
        height: 595.3,
        margin_top: 50.0,
        margin_bottom: 60.0,
        margin_left: 70.0,
        margin_right: 80.0,
        header_distance: 30.0,
        footer_distance: 25.0,
        columns: 2,
        column_gap: 24.0,
        title_page: true,
        page_number_start: 3,
        page_color: Some(Color::rgb(0xFF, 0xF8, 0xE0)),
        page_borders: Some(Borders {
            top: Some(Border { style: BorderStyle::Double, width: 1.5, color: Color::rgb(0x11, 0x22, 0x33), space: 24.0 }),
            bottom: Some(Border::single()),
            left: Some(Border::single()),
            right: Some(Border::single()),
            ..Default::default()
        }),
        watermark: Some("DRAFT".into()),
        start: SectionStart::NextPage,
    };
    doc.styles.push(Style {
        id: "MyStyle".into(),
        name: "My Style".into(),
        kind: StyleKind::Paragraph,
        based_on: Some("Normal".into()),
        next: Some("Normal".into()),
        para: ParaProps { space_before: Some(6.0), align: Some(Align::Center), ..Default::default() },
        chars: CharProps { color: Some(Color::rgb(0x80, 0, 0)), italic: Some(true), ..Default::default() },
        quick: true,
    });
    let bullets = doc.add_list(ListDef::bullets(0));
    let numbers = doc.add_list(ListDef::numbered(0));

    let mut body = vec![];
    body.push(Block::Para(Paragraph::with_style("Title", "The Title")));
    body.push(Block::Para(Paragraph::with_style("Heading1", "Chapter One")));
    body.push(Block::Para(Paragraph::with_style("Heading2", "Section 1.1")));
    body.push(Block::Para(Paragraph::with_style("MyStyle", "Custom styled paragraph")));
    body.push(Block::Para(para(vec![
        Run::text("Plain "),
        run("bold", |p| p.bold = Some(true)),
        Run::text(" "),
        run("italic", |p| p.italic = Some(true)),
        Run::text(" "),
        run("dunder", |p| p.underline = Some(Underline::Double)),
        Run::text(" "),
        run("struck", |p| p.strike = Some(true)),
        Run::text(" "),
        run("dstruck", |p| p.double_strike = Some(true)),
        Run::text(" "),
        run("red", |p| p.color = Some(Color::rgb(0xFF, 0, 0))),
        Run::text(" "),
        run("hilite", |p| p.highlight = Some(Color::rgb(255, 255, 0))),
        Run::text(" "),
        run("shaded", |p| p.shading = Some(Color::rgb(0xDD, 0xEE, 0xFF))),
        Run::text(" E=mc"),
        run("2", |p| p.vert_align = Some(VertAlign::Superscript)),
        Run::text(" H"),
        run("2", |p| p.vert_align = Some(VertAlign::Subscript)),
        Run::text("O "),
        run("caps", |p| p.caps = Some(true)),
        Run::text(" "),
        run("smallcaps", |p| p.small_caps = Some(true)),
        Run::text(" "),
        run("hidden", |p| p.hidden = Some(true)),
        Run::text(" "),
        run("spaced", |p| p.spacing = Some(1.5)),
        Run::text(" "),
        run("georgia", |p| {
            p.font = Some("Georgia".into());
            p.size = Some(14.5)
        }),
        Run::text(" "),
        run("emph", |p| p.style = Some("Emphasis".into())),
        Run::text("  two  spaces "),
    ])));
    let mut fancy = para(vec![
        Run::text("Fancy paragraph"),
        inline(Inline::Tab),
        Run::text("after tab"),
        inline(Inline::LineBreak),
        Run::text("after break"),
    ]);
    fancy.props = ParaProps {
        align: Some(Align::Justify),
        indent_left: Some(36.0),
        indent_right: Some(18.0),
        indent_first: Some(-18.0),
        space_before: Some(12.0),
        space_after: Some(6.0),
        line_spacing: Some(LineSpacing::Exactly(15.0)),
        contextual_spacing: Some(true),
        keep_next: Some(true),
        keep_lines: Some(true),
        widow_control: Some(false),
        shading: Some(Color::rgb(0xEE, 0xEE, 0xEE)),
        borders: Some(Borders {
            top: Some(Border { style: BorderStyle::Single, width: 1.0, color: Color::rgb(0, 0, 0xFF), space: 4.0 }),
            bottom: Some(Border { style: BorderStyle::Dashed, width: 0.75, color: Color::BLACK, space: 1.0 }),
            ..Default::default()
        }),
        tabs: vec![
            TabStop { pos: 144.0, align: TabAlign::Right, leader: TabLeader::Dot },
            TabStop { pos: 216.0, align: TabAlign::Center, leader: TabLeader::None },
            TabStop { pos: 288.0, align: TabAlign::Decimal, leader: TabLeader::Hyphen },
        ],
        outline_level: Some(3),
        ..Default::default()
    };
    body.push(Block::Para(fancy));
    let mut lsp = Paragraph::with_text("Line spacing 1.5 and at least");
    lsp.props.line_spacing = Some(LineSpacing::Multiple(1.5));
    lsp.props.align = Some(Align::Right);
    body.push(Block::Para(lsp));
    let mut al = Paragraph::with_text("At least 20pt");
    al.props.line_spacing = Some(LineSpacing::AtLeast(20.0));
    al.props.align = Some(Align::Center);
    al.props.indent_first = Some(24.0);
    body.push(Block::Para(al));
    body.push(Block::Para(para(vec![
        Run::text("Image: "),
        inline(Inline::Image(Image { data: png(), width: 72.0, height: 36.0, alt: "A pixel".into(), float: None })),
        Run::text(" Page "),
        inline(Inline::Field(Field::PageNumber)),
        Run::text(" of "),
        inline(Inline::Field(Field::PageCount)),
        Run::text(" date "),
        inline(Inline::Field(Field::Date("d MMMM yyyy".into()))),
        Run::text(" author "),
        inline(Inline::Field(Field::Other { instr: "AUTHOR".into(), result: "Ada".into() })),
    ])));
    body.push(Block::Para(para(vec![
        inline(Inline::Bookmark("target_mark".into())),
        Run::text("Visit "),
        run("example", |p| {
            p.link = Some("https://example.com/a?b=1&c=2".into());
            p.style = Some("Hyperlink".into())
        }),
        Run::text(" or jump "),
        run("inside", |p| p.link = Some("#target_mark".into())),
        Run::text(" note"),
        inline(Inline::FootnoteRef(7)),
        Run::text(" end"),
        inline(Inline::EndnoteRef(9)),
    ])));
    let rev_ins = Revision { kind: RevisionKind::Insert, author: "Ada".into(), date: "2026-03-01T10:00:00Z".into() };
    let rev_del = Revision { kind: RevisionKind::Delete, author: "Bob".into(), date: "2026-03-02T10:00:00Z".into() };
    body.push(Block::Para(para(vec![
        Run::text("Tracked: "),
        run("inserted", |p| p.revision = Some(rev_ins.clone())),
        Run::text(" "),
        run("deleted", |p| p.revision = Some(rev_del.clone())),
        Run::text(" "),
        run("commented", |p| p.comments = vec![1]),
        run(" both", |p| p.comments = vec![1, 2]),
        Run::text(" tail"),
    ])));
    for (i, t) in ["Bullet one", "Bullet two"].iter().enumerate() {
        let mut p = Paragraph::with_style("ListParagraph", t);
        p.props.list = Some(Some(ListRef { list: bullets, level: i as u8 }));
        body.push(Block::Para(p));
    }
    for t in ["Number one", "Number two"] {
        let mut p = Paragraph::with_style("ListParagraph", t);
        p.props.list = Some(Some(ListRef { list: numbers, level: 0 }));
        body.push(Block::Para(p));
    }
    // Table: 3 columns; header row; merged cells.
    let mut t = Table::new(3, 3, 432.0);
    t.columns = vec![100.0, 150.0, 182.0];
    t.align = Align::Center;
    t.indent = 0.0;
    t.cell_margin = 6.0;
    t.borders = Borders::all(Border { style: BorderStyle::Single, width: 1.0, color: Color::rgb(0x44, 0x44, 0x44), space: 0.0 });
    t.rows[0].header = true;
    t.rows[0].height = Some(24.0);
    for (i, c) in t.rows[0].cells.iter_mut().enumerate() {
        *c = Cell::with_text(&format!("H{}", i + 1));
        c.shading = Some(Color::rgb(0xCC, 0xCC, 0xCC));
        c.valign = VAlign::Center;
    }
    t.rows[1].cells = vec![
        Cell { span: 2, ..Cell::with_text("Span two") },
        Cell { vmerge: VMerge::Restart, valign: VAlign::Bottom, ..Cell::with_text("Tall") },
    ];
    t.rows[2].cells = vec![
        Cell::with_text("A"),
        Cell {
            borders: Some(Borders { bottom: Some(Border { style: BorderStyle::Double, width: 1.5, color: Color::RED, space: 0.0 }), ..Default::default() }),
            ..Cell::with_text("B")
        },
        Cell { vmerge: VMerge::Continue, ..Cell::default() },
    ];
    body.push(Block::Table(t));
    body.push(Block::Para(para(vec![Run::text("Before page break"), inline(Inline::PageBreak), Run::text("After page break")])));
    body.push(Block::Para(para(vec![Run::text("Col"), inline(Inline::ColumnBreak), Run::text("umn")])));
    let mut pb = Paragraph::with_text("Starts on new page");
    pb.props.page_break_before = Some(true);
    body.push(Block::Para(pb));
    body.push(Block::Para(Paragraph::with_text("Ünïcödé — “quotes” & <angle> 日本語 😀")));
    doc.body = body;

    doc.headers.header = vec![Block::Para(para(vec![Run::text("Header text"), inline(Inline::Tab), inline(Inline::Field(Field::PageNumber))]))];
    doc.headers.footer = vec![Block::Para(Paragraph::with_text("Footer text"))];
    doc.headers.first_header = vec![Block::Para(Paragraph::with_text("First header"))];
    doc.headers.first_footer = vec![Block::Para(Paragraph::with_text("First footer"))];
    doc.footnotes = vec![Note { id: 7, paragraphs: vec![Paragraph::with_text("The footnote text.")] }];
    doc.endnotes = vec![Note { id: 9, paragraphs: vec![Paragraph::with_text("The endnote text.")] }];
    doc.comments = vec![
        Comment {
            id: 1,
            author: "Reviewer".into(),
            initials: "RV".into(),
            date: "2026-04-01T09:00:00Z".into(),
            paragraphs: vec![Paragraph::with_text("First comment")],
            resolved: false,
        },
        Comment {
            id: 2,
            author: "Other".into(),
            initials: "OT".into(),
            date: "2026-04-02T09:00:00Z".into(),
            paragraphs: vec![Paragraph::with_text("Second comment")],
            resolved: true,
        },
    ];
    doc
}

/// A larger (24×12 px) PNG so rendered output shows a visible picture.
pub fn png_big() -> Arc<ImageData> {
    // Build a solid-colour PNG by hand (stored zlib blocks, no compression).
    let (w, h) = (24u32, 12u32);
    let row: Vec<u8> = std::iter::once(0u8).chain([0x20u8, 0x60, 0xC0].repeat(w as usize)).collect();
    let raw = row.repeat(h as usize);
    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn adler(data: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for &x in data {
            a = (a + x as u32) % 65521;
            b = (b + a) % 65521;
        }
        (b << 16) | a
    }
    let mut z = vec![0x78, 0x01, 1];
    let len = raw.len() as u16;
    z.extend_from_slice(&len.to_le_bytes());
    z.extend_from_slice(&(!len).to_le_bytes());
    z.extend_from_slice(&raw);
    z.extend_from_slice(&adler(&raw).to_be_bytes());
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |t: &[u8], d: &[u8]| {
        png.extend_from_slice(&(d.len() as u32).to_be_bytes());
        let mut c = t.to_vec();
        c.extend_from_slice(d);
        png.extend_from_slice(&c);
        png.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    image_data(png)
}

/// Three sections (portrait Letter with own headers; landscape A4 linked to the previous headers;
/// a continuous last section with its own headers), a Square-wrapped floating picture positioned
/// relative to the margin, a picture behind the text and a filled, bordered text box.
pub fn sections_doc() -> Document {
    let mut doc = Document::new();
    let hf = |h: &str, f: &str| HeadersFooters {
        header: vec![Block::Para(Paragraph::with_text(h))],
        footer: vec![Block::Para(Paragraph::with_text(f))],
        ..Default::default()
    };
    let letter = PageSetup { width: 612.0, height: 792.0, ..Default::default() };
    let a4_land = PageSetup { width: 841.9, height: 595.3, margin_left: 54.0, margin_right: 54.0, ..Default::default() };
    let square = Float { wrap: WrapMode::Square, h_rel: HRel::Margin, v_rel: VRel::Paragraph, x: 36.0, y: 18.0, h_align: None, v_align: None, dist: 9.0 };
    let behind = Float { wrap: WrapMode::Behind, h_rel: HRel::Page, v_rel: VRel::Page, x: 0.0, y: 0.0, h_align: Some(FloatAlign::Center), v_align: Some(FloatAlign::Center), dist: 0.0 };
    let tb = TextBox {
        paragraphs: vec![Paragraph::with_text("Box line one"), Paragraph::with_text("Box line two")],
        width: 180.0,
        height: 72.0,
        float: Some(Float { wrap: WrapMode::Square, h_rel: HRel::Margin, v_rel: VRel::Margin, x: 270.0, y: 300.0, h_align: None, v_align: None, dist: 9.0 }),
        fill: Some(Color::rgb(0xFF, 0xF2, 0xCC)),
        border: Some(Border { style: BorderStyle::Single, width: 1.5, color: Color::rgb(0xC0, 0x00, 0x00), space: 0.0 }),
        padding: 7.2,
        auto_height: false,
    };
    let filler = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.";
    let mut body = vec![Block::Para(Paragraph::with_text("Section one text"))];
    body.push(Block::Para(para(vec![
        inline(Inline::Image(Image { data: png_big(), width: 144.0, height: 72.0, alt: "Square picture".into(), float: Some(square) })),
        Run::text(filler),
        Run::text(" "),
        Run::text(filler),
    ])));
    body.push(Block::Para(para(vec![Run::text("Anchor of the box"), inline(Inline::TextBox(Box::new(tb)))])));
    let mut end1 = Paragraph::with_text("End of section one");
    end1.section = Some(Box::new(Section { page: letter, headers: Some(hf("Header S1", "Footer S1")) }));
    body.push(Block::Para(end1));
    body.push(Block::Para(para(vec![
        Run::text("Section two text"),
        inline(Inline::Image(Image { data: png_big(), width: 200.0, height: 100.0, alt: "Behind picture".into(), float: Some(behind) })),
    ])));
    let mut end2 = Paragraph::with_text("End of section two");
    end2.section = Some(Box::new(Section { page: a4_land.clone(), headers: None }));
    body.push(Block::Para(end2));
    body.push(Block::Para(Paragraph::with_text("Section three text")));
    for i in 0..60 {
        body.push(Block::Para(Paragraph::with_text(&format!("Filler line {i} of section three"))));
    }
    doc.body = body;
    doc.page = PageSetup { start: SectionStart::Continuous, columns: 2, column_gap: 18.0, ..a4_land };
    doc.headers = hf("Header S3", "Footer S3");
    doc
}

/// Check the structure of a re-read [`sections_doc`].
pub fn check_sections_doc(doc: &Document, what: &str) {
    let secs = doc.sections();
    assert_eq!(secs.len(), 2, "{what}: section count");
    let s0 = &secs[0].page;
    assert!((s0.width - 612.0).abs() < 0.5 && (s0.height - 792.0).abs() < 0.5, "{what}: section 1 page {}x{}", s0.width, s0.height);
    let s1 = &secs[1].page;
    assert!((s1.width - 841.9).abs() < 0.5 && (s1.height - 595.3).abs() < 0.5, "{what}: section 2 page {}x{}", s1.width, s1.height);
    assert!((s1.margin_left - 54.0).abs() < 0.5, "{what}: section 2 margin");
    assert_eq!(doc.page.start, SectionStart::Continuous, "{what}: last section start");
    assert_eq!(doc.page.columns, 2, "{what}: last section columns");
    let text = |b: &[Block]| b.iter().filter_map(|b| b.as_para()).map(|p| p.text()).collect::<Vec<_>>().join("|");
    assert_eq!(text(&doc.section_headers(0).header), "Header S1", "{what}: section 1 header");
    assert_eq!(text(&doc.section_headers(0).footer), "Footer S1", "{what}: section 1 footer");
    assert_eq!(text(&doc.section_headers(1).header), "Header S1", "{what}: section 2 header (linked)");
    assert_eq!(text(&doc.headers.header), "Header S3", "{what}: section 3 header");
    assert_eq!(text(&doc.headers.footer), "Footer S3", "{what}: section 3 footer");
    // Section-ending paragraphs.
    assert!(find_para(doc, "End of section one").section.is_some(), "{what}: end of section one");
    assert!(find_para(doc, "End of section two").section.is_some(), "{what}: end of section two");
    assert!(find_para(doc, "Section two text").section.is_none(), "{what}");
    // Floating pictures.
    let imgs: Vec<&Image> = body_paras(doc).iter().flat_map(|p| p.runs.iter()).filter_map(|r| if let Inline::Image(i) = &r.content { Some(i) } else { None }).collect();
    assert_eq!(imgs.len(), 2, "{what}: images");
    let sq = imgs[0].float.unwrap_or_else(|| panic!("{what}: square picture is not floating"));
    assert_eq!(sq.wrap, WrapMode::Square, "{what}");
    assert_eq!(sq.h_rel, HRel::Margin, "{what}");
    assert_eq!(sq.v_rel, VRel::Paragraph, "{what}");
    assert!((sq.x - 36.0).abs() < 0.5 && (sq.y - 18.0).abs() < 0.5, "{what}: square offsets {} {}", sq.x, sq.y);
    assert!((imgs[0].width - 144.0).abs() < 0.5 && (imgs[0].height - 72.0).abs() < 0.5, "{what}: square size");
    let bh = imgs[1].float.unwrap_or_else(|| panic!("{what}: behind picture is not floating"));
    assert_eq!(bh.wrap, WrapMode::Behind, "{what}");
    assert_eq!(bh.h_align, Some(FloatAlign::Center), "{what}");
    assert_eq!(bh.v_align, Some(FloatAlign::Center), "{what}");
    // Text box.
    let tb = body_paras(doc)
        .iter()
        .flat_map(|p| p.runs.iter())
        .find_map(|r| if let Inline::TextBox(t) = &r.content { Some(t.as_ref().clone()) } else { None })
        .unwrap_or_else(|| panic!("{what}: no text box"));
    assert_eq!(tb.paragraphs.iter().map(|p| p.text()).collect::<Vec<_>>(), vec!["Box line one", "Box line two"], "{what}: text box text");
    assert_eq!(tb.fill, Some(Color::rgb(0xFF, 0xF2, 0xCC)), "{what}: text box fill");
    let b = tb.border.unwrap_or_else(|| panic!("{what}: text box border"));
    assert_eq!(b.color, Color::rgb(0xC0, 0, 0), "{what}: border colour");
    assert!((b.width - 1.5).abs() < 0.1, "{what}: border width {}", b.width);
    assert!((tb.width - 180.0).abs() < 0.5 && (tb.height - 72.0).abs() < 0.5, "{what}: text box size {}x{}", tb.width, tb.height);
    let f = tb.float.unwrap_or_else(|| panic!("{what}: text box not floating"));
    assert_eq!((f.h_rel, f.v_rel), (HRel::Margin, VRel::Margin), "{what}: text box relative to");
    assert!((f.x - 270.0).abs() < 0.5 && (f.y - 300.0).abs() < 0.5, "{what}: text box offsets {} {}", f.x, f.y);
    assert!(!plain_has(doc, "Box line one"), "{what}: text box text leaked into the body");
}

/// Whether some top-level body paragraph's own text contains `needle`.
pub fn plain_has(doc: &Document, needle: &str) -> bool {
    body_paras(doc).iter().any(|p| p.text().contains(needle))
}

/// Render a PDF page to PNG (for manual inspection).
pub fn pdf_to_png(pdf: &std::path::Path, prefix: &std::path::Path) {
    let _ = std::process::Command::new("pdftoppm").args(["-png", "-r", "50"]).arg(pdf).arg(prefix).output();
}

/// Page sizes (points) of a PDF via pdfinfo.
pub fn pdf_page_sizes(pdf: &std::path::Path) -> Vec<(f32, f32)> {
    let Ok(out) = std::process::Command::new("pdfinfo").args(["-f", "1", "-l", "50"]).arg(pdf).output() else { return vec![] };
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines()
        .filter(|l| l.starts_with("Page") && l.contains("size:"))
        .filter_map(|l| {
            let rest = l.split("size:").nth(1)?;
            let mut it = rest.split_whitespace();
            let w = it.next()?.parse().ok()?;
            it.next();
            let h = it.next()?.parse().ok()?;
            Some((w, h))
        })
        .collect()
}
