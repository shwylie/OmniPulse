// OmniPulse Mod View: for moderators. A channel you moderate, watched one at a time (on Twitch, Kick, or both
// when the streamer multi-streams, as one chat): its chat (Twitch's IRC, Kick's live chat), with what you do there
// done as you (time-outs, bans, deleted messages, what you say), its chat modes and Shield Mode (Twitch), and the messages Twitch's AutoMod holds back, for you to allow or deny.
// Its sign-ins are the Stream Suite's (one sign-in for both: the app keeps them as one, and tells each when the
// other signs in, out, or renews). Signing in here asks for everything the Stream Suite needs too. Everything
// goes to the app for its Mod View page (/mod): the chat (modChat, modGone, modClear) and the state (modState).

const S = () => OmniPulse.settings;
const yes = (v) => v === true || v === "true";
const num = (v) => { const n = Number(v); return isFinite(n) ? n : 0; };
// (the Stream Suite's own, as the sign-in is shared: kept the same as plugins/chatbot's)
const TWITCH_SCOPES = [
  "chat:read", "chat:edit", "user:read:chat", "moderator:read:followers", "channel:read:subscriptions", "bits:read",
  "channel:read:hype_train", "channel:read:polls", "moderator:read:shoutouts", "channel:read:redemptions",
  "moderator:manage:banned_users", "moderator:manage:shoutouts", "moderator:manage:chat_messages",
  "moderator:manage:announcements", "channel:manage:broadcast", "clips:edit", "channel:read:ads",
  "channel:read:predictions", "channel:read:vips", "moderation:read", "channel:read:charity",
  "channel:manage:polls", "channel:manage:predictions", "channel:manage:redemptions", "channel:edit:commercial",
  "channel:manage:raids", "moderator:manage:chat_settings", "channel:manage:vips", "channel:manage:moderators",
  "moderator:manage:shield_mode", "moderator:read:shield_mode", "user:read:emotes", "moderator:read:chatters",
  "user:read:moderated_channels", "moderator:manage:automod",
].join(" ");
const KICK_SCOPES = "user:read channel:read channel:write chat:write moderation:ban moderation:chat_message:manage channel:rewards:read channel:rewards:write events:subscribe";
const REDIRECT_PORT = 8476;
const TWITCH_REDIRECT = "http://localhost:" + REDIRECT_PORT + "/twitch";
const KICK_REDIRECT = "http://localhost:" + REDIRECT_PORT + "/kick";

// SHA-256 (for Kick's sign-in: the script has no crypto of its own), of bytes, as bytes
function sha256(bytes) {
  const K = [], H = [];
  let n = 2, found = 0;
  const frac = (x) => ((x - Math.floor(x)) * 4294967296) >>> 0;
  while (found < 64) {
    let prime = true;
    for (let d = 2; d * d <= n; d++) if (n % d === 0) { prime = false; break; }
    if (prime) { if (found < 8) H[found] = frac(Math.pow(n, 1 / 2)); K[found] = frac(Math.pow(n, 1 / 3)); found++; }
    n++;
  }
  const m = bytes.slice();
  const bits = bytes.length * 8;
  m.push(0x80);
  while (m.length % 64 !== 56) m.push(0);
  m.push(0, 0, 0, 0, (bits >>> 24) & 255, (bits >>> 16) & 255, (bits >>> 8) & 255, bits & 255);
  const rotr = (x, k) => (x >>> k) | (x << (32 - k));
  for (let i = 0; i < m.length; i += 64) {
    const w = [];
    for (let j = 0; j < 16; j++) w[j] = (m[i + j * 4] << 24) | (m[i + j * 4 + 1] << 16) | (m[i + j * 4 + 2] << 8) | m[i + j * 4 + 3];
    for (let j = 16; j < 64; j++) {
      const s0 = rotr(w[j - 15], 7) ^ rotr(w[j - 15], 18) ^ (w[j - 15] >>> 3);
      const s1 = rotr(w[j - 2], 17) ^ rotr(w[j - 2], 19) ^ (w[j - 2] >>> 10);
      w[j] = (w[j - 16] + s0 + w[j - 7] + s1) | 0;
    }
    let [a, b, c, d, e, f, g, h] = H;
    for (let j = 0; j < 64; j++) {
      const t1 = (h + (rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)) + ((e & f) ^ (~e & g)) + K[j] + w[j]) | 0;
      const t2 = ((rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)) + ((a & b) ^ (a & c) ^ (b & c))) | 0;
      h = g; g = f; f = e; e = (d + t1) | 0; d = c; c = b; b = a; a = (t1 + t2) | 0;
    }
    H[0] = (H[0] + a) | 0; H[1] = (H[1] + b) | 0; H[2] = (H[2] + c) | 0; H[3] = (H[3] + d) | 0;
    H[4] = (H[4] + e) | 0; H[5] = (H[5] + f) | 0; H[6] = (H[6] + g) | 0; H[7] = (H[7] + h) | 0;
  }
  const out = [];
  H.forEach((x) => out.push((x >>> 24) & 255, (x >>> 16) & 255, (x >>> 8) & 255, x & 255));
  return out;
}

function utf8(t) {
  const out = [];
  for (const ch of String(t)) {
    const c = ch.codePointAt(0);
    if (c < 0x80) out.push(c);
    else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
    else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
  }
  return out;
}

function b64(bytes) {
  const A = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  let out = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const n = (bytes[i] << 16) | ((bytes[i + 1] || 0) << 8) | (bytes[i + 2] || 0);
    out += A[(n >> 18) & 63] + A[(n >> 12) & 63] + (i + 1 < bytes.length ? A[(n >> 6) & 63] : "=") + (i + 2 < bytes.length ? A[n & 63] : "=");
  }
  return out;
}

function randomText(n) {
  const A = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
  let out = "";
  for (let i = 0; i < n; i++) out += A[Math.floor(Math.random() * A.length)];
  return out;
}
function b64url(bytes) { return b64(bytes).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""); }

async function form(url, fields) {
  const body = Object.entries(fields).map(([k, v]) => encodeURIComponent(k) + "=" + encodeURIComponent(v)).join("&");
  const r = await OmniPulse.fetch(url, { method: "POST", headers: { "Content-Type": "application/x-www-form-urlencoded" }, body });
  let j = {};
  try { j = await r.json(); } catch (e) {}
  return { ok: r.ok, status: r.status, j };
}

// A sign-in (or renewal) finished by OmniPulse's relay, which keeps its apps' secrets
async function viaRelay(relay, platform, what, body) {
  const r = await OmniPulse.fetch(relay.replace(/\/$/, "") + "/api/" + platform + "/" + what, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  let j = {};
  try { j = await r.json(); } catch (e) {}
  return { ok: r.ok, status: r.status, j };
}

async function keep(values) {
  Object.assign(OmniPulse.settings, values);
  await OmniPulse.app.saveSettings(values);
}

// What the page should say, in its chat
function notice(text) { OmniPulse.app.modChat({ notice: String(text).slice(0, 300) }).catch(() => {}); }

// ---------- signing in: the page's Sign In, the code given back at once and the rest waited for ----------
function signIn(platform, values) {
  let said = null;
  const first = new Promise((ok) => { said = ok; });
  const flow = platform === "kick" ? kickSignIn(values, said) : twitchSignIn(values, said);
  flow.then((msg) => { if (msg) { said(msg); notice(msg); push(); } }, (e) => { const m = "Couldn't sign in: " + e.message; said(m); notice(m); });
  return first;
}

async function twitchSignIn(values, said) {
  const clientId = String((values && values.twitchClientId) || S().twitchClientId || "").trim();
  const relay = String((values && values.twitchRelay) || S().twitchRelay || "").trim();
  if (!clientId || !relay) return "OmniPulse's Twitch sign-in isn't set up in this version.";
  const state = randomText(24);
  const q = { client_id: clientId, redirect_uri: TWITCH_REDIRECT, response_type: "code", scope: TWITCH_SCOPES, state };
  const waiting = OmniPulse.app.awaitRedirect(REDIRECT_PORT, "/twitch");
  OmniPulse.app.openUrl("https://id.twitch.tv/oauth2/authorize?" + Object.entries(q).map(([k, x]) => k + "=" + encodeURIComponent(x)).join("&"));
  said("Twitch is open in your browser. Sign in with the account you moderate with, and click Authorize.");
  const back = await waiting;
  const got = query(back.query);
  if (got.error) return "Twitch's sign-in didn't finish: " + (got.error_description || got.error);
  if (got.state !== state || !got.code) return "Twitch's sign-in didn't come back as it should. Try again.";
  const t = await viaRelay(relay, "twitch", "token", { code: got.code, redirect_uri: TWITCH_REDIRECT });
  if (!t.ok || !t.j.access_token) return "Twitch didn't take the sign-in: " + (t.j.message || t.j.error || t.status) + ". Try again in a moment.";
  const me = await validate(t.j.access_token);
  if (!me) return "Signed in, but Twitch didn't say who as. Try again.";
  await keep({ twitchClientId: clientId, twitchRelay: relay, token: t.j.access_token, refresh: t.j.refresh_token || "", login: me.login, userId: me.user_id });
  await channelsRead();
  return "Signed in to Twitch as " + me.login + ".";
}

function query(q) {
  const got = {};
  String(q || "").split("&").forEach((kv) => { const i = kv.indexOf("="); if (i > 0) got[kv.slice(0, i)] = decodeURIComponent(kv.slice(i + 1).replace(/\+/g, " ")); });
  return got;
}

async function validate(token) {
  const r = await OmniPulse.fetch("https://id.twitch.tv/oauth2/validate", { headers: { Authorization: "OAuth " + token } });
  return r.ok ? r.json() : null;
}

async function renew() {
  const s = S();
  if (!s.refresh || !s.twitchRelay) return false;
  const t = await viaRelay(s.twitchRelay, "twitch", "refresh", { refresh_token: s.refresh });
  if (!t.ok || !t.j.access_token) return false;
  await keep({ token: t.j.access_token, refresh: t.j.refresh_token || s.refresh });
  return true;
}

async function kickSignIn(values, said) {
  const v = Object.assign({}, S(), values || {});
  const id = String(v.kickClientId || "").trim(), relay = String(v.kickRelay || "").trim();
  if (!id || !relay) return "OmniPulse's Kick sign-in isn't set up in this version.";
  const verifier = randomText(64), state = randomText(24);
  const q = { client_id: id, response_type: "code", redirect_uri: KICK_REDIRECT, scope: KICK_SCOPES, state, code_challenge: b64url(sha256(utf8(verifier))), code_challenge_method: "S256" };
  const waiting = OmniPulse.app.awaitRedirect(REDIRECT_PORT, "/kick");
  OmniPulse.app.openUrl("https://id.kick.com/oauth/authorize?" + Object.entries(q).map(([k, x]) => k + "=" + encodeURIComponent(x)).join("&"));
  said("Kick is open in your browser. Sign in with the account you moderate with, and click Allow.");
  const back = await waiting;
  const got = query(back.query);
  if (got.error) return "Kick's sign-in didn't finish: " + (got.error_description || got.error);
  if (got.state !== state || !got.code) return "Kick's sign-in didn't come back as it should. Try again.";
  const t = await viaRelay(relay, "kick", "token", { code: got.code, code_verifier: verifier, redirect_uri: KICK_REDIRECT });
  if (!t.ok || !t.j.access_token) return "Kick didn't take the sign-in: " + (t.j.error_description || t.j.message || t.j.error || t.status) + ". Try again in a moment.";
  Object.assign(OmniPulse.settings, { kickClientId: id, kickRelay: relay, kickToken: t.j.access_token, kickRefreshToken: t.j.refresh_token || "" });
  const me = await kickApi("GET", "users");
  const u = me.data && me.data[0];
  if (!u) return "Signed in, but Kick didn't say who as. Try again.";
  await keep({ kickClientId: id, kickRelay: relay, kickToken: t.j.access_token, kickRefreshToken: t.j.refresh_token || "", kickLogin: String(u.name || ""), kickUserId: String(u.user_id || "") });
  return "Signed in to Kick as " + (u.name || "you") + ".";
}

async function kickRenew() {
  const s = S();
  if (!s.kickRefreshToken || !s.kickRelay) return false;
  const t = await viaRelay(s.kickRelay, "kick", "refresh", { refresh_token: s.kickRefreshToken });
  if (!t.ok || !t.j.access_token) return false;
  await keep({ kickToken: t.j.access_token, kickRefreshToken: t.j.refresh_token || s.kickRefreshToken });
  return true;
}

// ---------- the platforms' APIs, as you ----------
async function helix(method, path, body, again) {
  if (!S().token) throw new Error("sign in to Twitch first (Channel, in Mod View)");
  const r = await OmniPulse.fetch("https://api.twitch.tv/helix/" + path, {
    method,
    headers: { "Client-Id": S().twitchClientId, Authorization: "Bearer " + S().token, "Content-Type": "application/json" },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (r.status === 401 && !again && (await renew())) return helix(method, path, body, true);
  if (!r.ok) {
    let why = "";
    try { why = (await r.json()).message || ""; } catch (e) {}
    throw new Error("Twitch said " + r.status + (why ? ": " + why : "") + (r.status === 403 ? " (are you a moderator there?)" : r.status === 401 ? " (sign in to Twitch again)" : ""));
  }
  if (r.status === 204) return {};
  try { return await r.json(); } catch (e) { return {}; }
}

async function kickApi(method, path, body, again) {
  if (!S().kickToken) throw new Error("sign in to Kick first (Channel, in Mod View)");
  const r = await OmniPulse.fetch("https://api.kick.com/public/v1/" + path, {
    method,
    headers: { Authorization: "Bearer " + S().kickToken, "Content-Type": "application/json", Accept: "application/json" },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (r.status === 401 && !again && (await kickRenew())) return kickApi(method, path, body, true);
  if (!r.ok) {
    let why = "";
    try { why = (await r.json()).message || ""; } catch (e) {}
    throw new Error("Kick said " + r.status + (why ? ": " + why : "") + (r.status === 403 ? " (are you a moderator there?)" : r.status === 401 ? " (sign in to Kick again)" : ""));
  }
  if (r.status === 204) return {};
  try { return await r.json(); } catch (e) { return {}; }
}

// ---------- what the page shows ----------
// Watching: the streamer, on one platform or both (multi-streaming): each platform's channel, whether it's live,
// whether its chat's connected, and whether you're a moderator there (null: not known yet)
let channels = [], watching = null, held = [], automodNote = "", note = "";
let modes = { chat: {}, shield: false, lock: null };
const PNAME = { twitch: "Twitch", kick: "Kick" };
let pushWait = null;
function push() {
  if (pushWait) return;
  pushWait = setTimeout(() => {
    pushWait = null;
    const tw = watching && watching.twitch, kk = watching && watching.kick;
    OmniPulse.app.modState({
      twitch: { login: S().login || "", name: S().login || "" },
      kick: { login: S().kickLogin || "" },
      channels, recent: (S().recent || []).slice(0, 8), watching,
      up: !!((tw && tw.up) || (kk && kk.up)),
      // (Twitch's chat modes; you a moderator on each, as far as it's known)
      modes: Object.assign({}, modes, { twitch: !!(tw && S().token && tw.mod !== false) }),
      can: { twitch: tw ? tw.mod : null, kick: kk ? kk.mod : null },
      automod: held, automodNote, note,
    }).catch(() => {});
  }, 150);
}

// The channels you moderate on Twitch (yours first)
async function channelsRead() {
  if (!S().token || !S().userId) { channels = []; push(); return; }
  const out = [{ platform: "twitch", login: String(S().login || "").toLowerCase(), name: S().login, id: String(S().userId), own: true }];
  try {
    let after = "";
    for (let page = 0; page < 10; page++) {
      const j = await helix("GET", "moderation/channels?user_id=" + S().userId + "&first=100" + (after ? "&after=" + encodeURIComponent(after) : ""));
      for (const c of j.data || []) out.push({ platform: "twitch", login: String(c.broadcaster_login || "").toLowerCase(), name: c.broadcaster_name || c.broadcaster_login, id: String(c.broadcaster_id) });
      after = (j.pagination && j.pagination.cursor) || "";
      if (!after) break;
    }
    note = "";
  } catch (e) {
    note = /403|401/.test(e.message) ? "Sign in to Twitch again so Mod View can list the channels you moderate." : "";
  }
  channels = out;
  // (the one being watched: a moderator there, by this list)
  const tw = watching && watching.twitch;
  if (tw && tw.mod == null && channels.some((c) => c.login === tw.login)) { tw.mod = true; }
  push();
}

// You turn out not to be a moderator on a platform: said once, its messages' buttons greyed out (the page does)
function notMod(platform) {
  const p = watching && watching[platform];
  if (!p || p.mod === false) return;
  p.mod = false;
  notice("You're not a moderator in " + p.name + "'s " + PNAME[platform] + " chat, so the mod buttons on " + PNAME[platform] + " messages are off. You can still read and chat there.");
  push();
}
function isMod(platform) {
  const p = watching && watching[platform];
  if (p && p.mod === true) return;
  if (p) p.mod = true;
  push();
}

// The emotes you can use on Twitch, for the chat's emote menu and for showing them (the Stream Suite gives its own
// list while it's running; this is for when only Mod View is)
async function emotesRead() {
  if (!S().token || !S().userId) return;
  const out = [];
  try {
    let after = "";
    for (let i = 0; i < 20; i++) {
      const r = await helix("GET", "chat/emotes/user?user_id=" + encodeURIComponent(S().userId) + (after ? "&after=" + encodeURIComponent(after) : ""));
      for (const e of r.data || []) out.push({ c: e.name, u: "https://static-cdn.jtvnw.net/emoticons/v2/" + e.id + "/default/dark/2.0", f: "twitch", g: e.emote_type === "globals" || e.owner_id === "0" ? "Twitch" : "Twitch: Channels" });
      after = (r.pagination && r.pagination.cursor) || "";
      if (!after) break;
    }
  } catch (e) {}
  if (out.length) OmniPulse.app.dockEmotes(out).catch(() => {});
}

// ---------- watching ----------
const conn = {
  twitch: { gen: 0, irc: null, reader: null, events: null, timers: [], retry: 2000 },
  kick: { gen: 0, sock: null, timers: [], retry: 2000, refresh: null },
};

function stopPlatform(platform) {
  const c = conn[platform];
  c.gen++;
  for (const k of ["irc", "reader", "events", "sock"]) if (c[k]) { c[k].onclose = null; c[k].close(); c[k] = null; }
  c.timers.forEach((t) => clearInterval(t));
  c.timers = [];
  if (c.refresh) { clearInterval(c.refresh); c.refresh = null; }
  if (platform === "twitch") { held = []; automodNote = ""; modes = { chat: {}, shield: false, lock: null }; }
  if (watching) delete watching[platform];
}
function stopWatching() { stopPlatform("twitch"); stopPlatform("kick"); watching = null; }

// A channel watched: on its own (also: false), or as the streamer's other platform (also: true), when they're
// multi-streaming. One platform's channel at a time; the other platform's kept.
async function watch(platform, channel, also) {
  const login = String(channel || "").trim().toLowerCase().replace(/^@/, "").replace(/^.*\//, "");
  if (!login || !/^[a-z0-9_-]{2,40}$/.test(login)) throw new Error("that isn't a channel's name");
  if (platform === "twitch" && !S().token) throw new Error("sign in to Twitch first");
  if (platform === "kick" && !S().kickToken) throw new Error("sign in to Kick first");
  if (!also) { stopWatching(); OmniPulse.app.modClear().catch(() => {}); }
  else stopPlatform(platform);
  watching = watching || { name: login };
  const p = { login, id: "", name: login, live: false, title: "", viewers: null, up: false, mod: null };
  watching[platform] = p;
  if (!also) watching.name = login;
  push();
  const my = conn[platform].gen;
  try {
    if (platform === "twitch") await twitchWatch(p, my);
    else await kickWatch(p, my);
  } catch (e) {
    if (conn[platform].gen === my) { delete watching[platform]; if (!watching.twitch && !watching.kick) watching = null; push(); }
    throw e;
  }
  if (!also) watching.name = p.name;
  const pair = { twitch: watching.twitch ? watching.twitch.login : "", kick: watching.kick ? watching.kick.login : "" };
  const name = watching.name;
  const recent = [{ name, twitch: pair.twitch, kick: pair.kick }].concat((S().recent || []).filter((r) => !(r.twitch === pair.twitch && r.kick === pair.kick))).slice(0, 8);
  keep({ recent, watched: pair }).catch(() => {});
  push();
  return "Watching " + p.name + " on " + PNAME[platform] + ".";
}

// ---------- Twitch: its chat over IRC, its modes, and AutoMod over EventSub ----------
let badges = {};
async function twitchWatch(p, my) {
  const u = ((await helix("GET", "users?login=" + encodeURIComponent(p.login))).data || [])[0];
  if (!u) throw new Error("Twitch has no channel called " + p.login);
  if (my !== conn.twitch.gen) return;
  p.id = String(u.id);
  p.name = u.display_name || p.login;
  // (a moderator there: your own channel, or one Twitch lists as yours to moderate; else known once you're in its chat)
  if (p.id === String(S().userId) || channels.some((c) => c.platform === "twitch" && c.id === p.id)) p.mod = true;
  badgesRead(u.id);
  ircConnect(p, my);
  readerConnect(p, my);
  eventsConnect(p, my);
  liveRead(p);
  modesRead();
  conn.twitch.timers.push(setInterval(() => liveRead(p), 60000), setInterval(modesRead, 30000));
}

async function badgesRead(id) {
  const into = {};
  const take = (r) => ((r && r.data) || []).forEach((s) => (s.versions || []).forEach((v) => { into[s.set_id + "/" + v.id] = { url: v.image_url_2x || v.image_url_1x || "", title: v.title || s.set_id }; }));
  try { take(await helix("GET", "chat/badges/global")); } catch (e) {}
  try { take(await helix("GET", "chat/badges?broadcaster_id=" + encodeURIComponent(id))); } catch (e) {}
  badges = into;
}

async function liveRead(p) {
  try {
    const s = ((await helix("GET", "streams?user_id=" + p.id)).data || [])[0];
    p.live = !!s;
    p.title = s ? s.title || "" : "";
    p.viewers = s ? num(s.viewer_count) : null;
    push();
  } catch (e) {}
}

function twitchMod() { return "broadcaster_id=" + watching.twitch.id + "&moderator_id=" + S().userId; }

async function modesRead() {
  const tw = watching && watching.twitch;
  if (!tw || tw.mod === false) return;
  try {
    const c = ((await helix("GET", "chat/settings?" + twitchMod())).data || [])[0] || {};
    modes.chat = { slow: !!c.slow_mode, slowSecs: c.slow_mode_wait_time, follow: !!c.follower_mode, followMins: c.follower_mode_duration, sub: !!c.subscriber_mode, emote: !!c.emote_mode, unique: !!c.unique_chat_mode };
  } catch (e) {}
  try { modes.shield = !!(((await helix("GET", "moderation/shield_mode?" + twitchMod())).data || [])[0] || {}).is_active; } catch (e) { if (/403/.test(e.message)) notMod("twitch"); }
  push();
}

function ircTags(raw) {
  const tags = {};
  raw.split(";").forEach((kv) => { const i = kv.indexOf("="); tags[kv.slice(0, i)] = kv.slice(i + 1).replace(/\\s/g, " ").replace(/\\:/g, ";"); });
  return tags;
}
function parse(line) {
  let tags = {}, rest = line;
  if (rest[0] === "@") { const i = rest.indexOf(" "); tags = ircTags(rest.slice(1, i)); rest = rest.slice(i + 1); }
  let prefix = "";
  if (rest[0] === ":") { const i = rest.indexOf(" "); prefix = rest.slice(1, i); rest = rest.slice(i + 1); }
  const t = rest.indexOf(" :");
  const head = (t >= 0 ? rest.slice(0, t) : rest).split(" ");
  return { tags, nick: prefix.split("!")[0], command: head[0], channel: head[1], text: t >= 0 ? rest.slice(t + 2) : "" };
}

function ircConnect(p, my) {
  const c = conn.twitch;
  const s = OmniPulse.socket("wss://irc-ws.chat.twitch.tv:443");
  c.irc = s;
  s.onopen = () => {
    s.send("CAP REQ :twitch.tv/tags twitch.tv/commands");
    s.send("PASS oauth:" + S().token);
    s.send("NICK " + String(S().login || "").toLowerCase());
    s.send("JOIN #" + p.login);
  };
  s.onmessage = (data) => String(data).split("\r\n").filter(Boolean).forEach((line) => {
    if (line.startsWith("PING")) { s.send("PONG" + line.slice(4)); return; }
    const m = parse(line);
    if (m.command === "JOIN" && m.nick === String(S().login || "").toLowerCase()) { p.up = true; c.retry = 2000; push(); }
    // (your own badges in this chat: a moderator (or the broadcaster) here, or not)
    else if (m.command === "USERSTATE") {
      const b = String(m.tags.badges || "");
      if (m.tags.mod === "1" || /(^|,)(broadcaster|moderator|lead_moderator)\//.test(b)) isMod("twitch"); else notMod("twitch");
    }
    // (what you send that Twitch turns down: said)
    else if (m.command === "NOTICE" && m.text) notice("Twitch: " + m.text);
  });
  s.onclose = () => {
    if (c.irc !== s) return;
    c.irc = null; p.up = false; push();
    if (my === c.gen) { setTimeout(() => { if (my === c.gen && !c.irc) ircConnect(p, my); }, c.retry); c.retry = Math.min(c.retry * 2, 60000); }
  };
}

// The chat itself, read on a connection of its own, signed in as no one: Twitch sends a connection everyone's
// messages but its own, so on this one yours come back too, as they were delivered: with their emotes, their
// badges, and an id to delete them by. (The other connection, signed in as you, sends, and says whether you're a
// moderator.)
function readerConnect(p, my) {
  const c = conn.twitch;
  const s = OmniPulse.socket("wss://irc-ws.chat.twitch.tv:443");
  c.reader = s;
  s.onopen = () => {
    s.send("CAP REQ :twitch.tv/tags twitch.tv/commands");
    s.send("NICK justinfan" + (10000 + Math.floor(Math.random() * 80000)));
    s.send("JOIN #" + p.login);
  };
  s.onmessage = (data) => String(data).split("\r\n").filter(Boolean).forEach((line) => {
    if (line.startsWith("PING")) { s.send("PONG" + line.slice(4)); return; }
    const m = parse(line);
    if (m.command === "PRIVMSG") onTwitch(m);
    else if (m.command === "CLEARMSG") gone("twitch", m.tags["target-msg-id"] || "", "");
    else if (m.command === "CLEARCHAT") gone("twitch", "", m.text || "");
    else if (m.command === "USERNOTICE" && m.tags["system-msg"]) notice("Twitch: " + m.tags["system-msg"] + (m.text ? ": " + m.text : ""));
    else if (m.command === "ROOMSTATE") {
      const t = m.tags, ch = modes.chat;
      if ("slow" in t) { ch.slow = num(t.slow) > 0; if (ch.slow) ch.slowSecs = num(t.slow); }
      if ("followers-only" in t) { ch.follow = t["followers-only"] !== "-1"; if (ch.follow) ch.followMins = num(t["followers-only"]); }
      if ("subs-only" in t) ch.sub = t["subs-only"] === "1";
      if ("emote-only" in t) ch.emote = t["emote-only"] === "1";
      if ("r9k" in t) ch.unique = t.r9k === "1";
      push();
    }
  });
  s.onclose = () => {
    if (c.reader !== s) return;
    c.reader = null;
    if (my === c.gen) setTimeout(() => { if (my === c.gen && !c.reader) readerConnect(p, my); }, c.retry);
  };
}

function onTwitch(m) {
  const login = String(m.nick || "").toLowerCase();
  let text = String(m.text || "");
  const me = /^\u0001ACTION (.*)\u0001?$/.exec(text);
  if (me) text = me[1].replace(/\u0001$/, "");
  const b = String(m.tags.badges || "").split(",").filter(Boolean);
  OmniPulse.app.modChat({
    platform: "twitch", id: m.tags.id || "", user: m.tags["display-name"] || login, login, userId: m.tags["user-id"] || "", color: m.tags.color || "",
    badges: b.map((x) => x.split("/")[0]), badgeImgs: b.map((x) => badges[x]).filter((x) => x && x.url), text, emotes: m.tags.emotes || "",
    first: m.tags["first-msg"] === "1", me: !!me, self: login === String(S().login || "").toLowerCase(),
  }).catch(() => {});
}

function gone(platform, id, login) { OmniPulse.app.modGone({ platform, id, login }).catch(() => {}); }

function eventsConnect(p, my, url) {
  const c = conn.twitch;
  const s = OmniPulse.socket(url || "wss://eventsub.wss.twitch.tv/ws");
  c.events = s;
  s.onmessage = async (data) => {
    let m;
    try { m = JSON.parse(data); } catch (e) { return; }
    const kind = m.metadata && m.metadata.message_type;
    if (kind === "session_welcome" && !url) {
      const session = m.payload.session.id;
      const cond = { broadcaster_user_id: p.id, moderator_user_id: String(S().userId) };
      for (const type of ["automod.message.hold", "automod.message.update"]) {
        try {
          await helix("POST", "eventsub/subscriptions", { type, version: "2", condition: cond, transport: { method: "websocket", session_id: session } });
          automodNote = "";
        } catch (e) {
          if (/409/.test(e.message)) continue;
          automodNote = /403/.test(e.message) ? "AutoMod's held messages are for the channel's moderators. You're not one on Twitch here." : /401/.test(e.message) ? "Sign in to Twitch again so Mod View can see AutoMod's held messages." : "Couldn't follow AutoMod here: " + e.message;
        }
      }
      push();
    } else if (kind === "session_reconnect") {
      const old = s;
      eventsConnect(p, my, m.payload.session.reconnect_url);
      old.onclose = null;
      old.close();
    } else if (kind === "notification") {
      const type = m.payload.subscription.type, e = m.payload.event || {};
      if (type === "automod.message.hold") {
        const text = (e.message && e.message.text) || "";
        const why = e.reason === "blocked_term" ? "A blocked term" : e.automod && e.automod.category ? "AutoMod: " + e.automod.category + (e.automod.level ? " (level " + e.automod.level + ")" : "") : "Held by AutoMod";
        held.push({ id: e.message_id, user: e.user_name || e.user_login, login: e.user_login, userId: e.user_id, text, reason: why, at: Date.now() });
        while (held.length > 50) held.shift();
        push();
      } else if (type === "automod.message.update") {
        held = held.filter((h) => h.id !== e.message_id);
        push();
      }
    }
  };
  s.onclose = () => {
    if (c.events !== s) return;
    c.events = null;
    if (my === c.gen) setTimeout(() => { if (my === c.gen && !c.events) eventsConnect(p, my); }, 10000);
  };
}

// ---------- Kick: its live chat (a guest's connection, as the Stream Suite's) ----------
async function kickPost(path, body) {
  const r = await OmniPulse.fetch("https://web.kick.com/api/v1/realtime/" + path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  if (!r.ok) throw new Error("Kick's chat said " + r.status);
  return (await r.json()).data || {};
}
const kickGet = async (path) => {
  const r = await OmniPulse.fetch("https://kick.com/api/v2/" + path, { headers: { Accept: "application/json", "User-Agent": "OmniPulse" } });
  if (!r.ok) throw new Error("Kick said " + r.status);
  return r.json();
};

let kickSubBadges = [], kickN = 10;
async function kickWatch(p, my) {
  const slug = p.login.replace(/_/g, "-");
  let ch;
  try { ch = await kickGet("channels/" + encodeURIComponent(slug)); } catch (e) { throw new Error("Kick has no channel called " + slug); }
  if (my !== conn.kick.gen) return;
  const room = ch.chatroom && ch.chatroom.id;
  if (!room) throw new Error("Kick has no channel called " + slug);
  p.id = String(ch.user_id || "");
  p.name = (ch.user && ch.user.username) || slug;
  p.live = !!ch.livestream;
  p.title = ch.livestream ? ch.livestream.session_title || "" : "";
  p.viewers = ch.livestream ? num(ch.livestream.viewer_count) : null;
  kickSubBadges = (ch.subscriber_badges || []).map((b) => ({ months: num(b.months), url: (b.badge_image && b.badge_image.src) || "" })).filter((b) => b.url);
  // (a moderator there: your own channel, or Kick's badges for you in it say so)
  if (p.id && p.id === String(S().kickUserId || "")) p.mod = true;
  else kickModRead(p, slug);
  await kickConnect(p, my, room);
  conn.kick.timers.push(setInterval(async () => {
    try {
      const x = await kickGet("channels/" + encodeURIComponent(slug));
      if (my !== conn.kick.gen) return;
      p.live = !!x.livestream;
      p.title = x.livestream ? x.livestream.session_title || "" : "";
      p.viewers = x.livestream ? num(x.livestream.viewer_count) : null;
      push();
    } catch (e) {}
  }, 60000));
}

// (your badges in their channel, as Kick shows anyone's: a moderator's badge, or none)
async function kickModRead(p, slug) {
  const me = String(S().kickLogin || "").toLowerCase();
  if (!me) return;
  try {
    const u = await kickGet("channels/" + encodeURIComponent(slug) + "/users/" + encodeURIComponent(me));
    const raw = Array.isArray(u.badges) ? u.badges : u.identity && Array.isArray(u.identity.badges) ? u.identity.badges : null;
    const list = (raw || []).map((b) => String(b.type || "").toLowerCase());
    // (only what Kick plainly said: anything else is found out by trying, a refusal greying them out then)
    if (u.is_moderator === true || list.some((t) => t === "moderator" || t === "broadcaster")) isMod("kick");
    else if (u.is_moderator === false || raw) notMod("kick");
  } catch (e) {}
}

async function kickConnect(p, my, room) {
  const c = conn.kick;
  const client = randomText(8) + "-" + randomText(4) + "-4" + randomText(3) + "-a" + randomText(3) + "-" + randomText(12);
  const cn = await kickPost("connection", { client: { id: client, type: "web" }, capabilities: { accepted_providers: [{ provider: "centrifugo" }] } });
  const url = cn.connections && cn.connections[0] && cn.connections[0].credentials && cn.connections[0].credentials.url;
  if (!url) throw new Error("Kick's chat didn't say where to connect");
  const token = (await kickPost("auth/connection", { client_id: client })).token;
  if (my !== c.gen) return;
  const s = OmniPulse.socket(url);
  c.sock = s;
  s.onopen = () => {
    s.send(JSON.stringify({ connect: { token, name: "js" }, id: 1 }));
    s.send(JSON.stringify({ subscribe: { channel: "chatrooms." + room + ".v2" }, id: 2 }));
  };
  s.onmessage = (data) => String(data).split("\n").filter(Boolean).forEach((line) => {
    if (line === "{}") { s.send("{}"); return; }
    let m;
    try { m = JSON.parse(line); } catch (e) { return; }
    if (m.id === 1 && m.connect) { p.up = true; c.retry = 2000; push(); }
    if (m.id === 1 && m.connect && m.connect.ttl) {
      if (c.refresh) clearInterval(c.refresh);
      c.refresh = setInterval(() => {
        kickPost("auth/connection", { client_id: client }).then((t) => { if (c.sock === s) s.send(JSON.stringify({ refresh: { token: t.token }, id: ++kickN })); }).catch(() => {});
      }, Math.max(60, m.connect.ttl * 0.75) * 1000);
    }
    const pub = m.push && m.push.pub && m.push.pub.data;
    if (!pub) return;
    let d = pub.data;
    if (typeof d === "string") { try { d = JSON.parse(d); } catch (e) { d = {}; } }
    onKick(String(pub.event || ""), d || {});
  });
  s.onclose = () => {
    if (c.sock !== s) return;
    c.sock = null; p.up = false; push();
    if (c.refresh) { clearInterval(c.refresh); c.refresh = null; }
    if (my === c.gen) { setTimeout(() => { if (my === c.gen && !c.sock) kickConnect(p, my, room).catch((e) => notice("Kick's chat: " + e.message)); }, c.retry); c.retry = Math.min(c.retry * 2, 60000); }
  };
}

function onKick(event, d) {
  const ev = event.replace(/^App\\Events\\/, "");
  if (ev === "MessageDeletedEvent") return gone("kick", String((d.message && d.message.id) || d.id || ""), "");
  if (ev === "UserBannedEvent") { const u = d.user || {}; return gone("kick", "", String(u.slug || u.username || "")); }
  if (ev === "ChatroomClearEvent") return gone("kick", "", "");
  if (ev !== "ChatMessageEvent") return;
  const sender = d.sender || {};
  const slug = String(sender.slug || sender.username || "").toLowerCase();
  const list = (sender.identity && sender.identity.badges) || [];
  const self = !!S().kickLogin && String(sender.username || "").toLowerCase() === String(S().kickLogin).toLowerCase();
  // (your own message: your badges here say whether you're a moderator)
  if (self) { if (list.some((b) => ["moderator", "broadcaster"].includes(String(b.type || "").toLowerCase()))) isMod("kick"); }
  const sub = list.find((b) => String(b.type || "").toLowerCase() === "subscriber");
  const subImg = sub ? kickSubBadges.filter((b) => b.months <= Math.max(1, num(sub.count))).sort((a, b) => b.months - a.months)[0] : null;
  OmniPulse.app.modChat({
    platform: "kick", id: String(d.id || ""), user: sender.username || slug, login: slug, userId: String(sender.id || ""), color: (sender.identity && sender.identity.color) || "",
    badges: list.map((b) => String(b.type || "").toLowerCase()), badgeImgs: subImg ? [{ url: subImg.url, title: "Subscriber (" + Math.max(1, num(sub.count)) + " months)" }] : [],
    badgeList: list.map((b) => ({ type: String(b.type || "").toLowerCase(), text: String(b.text || b.type || ""), count: num(b.count) })),
    text: String(d.content || ""), self,
  }).catch(() => {});
}

// ---------- what the page asks ----------
function modeBody(mode, on, secs, mins) {
  if (mode === "slow") return on ? { slow_mode: true, slow_mode_wait_time: Math.max(3, Math.min(120, num(secs) || 10)) } : { slow_mode: false };
  if (mode === "follow") return on ? { follower_mode: true, follower_mode_duration: Math.max(0, Math.min(129600, num(mins))) } : { follower_mode: false };
  if (mode === "sub") return { subscriber_mode: !!on };
  if (mode === "emote") return { emote_mode: !!on };
  if (mode === "unique") return { unique_chat_mode: !!on };
  return {};
}

function long(secs) {
  return secs % 3600 === 0 ? secs / 3600 + (secs === 3600 ? " hour" : " hours") : secs % 60 === 0 ? secs / 60 + (secs === 60 ? " minute" : " minutes") : secs + " seconds";
}

// One platform's moderation, as you: refused there (you're not a moderator on that platform), said so, by name
async function asMod(platform, go) {
  const p = watching && watching[platform];
  if (!p) throw new Error("you're not watching " + PNAME[platform] + " here");
  if (p.mod === false) throw new Error("you're not a moderator in " + p.name + "'s " + PNAME[platform] + " chat, so you can't do that on " + PNAME[platform]);
  try {
    const r = await go(p);
    isMod(platform);
    return r;
  } catch (e) {
    if (/ 403/.test(e.message)) { notMod(platform); throw new Error(PNAME[platform] + ": you're not a moderator in " + p.name + "'s chat there"); }
    throw e;
  }
}

async function act(cmd) {
  const who = cmd.user || cmd.login || "them";
  const plat = cmd.platform === "kick" ? "kick" : "twitch";
  const need = () => { if (!watching) throw new Error("watch a channel first (Channel, on the left)"); };
  // (Twitch's own: chat modes, Shield Mode, lockdowns and AutoMod; Kick lets apps do none of these)
  const twitchOnly = () => { need(); if (!watching.twitch) throw new Error("that's for Twitch channels only: Kick doesn't let apps change it"); };
  switch (cmd.do) {
    case "signin": return signIn(cmd.platform === "kick" ? "kick" : "twitch", cmd.values || {});
    case "signout":
      if (cmd.platform === "kick") { stopPlatform("kick"); if (watching && !watching.twitch) watching = null; await keep({ kickToken: "", kickRefreshToken: "", kickLogin: "", kickUserId: "" }); push(); return "Signed out of Kick."; }
      stopPlatform("twitch");
      if (watching && !watching.kick) watching = null;
      await keep({ token: "", refresh: "", login: "", userId: "" });
      channels = [];
      push();
      return "Signed out of Twitch.";
    case "watch": return watch(plat, cmd.channel, !!cmd.also);
    // (both: the streamer's Twitch and Kick channels, at once)
    case "watchboth": {
      const out = [];
      if (cmd.twitch) out.push(await watch("twitch", cmd.twitch, false));
      if (cmd.kick) out.push(await watch("kick", cmd.kick, !!cmd.twitch));
      return out.join(" ");
    }
    case "unwatch": stopPlatform(plat); if (watching && !watching.twitch && !watching.kick) watching = null; keep({ watched: watching ? { twitch: watching.twitch ? watching.twitch.login : "", kick: watching.kick ? watching.kick.login : "" } : null }).catch(() => {}); push(); return "";
    case "stop": stopWatching(); keep({ watched: null }).catch(() => {}); push(); return "";
    case "refresh": await channelsRead(); if (watching && watching.twitch) { liveRead(watching.twitch); modesRead(); } return "";
    case "say": {
      need();
      const text = String(cmd.text || "").trim().slice(0, 500);
      if (!text) return "";
      const to = cmd.platform === "kick" || cmd.platform === "twitch" ? [cmd.platform] : ["twitch", "kick"];
      const failed = [];
      for (const p of to) {
        const w = watching[p];
        if (!w) continue;
        try {
          if (p === "kick") await kickApi("POST", "chat", { content: text, type: "user", broadcaster_user_id: Number(w.id) });
          else {
            if (!conn.twitch.irc || !w.up) throw new Error("not in the chat yet");
            // (shown when Twitch sends it back, on the reading connection: delivered, with its emotes)
            conn.twitch.irc.send("PRIVMSG #" + w.login + " :" + text.slice(0, 480));
          }
        } catch (e) { failed.push(PNAME[p] + ": " + e.message); }
      }
      if (failed.length) throw new Error("couldn't send it on " + failed.join("; "));
      return "";
    }
    case "delete":
      need();
      if (!cmd.id) throw new Error("that message has no id to delete it by");
      return asMod(plat, async (w) => {
        if (plat === "kick") await kickApi("DELETE", "chat/" + encodeURIComponent(cmd.id));
        else await helix("DELETE", "moderation/chat?" + twitchMod() + "&message_id=" + encodeURIComponent(cmd.id));
        return "Deleted " + who + "'s message on " + PNAME[plat] + ".";
      });
    case "timeout":
    case "ban":
    case "unban": {
      need();
      const secs = Math.max(1, Math.min(1209600, Math.round(num(cmd.seconds)) || 600));
      return asMod(plat, async (w) => {
        if (plat === "kick") {
          if (!cmd.userId) throw new Error("Kick didn't say who that is");
          const body = { broadcaster_user_id: Number(w.id), user_id: Number(cmd.userId) };
          if (cmd.reason && cmd.do !== "unban") body.reason = String(cmd.reason).slice(0, 100);
          if (cmd.do === "unban") await kickApi("DELETE", "moderation/bans", body);
          else await kickApi("POST", "moderation/bans", Object.assign(body, cmd.do === "timeout" ? { duration: Math.max(1, Math.round(secs / 60)) } : {}));
        } else {
          let id = cmd.userId;
          if (!id) id = (((await helix("GET", "users?login=" + encodeURIComponent(String(cmd.login || cmd.user || "").toLowerCase()))).data || [])[0] || {}).id;
          if (!id) throw new Error("Twitch didn't say who that is");
          if (cmd.do === "unban") await helix("DELETE", "moderation/bans?" + twitchMod() + "&user_id=" + id);
          else await helix("POST", "moderation/bans?" + twitchMod(), { data: Object.assign({ user_id: id }, cmd.do === "timeout" ? { duration: secs } : {}, cmd.reason ? { reason: String(cmd.reason).slice(0, 500) } : {}) });
        }
        const on = " on " + PNAME[plat] + ".";
        return cmd.do === "timeout" ? "Timed out " + who + " for " + long(secs) + on : cmd.do === "ban" ? "Banned " + who + on : "Let " + who + " back in" + on;
      });
    }
    case "chatmode":
      twitchOnly();
      await asMod("twitch", () => helix("PATCH", "chat/settings?" + twitchMod(), modeBody(cmd.mode, cmd.on, cmd.secs, cmd.mins)));
      await modesRead();
      return "";
    case "shield":
      twitchOnly();
      await asMod("twitch", () => helix("PUT", "moderation/shield_mode?" + twitchMod(), { is_active: !!cmd.on }));
      await modesRead();
      return "";
    case "clearchat": {
      need();
      const done = [], failed = [];
      for (const p of ["twitch", "kick"]) {
        if (!watching[p]) continue;
        try {
          await asMod(p, (w) => p === "kick" ? kickApi("POST", "chat", { content: "/clear", type: "user", broadcaster_user_id: Number(w.id) }) : helix("DELETE", "moderation/chat?" + twitchMod()));
          done.push(PNAME[p]);
        } catch (e) { failed.push(e.message); }
      }
      if (!done.length) throw new Error(failed.join("; "));
      return "Cleared the chat on " + done.join(" and ") + "." + (failed.length ? " Not on " + failed.join("; ") + "." : "");
    }
    case "lockdown": {
      twitchOnly();
      await asMod("twitch", async () => {
        if (cmd.on) {
          modes.lock = { chat: Object.assign({}, modes.chat), shield: modes.shield };
          const body = Object.assign({}, cmd.follow ? modeBody("follow", true, 0, cmd.mins) : {}, cmd.slow ? modeBody("slow", true, cmd.secs) : {}, cmd.emote ? modeBody("emote", true) : {}, cmd.sub ? modeBody("sub", true) : {});
          if (Object.keys(body).length) await helix("PATCH", "chat/settings?" + twitchMod(), body);
          if (cmd.shield) await helix("PUT", "moderation/shield_mode?" + twitchMod(), { is_active: true });
        } else if (modes.lock) {
          const was = modes.lock, c = was.chat || {};
          modes.lock = null;
          await helix("PATCH", "chat/settings?" + twitchMod(), { follower_mode: !!c.follow, slow_mode: !!c.slow, emote_mode: !!c.emote, subscriber_mode: !!c.sub, ...(c.follow ? { follower_mode_duration: num(c.followMins) } : {}), ...(c.slow ? { slow_mode_wait_time: num(c.slowSecs) || 10 } : {}) });
          if (!was.shield) await helix("PUT", "moderation/shield_mode?" + twitchMod(), { is_active: false });
        }
      });
      await modesRead();
      return "";
    }
    case "automod": {
      twitchOnly();
      await asMod("twitch", () => helix("POST", "moderation/automod/message", { user_id: String(S().userId), msg_id: String(cmd.id || ""), action: cmd.allow ? "ALLOW" : "DENY" }));
      const h = held.find((x) => x.id === cmd.id);
      held = held.filter((x) => x.id !== cmd.id);
      push();
      return (cmd.allow ? "Allowed " : "Denied ") + (h ? h.user + "'s message." : "the message.");
    }
  }
  throw new Error("Mod View can't " + cmd.do);
}

// ---------- running ----------
async function start() {
  push();
  if (S().token) {
    if (!(await validate(S().token)) && !(await renew())) { await keep({ token: "", refresh: "" }); note = "Your Twitch sign-in ran out. Sign in again."; }
  }
  await channelsRead();
  emotesRead().catch(() => {});
  const last = S().watched;
  if (last && (last.twitch || last.kick)) act({ do: "watchboth", twitch: last.twitch || "", kick: last.kick || "" }).catch((e) => notice("Couldn't watch again: " + e.message));
}

OmniPulse.register({
  start() { start().catch((e) => OmniPulse.log("start: " + e.message)); return true; },
  // (what Mod View page asks: its answer, if any, said in its chat)
  do(cmd) { return act(cmd || {}); },
  status() { return { twitch: S().login || "", kick: S().kickLogin || "", watching }; },
  // (the shared sign-in changed in the Stream Suite: signed in, out, or renewed)
  account(v) {
    const was = S().login || "", tokenWas = S().token || "";
    Object.assign(OmniPulse.settings, v || {});
    // (signed in again, as the same account: its new sign-in may allow what the old one didn't, so the channels you
    // moderate are read again and the "sign in again" note goes)
    if ((S().token || "") !== tokenWas && was === (S().login || "")) { note = ""; channelsRead().catch(() => {}); }
    if (watching && watching.twitch && !S().token) stopPlatform("twitch");
    if (watching && watching.kick && !S().kickToken) stopPlatform("kick");
    if (watching && !watching.twitch && !watching.kick) watching = null;
    if (was !== (S().login || "")) channelsRead().catch(() => {});
    push();
    return true;
  },
});
