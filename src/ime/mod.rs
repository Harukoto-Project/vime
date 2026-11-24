mod conversion;
mod emoji;
mod japanese;
mod korean;
pub mod misconversion;

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
        self.conversion = Conversion::new(japanese::segments(&reading, hints));
        self.conversion.is_some()
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
        if self.language == Language::Japanese {
            candidates.extend(japanese::dictionary(&reading).map(str::to_owned));
        }
        candidates.extend(emoji::lookup(&reading).map(|entry| entry.symbol.to_owned()));
        candidates.push(reading.clone());
        if self.language == Language::Japanese {
            let katakana = japanese::katakana(&reading);
            if !candidates.contains(&katakana) {
                candidates.push(katakana.clone());
            }
            let halfwidth = japanese::halfwidth(&katakana);
            if !candidates.contains(&halfwidth) {
                candidates.push(halfwidth);
            }
        }
        candidates
    }

    pub fn commit(&mut self) {
        self.replace_preedit(&self.reading());
    }

    pub fn select(&mut self, candidate: &str) {
        if let Some((offset, _)) = self.shortcode() {
            self.committed.replace_range(offset..self.caret, "");
            self.caret = offset;
        }
        self.replace_preedit(candidate);
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
