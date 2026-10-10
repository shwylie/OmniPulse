// YouTube Music, without signing in: search and radio through YouTube Music's own web API
// (InnerTube, as music.youtube.com uses it), and audio through yt-dlp (yt_stream.rs).
//
// The web client's version and visitor id are read from music.youtube.com itself (and refreshed
// twice a day), so they don't go stale the way hard-coded client versions do. Search and radio
// need no tokens; playing does, which is what yt-dlp (with Deno) takes care of.
//
// This is YouTube's private API: it can change without notice, and using it isn't allowed by
// YouTube's terms. Nothing here signs in, so no Google account is involved.

use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const ORIGIN: &str = "https://music.youtube.com";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const FALLBACK_VERSION: &str = "1.20261004.17.00";
const CFG_TTL: Duration = Duration::from_secs(12 * 3600);
// search filters, as YouTube Music's own filter chips send them
const SONGS: &str = "EgWKAQIIAWoMEA4QChADEAQQCRAF";

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct YtTrack {
    pub id: String,
    pub title: String,
    pub artists: String,
    /// the first artist's channel ("UC..."), when YouTube Music links it
    pub artist_id: String,
    pub album: String,
    pub album_id: String,
    pub duration_ms: u64,
    pub thumb: String,
    /// how many times it's been played ("2.4B plays" in search results), 0 when not shown
    pub plays: u64,
}

#[derive(Clone, Debug)]
struct Cfg {
    version: String,
    visitor: String,
    hl: String,
    gl: String,
}

static CFG: Mutex<Option<(Instant, Cfg)>> = Mutex::new(None);

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default()
}

fn ytcfg(html: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":\"");
    let at = html.find(&pat)? + pat.len();
    html[at..].split('"').next().map(String::from).filter(|s| !s.is_empty())
}

async fn cfg() -> Cfg {
    if let Some((at, c)) = CFG.lock().ok().and_then(|g| g.clone()) {
        if at.elapsed() < CFG_TTL {
            return c;
        }
    }
    // SOCS=CAI: the cookie-consent answer for "reject all", so no consent page gets in the way
    let page = async {
        client().get(ORIGIN).header("Cookie", "SOCS=CAI").send().await.ok()?.text().await.ok()
    }
    .await
    .unwrap_or_default();
    let c = Cfg {
        version: ytcfg(&page, "INNERTUBE_CLIENT_VERSION").unwrap_or_else(|| FALLBACK_VERSION.into()),
        visitor: ytcfg(&page, "VISITOR_DATA").unwrap_or_default(),
        hl: ytcfg(&page, "HL").unwrap_or_else(|| "en".into()),
        gl: ytcfg(&page, "GL").unwrap_or_else(|| "US".into()),
    };
    if !page.is_empty() {
        if let Ok(mut g) = CFG.lock() {
            *g = Some((Instant::now(), c.clone()));
        }
    }
    c
}

async fn call(endpoint: &str, mut body: Value) -> Result<Value, String> {
    let c = cfg().await;
    body["context"] = json!({ "client": {
        "clientName": "WEB_REMIX", "clientVersion": c.version, "hl": c.hl, "gl": c.gl, "visitorData": c.visitor,
    } });
    let res = client()
        .post(format!("{ORIGIN}/youtubei/v1/{endpoint}?prettyPrint=false"))
        .header("Origin", ORIGIN)
        .header("Referer", format!("{ORIGIN}/"))
        .header("X-Goog-Visitor-Id", &c.visitor)
        .header("Cookie", "SOCS=CAI")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("YouTube Music didn't answer: {e}"))?;
    let status = res.status();
    let v: Value = res.json().await.map_err(|e| format!("YouTube Music sent something unreadable: {e}"))?;
    if !status.is_success() {
        let msg = v["error"]["message"].as_str().unwrap_or("");
        return Err(format!("YouTube Music said {}{}", status.as_u16(), if msg.is_empty() { String::new() } else { format!(": {msg}") }));
    }
    Ok(v)
}

// ---------- reading the responses ----------

fn find_all<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if k == key {
                    out.push(x);
                }
                find_all(x, key, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| find_all(x, key, out)),
        _ => {}
    }
}

fn runs(v: &Value) -> Vec<&Value> {
    v["runs"].as_array().map(|a| a.iter().collect()).unwrap_or_default()
}

fn text(v: &Value) -> String {
    runs(v).iter().filter_map(|r| r["text"].as_str()).collect()
}

fn page_type(run: &Value) -> &str {
    run["navigationEndpoint"]["browseEndpoint"]["browseEndpointContextSupportedConfigs"]["browseEndpointContextMusicConfig"]["pageType"]
        .as_str()
        .unwrap_or("")
}

fn browse_id(run: &Value) -> &str {
    run["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str().unwrap_or("")
}

/// "3:45" or "1:02:03" in ms
pub fn parse_duration(s: &str) -> Option<u64> {
    let parts: Vec<u64> = s.trim().split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    Some(parts.iter().fold(0, |acc, p| acc * 60 + p) * 1000)
}

/// "21m plays", "1.2M views", "950K plays", "2.4B plays" as a number
pub fn parse_plays(s: &str) -> Option<u64> {
    // (YouTube puts a non-breaking space before "plays")
    let s = s.replace('\u{a0}', " ").trim().to_lowercase();
    let num = s.strip_suffix(" plays").or_else(|| s.strip_suffix(" views")).or_else(|| s.strip_suffix(" play")).or_else(|| s.strip_suffix(" view"))?.trim();
    // ("bn" is how YouTube Music writes billions; "mn" turns up too)
    let (digits, mult) = [("bn", 1e9), ("mn", 1e6), ("k", 1e3), ("m", 1e6), ("b", 1e9)]
        .iter()
        .find_map(|(suf, m)| num.strip_suffix(suf).map(|d| (d, *m)))
        .unwrap_or((num, 1.0));
    let n: f64 = digits.trim().replace(',', "").parse().ok()?;
    Some((n * mult).round() as u64)
}

// The biggest thumbnail, asked for at 544 px (Google's image URLs take the size as a suffix)
fn thumb(v: &Value) -> String {
    let mut all = Vec::new();
    find_all(v, "thumbnails", &mut all);
    let url = all
        .first()
        .and_then(|t| t.as_array())
        .and_then(|a| a.last())
        .and_then(|t| t["url"].as_str())
        .unwrap_or("");
    match url.rfind("=w") {
        Some(at) if url.contains("googleusercontent.com") => format!("{}=w544-h544-l90-rj", &url[..at]),
        _ => url.to_string(),
    }
}

// The byline: artists, album, and (in search results) the length
fn byline(t: &mut YtTrack, line: &[&Value]) {
    let mut artists = Vec::new();
    for r in line {
        let s = r["text"].as_str().unwrap_or("").trim();
        match page_type(r) {
            "MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL" => {
                if t.artist_id.is_empty() {
                    t.artist_id = browse_id(r).to_string();
                }
                artists.push(s.to_string());
            }
            "MUSIC_PAGE_TYPE_ALBUM" => {
                t.album = s.to_string();
                t.album_id = browse_id(r).to_string();
            }
            _ => {
                if let Some(ms) = parse_duration(s) {
                    t.duration_ms = ms;
                }
            }
        }
    }
    if artists.is_empty() {
        // artists without a page of their own are plain text before the first " • "
        artists = line
            .iter()
            .map(|r| r["text"].as_str().unwrap_or(""))
            .collect::<String>()
            .split(" • ")
            .next()
            .map(|a| vec![a.trim().to_string()])
            .unwrap_or_default();
    }
    t.artists = artists.into_iter().filter(|a| !a.is_empty()).collect::<Vec<_>>().join(", ");
}

/// A song row in search results (musicResponsiveListItemRenderer)
pub fn parse_list_item(it: &Value) -> Option<YtTrack> {
    let cols = it["flexColumns"].as_array()?;
    let col = |i: usize| cols.get(i).map(|c| &c["musicResponsiveListItemFlexColumnRenderer"]["text"]);
    let title_runs = runs(col(0)?);
    let id = it["playlistItemData"]["videoId"]
        .as_str()
        .or_else(|| title_runs.first()?["navigationEndpoint"]["watchEndpoint"]["videoId"].as_str())?;
    let mut t = YtTrack { id: id.to_string(), title: text(col(0)?), thumb: thumb(&it["thumbnail"]), ..Default::default() };
    if let Some(c) = col(1) {
        byline(&mut t, &runs(c));
    }
    // the play count: a column of its own, or a part of the byline
    t.plays = cols
        .iter()
        .flat_map(|c| runs(&c["musicResponsiveListItemFlexColumnRenderer"]["text"]))
        .find_map(|r| parse_plays(r["text"].as_str().unwrap_or("")))
        .unwrap_or(0);
    if t.duration_ms == 0 {
        // sometimes the length sits in a fixed column on the right
        let mut fixed = Vec::new();
        find_all(&it["fixedColumns"], "text", &mut fixed);
        t.duration_ms = fixed.iter().find_map(|f| parse_duration(&text(f))).unwrap_or(0);
    }
    Some(t)
}

/// A row in a radio / up-next queue (playlistPanelVideoRenderer)
pub fn parse_panel_item(it: &Value) -> Option<YtTrack> {
    let id = it["videoId"].as_str()?;
    let mut t = YtTrack { id: id.to_string(), title: text(&it["title"]), thumb: thumb(&it["thumbnail"]), ..Default::default() };
    byline(&mut t, &runs(&it["longBylineText"]));
    t.duration_ms = parse_duration(&text(&it["lengthText"])).unwrap_or(t.duration_ms);
    Some(t)
}

fn valid_id(id: &str) -> bool {
    id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

// ---------- what the app asks for ----------

/// Songs matching a search
pub async fn search(query: &str) -> Result<Vec<YtTrack>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let v = call("search", json!({ "query": q, "params": SONGS })).await?;
    let mut items = Vec::new();
    find_all(&v, "musicResponsiveListItemRenderer", &mut items);
    let mut out: Vec<YtTrack> = items.into_iter().filter_map(parse_list_item).collect();
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

/// The radio YouTube Music would play after this song (up to ~50 songs, the song itself first)
pub async fn radio(video_id: &str) -> Result<Vec<YtTrack>, String> {
    if !valid_id(video_id) {
        return Err("Bad YouTube video id".into());
    }
    let v = call(
        "next",
        json!({
            "videoId": video_id, "playlistId": format!("RDAMVM{video_id}"), "isAudioOnly": true,
            "enablePersistentPlaylistPanel": true, "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
        }),
    )
    .await?;
    let mut items = Vec::new();
    find_all(&v, "playlistPanelVideoRenderer", &mut items);
    let mut seen = std::collections::HashSet::new();
    Ok(items.into_iter().filter_map(parse_panel_item).filter(|t| seen.insert(t.id.clone())).collect())
}

// ---------- artists, albums and playlists ----------
const ARTISTS: &str = "EgWKAQIgAWoMEA4QChADEAQQCRAF";
// community playlists: the ones people made (YouTube Music's own "featured" ones are rarely on topic)
const PLAYLISTS: &str = "EgeKAQQoAEABagwQDhAKEAMQBBAJEAU=";

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct YtPlaylist {
    /// "VL..."
    pub id: String,
    pub title: String,
    pub author: String,
    pub thumb: String,
}

/// A playlist row in search results
pub fn parse_playlist_item(it: &Value) -> Option<YtPlaylist> {
    let mut ids = Vec::new();
    find_all(&it["navigationEndpoint"], "browseId", &mut ids);
    let id = ids.iter().filter_map(|x| x.as_str()).find(|x| x.starts_with("VL"))?;
    let cols = it["flexColumns"].as_array()?;
    let col = |i: usize| cols.get(i).map(|c| text(&c["musicResponsiveListItemFlexColumnRenderer"]["text"])).unwrap_or_default();
    let author = col(1).split(" • ").next().unwrap_or("").to_string();
    Some(YtPlaylist { id: id.to_string(), title: col(0), author, thumb: thumb(&it["thumbnail"]) })
}

/// Playlists matching a search
pub async fn search_playlists(query: &str) -> Result<Vec<YtPlaylist>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let v = call("search", json!({ "query": q, "params": PLAYLISTS })).await?;
    let mut items = Vec::new();
    find_all(&v, "musicResponsiveListItemRenderer", &mut items);
    Ok(items.into_iter().filter_map(parse_playlist_item).collect())
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct YtArtist {
    pub id: String,
    pub name: String,
    pub thumb: String,
    pub subscribers: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct YtAlbum {
    pub id: String,
    pub title: String,
    pub artists: String,
    /// "Album", "Single", "EP"...
    pub kind: String,
    pub year: String,
    pub thumb: String,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct YtArtistPage {
    pub id: String,
    pub name: String,
    pub thumb: String,
    pub subscribers: String,
    pub songs: Vec<YtTrack>,
    /// the playlist of all their songs ("VL..."), when YouTube Music offers one
    pub all_songs: String,
    pub albums: Vec<YtAlbum>,
}

// The last (largest) picture, as it is (artist banners aren't square)
fn picture(v: &Value) -> String {
    let mut all = Vec::new();
    find_all(v, "thumbnails", &mut all);
    all.first().and_then(|t| t.as_array()).and_then(|a| a.last()).and_then(|t| t["url"].as_str()).unwrap_or("").to_string()
}

/// An artist row in search results
pub fn parse_artist_item(it: &Value) -> Option<YtArtist> {
    let id = it["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str().filter(|id| id.starts_with("UC"))?;
    let cols = it["flexColumns"].as_array()?;
    let col = |i: usize| cols.get(i).map(|c| text(&c["musicResponsiveListItemFlexColumnRenderer"]["text"])).unwrap_or_default();
    // "Artist • 1.37M subscribers"
    let subscribers = col(1).split(" • ").find(|p| p.contains("subscriber")).unwrap_or("").replace(" subscribers", "").to_string();
    Some(YtArtist { id: id.to_string(), name: col(0), thumb: thumb(&it["thumbnail"]), subscribers })
}

/// An album or single tile (musicTwoRowItemRenderer)
pub fn parse_two_row(it: &Value, kind: &str, artists: &str) -> Option<YtAlbum> {
    let id = it["navigationEndpoint"]["browseEndpoint"]["browseId"]
        .as_str()
        .or_else(|| runs(&it["title"]).first()?["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str())
        .filter(|id| id.starts_with("MPRE"))?;
    // the subtitle is "2024", or "Single • 2024", or "EP • Artist • 2024"
    let sub = text(&it["subtitle"]);
    let year = sub.split(" • ").find(|p| p.len() == 4 && p.chars().all(|c| c.is_ascii_digit())).unwrap_or("").to_string();
    let first = sub.split(" • ").next().unwrap_or("");
    let kind = if ["Single", "EP", "Album"].contains(&first) { first.to_string() } else { kind.to_string() };
    Some(YtAlbum { id: id.to_string(), title: text(&it["title"]), artists: artists.to_string(), kind, year, thumb: thumb(&it["thumbnailRenderer"]) })
}

/// Artists matching a search
pub async fn search_artists(query: &str) -> Result<Vec<YtArtist>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let v = call("search", json!({ "query": q, "params": ARTISTS })).await?;
    let mut items = Vec::new();
    find_all(&v, "musicResponsiveListItemRenderer", &mut items);
    Ok(items.into_iter().filter_map(parse_artist_item).collect())
}

pub fn parse_artist_page(id: &str, v: &Value) -> YtArtistPage {
    let mut heads = Vec::new();
    find_all(v, "musicImmersiveHeaderRenderer", &mut heads);
    find_all(v, "musicVisualHeaderRenderer", &mut heads);
    let head = heads.first().copied().unwrap_or(&Value::Null);
    let mut subs = Vec::new();
    find_all(head, "subscriberCountText", &mut subs);
    let mut page = YtArtistPage {
        id: id.to_string(),
        name: text(&head["title"]),
        thumb: picture(&head["thumbnail"]),
        subscribers: subs.first().map(|s| text(s)).unwrap_or_default(),
        ..Default::default()
    };
    // "Top songs", and the playlist of all of them
    let mut shelves = Vec::new();
    find_all(v, "musicShelfRenderer", &mut shelves);
    if let Some(shelf) = shelves.first() {
        let mut rows = Vec::new();
        find_all(&shelf["contents"], "musicResponsiveListItemRenderer", &mut rows);
        page.songs = rows.into_iter().filter_map(parse_list_item).collect();
        let mut ids = Vec::new();
        find_all(&shelf["title"], "browseId", &mut ids);
        find_all(&shelf["bottomEndpoint"], "browseId", &mut ids);
        page.all_songs = ids.iter().filter_map(|x| x.as_str()).find(|x| x.starts_with("VL")).unwrap_or("").to_string();
    }
    // albums, then singles & EPs
    let mut carousels = Vec::new();
    find_all(v, "musicCarouselShelfRenderer", &mut carousels);
    for c in carousels {
        let title = text(&c["header"]["musicCarouselShelfBasicHeaderRenderer"]["title"]);
        let kind = match title.as_str() {
            "Albums" => "Album",
            "Singles" | "Singles and EPs" | "Singles & EPs" => "Single",
            _ => continue,
        };
        for it in c["contents"].as_array().into_iter().flatten() {
            if let Some(a) = parse_two_row(&it["musicTwoRowItemRenderer"], kind, &page.name) {
                page.albums.push(a);
            }
        }
    }
    page
}

/// An artist's page: top songs, albums and singles
pub async fn artist(id: &str) -> Result<YtArtistPage, String> {
    if !id.starts_with("UC") || id.len() > 40 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("Bad YouTube artist id".into());
    }
    Ok(parse_artist_page(id, &call("browse", json!({ "browseId": id })).await?))
}

pub fn parse_album_page(id: &str, v: &Value) -> (YtAlbum, Vec<YtTrack>) {
    let mut heads = Vec::new();
    find_all(v, "musicResponsiveHeaderRenderer", &mut heads);
    find_all(v, "musicDetailHeaderRenderer", &mut heads);
    let head = heads.first().copied().unwrap_or(&Value::Null);
    let sub = text(&head["subtitle"]);
    let mut album = YtAlbum {
        id: id.to_string(),
        title: text(&head["title"]),
        artists: text(&head["straplineTextOne"]),
        kind: sub.split(" • ").next().unwrap_or("Album").to_string(),
        year: sub.split(" • ").find(|p| p.len() == 4 && p.chars().all(|c| c.is_ascii_digit())).unwrap_or("").to_string(),
        thumb: thumb(&head["thumbnail"]),
    };
    if album.artists.is_empty() {
        album.artists = sub.split(" • ").nth(1).unwrap_or("").to_string();
    }
    let mut rows = Vec::new();
    find_all(v, "musicResponsiveListItemRenderer", &mut rows);
    let tracks = rows
        .into_iter()
        .filter_map(parse_list_item)
        .map(|mut t| {
            // album rows leave out what the album already says
            if t.artists.is_empty() {
                t.artists = album.artists.clone();
            }
            if t.album.is_empty() {
                t.album = album.title.clone();
                t.album_id = album.id.clone();
            }
            if t.thumb.is_empty() {
                t.thumb = album.thumb.clone();
            }
            t
        })
        .collect();
    (album, tracks)
}

/// An album's songs
pub async fn album(id: &str) -> Result<(YtAlbum, Vec<YtTrack>), String> {
    if !id.starts_with("MPRE") || id.len() > 40 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("Bad YouTube album id".into());
    }
    Ok(parse_album_page(id, &call("browse", json!({ "browseId": id })).await?))
}

/// A playlist's songs (its first 100 or so), by its "VL..." id
pub async fn playlist(id: &str) -> Result<Vec<YtTrack>, String> {
    if !id.starts_with("VL") || id.len() > 80 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("Bad YouTube playlist id".into());
    }
    let v = call("browse", json!({ "browseId": id })).await?;
    let mut rows = Vec::new();
    find_all(&v, "musicResponsiveListItemRenderer", &mut rows);
    let mut seen = std::collections::HashSet::new();
    Ok(rows.into_iter().filter_map(parse_list_item).filter(|t| seen.insert(t.id.clone())).collect())
}

// ---------- audio ----------

static STREAMS: Mutex<Vec<(String, String, u64)>> = Mutex::new(Vec::new()); // (id, url, expires at)

pub fn forget_streams() {
    if let Ok(mut g) = STREAMS.lock() {
        g.clear();
    }
}

#[cfg(feature = "desktop")]
fn expiry(url: &str) -> u64 {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.query_pairs().find(|(k, _)| k == "expire").and_then(|(_, v)| v.parse().ok()))
        .unwrap_or_else(|| crate::now_secs() + 3600)
}

/// A direct audio URL for a video, from yt-dlp (kept until shortly before it expires)
#[cfg(feature = "desktop")]
pub async fn stream_url(video_id: &str, fresh: bool) -> Result<String, String> {
    if !valid_id(video_id) {
        return Err("Bad YouTube video id".into());
    }
    let now = crate::now_secs();
    if let Ok(mut g) = STREAMS.lock() {
        g.retain(|(_, _, exp)| *exp > now + 120);
        if fresh {
            g.retain(|(id, _, _)| id != video_id);
        } else if let Some((_, url, _)) = g.iter().find(|(id, _, _)| id == video_id) {
            return Ok(url.clone());
        }
    }
    let url = crate::yt_stream::resolve_video(video_id).await?;
    if let Ok(mut g) = STREAMS.lock() {
        g.push((video_id.to_string(), url.clone(), expiry(&url)));
        if g.len() > 200 {
            g.remove(0);
        }
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lengths() {
        assert_eq!(parse_duration("3:45"), Some(225_000));
        assert_eq!(parse_duration("1:02:03"), Some(3_723_000));
        assert_eq!(parse_duration("21m plays"), None);
        assert_eq!(parse_duration("2024"), None);
    }

    #[test]
    fn reads_a_search_row() {
        let artist = |name: &str, id: &str, kind: &str| json!({ "text": name, "navigationEndpoint": { "browseEndpoint": { "browseId": id,
            "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": kind } } } } });
        let row = json!({
            "flexColumns": [
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Language", "navigationEndpoint": { "watchEndpoint": { "videoId": "HDVIQ6E_J58" } } }] } } },
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [
                    artist("Porter Robinson", "UCUt2uP6O_UBJp4aBx5KjQjA", "MUSIC_PAGE_TYPE_ARTIST"), { "text": " & " },
                    artist("Madeon", "UCx", "MUSIC_PAGE_TYPE_ARTIST"), { "text": " • " },
                    artist("Language", "MPREb_FWVn0qzn7aW", "MUSIC_PAGE_TYPE_ALBUM"), { "text": " • " }, { "text": "6:08" }] } } },
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "21m plays" }] } } }
            ],
            "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [
                { "url": "https://yt3.googleusercontent.com/abc=w60-h60-l90-rj" }, { "url": "https://yt3.googleusercontent.com/abc=w120-h120-l90-rj" }] } } },
            "playlistItemData": { "videoId": "HDVIQ6E_J58" }
        });
        let t = parse_list_item(&row).unwrap();
        assert_eq!(t.id, "HDVIQ6E_J58");
        assert_eq!(t.title, "Language");
        assert_eq!(t.artists, "Porter Robinson, Madeon");
        assert_eq!((t.album.as_str(), t.album_id.as_str()), ("Language", "MPREb_FWVn0qzn7aW"));
        assert_eq!(t.duration_ms, 368_000);
        assert_eq!(t.plays, 21_000_000);
        assert_eq!(t.thumb, "https://yt3.googleusercontent.com/abc=w544-h544-l90-rj");
    }

    #[test]
    fn reads_a_radio_row_with_a_plain_text_artist() {
        let row = json!({ "videoId": "6M-F1vV3Dks", "title": { "runs": [{ "text": "Sad Machine" }] },
            "longBylineText": { "runs": [{ "text": "Some Uploader" }, { "text": " • " }, { "text": "1.2m views" }] },
            "lengthText": { "runs": [{ "text": "5:51" }] } });
        let t = parse_panel_item(&row).unwrap();
        assert_eq!((t.title.as_str(), t.artists.as_str(), t.duration_ms), ("Sad Machine", "Some Uploader", 351_000));
    }

    #[test]
    fn reads_play_counts() {
        assert_eq!(parse_plays("2.4B plays"), Some(2_400_000_000));
        assert_eq!(parse_plays("3.5bn plays"), Some(3_500_000_000));
        assert_eq!(parse_plays("653k plays"), Some(653_000));
        assert_eq!(parse_plays("291k\u{a0}plays"), Some(291_000));
        assert_eq!(parse_plays("291k plays"), Some(291_000));
        assert_eq!(parse_plays("950K views"), Some(950_000));
        assert_eq!(parse_plays("1,234 plays"), Some(1_234));
        assert_eq!(parse_plays("6:08"), None);
        assert_eq!(parse_plays("Language"), None);
    }

    #[test]
    fn only_real_video_ids() {
        assert!(valid_id("HDVIQ6E_J58"));
        assert!(!valid_id("HDVIQ6E_J5"));
        assert!(!valid_id("a b c d e f"));
    }

    // the real artist and album pages from music.youtube.com, saved while building this
    fn saved(name: &str) -> Option<Value> {
        let dir = std::env::var("OMNIPULSE_YT_SAMPLES").ok()?;
        serde_json::from_str(&std::fs::read_to_string(std::path::Path::new(&dir).join(name)).ok()?).ok()
    }

    #[test]
    fn reads_a_real_artist_and_album_page() {
        let (Some(a), Some(b)) = (saved("yt_artist.json"), saved("yt_album.json")) else { return };
        let page = parse_artist_page("UCUt2uP6O_UBJp4aBx5KjQjA", &a);
        assert_eq!(page.name, "Porter Robinson");
        assert!(page.songs.len() >= 5, "{:?}", page.songs);
        assert!(page.all_songs.starts_with("VL"), "{}", page.all_songs);
        assert!(page.albums.iter().any(|x| x.kind == "Album") && page.albums.iter().any(|x| x.kind != "Album"), "{:?}", page.albums);
        assert!(page.albums.iter().all(|x| x.id.starts_with("MPRE") && !x.title.is_empty()));
        let (album, tracks) = parse_album_page("MPREb_RvVZC5NhfWN", &b);
        assert_eq!(album.title, "SMILE! :D");
        assert_eq!(album.artists, "Porter Robinson");
        assert_eq!((album.kind.as_str(), album.year.as_str()), ("Album", "2024"));
        assert_eq!(tracks.len(), 10);
        assert!(tracks.iter().all(|t| t.duration_ms > 0 && t.album == "SMILE! :D" && t.artists.contains("Porter Robinson") && !t.thumb.is_empty()), "{:?}", tracks[0]);
    }
}
