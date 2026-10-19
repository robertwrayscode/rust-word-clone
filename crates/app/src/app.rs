//! Application state and editing commands.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use wordcore::edit::{ordered, ParaPath, Pos};
use wordcore::fonts::FontDb;
use wordcore::layout::{layout, DocLayout, LayoutOptions};
use wordcore::*;

use crate::dialogs::Dialog;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RibbonTab {
    Home,
    Insert,
    Design,
    Layout,
    References,
    Review,
    View,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selection {
    pub anchor: Pos,
    pub head: Pos,
}

impl Selection {
    pub fn caret(p: Pos) -> Self {
        Selection { anchor: p, head: p }
    }
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
    pub fn range(&self) -> (Pos, Pos) {
        ordered(self.anchor, self.head)
    }
}

#[derive(Clone)]
struct Snapshot {
    doc: Document,
    sel: Selection,
    label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Deleting,
    Other,
}

/// A request that may need confirmation when the document has unsaved changes.
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    New,
    Open(Option<PathBuf>),
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkupMode {
    AllMarkup,
    NoMarkup,
    Original,
}

pub struct ScreenshotState {
    pub path: PathBuf,
    pub requested: bool,
    pub done: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct FindState {
    pub query: String,
    pub replace: String,
    pub opts: FindOptions,
    pub matches: Vec<(Pos, Pos)>,
    pub current: usize,
}

pub struct App {
    pub doc: Document,
    pub db: &'static FontDb,
    pub layout: DocLayout,
    pub layout_dirty: bool,
    pub sel: Selection,
    /// Desired x for up/down movement.
    pub goal_x: Option<f32>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit: Option<(EditKind, Pos)>,
    pub path: Option<PathBuf>,
    pub modified: bool,
    pub zoom: f32,
    pub ribbon_tab: RibbonTab,
    pub dialog: Option<Dialog>,
    pub show_marks: bool,
    pub show_ruler: bool,
    pub show_nav: bool,
    pub markup: MarkupMode,
    /// Formatting to apply to the next typed text (set by toggling bold etc. with an empty selection).
    pub pending_chars: Option<CharProps>,
    pub format_painter: Option<(CharProps, ParaProps, bool)>,
    pub find: FindState,
    /// Internal rich clipboard: blocks + the plain text we put on the system clipboard.
    pub clip: Option<(Vec<Block>, String)>,
    pub recent: Vec<PathBuf>,
    pub status: String,
    pub args: crate::Args,
    pub screenshot: Option<ScreenshotState>,
    pub frame_no: u64,
    pub allow_close: bool,
    pub pending: Option<Pending>,
    /// Request the canvas to scroll the caret into view.
    pub scroll_to_caret: bool,
    /// Request scrolling to a specific page.
    pub scroll_to_page: Option<usize>,
    pub font_color: Color,
    pub highlight_color: Color,
    pub caret_blink: f64,
    pub textures: HashMap<usize, egui::TextureHandle>,
    pub canvas_focused: bool,
    pub side_panel: Option<SidePanel>,
    pub last_page_count: u32,
    pub view_pages: u32,
    pending_kind: EditKind,
    pub canvas_origin_x: f32,
    pub ime_rect: Option<egui::Rect>,
    pub focus_search: bool,
    pub search_text: String,
    pub zoom_mode: Option<ZoomMode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomMode {
    OnePage,
    PageWidth,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SidePanel {
    Styles,
    Comments,
    Thesaurus { word: String },
    Spelling,
}

impl App {
    pub fn new(args: crate::Args) -> Self {
        let db = FontDb::global();
        let mut app = App {
            doc: Document::new(),
            db,
            layout: DocLayout::default(),
            layout_dirty: true,
            sel: Selection::caret(Pos::default()),
            goal_x: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            path: None,
            modified: false,
            zoom: 1.0,
            ribbon_tab: RibbonTab::Home,
            dialog: None,
            show_marks: false,
            show_ruler: true,
            show_nav: false,
            markup: MarkupMode::AllMarkup,
            pending_chars: None,
            format_painter: None,
            find: FindState::default(),
            clip: None,
            recent: crate::fileio::load_recent(),
            status: String::new(),
            screenshot: args.screenshot.clone().map(|p| ScreenshotState { path: p, requested: false, done: Arc::new(AtomicBool::new(false)) }),
            args: args.clone(),
            frame_no: 0,
            allow_close: false,
            pending: None,
            scroll_to_caret: false,
            scroll_to_page: None,
            font_color: Color::RED,
            highlight_color: Color::YELLOW,
            caret_blink: 0.0,
            textures: HashMap::new(),
            canvas_focused: true,
            side_panel: None,
            last_page_count: 1,
            view_pages: 1,
            pending_kind: EditKind::Other,
            canvas_origin_x: 0.0,
            ime_rect: None,
            focus_search: false,
            search_text: String::new(),
            zoom_mode: None,
        };
        app.doc.settings.author = crate::fileio::user_name();
        if args.demo {
            app.doc = crate::demo::demo_document();
            app.doc.settings.author = crate::fileio::user_name();
        } else if let Some(f) = &args.file {
            app.open_path(f.clone());
        }
        app.relayout();
        app
    }

    // -------------------------------------------------------------------
    // Layout
    // -------------------------------------------------------------------

    pub fn layout_options(&self) -> LayoutOptions {
        let mut o = LayoutOptions::new();
        o.page_count_hint = self.last_page_count.max(1);
        o.show_hidden = self.show_marks;
        match self.markup {
            MarkupMode::AllMarkup => {}
            MarkupMode::NoMarkup => {
                o.show_markup = false;
                o.show_deletions = false;
            }
            MarkupMode::Original => {
                o.show_markup = false;
            }
        }
        o
    }

    pub fn relayout(&mut self) {
        let opts = self.layout_options();
        self.layout = layout(&self.doc, self.db, &opts);
        let n = self.layout.pages.len() as u32;
        if n != self.last_page_count {
            self.last_page_count = n;
            // NUMPAGES fields depend on the page count: lay out once more.
            let opts = self.layout_options();
            self.layout = layout(&self.doc, self.db, &opts);
        }
        self.layout_dirty = false;
    }

    pub fn ensure_layout(&mut self) {
        if self.layout_dirty {
            self.relayout();
        }
    }

    pub fn title(&self) -> String {
        let name = self.path.as_ref().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Document1".into());
        format!("{}{} - RWord", name, if self.modified { " •" } else { "" })
    }

    // -------------------------------------------------------------------
    // Undo
    // -------------------------------------------------------------------

    /// Record the state before an edit. Consecutive typing/deleting is merged into one undo step.
    pub fn checkpoint(&mut self, label: &str, kind: EditKind) {
        let merge = match (self.last_edit, kind) {
            (Some((k, p)), EditKind::Typing | EditKind::Deleting) => k == kind && p == self.sel.head && self.sel.is_empty(),
            _ => false,
        };
        if !merge {
            self.undo.push(Snapshot { doc: self.doc.clone(), sel: self.sel, label: label.to_string() });
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_edit = None;
        self.pending_kind = kind;
    }

    /// Mark the document changed after an edit.
    pub fn changed(&mut self) {
        self.modified = true;
        self.layout_dirty = true;
        self.scroll_to_caret = true;
        self.goal_x = None;
        self.caret_blink = 0.0;
        if matches!(self.pending_kind, EditKind::Typing | EditKind::Deleting) {
            self.last_edit = Some((self.pending_kind, self.sel.head));
        } else {
            self.last_edit = None;
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|s| s.label.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }

    pub fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            let cur = Snapshot { doc: std::mem::replace(&mut self.doc, s.doc), sel: self.sel, label: s.label.clone() };
            self.redo.push(cur);
            self.sel = s.sel;
            self.after_history();
        }
    }

    pub fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            let cur = Snapshot { doc: std::mem::replace(&mut self.doc, s.doc), sel: self.sel, label: s.label.clone() };
            self.undo.push(cur);
            self.sel = s.sel;
            self.after_history();
        }
    }

    fn after_history(&mut self) {
        self.sel.anchor = self.doc.clamp(self.sel.anchor);
        self.sel.head = self.doc.clamp(self.sel.head);
        self.last_edit = None;
        self.modified = true;
        self.layout_dirty = true;
        self.scroll_to_caret = true;
    }

    // -------------------------------------------------------------------
    // Selection helpers
    // -------------------------------------------------------------------

    pub fn story(&self) -> Story {
        self.sel.head.path.story
    }

    pub fn set_caret(&mut self, p: Pos, extend: bool) {
        let p = self.doc.clamp(p);
        self.sel.head = p;
        if !extend {
            self.sel.anchor = p;
        }
        self.pending_chars = None;
        self.caret_blink = 0.0;
        self.scroll_to_caret = true;
        self.last_edit = None;
    }

    pub fn select(&mut self, a: Pos, b: Pos) {
        self.sel = Selection { anchor: self.doc.clamp(a), head: self.doc.clamp(b) };
        self.pending_chars = None;
        self.scroll_to_caret = true;
        self.last_edit = None;
    }

    pub fn select_all(&mut self) {
        let s = self.story();
        let a = self.doc.start_of(s);
        let b = self.doc.end_of(s);
        self.select(a, b);
    }

    pub fn selected_text(&self) -> String {
        let (a, b) = self.sel.range();
        self.doc.text_range(a, b)
    }

    /// Formatting at the caret / common to the selection, including pending toggles.
    pub fn current_chars(&self) -> CharProps {
        let (a, b) = self.sel.range();
        let mut c = self.doc.common_chars(a, b);
        if let Some(p) = &self.pending_chars {
            c.merge(p);
        }
        c
    }

    /// Resolved formatting at the caret (for the ribbon display).
    pub fn current_resolved(&self) -> wordcore::styles::ResolvedChars {
        let c = self.current_chars();
        let style = self.doc.para(&self.sel.head.path).map(|p| p.style_id().to_string()).unwrap_or_else(|| "Normal".into());
        wordcore::styles::resolve_chars(&self.doc, &style, &c)
    }

    pub fn current_para(&self) -> wordcore::styles::ResolvedPara {
        let p = self.doc.para(&self.sel.head.path).map(|p| p.props.clone()).unwrap_or_default();
        wordcore::styles::resolve_para(&self.doc, &p)
    }

    pub fn current_para_props(&self) -> ParaProps {
        self.doc.para(&self.sel.head.path).map(|p| p.props.clone()).unwrap_or_default()
    }

    // -------------------------------------------------------------------
    // Text editing
    // -------------------------------------------------------------------

    /// Delete the selection if any; returns true if something was deleted.
    fn delete_selection(&mut self) -> bool {
        if self.sel.is_empty() {
            return false;
        }
        let (a, b) = self.sel.range();
        let p = self.doc.delete_range(a, b);
        self.sel = Selection::caret(p);
        true
    }

    /// Typing formatting at the caret.
    fn typing_props(&self) -> CharProps {
        let pos = self.sel.range().0;
        let mut props = self.doc.para(&pos.path).map(|p| if p.is_empty() { p.mark.clone() } else { p.props_at(pos.offset) }).unwrap_or_default();
        props.revision = None;
        props.comments.clear();
        // Typing after a hyperlink doesn't extend it.
        if props.link.is_some() {
            if let Some(p) = self.doc.para(&pos.path) {
                if pos.offset >= p.len() || p.props_at(pos.offset + 1).link != props.link {
                    props.link = None;
                    if props.style.as_deref() == Some("Hyperlink") {
                        props.style = None;
                    }
                }
            }
        }
        if let Some(p) = &self.pending_chars {
            props.merge(p);
            // explicit "off" toggles
            if p.vert_align == Some(VertAlign::Baseline) {
                props.vert_align = None;
            }
        }
        props
    }

    pub fn type_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.checkpoint("Typing", EditKind::Typing);
        let props = self.typing_props();
        self.delete_selection();
        let mut pos = self.sel.head;
        let mut text = text.to_string();
        // Smart quotes and AutoCorrect.
        if crate::proof::autocorrect_enabled() && text.chars().count() == 1 {
            let c = text.chars().next().unwrap();
            if c == '"' || c == '\'' {
                let prev = if pos.offset > 0 { self.doc.para(&pos.path).and_then(|p| p.char_at(pos.offset - 1)) } else { None };
                text = crate::proof::smart_quote(prev, c).to_string();
            } else if !c.is_alphanumeric() && c != '\'' && c != '’' {
                if let Some((del, ins)) = crate::proof::autocorrect(&self.doc, pos, c) {
                    let start = Pos { offset: pos.offset - del, ..pos };
                    let rp = self.doc.para(&pos.path).map(|p| p.props_at(start.offset + 1)).unwrap_or_default();
                    let at = self.doc.delete_range(start, pos);
                    pos = self.doc.insert_text(at, &ins, &rp);
                }
            }
        }
        let p = self.doc.insert_text(pos, &text, &props);
        self.sel = Selection::caret(p);
        self.pending_chars = None;
        self.changed();
    }

    pub fn insert_inline(&mut self, content: Inline, label: &str) {
        self.checkpoint(label, EditKind::Other);
        let props = self.typing_props();
        self.delete_selection();
        let p = self.doc.insert_inline(self.sel.head, content, &props);
        self.sel = Selection::caret(p);
        self.changed();
    }

    pub fn enter(&mut self) {
        // Enter in an empty list item ends the list.
        let pos = self.sel.head;
        if self.sel.is_empty() {
            if let Some(p) = self.doc.para(&pos.path) {
                if p.is_empty() && resolved_list(&self.doc, p).is_some() {
                    self.checkpoint("End List", EditKind::Other);
                    self.doc.apply_para(pos, pos, &|pp| {
                        pp.list = Some(None);
                        pp.indent_left = None;
                        pp.indent_first = None;
                        if pp.style.as_deref() == Some("ListParagraph") {
                            pp.style = None;
                        }
                    });
                    self.changed();
                    return;
                }
            }
        }
        self.checkpoint("Typing", EditKind::Other);
        // AutoCorrect the last word before splitting.
        if self.sel.is_empty() && crate::proof::autocorrect_enabled() {
            if let Some((del, ins)) = crate::proof::autocorrect(&self.doc, pos, '\n') {
                let start = Pos { offset: pos.offset - del, ..pos };
                let rp = self.doc.para(&pos.path).map(|p| p.props_at(start.offset + 1)).unwrap_or_default();
                let at = self.doc.delete_range(start, pos);
                let np = self.doc.insert_text(at, &ins, &rp);
                self.sel = Selection::caret(np);
            }
        }
        self.delete_selection();
        let p = self.doc.split_paragraph(self.sel.head);
        self.sel = Selection::caret(p);
        self.changed();
    }

    pub fn backspace(&mut self, word: bool) {
        if !self.sel.is_empty() {
            self.checkpoint("Delete", EditKind::Other);
            self.delete_selection();
            self.changed();
            return;
        }
        let pos = self.sel.head;
        if pos.offset == 0 {
            // At paragraph start: remove list/indent first, like Word.
            if let Some(p) = self.doc.para(&pos.path) {
                let has_list = p.props.list.flatten().is_some() || resolved_list(&self.doc, p).is_some();
                if has_list {
                    self.checkpoint("Remove List", EditKind::Other);
                    self.doc.apply_para(pos, pos, &|pp| {
                        pp.list = Some(None);
                    });
                    self.changed();
                    return;
                }
                if p.props.indent_first.unwrap_or(0.0) > 0.0 || p.props.indent_left.unwrap_or(0.0) > 0.0 {
                    self.checkpoint("Indent", EditKind::Other);
                    self.doc.apply_para(pos, pos, &|pp| {
                        if pp.indent_first.unwrap_or(0.0) > 0.0 {
                            pp.indent_first = None;
                        } else {
                            pp.indent_left = Some((pp.indent_left.unwrap_or(0.0) - 36.0).max(0.0));
                        }
                    });
                    self.changed();
                    return;
                }
            }
            // Don't merge out of a table cell.
            if pos.path.cell.is_some() && pos.path.cell.map(|c| c.para == 0).unwrap_or(false) {
                return;
            }
            if let Some(prev) = self.doc.prev_path(&pos.path) {
                if prev.cell.is_some() != pos.path.cell.is_some() {
                    // previous block is a table: move caret into it
                    let len = self.doc.para(&prev).map(|p| p.len()).unwrap_or(0);
                    if self.doc.para(&pos.path).map(|p| p.is_empty()).unwrap_or(false) && pos.path.cell.is_none() {
                        // delete the empty paragraph after a table unless it's the last block
                        if pos.path.block + 1 < self.doc.story(pos.path.story).len() {
                            self.checkpoint("Delete", EditKind::Deleting);
                            self.doc.story_mut(pos.path.story).remove(pos.path.block);
                            self.sel = Selection::caret(prev.at(len));
                            self.changed();
                            return;
                        }
                    }
                    self.set_caret(prev.at(len), false);
                    return;
                }
            }
        }
        let start = if word { self.doc.word_left(pos) } else { self.doc.left(pos) };
        if start == pos {
            return;
        }
        self.checkpoint("Delete", EditKind::Deleting);
        let p = self.doc.delete_range(start, pos);
        self.sel = Selection::caret(if self.doc.settings.track_changes { start } else { p });
        self.changed();
    }

    pub fn delete_forward(&mut self, word: bool) {
        if !self.sel.is_empty() {
            self.checkpoint("Delete", EditKind::Other);
            self.delete_selection();
            self.changed();
            return;
        }
        let pos = self.sel.head;
        let len = self.doc.para(&pos.path).map(|p| p.len()).unwrap_or(0);
        if pos.offset >= len {
            if let Some(next) = self.doc.next_path(&pos.path) {
                if next.cell.is_some() != pos.path.cell.is_some() || !next.same_cell(&pos.path) && next.cell.is_some() {
                    // Deleting into a table: delete this paragraph if empty, else nothing.
                    if len == 0 && pos.path.cell.is_none() {
                        self.checkpoint("Delete", EditKind::Deleting);
                        self.doc.story_mut(pos.path.story).remove(pos.path.block);
                        let np = self.doc.clamp(ParaPath { block: pos.path.block, ..next }.at(0));
                        self.sel = Selection::caret(np);
                        self.changed();
                    }
                    return;
                }
            } else {
                return;
            }
        }
        let end = if word { self.doc.word_right(pos) } else { self.doc.right(pos) };
        if end == pos {
            return;
        }
        self.checkpoint("Delete", EditKind::Deleting);
        let p = self.doc.delete_range(pos, end);
        self.sel = Selection::caret(if self.doc.settings.track_changes { end } else { p });
        self.changed();
    }

    /// Tab key: next cell in tables, list level, or a tab character.
    pub fn tab(&mut self, shift: bool) {
        let pos = self.sel.head;
        if let Some(c) = pos.path.cell {
            let next = if shift { self.prev_cell(pos.path) } else { self.next_cell(pos.path) };
            match next {
                Some(p) => {
                    let len = self.doc.para(&p).map(|x| x.len()).unwrap_or(0);
                    self.select(p.at(0), p.at(len));
                }
                None if !shift => {
                    // Tab in the last cell adds a row.
                    self.table_insert_row(false);
                    let np = ParaPath { cell: Some(CellRef { row: c.row + 1, col: 0, para: 0 }), ..pos.path };
                    self.set_caret(np.at(0), false);
                }
                None => {}
            }
            return;
        }
        if let Some(p) = self.doc.para(&pos.path) {
            let at_start = self.sel.range().0.offset == 0 && pos.path == self.sel.anchor.path;
            if resolved_list(&self.doc, p).is_some() && (at_start || !self.sel.is_empty()) {
                self.change_list_level(if shift { -1 } else { 1 });
                return;
            }
            if shift {
                self.change_indent(-1);
                return;
            }
        }
        self.insert_inline(Inline::Tab, "Typing");
    }

    fn next_cell(&self, path: ParaPath) -> Option<ParaPath> {
        let c = path.cell?;
        let t = self.doc.story(path.story).get(path.block)?.as_table()?;
        let (mut r, mut col) = (c.row, c.col + 1);
        while r < t.rows.len() {
            while col < t.rows[r].cells.len() {
                if t.rows[r].cells[col].vmerge != VMerge::Continue {
                    return Some(ParaPath { cell: Some(CellRef { row: r, col, para: 0 }), ..path });
                }
                col += 1;
            }
            r += 1;
            col = 0;
        }
        None
    }

    fn prev_cell(&self, path: ParaPath) -> Option<ParaPath> {
        let c = path.cell?;
        let t = self.doc.story(path.story).get(path.block)?.as_table()?;
        let (mut r, mut col) = (c.row as isize, c.col as isize - 1);
        while r >= 0 {
            while col >= 0 {
                if t.rows[r as usize].cells[col as usize].vmerge != VMerge::Continue {
                    return Some(ParaPath { cell: Some(CellRef { row: r as usize, col: col as usize, para: 0 }), ..path });
                }
                col -= 1;
            }
            r -= 1;
            if r >= 0 {
                col = t.rows[r as usize].cells.len() as isize - 1;
            }
        }
        None
    }

    // -------------------------------------------------------------------
    // Formatting
    // -------------------------------------------------------------------

    /// Apply character formatting to the selection (or the word at the caret / pending typing format).
    pub fn apply_chars(&mut self, label: &str, f: impl Fn(&mut CharProps) + 'static) {
        if self.sel.is_empty() {
            let pos = self.sel.head;
            // Word applies to the whole word when the caret is inside a word.
            let (a, b) = self.doc.word_at(pos);
            let inside = a.offset < pos.offset && pos.offset < b.offset && self.doc.para(&pos.path).map(|p| {
                let ch = p.chars_vec();
                pos.offset < ch.len() && wordcore::edit::is_word_char(ch[pos.offset]) && wordcore::edit::is_word_char(ch[pos.offset - 1])
            }).unwrap_or(false);
            if inside {
                // trim trailing spaces from the word range
                let ch = self.doc.para(&pos.path).map(|p| p.chars_vec()).unwrap_or_default();
                let mut e = b.offset;
                while e > a.offset && ch.get(e - 1).map(|c| c.is_whitespace()).unwrap_or(false) {
                    e -= 1;
                }
                self.checkpoint(label, EditKind::Other);
                self.doc.apply_chars(a, Pos { offset: e, ..b }, &f);
                self.changed();
                return;
            }
            let mut p = self.pending_chars.take().unwrap_or_default();
            let mut cur = self.current_chars();
            f(&mut cur);
            // record the changed fields
            let mut probe = CharProps::default();
            f(&mut probe);
            p.merge(&cur_diff(&probe, &cur));
            self.pending_chars = Some(p);
            // empty paragraph: also set the paragraph mark
            if self.doc.para(&pos.path).map(|p| p.is_empty()).unwrap_or(false) {
                self.checkpoint(label, EditKind::Other);
                self.doc.apply_chars(pos, pos, &f);
                self.changed();
            }
            return;
        }
        self.checkpoint(label, EditKind::Other);
        let (a, b) = self.sel.range();
        self.doc.apply_chars(a, b, &f);
        self.changed();
    }

    pub fn toggle_bold(&mut self) {
        let on = !self.current_resolved().bold;
        self.apply_chars("Bold", move |c| c.bold = Some(on));
    }
    pub fn toggle_italic(&mut self) {
        let on = !self.current_resolved().italic;
        self.apply_chars("Italic", move |c| c.italic = Some(on));
    }
    pub fn toggle_underline(&mut self, kind: Underline) {
        let cur = self.current_resolved().underline;
        let v = if cur == kind { Underline::None } else { kind };
        self.apply_chars("Underline", move |c| c.underline = Some(v));
    }
    pub fn toggle_strike(&mut self) {
        let on = !self.current_resolved().strike;
        self.apply_chars("Strikethrough", move |c| c.strike = Some(on));
    }
    pub fn toggle_vert(&mut self, v: VertAlign) {
        let cur = self.current_resolved().vert_align;
        let nv = if cur == v { VertAlign::Baseline } else { v };
        self.apply_chars(if v == VertAlign::Superscript { "Superscript" } else { "Subscript" }, move |c| c.vert_align = Some(nv));
    }
    pub fn set_font(&mut self, name: &str) {
        let n = name.to_string();
        self.apply_chars("Font", move |c| c.font = Some(n.clone()));
    }
    pub fn set_size(&mut self, size: f32) {
        let s = size.clamp(1.0, 1638.0);
        self.apply_chars("Font Size", move |c| c.size = Some(s));
    }
    pub fn grow_font(&mut self, up: bool) {
        const SIZES: [f32; 16] = [8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0, 72.0];
        let cur = self.current_resolved().size;
        let next = if up {
            SIZES.iter().copied().find(|s| *s > cur + 0.01).unwrap_or(cur + 10.0)
        } else {
            SIZES.iter().rev().copied().find(|s| *s < cur - 0.01).unwrap_or((cur - 1.0).max(1.0))
        };
        self.set_size(next);
    }
    pub fn set_color(&mut self, color: Option<Color>) {
        self.apply_chars("Font Color", move |c| c.color = Some(color.unwrap_or(Color::BLACK)));
    }
    pub fn set_highlight(&mut self, color: Option<Color>) {
        self.apply_chars("Highlight", move |c| c.highlight = color);
    }
    pub fn clear_formatting(&mut self) {
        self.checkpoint("Clear Formatting", EditKind::Other);
        let (a, b) = self.sel.range();
        self.doc.apply_chars(a, b, &|c| {
            let keep = (c.link.clone(), c.revision.clone(), c.comments.clone());
            *c = CharProps::default();
            c.link = keep.0;
            c.revision = keep.1;
            c.comments = keep.2;
        });
        self.doc.apply_para(a, b, &|p| {
            let list = p.list;
            *p = ParaProps::default();
            p.list = list;
        });
        self.pending_chars = None;
        self.changed();
    }

    pub fn change_case(&mut self, mode: u8) {
        if self.sel.is_empty() {
            let (a, b) = self.doc.word_at(self.sel.head);
            self.select(a, b);
        }
        let (a, b) = self.sel.range();
        let text = self.doc.text_range(a, b);
        let new: String = match mode {
            0 => {
                // Sentence case.
                let mut out = String::new();
                let mut cap = true;
                for c in text.chars() {
                    if cap && c.is_alphabetic() {
                        out.extend(c.to_uppercase());
                        cap = false;
                    } else {
                        out.extend(c.to_lowercase());
                    }
                    if matches!(c, '.' | '!' | '?' | '\n') {
                        cap = true;
                    }
                }
                out
            }
            1 => text.to_lowercase(),
            2 => text.to_uppercase(),
            3 => {
                let mut out = String::new();
                let mut start = true;
                for c in text.chars() {
                    if start && c.is_alphabetic() {
                        out.extend(c.to_uppercase());
                    } else {
                        out.extend(c.to_lowercase());
                    }
                    start = c.is_whitespace() || c == '-';
                }
                out
            }
            _ => text.chars().map(|c| if c.is_uppercase() { c.to_lowercase().next().unwrap_or(c) } else { c.to_uppercase().next().unwrap_or(c) }).collect(),
        };
        if new == text {
            return;
        }
        self.checkpoint("Change Case", EditKind::Other);
        // Replace char by char within each paragraph to keep formatting.
        let paths = self.doc.paths_in_range(a, b);
        let mut it = new.chars().filter(|c| *c != '\n');
        for path in paths {
            let s = if path == a.path { a.offset } else { 0 };
            let Some(p) = self.doc.para_mut(&path) else { continue };
            let e = if path == b.path { b.offset } else { p.len() };
            let mut acc = 0;
            for r in p.runs.iter_mut() {
                let l = r.len();
                if let Inline::Text(t) = &mut r.content {
                    let mut nt = String::new();
                    for (i, ch) in t.chars().enumerate() {
                        let o = acc + i;
                        if o >= s && o < e {
                            nt.push(it.next().unwrap_or(ch));
                        } else {
                            nt.push(ch);
                        }
                    }
                    *t = nt;
                } else if acc >= s && acc < e {
                    it.next();
                }
                acc += l;
            }
        }
        self.changed();
    }

    pub fn apply_para(&mut self, label: &str, f: impl Fn(&mut ParaProps)) {
        self.checkpoint(label, EditKind::Other);
        let (a, b) = self.sel.range();
        self.doc.apply_para(a, b, &f);
        self.changed();
    }

    pub fn set_align(&mut self, al: Align) {
        let cur = self.current_para().align;
        let v = if cur == al && al != Align::Left { Align::Left } else { al };
        self.apply_para("Align", move |p| p.align = Some(v));
    }

    pub fn set_line_spacing(&mut self, ls: LineSpacing) {
        self.apply_para("Line Spacing", move |p| p.line_spacing = Some(ls));
    }

    pub fn change_indent(&mut self, dir: i32) {
        let (a, b) = self.sel.range();
        let paths = self.doc.paths_in_range(a, b);
        // Lists change level instead.
        if paths.iter().all(|p| self.doc.para(p).map(|x| resolved_list(&self.doc, x).is_some()).unwrap_or(false)) {
            self.change_list_level(dir);
            return;
        }
        self.checkpoint("Indent", EditKind::Other);
        let doc = &self.doc;
        let updates: Vec<(ParaPath, f32)> = paths
            .iter()
            .filter_map(|p| doc.para(p).map(|x| (*p, wordcore::styles::resolve_para(doc, &x.props).indent_left)))
            .collect();
        for (p, cur) in updates {
            let step = 36.0;
            let nv = if dir > 0 { ((cur / step).floor() + 1.0) * step } else { (((cur - 0.01) / step).ceil() - 1.0).max(0.0) * step };
            let maxw = self.doc.page.text_width() - 36.0;
            if let Some(x) = self.doc.para_mut(&p) {
                x.props.indent_left = Some(nv.min(maxw));
            }
        }
        self.changed();
    }

    pub fn change_list_level(&mut self, dir: i32) {
        self.checkpoint("List Level", EditKind::Other);
        let (a, b) = self.sel.range();
        for p in self.doc.paths_in_range(a, b) {
            let doc = &self.doc;
            let Some(x) = doc.para(&p) else { continue };
            if let Some(l) = resolved_list(doc, x) {
                let nl = (l.level as i32 + dir).clamp(0, 8) as u8;
                if let Some(x) = self.doc.para_mut(&p) {
                    x.props.list = Some(Some(ListRef { list: l.list, level: nl }));
                    x.props.indent_left = None;
                    x.props.indent_first = None;
                }
            }
        }
        self.changed();
    }

    /// Toggle bullets/numbering. `kind`: 0 bullets, 1 numbers, 2 multilevel. `def` overrides the list definition.
    pub fn toggle_list(&mut self, kind: u8, def: Option<ListDef>) {
        let cur = self.doc.para(&self.sel.head.path).and_then(|p| resolved_list(&self.doc, p));
        let cur_kind = cur.and_then(|l| self.doc.list(l.list)).map(|d| match d.levels[0].format {
            NumFormat::Bullet => 0,
            _ if d.levels[1].text.contains("%1") => 2,
            _ => 1,
        });
        self.checkpoint(if kind == 0 { "Bullets" } else { "Numbering" }, EditKind::Other);
        let (a, b) = self.sel.range();
        if cur_kind == Some(kind) && def.is_none() {
            self.doc.apply_para(a, b, &|p| {
                p.list = Some(None);
                p.indent_left = None;
                p.indent_first = None;
                if p.style.as_deref() == Some("ListParagraph") {
                    p.style = None;
                }
            });
        } else {
            // Continue an adjacent list of the same kind when possible.
            let reuse = def.is_none().then(|| self.adjacent_list(a, kind)).flatten();
            let id = match reuse {
                Some(id) => id,
                None => {
                    let d = def.unwrap_or_else(|| match kind {
                        0 => ListDef::bullets(0),
                        1 => ListDef::numbered(0),
                        _ => ListDef::multilevel(0),
                    });
                    self.doc.add_list(d)
                }
            };
            self.doc.apply_para(a, b, &move |p| {
                let level = p.list.flatten().map(|l| l.level).unwrap_or(0);
                p.list = Some(Some(ListRef { list: id, level }));
                p.indent_left = None;
                p.indent_first = None;
                if p.style.is_none() || p.style.as_deref() == Some("Normal") {
                    p.style = Some("ListParagraph".into());
                }
            });
        }
        self.changed();
    }

    fn adjacent_list(&self, pos: Pos, kind: u8) -> Option<u32> {
        let prev = self.doc.prev_path(&pos.path)?;
        let p = self.doc.para(&prev)?;
        let l = resolved_list(&self.doc, p)?;
        let d = self.doc.list(l.list)?;
        let k = match d.levels[0].format {
            NumFormat::Bullet => 0,
            _ if d.levels[1].text.contains("%1") => 2,
            _ => 1,
        };
        (k == kind).then_some(l.list)
    }

    pub fn apply_style(&mut self, style_id: &str) {
        let Some(st) = self.doc.style(style_id).cloned() else { return };
        match st.kind {
            StyleKind::Character => {
                let id = style_id.to_string();
                self.apply_chars("Style", move |c| c.style = Some(id.clone()));
            }
            _ => {
                let id = style_id.to_string();
                self.apply_para("Style", move |p| {
                    p.style = if id == "Normal" { None } else { Some(id.clone()) };
                    // Applying a heading style clears direct paragraph spacing like Word.
                });
            }
        }
    }

    // -------------------------------------------------------------------
    // Clipboard
    // -------------------------------------------------------------------

    pub fn copy(&mut self, cut: bool) -> Option<String> {
        if self.sel.is_empty() {
            return None;
        }
        let (a, b) = self.sel.range();
        let blocks = self.doc.extract(a, b);
        let text = self.doc.text_range(a, b);
        crate::clipboard::set(&text, &blocks, &self.doc);
        self.clip = Some((blocks, text.clone()));
        if cut {
            self.checkpoint("Cut", EditKind::Other);
            self.delete_selection();
            self.changed();
        }
        Some(text)
    }

    /// Paste from the system clipboard (rich when it is our own content). `text` is from the paste event.
    pub fn paste(&mut self, text: Option<String>, plain: bool) {
        let sys_text = text.or_else(crate::clipboard::get_text);
        let blocks = match (&self.clip, &sys_text) {
            (Some((b, t)), Some(s)) if !plain && normalize_nl(t) == normalize_nl(s) => Some(b.clone()),
            (Some((b, _)), None) if !plain => Some(b.clone()),
            _ => None,
        };
        let blocks = blocks.or_else(|| if plain { None } else { crate::clipboard::get_html_blocks(&mut self.doc) });
        if let Some(img) = (blocks.is_none() && sys_text.is_none()).then(crate::clipboard::get_image).flatten() {
            self.insert_image(img, "Paste");
            return;
        }
        self.checkpoint("Paste", EditKind::Other);
        self.delete_selection();
        let pos = self.sel.head;
        let p = match blocks {
            Some(b) => self.doc.insert_blocks(pos, b),
            None => {
                let props = self.typing_props();
                let t = sys_text.unwrap_or_default();
                self.doc.insert_text(pos, &t, &props)
            }
        };
        self.sel = Selection::caret(p);
        self.changed();
    }

    // -------------------------------------------------------------------
    // Insert
    // -------------------------------------------------------------------

    pub fn insert_page_break(&mut self) {
        self.checkpoint("Page Break", EditKind::Other);
        self.delete_selection();
        let props = self.typing_props();
        let pos = self.doc.insert_inline(self.sel.head, Inline::PageBreak, &props);
        // Word puts the page break in its own paragraph; we split after it.
        let p = self.doc.split_paragraph(pos);
        self.sel = Selection::caret(p);
        self.changed();
    }

    pub fn insert_table(&mut self, rows: usize, cols: usize) {
        if self.sel.head.path.cell.is_some() {
            self.status = "Tables can't be nested in this version.".into();
            return;
        }
        self.checkpoint("Insert Table", EditKind::Other);
        self.delete_selection();
        let pos = self.sel.head;
        let story = pos.path.story;
        let width = self.doc.page.column_width();
        let mut t = Table::new(rows, cols, width);
        // Use the current paragraph's style formatting in cells: plain Normal without spacing after.
        for r in &mut t.rows {
            for c in &mut r.cells {
                for p in &mut c.paragraphs {
                    p.props.space_after = Some(0.0);
                    p.props.line_spacing = Some(LineSpacing::Multiple(1.0));
                }
            }
        }
        // Split paragraph at caret; insert table between.
        let len = self.doc.para(&pos.path).map(|p| p.len()).unwrap_or(0);
        let insert_at = if pos.offset == 0 && len == 0 {
            pos.path.block
        } else if pos.offset == 0 {
            pos.path.block
        } else {
            let np = self.doc.split_paragraph(pos);
            np.path.block
        };
        let blocks = self.doc.story_mut(story);
        blocks.insert(insert_at, Block::Table(t));
        if insert_at + 1 >= blocks.len() {
            blocks.push(Block::Para(Paragraph::new()));
        }
        // An empty paragraph we split from stays after the table.
        let cell = ParaPath { story, block: insert_at, cell: Some(CellRef { row: 0, col: 0, para: 0 }) };
        self.sel = Selection::caret(cell.at(0));
        self.changed();
    }

    fn with_table(&mut self, label: &str, f: impl FnOnce(&mut Table, CellRef) -> Option<CellRef>) {
        let pos = self.sel.head;
        let Some(c) = pos.path.cell else { return };
        self.checkpoint(label, EditKind::Other);
        let story = pos.path.story;
        let mut remove = false;
        let mut newcell = None;
        if let Some(Block::Table(t)) = self.doc.story_mut(story).get_mut(pos.path.block) {
            newcell = f(t, c);
            remove = t.rows.is_empty() || t.columns.is_empty() || t.rows.iter().all(|r| r.cells.is_empty());
        }
        if remove {
            let blocks = self.doc.story_mut(story);
            blocks.remove(pos.path.block);
            self.doc.ensure_story(story);
            let b = pos.path.block.min(self.doc.story(story).len() - 1);
            let p = self.doc.clamp(ParaPath { story, block: b, cell: None }.at(0));
            self.sel = Selection::caret(p);
        } else if let Some(nc) = newcell {
            let p = self.doc.clamp(ParaPath { cell: Some(nc), ..pos.path }.at(0));
            self.sel = Selection::caret(p);
        } else {
            let p = self.doc.clamp(pos);
            self.sel = Selection::caret(p);
        }
        self.changed();
    }

    pub fn table_insert_row(&mut self, above: bool) {
        self.with_table("Insert Row", |t, c| {
            let src = t.rows[c.row].clone();
            let mut row = Row { cells: src.cells.iter().map(|x| Cell { paragraphs: vec![Paragraph { props: x.paragraphs.first().map(|p| p.props.clone()).unwrap_or_default(), ..Default::default() }], span: x.span, shading: x.shading, valign: x.valign, borders: x.borders.clone(), vmerge: VMerge::None, width: x.width }).collect(), height: src.height, header: false };
            if row.cells.is_empty() {
                row.cells.push(Cell::default());
            }
            let at = if above { c.row } else { c.row + 1 };
            t.rows.insert(at, row);
            Some(CellRef { row: at, col: c.col.min(t.rows[at].cells.len() - 1), para: 0 })
        });
    }

    pub fn table_insert_col(&mut self, left: bool) {
        self.with_table("Insert Column", |t, c| {
            let g = t.grid_col(c.row, c.col);
            let gi = if left { g } else { g + t.rows[c.row].cells[c.col].span.max(1) as usize };
            let w = t.columns.get(g).copied().unwrap_or(72.0);
            t.columns.insert(gi.min(t.columns.len()), w);
            // scale to keep table width
            let total: f32 = t.columns.iter().sum();
            let target = total - w;
            if target > 0.0 {
                for x in &mut t.columns {
                    *x *= target / total;
                }
            }
            for r in &mut t.rows {
                // find cell index at grid gi
                let mut acc = 0;
                let mut idx = r.cells.len();
                for (i, cell) in r.cells.iter().enumerate() {
                    if acc >= gi {
                        idx = i;
                        break;
                    }
                    acc += cell.span.max(1) as usize;
                }
                r.cells.insert(idx, Cell::default());
            }
            Some(CellRef { row: c.row, col: if left { c.col } else { c.col + 1 }, para: 0 })
        });
    }

    pub fn table_delete_row(&mut self) {
        self.with_table("Delete Row", |t, c| {
            t.rows.remove(c.row);
            if t.rows.is_empty() {
                return None;
            }
            let r = c.row.min(t.rows.len() - 1);
            Some(CellRef { row: r, col: c.col.min(t.rows[r].cells.len().saturating_sub(1)), para: 0 })
        });
    }

    pub fn table_delete_col(&mut self) {
        self.with_table("Delete Column", |t, c| {
            let g = t.grid_col(c.row, c.col);
            for r in &mut t.rows {
                let mut acc = 0;
                let mut idx = None;
                for (i, cell) in r.cells.iter().enumerate() {
                    if acc == g {
                        idx = Some(i);
                        break;
                    }
                    acc += cell.span.max(1) as usize;
                }
                if let Some(i) = idx {
                    if r.cells[i].span > 1 {
                        r.cells[i].span -= 1;
                    } else {
                        r.cells.remove(i);
                    }
                }
            }
            if g < t.columns.len() {
                t.columns.remove(g);
            }
            if t.columns.is_empty() {
                t.rows.clear();
                return None;
            }
            Some(CellRef { row: c.row, col: c.col.saturating_sub(1), para: 0 })
        });
    }

    pub fn table_delete(&mut self) {
        self.with_table("Delete Table", |t, _| {
            t.rows.clear();
            None
        });
    }

    pub fn table_merge_right(&mut self) {
        self.with_table("Merge Cells", |t, c| {
            let row = &mut t.rows[c.row];
            if c.col + 1 < row.cells.len() {
                let next = row.cells.remove(c.col + 1);
                let cell = &mut row.cells[c.col];
                cell.span += next.span.max(1);
                if next.paragraphs.iter().any(|p| !p.is_empty()) {
                    cell.paragraphs.extend(next.paragraphs);
                }
            }
            Some(c)
        });
    }

    pub fn table_split_cell(&mut self) {
        self.with_table("Split Cells", |t, c| {
            let row = &mut t.rows[c.row];
            let cell = &mut row.cells[c.col];
            if cell.span > 1 {
                cell.span -= 1;
                row.cells.insert(c.col + 1, Cell::default());
            } else {
                // split the grid column in two
                let g = t.grid_col(c.row, c.col);
                let w = t.columns[g] / 2.0;
                t.columns[g] = w;
                t.columns.insert(g + 1, w);
                for (ri, r) in t.rows.iter_mut().enumerate() {
                    let mut acc = 0;
                    for (i, cell) in r.cells.iter_mut().enumerate() {
                        let span = cell.span.max(1) as usize;
                        if acc <= g && g < acc + span {
                            if ri == c.row {
                                r.cells.insert(i + 1, Cell::default());
                            } else {
                                cell.span += 1;
                            }
                            break;
                        }
                        acc += span;
                    }
                }
            }
            Some(c)
        });
    }

    pub fn table_set_shading(&mut self, color: Option<Color>) {
        self.with_table("Shading", |t, c| {
            t.rows[c.row].cells[c.col].shading = color;
            Some(c)
        });
    }

    pub fn table_toggle_header(&mut self) {
        self.with_table("Header Row", |t, c| {
            let on = !t.rows[0].header;
            t.rows[0].header = on;
            Some(c)
        });
    }

    pub fn table_style(&mut self, header: Option<Color>, band: Option<Color>, borders: Borders) {
        self.with_table("Table Style", |t, c| {
            t.header_shading = header;
            t.band_shading = band;
            t.borders = borders;
            if header.is_some() {
                for cell in &mut t.rows[0].cells {
                    for p in &mut cell.paragraphs {
                        for r in &mut p.runs {
                            r.props.bold = Some(true);
                            r.props.color = Some(Color::WHITE);
                        }
                        p.mark.bold = Some(true);
                        p.mark.color = Some(Color::WHITE);
                    }
                }
            }
            Some(c)
        });
    }

    pub fn table_distribute_columns(&mut self) {
        self.with_table("Distribute Columns", |t, c| {
            let total: f32 = t.columns.iter().sum();
            let n = t.columns.len().max(1) as f32;
            for w in &mut t.columns {
                *w = total / n;
            }
            Some(c)
        });
    }

    pub fn insert_image(&mut self, data: ImageData, label: &str) {
        let (w, h) = data.pixel_size.map(|(w, h)| (w as f32 * 0.75, h as f32 * 0.75)).unwrap_or((200.0, 150.0));
        let maxw = self.doc.page.column_width();
        let s = if w > maxw { maxw / w } else { 1.0 };
        let img = Image { data: Arc::new(data), width: w * s, height: h * s, alt: String::new() };
        self.insert_inline(Inline::Image(img), label);
    }

    pub fn insert_link(&mut self, text: &str, url: &str) {
        self.checkpoint("Insert Hyperlink", EditKind::Other);
        let url = url.trim().to_string();
        let url = if !url.contains(':') && !url.starts_with('#') && url.contains('.') { format!("https://{}", url) } else { url };
        if !self.sel.is_empty() && text.is_empty() {
            let (a, b) = self.sel.range();
            let u = url.clone();
            self.doc.apply_chars(a, b, &move |c| {
                c.link = Some(u.clone());
                c.style = Some("Hyperlink".into());
            });
        } else {
            self.delete_selection();
            let mut props = self.typing_props();
            props.link = Some(url.clone());
            props.style = Some("Hyperlink".into());
            let t = if text.is_empty() { url.clone() } else { text.to_string() };
            let p = self.doc.insert_text(self.sel.head, &t, &props);
            self.sel = Selection::caret(p);
        }
        self.changed();
    }

    pub fn remove_link(&mut self) {
        let pos = self.sel.head;
        let (a, b) = if self.sel.is_empty() { self.link_range(pos).unwrap_or((pos, pos)) } else { self.sel.range() };
        self.checkpoint("Remove Hyperlink", EditKind::Other);
        self.doc.apply_chars(a, b, &|c| {
            c.link = None;
            if c.style.as_deref() == Some("Hyperlink") {
                c.style = None;
            }
        });
        self.changed();
    }

    /// Range of the hyperlink at `pos`.
    pub fn link_range(&self, pos: Pos) -> Option<(Pos, Pos)> {
        let p = self.doc.para(&pos.path)?;
        let url = p.props_at(pos.offset + 1).link.or_else(|| p.props_at(pos.offset).link)?;
        let mut acc = 0;
        let mut start = None;
        let mut end = 0;
        for r in &p.runs {
            let l = r.len();
            if r.props.link.as_ref() == Some(&url) {
                if start.is_none() {
                    start = Some(acc);
                }
                end = acc + l;
            } else if start.is_some() && acc > pos.offset {
                break;
            } else if acc + l <= pos.offset {
                start = None;
            }
            acc += l;
        }
        start.map(|s| (pos.path.at(s), pos.path.at(end)))
    }

    pub fn link_at(&self, pos: Pos) -> Option<String> {
        let p = self.doc.para(&pos.path)?;
        let (ri, _) = p.run_at(pos.offset)?;
        p.runs[ri].props.link.clone()
    }

    pub fn insert_footnote(&mut self, endnote: bool) {
        if self.story() != Story::Body {
            return;
        }
        let text = self.ask_text_default(if endnote { "Endnote text" } else { "Footnote text" });
        self.checkpoint(if endnote { "Insert Endnote" } else { "Insert Footnote" }, EditKind::Other);
        let id = self.doc.next_note_id();
        let note = Note { id, paragraphs: vec![{
            let mut p = Paragraph::with_style(if endnote { "EndnoteText" } else { "FootnoteText" }, "");
            p.runs.push(Run { content: if endnote { Inline::EndnoteRef(id) } else { Inline::FootnoteRef(id) }, props: CharProps { style: Some(if endnote { "EndnoteReference" } else { "FootnoteReference" }.into()), ..Default::default() } });
            p.runs.push(Run::text(format!(" {}", text)));
            p
        }] };
        if endnote {
            self.doc.endnotes.push(note);
        } else {
            self.doc.footnotes.push(note);
        }
        self.sel.anchor = self.sel.range().1;
        self.sel.head = self.sel.anchor;
        let props = CharProps { style: Some(if endnote { "EndnoteReference" } else { "FootnoteReference" }.into()), ..Default::default() };
        let p = self.doc.insert_inline(self.sel.head, if endnote { Inline::EndnoteRef(id) } else { Inline::FootnoteRef(id) }, &props);
        self.sel = Selection::caret(p);
        self.changed();
        // open editor for the note text
        self.dialog = Some(Dialog::EditNote { id, endnote, text: String::new() });
    }

    fn ask_text_default(&self, _what: &str) -> String {
        String::new()
    }

    pub fn insert_comment(&mut self, text: &str) {
        if self.sel.is_empty() {
            let (a, b) = self.doc.word_at(self.sel.head);
            self.sel = Selection { anchor: a, head: b };
        }
        if self.sel.is_empty() {
            self.status = "Select some text to comment on.".into();
            return;
        }
        self.checkpoint("New Comment", EditKind::Other);
        let id = self.doc.next_comment_id();
        let author = self.doc.settings.author.clone();
        let initials: String = author.split_whitespace().filter_map(|w| w.chars().next()).collect::<String>().to_uppercase();
        self.doc.comments.push(Comment {
            id,
            author,
            initials,
            date: wordcore::util::now_iso(),
            paragraphs: text.split('\n').map(|l| Paragraph::with_style("CommentText", l)).collect(),
            resolved: false,
        });
        let (a, b) = self.sel.range();
        self.doc.apply_chars(a, b, &move |c| {
            if !c.comments.contains(&id) {
                c.comments.push(id);
            }
        });
        self.side_panel = Some(SidePanel::Comments);
        self.changed();
    }

    pub fn delete_comment(&mut self, id: u32) {
        self.checkpoint("Delete Comment", EditKind::Other);
        self.doc.comments.retain(|c| c.id != id);
        for s in [Story::Body, Story::Header, Story::Footer, Story::FirstHeader, Story::FirstFooter] {
            for path in self.doc.para_paths(s) {
                if let Some(p) = self.doc.para_mut(&path) {
                    for r in &mut p.runs {
                        r.props.comments.retain(|c| *c != id);
                    }
                    p.normalize();
                }
            }
        }
        self.changed();
    }

    /// Insert or update the table of contents at the caret (or update the existing one).
    pub fn insert_toc(&mut self) {
        self.ensure_layout();
        let headings: Vec<_> = self.layout.headings.iter().filter(|h| h.level < 3).cloned().collect();
        // Existing TOC: contiguous paragraphs with TOC styles (and an optional TOC Heading before).
        let blocks = &self.doc.body;
        let is_toc = |b: &Block| b.as_para().map(|p| p.style_id().starts_with("TOC")).unwrap_or(false);
        let existing = blocks.iter().position(is_toc);
        self.checkpoint("Table of Contents", EditKind::Other);
        let mut paras = vec![Block::Para(Paragraph::with_style("TOCHeading", "Contents"))];
        let tw = self.doc.page.text_width();
        for h in &headings {
            let page = self.layout.pages.get(h.page).map(|p| p.number).unwrap_or(1);
            let mut p = Paragraph::with_style(&format!("TOC{}", h.level + 1), &h.text);
            p.props.tabs = vec![TabStop { pos: tw - 0.5, align: TabAlign::Right, leader: TabLeader::Dot }];
            p.runs.push(Run { content: Inline::Tab, props: CharProps::default() });
            p.runs.push(Run::text(page.to_string()));
            // link each entry to its heading via a bookmark-style link
            for r in &mut p.runs {
                r.props.link = Some(format!("#_Toc{}", h.path.block));
            }
            paras.push(Block::Para(p));
        }
        if headings.is_empty() {
            paras.push(Block::Para(Paragraph::with_style("TOC1", "No table of contents entries found. Apply Heading styles to your titles first.")));
        }
        let n = paras.len();
        match existing {
            Some(start) => {
                let mut s = start;
                if s > 0 && self.doc.body[s - 1].as_para().map(|p| p.style_id() == "TOCHeading").unwrap_or(false) {
                    s -= 1;
                }
                let mut e = start;
                while e < self.doc.body.len() && is_toc(&self.doc.body[e]) {
                    e += 1;
                }
                self.doc.body.splice(s..e, paras);
                self.sel = Selection::caret(Pos::new(s, 0));
            }
            None => {
                let pos = self.sel.head;
                let at = if pos.path.story == Story::Body && pos.path.cell.is_none() { pos.path.block } else { 0 };
                let at = if self.doc.para(&pos.path).map(|p| p.is_empty()).unwrap_or(false) { at } else { at };
                self.doc.body.splice(at..at, paras);
                self.sel = Selection::caret(Pos::new(at + n, 0));
            }
        }
        // Page numbers may have shifted: relayout and refresh numbers once.
        self.relayout();
        let fresh: Vec<_> = self.layout.headings.iter().filter(|h| h.level < 3).cloned().collect();
        let mut k = 0;
        for b in self.doc.body.iter_mut() {
            if let Block::Para(p) = b {
                if p.style_id().starts_with("TOC") && p.style_id() != "TOCHeading" {
                    if let Some(h) = fresh.get(k) {
                        let page = self.layout.pages.get(h.page).map(|p| p.number).unwrap_or(1);
                        if let Some(Run { content: Inline::Text(t), .. }) = p.runs.last_mut() {
                            *t = page.to_string();
                        }
                        for r in &mut p.runs {
                            r.props.link = Some(format!("#_Toc{}", h.path.block));
                        }
                    }
                    k += 1;
                }
            }
        }
        self.changed();
    }

    pub fn insert_caption(&mut self, label: &str) {
        // Count existing captions with the label.
        let n = self.doc.body.iter().filter(|b| b.as_para().map(|p| p.style_id() == "Caption" && p.text().starts_with(label)).unwrap_or(false)).count() + 1;
        self.checkpoint("Insert Caption", EditKind::Other);
        let pos = self.sel.head;
        let at = if pos.path.cell.is_none() && pos.path.story == Story::Body { pos.path.block + 1 } else { pos.path.block + 1 };
        let at = at.min(self.doc.body.len());
        let p = Paragraph::with_style("Caption", &format!("{} {}: ", label, n));
        let len = p.len();
        self.doc.body.insert(at, Block::Para(p));
        self.sel = Selection::caret(Pos::new(at, len));
        self.changed();
    }

    // -------------------------------------------------------------------
    // Headers / footers
    // -------------------------------------------------------------------

    pub fn edit_header(&mut self, footer: bool, first_page: bool) {
        let story = match (footer, first_page && self.doc.page.title_page) {
            (false, false) => Story::Header,
            (true, false) => Story::Footer,
            (false, true) => Story::FirstHeader,
            (true, true) => Story::FirstFooter,
        };
        if self.doc.story(story).is_empty() {
            self.checkpoint("Header", EditKind::Other);
            let style = if footer { "Footer" } else { "Header" };
            self.doc.story_mut(story).push(Block::Para(Paragraph::with_style(style, "")));
            self.changed();
        }
        let end = self.doc.end_of(story);
        self.set_caret(end, false);
    }

    pub fn close_header(&mut self) {
        if self.story() != Story::Body {
            let p = self.doc.start_of(Story::Body);
            self.set_caret(p, false);
            self.layout_dirty = true;
        }
    }

    /// Insert page numbers into the header or footer (`pos`: 0 left, 1 centre, 2 right).
    pub fn insert_page_numbers(&mut self, footer: bool, align: Align, with_total: bool) {
        self.checkpoint("Page Number", EditKind::Other);
        let story = if footer { Story::Footer } else { Story::Header };
        let mut p = Paragraph::with_style(if footer { "Footer" } else { "Header" }, "");
        p.props.align = Some(align);
        if with_total {
            p.runs.push(Run::text("Page "));
        }
        p.runs.push(Run { content: Inline::Field(Field::PageNumber), props: CharProps::default() });
        if with_total {
            p.runs.push(Run::text(" of "));
            p.runs.push(Run { content: Inline::Field(Field::PageCount), props: CharProps::default() });
        }
        let blocks = self.doc.story_mut(story);
        // Replace an empty header/footer, else add a paragraph.
        if blocks.len() == 1 && blocks[0].as_para().map(|x| x.is_empty()).unwrap_or(false) {
            blocks[0] = Block::Para(p);
        } else {
            blocks.push(Block::Para(p));
        }
        if self.doc.page.title_page {
            let fs = if footer { Story::FirstFooter } else { Story::FirstHeader };
            let copy = self.doc.story(story).clone();
            *self.doc.story_mut(fs) = copy;
        }
        self.changed();
    }

    // -------------------------------------------------------------------
    // Review
    // -------------------------------------------------------------------

    pub fn toggle_track_changes(&mut self) {
        self.doc.settings.track_changes = !self.doc.settings.track_changes;
        self.modified = true;
    }

    pub fn accept_reject(&mut self, accept: bool, all: bool) {
        let story = self.story();
        self.checkpoint(if accept { "Accept Change" } else { "Reject Change" }, EditKind::Other);
        let n = if all {
            self.doc.resolve_revisions(story, None, accept)
        } else if !self.sel.is_empty() {
            let r = self.sel.range();
            self.doc.resolve_revisions(story, Some(r), accept)
        } else {
            // change at/after the caret
            let revs = self.doc.revisions(story);
            match revs.into_iter().find(|(a, b, _)| *b >= self.sel.head && *a <= self.sel.head).or_else(|| self.doc.revisions(story).into_iter().find(|(a, _, _)| *a >= self.sel.head)) {
                Some((a, b, _)) => self.doc.resolve_revisions(story, Some((a, b)), accept),
                None => 0,
            }
        };
        self.sel.anchor = self.doc.clamp(self.sel.anchor);
        self.sel.head = self.doc.clamp(self.sel.head);
        self.status = if n == 0 { "No tracked changes found.".into() } else { format!("{} change{} {}.", n, if n == 1 { "" } else { "s" }, if accept { "accepted" } else { "rejected" }) };
        self.changed();
        if !all {
            self.next_change(true);
        }
    }

    pub fn next_change(&mut self, forward: bool) {
        let revs = self.doc.revisions(self.story());
        let cur = self.sel.range();
        let hit = if forward {
            revs.into_iter().find(|(a, _, _)| *a > cur.0 || (*a == cur.0 && cur.0 == cur.1))
        } else {
            revs.into_iter().rev().find(|(a, _, _)| *a < cur.0)
        };
        if let Some((a, b, _)) = hit {
            self.select(a, b);
        }
    }

    // -------------------------------------------------------------------
    // Find / replace
    // -------------------------------------------------------------------

    pub fn find_update(&mut self) {
        self.find.matches = self.doc.find_all(self.story(), &self.find.query, self.find.opts);
        let caret = self.sel.range().0;
        self.find.current = self.find.matches.iter().position(|(a, _)| *a >= caret).unwrap_or(0);
    }

    pub fn find_next(&mut self, forward: bool) -> bool {
        self.find_update();
        if self.find.matches.is_empty() {
            self.status = format!("\"{}\" was not found.", self.find.query);
            return false;
        }
        let (a0, b0) = self.sel.range();
        let n = self.find.matches.len();
        let idx = if forward {
            self.find.matches.iter().position(|(a, _)| *a >= b0 && !(*a == a0 && a0 != b0)).unwrap_or(0)
        } else {
            self.find.matches.iter().rposition(|(a, _)| *a < a0).unwrap_or(n - 1)
        };
        self.find.current = idx;
        let (a, b) = self.find.matches[idx];
        self.select(a, b);
        self.status = format!("Result {} of {}", idx + 1, n);
        true
    }

    pub fn replace_one(&mut self) {
        let q = self.find.query.clone();
        let sel_text = self.selected_text();
        let matches = if self.find.opts.match_case { sel_text == q } else { sel_text.to_lowercase() == q.to_lowercase() };
        if matches && !q.is_empty() {
            self.checkpoint("Replace", EditKind::Other);
            let (a, _) = self.sel.range();
            let props = self.doc.para(&a.path).map(|p| p.props_at(a.offset + 1)).unwrap_or_default();
            self.delete_selection();
            let p = self.doc.insert_text(self.sel.head, &self.find.replace.clone(), &props);
            self.sel = Selection::caret(p);
            self.changed();
        }
        self.find_next(true);
    }

    pub fn replace_all(&mut self) -> usize {
        self.checkpoint("Replace All", EditKind::Other);
        let n = self.doc.replace_all(self.story(), &self.find.query.clone(), &self.find.replace.clone(), self.find.opts);
        self.sel.anchor = self.doc.clamp(self.sel.anchor);
        self.sel.head = self.doc.clamp(self.sel.head);
        self.status = format!("All done. We made {} replacement{}.", n, if n == 1 { "" } else { "s" });
        if n > 0 {
            self.changed();
        } else {
            self.undo.pop();
        }
        n
    }

    // -------------------------------------------------------------------
    // Files
    // -------------------------------------------------------------------

    pub fn open_path(&mut self, path: PathBuf) {
        match wordcore::io::open(&path) {
            Ok(mut d) => {
                if d.settings.author.is_empty() || d.settings.author == "Author" {
                    d.settings.author = crate::fileio::user_name();
                }
                self.doc = d;
                self.doc.ensure_story(Story::Body);
                self.sel = Selection::caret(self.doc.start_of(Story::Body));
                self.undo.clear();
                self.redo.clear();
                self.modified = false;
                self.path = Some(path.clone());
                self.layout_dirty = true;
                crate::fileio::add_recent(&mut self.recent, &path);
                self.status = format!("Opened {}", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            }
            Err(e) => {
                self.dialog = Some(Dialog::Message { title: "Couldn't open file".into(), text: format!("RWord couldn't open {}.\n\n{}", path.display(), e) });
            }
        }
    }

    pub fn new_document(&mut self) {
        let author = self.doc.settings.author.clone();
        self.doc = Document::new();
        self.doc.settings.author = author;
        self.sel = Selection::caret(Pos::default());
        self.undo.clear();
        self.redo.clear();
        self.modified = false;
        self.path = None;
        self.layout_dirty = true;
    }

    pub fn save(&mut self) -> bool {
        match self.path.clone() {
            Some(p) if wordcore::io::Format::from_path(&p).is_some() => self.save_to(p),
            _ => self.save_as(),
        }
    }

    pub fn save_as(&mut self) -> bool {
        match crate::fileio::save_dialog(self.path.as_deref()) {
            Some(p) => self.save_to(p),
            None => false,
        }
    }

    pub fn save_to(&mut self, path: PathBuf) -> bool {
        if path.extension().map(|e| e.eq_ignore_ascii_case("pdf")).unwrap_or(false) {
            return self.export_pdf_to(path);
        }
        self.doc.properties.modified = wordcore::util::now_iso();
        if self.doc.properties.created.is_empty() {
            self.doc.properties.created = self.doc.properties.modified.clone();
        }
        if self.doc.properties.author.is_empty() {
            self.doc.properties.author = self.doc.settings.author.clone();
        }
        self.doc.properties.last_modified_by = self.doc.settings.author.clone();
        match wordcore::io::save(&self.doc, &path) {
            Ok(()) => {
                self.modified = false;
                self.path = Some(path.clone());
                crate::fileio::add_recent(&mut self.recent, &path);
                self.status = format!("Saved {}", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
                true
            }
            Err(e) => {
                self.dialog = Some(Dialog::Message { title: "Couldn't save".into(), text: format!("RWord couldn't save {}.\n\n{}", path.display(), e) });
                false
            }
        }
    }

    pub fn export_pdf(&mut self) {
        let default = self.path.as_ref().map(|p| p.with_extension("pdf"));
        if let Some(p) = crate::fileio::pdf_dialog(default.as_deref()) {
            self.export_pdf_to(p);
        }
    }

    pub fn export_pdf_to(&mut self, path: PathBuf) -> bool {
        self.ensure_layout();
        let bytes = wordcore::pdf::export(&self.layout, self.db, &self.doc.properties);
        match std::fs::write(&path, bytes) {
            Ok(()) => {
                self.status = format!("Exported {}", path.display());
                true
            }
            Err(e) => {
                self.dialog = Some(Dialog::Message { title: "Couldn't export".into(), text: format!("{}", e) });
                false
            }
        }
    }

    /// Print: export a PDF to a temporary file and open it in the system viewer, which prints it.
    pub fn print(&mut self) {
        self.ensure_layout();
        let bytes = wordcore::pdf::export(&self.layout, self.db, &self.doc.properties);
        let name = self.path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Document1".into());
        let tmp = std::env::temp_dir().join(format!("{} (print).pdf", name));
        match std::fs::write(&tmp, bytes) {
            Ok(()) => {
                crate::fileio::print_pdf(&tmp);
                self.status = "Sent to your PDF viewer for printing.".into();
            }
            Err(e) => self.dialog = Some(Dialog::Message { title: "Print".into(), text: format!("Couldn't prepare the document for printing: {}", e) }),
        }
    }

    /// Run a request, asking to save first if needed.
    pub fn request(&mut self, p: Pending) {
        if self.modified {
            self.pending = Some(p);
            self.dialog = Some(Dialog::SaveChanges);
        } else {
            self.run_pending(p);
        }
    }

    pub fn run_pending(&mut self, p: Pending) {
        match p {
            Pending::New => self.new_document(),
            Pending::Open(path) => {
                let path = path.or_else(crate::fileio::open_dialog);
                if let Some(path) = path {
                    self.open_path(path);
                }
            }
            Pending::Close => {
                self.allow_close = true;
            }
        }
    }

    pub fn word_count(&self) -> usize {
        self.doc.para_paths(Story::Body).iter().filter_map(|p| self.doc.para(p)).map(|p| p.text().split_whitespace().count()).sum()
    }

    pub fn selection_word_count(&self) -> usize {
        if self.sel.is_empty() {
            return 0;
        }
        self.selected_text().split_whitespace().count()
    }

    // used by checkpoint/changed
    #[allow(dead_code)]
    fn _unused(&self) {}
}

fn normalize_nl(s: &str) -> String {
    s.replace("\r\n", "\n").trim_end_matches('\n').to_string()
}

/// The list a paragraph belongs to (direct or via style).
pub fn resolved_list(doc: &Document, p: &Paragraph) -> Option<ListRef> {
    wordcore::styles::resolve_para(doc, &p.props).list
}

/// Fields of `cur` that `probe` touched (set to Some) — used for pending typing formats.
fn cur_diff(probe: &CharProps, cur: &CharProps) -> CharProps {
    let mut out = CharProps::default();
    macro_rules! d {
        ($($f:ident),*) => { $( if probe.$f.is_some() { out.$f = cur.$f.clone(); } )* };
    }
    d!(style, font, size, bold, italic, underline, strike, double_strike, color, highlight, shading, vert_align, caps, small_caps, hidden, spacing, link);
    if probe.highlight.is_none() && cur.highlight.is_none() {
        // highlight removal
    }
    out
}

impl App {
    pub fn set_page_setup(&mut self, s: PageSetup) {
        self.checkpoint("Page Setup", EditKind::Other);
        self.doc.page = s;
        self.changed();
    }

    /// Apply a document theme: heading font, body font and accent colour.
    pub fn apply_theme(&mut self, heading_font: &str, body_font: &str, color: Color) {
        self.checkpoint("Theme", EditKind::Other);
        for st in &mut self.doc.styles {
            let id = st.id.as_str();
            let heading = id.starts_with("Heading") || id == "Title" || id == "TOCHeading";
            if heading {
                st.chars.font = Some(heading_font.to_string());
                if id != "Title" {
                    st.chars.color = Some(color);
                }
            } else if id == "Normal" {
                st.chars.font = Some(body_font.to_string());
            }
        }
        self.doc.default_chars.font = Some(body_font.to_string());
        self.changed();
    }

    /// Insert an empty paragraph with a bottom border (Word's "Horizontal Line").
    pub fn insert_horizontal_line(&mut self) {
        self.checkpoint("Horizontal Line", EditKind::Other);
        self.delete_selection();
        let mut at = self.sel.head;
        if at.offset > 0 {
            at = self.doc.split_paragraph(at);
        }
        let b = Border { style: BorderStyle::Single, width: 1.5, color: Color::rgb(0xA0, 0xA0, 0xA0), space: 1.0 };
        self.doc.apply_para(at, at, &|p| p.borders = Some(Borders { bottom: Some(b), ..Default::default() }));
        let next = self.doc.split_paragraph(at);
        self.doc.apply_para(next, next, &|p| p.borders = Some(Borders::default()));
        self.sel = Selection::caret(next);
        self.changed();
    }

    pub fn table_style_borders(&mut self, b: Borders) {
        self.with_table("Borders", |t, c| {
            t.borders = b;
            for r in &mut t.rows {
                for cell in &mut r.cells {
                    cell.borders = None;
                }
            }
            Some(c)
        });
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.screenshot.as_ref().map(|s| s.done.load(Ordering::Relaxed));
    }
}
