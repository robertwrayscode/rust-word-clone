//! English spell checker (US spellings, with British spellings accepted too).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use super::text::{self, apply_case, case_pattern, is_apostrophe, is_hyphen, CasePattern};
use super::{grammar, Issue, IssueKind};

/// Front-coded, deflated word list (see `assets/proofing/tools/build_proofing_data.py`).
static WORDS_BIN: &[u8] = include_bytes!("../../assets/proofing/words.bin");

/// Word class stored with each word: SCOWL size buckets 10, 20, 35, 40, 50,
/// 55, >=60, then NOSUGGEST (accepted, never suggested).
const CLASS_NOSUGGEST: u8 = 7;
/// Ranking penalty per class: common words are preferred.
const CLASS_PENALTY: [f32; 8] = [0.0, 0.08, 0.2, 0.28, 0.4, 0.5, 0.6, 9.0];

/// Words with the same lower-case char length, stored contiguously for fast
/// scanning during suggestion.
#[derive(Default)]
struct Bucket {
    /// Lower-case chars of every word in this bucket, as [`Alphabet`] codes, concatenated.
    codes: Vec<u8>,
    /// Index into `Speller::words` for each word in the bucket.
    ids: Vec<u32>,
}

/// The spell checker. Obtain the bundled English one with [`Speller::english`].
pub struct Speller {
    /// Every word in its dictionary spelling, with its class.
    words: Vec<(Box<str>, u8)>,
    /// Exact spelling -> index into `words`.
    exact: HashMap<Box<str>, u32>,
    /// Lower-cased forms of the words that contain capitals ("london", "iphone").
    lower_caps: HashSet<Box<str>>,
    /// Scanning buckets by char length.
    buckets: Vec<Bucket>,
    alphabet: Alphabet,
}

/// Maps lower-case chars to small codes so the substitution costs can be a
/// lookup table. Codes 0..26 are 'a'..'z', 26 is the apostrophe, 27 the
/// hyphen; other chars get codes as they are first seen. [`UNKNOWN`] is used
/// for chars not in the dictionary at all.
struct Alphabet {
    chars: Vec<char>,
    extra: HashMap<char, u8>,
    /// `sub[x * K + y]`: substitution cost between codes x and y.
    sub: Vec<f32>,
    /// Histogram bin of each code (accented letters share their base letter's bin).
    bin: Vec<u8>,
}

/// Number of histogram bins: 26 letters, apostrophe/hyphen, everything else.
const BINS: usize = 28;

const UNKNOWN: u8 = 255;
const K: usize = 256;

impl Alphabet {
    fn new() -> Self {
        let mut chars: Vec<char> = ('a'..='z').collect();
        chars.push('\'');
        chars.push('-');
        Alphabet { chars, extra: HashMap::new(), sub: Vec::new(), bin: Vec::new() }
    }

    fn code(&self, c: char) -> u8 {
        match c {
            'a'..='z' => c as u8 - b'a',
            '\'' => 26,
            '-' => 27,
            _ => self.extra.get(&c).copied().unwrap_or(UNKNOWN),
        }
    }

    fn code_or_add(&mut self, c: char) -> u8 {
        let code = self.code(c);
        if code != UNKNOWN || self.chars.len() >= UNKNOWN as usize {
            return code;
        }
        let code = self.chars.len() as u8;
        self.chars.push(c);
        self.extra.insert(c, code);
        code
    }

    fn build_table(&mut self) {
        let mut sub = vec![1.0f32; K * K];
        for (x, &cx) in self.chars.iter().enumerate() {
            for (y, &cy) in self.chars.iter().enumerate() {
                sub[x * K + y] = sub_cost(cx, cy);
            }
        }
        self.sub = sub;
        self.bin = (0..K)
            .map(|code| match self.chars.get(code) {
                Some('\'' | '-') => 26,
                Some(&c) => match base_letter(c) {
                    b @ 'a'..='z' => b as u8 - b'a',
                    _ => 27,
                },
                None => 27,
            })
            .collect();
    }

    /// Letter histogram of a code string.
    fn histogram(&self, codes: &[u8]) -> [i16; BINS] {
        let mut h = [0i16; BINS];
        for &c in codes {
            h[self.bin[c as usize] as usize] += 1;
        }
        h
    }
}

impl Speller {
    /// The bundled English dictionary (US + British spellings), decoded on
    /// first use and shared for the life of the program.
    pub fn english() -> &'static Speller {
        static SPELLER: OnceLock<Speller> = OnceLock::new();
        SPELLER.get_or_init(|| Speller::from_compressed(WORDS_BIN))
    }

    fn from_compressed(bin: &[u8]) -> Speller {
        let raw = super::inflate(bin);
        let mut words: Vec<(Box<str>, u8)> = Vec::with_capacity(130_000);
        let mut prev: Vec<u8> = Vec::new();
        let mut i = 0;
        while i < raw.len() {
            let keep = raw[i] as usize;
            i += 1;
            let s = i;
            while raw[i] >= 0x20 {
                i += 1;
            }
            prev.truncate(keep);
            prev.extend_from_slice(&raw[s..i]);
            let class = raw[i] - 1;
            i += 1;
            let w = std::str::from_utf8(&prev).expect("proofing data is UTF-8");
            words.push((w.into(), class));
        }
        Speller::from_words(words)
    }

    fn from_words(words: Vec<(Box<str>, u8)>) -> Speller {
        let mut exact = HashMap::with_capacity(words.len());
        let mut lower_caps = HashSet::new();
        let mut buckets: Vec<Bucket> = Vec::new();
        let mut alphabet = Alphabet::new();
        for (id, (w, class)) in words.iter().enumerate() {
            exact.insert(w.clone(), id as u32);
            let has_caps = w.chars().any(|c| c.is_uppercase());
            if has_caps {
                lower_caps.insert(w.to_lowercase().into_boxed_str());
            }
            if *class == CLASS_NOSUGGEST {
                continue;
            }
            let n = w.chars().count();
            if buckets.len() <= n {
                buckets.resize_with(n + 1, Bucket::default);
            }
            let b = &mut buckets[n];
            // to_lowercase can expand a few exotic chars; keep the length fixed.
            for c in w.chars().flat_map(|c| c.to_lowercase()).take(n) {
                b.codes.push(alphabet.code_or_add(c));
            }
            b.ids.push(id as u32);
        }
        alphabet.build_table();
        Speller { words, exact, lower_caps, buckets, alphabet }
    }

    /// Number of words in the dictionary.
    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Is `word` spelled correctly? See [`Speller::check_word`].
    pub fn is_correct(&self, word: &str) -> bool {
        self.check_word(word, None)
    }

    /// Is `word` spelled correctly, also accepting the words of `user`?
    ///
    /// * Case: "hello", "Hello" and "HELLO" are accepted when "hello" is in
    ///   the dictionary; "London"/"LONDON" when "London" is, but not "london".
    /// * Ignored (always accepted): single letters, words containing digits,
    ///   URLs / e-mail addresses, and all-caps acronyms of up to 5 letters.
    /// * Straight and curly apostrophes are equivalent; a possessive "’s" is
    ///   accepted on any correct word, and a trailing plural-possessive
    ///   apostrophe ("students’") is ignored.
    /// * Hyphenated compounds are correct if the whole compound or every part is.
    pub fn check_word(&self, word: &str, user: Option<&UserDictionary>) -> bool {
        let w: String = word.chars().map(|c| if is_apostrophe(c) { '\'' } else { c }).collect();
        let w = w.trim_start_matches('\'');
        let w = if w.ends_with("s'") { &w[..w.len() - 1] } else { w.trim_end_matches('\'') };
        if is_ignorable(w) {
            return true;
        }
        if self.lookup(w) || user.is_some_and(|u| u.contains(w) || u.is_ignored(w)) {
            return true;
        }
        // "café", "naïve", "résumé": accept the unaccented dictionary spelling.
        if !w.is_ascii() {
            let plain: String = w.chars().map(base_letter_any_case).collect();
            if plain != w && self.lookup(&plain) {
                return true;
            }
        }
        if let Some(stem) = w.strip_suffix("'s").or_else(|| w.strip_suffix("'S")) {
            if !stem.is_empty() && self.check_word(stem, user) {
                return true;
            }
        }
        if w.chars().any(is_hyphen) {
            let parts: Vec<&str> = w.split(is_hyphen).filter(|p| !p.is_empty()).collect();
            if parts.len() > 1 && parts.iter().all(|p| self.check_word(p, user)) {
                return true;
            }
        }
        false
    }

    /// Dictionary lookup with the capitalisation rules (no affix handling).
    fn lookup(&self, w: &str) -> bool {
        if self.exact.contains_key(w) {
            return true;
        }
        match case_pattern(w) {
            CasePattern::Title => {
                let lower = w.to_lowercase();
                self.exact.contains_key(lower.as_str())
            }
            CasePattern::Upper => {
                let lower = w.to_lowercase();
                if self.exact.contains_key(lower.as_str()) || self.lower_caps.contains(lower.as_str()) {
                    return true;
                }
                // "MCDONALD'S" -> "McDonald's" is in lower_caps; "IPHONE" -> "iPhone" too.
                false
            }
            _ => false,
        }
    }

    /// Up to `max` suggestions for a misspelled word, best first.
    ///
    /// Candidates are words within a weighted Damerau-Levenshtein distance of
    /// 2 (transpositions, doubled letters, vowel swaps, keyboard-adjacent
    /// keys and missing apostrophes are cheap), plus splits into two common
    /// words ("alot" -> "a lot"), ranked by distance and word commonness. The
    /// input's capitalisation is preserved ("Recieve" -> "Receive").
    /// NOSUGGEST (offensive) words are never suggested.
    pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        if max == 0 || word.is_empty() {
            return Vec::new();
        }
        let curly = word.contains('\u{2019}');
        let norm: String = word.chars().map(|c| if is_apostrophe(c) { '\'' } else { c }).collect();
        // Possessive: suggest for the stem and re-attach.
        if let Some(stem) = norm.strip_suffix("'s") {
            if stem.chars().count() > 1 && !self.is_correct(stem) {
                return self.suggest(stem, max).into_iter().map(|s| fix_apostrophes(format!("{s}'s"), curly)).collect();
            }
        }
        // Hyphenated compound: fix the first wrong part.
        if norm.chars().any(is_hyphen) && norm.chars().count() > 2 {
            let parts: Vec<&str> = norm.split(is_hyphen).collect();
            if let Some(bad) = parts.iter().position(|p| !p.is_empty() && !self.is_correct(p)) {
                let bad_offset: usize = parts[..bad].iter().map(|p| p.len() + 1).sum();
                let (pre, rest) = norm.split_at(bad_offset);
                let post = &rest[parts[bad].len()..];
                return self
                    .suggest(parts[bad], max)
                    .into_iter()
                    .map(|s| fix_apostrophes(format!("{pre}{s}{post}"), curly))
                    .collect();
            }
        }
        let input: Vec<u8> = norm.chars().flat_map(|c| c.to_lowercase()).map(|c| self.alphabet.code(c)).collect();
        let n = input.len();
        if n == 0 || n > 40 {
            return Vec::new();
        }
        let max_dist: f32 = if n <= 2 {
            1.0
        } else if n <= 4 {
            1.6
        } else {
            2.0
        };
        let input_lower = case_pattern(&norm) == CasePattern::Lower;
        let mut found: Vec<(f32, String)> = Vec::new();
        let mut row_buf = DistBuf::default();
        let del: Vec<f32> = (0..n).map(|i| edit_cost(&input, i)).collect();
        let input_hist = self.alphabet.histogram(&input);
        // Every edit that changes the letter histogram by one costs at least
        // 0.4, so a histogram difference above this cannot be within max_dist.
        let max_hist_diff = (max_dist / 0.4) as i16;
        let bins = &self.alphabet.bin;
        let lo = n.saturating_sub(2).max(1);
        for len in lo..=n + 2 {
            let Some(b) = self.buckets.get(len) else { continue };
            for (k, id) in b.ids.iter().enumerate() {
                let cand = &b.codes[k * len..(k + 1) * len];
                // Short words: only consider candidates sharing the first letter
                // (or with the first two letters swapped) to keep noise down.
                if n < 4 && cand[0] != input[0] && !(n >= 2 && len >= 2 && cand[0] == input[1] && cand[1] == input[0]) {
                    continue;
                }
                let mut h = input_hist;
                for &c in cand {
                    h[bins[c as usize] as usize] -= 1;
                }
                if h.iter().map(|x| x.abs()).sum::<i16>() > max_hist_diff {
                    continue;
                }
                let Some(d) = row_buf.distance(&input, &del, cand, max_dist, &self.alphabet.sub) else { continue };
                let (w, class) = &self.words[*id as usize];
                let mut score = d + CLASS_PENALTY[*class as usize];
                if input_lower && w.chars().any(|c| c.is_uppercase()) {
                    score += 0.25;
                }
                found.push((score, w.to_string()));
            }
        }
        // Two-word splits of common words: "alot" -> "a lot", "thankyou" -> "thank you".
        let lower: Vec<char> = norm.chars().flat_map(|c| c.to_lowercase()).collect();
        if lower.len() == n {
            let class_of = |s: &str| self.exact.get(s).map(|id| self.words[*id as usize].1).filter(|c| *c <= 2);
            for i in 1..n {
                let l: String = lower[..i].iter().collect();
                let r: String = lower[i..].iter().collect();
                if (i == 1 && l != "a") || n - i < 3 {
                    continue;
                }
                // (Single letters are not in the word list; "a" is always fine.)
                let cl = if l == "a" { Some(0) } else { class_of(&l) };
                if let (Some(cl), Some(cr)) = (cl, class_of(&r)) {
                    // "a lot" is a classic; other splits rank below one-word fixes.
                    let base = if i == 1 { 1.0 } else { 1.7 };
                    let score = base + CLASS_PENALTY[cl as usize] + CLASS_PENALTY[cr as usize];
                    found.push((score, format!("{l} {r}")));
                }
            }
        }
        found.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let mut out: Vec<String> = Vec::new();
        for (_, w) in found {
            let s = fix_apostrophes(apply_case(&norm, &w), curly);
            if s != word && !out.iter().any(|o| o.to_lowercase() == s.to_lowercase()) {
                out.push(s);
                if out.len() == max {
                    break;
                }
            }
        }
        out
    }

    /// Check a paragraph of text: spelling, repeated words and grammar-lite
    /// heuristics. Offsets in the returned issues are **char** offsets.
    /// Separators: whitespace, control characters and U+FFFC (non-text
    /// inlines) — they never form part of a word.
    ///
    /// Words in `user` (added or ignored) are not reported.
    pub fn check_text(&self, text: &str, user: Option<&UserDictionary>) -> Vec<Issue> {
        let chars: Vec<char> = text.chars().collect();
        let words = text::words(&chars);
        let mut issues = Vec::new();
        for w in &words {
            if self.check_word(&w.text, user) {
                continue;
            }
            issues.push(Issue {
                start: w.start,
                end: w.end,
                kind: IssueKind::Spelling,
                message: "Possible spelling mistake".into(),
                suggestion: self.suggest(&w.text, 1).into_iter().next(),
            });
        }
        // Repeated words, separated only by spaces/tabs.
        for pair in words.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let gap = &chars[a.end..b.start];
            if gap.is_empty() || !gap.iter().all(|c| matches!(c, ' ' | '\t' | '\u{00A0}')) {
                continue;
            }
            if !a.text.chars().all(|c| c.is_alphabetic() || is_apostrophe(c))
                || a.text.to_lowercase() != b.text.to_lowercase()
            {
                continue;
            }
            const OK_REPEATS: &[&str] = &[
                "had", "that", "bye", "ha", "haha", "no", "so", "very", "really", "knock", "tut", "chop", "boo",
                "yeah", "is",
            ];
            if OK_REPEATS.contains(&a.text.to_lowercase().as_str()) {
                continue;
            }
            issues.push(Issue {
                start: a.end,
                end: b.end,
                kind: IssueKind::RepeatedWord,
                message: format!("Repeated word: \"{}\"", b.text),
                suggestion: Some(String::new()),
            });
        }
        let spelling: Vec<(usize, usize)> =
            issues.iter().filter(|i| i.kind != IssueKind::Grammar).map(|i| (i.start, i.end)).collect();
        for g in grammar::check(&chars, &words) {
            if !spelling.iter().any(|&(s, e)| g.start < e && s < g.end) {
                issues.push(g);
            }
        }
        issues.sort_by_key(|i| (i.start, i.end));
        issues
    }
}

fn fix_apostrophes(s: String, curly: bool) -> String {
    if curly {
        s.replace('\'', "\u{2019}")
    } else {
        s
    }
}

/// Words the checker never flags: single letters, words with digits, URLs
/// and e-mail addresses, and all-caps acronyms of 2 to 5 letters.
fn is_ignorable(w: &str) -> bool {
    let n = w.chars().count();
    if n <= 1 || w.chars().any(|c| c.is_ascii_digit() || c.is_numeric()) || text::is_url_like(w) {
        return true;
    }
    let letters: Vec<char> = w.chars().filter(|c| c.is_alphabetic()).collect();
    if (2..=5).contains(&letters.len()) && letters.iter().all(|c| c.is_uppercase()) {
        return true;
    }
    // "URLs", "PDFs": all-caps acronym plus a plural "s".
    if let Some(stem) = w.strip_suffix('s') {
        let n = stem.chars().count();
        if (2..=5).contains(&n) && stem.chars().all(|c| c.is_uppercase()) {
            return true;
        }
    }
    !w.chars().any(|c| c.is_alphabetic())
}

// ---------------------------------------------------------------------------
// Weighted Damerau-Levenshtein (optimal string alignment) distance.

#[derive(Default)]
struct DistBuf {
    ins: Vec<f32>,
    prev2: Vec<f32>,
    prev: Vec<f32>,
    cur: Vec<f32>,
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
}

fn key_pos(c: char) -> Option<(f32, f32)> {
    const ROWS: [(&str, f32); 3] = [("qwertyuiop", 0.0), ("asdfghjkl", 0.25), ("zxcvbnm", 0.75)];
    for (r, (row, off)) in ROWS.iter().enumerate() {
        if let Some(i) = row.find(c) {
            return Some((i as f32 + off, r as f32));
        }
    }
    None
}

fn keyboard_adjacent(a: char, b: char) -> bool {
    match (key_pos(a), key_pos(b)) {
        (Some((x1, y1)), Some((x2, y2))) => {
            let (dx, dy) = (x1 - x2, y1 - y2);
            dx * dx + dy * dy < 1.3 * 1.3
        }
        _ => false,
    }
}

fn base_letter(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ò' | 'ó' | 'ô' | 'ö' | 'õ' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        'æ' => 'a',
        'œ' => 'o',
        _ => c,
    }
}

fn base_letter_any_case(c: char) -> char {
    if c.is_uppercase() {
        let l = c.to_lowercase().next().unwrap_or(c);
        let b = base_letter(l);
        if b != l {
            return b.to_ascii_uppercase();
        }
        c
    } else {
        base_letter(c)
    }
}

fn sub_cost(x: char, y: char) -> f32 {
    if x == y {
        0.0
    } else if base_letter(x) == base_letter(y) {
        0.3
    } else if (is_vowel(x) && is_vowel(y)) || keyboard_adjacent(x, y) {
        0.8
    } else {
        1.0
    }
}

/// Cost of deleting the char at `i` of `a` (or inserting it, when `a` is
/// the candidate): cheap for apostrophes/hyphens and doubled letters.
fn edit_cost(a: &[u8], i: usize) -> f32 {
    let c = a[i];
    if c == 26 || c == 27 {
        0.45
    } else if c != UNKNOWN && ((i > 0 && a[i - 1] == c) || (i + 1 < a.len() && a[i + 1] == c)) {
        0.6
    } else {
        1.0
    }
}

impl DistBuf {
    /// Weighted distance from `a` (input, with precomputed deletion costs
    /// `del`) to `b` (candidate), or None if it exceeds `max`.
    fn distance(&mut self, a: &[u8], del: &[f32], b: &[u8], max: f32, sub: &[f32]) -> Option<f32> {
        let m = b.len();
        self.ins.clear();
        self.ins.extend((0..m).map(|j| edit_cost(b, j)));
        self.prev2.clear();
        self.prev.clear();
        self.cur.clear();
        self.prev2.resize(m + 1, 0.0);
        self.prev.resize(m + 1, 0.0);
        self.cur.resize(m + 1, 0.0);
        for j in 1..=m {
            self.prev[j] = self.prev[j - 1] + self.ins[j - 1];
        }
        for i in 1..=a.len() {
            let ai = a[i - 1];
            let di = del[i - 1];
            let row = &sub[ai as usize * K..ai as usize * K + K];
            self.cur[0] = self.prev[0] + di;
            let mut row_min = self.cur[0];
            for j in 1..=m {
                let bj = b[j - 1];
                let sc = if ai == bj && ai != UNKNOWN { 0.0 } else { row[bj as usize] };
                let mut v = (self.prev[j] + di).min(self.cur[j - 1] + self.ins[j - 1]).min(self.prev[j - 1] + sc);
                if i > 1 && j > 1 && ai == b[j - 2] && a[i - 2] == bj && ai != a[i - 2] {
                    v = v.min(self.prev2[j - 2] + 0.6);
                }
                self.cur[j] = v;
                row_min = row_min.min(v);
            }
            if row_min > max {
                return None;
            }
            std::mem::swap(&mut self.prev2, &mut self.prev);
            std::mem::swap(&mut self.prev, &mut self.cur);
        }
        let d = self.prev[m];
        (d <= max).then_some(d)
    }
}
// ---------------------------------------------------------------------------

/// The user's custom dictionary ("Add to Dictionary") plus the session's
/// "Ignore All" list.
///
/// Saved as a UTF-8 text file with one word per line; blank lines and lines
/// starting with `#` are ignored. Ignored words are not saved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserDictionary {
    words: BTreeSet<String>,
    lower: HashSet<String>,
    ignored: HashSet<String>,
}

impl UserDictionary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a word permanently ("Add to Dictionary").
    pub fn add(&mut self, word: &str) {
        let w = normalize(word);
        if w.is_empty() {
            return;
        }
        self.lower.insert(w.to_lowercase());
        self.words.insert(w);
    }

    /// Remove a previously added word. Returns true if it was present.
    pub fn remove(&mut self, word: &str) -> bool {
        let w = normalize(word);
        let removed = self.words.remove(&w);
        if removed {
            self.lower = self.words.iter().map(|w| w.to_lowercase()).collect();
        }
        removed
    }

    /// Ignore every occurrence of `word` for this session ("Ignore All").
    pub fn ignore_all(&mut self, word: &str) {
        self.ignored.insert(normalize(word));
    }

    pub fn is_ignored(&self, word: &str) -> bool {
        self.ignored.contains(&normalize(word))
    }

    /// Is `word` in the dictionary? Same capitalisation rules as the main
    /// dictionary: an added "rword" also accepts "Rword" and "RWORD".
    pub fn contains(&self, word: &str) -> bool {
        let w = normalize(word);
        if self.words.contains(&w) {
            return true;
        }
        match case_pattern(&w) {
            CasePattern::Title => self.words.contains(&w.to_lowercase()),
            CasePattern::Upper => self.lower.contains(&w.to_lowercase()),
            _ => false,
        }
    }

    /// The added words, sorted.
    pub fn words(&self) -> impl Iterator<Item = &str> {
        self.words.iter().map(|s| s.as_str())
    }

    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Parse the text-file format.
    pub fn from_text(s: &str) -> Self {
        let mut d = UserDictionary::new();
        for line in s.lines() {
            let line = line.trim().trim_start_matches('\u{FEFF}');
            if !line.is_empty() && !line.starts_with('#') {
                d.add(line);
            }
        }
        d
    }

    /// Serialise to the text-file format (one word per line, sorted).
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for w in &self.words {
            s.push_str(w);
            s.push('\n');
        }
        s
    }

    /// Load from a file. A missing file gives an empty dictionary.
    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => Ok(Self::from_text(&s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(e),
        }
    }

    /// Save to a file (creating parent directories).
    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        std::fs::write(path, self.to_text())
    }
}

fn normalize(word: &str) -> String {
    word.trim().chars().map(|c| if is_apostrophe(c) { '\'' } else { c }).collect()
}
