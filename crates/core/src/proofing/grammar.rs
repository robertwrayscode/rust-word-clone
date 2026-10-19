//! Grammar-lite: cheap heuristics with a low false-positive rate.
//!
//! * two or more spaces between words
//! * a sentence starting with a lower-case letter
//! * "a" vs "an"
//! * lower-case "i" used as a pronoun

use super::text::{capitalize, case_pattern, is_separator, sentence_start, CasePattern, WordSpan};
use super::{Issue, IssueKind};

pub(crate) fn check(chars: &[char], words: &[WordSpan]) -> Vec<Issue> {
    let mut out = Vec::new();
    double_spaces(chars, &mut out);
    sentence_case(chars, words, &mut out);
    articles(chars, words, &mut out);
    lower_i(chars, words, &mut out);
    out
}

fn issue(start: usize, end: usize, message: &str, suggestion: String) -> Issue {
    Issue { start, end, kind: IssueKind::Grammar, message: message.into(), suggestion: Some(suggestion) }
}

fn double_spaces(chars: &[char], out: &mut Vec<Issue>) {
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != ' ' {
            i += 1;
            continue;
        }
        let s = i;
        while i < chars.len() && chars[i] == ' ' {
            i += 1;
        }
        if i - s >= 2 && s > 0 && i < chars.len() && !is_separator(chars[s - 1]) && !is_separator(chars[i]) {
            out.push(issue(s, i, "Extra space between words", " ".into()));
        }
    }
}

fn sentence_case(chars: &[char], words: &[WordSpan], out: &mut Vec<Issue>) {
    // Paragraph-initial words are only checked when the paragraph looks like
    // prose (several words and a sentence terminator), not a list item/cell.
    let prose = words.len() >= 3 && chars.iter().any(|c| matches!(c, '.' | '!' | '?'));
    for w in words {
        let first = w.text.chars().next().unwrap_or(' ');
        if !first.is_lowercase()
            || case_pattern(&w.text) != CasePattern::Lower
            || w.text.chars().any(|c| c.is_numeric())
            || (chars.get(w.end) == Some(&'.')
                && (w.text.chars().count() == 1 || chars.get(w.end + 1).is_some_and(|c| c.is_alphabetic())))
        {
            continue;
        }
        let at_start = match sentence_start(chars, w.start) {
            None => prose,
            Some(b) => b,
        };
        if at_start {
            out.push(issue(w.start, w.end, "Sentences should start with a capital letter", capitalize(&w.text)));
        }
    }
}

/// Does the word start with a vowel *sound*? `None` when unsure.
fn vowel_sound(word: &str) -> Option<bool> {
    let letters: Vec<char> = word.chars().take_while(|c| c.is_alphabetic()).collect();
    let first = *letters.first()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    let n = letters.len();
    // Initialisms ("MBA", "FBI", "URL"): pronounced letter by letter.
    if (2..=5).contains(&n) && letters.iter().all(|c| c.is_ascii_uppercase()) {
        const WORDLIKE: &[&str] = &[
            "NASA", "NATO", "NAFTA", "UNESCO", "UNICEF", "OPEC", "SCUBA", "LASER", "RADAR", "FIFA", "AIDS", "ASAP",
            "SARS", "GIF", "JPEG", "PIN", "SIM", "RAM", "ROM", "LAN", "WAN", "SAT", "SWAT", "FAQ", "HUD", "MOOC",
            "NIMBY", "OSHA", "SMART", "LOL", "YOLO", "EULA", "ESA", "ISIS", "IKEA",
        ];
        let w: String = letters.iter().collect();
        if WORDLIKE.contains(&w.as_str()) {
            return None;
        }
        return Some(matches!(first, 'A' | 'E' | 'F' | 'H' | 'I' | 'L' | 'M' | 'N' | 'O' | 'R' | 'S' | 'X'));
    }
    if n < 2 {
        return None;
    }
    let lw = word.to_lowercase();
    let w = lw.as_str();
    // Consonant-sounding vowels: "a university", "a one-time", "a European".
    if w == "one" || w == "once" || w.starts_with("one-") || w.starts_with("eu") || w.starts_with("ewe") {
        return Some(false);
    }
    if let Some(rest) = w.strip_prefix("uni") {
        // "unimportant", "uninformed", "unidentified" start with "un-".
        return Some(matches!(rest.chars().next(), Some('n' | 'm' | 'd')));
    }
    const YOU: &[&str] = &[
        "use", "usa", "usu", "uti", "ure", "uri", "uro", "ura", "ubiq", "uku", "ufo", "utop", "ute", "uga", "ukr",
        "unan", "u-",
    ];
    if YOU.iter().any(|p| w.starts_with(p)) {
        return Some(false);
    }
    if w.starts_with('h') {
        const SILENT_H: &[&str] = &["hour", "honest", "honor", "honour", "heir"];
        if SILENT_H.iter().any(|p| w.starts_with(p)) {
            return Some(true);
        }
        return None; // "a/an historic", "herb": leave alone
    }
    if w.starts_with("x-") {
        return Some(true);
    }
    Some(matches!(first.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u'))
}

fn articles(chars: &[char], words: &[WordSpan], out: &mut Vec<Issue>) {
    for pair in words.windows(2) {
        let (art, next) = (&pair[0], &pair[1]);
        let is_a = matches!(art.text.as_str(), "a" | "A");
        let is_an = matches!(art.text.as_str(), "an" | "An" | "AN");
        if !is_a && !is_an {
            continue;
        }
        // A capital "A" mid-sentence is likely a label ("Grade A", "vitamin A").
        if art.text == "A" && sentence_start(chars, art.start) == Some(false) {
            continue;
        }
        let gap = &chars[art.end..next.start];
        if gap.is_empty() || !gap.iter().all(|c| matches!(c, ' ' | '\u{00A0}')) || next.text.chars().count() < 2 {
            continue;
        }
        match (is_a, vowel_sound(&next.text)) {
            (true, Some(true)) => {
                let s = if art.text == "A" { "An" } else { "an" };
                out.push(issue(
                    art.start,
                    art.end,
                    "Use \"an\" before a word that starts with a vowel sound",
                    s.into(),
                ));
            }
            (false, Some(false)) => {
                let s = if art.text.starts_with('A') { "A" } else { "a" };
                out.push(issue(
                    art.start,
                    art.end,
                    "Use \"a\" before a word that starts with a consonant sound",
                    s.into(),
                ));
            }
            _ => {}
        }
    }
}

fn lower_i(chars: &[char], words: &[WordSpan], out: &mut Vec<Issue>) {
    for (k, w) in words.iter().enumerate() {
        if w.text != "i" || k == 0 {
            continue;
        }
        let next = chars.get(w.end).copied();
        let prev = if w.start > 0 { Some(chars[w.start - 1]) } else { None };
        // Skip list markers ("i)", "(i)"), "i.e." and letter sequences.
        if matches!(next, Some(')' | ']' | '-'))
            || matches!(prev, Some('(' | '['))
            || (next == Some('.') && chars.get(w.end + 1).is_some_and(|c| c.is_alphabetic()))
            || words[k - 1].text.chars().count() == 1
        {
            continue;
        }
        out.push(issue(w.start, w.end, "The pronoun \"I\" should be capitalised", "I".into()));
    }
}
