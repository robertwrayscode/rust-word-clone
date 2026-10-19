use wordcore::fonts::FontDb;
use wordcore::layout::{layout, LayoutOptions, Shape};
use wordcore::*;

fn db() -> FontDb {
    FontDb::load(false)
}

fn lorem(n: usize) -> String {
    let words = ["lorem", "ipsum", "dolor", "sit", "amet", "consectetur", "adipiscing", "elit", "sed", "do", "eiusmod", "tempor"];
    (0..n).map(|i| words[i % words.len()]).collect::<Vec<_>>().join(" ")
}

#[test]
fn single_line() {
    let db = db();
    let doc = Document::from_text("Hello world");
    let l = layout(&doc, &db, &LayoutOptions::new());
    assert_eq!(l.pages.len(), 1);
    let p = &l.pages[0];
    assert_eq!(p.lines.len(), 1);
    let line = &p.lines[0];
    assert_eq!((line.start, line.end), (0, 11));
    assert_eq!(line.carets.len(), 12);
    assert!((line.top - 72.0).abs() < 0.01);
    // Calibri 11pt single line = 13.43pt; Word default 1.08 multiple
    assert!((line.height - 11.0 * 1.2207 * 1.08).abs() < 0.2, "{}", line.height);
    assert!(p.shapes.iter().any(|s| matches!(s, Shape::Text { text, .. } if text.starts_with("Hello"))));
    // caret monotonic
    assert!(line.carets.windows(2).all(|w| w[1] >= w[0]));
    let c = l.caret(Pos::new(0, 5)).unwrap();
    assert_eq!(c.page, 0);
    let hit = l.hit(0, c.x + 0.5, c.top + 2.0, true).unwrap();
    assert_eq!(hit, Pos::new(0, 5));
}

#[test]
fn wraps_and_paginates() {
    let db = db();
    let mut doc = Document::new();
    doc.body = (0..60).map(|_| Block::Para(Paragraph::with_text(&lorem(80)))).collect();
    let l = layout(&doc, &db, &LayoutOptions::new());
    assert!(l.pages.len() > 5, "pages {}", l.pages.len());
    for p in &l.pages {
        for line in &p.lines {
            assert!(line.top + line.height <= p.body_bottom + 0.5);
            assert!(line.carets.iter().all(|x| *x <= 612.0 - 72.0 + 30.0), "line too wide");
        }
    }
    // every char offset of every paragraph maps to a caret
    for bi in 0..60 {
        let len = doc.body[bi].as_para().unwrap().len();
        for off in [0, len / 2, len] {
            assert!(l.caret(Pos::new(bi, off)).is_some());
        }
    }
    // vertical movement crosses pages
    let last_on_p0 = l.pages[0].lines.last().unwrap();
    let pos = last_on_p0.path.at(last_on_p0.start);
    let down = l.vertical(pos, 80.0, true).unwrap();
    assert_eq!(l.page_of(down), Some(1));
}

#[test]
fn page_break_and_headers() {
    let db = db();
    let mut doc = Document::from_text("one\ntwo");
    doc.insert_inline(Pos::new(0, 3), Inline::PageBreak, &CharProps::default());
    let mut f = Paragraph::with_style("Footer", "Page ");
    f.runs.push(Run { content: Inline::Field(Field::PageNumber), props: CharProps::default() });
    f.runs.push(Run::text(" of "));
    f.runs.push(Run { content: Inline::Field(Field::PageCount), props: CharProps::default() });
    doc.headers.footer.push(Block::Para(f));
    let l = layout(&doc, &db, &LayoutOptions::new());
    assert_eq!(l.pages.len(), 2);
    let texts: Vec<String> = l.pages[1].shapes.iter().filter_map(|s| if let Shape::Text { text, .. } = s { Some(text.clone()) } else { None }).collect();
    assert!(texts.contains(&"2".to_string()), "{:?}", texts);
    assert!(texts.contains(&"two".to_string()));
}

#[test]
fn tables_layout() {
    let db = db();
    let mut doc = Document::from_text("before\nafter");
    let mut t = Table::new(3, 3, 468.0);
    t.rows[1].cells[1] = Cell::with_text(&lorem(30));
    doc.body.insert(1, Block::Table(t));
    let l = layout(&doc, &db, &LayoutOptions::new());
    let p = &l.pages[0];
    let cell_lines: Vec<_> = p.lines.iter().filter(|l| l.path.cell.is_some()).collect();
    assert!(cell_lines.len() >= 9 + 2);
    let before = p.lines.iter().find(|l| l.path.block == 0).unwrap();
    let after = p.lines.iter().find(|l| l.path.block == 2).unwrap();
    assert!(after.top > before.top + 50.0);
    assert!(l.caret(ParaPath { story: Story::Body, block: 1, cell: Some(CellRef { row: 2, col: 2, para: 0 }) }.at(0)).is_some());
}

#[test]
fn lists_and_alignment() {
    let db = db();
    let mut doc = Document::from_text("a\nb\nc");
    let id = doc.add_list(ListDef::numbered(0));
    for b in &mut doc.body {
        b.as_para_mut().unwrap().props.list = Some(Some(ListRef { list: id, level: 0 }));
    }
    doc.body[2].as_para_mut().unwrap().props.align = Some(Align::Right);
    let l = layout(&doc, &db, &LayoutOptions::new());
    let texts: Vec<String> = l.pages[0].shapes.iter().filter_map(|s| if let Shape::Text { text, .. } = s { Some(text.clone()) } else { None }).collect();
    assert!(texts.contains(&"1.".to_string()) && texts.contains(&"3.".to_string()), "{:?}", texts);
    let line = &l.pages[0].lines[0];
    assert!((line.carets[0] - (72.0 + 36.0)).abs() < 0.5, "{}", line.carets[0]);
    let line = &l.pages[0].lines[2];
    assert!((line.carets[1] - (612.0 - 72.0)).abs() < 0.5);
}

#[test]
fn justify_fills_line() {
    let db = db();
    let mut doc = Document::from_text(&lorem(100));
    doc.body[0].as_para_mut().unwrap().props.align = Some(Align::Justify);
    let l = layout(&doc, &db, &LayoutOptions::new());
    let line = &l.pages[0].lines[0];
    let last = *line.carets.last().unwrap();
    assert!((last - 540.0).abs() < 8.0, "{}", last);
}
