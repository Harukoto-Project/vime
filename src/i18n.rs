//! Interface translations. Keys are the Japanese source texts; `i18n.tsv` holds
//! the English and Korean versions as `japanese<TAB>english<TAB>korean`.

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        LazyLock,
        atomic::{AtomicU8, Ordering},
    },
};

const TABLE: &str = include_str!("i18n.tsv");
static CURRENT: AtomicU8 = AtomicU8::new(Lang::Ja as u8);
static TRANSLATIONS: LazyLock<HashMap<&'static str, [&'static str; 2]>> = LazyLock::new(|| {
    TABLE
        .lines()
        .filter_map(|line| {
            let mut columns = line.split('\t');
            Some((columns.next()?, [columns.next()?, columns.next()?]))
        })
        .collect()
});

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    Ja,
    En,
    Ko,
}

impl Default for Lang {
    fn default() -> Self {
        Self::system()
    }
}

impl Lang {
    pub const ALL: [Self; 3] = [Self::Ja, Self::En, Self::Ko];

    /// The language's own name, shown in the language picker.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ja => "日本語",
            Self::En => "English",
            Self::Ko => "한국어",
        }
    }

    pub fn from_tag(tag: &str) -> Self {
        let tag = tag.to_ascii_lowercase();
        if tag.starts_with("ja") {
            Self::Ja
        } else if tag.starts_with("ko") {
            Self::Ko
        } else {
            Self::En
        }
    }

    /// The language of the Windows user profile.
    pub fn system() -> Self {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
            let mut name = [0u16; 85];
            let length = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
            if length > 1 {
                return Self::from_tag(&String::from_utf16_lossy(&name[..length as usize - 1]));
            }
        }
        std::env::var("LANG").map_or(Self::En, |tag| Self::from_tag(&tag))
    }
}

pub fn set_language(language: Lang) {
    CURRENT.store(language as u8, Ordering::Relaxed);
}

pub fn language() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        0 => Lang::Ja,
        1 => Lang::En,
        _ => Lang::Ko,
    }
}

/// Translates a Japanese source text. Unknown texts are returned unchanged.
pub fn tr(key: &str) -> &str {
    let column = match language() {
        Lang::Ja => return key,
        Lang::En => 0,
        Lang::Ko => 1,
    };
    TRANSLATIONS
        .get(key)
        .map_or(key, |translations| translations[column])
}

/// Translates a text with `{0}`, `{1}`... placeholders and fills them in.
pub fn tr_args(key: &str, args: &[&str]) -> String {
    args.iter()
        .enumerate()
        .fold(tr(key).to_owned(), |text, (index, value)| {
            text.replace(&format!("{{{index}}}"), value)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashSet, fs, path::Path};

    fn placeholders(text: &str) -> Vec<&str> {
        let mut found: Vec<&str> = text
            .match_indices('{')
            .filter_map(|(start, _)| {
                text[start..]
                    .find('}')
                    .map(|end| &text[start..=start + end])
            })
            .collect();
        found.sort_unstable();
        found
    }

    #[test]
    fn every_translation_is_complete_and_unique() {
        let mut keys = HashSet::new();
        for line in TABLE.lines() {
            let columns: Vec<_> = line.split('\t').collect();
            assert_eq!(columns.len(), 3, "three columns expected: {line}");
            assert!(columns.iter().all(|column| !column.trim().is_empty()));
            assert!(keys.insert(columns[0]), "duplicate key: {line}");
            for translation in &columns[1..] {
                assert_eq!(
                    placeholders(columns[0]),
                    placeholders(translation),
                    "placeholders differ: {line}"
                );
            }
        }
    }

    fn sources(directory: &Path, found: &mut Vec<String>) {
        for entry in fs::read_dir(directory).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                sources(&path, found);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && path.file_name().is_some_and(|name| name != "i18n.rs")
            {
                found.push(fs::read_to_string(path).unwrap());
            }
        }
    }

    #[test]
    fn every_key_used_by_the_code_is_translated() {
        let mut files = Vec::new();
        sources(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut used = 0;
        for source in &files {
            for call in ["tr(\"", "tr_args(\""] {
                for (start, _) in source.match_indices(call) {
                    let boundary = source[..start]
                        .chars()
                        .next_back()
                        .is_none_or(|c| !c.is_alphanumeric() && c != '_');
                    if !boundary {
                        continue;
                    }
                    let rest = &source[start + call.len()..];
                    let key = &rest[..rest.find('"').unwrap()];
                    assert!(
                        TRANSLATIONS.contains_key(key),
                        "missing translation for {key:?}"
                    );
                    used += 1;
                }
            }
        }
        assert!(used > 50, "only {used} translated texts were found");
    }

    #[test]
    fn fills_placeholders_in_each_language() {
        set_language(Lang::En);
        assert_eq!(
            tr_args("設定を保存できません ({0})", &["disk full"]),
            "Could not save settings (disk full)"
        );
        set_language(Lang::Ja);
        assert_eq!(
            tr_args("設定を保存できません ({0})", &["x"]),
            "設定を保存できません (x)"
        );
        assert_eq!(Lang::from_tag("ko-KR"), Lang::Ko);
        assert_eq!(Lang::from_tag("ja-JP"), Lang::Ja);
        assert_eq!(Lang::from_tag("de-DE"), Lang::En);
    }
}
