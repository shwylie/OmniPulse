// Search ranking, shared by the native app and the phone app (the web app's searchRanker, in Rust).

/// The words of a title or search, lowercased and without accents (single letters left out, numbers kept)
pub fn words(s: &str) -> Vec<String> {
    use unicode_normalization::UnicodeNormalization;
    let plain: String = s.to_lowercase().nfkd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect();
    let keep = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || ('\u{c0}'..='\u{24f}').contains(&c) || ('\u{400}'..='\u{4ff}').contains(&c)
        || ('\u{1100}'..='\u{11ff}').contains(&c) || ('\u{3040}'..='\u{9fff}').contains(&c) || ('\u{ac00}'..='\u{d7af}').contains(&c);
    plain.split(|c: char| !keep(c)).filter(|w| w.chars().count() > 1 || w.chars().any(|c| c.is_ascii_digit())).map(String::from).collect()
}

/// Best match first: more of the words searched for (in the title or artist), then a title (or
/// artist) that's just what was searched, then the more played; otherwise as they came (the
/// platforms taking turns). Unknown plays count as middling. `of` gives a result's title, artist and plays.
pub fn rank_by<T>(q: &str, list: Vec<T>, of: impl Fn(&T) -> (&str, &str, u64)) -> Vec<T> {
    let mut want = words(q);
    let asked = want.join(" ");
    want.sort();
    want.dedup();
    if want.is_empty() {
        return list;
    }
    let mut scored: Vec<(f64, u8, f64, usize, T)> = list
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            let (title, artist, plays) = of(&t);
            let have: std::collections::HashSet<String> = words(&format!("{title} {artist}")).into_iter().collect();
            let s = want.iter().filter(|w| have.contains(*w)).count() as f64 / want.len() as f64;
            let exact = u8::from(words(title).join(" ") == asked || words(artist).join(" ") == asked);
            let plays = if plays > 0 { plays as f64 } else { 1e5 };
            (s, exact, (1.0 + plays).log10(), i, t)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)).then(b.2.total_cmp(&a.2)).then(a.3.cmp(&b.3)));
    scored.into_iter().map(|x| x.4).collect()
}

/// The lists taking turns: the first of each, then the second of each, ...
pub fn interleave<T>(lists: Vec<Vec<T>>) -> Vec<T> {
    let mut its: Vec<_> = lists.into_iter().map(|l| l.into_iter()).collect();
    let mut out = Vec::new();
    loop {
        let mut any = false;
        for it in its.iter_mut() {
            if let Some(t) = it.next() {
                out.push(t);
                any = true;
            }
        }
        if !any {
            return out;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn best_match_first() {
        let list = vec![("Hello (cover)", "someone", 10), ("Hello", "Adele", 3_000_000_000), ("Goodbye", "x", 0)];
        let r = super::rank_by("hello adele", list, |t| (t.0, t.1, t.2));
        assert_eq!(r[0].1, "Adele");
        assert_eq!(r[2].0, "Goodbye");
    }
}
