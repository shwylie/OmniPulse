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
// Settings (wrangler.toml [vars], or the Cloudflare dashboard): DOWNLOAD_URL, where "Download OmniPulse"
// points; APP_SCHEME, the app's link scheme ("omnipulse").

const ROOM_IDLE_MS = 12 * 3600 * 1000;   // a room nobody has used for this long goes away
const MAX_LISTENERS = 50;
const MAX_MESSAGE = 64 * 1024;

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

export default {
  async fetch(req, env) {
    const url = new URL(req.url);
    const parts = url.pathname.split("/").filter(Boolean);
    if (req.method === "OPTIONS") return new Response(null, { headers: { "access-control-allow-origin": "*", "access-control-allow-methods": "GET, POST", "access-control-allow-headers": "content-type" } });

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
