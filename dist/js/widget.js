// The now-playing widget for OBS.

// ---------- now playing widget for OBS ----------
const WIDGET_NOTE = "A now-playing card for OBS.";

let widgetReady = false;
let widgetCleared = false;   // "Clear" hides the card until the next track starts

function widgetState() {
  const t = curTrack();
  if (!t || widgetCleared) return { playing: false };
  return {
    playing: nowPlaying(),
    title: t.title || "",
    artist: artistName(t),
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
  castPush();
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
    const url = await invoke("widget_start", { port: Number(prefs.widgetPort) || 7878 });
    $("widgetUrl").value = url;
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
  $("wColor").value = String(prefs.wColor === undefined ? 1.2 : prefs.wColor);
  ["wFade", "wScroll", "wSpin"].forEach((key) =>
    $(key).addEventListener("change", () => { prefs[key] = $(key).checked; savePrefs(); widgetPush(); }));
  $("wColor").addEventListener("change", () => { prefs.wColor = Number($("wColor").value); savePrefs(); widgetPush(); });

  widgetReady = true;
  widgetApply();
}
