// Plugins: third-party sources of music (search, playback, your library there, and their metadata).
//
// A plugin is a folder: manifest.json and a script (main.js by default). It's installed from a web
// address (of its manifest.json) or a folder on this PC into <data dir>\plugins\<id>. The script runs in
// a sandboxed frame in the page, which can't reach the app, your accounts or your files; it can only ask
// the app for web requests, and only to the sites its manifest lists (shown before it's installed).
// Those requests are made here, and every one (and every redirect) is checked against that list again.
// Audio from a plugin plays through the local server (/plug), under the same rule.
//
// manifest.json:
//   { "id": "internet-archive", "name": "Internet Archive", "version": "1.0.0", "author": "...",
//     "description": "...", "main": "main.js", "hosts": ["archive.org", "*.archive.org"],
//     "capabilities": ["search", "play", "library"], "icon": "<svg path data>",
//     "settings": [{ "key": "...", "label": "...", "type": "text|password|toggle|select", ... }] }

use crate::data_dir;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_SCRIPT: usize = 2 * 1024 * 1024;
const MAX_BODY: usize = 10 * 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_main")]
    pub main: String,
    #[serde(default)]
    pub hosts: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub settings: Value,
}

fn default_main() -> String {
    "main.js".into()
}

#[derive(Serialize, Clone, Debug)]
pub struct Package {
    pub manifest: Manifest,
    pub code: String,
}

fn plugins_dir() -> Result<PathBuf, String> {
    let d = data_dir()?.join("plugins");
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

/// A manifest the app will take: a plain id, a name, a script name without paths, and sites given as
/// names (optionally "*." for subdomains), not addresses
pub fn check_manifest(m: &Manifest) -> Result<(), String> {
    let plain = |s: &str| !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !plain(&m.id) {
        return Err("The plugin's id must be lower-case letters, digits and dashes.".into());
    }
    if m.name.trim().is_empty() || m.name.len() > 80 {
        return Err("The plugin needs a name.".into());
    }
    if m.main.contains(['/', '\\']) || m.main.contains("..") || !m.main.ends_with(".js") {
        return Err("The plugin's script must be a .js file next to its manifest.".into());
    }
    for h in &m.hosts {
        let bare = h.strip_prefix("*.").unwrap_or(h);
        let ok = bare.contains('.') && bare.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') && !bare.starts_with('.');
        if !ok {
            return Err(format!("\"{h}\" isn't a site name (like example.com or *.example.com)."));
        }
    }
    if m.icon.len() > 4000 {
        return Err("The plugin's icon is too big.".into());
    }
    Ok(())
}

/// Whether a plugin may reach this address: https (or http on this PC), to one of its sites
pub fn host_allowed(hosts: &[String], url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else { return false };
    let Some(host) = u.host_str().map(|h| h.to_ascii_lowercase()) else { return false };
    let local = host == "127.0.0.1" || host == "localhost";
    if u.scheme() != "https" && !(u.scheme() == "http" && local) {
        return false;
    }
    hosts.iter().any(|p| {
        let p = p.to_ascii_lowercase();
        match p.strip_prefix("*.") {
            Some(base) => host == base || host.ends_with(&format!(".{base}")),
            None => host == p,
        }
    })
}

// ---------- finding and installing ----------

async fn get_text(url: &str, limit: usize) -> Result<String, String> {
    let res = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("couldn't download {url}: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("{url} answered {}", res.status()));
    }
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err(format!("{url} is too big for a plugin."));
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| format!("{url} isn't text."))
}

/// Reads a plugin without installing it, so you can see what it is and asks for: from the web address
/// of its manifest.json, or a folder (or manifest.json) on this PC
pub async fn inspect(source: &str) -> Result<Package, String> {
    let source = source.trim().trim_matches('"');
    let (manifest_text, code) = if source.starts_with("https://") || source.starts_with("http://") {
        let manifest_text = get_text(source, 256 * 1024).await?;
        let m: Manifest = serde_json::from_str(&manifest_text).map_err(|e| format!("That manifest.json isn't valid: {e}"))?;
        check_manifest(&m)?;
        let base = reqwest::Url::parse(source).map_err(|e| e.to_string())?;
        let script = base.join(&m.main).map_err(|e| e.to_string())?;
        (manifest_text, get_text(script.as_str(), MAX_SCRIPT).await?)
    } else {
        let p = Path::new(source);
        let dir = if p.is_dir() { p.to_path_buf() } else { p.parent().map(Path::to_path_buf).unwrap_or_default() };
        let manifest_text = std::fs::read_to_string(dir.join("manifest.json")).map_err(|_| format!("No manifest.json in {}", dir.display()))?;
        let m: Manifest = serde_json::from_str(&manifest_text).map_err(|e| format!("That manifest.json isn't valid: {e}"))?;
        check_manifest(&m)?;
        let code = std::fs::read_to_string(dir.join(&m.main)).map_err(|_| format!("No {} next to the manifest", m.main))?;
        if code.len() > MAX_SCRIPT {
            return Err("The plugin's script is too big.".into());
        }
        (manifest_text, code)
    };
    let manifest: Manifest = serde_json::from_str(&manifest_text).map_err(|e| e.to_string())?;
    Ok(Package { manifest, code })
}

/// Installs what inspect() found (again from the source, so what's installed is what was read)
pub async fn install(source: &str) -> Result<Manifest, String> {
    let pkg = inspect(source).await?;
    let dir = plugins_dir()?.join(&pkg.manifest.id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(&pkg.manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("main.js"), &pkg.code).map_err(|e| e.to_string())?;
    Ok(pkg.manifest)
}

/// Every installed plugin, with its script
pub fn list() -> Vec<Package> {
    let Ok(dir) = plugins_dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Package> = rd
        .flatten()
        .filter_map(|e| {
            let m: Manifest = serde_json::from_str(&std::fs::read_to_string(e.path().join("manifest.json")).ok()?).ok()?;
            check_manifest(&m).ok()?;
            let code = std::fs::read_to_string(e.path().join("main.js")).ok()?;
            Some(Package { manifest: m, code })
        })
        .collect();
    out.sort_by_key(|p| p.manifest.name.to_lowercase());
    out
}

pub fn remove(id: &str) -> Result<(), String> {
    if !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') || id.is_empty() {
        return Err("Bad plugin id".into());
    }
    let dir = plugins_dir()?.join(id);
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The installed plugin's sites (what its requests are checked against)
pub fn hosts_of(id: &str) -> Option<Vec<String>> {
    let dir = plugins_dir().ok()?.join(id);
    let m: Manifest = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).ok()?).ok()?;
    Some(m.hosts)
}

// ---------- the web requests a plugin asks for ----------

/// A client whose redirects are followed only to the plugin's own sites
pub fn client_for(hosts: Vec<String>) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() > 5 || !host_allowed(&hosts, attempt.url().as_str()) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .unwrap_or_default()
}

/// Makes a request for a plugin: { status, headers, body (text) }
pub async fn http(id: &str, url: &str, method: &str, headers: Vec<(String, String)>, body: Option<String>) -> Result<Value, String> {
    let hosts = hosts_of(id).ok_or("That plugin isn't installed.")?;
    if !host_allowed(&hosts, url) {
        return Err(format!("This plugin may only reach {}; it asked for {url}.", hosts.join(", ")));
    }
    let m = match method.to_ascii_uppercase().as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "HEAD" => reqwest::Method::HEAD,
        _ => return Err("That request method isn't allowed.".into()),
    };
    let mut rb = client_for(hosts).request(m, url);
    for (k, v) in headers {
        // a plugin can't set what identifies the app's own sign-ins
        if !k.eq_ignore_ascii_case("cookie") && !k.eq_ignore_ascii_case("host") {
            rb = rb.header(k, v);
        }
    }
    if let Some(b) = body {
        rb = rb.body(b);
    }
    let res = rb.send().await.map_err(|e| e.to_string())?;
    let status = res.status().as_u16();
    let hdrs: serde_json::Map<String, Value> = res
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_string(), Value::String(v.to_str().ok()?.to_string()))))
        .collect();
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BODY {
        return Err("The answer was too big.".into());
    }
    Ok(json!({ "status": status, "headers": hdrs, "body": String::from_utf8_lossy(&bytes) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, hosts: &[&str]) -> Manifest {
        Manifest { id: id.into(), name: "Test".into(), main: "main.js".into(), hosts: hosts.iter().map(|h| h.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn checks_manifests() {
        assert!(check_manifest(&m("internet-archive", &["archive.org", "*.archive.org"])).is_ok());
        assert!(check_manifest(&m("Bad Id", &[])).is_err());
        assert!(check_manifest(&m("ok", &["https://evil.com"])).is_err());
        assert!(check_manifest(&m("ok", &["*"])).is_err());
        let mut bad = m("ok", &[]);
        bad.main = "../../x.js".into();
        assert!(check_manifest(&bad).is_err());
    }

    #[test]
    fn only_its_own_sites() {
        let hosts = vec!["archive.org".to_string(), "*.archive.org".to_string()];
        assert!(host_allowed(&hosts, "https://archive.org/advancedsearch.php?q=x"));
        assert!(host_allowed(&hosts, "https://ia800204.us.archive.org/1/items/x/y.flac"));
        assert!(!host_allowed(&hosts, "http://archive.org/x"));   // https only
        assert!(!host_allowed(&hosts, "https://archive.org.evil.com/x"));
        assert!(!host_allowed(&hosts, "https://evilarchive.org/x"));
        assert!(!host_allowed(&hosts, "https://api.soundcloud.com/me"));
        assert!(!host_allowed(&hosts, "file:///C:/Users"));
        assert!(host_allowed(&["127.0.0.1".to_string()], "http://127.0.0.1:9/x"));
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // only makes the tests take turns
    async fn installs_from_a_folder_and_refuses_other_sites() {
        let _turn = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("omnipulse-plugins-{}", std::process::id()));
        std::env::set_var("APPDATA", tmp.join("appdata"));
        let src = tmp.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("manifest.json"), r#"{"id":"demo","name":"Demo","hosts":["127.0.0.1"],"capabilities":["search","play"]}"#).unwrap();
        std::fs::write(src.join("main.js"), "OmniPulse.register({});").unwrap();
        let pkg = inspect(src.to_str().unwrap()).await.unwrap();
        assert_eq!(pkg.manifest.id, "demo");
        install(src.to_str().unwrap()).await.unwrap();
        assert_eq!(list().len(), 1);
        let err = http("demo", "https://example.com/", "GET", vec![], None).await.unwrap_err();
        assert!(err.contains("may only reach"), "{err}");
        // a request to its own site goes through
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            if let Ok(req) = server.recv() {
                let _ = req.respond(tiny_http::Response::from_string("hello"));
            }
        });
        let v = http("demo", &format!("http://127.0.0.1:{port}/x"), "GET", vec![], None).await.unwrap();
        assert_eq!((v["status"].as_u64(), v["body"].as_str()), (Some(200), Some("hello")));
        remove("demo").unwrap();
        assert!(list().is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
