mod common;

use common::*;
use wordcore::io::{self, html};
use wordcore::*;

fn roundtrip(doc: &Document) -> Document {
    html::read(&html::write(doc))
}

#[test]
fn html_export_is_standalone() {
    let s = html::write(&kitchen_sink());
    assert!(s.starts_with("<!DOCTYPE html>"));
    assert!(s.contains("<meta charset=\"utf-8\">"));
    assert!(s.contains("<title>The Title</title>") || s.contains("<title>"));
    assert!(s.contains("data:image/png;base64,"));
    assert!(s.contains("<h1"));
    assert!(s.contains("<ul") && s.contains("<ol"));
    assert!(s.contains("colspan=\"2\"") && s.contains("rowspan=\"2\""));
    assert!(s.contains("href=\"https://example.com/a?b=1&amp;c=2\""));
    assert!(!s.contains("deleted"), "tracked deletions are exported accepted");
    assert_eq!(io::Format::sniff(s.as_bytes()), io::Format::Html);
}

#[test]
fn html_roundtrip_formatting() {
    let doc = kitchen_sink();
    let back = roundtrip(&doc);
    assert_eq!(back.properties.title, doc.properties.title);
    assert_eq!(back.properties.author, doc.properties.author);
    assert_eq!(find_run(&back, "bold").props.bold, Some(true));
    assert_eq!(find_run(&back, "italic").props.italic, Some(true));
    assert_eq!(find_run(&back, "dunder").props.underline, Some(Underline::Double));
    assert_eq!(find_run(&back, "struck").props.strike, Some(true));
    assert_eq!(find_run(&back, "dstruck").props.double_strike, Some(true));
    assert_eq!(find_run(&back, "red").props.color, Some(Color::rgb(255, 0, 0)));
    assert_eq!(find_run(&back, "hilite").props.highlight, Some(Color::rgb(255, 255, 0)));
    assert_eq!(find_run(&back, "caps").props.caps, Some(true));
    assert_eq!(find_run(&back, "smallcaps").props.small_caps, Some(true));
    assert_eq!(find_run(&back, "spaced").props.spacing, Some(1.5));
    let g = find_run(&back, "georgia");
    assert_eq!(g.props.font.as_deref(), Some("Georgia"));
    assert_eq!(g.props.size, Some(14.5));
    let p = find_para(&back, "E=mc");
    assert!(p.runs.iter().any(|r| r.props.vert_align == Some(VertAlign::Superscript)));
    assert!(p.runs.iter().any(|r| r.props.vert_align == Some(VertAlign::Subscript)));
    assert_eq!(find_para(&back, "Ünïcödé").text(), "Ünïcödé — “quotes” & <angle> 日本語 😀");
    assert!(find_para(&back, "two  spaces").text().contains("  two  spaces"), "{:?}", find_para(&back, "two  spaces").text());

    let o = find_para(&doc, "Fancy paragraph");
    let p = find_para(&back, "Fancy paragraph");
    assert_eq!(p.props.align, o.props.align);
    assert_eq!(p.props.indent_left, o.props.indent_left);
    assert_eq!(p.props.indent_right, o.props.indent_right);
    assert_eq!(p.props.indent_first, o.props.indent_first);
    assert_eq!(p.props.space_before, o.props.space_before);
    assert_eq!(p.props.space_after, o.props.space_after);
    assert_eq!(p.props.shading, o.props.shading);
    assert!(p.props.borders.as_ref().unwrap().top.is_some());
    assert!(p.runs.iter().any(|r| r.content == Inline::Tab));
    assert_eq!(find_para(&back, "Line spacing 1.5").props.line_spacing, Some(LineSpacing::Multiple(1.5)));
    assert_eq!(find_para(&back, "Starts on new page").props.page_break_before, Some(true));
    assert_eq!(find_para(&back, "Chapter One").props.style.as_deref(), Some("Heading1"));
    assert_eq!(find_para(&back, "Section 1.1").props.style.as_deref(), Some("Heading2"));
    assert_eq!(find_para(&back, "The Title").props.style.as_deref(), Some("Title"));
}

#[test]
fn html_roundtrip_structure() {
    let back = roundtrip(&kitchen_sink());
    let b1 = find_para(&back, "Bullet one").props.list.flatten().unwrap();
    let b2 = find_para(&back, "Bullet two").props.list.flatten().unwrap();
    assert_eq!(b1.list, b2.list);
    assert_eq!((b1.level, b2.level), (0, 1));
    assert_eq!(back.list(b1.list).unwrap().levels[0].format, NumFormat::Bullet);
    let n1 = find_para(&back, "Number one").props.list.flatten().unwrap();
    let n2 = find_para(&back, "Number two").props.list.flatten().unwrap();
    assert_eq!(n1, n2);
    assert_ne!(n1.list, b1.list);
    assert_eq!(back.list(n1.list).unwrap().levels[0].format, NumFormat::Decimal);

    let t = back.body.iter().find_map(|b| b.as_table()).expect("table");
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
    assert!(t.borders.top.is_some());

    let p = find_para(&back, "Image:");
    let img = p.runs.iter().find_map(|r| if let Inline::Image(i) = &r.content { Some(i) } else { None }).unwrap();
    assert_eq!((img.width, img.height, img.alt.as_str()), (72.0, 36.0, "A pixel"));
    assert_eq!(img.data.pixel_size, Some((1, 1)));
    assert!(p.runs.iter().any(|r| r.content == Inline::Field(Field::PageNumber)));
    assert!(p.runs.iter().any(|r| r.content == Inline::Field(Field::PageCount)));

    let p = find_para(&back, "Visit");
    assert!(p.runs.iter().any(|r| r.content == Inline::Bookmark("target_mark".into())));
    assert_eq!(find_run(&back, "example").props.link.as_deref(), Some("https://example.com/a?b=1&c=2"));
    assert_eq!(find_run(&back, "inside").props.link.as_deref(), Some("#target_mark"));
    let fid = p.runs.iter().find_map(|r| if let Inline::FootnoteRef(i) = r.content { Some(i) } else { None }).expect("fn ref");
    let note = back.footnotes.iter().find(|n| n.id == fid).expect("footnote");
    assert_eq!(note.paragraphs[0].text(), " The footnote text.");
    let eid = p.runs.iter().find_map(|r| if let Inline::EndnoteRef(i) = r.content { Some(i) } else { None }).expect("en ref");
    assert!(back.endnotes.iter().find(|n| n.id == eid).unwrap().paragraphs[0].text().contains("The endnote text."));
    assert!(!back.plain_text().contains("The footnote text."), "notes are not body text");
    assert_eq!(find_para(&back, "Tracked:").text(), "Tracked: inserted  commented both tail");
}

#[test]
fn html_read_handwritten() {
    let src = r#"<html><head><title>T &amp; T</title><meta name="author" content="Ann">
<style>.big { font-size: 20pt } p.note { color: #00f }</style></head><body>
<h1>Heading</h1>
<p>Plain <b>bold <i>both</i></b> <u>under</u> <s>erased</s> <span style="color:rgb(255,0,0);font-family:'Times New Roman',serif">red</span>
   <span class="big">big</span> x<sup>2</sup> <font color="green" face="Arial">green</font> <a href="https://a.b/c">link</a><br>next line</p>
<p class="note" style="text-align:center;margin-left:36pt;line-height:200%">note &nbsp;text</p>
<ul><li>one<li>two<ul><li>nested</ul><li><p>three</p></ul>
<ol type="a" start="3"><li>c item</li></ol>
<pre>line 1
	tabbed</pre>
<table border="1"><tr><th>H1<th>H2<tr><td colspan=2>wide<tr><td rowspan=2>tall<td>r3<tr><td>r4</table>
<p>trailing</p>
<img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==" width="96" height="48" alt="px">
</body></html>"#;
    let doc = html::read(src);
    assert_eq!(doc.properties.title, "T & T");
    assert_eq!(doc.properties.author, "Ann");
    assert_eq!(find_para(&doc, "Heading").props.style.as_deref(), Some("Heading1"));
    let r = find_run(&doc, "both");
    assert_eq!((r.props.bold, r.props.italic), (Some(true), Some(true)));
    assert_eq!(find_run(&doc, "under").props.underline, Some(Underline::Single));
    assert_eq!(find_run(&doc, "erased").props.strike, Some(true));
    let r = find_run(&doc, "red");
    assert_eq!(r.props.color, Some(Color::rgb(255, 0, 0)));
    assert_eq!(r.props.font.as_deref(), Some("Times New Roman"));
    assert_eq!(find_run(&doc, "big").props.size, Some(20.0));
    assert_eq!(find_run(&doc, "2").props.vert_align, Some(VertAlign::Superscript));
    assert_eq!(find_run(&doc, "green").props.color, Some(Color::rgb(0, 128, 0)));
    assert_eq!(find_run(&doc, "link").props.link.as_deref(), Some("https://a.b/c"));
    let p = find_para(&doc, "Plain");
    assert!(p.text().starts_with("Plain bold both under erased red big x2 green link\nnext line"), "{:?}", p.text());
    let n = find_para(&doc, "note");
    assert_eq!(n.text(), "note  text");
    assert_eq!(n.props.align, Some(Align::Center));
    assert_eq!(n.props.indent_left, Some(36.0));
    assert_eq!(n.props.line_spacing, Some(LineSpacing::Multiple(1.74)));
    assert_eq!(find_run(&doc, "note").props.color, Some(Color::rgb(0, 0, 255)));
    let one = find_para(&doc, "one").props.list.flatten().unwrap();
    let nested = find_para(&doc, "nested").props.list.flatten().unwrap();
    let three = find_para(&doc, "three").props.list.flatten().unwrap();
    assert_eq!((one.level, nested.level, three.level), (0, 1, 0));
    assert_eq!(one.list, three.list);
    let c = find_para(&doc, "c item").props.list.flatten().unwrap();
    let l = doc.list(c.list).unwrap();
    assert_eq!(l.levels[0].format, NumFormat::LowerLetter);
    assert_eq!(l.levels[0].start, 3);
    let pre1 = find_para(&doc, "line 1");
    assert_eq!(find_run(&doc, "line 1").props.font.as_deref(), Some("Courier New"));
    assert_eq!(pre1.text(), "line 1");
    assert_eq!(find_para(&doc, "tabbed").text(), "\ttabbed");
    let t = doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.rows.len(), 4);
    assert!(t.rows[0].header);
    assert_eq!(t.rows[1].cells[0].span, 2);
    assert_eq!(t.rows[2].cells[0].vmerge, VMerge::Restart);
    assert_eq!(t.rows[3].cells[0].vmerge, VMerge::Continue);
    assert_eq!(t.rows[3].cells[1].text(), "r4");
    assert!(t.borders.top.is_some());
    let p = doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.runs.iter().any(|r| matches!(r.content, Inline::Image(_)))).unwrap();
    let Inline::Image(img) = &p.runs[0].content else { panic!() };
    assert_eq!((img.width, img.height), (72.0, 36.0));
}

#[test]
fn html_fragments() {
    let doc = kitchen_sink();
    // Copy: styles resolved inline, no stylesheet needed.
    let blocks: Vec<Block> = doc.body.iter().filter(|b| matches!(b, Block::Para(p) if p.text().contains("Chapter One") || p.text().contains("Bullet"))).cloned().collect();
    let frag = html::write_fragment(&blocks, &doc);
    assert!(!frag.contains("<html"));
    assert!(frag.contains("font-size:16pt"), "{frag}");
    assert!(frag.contains("<ul"));
    // Paste our own fragment back.
    let back = html::read_fragment(&frag);
    assert_eq!(back.len(), 3);
    assert_eq!(back[0].as_para().unwrap().props.style.as_deref(), Some("Heading1"));
    assert_eq!(back[1].as_para().unwrap().text(), "• Bullet one");
    let mut target = Document::new();
    let back = html::read_fragment_into(&frag, &mut target);
    let l = back[1].as_para().unwrap().props.list.flatten().unwrap();
    assert!(target.list(l.list).is_some());

    // Browser clipboard with markers: only the marked part counts.
    let cb = "Version:0.9\r\nStartHTML:0000000105\r\n<html><body>\r\n<p>outside</p><!--StartFragment--><b>Hello</b> <i>world</i><!--EndFragment--><p>after</p></body></html>";
    let b = html::read_fragment(cb);
    assert_eq!(b.len(), 1);
    let p = b[0].as_para().unwrap();
    assert_eq!(p.text(), "Hello world");
    assert_eq!(p.runs[0].props.bold, Some(true));
    assert!(html::read_fragment("<html><body></body></html>").is_empty());
    assert!(html::read_fragment("").is_empty());

    // Word clipboard HTML.
    let word = r##"<html xmlns:o="urn:schemas-microsoft-com:office:office"><head><style><!--
p.MsoNormal {margin:0cm; font-size:11.0pt; font-family:"Calibri",sans-serif;}
p.MsoTitle {font-size:28.0pt;}
--></style></head><body lang=EN-US><!--StartFragment-->
<p class=MsoTitle>Doc title</p>
<p class=MsoListParagraphCxSpFirst style='text-indent:-18.0pt;mso-list:l0 level1 lfo1'><![if !supportLists]><span style='font-family:Symbol'><span style='mso-list:Ignore'>·<span style='font:7.0pt "Times New Roman"'>&nbsp;&nbsp; </span></span></span><![endif]>First bullet</p>
<p class=MsoListParagraphCxSpLast style='text-indent:-18.0pt;mso-list:l0 level2 lfo1'><![if !supportLists]><span style='mso-list:Ignore'>o<span>&nbsp; </span></span><![endif]>Second level</p>
<p class=MsoNormal style='mso-list:l1 level1 lfo2'><span style='mso-list:Ignore'>1.<span>&nbsp; </span></span>Numbered</p>
<p class=MsoNormal>Tab<span style='mso-tab-count:1'>&nbsp;&nbsp;&nbsp; </span>after <span style='background:yellow;mso-highlight:yellow'>marked</span><a href="#_ftn1" name="_ftnref1"><span class=MsoFootnoteReference>[1]</span></a></p>
<!--EndFragment--></body></html>"##;
    let mut d = Document::new();
    let b = html::read_fragment_into(word, &mut d);
    let paras: Vec<&Paragraph> = b.iter().filter_map(|x| x.as_para()).collect();
    assert_eq!(paras[0].props.style.as_deref(), Some("Title"));
    assert_eq!(paras[1].text(), "First bullet");
    let l1 = paras[1].props.list.flatten().unwrap();
    let l2 = paras[2].props.list.flatten().unwrap();
    assert_eq!((l1.list, l1.level, l2.level), (l2.list, 0, 1));
    assert_eq!(d.list(l1.list).unwrap().levels[0].format, NumFormat::Bullet);
    let l3 = paras[3].props.list.flatten().unwrap();
    assert_ne!(l3.list, l1.list);
    assert_eq!(d.list(l3.list).unwrap().levels[0].format, NumFormat::Decimal);
    assert_eq!(paras[3].text(), "Numbered");
    assert_eq!(paras[4].text(), "Tab\tafter marked");
    assert_eq!(find_run_in(paras[4], "marked").props.highlight, Some(Color::YELLOW));
    assert!(paras[4].runs.iter().any(|r| r.content == Inline::FootnoteRef(1)));
}

fn find_run_in<'a>(p: &'a Paragraph, s: &str) -> &'a Run {
    p.runs.iter().find(|r| matches!(&r.content, Inline::Text(t) if t.contains(s))).unwrap()
}

#[test]
fn html_libreoffice_interop() {
    if !soffice_available() {
        eprintln!("soffice not available; skipping");
        return;
    }
    let dir = scratch_dir().join("html_lo");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // LibreOffice's HTML export of our DOCX reads back.
    let docx_path = dir.join("src.docx");
    std::fs::write(&docx_path, io::docx::write(&kitchen_sink()).unwrap()).unwrap();
    let out = soffice_convert(&docx_path, "html", &dir.join("lo")).expect("docx→html");
    let back = io::open(&out).unwrap();
    assert!(back.plain_text().contains("Fancy paragraph"));
    assert_eq!(find_run(&back, "bold").props.bold, Some(true));
    assert_eq!(find_run(&back, "italic").props.italic, Some(true));
    assert_eq!(find_run(&back, "red").props.color, Some(Color::rgb(255, 0, 0)));
    assert_eq!(find_para(&back, "Chapter One").props.style.as_deref(), Some("Heading1"));
    assert!(find_para(&back, "Bullet one").props.list.flatten().is_some());
    let t = back.body.iter().find_map(|b| b.as_table()).expect("table");
    assert_eq!(t.rows.len(), 3);
    assert!(back.plain_text().contains("Span two"));
    // And our HTML opens in LibreOffice.
    let path = dir.join("ours.html");
    std::fs::write(&path, html::write(&kitchen_sink())).unwrap();
    let odt = soffice_convert(&path, "odt", &dir.join("odt")).expect("html→odt");
    let d2 = io::open(&odt).unwrap();
    assert!(d2.plain_text().contains("Fancy paragraph"));
    assert!(d2.plain_text().contains("Span two"));
}
