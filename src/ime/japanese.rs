use std::sync::LazyLock;

// Group by the first letter while preserving each group's longest-first order.
const KANA: &str = include_str!("romaji.txt");
const WORDS: &str = include_str!("words.tsv");
type RomanSyllable = (&'static str, &'static str);
static SYLLABLES: LazyLock<[Vec<RomanSyllable>; 26]> = LazyLock::new(|| {
    let mut groups: [Vec<RomanSyllable>; 26] = Default::default();
    for (roman, kana) in KANA
        .split_ascii_whitespace()
        .filter_map(|entry| entry.split_once('='))
    {
        groups[(roman.as_bytes()[0] - b'a') as usize].push((roman, kana));
    }
    groups
});

pub fn kana(input: &str, finalizing: bool) -> String {
    let mut result = String::new();
    let mut rest = input;
    while let Some(&first) = rest.as_bytes().first() {
        let next = rest.as_bytes().get(1).copied();
        // A bare run of three or more n's is literal. Keep the explicit nn and n'
        // rules and normal syllables such as the nn in こんにちは.
        if rest.starts_with("nnn") {
            let len = rest.bytes().take_while(|&byte| byte == b'n').count();
            if rest
                .as_bytes()
                .get(len)
                .is_none_or(|byte| !byte.is_ascii_alphabetic() && *byte != b'\'')
            {
                result.push_str(&rest[..len]);
                rest = &rest[len..];
                continue;
            }
        }
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
        if next == Some(first)
            && first.is_ascii_lowercase()
            && !b"aiueon".contains(&first)
            && syllable(&rest[1..]).is_some()
        {
            result.push('っ');
            rest = &rest[1..];
            continue;
        }
        if first == b'-' {
            result.push('ー');
            rest = &rest[1..];
            continue;
        }
        if let Some((roman, kana)) = syllable(rest) {
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

fn syllable(rest: &str) -> Option<RomanSyllable> {
    let first = *rest.as_bytes().first()?;
    if !first.is_ascii_lowercase() {
        return None;
    }
    SYLLABLES[(first - b'a') as usize]
        .iter()
        .copied()
        .find(|(roman, _)| rest.starts_with(roman))
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

#[cfg(test)]
mod tests {
    use super::kana;

    #[test]
    fn repeated_consonants_without_a_syllable_remain_literal() {
        for consonant in "bcdfghjklmnpqrstvwxyz".chars() {
            let input = consonant.to_string().repeat(3);
            assert_eq!(kana(&input, false), input);
            assert_eq!(kana(&input, true), input);
        }
        assert_eq!(kana("kitte", true), "きって");
        assert_eq!(kana("aaa", true), "あああ");
        assert_eq!(kana("nn", true), "ん");
    }
}
