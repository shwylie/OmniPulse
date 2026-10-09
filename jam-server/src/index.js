// OmniPulse's relay for Listen Along and Jams (a Cloudflare Worker with a Durable Object per room).
//
// The host's OmniPulse opens a room and sends what it's playing: the track, where it's at, the queue
// coming up and its own touches (start/end times, title fixes). Listeners' OmniPulse apps receive
// that and play the same songs themselves, through their own accounts and their own EQ and loudness.
// No audio passes through here, only those small updates.
//
//   POST /api/rooms                 open a room: { kind: "listen" | "jam", host }  ->  { code, hostKey, inviteKey?, joinUrl }
//   GET  /join/<code>               the page a Listen Along button opens (open in OmniPulse, or download it)
//   GET  /jam/<code>?k=<invite>     the same for a Jam invite
//   GET  /api/rooms/<code>          who's hosting and what's playing (for that page)
//   GET  /ws/<code>?role=host&key=<hostKey>              the host's connection
//   GET  /ws/<code>?role=listener&name=<n>[&k=<invite>]  a listener's (a Jam needs its invite)
//
// Shared playlists (a playlist on someone's PC, with each song's links on every platform it's on):
//   POST /api/shares                { title, by, tracks }  ->  { code, key, url }
//   PUT  /api/shares/<code>?key=<k> the same, updated
//   GET  /api/shares/<code>         the playlist (for OmniPulse to add it)
//   GET  /p/<code>                  its page: the songs, their links, and Open in OmniPulse
//
// Settings (wrangler.toml [vars], or the Cloudflare dashboard): DOWNLOAD_URL, where "Download OmniPulse"
// points; APP_SCHEME, the app's link scheme ("omnipulse").

const ROOM_IDLE_MS = 12 * 3600 * 1000;   // a room nobody has used for this long goes away
const MAX_LISTENERS = 50;
const MAX_MESSAGE = 64 * 1024;
const SHARE_IDLE_MS = 180 * 86400 * 1000;   // a shared playlist nobody has updated or opened for this long goes away
const MAX_SHARE = 1024 * 1024;
const MAX_SHARE_TRACKS = 2000;

const json = (v, status = 200) => new Response(JSON.stringify(v), {
  status,
  headers: { "content-type": "application/json", "access-control-allow-origin": "*", "cache-control": "no-store" },
});

function randomCode(n, alphabet = "abcdefghjkmnpqrstuvwxyz23456789") {
  const bytes = crypto.getRandomValues(new Uint8Array(n));
  return [...bytes].map((b) => alphabet[b % alphabet.length]).join("");
}

const esc = (s) => String(s || "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

function joinPage(env, info, appLink) {
  const title = info ? (info.kind === "jam" ? "Join " + info.host + "'s Jam" : "Listen along with " + info.host) : "This session has ended";
  const now = info && info.track ? `<p class="now">Now playing <b>${esc(info.track.title)}</b>${info.track.artist ? " by " + esc(info.track.artist) : ""}</p>` : "";
  const download = env.DOWNLOAD_URL || "";
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(title)} · OmniPulse</title>
<meta property="og:title" content="${esc(title)}"><meta property="og:description" content="Listen together on OmniPulse">
<style>
  :root { color-scheme: dark; }
  body { margin: 0; min-height: 100vh; display: grid; place-items: center; font-family: system-ui, "Segoe UI", sans-serif;
    background: radial-gradient(circle at 20% 15%, #4b4fb8, #12131c 60%); color: #fff; }
  main { width: min(420px, calc(100% - 32px)); padding: 32px; border-radius: 20px; background: rgba(15, 16, 24, 0.75); box-shadow: 0 20px 60px rgba(0,0,0,.45); text-align: center; }
  h1 { margin: 0 0 8px; font-size: 26px; }
  .now { color: #c8cbe0; margin: 0 0 22px; }
  a.btn { display: block; margin: 10px 0; padding: 13px; border-radius: 999px; font-weight: 700; text-decoration: none; }
  .open { background: #8e9bff; color: #111318; }
  .get { background: rgba(255,255,255,.1); color: #fff; }
  .small { color: #9aa0bd; font-size: 13px; margin-top: 18px; }
</style></head><body><main>
  <h1>${esc(title)}</h1>${now}
  ${info ? `<a class="btn open" id="open" href="${esc(appLink)}">Open in OmniPulse</a>` : ""}
  ${download ? `<a class="btn get" href="${esc(download)}">Download OmniPulse</a>` : ""}
  <p class="small">${info ? "Have OmniPulse? It opens by itself. If nothing happens, download it, install it, then open this link again." : "Ask them to send a new link."}</p>
</main>
${info ? `<script>setTimeout(function () { location.href = ${JSON.stringify(appLink)}; }, 400);</script>` : ""}
</body></html>`;
}

const fmtLen = (ms) => { const s = Math.round((Number(ms) || 0) / 1000); return s ? Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0") : ""; };
const LINK_NAMES = { soundcloud: "SoundCloud", spotify: "Spotify", youtube: "YouTube Music" };
const httpsLink = (u) => typeof u === "string" && /^https:\/\//.test(u);

function sharePage(env, share, appLink) {
  const title = share ? share.title : "This playlist isn't shared any more";
  const download = env.DOWNLOAD_URL || "";
  const rows = share ? share.tracks.map((t, i) => {
    const links = Object.keys(LINK_NAMES).filter((k) => t.links && httpsLink(t.links[k])).map((k) => '<a href="' + esc(t.links[k]) + '" target="_blank" rel="noopener">' + LINK_NAMES[k] + "</a>").join(" · ");
    return '<li><span class="n">' + (i + 1) + '</span><span class="t"><b>' + esc(t.title) + '</b><br><span class="a">' + esc(t.artist) + '</span></span><span class="l">' + links + '</span><span class="d">' + fmtLen(t.ms) + "</span></li>";
  }).join("") : "";
  const sub = share ? (share.by ? "Shared by " + esc(share.by) + " · " : "") + share.tracks.length + (share.tracks.length === 1 ? " song" : " songs") : "Ask them to send a new link.";
  const acts = share ? '<div class="acts"><a class="btn open" href="' + esc(appLink) + '">Open in OmniPulse</a>' + (download ? '<a class="btn get" href="' + esc(download) + '">Download OmniPulse</a>' : "") + "</div><ol>" + rows + "</ol>" : "";
  return '<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">' +
    "<title>" + esc(title) + " · OmniPulse</title>" +
    '<meta property="og:title" content="' + esc(title) + '"><meta property="og:description" content="A playlist from OmniPulse, with every song on SoundCloud, Spotify and YouTube Music">' +
    "<style>" +
    ":root { color-scheme: dark; }" +
    'body { margin: 0; min-height: 100vh; font-family: system-ui, "Segoe UI", sans-serif; background: radial-gradient(circle at 20% 0%, #4b4fb8, #12131c 55%) fixed; color: #fff; }' +
    "main { width: min(760px, calc(100% - 32px)); margin: 40px auto; padding: 28px; border-radius: 20px; background: rgba(15, 16, 24, 0.78); box-shadow: 0 20px 60px rgba(0,0,0,.45); box-sizing: border-box; }" +
    "h1 { margin: 0 0 6px; font-size: 28px; overflow-wrap: anywhere; }" +
    ".sub { color: #c8cbe0; margin: 0 0 18px; }" +
    ".acts { display: flex; gap: 10px; flex-wrap: wrap; margin-bottom: 18px; }" +
    "a.btn { padding: 11px 18px; border-radius: 999px; font-weight: 700; text-decoration: none; }" +
    ".open { background: #8e9bff; color: #111318; } .get { background: rgba(255,255,255,.1); color: #fff; }" +
    "ol { list-style: none; margin: 0; padding: 0; }" +
    "li { display: grid; grid-template-columns: 28px 1fr auto 44px; gap: 12px; align-items: center; padding: 9px 4px; border-top: 1px solid rgba(255,255,255,.08); }" +
    ".n, .d, .a { color: #9aa0bd; font-size: 13px; } .d { text-align: right; } .t { min-width: 0; overflow-wrap: anywhere; }" +
    ".l { font-size: 13px; text-align: right; } .l a { color: #b9c0ff; }" +
    "@media (max-width: 560px) { li { grid-template-columns: 22px 1fr 40px; } .l { grid-column: 2 / 4; text-align: left; } }" +
    "</style></head><body><main><h1>" + esc(title) + '</h1><p class="sub">' + sub + "</p>" + acts + "</main></body></html>";
}

// What a shared playlist keeps (and nothing else from what was sent)
function shareClean(body) {
  const s = (v, n) => String(v == null ? "" : v).slice(0, n);
  const tracks = (Array.isArray(body.tracks) ? body.tracks : []).slice(0, MAX_SHARE_TRACKS).map((t) => {
    const links = {};
    Object.keys(LINK_NAMES).forEach((k) => { const u = t && t.links && t.links[k]; if (httpsLink(u)) links[k] = u.slice(0, 500); });
    return { title: s(t && t.title, 300), artist: s(t && t.artist, 300), ms: Number(t && t.ms) || 0, links, snap: t && t.snap && typeof t.snap === "object" ? t.snap : null };
  });
  return { title: s(body.title, 200) || "Playlist", by: s(body.by, 60), tracks };
}

async function readJson(req) {
  const text = await req.text();
  if (text.length > MAX_SHARE) return { error: json({ error: "too big" }, 413) };
  try { return { body: JSON.parse(text) }; } catch (e) { return { error: json({ error: "not JSON" }, 400) }; }
}

export default {
  async fetch(req, env) {
    const url = new URL(req.url);
    const parts = url.pathname.split("/").filter(Boolean);
    if (req.method === "OPTIONS") return new Response(null, { headers: { "access-control-allow-origin": "*", "access-control-allow-methods": "GET, POST, PUT", "access-control-allow-headers": "content-type" } });

    // shared playlists
    if (url.pathname === "/api/shares" && req.method === "POST") {
      const r = await readJson(req);
      if (r.error) return r.error;
      const code = randomCode(8), key = randomCode(32);
      const stub = env.SHARES.get(env.SHARES.idFromName(code));
      const put = await stub.fetch("https://share/put", { method: "POST", body: JSON.stringify({ key, share: shareClean(r.body), fresh: true }) });
      if (!put.ok) return put;
      return json({ code, key, url: url.origin + "/p/" + code });
    }
    if ((parts[0] === "p" && parts.length === 2) || (parts[0] === "api" && parts[1] === "shares" && parts.length === 3)) {
      const code = parts[parts.length - 1];
      if (!/^[a-z0-9]{8}$/.test(code)) return json({ error: "not found" }, 404);
      const stub = env.SHARES.get(env.SHARES.idFromName(code));
      if (parts[0] === "api" && req.method === "PUT") {
        const r = await readJson(req);
        if (r.error) return r.error;
        return stub.fetch("https://share/put", { method: "POST", body: JSON.stringify({ key: url.searchParams.get("key") || "", share: shareClean(r.body) }) });
      }
      const got = await stub.fetch("https://share/get");
      if (parts[0] === "api") return got;
      const share = got.ok ? await got.json().catch(() => null) : null;
      const appLink = (env.APP_SCHEME || "omnipulse") + "://playlist/" + code + "?relay=" + encodeURIComponent(url.origin);
      return new Response(sharePage(env, share, appLink), { headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store" } });
    }

    // a new room
    if (req.method === "POST" && url.pathname === "/api/rooms") {
      let body = {};
      try { body = await req.json(); } catch (e) {}
      const kind = body.kind === "jam" ? "jam" : "listen";
      const code = randomCode(8);
      const hostKey = randomCode(32);
      const inviteKey = kind === "jam" ? randomCode(16) : "";
      const stub = env.ROOMS.get(env.ROOMS.idFromName(code));
      await stub.fetch("https://room/init", { method: "POST", body: JSON.stringify({ kind, host: String(body.host || "Someone").slice(0, 40), hostKey, inviteKey }) });
      const base = url.origin;
      return json({ code, hostKey, inviteKey, joinUrl: kind === "jam" ? `${base}/jam/${code}?k=${inviteKey}` : `${base}/join/${code}` });
    }

    if (!parts.length) return Response.redirect(env.DOWNLOAD_URL || "https://github.com", 302);
    const code = (parts[0] === "api" ? parts[2] : parts[1]) || "";
    if (!/^[a-z0-9]{8}$/.test(code)) return json({ error: "not found" }, 404);
    const stub = env.ROOMS.get(env.ROOMS.idFromName(code));

    if (parts[0] === "api" && parts[1] === "rooms" && req.method === "GET") return stub.fetch("https://room/info");
    if (parts[0] === "join" || parts[0] === "jam") {
      const info = await (await stub.fetch("https://room/info")).json().catch(() => null);
      const ok = info && !info.error && (parts[0] === "join" ? info.kind === "listen" : info.kind === "jam");
      const scheme = env.APP_SCHEME || "omnipulse";
      const appLink = `${scheme}://${parts[0]}/${code}` + (parts[0] === "jam" ? "?k=" + encodeURIComponent(url.searchParams.get("k") || "") + "&relay=" + encodeURIComponent(url.origin) : "?relay=" + encodeURIComponent(url.origin));
      return new Response(joinPage(env, ok ? info : null, appLink), { headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store" } });
    }
    if (parts[0] === "ws") {
      if (req.headers.get("Upgrade") !== "websocket") return json({ error: "expected a websocket" }, 426);
      return stub.fetch(req);
    }
    return json({ error: "not found" }, 404);
  },
};

// One room: who's hosting, the latest state, and everyone connected
export class Room {
  constructor(ctx, env) {
    this.ctx = ctx;
    this.env = env;
  }

  async meta() {
    return (await this.ctx.storage.get("meta")) || null;
  }

  async fetch(req) {
    const url = new URL(req.url);
    if (url.pathname === "/init") {
      const m = await req.json();
      await this.ctx.storage.put("meta", Object.assign(m, { created: Date.now(), used: Date.now() }));
      await this.ctx.storage.setAlarm(Date.now() + ROOM_IDLE_MS);
      return json({ ok: true });
    }
    const meta = await this.meta();
    if (!meta) return json({ error: "this session has ended" }, 404);
    if (url.pathname === "/info") {
      const state = (await this.ctx.storage.get("state")) || {};
      return json({ kind: meta.kind, host: meta.host, track: state.track ? { title: state.track.title, artist: state.track.artist } : null, listeners: this.listeners().length });
    }
    // a connection
    const role = url.searchParams.get("role");
    if (role === "host") {
      if (url.searchParams.get("key") !== meta.hostKey) return json({ error: "not the host" }, 403);
      this.ctx.getWebSockets("host").forEach((w) => { try { w.close(4000, "the host connected again"); } catch (e) {} });
    } else {
      if (meta.kind === "jam" && url.searchParams.get("k") !== meta.inviteKey) return json({ error: "this Jam needs an invite" }, 403);
      if (this.listeners().length >= MAX_LISTENERS) return json({ error: "this session is full" }, 429);
    }
    const pair = new WebSocketPair();
    const name = String(url.searchParams.get("name") || "A listener").slice(0, 40);
    const id = randomCode(10);
    this.ctx.acceptWebSocket(pair[1], [role === "host" ? "host" : "listener"]);
    pair[1].serializeAttachment({ role: role === "host" ? "host" : "listener", name, id });
    if (role !== "host") {
      const state = await this.ctx.storage.get("state");
      pair[1].send(JSON.stringify({ type: "hello", kind: meta.kind, host: meta.host, canAdd: meta.kind === "jam", you: id }));
      if (state) pair[1].send(JSON.stringify({ type: "state", state }));
    }
    await this.touch();
    this.sendMembers();
    return new Response(null, { status: 101, webSocket: pair[0] });
  }

  listeners() {
    return this.ctx.getWebSockets("listener");
  }

  sendMembers() {
    const list = this.listeners().map((w) => (w.deserializeAttachment() || {}).name || "A listener");
    const msg = JSON.stringify({ type: "members", list });
    this.ctx.getWebSockets().forEach((w) => { try { w.send(msg); } catch (e) {} });
  }

  async touch() {
    const meta = await this.meta();
    if (!meta) return;
    meta.used = Date.now();
    await this.ctx.storage.put("meta", meta);
    await this.ctx.storage.setAlarm(Date.now() + ROOM_IDLE_MS);
  }

  async webSocketMessage(ws, raw) {
    if (typeof raw !== "string" || raw.length > MAX_MESSAGE) return;
    let m;
    try { m = JSON.parse(raw); } catch (e) { return; }
    const who = ws.deserializeAttachment() || {};
    if (who.role === "host") {
      if (m.type === "state" && m.state && typeof m.state === "object") {
        await this.ctx.storage.put("state", m.state);
        const msg = JSON.stringify({ type: "state", state: m.state });
        this.listeners().forEach((w) => { try { w.send(msg); } catch (e) {} });
        await this.touch();
      } else if (m.type === "end") {
        const msg = JSON.stringify({ type: "ended" });
        this.listeners().forEach((w) => { try { w.send(msg); w.close(1000, "ended"); } catch (e) {} });
        await this.ctx.storage.deleteAll();
      }
      return;
    }
    // a Jam member adding a song: passed to the host, who adds it
    const meta = await this.meta();
    if (m.type === "add" && meta && meta.kind === "jam" && m.track && typeof m.track === "object") {
      const msg = JSON.stringify({ type: "add", track: m.track, by: who.name || "A listener" });
      this.ctx.getWebSockets("host").forEach((w) => { try { w.send(msg); } catch (e) {} });
    }
  }

  async webSocketClose(ws) {
    try { ws.close(); } catch (e) {}
    this.sendMembers();
  }

  async webSocketError() {
    this.sendMembers();
  }

  // nobody's used it in a while: it goes
  async alarm() {
    const meta = await this.meta();
    if (meta && Date.now() - (meta.used || 0) < ROOM_IDLE_MS) {
      await this.ctx.storage.setAlarm((meta.used || Date.now()) + ROOM_IDLE_MS);
      return;
    }
    this.ctx.getWebSockets().forEach((w) => { try { w.close(1000, "ended"); } catch (e) {} });
    await this.ctx.storage.deleteAll();
  }
}

// One shared playlist: what it holds, and the key its owner updates it with
export class Share {
  constructor(ctx, env) {
    this.ctx = ctx;
    this.env = env;
  }

  async fetch(req) {
    const url = new URL(req.url);
    const meta = await this.ctx.storage.get("meta");
    if (url.pathname === "/put") {
      const m = await req.json();
      if (m.fresh ? !!meta : !meta || m.key !== meta.key) return json({ error: "not yours to change" }, 403);
      await this.ctx.storage.put("meta", { key: meta ? meta.key : m.key, used: Date.now() });
      await this.ctx.storage.put("share", m.share);
      await this.ctx.storage.setAlarm(Date.now() + SHARE_IDLE_MS);
      return json({ ok: true });
    }
    if (!meta) return json({ error: "this playlist isn't shared any more" }, 404);
    meta.used = Date.now();
    await this.ctx.storage.put("meta", meta);
    await this.ctx.storage.setAlarm(Date.now() + SHARE_IDLE_MS);
    return json(await this.ctx.storage.get("share"));
  }

  async alarm() {
    const meta = await this.ctx.storage.get("meta");
    if (meta && Date.now() - (meta.used || 0) < SHARE_IDLE_MS) {
      await this.ctx.storage.setAlarm((meta.used || Date.now()) + SHARE_IDLE_MS);
      return;
    }
    await this.ctx.storage.deleteAll();
  }
}
