// OmniPulse Chatbot: your chat's song requests, your actions (a trigger, then its steps, as Streamer.bot's),
// and your channel's events as alerts.
// Twitch: signed in by device code (your own Client ID), chat over Twitch's IRC WebSocket (as your
// channel, or a bot account), events over EventSub's WebSocket.

const S = () => OmniPulse.settings;
const TWITCH_SCOPES_CHANNEL = [
  "chat:read", "chat:edit", "user:read:chat", "moderator:read:followers", "channel:read:subscriptions", "bits:read",
  "channel:read:hype_train", "channel:read:polls", "moderator:read:shoutouts", "channel:read:redemptions",
  "moderator:manage:banned_users", "moderator:manage:shoutouts",
].join(" ");
const TWITCH_SCOPES_BOT = "chat:read chat:edit";

// ---------- signing in (Twitch's device code flow: no secret, no redirect) ----------
async function form(url, fields) {
  const body = Object.entries(fields).map(([k, v]) => encodeURIComponent(k) + "=" + encodeURIComponent(v)).join("&");
  const r = await OmniPulse.fetch(url, { method: "POST", headers: { "Content-Type": "application/x-www-form-urlencoded" }, body });
  let j = {};
  try { j = await r.json(); } catch (e) {}
  return { ok: r.ok, status: r.status, j };
}

// Signing in: the code given at once (a call gets 25 s, and signing in takes as long as it takes), the
// rest waited for in the background, said on the app's status line when it's done
async function signIn(who, scopes, values) {
  let said = null;
  const first = new Promise((ok) => { said = ok; });
  signInFlow(who, scopes, values, said).then((msg) => { if (msg) { said(msg); OmniPulse.app.say(msg); } }, (e) => { said("Couldn't sign in: " + e.message); });
  return first;
}

async function signInFlow(who, scopes, values, said) {
  const clientId = ((values && values.twitchClientId) || S().twitchClientId || "").trim();
  if (!clientId) return "Put your Twitch Client ID in first (its note says where), then sign in.";
  const d = await form("https://id.twitch.tv/oauth2/device", { client_id: clientId, scopes });
  if (!d.ok) return "Twitch didn't take that Client ID: " + (d.j.message || d.status) + ". Check its Client Type is Public.";
  OmniPulse.app.openUrl(d.j.verification_uri);
  said("Sign in on Twitch's page that's opened (" + (who === "bot" ? "as the bot account" : "as your channel") + "): its code is " + d.j.user_code + ". This says when it's done.");
  // waited for: Twitch says when it's done
  const until = Date.now() + (d.j.expires_in || 1800) * 1000;
  let every = Math.max(5, d.j.interval || 5) * 1000;
  while (Date.now() < until) {
    await new Promise((ok) => setTimeout(ok, every));
    const t = await form("https://id.twitch.tv/oauth2/token", { client_id: clientId, scopes, device_code: d.j.device_code, grant_type: "urn:ietf:params:oauth:grant-type:device_code" });
    if (t.ok && t.j.access_token) {
      const me = await validate(t.j.access_token);
      if (!me) return "Signed in, but Twitch didn't say who as: try again.";
      const keep = who === "bot"
        ? { twitchClientId: clientId, botToken: t.j.access_token, botRefresh: t.j.refresh_token, botLogin: me.login }
        : { twitchClientId: clientId, token: t.j.access_token, refresh: t.j.refresh_token, login: me.login, userId: me.user_id };
      await OmniPulse.app.saveSettings(keep);
      Object.assign(OmniPulse.settings, keep);
      restart();
      return "Signed in to Twitch as " + me.login + (who === "bot" ? " (the bot)." : ".");
    }
    const msg = (t.j.message || "").toLowerCase();
    if (msg.includes("slow_down")) every += 5000;
    else if (!msg.includes("pending")) return "Twitch's sign-in didn't finish: " + (t.j.message || t.status);
  }
  return "The sign-in wasn't finished in time: try again.";
}

async function validate(token) {
  const r = await OmniPulse.fetch("https://id.twitch.tv/oauth2/validate", { headers: { Authorization: "OAuth " + token } });
  return r.ok ? r.json() : null;
}

// A token that's run out, renewed (a public client's refresh needs no secret)
async function renew(who) {
  const s = S();
  const refresh = who === "bot" ? s.botRefresh : s.refresh;
  if (!refresh || !s.twitchClientId) return false;
  const t = await form("https://id.twitch.tv/oauth2/token", { client_id: s.twitchClientId, grant_type: "refresh_token", refresh_token: refresh });
  if (!t.ok || !t.j.access_token) return false;
  const keep = who === "bot" ? { botToken: t.j.access_token, botRefresh: t.j.refresh_token || refresh } : { token: t.j.access_token, refresh: t.j.refresh_token || refresh };
  await OmniPulse.app.saveSettings(keep);
  Object.assign(OmniPulse.settings, keep);
  return true;
}

async function fresh(who) {
  const token = who === "bot" ? S().botToken : S().token;
  if (!token) return null;
  if (await validate(token)) return token;
  return (await renew(who)) ? (who === "bot" ? S().botToken : S().token) : null;
}

// ---------- chat: Twitch's IRC over WebSocket ----------
let chat = null, botChat = null, events = null, stopped = true, retry = 1000, apiToken = "";

function ircTags(raw) {
  const tags = {};
  raw.split(";").forEach((kv) => { const i = kv.indexOf("="); tags[kv.slice(0, i)] = kv.slice(i + 1).replace(/\\s/g, " ").replace(/\\:/g, ";"); });
  return tags;
}

// One line of IRC: its tags, who sent it, its command, its channel and its text
function parse(line) {
  let tags = {}, rest = line;
  if (rest[0] === "@") { const i = rest.indexOf(" "); tags = ircTags(rest.slice(1, i)); rest = rest.slice(i + 1); }
  let prefix = "";
  if (rest[0] === ":") { const i = rest.indexOf(" "); prefix = rest.slice(1, i); rest = rest.slice(i + 1); }
  const t = rest.indexOf(" :");
  const head = (t >= 0 ? rest.slice(0, t) : rest).split(" ");
  return { tags, nick: prefix.split("!")[0], command: head[0], channel: head[1], text: t >= 0 ? rest.slice(t + 2) : "" };
}

function connectChat(token, login, onLine) {
  const s = OmniPulse.socket("wss://irc-ws.chat.twitch.tv:443");
  s.onopen = () => {
    s.send("CAP REQ :twitch.tv/tags twitch.tv/commands");
    s.send("PASS oauth:" + token);
    s.send("NICK " + login);
    s.send("JOIN #" + S().login);
  };
  s.onmessage = (data) => data.split("\r\n").filter(Boolean).forEach((line) => {
    if (line.startsWith("PING")) { s.send("PONG" + line.slice(4)); return; }
    onLine(parse(line));
  });
  s.onclose = (why) => { if (!stopped) { OmniPulse.log("chat closed: " + why); setTimeout(start, retry); retry = Math.min(retry * 2, 60000); } };
  return s;
}

// A reply in chat: from the bot account if there is one, else your channel; to the message it answers
function reply(to, text) {
  if (!text) return;
  const s = botChat || chat;
  if (!s) return;
  const lines = String(text).split("\n").filter(Boolean);
  lines.forEach((line) => s.send((to ? "@reply-parent-msg-id=" + to + " " : "") + "PRIVMSG #" + S().login + " :" + line.slice(0, 480)));
}

// ---------- commands ----------
const cooldowns = {};

function commands() {
  const out = {};
  String(S().commands || "").split(";").forEach((pair) => {
    const i = pair.indexOf("=");
    if (i < 0) return;
    const name = pair.slice(0, i).trim().toLowerCase();
    if (name.startsWith("!")) out[name] = pair.slice(i + 1).trim();
  });
  return out;
}

async function onChat(m) {
  if (m.command !== "PRIVMSG") return;
  // (the bot's own messages never start anything)
  if (S().botLogin && m.nick === S().botLogin.toLowerCase()) return;
  chatLines++;
  const text = m.text.trim();
  const space = text.indexOf(" ");
  const cmd = (space < 0 ? text : text.slice(0, space)).toLowerCase();
  const args = space < 0 ? "" : text.slice(space + 1).trim();
  const user = m.tags["display-name"] || m.nick;
  const badges = m.tags.badges || "";
  const isMod = m.tags.mod === "1" || badges.includes("broadcaster/");
  const id = m.tags.id;
  onActions(m, text, cmd, args, user, id);
  if (!text.startsWith("!")) return;
  // (a second from the same viewer: each command at most once every 3 s per viewer)
  const k = cmd + "|" + user.toLowerCase();
  if (cooldowns[k] && Date.now() - cooldowns[k] < 3000) return;
  cooldowns[k] = Date.now();
  if (S().requests !== false) {
    const sr = String(S().srCommands || "!sr, !request").split(",").map((x) => x.trim().toLowerCase()).filter(Boolean);
    if (sr.includes(cmd)) {
      if (!args) return reply(id, "@" + user + " what song? " + cmd + " song by artist");
      try { const r = await OmniPulse.app.request(user, args, "twitch"); reply(id, r.message); } catch (e) { OmniPulse.log("request: " + e.message); }
      return;
    }
    if (cmd === "!song") { try { reply(id, (await OmniPulse.app.nowPlaying()).message); } catch (e) {} return; }
    if (cmd === "!wrongsong") { try { reply(id, (await OmniPulse.app.wrongSong(user, isMod)).message); } catch (e) {} return; }
  }
}

// ---------- actions: a trigger, then its steps, in order ----------
let all = [], live = [], chatLines = 0, timers = [];
const lastRun = {};
const vars = {};

// The actions as they're kept (the old way's commands, as actions, until there are some)
function loadActions() {
  let a = S().actions;
  if (typeof a === "string") { try { a = JSON.parse(a); } catch (e) { a = null; } }
  if (!Array.isArray(a)) {
    a = Object.entries(commands()).map(([cmd, said]) => ({ name: cmd, on: true, trigger: { type: "command", command: cmd, aliases: "", who: "everyone", cooldown: 0, usercooldown: 3 }, steps: [{ type: "say", text: said.replace(/\{user\}/g, "%user%").replace(/\{args\}/g, "%rawInput%"), reply: true }] }));
  }
  all = a.filter((x) => x && x.trigger);
  live = all.filter((x) => x.on !== false);
  setTimers();
}

const yes = (v) => v === true || v === "true";
const num = (v) => { const n = Number(v); return isFinite(n) ? n : 0; };

// Who they are in your channel, from their badges
function rank(tags) {
  const b = tags.badges || "";
  const you = b.includes("broadcaster/");
  const mod = you || tags.mod === "1" || b.includes("moderator/");
  const vip = mod || b.includes("vip/");
  const sub = vip || tags.subscriber === "1" || b.includes("subscriber/") || b.includes("founder/");
  return { you, mod, vip, sub };
}

function allowed(who, r) {
  switch (who) {
    case "subscribers": return r.sub;
    case "vips": return r.vip;
    case "mods": return r.mod;
    case "you": return r.you;
    default: return true;
  }
}

// Its cooldowns: everyone's and this viewer's (seconds), passed or not; noted as run if they have
function cooled(a, user) {
  const t = a.trigger, now = Date.now(), k = a.name, ku = a.name + "|" + (user || "").toLowerCase();
  if (lastRun[k] && now - lastRun[k] < num(t.cooldown) * 1000) return false;
  if (user && lastRun[ku] && now - lastRun[ku] < num(t.usercooldown) * 1000) return false;
  lastRun[k] = now;
  if (user) lastRun[ku] = now;
  return true;
}

function words(rawInput) {
  const w = {};
  String(rawInput || "").split(/\s+/).filter(Boolean).slice(0, 10).forEach((x, i) => { w["input" + i] = x; });
  return w;
}

function onActions(m, text, cmd, args, user, msgId) {
  const r = rank(m.tags);
  const base = { user, userId: m.tags["user-id"] || "", message: text, msgId, platform: "twitch" };
  for (const a of live) {
    const t = a.trigger;
    if (t.type === "command") {
      const names = [t.command].concat(String(t.aliases || "").split(",")).map((x) => String(x || "").trim().toLowerCase()).filter(Boolean);
      if (!names.includes(cmd) || !allowed(t.who, r) || !cooled(a, user)) continue;
      run(a, Object.assign({}, base, { command: cmd, rawInput: args }, words(args)));
    } else if (t.type === "message") {
      const want = String(t.text || "").toLowerCase(), said = text.toLowerCase();
      let hit = false;
      if (t.match === "starts") hit = said.startsWith(want);
      else if (t.match === "exact") hit = said === want;
      else if (t.match === "regex") { try { hit = new RegExp(t.text || "", "i").test(text); } catch (e) {} }
      else hit = want !== "" && said.includes(want);
      if (!hit || !allowed(t.who, r) || !cooled(a, user)) continue;
      run(a, Object.assign({}, base, { rawInput: text }, words(text)));
    }
  }
}

// An event (EventSub's type), to the actions waiting for it
const EVENT_OF = {
  "channel.follow": "follow", "channel.subscribe": "sub", "channel.subscription.message": "resub",
  "channel.subscription.gift": "gift", "channel.cheer": "bits", "channel.raid": "raid",
  "channel.hype_train.begin": "hypetrain", "channel.poll.begin": "poll",
  "channel.channel_points_custom_reward_redemption.add": "reward",
};

function onEvent(type, e) {
  const ev = EVENT_OF[type];
  if (!ev) return;
  const a = asAlert(type, e) || {};
  if (ev === "sub" && e.is_gift) return;
  const amount = ev === "bits" ? String(e.bits || 0) : ev === "raid" ? String(e.viewers || 0) : ev === "gift" ? String(e.total || 1) : ev === "resub" ? String(e.cumulative_months || 1) : ev === "hypetrain" ? String(e.level || 1) : "";
  const reward = (e.reward && e.reward.title) || "";
  const ctx = { user: a.user || e.user_name || "", userId: e.user_id || e.from_broadcaster_user_id || "", amount, message: a.message || e.user_input || "", reward, rawInput: e.user_input || "", platform: "twitch" };
  Object.assign(ctx, words(ctx.rawInput));
  for (const x of live) {
    const t = x.trigger;
    if (t.type !== "event" || t.event !== ev) continue;
    if (num(t.min) > 0 && num(amount) < num(t.min)) continue;
    if (ev === "reward" && String(t.reward || "").trim() && String(t.reward).trim().toLowerCase() !== reward.toLowerCase()) continue;
    run(x, ctx);
  }
}

// Every few minutes: each timed action, if chat's said enough since
function setTimers() {
  timers.forEach((id) => clearInterval(id));
  timers = [];
  for (const a of live) {
    if (a.trigger.type !== "timer") continue;
    let seen = chatLines;
    timers.push(setInterval(() => {
      if (!chat || chatLines - seen < num(a.trigger.lines)) return;
      seen = chatLines;
      run(a, { user: "", platform: "twitch" });
    }, Math.max(1, num(a.trigger.minutes) || 15) * 60000));
  }
}

// A variable: this viewer's own, else everyone's; kept ones from the settings
function getVar(name, user) {
  const kept = S().vars || {};
  const mine = name + "|" + String(user || "").toLowerCase();
  for (const k of [mine, name]) {
    if (k in vars) return vars[k];
    if (k in kept) return kept[k];
  }
  return "";
}

let saveVarsSoon = null;
function setVar(name, value, per, keep, user) {
  if (!name) return;
  const k = per ? name + "|" + String(user || "").toLowerCase() : name;
  if (!keep) { vars[k] = value; return; }
  delete vars[k];
  const kept = Object.assign({}, S().vars || {});
  kept[k] = value;
  OmniPulse.settings.vars = kept;
  // (kept a moment after the last change, not on every one)
  if (saveVarsSoon) clearTimeout(saveVarsSoon);
  saveVarsSoon = setTimeout(() => { OmniPulse.app.saveSettings({ vars: S().vars }).catch(() => {}); }, 2000);
}

// %name% filled in: what started it, else a variable (%var:name%)
function fill(text, ctx) {
  return String(text == null ? "" : text).replace(/%([A-Za-z0-9_]+(?::[A-Za-z0-9_ ]+)?)%/g, (whole, k) => {
    if (k.startsWith("var:")) { const v = getVar(k.slice(4).trim(), ctx.user); return v === "" ? "0" : String(v); }
    if (k in ctx) return String(ctx[k] == null ? "" : ctx[k]);
    return k.startsWith("input") ? "" : whole;
  });
}

function test(left, op, right) {
  const l = String(left).trim(), r = String(right).trim();
  switch (op) {
    case "is": return l.toLowerCase() === r.toLowerCase();
    case "isnt": return l.toLowerCase() !== r.toLowerCase();
    case "contains": return l.toLowerCase().includes(r.toLowerCase());
    case "more": return num(l) > num(r);
    case "less": return num(l) < num(r);
    case "empty": return l === "";
    default: return l !== "";
  }
}

// An action's steps, in order (one that says stop, stops it)
async function run(a, ctx, depth) {
  depth = depth || 0;
  if (depth > 5) return;
  for (const st of a.steps || []) {
    try {
      if ((await step(st, ctx, depth)) === false) return;
    } catch (e) {
      OmniPulse.log(a.name + ": " + e.message);
      OmniPulse.app.say(a.name + ": " + e.message).catch(() => {});
      return;
    }
  }
}

async function step(st, ctx, depth) {
  const f = (k) => fill(st[k], ctx);
  const to = yes(st.reply) ? ctx.msgId : null;
  switch (st.type) {
    case "say": reply(to, f("text")); break;
    case "random": {
      const l = f("lines").split("|").map((x) => x.trim()).filter(Boolean);
      if (l.length) reply(null, l[Math.floor(Math.random() * l.length)]);
      break;
    }
    case "wait": await new Promise((ok) => setTimeout(ok, Math.min(3600, Math.max(0, num(f("seconds")))) * 1000)); break;
    case "if": return test(f("left"), st.test, f("right"));
    case "setvar": setVar(fill(st.name, ctx).trim(), f("value"), yes(st.per), yes(st.keep), ctx.user); break;
    case "addvar": {
      const n = fill(st.name, ctx).trim();
      const was = num(getVar(n, yes(st.per) ? ctx.user : ""));
      setVar(n, was + num(f("amount")), yes(st.per), yes(st.keep), ctx.user);
      break;
    }
    case "request": {
      const r = await OmniPulse.app.request(ctx.user || S().login || "", f("query"), "twitch");
      if (yes(st.reply)) reply(ctx.msgId, r.message);
      break;
    }
    case "nowplaying": reply(ctx.msgId, (await OmniPulse.app.nowPlaying()).message); break;
    case "skip": await OmniPulse.app.control("next"); break;
    case "playpause": await OmniPulse.app.control("play"); break;
    case "alert": await OmniPulse.app.alert({ type: st.alert || "follow", user: f("user"), amount: f("amount"), message: f("message") }); break;
    case "timeout": {
      if (!ctx.userId) break;
      await helix("POST", "moderation/bans?broadcaster_id=" + S().userId + "&moderator_id=" + S().userId, { data: { user_id: ctx.userId, duration: Math.max(1, num(f("seconds"))), reason: f("reason") } });
      break;
    }
    case "shoutout": {
      const login = f("who").replace("@", "").trim().toLowerCase();
      if (!login) break;
      const u = await helix("GET", "users?login=" + encodeURIComponent(login));
      const id = u.data && u.data[0] && u.data[0].id;
      if (!id) throw new Error("there's no channel called " + login);
      await helix("POST", "chat/shoutouts?from_broadcaster_id=" + S().userId + "&to_broadcaster_id=" + id + "&moderator_id=" + S().userId);
      break;
    }
    case "run": {
      const name = f("action").trim().toLowerCase();
      const next = all.find((x) => String(x.name || "").toLowerCase() === name);
      if (next) await run(next, ctx, depth + 1);
      break;
    }
    case "status": await OmniPulse.app.say(f("text")); break;
  }
  return true;
}

// Twitch's API, as your channel (its moderation, its shoutouts)
async function helix(method, path, body) {
  const r = await OmniPulse.fetch("https://api.twitch.tv/helix/" + path, {
    method,
    headers: { "Client-Id": S().twitchClientId, Authorization: "Bearer " + apiToken, "Content-Type": "application/json" },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!r.ok) throw new Error("Twitch said " + r.status + (r.status === 401 || r.status === 403 ? ": sign in to your channel again (in its settings), for the permission" : ""));
  if (r.status === 204) return {};
  try { return await r.json(); } catch (e) { return {}; }
}

// ---------- events: EventSub over WebSocket, to alerts ----------
const SUBS = (me) => [
  ["channel.follow", "2", { broadcaster_user_id: me, moderator_user_id: me }],
  ["channel.subscribe", "1", { broadcaster_user_id: me }],
  ["channel.subscription.message", "1", { broadcaster_user_id: me }],
  ["channel.subscription.gift", "1", { broadcaster_user_id: me }],
  ["channel.cheer", "1", { broadcaster_user_id: me }],
  ["channel.raid", "1", { to_broadcaster_user_id: me }],
  ["channel.hype_train.begin", "1", { broadcaster_user_id: me }],
  ["channel.poll.begin", "1", { broadcaster_user_id: me }],
  ["channel.channel_points_custom_reward_redemption.add", "1", { broadcaster_user_id: me }],
];

// An event, as an alert (the alerts' types, as the bots send them)
function asAlert(type, e) {
  const user = e.user_name || e.from_broadcaster_user_name || "";
  switch (type) {
    case "channel.follow": return { type: "follow", user };
    case "channel.subscribe": return e.is_gift ? null : { type: "sub", user, amount: tierName(e.tier) };
    case "channel.subscription.message": return { type: "sub", user, amount: (e.cumulative_months || 1) + " months", message: (e.message && e.message.text) || "" };
    case "channel.subscription.gift": return { type: "gift", user: e.is_anonymous ? "Anonymous" : user, amount: String(e.total || 1) };
    case "channel.cheer": return { type: "bits", user: e.is_anonymous ? "Anonymous" : user, amount: String(e.bits || 0), message: e.message || "" };
    case "channel.raid": return { type: "raid", user, amount: String(e.viewers || 0) };
    case "channel.hype_train.begin": return { type: "hypetrain", user: "", amount: "Level " + (e.level || 1) };
    case "channel.poll.begin": return { type: "poll", user: "", message: e.title || "" };
  }
  return null;
}

function tierName(t) { return t === "3000" ? "Tier 3" : t === "2000" ? "Tier 2" : "Tier 1"; }

function connectEvents(token, url) {
  const s = OmniPulse.socket(url || "wss://eventsub.wss.twitch.tv/ws");
  s.onmessage = async (data) => {
    let m;
    try { m = JSON.parse(data); } catch (e) { return; }
    const kind = m.metadata && m.metadata.message_type;
    if (kind === "session_welcome" && !url) {
      const session = m.payload.session.id;
      for (const [type, version, condition] of SUBS(S().userId)) {
        const r = await OmniPulse.fetch("https://api.twitch.tv/helix/eventsub/subscriptions", {
          method: "POST",
          headers: { "Client-Id": S().twitchClientId, Authorization: "Bearer " + token, "Content-Type": "application/json" },
          body: JSON.stringify({ type, version, condition, transport: { method: "websocket", session_id: session } }),
        });
        if (!r.ok) OmniPulse.log("couldn't follow " + type + ": " + r.status);
      }
    } else if (kind === "session_reconnect") {
      // (Twitch moving it: the new one's opened first, then this one let go)
      const old = s;
      events = connectEvents(token, m.payload.session.reconnect_url);
      old.onclose = null;
      old.close();
    } else if (kind === "notification") {
      const type = m.payload.subscription.type, e = m.payload.event || {};
      onEvent(type, e);
      if (S().alerts !== false) {
        const a = asAlert(type, e);
        if (a) OmniPulse.app.alert(a).catch((err) => OmniPulse.log("alert: " + err.message));
      }
    }
  };
  s.onclose = (why) => { if (!stopped && events === s) { OmniPulse.log("events closed: " + why); setTimeout(() => { events = connectEvents(token); }, retry); } };
  return s;
}

// ---------- running ----------
async function start() {
  stopAll();
  if (!S().token || !S().login) return;
  stopped = false;
  const token = await fresh("channel");
  if (!token) { OmniPulse.app.say("Your Twitch sign-in has run out: sign in again in its settings."); return; }
  apiToken = token;
  loadActions();
  chat = connectChat(token, S().login, onChat);
  if (S().botToken && S().botLogin) {
    const bot = await fresh("bot");
    if (bot) botChat = connectChat(bot, S().botLogin, () => {});
  }
  events = connectEvents(token);
  retry = 1000;
}

function stopAll() {
  stopped = true;
  timers.forEach((id) => clearInterval(id));
  timers = [];
  for (const s of [chat, botChat, events]) { if (s) { s.onclose = null; s.close(); } }
  chat = botChat = events = null;
}

function restart() { start().catch((e) => OmniPulse.log("start: " + e.message)); }

OmniPulse.register({
  start() { restart(); return true; },
  signInChannel(values) { return signIn("channel", TWITCH_SCOPES_CHANNEL, values); },
  signInBot(values) { return signIn("bot", TWITCH_SCOPES_BOT, values); },
  // the Actions page: its actions as they're changed (kept by the app), and Run Now
  setActions(list) { OmniPulse.settings.actions = list; loadActions(); return true; },
  async runAction(name) {
    loadActions();
    const a = all.find((x) => String(x.name || "").toLowerCase() === String(name || "").toLowerCase());
    if (!a) return "There's no action called " + name + ".";
    if (!chat) return "Not in your chat yet: sign in to Twitch first (in the chatbot's settings).";
    await run(a, { user: S().login || "", rawInput: "", message: "", platform: "twitch" });
    return "Ran " + a.name + ".";
  },
  // the app's own: a song starting, a song requested
  appEvent(kind, data) {
    const d = data || {};
    for (const a of live) {
      if (a.trigger.type !== "song" || (a.trigger.when || "start") !== kind) continue;
      run(a, { user: d.user || "", song: d.title || "", artist: d.artist || "", platform: "twitch" });
    }
    return true;
  },
  async signOut() {
    stopAll();
    const none = { token: "", refresh: "", login: "", userId: "", botToken: "", botRefresh: "", botLogin: "" };
    await OmniPulse.app.saveSettings(none);
    Object.assign(OmniPulse.settings, none);
    return "Signed out of Twitch.";
  },
});
