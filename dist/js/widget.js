// The now-playing widget for OBS.

// ---------- now playing widget for OBS ----------
const WIDGET_NOTE = "A now-playing card for OBS.";

let widgetReady = false;
let widgetCleared = false;   // "Clear" hides the card until the next track starts

// Liking a song someone requested: the card says so for a few seconds, with a word for them (a
// different one each time)
const WIDGET_LIKE_LINES = [
  "Good choice, {user}!", "{user} has taste.", "Certified banger, {user}.", "Nice one, {user}!",
  "{user} gets it.", "This one's a keeper, {user}.", "Top-tier pick, {user}.", "{user} cooked with this one.",
  "Straight into the likes, thanks {user}!", "Great call, {user}.", "{user} knows what's up.", "Taste confirmed, {user}.",
];
let widgetLikeNote = null;   // { text, until }
let widgetLikeOrder = [];
function widgetLiked(t) {
  if (!t || !t.requestedBy || t !== curTrack()) return;
  if (!widgetLikeOrder.length) widgetLikeOrder = shuffledOrder(WIDGET_LIKE_LINES.map((x, i) => i));   // each once before any repeats
  const line = WIDGET_LIKE_LINES[widgetLikeOrder.shift()];
  widgetLikeNote = { text: "♥ Liked · " + line.replace("{user}", String(t.requestedBy)), until: Date.now() + 8000 };
  widgetPush();
  setTimeout(widgetPush, 8100);
}

function widgetState() {
  const t = curTrack();
  // the look and the alerts' settings go along even with nothing playing (alerts still show)
  const look = {
    accent: accentNow[0],
    accent2: accentNow[1],
    design: prefs.widgetDesign || "card",
    rounded: prefs.widgetRounded !== false,
    align: prefs.widgetAlign === "right" ? "right" : "left",
    text_scale: prefs.widgetText || 1,
    alerts_cfg: alertsCfg(),
  };
  if (!t || widgetCleared) return Object.assign({ playing: false }, look);
  return {
    alerts_cfg: look.alerts_cfg,
    playing: nowPlaying(),
    title: t.title || "",
    artist: artistName(t),
    // who asked for it in chat (Settings: Show Who Requested It)
    requested_by: prefs.wRequester !== false && t.requestedBy ? String(t.requestedBy) : "",
    like_note: widgetLikeNote && Date.now() < widgetLikeNote.until && t.requestedBy ? widgetLikeNote.text : "",
    artwork: artUrl(t, "t500x500") || "",
    url: t.permalink_url || "",
    platform: srcOf(t),
    duration_ms: viewDur(t, t.duration || curDurMs()),
    position_ms: viewPos(t, curPosMs()),
    updated_at: Date.now(),
    accent: accentNow[0],
    accent2: accentNow[1],
    design: prefs.widgetDesign || "card",
    rounded: prefs.widgetRounded !== false,
    align: look.align,
    text_scale: prefs.widgetText || 1,
    anim: {
      fade: prefs.wFade !== false,
      scroll: prefs.wScroll !== false,
      spin: prefs.wSpin !== false,
      color: prefs.wColor === undefined ? 1.2 : Number(prefs.wColor),
    },
  };
}

function widgetPush() {
  hostPush();
  if (prefs.widget || prefs.req) invoke("widget_update", { state: widgetState() }).catch(() => {});
}

async function widgetApply() {
  const on = !!prefs.widget;
  $("widgetSection").hidden = !on;
  if (!on) {
    invoke("widget_stop").catch(() => {});
    $("widgetNote").textContent = WIDGET_NOTE;
    return;
  }
  try {
    if (prefs.botRemote && !prefs.alertKey) { prefs.alertKey = reqKeyNew() + reqKeyNew(); savePrefs(); }
    const url = await invoke("widget_start", { port: Number(prefs.widgetPort) || 7878, remote: !!prefs.botRemote && alertsOn(), key: prefs.alertKey || "" });
    $("widgetUrl").value = url;
    alertsPaint();   // its web request address goes by the same port
    $("widgetNote").textContent = WIDGET_NOTE;
    widgetPush();
  } catch (e) {
    $("widgetUrl").value = "";
    $("widgetNote").textContent = String(e);
  }
}

// Animation options that only some designs use (the rest, fades, scrolling and colour speed, apply
// to every design). Options for other designs are hidden.
const WIDGET_ANIM_DESIGNS = { wSpin: ["vinyl"] };

function paintWidgetAnims() {
  const d = prefs.widgetDesign || "card";
  Object.entries(WIDGET_ANIM_DESIGNS).forEach(([id, designs]) => { $(id).closest(".row").hidden = !designs.includes(d); });
}

function widgetInit() {
  $("widgetToggle").checked = !!prefs.widget;
  $("widgetPort").value = prefs.widgetPort || 7878;
  $("widgetDesign").value = prefs.widgetDesign || "card";
  $("widgetRounded").checked = prefs.widgetRounded !== false;
  // Alignment: left or right
  const paintAlign = () => document.querySelectorAll("#widgetAlignSeg button").forEach((b) => b.classList.toggle("on", b.dataset.v === (prefs.widgetAlign === "right" ? "right" : "left")));
  $("widgetAlignSeg").addEventListener("click", (e) => {
    const b = e.target.closest("button");
    if (!b) return;
    prefs.widgetAlign = b.dataset.v;
    savePrefs();
    paintAlign();
    widgetPush();
  });
  paintAlign();
  $("widgetDesign").addEventListener("change", () => {
    prefs.widgetDesign = $("widgetDesign").value;
    savePrefs();
    paintWidgetAnims();
    widgetPush();
  });
  $("widgetRounded").addEventListener("change", () => {
    prefs.widgetRounded = $("widgetRounded").checked;
    savePrefs();
    widgetPush();
  });
  $("widgetToggle").addEventListener("change", () => {
    prefs.widget = $("widgetToggle").checked;
    savePrefs();
    widgetApply();
  });
  $("widgetPort").addEventListener("change", () => {
    const p = Math.round(Number($("widgetPort").value));
    if (p >= 1024 && p <= 65535) { prefs.widgetPort = p; savePrefs(); widgetApply(); }
    else $("widgetPort").value = prefs.widgetPort || 7878;
  });
  $("widgetClear").addEventListener("click", () => {
    widgetCleared = true;
    widgetPush();
  });
  $("widgetCopy").addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText($("widgetUrl").value);
      $("widgetCopy").textContent = "Copied";
      setTimeout(() => { $("widgetCopy").textContent = "Copy"; }, 1500);
    } catch (e) {
      $("widgetUrl").select();
    }
  });
  audio.addEventListener("seeked", widgetPush);   // Spotify seeks push from spotifySeek
  setInterval(() => { if ((prefs.widget || prefs.req) && nowPlaying()) widgetPush(); }, 5000);
  const paintText = () => { $("widgetTextVal").textContent = Math.round((prefs.widgetText || 1) * 100) + "%"; };
  const stepText = (d) => {
    prefs.widgetText = Math.min(1.6, Math.max(0.7, Math.round(((prefs.widgetText || 1) + d) * 10) / 10));
    savePrefs();
    paintText();
    widgetPush();
  };
  $("widgetTextDown").addEventListener("click", () => stepText(-0.1));
  $("widgetTextUp").addEventListener("click", () => stepText(0.1));
  paintText();

  // Animation options
  paintWidgetAnims();
  $("wFade").checked = prefs.wFade !== false;
  $("wScroll").checked = prefs.wScroll !== false;
  $("wSpin").checked = prefs.wSpin !== false;
  $("wRequester").checked = prefs.wRequester !== false;
  $("wColor").value = String(prefs.wColor === undefined ? 1.2 : prefs.wColor);
  ["wFade", "wScroll", "wSpin", "wRequester"].forEach((key) =>
    $(key).addEventListener("change", () => { prefs[key] = $(key).checked; savePrefs(); widgetPush(); }));
  $("wColor").addEventListener("change", () => { prefs.wColor = Number($("wColor").value); savePrefs(); widgetPush(); });

  alertsInit();
  widgetReady = true;
  widgetApply();
}

// ---------- stream alerts, shown by the widget (crates/core/widget/widget.html) ----------
// From Streamer.bot (the widget connects to its WebSocket server) or web requests to /alert (Mix It
// Up's Web Request action, Streamer.bot's Fetch URL...). Each kind can be off, have a sound, and be
// tried with Test; dragging sets which goes first when several are waiting. The pencil by a name
// opens its own settings: its name and words, colour, length, animation, icon or picture. + adds a
// custom alert, set off by a web request with its trigger name.
//   [id, name in Settings, test values, heading, main line, icon]
const ALERT_KINDS = [
  ["follow", "Follows", { user: "A new friend" }, "New Follower", "{user}"],
  ["sub", "Subscriptions", { user: "A loyal viewer", amount: "6", message: "Six months already!" }, "New Subscriber", "{user}"],
  ["gift", "Gifted Subs", { user: "A generous viewer", amount: "5" }, "Gifted Subs", "{user} gifted {amount} subs"],
  ["bits", "Bits", { user: "A cheerful viewer", amount: "500", message: "gg!" }, "{amount} Bits", "{user}"],
  ["kicks", "Kicks", { user: "A kind viewer", amount: "100", message: "Love it" }, "{amount} Kicks", "{user}"],
  ["donation", "Donations", { user: "A kind viewer", amount: "$10.00", message: "Love the stream" }, "Donation", "{user} · {amount}"],
  ["raid", "Raids & Hosts", { user: "A fellow streamer", amount: "42" }, "{amount} Raiders", "{user} is here!"],
  ["hypetrain", "Hype Trains", { amount: "3" }, "Hype Train", "Level {amount}"],
  ["poll", "Polls", { user: "Which track next?", message: "Vote now!" }, "Poll", "{user}"],
  ["ban", "Bans", { user: "A troll" }, "Banned", "{user}"],
  ["watchstreak", "Watch Streaks", { user: "A regular", amount: "10" }, "Watch Streak", "{user} · {amount} streams"],
];
const ALERT_ICON_NAMES = [
  ["follow", "Heart"], ["sub", "Star"], ["gift", "Gift"], ["bits", "Gem"], ["kicks", "Lightning"], ["donation", "Money"],
  ["raid", "People"], ["hypetrain", "Train"], ["poll", "Chart"], ["ban", "No Entry"], ["watchstreak", "Flame"],
  ["bell", "Bell"], ["trophy", "Trophy"], ["hammer", "Hammer"], ["rocket", "Rocket"], ["music", "Music"], ["sparkle", "Sparkles"],
];
const ALERT_SOUND_EXTS = ["mp3", "wav", "ogg", "m4a", "aac", "flac"];
const ALERT_IMAGE_EXTS = ["gif", "png", "webp", "jpg", "jpeg"];
const ALERT_SECS = [3, 5, 8, 12, 20];
const ALERT_ANIMS = [["slide", "Slide"], ["pop", "Pop"], ["fade", "Fade"], ["bounce", "Bounce"], ["flip", "Flip"]];

const alertsOn = () => !!prefs.alertsOn;
const alertBuiltin = (k) => ALERT_KINDS.find((x) => x[0] === k);
const alertCustom = () => (prefs.alertCustom || []).filter((c) => c && c.id);
const alertEdits = (k) => (prefs.alertEdit || {})[k] || {};
function alertName(k) {
  const b = alertBuiltin(k), c = alertCustom().find((x) => x.id === k);
  return alertEdits(k).name || (b ? b[1] : c ? c.name : k);
}
function alertOrder() {
  const all = ALERT_KINDS.map((k) => k[0]).concat(alertCustom().map((c) => c.id));
  const saved = (prefs.alertOrder || []).filter((k) => all.includes(k));
  return saved.concat(all.filter((k) => !saved.includes(k)));
}

// What the widget needs: the switched-on kinds in priority order, how each looks, and their sounds
function alertsCfg() {
  if (!alertsOn()) return { on: false };
  const off = new Set(prefs.alertOff || []);
  const sounds = {}, kinds = {};
  Object.entries(prefs.alertSounds || {}).forEach(([k, v]) => { if (v && v.at) sounds[k] = String(v.at); });
  alertOrder().forEach((k) => {
    const e = alertEdits(k), img = (prefs.alertImages || {})[k];
    const c = {};
    ["head", "main", "color", "anim", "icon", "crop", "cicons", "cstyle", "music"].forEach((f) => { if (e[f]) c[f] = e[f]; });
    if (e.color && e.color2) c.color2 = e.color2;
    if (e.cpal) c.colors = collageColours(k);
    if (e.secs) c.secs = e.secs === "sound" ? "sound" : Number(e.secs);
    if (e.sdelay) c.sdelay = Number(e.sdelay);
    if (img && img.at) c.image = String(img.at);
    // its collage pictures (by place, so a gap stays a gap), and its sound's level and length
    const cp = (prefs.alertCollage || {})[k];
    if (cp && cp.some(Boolean)) c.cpics = cp.map((p) => (p ? String(p.at) : ""));
    const lv = (prefs.alertLevels || {})[k];
    if (lv && (prefs.alertSounds || {})[k]) { c.gain = alertGainDb(k); c.sndMs = lv.ms; }
    if (!alertBuiltin(k)) c.name = alertName(k);
    if (Object.keys(c).length) kinds[k] = c;
  });
  return {
    on: true,
    order: alertOrder().filter((k) => !off.has(k)),
    custom: alertCustom().map((c) => c.id),
    kinds,
    anim: prefs.alertAnim || "slide",
    transition: prefs.alertTrans || "seamless",
    collage: { size: prefs.alertCSize || "medium", dir: prefs.alertCDir || "diag", angle: Number(prefs.alertCAngle) || 0 },
    secs: Number(prefs.alertSecs) || 5,
    // their own colours (else the widget's, from the album art), and which way the gradient runs
    accent: prefs.alertOwnCol ? prefs.alertCol1 || "#8e9bff" : "",
    accent2: prefs.alertOwnCol ? prefs.alertCol2 || prefs.alertCol1 || "#b38cff" : "",
    dir: prefs.alertDir || "115deg",
    icon_scale: alertIconScale(),
    volume: prefs.alertVol === undefined ? 0.8 : Number(prefs.alertVol),
    sounds,
    sb: { on: prefs.sbOn !== false, host: prefs.sbHost || "127.0.0.1:8080" },
  };
}

// ---------- the collage's tile colours: each alert's own, a preset, or your own (up to six) ----------
const COLLAGE_PALETTES = [
  ["alert", "Alert's Colours", []],
  ["rainbow", "Rainbow", ["#ff4d4d", "#ff9f1c", "#ffe74c", "#3ddc84", "#3a86ff", "#8338ec"]],
  ["neon", "Neon", ["#ff00e5", "#00f0ff", "#39ff14", "#fff000", "#ff3131"]],
  ["sunset", "Sunset", ["#ff6b6b", "#ff9e57", "#ffd166", "#c94b7d", "#6a4c93"]],
  ["ocean", "Ocean", ["#0077b6", "#00b4d8", "#48cae4", "#90e0ef", "#023e8a"]],
  ["forest", "Forest", ["#2d6a4f", "#40916c", "#74c69d", "#b7e4c7", "#95d5b2"]],
  ["candy", "Candy", ["#ff8fab", "#ffc2d1", "#cdb4db", "#a2d2ff", "#bde0fe"]],
  ["fire", "Fire", ["#d00000", "#e85d04", "#faa307", "#ffba08", "#9d0208"]],
  ["streamer", "Streaming Sites", ["#ff0033", "#9146ff", "#53fc18", "#ff5500", "#1ed760"]],
  ["mono", "Black & White", ["#f5f5f5", "#bdbdbd", "#757575", "#424242"]],
  ["custom", "Your Own", null],
];
// An alert's palette (Settings > the alert > Collage > Tile Colours): its id and your own colours
const collagePalette = (kind) => alertEdits(kind).cpal || "alert";
const collageCustom = (kind) => (Array.isArray(alertEdits(kind).cpalOwn) && alertEdits(kind).cpalOwn.length ? alertEdits(kind).cpalOwn : ["#ff4d6d", "#ffb703", "#4cc9f0"]).slice(0, 6);
function collageColours(kind) {
  const p = COLLAGE_PALETTES.find((x) => x[0] === collagePalette(kind)) || COLLAGE_PALETTES[0];
  return p[0] === "custom" ? collageCustom(kind) : p[2];
}

// The picker, in the alert's settings: a swatch each, and + / x for your own
function collagePaletteEditor(kind) {
  const wrap = el("div", "alertCollagePal");
  const sel = el("select", "narrow");
  COLLAGE_PALETTES.forEach(([id, name]) => { const o = el("option", "", name); o.value = id; sel.append(o); });
  sel.value = collagePalette(kind);
  sel.addEventListener("change", () => { alertEditSet(kind, "cpal", sel.value === "alert" ? "" : sel.value); alertsPaintList(); });
  const own = collagePalette(kind) === "custom";
  const box = el("div", "alertInline");
  collageColours(kind).forEach((c, i) => {
    if (!own) { const sw = el("span", "palSwatch"); sw.style.background = c; box.append(sw); return; }
    const inp = el("input");
    inp.type = "color";
    inp.value = c;
    inp.setAttribute("aria-label", "Colour " + (i + 1));
    inp.addEventListener("change", () => { const l = collageCustom(kind).slice(); l[i] = inp.value; alertEditSet(kind, "cpalOwn", l); });
    box.append(inp);
    if (collageCustom(kind).length > 2) {
      const x = el("button", "soft palRemove", "\u00d7");
      x.type = "button";
      x.title = "Take this colour out";
      x.addEventListener("click", () => { const l = collageCustom(kind).slice(); l.splice(i, 1); alertEditSet(kind, "cpalOwn", l); alertsPaintList(); });
      box.append(x);
    }
  });
  if (own && collageCustom(kind).length < 6) {
    const add = el("button", "soft", "+ Colour");
    add.type = "button";
    add.addEventListener("click", () => { alertEditSet(kind, "cpalOwn", collageCustom(kind).concat(["#ffffff"])); alertsPaintList(); });
    box.append(add);
  }
  const top = el("div", "alertInline");
  top.append(el("span", "note", "Tile Colours"), sel);
  wrap.append(top);
  if (collagePalette(kind) !== "alert") wrap.append(box);
  wrap.append(el("div", "note", collagePalette(kind) === "alert" ? "Its own two colours." : "Its tiles in these colours. Queued after another alert, they blend in from that one's."));
  return wrap;
}

// The icons' size: 60% to 200% of how they started (200% fills the tile), in steps of 10%
const alertIconScale = () => Math.min(2, Math.max(0.6, Number(prefs.alertIcon) || 1));

function alertsSave() {
  savePrefs();
  widgetPush();
}

function alertEditSet(kind, field, value) {
  const all = Object.assign({}, prefs.alertEdit);
  const e = Object.assign({}, all[kind]);
  if (value === "" || value === null || value === undefined) delete e[field]; else e[field] = value;
  all[kind] = e;
  prefs.alertEdit = all;
  alertsSave();
}

let alertOpen = null;   // the alert whose settings are open

function alertRow(kind) {
  const off = new Set(prefs.alertOff || []);
  const snd = (prefs.alertSounds || {})[kind];
  const li = el("li", "alertRow" + (off.has(kind) ? " alertIsOff" : ""));
  li.draggable = true;
  li.dataset.kind = kind;
  const grip = el("span", "alertGrip", "☰");
  grip.title = "Drag to reorder";
  const label = el("span", "alertName");
  label.append(el("span", "", alertName(kind)));
  const pen = el("button", "alertPen");
  pen.title = "Edit this alert";
  pen.setAttribute("aria-label", "Edit " + alertName(kind));
  pen.append(icon("M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a1 1 0 0 0 0-1.41l-2.34-2.34a1 1 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z"));
  pen.addEventListener("click", () => { alertOpen = alertOpen === kind ? null : kind; alertsPaintList(); });
  label.append(pen);
  const sound = el("button", "soft alertSound", snd ? snd.name : "Add Sound");
  sound.title = snd ? "Change the sound" : "Pick a sound to play with this alert";
  sound.addEventListener("click", () => alertPickMedia(kind, "sound"));
  const parts = [grip, label, sound];
  if (snd) {
    const clear = el("button", "soft alertClear", "×");
    clear.title = "No sound";
    clear.setAttribute("aria-label", "Remove the sound");
    clear.addEventListener("click", () => alertClearMedia(kind, "sound"));
    parts.push(clear);
  }
  const test = el("button", "soft", "Test");
  test.title = "Show this alert on the widget now";
  test.addEventListener("click", () => alertTest(kind));
  const sw = el("label", "switch");
  const box = el("input");
  box.type = "checkbox";
  box.checked = !off.has(kind);
  box.setAttribute("aria-label", alertName(kind));
  box.addEventListener("change", () => {
    const s = new Set(prefs.alertOff || []);
    if (box.checked) s.delete(kind); else s.add(kind);
    prefs.alertOff = [...s];
    li.classList.toggle("alertIsOff", !box.checked);
    alertsSave();
  });
  sw.append(box, el("span"));
  parts.push(test, sw);
  li.append(...parts);
  return li;
}

// ---------- one alert's own settings, under its row ----------
function alertEditor(kind) {
  const b = alertBuiltin(kind);
  const custom = !b;
  const e = alertEdits(kind);
  const box = el("li", "alertEdit");
  box.dataset.for = kind;
  const field = (label, input, note) => {
    const r = el("div", "alertField");
    r.append(el("label", "", label), input);
    if (note) r.append(el("div", "note", note));
    return r;
  };
  const text = (value, placeholder, onChange) => {
    const i = el("input");
    i.type = "text";
    i.value = value || "";
    i.placeholder = placeholder || "";
    i.spellcheck = false;
    i.addEventListener("change", () => onChange(i.value.trim()));
    return i;
  };
  const select = (opts, value, onChange) => {
    const s = el("select", "narrow");
    opts.forEach(([v, n]) => { const o = el("option", "", n); o.value = v; s.append(o); });
    s.value = value;
    s.addEventListener("change", () => onChange(s.value));
    return s;
  };
  const defName = b ? b[1] : (alertCustom().find((c) => c.id === kind) || {}).name || "";
  box.append(field("Name", text(custom ? alertName(kind) : e.name, defName, (v) => {
    if (custom) {
      prefs.alertCustom = alertCustom().map((c) => (c.id === kind ? Object.assign({}, c, { name: v || c.name }) : c));
      alertsSave();
    } else alertEditSet(kind, "name", v);
    alertsPaintList();
  })));
  if (custom) {
    const url = ($("alertUrl").value || "").replace(/type=[^&]*/, "type=" + kind);
    const trig = text(kind, "", (v) => alertRename(kind, v));
    box.append(field("Trigger", trig, "Its web request: " + url));
  }
  box.append(
    field("Heading", text(e.head, b ? b[3] : alertName(kind), (v) => alertEditSet(kind, "head", v))),
    field("Main Line", text(e.main, b ? b[4] : "{user}", (v) => alertEditSet(kind, "main", v)), "{user}, {amount} and {message} are filled in. Empty: as shown faintly."),
  );
  // colour: its own, or the widget's
  const colRow = el("div", "alertInline");
  const own = el("input");
  own.type = "checkbox";
  own.checked = !!e.color;
  const col = el("input");
  col.type = "color";
  col.value = e.color || "#8e9bff";
  col.disabled = !e.color;
  col.setAttribute("aria-label", "Its first colour");
  // a second colour for its gradient (and its collage's tiles); the same as the first unless chosen
  const col2 = el("input");
  col2.type = "color";
  col2.value = e.color2 || e.color || "#b38cff";
  col2.disabled = !e.color;
  col2.setAttribute("aria-label", "Its second colour");
  const swap = el("button", "soft alertSwap", "⇄");
  swap.title = "Swap the two colours";
  swap.disabled = !e.color;
  own.addEventListener("change", () => {
    [col, col2, swap].forEach((x) => { x.disabled = !own.checked; });
    alertEditSet(kind, "color", own.checked ? col.value : "");
    alertEditSet(kind, "color2", own.checked && col2.value !== col.value ? col2.value : "");
  });
  col.addEventListener("change", () => alertEditSet(kind, "color", col.value));
  col2.addEventListener("change", () => alertEditSet(kind, "color2", col2.value));
  swap.addEventListener("click", () => {
    const a = col.value;
    col.value = col2.value;
    col2.value = a;
    alertEditSet(kind, "color", col.value);
    alertEditSet(kind, "color2", col2.value);
  });
  const ownLbl = el("label", "alertCheck");
  ownLbl.append(own, document.createTextNode(" Its own colours"));
  colRow.append(ownLbl, col, col2, swap);
  box.append(field("Colours", colRow, "Its gradient runs from the first to the second, as set under Gradient."));
  const hasSound = !!(prefs.alertSounds || {})[kind];
  const secsOpts = [["", "As set above"]].concat(hasSound || e.secs === "sound" ? [["sound", "As long as its sound"]] : [], ALERT_SECS.map((s) => [String(s), s + " s"]));
  const animSel = select([["", "As set above"]].concat(ALERT_ANIMS), e.anim || "", (v) => alertEditSet(kind, "anim", v));
  const collageOn = (prefs.alertTrans || "seamless") === "collage";
  animSel.disabled = collageOn;
  box.append(
    field("Show For", select(secsOpts, String(e.secs || ""), (v) => alertEditSet(kind, "secs", v === "sound" ? "sound" : v ? Number(v) : ""))),
    field("Animation", animSel, collageOn ? "Not used with Collage: its tiles bring the alert in." : ""),
  );
  // the picture: an icon, or your own image or GIF
  const img = (prefs.alertImages || {})[kind];
  const picRow = el("div", "alertInline");
  if (img) {
    picRow.append(el("span", "alertFile", img.name));
    const change = el("button", "soft", "Change");
    change.addEventListener("click", () => alertPickMedia(kind, "image"));
    const rm = el("button", "soft", "Use an Icon");
    rm.addEventListener("click", () => alertClearMedia(kind, "image"));
    picRow.append(change, rm);
  } else {
    picRow.append(select(ALERT_ICON_NAMES, e.icon || (b ? kind : "bell"), (v) => alertEditSet(kind, "icon", v === (b ? kind : "bell") ? "" : v)));
    const pick = el("button", "soft", "Image or GIF...");
    pick.title = "Show your own picture (a GIF, say) in place of the icon";
    pick.addEventListener("click", () => alertPickMedia(kind, "image"));
    picRow.append(pick);
  }
  box.append(field("Picture", picRow));
  if (img) box.append(field("Crop", alertCropEditor(kind)));
  box.append(field("Collage", alertCollageEditor(kind)));
  box.append(field("Sound", alertSoundEditor(kind)));
  const foot = el("div", "alertInline alertFoot");
  if (custom) {
    const del = el("button", "soft danger", "Delete This Alert");
    del.addEventListener("click", () => alertDelete(kind));
    foot.append(del);
  } else if (Object.keys(e).length) {
    const reset = el("button", "soft", "Back to How It Was");
    reset.addEventListener("click", () => {
      const all = Object.assign({}, prefs.alertEdit);
      delete all[kind];
      prefs.alertEdit = all;
      alertsSave();
      alertsPaintList();
    });
    foot.append(reset);
  }
  const done = el("button", "soft", "Done");
  done.addEventListener("click", () => { alertOpen = null; alertsPaintList(); });
  foot.append(done);
  box.append(foot);
  return box;
}

// A trigger name: what a web request's type= says (letters, digits, - and _)
const alertSlug = (s) => String(s).toLowerCase().normalize("NFKD").replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 32);

function alertRename(kind, v) {
  const id = alertSlug(v);
  if (!id || id === kind) { alertsPaintList(); return; }
  if (alertBuiltin(id) || alertCustom().some((c) => c.id === id)) { say("\"" + id + "\" is already an alert's trigger."); alertsPaintList(); return; }
  const move = (o) => { if (o && o[kind] !== undefined) { o = Object.assign({}, o); o[id] = o[kind]; delete o[kind]; } return o; };
  prefs.alertCustom = alertCustom().map((c) => (c.id === kind ? Object.assign({}, c, { id }) : c));
  prefs.alertEdit = move(prefs.alertEdit);
  prefs.alertOrder = (prefs.alertOrder || []).map((k) => (k === kind ? id : k));
  prefs.alertOff = (prefs.alertOff || []).map((k) => (k === kind ? id : k));
  // its sound, picture and collage are files named after the trigger: those are picked again
  alertCollageDrop(kind);
  if (prefs.alertLevels && prefs.alertLevels[kind]) { prefs.alertLevels = Object.assign({}, prefs.alertLevels); delete prefs.alertLevels[kind]; }
  ["sound", "image"].forEach((slot) => {
    const key = slot === "sound" ? "alertSounds" : "alertImages";
    if (prefs[key] && prefs[key][kind]) { invoke("alert_media_clear", { kind, slot }).catch(() => {}); prefs[key] = Object.assign({}, prefs[key]); delete prefs[key][kind]; }
  });
  alertOpen = id;
  alertsSave();
  alertsPaintList();
}

function alertAdd() {
  let n = 1, id = "custom";
  while (alertBuiltin(id) || alertCustom().some((c) => c.id === id)) id = "custom-" + ++n;
  prefs.alertCustom = alertCustom().concat([{ id, name: "New Alert" + (n > 1 ? " " + n : "") }]);
  alertOpen = id;
  alertsSave();
  alertsPaintList();
  const ed = document.querySelector('#alertList .alertEdit[data-for="' + id + '"]');
  if (ed) { ed.scrollIntoView({ block: "nearest" }); const first = ed.querySelector("input"); if (first) { first.focus(); first.select(); } }
}

function alertDelete(kind) {
  prefs.alertCustom = alertCustom().filter((c) => c.id !== kind);
  alertCollageDrop(kind);
  ["alertEdit", "alertSounds", "alertImages", "alertLevels"].forEach((key) => { if (prefs[key] && prefs[key][kind]) { prefs[key] = Object.assign({}, prefs[key]); delete prefs[key][kind]; } });
  prefs.alertOrder = (prefs.alertOrder || []).filter((k) => k !== kind);
  prefs.alertOff = (prefs.alertOff || []).filter((k) => k !== kind);
  ["sound", "image"].forEach((slot) => invoke("alert_media_clear", { kind, slot }).catch(() => {}));
  alertOpen = null;
  alertsSave();
  alertsPaintList();
}

function alertsPaintList() {
  const items = [];
  alertOrder().forEach((k) => {
    items.push(alertRow(k));
    if (alertOpen === k) items.push(alertEditor(k));
  });
  $("alertList").replaceChildren(...items);
}

async function alertPickMedia(kind, slot) {
  const dlg = window.__TAURI__ && window.__TAURI__.dialog;
  if (!dlg) { say("Picking a file needs the app."); return; }
  const image = slot === "image";
  try {
    const path = await dlg.open({ multiple: false, title: image ? "Choose a picture or GIF for this alert" : "Choose a sound for this alert",
      filters: [{ name: image ? "Pictures" : "Sounds", extensions: image ? ALERT_IMAGE_EXTS : ALERT_SOUND_EXTS }] });
    if (!path) return;
    const name = await invoke("alert_media_set", { kind, slot, path: String(path) });
    const key = image ? "alertImages" : "alertSounds";
    prefs[key] = Object.assign({}, prefs[key], { [kind]: { name, at: Date.now() } });
    // a new picture starts uncropped; a new sound is measured, so it plays as loud as the others
    if (image) alertEditSet(kind, "crop", "");
    alertsSave();
    if (!image) { await alertAnalyse(kind, false); alertOpen = kind; }
    alertsPaintList();
  } catch (e) {
    say(String(e && e.message ? e.message : e));
  }
}

async function alertClearMedia(kind, slot) {
  await invoke("alert_media_clear", { kind, slot }).catch(() => {});
  const key = slot === "image" ? "alertImages" : "alertSounds";
  if (prefs[key]) { prefs[key] = Object.assign({}, prefs[key]); delete prefs[key][kind]; }
  if (slot === "sound") alertLevelSet(kind, null);
  alertsSave();
  alertsPaintList();
}

async function alertTest(kind) {
  if (!prefs.widget) { say("Switch on the widget first."); return; }
  const b = alertBuiltin(kind);
  const sample = b ? b[2] : { user: "Someone", amount: "1", message: "" };
  widgetPush();   // the widget has this alert's newest settings first
  try {
    await invoke("widget_alert", { alert: Object.assign({ type: kind, user: "", amount: "", message: "" }, sample, { test: true }) });
    say("Sent a test alert to the widget.");
  } catch (e) {
    say(String(e));
  }
}

// Dragging a row to reorder them (an open editor closes first, so it doesn't get left behind)
function alertsDrag() {
  const list = $("alertList");
  let dragging = null;
  list.addEventListener("dragstart", (e) => {
    dragging = e.target.closest && e.target.closest(".alertRow");
    if (!dragging || e.target.closest("input, select, button")) { dragging = null; return; }
    if (alertOpen) { const ed = list.querySelector(".alertEdit"); if (ed) ed.remove(); alertOpen = null; }
    dragging.classList.add("dragging");
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", dragging.dataset.kind);
  });
  list.addEventListener("dragover", (e) => {
    if (!dragging) return;
    e.preventDefault();
    const rows = [...list.querySelectorAll(".alertRow:not(.dragging)")];
    const after = rows.find((r) => { const b = r.getBoundingClientRect(); return e.clientY < b.top + b.height / 2; });
    if (after) list.insertBefore(dragging, after); else list.append(dragging);
  });
  const done = () => {
    if (!dragging) return;
    dragging.classList.remove("dragging");
    dragging = null;
    prefs.alertOrder = [...list.querySelectorAll(".alertRow")].map((r) => r.dataset.kind);
    alertsSave();
  };
  list.addEventListener("drop", (e) => { e.preventDefault(); done(); });
  list.addEventListener("dragend", done);
}

// ---------- bots that aren't on this PC (Generic) ----------
// Off unless switched on: then the alerts and song request servers listen to other devices too
// (alerts need a key from them; requests always do). An online bot needs the port forwarded on your
// router, and your public IP address, which is yours to keep to yourself.
let lanIp = "";
async function botRemoteSet(on) {
  prefs.botRemote = on;
  if (on && !prefs.alertKey) prefs.alertKey = reqKeyNew() + reqKeyNew();
  savePrefs();
  await widgetApply();
  if (typeof reqApply === "function") await reqApply();
  document.querySelectorAll(".botRemoteOn").forEach((b) => { b.checked = on; });
  alertsPaint();
  if (typeof reqRenderBot === "function") reqRenderBot();
}

// The part of a bot's setup about bots elsewhere: what, how, and a word of warning. what: "alerts" or "requests"
function botRemoteBlock(what, port, path) {
  const box = el("div", "botRemote");
  const row = el("div", "row");
  const txt = el("div");
  txt.append(el("div", "lbl", "Bots on Other Devices"), el("div", "note", "Off: only programs on this PC can reach these addresses. On: other devices can too" + (what === "alerts" ? " (with the key in their address)." : " (with the key).")));
  const sw = el("label", "switch");
  const cb = el("input", "botRemoteOn");
  cb.type = "checkbox";
  cb.checked = !!prefs.botRemote;
  cb.addEventListener("change", () => botRemoteSet(cb.checked));
  sw.append(cb, el("span"));
  row.append(txt, sw);
  box.append(row);
  if (prefs.botRemote) {
    if (!lanIp) invoke("lan_ip").then((ip) => { if (ip) { lanIp = ip; alertsPaint(); if (typeof reqRenderBot === "function") reqRenderBot(); } }).catch(() => {});
    const ip = lanIp || "THIS-PC";
    const l = el("ol");
    const li = (...parts) => { const x = el("li"); parts.forEach((p) => x.append(typeof p === "string" ? document.createTextNode(p) : p)); return x; };
    l.append(
      li("A bot on another computer at home: give it ", el("code", "", "http://" + ip + ":" + port + path), " in place of 127.0.0.1."),
      li("An online bot (one that isn't on your network): on your router, forward port ", el("code", "", String(port)), " (TCP) to this PC, ", el("code", "", ip), ". Then give the bot the same address with your public IP address in place of ", el("code", "", ip), "."),
    );
    box.append(l);
    box.append(el("p", "note warn", "Careful: your public IP address points to your home. Don't show it on stream or in screenshots, and don't share it; keep Streamer Mode on while you set this up. With it and the key, anyone could " + (what === "alerts" ? "send alerts to your stream." : "request songs.")));
  }
  return box;
}

function alertsPaint() {
  $("alertsOn").checked = alertsOn();
  $("alertsBody").hidden = !alertsOn();
  $("sbOn").checked = prefs.sbOn !== false;
  $("sbHost").value = prefs.sbHost || "127.0.0.1:8080";
  $("alertAnim").value = prefs.alertAnim || "slide";
  $("alertTrans").value = prefs.alertTrans || "seamless";
  const collage = (prefs.alertTrans || "seamless") === "collage";
  $("alertCollageBox").hidden = !collage;
  $("alertAnim").disabled = collage;
  $("alertAnimNote").textContent = collage ? "Not used with Collage: its tiles bring each alert in, even with nothing playing." : "With Over the Top, and while nothing's playing.";
  $("alertCSize").value = prefs.alertCSize || "medium";
  $("alertCDir").value = prefs.alertCDir || "diag";
  $("alertCAngle").value = String(Number(prefs.alertCAngle) || 0);
  $("alertCAngleVal").textContent = $("alertCAngle").value + "\u00b0";
  $("alertSecs").value = String(Number(prefs.alertSecs) || 5);
  $("alertIconVal").textContent = Math.round(alertIconScale() * 100) + "%";
  $("alertIconDown").disabled = alertIconScale() <= 0.6;
  $("alertIconUp").disabled = alertIconScale() >= 2;
  $("alertOwnCol").checked = !!prefs.alertOwnCol;
  $("alertColRow").hidden = !prefs.alertOwnCol;
  $("alertCol1").value = prefs.alertCol1 || "#8e9bff";
  $("alertCol2").value = prefs.alertCol2 || "#b38cff";
  $("alertDir").value = prefs.alertDir || "115deg";
  const v = prefs.alertVol === undefined ? 0.8 : Number(prefs.alertVol);
  $("alertVol").value = String(Math.round(v * 100));
  $("alertVolVal").textContent = Math.round(v * 100) + "%";
  const port = Number(prefs.widgetPort) || 7878;
  const bot = prefs.alertBot || (prefs.reqBot === "mixitup" ? "mixitup" : prefs.reqBot === "other" ? "generic" : "streamerbot");
  $("alertBot").value = bot;
  $("sbRow").hidden = bot !== "streamerbot";
  // (Mix It Up: an address for each of its events, below, rather than one to change for each)
  $("alertWebRow").hidden = bot === "streamerbot" || bot === "mixitup";
  const vars = bot === "mixitup" ? "type=follow&user=$username" : "type=follow&user=NAME&amount=&message=";
  const keyPart = prefs.botRemote && prefs.alertKey ? "key=" + prefs.alertKey + "&" : "";
  $("alertUrl").value = "http://127.0.0.1:" + port + "/alert?" + keyPart + vars;
  $("alertWebNote").textContent = bot === "mixitup" ? "In Mix It Up, its Web Request action, with this address." : "Your bot opens this address when something happens.";
  const help = $("alertBotHelp");
  help.replaceChildren();
  const code = (s) => el("code", "", s);
  const li = (...parts) => { const x = el("li"); parts.forEach((p) => x.append(typeof p === "string" ? document.createTextNode(p) : p)); return x; };
  const ol = el("ol");
  const TYPES = "follow, sub, gift, bits, kicks, donation, raid, hypetrain, poll, ban, watchstreak";
  if (bot === "streamerbot") {
    ol.append(
      li("In Streamer.bot: Servers/Clients > WebSocket Server. Tick Auto Start, then Start Server."),
      li("Put its address and port here (", code("127.0.0.1:8080"), " unless you've changed it), and switch it on."),
      li("That's all: follows, subs, bits, raids and the rest come through by themselves."),
    );
    help.append(ol);
  } else if (bot === "mixitup") {
    // one address for each event, ready to paste: its type and Mix It Up's own special identifiers in
    help.append(el("p", "note", "In Mix It Up, open each event below, add a Web Request action, and paste its address. Nothing in it needs changing."));
    const base = "http://127.0.0.1:" + port + "/alert?" + keyPart + "type=";
    const rows = MIXITUP_EVENTS.map(([event, rest]) => [event, base + rest, ""]);
    alertCustom().forEach((c) => rows.push([(c.name || c.id) + " (yours)", base + c.id + "&user=$username", "Any event or command you like."]));
    rows.forEach(([event, url, note]) => {
      const row = el("div", "row stack");
      const head = el("div");
      head.append(el("div", "lbl", event));
      if (note) head.append(el("div", "note", note));
      const box = el("div", "widgetRow");
      const input = el("input", "sensitive");
      input.type = "text"; input.readOnly = true; input.value = url; input.setAttribute("aria-label", event + " address");
      const copy = el("button", "soft", "Copy");
      copy.addEventListener("click", async () => {
        try { await navigator.clipboard.writeText(url); copy.textContent = "Copied"; setTimeout(() => { copy.textContent = "Copy"; }, 1500); }
        catch (e) { input.select(); }
      });
      box.append(input, copy);
      row.append(head, box);
      help.append(row);
    });
    help.append(el("p", "note", "Polls, bans and watch streaks have no Mix It Up event to send them from."));
  } else {
    ol.append(
      li("When something happens, have your bot open the address above."),
      li("Swap ", code("NAME"), " for your bot's name for the viewer, and fill in ", code("amount"), " and ", code("message"), " if it has them (or leave them empty)."),
      li(code("type"), " says what happened: ", code(TYPES), ", or one of your own alerts' triggers."),
    );
    help.append(ol, botRemoteBlock("alerts", port, "/alert?" + keyPart + vars));
  }
  markSensitive(help);
  if (typeof stateNotes === "function") stateNotes();
}

// Mix It Up's events, each with its alert's type and Mix It Up's documented special identifiers for it
const MIXITUP_EVENTS = [
  ["Channel Followed", "follow&user=$username"],
  ["Channel Subscribed", "sub&user=$username&amount=$usersubdurationmonths"],
  ["Channel Resubscribed", "sub&user=$username&amount=$usersubmonths&message=$message"],
  ["Mass Subscriptions Gifted", "gift&user=$username&amount=$subsgiftedamount"],
  ["Bits Cheered", "bits&user=$username&amount=$bitsamount&message=$messagenocheermotes"],
  ["Donation Received", "donation&user=$username&amount=$donationamount&message=$donationmessage"],
  ["Channel Raided", "raid&user=$username&amount=$raidviewercount"],
  ["Hype Train Level Up", "hypetrain&amount=$hypetrainlevel"],
];

function alertsInit() {
  $("alertsOn").addEventListener("change", () => { prefs.alertsOn = $("alertsOn").checked; alertsSave(); alertsPaint(); });
  $("sbOn").addEventListener("change", () => { prefs.sbOn = $("sbOn").checked; alertsSave(); });
  $("sbHost").addEventListener("change", () => {
    prefs.sbHost = $("sbHost").value.trim().replace(/^wss?:\/\//, "").replace(/\/+$/, "") || "127.0.0.1:8080";
    $("sbHost").value = prefs.sbHost;
    alertsSave();
  });
  $("alertAnim").addEventListener("change", () => { prefs.alertAnim = $("alertAnim").value; alertsSave(); });
  $("alertBot").addEventListener("change", () => { prefs.alertBot = $("alertBot").value; savePrefs(); alertsPaint(); });
  $("alertTrans").addEventListener("change", () => { prefs.alertTrans = $("alertTrans").value; alertsSave(); alertsPaint(); alertsPaintList(); });
  ["alertCSize", "alertCDir"].forEach((id) => $(id).addEventListener("change", () => { prefs[id] = $(id).value; alertsSave(); }));
  $("alertCAngle").addEventListener("input", () => { $("alertCAngleVal").textContent = $("alertCAngle").value + "\u00b0"; });
  $("alertCAngle").addEventListener("change", () => { prefs.alertCAngle = Number($("alertCAngle").value); delete prefs.alertCRot; delete prefs.alertCSpin; alertsSave(); });
  $("alertSecs").addEventListener("change", () => { prefs.alertSecs = Number($("alertSecs").value); alertsSave(); });
  const stepIcon = (d) => { prefs.alertIcon = Math.round(Math.min(2, Math.max(0.6, alertIconScale() + d)) * 10) / 10; alertsSave(); alertsPaint(); };
  $("alertIconDown").addEventListener("click", () => stepIcon(-0.1));
  $("alertIconUp").addEventListener("click", () => stepIcon(0.1));
  $("alertOwnCol").addEventListener("change", () => { prefs.alertOwnCol = $("alertOwnCol").checked; alertsSave(); alertsPaint(); });
  $("alertCol1").addEventListener("change", () => { prefs.alertCol1 = $("alertCol1").value; alertsSave(); });
  $("alertCol2").addEventListener("change", () => { prefs.alertCol2 = $("alertCol2").value; alertsSave(); });
  $("alertDir").addEventListener("change", () => { prefs.alertDir = $("alertDir").value; alertsSave(); });
  $("alertVol").addEventListener("input", () => {
    prefs.alertVol = Number($("alertVol").value) / 100;
    $("alertVolVal").textContent = $("alertVol").value + "%";
  });
  $("alertVol").addEventListener("change", alertsSave);
  $("alertCopy").addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText($("alertUrl").value);
      $("alertCopy").textContent = "Copied";
      setTimeout(() => { $("alertCopy").textContent = "Copy"; }, 1500);
    } catch (e) {
      $("alertUrl").select();
    }
  });
  $("alertAdd").addEventListener("click", alertAdd);
  // the section folds away, and stays as you left it
  $("alertsBody").open = prefs.alertsOpen !== false;
  $("alertsBody").addEventListener("toggle", () => { prefs.alertsOpen = $("alertsBody").open; savePrefs(); });
  alertsDrag();
  alertsPaint();
  alertsPaintList();
}
