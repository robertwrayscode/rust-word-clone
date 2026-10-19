use wordcore::proofing::autocorrect::smart_quote;
use wordcore::proofing::{
    count_words, readability, stats::syllables, thesaurus, AutoCorrect, DocStats, IssueKind, Replacement, Speller,
    UserDictionary,
};
use wordcore::{Block, Cell, Document, Note, Paragraph, Row, Table};

fn sp() -> &'static Speller {
    Speller::english()
}

// ---------------------------------------------------------------- spelling

#[test]
fn correct_words() {
    for w in [
        "colour",
        "color",
        "organise",
        "organize",
        "recognize",
        "recognise",
        "it's",
        "it’s",
        "don't",
        "don’t",
        "Robert",
        "Monday",
        "London",
        "hello",
        "Hello",
        "HELLO",
        "receive",
        "the",
        "centre",
        "center",
        "travelling",
        "traveling",
    ] {
        assert!(sp().is_correct(w), "{w} should be correct");
    }
}

#[test]
fn misspelled_words() {
    for w in ["recieve", "teh", "definately", "seperate", "london", "monday", "accomodate", "wierd"] {
        assert!(!sp().is_correct(w), "{w} should be flagged");
    }
}

#[test]
fn modern_words() {
    for w in [
        "email",
        "online",
        "website",
        "smartphone",
        "WiFi",
        "iPhone",
        "Google",
        "Microsoft",
        "Excel",
        "Bluetooth",
        "app",
        "apps",
        "blog",
        "emoji",
        "IPHONE",
    ] {
        assert!(sp().is_correct(w), "{w} should be correct");
    }
}

#[test]
fn ignored_tokens() {
    for w in ["x", "abc123", "3rd", "NASA", "HTML", "URLs", "https://example.com/foo", "someone@example.com", "PDFs"] {
        assert!(sp().is_correct(w), "{w} should be ignored");
    }
}

#[test]
fn possessives_and_compounds() {
    assert!(sp().is_correct("teacher's"));
    assert!(sp().is_correct("Robert’s"));
    assert!(sp().is_correct("students'"));
    assert!(sp().is_correct("well-known"));
    assert!(sp().is_correct("state-of-the-art"));
    assert!(sp().is_correct("blue-green"));
    assert!(!sp().is_correct("blue-gren"));
}

#[test]
fn suggestions() {
    let s = sp().suggest("recieve", 5);
    assert_eq!(s.first().map(String::as_str), Some("receive"), "{s:?}");
    let s = sp().suggest("definately", 5);
    assert!(s.iter().take(3).any(|w| w == "definitely"), "{s:?}");
    let s = sp().suggest("teh", 5);
    assert_eq!(s.first().map(String::as_str), Some("the"), "{s:?}");
    let s = sp().suggest("Recieve", 5);
    assert_eq!(s.first().map(String::as_str), Some("Receive"), "{s:?}");
    let s = sp().suggest("RECIEVE", 5);
    assert_eq!(s.first().map(String::as_str), Some("RECEIVE"), "{s:?}");
    let s = sp().suggest("london", 5);
    assert_eq!(s.first().map(String::as_str), Some("London"), "{s:?}");
    let s = sp().suggest("dont", 5);
    assert!(s.iter().take(3).any(|w| w == "don't"), "{s:?}");
    let s = sp().suggest("acommodate", 5);
    assert!(s.iter().take(3).any(|w| w == "accommodate"), "{s:?}");
    let s = sp().suggest("thier", 5);
    assert!(s.iter().take(3).any(|w| w == "their"), "{s:?}");
    let s = sp().suggest("alot", 5);
    assert!(s.iter().any(|w| w == "a lot"), "{s:?}");
    let s = sp().suggest("teacher’z", 5);
    assert!(s.iter().all(|w| !w.contains('\'')), "curly apostrophes are kept: {s:?}");
    assert!(sp().suggest("recieve", 2).len() <= 2);
}

#[test]
fn suggestions_are_dictionary_words() {
    for w in ["helo", "wrold", "speling", "grammer", "beautifull", "neccessary"] {
        let s = sp().suggest(w, 5);
        assert!(!s.is_empty(), "{w}");
        for c in &s {
            assert!(c.split(' ').all(|p| sp().is_correct(p)), "{w} -> {c}");
        }
    }
}

#[test]
fn user_dictionary() {
    let mut u = UserDictionary::new();
    assert!(!sp().check_word("rwordish", Some(&u)));
    u.add("rwordish");
    assert!(sp().check_word("rwordish", Some(&u)));
    assert!(sp().check_word("Rwordish", Some(&u)));
    assert!(sp().check_word("RWORDISH", Some(&u)));
    u.ignore_all("Zzyzx");
    assert!(sp().check_word("Zzyzx", Some(&u)));
    let issues = sp().check_text("The rwordish Zzyzx flurbleglop.", Some(&u));
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!((issues[0].start, issues[0].end), (19, 30));

    let dir = std::env::temp_dir().join(format!("rword-userdict-{}", std::process::id()));
    let path = dir.join("custom.dic");
    u.add("Anthropicish");
    u.save(&path).unwrap();
    let loaded = UserDictionary::load(&path).unwrap();
    assert_eq!(loaded.words().collect::<Vec<_>>(), vec!["Anthropicish", "rwordish"]);
    assert!(!loaded.is_ignored("Zzyzx"), "ignore-all is per session");
    assert!(UserDictionary::load(dir.join("missing.dic")).unwrap().is_empty());
    let mut l2 = loaded.clone();
    assert!(l2.remove("rwordish"));
    assert!(!l2.contains("rwordish"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_text_spelling_offsets() {
    // "é" is one char: offsets are char offsets.
    let t = "Café owners recieve teh mail\u{FFFC}definately.";
    let issues = sp().check_text(t, None);
    let spell: Vec<_> = issues.iter().filter(|i| i.kind == IssueKind::Spelling).collect();
    assert_eq!(spell.len(), 3, "{issues:?}");
    assert_eq!((spell[0].start, spell[0].end), (12, 19));
    assert_eq!(spell[0].suggestion.as_deref(), Some("receive"));
    assert_eq!((spell[1].start, spell[1].end), (20, 23));
    assert_eq!(spell[1].suggestion.as_deref(), Some("the"));
    assert_eq!((spell[2].start, spell[2].end), (29, 39));
}

#[test]
fn check_text_skips_urls_and_numbers() {
    let t = "See https://exampel.com/pagez or mail bob@exampel.org about item 42b and the BBC.";
    let issues = sp().check_text(t, None);
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn repeated_words() {
    let t = "This is the the end.";
    let issues = sp().check_text(t, None);
    assert_eq!(issues.len(), 1, "{issues:?}");
    let i = &issues[0];
    assert_eq!(i.kind, IssueKind::RepeatedWord);
    assert_eq!((i.start, i.end), (11, 15)); // " the"
    assert_eq!(i.suggestion.as_deref(), Some(""));
    assert!(sp().check_text("He had had enough.", None).is_empty());
    assert!(sp().check_text("The end. The cat.", None).is_empty());
}

// ---------------------------------------------------------------- grammar

fn grammar(t: &str) -> Vec<(usize, usize, String)> {
    sp().check_text(t, None)
        .into_iter()
        .filter(|i| i.kind == IssueKind::Grammar)
        .map(|i| (i.start, i.end, i.suggestion.unwrap_or_default()))
        .collect()
}

#[test]
fn grammar_double_space() {
    assert_eq!(grammar("Hello  world."), vec![(5, 7, " ".to_string())]);
    assert!(grammar("Hello world.  ").is_empty(), "trailing spaces are fine");
    assert!(grammar("Name:\t\tvalue").is_empty());
}

#[test]
fn grammar_sentence_case() {
    assert_eq!(grammar("It rained. then it stopped."), vec![(11, 15, "Then".to_string())]);
    assert_eq!(grammar("Really? yes."), vec![(8, 11, "Yes".to_string())]);
    assert!(grammar("Ask Mr. smith, e.g. this one.").is_empty());
    assert!(grammar("Wait... what happened?").is_empty());
    assert!(grammar("Buy an iPhone. iPhones are popular.").is_empty());
    assert_eq!(grammar("the cat sat on the mat."), vec![(0, 3, "The".to_string())]);
    assert!(grammar("yes").is_empty(), "short non-prose paragraphs are not checked");
}

#[test]
fn grammar_articles() {
    assert_eq!(grammar("It was a apple."), vec![(7, 8, "an".to_string())]);
    assert_eq!(grammar("It was an banana."), vec![(7, 9, "a".to_string())]);
    assert_eq!(grammar("A elephant came."), vec![(0, 1, "An".to_string())]);
    for ok in [
        "It is a university.",
        "It is an hour.",
        "It is a one-time offer.",
        "She has an MBA.",
        "It was a European car.",
        "It was a useful tool.",
        "It was an honest mistake.",
        "It was an historic day.",
        "It was a historic day.",
        "He got an FBI badge.",
        "It is a NASA project.",
        "Take vitamin A every day.",
        "It was an unimportant detail.",
        "Make a U-turn.",
    ] {
        assert!(grammar(ok).is_empty(), "{ok}: {:?}", grammar(ok));
    }
}

#[test]
fn grammar_lowercase_i() {
    assert_eq!(grammar("Yesterday i went home."), vec![(10, 11, "I".to_string())]);
    assert!(grammar("Item (i) is first.").is_empty());
    assert!(grammar("Use i.e. sparingly.").is_empty());
}

// ---------------------------------------------------------------- autocorrect

fn ac(before: &str) -> Option<(usize, String)> {
    AutoCorrect::default().on_delimiter(before).map(|r| (r.delete_chars, r.insert))
}

#[test]
fn autocorrect_table() {
    assert_eq!(ac("I saw teh"), Some((3, "the".into())));
    assert_eq!(ac("Bread adn"), Some((3, "and".into())));
    assert_eq!(ac("You will recieve"), Some((7, "receive".into())));
    assert_eq!(ac("I dont"), Some((4, "don\u{2019}t".into())));
    assert_eq!(ac("Then im"), Some((2, "I\u{2019}m".into())));
    assert_eq!(ac("So Teh"), Some((3, "The".into())));
    assert_eq!(ac("(teh"), Some((3, "The".into())), "opening bracket before the word");
    assert_eq!(ac("We see"), None);
    assert_eq!(ac("x.teh"), None, "only whole words");
    let mut plain = AutoCorrect::default();
    plain.smart_apostrophes = false;
    assert_eq!(plain.on_delimiter("so dont").unwrap().insert, "don't");
}

#[test]
fn autocorrect_every_typo_key_is_misspelled() {
    let a = AutoCorrect::default();
    assert!(a.len() >= 300, "{}", a.len());
    let real: Vec<&str> = a
        .entries()
        .into_iter()
        .map(|(k, _)| k)
        .filter(|k| k.chars().all(|c| c.is_alphabetic()) && sp().is_correct(k))
        .collect();
    assert!(real.is_empty(), "AutoCorrect keys that are real words: {real:?}");
}

#[test]
fn autocorrect_symbols() {
    assert_eq!(ac("Copyright (c)"), Some((3, "©".into())));
    assert_eq!(ac("Acme(tm)"), Some((4, "™".into())));
    assert_eq!(ac("(r)"), Some((3, "®".into())));
    assert_eq!(ac("pages 1 --"), Some((2, "–".into())));
    assert_eq!(ac("go ->"), Some((2, "→".into())));
    assert_eq!(ac("back <-"), Some((2, "←".into())));
    assert_eq!(ac("Thanks :)"), Some((2, "☺".into())));
    assert_eq!(ac("Wait..."), Some((3, "…".into())));
    assert_eq!(ac("a well--known"), Some((7, "—known".into())));
}

#[test]
fn autocorrect_rules() {
    // Sentence capitalisation.
    assert_eq!(ac("hello"), Some((5, "Hello".into())));
    assert_eq!(ac("It works. now"), Some((3, "Now".into())));
    assert_eq!(ac("It works. \u{201C}now"), Some((3, "Now".into())));
    assert_eq!(ac("See Mr. smith"), None);
    assert_eq!(ac("It works now"), None);
    // Days.
    assert_eq!(ac("See you on monday"), Some((6, "Monday".into())));
    // TWo INitial CApitals.
    assert_eq!(ac("We have THe"), Some((3, "The".into())));
    assert_eq!(ac("Our IDs"), None);
    assert_eq!(ac("The USA"), None);
    // "i" pronoun.
    assert_eq!(ac("Then i"), Some((1, "I".into())));
    let a = AutoCorrect::default();
    assert_eq!(a.on_delimiter_char("Item i", ')'), None);
    // '.' after a single letter at paragraph start must not capitalise ("e.g.").
    assert_eq!(a.on_delimiter_char("e", '.'), None);
    assert_eq!(a.on_delimiter_char("see www", '.'), None);
    assert_eq!(a.on_delimiter_char("mail bob", '@'), None);
    // Combined: typo fix + sentence start.
    assert_eq!(ac("Done. teh"), Some((3, "The".into())));
    // URLs and e-mail addresses are left alone.
    assert_eq!(ac("www.teh.com"), None);
    assert_eq!(ac("teh@example.com"), None);
    // Separators: U+FFFC and tabs.
    assert_eq!(ac("x\u{FFFC}teh"), Some((3, "the".into())));
}

#[test]
fn autocorrect_contract_applied() {
    // Simulate the editor: type "teh cat" char by char.
    let a = AutoCorrect::default();
    let mut para = String::from("Done. ");
    for ch in "teh cat sat".chars().chain(std::iter::once(' ')) {
        if ch == ' ' {
            if let Some(Replacement { delete_chars, insert }) = a.on_delimiter_char(&para, ch) {
                let keep = para.chars().count() - delete_chars;
                para = para.chars().take(keep).collect::<String>() + &insert;
            }
        }
        para.push(ch);
    }
    assert_eq!(para, "Done. The cat sat ");
}

#[test]
fn autocorrect_custom_entries() {
    let mut a = AutoCorrect::empty();
    assert!(a.on_delimiter("so teh").is_none());
    a.add("rwd", "RWord");
    assert_eq!(a.on_delimiter("use rwd").unwrap().insert, "RWord");
    a.add("+-", "±");
    assert_eq!(a.on_delimiter("x +-").unwrap(), Replacement { delete_chars: 2, insert: "±".into() });
    assert!(a.remove("rwd"));
    assert!(a.on_delimiter("use rwd").is_none());
}

#[test]
fn smart_quotes() {
    assert_eq!(smart_quote(None, '"'), '“');
    assert_eq!(smart_quote(Some(' '), '"'), '“');
    assert_eq!(smart_quote(Some('('), '"'), '“');
    assert_eq!(smart_quote(Some('d'), '"'), '”');
    assert_eq!(smart_quote(Some('.'), '"'), '”');
    assert_eq!(smart_quote(None, '\''), '‘');
    assert_eq!(smart_quote(Some('n'), '\''), '’'); // don't
    assert_eq!(smart_quote(Some('“'), '\''), '‘');
    assert_eq!(smart_quote(Some('\u{FFFC}'), '"'), '“');
    assert_eq!(smart_quote(Some('a'), 'x'), 'x');
    assert_eq!(AutoCorrect::smart_quote(Some('s'), '\''), '’');
}

// ---------------------------------------------------------------- thesaurus

#[test]
fn thesaurus_lookup() {
    let m = thesaurus::lookup("happy");
    assert!(!m.is_empty());
    assert_eq!(m[0].part_of_speech, "adjective");
    let syn = thesaurus::synonyms("happy");
    assert!(syn.iter().any(|s| s == "glad"), "{syn:?}");
    assert!(!syn.iter().any(|s| s == "unhappy"), "antonyms are removed");
    assert!(m.iter().all(|m| m.synonyms.len() <= 12));
    assert!(!thesaurus::synonyms("Happy").is_empty());
    assert!(!thesaurus::synonyms("houses").is_empty(), "plural falls back to singular");
    assert!(thesaurus::lookup("flurbleglop").is_empty());
    assert!(thesaurus::entry_count() > 20_000);
    // Every synonym of a sample of words is a correctly spelled word or phrase.
    for w in ["big", "run", "fast", "house", "good", "write", "quick"] {
        for s in thesaurus::synonyms(w) {
            assert!(s.split([' ', '-']).all(|p| sp().is_correct(p)), "{w} -> {s}");
        }
    }
}

// ---------------------------------------------------------------- stats

#[test]
fn word_counts() {
    assert_eq!(count_words(""), 0);
    assert_eq!(count_words("Hello, world!"), 2);
    assert_eq!(count_words("  well-known   don't\tstop\u{FFFC}now "), 4);
    assert_eq!(count_words("a – b"), 3);
    assert_eq!(count_words("3.14 is pi"), 3);
}

#[test]
fn doc_stats() {
    let mut d = Document::from_text("Hello world. This is RWord!\n\nSecond paragraph here");
    let mut t = Table::new(1, 2, 400.0);
    t.rows[0] = Row { cells: vec![Cell::with_text("cell one"), Cell::with_text("")], height: None, header: false };
    d.body.push(Block::Table(t));
    let id = d.next_note_id();
    d.footnotes.push(Note { id, paragraphs: vec![Paragraph::with_text("A footnote.")] });
    let s = DocStats::compute(&d, false);
    assert_eq!(s.words, 5 + 3 + 2);
    assert_eq!(s.paragraphs, 3);
    assert_eq!(s.sentences, 2 + 1 + 1);
    assert_eq!(s.characters_with_spaces, 27 + 21 + 8);
    assert_eq!(s.characters_no_spaces, 23 + 19 + 7);
    let s2 = DocStats::compute(&d, true);
    assert_eq!(s2.words, s.words + 2);
    assert_eq!(s2.paragraphs, 4);
}

#[test]
fn sentence_counting() {
    use wordcore::proofing::stats::count_sentences;
    assert_eq!(count_sentences("One. Two! Three?"), 3);
    assert_eq!(count_sentences("Mr. Smith paid 3.50 dollars. Done"), 2);
    assert_eq!(count_sentences("Wait... what?"), 2);
    assert_eq!(count_sentences("Apples, pears, etc. are fruit."), 1);
    assert_eq!(count_sentences(""), 0);
}

#[test]
fn syllable_estimates() {
    for (w, n) in [
        ("the", 1),
        ("cat", 1),
        ("happy", 2),
        ("table", 2),
        ("make", 1),
        ("walked", 1),
        ("wanted", 2),
        ("boxes", 2),
        ("beautiful", 3),
        ("readability", 5),
        ("radio", 3),
        ("nation", 2),
        ("university", 5),
        ("well-known", 2),
    ] {
        assert_eq!(syllables(w), n, "{w}");
    }
}

#[test]
fn readability_scores() {
    let easy = Document::from_text("The cat sat on the mat. The dog ran to the cat. It was fun.");
    let r = readability(&easy);
    assert!(r.flesch_reading_ease > 90.0, "{r:?}");
    assert!(r.flesch_kincaid_grade < 3.0, "{r:?}");
    assert!((r.words_per_sentence - 5.0).abs() < 1e-9);
    let hard = Document::from_text(
        "Notwithstanding considerable institutional reluctance, the administration implemented comprehensive organizational restructuring initiatives.",
    );
    let h = readability(&hard);
    assert!(h.flesch_reading_ease < 20.0, "{h:?}");
    assert!(h.flesch_kincaid_grade > 15.0, "{h:?}");
    assert!(h.chars_per_word > r.chars_per_word);
    assert_eq!(readability(&Document::new()), Default::default());
}

// ---------------------------------------------------------------- timing

/// Run with `cargo test --release -p wordcore --test proofing -- --ignored --nocapture timing`.
#[test]
#[ignore = "timing report; run in release mode"]
fn timing() {
    use std::time::Instant;
    let t = Instant::now();
    let s = Speller::english();
    println!("speller build: {:?} ({} words)", t.elapsed(), s.len());
    let words = [
        "recieve",
        "definately",
        "teh",
        "acommodate",
        "Wensday",
        "beautifull",
        "proffesional",
        "xylophonne",
        "a",
        "zzzzzzzzzzzzzzzz",
        "responsibilites",
        "internationalisaton",
    ];
    let mut worst = (std::time::Duration::ZERO, "");
    let t = Instant::now();
    for w in words {
        let t1 = Instant::now();
        let _ = s.suggest(w, 5);
        worst = worst.max((t1.elapsed(), w));
    }
    println!("suggest: avg {:?}, worst {:?}", t.elapsed() / words.len() as u32, worst);
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(200) + "Thier recieve teh definately.";
    let t = Instant::now();
    let issues = s.check_text(&text, None);
    println!("check_text 1800 words: {:?} ({} issues)", t.elapsed(), issues.len());
    let t = Instant::now();
    let n = thesaurus::synonyms("happy").len();
    println!("thesaurus first lookup: {:?} ({n} synonyms)", t.elapsed());
    let t = Instant::now();
    let _ = thesaurus::synonyms("fast");
    println!("thesaurus lookup: {:?}", t.elapsed());
}
