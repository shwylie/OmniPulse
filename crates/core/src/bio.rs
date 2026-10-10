// Artist bios, from Wikipedia: the opening of the artist's article, its picture and link. Found by a
// search for the name, and only taken if the article is about a musician, band, DJ or producer (its
// short description says so), so a same-named film or footballer isn't shown. Wikimedia asks for a
// User-Agent that names the app and how to reach its maintainer. A bio is kept for a week.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const USER_AGENT: &str = concat!("OmniPulse/", env!("CARGO_PKG_VERSION"), " ( https://github.com/shwylie/OmniPulse )");
const KEEP_FOR: Duration = Duration::from_secs(7 * 86400);
static CACHE: Mutex<Option<HashMap<String, (Instant, Value)>>> = Mutex::new(None);

/// Words that say an article is about music people
const MUSIC: [&str; 24] = ["musician", "singer", "band", "rapper", "dj", "disc jockey", "producer", "songwriter", "duo", "group", "composer", "guitarist",
    "drummer", "pianist", "vocalist", "orchestra", "ensemble", "trio", "quartet", "electronic music", "record producer", "music project", "boy band", "girl group"];

fn about_music(description: &str) -> bool {
    let d = description.to_lowercase();
    MUSIC.iter().any(|w| d.split(|c: char| !c.is_alphanumeric() && c != ' ').any(|part| part.contains(w)))
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().user_agent(USER_AGENT).timeout(Duration::from_secs(12)).build().unwrap_or_default()
}

async fn get(url: &str) -> Result<Value, String> {
    let res = client().get(url).send().await.map_err(|e| format!("Couldn't reach Wikipedia: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("Wikipedia answered {}", res.status()));
    }
    res.json().await.map_err(|e| e.to_string())
}

/// { title, text, url, image } for the artist, or null when Wikipedia has nothing about them as a musician
pub async fn artist(name: &str) -> Result<Value, String> {
    let name = name.trim();
    if name.is_empty() {
        return Ok(Value::Null);
    }
    let key = name.to_lowercase();
    if let Some((at, v)) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(&key)).cloned() {
        if at.elapsed() < KEEP_FOR {
            return Ok(v);
        }
    }
    // the likely articles, asked about together: "Muse (band)", "Adele", "Avicii (DJ)"...; the first in
    // this order whose short description is about music wins
    let kinds = ["band", "musician", "DJ", "rapper", "singer", "producer", "duo", "group", "artist"];
    let mut titles: Vec<String> = kinds.iter().map(|k| format!("{name} ({k})")).collect();
    titles.insert(0, name.to_string());
    let q = get(&format!(
        "https://en.wikipedia.org/w/api.php?action=query&format=json&prop=description&redirects=1&titles={}",
        urlencoding::encode(&titles.join("|"))
    ))
    .await?;
    // (redirects: "Avicii (DJ)" may be filed as "Avicii")
    let redirect = |t: &str| -> String {
        q["query"]["normalized"].as_array().into_iter().flatten().chain(q["query"]["redirects"].as_array().into_iter().flatten())
            .filter(|r| r["from"].as_str() == Some(t)).find_map(|r| r["to"].as_str().map(String::from)).unwrap_or_else(|| t.to_string())
    };
    let pages: Vec<&Value> = q["query"]["pages"].as_object().map(|o| o.values().collect()).unwrap_or_default();
    let pick = titles.iter().map(|t| redirect(&redirect(t))).find(|t| {
        pages.iter().any(|p| p["title"].as_str() == Some(t.as_str()) && p.get("missing").is_none() && about_music(p["description"].as_str().unwrap_or("")))
    });
    let mut found = Value::Null;
    if let Some(title) = pick {
        let s = get(&format!("https://en.wikipedia.org/api/rest_v1/page/summary/{}", urlencoding::encode(&title.replace(' ', "_")))).await?;
        if s["type"].as_str() != Some("disambiguation") {
            found = json!({
                "title": s["title"],
                "text": s["extract"],
                "url": s["content_urls"]["desktop"]["page"],
                "image": s["thumbnail"]["source"],
                "description": s["description"],
            });
        }
    }
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(key, (Instant::now(), found.clone()));
    Ok(found)
}

#[cfg(test)]
mod tests {
    #[test]
    fn musicians_only() {
        assert!(super::about_music("French electronic music duo"));
        assert!(super::about_music("English singer-songwriter"));
        assert!(!super::about_music("2015 American film"));
        assert!(!super::about_music("Brazilian footballer"));
    }

    /// cargo test -p omnipulse-core bio -- --ignored (on the internet)
    #[test]
    #[ignore]
    fn finding() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let v = rt.block_on(super::artist("Daft Punk")).unwrap();
        println!("{v}");
        assert!(v["text"].as_str().unwrap().contains("duo"));
        let m = rt.block_on(super::artist("Muse")).unwrap();
        println!("{}", m["title"]);
        assert!(m["description"].as_str().unwrap_or("").to_lowercase().contains("band"));
    }
}
