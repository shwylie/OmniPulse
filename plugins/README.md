# OmniPulse plugins

A plugin adds a source of music: songs to search, play, and (optionally) a library of saved songs.
Add one in **Settings > Plugins > Add plugin**, from the web address of its
`manifest.json` or its folder. `internet-archive/` here is a working example.

## How plugins are kept safe

A plugin's script runs in a sandboxed frame with its own origin and a policy that blocks every network
request. It can't see your accounts, your files, your settings or the rest of the app. It can only:

- register what it offers (`OmniPulse.register`),
- ask for web requests (`OmniPulse.fetch`), which the app makes for it **only to the sites listed in
  its manifest** (checked again in Rust, redirects included; cookies can't be sent),
- read its own settings (`OmniPulse.settings`).

You see the sites before installing. Audio plays through the app's local server, from those sites only.

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
- `settings` types: `text`, `password`, `number`, `toggle`, `select`. They show under the plugin in
  Settings; values are kept on this PC.

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
      duration_ms: x.ms, artwork: x.cover, link: x.pageUrl,   // and anything else you need later
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
