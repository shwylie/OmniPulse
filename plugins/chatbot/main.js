// OmniPulse Chatbot: your chat's song requests, your actions (a trigger, then its steps, as Streamer.bot's),
// and your channel's events as alerts.
// Twitch: signed in by device code (your own Client ID), chat over Twitch's IRC WebSocket (as your
// channel, or a bot account), events over EventSub's WebSocket.

const S = () => OmniPulse.settings;
const TWITCH_SCOPES_CHANNEL = [
  "chat:read", "chat:edit", "user:read:chat", "moderator:read:followers", "channel:read:subscriptions", "bits:read",
  "channel:read:hype_train", "channel:read:polls", "moderator:read:shoutouts", "channel:read:redemptions",
  "moderator:manage:banned_users", "moderator:manage:shoutouts", "moderator:manage:chat_messages",
  "moderator:manage:announcements", "channel:manage:broadcast", "clips:edit", "channel:read:ads",
  "channel:read:predictions", "channel:read:vips", "moderation:read", "channel:read:charity",
  "channel:manage:polls", "channel:manage:predictions", "channel:manage:redemptions", "channel:edit:commercial",
  "channel:manage:raids", "moderator:manage:chat_settings", "channel:manage:vips", "channel:manage:moderators",
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
// (copy: its link copied, not opened, for a browser of your choice: a private window, signed in as the bot
// while your own account stays signed in where it is)
async function signIn(who, scopes, values, copy) {
  let said = null;
  const first = new Promise((ok) => { said = ok; });
  signInFlow(who, scopes, values, said, copy).then((msg) => { if (msg) { said(msg); OmniPulse.app.say(msg); } }, (e) => { said("Couldn't sign in: " + e.message); });
  return first;
}

// A sign-in's page: opened in your browser, or its link copied for you to open where you like
async function signInPage(url, copy) {
  if (copy) await OmniPulse.app.copyText(url);
  else OmniPulse.app.openUrl(url);
}

// OmniPulse's own Twitch app: Twitch's Authorize page, its answer caught on this PC, finished by the relay
const TWITCH_REDIRECT = "http://localhost:" + 8476 + "/twitch";
async function twitchCodeFlow(who, scopes, clientId, relay, said, copy) {
  const state = randomText(24);
  // (the bot: Twitch asked to show whose account it is, so it isn't signed in as you by mistake)
  const q = { client_id: clientId, redirect_uri: TWITCH_REDIRECT, response_type: "code", scope: scopes, state };
  if (who === "bot") q.force_verify = "true";
  const waiting = OmniPulse.app.awaitRedirect(8476, "/twitch");
  await signInPage("https://id.twitch.tv/oauth2/authorize?" + Object.entries(q).map(([k, x]) => k + "=" + encodeURIComponent(x)).join("&"), copy);
  const as = who === "bot" ? "as the bot account" : "as your channel";
  said(copy
    ? "Sign-in link copied. Within five minutes, paste it into a private window on this PC, sign in " + as + ", and click Authorize."
    : "Twitch is open in your browser. Sign in " + as + " and click Authorize.");
  const back = await waiting;
  const got = {};
  String(back.query || "").split("&").forEach((kv) => { const i = kv.indexOf("="); if (i > 0) got[kv.slice(0, i)] = decodeURIComponent(kv.slice(i + 1).replace(/\+/g, " ")); });
  if (got.error) return "Twitch's sign-in didn't finish: " + (got.error_description || got.error);
  if (got.state !== state || !got.code) return "Twitch's sign-in didn't come back as it should: try again.";
  const t = await viaRelay(relay, "twitch", "token", { code: got.code, redirect_uri: TWITCH_REDIRECT });
  if (!t.ok || !t.j.access_token) return "Twitch didn't take the sign-in: " + (t.j.message || t.j.error || t.status) + ". Try again in a moment.";
  return t.j;
}

async function signInFlow(who, scopes, values, said, copy) {
  const clientId = ((values && values.twitchClientId) || S().twitchClientId || "").trim();
  if (!clientId) return "Put your Twitch Client ID in first (its note says where), then sign in.";
  const relay = String((values && values.twitchRelay) || "").trim();
  if (relay) {
    const t = await twitchCodeFlow(who, scopes, clientId, relay, said, copy);
    if (typeof t === "string") return t;
    return twitchSignedIn(who, clientId, relay, t);
  }
  const d = await form("https://id.twitch.tv/oauth2/device", { client_id: clientId, scopes });
  if (!d.ok) return "Twitch didn't take that Client ID: " + (d.j.message || d.status) + ". Check its Client Type is Public.";
  // (the link carries its code, so there's nothing to type)
  let link = d.j.verification_uri || "https://www.twitch.tv/activate";
  if (!/device-code=/.test(link)) link += (link.includes("?") ? "&" : "?") + "device-code=" + encodeURIComponent(d.j.user_code || "");
  await signInPage(link, copy);
  const as = who === "bot" ? "as the bot account" : "as your channel";
  said(copy
    ? "Sign-in link copied. Paste it into a private window, sign in " + as + ", and click Authorize."
    : "Twitch is open in your browser. Sign in " + as + " and click Authorize.");
  // waited for: Twitch says when it's done
  const until = Date.now() + (d.j.expires_in || 1800) * 1000;
  let every = Math.max(5, d.j.interval || 5) * 1000;
  while (Date.now() < until) {
    await new Promise((ok) => setTimeout(ok, every));
    const t = await form("https://id.twitch.tv/oauth2/token", { client_id: clientId, scopes, device_code: d.j.device_code, grant_type: "urn:ietf:params:oauth:grant-type:device_code" });
    if (t.ok && t.j.access_token) return twitchSignedIn(who, clientId, "", t.j);
    const msg = (t.j.message || "").toLowerCase();
    if (msg.includes("slow_down")) every += 5000;
    else if (!msg.includes("pending")) return "Twitch's sign-in didn't finish: " + (t.j.message || t.status);
  }
  return "The sign-in wasn't finished in time: try again.";
}

// A Twitch sign-in done: who it is, kept (with how it's renewed: by the relay, or your own public app)
async function twitchSignedIn(who, clientId, relay, j) {
  const me = await validate(j.access_token);
  if (!me) return "Signed in, but Twitch didn't say who as: try again.";
  if (who === "bot" && S().login && me.login === String(S().login).toLowerCase()) return "That's your channel's own account: sign in the bot with its own Twitch account (a private window keeps you signed in as you).";
  const keep = who === "bot"
    ? { twitchClientId: clientId, twitchRelay: relay, botToken: j.access_token, botRefresh: j.refresh_token, botLogin: me.login }
    : { twitchClientId: clientId, twitchRelay: relay, token: j.access_token, refresh: j.refresh_token, login: me.login, userId: me.user_id };
  await OmniPulse.app.saveSettings(keep);
  Object.assign(OmniPulse.settings, keep);
  restart();
  return "Signed in to Twitch as " + me.login + (who === "bot" ? " (the bot)." : ".");
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
  const t = s.twitchRelay
    ? await viaRelay(s.twitchRelay, "twitch", "refresh", { refresh_token: refresh })
    : await form("https://id.twitch.tv/oauth2/token", { client_id: s.twitchClientId, grant_type: "refresh_token", refresh_token: refresh });
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
    if (/^:\S+ 001 /.test(line)) s.up = true;
    onLine(parse(line));
  });
  s.onclose = (why) => { s.up = false; if (!stopped) { OmniPulse.log("chat closed: " + why); setTimeout(start, retry); retry = Math.min(retry * 2, 60000); } };
  return s;
}

// A reply in chat, on the platform it's for ("all": every one connected): on Twitch from the bot account if
// there is one, else your channel; to the message it answers
// What it's done since it started (the settings page's bot card), and its last few doings (its live list)
const counts = { replies: 0, requests: 0, alerts: 0, actions: 0 };
const recent = [];
function noted(who, heard, did, platform) {
  recent.push({ who: String(who || ""), heard: String(heard || "").slice(0, 80), did: String(did || "").slice(0, 120), platform: platform || "twitch", at: Date.now() });
  while (recent.length > 30) recent.shift();
}

function reply(to, text, platform) {
  if (!text) return;
  counts.replies++;
  const p = platform || "twitch";
  if (p === "kick" || p === "all") kickSay(p === "kick" ? to : null, text);
  if (p === "kick") return;
  const s = botChat || chat;
  if (!s) return;
  const lines = String(text).split("\n").filter(Boolean);
  if (!botChat) lines.forEach((line) => OmniPulse.app.dockChat({ platform: "twitch", id: "", user: S().login, login: String(S().login || "").toLowerCase(), text: line.slice(0, 480), self: true, badges: ["broadcaster"] }).catch(() => {}));
  lines.forEach((line) => s.send((to && p !== "all" ? "@reply-parent-msg-id=" + to + " " : "") + "PRIVMSG #" + S().login + " :" + line.slice(0, 480)));
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
  if (m.command === "USERNOTICE" && m.tags["msg-id"] === "viewermilestone" && m.tags["msg-param-category"] === "watch-streak") {
    onEvent("watch.streak", { user_name: m.tags["display-name"] || m.nick, user_id: m.tags["user-id"] || "", streak: m.tags["msg-param-value"] || "", message: { text: m.text || "" } });
    return;
  }
  if (m.command === "CLEARMSG") { dockGone("twitch", m.tags["target-msg-id"] || "", ""); return; }
  if (m.command === "CLEARCHAT") { dockGone("twitch", "", m.text || ""); return; }
  if (m.command !== "PRIVMSG") return;
  const platform = m.platform || "twitch";
  if (platform === "twitch") dockTwitch(m);
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
  // (moderation first: a message Auto-Ban takes down starts nothing; the check is at once, only taking it down
  // waits, so nothing else is held up for it)
  const banned = autobanHit(m, text, user, platform);
  if (banned) { autoban(m, text, user, id, platform, banned); return; }
  seen[(m.nick || user).toLowerCase()] = Date.now();
  present[platform + "|" + (m.nick || user).toLowerCase()] = { at: Date.now(), user, userId: m.tags["user-id"] || "", platform };
  onActions(m, text, cmd, args, user, id);
  gameChat(user, text, platform);
  funChat(user);
  const firstKey = platform + "|" + (m.nick || user).toLowerCase();
  if (!spoken.has(firstKey)) {
    spoken.add(firstKey);
    credit("chat", user);
    fire("firstwords", Object.assign({ user, userId: m.tags["user-id"] || "", message: text, rawInput: text, msgId: id, platform, isMod }, words(text)));
  }
  if (platform === "twitch" && m.tags["first-msg"] === "1") onEvent("first.message", { user_name: user, user_id: m.tags["user-id"] || "", message: { text }, msg_id: id });
  if (!text.startsWith("!")) return;
  // (a second from the same viewer: each command at most once every 3 s per viewer)
  const k = cmd + "|" + user.toLowerCase();
  if (cooldowns[k] && Date.now() - cooldowns[k] < 3000) return;
  cooldowns[k] = Date.now();
  if (await suiteCommand(cmd, args, rank(m.tags), id, platform, user)) return;
  if (S().requests !== false) {
    const rq = reqCommands();
    if (rq.sr.includes(cmd)) {
      if (!args) return reply(id, "@" + user + " what song? " + cmd + " song by artist", platform);
      // (who may, how often, and what it costs: as Settings > Streaming > Song Requests has them)
      const who = String(S().srWho || "everyone");
      if (!allowed(who, rank(m.tags))) return reply(id, "@" + user + " song requests are for " + WHO_NAMES[who] + " right now.", platform);
      const wait = Math.max(0, num(S().srCooldown)) * 1000, k = user.toLowerCase();
      if (wait && srLast[k] && Date.now() - srLast[k] < wait) return reply(id, "@" + user + " you can request again in " + Math.ceil((wait - (Date.now() - srLast[k])) / 1000) + " s.", platform);
      const cost = yes(S().pointsOn) ? Math.max(0, num(S().srCost)) : 0;
      if (cost && pointsOf(user) < cost) return reply(id, "@" + user + " a request costs " + cost + " " + (S().pointsName || "points") + ". You have " + pointsOf(user) + ".", platform);
      try {
        const r = await OmniPulse.app.request(user, args, platform);
        counts.requests++;
        // (taken only for a song that's been added)
        if (r.ok !== false) { srLast[k] = Date.now(); if (cost) addPoints(user, -cost); }
        noted(user, cmd + " " + args, r.message, platform);
        reply(id, r.message, platform);
      } catch (e) { OmniPulse.log("request: " + e.message); }
      return;
    }
    if (rq.song.includes(cmd)) { try { reply(id, (await OmniPulse.app.nowPlaying()).message, platform); } catch (e) {} return; }
    if (rq.wrong.includes(cmd)) { try { reply(id, (await OmniPulse.app.wrongSong(user, isMod)).message, platform); } catch (e) {} return; }
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

// The request commands, as set (each a list, with commas between)
const srLast = {};
const WHO_NAMES = { everyone: "everyone", subscribers: "subscribers", vips: "VIPs", mods: "moderators" };
function reqCommands() {
  const list = (v, d) => String(v == null || v === "" ? d : v).split(",").map((x) => x.trim().toLowerCase()).filter(Boolean).map((x) => (x.startsWith("!") ? x : "!" + x));
  return { sr: list(S().srCommands, "!sr, !request"), song: list(S().songCommands, "!song"), wrong: list(S().wrongCommands, "!wrongsong") };
}

// Who they are in your channel, from their badges
// ---------- moderation: Auto-Ban ----------
// A message with one of your terms in it (or one of the common spam ones, if that's on): taken down, and its
// sender banned, timed out, or only that, as set. Loosely, by default: capitals, spaces, dots and symbols don't
// count, nor letters swapped for numbers, so "stream . ai" is "streamai" and "fr3e v1ews" is "free views".
const SPAM_PACK = [
  "buy followers", "buy viewers", "buy views", "cheap viewers", "cheap followers", "best viewers on", "get viewers", "free viewers", "free followers",
  "followers primes and viewers", "primes and viewers", "wanna become famous", "boost your stream", "promote your stream", "remove the space",
  "dogehype", "bigfollows", "streamboo", "viewerlabs", "twitchboost", "streamviewers", "viewbotting",
];
const LEET = { "@": "a", "4": "a", "$": "s", "5": "s", "0": "o", "1": "i", "!": "i", "|": "i", "3": "e", "7": "t", "8": "b" };
function loose(t) {
  return String(t || "").toLowerCase().normalize("NFKD").replace(/[̀-ͯ]/g, "")
    .replace(/[@4$50!|1378]/g, (c) => LEET[c] || c).replace(/[^a-z0-9]/g, "");
}
function autobanTerms() {
  const own = String(S().abTerms || "").split(/\r?\n|\|/).map((x) => x.trim()).filter(Boolean);
  return yes(S().abPack === undefined ? true : S().abPack) ? own.concat(SPAM_PACK) : own;
}
let autobanned = 0;
// (the term a message has, if Auto-Ban's to take it down: "" if not)
function autobanHit(m, text, user, platform) {
  const x = S();
  if (!yes(x.abOn)) return "";
  const r = rank(m.tags || {});
  // (you and your moderators always; VIPs or subscribers too, if it's set to)
  const spare = String(x.abSpare || "mods");
  if (r.mod || (spare === "vips" && r.vip) || (spare === "subs" && r.sub)) return "";
  if (yes(x.abFirstOnly) && !(m.tags["first-msg"] === "1" || !spoken.has(platform + "|" + String(m.nick || user).toLowerCase()))) return "";
  const lax = yes(x.abLoose === undefined ? true : x.abLoose);
  const hay = lax ? loose(text) : String(text).toLowerCase();
  return autobanTerms().find((t) => { const n = lax ? loose(t) : t.toLowerCase(); return n.length >= 3 && hay.includes(n); }) || "";
}
async function autoban(m, text, user, id, platform, hit) {
  const x = S();
  const what = ["ban", "timeout", "delete"].includes(x.abDo) ? x.abDo : "ban";
  const base = { platform, id, userId: (m.tags && m.tags["user-id"]) || "", user, login: m.nick, reason: String(x.abReason || "Spam (auto-ban)") };
  try { if (id) await dockDo(Object.assign({ do: "delete" }, base)); } catch (e) { OmniPulse.log("auto-ban delete: " + e.message); }
  let said = "";
  try { if (what !== "delete") said = await dockDo(Object.assign({ do: what, seconds: Math.max(1, num(x.abSecs) || 600) }, base)); } catch (e) { OmniPulse.log("auto-ban: " + e.message); said = "Couldn't " + (what === "ban" ? "ban " : "time out ") + user + ": " + e.message; }
  autobanned++;
  // (said in the Chat Dock, and kept in the Activity Feed, with the term that caught it)
  OmniPulse.app.dockChat({ platform, id: "", user: "Auto-Ban", login: "", text: (said || "Deleted " + user + "'s message.") + " Matched: " + hit, self: true, badges: [] }).catch(() => {});
  feed("ban", { user, platform, message: "Auto-Ban: " + hit, rawInput: text, amount: "" });
  return true;
}

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

// A rule broken (a chat filter): a link, mostly capitals, a banned word, too long, too many emotes
function breaks(t, text, tags) {
  switch (t.rule) {
    case "caps": {
      const letters = text.replace(/[^A-Za-z]/g, "");
      return letters.length >= 12 && letters.replace(/[^A-Z]/g, "").length / letters.length >= 0.7;
    }
    case "words": {
      const words = String(t.words || "").split(",").map((x) => x.trim().toLowerCase()).filter(Boolean);
      const said = " " + text.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, " ") + " ";
      return words.some((w) => said.includes(" " + w + " "));
    }
    case "long": return text.length > Math.max(1, num(t.most) || 300);
    case "emotes": {
      const n = String(tags.emotes || "").split("/").filter(Boolean).reduce((c, e) => c + (e.split(":")[1] || "").split(",").filter(Boolean).length, 0);
      return n > Math.max(1, num(t.most) || 10);
    }
    default: return /(https?:\/\/|www\.)\S+|\b[a-z0-9-]{2,}\.(com|net|org|tv|gg|io|ly|co|me|xyz|app|link|to)\b/i.test(text);
  }
}

function exempt(who, r) {
  if (r.you) return true;
  switch (who) {
    case "subscribers": return r.sub;
    case "vips": return r.vip;
    case "mods": return r.mod;
    default: return false;
  }
}

function matchAction(a, base, r, tags, text, cmd, args) {
  const t = a.trigger;
  if (!onPlatform(t, base.platform)) return null;
  if (t.type === "filter") {
    if (exempt(t.exempt, r) || !breaks(t, text, tags)) return null;
    return Object.assign({}, base, { rawInput: text }, words(text));
  } else if (t.type === "command") {
    const names = [t.command].concat(String(t.aliases || "").split(",")).map((x) => String(x || "").trim().toLowerCase()).filter(Boolean);
    if (!names.includes(cmd) || !allowed(t.who, r)) return null;
    return Object.assign({}, base, { command: cmd, rawInput: args, rawInputUrlEncoded: encodeURIComponent(args) }, words(args));
  } else if (t.type === "message") {
    const want = String(t.text || "").toLowerCase(), said = text.toLowerCase();
    let hit = false;
    if (t.match === "starts") hit = said.startsWith(want);
    else if (t.match === "exact") hit = said === want;
    else if (t.match === "regex") { try { hit = new RegExp(t.text || "", "i").test(text); } catch (e) {} }
    else hit = want !== "" && said.includes(want);
    if (!hit || !allowed(t.who, r)) return null;
    return Object.assign({}, base, { rawInput: text }, words(text));
  }
  return null;
}

function onActions(m, text, cmd, args, user, msgId) {
  const r = rank(m.tags);
  const months = /subscriber\/(\d+)/.exec(m.tags["badge-info"] || "");
  const base = { user, userId: m.tags["user-id"] || "", message: text, msgId, platform: m.platform || "twitch", isMod: r.mod, submonths: months ? months[1] : "" };
  for (const a of live) {
    const ctx = matchAction(a, base, r, m.tags, text, cmd, args);
    if (ctx && cooled(a, user)) run(a, ctx);
  }
}

// ---------- trying it: a line as a viewer would type it, what it'd answer and do, nothing sent or done ----------
const DRY_SAFE = new Set(["say", "random", "wait", "if", "ifblock", "else", "endif", "repeat", "endrepeat", "randomblock", "endrandom", "stop", "math", "randomnum", "text", "note", "run", "nowplaying", "quote", "status"]);
const DRY_SAYS = {
  timeout: "time them out", ban: "ban or unban them", delete: "delete their message", role: "change their role", shoutout: "shout them out", announce: "post an announcement",
  sound: "play a sound", speak: "say it out loud", overlay: "show it on stream", credits: "roll the credits", alert: "show an alert", request: "request a song", skip: "skip the song",
  playpause: "play or pause", points: "change their points", setvar: "set a variable", addvar: "add to a variable", quoteadd: "add a quote", giveenter: "enter them in the giveaway",
  givepick: "pick a giveaway winner", gamble: "gamble their points", slots: "spin the slots", duel: "start a duel", heist: "start a heist", trivia: "ask a trivia question",
  strike: "give them a strike", writefile: "write to a file", discord: "post to Discord", fetch: "fetch a web address", marker: "add a stream marker", clip: "make a clip",
  title: "change the title", game: "change the category", poll: "start a poll", predict: "start a prediction", ad: "run an ad", raid: "start a raid", chatmode: "change the chat mode",
  obsscene: "switch the OBS scene", obssource: "show or hide an OBS source", obstext: "change OBS text", obsmute: "mute or unmute in OBS", obsfilter: "switch an OBS filter",
  obsrecord: "start or stop recording", obsstream: "start or stop streaming", obsmedia: "control OBS media", obsbrowser: "change a browser source", obsreplay: "save a replay", obsshot: "take a screenshot", obsraw: "send OBS a request",
};

async function tryChat(text, platform, asMod) {
  const out = [];
  const t = String(text || "").trim();
  if (!t) return out;
  const p = platform === "kick" ? "kick" : "twitch";
  const user = "You";
  const space = t.indexOf(" ");
  const cmd = (space < 0 ? t : t.slice(0, space)).toLowerCase();
  const args = space < 0 ? "" : t.slice(space + 1).trim();
  if (S().requests !== false && t.startsWith("!")) {
    const rq = reqCommands();
    if (rq.sr.includes(cmd)) out.push(args ? "(would request \u201c" + args + "\u201d, and say what it found)" : "@" + user + " what song? " + cmd + " song by artist");
    else if (rq.song.includes(cmd)) { try { out.push((await OmniPulse.app.nowPlaying()).message); } catch (e) {} }
    else if (rq.wrong.includes(cmd)) out.push("(would take back your last request)");
  }
  const tags = { "display-name": user, badges: asMod ? "moderator/1" : "", mod: asMod ? "1" : "0" };
  const r = rank(tags);
  const base = { user, userId: "", message: t, msgId: "", platform: p, isMod: r.mod, submonths: "", dry: out };
  const jobs = [];
  for (const a of live) {
    const ctx = matchAction(a, base, r, tags, t, cmd, args);
    if (ctx) { ctx.dry = out; jobs.push(runNow(a, ctx, 0)); }
  }
  await Promise.race([Promise.all(jobs), new Promise((ok) => setTimeout(ok, 5000))]);
  return out;
}

// An event (EventSub's type), to the actions waiting for it
const EVENT_OF = {
  "channel.follow": "follow", "channel.subscribe": "sub", "channel.subscription.message": "resub",
  "channel.subscription.gift": "gift", "channel.cheer": "bits", "channel.raid": "raid",
  "channel.hype_train.begin": "hypetrain", "channel.poll.begin": "poll",
  "channel.channel_points_custom_reward_redemption.add": "reward",
  "first.message": "firstmsg", "stream.online": "online", "stream.offline": "offline", "channel.ad_break.begin": "adbreak",
  "channel.prediction.begin": "prediction", "channel.prediction.end": "predictionend", "channel.poll.end": "pollend",
  "channel.hype_train.end": "hypetrainend", "channel.charity_campaign.donate": "charity", "channel.shoutout.receive": "shoutoutin",
  "channel.vip.add": "vip", "channel.moderator.add": "mod", "watch.streak": "watchstreak",
};

// A trigger's platform (On: any, Twitch or Kick), and where this came from
function onPlatform(t, platform) {
  const want = t.from || "any";
  return want === "any" || platform === "all" || platform === want;
}

// An event (ours: follow, sub...), to the actions waiting for it
function fire(ev, ctx) {
  if (!ctx.replay) { credit(ev, ctx.user); feed(ev, ctx); suiteHear(ev, ctx); }
  ctx.fromEvent = ev;
  for (const x of live) {
    const t = x.trigger;
    if (t.type !== "event" || t.event !== ev || !onPlatform(t, ctx.platform)) continue;
    if (num(t.min) > 0 && num(ctx.amount) < num(t.min)) continue;
    if (String(t.users || "").trim() && !oneOf(ctx.user, t.users)) continue;
    // (by its id, or one of several: a Twitch reward's and a Kick reward's in one action)
    if (ev === "reward" && String(t.rewardid || "").trim()) { if (!String(t.rewardid).split(",").map((x) => x.trim()).filter(Boolean).includes(String(ctx.rewardId || ""))) continue; }
    else if (ev === "reward" && String(t.reward || "").trim() && String(t.reward).trim().toLowerCase() !== String(ctx.reward || "").toLowerCase()) continue;
    run(x, ctx);
  }
}

function onEvent(type, e) {
  const ev = EVENT_OF[type];
  if (!ev) return;
  if (ev === "online") { isLive = true; spoken.clear(); credits = {}; }
  if (ev === "offline") isLive = false;
  const a = asAlert(type, e) || {};
  if (ev === "sub" && e.is_gift) return;
  // (a poll's or a prediction's winner: its top choice, or the outcome that won)
  const top = (list) => (list || []).slice().sort((x, y) => num(y.votes || y.channel_points) - num(x.votes || x.channel_points))[0];
  const won = ev === "predictionend" ? ((e.outcomes || []).find((o) => o.id === e.winning_outcome_id) || {}).title || "" : ev === "pollend" ? (top(e.choices) || {}).title || "" : "";
  const amount = ev === "bits" ? String(e.bits || 0) : ev === "raid" ? String(e.viewers || 0) : ev === "gift" ? String(e.total || 1) : ev === "resub" ? String(e.cumulative_months || 1)
    : ev === "hypetrain" || ev === "hypetrainend" ? String(e.level || 1) : ev === "charity" ? String(e.amount ? num(e.amount.value) / Math.pow(10, num(e.amount.decimal_places)) : 0)
    : ev === "shoutoutin" ? String(e.viewer_count || 0) : ev === "watchstreak" ? String(e.streak || 0) : "";
  const reward = (e.reward && e.reward.title) || "";
  const rewardId = (e.reward && e.reward.id) || "";
  const said = (e.message && e.message.text) || e.title || "";
  const ctx = { user: a.user || e.user_name || e.from_broadcaster_user_name || "", userId: e.user_id || e.from_broadcaster_user_id || "", amount: ev === "adbreak" ? String(e.duration_seconds || 0) : amount, message: a.message || said || e.user_input || "", reward, rewardId, rawInput: e.user_input || said || "", msgId: e.msg_id || "", platform: "twitch", winner: won, tier: String(e.tier || "") };
  Object.assign(ctx, words(ctx.rawInput));
  fire(ev, ctx);
}

// Every few minutes: each timed action, if chat's said enough since
function setTimers() {
  timers.forEach((id) => clearInterval(id));
  timers = [];
  for (const a of live) {
    if (a.trigger.type === "presence") {
      timers.push(setInterval(() => {
        const since = Date.now() - Math.max(1, num(a.trigger.within) || 15) * 60000;
        for (const p of Object.values(present)) if (p.at >= since) run(a, { user: p.user, userId: p.userId, platform: p.platform });
      }, Math.max(1, num(a.trigger.minutes) || 10) * 60000));
      continue;
    }
    if (a.trigger.type !== "timer") continue;
    let seen = chatLines;
    timers.push(setInterval(() => {
      if ((!chat && !kick) || chatLines - seen < num(a.trigger.lines)) return;
      seen = chatLines;
      run(a, { user: "", platform: "all" });
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
    if (k === "points") return String(pointsOf(ctx.user));
    if (k === "pointsname") return pointsName();
    if (k === "rank") return rankOf(ctx.user);
    if (k === "credits") return creditsLine();
    if (k in ctx) return String(ctx[k] == null ? "" : ctx[k]);
    return k.startsWith("input") ? "" : whole;
  });
}

// A name in a list (commas between), as it's written or with its @
function oneOf(who, list) {
  const w = String(who || "").replace("@", "").trim().toLowerCase();
  return !!w && String(list || "").split(",").some((x) => x.replace("@", "").trim().toLowerCase() === w);
}

function test(left, op, right) {
  const l = String(left).trim(), r = String(right).trim();
  switch (op) {
    case "oneof": return oneOf(l, r);
    case "is": return l.toLowerCase() === r.toLowerCase();
    case "isnt": return l.toLowerCase() !== r.toLowerCase();
    case "contains": return l.toLowerCase().includes(r.toLowerCase());
    case "more": return num(l) > num(r);
    case "less": return num(l) < num(r);
    case "empty": return l === "";
    default: return l !== "";
  }
}

// An action's steps as blocks: If ... Otherwise ... End If, Repeat ... End Repeat, Pick One at Random ...
// End Pick (one left open runs to the end)
const OPENS = { ifblock: "endif", repeat: "endrepeat", randomblock: "endrandom" };
function blocks(steps, from, until) {
  const out = [];
  let i = from;
  while (i < steps.length) {
    const st = steps[i];
    if (until && (st.type === until || (until === "endif" && st.type === "else"))) return { list: out, at: i };
    if (OPENS[st.type]) {
      const inner = blocks(steps, i + 1, OPENS[st.type]);
      const node = { st, then: inner.list, otherwise: [] };
      i = inner.at;
      if (st.type === "ifblock" && steps[i] && steps[i].type === "else") {
        const rest = blocks(steps, i + 1, "endif");
        node.otherwise = rest.list.filter((x) => x.st.type !== "else");
        i = rest.at;
      }
      out.push(node);
      i++;
      continue;
    }
    if (!["else", "endif", "endrepeat", "endrandom"].includes(st.type)) out.push({ st });
    i++;
  }
  return { list: out, at: i };
}

// An action's steps, in order (a step that says stop, stops it). Queued (One at a Time): after the one
// before it's finished
let queueTail = Promise.resolve();
function run(a, ctx, depth) {
  if (a.queued && !depth) {
    const go = queueTail.then(() => runNow(a, ctx, 0));
    queueTail = go.catch(() => {});
    return go;
  }
  return runNow(a, ctx, depth);
}

async function runNow(a, ctx, depth) {
  depth = depth || 0;
  if (depth > 5) return;
  if (!ctx.action) ctx.action = a.name;
  if (!depth && !ctx.dry) counts.actions++;
  try {
    await runBlocks(blocks(a.steps || [], 0, null).list, ctx, depth);
    if (!depth && !ctx.dry) noted(ctx.user, ctx.command ? ctx.command + (ctx.rawInput ? " " + ctx.rawInput : "") : ctx.fromEvent || (a.trigger && a.trigger.type) || "", a.name + (ctx._said ? ": " + ctx._said : ""), ctx.platform);
  } catch (e) {
    if (e === STOP) return;
    OmniPulse.log(a.name + ": " + e.message);
    OmniPulse.app.say(a.name + ": " + e.message).catch(() => {});
  }
}

const STOP = { stop: true };
async function runBlocks(list, ctx, depth) {
  for (const n of list) {
    const st = n.st;
    const f = (k) => fill(st[k], ctx);
    if (st.type === "ifblock") { await runBlocks(test(f("left"), st.test, f("right")) ? n.then : n.otherwise, ctx, depth); continue; }
    if (st.type === "repeat") {
      const times = Math.min(100, Math.max(0, Math.floor(num(f("times")))));
      for (let k = 1; k <= times; k++) { ctx.loop = String(k); await runBlocks(n.then, ctx, depth); }
      continue;
    }
    if (st.type === "randomblock") { if (n.then.length) await runBlocks([n.then[Math.floor(Math.random() * n.then.length)]], ctx, depth); continue; }
    if (st.type === "stop") throw STOP;
    if ((await step(st, ctx, depth)) === false) throw STOP;
  }
}

// A step's "As": the variable its answer's kept in (for this run), %name%
function name(v, dflt) { return String(v || dflt).replace(/%/g, "").trim() || dflt; }

// A sum, worked out safely (numbers, + - * / % ^, brackets, and a few functions): never run as code
function maths(text) {
  const t = String(text).replace(/\s+/g, "");
  let i = 0;
  const FN = { round: Math.round, floor: Math.floor, ceil: Math.ceil, abs: Math.abs, sqrt: Math.sqrt, min: Math.min, max: Math.max,
    random: (a, b) => { const lo = Math.ceil(num(a)), hi = Math.floor(b === undefined ? num(a) : num(b)); return b === undefined ? Math.floor(Math.random() * lo) + 1 : lo + Math.floor(Math.random() * (hi - lo + 1)); } };
  const fail = () => { throw new Error("that sum (" + text + ") isn't one I can do"); };
  const atom = () => {
    if (t[i] === "(") { i++; const v = sum(); if (t[i] !== ")") fail(); i++; return v; }
    if (t[i] === "-") { i++; return -atom(); }
    if (t[i] === "+") { i++; return atom(); }
    const name = /^[a-z]+/i.exec(t.slice(i));
    if (name) {
      i += name[0].length;
      const fn = FN[name[0].toLowerCase()];
      if (!fn || t[i] !== "(") fail();
      i++;
      const args = [];
      if (t[i] !== ")") { args.push(sum()); while (t[i] === ",") { i++; args.push(sum()); } }
      if (t[i] !== ")") fail();
      i++;
      return fn(...args);
    }
    const n = /^\d*\.?\d+(e[+-]?\d+)?/i.exec(t.slice(i));
    if (!n) fail();
    i += n[0].length;
    return parseFloat(n[0]);
  };
  const power = () => { const b = atom(); if (t[i] === "^") { i++; return Math.pow(b, power()); } return b; };
  const product = () => { let v = power(); while (t[i] === "*" || t[i] === "/" || t[i] === "%") { const o = t[i++]; const r = power(); v = o === "*" ? v * r : o === "/" ? v / r : v % r; } return v; };
  const sum = () => { let v = product(); while (t[i] === "+" || t[i] === "-") { const o = t[i++]; const r = product(); v = o === "+" ? v + r : v - r; } return v; };
  const v = sum();
  if (i !== t.length || !isFinite(v)) fail();
  return Math.round(v * 1e6) / 1e6;
}

// How long ago a date was, in words ("2 years, 3 months")
function since(date) {
  const d = new Date(date);
  if (isNaN(d)) return "";
  const now = new Date();
  let months = (now.getFullYear() - d.getFullYear()) * 12 + (now.getMonth() - d.getMonth());
  if (now.getDate() < d.getDate()) months--;
  const years = Math.floor(months / 12);
  months %= 12;
  const days = Math.floor((now - d) / 86400000);
  const n = (k, w) => k + " " + w + (k === 1 ? "" : "s");
  if (years > 0) return n(years, "year") + (months ? ", " + n(months, "month") : "");
  if (months > 0) return n(months, "month");
  return n(Math.max(0, days), "day");
}

async function step(st, ctx, depth) {
  const f = (k) => fill(st[k], ctx);
  // (Send To: where it came from, or Twitch, Kick or both; a reply only in the chat it's replying to)
  const where = st.where || "here";
  const sendTo = where === "here" ? ctx.platform : where === "both" ? "all" : where;
  const say = (to, text) => { if (!text) return; ctx._said = String(text).slice(0, 100); if (ctx.dry) ctx.dry.push(String(text)); else reply(sendTo === ctx.platform ? to : null, text, sendTo); };
  if (ctx.dry && !DRY_SAFE.has(st.type)) { ctx.dry.push("(would " + (DRY_SAYS[st.type] || "do its " + st.type + " step") + ")"); return true; }
  if (ctx.dry && st.type === "wait") return true;
  const onKick = ctx.platform === "kick";
  // (Twitch's own steps, not Kick's: left out there)
  if (onKick && ["shoutout", "announce", "marker", "clip"].includes(st.type)) { OmniPulse.log(st.type + ": Twitch's own, not done on Kick"); return true; }
  const to = yes(st.reply) ? ctx.msgId : null;
  switch (st.type) {
    case "say": say(to, f("text")); break;
    case "random": {
      const l = f("lines").split("|").map((x) => x.trim()).filter(Boolean);
      if (l.length) say(null, l[Math.floor(Math.random() * l.length)]);
      break;
    }
    case "wait": await new Promise((ok) => setTimeout(ok, Math.min(3600, Math.max(0, num(f("seconds")))) * 1000)); break;
    case "sound": await OmniPulse.app.sound(f("file").trim().replace(/^"|"$/g, ""), num(f("volume"))); break;
    case "speak": await OmniPulse.app.speak(f("text").slice(0, 500), f("voice").trim(), Math.min(3, Math.max(0.5, num(f("rate")) || 1)), num(f("volume"))); break;
    case "overlay": await OmniPulse.app.overlay({ text: f("text").slice(0, 300), image: f("image").trim().replace(/^"|"$/g, ""), video: f("video").trim().replace(/^"|"$/g, ""), place: st.place || "middle", seconds: Math.min(120, Math.max(1, num(f("seconds")) || 6)) }); break;
    case "if": return test(f("left"), st.test, f("right"));
    case "math": ctx[name(st.into, "answer")] = String(maths(f("expr"))); break;
    case "randomnum": {
      const lo = Math.ceil(num(f("min"))), hi = Math.floor(num(f("max")));
      ctx[name(st.into, "number")] = String(hi < lo ? lo : lo + Math.floor(Math.random() * (hi - lo + 1)));
      break;
    }
    case "text": {
      const v = f("value"), w = v.trim().split(/\s+/).filter(Boolean);
      const out = { upper: v.toUpperCase(), lower: v.toLowerCase(), title: v.toLowerCase().replace(/(^|\s)\S/g, (c) => c.toUpperCase()), trim: v.trim(),
        length: String(v.length), first: w[0] || "", last: w[w.length - 1] || "", word: w[Math.max(0, Math.floor(num(f("n"))) - 1)] || "",
        replace: f("n") ? v.split(f("n")).join(f("with")) : v, rest: w.slice(1).join(" ") }[st.do || "upper"];
      ctx[name(st.into, "text")] = out === undefined ? v : out;
      break;
    }
    case "userinfo": {
      if (onKick || ctx.platform === "kick") { OmniPulse.log("Look Up the Viewer: Twitch's, not Kick's yet"); break; }
      const login = f("who").replace("@", "").trim().toLowerCase();
      if (!login) break;
      const u = (await helix("GET", "users?login=" + encodeURIComponent(login))).data || [];
      if (!u[0]) throw new Error("there's no Twitch account called " + login);
      ctx.displayname = u[0].display_name || login;
      ctx.accountdate = String(u[0].created_at || "").slice(0, 10);
      ctx.accountage = since(u[0].created_at);
      const fo = ((await helix("GET", "channels/followers?broadcaster_id=" + S().userId + "&user_id=" + u[0].id)).data || [])[0];
      ctx.followdate = fo ? String(fo.followed_at || "").slice(0, 10) : "";
      ctx.followage = fo ? since(fo.followed_at) : "";
      let sub = null;
      try { sub = ((await helix("GET", "subscriptions?broadcaster_id=" + S().userId + "&user_id=" + u[0].id)).data || [])[0]; } catch (e) { sub = null; }
      ctx.subtier = sub ? tierName(sub.tier) : "";
      break;
    }
    case "setvar": setVar(fill(st.name, ctx).trim(), f("value"), yes(st.per), yes(st.keep), ctx.user); break;
    case "addvar": {
      const n = fill(st.name, ctx).trim();
      const was = num(getVar(n, yes(st.per) ? ctx.user : ""));
      setVar(n, was + num(f("amount")), yes(st.per), yes(st.keep), ctx.user);
      break;
    }
    case "request": {
      const r = await OmniPulse.app.request(ctx.user || S().login || "", f("query"), onKick ? "kick" : "twitch");
      ctx.reply = r.message;
      if (yes(st.reply)) say(ctx.msgId, r.message);
      break;
    }
    case "wrongsong": {
      const r = await OmniPulse.app.wrongSong(ctx.user || S().login || "", !!ctx.isMod);
      ctx.reply = r.message;
      if (st.say !== false && st.say !== "false") say(ctx.msgId, r.message);
      break;
    }
    case "nowplaying": {
      ctx.reply = (await OmniPulse.app.nowPlaying()).message;
      if (st.say !== false && st.say !== "false") say(ctx.msgId, ctx.reply);
      break;
    }
    case "fetch": {
      const r = await OmniPulse.app.fetchText(f("url").trim());
      ctx[String(st.into || "reply").replace(/%/g, "").trim() || "reply"] = r.text;
      break;
    }
    case "skip": await OmniPulse.app.control("next"); break;
    case "playpause": await OmniPulse.app.control("play"); break;
    case "alert": await OmniPulse.app.alert({ type: st.alert || "follow", user: f("user"), amount: f("amount"), message: f("message"), feed: ctx.fromEvent ? false : undefined }); break;
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
    case "delete": {
      if (!ctx.msgId) break;
      await helix("DELETE", "moderation/chat?broadcaster_id=" + S().userId + "&moderator_id=" + S().userId + "&message_id=" + encodeURIComponent(ctx.msgId));
      break;
    }
    case "announce": await helix("POST", "chat/announcements?broadcaster_id=" + S().userId + "&moderator_id=" + S().userId, { message: f("text").slice(0, 500), color: st.colour || "primary" }); break;
    case "marker": await helix("POST", "streams/markers", { user_id: S().userId, description: f("text").slice(0, 140) }); break;
    case "clip": {
      const c = await helix("POST", "clips?broadcaster_id=" + S().userId);
      const id = c.data && c.data[0] && c.data[0].id;
      if (!id) throw new Error("Twitch didn't make the clip (are you live?)");
      ctx.clip = "https://clips.twitch.tv/" + id;
      break;
    }
    case "title": if (onKick) { await kickApi("PATCH", "channels", { stream_title: f("text").slice(0, 140) }); break; } await helix("PATCH", "channels?broadcaster_id=" + S().userId, { title: f("text").slice(0, 140) }); break;
    case "game": {
      const where = st.where || "here";
      const toKick = where === "kick" || where === "both" || (where === "here" && onKick);
      const toTwitch = where === "twitch" || where === "both" || (where === "here" && !onKick);
      if (toKick) {
        const c = await kickApi("GET", "categories?q=" + encodeURIComponent(f("text").trim()));
        const id = c.data && c.data[0] && c.data[0].id;
        if (!id) throw new Error("Kick has no category called " + f("text"));
        await kickApi("PATCH", "channels", { category_id: id });
      }
      if (toTwitch) {
        const g = await helix("GET", "games?name=" + encodeURIComponent(f("text").trim()));
        const id = g.data && g.data[0] && g.data[0].id;
        if (!id) throw new Error("Twitch has no category called " + f("text"));
        await helix("PATCH", "channels?broadcaster_id=" + S().userId, { game_id: id });
      }
      break;
    }
    case "poll": {
      const choices = f("choices").split("|").map((x) => ({ title: x.trim().slice(0, 25) })).filter((x) => x.title).slice(0, 5);
      if (choices.length < 2) throw new Error("a poll needs two choices at least");
      await helix("POST", "polls", { broadcaster_id: S().userId, title: f("title").slice(0, 60) || "Poll", choices, duration: Math.min(1800, Math.max(15, num(f("seconds")) || 60)) });
      break;
    }
    case "pollend": {
      const p = ((await helix("GET", "polls?broadcaster_id=" + S().userId + "&first=1")).data || [])[0];
      if (!p || p.status !== "ACTIVE") throw new Error("there's no poll going");
      await helix("PATCH", "polls", { broadcaster_id: S().userId, id: p.id, status: "TERMINATED" });
      break;
    }
    case "predict": {
      const outcomes = f("outcomes").split("|").map((x) => ({ title: x.trim().slice(0, 25) })).filter((x) => x.title).slice(0, 10);
      if (outcomes.length < 2) throw new Error("a prediction needs two outcomes at least");
      await helix("POST", "predictions", { broadcaster_id: S().userId, title: f("title").slice(0, 45) || "Prediction", outcomes, prediction_window: Math.min(1800, Math.max(30, num(f("seconds")) || 120)) });
      break;
    }
    case "predictend": {
      const p = ((await helix("GET", "predictions?broadcaster_id=" + S().userId + "&first=1")).data || [])[0];
      if (!p || !["ACTIVE", "LOCKED"].includes(p.status)) throw new Error("there's no prediction going");
      if (st.what === "lock") { await helix("PATCH", "predictions", { broadcaster_id: S().userId, id: p.id, status: "LOCKED" }); break; }
      if (st.what === "cancel") { await helix("PATCH", "predictions", { broadcaster_id: S().userId, id: p.id, status: "CANCELED" }); break; }
      const w = f("winner").trim().toLowerCase();
      const o = (p.outcomes || []).find((x) => String(x.title).toLowerCase() === w) || (p.outcomes || []).find((x) => String(x.title).toLowerCase().startsWith(w) && w);
      if (!o) throw new Error("the prediction has no outcome called " + f("winner"));
      await helix("PATCH", "predictions", { broadcaster_id: S().userId, id: p.id, status: "RESOLVED", winning_outcome_id: o.id });
      break;
    }
    case "reward": {
      const title = f("title").trim();
      if (st.what === "create") { await helix("POST", "channel_points/custom_rewards?broadcaster_id=" + S().userId, { title: title.slice(0, 45), cost: Math.max(1, Math.round(num(f("cost")) || 500)) }); break; }
      const all = (await helix("GET", "channel_points/custom_rewards?broadcaster_id=" + S().userId + "&only_manageable_rewards=true")).data || [];
      const r = all.find((x) => String(x.title).toLowerCase() === title.toLowerCase());
      if (!r) throw new Error("there's no reward called " + title + " that OmniPulse made (Twitch only lets an app change its own)");
      const set = { enable: { is_enabled: true }, disable: { is_enabled: false }, pause: { is_paused: true }, unpause: { is_paused: false }, cost: { cost: Math.max(1, Math.round(num(f("cost")))) } }[st.what || "enable"];
      await helix("PATCH", "channel_points/custom_rewards?broadcaster_id=" + S().userId + "&id=" + r.id, set);
      break;
    }
    case "ad": await helix("POST", "channels/commercial", { broadcaster_id: S().userId, length: Math.min(180, Math.max(30, Math.round(num(f("seconds")) / 30) * 30 || 60)) }); break;
    case "raid": {
      if (st.what === "cancel") { await helix("DELETE", "raids?broadcaster_id=" + S().userId); break; }
      const id = await twitchUserId(f("who"));
      await helix("POST", "raids?from_broadcaster_id=" + S().userId + "&to_broadcaster_id=" + id);
      break;
    }
    case "chatmode": {
      const on = yes(st.on), wait = Math.max(0, Math.round(num(f("seconds"))));
      const body = { emote: { emote_mode: on }, subscriber: { subscriber_mode: on }, unique: { unique_chat_mode: on },
        slow: on ? { slow_mode: true, slow_mode_wait_time: Math.min(120, Math.max(3, wait || 10)) } : { slow_mode: false },
        follower: on ? { follower_mode: true, follower_mode_duration: Math.min(129600, wait) } : { follower_mode: false } }[st.mode || "emote"];
      await helix("PATCH", "chat/settings?broadcaster_id=" + S().userId + "&moderator_id=" + S().userId, body);
      break;
    }
    case "ban": {
      const who = f("who").replace("@", "").trim();
      if (!who) break;
      if (onKick) {
        if (who.toLowerCase() !== String(ctx.user || "").toLowerCase() || !ctx.userId) throw new Error("on Kick, only whoever started it can be banned or unbanned (Kick's API has no look-up by name)");
        if (st.what === "unban") await kickApi("DELETE", "moderation/bans", { broadcaster_user_id: Number(S().kickUserId), user_id: Number(ctx.userId) });
        else await kickApi("POST", "moderation/bans", { broadcaster_user_id: Number(S().kickUserId), user_id: Number(ctx.userId), reason: f("reason") || undefined });
        break;
      }
      const id = await twitchUserId(who);
      if (st.what === "unban") await helix("DELETE", "moderation/bans?broadcaster_id=" + S().userId + "&moderator_id=" + S().userId + "&user_id=" + id);
      else await helix("POST", "moderation/bans?broadcaster_id=" + S().userId + "&moderator_id=" + S().userId, { data: { user_id: id, reason: f("reason") } });
      break;
    }
    case "role": {
      const id = await twitchUserId(f("who"));
      const q = "?broadcaster_id=" + S().userId + "&user_id=" + id;
      const path = { vip: ["POST", "channels/vips"], unvip: ["DELETE", "channels/vips"], mod: ["POST", "moderation/moderators"], unmod: ["DELETE", "moderation/moderators"] }[st.what || "vip"];
      await helix(path[0], path[1] + q);
      break;
    }
    case "discord": {
      const body = { content: f("text").slice(0, 2000) };
      if (f("name").trim()) body.username = f("name").trim().slice(0, 80);
      await OmniPulse.app.postJson(f("webhook").trim(), body);
      break;
    }
    case "obsmedia": await obsCall("TriggerMediaInputAction", { inputName: f("source"), mediaAction: "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_" + String(st.what || "restart").toUpperCase() }); break;
    case "obsbrowser": await obsCall("SetInputSettings", { inputName: f("source"), inputSettings: { url: f("url") } }); break;
    case "obsreplay": await obsCall("SaveReplayBuffer"); break;
    case "obsshot": await obsCall("SaveSourceScreenshot", { sourceName: f("source"), imageFormat: "png", imageFilePath: f("file") }); break;
    case "obsraw": {
      let data = {};
      try { data = JSON.parse(f("data") || "{}"); } catch (e) { throw new Error("its request data isn't JSON"); }
      ctx.obs = JSON.stringify(await obsCall(f("request").trim(), data)).slice(0, 500);
      break;
    }
    case "obsscene": await obsCall("SetCurrentProgramScene", { sceneName: f("scene") }); break;
    case "obssource": {
      const sceneName = f("scene") || (await obsCall("GetCurrentProgramScene")).currentProgramSceneName;
      const sceneItemId = (await obsCall("GetSceneItemId", { sceneName, sourceName: f("source") })).sceneItemId;
      const was = (await obsCall("GetSceneItemEnabled", { sceneName, sceneItemId })).sceneItemEnabled;
      const on = st.show === "toggle" ? !was : st.show !== "hide";
      await obsCall("SetSceneItemEnabled", { sceneName, sceneItemId, sceneItemEnabled: on });
      const back = num(f("for"));
      if (back > 0) setTimeout(() => { obsCall("SetSceneItemEnabled", { sceneName, sceneItemId, sceneItemEnabled: was }).catch(() => {}); }, Math.min(3600, back) * 1000);
      break;
    }
    case "obstext": await obsCall("SetInputSettings", { inputName: f("source"), inputSettings: { text: f("text") } }); break;
    case "obsmute": {
      if (st.what === "toggle") await obsCall("ToggleInputMute", { inputName: f("source") });
      else await obsCall("SetInputMute", { inputName: f("source"), inputMuted: st.what !== "unmute" });
      break;
    }
    case "obsfilter": await obsCall("SetSourceFilterEnabled", { sourceName: f("source"), filterName: f("filter"), filterEnabled: st.what !== "off" }); break;
    case "obsrecord": await obsCall(st.what === "stop" ? "StopRecord" : st.what === "toggle" ? "ToggleRecord" : "StartRecord"); break;
    case "obsstream": await obsCall(st.what === "stop" ? "StopStream" : st.what === "toggle" ? "ToggleStream" : "StartStream"); break;
    case "quoteadd": {
      const text = f("text").trim();
      if (!text) break;
      const list = (S().quotes || []).slice();
      const d = new Date();
      list.push({ text, by: ctx.user || "", date: d.getFullYear() + "-" + String(d.getMonth() + 1).padStart(2, "0") + "-" + String(d.getDate()).padStart(2, "0") });
      OmniPulse.settings.quotes = list;
      await OmniPulse.app.saveSettings({ quotes: list });
      ctx.quotenumber = String(list.length);
      break;
    }
    case "quote": {
      const list = S().quotes || [];
      if (!list.length) { ctx.quote = "No quotes yet."; break; }
      const n = parseInt(f("number"), 10);
      const i = n >= 1 && n <= list.length ? n - 1 : Math.floor(Math.random() * list.length);
      ctx.quote = "#" + (i + 1) + ": \"" + list[i].text + "\" (" + list[i].date + ")";
      ctx.quotenumber = String(i + 1);
      break;
    }
    case "points": {
      const who = f("who").replace("@", "").trim();
      const n = Math.round(num(f("amount")));
      if (!who) break;
      if (n < 0 && pointsOf(who) < -n) { say(ctx.msgId, "@" + who + ", that takes " + -n + " " + pointsName() + " (you have " + pointsOf(who) + ")."); return false; }
      addPoints(who, n);
      break;
    }
    case "gamble":
    case "slots": {
      const bet = betOf(f("bet"), ctx.user, num(f("least")), say, ctx);
      if (bet === null) return false;
      let pays = 0;
      if (st.type === "gamble") pays = Math.random() * 100 < num(f("chance")) ? num(f("payout")) : 0;
      else {
        const sym = f("symbols").split(/\s+/).filter(Boolean);
        const reels = [0, 1, 2].map(() => sym[Math.floor(Math.random() * sym.length)] || "?");
        ctx.reels = reels.join(" ");
        pays = reels[0] === reels[1] && reels[1] === reels[2] ? num(f("three")) : reels[0] === reels[1] || reels[1] === reels[2] || reels[0] === reels[2] ? num(f("two")) : 0;
      }
      const winnings = Math.round(bet * pays);
      addPoints(ctx.user, winnings - bet);
      Object.assign(ctx, { bet: String(bet), winnings: String(winnings), won: winnings > bet ? "yes" : "no" });
      say(ctx.msgId, f(winnings > bet ? "win" : "lose"));
      break;
    }
    case "duel": {
      const target = f("who").replace("@", "").trim();
      if (!target || target.toLowerCase() === String(ctx.user).toLowerCase()) break;
      const bet = betOf(f("bet"), ctx.user, 1, say, ctx);
      if (bet === null) return false;
      if (pointsOf(target) < bet) { say(ctx.msgId, "@" + ctx.user + ", " + target + " hasn't " + bet + " " + pointsName() + "."); return false; }
      const secs = Math.min(300, Math.max(5, num(f("seconds")) || 30));
      const c = Object.assign({}, ctx, { target, bet: String(bet), accept: f("accept").trim().toLowerCase() || "!accept", seconds: String(secs) });
      const key = c.platform + "|" + target.toLowerCase();
      say(null, fill(st.ask, c));
      duels[key] = { from: ctx.user, bet, word: c.accept, go: () => {
        delete duels[key];
        if (pointsOf(ctx.user) < bet || pointsOf(target) < bet) return;
        const first = Math.random() < 0.5;
        c.winner = first ? ctx.user : target; c.loser = first ? target : ctx.user;
        addPoints(c.winner, bet); addPoints(c.loser, -bet);
        say(null, fill(st.win, c));
      }, off: setTimeout(() => { if (duels[key]) { delete duels[key]; say(null, fill(st.off, c)); } }, secs * 1000) };
      break;
    }
    case "heist": {
      const bet = betOf(f("bet"), ctx.user, num(f("least")), say, ctx);
      if (bet === null) return false;
      const key = ctx.action || "heist";
      const h = heists[key];
      if (h) {
        if (h.crew.some((x) => x.user.toLowerCase() === String(ctx.user).toLowerCase())) break;
        addPoints(ctx.user, -bet);
        h.crew.push({ user: ctx.user, bet });
        break;
      }
      const secs = Math.min(600, Math.max(10, num(f("seconds")) || 60));
      const c = Object.assign({}, ctx, { seconds: String(secs) });
      addPoints(ctx.user, -bet);
      heists[key] = { crew: [{ user: ctx.user, bet }] };
      say(null, fill(st.start, c));
      setTimeout(() => {
        const crew = (heists[key] || { crew: [] }).crew;
        delete heists[key];
        const out = [], caught = [];
        for (const x of crew) {
          if (Math.random() * 100 < num(fill(st.chance, c))) { const w = Math.round(x.bet * num(fill(st.payout, c))); addPoints(x.user, w); out.push(x.user + " (" + w + ")"); }
          else caught.push(x.user);
        }
        c.winners = out.join(", ") || "no one"; c.losers = caught.join(", ") || "no one";
        say(null, fill(st.end, c));
      }, secs * 1000);
      break;
    }
    case "trivia": {
      if (trivia) break;
      const r = await OmniPulse.fetch("https://opentdb.com/api.php?amount=1&type=multiple&encode=url3986");
      const q = r.ok ? ((await r.json()).results || [])[0] : null;
      if (!q) throw new Error("the Open Trivia Database didn't answer");
      const d = (x) => decodeURIComponent(String(x || ""));
      const choices = q.incorrect_answers.map(d).concat([d(q.correct_answer)]).sort(() => Math.random() - 0.5);
      const letter = "ABCD"[choices.indexOf(d(q.correct_answer))];
      const secs = Math.min(300, Math.max(5, num(f("seconds")) || 30));
      const c = Object.assign({}, ctx, { question: d(q.question), choices: choices.map((x, i) => "ABCD"[i] + ") " + x).join("  "), answer: letter + ") " + d(q.correct_answer), prize: String(Math.round(num(f("prize")))), seconds: String(secs) });
      say(null, fill(st.ask, c));
      trivia = { right: [letter.toLowerCase(), d(q.correct_answer).toLowerCase()], platform: ctx.platform, won: (who) => {
        clearTimeout(trivia.off); trivia = null;
        c.winner = who; addPoints(who, num(c.prize));
        say(null, fill(st.win, c));
      }, off: setTimeout(() => { trivia = null; say(null, fill(st.none, c)); }, secs * 1000) };
      break;
    }
    case "strike": {
      const who = f("who").replace("@", "").trim();
      if (!who) break;
      const k = who.toLowerCase(), all = Object.assign({}, S().strikes || {});
      const had = all[k] && Date.now() - all[k].at < Math.max(1, num(f("forget")) || 24) * 3600000 ? all[k].n : 0;
      const n = had + 1;
      all[k] = { n, at: Date.now() };
      OmniPulse.settings.strikes = all;
      OmniPulse.app.saveSettings({ strikes: all }).catch(() => {});
      ctx.strikes = String(n);
      const banAt = num(f("banat")), outAt = num(f("timeoutat"));
      const c = Object.assign({}, ctx, { user: who });
      if (banAt > 0 && n >= banAt) await step(Object.assign({}, st, { type: "ban", what: "ban", who, reason: f("reason") || "strike " + n }), c, depth);
      else if (outAt > 0 && n >= outAt) {
        if (onKick) await step(Object.assign({}, st, { type: "ban", what: "ban", who, reason: f("reason") }), c, depth);
        else await helix("POST", "moderation/bans?broadcaster_id=" + S().userId + "&moderator_id=" + S().userId, { data: { user_id: await twitchUserId(who), duration: Math.max(1, num(f("seconds"))), reason: f("reason") || "strike " + n } });
      } else say(null, f("warn"));
      break;
    }
    case "writefile": await OmniPulse.app.writeFile(f("file").trim().replace(/^"|"$/g, ""), f("text"), st.how === "append"); break;
    case "credits": await OmniPulse.app.overlay({ text: creditsRoll(f("title")), place: "roll", seconds: Math.min(120, Math.max(5, num(f("seconds")) || 25)) }); break;
    case "giveenter": {
      if (ctx.user && !entries.some((x) => x.toLowerCase() === ctx.user.toLowerCase())) entries.push(ctx.user);
      break;
    }
    case "givepick": {
      ctx.entries = String(entries.length);
      ctx.winner = entries.length ? entries[Math.floor(Math.random() * entries.length)] : "no one (no one entered)";
      if (yes(st.clear)) entries = [];
      break;
    }
    case "run": {
      const name = f("action").trim().toLowerCase();
      const next = all.find((x) => String(x.name || "").toLowerCase() === name);
      if (next) await run(next, ctx, depth + 1);
      break;
    }
    case "status": await OmniPulse.app.say(f("text")); break;
    // (the fun's steps: a free spin for whoever started it; hype added; a burst of emotes)
    case "spin": await wheelSpin(ctx.user || "", ctx.msgId || null, ctx.platform, true); break;
    case "hypeadd": hypeAdd(Math.max(0, num(f("amount")))); break;
    case "emoterain": fun.burst++; suitePush(); break;
    case "groupstate": {
      const g = f("group").trim().toLowerCase();
      const list = (S().actions || []).map((x) => {
        if (String(x.group || "").trim().toLowerCase() !== g) return x;
        return Object.assign({}, x, { on: st.what === "toggle" ? x.on === false : st.what !== "off" });
      });
      OmniPulse.settings.actions = list;
      loadActions();
      await OmniPulse.app.saveSettings({ actions: list });
      break;
    }
    case "keys": await OmniPulse.app.pressKeys(String(st.keys || "")); break;
    case "open": await OmniPulse.app.openThing(String(st.target || ""), String(st.args || "")); break;
  }
  return true;
}

// ---------- OBS: its own WebSocket server (obs-websocket 5, OBS 28 and later), its password ----------
let obs = null, obsReady = false, obsN = 0;
const obsWaiting = {};

// SHA-256 (OBS's sign-in: the script has no crypto of its own), of bytes, as bytes
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

// What OBS asks for to sign in: base64(sha256(base64(sha256(password + salt)) + challenge))
function obsAuth(password, salt, challenge) {
  return b64(sha256(utf8(b64(sha256(utf8(password + salt))) + challenge)));
}

function obsConnect() {
  if (!yes(S().obsOn)) return;
  const addr = String(S().obsAddress || "127.0.0.1:4455").trim().replace(/^wss?:\/\//, "");
  const s = OmniPulse.socket("ws://" + addr);
  obs = s;
  obsReady = false;
  s.onmessage = (data) => {
    let m;
    try { m = JSON.parse(data); } catch (e) { return; }
    if (m.op === 0) {
      // (its scenes' events and its outputs')
      const d = { rpcVersion: 1, eventSubscriptions: (1 << 2) | (1 << 6) };
      const a = m.d && m.d.authentication;
      if (a) d.authentication = obsAuth(String(S().obsPassword || ""), a.salt, a.challenge);
      s.send(JSON.stringify({ op: 1, d }));
    } else if (m.op === 2) {
      obsReady = true;
    } else if (m.op === 5) {
      onObs(m.d.eventType, m.d.eventData || {});
    } else if (m.op === 7) {
      const w = obsWaiting[m.d.requestId];
      if (!w) return;
      delete obsWaiting[m.d.requestId];
      if (m.d.requestStatus && m.d.requestStatus.result) w.ok(m.d.responseData || {});
      else w.no(new Error("OBS said: " + ((m.d.requestStatus && m.d.requestStatus.comment) || "no")));
    }
  };
  s.onclose = (why) => {
    obsReady = false;
    if (obs !== s) return;
    obs = null;
    // (OBS closed, or its password's wrong: tried again a little later while the chatbot runs)
    if (String(why).includes("4009")) { OmniPulse.app.say("OBS didn't take the password in the chatbot's settings.").catch(() => {}); return; }
    if (!stopped) setTimeout(() => { if (!obs && !stopped) obsConnect(); }, 15000);
  };
}

function obsCall(requestType, requestData) {
  if (!obs || !obsReady) return Promise.reject(new Error("OBS isn't connected: switch it on in the chatbot's settings, with OBS open (Tools > WebSocket Server Settings: Enable)"));
  const requestId = "r" + ++obsN;
  return new Promise((ok, no) => {
    obsWaiting[requestId] = { ok, no };
    obs.send(JSON.stringify({ op: 6, d: { requestType, requestId, requestData: requestData || {} } }));
    setTimeout(() => { if (obsWaiting[requestId]) { delete obsWaiting[requestId]; no(new Error("OBS didn't answer")); } }, 5000);
  });
}

// OBS's events, to the actions waiting for them
function onObs(type, d) {
  let when = "", scene = d.sceneName || "";
  if (type === "CurrentProgramSceneChanged") when = "scene";
  else if (type === "StreamStateChanged" && d.outputState === "OBS_WEBSOCKET_OUTPUT_STARTED") when = "streamstart";
  else if (type === "StreamStateChanged" && d.outputState === "OBS_WEBSOCKET_OUTPUT_STOPPED") when = "streamstop";
  else if (type === "RecordStateChanged" && d.outputState === "OBS_WEBSOCKET_OUTPUT_STARTED") when = "recordstart";
  else if (type === "RecordStateChanged" && d.outputState === "OBS_WEBSOCKET_OUTPUT_STOPPED") when = "recordstop";
  if (!when) return;
  for (const a of live) {
    const t = a.trigger;
    if (t.type !== "obs" || (t.when || "scene") !== when) continue;
    if (when === "scene" && String(t.scene || "").trim() && String(t.scene).trim().toLowerCase() !== scene.toLowerCase()) continue;
    run(a, { user: "", scene, platform: "all" });
  }
}

// ---------- points (a currency, as Mix It Up's): for chatting while you're live, kept ----------
const seen = {};
// (who's chatted, and when: Each Viewer, Every Few Minutes)
const present = {};
let entries = [], pointsTimer = null, saveSoon = null, isLive = false;

// ---------- Mix It Up's extras: games on points, ranks, the credits ----------
let duels = {}, heists = {}, trivia = null, credits = {};

// A bet: a number or "all", at least so many, and theirs to bet (taken from them by the game)
function betOf(text, user, least, say, ctx) {
  const have = pointsOf(user);
  const bet = String(text).trim().toLowerCase() === "all" ? have : Math.floor(num(text));
  if (!(bet > 0) || bet < Math.max(1, least)) { say(ctx.msgId, "@" + user + ", bet at least " + Math.max(1, least) + " " + pointsName() + " (you have " + have + ")."); return null; }
  if (bet > have) { say(ctx.msgId, "@" + user + ", you have " + have + " " + pointsName() + "."); return null; }
  return bet;
}

// (chat: a duel's accept word from whoever was challenged; a trivia question's answer)
function gameChat(user, text, platform) {
  const said = text.trim().toLowerCase(), key = platform + "|" + String(user).toLowerCase();
  const d = duels[key];
  if (d && said === d.word) { clearTimeout(d.off); d.go(); }
  if (trivia && trivia.platform === platform && trivia.right.includes(said)) trivia.won(user);
}

// A viewer's rank, by their points (the settings' list: "0 Newcomer, 100 Regular...")
function rankOf(user) {
  const have = pointsOf(user);
  let best = "";
  String(S().pointsRanks || "").split(",").map((x) => /^\s*(\d+)\s+(.+?)\s*$/.exec(x)).filter(Boolean)
    .map((m) => [num(m[1]), m[2]]).sort((a, b) => a[0] - b[0]).forEach(([at, title]) => { if (have >= at) best = title; });
  return best;
}

// Who did what since going live, for the credits
const CREDIT_OF = { follow: "Followers", sub: "Subscribers", resub: "Subscribers", gift: "Gifted Subs", bits: "Cheers", raid: "Raiders", donation: "Donations", charity: "Donations", chat: "Chatters" };
function credit(ev, user) {
  const part = CREDIT_OF[ev];
  if (!part || !user) return;
  const list = (credits[part] = credits[part] || []);
  if (!list.some((x) => x.toLowerCase() === String(user).toLowerCase())) list.push(String(user));
}
function creditParts() { return Object.values(CREDIT_OF).filter((p, i, a) => a.indexOf(p) === i && credits[p] && credits[p].length); }
function creditsLine() { return creditParts().map((p) => p + ": " + credits[p].join(", ")).join(" \u{b7} ") || "no one yet"; }
function creditsRoll(title) { return [title].concat(creditParts().map((p) => "\n" + p + "\n" + credits[p].join("\n"))).join("\n").trim(); }

function pointsName() { return String(S().pointsName || "points").trim() || "points"; }
function pointsOf(who) { return num((S().points || {})[String(who || "").toLowerCase()]); }

function addPoints(who, n) {
  const k = String(who || "").toLowerCase();
  if (!k || !n) return;
  const all = Object.assign({}, S().points || {});
  all[k] = Math.max(0, num(all[k]) + n);
  OmniPulse.settings.points = all;
  if (saveSoon) clearTimeout(saveSoon);
  saveSoon = setTimeout(() => { OmniPulse.app.saveSettings({ points: S().points }).catch(() => {}); }, 3000);
}

// Every so often while you're live (or always, if it's set so): each viewer who's chatted since, given theirs
function setPoints() {
  if (pointsTimer) clearInterval(pointsTimer);
  pointsTimer = null;
  if (!yes(S().pointsOn)) return;
  const every = Math.max(1, num(S().pointsEvery) || 10) * 60000;
  pointsTimer = setInterval(() => {
    if (!chat || (!isLive && !yes(S().pointsOffline))) return;
    const since = Date.now() - every;
    for (const [who, at] of Object.entries(seen)) if (at >= since) addPoints(who, Math.round(num(S().pointsAmount) || 10));
  }, every);
}


// ---------- Kick: signed in with its OAuth (your own app's Client ID and secret, PKCE), chat read from
// Kick's own realtime socket (as kick.com's page reads it: a guest's), sent through its API ----------
const KICK_SCOPES = "user:read channel:read channel:write chat:write moderation:ban moderation:chat_message:manage channel:rewards:read channel:rewards:write";
// (a bot account: only what posting needs, as Mix It Up's)
const KICK_BOT_SCOPES = "user:read chat:write";
const KICK_REDIRECT_PORT = 8476;
const KICK_REDIRECT = "http://localhost:" + KICK_REDIRECT_PORT + "/kick";
let kick = null, kickRoom = 0, kickChannelId = 0, kickRefresh = null, kickN = 10, kickRetry = 2000;
const kickSent = [];

function b64url(bytes) { return b64(bytes).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""); }
function randomText(n) {
  const A = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
  let out = "";
  for (let i = 0; i < n; i++) out += A[Math.floor(Math.random() * A.length)];
  return out;
}

// (who: "bot" for a second Kick account that posts the replies; else your channel)
function signInKick(values, copy, who) {
  let said = null;
  const first = new Promise((ok) => { said = ok; });
  kickSignInFlow(values, said, copy, who).then((msg) => { if (msg) { said(msg); OmniPulse.app.say(msg); } }, (e) => { said("Couldn't sign in to Kick: " + e.message); OmniPulse.app.say("Couldn't sign in to Kick: " + e.message); });
  return first;
}

async function kickSignInFlow(values, said, copy, who) {
  const bot = who === "bot";
  const v = Object.assign({}, S(), values || {});
  const id = String(v.kickClientId || "").trim(), secret = String(v.kickClientSecret || "").trim();
  // (OmniPulse's own app: no secret here, the relay finishes the sign-in)
  const relay = secret ? "" : String(v.kickRelay || "").trim();
  if (!id || (!secret && !relay)) return "Put your Kick app's Client ID and Client Secret in first, then sign in.";
  const verifier = randomText(64), state = randomText(24);
  const q = { client_id: id, response_type: "code", redirect_uri: KICK_REDIRECT, scope: bot ? KICK_BOT_SCOPES : KICK_SCOPES, state, code_challenge: b64url(sha256(utf8(verifier))), code_challenge_method: "S256" };
  const waiting = OmniPulse.app.awaitRedirect(KICK_REDIRECT_PORT, "/kick");
  await signInPage("https://id.kick.com/oauth/authorize?" + Object.entries(q).map(([k, x]) => k + "=" + encodeURIComponent(x)).join("&"), copy);
  const as = bot ? "as the bot account" : "as your channel";
  said(copy
    ? "Sign-in link copied. Within five minutes, paste it into a private window on this PC, sign in " + as + ", and click Allow."
    : "Kick is open in your browser. Sign in " + as + " and click Allow.");
  const back = await waiting;
  const got = {};
  String(back.query || "").split("&").forEach((kv) => { const i = kv.indexOf("="); if (i > 0) got[kv.slice(0, i)] = decodeURIComponent(kv.slice(i + 1).replace(/\+/g, " ")); });
  if (got.error) return "Kick's sign-in didn't finish: " + (got.error_description || got.error);
  if (got.state !== state || !got.code) return "Kick's sign-in didn't come back as it should: try again.";
  const t = relay
    ? await viaRelay(relay, "kick", "token", { code: got.code, code_verifier: verifier, redirect_uri: KICK_REDIRECT })
    : await form("https://id.kick.com/oauth/token", { grant_type: "authorization_code", client_id: id, client_secret: secret, redirect_uri: KICK_REDIRECT, code: got.code, code_verifier: verifier });
  if (!t.ok || !t.j.access_token) return "Kick didn't take the sign-in: " + (t.j.error_description || t.j.message || t.j.error || t.status) + (relay ? ". Try again in a moment." : ". Check your app's Client Secret, and that its Redirect URL is " + KICK_REDIRECT + ".");
  const p = bot ? "kickBot" : "kick";
  const keep = { kickClientId: id, kickClientSecret: secret, kickRelay: relay, [p + "Token"]: t.j.access_token, [p + "RefreshToken"]: t.j.refresh_token || "" };
  Object.assign(OmniPulse.settings, keep);
  const me = await kickApi("GET", "users", null, false, who);
  const u = me.data && me.data[0];
  if (!u) return "Signed in, but Kick didn't say who as: try again.";
  if (bot) {
    keep.kickBotLogin = String(u.name || "");
    if (keep.kickBotLogin && keep.kickBotLogin.toLowerCase() === String(S().kickLogin || "").toLowerCase()) return "That's your channel's own account: sign in the bot with its own Kick account (a private window keeps you signed in as you).";
  } else {
    keep.kickUserId = String(u.user_id);
    keep.kickLogin = String(u.name || "");
    if (!String(v.kickChannel || "").trim()) keep.kickChannel = String(u.name || "").toLowerCase();
  }
  await OmniPulse.app.saveSettings(keep);
  Object.assign(OmniPulse.settings, keep);
  restart();
  return "Signed in to Kick as " + (u.name || "you") + (bot ? " (the bot)." : ".");
}

// Kick's API, as your channel (or who: "bot", the bot account); a token that's run out, renewed once
async function kickApi(method, path, body, again, who) {
  const token = who === "bot" ? S().kickBotToken : S().kickToken;
  if (!token) throw new Error("sign in to Kick first (in the chatbot's settings)");
  const r = await OmniPulse.fetch("https://api.kick.com/public/v1/" + path, {
    method,
    headers: { Authorization: "Bearer " + token, "Content-Type": "application/json", Accept: "application/json" },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (r.status === 401 && !again && (await kickRenew(who))) return kickApi(method, path, body, true, who);
  if (!r.ok) {
    let why = "";
    try { why = (await r.json()).message || ""; } catch (e) {}
    throw new Error("Kick said " + r.status + (why ? ": " + why : "") + (r.status === 401 || r.status === 403 ? " (sign in to Kick again, in the chatbot's settings)" : ""));
  }
  if (r.status === 204) return {};
  try { return await r.json(); } catch (e) { return {}; }
}

// A sign-in (or renewal) finished by OmniPulse's relay, which keeps its app's secret (platform: kick, twitch)
async function viaRelay(relay, platform, what, body) {
  const r = await OmniPulse.fetch(relay.replace(/\/$/, "") + "/api/" + platform + "/" + what, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  let j = {};
  try { j = await r.json(); } catch (e) {}
  return { ok: r.ok, status: r.status, j };
}

async function kickRenew(who) {
  const s = S();
  const p = who === "bot" ? "kickBot" : "kick";
  if (!s[p + "RefreshToken"] || !s.kickClientId || (!s.kickClientSecret && !s.kickRelay)) return false;
  const t = s.kickClientSecret
    ? await form("https://id.kick.com/oauth/token", { grant_type: "refresh_token", client_id: s.kickClientId, client_secret: s.kickClientSecret, refresh_token: s[p + "RefreshToken"] })
    : await viaRelay(s.kickRelay, "kick", "refresh", { refresh_token: s[p + "RefreshToken"] });
  if (!t.ok || !t.j.access_token) return false;
  const keep = { [p + "Token"]: t.j.access_token, [p + "RefreshToken"]: t.j.refresh_token || s[p + "RefreshToken"] };
  await OmniPulse.app.saveSettings(keep);
  Object.assign(OmniPulse.settings, keep);
  return true;
}

// A message in your Kick chat (to the one it answers): from the bot account if there is one, else you; kept
// a moment, so its echo isn't taken as chat
function kickSay(to, text) {
  const who = S().kickBotToken ? "bot" : "channel";
  if (!kick || !(who === "bot" ? S().kickBotToken : S().kickToken)) return;
  const channel = Number(S().kickUserId || 0);
  String(text).split("\n").filter(Boolean).forEach((line) => {
    const content = line.slice(0, 500);
    kickSent.push({ content, at: Date.now() });
    while (kickSent.length > 30) kickSent.shift();
    const body = { content, type: "user", broadcaster_user_id: channel };
    if (to) body.reply_to_message_id = to;
    kickApi("POST", "chat", body, false, who).catch((e) => OmniPulse.log("kick chat: " + e.message));
  });
}

async function kickPost(path, body) {
  const r = await OmniPulse.fetch("https://web.kick.com/api/v1/realtime/" + path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
  if (!r.ok) throw new Error("Kick's chat said " + r.status);
  return (await r.json()).data || {};
}

// Kick's chat and its channel's events: a guest's connection (Centrifugo), kept going
async function kickConnect() {
  if (!yes(S().kickOn) || !String(S().kickChannel || "").trim()) return;
  const slug = String(S().kickChannel).trim().toLowerCase().replace(/_/g, "-");
  // (its front door turns away a request that doesn't say what it is)
  const r = await OmniPulse.fetch("https://kick.com/api/v2/channels/" + encodeURIComponent(slug), { headers: { Accept: "application/json", "User-Agent": "OmniPulse" } });
  if (!r.ok) throw new Error("Kick didn't answer for " + slug + " (" + r.status + ")");
  const ch = await r.json();
  kickRoom = ch.chatroom && ch.chatroom.id;
  kickChannelId = ch.id;
  if (!kickRoom) throw new Error("Kick has no channel called " + slug);
  if (!S().kickUserId && ch.user_id) OmniPulse.settings.kickUserId = String(ch.user_id);
  const client = randomText(8) + "-" + randomText(4) + "-4" + randomText(3) + "-a" + randomText(3) + "-" + randomText(12);
  const conn = await kickPost("connection", { client: { id: client, type: "web" }, capabilities: { accepted_providers: [{ provider: "centrifugo" }] } });
  const url = conn.connections && conn.connections[0] && conn.connections[0].credentials && conn.connections[0].credentials.url;
  if (!url) throw new Error("Kick's chat didn't say where to connect");
  const token = (await kickPost("auth/connection", { client_id: client })).token;
  const s = OmniPulse.socket(url);
  kick = s;
  s.onopen = () => {
    s.send(JSON.stringify({ connect: { token, name: "js" }, id: 1 }));
    s.send(JSON.stringify({ subscribe: { channel: "chatrooms." + kickRoom + ".v2" }, id: 2 }));
    s.send(JSON.stringify({ subscribe: { channel: "channel." + kickChannelId }, id: 3 }));
    kickRetry = 2000;
  };
  s.onmessage = (data) => String(data).split("\n").filter(Boolean).forEach((line) => {
    if (line === "{}") { s.send("{}"); return; }
    let m;
    try { m = JSON.parse(line); } catch (e) { return; }
    // (its guest token, renewed before it runs out)
    if (m.id === 1 && m.connect) s.up = true;
    if (m.id === 1 && m.connect && m.connect.ttl) {
      if (kickRefresh) clearInterval(kickRefresh);
      kickRefresh = setInterval(() => {
        kickPost("auth/connection", { client_id: client }).then((t) => { if (kick === s) s.send(JSON.stringify({ refresh: { token: t.token }, id: ++kickN })); }).catch((e) => OmniPulse.log("kick token: " + e.message));
      }, Math.max(60, m.connect.ttl * 0.75) * 1000);
    }
    const pub = m.push && m.push.pub && m.push.pub.data;
    if (!pub) return;
    let d = pub.data;
    if (typeof d === "string") { try { d = JSON.parse(d); } catch (e) { d = {}; } }
    onKick(String(pub.event || ""), d || {});
  });
  s.onclose = (why) => {
    if (kick !== s) return;
    kick = null;
    if (kickRefresh) { clearInterval(kickRefresh); kickRefresh = null; }
    if (!stopped && yes(S().kickOn)) { OmniPulse.log("kick closed: " + why); setTimeout(() => { if (!kick && !stopped) kickConnect().catch((e) => OmniPulse.log("kick: " + e.message)); }, kickRetry); kickRetry = Math.min(kickRetry * 2, 60000); }
  };
}

const KICK_SEEN = {};
// (who's spoken this stream: their first words; begun again as you go live)
const spoken = new Set();
// What Kick sends: its chat as ours (a Twitch line's shape, its platform Kick), its events as ours
function onKick(event, d) {
  const ev = event.replace(/^App\\Events\\/, "");
  if (ev === "MessageDeletedEvent") { dockGone("kick", String((d.message && d.message.id) || d.id || ""), ""); return; }
  if (ev === "UserBannedEvent") { const u = d.user || {}; dockGone("kick", "", String(u.slug || u.username || "")); return; }
  if (ev === "ChatroomClearEvent") { dockGone("kick", "", ""); return; }
  if (ev === "ChatMessageEvent") {
    const sender = d.sender || {};
    const content = String(d.content || "");
    {
      const slug = String(sender.slug || sender.username || "").toLowerCase();
      const own = [S().kickLogin, S().kickBotLogin].some((x) => x && String(x).toLowerCase() === String(sender.username || "").toLowerCase());
      const kinds = ((sender.identity && sender.identity.badges) || []).map((b) => String(b.type || "").toLowerCase());
      OmniPulse.app.dockChat({ platform: "kick", id: String(d.id || ""), user: sender.username || slug, login: slug, userId: String(sender.id || ""), color: (sender.identity && sender.identity.color) || "", badges: kinds, text: content, self: own }).catch(() => {});
    }
    // (the bot account's messages never start anything; our own message coming back: not chat)
    if (S().kickBotLogin && String(sender.username || "").toLowerCase() === String(S().kickBotLogin).toLowerCase()) return;
    if (S().kickLogin && String(sender.username || "").toLowerCase() === String(S().kickLogin).toLowerCase()) {
      const k = kickSent.findIndex((x) => x.content === content && Date.now() - x.at < 30000);
      if (k >= 0) { kickSent.splice(k, 1); return; }
    }
    const types = ((sender.identity && sender.identity.badges) || []).map((b) => String(b.type || "").toLowerCase());
    const badges = types.map((t) => (t === "moderator" ? "moderator" : t === "founder" || t === "og" ? "founder" : t) + "/1").join(",");
    const emotes = (content.match(/\[emote:\d+:[^\]]*\]/g) || []).length;
    onChat({
      command: "PRIVMSG", platform: "kick", nick: String(sender.slug || sender.username || "").toLowerCase(), text: content.replace(/\[emote:\d+:([^\]]*)\]/g, "$1"),
      tags: { "display-name": sender.username || "", badges, mod: types.includes("moderator") ? "1" : "0", subscriber: types.includes("subscriber") ? "1" : "0", id: String(d.id || ""), "user-id": String(sender.id || ""), emotes: emotes ? "kick:" + new Array(emotes).fill("0-0").join(",") : "" },
    });
    return;
  }
  const alert = (a) => { if (S().alerts !== false) sendAlert(Object.assign({ platform: "kick" }, a)); };
  const go = (ours, ctx) => { const c = Object.assign({ platform: "kick", rawInput: "", message: "", amount: "" }, ctx); Object.assign(c, words(c.rawInput)); fire(ours, c); };
  switch (ev) {
    case "SubscriptionEvent": {
      const months = num(d.months);
      const user = d.username || "";
      alert({ type: "sub", user, amount: months > 1 ? months + " months" : "" });
      go(months > 1 ? "resub" : "sub", { user, amount: String(months || 1) });
      return;
    }
    case "GiftedSubscriptionsEvent": {
      const n = (d.gifted_usernames || []).length || 1;
      const user = d.gifter_username || "Anonymous";
      alert({ type: "gift", user, amount: String(n) });
      go("gift", { user, amount: String(n) });
      return;
    }
    case "StreamHostEvent": {
      const user = d.host_username || "";
      alert({ type: "raid", user, amount: String(d.number_viewers || 0) });
      go("raid", { user, amount: String(d.number_viewers || 0), message: d.optional_message || "" });
      return;
    }
    case "FollowersUpdated": {
      const user = d.username || "";
      if (d.followed === false || !user) return;
      alert({ type: "follow", user });
      go("follow", { user });
      return;
    }
    case "RewardRedeemedEvent": {
      const user = d.username || (d.redeemer && d.redeemer.username) || "";
      const input = String(d.user_input || "");
      go("reward", { user, reward: d.reward_title || (d.reward && d.reward.title) || "", rewardId: String(d.reward_id || (d.reward && d.reward.id) || ""), rawInput: input, message: input });
      return;
    }
    case "StreamerIsLive": spoken.clear(); go("online", {}); return;
    case "StopStreamBroadcast": go("offline", {}); return;
  }
  // (one Kick sends that OmniPulse hasn't a use for yet: noted once, to add later)
  if (!KICK_SEEN[ev]) { KICK_SEEN[ev] = true; OmniPulse.log("kick event not used yet: " + ev + " " + JSON.stringify(d).slice(0, 300)); }
}

// A Twitch channel's id, by its name
async function twitchUserId(name) {
  const login = String(name || "").replace("@", "").trim().toLowerCase();
  if (!login) throw new Error("no channel named");
  const u = ((await helix("GET", "users?login=" + encodeURIComponent(login))).data || [])[0];
  if (!u) throw new Error("there's no Twitch account called " + login);
  return u.id;
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
  ["stream.online", "1", { broadcaster_user_id: me }],
  ["stream.offline", "1", { broadcaster_user_id: me }],
  ["channel.ad_break.begin", "1", { broadcaster_user_id: me }],
  ["channel.prediction.begin", "1", { broadcaster_user_id: me }],
  ["channel.prediction.end", "1", { broadcaster_user_id: me }],
  ["channel.poll.end", "1", { broadcaster_user_id: me }],
  ["channel.hype_train.end", "1", { broadcaster_user_id: me }],
  ["channel.charity_campaign.donate", "1", { broadcaster_user_id: me }],
  ["channel.shoutout.receive", "1", { broadcaster_user_id: me, moderator_user_id: me }],
  ["channel.vip.add", "1", { broadcaster_user_id: me }],
  ["channel.moderator.add", "1", { broadcaster_user_id: me }],
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
      if (S().alerts !== false) {
        const a = asAlert(type, e);
        if (a) sendAlert(a);
      }
      onEvent(type, e);
    }
  };
  s.onclose = (why) => { if (!stopped && events === s) { OmniPulse.log("events closed: " + why); setTimeout(() => { events = connectEvents(token); }, retry); } };
  return s;
}

// (some of the sign-ins forgotten, the rest kept; the chatbot started again without them)
async function signOutOf(none, said) {
  await OmniPulse.app.saveSettings(none);
  Object.assign(OmniPulse.settings, none);
  restart();
  return said;
}

// ---------- running ----------
async function start() {
  stopAll();
  stopped = false;
  loadActions();
  // (the Suite's widgets: what's kept, sent to the app as it starts)
  suite = null;
  suitePush();
  setPoints();
  obsConnect();
  tipConnect();
  kickConnect().catch((e) => { OmniPulse.log("kick: " + e.message); OmniPulse.app.say("Kick's chat didn't connect: " + e.message).catch(() => {}); });
  if (!S().token || !S().login) return;
  const token = await fresh("channel");
  if (!token) { OmniPulse.app.say("Your Twitch sign-in has run out: sign in again in its settings."); return; }
  apiToken = token;
  chat = connectChat(token, S().login, onChat);
  if (S().botToken && S().botLogin) {
    const bot = await fresh("bot");
    if (bot) botChat = connectChat(bot, S().botLogin, () => {});
  }
  events = connectEvents(token);
  retry = 1000;
}

// ---------- the Suite's widgets: goals, a timer, the latest follow and sub, a raid's shoutout ----------
// Each event heard is counted here, kept (suiteState), and sent to the app for its pages (/goals, /timer,
// /latest-follow, /latest-sub, /raid). What each shows and how is in the Suite's settings.
let suite = null, suiteSaveT = null, suitePushT = null;
const today = () => new Date().toISOString().slice(0, 10);
function suiteNow() {
  if (!suite) {
    const k = S().suiteState || {};
    suite = {
      goals: Object.assign({ follow: 0, sub: 0, bits: 0, donation: 0, day: today() }, k.goals || {}),
      latest: Object.assign({ follow: null, sub: null }, k.latest || {}),
      timer: Object.assign({ running: false, leftMs: num(S().timerStart || 60) * 60000, endsAt: 0, startedAt: 0, elapsedMs: 0, addedMs: 0 }, k.timer || {}),
      raid: null,
      lastRaid: k.lastRaid || null,
    };
  }
  // (goals reset each day, if that's how they're set)
  if (String(S().goalReset || "stream") === "daily" && suite.goals.day !== today()) Object.assign(suite.goals, { follow: 0, sub: 0, bits: 0, donation: 0, day: today() });
  return suite;
}
function suiteKeep() {
  clearTimeout(suiteSaveT);
  suiteSaveT = setTimeout(() => { const k = { goals: suite.goals, latest: suite.latest, timer: suite.timer, lastRaid: suite.lastRaid }; OmniPulse.settings.suiteState = k; OmniPulse.app.saveSettings({ suiteState: k }).catch(() => {}); }, 4000);
}
// (to the app, for the pages: what's counted, and how each is set)
function suitePush() {
  clearTimeout(suitePushT);
  suitePushT = setTimeout(() => {
    const st = suiteNow(), x = S();
    const goal = (k, label, dflt) => ({ on: yes(x["goal" + k + "On"] === undefined ? true : x["goal" + k + "On"]), label: String(x["goal" + k + "Label"] || label), target: Math.max(1, num(x["goal" + k + "Target"]) || dflt) });
    OmniPulse.app.suiteWidget({
      goals: { counts: st.goals, follow: goal("Follow", "Follower goal", 10), sub: goal("Sub", "Sub goal", 5), bits: goal("Bits", "Bits goal", 1000), donation: goal("Dono", "Donation goal", 50), currency: String(x.goalCurrency || "$"), style: String(x.goalStyle || "bars") },
      latest: { follow: st.latest.follow, sub: st.latest.sub },
      timer: Object.assign({ stopwatch: yes(x.timerStopwatch), label: String(x.timerLabel || ""), now: Date.now() }, st.timer),
      raid: st.raid,
      // (the fun's: the meter's level and how full, the skip votes, the wheel's last spin, the top points, and
      // emote rain's bursts)
      hype: { on: funOn("hypeOn"), level: fun.level, fill: (fun.hype % hypeStep()) / hypeStep(), levelAt: fun.levelAt, label: String(x.hypeLabel || "Hype") },
      voteskip: { on: yes(x.skipOn), votes: fun.votes.size, need: Math.max(1, num(x.skipVotes) || 5), song: fun.song, command: String(x.skipCommand || "!skip") },
      wheel: Object.assign({ prizes: wheelPrizes(), command: String(x.wheelCommand || "!spin") }, fun.wheel || {}),
      leaderboard: { list: leaders(), name: pointsName() },
      burst: fun.burst,
      anim: String(x.suiteAnim || "rise"),
      theme: String(x.suiteTheme || "dark"),
    }).catch(() => {});
  }, 250);
}
function money(v) { const n = parseFloat(String(v || "").replace(/[^0-9.]/g, "")); return isNaN(n) ? 0 : n; }
function tierOf(t) { return t === "3000" ? 3 : t === "2000" ? 2 : 1; }

// (the subathon: time added for what's happened, up to its cap)
function timerAdd(ms) {
  const t = suiteNow().timer, x = S();
  if (yes(x.timerStopwatch) || ms <= 0) return;
  const cap = num(x.timerCap) * 3600000;
  if (cap > 0) ms = Math.max(0, Math.min(ms, cap - t.addedMs));
  if (!ms) return;
  t.addedMs += ms;
  if (t.running) t.endsAt += ms; else t.leftMs += ms;
}

function suiteHear(ev, ctx) {
  const st = suiteNow(), x = S(), g = st.goals;
  if (ev === "online" && String(x.goalReset || "stream") === "stream") Object.assign(g, { follow: 0, sub: 0, bits: 0, donation: 0, day: today() });
  const sub = ev === "sub" || ev === "resub" || ev === "gift";
  if (ev === "follow") { g.follow++; st.latest.follow = { user: ctx.user, platform: ctx.platform, at: Date.now() }; if (yes(x.timerSubathon)) timerAdd(num(x.timerFollow) * 1000); }
  if (sub) {
    const n = ev === "gift" ? Math.max(1, num(ctx.amount)) : 1, tier = tierOf(ctx.tier);
    g.sub += n;
    st.latest.sub = { user: ctx.user, platform: ctx.platform, tier, months: ev === "resub" ? num(ctx.amount) : 0, gift: ev === "gift" ? n : 0, at: Date.now() };
    if (yes(x.timerSubathon)) timerAdd(n * num(x["timerSub" + tier] || [0, 5, 10, 25][tier]) * 60000);
  }
  if (ev === "bits") { g.bits += num(ctx.amount); if (yes(x.timerSubathon)) timerAdd(num(ctx.amount) / 100 * num(x.timerBits) * 60000); }
  if (ev === "donation") { g.donation += money(ctx.amount); if (yes(x.timerSubathon)) timerAdd(money(ctx.amount) * num(x.timerDono) * 60000); }
  if (ev === "raid" && num(ctx.amount) >= Math.max(1, num(x.raidMin) || 1)) st.lastRaid = { user: ctx.user, userId: ctx.userId, viewers: num(ctx.amount), platform: ctx.platform, at: Date.now() };
  // (the hype meter: what happens fills it more than chat does; a sub, a raid or a good cheer pours emotes)
  if (ev === "follow") hypeAdd(10);
  if (sub) hypeAdd(25 * (ev === "gift" ? Math.max(1, num(ctx.amount)) : 1));
  if (ev === "bits") hypeAdd(num(ctx.amount) / 10);
  if (ev === "donation") hypeAdd(money(ctx.amount) * 10);
  if (ev === "raid") hypeAdd(Math.max(10, num(ctx.amount)));
  if (sub || ev === "raid" || (ev === "bits" && num(ctx.amount) >= 100)) fun.burst++;
  if (["follow", "bits", "donation", "raid", "online"].includes(ev) || sub) { suiteKeep(); suitePush(); }
}

// (the shoutout, when it's asked for (the Stream Deck's Raid Shoutout, or its sheet): your latest raider shown,
// one of their recent clips if it's set to, and said in chat if that's on)
async function shoutout() {
  const st = suiteNow();
  if (!st.lastRaid) return "No one's raided yet.";
  await raidIn(st.lastRaid);
  return "Shouting out " + st.lastRaid.user + ".";
}
async function raidIn(ctx) {
  const st = suiteNow(), x = S();
  st.raid = { user: ctx.user, login: String(ctx.user || "").toLowerCase(), viewers: num(ctx.viewers != null ? ctx.viewers : ctx.amount), platform: ctx.platform, at: Date.now(), secs: Math.max(5, num(x.raidSecs) || 20), clip: null };
  if (ctx.viewers != null) ctx = Object.assign({}, ctx, { amount: ctx.viewers });
  if (yes(x.raidChat === undefined ? true : x.raidChat)) {
    const said = String(x.raidMessage || "Thanks for the raid, {user}! Go follow them at twitch.tv/{login} 💜").replace(/\{user\}/g, ctx.user).replace(/\{login\}/g, String(ctx.user || "").toLowerCase()).replace(/\{viewers\}/g, String(num(ctx.amount)));
    reply(null, said, ctx.platform === "kick" ? "kick" : "twitch");
  }
  if (yes(x.raidClip) && ctx.platform === "twitch" && ctx.userId) {
    try {
      const since = new Date(Date.now() - 60 * 86400000).toISOString();
      let r = await helix("GET", "clips?broadcaster_id=" + encodeURIComponent(ctx.userId) + "&first=20&started_at=" + encodeURIComponent(since));
      if (!r.data || !r.data.length) r = await helix("GET", "clips?broadcaster_id=" + encodeURIComponent(ctx.userId) + "&first=20");
      const c = (r.data || [])[Math.floor(Math.random() * Math.max(1, (r.data || []).length))];
      if (c) {
        st.raid.clip = { id: c.id, title: c.title || "", secs: Math.ceil(num(c.duration) || 20) };
        st.raid.secs = Math.max(st.raid.secs, st.raid.clip.secs + 4);
      }
    } catch (e) { OmniPulse.log("raid clip: " + e.message); }
  }
  suitePush();
}

// ---------- the Suite's fun: a hype meter, vote skip, a wheel, emote rain's bursts, a leaderboard ----------
// Each shown by its own page (/hype, /voteskip, /wheel, /leaderboard, /emotes), sent with the others in suitePush.
const fun = { hype: 0, level: 0, levelAt: 0, chatAt: {}, votes: new Set(), song: "", wheel: null, spunAt: {}, burst: 0 };
const funOn = (k) => yes(S()[k] === undefined ? true : S()[k]);
const hypeStep = () => Math.max(10, num(S().hypeLevel) || 100);

// (the meter: chat and what happens fill it; each level reached, said and celebrated, and actions run)
function hypeAdd(n) {
  if (!funOn("hypeOn") || n <= 0) return;
  fun.hype += n;
  const lvl = Math.floor(fun.hype / hypeStep());
  if (lvl > fun.level) {
    fun.level = lvl;
    fun.levelAt = Date.now();
    fun.burst++;
    if (funOn("hypeChat")) reply(null, String(S().hypeMessage || "Hype level {level}! 🔥").replace(/\{level\}/g, String(lvl)), "all");
    fire("hypelevel", { user: "", amount: String(lvl), message: "", rawInput: "", platform: "all", replay: true });
  }
  suitePush();
}
// (a message: a little each, at most once every 10 s per viewer, so one can't fill it alone)
function funChat(user) {
  const k = String(user || "").toLowerCase();
  if (!fun.chatAt[k] || Date.now() - fun.chatAt[k] > 10000) { fun.chatAt[k] = Date.now(); hypeAdd(Math.max(0, num(S().hypeChatPts) || 2)); }
}
// (and it cools, slowly: a level's lost again only once it's well under it)
setInterval(() => {
  if (fun.hype <= 0) return;
  const step = hypeStep();
  fun.hype = Math.max(0, fun.hype - step * Math.max(0, num(S().hypeCool) || 2) / 100);
  const lvl = Math.floor(fun.hype / step);
  if (lvl < fun.level) fun.level = lvl;
  suitePush();
}, 5000);

// (vote skip: each viewer once a song; enough of them, and it's skipped)
function voteSkip(user, id, platform) {
  const x = S();
  const k = String(user || "").toLowerCase();
  if (fun.votes.has(k)) return reply(id, "@" + user + " you've voted to skip this one already.", platform);
  fun.votes.add(k);
  const need = Math.max(1, num(x.skipVotes) || 5);
  if (fun.votes.size >= need) {
    fun.votes.clear();
    OmniPulse.app.control("next").catch(() => {});
    reply(null, "Chat voted: skipped. ⏭", "all");
    fire("voteskip", { user, amount: String(need), message: fun.song, rawInput: fun.song, platform: "all", replay: true });
  } else reply(id, "@" + user + " voted to skip (" + fun.votes.size + "/" + need + ").", platform);
  suitePush();
}

// (the wheel: its prizes, one picked; shown spinning, then said, and the actions waiting for it run with %prize%)
function wheelPrizes() { return String(S().wheelPrizes || "A shoutout | 100 points | Pick the next song | Nothing! | Hydrate").split("|").map((p) => p.trim()).filter(Boolean).slice(0, 16); }
async function wheelSpin(user, id, platform, free) {
  const x = S(), k = String(user || "").toLowerCase();
  const prizes = wheelPrizes();
  if (prizes.length < 2) return reply(id, "The wheel needs two prizes at least (in the Suite's Spin the Wheel).", platform);
  if (fun.wheel && Date.now() - fun.wheel.at < 9000) return reply(id, "@" + user + " the wheel's still spinning.", platform);
  if (!free) {
    const wait = Math.max(0, num(x.wheelCooldown)) * 1000;
    if (wait && fun.spunAt[k] && Date.now() - fun.spunAt[k] < wait) return reply(id, "@" + user + " you can spin again in " + Math.ceil((wait - (Date.now() - fun.spunAt[k])) / 1000) + " s.", platform);
    const cost = Math.max(0, num(x.wheelCost));
    if (cost && yes(x.pointsOn)) {
      if (pointsOf(user) < cost) return reply(id, "@" + user + " a spin costs " + cost + " " + pointsName() + ".", platform);
      addPoints(user, -cost);
    }
    fun.spunAt[k] = Date.now();
  }
  const pick = Math.floor(Math.random() * prizes.length);
  fun.wheel = { prizes, pick, user, at: Date.now() };
  suitePush();
  // (said once the wheel's stopped, as the page has it spin for 6 s)
  setTimeout(() => {
    const prize = prizes[pick];
    if (funOn("wheelChat")) reply(null, String(x.wheelMessage || "{user} spun the wheel: {prize}!").replace(/\{user\}/g, user || "The wheel").replace(/\{prize\}/g, prize), platform || "all");
    fire("wheel", { user: user || "", amount: String(pick + 1), message: prize, rawInput: prize, prize, platform: platform || "all", replay: true });
  }, 6500);
}

// (the leaderboard: the most points, from the points kept)
function leaders() {
  const all = S().points || {};
  return Object.keys(all).map((k) => ({ user: k, points: num(all[k]) })).filter((x) => x.points > 0).sort((a, b) => b.points - a.points).slice(0, Math.min(10, Math.max(3, num(S().boardCount) || 5)));
}

// (the timer's command: start, pause, add or take away minutes, reset; moderators and you. And the fun's own:
// vote skip and the wheel)
async function suiteCommand(cmd, args, r, id, platform, user) {
  const x = S();
  // (both off until they're switched on: neither takes a command you might already use)
  if (yes(x.skipOn) && cmd === String(x.skipCommand || "!skip").trim().toLowerCase()) { voteSkip(user, id, platform); return true; }
  if (yes(x.wheelOn) && cmd === String(x.wheelCommand || "!spin").trim().toLowerCase()) { await wheelSpin(user, id, platform, false); return true; }
  const c = String(S().timerCommand || "!timer").trim().toLowerCase();
  if (!c || cmd !== c) return false;
  if (!r.mod) return true;
  const [what, n] = String(args || "").toLowerCase().split(/\s+/);
  const t = suiteNow().timer;
  const said = timerDo(what, num(n));
  if (said) reply(id, said, platform);
  return true;
}
function timerDo(what, n) {
  const t = suiteNow().timer, x = S(), now = Date.now();
  const left = () => t.running ? Math.max(0, t.endsAt - now) : t.leftMs;
  if (what === "start" || what === "resume") {
    if (t.running) return "The timer's already running.";
    t.running = true;
    if (yes(x.timerStopwatch)) t.startedAt = now - t.elapsedMs; else t.endsAt = now + t.leftMs;
  } else if (what === "pause" || what === "stop") {
    if (!t.running) return "The timer's already paused.";
    t.running = false;
    if (yes(x.timerStopwatch)) t.elapsedMs = now - t.startedAt; else t.leftMs = left();
  } else if (what === "add" || what === "remove") {
    const ms = (what === "remove" ? -1 : 1) * Math.max(0, n) * 60000;
    if (t.running) t.endsAt += ms; else t.leftMs = Math.max(0, t.leftMs + ms);
  } else if (what === "reset") {
    Object.assign(t, { running: false, leftMs: num(x.timerStart || 60) * 60000, endsAt: 0, startedAt: 0, elapsedMs: 0, addedMs: 0 });
  } else return "Use " + (x.timerCommand || "!timer") + " start, pause, add 5, remove 5 or reset.";
  suiteKeep();
  suitePush();
  return what === "add" || what === "remove" ? "Timer: " + (what === "add" ? "added " : "took off ") + n + " min." : "Timer: " + what + ".";
}
// (a widget tried from its sheet: a follow, a sub, a raid, as if it had happened; not counted for keeps)
function suiteTest(kind) {
  const st = suiteNow();
  if (kind === "follow") st.latest.follow = { user: "TestFollower", platform: "twitch", at: Date.now() };
  if (kind === "sub") st.latest.sub = { user: "TestSubscriber", platform: "twitch", tier: 1, months: 6, gift: 0, at: Date.now() };
  if (kind === "raid") st.raid = { user: "TestRaider", login: "testraider", viewers: 42, platform: "twitch", at: Date.now(), secs: Math.max(5, num(S().raidSecs) || 20), clip: null };
  if (kind === "goals") { st.goals.follow++; st.goals.sub++; st.goals.bits += 100; st.goals.donation += 5; suiteKeep(); }
  // (the fun's: a level's worth of hype, a couple of skip votes, a free spin, a burst of emotes)
  if (kind === "hype") { hypeAdd(hypeStep()); return "Added a level's worth of hype."; }
  if (kind === "voteskip") { fun.votes.add("test-a"); fun.votes.add("test-b"); suitePush(); return "Two test votes added. The next song clears them."; }
  if (kind === "wheel") { wheelSpin("You", null, "all", true); return "Spinning."; }
  if (kind === "emoterain") { fun.burst++; suitePush(); return "Raining emotes."; }
  suitePush();
  return "Sent to the widget.";
}

// ---------- OBS's docks: the chat (its moderation buttons) and the activity feed ----------
let lastAlert = null;

// An alert the chatbot made for an event: shown, and kept for the event's place in the feed (not fed twice)
function sendAlert(a) {
  counts.alerts++;
  lastAlert = { a, at: Date.now() };
  OmniPulse.app.alert(Object.assign({ feed: false }, a)).catch((e) => OmniPulse.log("alert: " + e.message));
}

function feed(ev, ctx) {
  let alert = null;
  if (lastAlert && Date.now() - lastAlert.at < 3000 && String(lastAlert.a.user || "").toLowerCase() === String(ctx.user || "").toLowerCase()) { alert = lastAlert.a; lastAlert = null; }
  const keep = { type: ev, platform: ctx.platform || "twitch", user: ctx.user || "", userId: ctx.userId || "", amount: String(ctx.amount == null ? "" : ctx.amount), message: ctx.message || "", rawInput: ctx.rawInput || "", reward: ctx.reward || "", currency: ctx.currency || "", winner: ctx.winner || "", chatbot: true };
  if (alert) keep.alert = Object.assign({ feed: false }, alert);
  OmniPulse.app.dockActivity(keep).catch(() => {});
}

function dockGone(platform, id, login) { OmniPulse.app.dockGone({ platform, id, login }).catch(() => {}); }

function dockTwitch(m) {
  const login = String(m.nick || "").toLowerCase();
  let text = String(m.text || "");
  const me = /^\u0001ACTION (.*)\u0001?$/.exec(text);
  if (me) text = me[1].replace(/\u0001$/, "");
  OmniPulse.app.dockChat({
    platform: "twitch", id: m.tags.id || "", user: m.tags["display-name"] || login, login, userId: m.tags["user-id"] || "", color: m.tags.color || "",
    badges: String(m.tags.badges || "").split(",").map((b) => b.split("/")[0]).filter(Boolean), text, emotes: m.tags.emotes || "", first: m.tags["first-msg"] === "1", me: !!me,
    self: [S().login, S().botLogin].some((x) => x && String(x).toLowerCase() === login),
  }).catch(() => {});
}

// A dock's button: a message taken down, a viewer timed out, banned or let back, something said, an event's actions run again
async function dockDo(cmd) {
  const kick = cmd.platform === "kick";
  const who = cmd.user || cmd.login || "them";
  const twitchMod = "broadcaster_id=" + S().userId + "&moderator_id=" + S().userId;
  switch (cmd.do) {
    case "say": {
      const text = String(cmd.text || "").trim().slice(0, 500);
      if (text) reply(null, text, cmd.platform === "kick" || cmd.platform === "twitch" ? cmd.platform : "all");
      return "";
    }
    case "delete":
      if (!cmd.id) throw new Error("that message has no id to delete it by");
      if (kick) await kickApi("DELETE", "chat/" + encodeURIComponent(cmd.id));
      else await helix("DELETE", "moderation/chat?" + twitchMod + "&message_id=" + encodeURIComponent(cmd.id));
      return "Deleted " + who + "'s message.";
    case "timeout":
    case "ban":
    case "unban": {
      const secs = Math.max(1, Math.min(1209600, Math.round(num(cmd.seconds)) || 600));
      if (kick) {
        if (!cmd.userId) throw new Error("Kick didn't say who that is");
        const body = { broadcaster_user_id: Number(S().kickUserId), user_id: Number(cmd.userId) };
        if (cmd.reason && cmd.do !== "unban") body.reason = String(cmd.reason).slice(0, 100);
        if (cmd.do === "unban") await kickApi("DELETE", "moderation/bans", body);
        else await kickApi("POST", "moderation/bans", Object.assign(body, cmd.do === "timeout" ? { duration: Math.max(1, Math.round(secs / 60)) } : {}));
      } else {
        const id = cmd.userId || (await twitchUserId(cmd.login || cmd.user));
        if (cmd.do === "unban") await helix("DELETE", "moderation/bans?" + twitchMod + "&user_id=" + id);
        else await helix("POST", "moderation/bans?" + twitchMod, { data: Object.assign({ user_id: id }, cmd.do === "timeout" ? { duration: secs } : {}, cmd.reason ? { reason: String(cmd.reason).slice(0, 500) } : {}) });
      }
      const long = secs % 3600 === 0 ? secs / 3600 + (secs === 3600 ? " hour" : " hours") : secs % 60 === 0 ? secs / 60 + (secs === 60 ? " minute" : " minutes") : secs + " seconds";
      return cmd.do === "timeout" ? "Timed out " + who + " for " + long + "." : cmd.do === "ban" ? "Banned " + who + "." : "Let " + who + " back in.";
    }
    case "rerun": {
      const e = cmd.event || {};
      const ctx = { user: e.user || "", userId: e.userId || "", amount: e.amount || "", message: e.message || "", rawInput: e.rawInput || "", reward: e.reward || "", currency: e.currency || "", winner: e.winner || "", platform: e.platform || "twitch", replay: true };
      Object.assign(ctx, words(ctx.rawInput));
      fire(String(e.type || ""), ctx);
      return "Ran " + (e.type || "its") + "'s actions again" + (ctx.user ? " for " + ctx.user : "") + ".";
    }
  }
  throw new Error("a dock can't " + cmd.do);
}

// ---------- donations: Streamlabs' and StreamElements' own sockets ----------
let tipSockets = [];

// A donation, from whichever: its alert (if alerts are on) and A Donation's actions
function onTip(user, amount, currency, message, from) {
  const value = String(Math.round(num(amount) * 100) / 100);
  if (S().alerts !== false) sendAlert({ type: "donation", user, amount: (currency ? currency + " " : "") + value, message });
  const ctx = { user, userId: "", amount: value, currency: currency || "", message: message || "", rawInput: message || "", platform: "all", service: from };
  Object.assign(ctx, words(ctx.rawInput));
  fire("donation", ctx);
}

// (each kept going: tried again a little later if it closes, while the chatbot runs)
function tipSocket(url, opened, onmessage, label) {
  const s = OmniPulse.socket(url);
  s.label = label;
  tipSockets.push(s);
  let ping = null;
  s.onmessage = (data) => { s.up = true; onmessage(s, String(data), (ms) => { if (ping) clearInterval(ping); ping = setInterval(() => s.send("2"), ms); }); };
  if (opened) opened(s);
  s.onclose = () => {
    if (ping) clearInterval(ping);
    tipSockets = tipSockets.filter((x) => x !== s);
    if (!stopped) setTimeout(() => { if (!stopped) tipConnect(label); }, 30000);
  };
}

function tipConnect(only) {
  const sl = String(S().streamlabsToken || "").trim(), se = String(S().streamelementsToken || "").trim();
  // Streamlabs: socket.io (its version 3): "0" opens (how often to ping), "42" an event
  if (sl && (!only || only === "streamlabs")) tipSocket("wss://sockets.streamlabs.com/socket.io/?token=" + encodeURIComponent(sl) + "&EIO=3&transport=websocket", null, (s, d, pings) => {
    if (d.startsWith("0{")) { try { pings(num(JSON.parse(d.slice(1)).pingInterval) || 25000); } catch (e) { pings(25000); } return; }
    if (!d.startsWith("42")) return;
    let m;
    try { m = JSON.parse(d.slice(2)); } catch (e) { return; }
    const e = m[1] || {};
    if (m[0] !== "event" || e.type !== "donation") return;
    for (const t of [].concat(e.message || [])) onTip(t.name || t.from || "Someone", t.amount, t.currency, t.message, "streamlabs");
  }, "streamlabs");
  // StreamElements: its Astro socket, tips' topic
  if (se && (!only || only === "streamelements")) tipSocket("wss://astro.streamelements.com", (s) => {
    s.send(JSON.stringify({ type: "subscribe", nonce: "omnipulse" + Date.now(), data: { topic: "channel.tips", token: se, token_type: "jwt" } }));
  }, (s, d) => {
    let m;
    try { m = JSON.parse(d); } catch (e) { return; }
    if (m.type === "error") { OmniPulse.app.say("StreamElements didn't take its token: " + ((m.data && m.data.message) || m.error || "check it in the chatbot's settings")).catch(() => {}); return; }
    if (m.type !== "message" || m.topic !== "channel.tips") return;
    const x = m.data || {}, t = x.donation || x;
    onTip((t.user && (t.user.username || t.user.name)) || t.username || t.name || "Someone", t.amount, t.currency, t.message, "streamelements");
  }, "streamelements");
}

function stopAll() {
  stopped = true;
  timers.forEach((id) => clearInterval(id));
  timers = [];
  if (pointsTimer) { clearInterval(pointsTimer); pointsTimer = null; }
  if (kickRefresh) { clearInterval(kickRefresh); kickRefresh = null; }
  for (const s of [chat, botChat, events, obs, kick]) { if (s) { s.onclose = null; s.close(); } }
  chat = botChat = events = obs = kick = null;
  for (const s of tipSockets) { s.onclose = null; s.close(); }
  for (const d of Object.values(duels)) clearTimeout(d.off);
  if (trivia) clearTimeout(trivia.off);
  duels = {}; heists = {}; trivia = null;
  tipSockets = [];
  obsReady = false;
}

function restart() { start().catch((e) => OmniPulse.log("start: " + e.message)); }

OmniPulse.register({
  start() { restart(); return true; },
  dock(cmd) { return dockDo(cmd || {}); },
  suiteTest(kind) { return suiteTest(String(kind || "")); },
  suiteShoutout() { return shoutout(); },
  suiteTimer(what) { return timerDo(String(what || "").split(":")[0], num(String(what || "").split(":")[1])) || "Done."; },
  suiteRefresh() { suitePush(); return "ok"; },
  // the Actions page's pickers: your rewards on Twitch and Kick, and OBS's scenes and sources (each one only
  // if it's reachable now)
  async pickLists() {
    const out = { rewards: [], scenes: [], sources: [] };
    const jobs = [];
    if (S().token && S().userId) jobs.push(helix("GET", "channel_points/custom_rewards?broadcaster_id=" + S().userId).then((j) => (j.data || []).forEach((r) => out.rewards.push({ platform: "twitch", id: r.id, title: r.title, cost: r.cost }))).catch(() => {}));
    if (S().kickToken) jobs.push(kickApi("GET", "channels/rewards").then((j) => (j.data || []).forEach((r) => out.rewards.push({ platform: "kick", id: String(r.id), title: r.title, cost: r.cost }))).catch(() => {}));
    if (obs && obsReady) {
      jobs.push(obsCall("GetSceneList").then((d) => { out.scenes = (d.scenes || []).map((x) => x.sceneName).reverse(); }).catch(() => {}));
      jobs.push(obsCall("GetInputList").then((d) => { out.sources = (d.inputs || []).map((x) => x.inputName).sort((a, b) => a.localeCompare(b)); }).catch(() => {}));
    }
    await Promise.all(jobs);
    return out;
  },
  // a new Channel Points reward, from the Actions page's wizard: on Twitch, Kick or both
  async createReward(f) {
    const made = [], failed = [];
    if (f.where !== "kick") {
      try {
        if (!S().token) throw new Error("sign in to Twitch first");
        const body = { title: String(f.title).slice(0, 45), cost: Math.max(1, Math.round(num(f.cost))), prompt: String(f.prompt || "").slice(0, 200), is_user_input_required: !!f.input, background_color: f.colour,
          is_global_cooldown_enabled: num(f.cooldown) > 0, global_cooldown_seconds: Math.max(1, Math.round(num(f.cooldown))),
          is_max_per_stream_enabled: num(f.perStream) > 0, max_per_stream: Math.max(1, Math.round(num(f.perStream))),
          is_max_per_user_per_stream_enabled: num(f.perUser) > 0, max_per_user_per_stream: Math.max(1, Math.round(num(f.perUser))),
          should_redemptions_skip_request_queue: !!f.skip };
        const j = await helix("POST", "channel_points/custom_rewards?broadcaster_id=" + S().userId, body);
        const r = (j.data || [])[0];
        if (!r) throw new Error("Twitch didn't say");
        made.push({ platform: "twitch", id: r.id });
      } catch (e) { failed.push("Twitch: " + e.message); }
    }
    if (f.where !== "twitch") {
      try {
        if (!S().kickToken) throw new Error("sign in to Kick first");
        const j = await kickApi("POST", "channels/rewards", { title: String(f.title).slice(0, 50), cost: Math.max(1, Math.round(num(f.cost))), description: String(f.prompt || ""), is_user_input_required: !!f.input, background_color: f.colour, should_redemptions_skip_request_queue: !!f.skip });
        const r = j.data || {};
        if (!r.id) throw new Error("Kick didn't say");
        made.push({ platform: "kick", id: String(r.id) });
      } catch (e) { failed.push("Kick: " + e.message + (/403|scope|permission/i.test(e.message) ? " (sign in to Kick again in the Suite, for the permission)" : "")); }
    }
    return { made, failed: failed.join("; ") };
  },
  signInChannel(values) { return signIn("channel", TWITCH_SCOPES_CHANNEL, values); },
  signInBot(values) { return signIn("bot", TWITCH_SCOPES_BOT, values); },
  signInChannelCopy(values) { return signIn("channel", TWITCH_SCOPES_CHANNEL, values, true); },
  signInBotCopy(values) { return signIn("bot", TWITCH_SCOPES_BOT, values, true); },
  signInKick(values) { return signInKick(values); },
  signInKickCopy(values) { return signInKick(values, true); },
  signInKickBot(values) { return signInKick(values, false, "bot"); },
  signInKickBotCopy(values) { return signInKick(values, true, "bot"); },
  async signOutChannel() { return signOutOf({ token: "", refresh: "", login: "", userId: "" }, "Signed your channel out of Twitch."); },
  async signOutBot() { return signOutOf({ botToken: "", botRefresh: "", botLogin: "" }, "Signed the bot out of Twitch."); },
  async signOutKickChannel() { return signOutOf({ kickToken: "", kickRefreshToken: "", kickUserId: "", kickLogin: "" }, "Signed your channel out of Kick."); },
  async signOutKickBot() { return signOutOf({ kickBotToken: "", kickBotRefreshToken: "", kickBotLogin: "" }, "Signed the bot out of Kick."); },
  tryChat(text, platform, asMod) { return tryChat(text, platform, asMod); },
  status() {
    const tip = (label) => tipSockets.some((x) => x.label === label && x.up);
    return {
      running: !stopped,
      twitch: { clientId: !!String(S().twitchClientId || "").trim(), login: S().login || "", bot: S().botLogin || "", up: !!(chat && chat.up) },
      kick: { on: yes(S().kickOn), app: !!String(S().kickClientId || "").trim() && (!!String(S().kickClientSecret || "").trim() || !!String(S().kickRelay || "").trim()), login: S().kickLogin || "", bot: S().kickBotLogin || "", channel: S().kickChannel || "", up: !!(kick && kick.up) },
      obs: { on: yes(S().obsOn), up: !!obsReady },
      streamlabs: { on: !!String(S().streamlabsToken || "").trim(), up: tip("streamlabs") },
      streamelements: { on: !!String(S().streamelementsToken || "").trim(), up: tip("streamelements") },
      counts, recent: recent.slice(-12).reverse(), actions: { all: all.length, on: live.length },
    };
  },
  async signOutKick() {
    const none = { kickToken: "", kickRefreshToken: "", kickUserId: "", kickLogin: "", kickBotToken: "", kickBotRefreshToken: "", kickBotLogin: "" };
    await OmniPulse.app.saveSettings(none);
    Object.assign(OmniPulse.settings, none);
    restart();
    return "Signed out of Kick.";
  },
  // the Actions page: its actions as they're changed (kept by the app), and Run Now
  setActions(list) { OmniPulse.settings.actions = list; loadActions(); return true; },
  // (OBS's sign-in, for its test)
  obsAuth(password, salt, challenge) { return obsAuth(password, salt, challenge); },
  async runAction(name) {
    loadActions();
    const a = all.find((x) => String(x.name || "").toLowerCase() === String(name || "").toLowerCase());
    if (!a) return "There's no action called " + name + ".";
    if (!chat && !obs && !kick) return "Not in your chat yet: sign in to Twitch or Kick first (in the chatbot's settings).";
    await run(a, { user: S().login || S().kickLogin || "", rawInput: "", message: "", platform: "all" });
    return "Ran " + a.name + ".";
  },
  // the app's own: a song starting, a song requested
  appEvent(kind, data) {
    const d = data || {};
    if (kind === "web") {
      const want = String(d.name || "").trim().toLowerCase();
      const hits = live.filter((a) => a.trigger.type === "web" && String(a.trigger.name || a.name || "").trim().toLowerCase() === want);
      hits.forEach((a) => run(a, Object.assign({ user: d.user || "", rawInput: d.input || "", message: d.input || "", platform: "all" }, words(d.input || ""))));
      return hits.length ? "Ran " + hits.map((a) => a.name).join(", ") + "." : "There's no action called " + d.name + " with A Web Address as its trigger.";
    }
    if (kind === "process") {
      const plain = (n) => String(n || "").trim().toLowerCase().replace(/^.*[\\/]/, "").replace(/\.exe$/, "");
      for (const a of live) if (a.trigger.type === "process" && plain(a.trigger.name) === plain(d.name)) run(a, { user: S().login || S().kickLogin || "", platform: "all", path: d.path || "", process: d.name || "" });
      return true;
    }
    if (kind === "hotkey") {
      const keys = (k) => String(k || "").toLowerCase().replace(/\s+/g, "").replace(/win|windows|cmd|command/g, "super").replace(/control/g, "ctrl");
      for (const a of live) if (a.trigger.type === "hotkey" && keys(a.trigger.keys) === keys(d.keys)) run(a, { user: S().login || S().kickLogin || "", platform: "all" });
      return true;
    }
    // (a new song: the skip votes start again)
    if (kind === "start") { fun.votes.clear(); fun.song = [d.title, d.artist].filter(Boolean).join(" — "); suitePush(); }
    for (const a of live) {
      if (a.trigger.type !== "song" || (a.trigger.when || "start") !== kind) continue;
      run(a, { user: d.user || "", song: d.title || "", artist: d.artist || "", platform: "all" });
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
