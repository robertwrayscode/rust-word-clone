//! Proofing glue between the editor and `wordcore::proofing`: spelling squiggles,
//! suggestions, AutoCorrect, the Spelling and Thesaurus panes, and word counts.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use egui::{Rect, RichText, Ui};
use wordcore::edit::Pos;
use wordcore::proofing::{AutoCorrect, DocStats, Issue, IssueKind, Speller, UserDictionary};
use wordcore::{Document, Story};

use crate::app::{App, EditKind};
use crate::canvas::Geometry;

static AUTOCORRECT: AtomicBool = AtomicBool::new(true);
pub static CHECK_SPELLING: AtomicBool = AtomicBool::new(true);

pub fn autocorrect_enabled() -> bool {
    AUTOCORRECT.load(Ordering::Relaxed)
}

pub fn set_autocorrect(on: bool) {
    AUTOCORRECT.store(on, Ordering::Relaxed);
}

fn autocorrector() -> &'static AutoCorrect {
    static AC: OnceLock<AutoCorrect> = OnceLock::new();
    AC.get_or_init(AutoCorrect::default)
}

fn user_dict() -> &'static Mutex<UserDictionary> {
    static UD: OnceLock<Mutex<UserDictionary>> = OnceLock::new();
    UD.get_or_init(|| {
        let d = crate::fileio::settings_path("custom.dic").and_then(|p| UserDictionary::load(p).ok()).unwrap_or_default();
        Mutex::new(d)
    })
}

fn save_user_dict() {
    if let (Some(p), Ok(d)) = (crate::fileio::settings_path("custom.dic"), user_dict().lock()) {
        let _ = d.save(p);
    }
}

thread_local! {
    /// Issues per paragraph text, so checking is done once per change, not per frame.
    static CACHE: RefCell<(u64, HashMap<String, Vec<Issue>>)> = RefCell::new((0, HashMap::new()));
}

fn dict_generation() -> u64 {
    user_dict().lock().map(|d| d.len() as u64).unwrap_or(0)
}

/// Spelling and grammar issues in a paragraph's text (cached).
pub fn issues_for(text: &str) -> Vec<Issue> {
    let gen = dict_generation();
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.0 != gen || c.1.len() > 5000 {
            c.1.clear();
            c.0 = gen;
        }
        if let Some(v) = c.1.get(text) {
            return v.clone();
        }
        let d = user_dict().lock().ok();
        let v = Speller::english().check_text(text, d.as_deref());
        c.1.insert(text.to_string(), v.clone());
        v
    })
}

fn clear_cache() {
    CACHE.with(|c| c.borrow_mut().1.clear());
}

pub fn smart_quote(prev: Option<char>, q: char) -> char {
    wordcore::proofing::autocorrect::smart_quote(prev, q)
}

/// AutoCorrect when `delim` is typed at `pos`: returns (chars to delete before pos, replacement).
pub fn autocorrect(doc: &Document, pos: Pos, delim: char) -> Option<(usize, String)> {
    let p = doc.para(&pos.path)?;
    let chars = p.chars_vec();
    let before: String = chars[..pos.offset.min(chars.len())].iter().collect();
    let r = autocorrector().on_delimiter_char(&before, delim)?;
    if r.delete_chars > pos.offset {
        return None;
    }
    Some((r.delete_chars, r.insert))
}

/// Misspelled word underlines visible on screen: (page, x0, x1, baseline) in page points.
pub fn visible_issues(app: &App, geo: &Geometry, clip: Rect) -> Vec<(usize, f32, f32, f32)> {
    let mut out = Vec::new();
    if !CHECK_SPELLING.load(Ordering::Relaxed) {
        return out;
    }
    let caret = app.sel.head;
    for (i, page) in app.layout.pages.iter().enumerate() {
        if !geo.page_screen_rect(app, i).intersects(clip) {
            continue;
        }
        let mut texts: HashMap<&wordcore::ParaPath, Vec<Issue>> = HashMap::new();
        for l in &page.lines {
            if !texts.contains_key(&l.path) {
                let Some(p) = app.doc.para(&l.path) else { continue };
                texts.insert(&l.path, issues_for(&p.plain_text()));
            }
            for is in &texts[&l.path] {
                if is.kind == IssueKind::Grammar {
                    continue;
                }
                // Don't flag the word being typed.
                if l.path == caret.path && app.sel.is_empty() && is.end == caret.offset {
                    continue;
                }
                let a = is.start.max(l.start);
                let b = is.end.min(l.end);
                if a < b {
                    out.push((i, l.caret_x(a), l.caret_x(b), l.baseline));
                }
            }
        }
    }
    out
}

/// The spelling issue covering `pos`, if any, with its paragraph range.
fn issue_at(doc: &Document, pos: Pos) -> Option<Issue> {
    let p = doc.para(&pos.path)?;
    issues_for(&p.plain_text()).into_iter().find(|i| i.kind != IssueKind::Grammar && i.start <= pos.offset && pos.offset <= i.end)
}

fn replace_range(app: &mut App, pos: Pos, start: usize, end: usize, with: &str) {
    app.checkpoint("Spelling", EditKind::Other);
    let a = Pos { offset: start, ..pos };
    let b = Pos { offset: end, ..pos };
    let props = app.doc.para(&pos.path).map(|p| p.props_at(start + 1)).unwrap_or_default();
    let at = app.doc.delete_range(a, b);
    let np = app.doc.insert_text(at, with, &props);
    app.sel = crate::app::Selection::caret(np);
    app.changed();
}

/// Replace every flagged occurrence of `word` in the body with `with`, as one undo step.
pub fn change_all(app: &mut App, word: &str, with: &str) -> usize {
    app.checkpoint("Change All", EditKind::Other);
    let mut n = 0;
    for path in app.doc.para_paths(Story::Body) {
        let Some(p) = app.doc.para(&path) else { continue };
        let text = p.plain_text();
        let mut hits: Vec<(usize, usize)> = issues_for(&text)
            .into_iter()
            .filter(|i| i.kind == IssueKind::Spelling && word_text(&app.doc, path.at(0), i) == word)
            .map(|i| (i.start, i.end))
            .collect();
        hits.reverse();
        for (start, end) in hits {
            let props = app.doc.para(&path).map(|p| p.props_at(start + 1)).unwrap_or_default();
            let at = app.doc.delete_range(path.at(start), path.at(end));
            let np = app.doc.insert_text(at, with, &props);
            app.sel = crate::app::Selection::caret(np);
            n += 1;
        }
    }
    app.changed();
    n
}

fn word_text(doc: &Document, pos: Pos, is: &Issue) -> String {
    doc.text_range(Pos { offset: is.start, ..pos }, Pos { offset: is.end, ..pos })
}

pub fn word_for_thesaurus(app: &App) -> String {
    let (a, b) = if app.sel.is_empty() { app.doc.word_at(app.sel.head) } else { app.sel.range() };
    app.doc.text_range(a, b).trim().to_string()
}

/// Select the next spelling or grammar issue after the caret (wrapping around).
pub fn spelling_next(app: &mut App) -> bool {
    let story = Story::Body;
    let paths = app.doc.para_paths(story);
    if paths.is_empty() {
        return false;
    }
    let (_, from) = app.sel.range();
    let start_i = paths.iter().position(|p| *p == from.path).unwrap_or(0);
    for k in 0..=paths.len() {
        let i = (start_i + k) % paths.len();
        let path = &paths[i];
        let Some(p) = app.doc.para(path) else { continue };
        for is in issues_for(&p.plain_text()) {
            if k == 0 && is.start < from.offset {
                continue;
            }
            if k == paths.len() && is.start >= from.offset {
                break;
            }
            let a = path.at(is.start);
            let b = path.at(is.end);
            app.select(a, b);
            app.scroll_to_caret = true;
            return true;
        }
    }
    false
}

/// Add spelling suggestions to the context menu. Returns true if anything was added.
pub fn context_suggestions(app: &mut App, ui: &mut Ui) -> bool {
    if !CHECK_SPELLING.load(Ordering::Relaxed) || !app.sel.is_empty() {
        return false;
    }
    let pos = app.sel.head;
    let Some(is) = issue_at(&app.doc, pos) else { return false };
    let word = word_text(&app.doc, pos, &is);
    let mut sugg: Vec<String> = match (&is.kind, &is.suggestion) {
        (_, Some(s)) => vec![s.clone()],
        (IssueKind::Spelling, None) => Speller::english().suggest(&word, 5),
        _ => Vec::new(),
    };
    sugg.dedup();
    if is.kind != IssueKind::Spelling {
        ui.label(RichText::new(&is.message).weak().italics());
    }
    if sugg.is_empty() && is.kind == IssueKind::Spelling {
        ui.label(RichText::new("(No spelling suggestions)").weak());
    }
    for s in sugg {
        let label = if s.is_empty() { "Delete Repeated Word".to_string() } else { s.clone() };
        if ui.button(RichText::new(label).strong()).clicked() {
            replace_range(app, pos, is.start, is.end, &s);
            ui.close();
        }
    }
    if is.kind == IssueKind::Spelling {
        if ui.button("Ignore All").clicked() {
            ignore_word(&word);
            ui.close();
        }
        if ui.button("Add to Dictionary").clicked() {
            if let Ok(mut d) = user_dict().lock() {
                d.add(&word);
            }
            save_user_dict();
            clear_cache();
            ui.close();
        }
    }
    true
}

fn ignore_word(word: &str) {
    if let Ok(mut d) = user_dict().lock() {
        d.ignore_all(word);
    }
    clear_cache();
}

/// The Spelling & Grammar pane.
pub fn spelling_panel(app: &mut App, ui: &mut Ui) {
    let (a, b) = app.sel.range();
    let current = if a.path == b.path && a != b {
        app.doc.para(&a.path).and_then(|p| issues_for(&p.plain_text()).into_iter().find(|i| i.start == a.offset && i.end == b.offset))
    } else {
        None
    };
    let Some(is) = current else {
        ui.add_space(8.0);
        if spelling_next(app) {
            ui.ctx().request_repaint();
        } else {
            ui.label(RichText::new("✔ The spelling and grammar check is complete.").size(14.0));
            ui.add_space(6.0);
            ui.label("No issues were found in the document.");
        }
        return;
    };
    let word = word_text(&app.doc, a, &is);
    let title = match is.kind {
        IssueKind::Spelling => "Spelling",
        IssueKind::RepeatedWord => "Repeated Word",
        IssueKind::Grammar => "Grammar",
    };
    ui.label(RichText::new(title).size(15.0).strong());
    ui.add_space(4.0);
    ui.label(RichText::new(format!("“{}”", word.trim())).size(15.0));
    if !is.message.is_empty() {
        ui.label(RichText::new(&is.message).weak());
    }
    ui.add_space(8.0);
    let sugg: Vec<String> = match &is.suggestion {
        Some(s) => vec![s.clone()],
        None if is.kind == IssueKind::Spelling => Speller::english().suggest(&word, 8),
        None => Vec::new(),
    };
    // The chosen suggestion, kept per issue so it resets when the issue changes.
    let key = ui.scope_id().with(("spell_pick", a.path.clone(), is.start, word.clone()));
    let mut pick: usize = ui.ctx().data(|d| d.get_temp(key)).unwrap_or(0);
    if pick >= sugg.len() {
        pick = 0;
    }
    ui.label("Suggestions:");
    if sugg.is_empty() {
        ui.label(RichText::new("(no suggestions)").weak());
    }
    let mut change_now = None;
    egui::Frame::new().stroke(ui.visuals().widgets.noninteractive.bg_stroke).inner_margin(4.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        for (i, s) in sugg.iter().enumerate() {
            let label = if s.is_empty() { "(delete)".to_string() } else { s.clone() };
            let r = ui.add_sized([ui.available_width(), 22.0], egui::Button::selectable(i == pick, label));
            if r.clicked() {
                pick = i;
            }
            if r.double_clicked() {
                change_now = Some(i);
            }
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(key, pick));
    ui.add_space(8.0);
    let chosen = sugg.get(pick).cloned();
    ui.horizontal_wrapped(|ui| {
        let can = chosen.is_some();
        if ui.add_enabled(can, egui::Button::new("Change")).clicked() {
            change_now = Some(pick);
        }
        if ui.add_enabled(can && is.kind == IssueKind::Spelling, egui::Button::new("Change All")).clicked() {
            if let Some(c) = &chosen {
                change_all(app, &word, c);
                spelling_next(app);
                return;
            }
        }
    });
    if let Some(i) = change_now {
        if let Some(s) = sugg.get(i) {
            replace_range(app, a, is.start, is.end, s);
            spelling_next(app);
            return;
        }
    }
    ui.add_space(10.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button("Ignore Once").clicked() {
            app.set_caret(b, false);
            spelling_next(app);
        }
        if is.kind == IssueKind::Spelling {
            if ui.button("Ignore All").clicked() {
                ignore_word(&word);
                spelling_next(app);
            }
            if ui.button("Add to Dictionary").clicked() {
                if let Ok(mut d) = user_dict().lock() {
                    d.add(&word);
                }
                save_user_dict();
                clear_cache();
                spelling_next(app);
            }
        }
    });
}

/// The Thesaurus pane.
pub fn thesaurus_panel(app: &mut App, ui: &mut Ui, word: &mut String) {
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(word).desired_width(ui.available_width() - 40.0));
        if ui.button("🔍").clicked() {}
    });
    ui.add_space(6.0);
    let meanings = wordcore::proofing::thesaurus::lookup(word.trim());
    if word.trim().is_empty() {
        ui.label(RichText::new("Select a word to find synonyms.").weak());
        return;
    }
    if meanings.is_empty() {
        ui.label(RichText::new("No results found.").weak());
        return;
    }
    let mut pick = None;
    egui::ScrollArea::vertical().id_salt("thes").show(ui, |ui| {
        for m in &meanings {
            ui.add_space(4.0);
            ui.label(RichText::new(format!("{} ({})", m.synonyms.first().cloned().unwrap_or_default(), m.part_of_speech)).strong());
            for s in &m.synonyms {
                let r = ui.add(egui::Label::new(format!("    {}", s)).sense(egui::Sense::click()));
                let r = r.on_hover_text("Click to look up · double-click to insert");
                if r.double_clicked() {
                    pick = Some((s.clone(), true));
                } else if r.clicked() {
                    pick = Some((s.clone(), false));
                }
            }
        }
    });
    if let Some((s, insert)) = pick {
        if insert {
            let (a, b) = if app.sel.is_empty() { app.doc.word_at(app.sel.head) } else { app.sel.range() };
            if a.path == b.path {
                replace_range(app, a, a.offset, b.offset, &s);
            }
        } else {
            *word = s;
        }
    }
}

#[allow(dead_code)]
pub fn synonyms(word: &str) -> Vec<String> {
    wordcore::proofing::thesaurus::synonyms(word)
}

/// Rows for the Word Count dialog.
pub fn stats(app: &App) -> Vec<(String, String)> {
    let s = DocStats::compute(&app.doc, true);
    let lines: usize = app.layout.pages.iter().map(|p| p.lines.iter().filter(|l| l.path.story == Story::Body).count()).sum();
    let fmt = |n: usize| {
        let s = n.to_string();
        let mut out = String::new();
        for (i, c) in s.chars().enumerate() {
            if i > 0 && (s.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        out
    };
    vec![
        ("Pages".into(), fmt(app.layout.pages.len())),
        ("Words".into(), fmt(s.words)),
        ("Characters (no spaces)".into(), fmt(s.characters_no_spaces)),
        ("Characters (with spaces)".into(), fmt(s.characters_with_spaces)),
        ("Paragraphs".into(), fmt(s.paragraphs)),
        ("Lines".into(), fmt(lines)),
    ]
}

/// Words in the document (status bar).
pub fn word_count(doc: &Document) -> usize {
    DocStats::compute(doc, false).words
}
