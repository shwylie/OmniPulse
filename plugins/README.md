# OmniPulse plugins

A plugin adds a source of music: songs to search, play, and (optionally) a library of saved songs.
Add one in **Settings > Plugins > Add plugin**: choose its file, or paste the web address of its
`manifest.json`. OmniPulse reads it first and shows, in plain words, what it will and won't be able
to do; it's added only if you agree, and only if it's still exactly what you saw.

`internet-archive/` here is a working example, and `internet-archive.omniplugin` is the same plugin
as one file.

## One file: .omniplugin

An `.omniplugin` file is a zip with `manifest.json` and the script, at the top or in one folder. To
make one on Windows: select both files, right-click > Send to > Compressed (zipped) folder, then
rename the .zip to .omniplugin. (A plugin's `manifest.json` can also be chosen directly while you
work on it; its script is read from beside it.)

## How plugins are kept safe

A plugin's script runs in a sandboxed frame with its own origin and a policy that blocks every network
request. It can't see your accounts, your files, your settings or the rest of the app. It can only:

- register what it offers (`OmniPulse.register`),
- ask for web requests (`OmniPulse.fetch`), which the app makes for it **only to the sites listed in
  its manifest** (checked again in Rust, redirects included; cookies can't be sent),
- read its own settings (`OmniPulse.settings`).

You see the sites before installing. Audio plays through the app's local server, from those sites only.

## More than music: live connections and the app

A plugin can also run on its own (a chatbot does), with two capabilities more, each shown to you before
it's added:

- `"sockets"`: live connections (WebSockets) to its sites: `OmniPulse.socket("wss://...")` gives
  `{ send(text), close(), onopen, onmessage(text), onclose(why) }`. Only `wss://` to the sites in its
  manifest, or `ws://` to this PC (`127.0.0.1` in its hosts: OBS's WebSocket server, say).
- `"app"`: the app itself, each call a promise of its answer:
  - `OmniPulse.app.request(user, query, platform)`: a song request, with Song Requests' limits and replies.
  - `.wrongSong(user, isMod)`: takes back the viewer's last request (a moderator's: the last anyone made).
  - `.nowPlaying()`: what's playing, as a sentence for chat.
  - `.control("next" | "play")`: the next song, or play / pause.
  - `.alert({ type, user, amount, message })`: an alert on the Stream Widget, as a bot's `/alert` sends.
  - `.sound(path, volume)`: a sound file on this PC (MP3, WAV, Ogg, FLAC, M4A, AAC, Opus) over the music,
    its volume 0 to 100.
  - `.openUrl("https://...")`: a page in your browser (a sign-in, say); `.copyText("https://...")`: the
    address copied instead, for a browser of your choosing. Both https only.
  - `.say(text)`: a line on the app's status line.
  - `.saveSettings({ ... })`: its own settings kept (a sign-in's tokens, say).

Every plugin also has `setTimeout`, `setInterval` and their clears, and can register `start()`, called
once it's running. A settings field of type `"button"` (with `"press"`: its label, and `"call"`: the
method it calls with the page's values) runs one of its methods, a sign-in say; what it returns is shown.

`chatbot/` here is OmniPulse's own **Stream Suite** (its chatbot, alerts, goals, timers, widgets and OBS
docks), made this way. A few calls are kept for it alone and aren't open to other plugins: OBS's docks,
the Suite's widgets, Show on Stream, speech, web requests outside its manifest, and writing files.

## manifest.json

```json
{
  "id": "my-source",
  "name": "My Source",
  "version": "1.0.0",
  "author": "You",
  "description": "What it plays.",
  "main": "main.js",
  "hosts": ["api.example.com", "*.cdn.example.com"],
  "capabilities": ["search", "play", "library"],
  "icon": "M12 2a10 10 0 1 0 0 20 10 10 0 1 0 0-20z",
  "settings": [
    { "key": "token", "label": "API token", "type": "password", "note": "From your account page." },
    { "key": "hq", "label": "High quality", "type": "toggle", "default": true },
    { "key": "region", "label": "Region", "type": "select", "options": ["us", "eu"], "default": "eu" }
  ]
}
```

- `id`: lower-case letters, digits and dashes.
- `hosts`: site names, optionally `*.` for their subdomains. Only https (or http to 127.0.0.1).
- `icon`: SVG path data for a 24x24 icon.
- `settings` types: `text`, `password`, `number`, `toggle`, `select` (with `"options"`: its values), and
  `button` (see above). Each can have a `"note"`, shown under it, and a `"default"`. They show under the
  plugin in Settings > Plugins (its gear); values are kept on this PC.

## main.js

```js
OmniPulse.register({
  // required: the songs matching a search
  async search(query) {
    const r = await OmniPulse.fetch("https://api.example.com/search?q=" + encodeURIComponent(query), {
      headers: { Authorization: "Bearer " + OmniPulse.settings.token },
    });
    const data = await r.json();
    return data.items.map((x) => ({
      id: x.id, title: x.title, artist: x.artist, album: x.album,
      duration_ms: x.ms, artwork: x.cover, link: x.pageUrl,
      license: x.licence,   // optional: "cc-by", "cc-by-nc-sa" and the like, or "cc0"; only these play with Stream-Safe Music on
      // and anything else you need later
    }));
  },
  // required: a playable address for one of them (any format the app's player takes: MP3, AAC, FLAC, Ogg, Opus, WAV)
  async resolve(track) {
    return { url: "https://cdn.example.com/stream/" + track.id };
  },
  // optional: your saved songs there, for the Library
  async library() { return []; },
});

OmniPulse.onSettings((s) => { /* settings changed */ });
OmniPulse.log("started");   // shows in the app's console
```

`OmniPulse.fetch(url, { method, headers, body })` resolves to `{ ok, status, headers, text(), json() }`.
Calls that take longer than 25 seconds are given up on.

Only make plugins for music you're allowed to stream.
