//! English thesaurus (from the MyThes / WordNet thesaurus shipped with
//! LibreOffice; see `assets/proofing/SOURCES.md`).
//!
//! The bundled data keeps entries for common single words (SCOWL size ≤ 50)
//! with at most 8 meanings and 12 synonyms per meaning. Antonyms and words
//! marked NOSUGGEST in the spelling dictionary are removed at build time.

use std::sync::OnceLock;

static THES_BIN: &[u8] = include_bytes!("../../assets/proofing/thesaurus.bin");

/// One sense of a word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meaning {
    /// "noun", "verb", "adjective" or "adverb".
    pub part_of_speech: String,
    pub synonyms: Vec<String>,
}

struct Thesaurus {
    text: String,
    /// (line start, line end) byte offsets, sorted by head word.
    lines: Vec<(usize, usize)>,
}

impl Thesaurus {
    fn get() -> &'static Thesaurus {
        static T: OnceLock<Thesaurus> = OnceLock::new();
        T.get_or_init(|| {
            let text = String::from_utf8(super::inflate(THES_BIN)).expect("thesaurus is UTF-8");
            let mut lines = Vec::with_capacity(33_000);
            let mut s = 0;
            for (i, b) in text.bytes().enumerate() {
                if b == b'\n' {
                    lines.push((s, i));
                    s = i + 1;
                }
            }
            if s < text.len() {
                lines.push((s, text.len()));
            }
            Thesaurus { text, lines }
        })
    }

    fn head(&self, i: usize) -> &str {
        let (s, e) = self.lines[i];
        let line = &self.text[s..e];
        line.split('\u{1f}').next().unwrap_or("")
    }

    fn find(&self, word: &str) -> Option<&str> {
        let i = self.lines.binary_search_by(|&(s, e)| {
            let head = self.text[s..e].split('\u{1f}').next().unwrap_or("");
            head.as_bytes().cmp(word.as_bytes())
        });
        i.ok().map(|i| {
            debug_assert_eq!(self.head(i), word);
            let (s, e) = self.lines[i];
            &self.text[s..e]
        })
    }
}

/// Look a word up. Tries the word as typed, then lower-case, then simple
/// inflections ("houses" -> "house", "walked" -> "walk"). Returns an empty
/// vector when nothing is found.
pub fn lookup(word: &str) -> Vec<Meaning> {
    let t = Thesaurus::get();
    let w = word.trim().replace('\u{2019}', "'");
    let lower = w.to_lowercase();
    let mut tries = vec![w.clone(), lower.clone()];
    for (suf, rep) in [
        ("ies", "y"),
        ("es", ""),
        ("s", ""),
        ("ied", "y"),
        ("ed", ""),
        ("ed", "e"),
        ("ing", ""),
        ("ing", "e"),
        ("ly", ""),
    ] {
        if let Some(stem) = lower.strip_suffix(suf) {
            if stem.chars().count() >= 3 {
                tries.push(format!("{stem}{rep}"));
            }
        }
    }
    for cand in tries {
        if let Some(line) = t.find(&cand) {
            return line
                .split('\u{1f}')
                .skip(1)
                .map(|m| {
                    let mut parts = m.split('\u{1e}');
                    let pos = parts.next().unwrap_or("").to_string();
                    let part_of_speech = match pos.as_str() {
                        "adj" => "adjective".to_string(),
                        "adv" => "adverb".to_string(),
                        _ => pos,
                    };
                    Meaning { part_of_speech, synonyms: parts.map(str::to_string).collect() }
                })
                .collect();
        }
    }
    Vec::new()
}

/// All synonyms of `word` across its meanings, de-duplicated, most relevant
/// (first meaning) first.
pub fn synonyms(word: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in lookup(word) {
        for s in m.synonyms {
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// Number of head words in the bundled thesaurus.
pub fn entry_count() -> usize {
    Thesaurus::get().lines.len()
}
