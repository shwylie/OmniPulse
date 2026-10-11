// Your Spotify likes through the playback sign-in's own session, the way Spotify's apps keep them
// (its library service, "collection v2"), not the Web API: that one limits requests per app, and
// the sign-in's app is shared with everyone signing in that way, so it's often too busy to answer.
// A free account signs in this way too (it only can't play through it).
//
// The library service speaks protobuf; its few messages are encoded here by hand.

use crate::spotify_engine::spe_session;

// ---------- protobuf, by hand ----------
fn varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 { out.push(b); return; }
        out.push(b | 0x80);
    }
}

fn field_str(out: &mut Vec<u8>, n: u32, s: &str) {
    if s.is_empty() { return; }
    varint(out, ((n << 3) | 2) as u64);
    varint(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

fn field_bytes(out: &mut Vec<u8>, n: u32, b: &[u8]) {
    varint(out, ((n << 3) | 2) as u64);
    varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

fn field_int(out: &mut Vec<u8>, n: u32, v: u64) {
    if v == 0 { return; }
    varint(out, (n << 3) as u64);
    varint(out, v);
}

/// A message's fields: (number, a varint's value, or a length-delimited field's bytes)
enum Val<'a> { Int(u64), Bytes(&'a [u8]) }

fn fields(mut b: &[u8]) -> Result<Vec<(u32, Val<'_>)>, String> {
    fn read_varint(b: &mut &[u8]) -> Result<u64, String> {
        let mut v = 0u64;
        for i in 0..10 {
            let (&x, rest) = b.split_first().ok_or("cut short")?;
            *b = rest;
            v |= ((x & 0x7f) as u64) << (7 * i);
            if x & 0x80 == 0 { return Ok(v); }
        }
        Err("bad varint".into())
    }
    let mut out = Vec::new();
    while !b.is_empty() {
        let key = read_varint(&mut b)?;
        let n = (key >> 3) as u32;
        match key & 7 {
            0 => out.push((n, Val::Int(read_varint(&mut b)?))),
            2 => {
                let len = read_varint(&mut b)? as usize;
                if len > b.len() { return Err("cut short".into()); }
                out.push((n, Val::Bytes(&b[..len])));
                b = &b[len..];
            }
            1 => { if b.len() < 8 { return Err("cut short".into()); } b = &b[8..]; }
            5 => { if b.len() < 4 { return Err("cut short".into()); } b = &b[4..]; }
            w => return Err(format!("wire type {w}")),
        }
    }
    Ok(out)
}

fn text(v: &Val) -> String {
    match v { Val::Bytes(b) => String::from_utf8_lossy(b).into_owned(), Val::Int(_) => String::new() }
}

// ---------- the library service ----------
/// One liked item: its uri, when it was liked (unix seconds), and whether it's been taken out
#[derive(Debug, Clone, PartialEq)]
pub struct Liked { pub uri: String, pub added_at: u64, pub removed: bool }

async fn post(path: &str, body: Vec<u8>) -> Result<Vec<u8>, String> {
    let session = spe_session().await?;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::CONTENT_TYPE, reqwest::header::HeaderValue::from_static("application/vnd.collection-v2.spotify.proto"));
    headers.insert(reqwest::header::ACCEPT, reqwest::header::HeaderValue::from_static("application/vnd.collection-v2.spotify.proto"));
    session.spclient()
        .request(&reqwest::Method::POST, path, Some(headers), Some(&body))
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("Spotify's library: {e}"))
}

/// A page of your liked songs, from the newest; the next page's token ("" at the end)
pub async fn liked_page(token: &str, limit: u32) -> Result<(Vec<Liked>, String), String> {
    let username = spe_session().await?.username();
    let mut req = Vec::new();
    field_str(&mut req, 1, &username);
    field_str(&mut req, 2, "collection");
    field_str(&mut req, 3, token);
    field_int(&mut req, 4, limit as u64);
    let res = post("/collection/v2/paging", req).await?;
    let mut items = Vec::new();
    let mut next = String::new();
    for (n, v) in fields(&res)? {
        match (n, &v) {
            (1, Val::Bytes(b)) => {
                let mut it = Liked { uri: String::new(), added_at: 0, removed: false };
                for (m, w) in fields(b)? {
                    match (m, &w) {
                        (1, _) => it.uri = text(&w),
                        (2, Val::Int(x)) => it.added_at = *x,
                        (3, Val::Int(x)) => it.removed = *x != 0,
                        _ => {}
                    }
                }
                items.push(it);
            }
            (2, _) => next = text(&v),
            _ => {}
        }
    }
    Ok((items, next))
}

/// Likes or unlikes songs (spotify:track:... uris)
pub async fn set_liked(uris: &[String], on: bool) -> Result<(), String> {
    let username = spe_session().await?.username();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut req = Vec::new();
    field_str(&mut req, 1, &username);
    field_str(&mut req, 2, "collection");
    for uri in uris {
        let mut item = Vec::new();
        field_str(&mut item, 1, uri);
        field_int(&mut item, 2, now);
        if !on { field_int(&mut item, 3, 1); }
        field_bytes(&mut req, 3, &item);
    }
    post("/collection/v2/write", req).await.map(|_| ())
}

/// All your liked songs (not albums), newest first, from Spotify's library
pub async fn liked_all() -> Result<Vec<Liked>, String> {
    let mut all = Vec::new();
    let mut token = String::new();
    for _ in 0..200 {
        let (page, next) = liked_page(&token, 300).await?;
        all.extend(page.into_iter().filter(|l| !l.removed && l.uri.starts_with("spotify:track:")));
        if next.is_empty() { break; }
        token = next;
    }
    all.sort_by(|a, b| b.added_at.cmp(&a.added_at));
    Ok(all)
}

/// Your liked songs for the page: every one's id, newest first, and the songs themselves (shaped like
/// the Web API's) for those it doesn't know yet (known: ids it keeps already), read ten at a time
pub async fn spe_liked(known: Vec<String>) -> Result<serde_json::Value, String> {
    use futures::stream::{self, StreamExt};
    use librespot_metadata::{Metadata, Track};
    let liked = liked_all().await?;
    let known: std::collections::HashSet<String> = known.into_iter().collect();
    let ids: Vec<String> = liked.iter().map(|l| l.uri.trim_start_matches("spotify:track:").to_string()).collect();
    let session = spe_session().await?;
    let todo: Vec<String> = liked.iter().map(|l| l.uri.clone()).filter(|u| !known.contains(u.trim_start_matches("spotify:track:"))).collect();
    let tracks: Vec<serde_json::Value> = stream::iter(todo)
        .map(|uri| {
            let session = session.clone();
            async move {
                let id = librespot_core::SpotifyUri::from_uri(&uri).ok()?;
                Track::get(&session, &id).await.ok().map(|t| crate::spotify_engine::track_json(&t))
            }
        })
        .buffered(10)
        .filter_map(|t| async move { t })
        .collect()
        .await;
    crate::diag::note(&format!("spotify library: {} liked songs, {} read now", ids.len(), tracks.len()));
    Ok(serde_json::json!({ "ids": ids, "tracks": tracks }))
}

/// Likes or unlikes one song (its spotify:track:... uri)
pub async fn spe_set_liked(uri: String, on: bool) -> Result<(), String> {
    if !uri.starts_with("spotify:track:") { return Err("not a Spotify song".into()); }
    set_liked(&[uri], on).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_request_encodes_and_a_response_decodes() {
        let mut req = Vec::new();
        field_str(&mut req, 1, "me");
        field_str(&mut req, 2, "collection");
        field_int(&mut req, 4, 300);
        assert_eq!(req, [0x0a, 2, b'm', b'e', 0x12, 10, b'c', b'o', b'l', b'l', b'e', b'c', b't', b'i', b'o', b'n', 0x20, 0xac, 0x02]);
        // a response: one item (uri, added_at), then the next token
        let mut item = Vec::new();
        field_str(&mut item, 1, "spotify:track:x");
        field_int(&mut item, 2, 1_700_000_000);
        let mut res = Vec::new();
        field_bytes(&mut res, 1, &item);
        field_str(&mut res, 2, "next");
        let f = fields(&res).unwrap();
        assert!(matches!(f[0], (1, Val::Bytes(_))));
        assert_eq!(text(&f[1].1), "next");
        let Val::Bytes(b) = f[0].1 else { panic!() };
        let g = fields(b).unwrap();
        assert_eq!(text(&g[0].1), "spotify:track:x");
        assert!(matches!(g[1], (2, Val::Int(1_700_000_000))));
    }

    /// cargo test -p omnipulse-core liked_live -- --ignored --nocapture (this PC's saved playback
    /// sign-in; reads only)
    #[test]
    #[ignore]
    fn liked_live() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (first, mut next) = rt.block_on(liked_page("", 50)).expect("a page");
        println!("first page: {} items, next token: {}", first.len(), !next.is_empty());
        println!("first: {:?}", first.first());
        let mut total = first.len();
        let mut pages = 1;
        while !next.is_empty() && pages < 100 {
            let (p, n) = rt.block_on(liked_page(&next, 300)).expect("a page");
            total += p.len();
            next = n;
            pages += 1;
        }
        println!("{total} in {pages} pages");
        assert!(total > 0);
    }

    /// cargo test -p omnipulse-core liked_tracks_live -- --ignored --nocapture (reads only: every
    /// liked song's id, and five of them in full)
    #[test]
    #[ignore]
    fn liked_tracks_live() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let all = rt.block_on(liked_all()).expect("the likes");
        println!("{} liked songs; newest {:?}", all.len(), all.first());
        let known: Vec<String> = all.iter().skip(5).map(|l| l.uri.trim_start_matches("spotify:track:").to_string()).collect();
        let t0 = std::time::Instant::now();
        let v = rt.block_on(spe_liked(known)).expect("spe_liked");
        let tracks = v["tracks"].as_array().unwrap();
        println!("{} ids, {} read in {:?}", v["ids"].as_array().unwrap().len(), tracks.len(), t0.elapsed());
        for t in tracks { println!("  {} - {} ({} ms, art {})", t["artists"][0]["name"], t["name"], t["duration_ms"], t["album"]["images"][0]["url"].is_string()); }
        assert_eq!(tracks.len(), 5);
    }
}
