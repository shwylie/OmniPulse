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
  // the two settings still go on and off on their own
  $("smPrivate").addEventListener("change", () => {
    prefs.streamerMode = $("smPrivate").checked;
    savePrefs();
    streamerPaint();
  });
  $("streamerBtn").addEventListener("click", streamerStep);
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
  pluginsInit();
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
