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
        self.committed.push_str(text);
    }

    pub fn backspace(&mut self) {
        if self.reading.pop().is_none()
            && let Some((index, _)) = self.committed.grapheme_indices(true).next_back()
        {
            self.committed.truncate(index);
        }
    }

    pub fn clear(&mut self) {
        self.committed.clear();
        self.reading.clear();
    }

    pub fn committed(&self) -> &str {
        &self.committed
    }

    pub fn preedit(&self) -> String {
        self.convert(false)
    }

    pub fn reading(&self) -> String {
        self.convert(true)
    }

    pub fn text(&self) -> String {
        self.committed.clone() + &self.reading()
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
            self.committed.truncate(offset);
        }
        self.replace_preedit(candidate);
    }

    fn shortcode(&self) -> Option<(usize, &str)> {
        if self.language != Language::English {
            return None;
        }
        let end = self.committed.trim_end_matches(':');
        let (prefix, tag) = end.rsplit_once(':')?;
        let valid_start = prefix.chars().next_back().is_none_or(char::is_whitespace);
        (valid_start && tag.len() >= 2 && tag.bytes().all(|b| b.is_ascii_lowercase()))
            .then_some((prefix.len(), tag))
    }

    pub fn replace_preedit(&mut self, candidate: &str) {
        self.committed.push_str(candidate);
        self.reading.clear();
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
