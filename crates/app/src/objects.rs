//! Sections, floating pictures and text boxes: editing commands.

use wordcore::edit::{ParaPath, Pos};
use wordcore::*;

use crate::app::{App, EditKind, Selection};

/// A floating object being moved or resized with the mouse.
#[derive(Clone, Copy, Debug)]
pub struct FloatDrag {
    pub pos: Pos,
    pub page: usize,
    /// Pointer offset from the object's top-left corner (points).
    pub grab: (f32, f32),
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub resize: bool,
}

impl App {
    // -------------------------------------------------------------------
    // Sections
    // -------------------------------------------------------------------

    /// Section of the caret (for headers/footers: the section of the page being edited).
    pub fn current_section(&self) -> usize {
        let pos = self.sel.head;
        if pos.path.story == Story::Body {
            return self.doc.block_sections().get(pos.path.block).copied().unwrap_or(0);
        }
        let pg = self.layout.page_of(pos).unwrap_or(0);
        self.layout.pages.get(pg).map(|p| p.section).unwrap_or(0)
    }

    /// Page setup of the caret's section.
    pub fn page_setup(&self) -> PageSetup {
        self.doc.section_page(self.current_section()).clone()
    }

    fn section_page_mut(&mut self, i: usize) -> &mut PageSetup {
        let mut n = 0;
        let mut idx = None;
        for (bi, b) in self.doc.body.iter().enumerate() {
            if let Block::Para(p) = b {
                if p.section.is_some() {
                    if n == i {
                        idx = Some(bi);
                        break;
                    }
                    n += 1;
                }
            }
        }
        match idx {
            Some(bi) => match &mut self.doc.body[bi] {
                Block::Para(p) => &mut p.section.as_mut().unwrap().page,
                _ => unreachable!(),
            },
            None => &mut self.doc.page,
        }
    }

    /// Apply page setup `s` (based on [`App::page_setup`]). Size, margins, columns and
    /// header/footer options change the caret's section; watermark, page colour and page
    /// borders apply to the whole document, as in Word.
    pub fn set_page_setup(&mut self, s: PageSetup) {
        self.checkpoint("Page Setup", EditKind::Other);
        let cur = self.current_section();
        let old = self.page_setup();
        let n = self.doc.section_count();
        for i in 0..n {
            let p = self.section_page_mut(i);
            if i == cur {
                let start = p.start;
                *p = s.clone();
                p.start = start;
            }
            if s.watermark != old.watermark {
                p.watermark = s.watermark.clone();
            }
            if s.page_color != old.page_color {
                p.page_color = s.page_color;
            }
            if s.page_borders != old.page_borders {
                p.page_borders = s.page_borders.clone();
            }
        }
        self.changed();
    }

    /// Insert a section break of the given kind at the caret.
    pub fn insert_section_break(&mut self, kind: SectionStart) {
        if self.sel.head.path.story != Story::Body || self.sel.head.path.cell.is_some() {
            self.status = "Section breaks can only be inserted in the main text.".into();
            return;
        }
        self.checkpoint("Section Break", EditKind::Other);
        self.delete_selection_pub();
        let pos = self.sel.head;
        let k = self.current_section();
        let page = self.doc.section_page(k).clone();
        let headers = if self.doc.headers_owner(k) == Some(k) { Some(self.doc.section_headers(k).clone()) } else { None };
        let next = self.doc.split_paragraph(pos);
        // The first half now ends section k; the second half continues as section k+1.
        let first = ParaPath { story: Story::Body, block: pos.path.block, cell: None };
        if let Some(p) = self.doc.para_mut(&first) {
            if p.section.is_some() {
                // split_paragraph moved the old mark's section to the second half already.
            }
            p.section = Some(Box::new(Section { page, headers }));
        }
        self.section_page_mut(k + 1).start = kind;
        self.sel = Selection::caret(next);
        self.changed();
    }

    // -------------------------------------------------------------------
    // Headers and footers per section
    // -------------------------------------------------------------------

    /// The header (or footer) story shown on `page`.
    pub fn hf_story_for_page(&self, footer: bool, page: usize) -> Story {
        if let Some(p) = self.layout.pages.get(page) {
            let s = if footer { p.footer_story } else { p.header_story };
            if let Some(s) = s {
                return s;
            }
        }
        let sec = self.layout.pages.get(page).map(|p| p.section).unwrap_or(0);
        self.doc.header_story(sec, if footer { HfKind::Footer } else { HfKind::Header })
    }

    /// The header/footer story for the caret's current page.
    pub fn hf_story_here(&self, footer: bool) -> Story {
        let page = self.layout.page_of(self.sel.head).unwrap_or(0);
        self.hf_story_for_page(footer, page)
    }

    // -------------------------------------------------------------------
    // Floating objects
    // -------------------------------------------------------------------

    /// The picture or text box selected (selection covers exactly that one object).
    pub fn selected_object(&self) -> Option<(Pos, Inline)> {
        let (a, b) = self.sel.range();
        if a.path != b.path || b.offset != a.offset + 1 {
            return None;
        }
        let p = self.doc.para(&a.path)?;
        let (i, _) = p.run_at(a.offset)?;
        let run = p.runs.get(i)?;
        match &run.content {
            c @ (Inline::Image(_) | Inline::TextBox(_)) => Some((a, c.clone())),
            _ => None,
        }
    }

    fn with_object(&mut self, pos: Pos, label: &str, f: impl FnOnce(&mut Inline)) {
        self.checkpoint(label, EditKind::Other);
        if let Some(p) = self.doc.para_mut(&pos.path) {
            p.split_at(pos.offset);
            p.split_at(pos.offset + 1);
            if let Some((i, _)) = p.run_at(pos.offset) {
                if let Some(r) = p.runs.get_mut(i) {
                    f(&mut r.content);
                }
            }
            p.normalize();
        }
        self.changed();
    }

    /// Wrap Text: `None` = In Line with Text.
    pub fn set_wrap(&mut self, wrap: Option<WrapMode>) {
        let Some((pos, obj)) = self.selected_object() else { return };
        // Current position on the page, so the object doesn't jump.
        let page = self.layout.page_of(pos).unwrap_or(0);
        let fb = self.layout.pages.get(page).and_then(|p| p.floats.iter().find(|f| f.path == pos.path && f.offset == pos.offset)).map(|f| (f.x, f.y));
        let here = fb.or_else(|| self.layout.caret_on_page(pos, page).map(|c| (c.x, c.top)));
        let cur_float = match &obj {
            Inline::Image(i) => i.float,
            Inline::TextBox(t) => t.float,
            _ => None,
        };
        let new_float = wrap.map(|w| match cur_float {
            Some(f) => Float { wrap: w, ..f },
            None => {
                let (x, y) = here.unwrap_or((72.0, 72.0));
                Float { wrap: w, h_rel: HRel::Page, v_rel: VRel::Page, x, y, ..Float::default() }
            }
        });
        self.with_object(pos, "Wrap Text", |c| match c {
            Inline::Image(i) => i.float = new_float,
            Inline::TextBox(t) => t.float = new_float,
            _ => {}
        });
        self.sel = Selection { anchor: pos, head: Pos { offset: pos.offset + 1, ..pos } };
    }

    /// Move a floating object to page coordinates (x, y) in points.
    pub fn move_object(&mut self, pos: Pos, x: f32, y: f32) {
        self.with_object(pos, "Move Object", |c| {
            let f = match c {
                Inline::Image(i) => &mut i.float,
                Inline::TextBox(t) => &mut t.float,
                _ => return,
            };
            if let Some(f) = f {
                f.h_rel = HRel::Page;
                f.v_rel = VRel::Page;
                f.h_align = None;
                f.v_align = None;
                f.x = x.max(0.0);
                f.y = y.max(0.0);
            }
        });
        self.sel = Selection { anchor: pos, head: Pos { offset: pos.offset + 1, ..pos } };
    }

    /// Resize the selected picture or text box (points).
    pub fn resize_object(&mut self, pos: Pos, w: f32, h: f32) {
        self.with_object(pos, "Resize", |c| match c {
            Inline::Image(i) => {
                i.width = w.max(8.0);
                i.height = h.max(8.0);
            }
            Inline::TextBox(t) => {
                t.width = w.max(18.0);
                t.height = h.max(18.0);
            }
            _ => {}
        });
        self.sel = Selection { anchor: pos, head: Pos { offset: pos.offset + 1, ..pos } };
    }

    /// Insert a floating text box near the caret and open it for editing.
    pub fn insert_text_box(&mut self) {
        let pos = self.sel.range().0;
        let page = self.layout.page_of(pos).unwrap_or(0);
        let (x, y) = self.layout.caret_on_page(pos, page).map(|c| (c.x, c.top + 18.0)).unwrap_or((90.0, 90.0));
        let mut tb = TextBox::default();
        tb.float = Some(Float { wrap: WrapMode::Square, h_rel: HRel::Page, v_rel: VRel::Page, x, y, ..Float::default() });
        tb.width = 180.0;
        tb.height = 72.0;
        tb.paragraphs = vec![Paragraph::with_text("Type your text here")];
        if let Some(p) = tb.paragraphs.first_mut() {
            p.props.space_after = Some(0.0);
        }
        let props = self.current_chars();
        self.checkpoint("Text Box", EditKind::Other);
        self.delete_selection_pub();
        let at = self.sel.head;
        let end = self.doc.insert_inline(at, Inline::TextBox(Box::new(tb)), &CharProps { style: props.style.clone(), ..Default::default() });
        self.sel = Selection { anchor: at, head: end };
        self.changed();
        self.open_text_box_dialog();
    }

    /// Edit the selected text box's text in a dialog.
    pub fn open_text_box_dialog(&mut self) {
        let Some((pos, Inline::TextBox(tb))) = self.selected_object() else { return };
        let text = tb.paragraphs.iter().map(|p| p.text()).collect::<Vec<_>>().join("\n");
        self.dialog = Some(crate::dialogs::Dialog::TextBox {
            pos,
            text,
            fill: tb.fill,
            border: tb.border.is_some(),
            width: tb.width,
            height: tb.height,
        });
    }

    /// Apply the Text Box dialog.
    pub fn apply_text_box(&mut self, pos: Pos, text: &str, fill: Option<Color>, border: bool, width: f32, height: f32) {
        self.with_object(pos, "Text Box", |c| {
            if let Inline::TextBox(tb) = c {
                let template = tb.paragraphs.first().cloned().unwrap_or_default();
                let run_props = template.runs.first().map(|r| r.props.clone()).unwrap_or_default();
                tb.paragraphs = text
                    .split('\n')
                    .map(|line| {
                        let mut p = Paragraph { props: template.props.clone(), mark: template.mark.clone(), ..Default::default() };
                        if !line.is_empty() {
                            p.runs.push(Run::styled(line, run_props.clone()));
                        }
                        p
                    })
                    .collect();
                tb.fill = fill;
                tb.border = if border { Some(tb.border.unwrap_or(Border { style: BorderStyle::Single, width: 0.75, color: Color::BLACK, space: 0.0 })) } else { None };
                tb.width = width.max(18.0);
                tb.height = height.max(18.0);
            }
        });
        self.sel = Selection { anchor: pos, head: Pos { offset: pos.offset + 1, ..pos } };
    }
}
