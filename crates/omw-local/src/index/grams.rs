use std::collections::HashSet;

/// Build the deduplicated ASCII token stream used by `search_short`.
pub fn encode_fields(fields: &[&str]) -> String {
    let mut tokens = HashSet::new();
    for field in fields {
        let scalars = field.chars().collect::<Vec<_>>();
        for scalar in &scalars {
            tokens.insert(unigram(*scalar));
        }
        for pair in scalars.windows(2) {
            tokens.insert(bigram(pair[0], pair[1]));
        }
    }
    let mut tokens = tokens.into_iter().collect::<Vec<_>>();
    tokens.sort_unstable();
    tokens.join(" ")
}

pub fn query_token(normalized_query: &str) -> Option<String> {
    let scalars = normalized_query.chars().collect::<Vec<_>>();
    match scalars.as_slice() {
        [scalar] => Some(unigram(*scalar)),
        [first, second] => Some(bigram(*first, *second)),
        _ => None,
    }
}

fn unigram(scalar: char) -> String {
    format!("u{:x}", scalar as u32)
}

fn bigram(first: char, second: char) -> String {
    format!("b{:x}_{:x}", first as u32, second as u32)
}
