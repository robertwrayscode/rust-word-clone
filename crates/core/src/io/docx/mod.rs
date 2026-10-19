//! Office Open XML WordprocessingML (.docx) reader and writer.

mod read;
mod write;

pub use read::read;
pub(crate) use read::merge_runs;
pub use write::write;

/// Built-in style ids and the names Word uses for them in styles.xml. Word recognises built-in styles by
/// these (mostly lower-case) names, so they must be written exactly like this.
pub(crate) const BUILTIN_NAMES: &[(&str, &str)] = &[
    ("Normal", "Normal"),
    ("Heading1", "heading 1"),
    ("Heading2", "heading 2"),
    ("Heading3", "heading 3"),
    ("Heading4", "heading 4"),
    ("Heading5", "heading 5"),
    ("Heading6", "heading 6"),
    ("Heading7", "heading 7"),
    ("Heading8", "heading 8"),
    ("Heading9", "heading 9"),
    ("Title", "Title"),
    ("Subtitle", "Subtitle"),
    ("Quote", "Quote"),
    ("IntenseQuote", "Intense Quote"),
    ("ListParagraph", "List Paragraph"),
    ("NoSpacing", "No Spacing"),
    ("Caption", "caption"),
    ("Header", "header"),
    ("Footer", "footer"),
    ("FootnoteText", "footnote text"),
    ("EndnoteText", "endnote text"),
    ("CommentText", "annotation text"),
    ("CommentReference", "annotation reference"),
    ("TOCHeading", "TOC Heading"),
    ("TOC1", "toc 1"),
    ("TOC2", "toc 2"),
    ("TOC3", "toc 3"),
    ("TOC4", "toc 4"),
    ("TOC5", "toc 5"),
    ("TOC6", "toc 6"),
    ("TOC7", "toc 7"),
    ("TOC8", "toc 8"),
    ("TOC9", "toc 9"),
    ("Hyperlink", "Hyperlink"),
    ("FollowedHyperlink", "FollowedHyperlink"),
    ("Strong", "Strong"),
    ("Emphasis", "Emphasis"),
    ("IntenseEmphasis", "Intense Emphasis"),
    ("SubtleEmphasis", "Subtle Emphasis"),
    ("IntenseReference", "Intense Reference"),
    ("SubtleReference", "Subtle Reference"),
    ("BookTitle", "Book Title"),
    ("FootnoteReference", "footnote reference"),
    ("EndnoteReference", "endnote reference"),
    ("TableGrid", "Table Grid"),
    ("TableNormal", "Normal Table"),
    ("BodyText", "Body Text"),
    ("ListBullet", "List Bullet"),
    ("ListNumber", "List Number"),
];

/// The styles.xml name to write for a style.
pub(crate) fn docx_style_name(id: &str, name: &str) -> String {
    BUILTIN_NAMES.iter().find(|(i, _)| *i == id).map(|(_, n)| n.to_string()).unwrap_or_else(|| name.to_string())
}

/// The model style id for a styles.xml name that denotes a built-in style.
pub(crate) fn builtin_id_for_name(name: &str) -> Option<&'static str> {
    let n = name.trim();
    BUILTIN_NAMES.iter().find(|(_, bn)| bn.eq_ignore_ascii_case(n)).map(|(i, _)| *i)
}
