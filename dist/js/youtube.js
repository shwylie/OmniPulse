// YouTube Music, without signing in: search and radio from YouTube Music itself (crates/core/src/youtube.rs),
// audio through yt-dlp, played via the local server's /yt (which fetches it in pieces). Likes are kept on
// this PC, like favourites, so they show in your Library and seed autoplay and Rediscover.

let ytInfo = { ytdlp: null, deno: false, node: false, checked: false };
const ytAvailable = () => !!ytInfo.ytdlp && (ytInfo.deno || ytInfo.node);

function ytTrack(x) {
  return tidyTrack({
    source: "youtube",
    id: "yt:" + x.id,
    ytId: x.id,
    title: x.title,
    user: { username: x.artists || "Unknown artist" },
    album: x.album || "",
    duration: x.duration_ms || 0,
    artwork_url: x.thumb || "",
    permalink_url: "https://music.youtube.com/watch?v=" + x.id,
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
}

const ytLikedTracks = () => ytLikes.map((x) => Object.assign({}, x, { user: Object.assign({}, x.user) }));

// ---------- playing ----------
// Finds the audio first (so a missing yt-dlp gets a clear message), then plays it through the local server
async function ytPlay(t) {
  await invoke("yt_prepare", { id: t.ytId });
  if (!localInfo) localInfo = await invoke("local_server");
  return localUrl("yt", t.ytId);
}

// The next track's audio is looked up while this one plays, so it starts straight away
function ytPrepareNext() {
  const next = queue[qi + 1];
  if (next && srcOf(next) === "youtube" && ytAvailable()) invoke("yt_prepare", { id: next.ytId }).catch(() => {});
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
const mbOf = (n) => (n / 1048576).toFixed(0);

function ytPaint() {
  const note = $("ytNote");
  const setupBtn = $("ytSetup");
  const removeBtn = $("ytRemove");
  $("ytOn").checked = ytWanted();
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
      ? "Ready (" + mbOf(ytInfo.diskUse) + " MB on this PC). Nothing to sign in to: likes are kept on this PC."
      : "Ready, with your own yt-dlp " + ytInfo.ytdlp + ". Nothing to sign in to: likes are kept on this PC.";
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
  say("Setting up YouTube Music (about 58 MB)...");
  ytCheck();
}

async function ytCheck() {
  try { ytInfo = Object.assign(await invoke("yt_status"), { checked: true }); } catch (e) { ytInfo = { ytdlp: null, deno: false, node: false, checked: true }; }
  const setup = ytInfo.setup || {};
  ytPaint();
  renderPlatChips();
  clearTimeout(ytSetupTimer);
  if (setup.busy) { ytSetupTimer = setTimeout(ytCheck, 700); return; }   // follow the download
  if (setup.step === "Done" && ytAvailable()) {
    ytInfo.setup = {};
    say("YouTube Music is ready.");
    return;
  }
  if (setup.error) { say("YouTube Music couldn't be set up: " + setup.error); return; }
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
