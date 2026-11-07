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
    /// the keys of text settings whose value is an address of its own site too (a server of yours, say):
    /// its host is reached as the manifest's are, https only; shown before the plugin's added
    #[serde(default, rename = "hostSettings")]
    pub host_settings: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub settings: Value,
    /// the Settings category its settings live in ("Streaming"), with a switch of its own there; empty: on
    /// the Plugins page, as the rest
    #[serde(default)]
    pub section: String,
    /// still being built: "(WIP)" after its name
    #[serde(default)]
    pub wip: bool,
    /// an addon (a server with a manifest.json at its address, the BeatBoss / Eclipse Music shape): that
    /// address; its script is OmniPulse's own adapter (addon.js), its fetches go to that site only, and
    /// what it plays may come from any https site the addon points to
    #[serde(default)]
    pub addon: String,
}

fn default_main() -> String {
    "main.js".into()
}

#[derive(Serialize, Clone, Debug)]
pub struct Package {
    pub manifest: Manifest,
    pub code: String,
    /// what was read (manifest and script): installing checks it's still the same
    pub digest: String,
    /// came with OmniPulse (installed from its own folder, kept up to date with it)
    pub bundled: bool,
}

fn digest_of(manifest_text: &str, code: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(manifest_text.as_bytes());
    h.update([0u8]);
    h.update(code.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The largest .omniplugin package taken
const MAX_PACKAGE: u64 = 4 * 1024 * 1024;

/// An .omniplugin file: a zip with manifest.json and its script, at the top or in one folder
fn read_package(path: &Path) -> Result<(String, String), String> {
    use std::io::Read;
    let f = std::fs::File::open(path).map_err(|e| format!("Couldn't open {}: {e}", path.display()))?;
    if f.metadata().map(|m| m.len()).unwrap_or(0) > MAX_PACKAGE {
        return Err("That plugin file is too big.".into());
    }
    let mut zip = zip::ZipArchive::new(f).map_err(|_| "That isn't an OmniPulse plugin file (it should be a .omniplugin package).".to_string())?;
    let names: Vec<String> = zip.file_names().map(String::from).collect();
    let man = names
        .iter()
        .filter(|n| n.ends_with("manifest.json") && n.matches('/').count() <= 1)
        .min_by_key(|n| n.len())
        .cloned()
        .ok_or("There's no manifest.json in that plugin file.")?;
    let dir = &man[..man.len() - "manifest.json".len()];
    let mut read = |name: &str, limit: usize| -> Result<String, String> {
        let mut e = zip.by_name(name).map_err(|_| format!("There's no {} in that plugin file.", &name[dir.len()..]))?;
        if e.size() as usize > limit {
            return Err(format!("{} in that plugin file is too big.", &name[dir.len()..]));
        }
        let mut s = String::new();
        e.by_ref().take(limit as u64 + 1).read_to_string(&mut s).map_err(|_| format!("{} in that plugin file isn't text.", &name[dir.len()..]))?;
        if s.len() > limit {
            return Err(format!("{} in that plugin file is too big.", &name[dir.len()..]));
        }
        Ok(s)
    };
    let manifest_text = read(&man, 256 * 1024)?;
    let m: Manifest = serde_json::from_str(&manifest_text).map_err(|e| format!("That plugin's manifest.json isn't valid: {e}"))?;
    check_manifest(&m)?;
    let code = read(&format!("{dir}{}", m.main), MAX_SCRIPT)?;
    Ok((manifest_text, code))
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
    for k in &m.host_settings {
        let is_text = m.settings.as_array().into_iter().flatten().any(|s| s["key"] == *k && s["type"].as_str().unwrap_or("text") == "text");
        if !is_text {
            return Err(format!("\"{k}\" (hostSettings) isn't one of its text settings."));
        }
    }
    Ok(())
}

/// The sites a plugin may reach with these settings: its manifest's, and the host of each address typed
/// into a setting its manifest names (hostSettings), https only (or this PC)
pub fn hosts_with(m: &Manifest, settings: &Value) -> Vec<String> {
    let mut hosts = m.hosts.clone();
    for k in &m.host_settings {
        let Some(v) = settings[k].as_str().map(str::trim).filter(|v| !v.is_empty()) else { continue };
        let Ok(u) = reqwest::Url::parse(v) else { continue };
        let Some(host) = u.host_str().map(|h| h.to_ascii_lowercase()) else { continue };
        let local = host == "127.0.0.1" || host == "localhost";
        if (u.scheme() == "https" || (u.scheme() == "http" && local)) && !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    hosts
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
/// of its manifest.json, or one file on this PC (an .omniplugin package, or a manifest.json with its
/// script beside it). Folders aren't taken: adding a plugin is meant to be one deliberate pick.
pub async fn inspect(source: &str) -> Result<Package, String> {
    let source = source.trim().trim_matches('"');
    let (manifest_text, code) = if source.starts_with("https://") || source.starts_with("http://") {
        let manifest_text = get_text(source, 256 * 1024).await?;
        if let Some(pkg) = addon_package(source, &manifest_text)? {
            return Ok(pkg);
        }
        let m: Manifest = serde_json::from_str(&manifest_text).map_err(|e| format!("That manifest.json isn't valid: {e}"))?;
        check_manifest(&m)?;
        let base = reqwest::Url::parse(source).map_err(|e| e.to_string())?;
        let script = base.join(&m.main).map_err(|e| e.to_string())?;
        (manifest_text, get_text(script.as_str(), MAX_SCRIPT).await?)
    } else {
        let p = Path::new(source);
        if p.is_dir() {
            return Err("Pick the plugin's file (an .omniplugin package, or its manifest.json), not a folder.".into());
        }
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "omniplugin" || ext == "zip" {
            read_package(p)?
        } else if p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case("manifest.json")) {
            let manifest_text = std::fs::read_to_string(p).map_err(|_| format!("Couldn't read {}", p.display()))?;
            let m: Manifest = serde_json::from_str(&manifest_text).map_err(|e| format!("That manifest.json isn't valid: {e}"))?;
            check_manifest(&m)?;
            let dir = p.parent().map(Path::to_path_buf).unwrap_or_default();
            let code = std::fs::read_to_string(dir.join(&m.main)).map_err(|_| format!("No {} next to the manifest", m.main))?;
            if code.len() > MAX_SCRIPT {
                return Err("The plugin's script is too big.".into());
            }
            (manifest_text, code)
        } else {
            return Err("That isn't a plugin file: pick an .omniplugin package or a plugin's manifest.json.".into());
        }
    };
    let manifest: Manifest = serde_json::from_str(&manifest_text).map_err(|e| e.to_string())?;
    let digest = digest_of(&manifest_text, &code);
    Ok(Package { manifest, code, digest, bundled: false })
}

/// OmniPulse's own script for an addon, with the addon's address in it
const ADDON_JS: &str = include_str!("addon.js");

/// An addon's manifest (the BeatBoss / Eclipse Music shape: `resources` and no script of its own), read
/// from its address, as a plugin: its name and description, its site as its only host, search and play as
/// it offers them, and the adapter as its script. Not an addon's: None.
pub fn addon_package(source: &str, manifest_text: &str) -> Result<Option<Package>, String> {
    let v: Value = serde_json::from_str(manifest_text).map_err(|e| format!("That manifest.json isn't valid: {e}"))?;
    let resources: Vec<String> = v["resources"].as_array().into_iter().flatten().filter_map(|r| r.as_str().or_else(|| r["name"].as_str()).map(|s| s.to_ascii_lowercase())).collect();
    if v["main"].is_string() || !v["resources"].is_array() {
        return Ok(None);
    }
    if !resources.iter().any(|r| r == "search") {
        return Err("That addon doesn't offer search, so there'd be nothing to find in it.".into());
    }
    let url = reqwest::Url::parse(source).map_err(|e| e.to_string())?;
    let host = url.host_str().map(|h| h.to_ascii_lowercase()).ok_or("That address has no site in it.")?;
    let local = host == "127.0.0.1" || host == "localhost";
    if url.scheme() != "https" && !(url.scheme() == "http" && local) {
        return Err("An addon's address must be https (or http on this PC).".into());
    }
    // (its base: the address without its manifest.json; the token some addons put in the path stays in it)
    let base = source.trim_end_matches('/').rsplit_once('/').map(|(b, last)| if last.eq_ignore_ascii_case("manifest.json") { b.to_string() } else { source.trim_end_matches('/').to_string() }).unwrap_or_else(|| source.to_string());
    let raw_id = v["id"].as_str().unwrap_or("").to_ascii_lowercase();
    let mut id: String = raw_id.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
    if id.is_empty() { id = host.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect(); }
    let name = v["name"].as_str().unwrap_or("").trim().to_string();
    if name.is_empty() {
        return Err("That addon's manifest has no name.".into());
    }
    let m = Manifest {
        id: format!("addon-{id}"),
        name: name.chars().take(80).collect(),
        version: v["version"].as_str().unwrap_or("").to_string(),
        author: v["author"].as_str().unwrap_or("").to_string(),
        description: v["description"].as_str().unwrap_or("").to_string(),
        main: "main.js".into(),
        hosts: vec![host],
        capabilities: vec!["search".into(), "play".into()],
        addon: base.clone(),
        ..Default::default()
    };
    check_manifest(&m)?;
    let code = ADDON_JS.replace("__BASE__", &serde_json::to_string(&base).unwrap_or_default());
    let pretty = serde_json::to_string_pretty(&m).unwrap_or_default();
    let digest = digest_of(&pretty, &code);
    Ok(Some(Package { manifest: m, code, digest, bundled: false }))
}

/// An addon's site, to show for it: the host of its address (never the address itself, which may carry a key)
pub fn addon_site(m: &Manifest) -> String {
    reqwest::Url::parse(&m.addon).ok().and_then(|u| u.host_str().map(|h| h.to_string())).unwrap_or_else(|| m.hosts.first().cloned().unwrap_or_default())
}

/// Installs what inspect() found and you agreed to: read again from the source, and only if it's
/// still exactly what you saw
pub async fn install(source: &str, digest: &str) -> Result<Manifest, String> {
    let pkg = inspect(source).await?;
    if pkg.digest != digest {
        return Err("The plugin has changed since you looked at it, so it wasn't added. Check it again.".into());
    }
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
            let bundled = e.path().join(BUNDLED_MARK).exists();
            Some(Package { manifest: m, code, digest: String::new(), bundled })
        })
        .collect();
    out.sort_by_key(|p| p.manifest.name.to_lowercase());
    out
}

// ---------- the plugins that come with OmniPulse ----------
// They ship as ordinary plugin folders (manifest.json and main.js, readable like any other) beside
// the app, and are installed from there: the first time, and again whenever an update brings a
// different one (its script or manifest changed, whatever its version says). One you remove stays
// removed (its id is kept in REMOVED_LIST).
const BUNDLED_MARK: &str = ".bundled";
const REMOVED_LIST: &str = "removed-bundled.txt";

/// Installs (or brings up to date) each plugin folder in `src` (the app's own plugins folder)
pub fn install_bundled(src: &Path) {
    if let Ok(dest) = plugins_dir() {
        install_bundled_into(src, &dest);
    }
}

fn install_bundled_into(src: &Path, dest: &Path) {
    let removed = std::fs::read_to_string(dest.join(REMOVED_LIST)).unwrap_or_default();
    let Ok(rd) = std::fs::read_dir(src) else { return };
    for e in rd.flatten() {
        let Ok(manifest_text) = std::fs::read_to_string(e.path().join("manifest.json")) else { continue };
        let Ok(m) = serde_json::from_str::<Manifest>(&manifest_text) else { continue };
        if check_manifest(&m).is_err() || removed.lines().any(|l| l.trim() == m.id) {
            continue;
        }
        let Ok(code) = std::fs::read_to_string(e.path().join(&m.main)) else { continue };
        let dir = dest.join(&m.id);
        let have: Option<Manifest> = std::fs::read_to_string(dir.join("manifest.json")).ok().and_then(|t| serde_json::from_str(&t).ok());
        let pretty = serde_json::to_string_pretty(&m).unwrap_or_else(|_| manifest_text.clone());
        let same = std::fs::read_to_string(dir.join("main.js")).is_ok_and(|c| c == code) && std::fs::read_to_string(dir.join("manifest.json")).is_ok_and(|t| t == pretty);
        // (the same one already there; or one of your own by its id, not ours, at the same version: left alone)
        if same || (have.as_ref().is_some_and(|h| h.version == m.version) && !dir.join(BUNDLED_MARK).exists() && dir.join("main.js").exists()) {
            continue;
        }
        if std::fs::create_dir_all(&dir).is_err() {
            continue;
        }
        // (as an installed plugin is kept: its script as main.js)
        let _ = std::fs::write(dir.join("manifest.json"), pretty);
        let _ = std::fs::write(dir.join("main.js"), code);
        let _ = std::fs::write(dir.join(BUNDLED_MARK), "Came with OmniPulse. Removing it keeps it removed.\n");
    }
}

/// The folder an installed plugin's files are in
pub fn folder_of(id: &str) -> Result<PathBuf, String> {
    if !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') || id.is_empty() {
        return Err("Bad plugin id".into());
    }
    let dir = plugins_dir()?.join(id);
    if dir.is_dir() { Ok(dir) } else { Err("That plugin isn't installed.".into()) }
}

pub fn remove(id: &str) -> Result<(), String> {
    if !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') || id.is_empty() {
        return Err("Bad plugin id".into());
    }
    let dir = plugins_dir()?.join(id);
    // one that came with OmniPulse isn't put back by the next start
    if dir.join(BUNDLED_MARK).exists() {
        let list = plugins_dir()?.join(REMOVED_LIST);
        let mut have = std::fs::read_to_string(&list).unwrap_or_default();
        if !have.lines().any(|l| l.trim() == id) {
            have.push_str(id);
            have.push('\n');
            let _ = std::fs::write(list, have);
        }
    }
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
    http_for(hosts, url, method, headers, body).await
}

/// The same, to a running plugin's own sites (its manifest as it was started: a newer one, come with an
/// update, reaches its new sites before it's installed again)
pub async fn http_for(hosts: Vec<String>, url: &str, method: &str, headers: Vec<(String, String)>, body: Option<String>) -> Result<Value, String> {
    if !host_allowed(&hosts, url) {
        return Err(format!("This plugin may only reach {}; it asked for {url}.", hosts.join(", ")));
    }
    let m = match method.to_ascii_uppercase().as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "PATCH" => reqwest::Method::PATCH,
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
        // (a site typed into a named setting: reached too, https only)
        let mut m = m("idx", &["api.example.com"]);
        m.host_settings = vec!["indexUrl".into()];
        m.settings = serde_json::json!([{ "key": "indexUrl", "type": "text" }]);
        assert!(check_manifest(&m).is_ok());
        assert_eq!(hosts_with(&m, &serde_json::json!({ "indexUrl": "https://My.Example.com:8443/api/search?x=1" })), vec!["api.example.com".to_string(), "my.example.com".into()]);
        assert_eq!(hosts_with(&m, &serde_json::json!({ "indexUrl": "http://my.example.com/api" })), vec!["api.example.com".to_string()]);
        assert_eq!(hosts_with(&m, &serde_json::json!({ "indexUrl": "http://127.0.0.1:9117/api" })), vec!["api.example.com".to_string(), "127.0.0.1".into()]);
        assert_eq!(hosts_with(&m, &serde_json::json!({})), vec!["api.example.com".to_string()]);
        m.host_settings = vec!["apiKey".into()];
        m.settings = serde_json::json!([{ "key": "apiKey", "type": "password" }]);
        assert!(check_manifest(&m).is_err());
    }

    /// An addon's manifest (a server's, with resources and no script): a plugin of its own, with the
    /// adapter as its script and the addon's site as its host; a plugin's own manifest isn't one
    #[test]
    fn an_addon_manifest_is_a_plugin_with_our_script() {
        let text = r#"{ "id": "com.example.music", "name": "Example Music", "version": "1.2.0", "description": "Songs.", "icon": "https://x/icon.png", "types": ["track"], "resources": ["search", "stream", "lyrics"] }"#;
        let pkg = addon_package("https://addons.example.com/tok123/manifest.json", text).unwrap().unwrap();
        let m = &pkg.manifest;
        // (shown by its site: the token in its path stays out of view)
        assert_eq!(addon_site(m), "addons.example.com");
        assert_eq!((m.id.as_str(), m.name.as_str(), m.version.as_str(), m.description.as_str()), ("addon-com-example-music", "Example Music", "1.2.0", "Songs."));
        assert_eq!(m.hosts, vec!["addons.example.com".to_string()]);
        assert_eq!(m.addon, "https://addons.example.com/tok123");
        assert_eq!(m.capabilities, vec!["search".to_string(), "play".into()]);
        assert!(m.icon.is_empty() && m.settings.is_null());
        assert!(pkg.code.contains(r#"const BASE = "https://addons.example.com/tok123";"#));
        assert!(!pkg.digest.is_empty());
        // (the same again: the same digest, so installing checks out)
        assert_eq!(addon_package("https://addons.example.com/tok123/manifest.json", text).unwrap().unwrap().digest, pkg.digest);
        // (a base without manifest.json at its end is taken as it is)
        assert_eq!(addon_package("http://127.0.0.1:7000/api/", text).unwrap().unwrap().manifest.addon, "http://127.0.0.1:7000/api");
        // (not an addon: a plugin's own manifest)
        assert!(addon_package("https://x.example.com/manifest.json", r#"{ "id": "mine", "name": "Mine", "main": "main.js", "hosts": [] }"#).unwrap().is_none());
        // (no search: nothing to do with it; plain http elsewhere: refused)
        assert!(addon_package("https://x.example.com/manifest.json", r#"{ "id": "a", "name": "A", "resources": ["lyrics"] }"#).is_err());
        assert!(addon_package("http://x.example.com/manifest.json", text).is_err());
        assert!(host_allowed(&["127.0.0.1".to_string()], "http://127.0.0.1:9/x"));
    }

    #[test]
    fn bundled_plugins_install_update_and_stay_removed() {
        let tmp = std::env::temp_dir().join(format!("op-bundled-{}", std::process::id()));
        let src = tmp.join("src");
        let dest = tmp.join("dest");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&dest).unwrap();
        let ia = concat!(env!("CARGO_MANIFEST_DIR"), "/../../plugins");
        install_bundled_into(Path::new(ia), &dest);
        assert!(dest.join("internet-archive/main.js").exists() && dest.join("internet-archive").join(BUNDLED_MARK).exists());
        // a changed one replaces it, whatever its version says; a newer one too
        std::fs::create_dir_all(src.join("x")).unwrap();
        let m = |v: &str| format!(r#"{{"id":"demo","name":"Demo","version":"{v}","main":"main.js","hosts":[]}}"#);
        std::fs::write(src.join("x/manifest.json"), m("1.0.0")).unwrap();
        std::fs::write(src.join("x/main.js"), "one").unwrap();
        install_bundled_into(&src, &dest);
        std::fs::write(src.join("x/main.js"), "two").unwrap();
        install_bundled_into(&src, &dest);
        assert_eq!(std::fs::read_to_string(dest.join("demo/main.js")).unwrap(), "two");
        std::fs::write(src.join("x/manifest.json"), m("1.1.0")).unwrap();
        std::fs::write(src.join("x/main.js"), "three").unwrap();
        install_bundled_into(&src, &dest);
        assert_eq!(std::fs::read_to_string(dest.join("demo/main.js")).unwrap(), "three");
        // one of your own by the same id and version (not marked as ours): left alone
        std::fs::remove_file(dest.join("demo").join(BUNDLED_MARK)).unwrap();
        std::fs::write(dest.join("demo/main.js"), "mine").unwrap();
        install_bundled_into(&src, &dest);
        assert_eq!(std::fs::read_to_string(dest.join("demo/main.js")).unwrap(), "mine");
        // removed: not put back
        std::fs::remove_dir_all(dest.join("demo")).unwrap();
        std::fs::write(dest.join(REMOVED_LIST), "demo\n").unwrap();
        install_bundled_into(&src, &dest);
        assert!(!dest.join("demo").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn reads_the_example_package() {
        let pack = concat!(env!("CARGO_MANIFEST_DIR"), "/../../plugins/internet-archive.omniplugin");
        let pkg = inspect(pack).await.unwrap();
        assert_eq!(pkg.manifest.id, "internet-archive");
        assert!(pkg.code.contains("OmniPulse.register"));
        assert_eq!(pkg.digest.len(), 64);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // only makes the tests take turns
    async fn installs_from_one_file_and_refuses_other_sites() {
        let _turn = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("omnipulse-plugins-{}", std::process::id()));
        std::env::set_var("APPDATA", tmp.join("appdata"));
        let src = tmp.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("manifest.json"), r#"{"id":"demo","name":"Demo","hosts":["127.0.0.1"],"capabilities":["search","play"]}"#).unwrap();
        std::fs::write(src.join("main.js"), "OmniPulse.register({});").unwrap();
        // a folder isn't taken; its manifest.json is
        assert!(inspect(src.to_str().unwrap()).await.unwrap_err().contains("not a folder"));
        let man = src.join("manifest.json");
        let pkg = inspect(man.to_str().unwrap()).await.unwrap();
        assert_eq!(pkg.manifest.id, "demo");
        // changed after it was looked at: not added
        std::fs::write(src.join("main.js"), "OmniPulse.register({ search() {} });").unwrap();
        assert!(install(man.to_str().unwrap(), &pkg.digest).await.unwrap_err().contains("changed"));
        // an .omniplugin package (a zip, here with its files in a folder)
        let pack = tmp.join("demo.omniplugin");
        {
            use std::io::Write;
            let mut z = zip::ZipWriter::new(std::fs::File::create(&pack).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("demo/manifest.json", o).unwrap();
            z.write_all(std::fs::read(&man).unwrap().as_slice()).unwrap();
            z.start_file("demo/main.js", o).unwrap();
            z.write_all(b"OmniPulse.register({});").unwrap();
            z.finish().unwrap();
        }
        let pkg = inspect(pack.to_str().unwrap()).await.unwrap();
        assert_eq!(pkg.code, "OmniPulse.register({});");
        install(pack.to_str().unwrap(), &pkg.digest).await.unwrap();
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
