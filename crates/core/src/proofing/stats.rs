//! Word count and readability statistics (Review > Word Count, and the
//! readability statistics shown after a spelling & grammar check).

use super::text::{self, is_abbreviation_before, is_separator};
use crate::model::{Block, Document, Paragraph};

/// Counts shown in Word's "Word Count" dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DocStats {
    pub words: usize,
    pub characters_no_spaces: usize,
    pub characters_with_spaces: usize,
    /// Paragraphs containing at least one non-whitespace character.
    pub paragraphs: usize,
    pub sentences: usize,
}

impl DocStats {
    /// Statistics for the body (paragraphs and table cells), plus footnotes
    /// and endnotes when `include_notes` is set (Word's "Include textboxes,
    /// footnotes and endnotes" checkbox). Headers and footers are not counted.
    ///
    /// Characters are counted on the paragraph text: tabs and line breaks
    /// count as spaces; images, fields and other non-text inlines are not
    /// counted; paragraph marks are not counted.
    pub fn compute(doc: &Document, include_notes: bool) -> DocStats {
        let mut st = DocStats::default();
        for p in paragraphs(doc, include_notes) {
            st.add_paragraph(p);
        }
        st
    }

    /// Add one paragraph's counts.
    pub fn add_paragraph(&mut self, p: &Paragraph) {
        self.add_text(&p.text());
    }

    /// Add the counts for one paragraph of plain text.
    pub fn add_text(&mut self, t: &str) {
        let non_space = t.chars().filter(|c| !c.is_whitespace() && !is_separator(*c)).count();
        if non_space == 0 {
            self.characters_with_spaces += t.chars().filter(|c| *c != '\u{FFFC}').count();
            return;
        }
        self.paragraphs += 1;
        self.words += count_words(t);
        self.characters_no_spaces += non_space;
        self.characters_with_spaces += t.chars().filter(|c| *c != '\u{FFFC}').count();
        self.sentences += count_sentences(t);
    }
}

/// Every paragraph counted by [`DocStats::compute`], in document order.
fn paragraphs(doc: &Document, include_notes: bool) -> Vec<&Paragraph> {
    fn blocks<'a>(bs: &'a [Block], out: &mut Vec<&'a Paragraph>) {
        for b in bs {
            match b {
                Block::Para(p) => out.push(p),
                Block::Table(t) => {
                    for r in &t.rows {
                        for c in &r.cells {
                            out.extend(c.paragraphs.iter());
                        }
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    blocks(&doc.body, &mut out);
    if include_notes {
        for n in doc.footnotes.iter().chain(&doc.endnotes) {
            out.extend(n.paragraphs.iter());
        }
    }
    out
}

/// Count words the way Word does: a word is a maximal run of characters
/// that are not whitespace. Hyphenated words and contractions are one word,
/// numbers count as words, and a stand-alone punctuation token such as a
/// spaced dash ("a – b") also counts, as in Word. Control characters and
/// U+FFFC (non-text inlines) are treated as whitespace.
pub fn count_words(text: &str) -> usize {
    text.split(is_separator).filter(|t| !t.is_empty()).count()
}

/// Count sentences in one paragraph: runs of text ended by '.', '!' or '?'
/// (followed by whitespace or the end), ignoring common abbreviations
/// ("e.g.", "Mr.") and decimal points. Text after the last terminator counts
/// as a sentence too.
pub fn count_sentences(text: &str) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut n = 0;
    let mut in_sentence = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_alphanumeric() {
            in_sentence = true;
        } else if matches!(c, '.' | '!' | '?' | '\u{2026}') && in_sentence {
            // Skip a run of terminators and closing quotes/brackets.
            let mut j = i + 1;
            while j < chars.len()
                && matches!(chars[j], '.' | '!' | '?' | '"' | '\'' | '\u{201D}' | '\u{2019}' | ')' | ']')
            {
                j += 1;
            }
            let at_end = j >= chars.len() || is_separator(chars[j]);
            let mut ends = at_end;
            if ends && c == '.' && j == i + 1 && is_abbreviation_before(&chars, i) {
                // "Mr. Smith" continues; "etc." at paragraph end or before a capital ends.
                let mut k = j;
                while k < chars.len() && is_separator(chars[k]) {
                    k += 1;
                }
                ends = k >= chars.len() || chars[k].is_uppercase() && !is_title_abbrev(&chars, i);
            }
            if ends {
                n += 1;
                in_sentence = false;
            }
            i = j;
            continue;
        }
        i += 1;
    }
    if in_sentence {
        n += 1;
    }
    n
}

/// Abbreviations that are normally followed by a name ("Mr. Smith").
fn is_title_abbrev(chars: &[char], dot: usize) -> bool {
    let mut s = dot;
    while s > 0 && chars[s - 1].is_alphabetic() {
        s -= 1;
    }
    let w: String = chars[s..dot].iter().collect::<String>().to_lowercase();
    matches!(
        w.as_str(),
        "mr" | "mrs" | "ms" | "dr" | "prof" | "st" | "mt" | "jr" | "sr" | "gen" | "col" | "lt" | "capt" | "rev"
    ) || w.chars().count() == 1
}

/// Readability statistics as shown by Word.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Readability {
    /// 0–100, higher is easier.
    pub flesch_reading_ease: f64,
    /// US school grade level (≥ 0).
    pub flesch_kincaid_grade: f64,
    pub words_per_sentence: f64,
    pub sentences_per_paragraph: f64,
    /// Average letters/digits per word.
    pub chars_per_word: f64,
}

/// Readability of the document body (paragraphs and table cells).
pub fn readability(doc: &Document) -> Readability {
    let texts: Vec<String> = paragraphs(doc, false).into_iter().map(|p| p.text()).collect();
    readability_of(texts.iter().map(|s| s.as_str()))
}

/// Readability of some paragraphs of plain text.
pub fn readability_of<'a>(paras: impl IntoIterator<Item = &'a str>) -> Readability {
    let (mut words, mut syll, mut letters, mut sentences, mut pars) = (0usize, 0usize, 0usize, 0usize, 0usize);
    for t in paras {
        let chars: Vec<char> = t.chars().collect();
        let ws: Vec<_> =
            text::words(&chars).into_iter().filter(|w| w.text.chars().any(|c| c.is_alphabetic())).collect();
        if ws.is_empty() {
            continue;
        }
        pars += 1;
        sentences += count_sentences(t).max(1);
        for w in &ws {
            words += 1;
            letters += w.text.chars().filter(|c| c.is_alphanumeric()).count();
            syll += syllables(&w.text);
        }
    }
    if words == 0 {
        return Readability::default();
    }
    let wps = words as f64 / sentences as f64;
    let spw = syll as f64 / words as f64;
    Readability {
        flesch_reading_ease: (206.835 - 1.015 * wps - 84.6 * spw).clamp(0.0, 100.0),
        flesch_kincaid_grade: (0.39 * wps + 11.8 * spw - 15.59).max(0.0),
        words_per_sentence: wps,
        sentences_per_paragraph: sentences as f64 / pars as f64,
        chars_per_word: letters as f64 / words as f64,
    }
}

/// Estimate the number of syllables in an English word (heuristic: vowel
/// groups, minus silent endings, plus a few vowel pairs that split).
pub fn syllables(word: &str) -> usize {
    if word.contains(['-', '\u{2010}']) {
        return word.split(['-', '\u{2010}']).filter(|p| !p.is_empty()).map(syllables).sum::<usize>().max(1);
    }
    let w: Vec<char> = word.chars().filter(|c| c.is_alphabetic()).flat_map(|c| c.to_lowercase()).collect();
    if w.is_empty() {
        return if word.chars().any(|c| c.is_ascii_digit()) { 1 } else { 0 };
    }
    if w.len() <= 3 {
        return 1;
    }
    let vowel = |c: char| matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y' | 'é' | 'è' | 'à' | 'ü' | 'ö' | 'ä');
    let mut count = 0;
    let mut prev = false;
    for (i, &c) in w.iter().enumerate() {
        let v = vowel(c) && !(c == 'y' && i == 0);
        if v && !prev {
            count += 1;
        }
        prev = v;
    }
    let s: String = w.iter().collect();
    let n = w.len();
    if s.ends_with('e') && !s.ends_with("le") && !s.ends_with("ee") && !s.ends_with("ye") && count > 1 {
        count -= 1;
    } else if s.ends_with("le") && n > 2 && vowel(w[n - 3]) && count > 1 {
        count -= 1; // "whale", "smile"
    }
    if s.ends_with("ed") && !s.ends_with("ted") && !s.ends_with("ded") && count > 1 && !vowel(w[n - 3]) {
        count -= 1;
    }
    if s.ends_with("es")
        && count > 1
        && !["ses", "zes", "ces", "ges", "xes", "shes", "ches"].iter().any(|e| s.ends_with(e))
        && !vowel(w[n - 3])
    {
        count -= 1;
    }
    for pair in ["ia", "iu", "ii", "io", "eo", "ua", "uo"] {
        let mut from = 0;
        while let Some(p) = s[from..].find(pair) {
            let at = from + p;
            let before = &s[..at];
            let split = !(pair == "io"
                && (before.ends_with('t')
                    || before.ends_with('s')
                    || before.ends_with('c')
                    || before.ends_with('g')
                    || before.ends_with('x')))
                && !(pair == "ia" && (before.ends_with('c') || before.ends_with('t')) && s[at..].starts_with("ial"))
                && !(pair == "ua" && before.ends_with('q'))
                && !(pair == "uo" && before.ends_with('q'))
                && !(pair == "eo" && s[at..].starts_with("eou"));
            if split {
                count += 1;
            }
            from = at + pair.len();
        }
    }
    count.max(1)
}
