# OmniPulse

A small SoundCloud player built on the official API. Personal use only.

## Setup (Windows)
1. Create the file `%APPDATA%\sc-client\config.json` (paste that path into File Explorer) containing:

       { "client_id": "YOUR_CLIENT_ID", "client_secret": "YOUR_CLIENT_SECRET" }

   The redirect URI defaults to `http://localhost:8080/callback`. It must be added to your app at https://soundcloud.com/you/apps. To use another one, add `"redirect_uri": "..."` to the file.
2. `npm install`
3. Icons: copy the `src-tauri\icons` folder from your sc-lite project, or run `npx tauri icon C:\dev\extra\sc-logo.png`
4. `npm run dev` to run, `npm run build` for an installer.

## Shortcuts
Media keys: play/pause, next, previous
In the window: Space, Shift+Left/Right, / to search

## Files
- `src-tauri/src/main.rs`: sign-in (OAuth with PKCE), token storage, API calls, media keys
- `dist/`: the interface (index.html, style.css)
  - `app.js`: core player: views, track list, queue, playback, settings dialog. Loaded last; starts everything.
  - `spotify.js`: Spotify sign-in, player and Web API calls
  - `js/`: one file per feature (titles, appearance, playback, audio, stats, lyrics, integrations,
    discover, library, widget, nav, platforms, requests, albums). Each declares its functions and an
    `xxxInit()`; `js/init.js` calls them in order. They're plain scripts sharing globals, so the order
    of the `<script>` tags in index.html matters only for code that runs as a file loads.

## Settings (gear in the sidebar)
- Appearance: theme, accent color, list density (saved automatically)
- Audio: choose the output device, e.g. a second pair of headphones
- Performance: hardware acceleration on/off (needs a restart)

## Library and queue
- Sidebar: Likes, Search, and your playlists (yours and ones you liked)
- Hover a track for "Play next" and "Add to queue"; Play and Shuffle sit under each list title
- Queue panel: Q or the queue button. Click a track to jump to it, X removes it
- Now Playing: click the artwork in the player bar for the large view

## Likes
- Heart on each track (hover), in the player bar, and on the Now Playing view; press L to like or unlike the current track

## Playback
Full tracks are streamed over HLS using hls.js. `npm install` fetches it and copies it to `dist/hls.min.js`.

## Spotify (optional, Premium only)
1. Create an app at https://developer.spotify.com/dashboard and tick Web API and Web Playback SDK.
2. Add this redirect URI to the app: http://127.0.0.1:8081/callback
3. Add your Client ID to %APPDATA%\sc-client\config.json:
   { "client_id": "...", "client_secret": "...", "spotify_client_id": "YOUR_SPOTIFY_CLIENT_ID" }
4. Settings > Spotify > Connect. The Spotify tab appears only for a Premium account.
Spotify and SoundCloud tracks never share a queue or a list.

## Feature switches
Spotify is switched off for now. To bring it back, set `spotify: true` in `dist/features.js` and restart.
The Spotify code is still in the project (dist/spotify.js and the Spotify commands in src-tauri/src/main.rs).

## Extras (v13)
- Player bar: shuffle, repeat (off / all / one), sleep timer (15, 30, 60 minutes or end of track)
- Queue: drag tracks under "Next up" to reorder them
- Settings > Appearance: theme presets, accent from album art, custom CSS
- Stats tab: plays and top artists/tracks, stored only on this PC
- Now Playing: synced lyrics from LRCLIB when available
- Tray icon (left-click shows the window; right-click for controls). Optional: close button hides to tray.

### Discord activity
1. Create an application at https://discord.com/developers/applications and name it "Music".
   That name is what your profile shows ("Listening to Music").
2. Copy its Application ID into config.json as "discord_client_id".

### Last.fm
1. Get an API key at https://www.last.fm/api/account/create
2. Add "lastfm_api_key" and "lastfm_secret" to config.json, then Settings > Integrations > Connect.

## Taskbar buttons (v15)
Hover the app on the taskbar for Shuffle / Previous / Play-Pause / Next / Like, like VLC. The button icons are in src-tauri/icons/taskbar.

## Discord troubleshooting
Settings > Integrations > Discord activity > Test shows exactly what Discord answers.

## Loudness and language (v19)
- Settings > Audio > Even out loudness: Off, Gentle or Strong. Rides the gain toward a target level,
  then a compressor and limiter, all inside the app. Takes effect from the next track if just switched on.
- Discover has a language picker (default English). Titles and tags are checked for other alphabets,
  accented letters and common words; tracks it can't place are kept.

## v20
- Search tab: its own page with one big search box
- Mixes (sidebar): anything over 15 minutes. Mixes are kept out of search, Discover and artist pages,
  but still show in Likes and playlists.
- Playlists: "+" in the sidebar, or the playlist button on any track. New playlists are private
  and saved to your SoundCloud account. Your own playlists get remove buttons and "Delete playlist".
- Titles: promo tags like "FREE DL", "OUT NOW" or "Supported by ..." are removed.
- Lists load more as you scroll.

## Now playing widget (v22)
Settings > Integrations > Now playing widget. Copy the address (default http://127.0.0.1:7878/)
into OBS as a Browser Source, about 460 x 130. It only displays what's playing. The page itself is
src-tauri/widget/widget.html if you want to restyle it.
SoundCloud sign in / sign out is now in Settings > Integrations.

## v23
- Accent: two colours from the album art, fading smoothly between tracks; used across the player bar,
  page header and Now Playing (and the OBS widget's background)
- Player controls stay centred; long titles scroll when you hover them
- Lyrics button in the player bar (replaces the sleep timer)
- OBS widget scales to any Browser Source size, scrolls long titles, and has a Clear button
- Tracks with "PREVIEW" in the title are hidden everywhere

## v25
- Widget designs (Settings > Integrations): Card, Slim bar, Square, Minimal, all scalable; rounded or square corners
- Track changes fade (cover art and text) in the app and the widget; accent colours fade too

## v26
- Header: Back button on the left, search in the middle
- Settings: sign in / sign out wording everywhere; "Output Preferences" holds loudness, output device and the OBS widget;
  second accent colour (greyed out while album-art colours are on)
- Discover never shows tracks you've liked; Rediscover shows likes you haven't played in a month
- Likes: All / Tracks / Mixes (Mixes left the sidebar)
- Right-click a track: play, play next, queue, playlist, like, artist, remove recommendation, open, copy link
- Platform filter chips appear on Likes, Discover, Rediscover and Search once more than one platform is on
- OBS widget fills the whole Browser Source (no margin or shadow)

## v27
- Discover / Rediscover keep their list when you come back; the refresh button beside the title gets a new one
- Settings: Last.fm options appear when signed in; widget options get their own section when the widget is on;
  Discord Rich Presence is under Output Preferences; "close to tray" is under Performance
- Widget: four more designs (Pill, Vinyl, Poster, Ticker) and a text size control
- "Artist - Title" uploads show the artist from the title; hover the artist to see who uploaded it

## v28
- Spotify is switched back on (Settings > Integrations). Last.fm and Spotify each have an on/off switch
  next to Sign in / Sign out that keeps you signed in.
- Right-click a track > Edit title and artist: fixes file-name titles; saved on this PC
- Widget design names show the size they were drawn at

## v29
- Platform switch on Likes, Search and Rediscover (shows only linked platforms: SoundCloud, Spotify, Local files;
  YouTube Music / Apple Music / Amazon Music are listed in PLATFORM_LIST for later). The Spotify tab is gone.
- Rediscover keeps loading as you scroll
- Settings > Local Music: on/off, "+ Add folder" (several folders, any drive), Rescan. Tags are indexed once
  in local_index.json; files play through a small server on 127.0.0.1 only.

## v30: chat song requests
Settings > Integrations > Chat song requests, then the Song Requests section:
- Request address: http://127.0.0.1:7879/request?key=YOURKEY&user=USER&q=SONG (plain-text reply for chat; add &format=json for JSON)
- Now playing: http://127.0.0.1:7879/now
- Streamer.bot: Fetch URL with user=%user% and q=%rawInputUrlEncoded%. Mix It Up: Web Request with user=$username and q=$allargs.
- Limits for waiting requests and per viewer; mixes are refused; "Remove all" clears waiting requests.
Each service in Integrations has a "Manage subscription" link.

## v31
Song requests follow the Search page's platform switch: SoundCloud, Spotify or Local files.
Spotify requests go into the app's own Spotify queue (Spotify tracks never share a queue with other platforms).

## v32: stream-safe output
Settings > Output Preferences:
- Stream-safe output (on by default): nothing plays on the Windows default device; pauses if the chosen device is unplugged.
- Spotify plays through: This app (always the default device) or the Spotify desktop app (remote-controlled;
  set Spotify's output in Windows' Volume mixer).

## v33
- Settings: Appearance, Integrations (SoundCloud / Last.fm / Spotify / Local music, each with an on/off switch;
  options fold out underneath), Output Preferences (Discord, widget, chat song requests), Audio (loudness,
  10-band equalizer, output device, stream-safe output, Spotify route), Performance (GPU, animations, tray).
- SoundCloud can be switched off; Likes / Search / Rediscover use Spotify or local files instead.
- Song requests: pick your chat bot for tailored setup steps; editable chat messages; "Clear requests" in the queue panel.
- Widget: animation options (fades, scrolling, vinyl spin, colour change speed).

## v34
- Equalizer and loudness changes are heard immediately (the audio chain is set up at start)
- Stream-safe output only shows, and only applies, while chat song requests are on

## v35
- Platform icons in the title bar (next to the name) choose the platform for the whole app
- App name: Settings > Appearance, or right-click the name in the title bar
- Fixed "Manage subscription" links for SoundCloud and Spotify

## v36: Discover on Spotify
Spotify removed recommendations and related artists for new apps (November 2024), so Discover on Spotify uses
Last.fm's similar artists plus Spotify search: For you (artists like the ones in your liked songs), Similar artists
(like the current Spotify track's artist) and genre chips. Without Last.fm, For you shows more songs from artists you like.

## v37
The sidebar follows the title-bar platform: SoundCloud playlists, Spotify playlists, or albums from your local files.
Spotify playlists need a fresh Spotify sign-in once (Settings > Integrations > Spotify > Sign out, then Sign in).

## v38: albums in lists
Album tiles (bigger cover, track count, play button) appear among the tracks on Likes, Search, Discover,
Rediscover and artist pages, for SoundCloud, Spotify and local files. The Albums button beside the page title
hides them; tracks from albums are never hidden.

## v39
- Autoplay (switch at the bottom of the queue panel, on by default): about 30 seconds before the
  queue runs out, about 10 similar tracks are added: SoundCloud related tracks, Spotify artists
  similar to the current one (via Last.fm), or more by the same artist / album from your files.
  Skips anything queued, removed from recommendations, or played in the last 3 hours, and keeps
  the Discover language choice. Waits while repeat is on; switching it off removes unplayed picks.
- "Add to queue" and chat requests go ahead of autoplay picks
- Repeat one / all now work for Spotify tracks too
- Long lists scroll faster (new pages are added instead of redrawing everything); the queue panel
  shows the next 100 tracks
- Discover shows why it picked tracks ("Because you liked ...") and how many the language filter hid
- Code: extras.js is split into dist/js/ (see Files)

## v40
- Removed stream-safe output, "Spotify plays through" (the Spotify desktop app mode) and
  "This app is routed away from my default device". Route the app in Windows instead:
  Settings > System > Sound > Volume mixer > OmniPulse.
- Spotify always plays in the app. Its sound comes from Spotify's protected player, which the app
  can't reach, so it follows the Windows routing above but skips the equalizer, leveling and the
  in-app output picker.

## v41: Spotify through the app's audio chain
- Spotify now plays through librespot (an unofficial open-source Spotify client, built into the
  Rust side) instead of Spotify's web player, at 320 kbps. Its audio goes through the same chain as
  everything else: equalizer, loudness leveling, volume and the output device in Settings.
- The first time you play a Spotify track, a browser tab asks you to approve "Spotify playback".
  This is a separate sign-in from the Spotify account above (it uses Spotify's own app id, which
  librespot needs). It's remembered in %APPDATA%\sc-client\librespot; Settings > Integrations >
  Spotify playback signs it in or out. Signing out of Spotify signs out of both.
- Needs Premium. librespot isn't approved by Spotify and is against its terms of use; it can also
  stop working when Spotify changes things, until librespot is updated (`cargo update -p librespot-core`).
- How it works: src-tauri/src/spotify_engine.rs decodes and streams PCM over a Tauri channel;
  dist/js/spotify-worklet.js plays it into the audio chain and reports back how much it has played.

## v42
- Spotify gets Discord activity, the OBS widget, lyrics, listening stats and Last.fm, like everything else.
- Platforms are merged: Likes, Search, Discover, Rediscover, album tiles and the sidebar show every
  active platform together. The title-bar icons now switch a platform on or off (same as its switch in
  Settings > Integrations). The queue can mix platforms; chat requests search the active platforms in
  order (SoundCloud, Spotify, local), or the platform a pasted link points to.
- Playlists on this PC: "+" in the sidebar (or New playlist... when adding a track) asks whether to keep
  it on this PC (any platform, stored only in the app) or on SoundCloud. Right-click a sidebar playlist to
  switch it to a playlist on this PC (the original stays on its platform and is just hidden; deleting the
  copy brings it back), or to rename / delete one.
- Drag tracks (from lists, the now-playing area or the queue) onto a playlist, the queue panel or the queue button.
- Player bar: transport, seek and volume on the left; cover, title, artist and album in the middle; like,
  add to playlist, lyrics, output device, more (...) and queue on the right. Right-clicking the
  now-playing area or "..." opens the track menu.
- Albums and the Discover language filter are chips beside Play / Shuffle; genre chips scroll sideways.
- Spotify playback sign-in moved into Spotify options (under Spotify in Integrations).
- Rediscover: "Fall in love all over again". Page subtitles no longer count tracks.
- Go+ tracks still only come as 30-second previews: SoundCloud doesn't give full Go+ streams to other apps.

## v43
- Status and error messages slide out above the player bar for a few seconds instead of sitting under the page title.
- The widget settings only show animation options the chosen design uses ("Spin the record" is Vinyl-only).
- Long tracks (over 15 minutes) are called "Sets & podcasts" now, so they aren't confused with Spotify's own mixes.
- Sidebar: Playlists and Albums split the space below the pages and scroll separately.
- Platform logos are plain white (dark themes) or black (Light, Paper); Spotify uses its own mark.
- Spotify tracks can be liked (saved to your Spotify library). Sign out of Spotify and back in once so the
  app is allowed to change your library.
- Favourites: the star next to the heart (in lists, the player bar and Now Playing) works for every
  platform, including local files. Favourites are kept on this PC and listed on the Favourites page.
- Cover art on playlist, album and Favourites pages: an album shows its cover; a playlist shows up to four
  different covers from its tracks in one square, split diagonally (two halves, a "Y" of three, an "X" of four).
- Search left the sidebar: click the magnifying glass in the search bar for the Search page (or to search for what you typed).
- Search: "Hide likes" (beside Albums) leaves tracks you have liked on SoundCloud or Spotify out of the results.
- Chat requests prefer your likes: a liked song whose title and artist contain every word asked for wins; otherwise a liked track among the top 5 results beats the first result. Links still play exactly what was linked.

## v44
- Discover is a feed of shelves, like SoundCloud's feed and Spotify's home: Daily Mixes and artist stations
  made from your likes, More of what you like, New from artists you follow, Trending by genre. Every
  platform takes part and each tile shows its platform's logo. Shelves scroll sideways (or Show all).
  Spotify's own Daily Mixes can't be read by apps like this one, so these are made by the app.
- Sets & podcasts page in the sidebar (anything over 15 minutes), with a "Liked only" chip.
- Favourites: starring a track likes it too (unstarring keeps the like); a "Favourites only" chip; the
  whole feature can be switched off in Settings > Library.
- Settings reorganised into Look, Accounts, Library, Sound, Streaming and App, with shorter wording and
  the same row design throughout. Spotify shows the account you're signed in as.
- Spotify playlists you follow (and Spotify's own) open again. Since February 2026 the Web API only lists the tracks of playlists you own, so the others are read through the Spotify playback engine (librespot), up to 1000 tracks.
- Discover shows Spotify's own Daily Mixes (and Discover Weekly, Release Radar and your top mixes, under "More mixes from Spotify"), read with the playback engine's token. The app no longer builds Spotify-only mixes: its "Your Mix" and station tiles are seeded by what you like on SoundCloud and draw on every platform (your likes anywhere, your files, new tracks from SoundCloud and Spotify).

## v45
- Likes is now Library: everything you've liked on each platform plus your music files. It opens at once
  from a cache and checks for updates in the background (playlists too).
- Queue: Now Playing, Next up (what you queued) and Autoplay, which can be switched off. The queue and
  position come back when you restart. Autoplay takes turns between platforms, can avoid tracks played in
  the last few hours, and can leave out sets & podcasts.
- Shuffle is fair: tracks you haven't heard (or not for longest) come first, remembered between sessions.
  Tracks you queued and chat requests aren't shuffled. Rediscover alternates platforms evenly.
- Discover: I'm Feeling Lucky plays a random mix from every platform.
- Even out loudness works like RoughRider (a compressor that rides the level), with a Custom strength.
- Discord: choose what's shown (art, time, links) and a Listen along button.
- Stream Deck: switch on in Settings > App, then install the plugin in `streamdeck/` (see its README).
- Also: album on Now Playing, hidden playlists, coloured platform icons, your SoundCloud plan in Settings,
  and a player bar and track list that fit narrow windows.

## v46: match loudness, skip silences
- Settings > Sound > Match loudness (Off, Quiet, Normal, Loud) replaces Even out loudness. Each track is
  measured over its whole length (integrated loudness in LUFS, as Spotify and YouTube do) and played at
  one fixed gain, so tracks match without being squashed. Normal is -14 LUFS, Quiet -18, Loud -11.
  - Local files are measured from disk, SoundCloud tracks by downloading the stream (the next track in
    the queue is measured ahead, so it starts at the right level). Sets and podcasts over 15 minutes are
    measured from their first 10 minutes. Results are kept in %APPDATA%\sc-client\loudness.json.
  - Spotify uses Spotify's own whole-track values (librespot's normalisation), from the next track.
  - Quiet tracks are turned up at most 9 dB; the limiter stops peaks clipping.
  - Settings shows what the current track measured and how much it's turned up or down.
- Settings > Sound > Skip long silences (Off, 5, 8, 15 or 30 s): jumps over silence inside a track, and
  moves on when the rest of it is silent. Where a track isn't measured (Spotify, the rest of a long set)
  it moves on after that long in silence.
- To check a file: `cargo run -p omnipulse-core --example measure -- <file>`.

## v47: YouTube Music
- A fourth platform, next to SoundCloud, Spotify and your files: in Search, the Library, Discover's mixes
  and genres, Rediscover, autoplay (YouTube Music's own radio) and chat requests (YouTube links work).
- No sign-in: search and radio come from YouTube Music's web API (the client version is read from
  music.youtube.com, so it doesn't go stale), likes are kept on this PC, and nothing touches a Google
  account. This uses YouTube's private API, which YouTube's terms don't allow and which can change.
- Audio comes from yt-dlp, which needs a JavaScript runtime. Settings > Accounts > YouTube Music > Set up
  downloads both from their GitHub releases into %APPDATA%sc-clientbin (about 58 MB, checked against
  their published checksums), and yt-dlp updates itself daily. The next track's audio is looked up ahead, and the local server
  passes the audio through in 4 MB pieces (as YouTube's player asks for it).
- YouTube tracks are measured for Match loudness from their audio, like SoundCloud's.
- Try it from a terminal: `cargo run -p omnipulse-core --example yt -- <search words>`.
