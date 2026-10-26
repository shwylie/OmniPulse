# OmniPulse

<!-- (edit me) a line or two in your own words: what OmniPulse is and who it's for -->
A music player and streaming toolkit for Windows. It plays SoundCloud, Spotify, YouTube Music and your own
files in one library, mixes from one song into the next like a DJ, and comes with a chatbot, alerts,
overlays and Stream Deck controls for streaming on Twitch, Kick and YouTube.

- **Download:** <!-- (edit me) the installer's link -->
- **Support:** [Ko-fi](https://ko-fi.com/shwylie) · bugs and ideas from Settings > Bugs and Ideas, or
  @shwylie on Discord

## A note on how this was made (AI disclaimer)

<!-- (edit me) this is a starting point: say it the way you want it said -->
OmniPulse is developed by one person, @shwylie, with a great deal of help from AI. Most of the code was
written by Anthropic's Claude (through Claude Code), working from my ideas, designs, feedback and testing:
I decide what gets built and how it should look and feel, try every change, and report what's wrong, and
Claude writes and revises the code. Tuning like Seamless Mix's transitions comes from my own ratings and
listening.

That means:

- The code hasn't had a line-by-line review by an experienced human developer. It's tested (automated
  tests, and daily use), but treat it as you would any hobby project.
- Bugs and odd choices are possible. Reports are welcome.
- No AI-generated music, art or voices are shipped in the app. <!-- (edit me) check this is true for you -->

## What's in it

- **Music:** one library across SoundCloud, Spotify, YouTube Music and local files; Discover, Rediscover,
  stats, lyrics, playlists across platforms, Apple Music import.
- **Seamless Mix:** blends songs on the beat (tempo and key matched), measured with Beat This! where it
  can be.
- **Streaming Suite:** a chatbot with actions (Streamer.bot and Mix It Up imports), alerts (Streamlabs
  import), song requests, widgets and docks for OBS, overlays, panels, Stream Deck plugin.
- **Visualisers**, themes, and a taskbar widget.

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

## Credits and licences

<!-- (edit me) choose a licence for OmniPulse itself and add a LICENSE file -->
- [Slint](https://slint.dev) draws the window (Royalty-free License, with its credit in Settings).
- [librespot](https://github.com/librespot-org/librespot) (MIT) plays Spotify, from a copy in `vendor/`.
- [Beat This!](https://github.com/CPJKU/beat_this) (MIT) finds beats and downbeats; its model is in
  `crates/core/models/` with its licence.
- [yt-dlp](https://github.com/yt-dlp/yt-dlp) is downloaded on first use, not bundled.
- OmniPulse isn't affiliated with SoundCloud, Spotify, YouTube, Twitch, Kick, Streamlabs, StreamElements,
  Ko-fi or Elgato.
