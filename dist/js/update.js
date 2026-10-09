// Updates (Settings > App & Look > Updates): a little after each launch, the public releases repo is
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

function updateInit() {
  $("updBtn").addEventListener("click", () => (updateFound ? updateInstall() : updateCheck(true)));
  updatePaint();
  setTimeout(() => updateCheck(false), 8000);   // once the app's settled
}
