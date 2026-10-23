// Plugins, run here (as the web app's js/plugins.js runs them in a sandboxed frame): each one in its
// own QuickJS engine on its own thread, with nothing it can reach but what the app gives it: fetch (to
// the sites its manifest lists, checked in core::plugins::http), and, if its manifest asks for them,
// sockets (live connections, to the same sites) and the app (chat's requests, alerts, what's playing).
// It gets a memory limit, and each turn of its script a time limit: one that runs on past it is
// stopped mid-way. Between turns its thread sleeps until something comes for it (a call, an answer, a
// socket's message, a timer): an idle plugin costs nothing.
//
// The API, as the web app's: OmniPulse.register({ search, resolve, library? }), OmniPulse.fetch(url,
// { method, headers, body }) -> { ok, status, headers, text(), json() }, OmniPulse.settings,
// OmniPulse.onSettings(fn), OmniPulse.log(...); and setTimeout/setInterval (and their clears). With
// "sockets": OmniPulse.socket(url) -> { send(text), close(), onopen, onmessage(text), onclose(why) }.
// With "app": OmniPulse.app.request(user, query, platform), .wrongSong(user, isMod), .nowPlaying(),
// .alert({ type, user, amount, message }), .openUrl(url), .say(text), .saveSettings(values), each a
// promise of the app's answer; and register({ start() }) runs once it's up.

use omnipulse_core as core;
use rquickjs::{Context, Function, Runtime};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MEMORY: usize = 64 * 1024 * 1024;
const CALL_SECS: u64 = 25;

/// A running plugin
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub hosts: Vec<String>,
    /// what it registered (search, resolve, library...)
    pub methods: Vec<String>,
    jobs: std::sync::mpsc::Sender<Msg>,
}

struct Job {
    method: String,
    /// the arguments, as a JSON array
    args: String,
    reply: tokio::sync::oneshot::Sender<Result<Value, String>>,
}

/// What wakes a plugin's thread
enum Msg {
    Call(Job),
    /// a fetch's answer (its number, the answer)
    Fetched(u32, String),
    /// a socket's: (its number, "open" | "message" | "close", its text)
    Socket(u32, &'static str, String),
    /// the app's answer to an app call (its number, the answer)
    App(u32, String),
}

/// The app's side of OmniPulse.app: (the plugin, what's asked as JSON, where its answer goes)
pub type AppHandler = Arc<dyn Fn(&str, Value, Box<dyn FnOnce(Value) + Send>) + Send + Sync>;
static APP: Mutex<Option<AppHandler>> = Mutex::new(None);

/// What answers plugins' OmniPulse.app calls (set by the app at its start)
pub fn set_app_handler(h: AppHandler) {
    *APP.lock().unwrap() = Some(h);
}

static RUNNING: Mutex<Option<HashMap<String, Arc<Plugin>>>> = Mutex::new(None);

/// The running plugins that can search and play (the platforms they add)
pub fn players() -> Vec<Arc<Plugin>> {
    let r = RUNNING.lock().unwrap();
    let mut v: Vec<Arc<Plugin>> = r.as_ref().map(|m| m.values().filter(|p| p.can("search") && p.can("resolve")).cloned().collect()).unwrap_or_default();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

pub fn get(id: &str) -> Option<Arc<Plugin>> {
    RUNNING.lock().unwrap().as_ref().and_then(|m| m.get(id).cloned())
}

pub fn stop(id: &str) {
    // (dropping its job sender ends its thread)
    if let Some(m) = RUNNING.lock().unwrap().as_mut() {
        m.remove(id);
    }
}

impl Plugin {
    pub fn can(&self, method: &str) -> bool {
        self.methods.iter().any(|m| m == method)
    }

    /// Calls one of its methods; the answer as JSON
    pub async fn call(&self, method: &str, args: Vec<Value>) -> Result<Value, String> {
        let (reply, got) = tokio::sync::oneshot::channel();
        self.jobs.send(Msg::Call(Job { method: method.to_string(), args: Value::Array(args).to_string(), reply })).map_err(|_| format!("{} has stopped.", self.name))?;
        got.await.map_err(|_| format!("{} has stopped.", self.name))?
    }
}

// What the plugin's script sees: the OmniPulse object, its timers, and how calls and events reach it
const BOOT: &str = r#"
"use strict";
var __impl = null, __settingsFns = [], __waiting = {}, __timers = {}, __timerN = 0, __socks = {};
// an answer from the app (a fetch's, an app call's): its promise settles
var __host_done = (id, r) => { const w = __waiting[id]; if (!w) return; delete __waiting[id]; w(JSON.parse(r)); };
var __ask = (id) => new Promise((done) => { __waiting[id] = done; });
// timers: kept here; the app asks when the next is due, and runs those that are
var setTimeout = (fn, ms) => { const id = ++__timerN; __timers[id] = { at: Date.now() + Math.max(0, Number(ms) || 0), fn, every: 0 }; return id; };
var setInterval = (fn, ms) => { const id = ++__timerN; const every = Math.max(10, Number(ms) || 0); __timers[id] = { at: Date.now() + every, fn, every }; return id; };
var clearTimeout = (id) => { delete __timers[id]; };
var clearInterval = clearTimeout;
var __next_timer = () => { let n = -1; for (const k in __timers) { const d = Math.max(0, __timers[k].at - Date.now()); if (n < 0 || d < n) n = d; } return n; };
var __run_timers = () => {
  const now = Date.now();
  for (const k of Object.keys(__timers)) {
    const t = __timers[k];
    if (!t || t.at > now) continue;
    if (t.every) t.at = now + t.every; else delete __timers[k];
    try { t.fn(); } catch (e) { __host_log("timer: " + ((e && e.message) || e)); }
  }
};
// a socket's events, from the app
var __sock_event = (id, kind, data) => {
  const s = __socks[id]; if (!s) return;
  if (kind === "close") delete __socks[id];
  const f = kind === "open" ? s.onopen : kind === "message" ? s.onmessage : s.onclose;
  if (typeof f === "function") { try { f(data); } catch (e) { __host_log("socket: " + ((e && e.message) || e)); } }
};
var OmniPulse = {
  settings: {},
  register(x) { __impl = x || {}; },
  async fetch(url, init) {
    init = init || {};
    const req = { url: String(url), method: init.method || "GET", headers: Object.entries(init.headers || {}).map(([k, v]) => [String(k), String(v)]), body: init.body == null ? null : String(init.body) };
    // (made by the app while this carries on, so several can be on their way at once)
    const r = await __ask(__host_fetch(JSON.stringify(req)));
    if (r.error) throw new Error(r.error);
    return { ok: r.status >= 200 && r.status < 300, status: r.status, headers: r.headers, text: async () => r.body, json: async () => JSON.parse(r.body) };
  },
  socket(url) {
    const id = __host_socket_open(String(url));
    const s = { onopen: null, onmessage: null, onclose: null, send: (t) => __host_socket_send(id, String(t)), close: () => __host_socket_close(id) };
    if (id > 0) __socks[id] = s; else Promise.resolve().then(() => { if (typeof s.onclose === "function") s.onclose("not allowed"); });
    return s;
  },
  app: new Proxy({}, { get: (_, op) => (...args) => __ask(__host_app(JSON.stringify({ op: String(op), args }))).then((r) => { if (r && r.error) throw new Error(r.error); return r; }) }),
  onSettings(fn) { __settingsFns.push(fn); },
  log(...a) { __host_log(a.map(String).join(" ")); },
};
var __methods = () => JSON.stringify(__impl ? Object.keys(__impl).filter((k) => typeof __impl[k] === "function") : []);
var __call = (id, m, args) => {
  Promise.resolve()
    .then(() => { if (!__impl || typeof __impl[m] !== "function") throw new Error("this plugin has no " + m); return __impl[m](...JSON.parse(args)); })
    .then((r) => __host_reply(id, JSON.stringify({ ok: r === undefined ? null : r })), (e) => __host_reply(id, JSON.stringify({ error: String((e && e.message) || e) })));
};
var __settings = (s) => { OmniPulse.settings = JSON.parse(s); __settingsFns.forEach((f) => { try { f(OmniPulse.settings); } catch (e) {} }); };
"#;

/// Starts an installed plugin with its settings; it's running once it has registered (and its start(),
/// if it has one, has been called: a chatbot's connections)
pub fn start(pkg: &core::plugins::Package, settings: &Value, rt: tokio::runtime::Handle) -> Result<Arc<Plugin>, String> {
    let id = pkg.manifest.id.clone();
    stop(&id);
    let (jobs, inbox) = std::sync::mpsc::channel::<Msg>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<Vec<String>, String>>();
    let caps = Caps { sockets: pkg.manifest.capabilities.iter().any(|c| c == "sockets"), app: pkg.manifest.capabilities.iter().any(|c| c == "app"), hosts: pkg.manifest.hosts.clone() };
    let (code, settings, pid, tx) = (pkg.code.clone(), settings.to_string(), id.clone(), jobs.clone());
    std::thread::Builder::new()
        .name(format!("plugin {id}"))
        .spawn(move || run(pid, code, settings, caps, rt, tx, inbox, ready_tx))
        .map_err(|e| e.to_string())?;
    let methods = ready_rx.recv_timeout(Duration::from_secs(CALL_SECS)).map_err(|_| "it took too long to start".to_string())??;
    let p = Arc::new(Plugin { id: id.clone(), name: pkg.manifest.name.clone(), hosts: pkg.manifest.hosts.clone(), methods, jobs });
    RUNNING.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, p.clone());
    // (a plugin that runs on its own, a chatbot: started once it's in)
    if p.can("start") {
        let p2 = p.clone();
        rt_spawn_start(p2);
    }
    Ok(p)
}

fn rt_spawn_start(p: Arc<Plugin>) {
    let (reply, _got) = tokio::sync::oneshot::channel();
    let _ = p.jobs.send(Msg::Call(Job { method: "start".into(), args: "[]".into(), reply }));
}

/// What its manifest lets it have, beyond fetch
struct Caps {
    sockets: bool,
    app: bool,
    hosts: Vec<String>,
}

/// Whether a plugin may open this socket: wss:// to one of its sites, or ws:// to this PC (OBS's)
pub fn socket_allowed(hosts: &[String], url: &str) -> bool {
    let as_http = if let Some(r) = url.strip_prefix("wss://") { format!("https://{r}") } else if let Some(r) = url.strip_prefix("ws://") { format!("http://{r}") } else { return false };
    core::plugins::host_allowed(hosts, &as_http)
}

/// The plugin's thread: its engine, then a turn of its script each time something comes for it
#[allow(clippy::too_many_arguments)]
fn run(id: String, code: String, settings: String, caps: Caps, rt: tokio::runtime::Handle, tx: std::sync::mpsc::Sender<Msg>, inbox: std::sync::mpsc::Receiver<Msg>, ready: std::sync::mpsc::Sender<Result<Vec<String>, String>>) {
    let fail = |e: String| { let _ = ready.send(Err(e)); };
    let Ok(engine) = Runtime::new() else { return fail("its engine didn't start".into()) };
    engine.set_memory_limit(MEMORY);
    // a turn that runs on past its time is stopped
    let deadline: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(Some(Instant::now() + Duration::from_secs(CALL_SECS))));
    let d2 = deadline.clone();
    engine.set_interrupt_handler(Some(Box::new(move || d2.lock().unwrap().is_some_and(|d| Instant::now() > d))));
    let Ok(ctx) = Context::full(&engine) else { return fail("its engine didn't start".into()) };
    // its calls' answers as the script gives them (call number, answer), and its sockets' senders
    let replies: Arc<Mutex<Vec<(u32, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let socks: Arc<Mutex<HashMap<u32, tokio::sync::mpsc::UnboundedSender<Option<String>>>>> = Arc::new(Mutex::new(HashMap::new()));
    let next_n = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let next = { let n = next_n.clone(); move || n.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1 };
    let started: Result<Vec<String>, String> = ctx.with(|ctx| {
        let g = ctx.globals();
        let e = |e: rquickjs::Error| e.to_string();
        // fetch: made by the app, answered as a message
        let (pid, rt2, tx2, nx) = (id.clone(), rt.clone(), tx.clone(), next.clone());
        g.set("__host_fetch", Function::new(ctx.clone(), move |req: String| -> u32 {
            let n = nx();
            let (pid, tx) = (pid.clone(), tx2.clone());
            rt2.spawn(async move {
                let r: Value = serde_json::from_str(&req).unwrap_or_default();
                let headers: Vec<(String, String)> = r["headers"].as_array().into_iter().flatten().take(50)
                    .filter_map(|h| Some((h[0].as_str()?.to_string(), h[1].as_str()?.to_string()))).collect();
                let got = core::plugins::http(&pid, r["url"].as_str().unwrap_or(""), r["method"].as_str().unwrap_or("GET"), headers, r["body"].as_str().map(String::from)).await;
                let out = match got { Ok(v) => v.to_string(), Err(e) => json!({ "error": e }).to_string() };
                let _ = tx.send(Msg::Fetched(n, out));
            });
            n
        }).map_err(e)?).map_err(e)?;
        // sockets: only with "sockets", only to its sites (or OBS's, on this PC)
        let (rt3, tx3, nx, sk, allowed, hosts, lid) = (rt.clone(), tx.clone(), next.clone(), socks.clone(), caps.sockets, caps.hosts.clone(), id.clone());
        g.set("__host_socket_open", Function::new(ctx.clone(), move |url: String| -> u32 {
            if !allowed || !socket_allowed(&hosts, &url) { eprintln!("[plugin {lid}] socket refused: {url}"); return 0; }
            let n = nx();
            let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Option<String>>();
            sk.lock().unwrap().insert(n, out_tx);
            let (tx, sk2) = (tx3.clone(), sk.clone());
            rt3.spawn(async move {
                use futures_util::{SinkExt, StreamExt};
                let ws = match tokio_tungstenite::connect_async(url.as_str()).await {
                    Ok((ws, _)) => ws,
                    Err(e) => { sk2.lock().unwrap().remove(&n); let _ = tx.send(Msg::Socket(n, "close", e.to_string())); return; }
                };
                let _ = tx.send(Msg::Socket(n, "open", String::new()));
                let (mut w, mut r) = ws.split();
                let why = loop {
                    tokio::select! {
                        m = r.next() => match m {
                            Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t))) => { if tx.send(Msg::Socket(n, "message", t.to_string())).is_err() { break "the plugin stopped".to_string(); } }
                            Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(p))) => { let _ = w.send(tokio_tungstenite::tungstenite::Message::Pong(p)).await; }
                            Some(Ok(tokio_tungstenite::tungstenite::Message::Close(c))) => break c.map(|c| c.reason.to_string()).unwrap_or_else(|| "closed".into()),
                            Some(Ok(_)) => {}
                            Some(Err(e)) => break e.to_string(),
                            None => break "closed".into(),
                        },
                        o = out_rx.recv() => match o {
                            Some(Some(t)) => { if let Err(e) = w.send(tokio_tungstenite::tungstenite::Message::Text(t.into())).await { break e.to_string(); } }
                            // (closed by the plugin, or the plugin's gone)
                            _ => { let _ = w.close().await; break "closed".into(); }
                        },
                    }
                };
                sk2.lock().unwrap().remove(&n);
                let _ = tx.send(Msg::Socket(n, "close", why));
            });
            n
        }).map_err(e)?).map_err(e)?;
        let sk = socks.clone();
        g.set("__host_socket_send", Function::new(ctx.clone(), move |n: u32, text: String| -> bool { sk.lock().unwrap().get(&n).is_some_and(|s| s.send(Some(text)).is_ok()) }).map_err(e)?).map_err(e)?;
        let sk = socks.clone();
        g.set("__host_socket_close", Function::new(ctx.clone(), move |n: u32| { if let Some(s) = sk.lock().unwrap().get(&n) { let _ = s.send(None); } }).map_err(e)?).map_err(e)?;
        // the app: only with "app"
        let (tx4, nx, app_ok, pid) = (tx.clone(), next.clone(), caps.app, id.clone());
        g.set("__host_app", Function::new(ctx.clone(), move |ask: String| -> u32 {
            let n = nx();
            let tx = tx4.clone();
            let answer = move |v: Value| { let _ = tx.send(Msg::App(n, v.to_string())); };
            let h = APP.lock().unwrap().clone();
            match (app_ok, h) {
                (true, Some(h)) => h(&pid, serde_json::from_str(&ask).unwrap_or_default(), Box::new(answer)),
                (false, _) => answer(json!({ "error": "this plugin can't use the app (its manifest doesn't ask to)" })),
                (true, None) => answer(json!({ "error": "the app isn't ready" })),
            }
            n
        }).map_err(e)?).map_err(e)?;
        let rp = replies.clone();
        g.set("__host_reply", Function::new(ctx.clone(), move |n: u32, r: String| { rp.lock().unwrap().push((n, r)); }).map_err(e)?).map_err(e)?;
        let lid = id.clone();
        g.set("__host_log", Function::new(ctx.clone(), move |s: String| { eprintln!("[plugin {lid}] {}", s.chars().take(2000).collect::<String>()); }).map_err(e)?).map_err(e)?;
        ctx.eval::<(), _>(BOOT).map_err(|e| format!("its setup failed: {e}"))?;
        let set: Function = g.get("__settings").map_err(|e| e.to_string())?;
        set.call::<_, ()>((settings,)).map_err(|e| e.to_string())?;
        ctx.eval::<(), _>(format!("(function (OmniPulse) {{\n{code}\n}})(OmniPulse);")).map_err(|e| format!("it didn't start: {}", caught(&ctx, e)))?;
        let methods: String = ctx.eval("__methods()").map_err(|e| e.to_string())?;
        Ok(serde_json::from_str(&methods).unwrap_or_default())
    });
    *deadline.lock().unwrap() = None;
    let ok = started.is_ok();
    let _ = ready.send(started);
    if !ok {
        return;
    }
    // its calls waiting for their answers: (their reply, when they give up)
    let mut calls: HashMap<u32, (tokio::sync::oneshot::Sender<Result<Value, String>>, Instant)> = HashMap::new();
    loop {
        // asleep till something comes, its next timer's due, or a call gives up
        let timer: Option<u64> = ctx.with(|ctx| ctx.eval::<f64, _>("__next_timer()").ok()).filter(|ms| *ms >= 0.0).map(|ms| ms as u64);
        let give_up = calls.values().map(|(_, at)| at.saturating_duration_since(Instant::now())).min();
        let wait = [timer.map(Duration::from_millis), give_up].into_iter().flatten().min().unwrap_or(Duration::from_secs(3600));
        let mut msgs = Vec::new();
        match inbox.recv_timeout(wait) {
            Ok(m) => msgs.push(m),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            // (stopped: its sender's gone; its sockets go with their senders)
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        while let Ok(m) = inbox.try_recv() { msgs.push(m); }
        // one turn: what came, the timers due, then the promises' work till there's none (in its time)
        *deadline.lock().unwrap() = Some(Instant::now() + Duration::from_secs(CALL_SECS));
        ctx.with(|ctx| {
            let g = ctx.globals();
            for m in msgs {
                let r = match m {
                    Msg::Call(job) => {
                        let n = next();
                        calls.insert(n, (job.reply, Instant::now() + Duration::from_secs(CALL_SECS)));
                        g.get::<_, Function>("__call").and_then(|f| f.call::<_, ()>((n, job.method, job.args)))
                    }
                    Msg::Fetched(n, r) | Msg::App(n, r) => g.get::<_, Function>("__host_done").and_then(|f| f.call::<_, ()>((n, r))),
                    Msg::Socket(n, kind, data) => g.get::<_, Function>("__sock_event").and_then(|f| f.call::<_, ()>((n, kind, data))),
                };
                if let Err(e) = r { eprintln!("[plugin {id}] {}", caught(&ctx, e)); }
            }
            let _ = ctx.eval::<(), _>("__run_timers()");
        });
        // (outside the context: the engine runs its promises' work itself)
        let broke = loop {
            match engine.execute_pending_job() {
                Ok(true) => {}
                Ok(false) => break false,
                Err(_) => break true,
            }
        };
        *deadline.lock().unwrap() = None;
        if broke { eprintln!("[plugin {id}] a turn of it stopped with an error, or ran out of time"); }
        // its answers to their calls; those it hasn't answered in time, given up
        for (n, out) in std::mem::take(&mut *replies.lock().unwrap()) {
            if let Some((reply, _)) = calls.remove(&n) {
                let v: Value = serde_json::from_str(&out).unwrap_or_default();
                let _ = reply.send(match v.get("error").and_then(|e| e.as_str()) { Some(e) => Err(e.to_string()), None => Ok(v["ok"].clone()) });
            }
        }
        let late: Vec<u32> = calls.iter().filter(|(_, (_, at))| Instant::now() >= *at).map(|(n, _)| *n).collect();
        for n in late { if let Some((reply, _)) = calls.remove(&n) { let _ = reply.send(Err("it took too long to answer".into())); } }
    }
    // (its sockets closed as it goes)
    for (_, s) in socks.lock().unwrap().drain() { let _ = s.send(None); }
}

/// The message of what the script threw
fn caught(ctx: &rquickjs::Ctx, e: rquickjs::Error) -> String {
    if matches!(e, rquickjs::Error::Exception) {
        let ex = ctx.catch();
        if let Some(o) = ex.as_object() {
            if let Ok(m) = o.get::<_, String>("message") {
                return m;
            }
        }
        return format!("{ex:?}");
    }
    e.to_string()
}

/// A plugin's settings: its manifest's defaults, with what's been set (the web app's prefs.pluginSettings)
pub fn settings_of(m: &core::plugins::Manifest, prefs: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for f in m.settings.as_array().into_iter().flatten() {
        if let (Some(k), Some(d)) = (f["key"].as_str(), f.get("default")) {
            out.insert(k.to_string(), d.clone());
        }
    }
    if let Some(set) = prefs["pluginSettings"][&m.id].as_object() {
        for (k, v) in set {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(code: &str) -> core::plugins::Package {
        core::plugins::Package {
            manifest: core::plugins::Manifest { id: "test-plug".into(), name: "Test".into(), main: "main.js".into(), ..Default::default() },
            code: code.into(),
            digest: String::new(),
            bundled: false,
        }
    }

    #[test]
    fn calls_and_answers() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let p = start(&pkg(r#"OmniPulse.register({ async search(q) { return [{ id: 1, title: q + "!", artist: OmniPulse.settings.who }]; }, resolve(t) { return { url: "https://x/" + t.id }; } });"#), &json!({ "who": "me" }), rt.handle().clone()).unwrap();
        assert!(p.can("search") && p.can("resolve") && !p.can("library"));
        let got = rt.block_on(p.call("search", vec![json!("hi")])).unwrap();
        assert_eq!(got[0]["title"], "hi!");
        assert_eq!(got[0]["artist"], "me");
        let err = rt.block_on(p.call("library", vec![])).unwrap_err();
        assert!(err.contains("no library"), "{err}");
        stop("test-plug");
    }

    #[test]
    fn it_can_reach_nothing_but_its_fetch() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        // no window, no network of its own
        let p = start(&pkg(r#"OmniPulse.register({ search() { return [typeof fetch, typeof XMLHttpRequest, typeof require]; }, resolve() { return null; } });"#), &json!({}), rt.handle().clone()).unwrap();
        assert_eq!(rt.block_on(p.call("search", vec![])).unwrap(), json!(["undefined", "undefined", "undefined"]));
        stop("test-plug");
    }

    #[test]
    fn timers_run_and_calls_wait_for_them() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let p = start(&pkg(r#"let n = 0; const t = setInterval(() => { n++; if (n >= 3) clearInterval(t); }, 20);
            OmniPulse.register({ search() { return new Promise((done) => setTimeout(() => done([n]), 150)); }, resolve() { return null; } });"#), &json!({}), rt.handle().clone()).unwrap();
        assert_eq!(rt.block_on(p.call("search", vec![])).unwrap(), json!([3]));
        stop("test-plug");
    }

    #[test]
    fn sockets_and_the_app_only_with_their_capabilities() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let code = r#"OmniPulse.register({ async search() {
            const why = await new Promise((done) => { const s = OmniPulse.socket("wss://irc-ws.chat.twitch.tv:443"); s.onclose = done; });
            let app = ""; try { await OmniPulse.app.say("hi"); } catch (e) { app = e.message; }
            return [why, app];
        }, resolve() { return null; } });"#;
        let p = start(&pkg(code), &json!({}), rt.handle().clone()).unwrap();
        let got = rt.block_on(p.call("search", vec![])).unwrap();
        assert_eq!(got[0], "not allowed");
        assert!(got[1].as_str().unwrap().contains("manifest"), "{got}");
        stop("test-plug");
        // (and a socket to a site it doesn't list, refused even with the capability)
        assert!(!socket_allowed(&["irc-ws.chat.twitch.tv".into()], "wss://example.com/"));
        assert!(socket_allowed(&["irc-ws.chat.twitch.tv".into()], "wss://irc-ws.chat.twitch.tv:443"));
        assert!(socket_allowed(&["127.0.0.1".into()], "ws://127.0.0.1:4455"));
        assert!(!socket_allowed(&["example.com".into()], "ws://example.com/"));
    }

    /// The chatbot plugin (plugins/chatbot) against a stand-in Twitch on this PC: its chat (IRC over a
    /// WebSocket) and its events (EventSub's). A request and a command answered in chat, a follow an alert.
    #[test]
    fn the_chatbot_answers_chat_and_turns_events_into_alerts() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let rt = tokio::runtime::Runtime::new().unwrap();
        // (the stand-in: one listener; /irc is chat, /events EventSub)
        let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let port = listener.local_addr().unwrap().port();
        let said: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let s2 = said.clone();
        rt.spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let path = Arc::new(Mutex::new(String::new()));
                let p2 = path.clone();
                let ws = tokio_tungstenite::accept_hdr_async(tcp, move |req: &tokio_tungstenite::tungstenite::handshake::server::Request, res| { *p2.lock().unwrap() = req.uri().path().to_string(); Ok(res) }).await.unwrap();
                let (mut w, mut r) = ws.split();
                let said = s2.clone();
                if path.lock().unwrap().as_str() == "/irc" {
                    tokio::spawn(async move {
                        while let Some(Ok(Message::Text(t))) = r.next().await {
                            for line in t.split("\r\n").filter(|l| !l.is_empty()) {
                                said.lock().unwrap().push(line.to_string());
                                if line.starts_with("JOIN") {
                                    let _ = w.send(Message::Text("@badges=;display-name=Viewer;id=m1;mod=0 :viewer!viewer@viewer.tmi.twitch.tv PRIVMSG #me :!sr dancing queen\r\n".into())).await;
                                    let _ = w.send(Message::Text("@badges=;display-name=Viewer;id=m2;mod=0 :viewer!viewer@viewer.tmi.twitch.tv PRIVMSG #me :!discord\r\nPING :tmi.twitch.tv\r\n".into())).await;
                                }
                            }
                        }
                    });
                } else {
                    let _ = w.send(Message::Text(r#"{"metadata":{"message_type":"session_welcome"},"payload":{"session":{"id":"s1"}}}"#.into())).await;
                    let _ = w.send(Message::Text(r#"{"metadata":{"message_type":"notification"},"payload":{"subscription":{"type":"channel.follow"},"event":{"user_name":"NewFan"}}}"#.into())).await;
                    tokio::spawn(async move { while r.next().await.is_some() {} });
                }
            }
        });
        // the app: a request answered, an alert noted
        let asked: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let a2 = asked.clone();
        set_app_handler(Arc::new(move |_p: &str, ask: Value, answer: Box<dyn FnOnce(Value) + Send>| {
            a2.lock().unwrap().push(ask.clone());
            answer(match ask["op"].as_str() { Some("request") => json!({ "ok": true, "message": "Added \"Dancing Queen\" by ABBA." }), _ => json!({ "ok": true }) });
        }));
        // the plugin as it comes, pointed at the stand-in (its sign-in taken as good)
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/chatbot");
        let code = std::fs::read_to_string(root.join("main.js")).unwrap()
            .replace("wss://irc-ws.chat.twitch.tv:443", &format!("ws://127.0.0.1:{port}/irc"))
            .replace("wss://eventsub.wss.twitch.tv/ws", &format!("ws://127.0.0.1:{port}/events"))
            .replace("if (await validate(token)) return token;", "return token;");
        let mut m: core::plugins::Manifest = serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
        m.hosts.push("127.0.0.1".into());
        let pkg = core::plugins::Package { manifest: m, code, digest: String::new(), bundled: true };
        let p = start(&pkg, &json!({ "token": "t", "login": "me", "userId": "1", "twitchClientId": "c", "commands": "!discord = Join us: https://discord.gg/x" }), rt.handle().clone()).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        let done = || {
            let s = said.lock().unwrap();
            s.iter().any(|l| l.contains("PRIVMSG #me :Added \"Dancing Queen\"")) && s.iter().any(|l| l.contains("PRIVMSG #me :Join us")) && s.iter().any(|l| l.starts_with("PONG"))
                && asked.lock().unwrap().iter().any(|a| a["op"] == "alert" && a["args"][0]["user"] == "NewFan")
        };
        while !done() && Instant::now() < until { std::thread::sleep(Duration::from_millis(50)); }
        let (s, a) = (said.lock().unwrap().clone(), asked.lock().unwrap().clone());
        assert!(done(), "chat said {s:#?}, the app was asked {a:#?}");
        // (the reply to the request answers its message; the request as asked)
        assert!(s.iter().any(|l| l.starts_with("@reply-parent-msg-id=m1 PRIVMSG")), "{s:#?}");
        assert!(a.iter().any(|x| x["op"] == "request" && x["args"] == json!(["Viewer", "dancing queen", "twitch"])), "{a:#?}");
        drop(p);
        stop("chatbot");
    }

    /// The chatbot's sign-in to OBS (its script's own SHA-256 and base64), as OBS works it out
    #[test]
    fn the_chatbot_signs_in_to_obs_as_obs_expects() {
        use base64::Engine;
        use sha2::Digest;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/chatbot");
        let code = std::fs::read_to_string(root.join("main.js")).unwrap();
        let m: core::plugins::Manifest = serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
        let pkg = core::plugins::Package { manifest: m, code, digest: String::new(), bundled: true };
        let p = start(&pkg, &json!({}), rt.handle().clone()).unwrap();
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        for (pw, salt, ch) in [("hunter2", "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=", "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY="), ("", "a", "b"), ("p\u{e4}ss w\u{f6}rd \u{1f3b5}", "salt", "challenge")] {
            let secret = b64(&sha2::Sha256::digest(format!("{pw}{salt}").as_bytes()));
            let want = b64(&sha2::Sha256::digest(format!("{secret}{ch}").as_bytes()));
            let got = rt.block_on(p.call("obsAuth", vec![json!(pw), json!(salt), json!(ch)])).unwrap();
            assert_eq!(got, json!(want), "{pw:?}");
        }
        drop(p);
        stop("chatbot");
    }

    /// The chatbot and OBS (a stand-in obs-websocket 5 on this PC): signed in with its password, a scene
    /// change in OBS starting an action, whose steps switch the scene and set a text source
    #[test]
    fn the_chatbot_works_obs() {
        use base64::Engine;
        use futures_util::{SinkExt, StreamExt};
        use sha2::Digest;
        use tokio_tungstenite::tungstenite::Message;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let port = listener.local_addr().unwrap().port();
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        let secret = b64(&sha2::Sha256::digest(b"pw123salty"));
        let want = b64(&sha2::Sha256::digest(format!("{secret}chally").as_bytes()));
        let asked: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let a2 = asked.clone();
        let want2 = want.clone();
        rt.spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let (mut w, mut r) = ws.split();
            let _ = w.send(Message::Text(r#"{"op":0,"d":{"obsWebSocketVersion":"5.0.0","rpcVersion":1,"authentication":{"salt":"salty","challenge":"chally"}}}"#.into())).await;
            while let Some(Ok(Message::Text(t))) = r.next().await {
                let m: Value = serde_json::from_str(&t).unwrap();
                a2.lock().unwrap().push(m.clone());
                if m["op"] == 1 {
                    if m["d"]["authentication"] != json!(want2) { let _ = w.close().await; return; }
                    let _ = w.send(Message::Text(r#"{"op":2,"d":{"negotiatedRpcVersion":1}}"#.into())).await;
                    let _ = w.send(Message::Text(r#"{"op":5,"d":{"eventType":"CurrentProgramSceneChanged","eventIntent":4,"eventData":{"sceneName":"Starting Soon"}}}"#.into())).await;
                } else if m["op"] == 6 {
                    let back = json!({ "op": 7, "d": { "requestType": m["d"]["requestType"], "requestId": m["d"]["requestId"], "requestStatus": { "result": true, "code": 100 }, "responseData": {} } });
                    let _ = w.send(Message::Text(back.to_string().into())).await;
                }
            }
        });
        set_app_handler(Arc::new(|_p: &str, _ask: Value, answer: Box<dyn FnOnce(Value) + Send>| answer(json!({ "ok": true }))));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/chatbot");
        let code = std::fs::read_to_string(root.join("main.js")).unwrap();
        let m: core::plugins::Manifest = serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
        let pkg = core::plugins::Package { manifest: m, code, digest: String::new(), bundled: true };
        let t = |id: &str, set: Value| { let mut d = crate::flows::defaults(crate::flows::TRIGGERS, id); for (k, v) in set.as_object().unwrap() { d[k] = v.clone(); } d };
        let st = |id: &str, set: Value| { let mut d = crate::flows::defaults(crate::flows::STEPS, id); for (k, v) in set.as_object().unwrap() { d[k] = v.clone(); } d };
        let actions = json!([
            { "name": "Going Live", "on": true, "trigger": t("obs", json!({ "when": "scene", "scene": "starting soon" })), "steps": [
                st("obstext", json!({ "source": "Banner", "text": "Live from %scene%" })),
                st("obsscene", json!({ "scene": "Main" })),
            ] },
            { "name": "Other Scene", "on": true, "trigger": t("obs", json!({ "when": "scene", "scene": "Ending" })), "steps": [st("obsscene", json!({ "scene": "Never" }))] },
        ]);
        let p = start(&pkg, &json!({ "obsOn": true, "obsAddress": format!("127.0.0.1:{port}"), "obsPassword": "pw123", "actions": actions }), rt.handle().clone()).unwrap();
        let req = |a: &[Value], kind: &str| a.iter().find(|x| x["op"] == 6 && x["d"]["requestType"] == kind).cloned();
        let until = Instant::now() + Duration::from_secs(10);
        while req(&asked.lock().unwrap(), "SetCurrentProgramScene").is_none() && Instant::now() < until { std::thread::sleep(Duration::from_millis(50)); }
        let a = asked.lock().unwrap().clone();
        assert_eq!(a.first().map(|x| x["d"]["authentication"].clone()), Some(json!(want)), "{a:#?}");
        assert_eq!(req(&a, "SetInputSettings").map(|x| x["d"]["requestData"].clone()), Some(json!({ "inputName": "Banner", "inputSettings": { "text": "Live from Starting Soon" } })), "{a:#?}");
        assert_eq!(req(&a, "SetCurrentProgramScene").map(|x| x["d"]["requestData"]["sceneName"].clone()), Some(json!("Main")), "{a:#?}");
        assert!(!a.iter().any(|x| x["d"]["requestData"]["sceneName"] == "Never"), "{a:#?}");
        drop(p);
        stop("chatbot");
    }

    /// The chatbot's actions (crate::flows' JSON, run by the plugin), against the stand-in Twitch: a command
    /// that answers, a counter kept between uses, a chat message's, "only go on if" stopping one, a raid
    /// (with its least), and a song requested from a step
    #[test]
    fn the_chatbots_actions_run_their_steps() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let port = listener.local_addr().unwrap().port();
        let said: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let s2 = said.clone();
        rt.spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let path = Arc::new(Mutex::new(String::new()));
                let p2 = path.clone();
                let ws = tokio_tungstenite::accept_hdr_async(tcp, move |req: &tokio_tungstenite::tungstenite::handshake::server::Request, res| { *p2.lock().unwrap() = req.uri().path().to_string(); Ok(res) }).await.unwrap();
                let (mut w, mut r) = ws.split();
                let said = s2.clone();
                if path.lock().unwrap().as_str() == "/irc" {
                    tokio::spawn(async move {
                        while let Some(Ok(Message::Text(t))) = r.next().await {
                            for line in t.split("\r\n").filter(|l| !l.is_empty()) {
                                said.lock().unwrap().push(line.to_string());
                                if line.starts_with("JOIN") {
                                    let msg = |id: &str, text: &str| format!("@badges=;display-name=Viewer;id={id};mod=0;user-id=42 :viewer!viewer@viewer.tmi.twitch.tv PRIVMSG #me :{text}\r\n");
                                    for (id, text) in [("a", "!hello"), ("b", "!deaths"), ("c", "!deaths"), ("d", "good morning all"), ("e", "!check no"), ("f", "!check yes"), ("g", "!play take on me")] {
                                        let _ = w.send(Message::Text(msg(id, text).into())).await;
                                    }
                                }
                            }
                        }
                    });
                } else {
                    let _ = w.send(Message::Text(r#"{"metadata":{"message_type":"session_welcome"},"payload":{"session":{"id":"s1"}}}"#.into())).await;
                    let _ = w.send(Message::Text(r#"{"metadata":{"message_type":"notification"},"payload":{"subscription":{"type":"channel.raid"},"event":{"from_broadcaster_user_name":"Small","viewers":2}}}"#.into())).await;
                    let _ = w.send(Message::Text(r#"{"metadata":{"message_type":"notification"},"payload":{"subscription":{"type":"channel.raid"},"event":{"from_broadcaster_user_name":"Big","viewers":30}}}"#.into())).await;
                    tokio::spawn(async move { while r.next().await.is_some() {} });
                }
            }
        });
        let asked: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let a2 = asked.clone();
        set_app_handler(Arc::new(move |_p: &str, ask: Value, answer: Box<dyn FnOnce(Value) + Send>| {
            a2.lock().unwrap().push(ask.clone());
            answer(match ask["op"].as_str() { Some("request") => json!({ "ok": true, "message": "Added it." }), _ => json!({ "ok": true }) });
        }));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/chatbot");
        let code = std::fs::read_to_string(root.join("main.js")).unwrap()
            .replace("wss://irc-ws.chat.twitch.tv:443", &format!("ws://127.0.0.1:{port}/irc"))
            .replace("wss://eventsub.wss.twitch.tv/ws", &format!("ws://127.0.0.1:{port}/events"))
            .replace("if (await validate(token)) return token;", "return token;");
        let mut m: core::plugins::Manifest = serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
        m.hosts.push("127.0.0.1".into());
        let pkg = core::plugins::Package { manifest: m, code, digest: String::new(), bundled: true };
        // (the actions as the Actions page keeps them: crate::flows' defaults, with what's changed)
        let t = |id: &str, set: Value| { let mut d = crate::flows::defaults(crate::flows::TRIGGERS, id); for (k, v) in set.as_object().unwrap() { d[k] = v.clone(); } d };
        let st = |id: &str, set: Value| { let mut d = crate::flows::defaults(crate::flows::STEPS, id); for (k, v) in set.as_object().unwrap() { d[k] = v.clone(); } d };
        let actions = json!([
            { "name": "Hello", "on": true, "trigger": t("command", json!({ "command": "!hello", "cooldown": 0, "usercooldown": 0 })), "steps": [st("say", json!({ "text": "Hi %user%!" }))] },
            { "name": "Deaths", "on": true, "trigger": t("command", json!({ "command": "!deaths", "cooldown": 0, "usercooldown": 0 })), "steps": [st("addvar", json!({ "name": "deaths", "keep": false })), st("say", json!({ "text": "Deaths: %var:deaths%" }))] },
            { "name": "Morning", "on": true, "trigger": t("message", json!({ "text": "morning" })), "steps": [st("say", json!({ "text": "Morning, %user%", "reply": false }))] },
            { "name": "Check", "on": true, "trigger": t("command", json!({ "command": "!check", "cooldown": 0, "usercooldown": 0 })), "steps": [st("if", json!({ "left": "%input0%", "test": "is", "right": "yes" })), st("say", json!({ "text": "Checked: %input0%" }))] },
            { "name": "Raid", "on": true, "trigger": t("event", json!({ "event": "raid", "min": 5 })), "steps": [st("say", json!({ "text": "Raid of %amount% from %user%", "reply": false }))] },
            { "name": "Play", "on": true, "trigger": t("command", json!({ "command": "!play", "cooldown": 0, "usercooldown": 0 })), "steps": [st("request", json!({}))] },
            { "name": "Off", "on": false, "trigger": t("command", json!({ "command": "!hello" })), "steps": [st("say", json!({ "text": "never" }))] },
        ]);
        let p = start(&pkg, &json!({ "token": "t", "login": "me", "userId": "1", "twitchClientId": "c", "actions": actions }), rt.handle().clone()).unwrap();
        let line = |text: &str| { let s = said.lock().unwrap(); s.iter().any(|l| l.ends_with(&format!(":{text}"))) };
        let until = Instant::now() + Duration::from_secs(10);
        let done = || line("Hi Viewer!") && line("Deaths: 2") && line("Morning, Viewer") && line("Checked: yes") && line("Raid of 30 from Big") && line("Added it.");
        while !done() && Instant::now() < until { std::thread::sleep(Duration::from_millis(50)); }
        let s = said.lock().unwrap().clone();
        assert!(done(), "chat said {s:#?}");
        // (stopped where it should: no "no", no small raid, nothing from the action that's off)
        assert!(!line("Checked: no") && !line("Raid of 2 from Small") && !line("never"), "{s:#?}");
        assert!(asked.lock().unwrap().iter().any(|x| x["op"] == "request" && x["args"] == json!(["Viewer", "take on me", "twitch"])));
        drop(p);
        stop("chatbot");
    }
}



