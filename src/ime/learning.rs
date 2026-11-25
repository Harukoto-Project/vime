//! Words the user chose, kept on this PC to rank and predict later conversions.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const FILE: &str = "learned.tsv";
const MAX_ENTRIES: usize = if cfg!(test) { 40 } else { 20_000 };
const MAX_READING_CHARS: usize = 48;
const MAX_TEXT_CHARS: usize = 64;
/// How long a change waits before it is written, so that a burst of choices is
/// saved once.
const SAVE_DELAY: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Eq, PartialEq)]
struct Entry {
    reading: String,
    text: String,
    count: u32,
    last: u64,
}

pub struct Learned {
    entries: Vec<Entry>,
    clock: u64,
    enabled: bool,
    path: Option<PathBuf>,
    dirty_since: Option<Instant>,
}

impl Default for Learned {
    /// Learns in memory only. `load` makes the words persist.
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            clock: 0,
            enabled: true,
            path: None,
            dirty_since: None,
        }
    }
}

impl Learned {
    /// Reads the words saved in `directory`. A missing or damaged file is an
    /// empty vocabulary; the next save replaces it.
    pub fn load(directory: &Path) -> Self {
        let path = directory.join(FILE);
        let mut learned = Self {
            path: Some(path.clone()),
            ..Self::default()
        };
        let Ok(text) = fs::read_to_string(&path) else {
            return learned;
        };
        for line in text.lines() {
            let mut fields = line.split('\t');
            let (Some(reading), Some(word), Some(count), Some(last)) = (
                fields.next(),
                fields.next(),
                fields.next().and_then(|count| count.parse().ok()),
                fields.next().and_then(|last| last.parse().ok()),
            ) else {
                continue;
            };
            if valid(reading, word) && count > 0 {
                learned.clock = learned.clock.max(last);
                learned.entries.push(Entry {
                    reading: reading.to_owned(),
                    text: word.to_owned(),
                    count,
                    last,
                });
            }
        }
        learned.trim();
        learned
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Where the words are saved, if they are.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Remembers that `reading` was written as `text`. Choosing a word again
    /// ranks it higher, and the other words of the same reading lose a little.
    pub fn learn(&mut self, reading: &str, text: &str) {
        if !self.enabled || !valid(reading, text) {
            return;
        }
        self.clock += 1;
        let mut found = false;
        for entry in self
            .entries
            .iter_mut()
            .filter(|entry| entry.reading == reading)
        {
            if entry.text == text {
                entry.count = entry.count.saturating_add(1);
                entry.last = self.clock;
                found = true;
            } else {
                entry.count = (entry.count * 3 / 4).max(1);
            }
        }
        if !found {
            self.entries.push(Entry {
                reading: reading.to_owned(),
                text: text.to_owned(),
                count: 1,
                last: self.clock,
            });
            self.trim();
        }
        self.dirty_since.get_or_insert_with(Instant::now);
    }

    /// Words written for exactly `reading`, the best first.
    pub fn exact(&self, reading: &str) -> Vec<&str> {
        if !self.enabled {
            return Vec::new();
        }
        let mut found: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|entry| entry.reading == reading)
            .collect();
        found.sort_by_key(|entry| std::cmp::Reverse(rank(entry)));
        found.into_iter().map(|entry| entry.text.as_str()).collect()
    }

    /// Words of longer readings that start with `prefix` as `(reading, text)`,
    /// the best first.
    pub fn prefix(&self, prefix: &str, limit: usize) -> Vec<(&str, &str)> {
        if !self.enabled || prefix.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|entry| entry.reading.len() > prefix.len() && entry.reading.starts_with(prefix))
            .collect();
        found.sort_by_key(|entry| std::cmp::Reverse(rank(entry)));
        let mut words: Vec<(&str, &str)> = Vec::new();
        for entry in found {
            if words.len() >= limit {
                break;
            }
            if !words.iter().any(|(_, text)| *text == entry.text) {
                words.push((&entry.reading, &entry.text));
            }
        }
        words
    }

    /// Forgets everything, including the saved file.
    pub fn clear(&mut self) -> io::Result<()> {
        self.entries.clear();
        self.dirty_since = None;
        match &self.path {
            Some(path) => match fs::remove_file(path) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            },
            None => Ok(()),
        }
    }

    /// Whether there are changes that were not saved yet.
    pub fn pending(&self) -> bool {
        self.dirty_since.is_some()
    }

    /// Saves the changes once they have waited long enough.
    pub fn save_when_due(&mut self) {
        if self
            .dirty_since
            .is_some_and(|since| since.elapsed() >= SAVE_DELAY)
        {
            self.save();
        }
    }

    /// Saves the changes now. A failure keeps them pending for the next try.
    pub fn save(&mut self) {
        let Some(path) = &self.path else {
            self.dirty_since = None;
            return;
        };
        if self.dirty_since.is_none() {
            return;
        }
        let mut text = String::new();
        for entry in &self.entries {
            text.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                entry.reading, entry.text, entry.count, entry.last
            ));
        }
        let temporary = path.with_extension("tmp");
        let written = path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(&temporary, text))
            .and_then(|()| fs::rename(&temporary, path));
        if written.is_ok() {
            self.dirty_since = None;
        } else {
            // Try again later instead of on every choice.
            self.dirty_since = Some(Instant::now());
        }
    }

    fn trim(&mut self) {
        if self.entries.len() > MAX_ENTRIES {
            self.entries
                .sort_by_key(|entry| std::cmp::Reverse(rank(entry)));
            self.entries.truncate(MAX_ENTRIES);
        }
    }
}

/// Frequent words first, the more recent first among equals.
fn rank(entry: &Entry) -> (u32, u64) {
    (entry.count, entry.last)
}

fn valid(reading: &str, text: &str) -> bool {
    !reading.is_empty()
        && !text.is_empty()
        && reading.chars().count() <= MAX_READING_CHARS
        && text.chars().count() <= MAX_TEXT_CHARS
        && reading.chars().all(super::japanese::is_reading_char)
        && !text.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_the_word_chosen_most_and_most_recently_first() {
        let mut learned = Learned::default();
        for _ in 0..3 {
            learned.learn("かみ", "紙");
        }
        learned.learn("かみ", "神");
        assert_eq!(learned.exact("かみ"), ["紙", "神"]);
        // Each new choice also wears down the older ones.
        for _ in 0..3 {
            learned.learn("かみ", "神");
        }
        assert_eq!(learned.exact("かみ"), ["神", "紙"]);
        assert!(learned.exact("かめ").is_empty());
    }

    #[test]
    fn predicts_longer_readings_from_a_prefix() {
        let mut learned = Learned::default();
        learned.learn("おつかれさまです", "お疲れ様です");
        learned.learn("おつかれ", "お疲れ");
        learned.learn("おつかれ", "お疲れ");
        assert_eq!(
            learned.prefix("おつ", 5),
            [("おつかれ", "お疲れ"), ("おつかれさまです", "お疲れ様です")]
        );
        assert_eq!(learned.prefix("おつかれ", 5).len(), 1);
        assert!(learned.prefix("", 5).is_empty());
    }

    #[test]
    fn ignores_unsuitable_words_and_a_disabled_vocabulary() {
        let mut learned = Learned::default();
        learned.learn("", "あ");
        learned.learn("abc", "あ");
        learned.learn("あ", "a\tb");
        assert!(learned.is_empty());
        learned.set_enabled(false);
        learned.learn("あ", "亜");
        assert!(learned.is_empty());
        learned.set_enabled(true);
        learned.learn("あ", "亜");
        learned.set_enabled(false);
        assert!(learned.exact("あ").is_empty());
    }

    #[test]
    fn keeps_the_most_used_words_when_full() {
        let mut learned = Learned::default();
        learned.learn("あ", "亜");
        learned.learn("あ", "亜");
        for index in 0..MAX_ENTRIES + 10 {
            let reading: String = (0..4)
                .map(|digit| {
                    char::from_u32('あ' as u32 + (index >> (digit * 4) & 15) as u32 + 1).unwrap()
                })
                .collect();
            learned.learn(&reading, "語");
        }
        assert_eq!(learned.len(), MAX_ENTRIES);
        assert_eq!(learned.exact("あ"), ["亜"]);
    }

    #[test]
    fn saves_loads_and_clears_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let mut learned = Learned::load(directory.path());
        assert!(learned.is_empty());
        learned.learn("ぶいえむ", "ViME");
        learned.learn("ぶいえむ", "ViME");
        learned.learn("ねこ", "(=^・^=)");
        assert!(learned.pending());
        learned.save();
        assert!(!learned.pending());

        let mut reloaded = Learned::load(directory.path());
        assert_eq!(reloaded.len(), 2);
        assert_eq!(reloaded.exact("ぶいえむ"), ["ViME"]);
        assert_eq!(reloaded.exact("ねこ"), ["(=^・^=)"]);
        reloaded.learn("ねこ", "猫");
        assert_eq!(reloaded.exact("ねこ"), ["猫", "(=^・^=)"]);

        reloaded.clear().unwrap();
        assert!(Learned::load(directory.path()).is_empty());
        assert!(reloaded.is_empty());
        reloaded.clear().unwrap();
    }

    #[test]
    fn skips_damaged_lines_when_loading() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join(FILE),
            "ねこ\t猫\t2\t7\nbroken\nあ\t亜\tx\t1\nabc\t亜\t1\t1\nいぬ\t犬\t1\t3\n",
        )
        .unwrap();
        let mut learned = Learned::load(directory.path());
        assert_eq!(learned.len(), 2);
        learned.learn("いぬ", "犬");
        assert_eq!(learned.exact("いぬ"), ["犬"]);
        assert_eq!(learned.clock, 8);
    }
}
