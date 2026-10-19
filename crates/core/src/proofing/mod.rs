//! Proofing tools: spelling, grammar-lite, AutoCorrect, thesaurus and
//! document statistics.
//!
//! All dictionary data is bundled into the binary (compressed with deflate)
//! and decoded lazily on first use. See `crates/core/assets/proofing/SOURCES.md`
//! for data sources and licences.
//!
//! Offsets reported by [`Issue`] are **char offsets** into the text that was
//! checked (the same unit as [`crate::Paragraph`] offsets).

pub mod autocorrect;
mod grammar;
mod speller;
pub mod stats;
pub mod text;
pub mod thesaurus;

pub use autocorrect::{AutoCorrect, Replacement};
pub use speller::{Speller, UserDictionary};
pub use stats::{count_words, readability, DocStats, Readability};

/// What kind of problem an [`Issue`] reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IssueKind {
    /// A word that is not in the dictionary (red squiggle).
    Spelling,
    /// The same word twice in a row ("the the").
    RepeatedWord,
    /// A grammar / style heuristic (blue double underline).
    Grammar,
}

/// A proofing problem found by [`Speller::check_text`].
///
/// `start..end` is a half-open range of **char** offsets into the checked
/// text. To apply the fix, replace that range with `suggestion` (an empty
/// suggestion means "delete the range").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub start: usize,
    pub end: usize,
    pub kind: IssueKind,
    pub message: String,
    pub suggestion: Option<String>,
}

/// Inflate a zlib stream bundled with the binary.
pub(crate) fn inflate(bytes: &[u8]) -> Vec<u8> {
    miniz_oxide::inflate::decompress_to_vec_zlib(bytes).expect("bundled proofing data is corrupt")
}
