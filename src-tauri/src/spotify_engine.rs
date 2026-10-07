// Spotify playback through librespot, so Spotify audio goes through the app's own audio chain
// (equalizer, leveling, volume, output device) like SoundCloud and local files.
//
// librespot decodes the track; PcmSink below hands the samples to the page over a Tauri
// channel as interleaved stereo f32 at 44.1 kHz, where an AudioWorklet plays them. The page
// reports how much it has played, and the sink waits so it stays about LEAD_FRAMES ahead.
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
    config::{Bitrate, PlayerConfig},
    convert::Converter,
    decoder::AudioPacket,
    mixer::NoOpVolume,
    player::{Player, PlayerEvent},
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::ipc::{Channel, InvokeResponseBody};

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
static PCM: Mutex<Option<Channel<InvokeResponseBody>>> = Mutex::new(None);
static EVENTS: Mutex<Option<Channel<Value>>> = Mutex::new(None);
static GEN: AtomicU32 = AtomicU32::new(0); // bumped on every load / seek; older audio is dropped
static SENT: AtomicU64 = AtomicU64::new(0); // frames sent for the current GEN
static PLAYED: AtomicU64 = AtomicU64::new(0); // frames the page has played for the current GEN
static PAUSED: AtomicBool = AtomicBool::new(false);

struct Engine {
    session: Session,
    player: Arc<Player>,
}
static ENGINE: tokio::sync::Mutex<Option<Engine>> = tokio::sync::Mutex::const_new(None);

fn cache_dir() -> Result<std::path::PathBuf, String> {
    Ok(crate::data_dir()?.join("librespot"))
}

fn cache() -> Result<Cache, String> {
    let dir = cache_dir()?;
    Cache::new(Some(&dir), None, None, None).map_err(|e| format!("librespot cache: {e}"))
}

fn emit(v: Value) {
    if let Some(ch) = EVENTS.lock().unwrap().as_ref() {
        let _ = ch.send(v);
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
        // [gen: u32][reserved: u32][f32 samples...], little endian
        let mut bytes = Vec::with_capacity(8 + self.batch.len() * 4);
        bytes.extend_from_slice(&gen.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        for s in &self.batch {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        self.batch.clear();
        let ch = PCM.lock().unwrap().clone();
        let Some(ch) = ch else { return };
        if ch.send(InvokeResponseBody::Raw(bytes)).is_err() {
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
async fn connect(credentials: Credentials) -> Result<Engine, String> {
    let session = Session::new(SessionConfig::default(), Some(cache()?));
    session
        .connect(credentials, true)
        .await
        .map_err(|e| format!("Spotify playback sign-in failed: {e}"))?;
    let config = PlayerConfig { bitrate: Bitrate::Bitrate320, ..PlayerConfig::default() };   // Premium quality
    let player = Player::new(config, session.clone(), Box::new(NoOpVolume), || {
        Box::new(PcmSink { batch: Vec::with_capacity(BATCH_FRAMES * 2 + 4096), batch_gen: GEN.load(Ordering::SeqCst) })
            as Box<dyn Sink>
    });
    let mut events = player.get_player_event_channel();
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = events.recv().await {
            match ev {
                PlayerEvent::Playing { position_ms, .. } => emit(json!({ "type": "playing", "position_ms": position_ms })),
                PlayerEvent::Paused { position_ms, .. } => emit(json!({ "type": "paused", "position_ms": position_ms })),
                PlayerEvent::EndOfTrack { .. } => emit(json!({ "type": "end" })),
                PlayerEvent::Unavailable { .. } => emit(json!({ "type": "unavailable" })),
                PlayerEvent::Stopped { .. } => emit(json!({ "type": "stopped" })),
                _ => {}
            }
        }
    });
    Ok(Engine { session, player })
}

// The running engine, reconnecting from saved credentials if needed
async fn engine() -> Result<tokio::sync::MutexGuard<'static, Option<Engine>>, String> {
    let mut guard = ENGINE.lock().await;
    let healthy = guard.as_ref().map_or(false, |e| !e.session.is_invalid() && !e.player.is_invalid());
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
#[tauri::command]
pub fn spe_signed_in() -> bool {
    cache().ok().and_then(|c| c.credentials()).is_some()
}

#[tauri::command]
pub async fn spe_login() -> Result<(), String> {
    let client_id = SessionConfig::default().client_id;
    let client = OAuthClientBuilder::new(&client_id, OAUTH_REDIRECT, OAUTH_SCOPES.to_vec())
        .open_in_browser()
        .with_custom_message("Signed in to Spotify playback. You can close this tab and go back to SC Client.")
        .build()
        .map_err(|e| format!("Spotify sign-in: {e}"))?;
    let token = client
        .get_access_token_async()
        .await
        .map_err(|e| format!("Spotify sign-in: {e}"))?;
    let engine = connect(Credentials::with_access_token(token.access_token)).await?;
    *ENGINE.lock().await = Some(engine);
    Ok(())
}

#[tauri::command]
pub async fn spe_logout() -> Result<(), String> {
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

// The page hands over two channels: raw audio, and player events
#[tauri::command]
pub fn spe_attach(audio: Channel<InvokeResponseBody>, events: Channel<Value>) {
    *PCM.lock().unwrap() = Some(audio);
    *EVENTS.lock().unwrap() = Some(events);
}

// Returns the generation number the page should accept audio for
#[tauri::command]
pub async fn spe_play(uri: String, position_ms: u32) -> Result<u32, String> {
    let track = track_uri(&uri)?;
    let guard = engine().await?;
    let e = guard.as_ref().unwrap();
    PAUSED.store(false, Ordering::SeqCst);
    let gen = new_gen();
    e.player.load(track, true, position_ms);
    Ok(gen)
}

#[tauri::command]
pub async fn spe_pause() -> Result<(), String> {
    PAUSED.store(true, Ordering::SeqCst);
    if let Some(e) = ENGINE.lock().await.as_ref() {
        e.player.pause();
    }
    Ok(())
}

#[tauri::command]
pub async fn spe_resume() -> Result<(), String> {
    PAUSED.store(false, Ordering::SeqCst);
    if let Some(e) = ENGINE.lock().await.as_ref() {
        e.player.play();
    }
    Ok(())
}

#[tauri::command]
pub async fn spe_seek(position_ms: u32) -> Result<u32, String> {
    let guard = ENGINE.lock().await;
    let e = guard.as_ref().ok_or("Spotify playback isn't running")?;
    let gen = new_gen();
    e.player.seek(position_ms);
    Ok(gen)
}

#[tauri::command]
pub async fn spe_stop() -> Result<(), String> {
    new_gen();
    if let Some(e) = ENGINE.lock().await.as_ref() {
        e.player.stop();
    }
    Ok(())
}

// The page's worklet reports how many frames of this generation it has played
#[tauri::command]
pub fn spe_played(gen: u32, frames: u64) {
    if gen == GEN.load(Ordering::SeqCst) {
        PLAYED.store(frames, Ordering::SeqCst);
    }
}
