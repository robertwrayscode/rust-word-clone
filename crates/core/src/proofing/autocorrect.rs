//! AutoCorrect: fix common typos and apply Word's capitalisation rules as
//! the user types, plus smart (curly) quotes.
//!
//! # Editor contract
//!
//! Call [`AutoCorrect::on_delimiter`] (or [`AutoCorrect::on_delimiter_char`])
//! when the user types a word delimiter — a space, punctuation, or Enter —
//! **before** inserting that delimiter:
//!
//! 1. `before` is the paragraph's text from its start up to the caret,
//!    *not* including the delimiter just typed. Non-text inlines may appear
//!    as U+FFFC or control characters; they act as separators.
//! 2. If `Some(Replacement { delete_chars, insert })` is returned, delete the
//!    `delete_chars` chars immediately before the caret (they are always the
//!    last `delete_chars` chars of `before`), then insert `insert` at the
//!    caret.
//! 3. Then insert the delimiter as usual.
//!
//! ```
//! use wordcore::proofing::{AutoCorrect, Replacement};
//! let ac = AutoCorrect::default();
//! // The user typed "I saw teh" and then pressed space:
//! let r = ac.on_delimiter("I saw teh").unwrap();
//! assert_eq!(r, Replacement { delete_chars: 3, insert: "the".into() });
//! // Paragraph text becomes "I saw " + "the" + " ".
//! ```
//!
//! Undo of an AutoCorrect replacement is the app's business (Word puts the
//! replacement in its own undo step).

use std::collections::HashMap;

use super::text::{
    apply_case, capitalize, case_pattern, is_apostrophe, is_separator, is_url_like, sentence_start, CasePattern,
};

/// What to change before inserting the delimiter. See the module docs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replacement {
    /// Number of chars to delete immediately before the caret.
    pub delete_chars: usize,
    /// Text to insert at the caret after deleting.
    pub insert: String,
}

/// AutoCorrect settings and replacement list.
#[derive(Clone, Debug)]
pub struct AutoCorrect {
    /// Replace text as you type (the typo / symbol table).
    pub replace_text: bool,
    /// Capitalise the first letter of sentences.
    pub capitalize_sentences: bool,
    /// Capitalise names of days.
    pub capitalize_days: bool,
    /// Correct TWo INitial CApitals.
    pub fix_two_initial_caps: bool,
    /// Capitalise the pronoun "i".
    pub capitalize_i: bool,
    /// Insert apostrophes in replacements as ’ (U+2019) rather than '.
    pub smart_apostrophes: bool,
    /// Word entries: lower-case key -> replacement.
    words: HashMap<String, String>,
    /// Symbol entries matched against the whole trailing token, exact case.
    symbols: Vec<(String, String)>,
}

impl Default for AutoCorrect {
    fn default() -> Self {
        let mut ac = AutoCorrect {
            replace_text: true,
            capitalize_sentences: true,
            capitalize_days: true,
            fix_two_initial_caps: true,
            capitalize_i: true,
            smart_apostrophes: true,
            words: HashMap::new(),
            symbols: Vec::new(),
        };
        for (from, to) in SYMBOLS.iter().chain(TYPOS) {
            ac.add(from, to);
        }
        ac
    }
}

/// Symbols that are also replaced at the end of a longer token ("Acme(tm)").
const SUFFIX_SYMBOLS: &[&str] = &["(c)", "(r)", "(tm)", "(TM)", "..."];

const DAYS: &[&str] = &["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];

impl AutoCorrect {
    /// An AutoCorrect with no replacement entries (rules only).
    pub fn empty() -> Self {
        AutoCorrect { words: HashMap::new(), symbols: Vec::new(), ..AutoCorrect::default() }
    }

    /// Add (or replace) an entry. Entries made only of letters, digits and
    /// apostrophes match whole words case-insensitively (keeping the typed
    /// capitalisation); anything else matches the trailing token exactly.
    pub fn add(&mut self, from: &str, to: &str) {
        if is_wordlike(from) {
            self.words.insert(norm_key(from), to.to_string());
        } else {
            self.symbols.retain(|(k, _)| k != from);
            self.symbols.push((from.to_string(), to.to_string()));
            // Longest first so "-->" wins over "->".
            self.symbols.sort_by_key(|(k, _)| std::cmp::Reverse(k.chars().count()));
        }
    }

    /// Remove an entry. Returns true if it existed.
    pub fn remove(&mut self, from: &str) -> bool {
        if is_wordlike(from) {
            self.words.remove(&norm_key(from)).is_some()
        } else {
            let n = self.symbols.len();
            self.symbols.retain(|(k, _)| k != from);
            n != self.symbols.len()
        }
    }

    /// All entries (word entries have lower-case keys), sorted by key.
    pub fn entries(&self) -> Vec<(&str, &str)> {
        let mut v: Vec<(&str, &str)> = self
            .words
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .chain(self.symbols.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .collect();
        v.sort();
        v
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.words.len() + self.symbols.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Look up a single word in the replacement table, applying the word's
    /// capitalisation ("Teh" -> "The").
    pub fn lookup(&self, word: &str) -> Option<String> {
        let r = self.words.get(&norm_key(word))?;
        Some(self.apostrophes(apply_case(word, r)))
    }

    fn apostrophes(&self, s: String) -> String {
        if self.smart_apostrophes {
            s.replace('\'', "\u{2019}")
        } else {
            s
        }
    }

    /// Same as [`AutoCorrect::on_delimiter_char`] with a space delimiter.
    pub fn on_delimiter(&self, before: &str) -> Option<Replacement> {
        self.on_delimiter_char(before, ' ')
    }

    /// Called when `delimiter` has just been typed (use `'\n'` for Enter);
    /// `before` is the paragraph text up to the caret *excluding* the
    /// delimiter. See the module docs for the contract.
    ///
    /// Knowing the delimiter avoids some mistakes: after a '.', '@', '/', ':'
    /// or '-' sentence capitalisation is not applied ("e.g.", "www.") and
    /// "i" followed by '.' or ')' is left alone (list numbering).
    pub fn on_delimiter_char(&self, before: &str, delimiter: char) -> Option<Replacement> {
        let chars: Vec<char> = before.chars().collect();
        let n = chars.len();
        let mut ts = n;
        while ts > 0 && !is_separator(chars[ts - 1]) {
            ts -= 1;
        }
        if ts == n {
            return None;
        }
        let token: String = chars[ts..].iter().collect();
        if is_url_like(&token) || token.contains('@') || matches!(delimiter, '@' | '/' | ':') {
            return None;
        }

        if self.replace_text {
            for (k, v) in &self.symbols {
                if token == *k || (SUFFIX_SYMBOLS.contains(&k.as_str()) && token.ends_with(k.as_str())) {
                    return Some(Replacement { delete_chars: k.chars().count(), insert: v.clone() });
                }
            }
            // "well--known" -> em dash between words.
            if let Some(p) = token.find("--") {
                let (l, r) = (&token[..p], &token[p + 2..]);
                if l.chars().last().is_some_and(|c| c.is_alphanumeric())
                    && r.chars().next().is_some_and(|c| c.is_alphanumeric())
                    && !r.contains("--")
                {
                    let tail = token[p..].chars().count();
                    return Some(Replacement { delete_chars: tail, insert: format!("\u{2014}{r}") });
                }
            }
        }

        // The word at the end of the token.
        let mut ws = n;
        while ws > ts && (chars[ws - 1].is_alphanumeric() || is_apostrophe(chars[ws - 1])) {
            ws -= 1;
        }
        while ws < n && is_apostrophe(chars[ws]) {
            ws += 1;
        }
        if ws == n {
            return None;
        }
        // Only opening punctuation may precede the word inside the token.
        if !chars[ts..ws].iter().all(|c| matches!(c, '(' | '[' | '{' | '"' | '\'' | '\u{201C}' | '\u{2018}')) {
            return None;
        }
        let word: String = chars[ws..].iter().collect();
        let mut new = word.clone();

        let mut replaced = false;
        if self.replace_text {
            if let Some(r) = self.lookup(&word) {
                new = r;
                replaced = true;
            }
        }
        if self.fix_two_initial_caps && !replaced {
            new = fix_two_caps(&new);
        }
        if self.capitalize_days && DAYS.contains(&new.as_str()) {
            new = capitalize(&new);
        }
        if self.capitalize_i
            && new == "i"
            && !matches!(delimiter, '.' | ')' | ']' | '-')
            && !matches!(chars[ts..ws].last(), Some('(' | '['))
        {
            new = "I".into();
        }
        if self.capitalize_sentences
            && !matches!(delimiter, '.' | '@' | '/' | ':' | '-' | '\'' | '\u{2019}')
            && case_pattern(&new) == CasePattern::Lower
            && new.chars().next().is_some_and(|c| c.is_lowercase())
            && sentence_start(&chars, ws).unwrap_or(true)
        {
            new = capitalize(&new);
        }

        (new != word).then(|| Replacement { delete_chars: word.chars().count(), insert: new })
    }

    /// See [`smart_quote`].
    pub fn smart_quote(prev_char: Option<char>, quote: char) -> char {
        smart_quote(prev_char, quote)
    }
}

/// Convert a typed straight quote (`"` or `'`) to the right curly quote,
/// given the char just before the caret (`None` at the start of a
/// paragraph). An opening quote is used at the start, after whitespace or
/// separators, after opening brackets and dashes, and after another opening
/// quote; otherwise a closing quote (which is also the apostrophe, ’).
/// Other characters are returned unchanged.
pub fn smart_quote(prev_char: Option<char>, quote: char) -> char {
    let opening = match prev_char {
        None => true,
        Some(c) => {
            is_separator(c)
                || matches!(c, '(' | '[' | '{' | '<' | '\u{2013}' | '\u{2014}' | '/' | '\u{201C}' | '\u{2018}' | '"')
                || (c == '\'' && quote == '"')
        }
    };
    match (quote, opening) {
        ('"' | '\u{201C}' | '\u{201D}', true) => '\u{201C}',
        ('"' | '\u{201C}' | '\u{201D}', false) => '\u{201D}',
        ('\'' | '\u{2018}' | '\u{2019}', true) => '\u{2018}',
        ('\'' | '\u{2018}' | '\u{2019}', false) => '\u{2019}',
        (c, _) => c,
    }
}

fn is_wordlike(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || is_apostrophe(c))
}

fn norm_key(s: &str) -> String {
    s.chars().map(|c| if is_apostrophe(c) { '\'' } else { c }).collect::<String>().to_lowercase()
}

/// "THe" -> "The", "MOnday" -> "Monday"; leaves "IDs", "PCs", "USA" alone.
fn fix_two_caps(w: &str) -> String {
    let c: Vec<char> = w.chars().collect();
    if c.len() >= 3
        && c.iter().all(|c| c.is_alphabetic())
        && c[0].is_uppercase()
        && c[1].is_uppercase()
        && c[2..].iter().all(|c| c.is_lowercase())
        && !(c.len() == 3 && c[2] == 's')
    {
        let mut s = String::new();
        s.push(c[0]);
        s.extend(c[1].to_lowercase());
        s.extend(&c[2..]);
        return s;
    }
    w.to_string()
}

/// Symbol replacements (matched against the whole trailing token).
const SYMBOLS: &[(&str, &str)] = &[
    ("(c)", "\u{00A9}"),
    ("(C)", "\u{00A9}"),
    ("(r)", "\u{00AE}"),
    ("(R)", "\u{00AE}"),
    ("(tm)", "\u{2122}"),
    ("(TM)", "\u{2122}"),
    ("(e)", "\u{20AC}"),
    ("--", "\u{2013}"),
    ("->", "\u{2192}"),
    ("<-", "\u{2190}"),
    ("-->", "\u{2192}"),
    ("<--", "\u{2190}"),
    ("=>", "\u{21D2}"),
    ("==>", "\u{21D2}"),
    ("<==", "\u{21D0}"),
    ("<=>", "\u{21D4}"),
    ("<->", "\u{2194}"),
    (":)", "\u{263A}"),
    (":-)", "\u{263A}"),
    (":(", "\u{2639}"),
    (":-(", "\u{2639}"),
    (":|", "\u{1F610}"),
    ("...", "\u{2026}"),
    ("1/2", "\u{00BD}"),
    ("1/4", "\u{00BC}"),
    ("3/4", "\u{00BE}"),
];

/// Hand-curated common typos (lower-case key -> fix). Keys are never real
/// words, so replacing them is always safe.
const TYPOS: &[(&str, &str)] = &[
    // contractions typed without the apostrophe
    ("dont", "don't"),
    ("doesnt", "doesn't"),
    ("didnt", "didn't"),
    ("isnt", "isn't"),
    ("wasnt", "wasn't"),
    ("werent", "weren't"),
    ("arent", "aren't"),
    ("havent", "haven't"),
    ("hasnt", "hasn't"),
    ("hadnt", "hadn't"),
    ("couldnt", "couldn't"),
    ("shouldnt", "shouldn't"),
    ("wouldnt", "wouldn't"),
    ("mustnt", "mustn't"),
    ("neednt", "needn't"),
    ("im", "I'm"),
    ("ive", "I've"),
    ("youre", "you're"),
    ("youve", "you've"),
    ("youll", "you'll"),
    ("theyre", "they're"),
    ("theyve", "they've"),
    ("theyll", "they'll"),
    ("weve", "we've"),
    ("thats", "that's"),
    ("theres", "there's"),
    ("whos", "who's"),
    ("shouldve", "should've"),
    ("couldve", "could've"),
    ("wouldve", "would've"),
    ("doesn't't", "doesn't"),
    ("itd", "it'd"),
    ("itll", "it'll"),
    ("thatll", "that'll"),
    ("whatll", "what'll"),
    // transpositions and slips
    ("teh", "the"),
    ("hte", "the"),
    ("tghe", "the"),
    ("tthe", "the"),
    ("thge", "the"),
    ("adn", "and"),
    ("nad", "and"),
    ("andd", "and"),
    ("taht", "that"),
    ("tath", "that"),
    ("thta", "that"),
    ("thier", "their"),
    ("theri", "their"),
    ("wich", "which"),
    ("whcih", "which"),
    ("wihch", "which"),
    ("whihc", "which"),
    ("waht", "what"),
    ("whta", "what"),
    ("wiht", "with"),
    ("wtih", "with"),
    ("whith", "with"),
    ("thsi", "this"),
    ("tihs", "this"),
    ("htis", "this"),
    ("jsut", "just"),
    ("konw", "know"),
    ("knwo", "know"),
    ("liek", "like"),
    ("mroe", "more"),
    ("smae", "same"),
    ("soem", "some"),
    ("somthing", "something"),
    ("somethign", "something"),
    ("tiem", "time"),
    ("yuo", "you"),
    ("yoru", "your"),
    ("yuor", "your"),
    ("woudl", "would"),
    ("coudl", "could"),
    ("shoudl", "should"),
    ("cna", "can"),
    ("nto", "not"),
    ("ahve", "have"),
    ("hvae", "have"),
    ("haev", "have"),
    ("beign", "being"),
    ("bakc", "back"),
    ("peopel", "people"),
    ("poeple", "people"),
    ("pepole", "people"),
    ("probelm", "problem"),
    ("rigth", "right"),
    ("wroking", "working"),
    ("owrk", "work"),
    ("lookign", "looking"),
    ("makeing", "making"),
    ("comming", "coming"),
    ("comign", "coming"),
    ("goign", "going"),
    ("doign", "doing"),
    ("thnig", "thing"),
    ("thign", "thing"),
    ("thigns", "things"),
    ("abotu", "about"),
    ("abuot", "about"),
    ("agian", "again"),
    ("alwasy", "always"),
    ("alwyas", "always"),
    ("becuase", "because"),
    ("becasue", "because"),
    ("beacuse", "because"),
    ("becuse", "because"),
    ("befroe", "before"),
    ("diferent", "different"),
    ("diffrent", "different"),
    ("eveyr", "every"),
    ("frmo", "from"),
    ("fomr", "from"),
    ("freind", "friend"),
    ("freinds", "friends"),
    ("gerat", "great"),
    ("hwo", "how"),
    ("littel", "little"),
    ("mkae", "make"),
    ("myslef", "myself"),
    ("mysefl", "myself"),
    ("ohter", "other"),
    ("otehr", "other"),
    ("realy", "really"),
    ("reallly", "really"),
    ("sicne", "since"),
    ("thsoe", "those"),
    ("tihng", "thing"),
    ("togehter", "together"),
    ("tomorow", "tomorrow"),
    ("tommorow", "tomorrow"),
    ("tommorrow", "tomorrow"),
    ("wnat", "want"),
    ("wnats", "wants"),
    ("yera", "year"),
    ("yeras", "years"),
    ("youself", "yourself"),
    // run-together phrases
    ("alot", "a lot"),
    ("aswell", "as well"),
    ("infact", "in fact"),
    ("eachother", "each other"),
    ("incase", "in case"),
    ("atleast", "at least"),
    ("inspite", "in spite"),
    ("noone", "no one"),
    ("thankyou", "thank you"),
    ("alittle", "a little"),
    ("ofcourse", "of course"),
    // classic misspellings
    ("recieve", "receive"),
    ("recieved", "received"),
    ("recieves", "receives"),
    ("recieving", "receiving"),
    ("reciept", "receipt"),
    ("beleive", "believe"),
    ("belive", "believe"),
    ("beleived", "believed"),
    ("beleif", "belief"),
    ("definately", "definitely"),
    ("definatly", "definitely"),
    ("defintely", "definitely"),
    ("deffinitely", "definitely"),
    ("seperate", "separate"),
    ("seperately", "separately"),
    ("seperated", "separated"),
    ("occured", "occurred"),
    ("occurence", "occurrence"),
    ("occurance", "occurrence"),
    ("occuring", "occurring"),
    ("untill", "until"),
    ("accomodate", "accommodate"),
    ("accomodation", "accommodation"),
    ("acomodate", "accommodate"),
    ("acheive", "achieve"),
    ("acheived", "achieved"),
    ("acheivement", "achievement"),
    ("adress", "address"),
    ("arguement", "argument"),
    ("basicly", "basically"),
    ("basicaly", "basically"),
    ("begining", "beginning"),
    ("beggining", "beginning"),
    ("buisness", "business"),
    ("bussiness", "business"),
    ("calender", "calendar"),
    ("cemetary", "cemetery"),
    ("collegue", "colleague"),
    ("collegues", "colleagues"),
    ("commitee", "committee"),
    ("comittee", "committee"),
    ("completly", "completely"),
    ("concious", "conscious"),
    ("curiousity", "curiosity"),
    ("decieve", "deceive"),
    ("desparate", "desperate"),
    ("develope", "develop"),
    ("dilemna", "dilemma"),
    ("dissapear", "disappear"),
    ("dissapoint", "disappoint"),
    ("disapoint", "disappoint"),
    ("dissapointed", "disappointed"),
    ("embarass", "embarrass"),
    ("embarassed", "embarrassed"),
    ("enviroment", "environment"),
    ("existance", "existence"),
    ("experiance", "experience"),
    ("familar", "familiar"),
    ("finaly", "finally"),
    ("foriegn", "foreign"),
    ("foward", "forward"),
    ("goverment", "government"),
    ("gaurd", "guard"),
    ("happend", "happened"),
    ("harrass", "harass"),
    ("heirarchy", "hierarchy"),
    ("humourous", "humorous"),
    ("ignorence", "ignorance"),
    ("immediatly", "immediately"),
    ("imediately", "immediately"),
    ("independant", "independent"),
    ("independance", "independence"),
    ("interupt", "interrupt"),
    ("irrelevent", "irrelevant"),
    ("knowlege", "knowledge"),
    ("liason", "liaison"),
    ("libary", "library"),
    ("lisence", "license"),
    ("maintainance", "maintenance"),
    ("mantainance", "maintenance"),
    ("millenium", "millennium"),
    ("mispell", "misspell"),
    ("mispelled", "misspelled"),
    ("neccessary", "necessary"),
    ("necesary", "necessary"),
    ("neccesary", "necessary"),
    ("noticable", "noticeable"),
    ("ocasion", "occasion"),
    ("occassion", "occasion"),
    ("persue", "pursue"),
    ("posession", "possession"),
    ("prefered", "preferred"),
    ("presense", "presence"),
    ("prominant", "prominent"),
    ("publically", "publicly"),
    ("recomend", "recommend"),
    ("reccomend", "recommend"),
    ("reccommend", "recommend"),
    ("refered", "referred"),
    ("refering", "referring"),
    ("relevent", "relevant"),
    ("religous", "religious"),
    ("remeber", "remember"),
    ("resistence", "resistance"),
    ("responsability", "responsibility"),
    ("rythm", "rhythm"),
    ("sieze", "seize"),
    ("similiar", "similar"),
    ("simular", "similar"),
    ("sincerly", "sincerely"),
    ("speach", "speech"),
    ("succesful", "successful"),
    ("successfull", "successful"),
    ("sucessful", "successful"),
    ("succesfully", "successfully"),
    ("supercede", "supersede"),
    ("suprise", "surprise"),
    ("suprised", "surprised"),
    ("tendancy", "tendency"),
    ("threshhold", "threshold"),
    ("tounge", "tongue"),
    ("truely", "truly"),
    ("unfortunatly", "unfortunately"),
    ("unfortunatley", "unfortunately"),
    ("usefull", "useful"),
    ("vaccum", "vacuum"),
    ("vegtable", "vegetable"),
    ("wierd", "weird"),
    ("writting", "writing"),
    ("yeild", "yield"),
    ("abscence", "absence"),
    ("accross", "across"),
    ("agression", "aggression"),
    ("agressive", "aggressive"),
    ("allready", "already"),
    ("amatuer", "amateur"),
    ("apparant", "apparent"),
    ("apparantly", "apparently"),
    ("aquire", "acquire"),
    ("athiest", "atheist"),
    ("awfull", "awful"),
    ("beautifull", "beautiful"),
    ("bizzare", "bizarre"),
    ("brillant", "brilliant"),
    ("catagory", "category"),
    ("cheif", "chief"),
    ("cieling", "ceiling"),
    ("concensus", "consensus"),
    ("critisism", "criticism"),
    ("desicion", "decision"),
    ("ecstacy", "ecstasy"),
    ("eigth", "eighth"),
    ("equiptment", "equipment"),
    ("excercise", "exercise"),
    ("facinating", "fascinating"),
    ("flourescent", "fluorescent"),
    ("fourty", "forty"),
    ("gratefull", "grateful"),
    ("guage", "gauge"),
    ("hieght", "height"),
    ("hygene", "hygiene"),
    ("hipocrite", "hypocrite"),
    ("idiosyncracy", "idiosyncrasy"),
    ("inaccessable", "inaccessible"),
    ("incidently", "incidentally"),
    ("accidently", "accidentally"),
    ("innoculate", "inoculate"),
    ("inteligence", "intelligence"),
    ("lenght", "length"),
    ("mideval", "medieval"),
    ("miniscule", "minuscule"),
    ("mischevious", "mischievous"),
    ("neice", "niece"),
    ("oppurtunity", "opportunity"),
    ("oppertunity", "opportunity"),
    ("orignal", "original"),
    ("orginal", "original"),
    ("paralell", "parallel"),
    ("parralel", "parallel"),
    ("pasttime", "pastime"),
    ("peice", "piece"),
    ("percieve", "perceive"),
    ("perseverence", "perseverance"),
    ("privelege", "privilege"),
    ("priviledge", "privilege"),
    ("pronounciation", "pronunciation"),
    ("propoganda", "propaganda"),
    ("questionaire", "questionnaire"),
    ("rediculous", "ridiculous"),
    ("refrence", "reference"),
    ("restaraunt", "restaurant"),
    ("resturant", "restaurant"),
    ("secratary", "secretary"),
    ("sentance", "sentence"),
    ("speciman", "specimen"),
    ("strenght", "strength"),
    ("sucess", "success"),
    ("tatoo", "tattoo"),
    ("tecnology", "technology"),
    ("temperture", "temperature"),
    ("twelth", "twelfth"),
    ("tyrany", "tyranny"),
    ("unforseen", "unforeseen"),
    ("unneccessary", "unnecessary"),
    ("unecessary", "unnecessary"),
    ("vehical", "vehicle"),
    ("visable", "visible"),
    ("whereever", "wherever"),
    ("withold", "withhold"),
    ("wonderfull", "wonderful"),
    ("yatch", "yacht"),
    ("alledged", "alleged"),
    ("anually", "annually"),
    ("carefull", "careful"),
    ("charachter", "character"),
    ("commited", "committed"),
    ("comparision", "comparison"),
    ("concieve", "conceive"),
    ("convinient", "convenient"),
    ("disatisfied", "dissatisfied"),
    ("exagerate", "exaggerate"),
    ("exellent", "excellent"),
    ("existant", "existent"),
    ("explaination", "explanation"),
    ("familliar", "familiar"),
    ("febuary", "February"),
    ("gaurantee", "guarantee"),
    ("garantee", "guarantee"),
    ("geneology", "genealogy"),
    ("grammer", "grammar"),
    ("intresting", "interesting"),
    ("labratory", "laboratory"),
    ("liesure", "leisure"),
    ("mathmatics", "mathematics"),
    ("naturaly", "naturally"),
    ("ninty", "ninety"),
    ("offical", "official"),
    ("parliment", "parliament"),
    ("pavillion", "pavilion"),
    ("phenomenom", "phenomenon"),
    ("posible", "possible"),
    ("preceeding", "preceding"),
    ("probaly", "probably"),
    ("proffesional", "professional"),
    ("proffesor", "professor"),
    ("pursuade", "persuade"),
    ("reconize", "recognize"),
    ("rythym", "rhythm"),
    ("saftey", "safety"),
    ("scedule", "schedule"),
    ("shedule", "schedule"),
    ("seige", "siege"),
    ("stoped", "stopped"),
    ("studing", "studying"),
    ("succeded", "succeeded"),
    ("supose", "suppose"),
    ("suposed", "supposed"),
    ("tenative", "tentative"),
    ("thorougly", "thoroughly"),
    ("tradgedy", "tragedy"),
    ("wednseday", "Wednesday"),
    ("wensday", "Wednesday"),
    ("tuseday", "Tuesday"),
    ("thrusday", "Thursday"),
    ("acknowlege", "acknowledge"),
    ("adresses", "addresses"),
    ("advertisment", "advertisement"),
    ("annoucement", "announcement"),
    ("applicaiton", "application"),
    ("arrangment", "arrangement"),
    ("attatch", "attach"),
    ("attatched", "attached"),
    ("availible", "available"),
    ("avaliable", "available"),
    ("comission", "commission"),
    ("comunicate", "communicate"),
    ("comunication", "communication"),
    ("conveniant", "convenient"),
    ("corperation", "corporation"),
    ("dependancy", "dependency"),
    ("dicussion", "discussion"),
    ("dissappear", "disappear"),
    ("enterance", "entrance"),
    ("enviromental", "environmental"),
    ("finacial", "financial"),
    ("goverments", "governments"),
    ("immediatley", "immediately"),
    ("informaiton", "information"),
    ("infromation", "information"),
    ("managment", "management"),
    ("medecine", "medicine"),
    ("mesage", "message"),
    ("messsage", "message"),
    ("occassionally", "occasionally"),
    ("occurr", "occur"),
    ("oppositon", "opposition"),
    ("paticular", "particular"),
    ("particulary", "particularly"),
    ("performence", "performance"),
    ("permanant", "permanent"),
    ("posibility", "possibility"),
    ("prefrence", "preference"),
    ("recomendation", "recommendation"),
    ("reponse", "response"),
    ("responce", "response"),
    ("shcool", "school"),
    ("stregth", "strength"),
    ("sucessfully", "successfully"),
    ("thoguht", "thought"),
    ("throught", "through"),
    ("tomatos", "tomatoes"),
    ("potatos", "potatoes"),
    ("trully", "truly"),
    ("useing", "using"),
    ("usualy", "usually"),
    ("wheather", "weather"),
    ("writen", "written"),
];
