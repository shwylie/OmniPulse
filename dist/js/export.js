// A copy of what this app keeps in the WebView's storage (favourites, stats, settings, playlists on
// this PC, YouTube Music likes, play history), written to the data folder for the native OmniPulse,
// which can't read the WebView's storage. When the app opens, every two minutes, and as it closes.

const EXPORT_KEYS = ["sc.favourites", "sc.stats", "sc.prefs", "sc.xplaylists", "sc.ytLikes", "sc.lastPlayed", "sc.heardSongs"];
let exportLast = "";

function webExport() {
  const data = { savedAt: Date.now() };
  EXPORT_KEYS.forEach((k) => { try { const v = localStorage.getItem(k); if (v !== null) data[k] = v; } catch (e) {} });
  const text = JSON.stringify(data);
  // only when something changed (savedAt aside)
  const sig = text.slice(text.indexOf(",") + 1);
  if (sig === exportLast) return;
  exportLast = sig;
  invoke("web_export", { data: text }).catch(() => {});
}

function exportInit() {
  setTimeout(webExport, 4000);
  setInterval(webExport, 120000);
  window.addEventListener("beforeunload", webExport);
}
