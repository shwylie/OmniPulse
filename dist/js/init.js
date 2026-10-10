// Startup wiring for everything in js/. Loaded last of these, before app.js.

// Called by openSettings in app.js: copies the current prefs into the dialog
function extrasSyncSettings() {
  paintHiddenPlaylists();
  reqPaint();
  $("artAccentToggle").checked = !!prefs.artAccent;
  loudPaint();
  $("customCss").value = prefs.css || "";
  $("presToggle").checked = !!prefs.presence;
  $("scrobbleToggle").checked = !!prefs.scrobble;
  $("trayToggle").checked = !!prefs.closeToTray;
  lfmRefresh();
}

// Called once by app.js at startup. The order matters a little: the widget has to be
// ready before anything pushes to it, and the audio chain is built after the UI is wired.
function extrasInit() {
  playbackInit();
  autoplayInit();
  stateNotes();
  lyricsInit();
  statsInit();
  appearanceInit();
  integrationsInit();
  libraryInit();
  xplInit();
  xmInit();
  shareInit();
  favouritesInit();
  setsInit();
  feedInit();
  sessionInit();
  remoteInit();
  dndInit();
  selectInit();
  ytAccInit();
  streamerPaint();
  streamSafeInit();
  keySetupInit();
  document.querySelectorAll("#libSort button").forEach((b) => b.addEventListener("click", () => libSortPick(b.dataset.by)));
  libSortPaint();
  backupInit();
  streamerPageInit();
  safePaint();
  $("smSafe").addEventListener("change", () => safeSet($("smSafe").checked));
  // Extended goes with Streamer Mode: off with it (and never on without it)
  if (streamSafe() && !prefs.streamerMode) { prefs.streamerMode = true; savePrefs(); streamerPaint(); }
  $("smPrivate").addEventListener("change", () => {
    prefs.streamerMode = $("smPrivate").checked;
    savePrefs();
    if (!prefs.streamerMode && streamSafe()) safeSet(false);
    streamerPaint();
  });
  $("streamerBtn").addEventListener("click", streamerStep);
  // Settings' categories: opened and closed at their titles, and kept that way
  document.querySelectorAll("#settings .group > h3").forEach((h3) => {
    const key = h3.textContent.trim();
    const group = h3.parentElement;
    group.classList.toggle("collapsed", !!(prefs.setClosed || {})[key]);
    h3.setAttribute("role", "button");
    h3.tabIndex = 0;
    const flip = () => {
      const closed = !group.classList.contains("collapsed");
      group.classList.toggle("collapsed", closed);
      prefs.setClosed = Object.assign({}, prefs.setClosed, { [key]: closed });
      savePrefs();
    };
    h3.addEventListener("click", flip);
    h3.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); flip(); } });
  });
  widgetInit();
  navInit();
  discoverInit();
  titlesInit();
  platformsInit();
  requestsInit();
  albumsInit();
  audioInit();
  loudInit();
  out2Init();
  ytInit();
  mbInit();
  catInit();
  sbInit();
  appleInit();
  pluginsInit();
  updateInit();
  collabInit();
  castInit();
  nativeOutInit();
  togetherInit();
  fadeInit();
  pageFindInit();
  exportInit();
  artistsInit();
  searchPageInit();
  trimsInit();

  $("trayToggle").addEventListener("change", () => {
    prefs.closeToTray = $("trayToggle").checked;
    savePrefs();
  });
  // "Manage subscription" links
  document.addEventListener("click", (e) => {
    const b = e.target.closest(".linkBtn[data-url]");
    if (b) invoke("open_url", { url: b.dataset.url }).catch((err) => say(String(err)));
  });
}
