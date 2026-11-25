mod conversion;
mod emoji;
mod extras;
mod japanese;
mod korean;
mod learning;
pub mod misconversion;

use learning::Learned;
use std::{io, path::Path};
use unicode_segmentation::UnicodeSegmentation;

pub use conversion::{Conversion, Segment};
pub use japanese::key_label as japanese_key_label;
pub use japanese::warm_up as warm_up_dictionary;
pub use japanese::{is_reading_char, reading_corrections};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Language {
    #[default]
    Japanese,
    English,
    Korean,
}

impl Language {
    pub const ALL: [Self; 3] = [Self::Japanese, Self::English, Self::Korean];

    pub fn label(self) -> &'static str {
        match self {
            Self::Japanese => "日本語",
            Self::English => "English",
            Self::Korean => "한국어",
        }
    }
}

#[derive(Default)]
pub struct Composer {
    language: Language,
    committed: String,
    caret: usize,
    // Rendered Japanese preedit retained after deleting a character.
    japanese_prefix: String,
    reading: String,
    // Set while the reading is converted. `reading` is kept to return to it.
    conversion: Option<Conversion>,
    // The words chosen so far, which lead later candidates and predictions.
    learned: Learned,
}

/// Dictionary words listed for a typed reading before the other candidates.
/// Pressing Space lists all of them.
const MAX_BAR_WORDS: usize = 12;

/// Whether `text` is a plain rendering of the reading that should not be learned.
fn is_plain_form(reading: &str, text: &str) -> bool {
    let katakana = japanese::katakana(reading);
    let halfwidth = japanese::halfwidth(&katakana);
    text == reading
        || text == katakana
        || text == halfwidth
        || japanese::with_ascii_brackets(&halfwidth).as_deref() == Some(text)
}

fn push_unique(candidates: &mut Vec<String>, value: &str) {
    if !value.is_empty() && !candidates.iter().any(|known| known == value) {
        candidates.push(value.to_owned());
    }
}

impl Composer {
    pub fn language(&self) -> Language {
        self.language
    }

    pub fn set_language(&mut self, language: Language) {
        if self.language != language {
            self.commit();
            self.language = language;
        }
    }

    pub fn key(&mut self, key: char) {
        if self.conversion.is_some() {
            self.commit();
        }
        match self.language {
            Language::Japanese
                if key.is_ascii_alphabetic() || key.is_ascii_punctuation() || key == '¥' =>
            {
                self.reading.push(key);
            }
            Language::Korean if korean::key_label(key).is_some() => self.reading.push(key),
            _ => self.insert(&key.to_string()),
        }
    }

    pub fn insert(&mut self, text: &str) {
        self.commit();
        self.insert_committed(text);
    }

    pub fn backspace(&mut self) {
        // Like desktop IMEs, the first Backspace only undoes the conversion.
        if self.conversion.take().is_some() {
            return;
        }
        if self.language == Language::Japanese && self.has_preedit() {
            let mut preedit = self.preedit();
            if let Some((index, last)) = preedit.grapheme_indices(true).next_back() {
                // Keep a pending romaji suffix editable when deletion leaves the
                // preceding display intact.
                if let Some((last_key, _)) = self.reading.char_indices().next_back()
                    && (preedit[..index].strip_prefix(&self.japanese_prefix)
                        == Some(japanese::kana(&self.reading[..last_key], false).as_str())
                        || (last.as_bytes()[0].is_ascii_alphabetic()
                            && self.reading[..last_key].ends_with(['n', 'N'])
                            && preedit[..index].ends_with('ん')))
                {
                    self.reading.truncate(last_key);
                } else {
                    preedit.truncate(index);
                    // Deleting a kana can expose unfinished letters (kpa → k).
                    // Keep the trailing letters editable without changing the
                    // displayed text (e.g. literal "nn" after deleting from "nnn").
                    let mut split = preedit
                        .trim_end_matches(|c: char| c.is_ascii_alphabetic())
                        .len();
                    while japanese::kana(&preedit[split..], false) != preedit[split..] {
                        split += 1;
                    }
                    self.reading = preedit.split_off(split);
                    self.japanese_prefix = preedit;
                }
            }
            return;
        }
        if self.reading.pop().is_none()
            && let Some((index, _)) = self.committed[..self.caret]
                .grapheme_indices(true)
                .next_back()
        {
            self.committed.replace_range(index..self.caret, "");
            self.caret = self.boundary_at_or_after(index);
        }
    }

    pub fn clear(&mut self) {
        self.committed.clear();
        self.japanese_prefix.clear();
        self.reading.clear();
        self.conversion = None;
        self.caret = 0;
    }

    pub fn committed(&self) -> &str {
        &self.committed
    }

    pub fn committed_parts(&self) -> (&str, &str) {
        self.committed.split_at(self.caret)
    }

    pub fn has_preedit(&self) -> bool {
        !self.japanese_prefix.is_empty() || !self.reading.is_empty()
    }

    pub fn move_caret(&mut self, display_index: usize) {
        let prefix = self.committed[..self.caret].chars().count();
        let display_len = self.preedit().chars().count();
        let committed_len = self.reading().chars().count();
        let index = if display_index >= prefix + display_len {
            display_index - display_len + committed_len
        } else if display_index > prefix {
            prefix + (display_index - prefix).min(committed_len)
        } else {
            display_index
        };
        self.commit();
        let mut position = 0;
        for (byte, grapheme) in self.committed.grapheme_indices(true) {
            let next = position + grapheme.chars().count();
            if index < next {
                self.caret = if index - position < next - index {
                    byte
                } else {
                    byte + grapheme.len()
                };
                return;
            }
            position = next;
        }
        self.caret = self.committed.len();
    }

    /// The underlined text as shown in the editor.
    pub fn preedit(&self) -> String {
        match &self.conversion {
            Some(conversion) => conversion.text(),
            None => self.convert(false),
        }
    }

    /// The preedit split into clauses. The flag marks the clause being converted.
    pub fn preedit_segments(&self) -> Vec<(String, bool)> {
        match &self.conversion {
            Some(conversion) => conversion
                .segments()
                .iter()
                .enumerate()
                .map(|(index, segment)| (segment.text().to_owned(), index == conversion.focus()))
                .collect(),
            None => vec![(self.convert(false), false)],
        }
    }

    /// The text that committing the preedit would insert.
    pub fn reading(&self) -> String {
        match &self.conversion {
            Some(conversion) => conversion.text(),
            None => self.convert(true),
        }
    }

    /// The typed reading, ignoring any conversion of it.
    pub fn raw_reading(&self) -> String {
        self.convert(true)
    }

    pub fn conversion(&self) -> Option<&Conversion> {
        self.conversion.as_ref()
    }

    pub fn converting(&self) -> bool {
        self.conversion.is_some()
    }

    /// Converts the Japanese reading into clauses. `hints` are clauses from the
    /// system IME as `(reading, text)`. Returns false if there is nothing to convert.
    pub fn convert_reading(&mut self, hints: &[(String, String)]) -> bool {
        if self.language != Language::Japanese || self.conversion.is_some() {
            return false;
        }
        let reading = self.raw_reading();
        if reading.is_empty() {
            return false;
        }
        self.conversion = Conversion::new(japanese::segments(&reading, hints, &self.learned));
        self.conversion.is_some()
    }

    /// Turns learning on or off and keeps the learned words in `directory`.
    pub fn configure_learning(&mut self, enabled: bool, directory: &Path) {
        if self.learned.path() != Some(directory.join(learning::FILE).as_path()) {
            self.learned.save();
            self.learned = Learned::load(directory);
        }
        self.learned.set_enabled(enabled);
    }

    /// How many words were learned.
    pub fn learned_count(&self) -> usize {
        self.learned.len()
    }

    /// Forgets the learned words and deletes them from the disk.
    pub fn forget_learned(&mut self) -> io::Result<()> {
        self.learned.clear()
    }

    /// Whether learned words are waiting to be saved.
    pub fn learning_pending(&self) -> bool {
        self.learned.pending()
    }

    /// Saves the learned words once the latest choice is a moment old.
    pub fn save_learned_when_due(&mut self) {
        self.learned.save_when_due();
    }

    /// Chooses the next candidate of the focused clause.
    pub fn next_candidate(&mut self) {
        if let Some(conversion) = &mut self.conversion {
            conversion.next_candidate();
        }
    }

    /// Chooses `candidate` for the focused clause and moves to the next clause.
    /// Choosing in the last clause commits the whole conversion.
    pub fn pick_candidate(&mut self, candidate: &str) {
        let Some(conversion) = &mut self.conversion else {
            return;
        };
        conversion.select(candidate);
        if conversion.is_last() {
            self.commit();
        } else {
            conversion.move_focus(1);
        }
    }

    pub fn move_segment(&mut self, step: i32) {
        if let Some(conversion) = &mut self.conversion {
            conversion.move_focus(step);
        }
    }

    pub fn text(&self) -> String {
        let (before, after) = self.committed_parts();
        before.to_owned() + &self.reading() + after
    }

    /// Replace a range of the editor text while retaining its caret. A range
    /// inside a Japanese reading stays part of the reading, so that it can still
    /// be converted afterwards.
    pub fn replace_text(&mut self, range: std::ops::Range<usize>, replacement: &str) -> bool {
        let text = self.text();
        if self.converting() || text.get(range.clone()).is_none() {
            return false;
        }
        if self.language == Language::Japanese && range.start >= self.caret {
            let mut kana = self.reading();
            let (start, end) = (range.start - self.caret, range.end - self.caret);
            if end <= kana.len() {
                kana.replace_range(start..end, replacement);
                self.japanese_prefix.clear();
                self.reading = kana;
                return true;
            }
        }
        self.commit();
        let caret = if self.caret >= range.end {
            self.caret - range.len() + replacement.len()
        } else if self.caret > range.start {
            range.start + replacement.len()
        } else {
            self.caret
        };
        self.committed.replace_range(range, replacement);
        self.caret = self.boundary_at_or_after(caret);
        true
    }

    pub fn candidates(&self) -> Vec<String> {
        if self.conversion.is_some() {
            return Vec::new();
        }
        if let Some((_, shortcode)) = self.shortcode() {
            return emoji::lookup(shortcode)
                .map(|entry| entry.symbol.to_owned())
                .collect();
        }
        let reading = self.reading();
        if reading.is_empty() {
            return Vec::new();
        }
        let mut candidates = Vec::new();
        if self.language != Language::Japanese {
            for entry in emoji::lookup(&reading) {
                push_unique(&mut candidates, entry.symbol);
            }
            push_unique(&mut candidates, &reading);
            return candidates;
        }
        // Words the user chose before, then the dictionary.
        for text in self.learned.exact(&reading) {
            push_unique(&mut candidates, text);
        }
        for surface in japanese::dictionary(&reading).take(MAX_BAR_WORDS) {
            push_unique(&mut candidates, surface);
        }
        push_unique(&mut candidates, &reading);
        let katakana = japanese::katakana(&reading);
        push_unique(&mut candidates, &katakana);
        let halfwidth = japanese::halfwidth(&katakana);
        push_unique(&mut candidates, &halfwidth);
        if let Some(value) = japanese::with_ascii_brackets(&halfwidth) {
            push_unique(&mut candidates, &value);
        }
        // Numbers and dates, then words that the typed reading may lead to.
        for text in japanese::computed(&reading) {
            push_unique(&mut candidates, &text);
        }
        for (_, text) in self.learned.prefix(&reading, japanese::MAX_PREDICTIONS) {
            push_unique(&mut candidates, text);
        }
        for (_, surface) in japanese::predictions(&reading, japanese::MAX_PREDICTIONS) {
            push_unique(&mut candidates, surface);
        }
        // Symbols, kaomoji and emoji.
        for entry in emoji::lookup(&reading) {
            push_unique(&mut candidates, entry.symbol);
        }
        for symbol in japanese::symbols(&reading) {
            push_unique(&mut candidates, symbol);
        }
        candidates
    }

    pub fn commit(&mut self) {
        self.learn_conversion();
        self.replace_preedit(&self.reading());
    }

    pub fn select(&mut self, candidate: &str) {
        if let Some((offset, _)) = self.shortcode() {
            self.committed.replace_range(offset..self.caret, "");
            self.caret = offset;
        } else {
            self.learn_choice(candidate);
        }
        self.replace_preedit(candidate);
    }

    /// Remembers a candidate that was picked for the typed Japanese reading.
    fn learn_choice(&mut self, text: &str) {
        if self.language != Language::Japanese || self.conversion.is_some() {
            return;
        }
        let typed = self.reading();
        if is_plain_form(&typed, text) {
            return;
        }
        if let Some(reading) = japanese::reading_of(&typed, text, &self.learned) {
            self.learned.learn(&reading, text);
        }
    }

    /// Remembers the clauses of a conversion that is about to be committed, and
    /// the whole sentence when there are several.
    fn learn_conversion(&mut self) {
        let Some(conversion) = &self.conversion else {
            return;
        };
        let (mut reading, mut text) = (String::new(), String::new());
        for segment in conversion.segments() {
            reading += &segment.reading;
            text += segment.text();
            if !is_plain_form(&segment.reading, segment.text()) {
                self.learned.learn(&segment.reading, segment.text());
            }
        }
        if conversion.segments().len() > 1 && !is_plain_form(&reading, &text) {
            self.learned.learn(&reading, &text);
        }
    }

    fn shortcode(&self) -> Option<(usize, &str)> {
        if self.language != Language::English {
            return None;
        }
        let end = self.committed[..self.caret].trim_end_matches(':');
        let (prefix, tag) = end.rsplit_once(':')?;
        let valid_start = prefix.chars().next_back().is_none_or(char::is_whitespace);
        (valid_start && tag.len() >= 2 && tag.bytes().all(|b| b.is_ascii_lowercase()))
            .then_some((prefix.len(), tag))
    }

    pub fn replace_preedit(&mut self, candidate: &str) {
        self.insert_committed(candidate);
        self.japanese_prefix.clear();
        self.reading.clear();
        self.conversion = None;
    }

    fn insert_committed(&mut self, text: &str) {
        self.committed.insert_str(self.caret, text);
        self.caret = self.boundary_at_or_after(self.caret + text.len());
    }

    fn boundary_at_or_after(&self, byte: usize) -> usize {
        if byte == self.committed.len() {
            return byte;
        }
        self.committed
            .grapheme_indices(true)
            .find_map(|(index, _)| (index >= byte).then_some(index))
            .unwrap_or(self.committed.len())
    }

    fn convert(&self, finalizing: bool) -> String {
        match self.language {
            Language::Japanese => {
                self.japanese_prefix.clone() + &japanese::kana(&self.reading, finalizing)
            }
            Language::Korean => korean::compose(&self.reading),
            Language::English => String::new(),
        }
    }
}

pub use emoji::{Emoji, search as search_emoji};
pub use korean::key_label as hangul_key_label;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_romaji_and_commits_selected_candidate() {
        let mut composer = Composer::default();
        "nihonngo".chars().for_each(|key| composer.key(key));
        assert_eq!(composer.preedit(), "にほんご");
        assert!(composer.candidates().contains(&"日本語".into()));
        composer.replace_preedit("日本語");
        "gakkou".chars().for_each(|key| composer.key(key));
        assert_eq!(composer.text(), "日本語がっこう");
    }

    #[test]
    fn composes_hangul_and_revises_the_final_consonant() {
        let mut composer = Composer::default();
        composer.set_language(Language::Korean);
        "dkssudgktpdy".chars().for_each(|key| composer.key(key));
        assert_eq!(composer.preedit(), "안녕하세요");
        composer.backspace();
        assert_eq!(composer.preedit(), "안녕하셍");
    }

    #[test]
    fn completes_each_romaji_spelling_after_correcting_a_typo() {
        for entry in include_str!("romaji.txt").split_ascii_whitespace() {
            let (roman, kana) = entry.split_once('=').unwrap();
            for split in 0..roman.len() {
                let mut composer = Composer::default();
                let (start, end) = roman.split_at(split);
                "kaki"
                    .chars()
                    .chain(start.chars())
                    .for_each(|k| composer.key(k));
                composer.key('p');
                composer.backspace();
                end.chars().for_each(|k| composer.key(k));
                assert_eq!(
                    composer.preedit(),
                    format!("かき{kana}"),
                    "{roman} at {split}"
                );
            }
        }
    }

    #[test]
    fn backspace_removes_one_displayed_japanese_character() {
        for (keys, remaining) in [
            ("kana", "か"),
            ("kya", "き"),
            ("kitte", "きっ"),
            ("kan.", "かん"),
            ("nnn", "nn"),
        ] {
            let mut composer = Composer::default();
            keys.chars().for_each(|key| composer.key(key));
            composer.backspace();
            assert_eq!(composer.preedit(), remaining, "{keys}");
            assert!(composer.has_preedit());
            assert_eq!(composer.committed(), "");
        }

        let mut composer = Composer::default();
        "kashit".chars().for_each(|key| composer.key(key));
        composer.backspace();
        assert_eq!(composer.preedit(), "かし");
        composer.backspace();
        "sht".chars().for_each(|key| composer.key(key));
        composer.backspace();
        composer.key('i');
        assert_eq!(composer.preedit(), "かし");
        assert!(composer.convert_reading(&[]));
        composer.backspace();
        assert_eq!(composer.preedit(), "かし");
        composer.backspace();
        composer.backspace();
        assert!(!composer.has_preedit());
    }

    #[test]
    fn backspace_removes_a_complete_emoji() {
        let mut composer = Composer::default();
        composer.insert("A👩‍💻");
        composer.backspace();
        assert_eq!(composer.text(), "A");
    }

    #[test]
    fn editing_at_the_caret_preserves_the_suffix_and_graphemes() {
        let mut composer = Composer::default();
        composer.insert("A👩‍💻B");
        composer.move_caret(4);
        composer.backspace();
        "kana".chars().for_each(|key| composer.key(key));
        assert_eq!(composer.text(), "AかなB");
        composer.commit();
        composer.insert("!");
        assert_eq!(composer.text(), "Aかな!B");
    }

    #[test]
    fn selecting_an_english_emoji_replaces_only_the_shortcode() {
        let mut composer = Composer::default();
        composer.set_language(Language::English);
        composer.insert("Hello :cat:");
        assert!(composer.candidates().contains(&"🐱".into()));
        composer.select("🐱");
        assert_eq!(composer.text(), "Hello 🐱");
    }

    fn type_keys(composer: &mut Composer, keys: &str) {
        keys.chars().for_each(|key| composer.key(key));
    }

    #[test]
    fn offers_as_many_homophones_as_a_desktop_ime() {
        let mut composer = Composer::default();
        type_keys(&mut composer, "kouen");
        let candidates = composer.candidates();
        for word in ["公園", "公演", "講演", "後援", "好演"] {
            assert!(
                candidates.contains(&word.to_owned()),
                "{word}: {candidates:?}"
            );
        }
        assert!(composer.convert_reading(&[]));
        let clause = composer.conversion().unwrap().focused();
        assert!(clause.candidates.len() >= 8, "{:?}", clause.candidates);
        assert_eq!(clause.candidates[0], "公園");
    }

    #[test]
    fn types_symbols_and_kaomoji_by_their_readings() {
        let mut composer = Composer::default();
        type_keys(&mut composer, "maru");
        let candidates = composer.candidates();
        for symbol in ["○", "●", "◎"] {
            assert!(
                candidates.contains(&symbol.to_owned()),
                "{symbol}: {candidates:?}"
            );
        }
        composer.select("●");
        assert_eq!(composer.text(), "●");

        type_keys(&mut composer, "nikoniko");
        assert!(composer.candidates().contains(&"(^_^)".to_owned()));
        assert!(composer.convert_reading(&[]));
        assert!(
            composer
                .conversion()
                .unwrap()
                .focused()
                .candidates
                .contains(&"(^_^)".to_owned())
        );
        composer.commit();

        type_keys(&mut composer, "yajirusi");
        assert!(composer.candidates().contains(&"→".to_owned()));
        composer.clear();
        type_keys(&mut composer, "hosi");
        assert!(composer.candidates().contains(&"★".to_owned()));
    }

    #[test]
    fn writes_dates() {
        let mut composer = Composer::default();
        type_keys(&mut composer, "kyou");
        let year = extras::now().unwrap().year.to_string();
        assert!(
            composer
                .candidates()
                .iter()
                .any(|candidate| candidate.starts_with(&year)),
            "{:?}",
            composer.candidates()
        );
    }

    #[test]
    fn predicts_longer_words_from_the_start_of_a_reading() {
        let mut composer = Composer::default();
        type_keys(&mut composer, "kouen");
        let exact = composer.candidates();
        composer.clear();
        type_keys(&mut composer, "kouenn");
        let candidates = composer.candidates();
        assert_eq!(candidates[0], "公園");
        assert!(exact.len() > 3);
        composer.clear();
        type_keys(&mut composer, "arigatou");
        let candidates = composer.candidates();
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate == "有難うございます"),
            "{candidates:?}"
        );
        // A prediction is a longer word than what was typed.
        assert!(japanese::predictions("あ", 8).len() == 8);
        assert!(
            japanese::predictions("きょうは", 8)
                .iter()
                .all(|(reading, _)| reading.len() > "きょうは".len())
        );
        assert!(japanese::predictions("abc", 8).is_empty());
        assert!(japanese::predictions("", 8).is_empty());
    }

    #[test]
    fn learns_the_candidate_that_was_chosen() {
        let mut composer = Composer::default();
        type_keys(&mut composer, "kouen");
        assert_eq!(composer.candidates()[0], "公園");
        composer.select("講演");
        assert_eq!(composer.text(), "講演");

        type_keys(&mut composer, "kouen");
        assert_eq!(composer.candidates()[0], "講演");
        assert!(composer.convert_reading(&[]));
        assert_eq!(
            composer.conversion().unwrap().focused().candidates[0],
            "講演"
        );
        composer.next_candidate();
        composer.pick_candidate("公演");
        assert_eq!(composer.committed(), "講演公演");

        type_keys(&mut composer, "kouen");
        assert_eq!(composer.candidates()[0], "公演");
    }

    #[test]
    fn learns_a_symbol_and_the_clauses_of_a_conversion() {
        let mut composer = Composer::default();
        type_keys(&mut composer, "neko");
        composer.select("(=^・^=)");
        type_keys(&mut composer, "neko");
        assert_eq!(composer.candidates()[0], "(=^・^=)");
        composer.clear();

        type_keys(&mut composer, "kyouhaiitenki");
        assert!(composer.convert_reading(&[]));
        composer.commit();
        let sentence = composer.committed().to_owned();
        assert!(!sentence.is_empty());
        // The sentence is offered again from its first letters.
        composer.clear();
        type_keys(&mut composer, "kyouhaii");
        assert!(composer.candidates().contains(&sentence));
        // It also leads as one clause when the whole reading is converted again.
        composer.clear();
        type_keys(&mut composer, "kyouhaiitenki");
        assert!(composer.convert_reading(&[]));
        assert_eq!(composer.conversion().unwrap().segments().len(), 1);
        assert_eq!(composer.text(), sentence);
    }

    #[test]
    fn learns_a_predicted_word_under_its_own_reading() {
        let mut composer = Composer::default();
        type_keys(&mut composer, "kouenn");
        composer.select("公園");
        type_keys(&mut composer, "kouenn");
        assert_eq!(composer.candidates()[0], "公園");
        // The typed reading is not a reading of the word, so it only predicts it.
        composer.clear();
        type_keys(&mut composer, "kou");
        assert!(composer.candidates().contains(&"公園".to_owned()));
        assert!(
            composer
                .learned
                .exact("こう")
                .iter()
                .all(|text| *text != "公園")
        );
    }

    #[test]
    fn does_not_learn_plain_kana_computed_text_or_when_off() {
        let mut composer = Composer::default();
        type_keys(&mut composer, "kyou");
        composer.select("キョウ");
        type_keys(&mut composer, "kyou");
        composer.select("きょう");
        type_keys(&mut composer, "kyou");
        let date = composer
            .candidates()
            .into_iter()
            .find(|candidate| candidate.contains('/'))
            .unwrap();
        composer.select(&date);
        assert!(composer.learned.is_empty());

        composer.learned.set_enabled(false);
        type_keys(&mut composer, "kouen");
        composer.select("講演");
        composer.learned.set_enabled(true);
        assert!(composer.learned.is_empty());
        type_keys(&mut composer, "kouen");
        assert_eq!(composer.candidates()[0], "公園");
    }

    #[test]
    fn keeps_learned_words_between_sessions() {
        let directory = tempfile::tempdir().unwrap();
        let mut composer = Composer::default();
        composer.configure_learning(true, directory.path());
        type_keys(&mut composer, "kouen");
        composer.select("講演");
        assert!(composer.learning_pending());
        composer.learned.save();
        assert!(!composer.learning_pending());

        let mut next = Composer::default();
        next.configure_learning(true, directory.path());
        assert_eq!(next.learned_count(), 1);
        type_keys(&mut next, "kouen");
        assert_eq!(next.candidates()[0], "講演");
        next.configure_learning(false, directory.path());
        assert_eq!(next.candidates()[0], "公園");
        next.configure_learning(true, directory.path());
        next.forget_learned().unwrap();
        assert_eq!(next.learned_count(), 0);
        assert_eq!(next.candidates()[0], "公園");
        assert!(!directory.path().join(learning::FILE).exists());
    }

    #[test]
    fn all_three_languages_can_share_one_message() {
        let mut composer = Composer::default();
        "pan".chars().for_each(|key| composer.key(key));
        assert_eq!(composer.preedit(), "ぱn");
        composer.set_language(Language::English);
        composer.key(' ');
        composer.key('A');
        composer.set_language(Language::Korean);
        "rhk".chars().for_each(|key| composer.key(key));
        assert_eq!(composer.text(), "ぱん A과");
        assert_eq!(search_emoji("cat").next().unwrap().symbol, "🐱");
    }
}
