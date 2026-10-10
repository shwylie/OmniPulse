// Sharing a playlist (its page's Share button). A click shares the playlist itself, songs from every
// platform together: a page on your relay (jam-server/) lists each song with its links on SoundCloud,
// Spotify and YouTube Music, and opens it in OmniPulse for anyone who has it (they can add it to
// theirs). Sharing it again updates that same page. A right-click offers the SoundCloud or Spotify
// copy instead (js/xplmirror.js), as those platforms' own links.

const SHARE_PLATS = [["soundcloud", "SoundCloud"], ["spotify", "Spotify"], ["youtube", "YouTube Music"]];

const shareInfoFor = (p) => (isXpl(p) ? (xplFind(p.id) || {}).share : (prefs.shares || {})[String(p.id)]) || null;
function shareInfoSet(p, info) {
  if (isXpl(p)) { const x = xplFind(p.id); if (x) { x.share = info; xplSave(true); } return; }
  prefs.shares = Object.assign({}, prefs.shares, { [String(p.id)]: info });
  savePrefs();
}

// Where a song is on each platform: its own, and the matches found for it
function shareLinks(t) {
  const links = {};
  if (!t) return links;
  const src = srcOf(t);
  const spLink = (uri) => "https://open.spotify.com/track/" + String(uri).split(":").pop();
  if (src === "soundcloud" && t.permalink_url && t.sharing !== "private") links.soundcloud = t.permalink_url;
  if (src === "spotify" && t.uri) links.spotify = spLink(t.uri);
  if (src === "youtube") links.youtube = "https://www.youtube.com/watch?v=" + ytIdOf(t);
  const m = xmMatchesLoad()[String(t.id)] || {};
  if (!links.spotify && m.spotify && m.spotify.id) links.spotify = spLink(m.spotify.id);
  const yt = ytMatchesLoad()[t.id];
  if (!links.youtube && yt && yt.id) links.youtube = "https://www.youtube.com/watch?v=" + yt.id;
  return links;
}

// (quiet: only shared, for an invite to collaborate: no link of its own copied)
async function sharePlaylist(p, quiet) {
  const relay = relayUrl();
  say("Sharing...");
  let tracks;
  try { tracks = isXpl(p) ? xplTracks(xplFind(p.id) || { tracks: [] }) : await platformPlaylistTracks(p); }
  catch (e) { say("Could not read the playlist: " + e); return; }
  // (your own files can't be played by anyone else)
  tracks = tracks.filter((t) => !t.missing && srcOf(t) !== "local");
  const body = {
    title: p.title || "Playlist",
    by: myName(),
    tracks: tracks.map((t) => ({ title: matchTitle(t), artist: artistName(t), ms: t.full_duration || t.duration || 0, links: shareLinks(t), snap: xplSnap(t) })),
  };
  const was = shareInfoFor(p);
  let url;
  try {
    if (was && was.relay === relay) {
      const r = await fetch(relay + "/api/shares/" + was.code + "?key=" + encodeURIComponent(was.key), { method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
      if (r.ok) url = relay + "/p/" + was.code;
    }
    if (!url) {
      const r = await fetch(relay + "/api/shares", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
      if (!r.ok) throw new Error("the relay answered " + r.status + (r.status === 404 ? " (deploy it again to add shared playlists)" : ""));
      const made = await r.json();
      shareInfoSet(p, { code: made.code, key: made.key, relay });
      url = made.url;
    }
    if (quiet) return;
    await navigator.clipboard.writeText(url);
    say("Link copied: anyone can see its songs on every platform, and open it in OmniPulse." + (was ? " (The same link as before, now up to date.)" : ""), { force: true });
  } catch (e) {
    say("Could not share it: " + (e && e.message ? e.message : e), { force: true });
  }
}

// A track's own link on one platform (not an OmniPulse one)
async function shareTrackLink(t, plat) {
  const url = shareLinks(t)[plat];
  if (!url) return;
  try {
    await navigator.clipboard.writeText(url);
    say((SHARE_PLATS.find(([k]) => k === plat) || ["", "The"])[1] + " link copied.", { force: true });
  } catch (e) {
    say("Could not copy the link.");
  }
}

// The playlist on one platform: the copy kept there (or the playlist itself, if it's one of theirs)
function sharePlatformId(p, plat) {
  if (isXpl(p)) return (xplFind(p.id) || {}).mirrors ? xplFind(p.id).mirrors[plat] || null : null;
  if (plat === "spotify") return p.source === "spotify" ? p.spId : null;
  return plat === "soundcloud" && (!p.source || p.source === "soundcloud") ? String(p.id) : null;
}

async function sharePlatform(p, plat) {
  const id = sharePlatformId(p, plat);
  if (!id) return;
  try {
    let url;
    if (plat === "spotify") {
      // the copy kept for one here is private until it's shared
      if (isXpl(p) && (xplFind(p.id).made || {}).spotify) await spApi("PUT", "/playlists/" + id, { public: true }).catch(() => {});
      url = "https://open.spotify.com/playlist/" + id;
    } else {
      const sc = await api("/playlists/" + id);
      // a private one: its secret link
      url = sc.sharing === "private" && sc.secret_token ? sc.permalink_url + "/" + sc.secret_token : sc.permalink_url;
    }
    await navigator.clipboard.writeText(url);
    say((plat === "spotify" ? "Spotify" : "SoundCloud") + " link copied: only this playlist's songs that are on " + (plat === "spotify" ? "Spotify" : "SoundCloud") + ".", { force: true });
  } catch (e) {
    say("Could not get its link: " + (e && e.message ? e.message : e), { force: true });
  }
}

function shareMenu(p, anchor) {
  const m = $("plMenu");
  m.replaceChildren(el("div", "menuHead", "Share on one platform"));
  let any = false;
  SHARE_PLATS.forEach(([plat, name]) => {
    if (!sharePlatformId(p, plat)) return;
    const b = el("button", "menuItem", "Copy " + name + " link");
    b.dataset.share = plat;
    m.append(b);
    any = true;
  });
  if (!any) m.append(el("div", "menuNote", "It isn't on SoundCloud or Spotify yet: add a song from there first."));
  m.hidden = false;
  placeMenu(anchor);
  m._anchor = anchor;
}

// Someone's shared playlist, opened from its page (omnipulse://playlist/<code>?relay=...)
async function shareOpen(code, relay) {
  let share;
  try {
    const r = await fetch(relay + "/api/shares/" + code);
    share = r.ok ? await r.json() : null;
  } catch (e) { share = null; }
  if (!share || !Array.isArray(share.tracks)) { say("That playlist isn't shared any more."); return; }
  const tracks = share.tracks.map((t) => t.snap).filter((s) => s && s.id && s.source !== "local");
  if (!confirm('Add "' + share.title + '"' + (share.by ? " from " + share.by : "") + " (" + plural(tracks.length, "song", "songs") + ") to your playlists?")) return;
  const p = xplCreate(share.title, tracks);
  await refreshSidebar();
  openPlaylist(playlists.find((x) => x.id === p.id));
  say('Added "' + p.title + '" to your playlists.');
}

function shareInit() {
  $("plShare").addEventListener("click", () => { if (currentPlaylist) sharePlaylist(currentPlaylist); });
  $("plShare").addEventListener("contextmenu", (e) => {
    e.preventDefault();
    if (currentPlaylist) shareMenu(currentPlaylist, $("plShare"));
  });
}
