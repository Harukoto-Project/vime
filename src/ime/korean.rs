const KEYS: &str = "rRseEfaqQtTdwWczxvgkoiOjpuPhynbml";
const JAMO: &str = "ㄱㄲㄴㄷㄸㄹㅁㅂㅃㅅㅆㅇㅈㅉㅊㅋㅌㅍㅎㅏㅐㅑㅒㅓㅔㅕㅖㅗㅛㅜㅠㅡㅣ";
const ONSETS: &str = "ㄱㄲㄴㄷㄸㄹㅁㅂㅃㅅㅆㅇㅈㅉㅊㅋㅌㅍㅎ";
const VOWELS: &str = "ㅏㅐㅑㅒㅓㅔㅕㅖㅗㅘㅙㅚㅛㅜㅝㅞㅟㅠㅡㅢㅣ";
const CODAS: &str = " ㄱㄲㄳㄴㄵㄶㄷㄹㄺㄻㄼㄽㄾㄿㅀㅁㅂㅄㅅㅆㅇㅈㅊㅋㅌㅍㅎ";
const CLUSTERS: &[(char, char, char)] = &[
    ('ㄱ', 'ㅅ', 'ㄳ'),
    ('ㄴ', 'ㅈ', 'ㄵ'),
    ('ㄴ', 'ㅎ', 'ㄶ'),
    ('ㄹ', 'ㄱ', 'ㄺ'),
    ('ㄹ', 'ㅁ', 'ㄻ'),
    ('ㄹ', 'ㅂ', 'ㄼ'),
    ('ㄹ', 'ㅅ', 'ㄽ'),
    ('ㄹ', 'ㅌ', 'ㄾ'),
    ('ㄹ', 'ㅍ', 'ㄿ'),
    ('ㄹ', 'ㅎ', 'ㅀ'),
    ('ㅂ', 'ㅅ', 'ㅄ'),
];
const DIPHTHONGS: &[(char, char, char)] = &[
    ('ㅗ', 'ㅏ', 'ㅘ'),
    ('ㅗ', 'ㅐ', 'ㅙ'),
    ('ㅗ', 'ㅣ', 'ㅚ'),
    ('ㅜ', 'ㅓ', 'ㅝ'),
    ('ㅜ', 'ㅔ', 'ㅞ'),
    ('ㅜ', 'ㅣ', 'ㅟ'),
    ('ㅡ', 'ㅣ', 'ㅢ'),
];

pub fn key_label(key: char) -> Option<char> {
    let canonical = if "REQTWOP".contains(key) {
        key
    } else {
        key.to_ascii_lowercase()
    };
    KEYS.find(canonical)
        .and_then(|index| JAMO.chars().nth(index))
}

fn join(table: &[(char, char, char)], a: char, b: char) -> Option<char> {
    table
        .iter()
        .find(|&&(left, right, _)| (a, b) == (left, right))
        .map(|entry| entry.2)
}

#[derive(Default)]
struct Syllable {
    onset: Option<char>,
    vowel: Option<char>,
    coda: Option<char>,
}

impl Syllable {
    fn flush(&mut self, text: &mut String) {
        match (self.onset.take(), self.vowel.take(), self.coda.take()) {
            (Some(onset), Some(vowel), coda) => {
                let l = ONSETS.chars().position(|c| c == onset).unwrap();
                let v = VOWELS.chars().position(|c| c == vowel).unwrap();
                let t = coda
                    .and_then(|coda| CODAS.chars().position(|c| c == coda))
                    .unwrap_or(0);
                text.push(char::from_u32(0xac00 + ((l * 21 + v) * 28 + t) as u32).unwrap());
            }
            (onset, vowel, _) => text.extend(onset.into_iter().chain(vowel)),
        }
    }
}

pub fn compose(keys: &str) -> String {
    let mut text = String::new();
    let mut syllable = Syllable::default();
    for jamo in keys.chars().filter_map(key_label) {
        if VOWELS.contains(jamo) {
            if let Some(coda) = syllable.coda.take() {
                let next_onset =
                    if let Some(&(a, b, _)) = CLUSTERS.iter().find(|entry| entry.2 == coda) {
                        syllable.coda = Some(a);
                        b
                    } else {
                        coda
                    };
                syllable.flush(&mut text);
                syllable.onset = Some(next_onset);
            } else if let Some(vowel) = syllable.vowel {
                if let Some(joined) = join(DIPHTHONGS, vowel, jamo) {
                    syllable.vowel = Some(joined);
                    continue;
                }
                syllable.flush(&mut text);
            }
            syllable.vowel = Some(jamo);
        } else if syllable.onset.is_some() && syllable.vowel.is_some() {
            let coda = match syllable.coda {
                Some(current) => join(CLUSTERS, current, jamo),
                None => CODAS.contains(jamo).then_some(jamo),
            };
            if let Some(coda) = coda {
                syllable.coda = Some(coda);
            } else {
                syllable.flush(&mut text);
                syllable.onset = Some(jamo);
            }
        } else {
            syllable.flush(&mut text);
            syllable.onset = Some(jamo);
        }
    }
    syllable.flush(&mut text);
    text
}

#[cfg(test)]
mod tests {
    #[test]
    fn splits_compound_codas_before_vowels() {
        assert_eq!(super::compose("rkqtdl"), "값이");
        assert_eq!(super::compose("rkqtk"), "갑사");
        assert_eq!(super::compose("Rk"), "까");
    }
}
