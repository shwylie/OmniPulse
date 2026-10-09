// Picking several tracks at once on a page: Ctrl+click picks one more (or lets it go), Shift+click picks
// everything from the last one picked, Ctrl+A picks every track on the page, Escape lets them all go.
// Dragging a picked track drags all of them; Ctrl+C copies them and Ctrl+V pastes them into the
// playlist that's open (or the queue, elsewhere). Tracks already there aren't added twice, and ones
// that look like the same song under a slightly different name are asked about, all in one go.

const sel = { keys: new Set(), anchor: -1 };
let selClip = [];   // what Ctrl+C copied (the tracks themselves)

const selKey = (t) => String(t && t.id);
const selAll = () => (localPool && localPool.length > viewTracks.length && nextHref === "local:" ? localPool : viewTracks);

// The picked tracks, in the page's order
function selTracks() {
  if (!sel.keys.size) return [];
  return selAll().filter((t) => sel.keys.has(selKey(t)));
}

function selPaint() {
  const box = $("list");
  if (!box) return;
  box.querySelectorAll("li[data-i]").forEach((li) => {
    const t = viewTracks[Number(li.dataset.i)];
    li.classList.toggle("picked", !!t && sel.keys.has(selKey(t)));
  });
  document.body.classList.toggle("picking", sel.keys.size > 0);
}

function selClear() {
  if (!sel.keys.size) return;
  sel.keys.clear();
  sel.anchor = -1;
  selPaint();
}

function selToggle(i) {
  const t = viewTracks[i];
  if (!t) return;
  const k = selKey(t);
  if (sel.keys.has(k)) sel.keys.delete(k); else sel.keys.add(k);
  sel.anchor = i;
  selPaint();
}

function selRange(i) {
  const from = sel.anchor >= 0 ? sel.anchor : i;
  const [a, b] = from < i ? [from, i] : [i, from];
  for (let k = a; k <= b; k++) if (viewTracks[k]) sel.keys.add(selKey(viewTracks[k]));
  selPaint();
}

// ---------- the same song under a slightly different name ----------
// "Song (Official Audio)" and "Song" by the same artist, about the same length: probably one song.
// Versions ("Remix", "Radio Edit", "Live") stay apart: those words are kept.
const SEL_NOISE = /\b(official( music)?( video| audio| lyric video)?|lyrics?( video)?|audio|video|visuali[sz]er|hq|hd|4k|explicit|clean|remaster(ed)?( \d{4})?|\d{4} remaster(ed)?|free download|out now)\b/gi;
function selSongKey(t) {
  const n = guessNames(t);
  const title = String(t.title || n.title || "")
    .replace(/\s*[(\[]([^)\]]*)[)\]]/g, (m, inner) => (SEL_NOISE.test(inner) && !/\b(remix|mix|edit|live|acoustic|vip|bootleg|flip|cover|version|instrumental)\b/i.test(inner) ? " " : " " + inner + " "))
    .replace(/\b(feat\.?|ft\.?|featuring)\s.*$/i, "")
    .replace(SEL_NOISE, " ");
  const artist = String(n.artist || artistName(t) || "").split(/\s*(?:,|&| x | feat\.? | ft\.? )\s*/i)[0];
  const norm = (s) => s.toLowerCase().normalize("NFKD").replace(/[̀-ͯ]/g, "").replace(/[^a-z0-9]+/g, " ").trim();
  // a title that already starts with the artist ("Artist - Song") counts the same as without
  const a = norm(artist);
  let ti = norm(title);
  if (a && ti.startsWith(a + " ")) ti = ti.slice(a.length + 1);
  return a + "|" + ti;
}

function selLikelySame(a, b) {
  if (selKey(a) === selKey(b)) return true;
  if (selSongKey(a) !== selSongKey(b)) return false;
  const da = Number(a.duration || 0), db = Number(b.duration || 0);
  return !da || !db || Math.abs(da - db) <= 15000;
}

// ---------- pasting (or dropping several) into a playlist ----------
// tracks: what to add; have: what's there already. Same track: left out. Probably the same song: asked.
// Resolves with { add: [...], replace: [[old, new]...] } or null if cancelled.
async function selPlan(tracks, have) {
  const add = [], conflicts = [];
  const seen = new Set(have.map(selKey));
  for (const t of tracks) {
    if (seen.has(selKey(t))) continue;   // exactly this track is there already
    const twin = have.find((h) => selLikelySame(h, t)) || add.find((h) => selLikelySame(h, t));
    seen.add(selKey(t));
    if (twin) conflicts.push({ t, twin, choice: "both" });
    else add.push(t);
  }
  if (!conflicts.length) return { add, replace: [] };
  const choices = await selAsk(conflicts);
  if (!choices) return null;
  const replace = [];
  choices.forEach((c) => {
    if (c.choice === "both") add.push(c.t);
    else if (c.choice === "new") {
      const i = add.indexOf(c.twin);
      if (i >= 0) add[i] = c.t; else replace.push([c.twin, c.t]);
    }
  });
  return { add, replace };
}

const selLine = (t) => (t.title || "Untitled") + " — " + (artistName(t) || "Unknown") + " · " + platInfo(srcOf(t)).name + (t.duration ? " · " + fmt(t.duration) : "");

// One dialog for every look-alike: each gets Keep Both / Keep the One There / Use the New One, and the
// buttons at the top set them all at once
function selAsk(conflicts) {
  return new Promise((done) => {
    const d = el("dialog", "fixDialog selDialog");
    const sheet = el("div", "sheet");
    d.append(sheet);
    sheet.append(el("h2", "", conflicts.length === 1 ? "This might already be here" : conflicts.length + " of these might already be here"));
    sheet.append(el("p", "note", "Each looks like a song that's there already, under a slightly different name."));
    const all = el("div", "selAll");
    const rows = conflicts.map((c) => {
      const r = el("div", "selRow");
      const names = el("div", "selNames");
      names.append(el("div", "selNew", "New: " + selLine(c.t)), el("div", "selOld", "There: " + selLine(c.twin)));
      const pick = el("select", "narrow");
      [["both", "Keep both"], ["old", "Keep the one there"], ["new", "Use the new one"]].forEach(([v, n]) => { const o = el("option", "", n); o.value = v; pick.append(o); });
      pick.addEventListener("change", () => { c.choice = pick.value; });
      r.append(names, pick);
      c.pick = pick;
      return r;
    });
    if (conflicts.length > 1) {
      all.append(el("span", "note", "For all:"));
      [["both", "Keep Both"], ["old", "Keep the Ones There"], ["new", "Use the New Ones"]].forEach(([v, n]) => {
        const b = el("button", "soft", n);
        b.type = "button";
        b.addEventListener("click", () => conflicts.forEach((c) => { c.choice = v; c.pick.value = v; }));
        all.append(b);
      });
      sheet.append(all);
    }
    const list = el("div", "selRows");
    list.append(...rows);
    sheet.append(list);
    const foot = el("div", "fixActions");
    const cancel = el("button", "soft", "Cancel");
    const ok = el("button", "soft fixSave", "Add");
    cancel.type = ok.type = "button";
    const finish = (v) => { d.close(); d.remove(); done(v); };
    cancel.addEventListener("click", () => finish(null));
    ok.addEventListener("click", () => finish(conflicts));
    d.addEventListener("cancel", (e) => { e.preventDefault(); finish(null); });
    foot.append(el("span", "grow"), cancel, ok);
    sheet.append(foot);
    document.body.append(d);
    d.showModal();
    ok.focus();
  });
}

// Adds several to a playlist, as asked about above; tells you what happened
async function selPasteInto(p, tracks) {
  if (!p || !tracks.length) return;
  if (!isXpl(p)) {
    // one of your SoundCloud playlists, with only SoundCloud songs: as it is. Anything else becomes a
    // playlist here (js/xplmirror.js), which takes them all.
    const asIs = p.source !== "spotify" && isMyPlatformList(p) && tracks.every((t) => srcOf(t) === "soundcloud");
    if (!asIs) {
      try { await xplAdopt(p, tracks); } catch (e) { say("Could not add to the playlist: " + e); }
      return;
    }
  }
  let have = [];
  try {
    if (isXpl(p)) have = xplTracks(xplFind(p.id) || { tracks: [] });
    else if (view === "playlist" && currentPlaylist && String(currentPlaylist.id) === String(p.id)) have = selAll().slice();
    else have = (await playlistTrackIds(p.id)).map((id) => ({ id }));
  } catch (e) { have = []; }
  const plan = await selPlan(tracks, have);
  if (!plan) { say("Nothing was added."); return; }
  const changed = plan.add.length + plan.replace.length;
  try {
    if (isXpl(p)) {
      const pl = xplFind(p.id);
      plan.replace.forEach(([old, nu]) => {
        const i = pl.tracks.findIndex((s) => String(s.id) === selKey(old));
        if (i >= 0) pl.tracks[i] = xplSnap(nu);
      });
      if (plan.replace.length) xplSave();
      xplAdd(p.id, plan.add);
    } else {
      let ids = await playlistTrackIds(p.id);
      plan.replace.forEach(([old, nu]) => { const i = ids.indexOf(old.id); if (i >= 0) ids[i] = nu.id; });
      ids = ids.concat(plan.add.map((t) => t.id).filter((id) => !ids.includes(id)));
      await savePlaylistTracks(p.id, ids);
    }
  } catch (e) {
    say("Could not add to the playlist: " + e);
    return;
  }
  const left = tracks.length - changed;
  say((changed ? (plan.add.length ? "Added " + plural(plan.add.length, "track", "tracks") : "") +
      (plan.replace.length ? (plan.add.length ? ", " : "") + "swapped in " + plural(plan.replace.length, "track", "tracks") : "") + " to " + (p.title || "the playlist")
    : "Nothing new for " + (p.title || "the playlist")) +
    (left > 0 ? " (" + plural(left, "was", "were") + " there already)" : "") + ".");
  if (view === "playlist" && currentPlaylist && String(currentPlaylist.id) === String(p.id)) openPlaylist(currentPlaylist);
  refreshSidebar();
}

function selPasteQueue(tracks) {
  const fresh = tracks.filter((t) => !queue.slice(Math.max(qi, 0)).some((q) => q.id === t.id));
  fresh.forEach((t) => addToQueue(t));
  say(fresh.length ? "Added " + plural(fresh.length, "track", "tracks") + " to the queue." : "They're all in the queue already.");
}

// ---------- keys, clicks ----------
const selTyping = (e) => { const t = e.target; return !!t && typeof t.closest === "function" && (!!t.closest("input, textarea, select, [contenteditable=true]") || t.isContentEditable); };
const selOnList = () => !$("listView").hidden && view !== "stats" && !document.querySelector("dialog[open]");

function selectInit() {
  // Ctrl / Shift + click picks, before the list's own click (which would play)
  $("list").addEventListener("click", (e) => {
    const li = e.target.closest("li[data-i]");
    if (!li || e.target.closest("button")) return;
    if (e.ctrlKey || e.metaKey) { e.preventDefault(); e.stopImmediatePropagation(); selToggle(Number(li.dataset.i)); return; }
    if (e.shiftKey) { e.preventDefault(); e.stopImmediatePropagation(); selRange(Number(li.dataset.i)); return; }
    selClear();   // a plain click plays it, as always, and lets the others go
  }, true);
  // the list is drawn again as you scroll or load more: the picked ones stay picked
  new MutationObserver(() => { if (sel.keys.size) selPaint(); }).observe($("list"), { childList: true });

  document.addEventListener("keydown", (e) => {
    if (selTyping(e)) return;
    const mod = e.ctrlKey || e.metaKey;
    if (e.key === "Escape" && sel.keys.size) { selClear(); return; }
    if (!mod || e.altKey) return;
    const k = e.key.toLowerCase();
    if (k === "a" && selOnList() && viewTracks.length) {
      e.preventDefault();
      selAll().forEach((t) => sel.keys.add(selKey(t)));
      selPaint();
      say(plural(sel.keys.size, "track", "tracks") + " picked. Ctrl+C copies them; drag one to move them all.");
    } else if (k === "c" && sel.keys.size && selOnList()) {
      e.preventDefault();
      selClip = selTracks().map((t) => Object.assign({}, t));
      const text = selClip.map((t) => (artistName(t) ? artistName(t) + " - " : "") + (t.title || "") + (t.permalink_url ? "  " + t.permalink_url : "")).join("\n");
      navigator.clipboard.writeText(text).catch(() => {});
      say("Copied " + plural(selClip.length, "track", "tracks") + ". Open a playlist and press Ctrl+V to add them.");
    } else if (k === "v" && selClip.length && !document.querySelector("dialog[open]")) {
      e.preventDefault();
      if (view === "playlist" && currentPlaylist && (isXpl(currentPlaylist) || isOwnPlaylist(currentPlaylist))) selPasteInto(currentPlaylist, selClip);
      else if (view === "playlist") say("This playlist isn't yours to change. Paste into one of yours, or one on this PC.");
      else selPasteQueue(selClip);
    }
  });
  // leaving the page lets them go
  ["navDiscover", "navLikes", "navRediscover", "navSets", "navStats"].forEach((id) => { const b = $(id); if (b) b.addEventListener("click", selClear); });
  $("plList").addEventListener("click", selClear);
}
