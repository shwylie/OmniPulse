// Plugins, run here (as the web app's js/plugins.js runs them in a sandboxed frame): each one in its
// own QuickJS engine on its own thread, with nothing it can reach but OmniPulse.fetch, which asks
// core::plugins::http (only to the sites its manifest lists, checked there). It gets a memory limit
// and each call a time limit; a plugin that runs on past it is stopped mid-way.
//
// The same API as the web app's: OmniPulse.register({ search, resolve, library? }), OmniPulse.fetch(url,
// { method, headers, body }) -> { ok, status, headers, text(), json() }, OmniPulse.settings,
// OmniPulse.onSettings(fn), OmniPulse.log(...).

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
    jobs: std::sync::mpsc::Sender<Job>,
}

struct Job {
    method: String,
    /// the arguments, as a JSON array
    args: String,
    reply: tokio::sync::oneshot::Sender<Result<Value, String>>,
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
        self.jobs.send(Job { method: method.to_string(), args: Value::Array(args).to_string(), reply }).map_err(|_| format!("{} has stopped.", self.name))?;
        got.await.map_err(|_| format!("{} has stopped.", self.name))?
    }
}

// What the plugin's script sees: the OmniPulse object, and how calls reach it
const BOOT: &str = r#"
"use strict";
var __impl = null, __settingsFns = [], __out = null, __waiting = {};
// a request's answer, from the app: its promise settles
var __host_done = (id, r) => { const w = __waiting[id]; if (!w) return; delete __waiting[id]; w(JSON.parse(r)); };
var OmniPulse = {
  settings: {},
  register(x) { __impl = x || {}; },
  async fetch(url, init) {
    init = init || {};
    const req = { url: String(url), method: init.method || "GET", headers: Object.entries(init.headers || {}).map(([k, v]) => [String(k), String(v)]), body: init.body == null ? null : String(init.body) };
    // (made by the app while this carries on, so several can be on their way at once)
    const id = __host_fetch(JSON.stringify(req));
    const r = await new Promise((done) => { __waiting[id] = done; });
    if (r.error) throw new Error(r.error);
    return { ok: r.status >= 200 && r.status < 300, status: r.status, headers: r.headers, text: async () => r.body, json: async () => JSON.parse(r.body) };
  },
  onSettings(fn) { __settingsFns.push(fn); },
  log(...a) { __host_log(a.map(String).join(" ")); },
};
var __methods = () => JSON.stringify(__impl ? Object.keys(__impl).filter((k) => typeof __impl[k] === "function") : []);
var __call = (m, args) => {
  __out = null;
  Promise.resolve()
    .then(() => { if (!__impl || typeof __impl[m] !== "function") throw new Error("this plugin has no " + m); return __impl[m](...JSON.parse(args)); })
    .then((r) => { __out = JSON.stringify({ ok: r === undefined ? null : r }); }, (e) => { __out = JSON.stringify({ error: String((e && e.message) || e) }); });
};
var __settings = (s) => { OmniPulse.settings = JSON.parse(s); __settingsFns.forEach((f) => { try { f(OmniPulse.settings); } catch (e) {} }); };
"#;

/// Starts an installed plugin with its settings; it's running once it has registered
pub fn start(pkg: &core::plugins::Package, settings: &Value, rt: tokio::runtime::Handle) -> Result<Arc<Plugin>, String> {
    let id = pkg.manifest.id.clone();
    stop(&id);
    let (jobs, inbox) = std::sync::mpsc::channel::<Job>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<Vec<String>, String>>();
    let (code, settings, pid) = (pkg.code.clone(), settings.to_string(), id.clone());
    std::thread::Builder::new()
        .name(format!("plugin {id}"))
        .spawn(move || run(pid, code, settings, rt, inbox, ready_tx))
        .map_err(|e| e.to_string())?;
    let methods = ready_rx.recv_timeout(Duration::from_secs(CALL_SECS)).map_err(|_| "it took too long to start".to_string())??;
    let p = Arc::new(Plugin { id: id.clone(), name: pkg.manifest.name.clone(), hosts: pkg.manifest.hosts.clone(), methods, jobs });
    RUNNING.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, p.clone());
    Ok(p)
}

/// The plugin's thread: its engine, then its calls one at a time
fn run(id: String, code: String, settings: String, rt: tokio::runtime::Handle, inbox: std::sync::mpsc::Receiver<Job>, ready: std::sync::mpsc::Sender<Result<Vec<String>, String>>) {
    let fail = |e: String| { let _ = ready.send(Err(e)); };
    let Ok(engine) = Runtime::new() else { return fail("its engine didn't start".into()) };
    engine.set_memory_limit(MEMORY);
    // a call (or the start) that runs on past its time is stopped
    let deadline: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(Some(Instant::now() + Duration::from_secs(CALL_SECS))));
    let d2 = deadline.clone();
    engine.set_interrupt_handler(Some(Box::new(move || d2.lock().unwrap().is_some_and(|d| Instant::now() > d))));
    let Ok(ctx) = Context::full(&engine) else { return fail("its engine didn't start".into()) };
    // the requests' answers as they come back (request id, answer), for the job loop to hand over
    let done: Arc<Mutex<Vec<(u32, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let started: Result<Vec<String>, String> = ctx.with(|ctx| {
        let g = ctx.globals();
        let pid = id.clone();
        let (d2, rt2) = (done.clone(), rt.clone());
        let next_id = std::sync::atomic::AtomicU32::new(0);
        let fetch = Function::new(ctx.clone(), move |req: String| -> u32 {
            let n = next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            let (pid, d) = (pid.clone(), d2.clone());
            rt2.spawn(async move {
                let r: Value = serde_json::from_str(&req).unwrap_or_default();
                let headers: Vec<(String, String)> = r["headers"].as_array().into_iter().flatten().take(50)
                    .filter_map(|h| Some((h[0].as_str()?.to_string(), h[1].as_str()?.to_string()))).collect();
                let got = core::plugins::http(&pid, r["url"].as_str().unwrap_or(""), r["method"].as_str().unwrap_or("GET"), headers, r["body"].as_str().map(String::from)).await;
                let out = match got { Ok(v) => v.to_string(), Err(e) => json!({ "error": e }).to_string() };
                d.lock().unwrap().push((n, out));
            });
            n
        }).map_err(|e| e.to_string())?;
        g.set("__host_fetch", fetch).map_err(|e| e.to_string())?;
        let lid = id.clone();
        g.set("__host_log", Function::new(ctx.clone(), move |s: String| { eprintln!("[plugin {lid}] {}", s.chars().take(2000).collect::<String>()); }).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        ctx.eval::<(), _>(BOOT).map_err(|e| format!("its setup failed: {e}"))?;
        let set: Function = g.get("__settings").map_err(|e| e.to_string())?;
        set.call::<_, ()>((settings,)).map_err(|e| e.to_string())?;
        ctx.eval::<(), _>(format!("(function (OmniPulse) {{\n{code}\n}})(OmniPulse);")).map_err(|e| format!("it didn't start: {}", caught(&ctx, e)))?;
        let methods: String = ctx.eval("__methods()").map_err(|e| e.to_string())?;
        Ok(serde_json::from_str(&methods).unwrap_or_default())
    });
    let ok = started.is_ok();
    let _ = ready.send(started);
    if !ok {
        return;
    }
    while let Ok(job) = inbox.recv() {
        *deadline.lock().unwrap() = Some(Instant::now() + Duration::from_secs(CALL_SECS));
        let began = ctx.with(|ctx| -> Result<(), String> {
            let call: Function = ctx.globals().get("__call").map_err(|e| e.to_string())?;
            call.call::<_, ()>((job.method.clone(), job.args.clone())).map_err(|e| caught(&ctx, e))
        });
        let answer = began.and_then(|()| loop {
            // its promise's work, until it has answered (or its time is up)
            let out: Option<String> = ctx.with(|ctx| ctx.globals().get::<_, Option<String>>("__out").ok().flatten());
            if let Some(out) = out {
                let v: Value = serde_json::from_str(&out).unwrap_or_default();
                break match v.get("error").and_then(|e| e.as_str()) { Some(e) => Err(e.to_string()), None => Ok(v["ok"].clone()) };
            }
            if deadline.lock().unwrap().is_some_and(|d| Instant::now() > d) {
                break Err("it took too long to answer".into());
            }
            // answers that have come back go to their promises
            let back: Vec<(u32, String)> = std::mem::take(&mut *done.lock().unwrap());
            if !back.is_empty() {
                ctx.with(|ctx| {
                    if let Ok(f) = ctx.globals().get::<_, Function>("__host_done") {
                        for (n, r) in back { let _ = f.call::<_, ()>((n, r)); }
                    }
                });
                continue;
            }
            match engine.execute_pending_job() {
                Ok(true) => {}
                Ok(false) => std::thread::sleep(Duration::from_millis(5)),
                Err(_) => break Err("it stopped with an error".into()),
            }
        });
        *deadline.lock().unwrap() = None;
        let _ = job.reply.send(answer);
    }
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
}



