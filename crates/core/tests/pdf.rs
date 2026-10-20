use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use wordcore::fonts::FontDb;
use wordcore::layout::{layout, LayoutOptions};
use wordcore::pdf::{export, export_pages};
use wordcore::*;

fn lorem(n: usize) -> String {
    let words = ["lorem", "ipsum", "dolor", "sit", "amet", "consectetur", "adipiscing", "elit", "sed", "do", "eiusmod", "tempor"];
    (0..n).map(|i| words[i % words.len()]).collect::<Vec<_>>().join(" ")
}

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

/// A small RGBA PNG: red→blue gradient with a transparent band.
fn make_png(w: u32, h: u32) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(if y % 2 == 0 { 1 } else { 4 }); // exercise Sub and Paeth filters
        let mut row = Vec::new();
        for x in 0..w {
            let a = if (h / 3..h / 3 + 3).contains(&y) { 0 } else { 255 };
            row.extend_from_slice(&[(255 * x / w) as u8, 64, (255 * y / h) as u8, a]);
        }
        // Apply the filter we announced.
        let prev: Vec<u8> = if y == 0 {
            vec![0; row.len()]
        } else {
            (0..row.len())
                .map(|i| {
                    let x = (i / 4) as u32;
                    let k = i % 4;
                    let yy = y - 1;
                    let a = if (h / 3..h / 3 + 3).contains(&yy) { 0 } else { 255 };
                    [(255 * x / w) as u8, 64, (255 * yy / h) as u8, a][k]
                })
                .collect()
        };
        for i in 0..row.len() {
            let a = if i >= 4 { row[i - 4] } else { 0 };
            let b = prev[i];
            let c = if i >= 4 { prev[i - 4] } else { 0 };
            let pred = if y % 2 == 0 {
                a
            } else {
                let p = a as i16 + b as i16 - c as i16;
                let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
                if pa <= pb && pa <= pc {
                    a
                } else if pb <= pc {
                    b
                } else {
                    c
                }
            };
            raw.push(row[i].wrapping_sub(pred));
        }
    }
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |ty: &[u8], body: &[u8]| {
        png.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let mut c = ty.to_vec();
        c.extend_from_slice(body);
        png.extend_from_slice(&c);
        png.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6));
    chunk(b"IEND", &[]);
    png
}

fn props(b: impl FnOnce(&mut CharProps)) -> CharProps {
    let mut p = CharProps::default();
    b(&mut p);
    p
}

/// A document exercising most shape kinds.
fn rich_doc() -> Document {
    let mut doc = Document::new();
    doc.properties.title = "Test Report — Ünïcode".into();
    doc.properties.author = "Jane (Doe)".into();
    doc.properties.created = "2024-05-06T07:08:09Z".into();
    doc.page.watermark = Some("DRAFT".into());
    let mut body = vec![Block::Para(Paragraph::with_style("Heading1", "Quarterly Report"))];
    let mut p = Paragraph::with_text("Plain, ");
    p.runs.push(Run::styled("bold", props(|p| p.bold = Some(true))));
    p.runs.push(Run::text(", "));
    p.runs.push(Run::styled("italic", props(|p| p.italic = Some(true))));
    p.runs.push(Run::text(", "));
    p.runs.push(Run::styled("red spaced", props(|p| {
        p.color = Some(Color::RED);
        p.spacing = Some(2.0);
    })));
    p.runs.push(Run::text(" and a "));
    p.runs.push(Run::styled("hyperlink", props(|p| {
        p.link = Some("https://example.com/a(b)".into());
        p.color = Some(Color::BLUE);
        p.underline = Some(Underline::Single);
    })));
    p.runs.push(Run::text(". Underlines: "));
    for (name, u) in [("dotted", Underline::Dotted), ("dashed", Underline::Dashed), ("double", Underline::Double), ("wavy", Underline::Wavy)] {
        p.runs.push(Run::styled(name, props(|p| p.underline = Some(u))));
        p.runs.push(Run::text(" "));
    }
    p.runs.push(Run::styled("highlighted", props(|p| p.highlight = Some(Color::YELLOW))));
    body.push(Block::Para(p));
    let mut t = Table::new(3, 3, 468.0);
    for (r, row) in t.rows.iter_mut().enumerate() {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            *cell = Cell::with_text(&format!("Cell {}{}", (b'A' + r as u8) as char, c + 1));
        }
    }
    t.rows[0].cells[0].shading = Some(Color::rgb(0xDD, 0xEB, 0xF7));
    body.push(Block::Table(t));
    let png = make_png(40, 30);
    let mut ip = Paragraph::with_text("Image: ");
    ip.runs.push(Run {
        content: Inline::Image(Image {
            data: Arc::new(ImageData { bytes: png, format: "png".into(), pixel_size: Some((40, 30)) }),
            width: 120.0,
            height: 90.0,
            alt: String::new(),
            float: None,
        }),
        props: CharProps::default(),
    });
    body.push(Block::Para(ip));
    for _ in 0..12 {
        body.push(Block::Para(Paragraph::with_text(&lorem(120))));
    }
    body.push(Block::Para(Paragraph::with_text("The final paragraph.")));
    doc.body = body;
    doc
}

fn has(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn count(hay: &[u8], needle: &str) -> usize {
    hay.windows(needle.len()).filter(|w| *w == needle.as_bytes()).count()
}

fn tool(name: &str) -> bool {
    Command::new(name).arg("-v").output().is_ok() || Command::new(name).arg("--version").output().is_ok()
}

fn out_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("rword-pdf-test-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn check_structure(pdf: &[u8], pages: usize) {
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(pdf.ends_with(b"%%EOF\n"));
    assert_eq!(count(pdf, "/Type /Page "), pages);
    assert!(has(pdf, &format!("/Count {pages}")));
    // startxref points at the xref table.
    let tail = std::str::from_utf8(&pdf[pdf.len() - 40..]).unwrap_or("");
    let off: usize = tail.split("startxref").nth(1).unwrap().split_whitespace().next().unwrap().parse().unwrap();
    assert!(pdf[off..].starts_with(b"xref"));
    // Every xref offset points at its object.
    let xref = std::str::from_utf8(&pdf[off..]).unwrap();
    for (i, l) in xref.lines().skip(3).take_while(|l| l.ends_with(" n")).enumerate() {
        let o: usize = l[..10].parse().unwrap();
        assert!(pdf[o..].starts_with(format!("{} 0 obj", i + 1).as_bytes()), "object {} at {o}", i + 1);
    }
}

#[test]
fn plain_text_pdf() {
    let db = FontDb::load(false);
    let doc = Document::from_text("Hello world");
    let l = layout(&doc, &db, &LayoutOptions::new());
    let pdf = export(&l, &db, &doc.properties);
    check_structure(&pdf, 1);
    assert!(has(&pdf, "/Subtype /Type0"));
    assert!(has(&pdf, "/CIDFontType2"));
    assert!(has(&pdf, "/FontFile2"));
    assert!(has(&pdf, "/ToUnicode"));
    assert!(has(&pdf, "/Creator (RWord)"));
    // Subsetted: a single font well under the size of Carlito (~600 KB).
    assert!(pdf.len() < 60_000, "size {}", pdf.len());
    if tool("pdftotext") {
        let p = out_dir().join("plain.pdf");
        std::fs::write(&p, &pdf).unwrap();
        let o = Command::new("pdftotext").arg(&p).arg("-").output().unwrap();
        assert!(String::from_utf8_lossy(&o.stdout).contains("Hello world"));
    }
}

#[test]
fn rich_document_pdf() {
    let db = FontDb::load(false);
    let doc = rich_doc();
    let l = layout(&doc, &db, &LayoutOptions::new());
    assert!(l.pages.len() >= 2);
    let pdf = export(&l, &db, &doc.properties);
    check_structure(&pdf, l.pages.len());
    assert!(has(&pdf, "/Subtype /Link"));
    assert!(has(&pdf, "/URI (https://example.com/a\\(b\\))"));
    assert!(has(&pdf, "/Subtype /Image"));
    assert!(has(&pdf, "/SMask"));
    assert!(has(&pdf, "/Author (Jane \\(Doe\\))"));
    assert!(has(&pdf, "/Title <FEFF"));
    assert!(has(&pdf, "/CreationDate (D:20240506070809Z)"));
    // Regular, bold and italic Carlito are three separate fonts.
    assert!(count(&pdf, "/Subtype /Type0") >= 3);

    let dir = out_dir();
    let path = dir.join("rich.pdf");
    std::fs::write(&path, &pdf).unwrap();
    if tool("qpdf") {
        let o = Command::new("qpdf").arg("--check").arg(&path).output().unwrap();
        assert!(o.status.success(), "qpdf: {}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    }
    if tool("pdftotext") {
        let o = Command::new("pdftotext").arg("-layout").arg(&path).arg("-").output().unwrap();
        let text = String::from_utf8_lossy(&o.stdout);
        for want in ["Quarterly Report", "bold", "italic", "hyperlink", "Cell B2", "The final paragraph."] {
            assert!(text.contains(want), "missing {want:?} in {text}");
        }
        let o = Command::new("pdfinfo").arg(&path).output();
        if let Ok(o) = o {
            let info = String::from_utf8_lossy(&o.stdout);
            assert!(info.contains("Test Report — Ünïcode"), "{info}");
            assert!(info.contains("Jane (Doe)"));
        }
    }
    if tool("pdftoppm") {
        let o = Command::new("pdftoppm").args(["-r", "40", "-f", "1", "-l", "1", "-png"]).arg(&path).arg(dir.join("rich")).output().unwrap();
        assert!(o.status.success());
        assert!(o.stderr.is_empty(), "pdftoppm: {}", String::from_utf8_lossy(&o.stderr));
    }
}

#[test]
fn page_range() {
    let db = FontDb::load(false);
    let doc = rich_doc();
    let l = layout(&doc, &db, &LayoutOptions::new());
    let pdf = export_pages(&l, &db, &doc.properties, &[1]);
    check_structure(&pdf, 1);
    // The image is on page 1 only.
    assert!(!has(&pdf, "/Subtype /Image"));
    let pdf = export_pages(&l, &db, &doc.properties, &[]);
    check_structure(&pdf, 1);
    let pdf = export_pages(&l, &db, &doc.properties, &[0, 99]);
    check_structure(&pdf, 1);
}

/// Shapes the document model can't easily produce: every line style,
/// synthetic bold/italic, a mailto link, a rotated watermark and an
/// undecodable image (drawn as a placeholder).
#[test]
fn hand_built_layout() {
    use wordcore::layout::{DocLayout, LineStyle, Page, Shape};
    let db = FontDb::load(false);
    let face = db.face_for("Arial", false, false);
    let text = |y: f32, t: &str, fake_bold: bool, fake_italic: bool, link: Option<&str>| Shape::Text {
        x: 72.0,
        y,
        text: t.into(),
        face,
        size: 12.0,
        color: Color::BLACK,
        spacing: 0.0,
        fake_bold,
        fake_italic,
        link: link.map(Into::into),
    };
    let mut shapes = vec![
        Shape::Watermark { cx: 300.0, cy: 400.0, text: "SAMPLE".into(), face, size: 72.0, color: Color::rgb(0xD9, 0xD9, 0xD9), angle: 30.0 },
        Shape::Rect { x: 72.0, y: 60.0, w: 200.0, h: 20.0, color: Color::YELLOW },
        text(100.0, "Synthetic bold", true, false, None),
        text(120.0, "Synthetic italic", false, true, None),
        text(140.0, "mail me", false, false, Some("mailto:someone@example.com")),
        text(160.0, "not a web link", false, false, Some("#bookmark")),
        Shape::Image {
            x: 72.0,
            y: 300.0,
            w: 50.0,
            h: 40.0,
            image: Arc::new(ImageData { bytes: b"not an image".to_vec(), format: "gif".into(), pixel_size: None }),
        },
    ];
    for (i, st) in [LineStyle::Solid, LineStyle::Dotted, LineStyle::Dashed, LineStyle::Double, LineStyle::Wavy].into_iter().enumerate() {
        let y = 200.0 + i as f32 * 15.0;
        shapes.push(Shape::Line { x1: 72.0, y1: y, x2: 300.0, y2: y, width: 1.0, color: Color::RED, style: st });
    }
    shapes.push(Shape::Line { x1: 350.0, y1: 200.0, x2: 450.0, y2: 260.0, width: 0.75, color: Color::BLUE, style: LineStyle::Wavy });
    let page = Page {
        index: 0,
        number: 1,
        width: 595.0,
        height: 842.0,
        shapes,
        lines: Vec::new(),
        body_top: 72.0,
        body_bottom: 770.0,
        comment_marks: Vec::new(),
    };
    let l = DocLayout { pages: vec![page], ..Default::default() };
    let pdf = export(&l, &db, &DocProperties::default());
    check_structure(&pdf, 1);
    assert!(has(&pdf, "/MediaBox [0 0 595 842 ]"));
    assert_eq!(count(&pdf, "/Subtype /Link"), 1);
    assert!(has(&pdf, "/URI (mailto:someone@example.com)"));
    assert!(!has(&pdf, "/Subtype /Image"));
    let path = out_dir().join("hand.pdf");
    std::fs::write(&path, &pdf).unwrap();
    if tool("qpdf") {
        let o = Command::new("qpdf").arg("--check").arg(&path).output().unwrap();
        assert!(o.status.success(), "qpdf: {}", String::from_utf8_lossy(&o.stdout));
        // Content streams are compressed; inspect them decompressed.
        let q = out_dir().join("hand-qdf.pdf");
        let o = Command::new("qpdf").args(["--qdf", "--object-streams=disable"]).arg(&path).arg(&q).output().unwrap();
        assert!(o.status.success());
        let raw = std::fs::read(&q).unwrap();
        assert!(has(&raw, " w 2 Tr"), "fake bold");
        assert!(has(&raw, "1 0 0.2 1 72 "), "fake italic");
        assert!(has(&raw, "re B"), "image placeholder");
        assert!(has(&raw, " c\n"), "wavy curve");
    }
    if tool("pdftotext") {
        // -raw keeps the rotated watermark text in the output.
        let o = Command::new("pdftotext").arg("-raw").arg(&path).arg("-").output().unwrap();
        let t = String::from_utf8_lossy(&o.stdout);
        for want in ["Synthetic bold", "Synthetic italic", "mail me", "SAMPLE"] {
            assert!(t.contains(want), "missing {want:?}: {t}");
        }
    }
}
