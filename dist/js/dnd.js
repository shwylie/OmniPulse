// Drag and drop: tracks from the lists, the now-playing area and the queue onto a playlist in the
// sidebar, onto the queue panel (before a row, or at the end) or onto the queue button; and
// reordering what's next in the queue.

const drag = { track: null, from: -1 };   // from: the track's place in the queue, when dragged from there

function dragClear() {
  drag.track = null;
  drag.from = -1;
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
  return !!p && !!t && (isXpl(p) || (isOwnPlaylist(p) && srcOf(t) === "soundcloud"));
}

function dndInit() {
  // ---------- what can be dragged ----------
  $("list").addEventListener("dragstart", (e) => {
    const li = e.target.closest("li[data-i]");
    const t = li && viewTracks[Number(li.dataset.i)];
    if (!t) return;
    li.classList.add("dragging");
    dragStart(e, t);
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
        const [moved] = queue.splice(drag.from, 1);
        if (intoAuto) moved.autoplay = true; else delete moved.autoplay;
        const at = onEmptyNext ? autoplayStart() : to;
        queue.splice(at, 0, moved);
        renderQueue();
      }
    } else if (drag.from < 0) {
      if (to > qi && qi >= 0) {   // just before the row it was dropped on
        queue.splice(to, 0, drag.track);
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
    addToQueue(drag.track);
    say("Added to the queue.");
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
    addToPlaylist(hit.p, drag.track);
    dragClear();
  });
}
