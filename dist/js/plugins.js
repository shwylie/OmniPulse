// Plugins: other sources of music, made by others (crates/core/src/plugins.rs installs and stores them).
//
// Each plugin's script runs in its own sandboxed frame: a separate origin with a policy that blocks every
// network request and anything else it could reach in the app. It can only send the app messages: to
// register what it offers, and to ask for web requests, which the app makes for it (only to the sites
// its manifest lists; the Rust side checks each one again). Its settings are given to it.
//
// What a plugin offers (OmniPulse.register({...}) in its script):
//   search(query)    -> [track]          the songs that match (required)
//   resolve(track)   -> { url }          a playable address for one of them (required)
//   library()        -> [track]          your saved songs there, for the Library (optional)
// A track: { id, title, artist, album?, duration_ms?, artwork?, link? }
//
// In its script, OmniPulse.fetch(url, { method, headers, body }) -> { ok, status, headers, text(), json() },
// OmniPulse.settings (its settings, as you set them), OmniPulse.onSettings(fn), OmniPulse.log(...).

const PLUGIN_ICON = "M20.5 11H19V7c0-1.1-.9-2-2-2h-4V3.5C13 2.12 11.88 1 10.5 1S8 2.12 8 3.5V5H4c-1.1 0-1.99.9-1.99 2v3.8H3.5c1.49 0 2.7 1.21 2.7 2.7s-1.21 2.7-2.7 2.7H2V20c0 1.1.9 2 2 2h3.8v-1.5c0-1.49 1.21-2.7 2.7-2.7 1.49 0 2.7 1.21 2.7 2.7V22H17c1.1 0 2-.9 2-2v-4h1.5c1.38 0 2.5-1.12 2.5-2.5S21.88 11 20.5 11z";
const PLUGIN_CALL_MS = 25000;

const pluginRuns = new Map();   // id -> { manifest, frame, ready, methods, calls, error }
const pluginLibrary = new Map(); // platform id -> tracks, once fetched

const plugPlat = (id) => "plug:" + id;
const isPluginTrack = (t) => srcOf(t).startsWith("plug:");
const pluginIdOf = (t) => srcOf(t).slice(5);
const pluginOn = (id) => prefs["plugOn_" + id] !== false;
const pluginSettings = (id) => Object.assign({}, (prefs.pluginSettings || {})[id] || {});

// ---------- the sandbox ----------
// Nothing in here can reach the network or the app: the policy allows only its own inline scripts.
const PLUGIN_FRAME = `<!doctype html><html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval'">
</head><body><script>
"use strict";
(() => {
  // no way back to the app other than postMessage: Tauri's bridge and WebView2's message channel go
  ["__TAURI__", "__TAURI_INTERNALS__", "__TAURI_IPC__", "ipc", "chrome"].forEach((k) => { try { delete window[k]; } catch (e) {} try { Object.defineProperty(window, k, { value: undefined, configurable: false, writable: false }); } catch (e) {} });
  const pending = new Map();
  let seq = 0, impl = null, settingsFns = [];
  const send = (m) => parent.postMessage(Object.assign({ omnipulsePlugin: true }, m), "*");
  const ask = (op, args) => new Promise((resolve, reject) => { const id = ++seq; pending.set(id, { resolve, reject }); send({ kind: "ask", id, op, args }); });
  const api = {
    settings: {},
    register(x) { impl = x || {}; send({ kind: "ready", methods: Object.keys(impl).filter((k) => typeof impl[k] === "function") }); },
    async fetch(url, init) {
      init = init || {};
      const r = await ask("fetch", { url: String(url), method: init.method || "GET", headers: Object.entries(init.headers || {}).map(([k, v]) => [String(k), String(v)]), body: init.body == null ? null : String(init.body) });
      return { ok: r.status >= 200 && r.status < 300, status: r.status, headers: r.headers, text: async () => r.body, json: async () => JSON.parse(r.body) };
    },
    onSettings(fn) { settingsFns.push(fn); },
    log(...a) { send({ kind: "log", text: a.map(String).join(" ") }); },
  };
  window.addEventListener("message", async (e) => {
    if (e.source !== parent) return;
    const m = e.data || {};
    if (m.kind === "load") {
      api.settings = m.settings || {};
      try { new Function("OmniPulse", m.code)(api); } catch (err) { send({ kind: "failed", error: String(err && err.stack || err) }); }
    } else if (m.kind === "settings") {
      api.settings = m.settings || {};
      settingsFns.forEach((f) => { try { f(api.settings); } catch (err) {} });
    } else if (m.kind === "answer") {
      const p = pending.get(m.id);
      if (!p) return;
      pending.delete(m.id);
      if (m.error) p.reject(new Error(m.error)); else p.resolve(m.result);
    } else if (m.kind === "call") {
      try {
        if (!impl || typeof impl[m.method] !== "function") throw new Error("this plugin has no " + m.method);
        const result = await impl[m.method](...(m.args || []));
        send({ kind: "result", id: m.id, result: JSON.parse(JSON.stringify(result === undefined ? null : result)) });
      } catch (err) {
        send({ kind: "result", id: m.id, error: String(err && err.message || err) });
      }
    }
  });
  send({ kind: "booted" });
})();
<\/script></body></html>`;

function pluginStart(pkg) {
  const id = pkg.manifest.id;
  pluginStop(id);
  const run = { manifest: pkg.manifest, code: pkg.code, frame: null, ready: false, methods: [], calls: new Map(), seq: 0, error: "" };
  const frame = document.createElement("iframe");
  frame.setAttribute("sandbox", "allow-scripts");   // its own origin: no app, no storage, no cookies
  frame.hidden = true;
  frame.title = "Plugin: " + pkg.manifest.name;
  frame.srcdoc = PLUGIN_FRAME;
  run.frame = frame;
  pluginRuns.set(id, run);
  document.body.append(frame);
  return run;
}

function pluginStop(id) {
  const run = pluginRuns.get(id);
  if (!run) return;
  run.calls.forEach((c) => c.reject(new Error("the plugin was stopped")));
  run.frame.remove();
  pluginRuns.delete(id);
}

window.addEventListener("message", async (e) => {
  const m = e.data;
  if (!m || !m.omnipulsePlugin) return;
  const run = [...pluginRuns.values()].find((r) => r.frame.contentWindow === e.source);
  if (!run) return;
  const id = run.manifest.id;
  const post = (msg) => run.frame.contentWindow.postMessage(msg, "*");
  if (m.kind === "booted") {
    post({ kind: "load", code: run.code, settings: pluginSettings(id) });
  } else if (m.kind === "ready") {
    run.ready = true;
    run.methods = Array.isArray(m.methods) ? m.methods.map(String) : [];
    pluginPlatformsSync();
  } else if (m.kind === "failed") {
    run.error = String(m.error || "it didn't start");
    console.warn("plugin " + id + ": " + run.error);
    pluginPaint();
  } else if (m.kind === "log") {
    console.log("[plugin " + id + "] " + String(m.text).slice(0, 2000));
  } else if (m.kind === "result") {
    const c = run.calls.get(m.id);
    if (!c) return;
    run.calls.delete(m.id);
    clearTimeout(c.timer);
    if (m.error) c.reject(new Error(m.error)); else c.resolve(m.result);
  } else if (m.kind === "ask") {
    // the only thing a plugin can ask for: a web request, to one of its sites (checked in Rust too)
    let result = null, error = "";
    try {
      if (m.op !== "fetch") throw new Error("plugins can't ask for " + m.op);
      const a = m.args || {};
      result = await invoke("plugin_http", { id, url: String(a.url), method: String(a.method || "GET"), headers: (a.headers || []).slice(0, 50), body: a.body == null ? null : String(a.body) });
    } catch (err) { error = String(err && err.message ? err.message : err); }
    post({ kind: "answer", id: m.id, result, error });
  }
});

function pluginCall(id, method, ...args) {
  const run = pluginRuns.get(id);
  if (!run || !run.ready) return Promise.reject(new Error("The plugin isn't running."));
  return new Promise((resolve, reject) => {
    const callId = ++run.seq;
    const timer = setTimeout(() => { run.calls.delete(callId); reject(new Error(run.manifest.name + " took too long to answer.")); }, PLUGIN_CALL_MS);
    run.calls.set(callId, { resolve, reject, timer });
    run.frame.contentWindow.postMessage({ kind: "call", id: callId, method, args }, "*");
  });
}

// ---------- a plugin's tracks in the app ----------
function pluginTrack(id, x) {
  if (!x || x.id == null) return null;
  return tidyTrack({
    source: plugPlat(id),
    id: plugPlat(id) + ":" + String(x.id),
    pTrack: x,
    title: String(x.title || "Untitled"),
    user: { username: String(x.artist || "Unknown artist") },
    album: x.album ? String(x.album) : "",
    duration: Number(x.duration_ms) || 0,
    artwork_url: x.artwork ? String(x.artwork) : "",
    permalink_url: x.link && /^https:\/\//.test(String(x.link)) ? String(x.link) : "",
    license: x.license ? String(x.license) : "",
  });
}

async function pluginSearch(id, q) {
  const list = await pluginCall(id, "search", q);
  return (Array.isArray(list) ? list : []).slice(0, 200).map((x) => pluginTrack(id, x)).filter(Boolean);
}

async function pluginLibraryOf(id) {
  const run = pluginRuns.get(id);
  if (!run || !run.methods.includes("library")) return [];
  if (!pluginLibrary.has(plugPlat(id))) {
    const list = await pluginCall(id, "library").catch(() => []);
    pluginLibrary.set(plugPlat(id), (Array.isArray(list) ? list : []).map((x) => pluginTrack(id, x)).filter(Boolean));
  }
  return pluginLibrary.get(plugPlat(id));
}

// What to play: the plugin's address, through the local server (only from the plugin's sites)
async function pluginPlayUrl(t) {
  const id = pluginIdOf(t);
  const r = await pluginCall(id, "resolve", t.pTrack);
  const url = r && r.url ? String(r.url) : "";
  if (!/^https?:\/\//.test(url)) throw new Error(PLATFORMS[srcOf(t)].name + " didn't give a playable address.");
  if (!localInfo) localInfo = await invoke("local_server");
  return "http://127.0.0.1:" + localInfo.port + "/plug?k=" + encodeURIComponent(localInfo.token) + "&p=" + encodeURIComponent(id) + "&u=" + encodeURIComponent(url);
}

// Each running plugin that can search and play is a platform (on Search, and the Library if it has one)
function pluginPlatformsSync() {
  for (let i = PLATFORM_LIST.length - 1; i >= 0; i--) if (PLATFORM_LIST[i].plugin) PLATFORM_LIST.splice(i, 1);
  pluginRuns.forEach((run, id) => {
    if (!run.ready || !run.methods.includes("search") || !run.methods.includes("resolve")) return;
    const pid = plugPlat(id);
    PLATFORMS[pid] = { name: run.manifest.name, icon: /^[MmZzLlHhVvCcSsQqTtAa0-9 ,.\-]+$/.test(run.manifest.icon || "") ? run.manifest.icon : PLUGIN_ICON };
    const pages = ["search"].concat(run.methods.includes("library") ? ["likes"] : []);
    PLATFORM_LIST.push({ id: pid, pref: "plugOn_" + id, pages, plugin: true, available: () => pluginRuns.has(id) && pluginRuns.get(id).ready });
  });
  pluginPaint();
  renderPlatChips();
}

// ---------- Settings > Plugins ----------
function pluginField(id, f) {
  const key = String(f.key || "");
  const vals = pluginSettings(id);
  const value = key in vals ? vals[key] : f.default;
  const row = el("div", "row");
  const left = el("div");
  left.append(el("div", "lbl", String(f.label || key)));
  if (f.note) left.append(el("div", "note", String(f.note)));
  let input;
  if (f.type === "toggle") {
    const sw = el("label", "switch");
    input = el("input");
    input.type = "checkbox";
    input.checked = !!value;
    sw.append(input, el("span"));
    row.append(left, sw);
  } else if (f.type === "select") {
    input = el("select", "narrow");
    (f.options || []).forEach((o) => { const v = typeof o === "object" ? o.value : o; input.append(new Option(String(typeof o === "object" ? o.label || o.value : o), String(v))); });
    input.value = value == null ? "" : String(value);
    row.append(left, input);
  } else {
    input = el("input", "nameInput");
    input.type = f.type === "password" ? "password" : f.type === "number" ? "number" : "text";
    input.value = value == null ? "" : String(value);
    input.autocomplete = "off";
    input.spellcheck = false;
    row.append(left, input);
  }
  input.addEventListener("change", () => {
    const all = Object.assign({}, prefs.pluginSettings || {});
    all[id] = Object.assign({}, all[id] || {}, { [key]: input.type === "checkbox" ? input.checked : f.type === "number" ? Number(input.value) : input.value });
    prefs.pluginSettings = all;
    savePrefs();
    const run = pluginRuns.get(id);
    if (run) run.frame.contentWindow.postMessage({ kind: "settings", settings: all[id] }, "*");
    pluginLibrary.delete(plugPlat(id));
  });
  return row;
}

let pluginPackages = [];

function pluginPaint() {
  const box = $("plugList");
  if (!box) return;
  box.replaceChildren();
  if (!pluginPackages.length) { box.append(el("p", "note", "No plugins yet.")); return; }
  pluginPackages.forEach((pkg) => {
    const m = pkg.manifest;
    const run = pluginRuns.get(m.id);
    const row = el("div", "row");
    const left = el("div");
    left.append(el("div", "lbl", m.name + (m.version ? " " + m.version : "")));
    const state = !pluginOn(m.id) ? "Off." : run && run.error ? "Couldn't start: " + run.error.split("\n")[0] : run && run.ready ? "Running." : "Starting...";
    left.append(el("div", "note", (m.description ? m.description + " " : "") + (m.author ? "By " + m.author + ". " : "") + state));
    const sw = el("label", "switch");
    const on = el("input");
    on.type = "checkbox";
    on.id = "plugOn_" + m.id;
    on.checked = pluginOn(m.id);
    on.addEventListener("change", () => pluginSetOn(m.id, on.checked));
    sw.append(on, el("span"));
    row.append(left, sw);
    const more = el("details", "sub");
    more.append(el("summary", "", m.name + " settings"));
    (Array.isArray(m.settings) ? m.settings : []).forEach((f) => more.append(pluginField(m.id, f)));
    more.append(el("p", "note", "Can reach: " + (m.hosts.length ? m.hosts.join(", ") : "no sites") + ". Offers: " + ((run && run.methods.join(", ")) || (m.capabilities || []).join(", ") || "nothing yet") + "."));
    const rm = el("button", "soft", "Remove " + m.name);
    rm.addEventListener("click", () => pluginRemove(m));
    more.append(rm);
    box.append(row, more);
  });
}

async function pluginSetOn(id, on) {
  prefs["plugOn_" + id] = on;
  savePrefs();
  const pkg = pluginPackages.find((p) => p.manifest.id === id);
  if (on && pkg) pluginStart(pkg); else pluginStop(id);
  pluginPlatformsSync();
}

async function pluginRemove(m) {
  if (!confirm("Remove the plugin " + m.name + "? Its settings go too.")) return;
  pluginStop(m.id);
  await invoke("plugin_remove", { id: m.id }).catch((e) => say(String(e)));
  const all = Object.assign({}, prefs.pluginSettings || {});
  delete all[m.id];
  prefs.pluginSettings = all;
  delete prefs["plugOn_" + m.id];
  savePrefs();
  await pluginsLoad();
  say("Removed " + m.name + ".");
}

async function pluginsLoad() {
  try { pluginPackages = await invoke("plugin_list"); } catch (e) { pluginPackages = []; }
  const ids = new Set(pluginPackages.map((p) => p.manifest.id));
  [...pluginRuns.keys()].forEach((id) => { if (!ids.has(id)) pluginStop(id); });
  pluginPackages.forEach((p) => { if (pluginOn(p.manifest.id) && !pluginRuns.has(p.manifest.id)) pluginStart(p); });
  pluginPlatformsSync();
}

// ---------- adding one ----------
// One file (an .omniplugin package or a manifest.json) or a web address. It's read first, and what it
// will and won't be able to do is shown in plain words; it's only added once you agree, and only if
// it's still exactly what you saw.
let plugPending = null;   // { source, digest }

// What a plugin can and can't do, from its manifest (the sandbox enforces the rest)
function plugAccess(m) {
  const can = [], cannot = [], warn = [];
  const caps = new Set(m.capabilities || []);
  const hosts = m.hosts || [];
  const local = hosts.filter((h) => h === "127.0.0.1" || h === "localhost");
  // "example.com" and "*.example.com" read as one site
  const bare = [...new Set(hosts.filter((h) => !local.includes(h)).map((h) => h.replace(/^\*\./, "")))];
  const web = bare.map((h) => hosts.includes("*." + h) ? h + " (any part of it)" : h);
  if (caps.has("search") || !caps.size) can.push("Show its songs in your search results, so it sees what you search for");
  if (caps.has("play") || !caps.size) can.push("Play its songs, from its own sites, and know when you play one");
  if (caps.has("library")) can.push("Add its saved songs to your Library");
  if (web.length) can.push("Connect to " + andList(web));
  if (local.length) warn.push("It can connect to programs running on this PC (" + local.join(", ") + "). Only allow this if you know what it talks to.");
  const sets = Array.isArray(m.settings) ? m.settings : [];
  if (sets.some((x) => x && x.type === "password")) can.push("Keep a password or key you give it, on this PC; it can send that to the sites above");
  else if (sets.length) can.push("Keep its own settings, on this PC");
  if (!hosts.length) cannot.push("Connect to the internet");
  else cannot.push("Connect to any other site");
  cannot.push(
    "See your accounts or sign-ins (SoundCloud, Spotify, YouTube Music, Last.fm, Discord)",
    "Open, read or change your files",
    "See your playlists, Library, listening history or settings",
    "Change anything else in the app",
  );
  return { can, cannot, warn };
}

function plugList(title, items, cls) {
  const box = el("div", "plugAccess " + cls);
  box.append(el("div", "plugAccessHead", title));
  const ul = el("ul");
  items.forEach((t) => ul.append(el("li", "", t)));
  box.append(ul);
  return box;
}

async function plugCheck() {
  const src = $("plugSource").value.trim();
  const box = $("plugPreview");
  $("plugInstall").hidden = $("plugDeny").hidden = true;
  $("plugCancel").hidden = false;
  plugPending = null;
  if (!src) return;
  box.replaceChildren(el("p", "note", "Reading it..."));
  try {
    const pkg = await invoke("plugin_inspect", { source: src });
    const m = pkg.manifest;
    const have = pluginPackages.find((p) => p.manifest.id === m.id);
    const acc = plugAccess(m);
    const kb = Math.max(1, Math.round((pkg.code || "").length / 1024));
    box.replaceChildren(
      el("h3", "plugName", "Add " + m.name + "?"),
      el("p", "note", [m.version && "Version " + m.version, m.author && "by " + m.author, kb + " KB"].filter(Boolean).join(" \u00b7 ")),
      el("p", "note", m.description || ""),
      plugList("It will be able to", acc.can, "can"),
      plugList("It won't be able to", acc.cannot, "cannot"),
      ...acc.warn.map((w) => el("p", "note warn", w)),
      el("p", "note", "It runs walled off from the rest of OmniPulse, and you can turn it off or remove it any time in Settings > Plugins." + (have ? " Adding it replaces the " + (have.manifest.version || "installed") + " version you have." : "")),
    );
    plugPending = { source: src, digest: pkg.digest };
    $("plugInstall").textContent = have ? "Update plugin" : "Add plugin";
    $("plugInstall").hidden = $("plugDeny").hidden = false;
    $("plugCancel").hidden = true;
    $("plugInstall").focus();
  } catch (e) {
    box.replaceChildren(el("p", "note warn", String(e)));
  }
}

async function plugInstallNow() {
  if (!plugPending) return;
  try {
    const m = await invoke("plugin_install", plugPending);
    pluginStop(m.id);
    $("plugDialog").close();
    await pluginsLoad();
    say("Added " + m.name + ".");
  } catch (e) {
    $("plugPreview").append(el("p", "note warn", String(e)));
  }
}

function plugReset() {
  plugPending = null;
  $("plugPreview").replaceChildren();
  $("plugInstall").hidden = $("plugDeny").hidden = true;
  $("plugCancel").hidden = false;
}

function pluginsInit() {
  $("plugAdd").addEventListener("click", () => {
    $("plugSource").value = "";
    plugReset();
    $("plugDialog").showModal();
    $("plugSource").focus();
  });
  $("plugCheck").addEventListener("click", plugCheck);
  $("plugSource").addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); plugCheck(); } });
  $("plugSource").addEventListener("input", plugReset);
  $("plugInstall").addEventListener("click", plugInstallNow);
  $("plugDeny").addEventListener("click", () => { $("plugDialog").close(); say("Not added."); });
  $("plugCancel").addEventListener("click", () => $("plugDialog").close());
  $("plugFile").addEventListener("click", async () => {
    const dlg = window.__TAURI__ && window.__TAURI__.dialog;
    if (!dlg || !dlg.open) { say("Type the plugin file's path instead."); return; }
    const file = await dlg.open({
      multiple: false, directory: false, title: "Choose a plugin",
      filters: [{ name: "OmniPulse plugin", extensions: ["omniplugin", "json"] }],
    }).catch(() => null);
    if (file) { $("plugSource").value = String(file); plugCheck(); }
  });
  pluginsLoad();
}
