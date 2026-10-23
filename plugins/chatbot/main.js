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

async function signInFlow(who, scopes, values, said, copy) {
  const clientId = ((values && values.twitchClientId) || S().twitchClientId || "").trim();
  if (!clientId) return "Put your Twitch Client ID in first (its note says where), then sign in.";
  const d = await form("https://id.twitch.tv/oauth2/device", { client_id: clientId, scopes });
  if (!d.ok) return "Twitch didn't take that Client ID: " + (d.j.message || d.status) + ". Check its Client Type is Public.";
  await signInPage(d.j.verification_uri, copy);
  const as = who === "bot" ? "as the bot account" : "as your channel";
  said(copy
    ? "The sign-in link is copied: paste it into a private window (or another browser) and sign in " + as + " there. Its code is " + d.j.user_code + ". This says when it's done."
    : "Sign in on Twitch's page that's opened (" + as + "): its code is " + d.j.user_code + ". This says when it's done.");
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

// A reply in chat, on the platform it's for ("all": every one connected): on Twitch from the bot account if
// there is one, else your channel; to the message it answers
function reply(to, text, platform) {
  if (!text) return;
  const p = platform || "twitch";
  if (p === "kick" || p === "all") kickSay(p === "kick" ? to : null, text);
  if (p === "kick") return;
  const s = botChat || chat;
  if (!s) return;
  const lines = String(text).split("\n").filter(Boolean);
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
  if (m.command !== "PRIVMSG") return;
  const platform = m.platform || "twitch";
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
  seen[(m.nick || user).toLowerCase()] = Date.now();
  onActions(m, text, cmd, args, user, id);
  const firstKey = platform + "|" + (m.nick || user).toLowerCase();
  if (!spoken.has(firstKey)) {
    spoken.add(firstKey);
    fire("firstwords", Object.assign({ user, userId: m.tags["user-id"] || "", message: text, rawInput: text, msgId: id, platform, isMod }, words(text)));
  }
  if (platform === "twitch" && m.tags["first-msg"] === "1") onEvent("first.message", { user_name: user, user_id: m.tags["user-id"] || "", message: { text }, msg_id: id });
  if (!text.startsWith("!")) return;
  // (a second from the same viewer: each command at most once every 3 s per viewer)
  const k = cmd + "|" + user.toLowerCase();
  if (cooldowns[k] && Date.now() - cooldowns[k] < 3000) return;
  cooldowns[k] = Date.now();
  if (S().requests !== false) {
    const sr = String(S().srCommands || "!sr, !request").split(",").map((x) => x.trim().toLowerCase()).filter(Boolean);
    if (sr.includes(cmd)) {
      if (!args) return reply(id, "@" + user + " what song? " + cmd + " song by artist", platform);
      try { const r = await OmniPulse.app.request(user, args, platform); reply(id, r.message, platform); } catch (e) { OmniPulse.log("request: " + e.message); }
      return;
    }
    if (cmd === "!song") { try { reply(id, (await OmniPulse.app.nowPlaying()).message, platform); } catch (e) {} return; }
    if (cmd === "!wrongsong") { try { reply(id, (await OmniPulse.app.wrongSong(user, isMod)).message, platform); } catch (e) {} return; }
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

function onActions(m, text, cmd, args, user, msgId) {
  const r = rank(m.tags);
  const months = /subscriber\/(\d+)/.exec(m.tags["badge-info"] || "");
  const base = { user, userId: m.tags["user-id"] || "", message: text, msgId, platform: m.platform || "twitch", isMod: r.mod, submonths: months ? months[1] : "" };
  for (const a of live) {
    const t = a.trigger;
    if (!onPlatform(t, base.platform)) continue;
    if (t.type === "filter") {
      if (exempt(t.exempt, r) || !breaks(t, text, m.tags)) continue;
      run(a, Object.assign({}, base, { rawInput: text }, words(text)));
    } else if (t.type === "command") {
      const names = [t.command].concat(String(t.aliases || "").split(",")).map((x) => String(x || "").trim().toLowerCase()).filter(Boolean);
      if (!names.includes(cmd) || !allowed(t.who, r) || !cooled(a, user)) continue;
      run(a, Object.assign({}, base, { command: cmd, rawInput: args, rawInputUrlEncoded: encodeURIComponent(args) }, words(args)));
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
  "first.message": "firstmsg", "stream.online": "online", "stream.offline": "offline", "channel.ad_break.begin": "adbreak",
};

// A trigger's platform (On: any, Twitch or Kick), and where this came from
function onPlatform(t, platform) {
  const want = t.from || "any";
  return want === "any" || platform === "all" || platform === want;
}

// An event (ours: follow, sub...), to the actions waiting for it
function fire(ev, ctx) {
  for (const x of live) {
    const t = x.trigger;
    if (t.type !== "event" || t.event !== ev || !onPlatform(t, ctx.platform)) continue;
    if (num(t.min) > 0 && num(ctx.amount) < num(t.min)) continue;
    if (ev === "reward" && String(t.reward || "").trim() && String(t.reward).trim().toLowerCase() !== String(ctx.reward || "").toLowerCase()) continue;
    run(x, ctx);
  }
}

function onEvent(type, e) {
  const ev = EVENT_OF[type];
  if (!ev) return;
  if (ev === "online") { isLive = true; spoken.clear(); }
  if (ev === "offline") isLive = false;
  const a = asAlert(type, e) || {};
  if (ev === "sub" && e.is_gift) return;
  const amount = ev === "bits" ? String(e.bits || 0) : ev === "raid" ? String(e.viewers || 0) : ev === "gift" ? String(e.total || 1) : ev === "resub" ? String(e.cumulative_months || 1) : ev === "hypetrain" ? String(e.level || 1) : "";
  const reward = (e.reward && e.reward.title) || "";
  const said = (e.message && e.message.text) || "";
  const ctx = { user: a.user || e.user_name || "", userId: e.user_id || e.from_broadcaster_user_id || "", amount: ev === "adbreak" ? String(e.duration_seconds || 0) : amount, message: a.message || said || e.user_input || "", reward, rawInput: e.user_input || said || "", msgId: e.msg_id || "", platform: "twitch" };
  Object.assign(ctx, words(ctx.rawInput));
  fire(ev, ctx);
}

// Every few minutes: each timed action, if chat's said enough since
function setTimers() {
  timers.forEach((id) => clearInterval(id));
  timers = [];
  for (const a of live) {
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
  try {
    await runBlocks(blocks(a.steps || [], 0, null).list, ctx, depth);
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
  const say = (to, text) => reply(to, text, ctx.platform);
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
      if (onKick) {
        const c = await kickApi("GET", "categories?q=" + encodeURIComponent(f("text").trim()));
        const id = c.data && c.data[0] && c.data[0].id;
        if (!id) throw new Error("Kick has no category called " + f("text"));
        await kickApi("PATCH", "channels", { category_id: id });
        break;
      }
      const g = await helix("GET", "games?name=" + encodeURIComponent(f("text").trim()));
      const id = g.data && g.data[0] && g.data[0].id;
      if (!id) throw new Error("Twitch has no category called " + f("text"));
      await helix("PATCH", "channels?broadcaster_id=" + S().userId, { game_id: id });
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
let entries = [], pointsTimer = null, saveSoon = null, isLive = false;

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
const KICK_SCOPES = "user:read channel:read channel:write chat:write moderation:ban moderation:chat_message:manage";
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
  if (!id || !secret) return "Put your Kick app's Client ID and Client Secret in first (their note says where), then sign in.";
  const verifier = randomText(64), state = randomText(24);
  const q = { client_id: id, response_type: "code", redirect_uri: KICK_REDIRECT, scope: bot ? KICK_BOT_SCOPES : KICK_SCOPES, state, code_challenge: b64url(sha256(utf8(verifier))), code_challenge_method: "S256" };
  const waiting = OmniPulse.app.awaitRedirect(KICK_REDIRECT_PORT, "/kick");
  await signInPage("https://id.kick.com/oauth/authorize?" + Object.entries(q).map(([k, x]) => k + "=" + encodeURIComponent(x)).join("&"), copy);
  const as = bot ? "as the bot account" : "as your channel";
  said(copy
    ? "The sign-in link is copied: paste it into a private window (or another browser) on this PC and sign in to Kick " + as + " there, within five minutes. This says when it's done."
    : "Sign in on Kick's page that's opened (" + as + "). This says when it's done.");
  const back = await waiting;
  const got = {};
  String(back.query || "").split("&").forEach((kv) => { const i = kv.indexOf("="); if (i > 0) got[kv.slice(0, i)] = decodeURIComponent(kv.slice(i + 1).replace(/\+/g, " ")); });
  if (got.error) return "Kick's sign-in didn't finish: " + (got.error_description || got.error);
  if (got.state !== state || !got.code) return "Kick's sign-in didn't come back as it should: try again.";
  const t = await form("https://id.kick.com/oauth/token", { grant_type: "authorization_code", client_id: id, client_secret: secret, redirect_uri: KICK_REDIRECT, code: got.code, code_verifier: verifier });
  if (!t.ok || !t.j.access_token) return "Kick didn't take the sign-in: " + (t.j.error_description || t.j.message || t.status) + ". Check its Client Secret, and that its Redirect URL is " + KICK_REDIRECT + ".";
  const p = bot ? "kickBot" : "kick";
  const keep = { kickClientId: id, kickClientSecret: secret, [p + "Token"]: t.j.access_token, [p + "RefreshToken"]: t.j.refresh_token || "" };
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

async function kickRenew(who) {
  const s = S();
  const p = who === "bot" ? "kickBot" : "kick";
  if (!s[p + "RefreshToken"] || !s.kickClientId || !s.kickClientSecret) return false;
  const t = await form("https://id.kick.com/oauth/token", { grant_type: "refresh_token", client_id: s.kickClientId, client_secret: s.kickClientSecret, refresh_token: s[p + "RefreshToken"] });
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
  if (ev === "ChatMessageEvent") {
    const sender = d.sender || {};
    const content = String(d.content || "");
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
  const alert = (a) => { if (S().alerts !== false) OmniPulse.app.alert(Object.assign({ platform: "kick" }, a)).catch((e) => OmniPulse.log("alert: " + e.message)); };
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
    case "StreamerIsLive": spoken.clear(); go("online", {}); return;
    case "StopStreamBroadcast": go("offline", {}); return;
  }
  // (one Kick sends that OmniPulse hasn't a use for yet: noted once, to add later)
  if (!KICK_SEEN[ev]) { KICK_SEEN[ev] = true; OmniPulse.log("kick event not used yet: " + ev + " " + JSON.stringify(d).slice(0, 300)); }
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
  stopped = false;
  loadActions();
  setPoints();
  obsConnect();
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

function stopAll() {
  stopped = true;
  timers.forEach((id) => clearInterval(id));
  timers = [];
  if (pointsTimer) { clearInterval(pointsTimer); pointsTimer = null; }
  if (kickRefresh) { clearInterval(kickRefresh); kickRefresh = null; }
  for (const s of [chat, botChat, events, obs, kick]) { if (s) { s.onclose = null; s.close(); } }
  chat = botChat = events = obs = kick = null;
  obsReady = false;
}

function restart() { start().catch((e) => OmniPulse.log("start: " + e.message)); }

OmniPulse.register({
  start() { restart(); return true; },
  signInChannel(values) { return signIn("channel", TWITCH_SCOPES_CHANNEL, values); },
  signInBot(values) { return signIn("bot", TWITCH_SCOPES_BOT, values); },
  signInChannelCopy(values) { return signIn("channel", TWITCH_SCOPES_CHANNEL, values, true); },
  signInBotCopy(values) { return signIn("bot", TWITCH_SCOPES_BOT, values, true); },
  signInKick(values) { return signInKick(values); },
  signInKickCopy(values) { return signInKick(values, true); },
  signInKickBot(values) { return signInKick(values, false, "bot"); },
  signInKickBotCopy(values) { return signInKick(values, true, "bot"); },
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
