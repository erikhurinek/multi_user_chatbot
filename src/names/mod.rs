use rand::seq::IndexedRandom;
use std::sync::LazyLock;

const ADJECTIVES: &str = include_str!("../../data/names/adjectives.txt");
const NOUNS: &str = include_str!("../../data/names/nouns.txt");

static ADJECTIVE_LIST: LazyLock<Vec<&str>> = LazyLock::new(|| load_wordlists(ADJECTIVES));

static NOUN_LIST: LazyLock<Vec<&str>> = LazyLock::new(|| load_wordlists(NOUNS));

fn load_wordlists(contents: &'static str) -> Vec<&'static str> {
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

pub fn generate_name() -> String {
    let mut rng = rand::rng();

    let adjective = ADJECTIVE_LIST.choose(&mut rng).unwrap();
    let noun = NOUN_LIST.choose(&mut rng).unwrap();

    format!("{}{}", capitalise(adjective), capitalise(noun))
}
