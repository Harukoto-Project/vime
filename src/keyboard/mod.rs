mod microsoft_ime;
mod view;

use microsoft_ime::Lookup;
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
    SetDirect(bool),
    ConvertJapanese,
    ToggleEmoji,
    PageEmoji(i32),
    PageCandidates,
    Clear,
    Enter,
    Close,
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
    pub status: String,
    pub revision: u64,
    ime: Lookup,
}

impl Keyboard {
    pub fn new(context: eframe::egui::Context) -> Self {
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
            status: String::new(),
            revision: 0,
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
        self.candidate_page = 0;
    }

    pub fn direct_input(&self) -> bool {
        match self.composer.language() {
            Language::Japanese => self.japanese_direct,
            Language::Korean => self.korean_direct,
            Language::English => false,
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
                | Action::SetDirect(_)
                | Action::ConvertJapanese
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
            Action::Candidate(value) => self.composer.select(&value),
            Action::Backspace => self.composer.backspace(),
            Action::Commit => self.composer.commit(),
            Action::MoveCaret(index) => self.composer.move_caret(index),
            Action::Space => self.composer.insert(" "),
            Action::Shift => self.shift = !self.shift,
            Action::ToggleDirect => self.set_direct_input(!self.direct_input()),
            Action::SetDirect(direct) => self.set_direct_input(direct),
            Action::ConvertJapanese => {
                if let Some(candidate) = self.candidates.first() {
                    self.composer.select(candidate);
                }
            }
            Action::ToggleEmoji => self.emoji = !self.emoji,
            Action::PageEmoji(step) => {
                let pages = vime::ime::search_emoji("")
                    .count()
                    .div_ceil(view::EMOJI_PAGE_SIZE);
                self.emoji_page = (self.emoji_page as i32 + step).rem_euclid(pages as i32) as usize;
            }
            Action::PageCandidates => {
                let pages = self
                    .candidates
                    .len()
                    .div_ceil(view::CANDIDATE_PAGE_SIZE)
                    .max(1);
                self.candidate_page = (self.candidate_page + 1) % pages;
            }
            Action::Clear => self.clear(),
            Action::Key(_) | Action::Insert(_) | Action::Enter | Action::Close => {}
        }
        let reading = self.composer.reading();
        if editing && (reading != before || self.composer.language() == Language::English) {
            self.refresh_candidates(&reading);
        }
        self.status("");
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
        if self.composer.language() == Language::Japanese && !self.direct_input() {
            self.ime.request(reading);
        }
        self.candidate_page = 0;
    }

    pub fn poll_candidates(&mut self) {
        let Some((reading, candidates)) = self.ime.latest() else {
            return;
        };
        if self.composer.language() != Language::Japanese
            || self.direct_input()
            || self.composer.reading() != reading
        {
            return;
        }
        for candidate in candidates.into_iter().rev() {
            if !self.candidates.contains(&candidate) {
                self.candidates.insert(0, candidate);
                self.revision = self.revision.wrapping_add(1);
            }
        }
    }
}
