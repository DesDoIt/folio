//! Liang hyphenation (as in TeX) with the US English patterns from hyph-utf8.

use std::collections::HashMap;
use std::sync::OnceLock;

static PATTERNS: &str = include_str!("../../data/hyph-en-us.pat.txt");

pub struct Hyphenator {
    pats: HashMap<String, Vec<u8>>,
    max_len: usize,
}

fn get() -> &'static Hyphenator {
    static H: OnceLock<Hyphenator> = OnceLock::new();
    H.get_or_init(|| {
        let mut pats = HashMap::new();
        let mut max_len = 0;
        for p in PATTERNS.split_whitespace() {
            let mut letters = String::new();
            let mut vals = vec![0u8];
            for c in p.chars() {
                if let Some(d) = c.to_digit(10) {
                    *vals.last_mut().unwrap() = d as u8;
                } else {
                    letters.push(c);
                    vals.push(0);
                }
            }
            max_len = max_len.max(letters.chars().count());
            pats.insert(letters, vals);
        }
        Hyphenator { pats, max_len }
    })
}

/// Character positions inside `word` after which a hyphen may be inserted.
/// Only plain ASCII-letter words of 6+ letters; keeps 2 letters before and 3 after a break.
pub fn breaks(word: &str) -> Vec<usize> {
    let n = word.chars().count();
    if n < 6 || !word.chars().all(|c| c.is_ascii_alphabetic()) {
        return vec![];
    }
    let h = get();
    let w: Vec<char> = format!(".{}.", word.to_ascii_lowercase()).chars().collect();
    let mut v = vec![0u8; w.len() + 1];
    for i in 0..w.len() {
        for j in i + 1..=(i + h.max_len).min(w.len()) {
            let sub: String = w[i..j].iter().collect();
            if let Some(p) = h.pats.get(&sub) {
                for (k, &val) in p.iter().enumerate() {
                    if val > v[i + k] {
                        v[i + k] = val;
                    }
                }
            }
        }
    }
    // v[p + 1] belongs to the gap after the p-th letter of the word (w has a leading '.')
    (2..=n - 3).filter(|&p| v[p + 1] % 2 == 1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyphenates_english() {
        let hy = |w: &str| {
            let b = breaks(w);
            let mut s = String::new();
            for (i, c) in w.chars().enumerate() {
                if b.contains(&i) {
                    s.push('-');
                }
                s.push(c);
            }
            s
        };
        assert_eq!(hy("hyphenation"), "hy-phen-ation");
        assert_eq!(hy("typography"), "ty-pog-ra-phy");
        assert_eq!(hy("cat"), "cat");
        assert_eq!(hy("Kindle"), "Kin-dle");
    }
}
