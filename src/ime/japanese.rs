// Longer syllables are checked before shorter ones.
const KANA: &str = include_str!("romaji.txt");
const WORDS: &str = include_str!("words.tsv");

pub fn kana(input: &str, finalizing: bool) -> String {
    let mut result = String::new();
    let mut rest = input;
    while let Some(&first) = rest.as_bytes().first() {
        let next = rest.as_bytes().get(1).copied();
        if first == b'n' {
            match next {
                Some(b'n')
                    if rest
                        .as_bytes()
                        .get(2)
                        .is_some_and(|next| b"aiueoy".contains(next)) =>
                {
                    result.push('ん');
                    rest = &rest[1..];
                    continue;
                }
                Some(b'n' | b'\'') => {
                    result.push('ん');
                    rest = &rest[2..];
                    continue;
                }
                Some(next) if next.is_ascii_alphabetic() && !b"aiueoy".contains(&next) => {
                    result.push('ん');
                    rest = &rest[1..];
                    continue;
                }
                None if finalizing => {
                    result.push('ん');
                    break;
                }
                _ => {}
            }
        }
        if next == Some(first) && first.is_ascii_lowercase() && !b"aiueon".contains(&first) {
            result.push('っ');
            rest = &rest[1..];
            continue;
        }
        if first == b'-' {
            result.push('ー');
            rest = &rest[1..];
            continue;
        }
        if let Some((roman, kana)) = KANA
            .split_ascii_whitespace()
            .filter_map(|entry| entry.split_once('='))
            .find(|(roman, _)| rest.starts_with(roman))
        {
            result.push_str(kana);
            rest = &rest[roman.len()..];
        } else {
            let ch = rest.chars().next().unwrap();
            result.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    result
}

pub fn katakana(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            'ぁ'..='ゖ' => char::from_u32(c as u32 + 0x60).unwrap(),
            _ => c,
        })
        .collect()
}

pub fn dictionary(reading: &str) -> impl Iterator<Item = &'static str> {
    WORDS
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .filter(move |(key, _)| *key == reading)
        .flat_map(|(_, words)| words.split(' '))
}
