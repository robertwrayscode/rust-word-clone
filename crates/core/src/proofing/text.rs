//! Small text helpers shared by the proofing tools: word tokenisation,
//! separators, case patterns.

/// True for characters that separate words and are never part of one:
/// whitespace, control characters (tabs, breaks) and U+FFFC (the object
/// replacement character used for non-text inlines).
pub fn is_separator(c: char) -> bool {
    c.is_whitespace() || c.is_control() || c == '\u{FFFC}'
}

/// Straight or curly apostrophe.
pub fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}' | '\u{02BC}')
}

/// Hyphen characters that join compound words.
pub fn is_hyphen(c: char) -> bool {
    matches!(c, '-' | '\u{2010}' | '\u{2011}')
}

/// A word found by [`words`]: `start..end` are char offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordSpan {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Does this whitespace-delimited chunk look like a URL, e-mail address,
/// domain name or file path (which must not be spell-checked)?
pub fn is_url_like(chunk: &str) -> bool {
    let lower = chunk.to_lowercase();
    if lower.contains("://") || lower.starts_with("www.") || lower.contains('\\') {
        return true;
    }
    if let Some(at) = lower.find('@') {
        if lower[at..].contains('.') {
            return true;
        }
    }
    // Bare domain names: "example.com", "docs.rs".
    const TLDS: &[&str] = &["com", "org", "net", "io", "edu", "gov", "uk", "de", "info", "dev", "rs", "ai", "app"];
    let trimmed =
        lower.trim_start_matches(|c: char| !c.is_alphanumeric()).trim_end_matches(|c: char| !c.is_alphanumeric());
    let parts: Vec<&str> = trimmed.split('.').collect();
    parts.len() >= 2
        && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '/'))
        && TLDS.contains(&parts[parts.len() - 1].split('/').next().unwrap_or(""))
}

/// Split text into words. Words are runs of letters/digits, joined by
/// internal apostrophes and hyphens ("don't", "well-known"). Separators and
/// punctuation split words; chunks that look like URLs or e-mail addresses
/// are skipped entirely.
pub fn words(chars: &[char]) -> Vec<WordSpan> {
    let mut out = Vec::new();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if is_separator(chars[i]) {
            i += 1;
            continue;
        }
        let a = i;
        while i < n && !is_separator(chars[i]) {
            i += 1;
        }
        let b = i;
        let chunk: String = chars[a..b].iter().collect();
        if is_url_like(&chunk) {
            continue;
        }
        let mut j = a;
        while j < b {
            if !chars[j].is_alphanumeric() {
                j += 1;
                continue;
            }
            let s = j;
            while j < b {
                let c = chars[j];
                let joiner = (is_apostrophe(c) || is_hyphen(c)) && j + 1 < b && chars[j + 1].is_alphanumeric();
                if !c.is_alphanumeric() && !joiner {
                    break;
                }
                j += 1;
            }
            out.push(WordSpan { start: s, end: j, text: chars[s..j].iter().collect() });
        }
    }
    out
}

/// Capitalisation pattern of a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CasePattern {
    /// "hello" (or no letters at all)
    Lower,
    /// "Hello"
    Title,
    /// "HELLO" (at least two letters)
    Upper,
    /// "iPhone", "McDonald"
    Mixed,
}

pub fn case_pattern(word: &str) -> CasePattern {
    let mut letters = word.chars().filter(|c| c.is_alphabetic());
    let Some(first) = letters.next() else { return CasePattern::Lower };
    let rest: Vec<char> = letters.collect();
    let rest_upper = rest.iter().all(|c| !c.is_lowercase());
    let rest_lower = rest.iter().all(|c| !c.is_uppercase());
    if first.is_uppercase() {
        if rest.is_empty() {
            CasePattern::Title
        } else if rest_upper {
            CasePattern::Upper
        } else if rest_lower {
            CasePattern::Title
        } else {
            CasePattern::Mixed
        }
    } else if rest_lower {
        CasePattern::Lower
    } else {
        CasePattern::Mixed
    }
}

/// Upper-case the first character.
pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Re-case `candidate` to follow the capitalisation of `input`: upper-case
/// input gives upper-case output, Title-case input capitalises the first
/// letter, otherwise the candidate is returned as is (so proper nouns keep
/// their capitals).
pub fn apply_case(input: &str, candidate: &str) -> String {
    match case_pattern(input) {
        CasePattern::Upper => candidate.to_uppercase(),
        CasePattern::Title => capitalize(candidate),
        _ => candidate.to_string(),
    }
}

/// Common abbreviations that end with a period without ending a sentence
/// (lower-case, without the final period).
pub(crate) const ABBREVIATIONS: &[&str] = &[
    "e.g", "i.e", "etc", "vs", "cf", "approx", "mr", "mrs", "ms", "dr", "prof", "st", "no", "fig", "ca", "al", "jr",
    "sr", "inc", "ltd", "co", "corp", "dept", "est", "vol", "p", "pp", "ch", "sec", "jan", "feb", "mar", "apr", "jun",
    "jul", "aug", "sep", "sept", "oct", "nov", "dec", "mt", "rev", "gen", "col", "lt", "sgt", "capt", "gov", "sen",
    "rep", "a.m", "p.m", "u.s", "u.k", "ph.d", "viz", "op", "ed", "eds", "nos",
];

/// Is the word ending at char offset `end` (exclusive, just before a '.')
/// a known abbreviation or an initial?
pub(crate) fn is_abbreviation_before(chars: &[char], end: usize) -> bool {
    let mut s = end;
    while s > 0 && (chars[s - 1].is_alphanumeric() || chars[s - 1] == '.') {
        s -= 1;
    }
    let w: String = chars[s..end].iter().collect::<String>().to_lowercase();
    if w.chars().count() == 1 && w.chars().all(|c| c.is_alphabetic()) {
        return true; // an initial, "J. Smith"
    }
    ABBREVIATIONS.contains(&w.as_str())
}

/// Is the word starting at `start` the first word of a sentence?
/// `None` means "paragraph start", `Some(true/false)` after other text.
pub(crate) fn sentence_start(chars: &[char], start: usize) -> Option<bool> {
    let mut k = start;
    // Skip opening quotes / brackets directly before the word.
    while k > 0 && matches!(chars[k - 1], '"' | '\'' | '\u{201C}' | '\u{2018}' | '(' | '[') {
        k -= 1;
    }
    let mut saw_space = false;
    while k > 0 && is_separator(chars[k - 1]) {
        saw_space = true;
        k -= 1;
    }
    if k == 0 {
        return None;
    }
    if !saw_space {
        return Some(false);
    }
    // Skip closing quotes / brackets after the terminator.
    while k > 0 && matches!(chars[k - 1], '"' | '\'' | '\u{201D}' | '\u{2019}' | ')' | ']') {
        k -= 1;
    }
    if k == 0 {
        return Some(false);
    }
    match chars[k - 1] {
        '!' | '?' => Some(true),
        '.' => {
            if k >= 2 && chars[k - 2] == '.' {
                return Some(false); // ellipsis "..."
            }
            Some(!is_abbreviation_before(chars, k - 1))
        }
        _ => Some(false),
    }
}
