#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// The Tauri app: the window, tray, media keys and taskbar buttons, plus #[tauri::command]
// wrappers that hand each call from the page to omnipulse-core (crates/core).

use omnipulse_core::{
    discord, lastfm, local, loudness, lyrics, plugins, remote, requests, soundcloud as sc, spotify, spotify_engine as spe, widget,
    tools, youtube, yt_stream, Settings,
};
use serde_json::Value;
use std::sync::{Arc, OnceLock};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{Code, Shortcut, ShortcutState};

static APP: OnceLock<AppHandle> = OnceLock::new();

// Runs a script in the page, off the current thread (calls made while Windows is delivering a
// message, or from a server thread, are otherwise dropped)
fn page_eval(js: String) {
    if let Some(w) = APP.get().and_then(|app| app.get_webview_window("main")) {
        std::thread::spawn(move || {
            let _ = w.eval(&js);
        });
    }
}

// ---------- the page's saved data, copied out for the native app ----------
#[tauri::command]
fn web_export(data: String) -> Result<(), String> { omnipulse_core::save_web_export(&data) }

// ---------- omnipulse:// links (Listen Along and Jam invites) ----------
// Links that arrive (opening the app, or reaching it while it runs) wait here until the page takes them
static LINKS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn link_arrived(url: String) {
    if !url.starts_with("omnipulse://") || url.len() > 2000 {
        return;
    }
    LINKS.lock().unwrap().push(url);
    page_eval("window.__deepLink && window.__deepLink()".into());
}

#[tauri::command]
fn deep_links_take() -> Vec<String> {
    std::mem::take(&mut *LINKS.lock().unwrap())
}

// ---------- SoundCloud ----------
#[tauri::command]
async fn login() -> Result<(), String> { sc::login().await }
#[tauri::command]
fn signed_in() -> bool { sc::signed_in() }
#[tauri::command]
fn logout() -> Result<(), String> { sc::logout() }
#[tauri::command]
async fn api_get(path: String) -> Result<Value, String> { sc::api_get(path).await }
#[tauri::command]
async fn stream_url(id: String) -> Result<Value, String> { sc::stream_url(id).await }
#[tauri::command]
async fn stream_resolve(url: String) -> Result<Value, String> { sc::stream_resolve(url).await }
#[tauri::command]
async fn set_like(id: String, like: bool) -> Result<(), String> { sc::set_like(id, like).await }
#[tauri::command]
async fn set_follow(id: String, follow: bool) -> Result<(), String> { sc::set_follow(id, follow).await }
#[tauri::command]
async fn playlist_write(method: String, path: String, body: Option<Value>) -> Result<Value, String> {
    sc::playlist_write(method, path, body).await
}
#[tauri::command]
fn open_url(url: String) -> Result<(), String> { omnipulse_core::open_url(url) }

// ---------- Spotify Web API ----------
#[tauri::command]
async fn spotify_login() -> Result<(), String> { spotify::spotify_login().await }
#[tauri::command]
fn spotify_signed_in() -> bool { spotify::spotify_signed_in() }
#[tauri::command]
fn spotify_logout() -> Result<(), String> { spotify::spotify_logout() }
#[tauri::command]
async fn spotify_token() -> Result<String, String> { spotify::spotify_token().await }
#[tauri::command]
async fn spotify_api(method: String, path: String, body: Option<Value>) -> Result<Value, String> {
    spotify::spotify_api(method, path, body).await
}

// ---------- Spotify playback (librespot) ----------
// The page's AudioWorklet gets the audio as [gen: u32][reserved: u32][f32 samples...], little endian
struct ChannelPcm(Channel<InvokeResponseBody>);
impl spe::PcmOut for ChannelPcm {
    fn send(&self, gen: u32, samples: &[f32]) -> bool {
        let mut bytes = Vec::with_capacity(8 + samples.len() * 4);
        bytes.extend_from_slice(&gen.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        for s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        self.0.send(InvokeResponseBody::Raw(bytes)).is_ok()
    }
}

#[tauri::command]
fn spe_signed_in() -> bool { spe::spe_signed_in() }
#[tauri::command]
async fn spe_login() -> Result<(), String> { spe::spe_login().await }
#[tauri::command]
async fn spe_logout() -> Result<(), String> { spe::spe_logout().await }
#[tauri::command]
fn spe_attach(audio: Channel<InvokeResponseBody>, events: Channel<Value>) {
    spe::spe_attach(Arc::new(ChannelPcm(audio)), Arc::new(move |v| {
        let _ = events.send(v);
    }));
}
#[tauri::command]
async fn spe_play(uri: String, position_ms: u32) -> Result<u32, String> { spe::spe_play(uri, position_ms).await }
#[tauri::command]
async fn spe_pause() -> Result<(), String> { spe::spe_pause().await }
#[tauri::command]
async fn spe_resume() -> Result<(), String> { spe::spe_resume().await }
#[tauri::command]
async fn spe_seek(position_ms: u32) -> Result<u32, String> { spe::spe_seek(position_ms).await }
#[tauri::command]
async fn spe_stop() -> Result<(), String> { spe::spe_stop().await }
#[tauri::command]
fn spe_played(gen: u32, frames: u64) { spe::spe_played(gen, frames) }
#[tauri::command]
async fn spe_playlist(uri: String) -> Result<Value, String> { spe::spe_playlist(uri).await }
// your likes through Spotify's own library service (not the Web API's shared allowance)
#[tauri::command]
async fn spe_liked(known: Vec<String>) -> Result<Value, String> { omnipulse_core::spotify_library::spe_liked(known).await }
#[tauri::command]
async fn spe_playlists() -> Result<Value, String> { omnipulse_core::spotify_library::spe_playlists().await }
#[tauri::command]
async fn spe_set_liked(uri: String, on: bool) -> Result<(), String> { omnipulse_core::spotify_library::spe_set_liked(uri, on).await }
#[tauri::command]
async fn spe_web_get(path: String) -> Result<Value, String> { spe::spe_web_get(path).await }
#[tauri::command]
async fn spe_account() -> Result<Value, String> { spe::spe_account().await }
#[tauri::command]
fn spe_crashed_last_time() -> bool { spe::spe_crashed_last_time() }
#[tauri::command]
fn spotify_app_signed_in() -> bool { omnipulse_core::spotify::spotify_app_signed_in() }
/// A step the page takes, in diag.log
#[tauri::command]
fn diag_note(msg: String) { omnipulse_core::diag::note(&format!("page: {}", msg.chars().take(300).collect::<String>())); }
#[tauri::command]
fn spe_set_normalisation(on: bool, target_lufs: f64) { spe::spe_set_normalisation(on, target_lufs) }

// ---------- whole-track loudness and silences ----------
#[tauri::command]
async fn loudness_analyse(source: String, id: String) -> Result<Value, String> { loudness::analyse(&source, &id).await }

// ---------- lyrics, Discord, Last.fm, yt-dlp ----------
#[tauri::command]
async fn lyrics_get(url: String) -> Result<Value, String> { lyrics::lyrics_get(url).await }
// Lyrics from one provider (lrclib, netease, kugou, youtube): { synced, plain, source, note } or null
#[tauri::command]
async fn lyrics_from(provider: String, artist: String, title: String, secs: u64, video_id: String) -> Result<Value, String> {
    lyrics::from(&provider, &artist, &title, secs, &video_id).await
}
#[tauri::command]
async fn presence_set(
    details: String,
    state: String,
    start: Option<i64>,
    end: Option<i64>,
    image: Option<String>,
    buttons: Option<Vec<discord::PresenceButton>>,
) -> Result<String, String> {
    discord::presence_set(details, state, start, end, image, buttons).await
}
#[tauri::command]
async fn presence_clear() -> Result<(), String> { discord::presence_clear().await }
#[tauri::command]
async fn presence_test() -> Result<String, String> { discord::presence_test().await }
#[tauri::command]
fn lastfm_status() -> Value { lastfm::lastfm_status() }
// ---------- your own keys (Settings > Integrations > Set up, js/apisetup.js) ----------
/// Which of your keys are set (never the keys themselves)
#[tauri::command]
fn config_status() -> Value { omnipulse_core::config_status() }
/// Saves keys you pasted in, into config.json (only the ones the app lets you set)
#[tauri::command]
fn config_set(fields: serde_json::Map<String, Value>) -> Result<(), String> { omnipulse_core::config_set(&fields) }
/// Settings > App > Back Up: settings and what you've added, into a .zip (crates/core/src/backup.rs)
#[tauri::command]
fn backup_create(dest: String, web: String) -> Result<usize, String> { omnipulse_core::backup::create(&dest, &web) }
/// Puts a backup back; returns the page's saved data from it
#[tauri::command]
fn backup_restore(src: String) -> Result<String, String> { omnipulse_core::backup::restore(&src) }
/// Opens a platform's developer pages in your browser (only those)
#[tauri::command]
fn open_setup_page(url: String) -> Result<(), String> { omnipulse_core::open_setup_page(&url) }
#[tauri::command]
fn lastfm_disconnect() -> Result<(), String> { lastfm::lastfm_disconnect() }
#[tauri::command]
async fn lastfm_connect() -> Result<String, String> { lastfm::lastfm_connect().await }
#[tauri::command]
async fn lastfm_call(method: String, params: Vec<(String, String)>) -> Result<Value, String> {
    lastfm::lastfm_call(method, params).await
}
/// Seconds before Spotify takes requests again (0: now)
#[tauri::command]
fn spotify_wait() -> u64 { omnipulse_core::spotify::spotify_wait() }
#[tauri::command]
async fn lastfm_recent(from: i64, pages: u32) -> Result<Value, String> { lastfm::lastfm_recent(from, pages).await }
#[tauri::command]
async fn resolve_yt_stream(query: String) -> Result<String, String> { yt_stream::resolve_yt_stream(query).await }

// ---------- YouTube Music ----------
#[tauri::command]
async fn yt_search(query: String) -> Result<Vec<youtube::YtTrack>, String> { youtube::search(&query).await }
#[tauri::command]
async fn yt_radio(id: String) -> Result<Vec<youtube::YtTrack>, String> { youtube::radio(&id).await }
#[tauri::command]
async fn yt_status() -> Value { yt_stream::status().await }
#[tauri::command]
async fn yt_artists(query: String) -> Result<Vec<youtube::YtArtist>, String> { youtube::search_artists(&query).await }
#[tauri::command]
async fn yt_artist(id: String) -> Result<youtube::YtArtistPage, String> { youtube::artist(&id).await }
#[tauri::command]
async fn yt_album(id: String) -> Result<Value, String> {
    let (album, tracks) = youtube::album(&id).await?;
    Ok(serde_json::json!({ "album": album, "tracks": tracks }))
}
#[tauri::command]
async fn yt_playlist(id: String) -> Result<Vec<youtube::YtTrack>, String> { youtube::playlist(&id).await }
#[tauri::command]
async fn yt_playlists(query: String) -> Result<Vec<youtube::YtPlaylist>, String> { youtube::search_playlists(&query).await }
// Finds the audio ahead of playing (the player then fetches it through the local server's /yt)
#[tauri::command]
async fn yt_prepare(id: String) -> Result<(), String> { youtube::stream_url(&id, false).await.map(|_| ()) }
// a SoundCloud track's own audio, through yt-dlp, for whoever isn't signed in to SoundCloud
#[tauri::command]
async fn sc_ytdlp(url: String) -> Result<String, String> { yt_stream::resolve_soundcloud(&url).await }
// ---------- updates: from the public releases repo (tauri.conf.json > plugins > updater) ----------
// Each release is signed with OmniPulse's own key and checked against the public key built in here,
// so only a build signed with it installs, wherever it was downloaded from.
#[tauri::command]
async fn update_check(app: AppHandle) -> Result<Option<Value>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let found = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    Ok(found.map(|u| serde_json::json!({ "version": u.version, "current": u.current_version, "notes": u.body.unwrap_or_default() })))
}

/// Downloads the update (progress as { done, total } on the channel), installs it and restarts
#[tauri::command]
async fn update_install(app: AppHandle, progress: Channel<Value>) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let Some(update) = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())? else {
        return Err("You're up to date.".into());
    };
    let mut done: u64 = 0;
    update
        .download_and_install(
            |chunk, total| {
                done += chunk as u64;
                let _ = progress.send(serde_json::json!({ "done": done, "total": total }));
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}

// ---------- the native app (0.3.x), offered: from its own releases repo ----------
// The same key signs it, so the same updater installs it: its installer takes this one's place (the
// same folder, links and data) and starts it. Only when asked: the web app's own updates stay its own.
const NATIVE_LATEST: &str = "https://github.com/shwylie/omnipulse-native-releases/releases/latest/download/latest.json";

fn native_updater(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    use tauri_plugin_updater::UpdaterExt;
    let url: tauri::Url = NATIVE_LATEST.parse().map_err(|_| "bad address".to_string())?;
    app.updater_builder().endpoints(vec![url]).map_err(|e| e.to_string())?.build().map_err(|e| e.to_string())
}

/// The native app's latest version, if there is one (newer than this one, as every native one is)
#[tauri::command]
async fn native_check(app: AppHandle) -> Result<Option<Value>, String> {
    let found = native_updater(&app)?.check().await.map_err(|e| e.to_string())?;
    Ok(found.map(|u| serde_json::json!({ "version": u.version, "notes": u.body.unwrap_or_default() })))
}

/// Moves to the native app: downloads it (progress as { done, total }), and its installer takes over
#[tauri::command]
async fn native_install(app: AppHandle, progress: Channel<Value>) -> Result<(), String> {
    let Some(update) = native_updater(&app)?.check().await.map_err(|e| e.to_string())? else {
        return Err("The native app isn't out yet.".into());
    };
    let mut done: u64 = 0;
    update
        .download_and_install(
            |chunk, total| {
                done += chunk as u64;
                let _ = progress.send(serde_json::json!({ "done": done, "total": total }));
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    // (its installer has closed this one and starts the native app by itself)
    Ok(())
}

// ---------- Apple Music, once (crates/core/src/apple.rs): a read-only import ----------
/// You sign in to music.apple.com in a private window of its own (nothing kept after it closes; the
/// app's own data isn't touched); once its sign-in cookie is there, it's taken (into memory only) and
/// the window closes. Five minutes to sign in.
#[tauri::command]
async fn apple_connect(app: AppHandle) -> Result<(), String> {
    use omnipulse_core::apple;
    apple::dev_token().await?;   // (Apple's page as expected, before you're asked to sign in)
    if let Some(w) = app.get_webview_window("apple") { let _ = w.close(); }
    let url: tauri::Url = "https://music.apple.com/us/home".parse().map_err(|_| "bad address".to_string())?;
    let w = WebviewWindowBuilder::new(&app, "apple", WebviewUrl::External(url))
        .title("Sign in to Apple Music (once, to import your music)")
        .inner_size(1000.0, 760.0)
        .incognito(true)
        .build()
        .map_err(|e| e.to_string())?;
    let site: tauri::Url = "https://music.apple.com".parse().map_err(|_| "bad address".to_string())?;
    tokio::task::spawn_blocking(move || {
        for _ in 0..300 {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let Ok(cookies) = w.cookies_for_url(site.clone()) else { return Err("You closed the sign-in window.".to_string()) };
            if let Some(c) = cookies.iter().find(|c| c.name() == "media-user-token" && !c.value().is_empty()) {
                apple::set_user_token(c.value().to_string());
                let _ = w.close();
                return Ok(());
            }
        }
        let _ = w.close();
        Err("Signing in took too long.".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Your loved songs and playlists, read from Apple Music (nothing there is changed)
#[tauri::command]
async fn apple_library() -> Result<Value, String> { omnipulse_core::apple::library().await }

/// The import's done: your Apple Music sign-in is forgotten
#[tauri::command]
fn apple_forget(app: AppHandle) {
    omnipulse_core::apple::forget();
    if let Some(w) = app.get_webview_window("apple") { let _ = w.close(); }
}

// MusicBrainz: songs to find with no account (played from YouTube Music)
#[tauri::command]
async fn mb_search(query: String) -> Result<Vec<Value>, String> { omnipulse_core::musicbrainz::search(&query).await }
// The catalogue (Deezer's and iTunes's open music data): songs to find with no account, played from YouTube Music
#[tauri::command]
async fn cat_search(query: String) -> Result<Vec<Value>, String> { omnipulse_core::catalog::search(&query).await }
// One Deezer catalogue request (artists, albums, charts, related artists), as Deezer answers it
#[tauri::command]
async fn deezer_get(path: String) -> Result<Value, String> { omnipulse_core::catalog::deezer(&path).await }
// An artist's bio from Wikipedia ({ title, text, url, image }, or null)
// Playlist files (M3U, CSV) the app imports and exports
#[tauri::command]
fn playlist_file_read(path: String) -> Result<String, String> { omnipulse_core::playlist_file_read(&path) }
#[tauri::command]
fn playlist_file_write(path: String, text: String) -> Result<(), String> { omnipulse_core::playlist_file_write(&path, &text) }
#[tauri::command]
async fn artist_bio(name: String) -> Result<Value, String> { omnipulse_core::bio::artist(&name).await }
// SponsorBlock's non-music parts of a YouTube video ([[start, end]] in seconds)
#[tauri::command]
async fn sb_segments(video_id: String) -> Result<Vec<(f64, f64)>, String> { omnipulse_core::sponsorblock::music_offtopic(&video_id).await }
// Downloads yt-dlp and Deno in the background; yt_status says how it's going
// async, so it runs on Tauri's async runtime rather than the window's thread
#[tauri::command]
async fn yt_setup() { tools::start_install() }
#[tauri::command]
fn yt_remove() -> Result<u64, String> { tools::uninstall() }

// ---------- OBS widget, local music, chat requests, Stream Deck remote ----------
#[tauri::command]
fn widget_start(port: u16, remote: Option<bool>, key: Option<String>) -> Result<String, String> {
    widget::widget_start(port, remote.unwrap_or(false), &key.unwrap_or_default())
}
/// This PC's address on your network, for bots on other devices
#[tauri::command]
fn lan_ip() -> Option<String> { omnipulse_core::lan_ip() }
#[tauri::command]
fn widget_stop() { widget::widget_stop() }
#[tauri::command]
fn widget_update(state: Value) { widget::widget_update(state) }
// ---------- the YouTube account (optional): likes here are likes on YouTube ----------
#[tauri::command]
fn youtube_status() -> Value { omnipulse_core::yt_account::status() }
#[tauri::command]
async fn youtube_connect() -> Result<Value, String> { omnipulse_core::yt_account::connect().await }
#[tauri::command]
async fn youtube_disconnect() -> Result<(), String> { omnipulse_core::yt_account::disconnect().await }
/// Likes a track on YouTube: by its video id, or found by its title and artist. Returns the video id.
#[tauri::command]
async fn like_youtube_track(video_id: Option<String>, title: String, artist: String) -> Result<String, String> {
    omnipulse_core::yt_account::rate(video_id, &title, &artist, true).await
}
/// Takes a like back on YouTube (its rating goes back to none). Returns the video id.
#[tauri::command]
async fn unlike_youtube_track(video_id: Option<String>, title: String, artist: String) -> Result<String, String> {
    omnipulse_core::yt_account::rate(video_id, &title, &artist, false).await
}
/// Each YouTube video's licence ("creativeCommon" or "youtube"), for Stream-Safe Music
#[tauri::command]
async fn youtube_licences(ids: Vec<String>) -> Result<Value, String> {
    omnipulse_core::yt_account::licences(ids).await
}
/// An alert from the app's Test buttons
#[tauri::command]
fn widget_alert(alert: Value) -> Result<Value, String> {
    widget::alert_push(alert.as_object().ok_or("an alert is { type, user, amount, message }")?)
}
/// An alert's sound or picture ("sound" / "image"), copied into the data folder
#[tauri::command]
fn alert_media_set(kind: String, slot: String, path: String) -> Result<String, String> { widget::alert_media_set(&kind, &slot, &path) }
#[tauri::command]
fn alert_media_clear(kind: String, slot: String) { widget::alert_media_clear(&kind, &slot) }
/// An alert's file as it's kept (a sound, to measure its loudness and preview it)
#[tauri::command]
fn alert_media_bytes(kind: String, slot: String) -> Result<tauri::ipc::Response, String> {
    widget::alert_media_bytes(&kind, &slot).map(tauri::ipc::Response::new)
}
#[tauri::command]
fn local_folders() -> Vec<String> { local::local_folders() }
#[tauri::command]
fn local_set_folders(folders: Vec<String>) -> Result<Vec<String>, String> { local::local_set_folders(folders) }
#[tauri::command]
fn local_scan() { local::local_scan() }
#[tauri::command]
fn local_status() -> Value { local::local_status() }
#[tauri::command]
fn local_tracks() -> Value { local::local_tracks() }
#[tauri::command]
fn local_server() -> Value { local::local_server() }
#[tauri::command]
fn requests_start(port: u16, key: String, remote: Option<bool>) -> Result<String, String> { requests::requests_start(port, key, remote.unwrap_or(false)) }
#[tauri::command]
fn requests_stop() { requests::requests_stop() }
#[tauri::command]
fn request_result(id: u64, result: Value) { requests::request_result(id, result) }
#[tauri::command]
fn remote_start(port: u16, key: String) -> Result<String, String> { remote::remote_start(port, key) }
#[tauri::command]
fn remote_stop() { remote::remote_stop() }
#[tauri::command]
fn remote_result(id: u64, state: Value) { remote::remote_result(id, state) }

// ---------- plugins ----------
#[tauri::command]
async fn plugin_inspect(source: String) -> Result<plugins::Package, String> { plugins::inspect(&source).await }
#[tauri::command]
async fn plugin_install(source: String, digest: String) -> Result<plugins::Manifest, String> { plugins::install(&source, &digest).await }
#[tauri::command]
fn plugin_list() -> Vec<plugins::Package> { plugins::list() }
/// Opens an installed plugin's folder in Explorer, to read its files
#[tauri::command]
fn plugin_folder(id: String) -> Result<(), String> {
    let dir = plugins::folder_of(&id)?;
    std::process::Command::new("explorer").arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string())
}
#[tauri::command]
fn plugin_remove(id: String) -> Result<(), String> { plugins::remove(&id) }
#[tauri::command]
async fn plugin_http(id: String, url: String, method: String, headers: Vec<(String, String)>, body: Option<String>) -> Result<Value, String> {
    plugins::http(&id, &url, &method, headers, body).await
}

// ---------- OmniPulse's own sound, from this process ----------
#[tauri::command]
async fn main_out_start(device: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || omnipulse_core::main_out::start(&device)).await.map_err(|e| e.to_string())?
}
#[tauri::command]
fn main_out_pcm(request: tauri::ipc::Request) {
    if let tauri::ipc::InvokeBody::Raw(bytes) = request.body() {
        let rate = request.headers().get("x-rate").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(48000);
        omnipulse_core::main_out::push(bytes, rate);
    }
}
#[tauri::command]
fn main_out_flush() { omnipulse_core::main_out::flush() }
#[tauri::command]
fn main_out_volume(volume: f32) { omnipulse_core::main_out::set_volume(volume) }
#[tauri::command]
fn main_out_stop() { omnipulse_core::main_out::stop() }

// ---------- OmniPulse Offset ----------
#[tauri::command]
async fn omni_input_start(muted: bool) -> Result<String, String> {
    tokio::task::spawn_blocking(move || omnipulse_core::omni_input::start(muted)).await.map_err(|e| e.to_string())?
}
#[tauri::command]
fn omni_input_pcm(request: tauri::ipc::Request) {
    if let tauri::ipc::InvokeBody::Raw(bytes) = request.body() {
        let rate = request.headers().get("x-rate").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(48000);
        omnipulse_core::omni_input::push_pcm(bytes, rate);
    }
}
#[tauri::command]
fn omni_input_stop() { omnipulse_core::omni_input::stop() }

// ---------- casting ----------
#[tauri::command]
fn cast_devices() -> Result<Vec<omnipulse_core::cast::Device>, String> {
    omnipulse_core::cast::discover_start()?;
    Ok(omnipulse_core::cast::devices())
}
#[tauri::command]
async fn cast_start(id: String, meta: Value, events: Channel<Value>) -> Result<omnipulse_core::cast::Device, String> {
    tokio::task::spawn_blocking(move || {
        omnipulse_core::cast::start(&id, meta, std::sync::Arc::new(move |v| { let _ = events.send(v); }))
    })
    .await
    .map_err(|e| e.to_string())?
}
// the app's sound: 16-bit stereo PCM as the request's body, its rate in a header
#[tauri::command]
fn cast_pcm(request: tauri::ipc::Request) {
    if let tauri::ipc::InvokeBody::Raw(bytes) = request.body() {
        let rate = request.headers().get("x-rate").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(48000);
        omnipulse_core::cast::push_pcm(bytes, rate);
    }
}
#[tauri::command]
fn cast_track(meta: Value) { omnipulse_core::cast::track(meta) }
#[tauri::command]
fn cast_reload() { omnipulse_core::cast::reload() }
#[tauri::command]
fn cast_volume(level: f64) { omnipulse_core::cast::set_volume(level) }
#[tauri::command]
fn cast_stop() { omnipulse_core::cast::stop() }

// ---------- app settings ----------
#[tauri::command]
fn get_settings() -> Settings { omnipulse_core::load_settings() }
#[tauri::command]
fn set_gpu(enabled: bool) -> Result<(), String> { omnipulse_core::set_gpu(enabled) }

// Media keys, the tray menu and the taskbar buttons all go through this:
// it calls window.__mediaAction(...) in the page directly.
fn media_action(_app: &AppHandle, action: &str) {
    page_eval(format!("window.__mediaAction && window.__mediaAction('{action}')"));
}

// Keeps the taskbar buttons' icons in step with the player
#[tauri::command]
fn taskbar_state(app: AppHandle, playing: bool, shuffle: bool, liked: bool, can_like: bool) {
    #[cfg(windows)]
    {
        let bits = (playing as usize)
            | ((shuffle as usize) << 1)
            | ((liked as usize) << 2)
            | ((can_like as usize) << 3);
        if let Some(w) = app.get_webview_window("main") {
            taskbar_buttons::set_state(&w, bits);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (app, playing, shuffle, liked, can_like);
    }
}

// ---------- Taskbar thumbnail buttons (Windows) ----------
// Shown when you hover the app on the taskbar, like VLC.
#[cfg(windows)]
mod taskbar_buttons {
    use std::sync::atomic::{AtomicU32, Ordering};
    use tauri::{path::BaseDirectory, Manager, WebviewWindow, Wry};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Shell::{
        DefSubclassProc, ITaskbarList3, SetWindowSubclass, TaskbarList, THBF_DISABLED, THBF_ENABLED,
        THBN_CLICKED, THB_FLAGS, THB_ICON, THB_TOOLTIP, THUMBBUTTON,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        LoadImageW, PostMessageW, RegisterWindowMessageW, HICON, IMAGE_ICON, LR_DEFAULTSIZE,
        LR_LOADFROMFILE, WM_COMMAND, WM_USER,
    };

    const SUBCLASS_ID: usize = 7311;
    const BTN_SHUFFLE: u32 = 2001;
    const BTN_PREV: u32 = 2002;
    const BTN_PLAY: u32 = 2003;
    const BTN_NEXT: u32 = 2004;
    const BTN_LIKE: u32 = 2005;
    const WM_UPDATE_STATE: u32 = WM_USER + 21;

    static CREATED_MSG: AtomicU32 = AtomicU32::new(0);

    struct Icons {
        shuffle_off: HICON,
        shuffle_on: HICON,
        prev: HICON,
        play: HICON,
        pause: HICON,
        next: HICON,
        like_off: HICON,
        like_on: HICON,
    }

    struct Ctx {
        window: WebviewWindow<Wry>,
        taskbar: Option<ITaskbarList3>,
        icons: Icons,
        state: usize,
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn set_tip(buf: &mut [u16; 260], s: &str) {
        let mut count = 0;
        for (i, c) in s.encode_utf16().enumerate().take(259) {
            buf[i] = c;
            count = i + 1;
        }
        buf[count] = 0;
    }

    fn load_icon(window: &WebviewWindow<Wry>, name: &str) -> Result<HICON, String> {
        let path = window
            .app_handle()
            .path()
            .resolve(format!("icons/taskbar/{name}.ico"), BaseDirectory::Resource)
            .map_err(|e| e.to_string())?;
        let wpath = wide(&path.to_string_lossy());
        unsafe {
            let h = LoadImageW(
                None,
                PCWSTR(wpath.as_ptr()),
                IMAGE_ICON,
                0,
                0,
                LR_LOADFROMFILE | LR_DEFAULTSIZE,
            )
            .map_err(|e| e.to_string())?;
            Ok(HICON(h.0))
        }
    }

    fn button(id: u32, icon: HICON, tip: &str, enabled: bool) -> THUMBBUTTON {
        let mut b = THUMBBUTTON {
            dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
            iId: id,
            hIcon: icon,
            szTip: [0; 260],
            dwFlags: if enabled { THBF_ENABLED } else { THBF_DISABLED },
            ..Default::default()
        };
        set_tip(&mut b.szTip, tip);
        b
    }

    fn buttons_for(icons: &Icons, bits: usize) -> [THUMBBUTTON; 5] {
        let playing = (bits & 1) != 0;
        let shuffle = (bits & 2) != 0;
        let liked = (bits & 4) != 0;
        let can_like = (bits & 8) != 0;
        [
            button(
                BTN_SHUFFLE,
                if shuffle { icons.shuffle_on } else { icons.shuffle_off },
                if shuffle { "Shuffle: on" } else { "Shuffle: off" },
                true,
            ),
            button(BTN_PREV, icons.prev, "Previous", true),
            button(
                BTN_PLAY,
                if playing { icons.pause } else { icons.play },
                if playing { "Pause" } else { "Play" },
                true,
            ),
            button(BTN_NEXT, icons.next, "Next", true),
            button(
                BTN_LIKE,
                if liked { icons.like_on } else { icons.like_off },
                if liked { "Remove from Likes" } else { "Like" },
                can_like,
            ),
        ]
    }

    unsafe extern "system" fn subclass_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        data: usize,
    ) -> LRESULT {
        let ctx = &mut *(data as *mut Ctx);

        // The taskbar button exists (or Explorer restarted): add the buttons
        let created = CREATED_MSG.load(Ordering::Relaxed);
        if created != 0 && msg == created {
            let tb: windows::core::Result<ITaskbarList3> =
                CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER);
            if let Ok(tb) = tb {
                if tb.HrInit().is_ok() {
                    // Fails harmlessly if the buttons were already added
                    let _ = tb.ThumbBarAddButtons(hwnd, &buttons_for(&ctx.icons, ctx.state));
                    ctx.taskbar = Some(tb);
                }
            }
            return DefSubclassProc(hwnd, msg, wparam, lparam);
        }

        if msg == WM_UPDATE_STATE {
            ctx.state = wparam.0;
            if let Some(ref tb) = ctx.taskbar {
                let _ = tb.ThumbBarUpdateButtons(hwnd, &buttons_for(&ctx.icons, ctx.state));
            }
            return LRESULT(0);
        }

        if msg == WM_COMMAND {
            let high = (wparam.0 >> 16) & 0xFFFF;
            let low = (wparam.0 & 0xFFFF) as u32;
            if high as u32 == THBN_CLICKED {
                let action = match low {
                    BTN_SHUFFLE => "shuffle",
                    BTN_PREV => "prev",
                    BTN_PLAY => "play",
                    BTN_NEXT => "next",
                    BTN_LIKE => "like",
                    _ => "",
                };
                if !action.is_empty() {
                    // Hand the click to the page a moment later, outside this window message.
                    // Calling into the page while Windows is still delivering the click gets dropped.
                    let w = ctx.window.clone();
                    let js = format!("window.__mediaAction && window.__mediaAction('{action}')");
                    std::thread::spawn(move || {
                        let _ = w.eval(&js);
                    });
                }
                return LRESULT(0);
            }
        }

        DefSubclassProc(hwnd, msg, wparam, lparam)
    }

    pub fn attach(window: &WebviewWindow<Wry>) -> Result<(), String> {
        let hwnd = HWND(window.hwnd().map_err(|e| e.to_string())?.0 as _);
        if CREATED_MSG.load(Ordering::Relaxed) == 0 {
            let name = wide("TaskbarButtonCreated");
            let id = unsafe { RegisterWindowMessageW(PCWSTR(name.as_ptr())) };
            CREATED_MSG.store(id, Ordering::Relaxed);
        }
        let icons = Icons {
            shuffle_off: load_icon(window, "SHUFFLE_OFF")?,
            shuffle_on: load_icon(window, "SHUFFLE_ON")?,
            prev: load_icon(window, "PREV_THUMB")?,
            play: load_icon(window, "PLAY_THUMB")?,
            pause: load_icon(window, "PAUSE_THUMB")?,
            next: load_icon(window, "NEXT_THUMB")?,
            like_off: load_icon(window, "LIKE_OFF")?,
            like_on: load_icon(window, "LIKE_ON")?,
        };
        let ctx = Box::new(Ctx {
            window: window.clone(),
            taskbar: None,
            icons,
            state: 0,
        });
        let ptr = Box::into_raw(ctx);
        unsafe {
            let _ = SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, ptr as usize);
            let id = CREATED_MSG.load(Ordering::Relaxed);
            if id != 0 {
                let _ = PostMessageW(Some(hwnd), id, WPARAM(0), LPARAM(0));
            }
        }
        Ok(())
    }

    pub fn set_state(window: &WebviewWindow<Wry>, bits: usize) {
        if let Ok(h) = window.hwnd() {
            let hwnd = HWND(h.0 as _);
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_UPDATE_STATE, WPARAM(bits), LPARAM(0));
            }
        }
    }
}

#[tauri::command]
fn restart_app(app: AppHandle) {
    app.restart();
}

// Passing our own browser arguments replaces Tauri's defaults, so the default
// feature list is repeated here.
#[allow(dead_code)]
fn browser_args(gpu: bool) -> String {
    let mut args = String::from(
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required --use-fake-ui-for-media-stream",
    );
    if !gpu {
        args.push_str(" --disable-gpu");
    }
    args
}

// ---------- app ----------

fn main() {
    omnipulse_core::crash_log_install();
    omnipulse_core::diag::install();
    let play = Shortcut::new(None, Code::MediaPlayPause);
    let next = Shortcut::new(None, Code::MediaTrackNext);
    let prev = Shortcut::new(None, Code::MediaTrackPrevious);

    tauri::Builder::default()
        // one OmniPulse at a time: opening it again (or a link to it) brings this one forward
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts([play, next, prev])
                .expect("could not register media keys")
                .with_handler(move |app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    let action = if shortcut == &play {
                        "play"
                    } else if shortcut == &next {
                        "next"
                    } else if shortcut == &prev {
                        "prev"
                    } else {
                        return;
                    };
                    media_action(app, action);
                })
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // the plugins that come with OmniPulse (beside the app, as plain folders): installed or
            // brought up to date before the page lists them
            if let Ok(dir) = app.path().resolve("plugins", tauri::path::BaseDirectory::Resource) {
                plugins::install_bundled(&dir);
            }
            let gpu = omnipulse_core::load_settings().gpu;
            #[allow(unused_mut)]
            let mut builder =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("OmniPulse")
                    .inner_size(1180.0, 760.0)
                    .min_inner_size(820.0, 520.0)
                    .decorations(false)
                    .disable_drag_drop_handler();
            #[cfg(windows)]
            {
                builder = builder.additional_browser_args(&browser_args(gpu));
            }
            #[cfg(not(windows))]
            {
                let _ = gpu;
            }
            let main_window = builder.build()?;
            let _ = APP.set(app.handle().clone());
            // omnipulse:// links: registered for this Windows account, and taken as they arrive
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                #[cfg(any(windows, target_os = "linux"))]
                let _ = app.deep_link().register_all();
                if let Ok(Some(urls)) = app.deep_link().get_current() {
                    urls.into_iter().for_each(|u| link_arrived(u.to_string()));
                }
                app.deep_link().on_open_url(|event| {
                    event.urls().into_iter().for_each(|u| link_arrived(u.to_string()));
                });
            }
            // the built-in yt-dlp keeps up with YouTube: it updates itself once a day
            tauri::async_runtime::spawn(async {
                tools::update_ytdlp_daily().await;
            });
            // Chat requests and Stream Deck presses are answered by the page
            requests::set_asker(Arc::new(|id, ask| {
                page_eval(format!(
                    "window.__songRequest && window.__songRequest({id}, {}, {}, {})",
                    serde_json::to_string(&ask["q"]).unwrap_or_else(|_| "\"\"".to_string()),
                    serde_json::to_string(&ask["user"]).unwrap_or_else(|_| "\"\"".to_string()),
                    ask
                ));
            }));
            // the action is one of remote's ACTIONS, so it's safe to put in the script
            // the widget is showing an alert: a musical one's sound is played here (js/alertsound.js)
            widget::set_shown_handler(Arc::new(|kind| {
                page_eval(format!("window.__alertShown && window.__alertShown({})", serde_json::json!(kind)));
            }));
            remote::set_handler(Arc::new(|id, action| {
                page_eval(format!("window.__remote && window.__remote({id}, {})", serde_json::json!(action)));
            }));
            // Shuffle / previous / play-pause / next / like buttons on the taskbar thumbnail
            #[cfg(windows)]
            {
                let _ = taskbar_buttons::attach(&main_window);
            }
            #[cfg(not(windows))]
            {
                let _ = &main_window;
            }

            // Tray icon: show the window, control playback, or quit
            let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let play = MenuItem::with_id(app, "play", "Play / Pause", true, None::<&str>)?;
            let next = MenuItem::with_id(app, "next", "Next", true, None::<&str>)?;
            let prev = MenuItem::with_id(app, "prev", "Previous", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &play, &next, &prev, &quit])?;
            let mut tray = TrayIconBuilder::new().menu(&menu).tooltip("Music");
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.on_menu_event(|app, event| match event.id.as_ref() {
                "show" => {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.unminimize();
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
                "play" => {
                    media_action(app, "play");
                }
                "next" => {
                    media_action(app, "next");
                }
                "prev" => {
                    media_action(app, "prev");
                }
                "quit" => {
                    app.exit(0);
                }
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    let app = tray.app_handle();
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.unminimize();
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
            })
            .build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            deep_links_take, web_export,
            omni_input_start, omni_input_pcm, omni_input_stop,
            main_out_start, main_out_pcm, main_out_flush, main_out_volume, main_out_stop,
            cast_devices, cast_start, cast_pcm, cast_track, cast_reload, cast_volume, cast_stop,
            login, signed_in, logout, api_get, stream_url, get_settings, set_gpu, restart_app, open_url, set_like, set_follow, playlist_write,
            spotify_login, spotify_signed_in, spotify_logout, spotify_token, spotify_api,
            lyrics_get, lyrics_from, presence_set, presence_clear,
            lastfm_status, lastfm_connect, lastfm_disconnect, lastfm_call, lastfm_recent, spotify_wait,
            stream_resolve, presence_test, taskbar_state,
            widget_start, lan_ip, widget_stop, widget_update, widget_alert, alert_media_set, alert_media_clear, alert_media_bytes,
            config_status, config_set, open_setup_page, backup_create, backup_restore, youtube_status, youtube_connect, youtube_disconnect, youtube_licences, like_youtube_track, unlike_youtube_track,
            local_folders, local_set_folders, local_scan, local_status, local_tracks, local_server,
            requests_start, requests_stop, request_result,
            spe_signed_in, spe_login, spe_logout, spe_attach, spe_play, spe_pause, spe_resume, spe_seek, spe_stop, spe_played, spe_playlist, spe_liked, spe_set_liked, spe_playlists, spe_web_get, spe_account, spe_crashed_last_time, spotify_app_signed_in, diag_note, spe_set_normalisation, loudness_analyse,
            resolve_yt_stream, plugin_inspect, plugin_install, plugin_list, plugin_folder, plugin_remove, plugin_http, yt_search, yt_radio, yt_status, yt_artists, yt_artist, yt_album, yt_playlist, yt_playlists, yt_prepare, sc_ytdlp, mb_search, cat_search, deezer_get, artist_bio, playlist_file_read, playlist_file_write, sb_segments, apple_connect, apple_library, apple_forget, update_check, update_install, native_check, native_install, yt_setup, yt_remove, remote_start, remote_stop, remote_result
        ])
        .build(tauri::generate_context!())
        .expect("error while building app")
        .run(|_app, event| {
            // (closed as an app does: so a log that stops without this means it didn't)
            if let tauri::RunEvent::Exit = event { omnipulse_core::diag::note("closed normally"); }
        });
}
