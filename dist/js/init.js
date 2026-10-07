// Startup wiring for everything in js/. Loaded last of these, before app.js.

// Called by openSettings in app.js: copies the current prefs into the dialog
function extrasSyncSettings() {
  reqPaint();
  $("artAccentToggle").checked = !!prefs.artAccent;
  $("normSel").value = prefs.normalize || "off";
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
  lyricsInit();
  statsInit();
  appearanceInit();
  integrationsInit();
  libraryInit();
  xplInit();
  favouritesInit();
  setsInit();
  dndInit();
  widgetInit();
  navInit();
  discoverInit();
  titlesInit();
  platformsInit();
  requestsInit();
  albumsInit();
  audioInit();

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
