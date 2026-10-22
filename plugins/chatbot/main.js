// OmniPulse Chatbot: your chat's song requests, your commands, and your channel's events as alerts.
// Twitch: signed in by device code (your own Client ID), chat over Twitch's IRC WebSocket (as your
// channel, or a bot account), events over EventSub's WebSocket.

const S = () => OmniPulse.settings;
const TWITCH_SCOPES_CHANNEL = [
  "chat:read", "chat:edit", "user:read:chat", "moderator:read:followers", "channel:read:subscriptions", "bits:read",
  "channel:read:hype_train", "channel:read:polls", "moderator:read:shoutouts",
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
let chat = null, botChat = null, events = null, stopped = true, retry = 1000;

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
  const text = m.text.trim();
  if (!text.startsWith("!")) return;
  const space = text.indexOf(" ");
  const cmd = (space < 0 ? text : text.slice(0, space)).toLowerCase();
  const args = space < 0 ? "" : text.slice(space + 1).trim();
  const user = m.tags["display-name"] || m.nick;
  const badges = m.tags.badges || "";
  const isMod = m.tags.mod === "1" || badges.includes("broadcaster/");
  const id = m.tags.id;
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
  const own = commands()[cmd];
  if (own) reply(id, own.replace(/\{user\}/g, user).replace(/\{args\}/g, args));
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
    } else if (kind === "notification" && S().alerts !== false) {
      const a = asAlert(m.payload.subscription.type, m.payload.event || {});
      if (a) OmniPulse.app.alert(a).catch((e) => OmniPulse.log("alert: " + e.message));
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
  for (const s of [chat, botChat, events]) { if (s) { s.onclose = null; s.close(); } }
  chat = botChat = events = null;
}

function restart() { start().catch((e) => OmniPulse.log("start: " + e.message)); }

OmniPulse.register({
  start() { restart(); return true; },
  signInChannel(values) { return signIn("channel", TWITCH_SCOPES_CHANNEL, values); },
  signInBot(values) { return signIn("bot", TWITCH_SCOPES_BOT, values); },
  async signOut() {
    stopAll();
    const none = { token: "", refresh: "", login: "", userId: "", botToken: "", botRefresh: "", botLogin: "" };
    await OmniPulse.app.saveSettings(none);
    Object.assign(OmniPulse.settings, none);
    return "Signed out of Twitch.";
  },
});
