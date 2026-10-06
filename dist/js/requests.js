// Chat song requests.

// ---------- chat song requests ----------
function reqKeyNew() {
  const a = new Uint8Array(12);
  crypto.getRandomValues(a);
  return Array.from(a, (b) => b.toString(16).padStart(2, "0")).join("");
}

const reqPendingList = () => queue.slice(qi + 1).filter((t) => t.requestedBy);

function reqPaint() {
  const rp = curPlat("search");
  $("reqPlat").textContent = "Requests are going to: " + (rp === "none" ? "nowhere (no platform is switched on)" : PLATFORMS[rp].name) +
    ". Change it with the platform icons in the title bar.";
  $("reqMaxVal").textContent = prefs.reqMax || 10;
  $("reqUserVal").textContent = prefs.reqPerUser || 2;
  const p = reqPendingList();
  $("reqPending").textContent = p.length
    ? p.map((t) => '"' + t.title + '" for ' + t.requestedBy).join(", ")
    : "None.";
}

async function reqApply() {
  const on = !!prefs.req;
  $("reqOn").checked = on;
  $("requestSection").hidden = !on;
  if (!on) {
    invoke("requests_stop").catch(() => {});
    return;
  }
  if (!prefs.reqKey) { prefs.reqKey = reqKeyNew(); savePrefs(); }
  const port = Number(prefs.reqPort) || 7879;
  $("reqPort").value = port;
  try {
    await invoke("requests_start", { port, key: prefs.reqKey });
    const base = "http://127.0.0.1:" + port;
    $("reqUrl").value = base + "/request?key=" + prefs.reqKey + "&user=USER&q=SONG";
    $("reqNowUrl").value = base + "/now";
    $("reqNote").textContent = "";
    reqRenderBot();
    widgetPush();
  } catch (e) {
    $("reqNote").textContent = String(e);
  }
  reqPaint();
}

// Called from the Rust side for each request; it answers with request_result
window.__songRequest = async (id, q, user) => {
  let result;
  try {
    result = await handleSongRequest(String(q || "").trim(), String(user || "").trim());
  } catch (e) {
    result = { ok: false, message: "Couldn't add that one: " + e };
  }
  invoke("request_result", { id, result }).catch(() => {});
};

async function findRequested(q) {
  if (/^https?:\/\/(www\.|m\.)?soundcloud\.com\//i.test(q)) {
    try {
      const found = asList([await api("/resolve?url=" + encodeURIComponent(q.split("?")[0]))]);
      if (found.length) return found;
    } catch (e) {
      // fall back to searching for the words in the link
    }
    q = q.split("?")[0].replace(/^https?:\/\/[^/]+\//, "").replace(/[\/\-_]+/g, " ");
  }
  return asList(await api("/tracks?q=" + encodeURIComponent(q) + "&access=playable&limit=10&linked_partitioning=true"));
}

// Spotify: a track link, or a search
async function findSpotifyRequested(q) {
  const m = q.match(/open\.spotify\.com\/(?:intl-[a-z-]+\/)?track\/([A-Za-z0-9]+)/i);
  if (m) {
    try {
      const x = await spApi("GET", "/tracks/" + m[1]);
      if (x && x.id) return [spTrack(x)];
    } catch (e) {
      // fall through to a normal search
    }
  }
  return spotifySearchTracks(q);
}

const REQ_MSG_DEFAULTS = {
  added: 'Added "{title}" by {artist} (#{position} in the queue).',
  playing: 'Added "{title}" by {artist}, playing now.',
  notFound: 'Couldn\'t find "{query}" on {platform}.',
  full: "The request queue is full. Try again in a bit!",
  perUser: "{user}, you already have {count} requests waiting.",
  duplicate: '"{title}" is already in the queue.',
  closed: "Song requests are closed right now.",
  busy: "Requests are set to {platform}, but something else is playing right now. Try again soon!",
  empty: "Tell me what to play, for example: artist - song",
};
const REQ_MSG_LABELS = {
  added: "Added", playing: "Added, playing now", notFound: "Not found", full: "Queue full",
  perUser: "Viewer limit", duplicate: "Already queued", closed: "Requests off", busy: "Other platform playing",
  empty: "Nothing asked for",
};

function reqMsg(key, vars) {
  const tpl = ((prefs.reqMsgs || {})[key] || "").trim() || REQ_MSG_DEFAULTS[key];
  return tpl.replace(/\{(\w+)\}/g, (m, k) => (vars && vars[k] !== undefined ? String(vars[k]) : m));
}

async function handleSongRequest(q, user) {
  if (!prefs.req) return { ok: false, message: reqMsg("closed", {}) };
  q = q.slice(0, 200);
  const who = user || "chat";
  const plat = curPlat("search");
  if (plat === "none") return { ok: false, message: reqMsg("closed", { user: who }) };
  const platName = PLATFORMS[plat].name;
  const base = { user: who, query: q, platform: platName };
  if (!q) return { ok: false, message: reqMsg("empty", base) };
  const pending = reqPendingList();
  if (pending.length >= (prefs.reqMax || 10)) return { ok: false, message: reqMsg("full", base) };
  const mine = pending.filter((t) => t.requestedBy.toLowerCase() === who.toLowerCase()).length;
  if (user && mine >= (prefs.reqPerUser || 2)) return { ok: false, message: reqMsg("perUser", Object.assign({ count: mine }, base)) };
  const cur = curTrack();
  if (cur && srcOf(cur) !== plat && (srcOf(cur) === "spotify" || plat === "spotify")) {
    return { ok: false, message: reqMsg("busy", base) };
  }
  let found;
  if (plat === "spotify") found = await findSpotifyRequested(q);
  else if (plat === "local") found = localSearch(q);
  else found = await findRequested(q);
  found = found.filter((t) => !isMix(t));
  if (!found.length) return { ok: false, message: reqMsg("notFound", base) };
  const t = found[0];
  const vars = Object.assign({ title: t.title, artist: artistName(t) }, base);
  if (queue.slice(Math.max(qi, 0)).some((x) => x.id === t.id)) return { ok: false, message: reqMsg("duplicate", vars) };
  t.requestedBy = who;
  addToQueue(t);
  reqPaint();
  reqQueueBtn();
  const pos = queue.indexOf(t) - qi;   // requests go ahead of autoplay picks, so not always last
  vars.position = pos;
  return {
    ok: true, title: t.title, artist: artistName(t), url: t.permalink_url || "", position: pos,
    message: reqMsg(pos > 0 ? "added" : "playing", vars),
  };
}

// Chat bot instructions: only the one you use
function reqRenderBot() {
  const bot = prefs.reqBot || "streamerbot";
  $("botSel").value = bot;
  const port = Number(prefs.reqPort) || 7879;
  const base = "http://127.0.0.1:" + port;
  const key = prefs.reqKey || "";
  const vars = {
    streamerbot: { user: "%user%", q: "%rawInputUrlEncoded%" },
    mixitup: { user: "$username", q: "$allargs" },
    other: { user: "USERNAME", q: "SONG" },
  }[bot];
  const reqAddr = base + "/request?key=" + key + "&user=" + vars.user + "&q=" + vars.q;
  $("reqUrl").value = reqAddr;
  $("reqNowUrl").value = base + "/now";
  const help = $("botHelp");
  help.replaceChildren();
  const code = (s) => { const c = el("code", "", s); return c; };
  const li = (...parts) => { const x = el("li"); parts.forEach((p) => x.append(typeof p === "string" ? document.createTextNode(p) : p)); return x; };
  const ol = el("ol");
  if (bot === "streamerbot") {
    $("reqUrlLbl").textContent = "Fetch URL address (variables already filled in)";
    ol.append(
      li("Create an Action called Song request, with a Command trigger such as ", code("!sr"), "."),
      li("Add the sub-action Core > Network > Fetch URL, paste the address above, and set its variable name to ", code("reply"), "."),
      li("Add the sub-action Twitch > Chat > Send Message to Channel with the message ", code("%reply%"), "."),
      li("For ", code("!song"), ": the same again with the now playing address."),
    );
    help.append(ol, el("p", "note", "If your version names things slightly differently, look for the sub-action that fetches a URL and saves the reply in a variable."));
  } else if (bot === "mixitup") {
    $("reqUrlLbl").textContent = "Web Request address (special identifiers already filled in)";
    ol.append(
      li("Create a Chat Command such as ", code("!sr"), "."),
      li("Add a Web Request action, paste the address above, and set the response to plain text."),
      li("Add a Chat action that sends ", code("$webrequestresult"), "."),
      li("For ", code("!song"), ": the same again with the now playing address."),
    );
    help.append(ol, el("p", "note", "If $webrequestresult doesn't post anything in your version, check Mix It Up's list of special identifiers for the web request result."));
  } else {
    $("reqUrlLbl").textContent = "Request address (replace USERNAME and SONG with your bot's variables)";
    help.append(
      el("p", "", "Any bot that can call a web address and post the reply in chat will work:"),
      (() => { const l = el("ol"); l.append(
        li("Call ", code(base + "/request"), " with ", code("key"), " (required, from this page), ", code("q"),
           " (what was asked for: search words or a SoundCloud or Spotify link) and ", code("user"),
           " (who asked; used for the per-viewer limit and \"requested by\")."),
        li("Text in ", code("q"), " and ", code("user"), " must be URL-encoded (spaces as %20). Most bots have an encoded version of the message variable."),
        li("The reply is one line of plain text, ready to post in chat. Add ", code("&format=json"),
           " to get JSON instead: ok, message, title, artist, url and position."),
        li("Call ", code(base + "/now"), " for the current song (no key needed). It also supports ", code("&format=json"), "."),
        li("Only programs on this PC can reach these addresses."),
      ); return l; })(),
    );
  }
}

function reqQueueBtn() {
  const b = $("qClearReq");
  if (b) b.hidden = !(prefs.req && reqPendingList().length);
}

function clearRequests() {
  queue = queue.filter((t, i) => i <= qi || !t.requestedBy);
  renderQueue();
  reqPaint();
  say("Waiting requests removed.");
}

function renderReqMsgs() {
  const box = $("reqMsgs");
  box.replaceChildren();
  Object.keys(REQ_MSG_DEFAULTS).forEach((k) => {
    const row = el("div", "msgRow");
    const lab = el("label", "", REQ_MSG_LABELS[k]);
    lab.htmlFor = "msg_" + k;
    const inp = el("input");
    inp.id = "msg_" + k;
    inp.type = "text";
    inp.maxLength = 300;
    inp.placeholder = REQ_MSG_DEFAULTS[k];
    inp.value = (prefs.reqMsgs || {})[k] || "";
    inp.dataset.k = k;
    row.append(lab, inp);
    box.append(row);
  });
}

function requestsInit() {
  $("reqOn").addEventListener("change", () => {
    prefs.req = $("reqOn").checked;
    savePrefs();
    reqApply();
    reqQueueBtn();
    spModePaint();
  });
  $("reqPort").addEventListener("change", () => {
    const p = Math.round(Number($("reqPort").value));
    if (p >= 1024 && p <= 65535) { prefs.reqPort = p; savePrefs(); reqApply(); }
    else $("reqPort").value = prefs.reqPort || 7879;
  });
  $("reqNewKey").addEventListener("click", () => {
    prefs.reqKey = reqKeyNew();
    savePrefs();
    reqApply();
    say("New request key made. Update the address in your chat bot.");
  });
  const copy = (inputId, btnId) => $(btnId).addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText($(inputId).value);
      $(btnId).textContent = "Copied";
      setTimeout(() => { $(btnId).textContent = "Copy"; }, 1500);
    } catch (e) {
      $(inputId).select();
    }
  });
  copy("reqUrl", "reqCopy");
  copy("reqNowUrl", "reqNowCopy");
  const stepper = (key, def, min, max, down, up) => {
    const set = (d) => { prefs[key] = Math.min(max, Math.max(min, (prefs[key] || def) + d)); savePrefs(); reqPaint(); };
    $(down).addEventListener("click", () => set(-1));
    $(up).addEventListener("click", () => set(1));
  };
  stepper("reqMax", 10, 1, 50, "reqMaxDown", "reqMaxUp");
  stepper("reqPerUser", 2, 1, 10, "reqUserDown", "reqUserUp");
  $("reqClear").addEventListener("click", clearRequests);
  $("qClearReq").addEventListener("click", clearRequests);

  // Chat bot picker and editable replies
  $("botSel").addEventListener("change", () => { prefs.reqBot = $("botSel").value; savePrefs(); reqRenderBot(); });
  renderReqMsgs();
  $("reqMsgs").addEventListener("input", (e) => {
    const k = e.target.dataset.k;
    if (!k) return;
    prefs.reqMsgs = prefs.reqMsgs || {};
    prefs.reqMsgs[k] = e.target.value;
    savePrefs();
  });
  $("msgReset").addEventListener("click", () => { prefs.reqMsgs = {}; savePrefs(); renderReqMsgs(); });

  reqApply();
  reqRenderBot();
  reqQueueBtn();
}
