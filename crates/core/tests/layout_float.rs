//! Layout of sections, floating pictures and text boxes.

use std::sync::Arc;

use wordcore::fonts::FontDb;
use wordcore::layout::{layout, DocLayout, LayoutOptions, Shape};
use wordcore::*;

fn db() -> FontDb {
    FontDb::load(false)
}

fn lorem(n: usize) -> String {
    let words = ["lorem", "ipsum", "dolor", "sit", "amet", "consectetur", "adipiscing", "elit", "sed", "do", "eiusmod", "tempor"];
    (0..n).map(|i| words[i % words.len()]).collect::<Vec<_>>().join(" ")
}

fn texts(l: &DocLayout, page: usize) -> Vec<String> {
    l.pages[page].shapes.iter().filter_map(|s| if let Shape::Text { text, .. } = s { Some(text.clone()) } else { None }).collect()
}

fn image_data() -> Arc<ImageData> {
    Arc::new(ImageData { bytes: vec![0; 16], format: "png".into(), pixel_size: Some((10, 10)) })
}

fn float_image(w: f32, h: f32, f: Float) -> Run {
    Run { content: Inline::Image(Image { data: image_data(), width: w, height: h, alt: String::new(), float: Some(f) }), props: CharProps::default() }
}

/// A paragraph that ends a section with the given page setup.
fn section_end(text: &str, page: PageSetup, headers: Option<HeadersFooters>) -> Block {
    let mut p = Paragraph::with_text(text);
    p.section = Some(Box::new(Section { page, headers }));
    Block::Para(p)
}

fn hf(header: &str) -> HeadersFooters {
    HeadersFooters { header: vec![Block::Para(Paragraph::with_text(header))], ..Default::default() }
}

fn landscape() -> PageSetup {
    PageSetup { width: 792.0, height: 612.0, ..PageSetup::default() }
}

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

#[test]
fn sections_with_different_page_sizes() {
    let db = db();
    let mut doc = Document::new();
    doc.body = vec![section_end("landscape part", landscape(), None), Block::Para(Paragraph::with_text("portrait part"))];
    let l = layout(&doc, &db, &LayoutOptions::new());
    assert_eq!(l.pages.len(), 2);
    assert_eq!((l.pages[0].width, l.pages[0].height), (792.0, 612.0));
    assert_eq!((l.pages[1].width, l.pages[1].height), (612.0, 792.0));
    assert_eq!((l.pages[0].section, l.pages[1].section), (0, 1));
    assert!(texts(&l, 0).iter().any(|t| t.contains("landscape")));
    assert!(texts(&l, 1).iter().any(|t| t.contains("portrait")));
    // Long text in the landscape section uses its wider text area.
    let mut doc = Document::new();
    doc.body = vec![section_end(&lorem(200), landscape(), None), Block::Para(Paragraph::with_text(&lorem(200)))];
    let l = layout(&doc, &db, &LayoutOptions::new());
    let max_right = |p: usize| l.pages[p].lines.iter().map(|x| *x.carets.last().unwrap()).fold(0.0f32, f32::max);
    assert!(max_right(0) > 600.0, "{}", max_right(0));
    assert!(max_right(1) < 545.0, "{}", max_right(1));
}

#[test]
fn continuous_section_stays_on_page() {
    let db = db();
    let mut doc = Document::new();
    doc.body = vec![section_end("one column title", PageSetup::default(), None), Block::Para(Paragraph::with_text(&lorem(400)))];
    doc.page.start = SectionStart::Continuous;
    doc.page.columns = 2;
    let l = layout(&doc, &db, &LayoutOptions::new());
    let p0 = &l.pages[0];
    assert_eq!(p0.section, 0);
    let title = p0.lines.iter().find(|x| x.path.block == 0).unwrap();
    let body: Vec<_> = p0.lines.iter().filter(|x| x.path.block == 1).collect();
    assert!(!body.is_empty());
    // body starts below the title, in two columns
    assert!(body.iter().all(|x| x.top >= title.top + title.height - 0.1));
    let col_w = PageSetup { columns: 2, ..PageSetup::default() }.column_width();
    assert!(body.iter().all(|x| x.right - x.left <= col_w + 0.5));
    assert!(body.iter().any(|x| x.left > 300.0), "second column used");
    // second column starts at the region top (below the title), not the page top
    let second_top = body.iter().filter(|x| x.left > 300.0).map(|x| x.top).fold(f32::MAX, f32::min);
    assert!(second_top >= title.top + title.height - 0.1, "{second_top}");
    if l.pages.len() > 1 {
        assert_eq!(l.pages[1].section, 1);
    }
}

#[test]
fn odd_and_even_page_sections() {
    let db = db();
    // Section 0 is one page; section 1 starts on an odd page => blank page 2.
    let mut doc = Document::new();
    doc.body = vec![section_end("first", PageSetup::default(), None), Block::Para(Paragraph::with_text("second"))];
    doc.page.start = SectionStart::OddPage;
    let l = layout(&doc, &db, &LayoutOptions::new());
    assert_eq!(l.pages.len(), 3);
    assert!(l.pages[1].lines.is_empty());
    assert_eq!(l.pages[1].section, 0);
    assert_eq!(l.pages[2].section, 1);
    assert_eq!(l.pages[2].number, 3);
    assert!(texts(&l, 2).contains(&"second".to_string()));
    // Even page: page 2 is already even, no blank page.
    doc.page.start = SectionStart::EvenPage;
    let l = layout(&doc, &db, &LayoutOptions::new());
    assert_eq!(l.pages.len(), 2);
    assert_eq!(l.pages[1].number, 2);
}

#[test]
fn headers_per_section_and_linking() {
    let db = db();
    let mut doc = Document::new();
    doc.body = vec![
        section_end("s0", PageSetup::default(), Some(hf("Header zero"))),
        section_end("s1", PageSetup::default(), None), // linked to previous
        Block::Para(Paragraph::with_text("s2")),
    ];
    doc.headers = hf("Last header");
    let l = layout(&doc, &db, &LayoutOptions::new());
    assert_eq!(l.pages.len(), 3);
    assert!(texts(&l, 0).iter().any(|t| t.contains("zero")));
    assert!(texts(&l, 1).iter().any(|t| t.contains("zero")), "linked header");
    assert!(texts(&l, 2).iter().any(|t| t.contains("Last")));
    assert_eq!(l.pages[0].header_story, Some(Story::SectionPart(0, HfKind::Header)));
    assert_eq!(l.pages[1].header_story, Some(Story::SectionPart(0, HfKind::Header)));
    assert_eq!(l.pages[2].header_story, Some(Story::Header));
    assert_eq!(l.pages[2].footer_story, Some(Story::Footer));
    for (pi, st) in [(0, Story::SectionPart(0, HfKind::Header)), (2, Story::Header)] {
        assert_eq!(st, doc.header_story(l.pages[pi].section, HfKind::Header));
        let line = l.pages[pi].lines.iter().find(|x| x.path.story != Story::Body).expect("header line");
        assert_eq!(line.path.story, st);
        // the header line can be found for caret placement
        assert!(l.caret_on_page(line.path.at(0), pi).is_some());
    }
    // first page header of a section with title_page
    let mut doc2 = doc.clone();
    if let Block::Para(p) = &mut doc2.body[1] {
        let s = p.section.as_mut().unwrap();
        s.page.title_page = true;
        s.headers = Some(HeadersFooters { first_header: vec![Block::Para(Paragraph::with_text("First of one"))], ..hf("Other of one") });
    }
    let l = layout(&doc2, &db, &LayoutOptions::new());
    assert_eq!(l.pages[1].header_story, Some(Story::SectionPart(1, HfKind::FirstHeader)));
    assert!(texts(&l, 1).iter().any(|t| t.contains("First")));
}

#[test]
fn page_numbering_restarts() {
    let db = db();
    let mut doc = Document::new();
    let mut ps0 = PageSetup::default();
    ps0.page_number_start = 3;
    doc.body = vec![section_end("a", ps0, None), Block::Para(Paragraph::with_text("b")), Block::Para(Paragraph::with_text("c"))];
    doc.insert_inline(Pos::new(1, 1), Inline::PageBreak, &CharProps::default());
    doc.page.page_number_start = 7;
    let mut f = Paragraph::with_text("Page ");
    f.runs.push(Run { content: Inline::Field(Field::PageNumber), props: CharProps::default() });
    f.runs.push(Run::text(" of "));
    f.runs.push(Run { content: Inline::Field(Field::PageCount), props: CharProps::default() });
    doc.headers.footer.push(Block::Para(f));
    let l = layout(&doc, &db, &LayoutOptions::new());
    let numbers: Vec<u32> = l.pages.iter().map(|p| p.number).collect();
    assert_eq!(numbers, vec![3, 7, 8]);
    assert!(texts(&l, 1).contains(&"7".to_string()));
    assert!(texts(&l, 2).contains(&"3".to_string()), "NUMPAGES = total pages: {:?}", texts(&l, 2));
    // page_number_start 1 on a later section continues numbering
    doc.page.page_number_start = 1;
    let l = layout(&doc, &db, &LayoutOptions::new());
    let numbers: Vec<u32> = l.pages.iter().map(|p| p.number).collect();
    assert_eq!(numbers, vec![3, 4, 5]);
}

// ---------------------------------------------------------------------------
// Floating objects
// ---------------------------------------------------------------------------

fn float_doc(f: Float, w: f32, h: f32) -> Document {
    let mut doc = Document::new();
    let mut p = Paragraph::new();
    p.runs.push(float_image(w, h, f));
    p.runs.push(Run::text(&lorem(150)));
    doc.body = vec![Block::Para(p), Block::Para(Paragraph::with_text(&lorem(150)))];
    doc
}

fn shapes_without_floats(l: &DocLayout) -> Vec<(f32, f32)> {
    l.pages[0].lines.iter().map(|x| (x.top, x.carets[0])).collect()
}

#[test]
fn square_wrap_shifts_lines() {
    let db = db();
    let f = Float { wrap: WrapMode::Square, h_rel: HRel::Column, v_rel: VRel::Paragraph, x: 0.0, y: 0.0, h_align: None, v_align: None, dist: 9.0 };
    let doc = float_doc(f, 150.0, 100.0);
    let l = layout(&doc, &db, &LayoutOptions::new());
    let p = &l.pages[0];
    assert_eq!(p.floats.len(), 1);
    let fb = &p.floats[0];
    assert_eq!((fb.x, fb.y, fb.w, fb.h), (72.0, 72.0, 150.0, 100.0));
    assert_eq!(fb.offset, 0);
    assert!(!fb.text_box);
    assert!(p.shapes.iter().any(|s| matches!(s, Shape::Image { x, y, .. } if *x == 72.0 && *y == 72.0)));
    let mut beside = 0;
    for line in &p.lines {
        let overlaps = line.top < fb.y + fb.h + 9.0 && line.top + line.height > fb.y - 9.0;
        if overlaps {
            beside += 1;
            assert!(line.carets[0] >= 72.0 + 150.0 + 9.0 - 0.1, "line at {} starts at {}", line.top, line.carets[0]);
        } else {
            assert!(line.carets[0] < 80.0);
        }
        assert!(line.carets.iter().all(|x| *x <= 540.5));
    }
    assert!(beside >= 5, "{beside}");
    // all text still laid out; carets reachable
    let len = doc.body[0].as_para().unwrap().len();
    for off in [0, 1, len / 2, len] {
        assert!(l.caret(Pos::new(0, off)).is_some());
    }
    // Float on the right: lines end before it.
    let f2 = Float { h_align: Some(FloatAlign::End), ..f };
    let l = layout(&float_doc(f2, 150.0, 100.0), &db, &LayoutOptions::new());
    let fb = l.pages[0].floats[0].clone();
    assert!((fb.x - (540.0 - 150.0)).abs() < 0.1);
    for line in &l.pages[0].lines {
        if line.top < fb.y + fb.h + 9.0 && line.top + line.height > fb.y - 9.0 {
            assert!(line.right <= fb.x - 9.0 + 0.1);
            assert!(*line.carets.last().unwrap() <= fb.x - 9.0 + 0.5);
        }
    }
}

#[test]
fn float_affects_following_paragraphs_and_page_relative() {
    let db = db();
    // A picture positioned on the page (not near its anchor) that later text wraps around.
    let f = Float { wrap: WrapMode::Square, h_rel: HRel::Page, v_rel: VRel::Page, x: 300.0, y: 200.0, h_align: None, v_align: None, dist: 0.0 };
    let mut doc = Document::new();
    let mut p = Paragraph::with_text("anchor ");
    p.runs.push(float_image(240.0, 120.0, f));
    doc.body = vec![Block::Para(p)];
    for _ in 0..6 {
        doc.body.push(Block::Para(Paragraph::with_text(&lorem(60))));
    }
    let l = layout(&doc, &db, &LayoutOptions::new());
    let p = &l.pages[0];
    let fb = &p.floats[0];
    assert_eq!((fb.x, fb.y), (300.0, 200.0));
    let mut n = 0;
    for line in p.lines.iter().filter(|x| x.path.block > 0) {
        if line.top < 320.0 && line.top + line.height > 200.0 {
            n += 1;
            assert!(line.right <= 300.0 + 0.1, "{}", line.right);
        }
    }
    assert!(n >= 5);
}

#[test]
fn top_bottom_wrap_pushes_lines_below() {
    let db = db();
    let f = Float { wrap: WrapMode::TopBottom, h_rel: HRel::Column, v_rel: VRel::Paragraph, x: 100.0, y: 20.0, h_align: None, v_align: None, dist: 0.0 };
    let doc = float_doc(f, 100.0, 80.0);
    let l = layout(&doc, &db, &LayoutOptions::new());
    let p = &l.pages[0];
    let fb = &p.floats[0];
    assert_eq!((fb.y, fb.h), (92.0, 80.0));
    let mut below = 0;
    for line in &p.lines {
        assert!(line.top + line.height <= fb.y + 0.1 || line.top >= fb.y + fb.h - 0.1, "line {}..{} overlaps", line.top, line.top + line.height);
        if line.top >= fb.y + fb.h - 0.1 {
            below += 1;
        }
        assert!(line.carets[0] < 80.0, "full width lines");
    }
    assert!(below > 3);
    assert!(p.lines.iter().any(|x| x.top < fb.y));
}

#[test]
fn behind_and_in_front_do_not_affect_text() {
    let db = db();
    let plain = {
        let mut d = float_doc(Float::default(), 1.0, 1.0);
        d.body[0].as_para_mut().unwrap().runs.remove(0);
        d.body[0].as_para_mut().unwrap().runs.insert(0, Run::text("x"));
        layout(&d, &db, &LayoutOptions::new())
    };
    for wrap in [WrapMode::Behind, WrapMode::InFront] {
        let f = Float { wrap, h_rel: HRel::Margin, v_rel: VRel::Margin, x: 0.0, y: 0.0, h_align: None, v_align: None, dist: 9.0 };
        let l = layout(&float_doc(f, 200.0, 200.0), &db, &LayoutOptions::new());
        let a = shapes_without_floats(&l);
        let b = shapes_without_floats(&plain);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x.0 - y.0).abs() < 0.01 && (x.1 - y.1).abs() < 0.01);
        }
        let shapes = &l.pages[0].shapes;
        let img = shapes.iter().position(|s| matches!(s, Shape::Image { .. })).unwrap();
        let first_text = shapes.iter().position(|s| matches!(s, Shape::Text { .. })).unwrap();
        let last_text = shapes.iter().rposition(|s| matches!(s, Shape::Text { .. })).unwrap();
        match wrap {
            WrapMode::Behind => assert!(img < first_text),
            _ => assert!(img > last_text),
        }
        assert_eq!(l.pages[0].floats[0].wrap, wrap);
    }
}

#[test]
fn float_moves_with_paragraph_to_next_page() {
    let db = db();
    let mut moved = 0;
    for k in (1..200).step_by(4) {
        let mut doc = Document::new();
        doc.body.clear();
        for _ in 0..12 {
            doc.body.push(Block::Para(Paragraph::with_text(&lorem(80))));
        }
        doc.body.push(Block::Para(Paragraph::with_text(&lorem(k))));
        let f = Float { wrap: WrapMode::Square, ..Float::default() };
        let mut p = Paragraph::new();
        p.runs.push(float_image(100.0, 100.0, f));
        p.runs.push(Run::text(&lorem(100)));
        doc.body.push(Block::Para(p));
        let l = layout(&doc, &db, &LayoutOptions::new());
        let anchor_page = l.page_of(Pos::new(13, 0)).unwrap();
        let on: Vec<usize> = l.pages.iter().enumerate().filter(|(_, p)| !p.floats.is_empty()).map(|(i, _)| i).collect();
        assert_eq!(on, vec![anchor_page], "k={k}");
        let fb = &l.pages[anchor_page].floats[0];
        let first = l.pages[anchor_page].lines.iter().find(|x| x.path.block == 13).unwrap();
        assert!((fb.y - first.top).abs() < 0.1, "k={k}: {} {}", fb.y, first.top);
        assert!(first.left >= 72.0 + 100.0 + 9.0 - 0.1, "anchor paragraph wraps: {}", first.left);
        if (first.top - l.pages[anchor_page].body_top).abs() < 0.1 {
            moved += 1;
        }
        for p in &l.pages {
            for line in &p.lines {
                assert!(line.top + line.height <= p.body_bottom + 0.5);
            }
        }
    }
    assert!(moved > 0);
}

#[test]
fn line_and_character_relative_floats() {
    let db = db();
    let f = Float { wrap: WrapMode::InFront, h_rel: HRel::Character, v_rel: VRel::Line, x: 0.0, y: 0.0, h_align: None, v_align: None, dist: 0.0 };
    let mut doc = Document::new();
    let mut p = Paragraph::with_text("abc ");
    p.runs.push(float_image(20.0, 20.0, f));
    p.runs.push(Run::text("def"));
    doc.body = vec![Block::Para(p)];
    let l = layout(&doc, &db, &LayoutOptions::new());
    let line = &l.pages[0].lines[0];
    let fb = &l.pages[0].floats[0];
    assert_eq!(fb.offset, 4);
    assert!((fb.x - line.caret_x(4)).abs() < 0.1);
    assert!((fb.y - line.top).abs() < 0.1);
    // the float takes no width
    assert!((line.caret_x(5) - line.caret_x(4)).abs() < 0.01);
}

// ---------------------------------------------------------------------------
// Text boxes
// ---------------------------------------------------------------------------

fn text_box(float: Option<Float>, text: &str) -> Run {
    let tb = TextBox { paragraphs: vec![Paragraph::with_text(text)], width: 160.0, height: 60.0, float, fill: Some(Color::YELLOW), ..TextBox::default() };
    Run { content: Inline::TextBox(Box::new(tb)), props: CharProps::default() }
}

#[test]
fn floating_text_box_draws_and_wraps() {
    let db = db();
    let f = Float { wrap: WrapMode::Square, h_rel: HRel::Margin, v_rel: VRel::Paragraph, x: 0.0, y: 0.0, h_align: Some(FloatAlign::End), v_align: None, dist: 9.0 };
    let mut doc = Document::new();
    let mut p = Paragraph::new();
    p.runs.push(text_box(Some(f), "Inside the box"));
    p.runs.push(Run::text(&lorem(120)));
    doc.body = vec![Block::Para(p)];
    let l = layout(&doc, &db, &LayoutOptions::new());
    let page = &l.pages[0];
    let fb = &page.floats[0];
    assert!(fb.text_box);
    assert_eq!((fb.x, fb.y, fb.w, fb.h), (540.0 - 160.0, 72.0, 160.0, 60.0));
    let fill = page.shapes.iter().position(|s| matches!(s, Shape::Rect { x, y, w, h, color } if *x == fb.x && *y == fb.y && *w == 160.0 && *h == 60.0 && *color == Color::YELLOW)).expect("fill");
    let inside = page.shapes.iter().position(|s| matches!(s, Shape::Text { text, x, y, .. } if text.contains("Inside") && *x >= fb.x && *x < fb.x + 20.0 && *y > fb.y && *y < fb.y + 60.0)).expect("text inside");
    assert!(inside > fill);
    let border_lines = page.shapes.iter().filter(|s| matches!(s, Shape::Line { x1, y1, x2, y2, .. } if (*x1 - fb.x).abs() < 0.01 && (*y1 - fb.y).abs() < 0.01 && (*x2 == fb.x || *y2 == fb.y))).count();
    assert_eq!(border_lines, 2);
    assert_eq!(page.shapes.iter().filter(|s| matches!(s, Shape::Line { .. })).count(), 4);
    // text box content is not editable: no lines indexed for it
    assert!(page.lines.iter().all(|x| x.path.block == 0 && x.path.cell.is_none()));
    // wrapped body lines stay left of the box
    for line in &page.lines {
        if line.top < fb.y + fb.h + 9.0 {
            assert!(line.right <= fb.x - 9.0 + 0.1);
        }
    }
}

#[test]
fn text_box_clips_and_grows() {
    let db = db();
    let long = lorem(80);
    let f = Float { wrap: WrapMode::InFront, ..Float::default() };
    let make = |auto: bool| {
        let mut doc = Document::new();
        let mut p = Paragraph::new();
        let tb = TextBox { paragraphs: vec![Paragraph::with_text(&long)], width: 160.0, height: 60.0, float: Some(f), auto_height: auto, ..TextBox::default() };
        p.runs.push(Run { content: Inline::TextBox(Box::new(tb)), props: CharProps::default() });
        doc.body = vec![Block::Para(p)];
        layout(&doc, &db, &LayoutOptions::new())
    };
    let l = make(false);
    let fb = &l.pages[0].floats[0];
    assert_eq!(fb.h, 60.0);
    for s in &l.pages[0].shapes {
        if let Shape::Text { y, .. } = s {
            assert!(*y <= fb.y + fb.h, "text drawn below the box: {y}");
        }
    }
    let l2 = make(true);
    let fb2 = &l2.pages[0].floats[0];
    assert!(fb2.h > 100.0, "{}", fb2.h);
    let n1 = l.pages[0].shapes.iter().filter(|s| matches!(s, Shape::Text { .. })).count();
    let n2 = l2.pages[0].shapes.iter().filter(|s| matches!(s, Shape::Text { .. })).count();
    assert!(n2 > n1);
}

#[test]
fn inline_text_box_sizes_line() {
    let db = db();
    let mut doc = Document::new();
    let mut p = Paragraph::with_text("before ");
    p.runs.push(text_box(None, "boxed"));
    p.runs.push(Run::text(" after"));
    doc.body = vec![Block::Para(p)];
    let l = layout(&doc, &db, &LayoutOptions::new());
    let page = &l.pages[0];
    assert!(page.floats.is_empty());
    let line = &page.lines[0];
    assert!(line.height >= 60.0);
    assert!((line.caret_x(8) - line.caret_x(7) - 160.0).abs() < 0.1);
    let x = line.caret_x(7);
    // the box sits on the baseline
    assert!(page.shapes.iter().any(|s| matches!(s, Shape::Rect { x: rx, y, h, .. } if (*rx - x).abs() < 0.01 && (*y + *h - line.baseline).abs() < 0.1)));
    assert!(texts(&l, 0).iter().any(|t| t.contains("boxed")));
}

#[test]
fn multi_column_paragraph_continues_in_next_column() {
    let db = db();
    let mut doc = Document::new();
    doc.page.columns = 2;
    doc.body = vec![Block::Para(Paragraph::with_text(&lorem(900)))];
    let l = layout(&doc, &db, &LayoutOptions::new());
    let p = &l.pages[0];
    let lefts: Vec<f32> = p.lines.iter().map(|x| x.left).collect();
    assert!(lefts.iter().any(|x| *x > 300.0), "second column used by the same paragraph");
}

#[test]
fn pdf_export_with_floats() {
    let db = db();
    let f = Float { wrap: WrapMode::Square, ..Float::default() };
    let mut doc = Document::new();
    let mut p = Paragraph::new();
    p.runs.push(text_box(Some(f), "Box text"));
    p.runs.push(Run::text(&lorem(100)));
    doc.body = vec![section_end(&lorem(20), landscape(), None), Block::Para(p)];
    let l = layout(&doc, &db, &LayoutOptions::new());
    let pdf = wordcore::pdf::export(&l, &db, &doc.properties);
    let s = String::from_utf8_lossy(&pdf);
    assert!(s.contains("/MediaBox [0 0 792 612 ]"));
    assert!(s.contains("/MediaBox [0 0 612 792 ]"));
}
