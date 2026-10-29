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
    /// stopped: its thread ends and its sockets close, though they still hold its inbox's senders
    Stop,
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
    // (told to stop, not only forgotten: its sockets and timers hold senders to its inbox, so dropping ours
    // alone left it running, a second chatbot reading and answering chat beside the new one)
    let gone = RUNNING.lock().unwrap().as_mut().and_then(|m| m.remove(id));
    if let Some(p) = gone { let _ = p.jobs.send(Msg::Stop); }
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
        let (hosts_f, rt2, tx2, nx) = (caps.hosts.clone(), rt.clone(), tx.clone(), next.clone());
        g.set("__host_fetch", Function::new(ctx.clone(), move |req: String| -> u32 {
            let n = nx();
            let (hosts, tx) = (hosts_f.clone(), tx2.clone());
            rt2.spawn(async move {
                let r: Value = serde_json::from_str(&req).unwrap_or_default();
                let headers: Vec<(String, String)> = r["headers"].as_array().into_iter().flatten().take(50)
                    .filter_map(|h| Some((h[0].as_str()?.to_string(), h[1].as_str()?.to_string()))).collect();
                let got = core::plugins::http_for(hosts, r["url"].as_str().unwrap_or(""), r["method"].as_str().unwrap_or("GET"), headers, r["body"].as_str().map(String::from)).await;
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
        if msgs.iter().any(|m| matches!(m, Msg::Stop)) { break; }
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
                    Msg::Stop => Ok(()),
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
    // (the app's handler is one for the whole process: tests that set it take turns)
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> { TURN.lock().unwrap_or_else(|e| e.into_inner()) }
    use super::*;

    fn pkg(code: &str) -> core::plugins::Package {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        core::plugins::Package {
            // (each its own id: the tests run side by side, and starting or stopping one stops another by its id)
            manifest: core::plugins::Manifest { id: format!("test-plug-{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)), name: "Test".into(), main: "main.js".into(), ..Default::default() },
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
        stop(&p.id);
    }

    #[test]
    fn it_can_reach_nothing_but_its_fetch() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        // no window, no network of its own
        let p = start(&pkg(r#"OmniPulse.register({ search() { return [typeof fetch, typeof XMLHttpRequest, typeof require]; }, resolve() { return null; } });"#), &json!({}), rt.handle().clone()).unwrap();
        assert_eq!(rt.block_on(p.call("search", vec![])).unwrap(), json!(["undefined", "undefined", "undefined"]));
        stop(&p.id);
    }

    #[test]
    fn timers_run_and_calls_wait_for_them() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let p = start(&pkg(r#"let n = 0; const t = setInterval(() => { n++; if (n >= 3) clearInterval(t); }, 20);
            OmniPulse.register({ search() { return new Promise((done) => setTimeout(() => done([n]), 150)); }, resolve() { return null; } });"#), &json!({}), rt.handle().clone()).unwrap();
        assert_eq!(rt.block_on(p.call("search", vec![])).unwrap(), json!([3]));
        stop(&p.id);
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
        stop(&p.id);
        // (and a socket to a site it doesn't list, refused even with the capability)
        assert!(!socket_allowed(&["irc-ws.chat.twitch.tv".into()], "wss://example.com/"));
        assert!(socket_allowed(&["irc-ws.chat.twitch.tv".into()], "wss://irc-ws.chat.twitch.tv:443"));
        assert!(socket_allowed(&["127.0.0.1".into()], "ws://127.0.0.1:4455"));
        assert!(!socket_allowed(&["example.com".into()], "ws://example.com/"));
        // (Twitch moving its events to another of its servers: allowed by the chatbot's own list)
        let m: core::plugins::Manifest = serde_json::from_str(&std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/chatbot/manifest.json")).unwrap()).unwrap();
        assert!(socket_allowed(&m.hosts, "wss://cell-a.eventsub.wss.twitch.tv/ws?challenge=x&id=y"));
    }

    /// Stopped (as Save restarts it): it really stops, its socket closed, though the socket held its inbox; not
    /// a second one left reading and answering chat beside the new one
    #[test]
    fn a_stopped_plugin_closes_its_sockets() {
        use futures_util::StreamExt;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let sock = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let port = sock.local_addr().unwrap().port();
        let closed = Arc::new(Mutex::new(false));
        let c2 = closed.clone();
        rt.spawn(async move {
            let (tcp, _) = sock.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            while let Some(Ok(m)) = ws.next().await { if m.is_close() { break; } }
            *c2.lock().unwrap() = true;
        });
        let code = format!(r#"OmniPulse.register({{ start() {{ OmniPulse.socket("ws://127.0.0.1:{port}/"); setInterval(() => {{}}, 1000); return true; }}, search() {{ return []; }}, resolve() {{ return null; }} }});"#);
        let mut k = pkg(&code);
        k.manifest.id = "test-stop".into();
        k.manifest.capabilities = vec!["sockets".into()];
        k.manifest.hosts = vec!["127.0.0.1".into()];
        start(&k, &json!({}), rt.handle().clone()).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        assert!(!*closed.lock().unwrap());
        stop("test-stop");
        let until = Instant::now() + Duration::from_secs(5);
        while !*closed.lock().unwrap() && Instant::now() < until { std::thread::sleep(Duration::from_millis(50)); }
        assert!(*closed.lock().unwrap(), "its socket stayed open after it was stopped");
    }

    /// The chatbot plugin (plugins/chatbot) against a stand-in Twitch on this PC: its chat (IRC over a
    /// WebSocket) and its events (EventSub's). A request and a command answered in chat, a follow an alert.
    #[test]
    fn the_chatbot_answers_chat_and_turns_events_into_alerts() {
        let _one = one_at_a_time();
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

    /// Twitch shows as connected as soon as its chat welcomes the chatbot (001), with no message in chat yet;
    /// and with a bot account too, which has its own connection
    #[test]
    fn the_chatbot_is_connected_before_anyone_chats() {
        let _one = one_at_a_time();
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let port = listener.local_addr().unwrap().port();
        rt.spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let Ok(ws) = tokio_tungstenite::accept_async(tcp).await else { continue };
                let (mut w, mut r) = ws.split();
                tokio::spawn(async move {
                    while let Some(Ok(Message::Text(t))) = r.next().await {
                        for line in t.split("\r\n").filter(|l| !l.is_empty()) {
                            let nick = line.strip_prefix("NICK ").map(String::from);
                            if let Some(n) = nick {
                                let _ = w.send(Message::Text(format!(":tmi.twitch.tv CAP * ACK :twitch.tv/tags twitch.tv/commands\r\n:tmi.twitch.tv 001 {n} :Welcome, GLHF!\r\n:tmi.twitch.tv 002 {n} :Your host is tmi.twitch.tv\r\n").into())).await;
                            }
                            if let Some(ch) = line.strip_prefix("JOIN ") { let _ = w.send(Message::Text(format!(":me!me@me.tmi.twitch.tv JOIN {ch}\r\n@emote-only=0 :tmi.twitch.tv ROOMSTATE {ch}\r\n").into())).await; }
                        }
                    }
                });
            }
        });
        set_app_handler(Arc::new(move |_p: &str, _ask: Value, answer: Box<dyn FnOnce(Value) + Send>| answer(json!({ "ok": true }))));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/chatbot");
        let code = std::fs::read_to_string(root.join("main.js")).unwrap()
            .replace("wss://irc-ws.chat.twitch.tv:443", &format!("ws://127.0.0.1:{port}/irc"))
            .replace("wss://eventsub.wss.twitch.tv/ws", &format!("ws://127.0.0.1:{port}/events"))
            .replace("if (await validate(token)) return token;", "return token;");
        let mut m: core::plugins::Manifest = serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
        m.hosts.push("127.0.0.1".into());
        let pkg = core::plugins::Package { manifest: m, code, digest: String::new(), bundled: true };
        let p = start(&pkg, &json!({ "token": "t", "login": "me", "userId": "1", "twitchClientId": "c", "botToken": "b", "botLogin": "mybot" }), rt.handle().clone()).unwrap();
        let until = Instant::now() + Duration::from_secs(8);
        let mut st = Value::Null;
        while Instant::now() < until {
            st = rt.block_on(p.call("status", vec![])).unwrap_or(Value::Null);
            if st["twitch"]["up"] == true { break; }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(st["twitch"]["up"], true, "{st:#}");
        // (and it stays connected: a second look, a few seconds on)
        std::thread::sleep(Duration::from_secs(3));
        let st = rt.block_on(p.call("status", vec![])).unwrap_or(Value::Null);
        assert_eq!(st["twitch"]["up"], true, "{st:#}");
        drop(p);
        stop("chatbot");
    }

    /// The chatbot's sign-in to OBS (its script's own SHA-256 and base64), as OBS works it out
    #[test]
    fn the_chatbot_signs_in_to_obs_as_obs_expects() {
        let _one = one_at_a_time();
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
        let _one = one_at_a_time();
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

    /// The chatbot reading a real Kick channel's chat (OMNIPULSE_KICK_LIVE=<channel>; nothing's sent, no
    /// sign-in): messages taken in as chat, an action run for each
    #[test]
    #[ignore]
    fn the_chatbot_reads_real_kick() {
        let Ok(channel) = std::env::var("OMNIPULSE_KICK_LIVE") else { return };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let asked: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let a2 = asked.clone();
        set_app_handler(Arc::new(move |_p: &str, ask: Value, answer: Box<dyn FnOnce(Value) + Send>| { a2.lock().unwrap().push(ask.clone()); answer(json!({ "ok": true })); }));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/chatbot");
        let m: core::plugins::Manifest = serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
        let pkg = core::plugins::Package { manifest: m, code: std::fs::read_to_string(root.join("main.js")).unwrap(), digest: String::new(), bundled: true };
        let t = crate::flows::defaults(crate::flows::TRIGGERS, "message");
        let mut t = t; t["match"] = json!("regex"); t["text"] = json!("."); t["from"] = json!("kick");
        let mut st = crate::flows::defaults(crate::flows::STEPS, "status"); st["text"] = json!("%user%: %message%");
        let p = start(&pkg, &json!({ "kickOn": true, "kickChannel": channel, "actions": [{ "name": "Seen", "on": true, "trigger": t, "steps": [st] }] }), rt.handle().clone()).unwrap();
        let until = Instant::now() + Duration::from_secs(30);
        let said = || asked.lock().unwrap().iter().filter(|a| a["op"] == "say").map(|a| a["args"][0].as_str().unwrap_or("").to_string()).collect::<Vec<_>>();
        while said().len() < 5 && Instant::now() < until { std::thread::sleep(Duration::from_millis(200)); }
        let s = said();
        println!("{} messages: {:#?}", s.len(), s.iter().take(5).collect::<Vec<_>>());
        assert!(s.len() >= 5, "{s:#?}");
        drop(p);
        stop("chatbot");
    }

    /// A program starting (the app tells the chatbot its name): its action runs, the name compared as Task
    /// Manager's (any case, with or without .exe or its folder)
    #[test]
    fn a_program_starting_runs_its_action() {
        let _one = one_at_a_time();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let asked: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let a2 = asked.clone();
        set_app_handler(Arc::new(move |_p: &str, ask: Value, answer: Box<dyn FnOnce(Value) + Send>| { a2.lock().unwrap().push(ask.clone()); answer(json!({ "ok": true })); }));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/chatbot");
        let m: core::plugins::Manifest = serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
        let pkg = core::plugins::Package { manifest: m, code: std::fs::read_to_string(root.join("main.js")).unwrap(), digest: String::new(), bundled: true };
        let mut t = crate::flows::defaults(crate::flows::TRIGGERS, "process"); t["name"] = json!("destiny2");
        let mut st = crate::flows::defaults(crate::flows::STEPS, "status"); st["text"] = json!("Started %process%");
        let p = start(&pkg, &json!({ "actions": [{ "name": "Game", "on": true, "trigger": t, "steps": [st] }] }), rt.handle().clone()).unwrap();
        rt.block_on(p.call("appEvent", vec![json!("process"), json!({ "name": "Destiny2.exe", "path": "C:/Games/Destiny2.exe" })])).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        let said = || asked.lock().unwrap().iter().any(|a| a["op"] == "say" && a["args"][0].as_str().is_some_and(|s| s.contains("Started")));
        while !said() && Instant::now() < until { std::thread::sleep(Duration::from_millis(50)); }
        let a = asked.lock().unwrap().clone();
        stop(&p.id);
        assert!(said(), "{a:#?}");
    }

    /// The test chat: off, a command's steps only described; Run for Real, they run (an alert shown), its reply
    /// still kept to the test chat, and a ban on the one trying it still only described
    #[test]
    fn the_test_chat_runs_for_real() {
        let _one = one_at_a_time();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let asked: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let a2 = asked.clone();
        set_app_handler(Arc::new(move |_p: &str, ask: Value, answer: Box<dyn FnOnce(Value) + Send>| { a2.lock().unwrap().push(ask.clone()); answer(json!({ "ok": true })); }));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/chatbot");
        let m: core::plugins::Manifest = serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
        let pkg = core::plugins::Package { manifest: m, code: std::fs::read_to_string(root.join("main.js")).unwrap(), digest: String::new(), bundled: true };
        let mut t = crate::flows::defaults(crate::flows::TRIGGERS, "command"); t["command"] = json!("!boom"); t["cooldown"] = json!(0); t["usercooldown"] = json!(0);
        let mut alert = crate::flows::defaults(crate::flows::STEPS, "alert"); alert["user"] = json!("Tester");
        let mut say = crate::flows::defaults(crate::flows::STEPS, "say"); say["text"] = json!("Boom!");
        let ban = crate::flows::defaults(crate::flows::STEPS, "ban");
        let p = start(&pkg, &json!({ "actions": [{ "name": "Boom", "on": true, "trigger": t, "steps": [alert, say, ban] }] }), rt.handle().clone()).unwrap();
        let alerts = || asked.lock().unwrap().iter().filter(|a| a["op"] == "alert").count();
        let dry = rt.block_on(p.call("tryChat", vec![json!("!boom"), json!("twitch"), json!(false), json!(false)])).unwrap();
        assert_eq!(alerts(), 0, "{dry:#?}");
        let real = rt.block_on(p.call("tryChat", vec![json!("!boom"), json!("twitch"), json!(false), json!(true)])).unwrap();
        let said: Vec<String> = real.as_array().unwrap().iter().filter_map(|x| x.as_str().map(String::from)).collect();
        stop(&p.id);
        assert_eq!(alerts(), 1, "{said:#?}");
        assert!(said.iter().any(|s| s == "Boom!") && said.iter().any(|s| s.contains("ban")), "{said:#?}");
    }

    /// The chatbot on the real Twitch chat, signed in as Twitch's anonymous reader (justinfan, no account;
    /// OMNIPULSE_TWITCH_LIVE=1): its status says it's connected, with no chat needed first
    #[test]
    #[ignore]
    fn the_chatbot_connects_to_real_twitch() {
        if std::env::var_os("OMNIPULSE_TWITCH_LIVE").is_none() { return; }
        let _one = one_at_a_time();
        let rt = tokio::runtime::Runtime::new().unwrap();
        set_app_handler(Arc::new(move |_p: &str, _ask: Value, answer: Box<dyn FnOnce(Value) + Send>| answer(json!({ "ok": true }))));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/chatbot");
        let m: core::plugins::Manifest = serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
        // (no sign-in to check, and no events: only chat)
        let code = std::fs::read_to_string(root.join("main.js")).unwrap()
            .replace("if (await validate(token)) return token;", "return token;")
            .replace("events = connectEvents(token);", "");
        let pkg = core::plugins::Package { manifest: m, code, digest: String::new(), bundled: true };
        let p = start(&pkg, &json!({ "token": "anonymous", "login": "justinfan54321", "userId": "", "twitchClientId": "c" }), rt.handle().clone()).unwrap();
        let until = Instant::now() + Duration::from_secs(15);
        let mut st = Value::Null;
        while Instant::now() < until {
            st = rt.block_on(p.call("status", vec![])).unwrap_or(Value::Null);
            if st["twitch"]["up"] == json!(true) { break; }
            std::thread::sleep(Duration::from_millis(300));
        }
        stop(&p.id);
        assert_eq!(st["twitch"]["up"], json!(true), "{st:#?}");
    }

    /// The chatbot on the real Kick, as a guest (OMNIPULSE_KICK_LIVE=<channel>, no sign-in): its status says
    /// it's connected, with no chat needed first
    #[test]
    #[ignore]
    fn the_chatbot_connects_to_real_kick() {
        let Ok(channel) = std::env::var("OMNIPULSE_KICK_LIVE") else { return };
        let _one = one_at_a_time();
        let rt = tokio::runtime::Runtime::new().unwrap();
        set_app_handler(Arc::new(move |_p: &str, _ask: Value, answer: Box<dyn FnOnce(Value) + Send>| answer(json!({ "ok": true }))));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/chatbot");
        let m: core::plugins::Manifest = serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
        let pkg = core::plugins::Package { manifest: m, code: std::fs::read_to_string(root.join("main.js")).unwrap(), digest: String::new(), bundled: true };
        let p = start(&pkg, &json!({ "kickOn": true, "kickChannel": channel }), rt.handle().clone()).unwrap();
        let until = Instant::now() + Duration::from_secs(15);
        let mut st = Value::Null;
        while Instant::now() < until {
            st = rt.block_on(p.call("status", vec![])).unwrap_or(Value::Null);
            if st["kick"]["up"] == json!(true) { break; }
            std::thread::sleep(Duration::from_millis(300));
        }
        stop(&p.id);
        assert_eq!(st["kick"]["up"], json!(true), "{st:#?}");
    }

    /// The chatbot on Kick, against a stand-in Kick on this PC: its channel looked up, its realtime socket
    /// (Centrifugo's protocol: a guest's token, the chatroom subscribed, its keep-alive answered), a request
    /// from its chat answered through its API (as a reply), an action for Kick alone, our own message's
    /// echo left alone, a sub an alert and an action
    #[test]
    fn the_chatbot_works_kick() {
        let _one = one_at_a_time();
        use futures_util::{SinkExt, StreamExt};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio_tungstenite::tungstenite::Message;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let web = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let sock = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let (wp, sp) = (web.local_addr().unwrap().port(), sock.local_addr().unwrap().port());
        // (its web side: the channel, the socket's address and a token, and its chat API, what's posted kept)
        let posted: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let helixed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (p2, x2) = (posted.clone(), helixed.clone());
        rt.spawn(async move {
            while let Ok((mut tcp, _)) = web.accept().await {
                let (posted, helixed) = (p2.clone(), x2.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        let n = tcp.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 { break; }
                        buf.extend_from_slice(&chunk[..n]);
                        let text = String::from_utf8_lossy(&buf).to_string();
                        if let Some(at) = text.find("\r\n\r\n") {
                            let len = text.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                            if buf.len() >= at + 4 + len { break; }
                        }
                    }
                    let text = String::from_utf8_lossy(&buf).to_string();
                    let line = text.lines().next().unwrap_or("").to_string();
                    let body = text.split_once("\r\n\r\n").map(|x| x.1.to_string()).unwrap_or_default();
                    if line.contains("/v1/moderation") || line.starts_with("DELETE /v1/chat") { helixed.lock().unwrap().push(format!("{} {}", line.split(' ').take(2).collect::<Vec<_>>().join(" "), body)); }
                    let answer = if line.contains("/helix/") {
                        helixed.lock().unwrap().push(format!("{} {}", line.split(' ').take(2).collect::<Vec<_>>().join(" "), body));
                        if line.contains("/helix/users") { json!({ "data": [{ "id": "99", "display_name": "Baddie" }] }) } else { json!({ "data": [] }) }
                    } else if line.contains("/channels/") { json!({ "id": 5, "user_id": 7, "chatroom": { "id": 9 } }) }
                        else if line.contains("/realtime/connection") { json!({ "data": { "connections": [{ "credentials": { "url": format!("ws://127.0.0.1:{sp}/ws") }, "provider": "centrifugo" }] } }) }
                        else if line.contains("/realtime/auth/connection") { json!({ "data": { "token": "guest" } }) }
                        else if line.contains("/v1/chat") {
                            let mut b: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                            b["as"] = json!(text.lines().find_map(|l| l.strip_prefix("Authorization: Bearer ").or_else(|| l.strip_prefix("authorization: Bearer "))).unwrap_or("").trim());
                            posted.lock().unwrap().push(b);
                            json!({ "data": {} })
                        }
                        else { json!({}) };
                    let b = answer.to_string();
                    let _ = tcp.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len()).as_bytes()).await;
                });
            }
        });
        // (its socket: connect and subscribes answered, its keep-alive sent, then chat and a sub)
        let heard: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let h2 = heard.clone();
        rt.spawn(async move {
            let (tcp, _) = sock.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let (mut w, mut r) = ws.split();
            let push = |ev: &str, data: Value| json!({ "push": { "channel": "chatrooms.9.v2", "pub": { "data": { "event": format!("App\\Events\\{ev}"), "data": data.to_string() } } } }).to_string();
            let chat = |id: &str, who: &str, badge: &str, text: &str| push("ChatMessageEvent", json!({ "id": id, "chatroom_id": 9, "content": text, "type": "message", "sender": { "id": 42, "username": who, "slug": who.to_lowercase(), "identity": { "badges": [{ "type": badge }] } } }));
            let mut subscribed = 0;
            while let Some(Ok(Message::Text(t))) = r.next().await {
                for line in t.split('\n').filter(|l| !l.is_empty()) {
                    let m: Value = serde_json::from_str(line).unwrap_or(Value::Null);
                    h2.lock().unwrap().push(m.clone());
                    if m["connect"].is_object() { let _ = w.send(Message::Text(r#"{"id":1,"connect":{"client":"c","ttl":2700,"ping":25}}"#.into())).await; }
                    if m["subscribe"].is_object() {
                        let _ = w.send(Message::Text(format!(r#"{{"id":{},"subscribe":{{}}}}"#, m["id"]).into())).await;
                        subscribed += 1;
                        if subscribed == 2 {
                            let _ = w.send(Message::Text("{}".into())).await;
                            let lines = [
                                chat("k1", "Viewer", "subscriber", "!sr take on me"),
                                chat("k2", "Viewer", "subscriber", "!hello"),
                                chat("k3", "MeBot", "broadcaster", "Hi Viewer on kick"),
                                chat("k4", "TheBot", "", "!hello"),
                                push("SubscriptionEvent", json!({ "chatroom_id": 9, "username": "NewSub", "months": 1 })),
                            ];
                            for l in lines { let _ = w.send(Message::Text(l.into())).await; }
                        }
                    }
                }
            }
        });
        let asked: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let a2 = asked.clone();
        set_app_handler(Arc::new(move |_p: &str, ask: Value, answer: Box<dyn FnOnce(Value) + Send>| {
            a2.lock().unwrap().push(ask.clone());
            answer(match ask["op"].as_str() { Some("request") => json!({ "ok": true, "message": "Added Take On Me." }), _ => json!({ "ok": true }) });
        }));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/chatbot");
        let code = std::fs::read_to_string(root.join("main.js")).unwrap()
            .replace("https://kick.com/api/v2/channels/", &format!("http://127.0.0.1:{wp}/channels/"))
            .replace("https://web.kick.com/api/v1/realtime/", &format!("http://127.0.0.1:{wp}/realtime/"))
            .replace("https://api.kick.com/public/v1/", &format!("http://127.0.0.1:{wp}/v1/"))
            .replace("https://api.twitch.tv/helix/", &format!("http://127.0.0.1:{wp}/helix/"));
        let mut m: core::plugins::Manifest = serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
        m.hosts.push("127.0.0.1".into());
        let pkg = core::plugins::Package { manifest: m, code, digest: String::new(), bundled: true };
        let t = |id: &str, set: Value| { let mut d = crate::flows::defaults(crate::flows::TRIGGERS, id); for (k, v) in set.as_object().unwrap() { d[k] = v.clone(); } d };
        let st = |id: &str, set: Value| { let mut d = crate::flows::defaults(crate::flows::STEPS, id); for (k, v) in set.as_object().unwrap() { d[k] = v.clone(); } d };
        let actions = json!([
            { "name": "Hello", "on": true, "trigger": t("command", json!({ "command": "!hello", "cooldown": 0, "usercooldown": 0, "from": "kick" })), "steps": [st("say", json!({ "text": "Hi %user% on %platform%" }))] },
            { "name": "Twitch Only", "on": true, "trigger": t("command", json!({ "command": "!hello", "cooldown": 0, "usercooldown": 0, "from": "twitch" })), "steps": [st("say", json!({ "text": "never" }))] },
            { "name": "Sub", "on": true, "trigger": t("event", json!({ "event": "sub" })), "steps": [st("say", json!({ "text": "Thanks %user%", "reply": false }))] },
            { "name": "Echo", "on": true, "trigger": t("message", json!({ "text": "hi viewer" })), "steps": [st("say", json!({ "text": "looped", "reply": false }))] },
            { "name": "Manage", "on": true, "trigger": t("manual", json!({})), "steps": [
                st("poll", json!({ "title": "Best?", "choices": "Cats | Dogs", "seconds": 30 })),
                st("ban", json!({ "who": "baddie", "reason": "spam" })),
                st("chatmode", json!({ "mode": "slow", "seconds": 20 })),
                st("discord", json!({ "webhook": "https://discord.com/api/webhooks/1/x", "text": "Live!" })),
                st("speak", json!({ "text": "Hello %user%", "voice": "Zira", "rate": 9, "volume": 50 })),
                st("overlay", json!({ "text": "Wow", "image": "\"C:/pics/a.png\"", "place": "bottom", "seconds": 4 })),
            ] },
            { "name": "Hook", "on": true, "trigger": t("web", json!({ "name": "deploy" })), "steps": [st("say", json!({ "text": "Web %user% %rawInput%", "reply": false }))] },
        ]);
        let p = start(&pkg, &json!({ "kickOn": true, "kickChannel": "me", "kickToken": "kt", "kickUserId": "7", "kickLogin": "MeBot", "kickBotToken": "bt", "kickBotLogin": "TheBot", "userId": "1", "twitchClientId": "c", "actions": actions }), rt.handle().clone()).unwrap();
        let said = |text: &str| posted.lock().unwrap().iter().any(|b| b["content"] == text);
        let until = Instant::now() + Duration::from_secs(10);
        let done = || said("Added Take On Me.") && said("Hi Viewer on kick") && said("Thanks NewSub") && asked.lock().unwrap().iter().any(|a| a["op"] == "alert" && a["args"][0]["user"] == "NewSub");
        while !done() && Instant::now() < until { std::thread::sleep(Duration::from_millis(50)); }
        let (b, a, h) = (posted.lock().unwrap().clone(), asked.lock().unwrap().clone(), heard.lock().unwrap().clone());
        assert!(done(), "posted {b:#?}, asked {a:#?}, heard {h:#?}");
        // (its status: Kick connected)
        let st = rt.block_on(p.call("status", vec![])).unwrap();
        assert_eq!(st["kick"]["up"], json!(true), "{st:#?}");
        // (the request as a Kick one, its answer a reply to its message, to the channel)
        assert!(a.iter().any(|x| x["op"] == "request" && x["args"] == json!(["Viewer", "take on me", "kick"])), "{a:#?}");
        assert!(b.iter().any(|x| x["content"] == "Added Take On Me." && x["reply_to_message_id"] == "k1" && x["broadcaster_user_id"] == 7), "{b:#?}");
        // (posted as the bot account, to your channel; the bot's own !hello not answered)
        assert!(b.iter().all(|x| x["as"] == "bt"), "{b:#?}");
        assert_eq!(b.iter().filter(|x| x["content"] == "Hi Viewer on kick").count(), 1, "{b:#?}");
        // (Twitch's action not on Kick; our own echo not taken as chat; the keep-alive answered; the chatroom subscribed)
        assert!(!said("never") && !said("looped"), "{b:#?}");
        assert!(h.iter().any(|x| x == &json!({})), "{h:#?}");
        assert!(h.iter().any(|x| x["subscribe"]["channel"] == "chatrooms.9.v2") && h.iter().any(|x| x["connect"]["token"] == "guest"), "{h:#?}");
        // (Twitch's management steps: as Twitch's API has them; Discord's webhook through the app)
        let ran = rt.block_on(p.call("runAction", vec![json!("Manage")])).unwrap();
        assert_eq!(ran, json!("Ran Manage."));
        let web = rt.block_on(p.call("appEvent", vec![json!("web"), json!({ "name": "Deploy", "user": "Tool", "input": "now please" })])).unwrap();
        assert_eq!(web, json!("Ran Hook."));
        let until = Instant::now() + Duration::from_secs(5);
        while (helixed.lock().unwrap().len() < 4 || !said("Web Tool now please")) && Instant::now() < until { std::thread::sleep(Duration::from_millis(50)); }
        let x = helixed.lock().unwrap().clone();
        let has = |want: &str| x.iter().any(|l| l.contains(want));
        assert!(has("POST /helix/polls") && has("\"title\":\"Best?\"") && has("\"duration\":30"), "{x:#?}");
        assert!(has("POST /helix/moderation/bans?broadcaster_id=1&moderator_id=1") && has("\"user_id\":\"99\""), "{x:#?}");
        assert!(has("PATCH /helix/chat/settings") && has("\"slow_mode_wait_time\":20"), "{x:#?}");
        assert!(asked.lock().unwrap().iter().any(|a| a["op"] == "postJson" && a["args"][1]["content"] == "Live!"));
        // (said out loud, and shown on stream: through the app, as it wants them)
        assert!(asked.lock().unwrap().iter().any(|a| a["op"] == "speak" && a["args"][1] == "Zira" && a["args"][2] == 3), "{:#?}", asked.lock().unwrap());
        assert!(asked.lock().unwrap().iter().any(|a| a["op"] == "overlay" && a["args"][0]["image"] == "C:/pics/a.png" && a["args"][0]["place"] == "bottom" && a["args"][0]["seconds"] == 4));
        assert!(said("Web Tool now please"), "{:#?}", posted.lock().unwrap());
        // (OBS's docks: Kick's chat to the chat dock, its sub to the activity feed with its alert; the
        // dock's buttons as Kick's and Twitch's APIs have them)
        {
            let a = asked.lock().unwrap().clone();
            assert!(a.iter().any(|x| x["op"] == "dockChat" && x["args"][0]["platform"] == "kick" && x["args"][0]["text"] == "!hello" && x["args"][0]["id"] == "k2" && x["args"][0]["userId"] == "42"), "{a:#?}");
            assert!(a.iter().any(|x| x["op"] == "dockActivity" && x["args"][0]["type"] == "sub" && x["args"][0]["user"] == "NewSub" && x["args"][0]["alert"]["type"] == "sub" && x["args"][0]["chatbot"] == true), "{a:#?}");
            assert!(a.iter().any(|x| x["op"] == "alert" && x["args"][0]["user"] == "NewSub" && x["args"][0]["feed"] == false), "{a:#?}");
        }
        for cmd in [
            json!({ "do": "delete", "platform": "kick", "id": "k2", "user": "Viewer", "userId": "42" }),
            json!({ "do": "timeout", "platform": "kick", "id": "k2", "user": "Viewer", "userId": "42", "seconds": 600 }),
            json!({ "do": "ban", "platform": "twitch", "id": "t1", "user": "Baddie", "userId": "77" }),
            json!({ "do": "unban", "platform": "twitch", "user": "Baddie", "login": "baddie" }),
            // (Clear Chat: no platform of its own; Twitch's and Kick's both)
            json!({ "do": "clearchat" }),
        ] {
            let r = rt.block_on(p.call("dock", vec![cmd.clone()]));
            assert!(r.is_ok(), "{cmd} {r:?}");
        }
        let said_dock = rt.block_on(p.call("dock", vec![json!({ "do": "say", "text": "from the dock", "platform": "kick" })]));
        assert_eq!(said_dock, Ok(json!("")));
        let ran = rt.block_on(p.call("dock", vec![json!({ "do": "rerun", "event": { "type": "sub", "user": "NewSub", "platform": "kick", "chatbot": true } })])).unwrap();
        assert_eq!(ran, json!("Ran sub's actions again for NewSub."));
        let until = Instant::now() + Duration::from_secs(5);
        while (!said("from the dock") || posted.lock().unwrap().iter().filter(|b| b["content"] == "Thanks NewSub").count() < 2) && Instant::now() < until { std::thread::sleep(Duration::from_millis(50)); }
        let x = helixed.lock().unwrap().clone();
        let has = |want: &str| x.iter().any(|l| l.contains(want));
        assert!(has("DELETE /v1/chat/k2"), "{x:#?}");
        assert!(has("POST /v1/moderation/bans") && has("\"duration\":10") && has("\"user_id\":42"), "{x:#?}");
        assert!(has("POST /helix/moderation/bans?broadcaster_id=1&moderator_id=1") && has("\"user_id\":\"77\"") && has("DELETE /helix/moderation/bans?broadcaster_id=1&moderator_id=1&user_id=99"), "{x:#?}");
        assert!(said("from the dock"), "{:#?}", posted.lock().unwrap());
        // (Kick only: Twitch isn't signed in here)
        assert!(said("/clear"), "{:#?}", posted.lock().unwrap());
        // (run again: its thanks said again, and not put in the feed twice)
        assert_eq!(posted.lock().unwrap().iter().filter(|b| b["content"] == "Thanks NewSub").count(), 2);
        assert_eq!(asked.lock().unwrap().iter().filter(|x| x["op"] == "dockActivity" && x["args"][0]["user"] == "NewSub").count(), 1);
        drop(p);
        stop("chatbot");
    }

    /// Donations, from stand-ins for Streamlabs' socket (socket.io: opened, pinged, an event) and StreamElements'
    /// (subscribed to tips with its token, then a tip): each an alert, and A Donation's action run
    #[test]
    fn the_chatbot_hears_donations() {
        let _one = one_at_a_time();
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let sl = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let se = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let (slp, sep) = (sl.local_addr().unwrap().port(), se.local_addr().unwrap().port());
        let heard: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (h1, h2) = (heard.clone(), heard.clone());
        rt.spawn(async move {
            let (tcp, _) = sl.accept().await.unwrap();
            let (mut w, mut r) = tokio_tungstenite::accept_async(tcp).await.unwrap().split();
            let _ = w.send(Message::Text(r#"0{"sid":"x","pingInterval":200,"pingTimeout":5000}"#.into())).await;
            let _ = w.send(Message::Text("40".into())).await;
            let _ = w.send(Message::Text(r#"42["event",{"type":"follow","for":"twitch_account","message":[{"name":"NotATip"}]}]"#.into())).await;
            let _ = w.send(Message::Text(r#"42["event",{"type":"donation","for":"streamlabs","message":[{"name":"Jeff","amount":"13.37","currency":"USD","message":"for the tunes"}]}]"#.into())).await;
            while let Some(Ok(Message::Text(t))) = r.next().await { h1.lock().unwrap().push(format!("sl {t}")); }
        });
        rt.spawn(async move {
            let (tcp, _) = se.accept().await.unwrap();
            let (mut w, mut r) = tokio_tungstenite::accept_async(tcp).await.unwrap().split();
            while let Some(Ok(Message::Text(t))) = r.next().await {
                h2.lock().unwrap().push(format!("se {t}"));
                let m: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
                if m["type"] == "subscribe" && m["data"]["token"] == "jwt1" {
                    let _ = w.send(Message::Text(json!({ "type": "response", "nonce": m["nonce"], "data": { "message": "ok" } }).to_string().into())).await;
                    let _ = w.send(Message::Text(json!({ "type": "message", "topic": "channel.tips", "data": { "donation": { "user": { "username": "Ann" }, "amount": 5, "currency": "EUR", "message": "hi" } } }).to_string().into())).await;
                }
            }
        });
        let asked: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let a2 = asked.clone();
        set_app_handler(Arc::new(move |_p: &str, ask: Value, answer: Box<dyn FnOnce(Value) + Send>| { a2.lock().unwrap().push(ask); answer(json!({ "ok": true })); }));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/chatbot");
        let code = std::fs::read_to_string(root.join("main.js")).unwrap()
            .replace("wss://sockets.streamlabs.com/socket.io/", &format!("ws://127.0.0.1:{slp}/socket.io/"))
            .replace("wss://astro.streamelements.com", &format!("ws://127.0.0.1:{sep}/"));
        let mut m: core::plugins::Manifest = serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
        m.hosts.push("127.0.0.1".into());
        let pkg = core::plugins::Package { manifest: m, code, digest: String::new(), bundled: true };
        let mut trig = crate::flows::defaults(crate::flows::TRIGGERS, "event");
        trig["event"] = json!("donation");
        let mut show = crate::flows::defaults(crate::flows::STEPS, "overlay");
        show["text"] = json!("%user% %amount% %currency% %message%");
        let p = start(&pkg, &json!({ "streamlabsToken": "sl1", "streamelementsToken": "jwt1", "actions": [{ "name": "Tip", "on": true, "trigger": trig, "steps": [show] }] }), rt.handle().clone()).unwrap();
        let shown = |t: &str| asked.lock().unwrap().iter().any(|a| a["op"] == "overlay" && a["args"][0]["text"] == t);
        let until = Instant::now() + Duration::from_secs(5);
        while !(shown("Jeff 13.37 USD for the tunes") && shown("Ann 5 EUR hi") && heard.lock().unwrap().iter().any(|h| h == "sl 2")) && Instant::now() < until { std::thread::sleep(Duration::from_millis(50)); }
        let (a, h) = (asked.lock().unwrap().clone(), heard.lock().unwrap().clone());
        assert!(shown("Jeff 13.37 USD for the tunes") && shown("Ann 5 EUR hi"), "{a:#?} {h:#?}");
        assert!(h.iter().any(|x| x == "sl 2"), "pinged: {h:#?}");
        assert!(a.iter().any(|x| x["op"] == "alert" && x["args"][0]["type"] == "donation" && x["args"][0]["user"] == "Jeff" && x["args"][0]["amount"] == "USD 13.37"), "{a:#?}");
        assert!(!a.iter().any(|x| x["args"][0]["user"] == "NotATip" || x["args"][0]["text"].as_str().unwrap_or("").contains("NotATip")), "{a:#?}");
        drop(p);
        stop("chatbot");
    }

    /// Mix It Up's extras, against the stand-in Twitch chat: slots and a gamble (made sure to win), a duel
    /// accepted, a heist started, a rank, strikes stepping up, an entrance greeting, a file written, the credits
    #[test]
    fn the_chatbots_games_and_extras() {
        let _one = one_at_a_time();
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
                if path.lock().unwrap().as_str() != "/irc" { tokio::spawn(async move { while r.next().await.is_some() {} }); continue; }
                tokio::spawn(async move {
                    while let Some(Ok(Message::Text(t))) = r.next().await {
                        for line in t.split("\r\n").filter(|l| !l.is_empty()) {
                            said.lock().unwrap().push(line.to_string());
                            if line.starts_with("NICK") { let _ = w.send(Message::Text(":tmi.twitch.tv 001 me :Welcome, GLHF!\r\n".into())).await; }
                            if !line.starts_with("JOIN") { continue; }
                            let msg = |who: &str, id: &str, text: &str| format!("@badges=;display-name={who};id={id};mod=0;user-id=4{} :{l}!{l}@{l}.tmi.twitch.tv PRIVMSG #me :{text}\r\n", who.len(), l = who.to_lowercase());
                            for (who, id, text) in [("Viewer", "a", "!slots 100"), ("Viewer", "b", "!gamble 50"), ("Viewer", "c", "!duel Other 10"), ("Other", "d", "hello all"), ("Viewer", "e", "!rank"),
                                ("Viewer", "f", "!strike"), ("Viewer", "g", "!strike"), ("Viewer", "h", "!heist 20"), ("Other", "i", "!heist 30"), ("Viewer", "j", "!file")] {
                                let _ = w.send(Message::Text(msg(who, id, text).into())).await;
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                            let _ = w.send(Message::Text(msg("Other", "k", "!accept").into())).await;
                            let _ = w.send(Message::Text(msg("Viewer", "l", "!credits").into())).await;
                        }
                    }
                });
            }
        });
        let asked: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let a2 = asked.clone();
        set_app_handler(Arc::new(move |_p: &str, ask: Value, answer: Box<dyn FnOnce(Value) + Send>| { a2.lock().unwrap().push(ask); answer(json!({ "ok": true })); }));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/chatbot");
        let code = std::fs::read_to_string(root.join("main.js")).unwrap()
            .replace("wss://irc-ws.chat.twitch.tv:443", &format!("ws://127.0.0.1:{port}/irc"))
            .replace("wss://eventsub.wss.twitch.tv/ws", &format!("ws://127.0.0.1:{port}/events"))
            .replace("if (await validate(token)) return token;", "return token;")
            .replace("else await helix(\"POST\", \"moderation/bans?broadcaster_id=\" + S().userId + \"&moderator_id=\" + S().userId, { data: { user_id: await twitchUserId(who), duration", "else await OmniPulse.app.say(\"helix bans\") || ({ data: { user_id: 42, duration");
        let mut m: core::plugins::Manifest = serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
        m.hosts.push("127.0.0.1".into());
        let pkg = core::plugins::Package { manifest: m, code, digest: String::new(), bundled: true };
        let t = |id: &str, set: Value| { let mut d = crate::flows::defaults(crate::flows::TRIGGERS, id); for (k, v) in set.as_object().unwrap() { d[k] = v.clone(); } d };
        let st = |id: &str, set: Value| { let mut d = crate::flows::defaults(crate::flows::STEPS, id); for (k, v) in set.as_object().unwrap() { d[k] = v.clone(); } d };
        let cmd = |c: &str| t("command", json!({ "command": c, "cooldown": 0, "usercooldown": 0 }));
        let actions = json!([
            { "name": "Slots", "on": true, "trigger": cmd("!slots"), "steps": [st("slots", json!({ "symbols": "7" }))] },
            { "name": "Gamble", "on": true, "trigger": cmd("!gamble"), "steps": [st("gamble", json!({ "chance": 100, "payout": 3 })), st("say", json!({ "text": "won %won% %winnings%" }))] },
            { "name": "Duel", "on": true, "trigger": cmd("!duel"), "steps": [st("duel", json!({}))] },
            { "name": "Rank", "on": true, "trigger": cmd("!rank"), "steps": [st("say", json!({ "text": "rank %rank%" }))] },
            { "name": "Strike", "on": true, "trigger": cmd("!strike"), "steps": [st("strike", json!({}))] },
            { "name": "Heist", "on": true, "trigger": cmd("!heist"), "steps": [st("heist", json!({}))] },
            { "name": "File", "on": true, "trigger": cmd("!file"), "steps": [st("writefile", json!({ "file": "C:\\x\\c.txt", "text": "%user% %points%", "how": "append" }))] },
            { "name": "Credits", "on": true, "trigger": cmd("!credits"), "steps": [st("credits", json!({})), st("say", json!({ "text": "credits %credits%" }))] },
            { "name": "Entrance", "on": true, "trigger": t("event", json!({ "event": "firstwords", "users": "@other, someone" })), "steps": [st("say", json!({ "text": "Welcome %user%" }))] },
        ]);
        let p = start(&pkg, &json!({ "token": "t", "login": "me", "userId": "1", "twitchClientId": "c", "points": { "viewer": 1000, "other": 1000 }, "pointsRanks": "0 New, 900 Regular, 5000 Legend", "actions": actions }), rt.handle().clone()).unwrap();
        let has = |want: &str| said.lock().unwrap().iter().any(|l| l.contains(want));
        let until = Instant::now() + Duration::from_secs(8);
        // (rolled on the Credits widget: told to roll, its chatters in it (sent a moment after))
        let rolled = |x: &Value| x["op"] == "suiteWidget" && x["args"][0]["credits"]["rollAt"].as_f64().unwrap_or(0.0) > 0.0
            && x["args"][0]["credits"]["parts"].as_array().is_some_and(|p| p.iter().any(|p| p["name"] == "Chatters" && p["list"] == json!(["Viewer", "Other"])));
        while !(has("credits ") && has("beat") && asked.lock().unwrap().iter().any(rolled)) && Instant::now() < until { std::thread::sleep(Duration::from_millis(50)); }
        let (l, a) = (said.lock().unwrap().clone(), asked.lock().unwrap().clone());
        let saved = |k: &str| a.iter().rev().find(|x| x["op"] == "saveSettings" && x["args"][0][k].is_object()).map(|x| x["args"][0][k].clone()).unwrap_or(Value::Null);
        // (slots of one symbol: three the same, 10x; the gamble sure to win, 3x)
        assert!(has("[ 7 7 7 ] Viewer won 1000 points!") && has("won yes 150"), "{l:#?}");
        // (the duel asked and accepted: one of them won the other's 10)
        assert!(has("@Other, Viewer challenges you to a duel for 10 points! Type !accept within 30 seconds.") && has("beat"), "{l:#?}");
        // (Viewer's points by now: well over 900, a Regular)
        assert!(has("rank Regular"), "{l:#?}");
        // (strikes: a warning, then a time-out through Twitch's API)
        assert!(has("Viewer, that's strike 1. Please stop.") && a.iter().any(|x| x["op"] == "say" && x["args"][0] == "helix bans"), "{l:#?} {a:#?}");
        assert_eq!(saved("strikes")["viewer"]["n"], 2);
        // (the heist: started once, Other joined, not over yet)
        assert_eq!(l.iter().filter(|x| x.contains("is planning a heist")).count(), 1, "{l:#?}");
        // (an entrance greeting for Other only, the file, the credits rolled and on one line)
        assert!(has("Welcome Other") && !has("Welcome Viewer"), "{l:#?}");
        assert!(a.iter().any(|x| x["op"] == "writeFile" && x["args"][0] == "C:\\x\\c.txt" && x["args"][2] == true), "{a:#?}");
        assert!(a.iter().any(rolled), "{a:#?}");
        assert!(has("credits Chatters: Viewer, Other"), "{l:#?}");
        // (trying it, from the settings page: what it'd say and do, nothing sent or done)
        let said_before = said.lock().unwrap().len();
        let tried = rt.block_on(p.call("tryChat", vec![json!("!slots 5"), json!("twitch"), json!(false)])).unwrap();
        let tried: Vec<String> = tried.as_array().unwrap().iter().filter_map(|x| x.as_str().map(String::from)).collect();
        assert_eq!(tried, vec!["(would spin the slots)".to_string()], "{tried:?}");
        let tried = rt.block_on(p.call("tryChat", vec![json!("!rank"), json!("twitch"), json!(false)])).unwrap();
        assert_eq!(tried, json!(["rank New"]));
        assert_eq!(rt.block_on(p.call("tryChat", vec![json!("hello there"), json!("twitch"), json!(false)])).unwrap(), json!([]));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(said.lock().unwrap().len(), said_before, "nothing sent: {:#?}", &said.lock().unwrap()[said_before..]);
        // (what's going on: running, signed in, its counts and its last doings)
        let st = rt.block_on(p.call("status", vec![])).unwrap();
        assert_eq!((st["running"].as_bool(), st["twitch"]["login"].as_str(), st["twitch"]["up"].as_bool()), (Some(true), Some("me"), Some(true)), "{st:#}");
        assert!(st["counts"]["actions"].as_u64().unwrap_or(0) >= 5 && st["recent"].as_array().map(|r| !r.is_empty()).unwrap_or(false), "{st:#}");
        assert_eq!(st["actions"]["all"], 9);
        drop(p);
        stop("chatbot");
    }

    /// The chatbot's actions (crate::flows' JSON, run by the plugin), against the stand-in Twitch: a command
    /// that answers, a counter kept between uses, a chat message's, "only go on if" stopping one, a raid
    /// (with its least), and a song requested from a step
    #[test]
    fn the_chatbots_actions_run_their_steps() {
        let _one = one_at_a_time();
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
                                    let _ = w.send(Message::Text("@display-name=Viewer;user-id=42;msg-id=viewermilestone;msg-param-category=watch-streak;msg-param-value=7 :tmi.twitch.tv USERNOTICE #me :yay\r\n".into())).await;
                                    for (id, text) in [("a", "!hello"), ("b", "!deaths"), ("c", "!deaths"), ("d", "good morning all"), ("e", "!check no"), ("f", "!check yes"), ("g", "!play take on me"),
                                        ("h", "look at bad.example.com now"), ("i", "!addquote it works on my machine"), ("j", "!quote 1"), ("k", "!enter"), ("l", "!enter"), ("m", "!winner"), ("n", "!buy"), ("o", "!points"),
                                        ("p", "!grade 85"), ("q", "!grade 20"), ("r", "!count")] {
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
                    let _ = w.send(Message::Text(r#"{"metadata":{"message_type":"notification"},"payload":{"subscription":{"type":"channel.poll.end"},"event":{"title":"Best?","choices":[{"title":"Cats","votes":3},{"title":"Dogs","votes":9}]}}}"#.into())).await;
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
            { "name": "No Links", "on": true, "trigger": t("filter", json!({ "rule": "links", "exempt": "mods" })), "steps": [st("say", json!({ "text": "No links, %user%", "reply": false }))] },
            { "name": "Add Quote", "on": true, "trigger": t("command", json!({ "command": "!addquote", "cooldown": 0, "usercooldown": 0 })), "steps": [st("quoteadd", json!({})), st("say", json!({ "text": "Quote #%quotenumber% added", "reply": false }))] },
            { "name": "Quote", "on": true, "trigger": t("command", json!({ "command": "!quote", "cooldown": 0, "usercooldown": 0 })), "steps": [st("quote", json!({})), st("say", json!({ "text": "Q %quote%", "reply": false }))] },
            { "name": "Enter", "on": true, "trigger": t("command", json!({ "command": "!enter", "cooldown": 0, "usercooldown": 0 })), "steps": [st("giveenter", json!({}))] },
            { "name": "Winner", "on": true, "trigger": t("command", json!({ "command": "!winner", "cooldown": 0, "usercooldown": 0 })), "steps": [st("givepick", json!({})), st("say", json!({ "text": "Winner %winner% of %entries%", "reply": false }))] },
            { "name": "Buy", "on": true, "trigger": t("command", json!({ "command": "!buy", "cooldown": 0, "usercooldown": 0 })), "steps": [st("points", json!({ "amount": -50 })), st("say", json!({ "text": "Bought", "reply": false }))] },
            { "name": "Points", "on": true, "trigger": t("command", json!({ "command": "!points", "cooldown": 0, "usercooldown": 0 })), "steps": [st("say", json!({ "text": "%user% has %points% %pointsname%", "reply": false }))] },
            { "name": "Grade", "on": true, "trigger": t("command", json!({ "command": "!grade", "cooldown": 0, "usercooldown": 0 })), "steps": [
                st("ifblock", json!({ "left": "%input0%", "test": "more", "right": "50" })), st("say", json!({ "text": "Pass %input0%", "reply": false })),
                st("else", json!({})), st("say", json!({ "text": "Fail %input0%", "reply": false })), st("endif", json!({})),
                st("math", json!({ "expr": "round((%input0% + 0.4) * 2) / 2 + max(1, 2) ^ 2", "into": "sum" })), st("say", json!({ "text": "Sum %sum%", "reply": false })),
            ] },
            { "name": "Count", "on": true, "trigger": t("command", json!({ "command": "!count", "cooldown": 0, "usercooldown": 0 })), "steps": [
                st("repeat", json!({ "times": 3 })), st("say", json!({ "text": "Loop %loop%", "reply": false })), st("endrepeat", json!({})),
                st("text", json!({ "value": "hello big world", "do": "title", "into": "t" })), st("say", json!({ "text": "T %t%", "reply": false })),
                st("stop", json!({})), st("say", json!({ "text": "never stopped", "reply": false })),
            ] },
            { "name": "First", "on": true, "trigger": t("event", json!({ "event": "firstwords" })), "steps": [st("say", json!({ "text": "First words from %user%", "reply": false }))] },
            { "name": "Streak", "on": true, "trigger": t("event", json!({ "event": "watchstreak" })), "steps": [st("say", json!({ "text": "Streak %amount% for %user%", "reply": false }))] },
            { "name": "Poll", "on": true, "trigger": t("event", json!({ "event": "pollend" })), "steps": [st("say", json!({ "text": "Poll %message% won by %winner%", "reply": false }))] },
        ]);
        let p = start(&pkg, &json!({ "token": "t", "login": "me", "userId": "1", "twitchClientId": "c", "actions": actions, "points": { "viewer": 70 }, "pointsName": "coins" }), rt.handle().clone()).unwrap();
        let line = |text: &str| { let s = said.lock().unwrap(); s.iter().any(|l| l.ends_with(&format!(":{text}"))) };
        let until = Instant::now() + Duration::from_secs(10);
        let done = || line("Hi Viewer!") && line("Deaths: 2") && line("Morning, Viewer") && line("Checked: yes") && line("Raid of 30 from Big") && line("Added it.")
            && line("No links, Viewer") && line("Quote #1 added") && line("Winner Viewer of 1") && line("Bought") && line("Viewer has 20 coins")
            && line("Pass 85") && line("Fail 20") && line("Sum 89.5") && line("Loop 3") && line("T Hello Big World") && line("First words from Viewer")
            && line("Streak 7 for Viewer") && line("Poll Best? won by Dogs");
        while !done() && Instant::now() < until { std::thread::sleep(Duration::from_millis(50)); }
        let s = said.lock().unwrap().clone();
        assert!(done(), "chat said {s:#?}");
        // (stopped where it should: no "no", no small raid, nothing from the action that's off)
        assert!(!line("Checked: no") && !line("Raid of 2 from Small") && !line("never"), "{s:#?}");
        assert!(s.iter().any(|l| l.contains(":Q #1: \"it works on my machine\" (")), "{s:#?}");
        assert!(!line("Pass 20") && !line("Fail 85") && !line("never stopped"), "{s:#?}");
        // (first words: once, for the one viewer)
        assert_eq!(s.iter().filter(|l| l.ends_with(":First words from Viewer")).count(), 1, "{s:#?}");
        assert!(asked.lock().unwrap().iter().any(|x| x["op"] == "request" && x["args"] == json!(["Viewer", "take on me", "twitch"])));
        drop(p);
        stop("chatbot");
    }
}



