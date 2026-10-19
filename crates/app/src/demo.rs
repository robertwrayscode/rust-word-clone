//! A sample document shown with `--demo` (and used for screenshots).

use wordcore::*;

fn run(t: &str, f: impl FnOnce(&mut CharProps)) -> Run {
    let mut p = CharProps::default();
    f(&mut p);
    Run::styled(t, p)
}

pub fn demo_document() -> Document {
    let mut d = Document::new();
    d.properties.title = "Quarterly Report".into();
    let bullets = d.add_list(ListDef::bullets(0));
    let numbers = d.add_list(ListDef::numbered(0));
    let mut body = vec![
        Block::Para(Paragraph::with_style("Title", "Quarterly Report")),
        Block::Para(Paragraph::with_style("Subtitle", "Prepared for the leadership team · October 2026")),
        Block::Para(Paragraph::with_style("Heading1", "Summary")),
    ];
    let mut p = Paragraph::new();
    p.runs = vec![
        Run::text("This quarter we delivered "),
        run("record revenue", |c| c.bold = Some(true)),
        Run::text(" across all regions, driven by strong demand for our "),
        run("new product line", |c| c.italic = Some(true)),
        Run::text(". Customer satisfaction reached its highest level in three years, and we "),
        run("reduced costs", |c| c.underline = Some(Underline::Single)),
        Run::text(" while expanding the team. The highlights are listed below, followed by a regional breakdown."),
        Run { content: Inline::FootnoteRef(1), props: CharProps { style: Some("FootnoteReference".into()), ..Default::default() } },
    ];
    p.props.align = Some(Align::Justify);
    body.push(Block::Para(p));
    for t in ["Revenue grew 18% compared with the same quarter last year.", "Three new offices opened in Lisbon, Toronto and Singapore.", "Support response times fell to under two hours."] {
        let mut p = Paragraph::with_style("ListParagraph", t);
        p.props.list = Some(Some(ListRef { list: bullets, level: 0 }));
        body.push(Block::Para(p));
    }
    body.push(Block::Para(Paragraph::with_style("Heading2", "Regional results")));
    let mut t = Table::new(4, 3, 468.0);
    let data = [["Region", "Revenue", "Growth"], ["Europe", "$4.2M", "+21%"], ["Americas", "$6.8M", "+15%"], ["Asia Pacific", "$3.1M", "+24%"]];
    for (r, row) in data.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            let mut p = Paragraph::with_text(v);
            p.props.space_after = Some(0.0);
            if c > 0 {
                p.props.align = Some(Align::Right);
            }
            if r == 0 {
                for run in &mut p.runs {
                    run.props.bold = Some(true);
                    run.props.color = Some(Color::WHITE);
                }
            }
            t.rows[r].cells[c].paragraphs = vec![p];
        }
    }
    t.header_shading = Some(Color::rgb(0x44, 0x72, 0xC4));
    t.band_shading = Some(Color::rgb(0xD9, 0xE2, 0xF3));
    t.rows[0].header = true;
    t.borders = Borders::all(Border { color: Color::rgb(0x8E, 0xAA, 0xDB), ..Border::single() });
    body.push(Block::Table(t));
    let mut cap = Paragraph::with_style("Caption", "Table 1: Revenue by region");
    cap.props.space_before = Some(6.0);
    body.push(Block::Para(cap));
    body.push(Block::Para(Paragraph::with_style("Heading2", "Next steps")));
    for t in ["Finalise the budget for the coming year.", "Hire twelve engineers for the platform team.", "Launch the customer portal in all regions."] {
        let mut p = Paragraph::with_style("ListParagraph", t);
        p.props.list = Some(Some(ListRef { list: numbers, level: 0 }));
        body.push(Block::Para(p));
    }
    let mut q = Paragraph::with_style("Quote", "“The best way to predict the future is to create it.”");
    q.props.space_before = Some(12.0);
    body.push(Block::Para(q));
    let mut p = Paragraph::new();
    p.runs = vec![
        Run::text("Questions? Visit "),
        Run { content: Inline::Text("our website".into()), props: CharProps { link: Some("https://example.com".into()), style: Some("Hyperlink".into()), ..Default::default() } },
        Run::text(" or contact the finance team. "),
        run("Highlighted", |c| c.highlight = Some(Color::YELLOW)),
        Run::text(" text, "),
        run("colour", |c| c.color = Some(Color::rgb(0xC0, 0x00, 0x00))),
        Run::text(", H"),
        run("2", |c| c.vert_align = Some(VertAlign::Subscript)),
        Run::text("O and E = mc"),
        run("2", |c| c.vert_align = Some(VertAlign::Superscript)),
        Run::text(" all work too."),
    ];
    body.push(Block::Para(p));
    d.body = body;
    d.footnotes.push(Note { id: 1, paragraphs: vec![Paragraph::with_style("FootnoteText", "Figures are unaudited and rounded to one decimal place.")] });
    let mut h = Paragraph::with_style("Header", "Quarterly Report");
    h.runs.push(Run { content: Inline::Tab, props: CharProps::default() });
    h.runs.push(Run { content: Inline::Tab, props: CharProps::default() });
    h.runs.push(Run::text("Confidential"));
    for r in &mut h.runs {
        r.props.color = Some(Color::rgb(0x76, 0x76, 0x76));
    }
    d.headers.header.push(Block::Para(h));
    let mut f = Paragraph::with_style("Footer", "Page ");
    f.props.align = Some(Align::Center);
    f.runs.push(Run { content: Inline::Field(Field::PageNumber), props: CharProps::default() });
    f.runs.push(Run::text(" of "));
    f.runs.push(Run { content: Inline::Field(Field::PageCount), props: CharProps::default() });
    d.headers.footer.push(Block::Para(f));
    d
}
