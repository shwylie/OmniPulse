// Chat song requests.

// ---------- chat song requests ----------
function reqKeyNew() {
  const a = new Uint8Array(12);
  crypto.getRandomValues(a);
  return Array.from(a, (b) => b.toString(16).padStart(2, "0")).join("");
}

const reqPendingList = () => queue.slice(qi + 1).filter((t) => t.requestedBy);
// Settings > Song Requests > Limits: on unless switched off (then no most waiting, none per viewer)
const reqLimited = () => prefs.reqLimits !== false;

function reqPaint() {
  const names = activePlats("search").map((id) => PLATFORMS[id].name);
  $("reqPlat").textContent = names.length
    ? "Requests search " + names.join(", then ") + " (a link goes straight to its own platform). Switch platforms with the icons in the title bar."
    : "Requests are going nowhere: no platform is switched on.";
  $("reqLimitsOn").checked = reqLimited();
  $("reqMaxRow").hidden = $("reqUserRow").hidden = !reqLimited();
  $("reqMaxVal").textContent = prefs.reqMax || 10;
  $("reqUserVal").textContent = prefs.reqPerUser || 2;
  const p = reqPendingList();
  $("reqPending").textContent = p.length
    ? p.map((t) => '"' + t.title + '" for ' + t.requestedBy).join(", ")
    : "None.";
}

async function reqApply() {
  const on = !!prefs.req;
  $("reqOn").checked = on;
  $("requestSection").hidden = !on;
  if (!on) {
    invoke("requests_stop").catch(() => {});
    return;
  }
  if (!prefs.reqKey) { prefs.reqKey = reqKeyNew(); savePrefs(); }
  const port = Number(prefs.reqPort) || 7879;
  $("reqPort").value = port;
  try {
    await invoke("requests_start", { port, key: prefs.reqKey, remote: !!prefs.botRemote });
    const base = "http://127.0.0.1:" + port;
    $("reqUrl").value = base + "/request?key=" + prefs.reqKey + "&user=USER&q=SONG";
    $("reqNowUrl").value = base + "/now";
    $("reqNote").textContent = "";
    reqRenderBot();
    widgetPush();
  } catch (e) {
    $("reqNote").textContent = String(e);
  }
  reqPaint();
}

// Called from the Rust side for each request; it answers with request_result
window.__songRequest = async (id, q, user, ask) => {
  let result;
  try {
    result = ask && ask.action === "wrongsong"
      ? handleWrongSong(String(user || "").trim(), !!ask.mod)
      : await handleSongRequest(String(q || "").trim(), String(user || "").trim());
  } catch (e) {
    result = { ok: false, message: "Couldn't add that one: " + e };
  }
  invoke("request_result", { id, result }).catch(() => {});
};

async function findRequested(q) {
  if (/^https?:\/\/(www\.|m\.)?soundcloud\.com\//i.test(q)) {
    try {
      const found = asList([await api("/resolve?url=" + encodeURIComponent(q.split("?")[0]))]);
      if (found.length) return found;
    } catch (e) {
      // fall back to searching for the words in the link
    }
    q = q.split("?")[0].replace(/^https?:\/\/[^/]+\//, "").replace(/[\/\-_]+/g, " ");
  }
  return asList(await api("/tracks?q=" + encodeURIComponent(q) + "&access=playable&limit=10&linked_partitioning=true"));
}

// Spotify: a track link, or a search
async function findSpotifyRequested(q) {
  const m = q.match(/open\.spotify\.com\/(?:intl-[a-z-]+\/)?track\/([A-Za-z0-9]+)/i);
  if (m) {
    try {
      const x = await spApi("GET", "/tracks/" + m[1]);
      if (x && x.id) return [spTrack(x)];
    } catch (e) {
      // fall through to a normal search
    }
  }
  return spotifySearchTracks(q);
}

const REQ_MSG_DEFAULTS = {
  added: 'Added "{title}" by {artist} (#{position} in the queue).',
  playing: 'Added "{title}" by {artist}, playing now.',
  notFound: 'Couldn\'t find "{query}" on {platform}.',
  full: "The request queue is full. Try again in a bit!",
  perUser: "{user}, you already have {count} requests waiting.",
  duplicate: '"{title}" is already in the queue.',
  closed: "Song requests are closed right now.",
  notSafe: 'Only music that\'s safe on stream plays right now, and nothing for "{query}" is.',
  empty: "Tell me what to play, for example: artist - song",
  wrongSong: 'Removed "{title}" by {artist} (asked for by {by}).',
  noRequest: "{user}, you don't have a request waiting.",
  noRequests: "There's no request waiting.",
};
const REQ_MSG_LABELS = {
  added: "Added", playing: "Added, playing now", notFound: "Not found", full: "Queue full",
  perUser: "Viewer limit", duplicate: "Already queued", closed: "Requests off", notSafe: "Not safe on stream",
  empty: "Nothing asked for",
  wrongSong: "!wrongsong: removed", noRequest: "!wrongsong: none of theirs", noRequests: "!wrongsong: none at all (a mod)",
};

function reqMsg(key, vars) {
  const tpl = ((prefs.reqMsgs || {})[key] || "").trim() || REQ_MSG_DEFAULTS[key];
  return tpl.replace(/\{(\w+)\}/g, (m, k) => (vars && vars[k] !== undefined ? String(vars[k]) : m));
}

// The words in a request or a title, for matching against your likes
const reqWords = (s) => String(s).toLowerCase().normalize("NFKD").replace(/[\u0300-\u036f]/g, "")
  .split(/[^a-z0-9\u00c0-\u024f\u0400-\u04ff\u3040-\u9fff]+/).filter((w) => w.length > 1 || /\d/.test(w));

async function likedMatch(q, plats) {
  const want = reqWords(q);
  if (!want.length) return null;
  await loadLikesFor(plats).catch(() => null);
  const pools = { soundcloud: () => likedTracks.filter(isLikedTrack), spotify: () => spLikedCache || [], youtube: () => ytLikes, local: () => [] };
  for (const id of plats) {
    if (!pools[id]) continue;   // plugins: no likes kept here
    // the same song, not just one with those words in it ("Beauty and a Strobe" isn't "Strobe")
    const hit = pools[id]().find((t) => !isMix(t) && reqSameSong(t, q));
    if (hit) return Object.assign({}, hit);   // a copy, so "requested by" stays on the queued one
  }
  return null;
}

// ---------- what chat asked for, tidied ----------
// Punctuation is dropped ("Artist - Song", "Song: Part 2", quotes, brackets), so it never breaks a
// search, and "Song by Artist" is searched as "artist song". A title with "by" in it still works:
// the words as typed (minus punctuation) are searched too if that finds nothing.
function reqClean(q) {
  return String(q).normalize("NFKC").toLowerCase()
    .replace(/[\u2018\u2019\u02bc]/g, "'")
    .replace(/(^|\s)'+|'+(\s|$)/g, " ")             // quotes around words, not apostrophes in them
    .replace(/[^\p{L}\p{N}'&+#$!]+/gu, " ")          // - : | / , . ( ) [ ] " and the like
    .replace(/(\s)[!]+|[!]+(\s|$)/g, " ")             // "!" between words, not in names like P!nk
    .replace(/\s+/g, " ").trim();
}

// The searches to try, best first: { q, artist } where artist (from "song by artist") has to match
// a result's artist, so "Stand by Me" isn't taken as a song called "Stand" by "Me"
function reqQueries(q) {
  const plain = reqClean(q);
  const m = plain.match(/^(.+) by (.+)$/);
  const out = [];
  if (m) out.push({ q: m[2] + " " + m[1], artist: m[2] });   // "i remember by deadmau5" -> "deadmau5 i remember"
  if (plain) out.push({ q: plain });
  return out;
}

const reqByArtist = (list, artist) => !artist ? list : list.filter((t) => {
  // the artist, or the "Artist - " an uploader often puts in front of the title
  const lead = String(t.title || "").includes(" - ") ? String(t.title).split(" - ")[0] : "";
  const have = new Set(reqWords(artistName(t) + " " + lead));
  return reqWords(artist).every((w) => have.has(w));
});

// ---------- radio edits ----------
// Words a version of the same song may add to its title without being another song (a remix, a live
// recording, a cover); version words that make it a different take aren't here
const REQ_VERSION_WORDS = new Set(["radio", "edit", "mix", "version", "original", "single", "short", "clean", "explicit", "the", "a", "and", "feat", "ft", "featuring", "with", "x", "vs", "remastered", "remaster", "mono", "stereo"]);
const REQ_RADIO = /\b(radio (edit|mix|version)|single (edit|version)|short (edit|version)|edit)\b/i;
const REQ_LONG_MS = 6 * 60 * 1000;

const reqLen = (t) => Number(t.duration || t.full_duration || t.duration_ms || 0);

// Whether a track is the same song as the request: every word asked for is there, and anything else in
// its title is only a version word (or the artist's name)
function reqSameSong(t, q) {
  const want = reqWords(q);
  const artist = new Set(reqWords(artistName(t)));
  const title = reqWords(t.title || "");
  const have = new Set(title.concat([...artist]));
  if (!want.every((w) => have.has(w))) return false;
  const asked = new Set(want);
  return title.every((w) => asked.has(w) || artist.has(w) || REQ_VERSION_WORDS.has(w) || /^\d{4}$/.test(w));
}

// Another version of a song (a remix, bootleg, flip, VIP, mashup, cover, live...), as opposed to
// the song itself (a radio edit, the original mix, a remaster are the song itself)
const REQ_SAME_SONG_PHRASES = /\b(radio (edit|mix|version)|single (edit|version)|short (edit|version)|clean (edit|version)|album (version|edit)|original (mix|version)|remaster(ed)?( \d{4})?|\d{4} remaster(ed)?)\b/gi;
const REQ_VERSION_MARK = /\b(remix|rmx|bootleg|boot|flip|vip|mash ?up|cover|live|acoustic|instrumental|sped ?up|slowed|nightcore|karaoke|rework|refix|re-?edit|dub|edit|mix|version|vs)\b/i;
const reqIsVersion = (s) => REQ_VERSION_MARK.test(String(s || "").replace(REQ_SAME_SONG_PHRASES, " "));

// Whether every word asked for is in a track's title or artist (more words are fine)
function reqHasAll(t, q) {
  const have = new Set(reqWords((t.title || "") + " " + artistName(t)));
  return reqWords(q).every((w) => have.has(w));
}

// The best of a search for what was asked for, and how good (lower is better): the song itself that
// you like, the song itself, one with every word asked for in it, one you like, then anything. Unless a
// version was asked for ("... remix"), other versions only come after the song itself; but one with
// every word in it beats one without them ("ghosts n stuff ginsu" is Ginsu's mashup, not the original).
function reqRank(found, asked, isLink) {
  if (isLink) return found.length ? { t: found[0], tier: 0 } : null;
  const wantVersion = reqIsVersion(asked);
  const top = found.slice(0, 10).filter((x) => wantVersion || !reqIsVersion(x.title));
  const same = (x) => reqSameSong(x, asked);
  const all = (x) => reqHasAll(x, asked);
  const tiers = [
    top.slice(0, 5).find((x) => same(x) && isLikedTrack(x)),
    top.find(same),
    top.find(all),
    found.slice(0, 10).find(all),
    top.slice(0, 5).find(isLikedTrack),
    top[0],
    found.slice(0, 5).find(isLikedTrack),
    found[0],
  ];
  const tier = tiers.findIndex(Boolean);
  return tier < 0 ? null : { t: tiers[tier], tier };
}

function reqRadioPick(list, q) {
  const same = list.filter((t) => reqSameSong(t, q) && reqLen(t) >= 90 * 1000);
  const radio = same.filter((t) => REQ_RADIO.test(t.title || ""));
  if (radio.length) return radio.sort((a, b) => reqLen(a) - reqLen(b))[0];
  const short = same.filter((t) => reqLen(t) < REQ_LONG_MS);
  return short.length ? short[0] : null;
}

// With "Prefer radio edits" on: a long track gives way to a radio edit of the same song, from the same
// results or a search for one
async function reqPreferRadio(t, found, q, order, finders) {
  if (!prefs.reqRadioEdit || !t || reqLen(t) < REQ_LONG_MS || REQ_RADIO.test(t.title || "")) return t;
  const pick = reqRadioPick(found, q);
  if (pick) return pick;
  for (const id of order) {
    const more = (await finders[id](q + " radio edit").catch(() => [])).filter((x) => !isMix(x));
    const p = reqRadioPick(more, q);
    if (p) return p;
  }
  return t;
}

// The video id in a YouTube or YouTube Music link
function ytLinkId(q) {
  const m = String(q).match(/(?:youtube\.com\/(?:watch\?(?:.*&)?v=|shorts\/)|youtu\.be\/)([\w-]{11})/i);
  return m ? m[1] : "";
}

// A YouTube link plays that song (YouTube Music's radio for it starts with the song itself); words are searched
async function findYouTubeRequested(q) {
  const id = ytLinkId(q);
  if (!id) return ytSearch(q);
  return (await invoke("yt_radio", { id })).map(ytTrack).filter((t) => t.ytId === id).slice(0, 1);
}

async function handleSongRequest(q, user) {
  if (!prefs.req) return { ok: false, message: reqMsg("closed", {}) };
  q = q.slice(0, 200);
  const who = user || "chat";
  const plats = activePlats("search");
  if (!plats.length) return { ok: false, message: reqMsg("closed", { user: who }) };
  const platName = plats.map((id) => PLATFORMS[id].name).join(" or ");
  const base = { user: who, query: q, platform: platName };
  if (!q) return { ok: false, message: reqMsg("empty", base) };
  const pending = reqPendingList();
  if (reqLimited() && pending.length >= (prefs.reqMax || 10)) return { ok: false, message: reqMsg("full", base) };
  const mine = pending.filter((t) => t.requestedBy.toLowerCase() === who.toLowerCase()).length;
  if (reqLimited() && user && mine >= (prefs.reqPerUser || 2)) return { ok: false, message: reqMsg("perUser", Object.assign({ count: mine }, base)) };
  // A link goes to its own platform; otherwise the first active platform with a match wins
  let order = plats;
  if (/soundcloud\.com\//i.test(q)) order = plats.filter((id) => id === "soundcloud");
  else if (/open\.spotify\.com\//i.test(q)) order = plats.filter((id) => id === "spotify");
  else if (ytLinkId(q)) order = plats.filter((id) => id === "youtube");
  const finders = { soundcloud: findRequested, spotify: findSpotifyRequested, local: async (x) => localSearch(x), youtube: findYouTubeRequested, musicbrainz: mbSearch };
  order.filter((id) => id.startsWith("plug:")).forEach((id) => { finders[id] = (x) => pluginSearch(id.slice(5), x); });
  const isLink = /^https?:\/\//i.test(q);
  // words are tidied (no punctuation, "song by artist" as "artist song"); links are left as they are
  const queries = isLink ? [{ q }] : reqQueries(q);
  if (!queries.length) return { ok: false, message: reqMsg("empty", base) };
  // Your liked songs come first: one whose title and artist contain every word asked for
  let t = null, found = [], asked = queries[0].q;
  if (!isLink) for (const qq of queries) { t = await likedMatch(qq.q, order); if (t && !safeOk(t)) t = null; if (t) { asked = qq.q; break; } }
  let unsafe = false;
  // otherwise a search: the song itself (yours if you like it) on the first platform that has it;
  // if none has it, the best that was found anywhere
  let best = null;
  if (!t) {
    search: for (const qq of queries) {
      for (const id of order) {
        let got = reqByArtist((await finders[id](qq.q).catch(() => [])).filter((x) => !isMix(x)), qq.artist);
        if (got.length && streamSafe()) {
          await streamLookups(got.slice(0, 20));
          const open = got.filter(streamOk);
          unsafe = unsafe || !open.length;
          got = open;
        }
        const r = reqRank(got, qq.q, isLink);
        if (r && (!best || r.tier < best.tier)) best = Object.assign(r, { found: got, asked: qq.q });
        if (best && best.tier <= 1) break search;
      }
    }
    if (best) { found = best.found; asked = best.asked; }
    if (!found.length) return { ok: false, message: reqMsg(unsafe ? "notSafe" : "notFound", base) };
    t = best.t;
  }
  if (!isLink) t = await reqPreferRadio(t, found, asked, order, finders);
  if (!safeOk(t)) return { ok: false, message: reqMsg("notSafe", base) };
  const vars = Object.assign({ title: t.title, artist: artistName(t) }, base);
  // already coming up: it moves up to play next (after the other requests), unless it's playing or
  // someone's requested it already
  const at = queue.findIndex((x, k) => k > qi && x.id === t.id);
  if ((qi >= 0 && queue[qi] && queue[qi].id === t.id) || (at > qi && queue[at].requestedBy)) return { ok: false, message: reqMsg("duplicate", vars) };
  if (at > qi) t = queue.splice(at, 1)[0];
  t.requestedBy = who;
  t.requestedAt = Date.now();   // (which is their last, for !wrongsong)
  addToQueue(t);
  reqPaint();
  reqQueueBtn();
  // requests go ahead of autoplay picks, so not always last (and the queue holds a copy)
  const pos = queue.findIndex((x, k) => k > qi && x.id === t.id && x.requestedBy === who) - qi;
  vars.position = pos;
  return {
    ok: true, title: t.title, artist: artistName(t), url: t.permalink_url || "", position: pos,
    message: reqMsg(pos > 0 ? "added" : "playing", vars),
  };
}

// !wrongsong: takes back the viewer's last request; a moderator's takes back the last one anyone made.
// One that's playing already is skipped.
function handleWrongSong(user, isMod) {
  const base = { user: user || "chat" };
  if (!prefs.req) return { ok: false, message: reqMsg("closed", base) };
  const mine = (t) => isMod || (!!user && String(t.requestedBy).toLowerCase() === user.toLowerCase());
  // what's waiting, and what's playing if it was a request
  const cands = queue.map((t, i) => [t, i]).filter(([t, i]) => i >= Math.max(qi, 0) && t.requestedBy && mine(t));
  if (!cands.length) return { ok: false, message: reqMsg(isMod ? "noRequests" : "noRequest", base) };
  // the last asked for (by when; one from before that was kept, by its place)
  const [t, i] = cands.reduce((a, b) => ((b[0].requestedAt || 0) > (a[0].requestedAt || 0) || ((b[0].requestedAt || 0) === (a[0].requestedAt || 0) && b[1] > a[1]) ? b : a));
  const vars = Object.assign({ title: t.title, artist: artistName(t), by: t.requestedBy }, base);
  if (i === qi) {
    if (qi < queue.length - 1) step(1); else if (typeof autoplayActive === "function" && autoplayActive()) autoplayAdvance(); else { audio.pause(); if (typeof spotifyPause === "function") spotifyPause(); }
  } else {
    queue.splice(i, 1);
    renderQueue();
  }
  reqPaint();
  reqQueueBtn();
  return { ok: true, title: t.title, artist: artistName(t), message: reqMsg("wrongSong", vars) };
}

// Chat bot instructions: only the one you use
function reqRenderBot() {
  const bot = prefs.reqBot || "streamerbot";
  $("botSel").value = bot;
  const port = Number(prefs.reqPort) || 7879;
  const base = "http://127.0.0.1:" + port;
  const key = prefs.reqKey || "";
  const vars = {
    streamerbot: { user: "%user%", q: "%rawInputUrlEncoded%", mod: "%isModerator%" },
    mixitup: { user: "$username", q: "$allargs", mod: "$userroles" },
    other: { user: "USERNAME", q: "SONG", mod: "MOD" },
  }[bot];
  const reqAddr = base + "/request?key=" + key + "&user=" + vars.user + "&q=" + vars.q;
  $("reqUrl").value = reqAddr;
  $("reqNowUrl").value = base + "/now";
  $("reqWrongUrl").value = base + "/wrongsong?key=" + key + "&user=" + vars.user + "&mod=" + vars.mod;
  const help = $("botHelp");
  help.replaceChildren();
  const code = (s) => { const c = el("code", "", s); return c; };
  const li = (...parts) => { const x = el("li"); parts.forEach((p) => x.append(typeof p === "string" ? document.createTextNode(p) : p)); return x; };
  const ol = el("ol");
  if (bot === "streamerbot") {
    $("reqUrlLbl").textContent = "Fetch URL Address (variables already filled in)";
    ol.append(
      li("Create an Action called Song request, with a Command trigger such as ", code("!sr"), "."),
      li("Add the sub-action Core > Network > Fetch URL, paste the address above, and set its variable name to ", code("reply"), "."),
      li("Add the sub-action Twitch > Chat > Send Message to Channel with the message ", code("%reply%"), "."),
      li("For ", code("!song"), ": the same again with the now playing address."),
      li("For ", code("!wrongsong"), ": the same again with the wrong song address. A viewer takes back their last request; a moderator takes back the last one anyone made."),
    );
    help.append(ol, el("p", "note", "If your version names things slightly differently, look for the sub-action that fetches a URL and saves the reply in a variable."));
  } else if (bot === "mixitup") {
    $("reqUrlLbl").textContent = "Web Request Address (special identifiers already filled in)";
    ol.append(
      li("Create a Chat Command such as ", code("!sr"), "."),
      li("Add a Web Request action, paste the address above, and set the response to plain text."),
      li("Add a Chat action that sends ", code("$webrequestresult"), "."),
      li("For ", code("!song"), ": the same again with the now playing address."),
      li("For ", code("!wrongsong"), ": the same again with the wrong song address (", code("$userroles"), " tells it who's a moderator). A viewer takes back their last request; a moderator, or you, takes back the last one anyone made."),
    );
    help.append(ol, el("p", "note", "If $webrequestresult doesn't post anything in your version, check Mix It Up's list of special identifiers for the web request result."));
  } else {
    $("reqUrlLbl").textContent = "Request Address (replace USERNAME and SONG with your bot's variables)";
    help.append(
      el("p", "", "Any bot that can call a web address and post the reply in chat will work:"),
      (() => { const l = el("ol"); l.append(
        li("Call ", code(base + "/request"), " with ", code("key"), " (required, from this page), ", code("q"),
           " (what was asked for: search words or a SoundCloud or Spotify link) and ", code("user"),
           " (who asked; used for the per-viewer limit and \"requested by\")."),
        li("Text in ", code("q"), " and ", code("user"), " must be URL-encoded (spaces as %20). Most bots have an encoded version of the message variable."),
        li("The reply is one line of plain text, ready to post in chat. Add ", code("&format=json"),
           " to get JSON instead: ok, message, title, artist, url and position."),
        li("Call ", code(base + "/now"), " for the current song (no key needed). It also supports ", code("&format=json"), "."),
        li("Call ", code(base + "/wrongsong"), " with ", code("key"), ", ", code("user"), " and ", code("mod"),
           " (true for a moderator, or a list of their roles with Moderator or Streamer in it) to take back that viewer's last request; a moderator's takes back the last one anyone made."),
      ); return l; })(),
      botRemoteBlock("requests", port, "/request?key=" + key + "&user=USERNAME&q=SONG"),
    );
  }
  markSensitive(help);
}

function reqQueueBtn() {
  const b = $("qClearReq");
  if (b) b.hidden = !(prefs.req && reqPendingList().length);
}

function clearRequests() {
  queue = queue.filter((t, i) => i <= qi || !t.requestedBy);
  renderQueue();
  reqPaint();
  say("Waiting requests removed.");
}

function renderReqMsgs() {
  const box = $("reqMsgs");
  box.replaceChildren();
  Object.keys(REQ_MSG_DEFAULTS).forEach((k) => {
    const row = el("div", "msgRow");
    const lab = el("label", "", REQ_MSG_LABELS[k]);
    lab.htmlFor = "msg_" + k;
    const inp = el("input");
    inp.id = "msg_" + k;
    inp.type = "text";
    inp.maxLength = 300;
    inp.placeholder = REQ_MSG_DEFAULTS[k];
    inp.value = (prefs.reqMsgs || {})[k] || "";
    inp.dataset.k = k;
    row.append(lab, inp);
    box.append(row);
  });
}

function requestsInit() {
  $("reqRadioOn").checked = !!prefs.reqRadioEdit;
  $("reqRadioOn").addEventListener("change", () => { prefs.reqRadioEdit = $("reqRadioOn").checked; savePrefs(); });
  $("reqOn").addEventListener("change", () => {
    prefs.req = $("reqOn").checked;
    savePrefs();
    reqApply();
    reqQueueBtn();
  });
  $("reqPort").addEventListener("change", () => {
    const p = Math.round(Number($("reqPort").value));
    if (p >= 1024 && p <= 65535) { prefs.reqPort = p; savePrefs(); reqApply(); }
    else $("reqPort").value = prefs.reqPort || 7879;
  });
  $("reqNewKey").addEventListener("click", () => {
    prefs.reqKey = reqKeyNew();
    savePrefs();
    reqApply();
    say("New request key made. Update the address in your chat bot.");
  });
  const copy = (inputId, btnId) => $(btnId).addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText($(inputId).value);
      $(btnId).textContent = "Copied";
      setTimeout(() => { $(btnId).textContent = "Copy"; }, 1500);
    } catch (e) {
      $(inputId).select();
    }
  });
  copy("reqUrl", "reqCopy");
  copy("reqNowUrl", "reqNowCopy");
  copy("reqWrongUrl", "reqWrongCopy");
  const stepper = (key, def, min, max, down, up) => {
    const set = (d) => { prefs[key] = Math.min(max, Math.max(min, (prefs[key] || def) + d)); savePrefs(); reqPaint(); };
    $(down).addEventListener("click", () => set(-1));
    $(up).addEventListener("click", () => set(1));
  };
  stepper("reqMax", 10, 1, 50, "reqMaxDown", "reqMaxUp");
  stepper("reqPerUser", 2, 1, 10, "reqUserDown", "reqUserUp");
  $("reqLimitsOn").addEventListener("change", () => { prefs.reqLimits = $("reqLimitsOn").checked; savePrefs(); reqPaint(); });
  $("reqClear").addEventListener("click", clearRequests);
  $("qClearReq").addEventListener("click", clearRequests);

  // Chat bot picker and editable replies
  $("botSel").addEventListener("change", () => { prefs.reqBot = $("botSel").value; savePrefs(); reqRenderBot(); });
  renderReqMsgs();
  $("reqMsgs").addEventListener("input", (e) => {
    const k = e.target.dataset.k;
    if (!k) return;
    prefs.reqMsgs = prefs.reqMsgs || {};
    prefs.reqMsgs[k] = e.target.value;
    savePrefs();
  });
  $("msgReset").addEventListener("click", () => { prefs.reqMsgs = {}; savePrefs(); renderReqMsgs(); });

  reqApply();
  reqRenderBot();
  reqQueueBtn();
}
