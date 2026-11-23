mod emoji;
mod japanese;
mod korean;

use unicode_segmentation::UnicodeSegmentation;

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
    reading: String,
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
        match self.language {
            Language::Japanese
                if key.is_ascii_alphabetic()
                    || key == '-'
                    || (key == '\'' && self.reading.ends_with('n')) =>
            {
                self.reading.push(key.to_ascii_lowercase());
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
        self.reading.clear();
        self.caret = 0;
    }

    pub fn committed(&self) -> &str {
        &self.committed
    }

    pub fn committed_parts(&self) -> (&str, &str) {
        self.committed.split_at(self.caret)
    }

    pub fn has_preedit(&self) -> bool {
        !self.reading.is_empty()
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

    pub fn preedit(&self) -> String {
        self.convert(false)
    }

    pub fn reading(&self) -> String {
        self.convert(true)
    }

    pub fn text(&self) -> String {
        let (before, after) = self.committed_parts();
        before.to_owned() + &self.reading() + after
    }

    pub fn candidates(&self) -> Vec<String> {
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
                candidates.push(katakana);
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
        self.reading.clear();
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
            Language::Japanese => japanese::kana(&self.reading, finalizing),
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
