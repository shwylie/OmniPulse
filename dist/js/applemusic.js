// Apple Music, once (Settings > Integrations > Apple Music > Import): a read-only import. You sign in
// to Apple Music in a window of its own; your loved songs and your playlists are read
// (crates/core/src/apple.rs), and your sign-in is forgotten straight away. Nothing on Apple Music is
// changed. Then, here:
//   - each loved song is liked where it's found: Spotify first (by its ISRC, the recording's own code,
//     else by its title and artist), then SoundCloud, then YouTube Music
//   - each playlist becomes a playlist on this PC, with each song from wherever it's found, so it's a
//     cross-platform one (kept in step on SoundCloud and Spotify, js/xplmirror.js)
//   - what isn't found anywhere is listed at the end, to copy

let appleBusy = false;

// An Apple song as a track to match with (its own platform is none of ours)
const appleAsTrack = (s) => ({ id: "am:" + s.id, source: "apple", title: s.title, user: { username: s.artist }, album: s.album, duration: s.ms, artwork_url: s.art, isrc: s.isrc });

// Where it is: { plat, track } (the track as that platform has it), or null
async function appleFind(s) {
  const t = appleAsTrack(s);
  const plats = activePlats();
  if (plats.includes("spotify") && (spotifyPremium || spotifyFree)) {
    try {
      if (s.isrc) {
        const d = await spApi("GET", "/search?type=track&limit=1&q=" + encodeURIComponent("isrc:" + s.isrc));
        const x = d && d.tracks && d.tracks.items && d.tracks.items[0];
        if (x && x.uri && x.is_playable !== false) return { plat: "spotify", track: spTrack(x) };
      }
      const uri = await xmFind(t, "spotify");
      if (uri) {
        const x = await spApi("GET", "/tracks/" + String(uri).split(":").pop());
        if (x && x.uri) return { plat: "spotify", track: spTrack(x) };
      }
    } catch (e) { /* on to the next platform */ }
  }
  if (plats.includes("soundcloud") && meId !== null) {
    try {
      const id = await xmFind(t, "soundcloud");
      if (id) { const x = await api("/tracks/" + id); if (x && x.id) return { plat: "soundcloud", track: x }; }
    } catch (e) {}
  }
  if (ytOn()) {
    try { const y = await ytMatch(t); if (y) return { plat: "youtube", track: y }; } catch (e) {}
  }
  return null;
}

// A few at a time, saying how far it's got
async function appleEach(list, what, fn, doing) {
  let done = 0, i = 0;
  const work = async () => {
    while (i < list.length) {
      const k = i++;
      await fn(list[k], k);
      done++;
      if (done % 5 === 0 || done === list.length) say((doing || "Importing from Apple Music") + ": " + what + " " + done + " of " + list.length + "...", { stay: true, force: true });
    }
  };
  await Promise.all([work(), work(), work()]);
}

async function appleImport() {
  if (appleBusy) return;
  appleBusy = true;
  const missing = [];
  try {
    say("Sign in to Apple Music in the window that opened (only to read your music: nothing there is changed)...", { stay: true, force: true });
    await invoke("apple_connect");
    say("Reading your Apple Music...", { stay: true, force: true });
    let lib;
    try { lib = await invoke("apple_library"); } finally { invoke("apple_forget").catch(() => {}); }   // (signed out of it straight away)
    const likes = lib.likes || [], lists = lib.playlists || [];
    // loved songs: liked where they're found
    let liked = 0;
    await appleEach(likes, "loved songs", async (s) => {
      const f = await appleFind(s);
      if (!f) { missing.push(s.artist + " - " + s.title); return; }
      if (isLiked(f.track)) return;
      try { await setLikedOn(f.track, true); likeState.set(f.track.id, true); liked++; } catch (e) { missing.push(s.artist + " - " + s.title + " (couldn't like it: " + e + ")"); }
    });
    // playlists: on this PC, each song from wherever it's found
    let made = 0;
    for (const p of lists) {
      const found = new Array(p.tracks.length).fill(null);
      await appleEach(p.tracks, '"' + p.name + '"', async (s, k) => {
        const f = await appleFind(s);
        if (f) found[k] = f.track; else missing.push(s.artist + " - " + s.title + ' (in "' + p.name + '")');
      });
      xplCreate(p.name, found.filter(Boolean));
      made++;
    }
    await refreshSidebar();
    const note = "Imported from Apple Music: " + plural(liked, "loved song", "loved songs") + " liked, " + plural(made, "playlist", "playlists") + " made" +
      (missing.length ? ". " + plural(missing.length, "song wasn't", "songs weren't") + " found anywhere." : ". Everything was found.") + " Your Apple Music sign-in is forgotten.";
    say(note, { stay: true, force: true, actions: missing.length ? [["Copy the missing", () => navigator.clipboard.writeText(missing.join("\n")).then(() => say("The missing songs are copied.", { force: true }))]] : [] });
    prefs.appleImported = Date.now();
    savePrefs();
    applePaint();
  } catch (e) {
    invoke("apple_forget").catch(() => {});
    say("Couldn't import from Apple Music: " + (e && e.message ? e.message : e), { force: true });
  } finally {
    appleBusy = false;
  }
}

function applePaint() {
  const when = prefs.appleImported ? new Date(prefs.appleImported).toLocaleDateString() : "";
  $("appleNote").textContent = "Once: your loved songs become likes on your other platforms, and your playlists become playlists on this PC. Nothing on Apple Music is changed, and your sign-in is forgotten after." +
    (when ? " Last imported " + when + "." : "");
  $("appleImport").textContent = when ? "Import again" : "Import";
}

function appleInit() {
  applePaint();
  $("appleImport").addEventListener("click", appleImport);
}
