// The search bar filters the page you're on as you type: its tracks, albums, shelves and artists,
// matched word by word (title, artist, album). Enter (or the magnifying glass) searches everywhere.

let pageQuery = "";

const pageWords = (s) => String(s || "").toLowerCase().normalize("NFKD").replace(/[̀-ͯ]/g, "")
  .split(/[^a-z0-9À-ɏЀ-ӿ぀-鿿]+/).filter(Boolean);

// every word typed is the start of a word in what's shown
function pageMatches(text, want) {
  const have = pageWords(text);
  return want.every((w) => have.some((h) => h.startsWith(w)));
}

function pageItemText(node) {
  if (node.dataset.i !== undefined && viewTracks[Number(node.dataset.i)]) {
    const t = viewTracks[Number(node.dataset.i)];
    return [t.title, artistName(t), t.album || (t.spAlbum && t.spAlbum.name) || "", t.genre || ""].join(" ");
  }
  if (node._album) return node._album.title + " " + (node._album.artist || "");
  if (node._tile) {
    const x = node._tile;
    return [x.name, x.sub, x.track && x.track.title, x.track && artistName(x.track), x.mix && x.mix.title].filter(Boolean).join(" ");
  }
  return node.textContent;
}

function pageFilterApply() {
  const want = pageWords(pageQuery);
  const items = [...document.querySelectorAll("#list > li[data-i], #list > li.albumTile, #listView .tile")];
  let shown = 0;
  items.forEach((n) => {
    const ok = !want.length || pageMatches(pageItemText(n), want);
    n.classList.toggle("pageMiss", !ok);
    if (ok) shown++;
  });
  // shelves with nothing left in them fold away
  document.querySelectorAll("#listView .shelf").forEach((s) => {
    s.classList.toggle("pageMiss", !!want.length && !s.querySelector(".tile:not(.pageMiss)"));
  });
  const note = $("pageFindNote");
  note.hidden = !want.length;
  if (want.length) {
    note.textContent = (shown ? shown + " of " + items.length + " on this page" : "Nothing on this page matches") + ". Press Enter to search everywhere.";
  }
}

function pageFilterClear() {
  if (!pageQuery && !$("q").value) return;
  pageQuery = "";
  $("q").value = "";
  pageFilterApply();
}

function pageFindInit() {
  $("q").addEventListener("input", () => {
    pageQuery = $("q").value.trim();
    // on Search itself the bar is hidden; anywhere else it narrows what's shown
    if (view !== "search") pageFilterApply();
  });
  $("q").addEventListener("keydown", (e) => { if (e.key === "Escape" && $("q").value) { e.preventDefault(); pageFilterClear(); } });
  $("q").placeholder = "Search this page, or Enter for everywhere";
}
