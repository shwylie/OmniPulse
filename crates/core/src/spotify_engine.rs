// Spotify playback through librespot, so Spotify audio goes through the app's own audio chain
// (equalizer, leveling, volume, output device) like SoundCloud and local files.
//
// librespot decodes the track; PcmSink below hands the samples, interleaved stereo f32 at 44.1 kHz,
// to whatever the app attached (spe_attach): the Tauri app forwards them to an AudioWorklet, the
// native app plays them itself. The app reports how much it has played (spe_played), and the sink
// waits so it stays about LEAD_FRAMES ahead.
//
// librespot is an unofficial client. It signs in once with its own OAuth flow (Spotify's
// desktop client id) and keeps reusable credentials in <data dir>/librespot.

use librespot_core::{
    authentication::Credentials, cache::Cache, config::SessionConfig, session::Session,
    spotify_id::SpotifyId, SpotifyUri,
};
use librespot_oauth::OAuthClientBuilder;
use librespot_playback::{
    audio_backend::{Sink, SinkResult},
    config::{Bitrate, NormalisationMethod, NormalisationType, PlayerConfig},
    convert::Converter,
    decoder::AudioPacket,
    mixer::NoOpVolume,
    player::{Player, PlayerEvent},
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Where the decoded audio goes: interleaved stereo f32 at 44.1 kHz, tagged with the generation
/// it belongs to. Returns false if it couldn't be delivered.
pub trait PcmOut: Send + Sync {
    fn send(&self, gen: u32, samples: &[f32]) -> bool;
}

/// Player events: {"type": "playing" | "paused" | "end" | "unavailable" | "stopped", ...}
pub type EventOut = Arc<dyn Fn(Value) + Send + Sync>;

const RATE: u64 = 44100;
const LEAD_FRAMES: u64 = RATE * 3 / 4; // how far ahead of what's audible the sink may get
const BATCH_FRAMES: usize = 4410; // ~100 ms per message
const OAUTH_REDIRECT: &str = "http://127.0.0.1:5588/login";
const OAUTH_SCOPES: &[&str] = &[
    "streaming",
    "user-read-private",
    "user-read-email",
    "user-read-playback-state",
    "user-modify-playback-state",
    "user-library-read",
    "playlist-read-private",
];

// Shared between the commands, the sink (on librespot's player thread) and the event task
static PCM: Mutex<Option<Arc<dyn PcmOut>>> = Mutex::new(None);
static EVENTS: Mutex<Option<EventOut>> = Mutex::new(None);
static GEN: AtomicU32 = AtomicU32::new(0); // bumped on every load / seek; older audio is dropped
static SENT: AtomicU64 = AtomicU64::new(0); // frames sent for the current GEN
static PLAYED: AtomicU64 = AtomicU64::new(0); // frames the page has played for the current GEN
static PAUSED: AtomicBool = AtomicBool::new(false);

struct Engine {
    session: Session,
    player: Arc<Player>,
    norm: Norm,
}

// Whole-track loudness: Spotify's files carry their own loudness values, aimed at -14 LUFS, and
// librespot applies them. (on, target LUFS); a change takes effect from the next track.
type Norm = (bool, f64);
static NORM: Mutex<Norm> = Mutex::new((false, -14.0));

pub fn spe_set_normalisation(on: bool, target_lufs: f64) {
    *NORM.lock().unwrap() = (on, target_lufs.clamp(-30.0, -5.0));
}

fn make_player(session: &Session, norm: Norm) -> Arc<Player> {
    let config = PlayerConfig {
        bitrate: Bitrate::Bitrate320, // Premium quality
        normalisation: norm.0,
        normalisation_type: NormalisationType::Track,
        // Dynamic can turn quiet tracks up (with librespot's limiter); Basic only ever turns down
        normalisation_method: NormalisationMethod::Dynamic,
        normalisation_pregain_db: norm.1 + 14.0,
        ..PlayerConfig::default()
    };
    let player = Player::new(config, session.clone(), Box::new(NoOpVolume), || {
        Box::new(PcmSink { batch: Vec::with_capacity(BATCH_FRAMES * 2 + 4096), batch_gen: GEN.load(Ordering::SeqCst) })
            as Box<dyn Sink>
    });
    let mut events = player.get_player_event_channel();
    tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            match ev {
                PlayerEvent::Playing { position_ms, .. } => emit(json!({ "type": "playing", "position_ms": position_ms })),
                PlayerEvent::Paused { position_ms, .. } => emit(json!({ "type": "paused", "position_ms": position_ms })),
                PlayerEvent::EndOfTrack { .. } => emit(json!({ "type": "end" })),
                PlayerEvent::Unavailable { .. } => emit(json!({ "type": "unavailable" })),
                // a track with nothing left to play is swapped by librespot for an "alternative"
                // (another release), which can be a different song: the page checks
                PlayerEvent::TrackChanged { audio_item } => {
                    let asked = REQUESTED.lock().unwrap().clone();
                    if !asked.is_empty() && audio_item.uri != asked {
                        emit(json!({ "type": "alternative", "title": audio_item.name, "artists": audio_item_artists(&audio_item) }));
                    }
                }
                PlayerEvent::Stopped { .. } => emit(json!({ "type": "stopped" })),
                _ => {}
            }
        }
    });
    player
}
static ENGINE: tokio::sync::Mutex<Option<Engine>> = tokio::sync::Mutex::const_new(None);
/// The track the page asked for (to spot an alternative played in its place)
static REQUESTED: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

fn audio_item_artists(a: &librespot_metadata::audio::AudioItem) -> String {
    match &a.unique_fields {
        librespot_metadata::audio::UniqueFields::Track { album_artists, .. } => album_artists.join(", "),
        _ => String::new(),
    }
}

fn cache_dir() -> Result<std::path::PathBuf, String> {
    Ok(crate::data_dir()?.join("librespot"))
}

fn cache() -> Result<Cache, String> {
    let dir = cache_dir()?;
    Cache::new(Some(&dir), None, None, None).map_err(|e| format!("librespot cache: {e}"))
}

fn emit(v: Value) {
    let out = EVENTS.lock().unwrap().clone();
    if let Some(out) = out {
        out(v);
    }
}

// Starts a new stretch of audio (new track or seek): the page throws away what it has buffered
fn new_gen() -> u32 {
    let g = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    SENT.store(0, Ordering::SeqCst);
    PLAYED.store(0, Ordering::SeqCst);
    g
}

// ---------- the sink ----------
struct PcmSink {
    batch: Vec<f32>,
    batch_gen: u32,
}

impl PcmSink {
    fn send(&mut self) {
        if self.batch.is_empty() {
            return;
        }
        let gen = self.batch_gen;
        let frames = (self.batch.len() / 2) as u64;
        let out = PCM.lock().unwrap().clone();
        let delivered = out.is_some_and(|out| out.send(gen, &self.batch));
        self.batch.clear();
        if !delivered {
            return;
        }
        SENT.fetch_add(frames, Ordering::SeqCst);
        // Stay a little ahead of what's audible. Stop waiting on a seek / new track, a pause,
        // or if the page stops answering (reloaded), so player commands never get stuck.
        let start = Instant::now();
        while GEN.load(Ordering::SeqCst) == gen
            && !PAUSED.load(Ordering::SeqCst)
            && SENT.load(Ordering::SeqCst).saturating_sub(PLAYED.load(Ordering::SeqCst)) > LEAD_FRAMES
            && start.elapsed() < Duration::from_secs(5)
        {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Sink for PcmSink {
    fn stop(&mut self) -> SinkResult<()> {
        self.send();
        Ok(())
    }

    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        let gen = GEN.load(Ordering::SeqCst);
        if gen != self.batch_gen {
            self.batch.clear(); // audio from before a seek / track change
            self.batch_gen = gen;
        }
        if let AudioPacket::Samples(samples) = packet {
            self.batch.extend(converter.f64_to_f32(&samples));
            if self.batch.len() >= BATCH_FRAMES * 2 {
                self.send();
            }
        }
        Ok(())
    }
}

// ---------- session and player ----------
// Using the engine (connecting, its tokens, its account) is marked on disk while it happens: Spotify's
// library can fail hard for some accounts (a free one), below where a crash is caught and written down,
// taking the app with it. A mark still there when the app starts again means it did: the saved sign-in
// is let go (and that's said), instead of the app closing again on every start.
static BUSY: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static STARTED: std::sync::Once = std::sync::Once::new();
static CRASHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn busy_mark() -> Option<std::path::PathBuf> {
    crate::data_dir().ok().map(|d| d.join("spotify_busy.txt"))
}

/// Marked while it's held (the first in writes the mark, the last out takes it away)
struct Busy;

impl Busy {
    fn new() -> Self {
        start_check();
        if BUSY.fetch_add(1, Ordering::SeqCst) == 0 {
            if let Some(m) = busy_mark() { let _ = std::fs::write(m, "busy"); }
        }
        Busy
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        if BUSY.fetch_sub(1, Ordering::SeqCst) == 1 {
            if let Some(m) = busy_mark() { let _ = std::fs::remove_file(m); }
        }
    }
}

// (once a run, before the engine's first use: last time's mark found, the saved sign-in let go)
fn start_check() {
    STARTED.call_once(|| {
        let Ok(dir) = crate::data_dir() else { return };
        let marks = [dir.join("spotify_busy.txt"), dir.join("spotify_connecting.txt")];
        if marks.iter().any(|m| m.exists()) {
            marks.iter().for_each(|m| { let _ = std::fs::remove_file(m); });
            if let Ok(c) = cache_dir() { let _ = std::fs::remove_dir_all(c); }
            CRASHED.store(true, Ordering::SeqCst);
            crate::diag::note("spotify playback: last run went down while using it: signed out of it");
        }
    });
}

/// Whether Spotify playback took the app down last time (and so its sign-in was let go)
pub fn spe_crashed_last_time() -> bool {
    start_check();
    CRASHED.load(Ordering::SeqCst)
}

async fn connect(credentials: Credentials) -> Result<Engine, String> {
    let _busy = Busy::new();
    crate::diag::note("spotify playback: connecting");
    let r = connect_now(credentials).await;
    crate::diag::note(&match &r { Ok(_) => "spotify playback: connected".to_string(), Err(e) => format!("spotify playback: connect failed: {e}") });
    r
}

async fn connect_now(credentials: Credentials) -> Result<Engine, String> {
    let session = Session::new(SessionConfig::default(), Some(cache()?));
    session
        .connect(credentials, true)
        .await
        .map_err(|e| format!("Spotify playback sign-in failed: {e}"))?;
    crate::diag::note("spotify playback: session up, making its player");
    // (its account type, kept: a free one is never played through)
    let _ = account_type(&session).await;
    let norm = *NORM.lock().unwrap();
    let player = make_player(&session, norm);
    Ok(Engine { session, player, norm })
}

// The running engine, reconnecting from saved credentials if needed
async fn engine() -> Result<tokio::sync::MutexGuard<'static, Option<Engine>>, String> {
    start_check();
    let mut guard = ENGINE.lock().await;
    let healthy = guard.as_ref().is_some_and(|e| !e.session.is_invalid() && !e.player.is_invalid());
    if !healthy {
        *guard = None;
        let creds = cache()?
            .credentials()
            .ok_or_else(|| "NOT_SIGNED_IN".to_string())?;
        *guard = Some(connect(creds).await?);
    }
    Ok(guard)
}

fn track_uri(uri: &str) -> Result<SpotifyUri, String> {
    let id = uri.strip_prefix("spotify:track:").ok_or_else(|| format!("not a Spotify track: {uri}"))?;
    let id = SpotifyId::from_base62(id).map_err(|e| format!("bad Spotify track id: {e}"))?;
    Ok(SpotifyUri::Track { id })
}

// ---------- commands ----------
pub fn spe_signed_in() -> bool {
    start_check();
    cache().ok().and_then(|c| c.credentials()).is_some()
}

pub async fn spe_login() -> Result<(), String> {
    let client_id = SessionConfig::default().client_id;
    let client = OAuthClientBuilder::new(&client_id, OAUTH_REDIRECT, OAUTH_SCOPES.to_vec())
        .open_in_browser()
        .with_custom_message("Signed in to Spotify playback. You can close this tab and go back to OmniPulse.")
        .build()
        .map_err(|e| format!("Spotify sign-in: {e}"))?;
    crate::diag::note("spotify playback sign-in: in the browser");
    let token = client
        .get_access_token_async()
        .await
        .map_err(|e| { crate::diag::note(&format!("spotify playback sign-in: the browser's sign-in failed: {e}")); format!("Spotify sign-in: {e}") })?;
    crate::diag::note("spotify playback sign-in: signed in in the browser");
    let engine = connect(Credentials::with_access_token(token.access_token)).await?;
    // a free account stays signed in, for its library (likes, playlists) through Spotify's Web API;
    // only playing through it is Premium's (spe_play turns it down: its songs play from YouTube Music)
    let product = account_type(&engine.session).await;
    crate::diag::note(&format!("spotify sign-in: a {} account", if product.is_empty() { "(not said)" } else { product.as_str() }));
    *ENGINE.lock().await = Some(engine);
    Ok(())
}

pub async fn spe_logout() -> Result<(), String> {
    if let Ok(mut p) = PRODUCT.lock() { p.clear(); }
    if let Ok(d) = crate::data_dir() { let _ = std::fs::remove_file(d.join("spotify_account.txt")); }
    if let Some(e) = ENGINE.lock().await.take() {
        e.player.stop();
        e.session.shutdown();
    }
    let dir = cache_dir()?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// The app hands over where the audio and the player events go
pub fn spe_attach(audio: Arc<dyn PcmOut>, events: EventOut) {
    *PCM.lock().unwrap() = Some(audio);
    *EVENTS.lock().unwrap() = Some(events);
}

// Returns the generation number the page should accept audio for
pub async fn spe_play(uri: String, position_ms: u32) -> Result<u32, String> {
    // (a free account: never played through Spotify; the app plays its songs from YouTube Music)
    if spe_premium() == Some(false) { return Err("Premium account required to play through Spotify (this one plays from YouTube Music)".into()); }
    let track = track_uri(&uri)?;
    *REQUESTED.lock().unwrap() = uri.clone();
    let mut guard = engine().await?;
    let e = guard.as_mut().unwrap();
    let norm = *NORM.lock().unwrap();
    if e.norm != norm {
        // the loudness setting changed: a player with the new setting, from this track on
        e.player.stop();
        e.player = make_player(&e.session, norm);
        e.norm = norm;
    }
    PAUSED.store(false, Ordering::SeqCst);
    let gen = new_gen();
    e.player.load(track, true, position_ms);
    Ok(gen)
}

pub async fn spe_pause() -> Result<(), String> {
    PAUSED.store(true, Ordering::SeqCst);
    if let Some(e) = ENGINE.lock().await.as_ref() {
        e.player.pause();
    }
    Ok(())
}

pub async fn spe_resume() -> Result<(), String> {
    PAUSED.store(false, Ordering::SeqCst);
    if let Some(e) = ENGINE.lock().await.as_ref() {
        e.player.play();
    }
    Ok(())
}

pub async fn spe_seek(position_ms: u32) -> Result<u32, String> {
    let guard = ENGINE.lock().await;
    let e = guard.as_ref().ok_or("Spotify playback isn't running")?;
    let gen = new_gen();
    e.player.seek(position_ms);
    Ok(gen)
}

pub async fn spe_stop() -> Result<(), String> {
    new_gen();
    if let Some(e) = ENGINE.lock().await.as_ref() {
        e.player.stop();
    }
    Ok(())
}

// The app reports how many frames of this generation it has played
pub fn spe_played(gen: u32, frames: u64) {
    if gen == GEN.load(Ordering::SeqCst) {
        PLAYED.store(frames, Ordering::SeqCst);
    }
}

// ---------- playlists the Web API won't open ----------
// Since February 2026 the Web API only lists the tracks of playlists you own. librespot reads
// any playlist the way Spotify's own app does; this returns its tracks shaped like the Web API's
// track objects, so the page treats them the same way.
const PLAYLIST_MAX: usize = 1000;

fn uri_id(uri: &SpotifyUri) -> String {
    uri.to_uri().ok().and_then(|u| u.rsplit(':').next().map(str::to_string)).unwrap_or_default()
}

fn track_json(t: &librespot_metadata::Track) -> Value {
    let cover = t.album.covers.iter().max_by_key(|c| c.width).and_then(|c| c.id.to_base16().ok())
        .map(|hex| format!("https://i.scdn.co/image/{hex}"));
    let mut artists: Vec<Value> = t.artists.iter().filter(|a| !a.name.is_empty()).map(|a| json!({ "name": a.name })).collect();
    if artists.is_empty() {
        artists = t.artists_with_role.iter().map(|a| json!({ "name": a.name })).collect();
    }
    let id = uri_id(&t.id);
    json!({
        "id": id,
        "uri": format!("spotify:track:{id}"),
        "name": t.name,
        "artists": artists,
        "duration_ms": t.duration,
        "external_urls": { "spotify": format!("https://open.spotify.com/track/{id}") },
        "album": {
            "id": uri_id(&t.album.id),
            "name": t.album.name,
            "images": cover.map(|u| vec![json!({ "url": u })]).unwrap_or_default(),
            "album_type": "album",
            "artists": t.album.artists.iter().map(|a| json!({ "name": a.name })).collect::<Vec<_>>(),
        },
    })
}

pub async fn spe_playlist(uri: String) -> Result<Value, String> {
    use futures::stream::{self, StreamExt};
    use librespot_metadata::{Metadata, Playlist, Track};
    let session = {
        let guard = engine().await?;
        guard.as_ref().unwrap().session.clone()
    };
    let id = SpotifyUri::from_uri(&uri).map_err(|e| format!("bad playlist address: {e}"))?;
    let playlist = Playlist::get(&session, &id).await.map_err(|e| format!("Spotify wouldn't open this playlist: {e}"))?;
    let ids: Vec<SpotifyUri> = playlist.tracks()
        .filter(|u| matches!(u, SpotifyUri::Track { .. }))   // podcast episodes are left out
        .take(PLAYLIST_MAX)
        .cloned()
        .collect();
    // ten at a time, kept in the playlist's order; tracks that can't be read are skipped
    let tracks: Vec<Value> = stream::iter(ids)
        .map(|tid| {
            let session = session.clone();
            async move { Track::get(&session, &tid).await.ok().map(|t| track_json(&t)) }
        })
        .buffered(10)
        .filter_map(|t| async move { t })
        .collect()
        .await;
    Ok(Value::Array(tracks))
}

/// A session's account type ("premium", "free", ...): it comes just after its login, so it's waited
/// for (a few seconds at most; "" if it never says). Kept, for spe_premium.
async fn account_type(session: &Session) -> String {
    for _ in 0..30 {
        if let Some(t) = session.get_user_attribute("type").filter(|t| !t.is_empty()) {
            if let Ok(mut p) = PRODUCT.lock() { *p = t.clone(); }
            // (kept on disk too: known from the start next time, before it connects)
            if let Ok(d) = crate::data_dir() { let _ = std::fs::write(d.join("spotify_account.txt"), &t); }
            return t;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    String::new()
}

static PRODUCT: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Whether the account signed in can play through Spotify here (Premium), if it's known yet. A free
/// one is signed in for its library only: its songs play from YouTube Music.
pub fn spe_premium() -> Option<bool> {
    let mut p = PRODUCT.lock().ok()?.clone();
    if p.is_empty() {
        p = crate::data_dir().ok().and_then(|d| std::fs::read_to_string(d.join("spotify_account.txt")).ok()).unwrap_or_default().trim().to_string();
    }
    (!p.is_empty()).then(|| p == "premium")
}

/// The playback sign-in's account, from its own login (not the Web API, so no rate limit can stand in
/// the way of signing in): its username and its product ("premium", "free", ...)
pub async fn spe_account() -> Result<Value, String> {
    let _busy = Busy::new();
    let session = {
        let guard = engine().await?;
        guard.as_ref().unwrap().session.clone()
    };
    let product = account_type(&session).await;
    crate::diag::note(&format!("spotify playback: account read: {}", if product.is_empty() { "(none given)" } else { product.as_str() }));
    Ok(json!({ "id": session.username(), "product": product }))
}

// ---------- the Web API through the playback sign-in ----------
// The playback engine signs in as Spotify's own app, so its token works for the Web API too: the
// app uses it for everything (library, likes, search, playlists), and a developer app of your own
// is only needed for a free account, which the playback sign-in turns down. librespot keeps the
// token until it expires.
const WEB_SCOPES: &str = "user-read-private,user-read-email,user-library-read,user-library-modify,playlist-read-private,playlist-read-collaborative,playlist-modify-private,playlist-modify-public,user-follow-read,user-follow-modify,user-read-playback-state,user-modify-playback-state";

pub async fn spe_web_token() -> Result<String, String> {
    let _busy = Busy::new();
    crate::diag::note("spotify playback: a Web API token");
    let session = {
        let guard = engine().await?;
        guard.as_ref().unwrap().session.clone()
    };
    match session.login5().auth_token().await {
        Ok(t) => Ok(t.access_token),
        Err(_) => session
            .token_provider()
            .get_token(WEB_SCOPES)
            .await
            .map(|t| t.access_token)
            .map_err(|e| format!("Spotify playback token: {e}")),
    }
}

// Daily Mixes and the rest of "Made For You" are hidden from developer apps' Web API tokens
// (since November 2024); the playback sign-in's token can read them. Only GET requests, only to
// api.spotify.com.
pub async fn spe_web_get(path: String) -> Result<Value, String> {
    if !path.starts_with('/') || path.contains("://") {
        return Err("bad path".into());
    }
    let token = spe_web_token().await?;
    let res = reqwest::Client::new()
        .get(format!("https://api.spotify.com/v1{path}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    let body = res.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("Spotify said {}: {}", status.as_u16(), body.chars().take(200).collect::<String>()));
    }
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

#[cfg(test)]
mod account_live {
    /// cargo test -p omnipulse-core spe_account_live -- --ignored --nocapture (this PC's saved playback sign-in)
    #[test]
    #[ignore]
    fn spe_account_live() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let v = rt.block_on(super::spe_account()).expect("the account");
        println!("product {:?}, username set: {}", v["product"], v["id"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(!v["product"].as_str().unwrap_or("").is_empty());
    }
}

#[cfg(test)]
mod crash_mark_tests {
    #[test]
    fn a_mark_left_by_a_crash_signs_playback_out_on_the_next_start() {
        let _turn = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("omnipulse-spemark-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        crate::set_data_dir(dir.clone());
        // last run: went down while using the engine (the mark left), signed in (credentials kept)
        std::fs::create_dir_all(dir.join("librespot")).unwrap();
        std::fs::write(dir.join("librespot").join("credentials.json"), "{}").unwrap();
        std::fs::write(dir.join("spotify_busy.txt"), "busy").unwrap();
        assert!(super::spe_crashed_last_time());
        assert!(!dir.join("librespot").exists() && !dir.join("spotify_busy.txt").exists());
        assert!(!super::spe_signed_in());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
