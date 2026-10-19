//! The document model: blocks, paragraphs, runs and their formatting.
//!
//! Units: all lengths are in points (1/72 inch) unless noted otherwise.
//! Text offsets inside a paragraph are counted in `char`s; every non-text
//! inline (tab, break, image, field, note reference) counts as one char.

use std::sync::Arc;

/// An RGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const RED: Color = Color::rgb(0xC0, 0, 0);
    pub const BLUE: Color = Color::rgb(0x05, 0x63, 0xC1);
    pub const YELLOW: Color = Color::rgb(255, 255, 0);
    pub const ACCENT1: Color = Color::rgb(0x2F, 0x54, 0x96);
    pub const ACCENT1_DARK: Color = Color::rgb(0x1F, 0x38, 0x64);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b }
    }

    /// Parse `RRGGBB` or `#RRGGBB`.
    pub fn from_hex(s: &str) -> Option<Color> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(s, 16).ok()?;
        Some(Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    /// `RRGGBB` (upper case, no `#`).
    pub fn to_hex(self) -> String {
        format!("{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Thick,
    Dotted,
    Dashed,
    Wavy,
    Words,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum VertAlign {
    #[default]
    Baseline,
    Superscript,
    Subscript,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RevisionKind {
    Insert,
    Delete,
}

/// A tracked change attached to a run.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Revision {
    pub kind: RevisionKind,
    pub author: String,
    /// ISO 8601 timestamp, e.g. `2026-10-19T16:00:00Z`.
    pub date: String,
}

/// Character formatting. Every field is optional: `None` means "inherit from
/// the paragraph style / character style / document defaults".
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CharProps {
    /// Character style id.
    pub style: Option<String>,
    pub font: Option<String>,
    /// Size in points.
    pub size: Option<f32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<Underline>,
    pub strike: Option<bool>,
    pub double_strike: Option<bool>,
    pub color: Option<Color>,
    pub highlight: Option<Color>,
    /// Character shading (background) colour.
    pub shading: Option<Color>,
    pub vert_align: Option<VertAlign>,
    pub caps: Option<bool>,
    pub small_caps: Option<bool>,
    pub hidden: Option<bool>,
    /// Extra spacing between characters, in points (may be negative).
    pub spacing: Option<f32>,
    /// Hyperlink target (URL, or `#bookmark`).
    pub link: Option<String>,
    /// Tracked change, if any.
    pub revision: Option<Revision>,
    /// Ids of the comments covering this text.
    pub comments: Vec<u32>,
}

impl CharProps {
    /// Overlay `other` on top of `self`: fields set in `other` win.
    pub fn merge(&mut self, other: &CharProps) {
        macro_rules! m {
            ($($f:ident),*) => { $( if other.$f.is_some() { self.$f = other.$f.clone(); } )* };
        }
        m!(style, font, size, bold, italic, underline, strike, double_strike, color, highlight, shading,
           vert_align, caps, small_caps, hidden, spacing, link, revision);
        if !other.comments.is_empty() {
            self.comments = other.comments.clone();
        }
    }

    /// True when no field is set.
    pub fn is_empty(&self) -> bool {
        *self == CharProps::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineSpacing {
    /// Multiple of single line spacing (1.0, 1.15, 1.5, 2.0 …).
    Multiple(f32),
    /// At least this many points.
    AtLeast(f32),
    /// Exactly this many points.
    Exactly(f32),
}

impl Default for LineSpacing {
    fn default() -> Self {
        LineSpacing::Multiple(1.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TabAlign {
    #[default]
    Left,
    Center,
    Right,
    Decimal,
    /// Removes an inherited tab stop at this position.
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TabLeader {
    #[default]
    None,
    Dot,
    Hyphen,
    Underscore,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabStop {
    /// Position from the paragraph's left indent origin (the left margin), in points.
    pub pos: f32,
    pub align: TabAlign,
    pub leader: TabLeader,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BorderStyle {
    #[default]
    None,
    Single,
    Double,
    Dotted,
    Dashed,
    Thick,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    pub style: BorderStyle,
    /// Line width in points.
    pub width: f32,
    pub color: Color,
    /// Space between border and text, in points.
    pub space: f32,
}

impl Default for Border {
    fn default() -> Self {
        Border { style: BorderStyle::Single, width: 0.5, color: Color::BLACK, space: 1.0 }
    }
}

impl Border {
    pub fn single() -> Self {
        Border::default()
    }
    pub fn is_visible(&self) -> bool {
        self.style != BorderStyle::None && self.width > 0.0
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Borders {
    pub top: Option<Border>,
    pub bottom: Option<Border>,
    pub left: Option<Border>,
    pub right: Option<Border>,
    /// Between consecutive paragraphs with identical borders (paragraphs), or inner horizontal lines (tables).
    pub between: Option<Border>,
    /// Inner vertical lines (tables only).
    pub inside_v: Option<Border>,
}

impl Borders {
    pub fn all(b: Border) -> Self {
        Borders {
            top: Some(b),
            bottom: Some(b),
            left: Some(b),
            right: Some(b),
            between: Some(b),
            inside_v: Some(b),
        }
    }
    pub fn is_empty(&self) -> bool {
        [self.top, self.bottom, self.left, self.right, self.between, self.inside_v]
            .iter()
            .all(|b| b.map(|b| !b.is_visible()).unwrap_or(true))
    }
}

/// Reference to a list (numbering) definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ListRef {
    /// Id of a [`ListDef`] in [`Document::lists`].
    pub list: u32,
    /// Level 0..=8.
    pub level: u8,
}

/// Paragraph formatting. `None` means inherit from the style.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ParaProps {
    /// Paragraph style id (e.g. "Normal", "Heading1").
    pub style: Option<String>,
    pub align: Option<Align>,
    pub indent_left: Option<f32>,
    pub indent_right: Option<f32>,
    /// First-line indent; negative values mean a hanging indent.
    pub indent_first: Option<f32>,
    pub space_before: Option<f32>,
    pub space_after: Option<f32>,
    pub line_spacing: Option<LineSpacing>,
    /// Don't add space between paragraphs of the same style.
    pub contextual_spacing: Option<bool>,
    pub keep_next: Option<bool>,
    pub keep_lines: Option<bool>,
    pub page_break_before: Option<bool>,
    pub widow_control: Option<bool>,
    /// `Some(None)` explicitly removes list numbering inherited from a style.
    pub list: Option<Option<ListRef>>,
    pub shading: Option<Color>,
    pub borders: Option<Borders>,
    /// Tab stops (merged with inherited ones; `TabAlign::Clear` removes).
    pub tabs: Vec<TabStop>,
    /// Outline level 0..=8 for headings (used by the navigation pane and TOC).
    pub outline_level: Option<u8>,
}

impl ParaProps {
    pub fn merge(&mut self, other: &ParaProps) {
        macro_rules! m {
            ($($f:ident),*) => { $( if other.$f.is_some() { self.$f = other.$f.clone(); } )* };
        }
        m!(style, align, indent_left, indent_right, indent_first, space_before, space_after, line_spacing,
           contextual_spacing, keep_next, keep_lines, page_break_before, widow_control, list, shading, borders,
           outline_level);
        for t in &other.tabs {
            self.tabs.retain(|x| (x.pos - t.pos).abs() > 0.5);
            if t.align != TabAlign::Clear {
                self.tabs.push(*t);
            }
        }
        self.tabs.sort_by(|a, b| a.pos.total_cmp(&b.pos));
    }
}

/// Pixel image data (any format the app can decode: PNG, JPEG, GIF, BMP).
#[derive(Debug, PartialEq)]
pub struct ImageData {
    /// Original encoded bytes.
    pub bytes: Vec<u8>,
    /// File extension / format hint, lower case: "png", "jpeg", "gif", "bmp".
    pub format: String,
    /// Intrinsic pixel size, if known.
    pub pixel_size: Option<(u32, u32)>,
}

/// An image placed inline in the text.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub data: Arc<ImageData>,
    /// Displayed size in points.
    pub width: f32,
    pub height: f32,
    /// Alternative text.
    pub alt: String,
}

/// A field whose displayed text is computed during layout.
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    PageNumber,
    PageCount,
    /// Date/time with a Word-style picture, e.g. "d MMMM yyyy".
    Date(String),
    /// Any other field: the instruction and its last computed result.
    Other { instr: String, result: String },
}

/// The content of a run.
#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String),
    Tab,
    LineBreak,
    PageBreak,
    ColumnBreak,
    Image(Image),
    Field(Field),
    /// Reference mark for footnote with this id.
    FootnoteRef(u32),
    /// Reference mark for endnote with this id.
    EndnoteRef(u32),
    /// A bookmark anchor (zero visible width, still one char long).
    Bookmark(String),
}

impl Inline {
    /// Length in chars.
    pub fn len(&self) -> usize {
        match self {
            Inline::Text(s) => s.chars().count(),
            _ => 1,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Plain-text rendering used for copy, search, word counts.
    pub fn plain(&self) -> String {
        match self {
            Inline::Text(s) => s.clone(),
            Inline::Tab => "\t".into(),
            Inline::LineBreak => "\u{000B}".into(),
            Inline::PageBreak => "\u{000C}".into(),
            Inline::ColumnBreak => "\u{000E}".into(),
            // Object replacement character for non-text inlines.
            _ => "\u{FFFC}".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub content: Inline,
    pub props: CharProps,
}

impl Run {
    pub fn text(s: impl Into<String>) -> Self {
        Run { content: Inline::Text(s.into()), props: CharProps::default() }
    }
    pub fn styled(s: impl Into<String>, props: CharProps) -> Self {
        Run { content: Inline::Text(s.into()), props }
    }
    pub fn len(&self) -> usize {
        self.content.len()
    }
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Paragraph {
    pub runs: Vec<Run>,
    pub props: ParaProps,
    /// Formatting for the paragraph mark (used for empty paragraphs and new typing).
    pub mark: CharProps,
}

impl Paragraph {
    pub fn new() -> Self {
        Paragraph::default()
    }
    pub fn with_text(s: &str) -> Self {
        let mut p = Paragraph::default();
        if !s.is_empty() {
            p.runs.push(Run::text(s));
        }
        p
    }
    pub fn with_style(style: &str, s: &str) -> Self {
        let mut p = Paragraph::with_text(s);
        p.props.style = Some(style.to_string());
        p
    }
    /// Length in chars.
    pub fn len(&self) -> usize {
        self.runs.iter().map(|r| r.len()).sum()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Plain text (non-text inlines become control/placeholder chars, one per inline).
    pub fn plain_text(&self) -> String {
        self.runs.iter().map(|r| r.content.plain()).collect()
    }
    /// Text only (skips non-text inlines except tabs and line breaks which become '\t' and '\n').
    pub fn text(&self) -> String {
        let mut s = String::new();
        for r in &self.runs {
            match &r.content {
                Inline::Text(t) => s.push_str(t),
                Inline::Tab => s.push('\t'),
                Inline::LineBreak => s.push('\n'),
                _ => {}
            }
        }
        s
    }
    pub fn style_id(&self) -> &str {
        self.props.style.as_deref().unwrap_or("Normal")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum VAlign {
    #[default]
    Top,
    Center,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum VMerge {
    #[default]
    None,
    /// First cell of a vertically merged group.
    Restart,
    /// Continuation of the group above (content ignored).
    Continue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub paragraphs: Vec<Paragraph>,
    /// Number of grid columns spanned.
    pub span: u32,
    pub vmerge: VMerge,
    pub shading: Option<Color>,
    pub valign: VAlign,
    /// Overrides the table borders for this cell.
    pub borders: Option<Borders>,
    /// Preferred width in points (informational; the table grid decides).
    pub width: Option<f32>,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            paragraphs: vec![Paragraph::new()],
            span: 1,
            vmerge: VMerge::None,
            shading: None,
            valign: VAlign::Top,
            borders: None,
            width: None,
        }
    }
}

impl Cell {
    pub fn with_text(s: &str) -> Self {
        Cell { paragraphs: vec![Paragraph::with_text(s)], ..Default::default() }
    }
    pub fn text(&self) -> String {
        self.paragraphs.iter().map(|p| p.text()).collect::<Vec<_>>().join("\n")
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// Minimum row height in points.
    pub height: Option<f32>,
    /// Repeat as header row on each page.
    pub header: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub rows: Vec<Row>,
    /// Grid column widths in points.
    pub columns: Vec<f32>,
    pub borders: Borders,
    /// Table style id (informational), e.g. "TableGrid".
    pub style: Option<String>,
    pub align: Align,
    /// Left indent of the table, points.
    pub indent: f32,
    /// Default cell margins (left/right), points.
    pub cell_margin: f32,
    /// Shade the first row (header) with this colour.
    pub header_shading: Option<Color>,
    /// Alternate row banding colour.
    pub band_shading: Option<Color>,
}

impl Default for Table {
    fn default() -> Self {
        Table {
            rows: Vec::new(),
            columns: Vec::new(),
            borders: Borders::all(Border::single()),
            style: Some("TableGrid".into()),
            align: Align::Left,
            indent: 0.0,
            cell_margin: 5.4,
            header_shading: None,
            band_shading: None,
        }
    }
}

impl Table {
    /// A `rows` × `cols` table filling `width` points.
    pub fn new(rows: usize, cols: usize, width: f32) -> Self {
        let cols = cols.max(1);
        let w = width / cols as f32;
        Table {
            rows: (0..rows.max(1))
                .map(|_| Row { cells: (0..cols).map(|_| Cell::default()).collect(), height: None, header: false })
                .collect(),
            columns: vec![w; cols],
            ..Default::default()
        }
    }
    pub fn width(&self) -> f32 {
        self.columns.iter().sum()
    }
    /// Grid column index where cell `c` of row `r` starts.
    pub fn grid_col(&self, r: usize, c: usize) -> usize {
        self.rows[r].cells.iter().take(c).map(|c| c.span.max(1) as usize).sum()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Para(Paragraph),
    Table(Table),
}

impl Block {
    pub fn as_para(&self) -> Option<&Paragraph> {
        match self {
            Block::Para(p) => Some(p),
            _ => None,
        }
    }
    pub fn as_para_mut(&mut self) -> Option<&mut Paragraph> {
        match self {
            Block::Para(p) => Some(p),
            _ => None,
        }
    }
    pub fn as_table(&self) -> Option<&Table> {
        match self {
            Block::Table(t) => Some(t),
            _ => None,
        }
    }
    pub fn as_table_mut(&mut self) -> Option<&mut Table> {
        match self {
            Block::Table(t) => Some(t),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

/// Page setup.
#[derive(Clone, Debug, PartialEq)]
pub struct PageSetup {
    pub width: f32,
    pub height: f32,
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub margin_right: f32,
    /// Distance from page top to header top.
    pub header_distance: f32,
    /// Distance from page bottom to footer bottom.
    pub footer_distance: f32,
    pub columns: u32,
    pub column_gap: f32,
    /// Different first page header/footer.
    pub title_page: bool,
    /// Number of the first page.
    pub page_number_start: u32,
    pub page_color: Option<Color>,
    /// Page border drawn around the margins.
    pub page_borders: Option<Borders>,
    /// Diagonal text watermark (Design > Watermark).
    pub watermark: Option<String>,
}

impl Default for PageSetup {
    fn default() -> Self {
        PageSetup {
            width: 612.0,
            height: 792.0,
            margin_top: 72.0,
            margin_bottom: 72.0,
            margin_left: 72.0,
            margin_right: 72.0,
            header_distance: 36.0,
            footer_distance: 36.0,
            columns: 1,
            column_gap: 36.0,
            title_page: false,
            page_number_start: 1,
            page_color: None,
            page_borders: None,
            watermark: None,
        }
    }
}

impl PageSetup {
    pub fn orientation(&self) -> Orientation {
        if self.width > self.height {
            Orientation::Landscape
        } else {
            Orientation::Portrait
        }
    }
    /// Width available for text (all columns together).
    pub fn text_width(&self) -> f32 {
        (self.width - self.margin_left - self.margin_right).max(36.0)
    }
    pub fn text_height(&self) -> f32 {
        (self.height - self.margin_top - self.margin_bottom).max(36.0)
    }
    pub fn column_width(&self) -> f32 {
        let n = self.columns.max(1) as f32;
        ((self.text_width() - self.column_gap * (n - 1.0)) / n).max(18.0)
    }
}

/// Named paper sizes (points).
pub const PAPER_SIZES: &[(&str, f32, f32)] = &[
    ("Letter", 612.0, 792.0),
    ("Legal", 612.0, 1008.0),
    ("A4", 595.3, 841.9),
    ("A5", 419.6, 595.3),
    ("A3", 841.9, 1190.6),
    ("B5 (JIS)", 515.9, 728.5),
    ("Executive", 522.0, 756.0),
    ("Tabloid", 792.0, 1224.0),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum NumFormat {
    #[default]
    Decimal,
    LowerLetter,
    UpperLetter,
    LowerRoman,
    UpperRoman,
    Bullet,
    /// Decimal with leading zero (01, 02 …).
    DecimalZero,
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListLevel {
    pub format: NumFormat,
    /// Level text: `%1.` style placeholders (`%1`..`%9` refer to levels), or the bullet character.
    pub text: String,
    pub start: u32,
    /// Left indent of the text, points.
    pub indent: f32,
    /// Hanging indent (number sits this far left of the text), points.
    pub hanging: f32,
    /// Font for the bullet/number (e.g. "Symbol"), if any.
    pub font: Option<String>,
}

/// A list (numbering) definition with up to 9 levels.
#[derive(Clone, Debug, PartialEq)]
pub struct ListDef {
    pub id: u32,
    pub levels: Vec<ListLevel>,
}

impl ListDef {
    pub fn bullets(id: u32) -> Self {
        let chars = ['•', '◦', '▪'];
        ListDef {
            id,
            levels: (0..9)
                .map(|i| ListLevel {
                    format: NumFormat::Bullet,
                    text: chars[i % 3].to_string(),
                    start: 1,
                    indent: 36.0 * (i as f32 + 1.0),
                    hanging: 18.0,
                    font: None,
                })
                .collect(),
        }
    }
    pub fn numbered(id: u32) -> Self {
        let fmts = [NumFormat::Decimal, NumFormat::LowerLetter, NumFormat::LowerRoman];
        ListDef {
            id,
            levels: (0..9)
                .map(|i| ListLevel {
                    format: fmts[i % 3],
                    text: format!("%{}.", i + 1),
                    start: 1,
                    indent: 36.0 * (i as f32 + 1.0),
                    hanging: 18.0,
                    font: None,
                })
                .collect(),
        }
    }
    /// Outline numbering 1. / 1.1. / 1.1.1.
    pub fn multilevel(id: u32) -> Self {
        ListDef {
            id,
            levels: (0..9)
                .map(|i| ListLevel {
                    format: NumFormat::Decimal,
                    text: (1..=i + 1).map(|l| format!("%{}.", l)).collect::<String>(),
                    start: 1,
                    indent: 18.0 + 25.0 * i as f32 + 18.0,
                    hanging: 18.0 + 7.0 * i as f32,
                    font: None,
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum StyleKind {
    #[default]
    Paragraph,
    Character,
    Table,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Style {
    /// Identifier used in `ParaProps::style` / `CharProps::style`, e.g. "Heading1".
    pub id: String,
    /// Display name, e.g. "Heading 1".
    pub name: String,
    pub kind: StyleKind,
    pub based_on: Option<String>,
    /// Style for the paragraph created when pressing Enter at the end.
    pub next: Option<String>,
    pub para: ParaProps,
    pub chars: CharProps,
    /// Show in the Styles gallery on the Home tab.
    pub quick: bool,
}

/// A footnote or endnote.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub id: u32,
    pub paragraphs: Vec<Paragraph>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    pub id: u32,
    pub author: String,
    pub initials: String,
    pub date: String,
    pub paragraphs: Vec<Paragraph>,
    pub resolved: bool,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DocProperties {
    pub title: String,
    pub subject: String,
    pub author: String,
    pub keywords: String,
    pub description: String,
    pub last_modified_by: String,
    /// ISO 8601.
    pub created: String,
    pub modified: String,
}

/// Header/footer content for a document (one set; Word's per-section headers are flattened).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct HeadersFooters {
    pub header: Vec<Block>,
    pub footer: Vec<Block>,
    /// Used on the first page when `PageSetup::title_page` is set.
    pub first_header: Vec<Block>,
    pub first_footer: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Track changes on/off (Review > Track Changes).
    pub track_changes: bool,
    /// Default tab stop interval, points.
    pub default_tab: f32,
    /// Author name used for revisions and comments.
    pub author: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { track_changes: false, default_tab: 36.0, author: "Author".into() }
    }
}

/// A whole document.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub body: Vec<Block>,
    /// Styles, including built-ins (see [`crate::styles::builtin_styles`]).
    pub styles: Vec<Style>,
    /// Document-wide default character formatting (below all styles).
    pub default_chars: CharProps,
    /// Document-wide default paragraph formatting (below all styles).
    pub default_para: ParaProps,
    pub lists: Vec<ListDef>,
    pub page: PageSetup,
    pub headers: HeadersFooters,
    pub footnotes: Vec<Note>,
    pub endnotes: Vec<Note>,
    pub comments: Vec<Comment>,
    pub properties: DocProperties,
    pub settings: Settings,
}

impl Default for Document {
    fn default() -> Self {
        Document::new()
    }
}

impl Document {
    /// A blank document with the built-in styles ("Normal" = Calibri 11pt,
    /// 8pt after, 1.08 line spacing as in Word's default template).
    pub fn new() -> Self {
        Document {
            body: vec![Block::Para(Paragraph::new())],
            styles: crate::styles::builtin_styles(),
            default_chars: CharProps {
                font: Some("Calibri".into()),
                size: Some(11.0),
                bold: Some(false),
                italic: Some(false),
                underline: Some(Underline::None),
                strike: Some(false),
                color: Some(Color::BLACK),
                vert_align: Some(VertAlign::Baseline),
                ..Default::default()
            },
            default_para: ParaProps {
                align: Some(Align::Left),
                indent_left: Some(0.0),
                indent_right: Some(0.0),
                indent_first: Some(0.0),
                space_before: Some(0.0),
                space_after: Some(8.0),
                line_spacing: Some(LineSpacing::Multiple(1.08)),
                widow_control: Some(true),
                ..Default::default()
            },
            lists: Vec::new(),
            page: PageSetup::default(),
            headers: HeadersFooters::default(),
            footnotes: Vec::new(),
            endnotes: Vec::new(),
            comments: Vec::new(),
            properties: DocProperties::default(),
            settings: Settings::default(),
        }
    }

    /// A document whose body is the given paragraphs of plain text.
    pub fn from_text(text: &str) -> Self {
        let mut d = Document::new();
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        d.body = text.split('\n').map(|l| Block::Para(Paragraph::with_text(l))).collect();
        if d.body.is_empty() {
            d.body.push(Block::Para(Paragraph::new()));
        }
        d
    }

    pub fn style(&self, id: &str) -> Option<&Style> {
        self.styles.iter().find(|s| s.id == id)
    }

    pub fn style_by_name(&self, name: &str) -> Option<&Style> {
        self.styles.iter().find(|s| s.name.eq_ignore_ascii_case(name) || s.id.eq_ignore_ascii_case(name))
    }

    pub fn list(&self, id: u32) -> Option<&ListDef> {
        self.lists.iter().find(|l| l.id == id)
    }

    /// Add a list definition and return its id.
    pub fn add_list(&mut self, mut def: ListDef) -> u32 {
        let id = self.lists.iter().map(|l| l.id).max().unwrap_or(0) + 1;
        def.id = id;
        self.lists.push(def);
        id
    }

    pub fn story(&self, s: Story) -> &Vec<Block> {
        match s {
            Story::Body => &self.body,
            Story::Header => &self.headers.header,
            Story::Footer => &self.headers.footer,
            Story::FirstHeader => &self.headers.first_header,
            Story::FirstFooter => &self.headers.first_footer,
        }
    }

    pub fn story_mut(&mut self, s: Story) -> &mut Vec<Block> {
        match s {
            Story::Body => &mut self.body,
            Story::Header => &mut self.headers.header,
            Story::Footer => &mut self.headers.footer,
            Story::FirstHeader => &mut self.headers.first_header,
            Story::FirstFooter => &mut self.headers.first_footer,
        }
    }

    /// Plain text of the body, paragraphs separated by '\n' (table cells by '\t').
    pub fn plain_text(&self) -> String {
        let mut out = Vec::new();
        for b in &self.body {
            match b {
                Block::Para(p) => out.push(p.text()),
                Block::Table(t) => {
                    for r in &t.rows {
                        out.push(r.cells.iter().map(|c| c.text().replace('\n', " ")).collect::<Vec<_>>().join("\t"));
                    }
                }
            }
        }
        out.join("\n")
    }

    /// Next free id for footnotes/endnotes/comments.
    pub fn next_note_id(&self) -> u32 {
        self.footnotes.iter().chain(self.endnotes.iter()).map(|n| n.id).max().unwrap_or(0) + 1
    }
    pub fn next_comment_id(&self) -> u32 {
        self.comments.iter().map(|c| c.id).max().unwrap_or(0) + 1
    }
}

/// Which text flow a position lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Story {
    #[default]
    Body,
    Header,
    Footer,
    FirstHeader,
    FirstFooter,
}
