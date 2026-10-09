// Your own keys for Spotify, Last.fm and a YouTube account (Settings > Integrations > Set up). Each
// platform gives them out free, from its developer pages; none lets an app make them for you. So this
// opens each page in your browser (where you're signed in already), says what to put where, with the
// values to copy, and takes the keys you paste back: anywhere in the window, or into their boxes.
// They're kept in config.json on this PC (config_set), never shown again after.

const KEY_SETUPS = {
  spotify: {
    name: "Spotify",
    fields: [["spotify_client_id", "Client ID", /^[0-9a-f]{32}$/i]],
    steps: (st) => [
      ["Open the Spotify Developer Dashboard and sign in. If it asks, accept the developer terms.", "https://developer.spotify.com/dashboard"],
      ["Create an app. Name and description: anything (say, OmniPulse). Redirect URI: the address below, then Add. Tick Web API and Web Playback SDK, agree to the terms, and Save.", "https://developer.spotify.com/dashboard/create",
        [["App name", "OmniPulse"], ["Redirect URI", st.spotify_redirect || "http://127.0.0.1:8081/callback"]]],
      ["On your app's page, open Settings and copy its Client ID. There's no secret to copy: OmniPulse signs in without one.", null],
    ],
    note: "A new Spotify app is in development mode: your own account can sign in to it straight away.",
    done: () => spotifyCheck(),
    signIn: "spConnect",
  },
  lastfm: {
    name: "Last.fm",
    fields: [["lastfm_api_key", "API Key", /^[0-9a-f]{32}$/i], ["lastfm_secret", "Shared Secret", /^[0-9a-f]{32}$/i]],
    steps: () => [
      ["Open Last.fm's API account page and sign in.", "https://www.last.fm/api/account/create"],
      ["Fill in an application name and description (anything: say, OmniPulse). The callback URL can stay empty. Submit.", null, [["Application name", "OmniPulse"]]],
      ["Copy the API key and the shared secret it shows you.", null],
    ],
    note: "Last.fm shows them once on that page; it also lists them under your API accounts later.",
    done: () => lfmRefresh(),
    signIn: "lfmConnect",
  },
  youtube: {
    name: "YouTube",
    fields: [["youtube_client_id", "Client ID", /^\d+-[0-9a-z]+\.apps\.googleusercontent\.com$/], ["youtube_client_secret", "Client Secret", /^GOCSPX-[\w-]{10,}$/]],
    steps: () => [
      ["Open Google Cloud and create a project (any name: say, OmniPulse).", "https://console.cloud.google.com/projectcreate"],
      ["Switch on the YouTube Data API v3 for it.", "https://console.cloud.google.com/apis/library/youtube.googleapis.com"],
      ["Set up its sign-in screen: app name OmniPulse, your email, audience External. Under Audience, add your own Google account as a test user.", "https://console.cloud.google.com/auth/overview"],
      ["Create an OAuth client: application type Desktop app, any name. Copy its Client ID and Client secret, or download its JSON file and drop it here.", "https://console.cloud.google.com/auth/clients/create"],
    ],
    note: "It's free. Google may say the app isn't verified when you sign in: it's yours, so that's expected.",
    done: () => ytAccRefresh(),
    signIn: "ytAccBtn",
  },
};

let keySetupFor = null;

// Keys pulled out of something pasted or dropped: a key on its own, several, or Google's JSON file
function keysFrom(text, plat) {
  const out = {};
  const s = KEY_SETUPS[plat];
  try {
    const j = JSON.parse(text);
    const c = j.installed || j.web || j;
    if (plat === "youtube" && c.client_id) { out.youtube_client_id = c.client_id; if (c.client_secret) out.youtube_client_secret = c.client_secret; return out; }
  } catch (e) { /* not JSON */ }
  const words = String(text).split(/[\s"',:;=]+/).filter(Boolean);
  s.fields.forEach(([key, , re]) => {
    const hit = words.find((w) => re.test(w) && !Object.values(out).includes(w));
    if (hit) out[key] = hit;
  });
  return out;
}

function keySetupPaint() {
  const plat = keySetupFor, s = KEY_SETUPS[plat];
  $("ksHead").textContent = "Set Up " + s.name;
  const st = keySetupPaint.status || {};
  const ol = $("ksSteps");
  ol.replaceChildren(...s.steps(st).map(([text, url, copies]) => {
    const li = el("li");
    li.append(el("span", "", text));
    (copies || []).forEach(([label, value]) => {
      const row = el("div", "ksCopy");
      const b = el("button", "soft", "Copy");
      b.type = "button";
      b.addEventListener("click", async () => {
        try { await navigator.clipboard.writeText(value); b.textContent = "Copied"; setTimeout(() => { b.textContent = "Copy"; }, 1500); } catch (e) { say("Couldn't copy: " + e); }
      });
      row.append(el("span", "note", label), el("code", "", value), b);
      li.append(row);
    });
    if (url) {
      const go = el("button", "soft ksOpen", "Open in Browser");
      go.type = "button";
      go.addEventListener("click", () => invoke("open_setup_page", { url }).catch((e) => say(String(e))));
      li.append(go);
    }
    return li;
  }));
  $("ksFields").replaceChildren(...s.fields.map(([key, label]) => {
    const row = el("label", "ksField");
    const input = el("input");
    input.type = "text";
    input.id = "ks_" + key;
    input.autocomplete = "off";
    input.spellcheck = false;
    input.placeholder = "Paste it here";
    input.addEventListener("input", keySetupCheck);
    row.append(el("span", "lbl", label), input);
    return row;
  }));
  $("ksNote").textContent = s.note;
  keySetupCheck();
}

// Each box: fine, not yet, or not what that platform's keys look like
function keySetupCheck() {
  const s = KEY_SETUPS[keySetupFor];
  let ok = true;
  s.fields.forEach(([key, , re]) => {
    const i = $("ks_" + key), v = i.value.trim();
    i.classList.toggle("bad", !!v && !re.test(v));
    if (!re.test(v)) ok = false;
  });
  $("ksSave").disabled = !ok;
}

// Something pasted (or a file dropped) anywhere in the window: the keys in it go in their boxes
function keySetupTake(text) {
  const found = keysFrom(text, keySetupFor);
  const keys = Object.keys(found);
  if (!keys.length) return false;
  keys.forEach((k) => { $("ks_" + k).value = found[k]; });
  keySetupCheck();
  say("Found " + keys.map((k) => KEY_SETUPS[keySetupFor].fields.find((f) => f[0] === k)[1]).join(" and ") + ".");
  return true;
}

async function keySetupOpen(plat) {
  keySetupFor = plat;
  try { keySetupPaint.status = await invoke("config_status"); } catch (e) { keySetupPaint.status = {}; }
  keySetupPaint();
  if ($("settings").open) $("settings").close();
  $("keySetup").showModal();
}

async function keySetupSave(e) {
  e.preventDefault();
  const s = KEY_SETUPS[keySetupFor];
  const fields = {};
  s.fields.forEach(([key]) => { fields[key] = $("ks_" + key).value.trim(); });
  try {
    await invoke("config_set", { fields });
  } catch (err) {
    say("Couldn't save them: " + err);
    return;
  }
  $("keySetup").close();
  await keySetupStatus();
  s.done();
  const btn = $(s.signIn);
  say(s.name + " is set up.", btn ? { stay: true, actions: [["Sign In Now", () => { $("settings").showModal(); btn.click(); }]] } : undefined);
}

// Set up where a platform has no keys yet; "Your keys" to change them where it has
async function keySetupStatus() {
  let st = {};
  try { st = await invoke("config_status"); } catch (e) { return; }
  [["spotify", "spSetup", "spConnect", "spKeys"], ["lastfm", "lfmSetup", "lfmConnect", "lfmKeys"], ["youtube", "ytAccSetup", "ytAccBtn", "ytKeys"]].forEach(([plat, setup, signIn, keys]) => {
    const has = !!st[plat];
    $(setup).hidden = has;
    $(keys).hidden = !has;
    if (!has) $(signIn).hidden = true;
    else if (plat !== "youtube" || ytWanted()) $(signIn).hidden = false;
    // Spotify's options (Your Keys) once it has keys, while it's switched on
    if (plat === "spotify") $("spotifyOptions").hidden = !(FEATURES.spotify && has && prefs.spOn !== false);
  });
}

function keySetupInit() {
  [["spSetup", "spotify"], ["lfmSetup", "lastfm"], ["ytAccSetup", "youtube"], ["spKeys", "spotify"], ["lfmKeys", "lastfm"], ["ytKeys", "youtube"]]
    .forEach(([id, plat]) => $(id).addEventListener("click", () => keySetupOpen(plat)));
  $("ksForm").addEventListener("submit", keySetupSave);
  $("ksCancel").addEventListener("click", () => $("keySetup").close());
  $("keySetup").addEventListener("paste", (e) => {
    const text = (e.clipboardData && e.clipboardData.getData("text")) || "";
    if (keySetupTake(text)) e.preventDefault();
  });
  $("keySetup").addEventListener("dragover", (e) => e.preventDefault());
  $("keySetup").addEventListener("drop", async (e) => {
    e.preventDefault();
    const f = e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files[0];
    if (f && f.size < 65536 && !keySetupTake(await f.text())) say("That file has no keys for " + KEY_SETUPS[keySetupFor].name + " in it.");
  });
  keySetupStatus();
}
