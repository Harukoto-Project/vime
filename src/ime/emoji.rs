#[derive(Clone, Copy)]
pub struct Emoji {
    pub symbol: &'static str,
    pub keywords: &'static str,
}

fn entries() -> impl Iterator<Item = Emoji> {
    include_str!("emoji.tsv").lines().filter_map(|line| {
        let (symbol, keywords) = line.split_once('\t')?;
        Some(Emoji { symbol, keywords })
    })
}

pub fn search(query: &str) -> impl Iterator<Item = Emoji> + '_ {
    let query = query.trim().trim_matches(':').to_lowercase();
    entries().filter(move |entry| {
        query
            .split_whitespace()
            .all(|term| entry.keywords.contains(term) || entry.symbol == term)
    })
}

pub fn lookup(reading: &str) -> impl Iterator<Item = Emoji> + '_ {
    entries().filter(move |entry| {
        entry
            .keywords
            .split_whitespace()
            .any(|word| word == reading)
    })
}
