mod microsoft_ime;
mod view;

use microsoft_ime::{Clause, Lookup};
use std::time::{Duration, Instant};
use vime::ime::{Composer, Language};

pub use view::{ASPECT, SIZE, VrKeys, show, show_vr};

#[derive(Clone, Debug)]
pub enum Action {
    Key(char),
    Language(Language),
    Candidate(String),
    Insert(String),
    Backspace,
    Commit,
    MoveCaret(usize),
    Space,
    Shift,
    ToggleDirect,
    MoveSegment(i32),
    ToggleEmoji,
    PageEmoji(i32),
    PageCandidates,
    Clear,
    Enter,
    Close,
}

impl Action {
    /// Keys that keep acting while they are held down.
    pub fn repeats(&self) -> bool {
        matches!(self, Self::Backspace | Self::Space)
    }

    pub fn same_key(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

const REPEAT_DELAY: Duration = Duration::from_millis(400);
const REPEAT_INTERVAL: Duration = Duration::from_millis(60);

/// Repeats a held key such as Backspace after an initial delay. One repeater
/// follows one pointer source, because each source keeps its own held state.
#[derive(Default)]
pub struct Repeater {
    held: Option<(Action, Instant)>,
}

impl Repeater {
    /// Whether `action` is the key currently being held.
    pub fn holding(&self, action: &Action) -> bool {
        self.held
            .as_ref()
            .is_some_and(|(current, _)| current.same_key(action))
    }

    /// Starts a hold whose first press was already applied by the caller.
    pub fn begin(&mut self, action: &Action, now: Instant) {
        self.held = Some((action.clone(), now + REPEAT_DELAY));
    }

    pub fn release(&mut self) {
        self.held = None;
    }

    /// Follows the held key of a pointer. A key that was just pressed is
    /// returned to be applied once; later calls return it on every repeat.
    pub fn tick(&mut self, held: Option<&Action>, now: Instant) -> Option<Action> {
        if let Some(action) = held
            && !self.holding(action)
        {
            self.begin(action, now);
            return Some(action.clone());
        }
        self.follow(held, now)
    }

    /// Continues a hold started with `begin`. Moving onto another key does not
    /// start repeating it, because its press was never delivered.
    pub fn follow(&mut self, held: Option<&Action>, now: Instant) -> Option<Action> {
        match (&mut self.held, held) {
            (Some((current, next)), Some(action)) if current.same_key(action) => {
                if now >= *next {
                    *next = now + REPEAT_INTERVAL;
                    Some(action.clone())
                } else {
                    None
                }
            }
            _ => {
                self.held = None;
                None
            }
        }
    }

    /// How long until the held key repeats again.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.held
            .as_ref()
            .map(|(_, next)| next.saturating_duration_since(now))
    }
}

pub struct Keyboard {
    pub composer: Composer,
    pub shift: bool,
    japanese_direct: bool,
    korean_direct: bool,
    caret_reset: Instant,
    pub emoji: bool,
    pub emoji_page: usize,
    pub candidate_page: usize,
    pub candidates: Vec<String>,
    /// The candidate in use while a clause is being converted.
    pub candidate_selected: Option<usize>,
    pub status: String,
    pub revision: u64,
    clauses: Option<(String, Vec<Clause>)>,
    ime: Lookup,
}

impl Keyboard {
    pub fn new(context: eframe::egui::Context) -> Self {
        vime::ime::warm_up_dictionary();
        Self {
            composer: Composer::default(),
            shift: false,
            japanese_direct: false,
            korean_direct: false,
            caret_reset: Instant::now(),
            emoji: false,
            emoji_page: 0,
            candidate_page: 0,
            candidates: Vec::new(),
            candidate_selected: None,
            status: String::new(),
            revision: 0,
            clauses: None,
            ime: Lookup::start(context),
        }
    }

    pub fn status(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.revision = self.revision.wrapping_add(1);
        self.caret_reset = Instant::now();
    }

    pub fn clear(&mut self) {
        self.composer.clear();
        self.candidates.clear();
        self.candidate_selected = None;
        self.candidate_page = 0;
        self.clauses = None;
    }

    pub fn direct_input(&self) -> bool {
        match self.composer.language() {
            Language::Japanese => self.japanese_direct,
            Language::Korean => self.korean_direct,
            Language::English => false,
        }
    }

    /// Whether Space starts a conversion instead of typing a space.
    pub fn can_convert(&self) -> bool {
        self.composer.language() == Language::Japanese
            && !self.direct_input()
            && self.composer.has_preedit()
            && !self.composer.converting()
    }

    pub fn page_size(&self) -> usize {
        if self.composer.converting() {
            view::CONVERSION_PAGE_SIZE
        } else {
            view::CANDIDATE_PAGE_SIZE
        }
    }

    pub fn caret_blink(&self, now: Instant) -> (bool, Duration) {
        const HALF_PERIOD: u128 = 530;
        let elapsed = now.saturating_duration_since(self.caret_reset).as_millis();
        (
            (elapsed / HALF_PERIOD).is_multiple_of(2),
            Duration::from_millis((HALF_PERIOD - elapsed % HALF_PERIOD) as u64),
        )
    }

    pub fn apply(&mut self, action: Action) {
        let before = self.composer.reading();
        let was_converting = self.composer.converting();
        let paging = matches!(action, Action::PageCandidates);
        let editing = matches!(
            action,
            Action::Key(_)
                | Action::Insert(_)
                | Action::Language(_)
                | Action::Candidate(_)
                | Action::Backspace
                | Action::Commit
                | Action::MoveCaret(_)
                | Action::ToggleDirect
                | Action::MoveSegment(_)
                | Action::Space
                | Action::Clear
        );
        match action {
            Action::Key(key) if self.composer.text().chars().count() < 512 => {
                if self.direct_input() {
                    self.composer.insert(&key.to_string());
                } else {
                    self.composer.key(key);
                }
                self.shift = false;
            }
            Action::Insert(text) if self.composer.text().chars().count() < 512 => {
                self.composer.insert(&text);
                self.emoji = false;
            }
            Action::Language(language) => {
                self.composer.set_language(language);
                self.emoji = false;
                self.shift = false;
            }
            Action::Candidate(value) if self.composer.converting() => {
                self.composer.pick_candidate(&value);
            }
            Action::Candidate(value) => self.composer.select(&value),
            Action::Backspace => self.composer.backspace(),
            Action::Commit => self.composer.commit(),
            Action::MoveCaret(index) => self.composer.move_caret(index),
            Action::Space if self.composer.converting() => self.composer.next_candidate(),
            Action::Space if self.can_convert() => self.convert(),
            Action::Space if self.composer.text().chars().count() < 512 => {
                self.composer.insert(" ");
            }
            Action::MoveSegment(step) => self.composer.move_segment(step),
            Action::Shift => self.shift = !self.shift,
            Action::ToggleDirect => self.set_direct_input(!self.direct_input()),
            Action::ToggleEmoji => self.emoji = !self.emoji,
            Action::PageEmoji(step) => {
                let pages = vime::ime::search_emoji("")
                    .count()
                    .div_ceil(view::EMOJI_PAGE_SIZE);
                self.emoji_page = (self.emoji_page as i32 + step).rem_euclid(pages as i32) as usize;
            }
            Action::PageCandidates => {
                let pages = self.candidates.len().div_ceil(self.page_size()).max(1);
                self.candidate_page = (self.candidate_page + 1) % pages;
            }
            Action::Clear => self.clear(),
            Action::Key(_) | Action::Insert(_) | Action::Space | Action::Enter | Action::Close => {}
        }
        if self.composer.converting() {
            if !paging {
                self.show_clause_candidates();
            }
        } else {
            let reading = self.composer.reading();
            if editing
                && (was_converting
                    || reading != before
                    || self.composer.language() == Language::English)
            {
                self.refresh_candidates(&reading);
            }
        }
        self.status("");
    }

    fn convert(&mut self) {
        let reading = self.composer.raw_reading();
        let hints: Vec<(String, String)> = self
            .clauses
            .as_ref()
            .filter(|(known, _)| *known == reading)
            .map(|(_, clauses)| {
                clauses
                    .iter()
                    .map(|clause| (clause.reading.clone(), clause.text.clone()))
                    .collect()
            })
            .unwrap_or_default();
        self.composer.convert_reading(&hints);
    }

    /// Lists the candidates of the clause that has the focus.
    fn show_clause_candidates(&mut self) {
        let Some(conversion) = self.composer.conversion() else {
            return;
        };
        let segment = conversion.focused();
        self.candidates = segment.candidates.clone();
        self.candidate_selected = Some(segment.selected);
        self.candidate_page = segment.selected / self.page_size();
    }

    fn set_direct_input(&mut self, direct: bool) {
        let mode = match self.composer.language() {
            Language::Japanese => &mut self.japanese_direct,
            Language::Korean => &mut self.korean_direct,
            Language::English => return,
        };
        if *mode != direct {
            self.composer.commit();
            *mode = direct;
            self.shift = false;
        }
    }

    fn refresh_candidates(&mut self, reading: &str) {
        self.candidates = self.composer.candidates();
        self.candidate_selected = None;
        self.clauses = None;
        if self.composer.language() == Language::Japanese && !self.direct_input() {
            self.ime.request(reading);
        }
        self.candidate_page = 0;
    }

    pub fn poll_candidates(&mut self) {
        let Some((reading, clauses)) = self.ime.latest() else {
            return;
        };
        if self.composer.language() != Language::Japanese
            || self.direct_input()
            || self.composer.converting()
            || self.composer.reading() != reading
        {
            return;
        }
        // The whole sentence as converted by the system IME leads the list.
        let sentence: String = clauses.iter().map(|clause| clause.text.as_str()).collect();
        if !sentence.is_empty() && sentence != reading && !self.candidates.contains(&sentence) {
            self.candidates.insert(0, sentence);
            self.revision = self.revision.wrapping_add(1);
        }
        self.clauses = Some((reading, clauses));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(keyboard: &mut Keyboard, text: &str) {
        text.chars()
            .for_each(|key| keyboard.apply(Action::Key(key)));
    }

    #[test]
    fn repeats_a_held_key_after_the_delay() {
        let start = Instant::now();
        let mut repeater = Repeater::default();
        let held = Action::Backspace;
        let at = |millis: u64| start + Duration::from_millis(millis);
        assert!(repeater.tick(Some(&held), start).is_some());
        assert!(repeater.tick(Some(&held), at(100)).is_none());
        assert!(repeater.tick(Some(&held), start + REPEAT_DELAY).is_some());
        assert!(repeater.tick(Some(&held), at(410)).is_none());
        assert!(
            repeater
                .tick(Some(&held), start + REPEAT_DELAY + REPEAT_INTERVAL)
                .is_some()
        );
        assert!(repeater.tick(None, at(1000)).is_none());
        assert!(!repeater.holding(&held));
    }

    #[test]
    fn a_followed_hold_needs_its_press_to_start() {
        let start = Instant::now();
        let mut repeater = Repeater::default();
        let late = start + REPEAT_DELAY * 2;
        assert!(repeater.follow(Some(&Action::Space), late).is_none());
        repeater.begin(&Action::Space, start);
        assert!(
            repeater
                .follow(Some(&Action::Space), start + REPEAT_DELAY)
                .is_some()
        );
        // Sliding onto another key ends the hold instead of repeating it.
        assert!(repeater.follow(Some(&Action::Backspace), late).is_none());
        assert!(!repeater.holding(&Action::Space));
    }

    #[test]
    fn space_converts_then_cycles_and_backspace_undoes_it() {
        let mut keyboard = Keyboard::new(eframe::egui::Context::default());
        keys(&mut keyboard, "kyouhaiitenki");
        assert!(keyboard.can_convert());
        let reading = keyboard.composer.text();
        keyboard.apply(Action::Space);
        assert!(keyboard.composer.converting());
        assert!(!keyboard.candidates.is_empty());
        assert_eq!(keyboard.candidate_selected, Some(0));
        let first = keyboard.composer.text();
        keyboard.apply(Action::Space);
        assert!(keyboard.composer.converting());
        assert_ne!(keyboard.composer.text(), first);
        assert_eq!(keyboard.candidate_selected, Some(1));
        keyboard.apply(Action::Backspace);
        assert!(!keyboard.composer.converting());
        assert_eq!(keyboard.composer.text(), reading);
        // Typing during a conversion commits it first.
        keyboard.apply(Action::Space);
        let converted = keyboard.composer.text();
        keys(&mut keyboard, "a");
        assert!(!keyboard.composer.converting());
        assert_eq!(keyboard.composer.committed(), converted);
        assert_eq!(keyboard.composer.text(), converted.clone() + "あ");
    }

    #[test]
    fn space_types_a_space_without_a_reading() {
        let mut keyboard = Keyboard::new(eframe::egui::Context::default());
        keyboard.apply(Action::Space);
        assert_eq!(keyboard.composer.text(), " ");
    }
}
