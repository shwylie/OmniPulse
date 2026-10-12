// Updates (Settings > Updates & Backups): a little after each launch, the public releases repo is
// asked whether there's a newer OmniPulse; if there is, the status bar offers it. Update downloads it,
// installs it (the installer's progress window, no questions) and starts the new one. Every release
// is signed with OmniPulse's key and checked against it before it installs (src-tauri: update_*).

let updateFound = null;   // { version, current, notes }

async function updateCheck(byHand) {
  if (byHand) $("updNote").textContent = "Checking...";
  try {
    updateFound = await invoke("update_check");
  } catch (e) {
    updateFound = null;
    if (byHand) $("updNote").textContent = "Couldn't check: " + String(e).replace(/^Error: /, "");
    return;
  }
  updatePaint();
  if (updateFound) say("OmniPulse " + updateFound.version + " is out.", { stay: true, force: true, actions: [["Update", updateInstall], ["Later", () => say("", { force: true })]] });
  else if (byHand) say("OmniPulse is up to date.", { force: true });
}

function updatePaint() {
  $("updNote").textContent = updateFound ? "OmniPulse " + updateFound.version + " is ready to install (you have " + updateFound.current + ")."
    : "Checked when OmniPulse starts. You're up to date.";
  $("updBtn").textContent = updateFound ? "Update" : "Check now";
}

async function updateInstall() {
  const ch = new window.__TAURI__.core.Channel();
  ch.onmessage = (p) => {
    const pct = p.total ? Math.round((100 * p.done) / p.total) : 0;
    say("Downloading OmniPulse " + (updateFound ? updateFound.version : "") + (pct ? ": " + pct + "%" : "") + "...", { stay: true, force: true });
  };
  say("Downloading the update...", { stay: true, force: true });
  try {
    await invoke("update_install", { progress: ch });   // (installs, then OmniPulse starts again)
  } catch (e) {
    say("Couldn't update: " + String(e).replace(/^Error: /, ""), { force: true });
  }
}

// ---------- the native app (0.3.x): offered, never forced ----------
// Its own releases repo (shwylie/omnipulse-native-releases). Once one's out, the status bar offers the
// move: Switch to the Native App installs it in this one's place (same folder and data, signed with
// the same key), Not Now puts it off until the next native version comes out. Settings > Updates
// offers it too, whenever there is one.
let nativeFound = null;   // { version, notes }

async function nativeCheck() {
  try { nativeFound = await invoke("native_check"); } catch (e) { nativeFound = null; }
  nativePaint();
  if (!nativeFound || updateFound) return;   // (this one's own update first)
  if (prefs.nativeDismissed === nativeFound.version) return;   // put off: asked again at the next one
  say("The native OmniPulse " + nativeFound.version + " is out: lighter and faster, with Seamless Mix.", { stay: true, force: true, actions: [
    ["Switch to the Native App", nativeInstall],
    ["Not Now", () => { prefs.nativeDismissed = nativeFound.version; savePrefs(); say("", { force: true }); }],
  ] });
}

function nativePaint() {
  $("nativeRow").hidden = !nativeFound;
  if (nativeFound) $("nativeNote").textContent = "OmniPulse " + nativeFound.version + ", rebuilt as a native app: lighter and faster, with Seamless Mix. It takes this one's place, with your settings, library and playlists.";
}

async function nativeInstall() {
  const ch = new window.__TAURI__.core.Channel();
  ch.onmessage = (p) => {
    const pct = p.total ? Math.round((100 * p.done) / p.total) : 0;
    say("Downloading the native OmniPulse" + (pct ? ": " + pct + "%" : "") + "...", { stay: true, force: true });
  };
  say("Downloading the native OmniPulse...", { stay: true, force: true });
  try {
    await invoke("native_install", { progress: ch });   // (its installer closes this one and starts it)
  } catch (e) {
    say("Couldn't switch: " + String(e).replace(/^Error: /, ""), { force: true });
  }
}

function updateInit() {
  $("updBtn").addEventListener("click", () => (updateFound ? updateInstall() : updateCheck(true)));
  $("nativeBtn").addEventListener("click", nativeInstall);
  updatePaint();
  setTimeout(() => updateCheck(false).then(() => setTimeout(nativeCheck, 4000)), 8000);   // once the app's settled
  // (a new native version while it's left running: asked about then)
  setInterval(nativeCheck, 6 * 3600 * 1000);
}
