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

/// Who a title says is featured: "(with JENNIE)", "[feat. A & B]", " ft. A, B" at the end. Their
/// names, and the title without them.
pub fn featured(title: &str) -> (Vec<String>, String) {
    let lower = title.to_ascii_lowercase();   // (byte for byte the same places as the title)
    let lead = ["feat. ", "feat ", "ft. ", "ft ", "featuring ", "with "];
    // bracketed: "(with X)"
    let mut i = 0;
    let b = lower.as_bytes();
    while i < b.len() {
        if b[i] == b'(' || b[i] == b'[' {
            let close = if b[i] == b'(' { ')' } else { ']' };
            if let Some(end) = lower[i..].find(close).map(|e| i + e) {
                let inner = lower[i + 1..end].trim_start();
                if let Some(l) = lead.iter().find(|l| inner.starts_with(**l)) {
                    let start = i + 1 + (lower[i + 1..end].len() - inner.len()) + l.len();
                    let names = split_names(&title[start..end]);
                    let rest = format!("{} {}", &title[..i], &title[end + 1..]).split_whitespace().collect::<Vec<_>>().join(" ");
                    return (names, rest);
                }
                i = end;
            }
        }
        i += 1;
    }
    // at the end, unbracketed: "Song ft. X" (not "with", which songs are named with)
    for l in ["feat. ", "feat ", "ft. ", "ft ", "featuring "] {
        if let Some(at) = lower.find(&format!(" {l}")) {
            return (split_names(&title[at + 1 + l.len()..]), title[..at].trim().to_string());
        }
    }
    (Vec::new(), title.to_string())
}

fn split_names(s: &str) -> Vec<String> {
    s.split([',', '&']).flat_map(|p| p.split(" and ")).flat_map(|p| p.split(" x ")).map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
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
mod featured_tests {
    #[test]
    fn featured_names_come_out_of_the_title() {
        assert_eq!(super::featured("Dracula (with JENNIE)"), (vec!["JENNIE".to_string()], "Dracula".to_string()));
        assert_eq!(super::featured("Song [feat. A & B] (Remix)"), (vec!["A".to_string(), "B".to_string()], "Song (Remix)".to_string()));
        assert_eq!(super::featured("Song ft. A, B"), (vec!["A".to_string(), "B".to_string()], "Song".to_string()));
        assert_eq!(super::featured("Stay With Me").0.len(), 0);
        assert_eq!(super::featured("Song (Radio Edit)").0.len(), 0);
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

/// One artist however their name is written (the web app's artistKey): "The Glitch Mob" and "Glitch
/// Mob", "Above & Beyond" and "Above and Beyond", "Tiësto" and "Tiesto"
pub fn artist_key(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let plain: String = s.to_lowercase().nfkd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect();
    let plain = plain.replace('&', " and ");
    let t = plain.trim_start();
    let t = match t.strip_prefix("the ") { Some(rest) if !rest.trim().is_empty() => rest, _ => t };
    t.chars().filter(|c| c.is_alphanumeric()).collect()
}

#[cfg(test)]
mod artist_key_tests {
    #[test]
    fn one_artist_however_written() {
        use super::artist_key as k;
        assert_eq!(k("The Glitch Mob"), k("Glitch Mob"));
        assert_eq!(k("Above & Beyond"), k("Above and Beyond"));
        assert_eq!(k("Tiësto"), k("Tiesto"));
        assert_eq!(k("The The"), "the");
        assert_eq!(k("The Weeknd"), "weeknd");
    }
}
