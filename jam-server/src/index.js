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
//   GET  /ask/<code>                the page the Discord "Ask to Jam" button opens (it asks the host)
//   GET  /api/rooms/<code>          who's hosting and what's playing (for that page)
//   GET  /ws/<code>?role=host&key=<hostKey>              the host's connection
//   GET  /ws/<code>?role=listener&name=<n>[&k=<invite>]  a listener's (a Jam needs its invite)
//   GET  /ws/<code>?role=asker&name=<n>&cid=<app id>     someone asking: in a Listen Along room, for a Jam
//        (the host is told { type: "ask", id, name, cid }, answers { type: "askReply", id, ok, url, busy },
//        the asker is told { type: "askReply", ok, url }); in a Jam, to join it (the host is told
//        { type: "joinAsk", ... }, answers { type: "admit", id, ok, url }, the asker is told
//        { type: "admitted", url } or { type: "declined" })
//   A Jam's invite lets you straight in (unless the host turned you away before). A Jam member can
//   send { type: "add", track, next }, { type: "skip", d } and { type: "move", id, to }, passed to the host to do.
//
// Shared playlists (a playlist on someone's PC, with each song's links on every platform it's on):
//   POST /api/shares                { title, by, tracks }  ->  { code, key, url }
//   PUT  /api/shares/<code>?key=<k> the same, updated
//   GET  /api/shares/<code>         the playlist (for OmniPulse to add it), with its rev (each change counts)
//   POST /api/shares/<code>/collab?key=<k>[&stop=1]   collaborative: an edit key for its invite -> { editKey }
//        (stop: no longer; a collaborative one is never let go for being idle)
//   POST /api/shares/<code>/edit?k=<key or editKey>   { op: "add", track } | { op: "remove", id } (a
//        track's snap.id), from its owner or a collaborator -> { rev }
//   GET  /p/<code>                  its page: the songs, their links, and Open in OmniPulse
//
// Settings (wrangler.toml [vars], or the Cloudflare dashboard): DOWNLOAD_URL, where "Download OmniPulse"
// points; APP_SCHEME, the app's link scheme ("omnipulse").

const ROOM_IDLE_MS = 12 * 3600 * 1000;   // a room nobody has used for this long goes away
const HOST_GRACE_MS = 90 * 1000;          // the host gone (their app closed: an update, say) this long: it's over for everyone
const MAX_LISTENERS = 50;
const MAX_MESSAGE = 64 * 1024;
// asking to join: each app (by its id) has one request waiting at a time, a network at most two,
// a room at most ten; someone declined can't ask again in that room
const MAX_WAITING = 10;
const MAX_WAITING_PER_IP = 2;
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

function joinPage(env, info, appLink, ask) {
  const title = info ? (ask ? (info.kind === "jam" ? "Ask to join " + info.host + "'s Jam" : "Ask " + info.host + " to start a Jam") : info.kind === "jam" ? "Join " + info.host + "'s Jam" : "Listen along with " + info.host) : "This session has ended";
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
  <p class="small">${info ? (download ? "Have OmniPulse? It opens by itself. If nothing happens, download it, install it, then open this link again." : "It opens in OmniPulse by itself. If nothing happens, click Open in OmniPulse.") : "Ask them to send a new link."}</p>
</main>
${info ? `<script>setTimeout(function () { location.href = ${JSON.stringify(appLink)}; }, 400);</script>` : ""}
</body></html>`;
}

const fmtLen = (ms) => { const s = Math.round((Number(ms) || 0) / 1000); return s ? Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0") : ""; };
const LINK_NAMES = { soundcloud: "SoundCloud", spotify: "Spotify", youtube: "YouTube Music" };
const httpsLink = (u) => typeof u === "string" && /^https:\/\//.test(u);

// (invite: a collaborator's link, which joins it rather than copies it)
function sharePage(env, share, appLink, invite) {
  const title = share ? share.title : "This playlist isn't shared any more";
  const download = env.DOWNLOAD_URL || "";
  const rows = share ? share.tracks.map((t, i) => {
    const links = Object.keys(LINK_NAMES).filter((k) => t.links && httpsLink(t.links[k])).map((k) => '<a href="' + esc(t.links[k]) + '" target="_blank" rel="noopener">' + LINK_NAMES[k] + "</a>").join(" · ");
    return '<li><span class="n">' + (i + 1) + '</span><span class="t"><b>' + esc(t.title) + '</b><br><span class="a">' + esc(t.artist) + '</span></span><span class="l">' + links + '</span><span class="d">' + fmtLen(t.ms) + "</span></li>";
  }).join("") : "";
  const sub = share ? (share.by ? "Shared by " + esc(share.by) + " · " : "") + share.tracks.length + (share.tracks.length === 1 ? " song" : " songs") : "Ask them to send a new link.";
  const acts = share ? '<div class="acts"><a class="btn open" href="' + esc(appLink) + '">' + (invite ? "Join in OmniPulse" : "Open in OmniPulse") + '</a>' + (download ? '<a class="btn get" href="' + esc(download) + '">Download OmniPulse</a>' : "") + "</div><ol>" + rows + "</ol>" : "";
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
    // (by: who added it, in a collaborative playlist)
    return { title: s(t && t.title, 300), artist: s(t && t.artist, 300), ms: Number(t && t.ms) || 0, links, snap: t && t.snap && typeof t.snap === "object" ? t.snap : null, by: s(t && t.by, 60) };
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
    // collaborative playlists: the edit key, and the edits
    if (parts[0] === "api" && parts[1] === "shares" && parts.length === 4 && req.method === "POST" && (parts[3] === "collab" || parts[3] === "edit")) {
      const code = parts[2];
      if (!/^[a-z0-9]{8}$/.test(code)) return json({ error: "not found" }, 404);
      const stub = env.SHARES.get(env.SHARES.idFromName(code));
      if (parts[3] === "collab") return stub.fetch("https://share/collab", { method: "POST", body: JSON.stringify({ key: url.searchParams.get("key") || "", stop: url.searchParams.get("stop") === "1" }) });
      const r = await readJson(req);
      if (r.error) return r.error;
      const op = r.body || {};
      const track = op.op === "add" ? shareClean({ tracks: [op.track || {}] }).tracks[0] : null;
      return stub.fetch("https://share/edit", { method: "POST", body: JSON.stringify({ k: url.searchParams.get("k") || "", op: op.op, id: String(op.id || "").slice(0, 200), track }) });
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
      // (a collaborator's invite carries its edit key on to the app)
      const e = (url.searchParams.get("e") || "").replace(/[^a-z0-9]/g, "").slice(0, 40);
      const appLink = (env.APP_SCHEME || "omnipulse") + "://playlist/" + code + "?relay=" + encodeURIComponent(url.origin) + (e ? "&e=" + e : "");
      return new Response(sharePage(env, share, appLink, !!e), { headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store" } });
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
      const admitted = (Array.isArray(body.admit) ? body.admit : []).filter((c) => typeof c === "string" && /^[a-z0-9]{8,40}$/.test(c)).slice(0, 5);
      await stub.fetch("https://room/init", { method: "POST", body: JSON.stringify({ kind, host: String(body.host || "Someone").slice(0, 40), hostKey, inviteKey, admitted, declined: [] }) });
      const base = url.origin;
      return json({ code, hostKey, inviteKey, joinUrl: kind === "jam" ? `${base}/jam/${code}?k=${inviteKey}` : `${base}/join/${code}` });
    }

    if (!parts.length) return Response.redirect(env.DOWNLOAD_URL || "https://github.com", 302);
    const code = (parts[0] === "api" ? parts[2] : parts[1]) || "";
    if (!/^[a-z0-9]{8}$/.test(code)) return json({ error: "not found" }, 404);
    const stub = env.ROOMS.get(env.ROOMS.idFromName(code));

    if (parts[0] === "api" && parts[1] === "rooms" && req.method === "GET") return stub.fetch("https://room/info");
    if (parts[0] === "join" || parts[0] === "jam" || parts[0] === "ask") {
      const info = await (await stub.fetch("https://room/info")).json().catch(() => null);
      const ok = info && !info.error && (parts[0] === "jam" ? info.kind === "jam" : parts[0] === "ask" || info.kind === "listen");
      const scheme = env.APP_SCHEME || "omnipulse";
      const appLink = `${scheme}://${parts[0]}/${code}` + (parts[0] === "jam" ? "?k=" + encodeURIComponent(url.searchParams.get("k") || "") + "&relay=" + encodeURIComponent(url.origin) : "?relay=" + encodeURIComponent(url.origin));
      return new Response(joinPage(env, ok ? info : null, appLink, parts[0] === "ask"), { headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store" } });
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
    const name = String(url.searchParams.get("name") || (role === "asker" ? "Someone" : "A listener")).slice(0, 40);
    const cid = String(url.searchParams.get("cid") || "");
    const ip = req.headers.get("CF-Connecting-IP") || "";
    const hosted = this.ctx.getWebSockets("host").length > 0;
    // someone wanting in (asking, or a Jam listener not let in yet): told to wait, the host asked
    const wait = (kind) => {
      if (!/^[a-z0-9]{8,40}$/.test(cid)) return json({ error: "this needs an up-to-date OmniPulse" }, 400);
      if ((meta.declined || []).includes(cid)) return json({ error: "declined" }, 403);
      const waiting = this.ctx.getWebSockets("asker").concat(this.ctx.getWebSockets("pending")).map((w) => w.deserializeAttachment() || {});
      if (waiting.some((w) => w.cid === cid)) return json({ error: "a request is already waiting" }, 409);
      if (ip && waiting.filter((w) => w.ip === ip).length >= MAX_WAITING_PER_IP) return json({ error: "too many requests from here" }, 429);
      if (waiting.length >= MAX_WAITING) return json({ error: "they have too many requests waiting" }, 429);
      const pair = new WebSocketPair();
      const id = randomCode(10);
      this.ctx.acceptWebSocket(pair[1], [kind]);
      pair[1].serializeAttachment({ role: kind, name, id, cid, ip });
      const msg = JSON.stringify({ type: meta.kind === "jam" ? "joinAsk" : "ask", id, name, cid });
      this.ctx.getWebSockets("host").forEach((w) => { try { w.send(msg); } catch (e) {} });
      if (kind === "pending") pair[1].send(JSON.stringify({ type: "waiting", host: meta.host }));
      return new Response(null, { status: 101, webSocket: pair[0] });
    };
    if (role === "asker") {
      if (!hosted) return json({ error: "they're not here right now" }, 409);
      return wait("asker");
    }
    if (role === "host") {
      if (url.searchParams.get("key") !== meta.hostKey) return json({ error: "not the host" }, 403);
      this.ctx.getWebSockets("host").forEach((w) => { try { w.close(4000, "the host connected again"); } catch (e) {} });
      // (back after being away: everyone's told)
      if (await this.ctx.storage.get("hostGone")) {
        await this.ctx.storage.delete("hostGone");
        const back = JSON.stringify({ type: "hostBack" });
        this.listeners().forEach((w) => { try { w.send(back); } catch (e) {} });
      }
    } else {
      if (meta.kind === "jam" && url.searchParams.get("k") !== meta.inviteKey) return json({ error: "this Jam needs an invite" }, 403);
      if (this.listeners().length >= MAX_LISTENERS) return json({ error: "this session is full" }, 429);
      // a Jam: the invite is the host's say-so (only someone they've turned away is kept out)
      if (meta.kind === "jam" && (meta.declined || []).includes(cid)) return json({ error: "declined" }, 403);
    }
    const pair = new WebSocketPair();
    const id = randomCode(10);
    this.ctx.acceptWebSocket(pair[1], [role === "host" ? "host" : "listener"]);
    pair[1].serializeAttachment({ role: role === "host" ? "host" : "listener", name, id, cid });
    if (role !== "host") {
      const state = await this.ctx.storage.get("state");
      pair[1].send(JSON.stringify({ type: "hello", kind: meta.kind, host: meta.host, canAdd: meta.kind === "jam", you: id }));
      if (state) pair[1].send(JSON.stringify({ type: "state", state }));
    }
    // (the host back: anyone still waiting is asked about again)
    if (role === "host") {
      this.ctx.getWebSockets("asker").concat(this.ctx.getWebSockets("pending")).forEach((w) => {
        const x = w.deserializeAttachment() || {};
        try { pair[1].send(JSON.stringify({ type: meta.kind === "jam" ? "joinAsk" : "ask", id: x.id, name: x.name, cid: x.cid })); } catch (e) {}
      });
    }
    await this.touch();
    this.sendMembers();
    return new Response(null, { status: 101, webSocket: pair[0] });
  }

  listeners() {
    return this.ctx.getWebSockets("listener");
  }

  // (gone: a socket that's closing is still listed while its close is handled, so it's left out)
  sendMembers(gone) {
    const list = this.listeners().filter((w) => w !== gone).map((w) => (w.deserializeAttachment() || {}).name || "A listener");
    const msg = JSON.stringify({ type: "members", list });
    this.ctx.getWebSockets().forEach((w) => { if (w !== gone) { try { w.send(msg); } catch (e) {} } });
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
      } else if ((m.type === "askReply" || m.type === "admit") && typeof m.id === "string") {
        // the host's answer, to whoever asked (a decline is kept: they can't ask again here; "busy"
        // isn't a decline, only a "not right now")
        const meta = await this.meta();
        if (!meta) return;
        const url = typeof m.url === "string" ? m.url.slice(0, 300) : "";
        this.ctx.getWebSockets("asker").concat(this.ctx.getWebSockets("pending")).forEach((w) => {
          const x = w.deserializeAttachment() || {};
          if (x.id !== m.id) return;
          if (m.ok && x.cid && !(meta.admitted || []).includes(x.cid)) meta.admitted = (meta.admitted || []).concat(x.cid).slice(-200);
          if (!m.ok && !m.busy && x.cid && !(meta.declined || []).includes(x.cid)) meta.declined = (meta.declined || []).concat(x.cid).slice(-500);
          const msg = m.type === "askReply" && x.role === "asker" && meta.kind === "listen"
            ? { type: "askReply", ok: !!m.ok, url, host: meta.host || "", busy: !!m.busy }
            : m.ok ? { type: "admitted", url, host: meta.host || "" } : { type: "declined", host: meta.host || "" };
          try { w.send(JSON.stringify(msg)); w.close(1000, "answered"); } catch (e) {}
        });
        await this.ctx.storage.put("meta", meta);
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
      const msg = JSON.stringify({ type: "add", track: m.track, next: !!m.next, by: who.name || "A listener" });
      this.ctx.getWebSockets("host").forEach((w) => { try { w.send(msg); } catch (e) {} });
    }
    // ...or moving one in the queue (its id, to its new place after what's playing)
    if (m.type === "move" && meta && meta.kind === "jam" && typeof m.id === "string") {
      const msg = JSON.stringify({ type: "move", id: m.id.slice(0, 200), to: Math.max(1, Math.min(200, Number(m.to) || 1)), by: who.name || "A listener" });
      this.ctx.getWebSockets("host").forEach((w) => { try { w.send(msg); } catch (e) {} });
    }
    // ...or skipping (back or on): the host's player does it, for everyone
    if (m.type === "skip" && meta && meta.kind === "jam") {
      const msg = JSON.stringify({ type: "skip", d: m.d === -1 ? -1 : 1, by: who.name || "A listener" });
      this.ctx.getWebSockets("host").forEach((w) => { try { w.send(msg); } catch (e) {} });
    }
  }

  async webSocketClose(ws) {
    try { ws.close(); } catch (e) {}
    const who = ws.deserializeAttachment() || {};
    // the host gone, and not back on another connection: everyone's told, and if they're not back
    // within a minute and a half, it's over (their app closed for an update comes back sooner)
    if (who.role === "host" && !this.ctx.getWebSockets("host").some((w) => w !== ws)) {
      const meta = await this.meta();
      if (meta) {
        await this.ctx.storage.put("hostGone", Date.now());
        const away = JSON.stringify({ type: "hostAway" });
        this.listeners().forEach((w) => { try { w.send(away); } catch (e) {} });
        await this.ctx.storage.setAlarm(Date.now() + HOST_GRACE_MS);
      }
    }
    // someone who asked (or was waiting to be let in), gone before an answer: the host's prompt goes too
    if (who.role === "asker" || who.role === "pending") {
      const msg = JSON.stringify({ type: "askGone", id: who.id });
      this.ctx.getWebSockets("host").forEach((w) => { try { w.send(msg); } catch (e) {} });
      return;
    }
    this.sendMembers(ws);
  }

  async webSocketError(ws) {
    this.sendMembers(ws);
  }

  // nobody's used it in a while: it goes (or its host has been gone too long)
  async alarm() {
    const gone = await this.ctx.storage.get("hostGone");
    if (gone && !this.ctx.getWebSockets("host").length) {
      if (Date.now() - gone >= HOST_GRACE_MS) {
        const msg = JSON.stringify({ type: "ended", why: "hostGone" });
        this.ctx.getWebSockets().forEach((w) => { try { w.send(msg); w.close(1000, "ended"); } catch (e) {} });
        await this.ctx.storage.deleteAll();
        return;
      }
      await this.ctx.storage.setAlarm(gone + HOST_GRACE_MS);
      return;
    }
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
      await this.ctx.storage.put("meta", { key: meta ? meta.key : m.key, editKey: meta ? meta.editKey : undefined, used: Date.now() });
      const old = await this.ctx.storage.get("share");
      await this.ctx.storage.put("share", Object.assign(m.share, { rev: ((old && old.rev) || 0) + 1, collab: !!(meta && meta.editKey) }));
      await this.ctx.storage.setAlarm(Date.now() + SHARE_IDLE_MS);
      return json({ ok: true });
    }
    if (!meta) return json({ error: "this playlist isn't shared any more" }, 404);
    if (url.pathname === "/collab") {
      const m = await req.json();
      if (m.key !== meta.key) return json({ error: "not yours to change" }, 403);
      // the owner stopping it: the edit key goes (collaborators keep their copies, no longer joined)
      if (m.stop) {
        delete meta.editKey;
        const share = await this.ctx.storage.get("share");
        if (share) { share.collab = false; share.rev = (share.rev || 0) + 1; await this.ctx.storage.put("share", share); }
        await this.ctx.storage.put("meta", meta);
        return json({ ok: true });
      }
      if (!meta.editKey) {
        meta.editKey = randomCode(24);
        const share = await this.ctx.storage.get("share");
        if (share) { share.collab = true; await this.ctx.storage.put("share", share); }
      }
      meta.used = Date.now();
      await this.ctx.storage.put("meta", meta);
      return json({ editKey: meta.editKey });
    }
    if (url.pathname === "/edit") {
      const m = await req.json();
      if (!m.k || (m.k !== meta.key && m.k !== meta.editKey)) return json({ error: "not yours to change" }, 403);
      const share = (await this.ctx.storage.get("share")) || { title: "Playlist", tracks: [] };
      const idOf = (t) => (t && t.snap && t.snap.id != null ? String(t.snap.id) : "");
      if (m.op === "add" && m.track) {
        if (!share.tracks.some((t) => idOf(t) && idOf(t) === idOf(m.track)) && share.tracks.length < MAX_SHARE_TRACKS) share.tracks.push(m.track);
      } else if (m.op === "remove" && m.id) {
        share.tracks = share.tracks.filter((t) => idOf(t) !== m.id);
      } else return json({ error: "unknown edit" }, 400);
      share.rev = (share.rev || 0) + 1;
      await this.ctx.storage.put("share", share);
      meta.used = Date.now();
      await this.ctx.storage.put("meta", meta);
      await this.ctx.storage.setAlarm(Date.now() + SHARE_IDLE_MS);
      return json({ rev: share.rev });
    }
    meta.used = Date.now();
    await this.ctx.storage.put("meta", meta);
    await this.ctx.storage.setAlarm(Date.now() + SHARE_IDLE_MS);
    return json(await this.ctx.storage.get("share"));
  }

  async alarm() {
    const meta = await this.ctx.storage.get("meta");
    // a collaborative one stays for as long as it's collaborative (only stopping it lets it go)
    if (meta && meta.editKey) {
      await this.ctx.storage.setAlarm(Date.now() + SHARE_IDLE_MS);
      return;
    }
    if (meta && Date.now() - (meta.used || 0) < SHARE_IDLE_MS) {
      await this.ctx.storage.setAlarm((meta.used || Date.now()) + SHARE_IDLE_MS);
      return;
    }
    await this.ctx.storage.deleteAll();
  }
}
