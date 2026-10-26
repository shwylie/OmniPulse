# OmniPulse

A music player and streaming toolkit for Windows. It plays SoundCloud, Spotify, YouTube Music and your own
files in one library (and imports your Apple Music library), and comes with a chatbot, alerts, overlays,
docks and Stream Deck controls for streaming on Twitch, Kick and YouTube.

## Download

- **[Web installer](https://omnipulse-jam.shwylie101.workers.dev/get)** (recommended): a small download
  that always fetches the latest version when you run it.
- **[Releases page](https://github.com/shwylie/omnipulse-native-releases/releases)** (backup): every
  version's full installer, if the web installer doesn't work for you or you want a particular version.

## Support and contact

- **Support the project:** [![Support OmniPulse at shwyl.ie](https://img.shields.io/badge/Support-shwyl.ie-8e9bff?style=for-the-badge)](https://shwyl.ie) [![Ko-fi](https://img.shields.io/badge/%E2%80%8B-FF5E5B?style=for-the-badge&logo=kofi&logoColor=white)](https://ko-fi.com/shwylie)
- **Bugs and ideas:** from the app (Settings > Bugs and Ideas), or message shwylie on Discord
- **Email:** [me@shwyl.ie](mailto:me@shwyl.ie?subject=OmniPulse) as a fallback. Please put "OmniPulse"
  in the subject line.

## A note on how this was made

OmniPulse is developed and maintained by one person. Me. SHWYLiE. With a great deal of help from Claude
Code. I couldn't have done this alone, as I'm still learning the fundamentals (thanks
[boot.dev](https://boot.dev) for being amazing). So while some of this code is my own, I've been using
Claude Code as my supervisor, double-checking and exploring ideas with it, so I can learn how
something is (or can be) made if I haven't learned it yet through boot.dev.

What this means:

- The code hasn't had a line-by-line review by an experienced human developer. It's tested (automated
  tests, and daily use), but treat it as you would any hobby project.
- Bugs and odd choices are possible. Reports are welcome.
- No AI-generated music, art or voices are shipped in the app.

## What's in it

- **Music:** one library across SoundCloud, Spotify, YouTube Music and local files; Discover, Rediscover,
  stats, lyrics, playlists across platforms, Apple Music import.
- **Streaming Suite:** a chatbot with actions (Streamer.bot and Mix It Up imports available), alerts
  (partial Streamlabs import available), song requests, widgets and docks for OBS, overlays, panels and a
  Stream Deck plugin.
- **Visualisers**, themes, and a taskbar widget.
- **Seamless Mix:** blends songs on the beat (tempo and key matched), measured with Beat This! where it
  can be. *Spotify's audio is never used for it.* With Seamless Mix on, music (apart from your own files)
  comes from third-party sources, and may not always be the exact version its details describe (another
  edit, remix or upload).

## Building it

Windows, with Rust (stable) and Node.js.

1. `cargo build --release -p omnipulse` builds the app (`target/release/OmniPulse-native.exe`).
2. Keys: no keys or secrets are kept in this repository.
   - SoundCloud's app keys go in `crates/core/keys.local.json` (git-ignored) before building:
     `{ "soundcloud_client_id": "...", "soundcloud_client_secret": "...", "soundcloud_redirect_uri": "http://localhost:8080/callback" }`.
     Without it, put `client_id` and `client_secret` in `%APPDATA%\OmniPulse\config.json`.
   - Spotify, Last.fm, YouTube and the like are signed in to from the app's Settings.
   - Twitch, Kick and SoundCloud sign-ins are finished by the relay (below), which holds their client
     secrets. The client IDs in the code are public by design.
3. `jam-server/` is the relay (a Cloudflare Worker): Jams, shared playlists, sign-ins, update hosting,
   bug reports, Ko-fi webhooks. Its secrets are set as Cloudflare secrets (`npx wrangler secret put ...`),
   never in `wrangler.toml`. To run your own, change `RELAY_URL` in `crates/core/src/bundled.rs`.
4. `scripts/release-native.mjs` makes a signed release. Signing keys are never in the repository.

`src-tauri/` and `dist/` are the old web-based app (0.2.x), kept for reference.

## Licence

OmniPulse is free software under the [GNU General Public License v3.0](LICENSE) (or any later version).
You can use, study, share and change it; if you share a changed version, it must stay under the GPL
with its source available.

## Credits

- [Slint](https://slint.dev) draws the window (used under the GPLv3, as OmniPulse is; its credit is in
  Settings).
- [librespot](https://github.com/librespot-org/librespot) (MIT) plays Spotify, from a copy in `vendor/`.
- [Beat This!](https://github.com/CPJKU/beat_this) (MIT) finds beats and downbeats; its model is in
  `crates/core/models/` with its licence.
- [yt-dlp](https://github.com/yt-dlp/yt-dlp) is downloaded on first use, not bundled.
- OmniPulse isn't affiliated with SoundCloud, Spotify, YouTube, Apple, Twitch, Kick, Streamlabs,
  StreamElements, Streamer.bot, Mix It Up, Ko-fi or Elgato.
