//! Positions and editing operations on a [`Document`].

use crate::model::*;

/// A paragraph inside a table cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellRef {
    pub row: usize,
    pub col: usize,
    pub para: usize,
}

/// Address of one paragraph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ParaPath {
    pub story: Story,
    /// Index into the story's blocks.
    pub block: usize,
    /// Set when the block is a table.
    pub cell: Option<CellRef>,
}

impl ParaPath {
    pub fn body(block: usize) -> Self {
        ParaPath { story: Story::Body, block, cell: None }
    }
    pub fn at(self, offset: usize) -> Pos {
        Pos { path: self, offset }
    }
    /// Same table cell (or same top-level paragraph) as `other`.
    pub fn same_cell(&self, other: &ParaPath) -> bool {
        self.story == other.story
            && self.block == other.block
            && match (self.cell, other.cell) {
                (Some(a), Some(b)) => a.row == b.row && a.col == b.col,
                (None, None) => true,
                _ => false,
            }
    }
}

/// A caret position: a paragraph and a char offset within it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Pos {
    pub path: ParaPath,
    pub offset: usize,
}

impl Pos {
    pub fn new(block: usize, offset: usize) -> Self {
        Pos { path: ParaPath::body(block), offset }
    }
}

/// Order two positions.
pub fn ordered(a: Pos, b: Pos) -> (Pos, Pos) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Options for find / replace.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub match_case: bool,
    pub whole_word: bool,
}

pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\'' || c == '’'
}

// ---------------------------------------------------------------------------
// Run-level helpers on a paragraph
// ---------------------------------------------------------------------------

impl Paragraph {
    /// Ensure a run boundary at `off`; returns the index of the first run at or after `off`.
    pub fn split_at(&mut self, off: usize) -> usize {
        let mut acc = 0;
        for i in 0..self.runs.len() {
            let len = self.runs[i].len();
            if off == acc {
                return i;
            }
            if off < acc + len {
                // inside run i (must be text since others have len 1)
                if let Inline::Text(s) = &self.runs[i].content {
                    let k = off - acc;
                    let byte = s.char_indices().nth(k).map(|(b, _)| b).unwrap_or(s.len());
                    let tail = s[byte..].to_string();
                    let head = s[..byte].to_string();
                    let props = self.runs[i].props.clone();
                    self.runs[i].content = Inline::Text(head);
                    self.runs.insert(i + 1, Run { content: Inline::Text(tail), props });
                    return i + 1;
                }
                return i;
            }
            acc += len;
        }
        self.runs.len()
    }

    /// Merge adjacent text runs with equal formatting and drop empty text runs.
    pub fn normalize(&mut self) {
        let mut out: Vec<Run> = Vec::with_capacity(self.runs.len());
        for r in self.runs.drain(..) {
            if let Inline::Text(s) = &r.content {
                if s.is_empty() {
                    continue;
                }
                if let Some(last) = out.last_mut() {
                    if last.props == r.props {
                        if let Inline::Text(ls) = &mut last.content {
                            ls.push_str(s);
                            continue;
                        }
                    }
                }
            }
            out.push(r);
        }
        self.runs = out;
    }

    /// Index of the run containing char `off` and the offset inside it (for `off == len` returns the end).
    pub fn run_at(&self, off: usize) -> Option<(usize, usize)> {
        let mut acc = 0;
        for (i, r) in self.runs.iter().enumerate() {
            let len = r.len();
            if off < acc + len {
                return Some((i, off - acc));
            }
            acc += len;
        }
        None
    }

    /// Formatting that typing at `off` would use: the run before the caret, else the run after, else the mark.
    pub fn props_at(&self, off: usize) -> CharProps {
        let mut acc = 0;
        let mut before: Option<&Run> = None;
        for r in &self.runs {
            let len = r.len();
            if off > acc && off <= acc + len {
                before = Some(r);
                break;
            }
            acc += len;
        }
        let mut p = match before {
            Some(r) => r.props.clone(),
            None => self.runs.first().map(|r| r.props.clone()).unwrap_or_else(|| self.mark.clone()),
        };
        // Don't carry over things that should not extend by typing.
        p.revision = None;
        p.comments.clear();
        p
    }

    /// Char `i` as text (non-text inlines map to their plain placeholder).
    pub fn char_at(&self, i: usize) -> Option<char> {
        let (ri, k) = self.run_at(i)?;
        match &self.runs[ri].content {
            Inline::Text(s) => s.chars().nth(k),
            other => other.plain().chars().next(),
        }
    }

    pub fn chars_vec(&self) -> Vec<char> {
        self.plain_text().chars().collect()
    }

    /// Remove chars `[a, b)`.
    pub fn remove_range(&mut self, a: usize, b: usize) {
        if a >= b {
            return;
        }
        self.split_at(b);
        let i = self.split_at(a);
        let j = self.index_at(b);
        self.runs.drain(i..j);
        self.normalize();
    }

    /// Index of the first run starting at or after `off` assuming a boundary exists there.
    fn index_at(&self, off: usize) -> usize {
        let mut acc = 0;
        for (i, r) in self.runs.iter().enumerate() {
            if acc >= off {
                return i;
            }
            acc += r.len();
        }
        self.runs.len()
    }

    /// Insert runs at `off`.
    pub fn insert_runs(&mut self, off: usize, runs: Vec<Run>) {
        let i = self.split_at(off);
        self.runs.splice(i..i, runs);
        self.normalize();
    }

    /// Copy of runs covering `[a, b)`.
    pub fn slice_runs(&self, a: usize, b: usize) -> Vec<Run> {
        let mut p = self.clone();
        p.remove_range(b, p.len());
        p.remove_range(0, a);
        p.runs
    }

    /// Split into `[0, off)` (self) and `[off, len)` (returned).
    pub fn split_off(&mut self, off: usize) -> Paragraph {
        let i = self.split_at(off);
        let tail = self.runs.split_off(i);
        let mut np = Paragraph { runs: tail, props: self.props.clone(), mark: self.mark.clone() };
        np.normalize();
        self.normalize();
        np
    }
}

/// Convert text (possibly with '\t') to runs with the given props.
pub fn text_runs(text: &str, props: &CharProps) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        match c {
            '\t' | '\u{000B}' | '\u{000C}' => {
                if !cur.is_empty() {
                    runs.push(Run { content: Inline::Text(std::mem::take(&mut cur)), props: props.clone() });
                }
                let content = match c {
                    '\t' => Inline::Tab,
                    '\u{000B}' => Inline::LineBreak,
                    _ => Inline::PageBreak,
                };
                runs.push(Run { content, props: props.clone() });
            }
            '\r' => {}
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        runs.push(Run { content: Inline::Text(cur), props: props.clone() });
    }
    runs
}

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

impl Document {
    pub fn para(&self, p: &ParaPath) -> Option<&Paragraph> {
        let b = self.story(p.story).get(p.block)?;
        match (b, p.cell) {
            (Block::Para(para), None) => Some(para),
            (Block::Table(t), Some(c)) => t.rows.get(c.row)?.cells.get(c.col)?.paragraphs.get(c.para),
            _ => None,
        }
    }

    pub fn para_mut(&mut self, p: &ParaPath) -> Option<&mut Paragraph> {
        let b = self.story_mut(p.story).get_mut(p.block)?;
        match (b, p.cell) {
            (Block::Para(para), None) => Some(para),
            (Block::Table(t), Some(c)) => t.rows.get_mut(c.row)?.cells.get_mut(c.col)?.paragraphs.get_mut(c.para),
            _ => None,
        }
    }

    /// Paragraph list of the cell (or None for body paragraphs).
    pub fn cell_paras_mut(&mut self, p: &ParaPath) -> Option<&mut Vec<Paragraph>> {
        let c = p.cell?;
        match self.story_mut(p.story).get_mut(p.block)? {
            Block::Table(t) => Some(&mut t.rows.get_mut(c.row)?.cells.get_mut(c.col)?.paragraphs),
            _ => None,
        }
    }

    /// All paragraph paths of a story in document order.
    pub fn para_paths(&self, story: Story) -> Vec<ParaPath> {
        let mut out = Vec::new();
        for (bi, b) in self.story(story).iter().enumerate() {
            match b {
                Block::Para(_) => out.push(ParaPath { story, block: bi, cell: None }),
                Block::Table(t) => {
                    for (ri, r) in t.rows.iter().enumerate() {
                        for (ci, c) in r.cells.iter().enumerate() {
                            if c.vmerge == VMerge::Continue {
                                continue;
                            }
                            for pi in 0..c.paragraphs.len() {
                                out.push(ParaPath { story, block: bi, cell: Some(CellRef { row: ri, col: ci, para: pi }) });
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// First paragraph path inside block `bi` of `story`.
    fn first_path_in_block(&self, story: Story, bi: usize) -> Option<ParaPath> {
        match self.story(story).get(bi)? {
            Block::Para(_) => Some(ParaPath { story, block: bi, cell: None }),
            Block::Table(t) => {
                for (ri, r) in t.rows.iter().enumerate() {
                    for (ci, c) in r.cells.iter().enumerate() {
                        if c.vmerge != VMerge::Continue && !c.paragraphs.is_empty() {
                            return Some(ParaPath { story, block: bi, cell: Some(CellRef { row: ri, col: ci, para: 0 }) });
                        }
                    }
                }
                None
            }
        }
    }

    fn last_path_in_block(&self, story: Story, bi: usize) -> Option<ParaPath> {
        match self.story(story).get(bi)? {
            Block::Para(_) => Some(ParaPath { story, block: bi, cell: None }),
            Block::Table(t) => {
                for (ri, r) in t.rows.iter().enumerate().rev() {
                    for (ci, c) in r.cells.iter().enumerate().rev() {
                        if c.vmerge != VMerge::Continue && !c.paragraphs.is_empty() {
                            return Some(ParaPath {
                                story,
                                block: bi,
                                cell: Some(CellRef { row: ri, col: ci, para: c.paragraphs.len() - 1 }),
                            });
                        }
                    }
                }
                None
            }
        }
    }

    /// The paragraph after `p` in document order.
    pub fn next_path(&self, p: &ParaPath) -> Option<ParaPath> {
        if let Some(c) = p.cell {
            if let Some(Block::Table(t)) = self.story(p.story).get(p.block) {
                let cell = &t.rows[c.row].cells[c.col];
                if c.para + 1 < cell.paragraphs.len() {
                    return Some(ParaPath { cell: Some(CellRef { para: c.para + 1, ..c }), ..*p });
                }
                // next cell in row, then next rows
                let (mut r, mut col) = (c.row, c.col + 1);
                while r < t.rows.len() {
                    while col < t.rows[r].cells.len() {
                        let cc = &t.rows[r].cells[col];
                        if cc.vmerge != VMerge::Continue && !cc.paragraphs.is_empty() {
                            return Some(ParaPath { cell: Some(CellRef { row: r, col, para: 0 }), ..*p });
                        }
                        col += 1;
                    }
                    r += 1;
                    col = 0;
                }
            }
        }
        let n = self.story(p.story).len();
        let mut bi = p.block + 1;
        while bi < n {
            if let Some(x) = self.first_path_in_block(p.story, bi) {
                return Some(x);
            }
            bi += 1;
        }
        None
    }

    /// The paragraph before `p` in document order.
    pub fn prev_path(&self, p: &ParaPath) -> Option<ParaPath> {
        if let Some(c) = p.cell {
            if c.para > 0 {
                return Some(ParaPath { cell: Some(CellRef { para: c.para - 1, ..c }), ..*p });
            }
            if let Some(Block::Table(t)) = self.story(p.story).get(p.block) {
                let (mut r, mut col) = (c.row as isize, c.col as isize - 1);
                while r >= 0 {
                    while col >= 0 {
                        let cc = &t.rows[r as usize].cells[col as usize];
                        if cc.vmerge != VMerge::Continue && !cc.paragraphs.is_empty() {
                            return Some(ParaPath {
                                cell: Some(CellRef { row: r as usize, col: col as usize, para: cc.paragraphs.len() - 1 }),
                                ..*p
                            });
                        }
                        col -= 1;
                    }
                    r -= 1;
                    if r >= 0 {
                        col = t.rows[r as usize].cells.len() as isize - 1;
                    }
                }
            }
        }
        let mut bi = p.block;
        while bi > 0 {
            bi -= 1;
            if let Some(x) = self.last_path_in_block(p.story, bi) {
                return Some(x);
            }
        }
        None
    }

    pub fn start_of(&self, story: Story) -> Pos {
        let n = self.story(story).len();
        for bi in 0..n {
            if let Some(p) = self.first_path_in_block(story, bi) {
                return p.at(0);
            }
        }
        ParaPath { story, block: 0, cell: None }.at(0)
    }

    pub fn end_of(&self, story: Story) -> Pos {
        let n = self.story(story).len();
        for bi in (0..n).rev() {
            if let Some(p) = self.last_path_in_block(story, bi) {
                let len = self.para(&p).map(|x| x.len()).unwrap_or(0);
                return p.at(len);
            }
        }
        ParaPath { story, block: 0, cell: None }.at(0)
    }

    /// Make sure a story has at least one paragraph and ends with one (Word requires a paragraph after a final table).
    pub fn ensure_story(&mut self, story: Story) {
        let s = self.story_mut(story);
        if s.is_empty() || matches!(s.last(), Some(Block::Table(_))) {
            s.push(Block::Para(Paragraph::new()));
        }
    }

    /// Clamp a position to a valid one.
    pub fn clamp(&self, pos: Pos) -> Pos {
        if let Some(p) = self.para(&pos.path) {
            return Pos { path: pos.path, offset: pos.offset.min(p.len()) };
        }
        // Path invalid: try the same block's first paragraph, else the last paragraph.
        let blocks = self.story(pos.path.story);
        if pos.path.block < blocks.len() {
            if let Some(p) = self.first_path_in_block(pos.path.story, pos.path.block) {
                return p.at(0);
            }
        }
        self.end_of(pos.path.story)
    }

    /// Position one char to the left (crossing paragraphs).
    pub fn left(&self, pos: Pos) -> Pos {
        if pos.offset > 0 {
            return Pos { offset: pos.offset - 1, ..pos };
        }
        match self.prev_path(&pos.path) {
            Some(pp) => pp.at(self.para(&pp).map(|p| p.len()).unwrap_or(0)),
            None => pos,
        }
    }

    /// Position one char to the right (crossing paragraphs).
    pub fn right(&self, pos: Pos) -> Pos {
        let len = self.para(&pos.path).map(|p| p.len()).unwrap_or(0);
        if pos.offset < len {
            return Pos { offset: pos.offset + 1, ..pos };
        }
        match self.next_path(&pos.path) {
            Some(np) => np.at(0),
            None => pos,
        }
    }

    /// Start of the previous word (Ctrl+Left).
    pub fn word_left(&self, pos: Pos) -> Pos {
        let Some(p) = self.para(&pos.path) else { return pos };
        if pos.offset == 0 {
            return self.left(pos);
        }
        let ch = p.chars_vec();
        let mut i = pos.offset.min(ch.len());
        while i > 0 && ch[i - 1].is_whitespace() {
            i -= 1;
        }
        if i > 0 && is_word_char(ch[i - 1]) {
            while i > 0 && is_word_char(ch[i - 1]) {
                i -= 1;
            }
        } else if i > 0 {
            i -= 1;
        }
        Pos { offset: i, ..pos }
    }

    /// Start of the next word (Ctrl+Right).
    pub fn word_right(&self, pos: Pos) -> Pos {
        let Some(p) = self.para(&pos.path) else { return pos };
        let ch = p.chars_vec();
        if pos.offset >= ch.len() {
            return self.right(pos);
        }
        let mut i = pos.offset;
        if is_word_char(ch[i]) {
            while i < ch.len() && is_word_char(ch[i]) {
                i += 1;
            }
        } else {
            i += 1;
        }
        while i < ch.len() && ch[i].is_whitespace() {
            i += 1;
        }
        Pos { offset: i, ..pos }
    }

    /// Word range around `pos` (double-click selection): includes trailing spaces like Word.
    pub fn word_at(&self, pos: Pos) -> (Pos, Pos) {
        let Some(p) = self.para(&pos.path) else { return (pos, pos) };
        let ch = p.chars_vec();
        if ch.is_empty() {
            return (pos, pos);
        }
        let mut i = pos.offset.min(ch.len().saturating_sub(1));
        if pos.offset == ch.len() && i > 0 {
            i = ch.len() - 1;
        }
        let (mut a, mut b) = (i, i + 1);
        if is_word_char(ch[i]) {
            while a > 0 && is_word_char(ch[a - 1]) {
                a -= 1;
            }
            while b < ch.len() && is_word_char(ch[b]) {
                b += 1;
            }
        }
        while b < ch.len() && ch[b] == ' ' {
            b += 1;
        }
        (Pos { offset: a, ..pos }, Pos { offset: b, ..pos })
    }

    /// Sentence range around `pos` (Ctrl+click).
    pub fn sentence_at(&self, pos: Pos) -> (Pos, Pos) {
        let Some(p) = self.para(&pos.path) else { return (pos, pos) };
        let ch = p.chars_vec();
        let ends = |c: char| matches!(c, '.' | '!' | '?');
        let mut a = pos.offset.min(ch.len());
        while a > 0 && !(ends(ch[a - 1]) && a < ch.len() && ch[a].is_whitespace()) {
            a -= 1;
        }
        while a < ch.len() && ch[a].is_whitespace() && a < pos.offset {
            a += 1;
        }
        let mut b = pos.offset.min(ch.len());
        while b < ch.len() && !ends(ch[b]) {
            b += 1;
        }
        while b < ch.len() && (ends(ch[b]) || ch[b] == ' ') {
            b += 1;
        }
        (Pos { offset: a, ..pos }, Pos { offset: b, ..pos })
    }

    /// Whole paragraph range.
    pub fn paragraph_range(&self, pos: Pos) -> (Pos, Pos) {
        let len = self.para(&pos.path).map(|p| p.len()).unwrap_or(0);
        (pos.path.at(0), pos.path.at(len))
    }

    /// Paths of paragraphs touched by the range `[a, b]` (inclusive of both ends' paragraphs).
    pub fn paths_in_range(&self, a: Pos, b: Pos) -> Vec<ParaPath> {
        let (a, b) = ordered(a, b);
        let mut out = vec![a.path];
        let mut cur = a.path;
        while cur != b.path {
            match self.next_path(&cur) {
                Some(n) if n <= b.path => {
                    out.push(n);
                    cur = n;
                }
                _ => break,
            }
        }
        out
    }

    // -----------------------------------------------------------------------
    // Editing
    // -----------------------------------------------------------------------

    /// Insert text at `pos` with `props`; '\n' starts a new paragraph. Returns the caret after the text.
    pub fn insert_text(&mut self, pos: Pos, text: &str, props: &CharProps) -> Pos {
        let mut pos = self.clamp(pos);
        let mut props = props.clone();
        if self.settings.track_changes {
            props.revision = Some(self.new_revision(RevisionKind::Insert));
        }
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut first = true;
        for line in text.split('\n') {
            if !first {
                pos = self.split_paragraph(pos);
            }
            first = false;
            if line.is_empty() {
                continue;
            }
            let runs = text_runs(line, &props);
            let n: usize = runs.iter().map(|r| r.len()).sum();
            if let Some(p) = self.para_mut(&pos.path) {
                p.insert_runs(pos.offset, runs);
            }
            pos.offset += n;
        }
        pos
    }

    /// Insert a non-text inline (tab, break, image, field …).
    pub fn insert_inline(&mut self, pos: Pos, content: Inline, props: &CharProps) -> Pos {
        let pos = self.clamp(pos);
        let mut props = props.clone();
        if self.settings.track_changes {
            props.revision = Some(self.new_revision(RevisionKind::Insert));
        }
        if let Some(p) = self.para_mut(&pos.path) {
            p.insert_runs(pos.offset, vec![Run { content, props }]);
        }
        Pos { offset: pos.offset + 1, ..pos }
    }

    pub fn new_revision(&self, kind: RevisionKind) -> Revision {
        Revision { kind, author: self.settings.author.clone(), date: crate::util::now_iso() }
    }

    /// Split the paragraph at `pos` (Enter). Returns the start of the new paragraph.
    pub fn split_paragraph(&mut self, pos: Pos) -> Pos {
        let pos = self.clamp(pos);
        let next_style = {
            let p = match self.para(&pos.path) {
                Some(p) => p,
                None => return pos,
            };
            if pos.offset >= p.len() && p.len() > 0 {
                self.style(p.style_id()).and_then(|s| s.next.clone())
            } else {
                None
            }
        };
        let Some(p) = self.para_mut(&pos.path) else { return pos };
        let mut tail = p.split_off(pos.offset);
        if let Some(ns) = next_style {
            if ns != tail.props.style.clone().unwrap_or_else(|| "Normal".into()) {
                tail.props = ParaProps { style: if ns == "Normal" { None } else { Some(ns) }, ..Default::default() };
                tail.mark = CharProps::default();
            }
        }
        // Page break before is not inherited by the new paragraph.
        tail.props.page_break_before = None;
        match pos.path.cell {
            Some(c) => {
                let paras = self.cell_paras_mut(&pos.path).expect("cell");
                paras.insert(c.para + 1, tail);
                ParaPath { cell: Some(CellRef { para: c.para + 1, ..c }), ..pos.path }.at(0)
            }
            None => {
                let blocks = self.story_mut(pos.path.story);
                blocks.insert(pos.path.block + 1, Block::Para(tail));
                ParaPath { block: pos.path.block + 1, ..pos.path }.at(0)
            }
        }
    }

    /// Delete the range between `a` and `b`. Returns the resulting caret.
    pub fn delete_range(&mut self, a: Pos, b: Pos) -> Pos {
        let (a, b) = ordered(self.clamp(a), self.clamp(b));
        if a == b {
            return a;
        }
        if self.settings.track_changes {
            return self.delete_tracked(a, b);
        }
        if a.path == b.path {
            if let Some(p) = self.para_mut(&a.path) {
                p.remove_range(a.offset, b.offset);
            }
            return a;
        }
        // Range within one cell: merge paragraphs of that cell.
        if a.path.cell.is_some() && a.path.same_cell(&b.path) {
            let (ca, cb) = (a.path.cell.unwrap(), b.path.cell.unwrap());
            let tail = {
                let p = self.para_mut(&b.path).unwrap();
                p.remove_range(0, b.offset);
                p.runs.clone()
            };
            let paras = self.cell_paras_mut(&a.path).unwrap();
            paras.drain(ca.para + 1..=cb.para);
            let pa = &mut paras[ca.para];
            let len = pa.len();
            pa.remove_range(a.offset, len);
            pa.runs.extend(tail);
            pa.normalize();
            return a;
        }
        if a.path.cell.is_none() && b.path.cell.is_none() && a.path.story == b.path.story {
            // Body paragraphs: cut tail of a, drop blocks between, cut head of b, merge.
            let tail = {
                let p = self.para_mut(&b.path).unwrap();
                p.remove_range(0, b.offset);
                p.runs.clone()
            };
            let blocks = self.story_mut(a.path.story);
            blocks.drain(a.path.block + 1..=b.path.block);
            if let Block::Para(pa) = &mut blocks[a.path.block] {
                let len = pa.len();
                pa.remove_range(a.offset, len);
                pa.runs.extend(tail);
                pa.normalize();
            }
            return a;
        }
        // Mixed (spans tables): clear covered text but keep structure, and delete whole tables
        // that are fully covered when both ends are outside them.
        let paths = self.paths_in_range(a, b);
        for p in &paths {
            let len = self.para(p).map(|x| x.len()).unwrap_or(0);
            let s = if *p == a.path { a.offset } else { 0 };
            let e = if *p == b.path { b.offset } else { len };
            if let Some(x) = self.para_mut(p) {
                x.remove_range(s, e.min(len));
            }
        }
        if a.path.cell.is_none() && b.path.cell.is_none() {
            // Remove tables strictly between, then merge a and b.
            let story = a.path.story;
            let tail = self.para(&b.path).map(|p| p.runs.clone()).unwrap_or_default();
            let blocks = self.story_mut(story);
            blocks.drain(a.path.block + 1..=b.path.block);
            if let Block::Para(pa) = &mut blocks[a.path.block] {
                pa.runs.extend(tail);
                pa.normalize();
            }
        }
        a
    }

    /// Track-changes deletion: mark text as deleted (or remove own insertions).
    fn delete_tracked(&mut self, a: Pos, b: Pos) -> Pos {
        let rev = self.new_revision(RevisionKind::Delete);
        let paths = self.paths_in_range(a, b);
        for path in &paths {
            let Some(p) = self.para_mut(path) else { continue };
            let len = p.len();
            let s = if *path == a.path { a.offset } else { 0 };
            let e = if *path == b.path { b.offset } else { len };
            if s >= e {
                continue;
            }
            let j = p.split_at(e);
            let i = p.split_at(s);
            let j = j.max(i);
            // recompute j: index of first run starting at e
            let mut acc = 0;
            let mut jj = p.runs.len();
            for (k, r) in p.runs.iter().enumerate() {
                if acc >= e {
                    jj = k;
                    break;
                }
                acc += r.len();
            }
            let _ = j;
            let mut k = i;
            let mut end = jj;
            while k < end {
                let r = &mut p.runs[k];
                match &r.props.revision {
                    Some(rv) if rv.kind == RevisionKind::Insert => {
                        p.runs.remove(k);
                        end -= 1;
                        continue;
                    }
                    Some(rv) if rv.kind == RevisionKind::Delete => {}
                    _ => r.props.revision = Some(rev.clone()),
                }
                k += 1;
            }
            p.normalize();
        }
        a
    }

    /// Accept or reject all tracked changes in `[a, b]` (whole story when `None`).
    pub fn resolve_revisions(&mut self, story: Story, range: Option<(Pos, Pos)>, accept: bool) -> usize {
        let paths = match range {
            Some((a, b)) => self.paths_in_range(a, b),
            None => self.para_paths(story),
        };
        let (ra, rb) = range.map(|(a, b)| ordered(a, b)).unwrap_or((Pos::default(), Pos::default()));
        let mut n = 0;
        for path in paths {
            let Some(p) = self.para_mut(&path) else { continue };
            let (s, e) = match range {
                Some(_) => (if path == ra.path { ra.offset } else { 0 }, if path == rb.path { rb.offset } else { usize::MAX }),
                None => (0, usize::MAX),
            };
            let mut acc = 0;
            let mut out = Vec::new();
            for mut r in std::mem::take(&mut p.runs) {
                let len = r.len();
                let inside = acc + len > s && acc < e || (len == 0 && acc >= s && acc <= e);
                acc += len;
                if inside {
                    if let Some(rv) = &r.props.revision {
                        n += 1;
                        let keep = matches!((rv.kind, accept), (RevisionKind::Insert, true) | (RevisionKind::Delete, false));
                        if keep {
                            r.props.revision = None;
                            out.push(r);
                        }
                        continue;
                    }
                }
                out.push(r);
            }
            p.runs = out;
            p.normalize();
        }
        n
    }

    /// Positions of every tracked change run (start, end) in a story.
    pub fn revisions(&self, story: Story) -> Vec<(Pos, Pos, Revision)> {
        let mut out: Vec<(Pos, Pos, Revision)> = Vec::new();
        for path in self.para_paths(story) {
            let p = self.para(&path).unwrap();
            let mut acc = 0;
            for r in &p.runs {
                let len = r.len();
                if let Some(rv) = &r.props.revision {
                    if let Some(last) = out.last_mut() {
                        if last.1 == path.at(acc) && last.2 == *rv {
                            last.1 = path.at(acc + len);
                            acc += len;
                            continue;
                        }
                    }
                    out.push((path.at(acc), path.at(acc + len), rv.clone()));
                }
                acc += len;
            }
        }
        out
    }

    /// Apply a change to the character formatting of `[a, b)`.
    pub fn apply_chars(&mut self, a: Pos, b: Pos, f: &dyn Fn(&mut CharProps)) {
        let (a, b) = ordered(self.clamp(a), self.clamp(b));
        let paths = self.paths_in_range(a, b);
        let single_empty = a == b;
        for path in &paths {
            let Some(p) = self.para_mut(path) else { continue };
            let len = p.len();
            let s = if *path == a.path { a.offset } else { 0 };
            let e = if *path == b.path { b.offset } else { len };
            if single_empty || (len == 0) {
                f(&mut p.mark);
                if single_empty {
                    continue;
                }
            }
            if s >= e {
                // whole-paragraph selection ends also format the mark
                if s == 0 && e == 0 && *path != b.path {
                    f(&mut p.mark);
                }
                continue;
            }
            p.split_at(e);
            p.split_at(s);
            let mut acc = 0;
            for r in p.runs.iter_mut() {
                let l = r.len();
                if acc >= s && acc + l <= e {
                    f(&mut r.props);
                }
                acc += l;
            }
            if e == len {
                f(&mut p.mark);
            }
            p.normalize();
        }
    }

    /// Apply a change to the paragraph formatting of every paragraph touched by `[a, b]`.
    pub fn apply_para(&mut self, a: Pos, b: Pos, f: &dyn Fn(&mut ParaProps)) {
        for path in self.paths_in_range(a, b) {
            if let Some(p) = self.para_mut(&path) {
                f(&mut p.props);
            }
        }
    }

    /// The character formatting shared by the whole range (fields that differ become `None`).
    pub fn common_chars(&self, a: Pos, b: Pos) -> CharProps {
        let (a, b) = ordered(a, b);
        if a == b {
            return self.para(&a.path).map(|p| if p.is_empty() { p.mark.clone() } else { p.props_at(a.offset) }).unwrap_or_default();
        }
        let mut first: Option<CharProps> = None;
        for path in self.paths_in_range(a, b) {
            let Some(p) = self.para(&path) else { continue };
            let s = if path == a.path { a.offset } else { 0 };
            let e = if path == b.path { b.offset } else { p.len() };
            let mut acc = 0;
            for r in &p.runs {
                let l = r.len();
                if acc + l > s && acc < e {
                    match &mut first {
                        None => first = Some(r.props.clone()),
                        Some(c) => intersect_chars(c, &r.props),
                    }
                }
                acc += l;
            }
        }
        first.unwrap_or_else(|| self.para(&a.path).map(|p| p.props_at(a.offset)).unwrap_or_default())
    }

    /// Plain text of `[a, b)`; paragraphs separated by '\n'.
    pub fn text_range(&self, a: Pos, b: Pos) -> String {
        let (a, b) = ordered(a, b);
        let mut parts = Vec::new();
        for path in self.paths_in_range(a, b) {
            let Some(p) = self.para(&path) else { continue };
            let s = if path == a.path { a.offset } else { 0 };
            let e = if path == b.path { b.offset } else { p.len() };
            let runs = p.slice_runs(s, e.max(s));
            parts.push(runs.iter().map(|r| match &r.content {
                Inline::Text(t) => t.clone(),
                Inline::Tab => "\t".into(),
                Inline::LineBreak => "\n".into(),
                _ => String::new(),
            }).collect::<String>());
        }
        parts.join("\n")
    }

    /// Copy `[a, b)` as blocks (tables partly covered become their covered paragraphs).
    pub fn extract(&self, a: Pos, b: Pos) -> Vec<Block> {
        let (a, b) = ordered(a, b);
        let mut out = Vec::new();
        if a == b {
            return out;
        }
        // Whole tables between body paragraphs are copied as tables.
        let mut done_tables = std::collections::HashSet::new();
        for path in self.paths_in_range(a, b) {
            if path.cell.is_some() && a.path.block != path.block && b.path.block != path.block {
                if done_tables.insert(path.block) {
                    out.push(self.story(path.story)[path.block].clone());
                }
                continue;
            }
            let Some(p) = self.para(&path) else { continue };
            let s = if path == a.path { a.offset } else { 0 };
            let e = if path == b.path { b.offset } else { p.len() };
            let mut np = p.clone();
            np.runs = p.slice_runs(s, e.max(s));
            out.push(Block::Para(np));
        }
        out
    }

    /// Paste blocks at `pos`. Returns the caret after the inserted content.
    pub fn insert_blocks(&mut self, pos: Pos, blocks: Vec<Block>) -> Pos {
        let mut pos = self.clamp(pos);
        if blocks.is_empty() {
            return pos;
        }
        let track = self.settings.track_changes;
        let rev = self.new_revision(RevisionKind::Insert);
        let mark = |mut runs: Vec<Run>| {
            if track {
                for r in &mut runs {
                    r.props.revision = Some(rev.clone());
                }
            }
            runs
        };
        // Single paragraph: inline insert.
        if blocks.len() == 1 {
            if let Block::Para(p) = &blocks[0] {
                let n = p.len();
                let runs = mark(p.runs.clone());
                if let Some(t) = self.para_mut(&pos.path) {
                    t.insert_runs(pos.offset, runs);
                }
                return Pos { offset: pos.offset + n, ..pos };
            }
        }
        let has_table = blocks.iter().any(|b| matches!(b, Block::Table(_)));
        if pos.path.cell.is_some() || (has_table && pos.path.story != Story::Body && false) {
            // Inside a cell: flatten everything to paragraphs.
            let mut first = true;
            for b in blocks {
                let paras: Vec<Paragraph> = match b {
                    Block::Para(p) => vec![p],
                    Block::Table(t) => t
                        .rows
                        .iter()
                        .map(|r| Paragraph::with_text(&r.cells.iter().map(|c| c.text()).collect::<Vec<_>>().join("\t")))
                        .collect(),
                };
                for p in paras {
                    if !first {
                        pos = self.split_paragraph(pos);
                    }
                    first = false;
                    let n = p.len();
                    let runs = mark(p.runs);
                    if let Some(t) = self.para_mut(&pos.path) {
                        t.insert_runs(pos.offset, runs);
                    }
                    pos.offset += n;
                }
            }
            return pos;
        }
        // Body: split at pos; first block's runs join the head, last para's runs join the tail.
        let story = pos.path.story;
        let tail_start = self.split_paragraph(pos);
        let head_idx = pos.path.block;
        let mut insert_at = head_idx + 1;
        let n = blocks.len();
        let mut caret = tail_start;
        for (i, b) in blocks.into_iter().enumerate() {
            match b {
                Block::Para(p) if i == 0 => {
                    let plen = p.len();
                    let props = p.props.clone();
                    let runs = mark(p.runs);
                    if let Some(Block::Para(h)) = self.story_mut(story).get_mut(head_idx) {
                        let off = h.len();
                        h.insert_runs(off, runs);
                        if h.len() == plen || off == 0 {
                            h.props = props;
                        }
                    }
                }
                Block::Para(p) if i == n - 1 => {
                    let len = p.len();
                    let runs = mark(p.runs);
                    let path = ParaPath { story, block: insert_at, cell: None };
                    if let Some(t) = self.para_mut(&path) {
                        t.insert_runs(0, runs);
                        t.props = p.props.clone();
                    }
                    caret = path.at(len);
                }
                Block::Para(mut p) => {
                    p.runs = mark(p.runs);
                    self.story_mut(story).insert(insert_at, Block::Para(p));
                    insert_at += 1;
                }
                Block::Table(t) => {
                    self.story_mut(story).insert(insert_at, Block::Table(t));
                    insert_at += 1;
                    caret = ParaPath { story, block: insert_at, cell: None }.at(0);
                }
            }
        }
        // Remove an empty head paragraph left before a table at the very start.
        caret
    }

    /// Find all matches of `needle` in `story`.
    pub fn find_all(&self, story: Story, needle: &str, opts: FindOptions) -> Vec<(Pos, Pos)> {
        let mut out = Vec::new();
        if needle.is_empty() {
            return out;
        }
        let nchars: Vec<char> = if opts.match_case { needle.chars().collect() } else { needle.to_lowercase().chars().collect() };
        for path in self.para_paths(story) {
            let p = self.para(&path).unwrap();
            let hay: Vec<char> = p.chars_vec();
            let cmp: Vec<char> = if opts.match_case {
                hay.clone()
            } else {
                hay.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect()
            };
            if cmp.len() < nchars.len() {
                continue;
            }
            let mut i = 0;
            while i + nchars.len() <= cmp.len() {
                if cmp[i..i + nchars.len()] == nchars[..] {
                    let e = i + nchars.len();
                    let ok = !opts.whole_word
                        || ((i == 0 || !is_word_char(hay[i - 1])) && (e == hay.len() || !is_word_char(hay[e])));
                    if ok {
                        out.push((path.at(i), path.at(e)));
                        i = e;
                        continue;
                    }
                }
                i += 1;
            }
        }
        out
    }

    /// Replace all matches; returns the count.
    pub fn replace_all(&mut self, story: Story, needle: &str, replacement: &str, opts: FindOptions) -> usize {
        let matches = self.find_all(story, needle, opts);
        // Replace from the end so earlier positions stay valid.
        for (a, b) in matches.iter().rev() {
            let props = self.para(&a.path).map(|p| p.props_at(a.offset + 1)).unwrap_or_default();
            let at = self.delete_range(*a, *b);
            self.insert_text(at, replacement, &props);
        }
        matches.len()
    }
}

/// Keep only fields that are equal in both.
pub fn intersect_chars(a: &mut CharProps, b: &CharProps) {
    macro_rules! i {
        ($($f:ident),*) => { $( if a.$f != b.$f { a.$f = None; } )* };
    }
    i!(style, font, size, bold, italic, underline, strike, double_strike, color, highlight, shading, vert_align, caps,
       small_caps, hidden, spacing, link, revision);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(paras: &[&str]) -> Document {
        let mut d = Document::new();
        d.body = paras.iter().map(|t| Block::Para(Paragraph::with_text(t))).collect();
        d
    }

    #[test]
    fn insert_and_split() {
        let mut d = doc(&["Hello world"]);
        let p = d.insert_text(Pos::new(0, 5), ",", &CharProps::default());
        assert_eq!(p, Pos::new(0, 6));
        assert_eq!(d.plain_text(), "Hello, world");
        let p = d.insert_text(Pos::new(0, 6), "\nX", &CharProps::default());
        assert_eq!(d.plain_text(), "Hello,\nX world");
        assert_eq!(p, Pos::new(1, 1));
    }

    #[test]
    fn delete_across_paragraphs() {
        let mut d = doc(&["abc", "def", "ghi"]);
        let p = d.delete_range(Pos::new(0, 1), Pos::new(2, 2));
        assert_eq!(p, Pos::new(0, 1));
        assert_eq!(d.plain_text(), "ai");
    }

    #[test]
    fn formatting_splits_runs() {
        let mut d = doc(&["Hello world"]);
        d.apply_chars(Pos::new(0, 0), Pos::new(0, 5), &|c| c.bold = Some(true));
        let p = d.body[0].as_para().unwrap();
        assert_eq!(p.runs.len(), 2);
        assert_eq!(p.runs[0].props.bold, Some(true));
        assert_eq!(d.common_chars(Pos::new(0, 1), Pos::new(0, 3)).bold, Some(true));
        assert_eq!(d.common_chars(Pos::new(0, 1), Pos::new(0, 8)).bold, None);
        d.apply_chars(Pos::new(0, 0), Pos::new(0, 11), &|c| c.bold = None);
        assert_eq!(d.body[0].as_para().unwrap().runs.len(), 1);
    }

    #[test]
    fn find_replace() {
        let mut d = doc(&["The cat sat on the mat.", "Cat!"]);
        assert_eq!(d.find_all(Story::Body, "cat", FindOptions::default()).len(), 2);
        assert_eq!(d.find_all(Story::Body, "cat", FindOptions { match_case: true, whole_word: false }).len(), 1);
        assert_eq!(d.find_all(Story::Body, "at", FindOptions { match_case: false, whole_word: true }).len(), 0);
        let n = d.replace_all(Story::Body, "the", "a", FindOptions { match_case: false, whole_word: true });
        assert_eq!(n, 2);
        assert_eq!(d.plain_text(), "a cat sat on a mat.\nCat!");
    }

    #[test]
    fn tables_navigation_and_delete() {
        let mut d = doc(&["before", "after"]);
        let mut t = Table::new(2, 2, 400.0);
        t.rows[0].cells[0] = Cell::with_text("A1");
        t.rows[1].cells[1] = Cell::with_text("B2");
        d.body.insert(1, Block::Table(t));
        let paths = d.para_paths(Story::Body);
        assert_eq!(paths.len(), 6);
        assert_eq!(d.next_path(&paths[0]), Some(paths[1]));
        assert_eq!(d.prev_path(&paths[5]), Some(paths[4]));
        assert_eq!(d.next_path(&paths[4]), Some(paths[5]));
        // delete from body into body across the table removes it
        d.delete_range(Pos::new(0, 3), Pos::new(2, 2));
        assert_eq!(d.body.len(), 1);
        assert_eq!(d.plain_text(), "befter");
    }

    #[test]
    fn track_changes() {
        let mut d = doc(&["Hello world"]);
        d.settings.track_changes = true;
        d.insert_text(Pos::new(0, 5), " big", &CharProps::default());
        d.delete_range(Pos::new(0, 0), Pos::new(0, 5));
        assert_eq!(d.plain_text(), "Hello big world");
        assert_eq!(d.revisions(Story::Body).len(), 2);
        let mut r = d.clone();
        r.resolve_revisions(Story::Body, None, false);
        assert_eq!(r.plain_text(), "Hello world");
        d.resolve_revisions(Story::Body, None, true);
        assert_eq!(d.plain_text(), " big world");
    }

    #[test]
    fn extract_and_paste() {
        let mut d = doc(&["one two", "three four"]);
        let frag = d.extract(Pos::new(0, 4), Pos::new(1, 5));
        assert_eq!(frag.len(), 2);
        let p = d.insert_blocks(Pos::new(1, 10), frag);
        assert_eq!(d.plain_text(), "one two\nthree fourtwo\nthree");
        assert_eq!(p, Pos::new(2, 5));
    }

    #[test]
    fn words() {
        let d = doc(&["Hello brave  new world"]);
        assert_eq!(d.word_right(Pos::new(0, 0)), Pos::new(0, 6));
        assert_eq!(d.word_left(Pos::new(0, 13)), Pos::new(0, 6));
        assert_eq!(d.word_at(Pos::new(0, 7)), (Pos::new(0, 6), Pos::new(0, 13)));
    }
}
