//! Kanji misconversions learned from the Japanese Wikipedia Typo Dataset.
//!
//! The dataset (JWTD v2, Kyoto University Language Media Processing Lab,
//! CC BY-SA 3.0) is downloaded by the user and boiled down on their PC to a
//! table of words that editors replaced by another kanji word. No sentences
//! are kept. The table is not part of ViME and is never redistributed.

use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
    fs,
    ops::Range,
    path::Path,
};

use serde::Deserialize;

const MIN_LENGTH: usize = 2;
const MAX_LENGTH: usize = 6;
const MAX_FIXES: usize = 3;
/// Entries below this share of corrected uses are not stored.
const STORE_RATIO: f64 = 0.1;
/// A word is only questioned when this share of its uses was corrected.
const MIN_RATIO: f64 = 0.2;
/// How far before the caret a questioned word may end.
const REACH: usize = 4;
pub const FILE: &str = "misconversions.tsv";
const CATEGORIES: [&str; 2] = ["kanji-conversion_a", "kanji-conversion_b"];

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    wrong: u32,
    kept: u32,
    fixes: Vec<String>,
}

impl Entry {
    fn ratio(&self) -> f64 {
        f64::from(self.wrong) / f64::from(self.wrong + self.kept)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    entries: HashMap<String, Entry>,
}

fn kanji(ch: char) -> bool {
    matches!(ch, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '々' | '〆')
}

fn usable(word: &str) -> bool {
    (MIN_LENGTH..=MAX_LENGTH).contains(&word.chars().count())
        && word.chars().any(kanji)
        && !word.chars().any(char::is_whitespace)
}

impl Table {
    /// Reads `word<TAB>wrong<TAB>kept<TAB>fix fix fix` lines; others are skipped.
    pub fn parse(text: &str) -> Self {
        let entries = text
            .lines()
            .filter_map(|line| {
                let mut fields = line.split('\t');
                let word = fields.next().filter(|word| usable(word))?;
                let wrong: u32 = fields.next()?.parse().ok().filter(|wrong| *wrong > 0)?;
                let kept = fields.next()?.parse().ok()?;
                let fixes: Vec<_> = fields
                    .next()?
                    .split(' ')
                    .filter(|fix| usable(fix))
                    .take(MAX_FIXES)
                    .map(str::to_owned)
                    .collect();
                (!fixes.is_empty()).then(|| (word.to_owned(), Entry { wrong, kept, fixes }))
            })
            .collect();
        Self { entries }
    }

    /// The table saved in directory, if there is a usable one.
    pub fn load(directory: &Path) -> Option<Self> {
        let table = Self::parse(&fs::read_to_string(directory.join(FILE)).ok()?);
        (!table.is_empty()).then_some(table)
    }

    pub fn serialize(&self) -> String {
        let mut words: Vec<_> = self.entries.iter().collect();
        words.sort_unstable_by(|a, b| a.0.cmp(b.0));
        let mut text = String::new();
        for (word, entry) in words {
            let _ = writeln!(
                text,
                "{word}\t{}\t{}\t{}",
                entry.wrong,
                entry.kept,
                entry.fixes.join(" ")
            );
        }
        text
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The word near the end of `text` that is most often a misconversion,
    /// with what editors replaced it by.
    pub fn find(&self, text: &str) -> Option<(Range<usize>, &[String])> {
        let offsets: Vec<_> = text
            .char_indices()
            .map(|(offset, _)| offset)
            .chain([text.len()])
            .collect();
        let count = offsets.len() - 1;
        let mut best: Option<(f64, usize, Range<usize>, &Entry)> = None;
        for end in count.saturating_sub(REACH)..=count {
            for length in MIN_LENGTH..=MAX_LENGTH.min(end) {
                let range = offsets[end - length]..offsets[end];
                let Some(entry) = self.entries.get(&text[range.clone()]) else {
                    continue;
                };
                let ratio = entry.ratio();
                if ratio >= MIN_RATIO
                    && best
                        .as_ref()
                        .is_none_or(|(top, long, ..)| (ratio, length) > (*top, *long))
                {
                    best = Some((ratio, length, range, entry));
                }
            }
        }
        best.map(|(_, _, range, entry)| (range, entry.fixes.as_slice()))
    }
}

#[derive(Deserialize)]
struct Revision {
    #[serde(default)]
    diffs: Vec<Difference>,
}

#[derive(Deserialize)]
struct Difference {
    #[serde(default)]
    pre_str: String,
    #[serde(default)]
    post_str: String,
    #[serde(default)]
    category: String,
}

#[derive(Deserialize)]
struct Sentence {
    #[serde(default)]
    post_text: String,
}

/// Counts, over JWTD `train.jsonl`, how often each word was corrected (first
/// pass) and how often it still appears in corrected sentences (second pass).
#[derive(Default)]
pub struct Tally {
    corrections: HashMap<String, HashMap<String, u32>>,
    kept: HashMap<String, u32>,
    lengths: Vec<usize>,
}

impl Tally {
    pub fn count_corrections(&mut self, line: &str) {
        let Ok(revision) = serde_json::from_str::<Revision>(line) else {
            return;
        };
        for diff in revision.diffs {
            if CATEGORIES.contains(&diff.category.as_str())
                && diff.pre_str != diff.post_str
                && usable(&diff.pre_str)
                && usable(&diff.post_str)
            {
                *self
                    .corrections
                    .entry(diff.pre_str)
                    .or_default()
                    .entry(diff.post_str)
                    .or_default() += 1;
            }
        }
    }

    pub fn corrected_words(&self) -> usize {
        self.corrections.len()
    }

    /// Starts the second pass, which counts only the corrected words.
    pub fn start_second_pass(&mut self) {
        let lengths: HashSet<_> = self
            .corrections
            .keys()
            .map(|word| word.chars().count())
            .collect();
        self.lengths = lengths.into_iter().collect();
        self.lengths.sort_unstable();
        self.kept = self
            .corrections
            .keys()
            .map(|word| (word.clone(), 0))
            .collect();
    }

    pub fn count_kept(&mut self, line: &str) {
        let Ok(sentence) = serde_json::from_str::<Sentence>(line) else {
            return;
        };
        let text = &sentence.post_text;
        let offsets: Vec<_> = text
            .char_indices()
            .map(|(offset, _)| offset)
            .chain([text.len()])
            .collect();
        let count = offsets.len() - 1;
        for start in 0..count {
            for &length in &self.lengths {
                if start + length > count {
                    break;
                }
                if let Some(kept) = self
                    .kept
                    .get_mut(&text[offsets[start]..offsets[start + length]])
                {
                    *kept += 1;
                }
            }
        }
    }

    pub fn table(&self) -> Table {
        let entries = self
            .corrections
            .iter()
            .filter_map(|(word, fixes)| {
                let wrong: u32 = fixes.values().sum();
                let kept = self.kept.get(word).copied().unwrap_or(0);
                let entry = Entry {
                    wrong,
                    kept,
                    fixes: Vec::new(),
                };
                if entry.ratio() < STORE_RATIO {
                    return None;
                }
                let mut ranked: Vec<_> = fixes.iter().collect();
                // Equal counts are ordered by text so the table is reproducible.
                ranked.sort_unstable_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
                let fixes = ranked
                    .into_iter()
                    .take(MAX_FIXES)
                    .map(|(fix, _)| fix.clone())
                    .collect();
                Some((word.clone(), Entry { fixes, ..entry }))
            })
            .collect();
        Table { entries }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(post_text: &str, pre: &str, post: &str, category: &str) -> String {
        format!(
            r#"{{"page":"1","post_text":"{post_text}","diffs":[{{"pre_str":"{pre}","post_str":"{post}","pre_bart_likelihood":-1.5,"category":"{category}"}}]}}"#
        )
    }

    fn tally(lines: &[String]) -> Table {
        let mut tally = Tally::default();
        for line in lines {
            tally.count_corrections(line);
        }
        tally.start_second_pass();
        for line in lines {
            tally.count_kept(line);
        }
        tally.table()
    }

    #[test]
    fn counts_corrections_against_the_uses_that_stay() {
        let mut lines = vec![
            line("それは意義がある。", "異義", "意義", "kanji-conversion_a"),
            line("意義を述べる。", "異義", "意義", "kanji-conversion_a"),
            line("別の意義に変える。", "異義", "意味", "kanji-conversion_b"),
            // Other kinds of mistakes and one-character or kana words are not
            // misconversions of a kanji word.
            line("彼は来る。", "きた", "来た", "kanji-conversion_a"),
            line("私が行く。", "行っ", "行く", "deletion"),
        ];
        // `異義` stays in four more sentences: 3 corrections against 4 uses.
        lines.extend((0..4).map(|_| line("異義語を調べる。", "調ベ", "調べ", "substitution")));
        let table = tally(&lines);
        assert_eq!(table.len(), 1);
        let entry = &table.entries["異義"];
        assert_eq!((entry.wrong, entry.kept), (3, 4));
        assert_eq!(entry.fixes, ["意義", "意味"].map(String::from));
    }

    #[test]
    fn leaves_out_words_that_are_rarely_a_mistake() {
        let mut lines: Vec<_> = (0..30)
            .map(|_| line("以外の人もいる。", "以外", "意外", "kanji-conversion_a"))
            .collect();
        lines.push(line("意外だった。", "以外", "意外", "kanji-conversion_a"));
        // 31 corrections against 30 uses that stayed: about half of its uses
        // were mistakes, so it is kept.
        assert!(tally(&lines).entries.contains_key("以外"));
        lines.extend((0..400).map(|_| line("以外にも", "x", "y", "other")));
        assert!(!tally(&lines).entries.contains_key("以外"));
    }

    #[test]
    fn survives_a_round_trip_and_bad_lines() {
        let lines = [
            line("それは意義がある。", "異義", "意義", "kanji-conversion_a"),
            "not json".to_owned(),
            line("意義を述べる。", "異義", "意義", "kanji-conversion_a"),
        ];
        let table = tally(&lines);
        assert_eq!(Table::parse(&table.serialize()), table);
        assert_eq!(table.serialize(), "異義\t2\t0\t意義\n");
        let damaged = "異義\t0\t0\t意義\nabc\t1\t0\tdef\n異義\tx\t0\t意義\n同音\t3\t1\t \n";
        assert!(Table::parse(damaged).is_empty());
    }

    #[test]
    fn finds_the_likeliest_word_near_the_caret() {
        let table = Table::parse("異義\t8\t2\t意義 異議\n以外\t1\t40\t意外\n外来\t5\t5\t外来語\n");
        let text = "それは異義";
        let (range, fixes) = table.find(text).unwrap();
        assert_eq!(&text[range], "異義");
        assert_eq!(fixes, ["意義", "異議"].map(String::from));
        // A word that is mostly used correctly is not questioned.
        assert!(table.find("彼以外").is_none());
        // Words ending a few characters before the caret still count, but
        // earlier ones do not.
        assert!(table.find("異義がある").is_some());
        assert!(table.find("異義がありますよね").is_none());
        assert!(table.find("").is_none());
        assert!(table.find("異").is_none());
        // The most often corrected word wins, then the longest.
        let table = Table::parse("異義\t8\t2\t意義\n異義語\t8\t2\t意義語\n");
        assert_eq!(table.find("異義語").unwrap().0, 0.."異義語".len());
        let table = Table::parse("異義\t8\t2\t意義\n異義語\t7\t3\t意義語\n");
        assert_eq!(table.find("異義語").unwrap().0, 0.."異義".len());
    }
}
