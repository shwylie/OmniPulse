// Drag and drop: tracks from the lists, the now-playing area and the queue onto a playlist in the
// sidebar, onto the queue panel (before a row, or at the end) or onto the queue button; and
// reordering what's next in the queue.

const drag = { track: null, from: -1, fromList: false, many: null };   // from: the track's place in the queue; fromList: dragged from the page's own list; many: several picked ones (js/select.js)

function dragClear() {
  drag.track = null;
  drag.from = -1;
  drag.fromList = false;
  drag.many = null;
  document.body.classList.remove("trackDrag");
  document.querySelectorAll(".dragging, .over, .dropOver").forEach((x) => x.classList.remove("dragging", "over", "dropOver"));
}

function dragStart(e, t, from) {
  drag.track = t;
  drag.from = from === undefined ? -1 : from;
  e.dataTransfer.effectAllowed = drag.from >= 0 ? "copyMove" : "copy";
  e.dataTransfer.setData("text/plain", (t.title || "") + (artistName(t) ? " - " + artistName(t) : ""));
  document.body.classList.add("trackDrag");
}

// Playlists on this PC take anything; your SoundCloud playlists only SoundCloud tracks
function playlistTakes(p, t) {
  // several: your SoundCloud playlist takes the SoundCloud ones among them (the rest are left out, and said so)
  // (any playlist but your local albums: one that can't take a song as it is becomes a playlist here)
  if (drag.many && p) return isXpl(p) || p.source !== "local";
  return !!p && !!t && (isXpl(p) || p.source !== "local");
}

function dndInit() {
  // ---------- what can be dragged ----------
  $("list").addEventListener("dragstart", (e) => {
    const li = e.target.closest("li[data-i]");
    const t = li && viewTracks[Number(li.dataset.i)];
    if (!t) return;
    li.classList.add("dragging");
    dragStart(e, t);
    drag.fromList = true;
    // one of several picked: all of them go
    const picked = sel.keys.has(selKey(t)) && sel.keys.size > 1 ? selTracks() : null;
    if (picked) {
      drag.many = picked;
      e.dataTransfer.setData("text/plain", picked.length + " tracks");
      const tag = el("div", "dragTag", picked.length + " tracks");
      document.body.append(tag);
      e.dataTransfer.setDragImage(tag, 10, 10);
      setTimeout(() => tag.remove(), 0);
    }
  });
  $("nowToggle").addEventListener("dragstart", (e) => {
    const t = curTrack();
    if (!t) { e.preventDefault(); return; }
    dragStart(e, t);
  });
  $("qAuto").addEventListener("dragstart", (e) => {
    const li = e.target.closest(".qRow");
    if (!li) return;
    const i = Number(li.dataset.q);
    li.classList.add("dragging");
    dragStart(e, queue[i], i);
  });
  $("qNow").addEventListener("dragstart", (e) => {
    const t = curTrack();
    if (!t) return;
    dragStart(e, t);   // what's playing: copied (to a playlist), not moved
  });
  $("qNext").addEventListener("dragstart", (e) => {
    const li = e.target.closest(".qRow");
    if (!li) return;
    const i = Number(li.dataset.q);
    li.classList.add("dragging");
    dragStart(e, queue[i], i);
  });
  document.addEventListener("dragend", dragClear);

  // ---------- the queue panel: move a queued track, or add one from elsewhere ----------
  const panel = $("queue");
  panel.addEventListener("dragover", (e) => {
    if (!drag.track) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = drag.from >= 0 ? "move" : "copy";
    const li = e.target.closest("#qNext .qRow, #qAuto .qRow");
    document.querySelectorAll(".qRow.over").forEach((x) => { if (x !== li) x.classList.remove("over"); });
    if (li) li.classList.add("over");
    panel.classList.toggle("dropOver", !li && drag.from < 0);
  });
  panel.addEventListener("dragleave", (e) => {
    if (!panel.contains(e.relatedTarget)) {
      panel.classList.remove("dropOver");
      document.querySelectorAll(".qRow.over").forEach((x) => x.classList.remove("over"));
    }
  });
  panel.addEventListener("drop", (e) => {
    if (!drag.track) return;
    e.preventDefault();
    const li = e.target.closest("#qNext .qRow, #qAuto .qRow");
    const to = li ? Number(li.dataset.q) : -1;
    const intoAuto = !!(li && li.closest("#qAuto"));
    if (drag.from > qi) {
      // reorder; dropped among your tracks it's yours, among autoplay's it's autoplay's
      const onEmptyNext = !li && e.target.closest("#qNext");
      if ((to > qi && to !== drag.from) || onEmptyNext) {
        const target = li ? queue[to] : null;
        const [moved] = queue.splice(drag.from, 1);
        if (intoAuto) moved.autoplay = true; else delete moved.autoplay;
        // it joins the part it's dropped in: what you queued, or the rest of the list
        if (!intoAuto && target && !moved.requestedBy) {
          if (isQueuedByYou(target)) moved.queuedByYou = true; else delete moved.queuedByYou;
        }
        const at = onEmptyNext ? autoplayStart() : to;
        queue.splice(at, 0, moved);
        renderQueue();
      }
    } else if (drag.from < 0) {
      if (drag.many) {
        if (to > qi && qi >= 0) { queue.splice(to, 0, ...drag.many.map((t) => queuedByYou(t))); renderQueue(); say("Added " + drag.many.length + " tracks to the queue."); }
        else selPasteQueue(drag.many);
      } else if (to > qi && qi >= 0) {   // just before the row it was dropped on, as something you queued
        queue.splice(to, 0, queuedByYou(drag.track));
        renderQueue();
        say("Added to the queue.");
      } else {
        addToQueue(drag.track);
        say("Added to the queue.");
      }
    }
    dragClear();
  });

  // ---------- the queue button, for when the panel is closed ----------
  const qb = $("queueBtn");
  qb.addEventListener("dragover", (e) => {
    if (!drag.track || drag.from >= 0) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    qb.classList.add("dropOver");
  });
  qb.addEventListener("dragleave", () => qb.classList.remove("dropOver"));
  qb.addEventListener("drop", (e) => {
    if (!drag.track || drag.from >= 0) return;
    e.preventDefault();
    if (drag.many) selPasteQueue(drag.many);
    else { addToQueue(drag.track); say("Added to the queue."); }
    dragClear();
  });

  // ---------- the playlist that's open: drop a track anywhere on its page ----------
  const page = $("listView");
  const pageTakes = () => !!drag.track && !drag.fromList && view === "playlist" && playlistTakes(currentPlaylist, drag.track);
  page.addEventListener("dragover", (e) => {
    if (!pageTakes()) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    page.classList.add("dropOver");
    page.dataset.drop = "Add to " + (currentPlaylist.title || "this playlist");
  });
  page.addEventListener("dragleave", (e) => { if (!page.contains(e.relatedTarget)) page.classList.remove("dropOver"); });
  page.addEventListener("drop", (e) => {
    if (!pageTakes()) return;
    e.preventDefault();
    page.classList.remove("dropOver");
    if (drag.many) selPasteInto(currentPlaylist, drag.many);
    else addToPlaylist(currentPlaylist, drag.track);
    dragClear();
  });

  // ---------- playlists in the sidebar ----------
  const side = $("plList");
  const target = (e) => {
    const b = e.target.closest(".plItem");
    const p = b && playlists.find((x) => String(x.id) === b.dataset.id);
    return p && playlistTakes(p, drag.track) ? { b, p } : null;
  };
  side.addEventListener("dragover", (e) => {
    const hit = drag.track && target(e);
    side.querySelectorAll(".plItem.dropOver").forEach((x) => { if (!hit || x !== hit.b) x.classList.remove("dropOver"); });
    if (!hit) return;   // not a playlist that takes this track: the cursor says no
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    hit.b.classList.add("dropOver");
  });
  side.addEventListener("dragleave", (e) => {
    if (!side.contains(e.relatedTarget)) side.querySelectorAll(".plItem.dropOver").forEach((x) => x.classList.remove("dropOver"));
  });
  side.addEventListener("drop", (e) => {
    const hit = drag.track && target(e);
    if (!hit) return;
    e.preventDefault();
    if (drag.many) selPasteInto(hit.p, drag.many);
    else addToPlaylist(hit.p, drag.track);
    dragClear();
  });
}
