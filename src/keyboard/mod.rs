mod icons;
mod microsoft_ime;
pub(crate) mod palette;
mod sound;
mod spellcheck;
mod translate;
mod typo;
mod view;

use microsoft_ime::{Clause, Lookup};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use translate::Translator;
use typo::{Snapshot, Typo};
use vime::{
    i18n::tr,
    ime::{Composer, Language},
    settings::{TranslationSettings, TypoProvider, TypoSettings},
};

pub use sound::play_click;
pub use typo::CalibrationJob;
pub(crate) use typo::typo_reporting_available;
pub use view::{SIZE, VrKeys, show, show_vr};

#[derive(Clone, Debug)]
pub enum Action {
    Key(char),
    Language(Language),
    Candidate(String),
    CorrectTypo(String),
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
    Copy,
    Paste,
    Send,
    Clear,
    Enter,
    Close,
    ToggleTheme,
    /// Puts the translation shown above the editor into the editor.
    AcceptTranslation,
    /// Switches to the next language the text is translated into.
    CycleTranslationTarget,
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
const INPUT_LIMIT: usize = 512;

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
        self.repeat_while(held.is_some(), now)
    }

    /// Repeats the key captured at press time while its button stays down.
    pub fn repeat_while(&mut self, down: bool, now: Instant) -> Option<Action> {
        if !down {
            self.release();
            return None;
        }
        let (action, next) = self.held.as_mut()?;
        if now < *next {
            return None;
        }
        *next = now + REPEAT_INTERVAL;
        Some(action.clone())
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
    typo: Option<Typo>,
    translator: Option<Translator>,
    context: eframe::egui::Context,
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
            ime: Lookup::start(context.clone()),
            typo: None,
            translator: None,
            context,
        }
    }

    /// Applies the translation settings. The key is read here because it is
    /// protected for this Windows user.
    pub fn configure_translation(&mut self, settings: &TranslationSettings) {
        if settings.enabled && self.translator.is_none() {
            self.translator = Some(Translator::start(self.context.clone()));
        }
        let Some(translator) = &mut self.translator else {
            return;
        };
        let was_shown = translator.enabled();
        translator.configure(settings, settings.api_key().unwrap_or_default());
        translator.changed(&self.composer.text());
        if was_shown || translator.enabled() {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// The translation to show above the editor, if translation is on.
    pub fn translation(&self) -> Option<&Translator> {
        self.translator.as_ref().filter(|translator| translator.enabled())
    }

    pub fn configure_typo(
        &mut self,
        settings: &TypoSettings,
        report_typos: bool,
        directory: &Path,
    ) {
        if settings.provider != TypoProvider::Off && self.typo.is_none() {
            self.typo = Some(Typo::start(directory.to_owned(), self.context.clone()));
        }
        if let Some(typo) = &mut self.typo {
            typo.configure(settings, report_typos);
        }
    }

    pub fn typo_suggestion(&self) -> Option<&str> {
        self.typo
            .as_ref()?
            .suggestion
            .as_ref()
            .map(|suggestion| suggestion.word.as_str())
    }

    /// Picks up a misconversion table that was just downloaded or removed.
    pub fn reload_misconversions(&self) {
        if let Some(typo) = &self.typo {
            typo.reload_misconversions();
        }
    }

    pub fn warm_local_typo(&self) {
        if let Some(typo) = &self.typo {
            typo.warm_local();
        }
    }

    pub fn calibrate(
        &mut self,
        settings: TypoSettings,
        directory: &Path,
    ) -> std::io::Result<CalibrationJob> {
        self.typo
            .get_or_insert_with(|| Typo::start(directory.to_owned(), self.context.clone()))
            .calibrate(settings)
    }

    pub fn typo_error(&self) -> Option<&std::io::Error> {
        self.typo.as_ref()?.error.as_ref()
    }

    pub fn finish_typo_edit(&mut self) {
        if let Some(typo) = &mut self.typo {
            typo.finish_edit(&self.composer);
        }
    }

    pub fn status(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.revision = self.revision.wrapping_add(1);
        self.caret_reset = Instant::now();
    }

    pub fn clear(&mut self) {
        if let Some(typo) = &mut self.typo {
            typo.reset();
        }
        if let Some(translator) = &mut self.translator {
            translator.reset();
        }
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
            view::CANDIDATE_PAGE_SIZE - usize::from(self.typo_suggestion().is_some())
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
                | Action::CorrectTypo(_)
                | Action::Backspace
                | Action::Commit
                | Action::MoveCaret(_)
                | Action::ToggleDirect
                | Action::MoveSegment(_)
                | Action::Space
        );
        let deleting = matches!(action, Action::Backspace) && !was_converting;
        let typing = matches!(action, Action::Key(key) if key.is_alphanumeric() || matches!(key, '\'' | '’'));
        let character =
            matches!(action, Action::Key(_)) && self.composer.text().chars().count() < INPUT_LIMIT;
        let text_before = self.translation().map(|_| self.composer.text());
        let snapshot = (editing && self.typo.as_ref().is_some_and(Typo::enabled))
            .then(|| Snapshot::of(&self.composer));
        if snapshot.is_some() && !deleting && !typing && !matches!(action, Action::CorrectTypo(_)) {
            self.finish_typo_edit();
        }
        match action {
            Action::Key(key) if self.composer.text().chars().count() < INPUT_LIMIT => {
                if self.direct_input() {
                    self.composer.insert(&key.to_string());
                } else {
                    self.composer.key(key);
                }
                self.shift = false;
            }
            Action::Insert(text) if self.composer.text().chars().count() < INPUT_LIMIT => {
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
            Action::CorrectTypo(value) => {
                if let Some(typo) = &mut self.typo
                    && let Some(suggestion) = &typo.suggestion
                    && suggestion.word == value
                {
                    self.composer.replace_text(suggestion.range.clone(), &value);
                    typo.reset();
                }
            }
            Action::Backspace => self.composer.backspace(),
            Action::Commit => self.composer.commit(),
            Action::MoveCaret(index) => self.composer.move_caret(index),
            Action::Space if self.composer.converting() => self.composer.next_candidate(),
            Action::Space if self.can_convert() => self.convert(),
            Action::Space if self.composer.text().chars().count() < INPUT_LIMIT => {
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
            Action::AcceptTranslation => {
                if let Some(translation) = self.translation().and_then(Translator::candidate) {
                    let translation: String = translation.chars().take(INPUT_LIMIT).collect();
                    self.clear();
                    self.composer.insert(&translation);
                    if let Some(translator) = &mut self.translator {
                        translator.accepted(&translation);
                    }
                }
            }
            Action::Key(_)
            | Action::Insert(_)
            | Action::Space
            | Action::Copy
            | Action::Paste
            | Action::Send
            | Action::Enter
            | Action::Close
            | Action::ToggleTheme
            | Action::CycleTranslationTarget => {}
        }
        if let Some(before) = text_before {
            let after = self.composer.text();
            if after != before
                && let Some(translator) = &mut self.translator
            {
                translator.changed(&after);
            }
        }
        if let Some(snapshot) = snapshot
            && let Some(typo) = &mut self.typo
        {
            if was_converting || self.composer.converting() {
                typo.reset();
            } else {
                let after = Snapshot::of(&self.composer);
                if character || snapshot != after {
                    typo.changed(deleting.then_some(snapshot), typing, after);
                }
            }
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

    pub fn paste(&mut self, text: &str) -> Result<(), &'static str> {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if text.is_empty() || text.contains('\0') {
            return Err(tr("クリップボードに貼り付け可能なテキストがありません"));
        }
        let remaining = INPUT_LIMIT.saturating_sub(self.composer.text().chars().count());
        if text.chars().take(remaining + 1).count() > remaining {
            return Err(tr("貼り付けると入力欄の上限を超えます"));
        }
        self.apply(Action::Insert(text));
        Ok(())
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
        if let Some(typo) = &mut self.typo
            && typo.poll(&self.composer)
        {
            self.candidate_page = 0;
            self.revision = self.revision.wrapping_add(1);
        }
        if let Some(translator) = &mut self.translator
            && translator.poll()
        {
            self.revision = self.revision.wrapping_add(1);
        }
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
    fn repeats_the_original_key_until_button_release() {
        let start = Instant::now();
        for key in [Action::Backspace, Action::Space] {
            let mut repeater = Repeater::default();
            let first_repeat = start + REPEAT_DELAY;
            let next_repeat = first_repeat + REPEAT_INTERVAL;
            assert!(repeater.repeat_while(true, start).is_none());
            repeater.begin(&key, start);
            assert!(repeater.repeat_while(true, start).is_none());
            assert!(
                repeater
                    .repeat_while(true, first_repeat)
                    .unwrap()
                    .same_key(&key)
            );
            assert!(repeater.repeat_while(false, next_repeat).is_none());
            assert!(repeater.repeat_while(true, next_repeat).is_none());
        }
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
    fn resumes_romaji_after_deleting_a_mistyped_key_or_kana() {
        for (typed, remaining, next, expected) in [
            ("kakikp", "かきk", "u", "かきく"),
            ("kakikpa", "かきk", "u", "かきく"),
            ("syka", "sy", "a", "しゃ"),
            ("SH.", "SH", "u", "しゅ"),
            ("kkpa", "kk", "a", "っか"),
        ] {
            let mut keyboard = Keyboard::new(eframe::egui::Context::default());
            keys(&mut keyboard, typed);
            keyboard.apply(Action::Backspace);
            assert_eq!(keyboard.composer.preedit(), remaining, "{typed}");
            keys(&mut keyboard, next);
            assert_eq!(keyboard.composer.preedit(), expected, "{typed}");
        }
    }

    #[test]
    fn space_types_a_space_without_a_reading() {
        let mut keyboard = Keyboard::new(eframe::egui::Context::default());
        keyboard.apply(Action::Space);
        assert_eq!(keyboard.composer.text(), " ");
    }

    #[test]
    fn pastes_unicode_at_the_caret_after_committing_preedit() {
        let mut keyboard = Keyboard::new(eframe::egui::Context::default());
        keyboard.apply(Action::Insert("前後".into()));
        keyboard.apply(Action::MoveCaret(1));
        keys(&mut keyboard, "neko");
        assert!(keyboard.composer.has_preedit());

        keyboard.paste("한글\r\n🌸").unwrap();

        assert!(!keyboard.composer.has_preedit());
        assert_eq!(
            keyboard.composer.committed_parts(),
            ("前ねこ한글\n🌸", "後")
        );
    }

    #[test]
    fn rejects_paste_over_the_input_limit_without_inserting_partial_text() {
        let mut keyboard = Keyboard::new(eframe::egui::Context::default());
        let original = "猫".repeat(INPUT_LIMIT - 1);
        keyboard.apply(Action::Insert(original.clone()));

        assert!(keyboard.paste("🌸🌸").is_err());
        assert_eq!(keyboard.composer.committed(), original);
        keyboard.paste("🌸").unwrap();
        assert_eq!(keyboard.composer.committed().chars().count(), INPUT_LIMIT);
    }

    #[test]
    fn japanese_symbols_can_be_converted_to_halfwidth() {
        let mut keyboard = Keyboard::new(eframe::egui::Context::default());
        keys(&mut keyboard, "12W,.![]¥");
        assert_eq!(keyboard.composer.text(), "12W、。！「」￥");
        keyboard.apply(Action::Space);
        assert_eq!(keyboard.composer.text(), "12W、。！「」￥");
        assert!(keyboard.candidates.contains(&"W,.!｢｣¥".into()));
        assert!(keyboard.candidates.contains(&"W,.![]¥".into()));
        keyboard.apply(Action::Candidate("W,.![]¥".into()));
        assert_eq!(keyboard.composer.committed(), "12W,.![]¥");

        keyboard.apply(Action::ToggleDirect);
        keys(&mut keyboard, "Ab9,.![]");
        assert_eq!(keyboard.composer.committed(), "12W,.![]¥Ab9,.![]");
    }

    #[test]
    fn japanese_conversion_offers_both_katakana_widths() {
        let mut keyboard = Keyboard::new(eframe::egui::Context::default());
        keys(&mut keyboard, "baggu");
        keyboard.apply(Action::Space);
        assert!(keyboard.candidates.contains(&"バッグ".into()));
        assert!(keyboard.candidates.contains(&"ﾊﾞｯｸﾞ".into()));
        keyboard.apply(Action::Candidate("ﾊﾞｯｸﾞ".into()));
        assert_eq!(keyboard.composer.committed(), "ﾊﾞｯｸﾞ");
    }
}
