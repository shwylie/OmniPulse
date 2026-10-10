// Collaborative playlists: a playlist on this PC that friends can add to and take from too. Its owner
// invites them (the playlist's menu > Invite collaborators): the playlist is shared on the relay
// (js/share.js) with an edit key, and the invite link carries it. Opening the invite joins it: it's
// one of their playlists from then on, for good, with nothing to reconnect. Each change anyone makes
// goes to the relay at once, and every app takes the others' in when the playlist is opened and once
// a minute. It stays joined until someone leaves (or its owner stops it); your own music files stay
// yours (no one else could play them), so they're never sent.
//
// p.collab = { code, relay, k (the owner's key, or the edit key), owner, rev }

const COLLAB_PULL_MS = 60 * 1000;
const isCollab = (p) => !!(p && p.collab && p.collab.code);

async function collabPost(c, path, body) {
  const r = await fetch(c.relay + "/api/shares/" + c.code + path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body || {}) });
  const v = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(v.error || "the relay answered " + r.status);
  return v;
}

// The owner: shared (if it isn't yet), made collaborative, and the invite copied
async function collabInvite(p) {
  const x = xplFind(p.id);
  if (!x) return;
  try {
    let info = shareInfoFor(p);
    if (!info || info.relay !== relayUrl()) { await sharePlaylist(p, true); info = shareInfoFor(p); }
    if (!info) throw new Error("it couldn't be shared");
    const r = await fetch(info.relay + "/api/shares/" + info.code + "/collab?key=" + encodeURIComponent(info.key), { method: "POST" });
    const v = await r.json();
    if (!r.ok || !v.editKey) throw new Error(v.error || "the relay answered " + r.status);
    if (!isCollab(x)) { x.collab = { code: info.code, relay: info.relay, k: info.key, owner: true, rev: 0 }; xplSave(true); }
    const link = info.relay + "/p/" + info.code + "?e=" + v.editKey;
    await navigator.clipboard.writeText(link);
    say('Invite link copied: friends who open it can add songs to "' + x.title + '" and take them out, from their own OmniPulse.', { force: true });
    refreshSidebar();
  } catch (e) {
    say("Couldn't make it collaborative: " + (e && e.message ? e.message : e), { force: true });
  }
}

// Opening an invite: joined, as one of your playlists (or the one you joined already, opened)
async function collabJoin(code, relay, e) {
  const had = xpls.find((x) => isCollab(x) && x.collab.code === code);
  if (had) { await refreshSidebar(); openPlaylist(playlists.find((p) => p.id === had.id)); collabPull(had); return; }
  let share;
  try { const r = await fetch(relay + "/api/shares/" + code); share = r.ok ? await r.json() : null; } catch (x) { share = null; }
  if (!share || !Array.isArray(share.tracks)) { say("That playlist isn't shared any more."); return; }
  if (!confirm('Join "' + share.title + '"' + (share.by ? " from " + share.by : "") + "? It's a collaborative playlist: you can add songs to it and take them out, and see everyone else's changes.")) return;
  const p = xplCreate(share.title, share.tracks.map((t) => t.snap).filter((s) => s && s.id && s.source !== "local"));
  p.collab = { code, relay, k: e, owner: false, rev: share.rev || 0 };
  xplSave(true);
  await refreshSidebar();
  openPlaylist(playlists.find((x) => x.id === p.id));
  say('Joined "' + p.title + '". Your changes go to everyone in it, and theirs come to you.', { force: true });
}

// A change here, to everyone (your own files stay here)
function collabSend(x, op) {
  if (!isCollab(x)) return;
  collabPost(x.collab, "/edit?k=" + encodeURIComponent(x.collab.k), op)
    // (only this change since: up to date; others came in between: theirs fetched)
    .then((v) => { if (!v.rev) return; if (v.rev === x.collab.rev + 1) { x.collab.rev = v.rev; xplSave(true); } else collabPull(x); })
    .catch((e) => say('Couldn\'t send that change to "' + x.title + '": ' + (e && e.message ? e.message : e)));
}
function collabAdded(x, tracks) {
  tracks.filter((t) => srcOf(t) !== "local").forEach((t) => collabSend(x, { op: "add", track: { title: matchTitle(t), artist: artistName(t), ms: t.full_duration || t.duration || 0, links: shareLinks(t), snap: xplSnap(t) } }));
}
function collabRemoved(x, id) {
  collabSend(x, { op: "remove", id: String(id) });
}

// Everyone else's changes: the playlist as it is on the relay (your own files kept where they were)
async function collabPull(x) {
  if (!isCollab(x)) return;
  let share;
  try { const r = await fetch(x.collab.relay + "/api/shares/" + x.collab.code); share = r.ok ? await r.json() : null; } catch (e) { return; }   // (offline: next time)
  if (!share || !Array.isArray(share.tracks)) return;
  // its owner stopped it: kept as yours, no longer joined
  if (share.collab === false && !x.collab.owner) {
    delete x.collab;
    xplSave(true);
    say('"' + x.title + '" isn\'t collaborative any more: it\'s kept as yours.');
    refreshSidebar();
    return;
  }
  if ((share.rev || 0) === x.collab.rev) return;
  const mine = x.tracks.filter((s) => srcOf(s) === "local");
  x.tracks = share.tracks.map((t) => t.snap).filter((s) => s && s.id && s.source !== "local").concat(mine);
  if (share.title) x.title = share.title;
  x.collab.rev = share.rev || 0;
  xplSave();
  // on its page right now: shown as it is
  if (view === "playlist" && currentPlaylist && currentPlaylist.id === x.id) { setLocalPool(xplTracks(x)); render(); }
  refreshSidebar();
}

// Leaving (or, its owner, stopping it): the playlist stays here as yours
async function collabLeave(p) {
  const x = xplFind(p.id);
  if (!isCollab(x)) return;
  const owner = x.collab.owner;
  if (!confirm(owner ? 'Stop "' + x.title + '" being collaborative? Everyone keeps a copy, but changes no longer go between you.' : 'Leave "' + x.title + '"? You keep it as yours, but your changes and theirs no longer go between you.')) return;
  if (owner) {
    try { await fetch(x.collab.relay + "/api/shares/" + x.collab.code + "/collab?stop=1&key=" + encodeURIComponent(x.collab.k), { method: "POST" }); } catch (e) {}
  }
  delete x.collab;
  xplSave(true);
  refreshSidebar();
  say(owner ? '"' + x.title + '" isn\'t collaborative any more.' : 'You left "' + x.title + '". It\'s kept as yours.', { force: true });
}

function collabInit() {
  // everyone's changes, every minute (and as each collaborative playlist is opened: js/app.js)
  setInterval(() => xpls.filter(isCollab).forEach((x) => collabPull(x)), COLLAB_PULL_MS);
  setTimeout(() => xpls.filter(isCollab).forEach((x) => collabPull(x)), 5000);
}
