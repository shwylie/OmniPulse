// Back Up and Restore (Settings > App): your settings, keys, favourites, playlists, stats, history and
// the alerts' sounds and pictures, in one .zip (crates/core/src/backup.rs). Sign-ins aren't in it.
// Restoring puts it all back and reloads the app.

// What this app keeps in the WebView's storage: everything under "sc."
function backupWebData() {
  const data = {};
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i);
      if (k && k.startsWith("sc.")) data[k] = localStorage.getItem(k);
    }
  } catch (e) { /* storage unreadable: the files still go in */ }
  return data;
}

async function backupMake() {
  const dlg = window.__TAURI__ && window.__TAURI__.dialog;
  if (!dlg || !dlg.save) { say("Backing up needs the app."); return; }
  const day = new Date().toISOString().slice(0, 10);
  const dest = await dlg.save({ title: "Back up OmniPulse", defaultPath: "OmniPulse Backup " + day + ".zip", filters: [{ name: "OmniPulse backup", extensions: ["zip"] }] }).catch(() => null);
  if (!dest) return;
  savePrefs();
  say("Backing up...");
  try {
    const n = await invoke("backup_create", { dest: String(dest), web: JSON.stringify(backupWebData()) });
    say("Backed up: your settings and " + plural(n, "file", "files") + ".", { force: true });
  } catch (e) {
    say("Couldn't back up: " + e, { force: true });
  }
}

async function backupRestore() {
  const dlg = window.__TAURI__ && window.__TAURI__.dialog;
  if (!dlg) { say("Restoring needs the app."); return; }
  const src = await dlg.open({ multiple: false, title: "Restore an OmniPulse backup", filters: [{ name: "OmniPulse backup", extensions: ["zip"] }] }).catch(() => null);
  if (!src) return;
  if (!confirm("Restore this backup? Your settings, favourites, playlists, stats and alerts are replaced with the backup's, and OmniPulse reloads.")) return;
  let web;
  try {
    web = JSON.parse(await invoke("backup_restore", { src: String(src) }));
  } catch (e) {
    say("Couldn't restore it: " + e, { force: true });
    return;
  }
  try {
    const old = [];
    for (let i = 0; i < localStorage.length; i++) { const k = localStorage.key(i); if (k && k.startsWith("sc.")) old.push(k); }
    old.forEach((k) => localStorage.removeItem(k));
    Object.entries(web).forEach(([k, v]) => { if (k.startsWith("sc.") && typeof v === "string") localStorage.setItem(k, v); });
  } catch (e) {
    say("The files are back, but the settings couldn't be: " + e, { force: true });
    return;
  }
  // nothing may save the old settings over the restored ones on the way out (the page saves as it closes)
  Storage.prototype.setItem = function () {};
  Storage.prototype.removeItem = function () {};
  location.reload();
}

function backupInit() {
  $("backupMake").addEventListener("click", backupMake);
  $("backupRestore").addEventListener("click", backupRestore);
}
