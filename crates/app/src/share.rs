// Sharing (as the web app's js/share.js). A track: its own link on each platform it's on (its own,
// and the same song found on the others), not an OmniPulse one. A playlist: a page on your relay
// (jam-server/) with every song's links on each platform, the same as the web app makes; or the
// SoundCloud or Spotify copy's own link (crate::mirror).

use crate::sources::Playlist;
use crate::track::{Src, Track};
use omnipulse_core as core;
use serde_json::{json, Value};

/// OmniPulse's relay (jam-server/, on Cloudflare), unless one was set by hand before
pub const RELAY_URL: &str = "https://omnipulse-jam.shwylie101.workers.dev";

pub const PLATS: [(&str, &str); 3] = [("soundcloud", "SoundCloud"), ("spotify", "Spotify"), ("youtube", "YouTube Music")];

pub fn name(plat: &str) -> &'static str {
    PLATS.iter().find(|(k, _)| *k == plat).map_or("The", |(_, n)| n)
}

fn sp_link(uri: &str) -> String {
    format!("https://open.spotify.com/track/{}", uri.rsplit(':').next().unwrap_or(""))
}

/// Where a track is on each platform: (platform, its link)
pub fn links(t: &Track) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    match &t.src {
        Src::SoundCloud(_) if !t.link.is_empty() => out.push(("soundcloud", t.link.clone())),
        Src::Spotify(uri) => out.push(("spotify", sp_link(uri))),
        Src::YouTube(id) => out.push(("youtube", format!("https://music.youtube.com/watch?v={id}"))),
        _ => {}
    }
    if !out.iter().any(|(p, _)| *p == "spotify") {
        if let Some(uri) = crate::mirror::matched(t, "spotify") { out.push(("spotify", sp_link(&uri))); }
    }
    if !out.iter().any(|(p, _)| *p == "youtube") {
        if let Some(id) = crate::versions::saved_id(&t.key) { out.push(("youtube", format!("https://music.youtube.com/watch?v={id}"))); }
    }
    out.sort_by_key(|(p, _)| PLATS.iter().position(|(k, _)| k == p));
    out
}

pub fn copy(text: &str) -> Result<(), String> {
    arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string())).map_err(|e| format!("Couldn't copy it: {e}"))
}

/// A playlist's id on a platform: the copy kept there for one here, or the playlist itself
pub fn platform_id(p: &Playlist, xpl: &Value, plat: &str) -> Option<String> {
    if crate::xpl::is_own(&p.id) {
        let x = xpl.as_array()?.iter().find(|x| x["id"].as_str() == Some(p.id.as_str()))?;
        return x["mirrors"][plat].as_str().map(String::from);
    }
    (p.plat == plat).then(|| p.id.clone())
}

/// The platform's own link for a playlist (a private SoundCloud one: its secret link; a hidden
/// Spotify copy made for one here: made public first, so the link opens for others)
pub async fn platform_link(plat: &str, id: &str, made: bool) -> Result<String, String> {
    if plat == "spotify" {
        if made {
            let _ = core::spotify::spotify_api("PUT".into(), format!("/playlists/{id}"), Some(json!({ "public": true }))).await;
        }
        return Ok(format!("https://open.spotify.com/playlist/{id}"));
    }
    let sc = core::soundcloud::api_get(format!("/playlists/{id}")).await?;
    let link = sc["permalink_url"].as_str().ok_or("SoundCloud didn't give its link")?.to_string();
    Ok(match (sc["sharing"].as_str(), sc["secret_token"].as_str()) {
        (Some("private"), Some(tok)) => format!("{link}/{tok}"),
        _ => link,
    })
}

/// Shares a playlist on the relay (again, if it was before: the same link, up to date). Returns its
/// link and what to keep to update it.
pub async fn share_playlist(relay: &str, title: &str, by: &str, tracks: &[Track], was: &Value) -> Result<(String, Value), String> {
    let body = json!({
        "title": title,
        "by": by,
        "tracks": tracks.iter().filter(|t| !matches!(t.src, Src::Local(_))).map(|t| {
            let links: serde_json::Map<String, Value> = links(t).into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
            json!({ "title": t.title, "artist": t.artist, "ms": t.ms, "links": links, "snap": t.to_snapshot() })
        }).collect::<Vec<_>>(),
    });
    let client = reqwest::Client::new();
    if was["relay"].as_str() == Some(relay) {
        if let (Some(code), Some(key)) = (was["code"].as_str(), was["key"].as_str()) {
            let r = client.put(format!("{relay}/api/shares/{code}?key={}", crate::sources::urlencode(key))).json(&body).send().await.map_err(|e| e.to_string())?;
            if r.status().is_success() {
                return Ok((format!("{relay}/p/{code}"), was.clone()));
            }
        }
    }
    let r = client.post(format!("{relay}/api/shares")).json(&body).send().await.map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("the relay answered {}", r.status().as_u16()));
    }
    let made: Value = r.json().await.map_err(|e| e.to_string())?;
    let url = made["url"].as_str().ok_or("the relay didn't give a link")?.to_string();
    Ok((url, json!({ "code": made["code"], "key": made["key"], "relay": relay })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tracks_own_link() {
        let sp = Track { src: Src::Spotify("spotify:track:abc".into()), key: "sp:abc".into(), ..Default::default() };
        assert_eq!(links(&sp)[0], ("spotify", "https://open.spotify.com/track/abc".to_string()));
        let sc = Track { src: Src::SoundCloud(1), key: "1".into(), link: "https://soundcloud.com/a/b".into(), ..Default::default() };
        assert_eq!(links(&sc)[0].0, "soundcloud");
        let private = Track { src: Src::SoundCloud(2), key: "2".into(), ..Default::default() };
        assert!(links(&private).iter().all(|(p, _)| *p != "soundcloud"));
    }
}
