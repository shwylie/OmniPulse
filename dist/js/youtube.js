// YouTube Music, without signing in: search and radio from YouTube Music itself (crates/core/src/youtube.rs),
// audio through yt-dlp, played via the local server's /yt (which fetches it in pieces). Likes are kept on
// this PC, like favourites, so they show in your Library and seed autoplay and Rediscover.

let ytInfo = { ytdlp: null, deno: false, node: false, checked: false };
const ytAvailable = () => !!ytInfo.ytdlp && (ytInfo.deno || ytInfo.node);

// A YouTube track's video id: its own, or from its id ("yt:<video>"). Tracks kept on this PC (likes,
// playlists, the queue) were saved without ytId before, so it's never taken for granted.
const ytIdOf = (t) => (t && (t.ytId || (String(t.id || "").startsWith("yt:") ? String(t.id).slice(3) : ""))) || "";

function ytTrack(x) {
  return tidyTrack({
    source: "youtube",
    id: "yt:" + x.id,
    ytId: x.id,
    title: x.title,
    user: { username: x.artists || "Unknown artist" },
    ytArtistId: x.artist_id || "",
    album: x.album || "",
    duration: x.duration_ms || 0,
    artwork_url: x.thumb || "",
    permalink_url: "https://www.youtube.com/watch?v=" + x.id,
  });
}

async function ytSearch(q) {
  return (await invoke("yt_search", { query: q })).map(ytTrack);
}

// YouTube Music's radio for a song (the song itself is left out)
async function ytRadio(videoId) {
  return (await invoke("yt_radio", { id: videoId })).map(ytTrack).filter((t) => t.ytId !== videoId);
}

// The YouTube Music song for any track: itself, or the best search match for its artist and title
async function ytFind(t) {
  if (srcOf(t) === "youtube") return t;
  const found = await ytSearch((seedArtist(t) + " " + guessNames(t).title).trim()).catch(() => []);
  return found[0] || null;
}

// ---------- the same song on YouTube Music ----------
// For tracks that can't be played where they're from (Go+ previews, Spotify without Premium). The
// search results are scored on title words, artist, length and version tags (remix, live, ...), and
// a weak best match counts as none rather than playing the wrong song. Matches are remembered.
const YT_MATCH_KEY = "sc.ytMatch";
let ytMatches = null;   // track id -> YouTube track snapshot, or null for "none found"

const ytOn = () => ytWanted() && ytAvailable();
// What uploads add to a title that says nothing about the song: "(Official Audio)", "[Lyric Video]",
// "| Official Music Video". Only bracketed or trailing parts go, so "Video Games" stays a title.
const UPLOAD_TAG = /\b(official|audio|video|lyrics?|visuali[sz]er|hd|hq|4k|mv|m\/v)\b/i;
const uploadTitle = (s) => String(s || "")
  .replace(/[(\[]([^)\]]*)[)\]]/g, (m, inner) => (UPLOAD_TAG.test(inner) ? " " : m))
  .replace(/\s[|\uFF5C]\s.*$/, " ");
const VERSION_WORDS = ["remix", "edit", "mix", "vip", "flip", "bootleg", "live", "acoustic", "instrumental", "remaster", "cover", "sped", "slowed", "nightcore", "karaoke"];

// A track's title for matching: without "Artist - " in front (SoundCloud and uploads put it there) or
// upload tags, but with its version: "(Remix)", "(Extended Mix)", Spotify's " - Radio Edit"
function matchTitle(t) {
  let title = String((t && t.title) || "").trim();
  if (srcOf(t) !== "spotify") {
    const m = title.match(/^(.+?)\s+[-\u2013\u2014]\s+(.+)$/);
    if (m) title = m[2];
  }
  return uploadTitle(title);
}

function matchWords(s) {
  return String(s || "").replace(/[(\[]\s*original( mix| version)?\s*[)\]]|\boriginal mix\b/gi, " ").toLowerCase().normalize("NFKD").replace(/[\u0300-\u036f]/g, "")
    .replace(/\b(feat|ft|featuring|prod|with)\b\.?/g, " ").replace(/[^a-z0-9]+/g, " ").trim().split(" ").filter(Boolean);
}

function ytMatchScore(t, c) {
  const names = guessNames(t);
  const artist = matchWords(names.artist || artistName(t));
  const want = new Set(matchWords(matchTitle(t)));
  // the artist's name in an upload's title ("Artist - Song") isn't part of the song's title
  const got = new Set(matchWords(uploadTitle(c.title)).filter((w) => !(artist.includes(w) && !want.has(w))));
  if (!want.size || !got.size) return 0;
  let shared = 0;
  want.forEach((w) => { if (got.has(w)) shared++; });
  // the title has to be (nearly) the same: "Track 7" is not "Track 8"
  let score = shared / Math.max(want.size, got.size);
  if (score < 0.75) return 0;
  // and so does the artist (credited, or named in the title): otherwise it's likely a cover
  const where = new Set(matchWords(artistName(c) + " " + c.title));
  if (artist.length && artist.some((w) => where.has(w))) score += 0.3;
  else score -= 0.6;
  // a remix isn't the original (and the other way round)
  VERSION_WORDS.forEach((v) => { if (want.has(v) !== got.has(v)) score -= 0.6; });
  // the length, when both are known
  const d1 = t.full_duration || t.duration || 0;
  const d2 = c.duration || 0;
  if (d1 > 40000 && d2) {
    const off = Math.abs(d1 - d2) / d1;
    score += off < 0.03 ? 0.3 : off < 0.1 ? 0.1 : off > 0.25 ? -0.5 : 0;
  }
  return score;
}

// The artist is credited (or named in the title): not a cover
function artistHit(t, c) {
  const artist = matchWords(guessNames(t).artist || artistName(t));
  const where = new Set(matchWords(artistName(c) + " " + c.title));
  return !artist.length || artist.some((w) => where.has(w));
}

const YT_NO_MATCH_TTL = 86400000;   // "no match" is looked for again after a day
let ytLastMiss = null;              // { id, closest }: why the last match came up empty

const ytSnap = (c) => ({ id: ytIdOf(c), title: c.title, artists: artistName(c), album: c.album, duration_ms: c.duration, thumb: c.artwork_url });
function ytMatchesLoad() {
  if (!ytMatches) { try { ytMatches = JSON.parse(localStorage.getItem(YT_MATCH_KEY) || "{}"); } catch (e) { ytMatches = {}; } }
  return ytMatches;
}
function ytMatchesSave() { try { localStorage.setItem(YT_MATCH_KEY, JSON.stringify(ytMatches)); } catch (e) {} }

// The YouTube Music search for a track, best match first
async function ytCandidates(t, query) {
  const names = guessNames(t);
  const found = await ytSearch(query || (names.artist || artistName(t)) + " " + names.title);
  return found.slice(0, 10).map((c) => ({ c, score: ytMatchScore(t, c) })).sort((a, b) => b.score - a.score);
}

// any: for a track that can't play at all otherwise (Go+), the best of the close ones is taken, even if
// it's another version or a different length, as long as the title and the artist match
async function ytMatch(t, any) {
  if (srcOf(t) === "youtube") return t;
  const saved = ytMatchesLoad()[t.id];
  if (saved && saved.id) return ytTrack(saved);
  // (an old "none" without a date, or one more than a day old, is looked for again; so is a strict one when any will do)
  if (saved && saved.none && (saved.any || !any) && Date.now() - (saved.at || 0) < YT_NO_MATCH_TTL) { ytLastMiss = { id: t.id, closest: saved.closest || "", near: saved.near || null }; return null; }
  let ranked;
  try { ranked = await ytCandidates(t); } catch (e) { return null; }   // couldn't search: try again next time
  // nothing close: once more, by the song's own title
  if (any && !(ranked[0] && ranked[0].score >= 0.5)) {
    try { ranked = ranked.concat(await ytCandidates(t, artistName(t) + " " + matchTitle(t))).sort((a, b) => b.score - a.score); } catch (e) {}
  }
  const top = ranked[0];
  const pick = any ? ranked.find((x) => x.score >= 0.5 && artistHit(t, x.c)) : top && top.score >= 0.8 ? top : null;
  const ok = pick ? pick.c : null;
  const closest = top ? '"' + top.c.title + '" by ' + artistName(top.c) : "";
  // (the closest one is kept too: a Go+ track with no match plays it rather than nothing)
  const near = top ? ytSnap(top.c) : null;
  ytMatches[t.id] = ok ? ytSnap(ok) : { none: true, at: Date.now(), closest, near, any: !!any };
  ytMatchesSave();
  if (!ok) ytLastMiss = { id: t.id, closest, near };
  return ok;
}

// Track preferences > Plays from YouTube Music: your choice wins over the automatic match
function ytChooseMatch(t, c) {
  ytMatchesLoad();
  ytMatches[t.id] = Object.assign(ytSnap(c), { chosen: true });
  ytMatchesSave();
  delete t.ytVia;
}

// The closest YouTube Music version found for a track that had no match (or null)
function ytClosest(t) {
  const miss = ytLastMiss && ytLastMiss.id === t.id ? ytLastMiss : (ytMatchesLoad()[t.id] || {});
  return miss.near && miss.near.id ? ytTrack(miss.near) : null;
}

// Plays a track by its YouTube Music match; false if there's none (or YouTube Music is off)
async function playViaYouTube(t, mine, note, any) {
  if (!ytOn()) return false;
  const m = await ytMatch(t, any).catch(() => null);
  if (!m || mine !== streamToken) return false;
  return playYouTubeVersion(t, m, mine, note);
}

// Plays a track as a YouTube Music version of it (its match, or the closest one)
async function playYouTubeVersion(t, m, mine, note) {
  t.ytVia = m.ytId;   // loudness and silences are measured on what's actually playing
  const url = await ytPlayAnyUpload(m);
  if (m.ytVia && m !== t) t.ytVia = m.ytVia;
  if (mine !== streamToken) return true;
  await tryStream(url, false, null);
  if (mine !== streamToken) return true;
  say(note);
  loudAnalyse(t).then(() => { if (curTrack() === t) loudApply(t, false); });
  return true;
}

// ---------- likes, kept on this PC ----------
const YT_LIKES_KEY = "sc.ytLikes";
let ytLikes = [];   // newest first

function ytLikesLoad() {
  try { ytLikes = JSON.parse(localStorage.getItem(YT_LIKES_KEY) || "[]"); } catch (e) { ytLikes = []; }
  ytLikes.forEach((t) => likeState.set(t.id, true));
}

function ytSetLiked(t, on) {
  ytLikes = ytLikes.filter((x) => x.id !== t.id);
  if (on) ytLikes.unshift(xplSnap(t));
  try { localStorage.setItem(YT_LIKES_KEY, JSON.stringify(ytLikes)); } catch (e) {}
  // with your YouTube account connected, the like is made there too (the one here stays either way)
  if (ytAcc.connected) ytRate(t, on, ytIdOf(t));
}

// ---------- your YouTube account (optional) ----------
// Connected (Settings > Integrations), liking a YouTube track likes it on YouTube and YouTube Music;
// with "Like Other Tracks There Too", liking any other track likes the same song there as well. The
// video a track was matched to is remembered, so unliking takes back the same like.
const YT_MIRROR_KEY = "sc.ytMirror";
const ytAcc = { connected: false, name: "", configured: false, busy: false };
let ytMirrorIds = {};
try { ytMirrorIds = JSON.parse(localStorage.getItem(YT_MIRROR_KEY) || "{}"); } catch (e) { ytMirrorIds = {}; }

async function ytRate(t, on, videoId) {
  const n = guessNames(t);
  try {
    const id = await invoke(on ? "like_youtube_track" : "unlike_youtube_track", { videoId: videoId || ytMirrorIds[t.id] || null, title: n.title || t.title || "", artist: n.artist || artistName(t) || "" });
    if (srcOf(t) !== "youtube") {
      if (on) ytMirrorIds[t.id] = id; else delete ytMirrorIds[t.id];
      try { localStorage.setItem(YT_MIRROR_KEY, JSON.stringify(ytMirrorIds)); } catch (e) {}
    }
  } catch (e) {
    say((on ? "Liked here, but not on YouTube: " : "Unliked here, but not on YouTube: ") + e);
    if (/connect your youtube account/i.test(String(e))) ytAccRefresh();
  }
}

// Liking a track from another platform: the same song on YouTube too, if you've asked for that
function ytMirrorLike(t, on) {
  if (ytAcc.connected && prefs.ytMirror && srcOf(t) !== "youtube" && srcOf(t) !== "local") ytRate(t, on, null);
}

async function ytAccRefresh() {
  try { Object.assign(ytAcc, await invoke("youtube_status")); } catch (e) { ytAcc.connected = false; }
  ytAccPaint();
}

function ytAccPaint() {
  const btn = $("ytAccBtn");
  btn.textContent = ytAcc.busy ? "Signing in..." : ytAcc.connected ? "Sign out" : "Sign in";
  btn.disabled = ytAcc.busy || (!ytAcc.connected && ytAcc.configured === false);
  btn.title = ytAcc.configured === false ? "Needs youtube_client_id and youtube_client_secret in config.json" : "Optional: your YouTube account, for likes there";
  $("ytMirrorRow").hidden = !ytAcc.connected;
  if (typeof ytPaint === "function") ytPaint();
  if ($("clearedYt")) clearedPaint();   // (js/streamsafe.js)
  $("ytMirror").checked = !!prefs.ytMirror;
  if (!ytAcc.busy && ytAcc.connected && ytAcc.name) { nameNote($("ytAccNote"), "Signed in as ", ytAcc.name, ". Your likes are on YouTube too, so they're on your phone and other devices."); return; }
  $("ytAccNote").textContent = ytAcc.busy ? "Finish signing in in your browser, then come back here..."
    : ytAcc.connected ? "Signed in. Your likes are on YouTube too, so they're on your phone and other devices."
    : ytAcc.configured === false ? "Optional: your likes, on YouTube too. Set up gets you free Google keys first (a few minutes, once)."
    : "Optional: sign in and your likes are on YouTube too, on your phone and other devices. Without it, YouTube Music still plays; likes stay on this PC.";
}

function ytAccInit() {
  $("ytAccBtn").addEventListener("click", async () => {
    if (!ytAcc.connected) {
      ytAcc.busy = true;
      ytAccPaint();
      try {
        Object.assign(ytAcc, await invoke("youtube_connect"));
        say("Signed in to YouTube" + (ytAcc.name ? " as " + ytAcc.name : "") + ".");
      } catch (e) {
        say("Couldn't sign in to YouTube: " + e);
      }
      ytAcc.busy = false;
      await ytAccRefresh();
    } else {
      if (!confirm("Sign out of YouTube? Likes stay on YouTube; new ones won't be sent there.")) return;
      try { await invoke("youtube_disconnect"); } catch (e) { say(String(e)); }
      await ytAccRefresh();
      say("Signed out of YouTube.");
    }
  });
  $("ytMirror").addEventListener("change", () => { prefs.ytMirror = $("ytMirror").checked; savePrefs(); });
  ytAccRefresh();
}

const ytLikedTracks = () => ytLikes.map((x) => Object.assign({}, x, { user: Object.assign({}, x.user) }));

// ---------- playing ----------
// Finds the audio first (so a missing yt-dlp gets a clear message), then plays it through the local server
async function ytPlay(t) {
  const vid = ytIdOf(t);
  if (!vid) throw new Error("this YouTube track has no video id");
  await invoke("yt_prepare", { id: vid });
  if (!localInfo) localInfo = await invoke("local_server");
  return localUrl("yt", vid);
}

// An age-restricted video: YouTube only plays it to a signed-in adult, which yt-dlp can't be. Another
// upload of the same song often isn't restricted (the YouTube Music "song" next to a video, a lyric
// video...), so the closest few are tried; with none, it's skipped (ytAgeSkip) and said why.
const ytAgeBlocked = (e) => /confirm your age|age[- ]restrict|inappropriate for some users/i.test(String(e && e.message ? e.message : e));
async function ytPlayAnyUpload(t) {
  try { return await ytPlay(t); } catch (e) {
    if (!ytAgeBlocked(e)) throw e;
    const ranked = await ytCandidates(t).catch(() => []);
    for (const r of ranked.filter((x) => x.score >= 0.8 && ytIdOf(x.c) !== ytIdOf(t)).slice(0, 3)) {
      try { const url = await ytPlay(r.c); t.ytVia = ytIdOf(r.c); return url; } catch (e2) { if (!ytAgeBlocked(e2)) break; }
    }
    throw e;
  }
}
function ytAgeSkip(t) {
  const note = 'Skipped "' + (t.title || "a song") + "\": YouTube only plays it to signed-in adults, and there's no other upload of it.";
  if (together.role === "listener") { playStandIn(t, Math.max(0, (t.duration || 0) - hostPos()) || hostLenMs(t)); return; }
  if (qi < queue.length - 1) step(1); else if (typeof autoplayActive === "function" && autoplayActive()) autoplayAdvance();
  setTimeout(() => say(note, { force: true }), 1200);   // (after the next one has started)
}

// The next track's audio is looked up while this one plays, so it starts straight away
function ytPrepareNext() {
  const next = queue[qi + 1];
  if (next && srcOf(next) === "youtube" && ytAvailable()) invoke("yt_prepare", { id: ytIdOf(next) }).catch(() => {});
}

// ---------- autoplay and Discover ----------
async function apYouTube(seed) {
  const song = await ytFind(seed);
  return song ? shuffleInPlace(await ytRadio(song.ytId)) : [];
}

// A genre on YouTube Music: its most played songs
async function ytDiscover(src) {
  const g = String(src).replace(/^genre:/, "");
  return { list: await ytSearch(g + " music") };
}

// ---------- settings: setting up, switching off, removing ----------
// With YouTube Music switched on (the default), yt-dlp and Deno are set up by themselves the first
// time; switched off, YouTube isn't used anywhere. Remove takes them (and their caches) off this PC.
let ytSetupTimer = null;
let ytAutoTried = false;   // set up by itself once per run; after a failure it waits for Try again

const ytWanted = () => prefs.ytOn !== false;
// A song's link is plain YouTube (it opens for anyone); you open it in YouTube Music
const ytMusicLink = (u) => String(u || "").replace(/^https:\/\/www\.youtube\.com\/(watch|channel)/,"https://music.youtube.com/$1");
const mbOf = (n) => (n / 1048576).toFixed(0);

function ytPaint() {
  if (typeof mbPaint === "function") mbPaint();   // (MusicBrainz plays through YouTube Music)
  const note = $("ytNote");
  const setupBtn = $("ytSetup");
  const removeBtn = $("ytRemove");
  $("ytOn").checked = ytWanted();
  $("ytOptions").hidden = !ytWanted();
  $("ytAccBtn").hidden = !ytWanted() || ytAcc.configured === false;
  const setup = ytInfo.setup || {};
  removeBtn.hidden = !ytInfo.checked || setup.busy || !ytInfo.diskUse;
  removeBtn.title = ytInfo.diskUse ? "Remove yt-dlp, Deno and their caches (" + mbOf(ytInfo.diskUse) + " MB)" : "";
  setupBtn.hidden = true;
  if (!ytInfo.checked) { note.textContent = "Checking..."; return; }
  if (setup.busy) {
    note.textContent = "Setting up: " + setup.step.toLowerCase() + (setup.total ? ", " + mbOf(setup.done) + " of " + mbOf(setup.total) + " MB" : "") + "...";
    return;
  }
  if (!ytWanted()) {
    note.textContent = "Off" + (ytInfo.diskUse ? ". yt-dlp and Deno are still on this PC (" + mbOf(ytInfo.diskUse) + " MB): Remove takes them off." : ". Switch on to set it up (about 58 MB).");
    return;
  }
  if (ytAvailable()) {
    note.textContent = ytInfo.builtIn
      ? "Ready. " + (ytAcc.connected ? "Signed in: likes go to YouTube too." : "Likes are kept on this PC; sign in to like on YouTube too.")
      : "Ready, with your own yt-dlp " + ytInfo.ytdlp + ". " + (ytAcc.connected ? "Signed in: likes go to YouTube too." : "Likes are kept on this PC; sign in to like on YouTube too.");
    return;
  }
  if (setup.error) {
    note.textContent = "Couldn't set up: " + setup.error;
    setupBtn.textContent = "Try again";
    setupBtn.hidden = false;
  } else {
    note.textContent = "Needs yt-dlp and Deno to play (about 58 MB from GitHub).";
    setupBtn.textContent = "Set up";
    setupBtn.hidden = false;
  }
}

async function ytStartSetup() {
  await invoke("yt_setup").catch((e) => say(String(e)));
  say("Installing prerequisites for YouTube Music...", { stay: true, force: true });
  ytCheck();
}

async function ytCheck() {
  try { ytInfo = Object.assign(await invoke("yt_status"), { checked: true }); } catch (e) { ytInfo = { ytdlp: null, deno: false, node: false, checked: true }; }
  const setup = ytInfo.setup || {};
  ytPaint();
  syncSpotifySettings();   // a free Spotify account plays through YouTube Music
  renderPlatChips();
  clearTimeout(ytSetupTimer);
  if (setup.busy) {
    // (the status bar follows it: the first launch has this to do before YouTube Music plays)
    say("Installing prerequisites for YouTube Music" + (setup.total ? ": " + Math.round((100 * (setup.done || 0)) / setup.total) + "%" : "") + "...", { stay: true, force: true });
    ytSetupTimer = setTimeout(ytCheck, 700);
    return;
  }
  if (setup.step === "Done" && ytAvailable()) {
    ytInfo.setup = {};
    say("YouTube Music is ready.", { force: true });
    return;
  }
  if (setup.error) { say("YouTube Music couldn't be set up: " + setup.error, { force: true }); return; }
  // the first time (or after Remove and switching on again): set up without being asked
  if (ytWanted() && !ytAvailable() && !ytAutoTried) {
    ytAutoTried = true;
    ytStartSetup();
  }
}

async function ytRemove() {
  const mb = mbOf(ytInfo.diskUse || 0);
  if (!confirm("Remove YouTube Music's yt-dlp and Deno from this PC (" + mb + " MB, caches included)? YouTube Music will be switched off. Your YouTube likes are kept, in case you set it up again.")) return;
  try {
    const freed = await invoke("yt_remove");
    prefs.ytOn = false;
    savePrefs();
    ytAutoTried = true;   // switched off now: nothing to set up
    await setPlatformOn("youtube", false);
    say("Removed yt-dlp and Deno (" + mbOf(freed) + " MB freed).");
  } catch (e) {
    say("Couldn't remove them: " + e);
  }
  ytCheck();
}

function ytInit() {
  ytLikesLoad();
  ytPaint();
  $("ytOn").addEventListener("change", async () => {
    await setPlatformOn("youtube", $("ytOn").checked);
    ytAutoTried = false;   // switched on: set up if it isn't yet
    ytCheck();
  });
  $("ytSetup").addEventListener("click", ytStartSetup);
  $("ytRemove").addEventListener("click", ytRemove);
  ytCheck();
}
