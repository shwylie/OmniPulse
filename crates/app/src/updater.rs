// Updates (as the web app's, through Tauri's updater): a little after each launch OmniPulse asks for the
// latest.json of its channel. Stable releases live on Cloudflare R2 (served by the relay); beta releases
// on GitHub (the native releases repo, shwylie/omnipulse-native-releases), and Beta Updates takes the
// newer of the two. A newer version is offered (Settings, Updates, and the status line); Update downloads its installer,
// checks it against OmniPulse's own key (the one Tauri's updater signs with: minisign), runs it and
// closes, and the installer starts the new one. Wherever it came from, only an installer signed with
// that key is run.

use serde_json::Value;

/// (the native app's own releases repo: the web app's, omnipulse-releases, stays the web app's)
pub const LATEST: &str = "https://github.com/shwylie/omnipulse-native-releases/releases/latest/download/latest.json";
/// the stable releases, on the relay (jam-server: /updates/stable/latest.json, the installers under /updates/download/)
fn stable_latest() -> String {
    format!("{}/updates/stable/latest.json", omnipulse_core::bundled::RELAY_URL)
}
/// OmniPulse's updater key (src-tauri/tauri.conf.json's plugins.updater.pubkey: a minisign public key, base64)
const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IENFQ0Y4MEMxMDNFNDhCRTYKUldUbWkrUUR3WURQem1TTE93NnBweCtpemIxdjRVWE5HMDVLQUc1QTZZV1E4TGoydG5OdWI2Z0QK";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub url: String,
    pub signature: String,
}

fn parts(v: &str) -> Vec<u64> {
    v.trim_start_matches('v').split(['.', '-']).take(3).map(|p| p.parse().unwrap_or(0)).collect()
}

/// a newer than b
pub fn newer(a: &str, b: &str) -> bool {
    parts(a) > parts(b)
}

fn release_of(v: &Value) -> Option<Release> {
    let w = &v["platforms"]["windows-x86_64"];
    Some(Release {
        version: v["version"].as_str()?.to_string(),
        notes: v["notes"].as_str().unwrap_or("").to_string(),
        url: w["url"].as_str().filter(|u| u.starts_with("https://github.com/shwylie/omnipulse-native-releases/") || u.starts_with(&format!("{}/updates/download/", omnipulse_core::bundled::RELAY_URL)))?.to_string(),
        signature: w["signature"].as_str()?.to_string(),
    })
}

async fn latest_from(url: &str) -> Result<Release, String> {
    let v: Value = reqwest::Client::new().get(url).header("User-Agent", format!("OmniPulse/{VERSION}")).timeout(std::time::Duration::from_secs(20)).send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    release_of(&v).ok_or_else(|| "the release info isn't readable".to_string())
}

/// The latest release on your channel, if it's newer than this one: stable's, or with Beta Updates on, the
/// newer of stable's and beta's (either one answering is enough)
pub async fn check(beta: bool) -> Result<Option<Release>, String> {
    let stable = latest_from(&stable_latest()).await;
    let r = if !beta { stable? } else {
        match (stable, latest_from(LATEST).await) {
            (Ok(s), Ok(b)) => if newer(&b.version, &s.version) { b } else { s },
            (Ok(s), Err(_)) => s,
            (Err(_), Ok(b)) => b,
            (Err(e), Err(_)) => return Err(e),
        }
    };
    Ok(newer(&r.version, VERSION).then_some(r))
}

/// Whether `data` is what OmniPulse's key signed (`signature`: Tauri's, base64 of minisign's .sig text)
pub fn verify(data: &[u8], signature: &str) -> Result<(), String> {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD;
    let key_text = String::from_utf8(b64.decode(PUBKEY).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let sig_text = String::from_utf8(b64.decode(signature.trim()).map_err(|e| format!("its signature isn't readable: {e}"))?).map_err(|e| e.to_string())?;
    let pk = minisign_verify::PublicKey::decode(&key_text).map_err(|e| e.to_string())?;
    let sig = minisign_verify::Signature::decode(&sig_text).map_err(|e| format!("its signature isn't readable: {e}"))?;
    pk.verify(data, &sig, false).map_err(|_| "it isn't signed with OmniPulse's key".to_string())
}

/// Downloads the release's installer, checks it, and keeps it in the temp folder
pub async fn download(r: &Release) -> Result<std::path::PathBuf, String> {
    let bytes = reqwest::Client::new().get(&r.url).header("User-Agent", format!("OmniPulse/{VERSION}")).send().await.map_err(|e| e.to_string())?
        .error_for_status().map_err(|e| e.to_string())?.bytes().await.map_err(|e| e.to_string())?;
    verify(&bytes, &r.signature)?;
    let path = std::env::temp_dir().join(format!("OmniPulse_{}_setup.exe", r.version));
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Runs the installer (passive: its progress only; it starts OmniPulse again when it's done)
pub fn install(path: &std::path::Path) -> Result<(), String> {
    std::process::Command::new(path).arg("/P").spawn().map(|_| ()).map_err(|e| format!("the installer wouldn't start: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare() {
        assert!(newer("0.3.1", "0.2.10"));
        assert!(newer("0.2.10", "0.2.9"));
        assert!(!newer("0.3.1", "0.3.1"));
        assert!(!newer("0.2.9", "0.3.0"));
    }

    #[test]
    fn the_key_reads_and_a_wrong_signature_fails() {
        assert!(verify(b"not an installer", "bm90IGEgc2lnbmF0dXJl").is_err());
        let r = release_of(&serde_json::json!({ "version": "0.3.1", "platforms": { "windows-x86_64": { "url": "https://evil.example/x.exe", "signature": "x" } } }));
        assert!(r.is_none(), "only the releases repo's");
        let ours = release_of(&serde_json::json!({ "version": "0.3.1", "platforms": { "windows-x86_64": { "url": format!("{}/updates/download/0.3.1/x.exe", omnipulse_core::bundled::RELAY_URL), "signature": "x" } } }));
        assert!(ours.is_some(), "and the relay's");
    }

    /// cargo test -p omnipulse updater_live -- --ignored (the published latest release, checked against the key)
    #[test]
    #[ignore]
    fn updater_live() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // (each place it's offered from: the relay's channels, and GitHub)
            for url in [stable_latest(), LATEST.to_string()] {
                // (one that can't be read is said, not failed: the beta repo, while it's private)
                let Ok(v) = reqwest::get(&url).await.unwrap().error_for_status().map_err(|e| println!("{url}: not readable ({e})")) else { continue };
                let v: Value = v.json().await.unwrap();
                let r = release_of(&v).expect("readable");
                let bytes = reqwest::get(&r.url).await.unwrap().bytes().await.unwrap();
                verify(&bytes, &r.signature).expect("signed with OmniPulse's key");
                println!("{url}: {} verified ({} bytes)", r.version, bytes.len());
            }
        });
    }
}

/// OMNIPULSE_VERIFY=<installer> cargo test -p omnipulse verify_a_file -- --ignored (its .sig beside it)
#[cfg(test)]
#[test]
#[ignore]
fn verify_a_file() {
    let f = std::env::var("OMNIPULSE_VERIFY").expect("OMNIPULSE_VERIFY");
    let data = std::fs::read(&f).unwrap();
    let sig = std::fs::read_to_string(format!("{f}.sig")).unwrap();
    verify(&data, &sig).expect("signed with OmniPulse's key");
    assert!(verify(&data[1..], &sig).is_err(), "a changed file fails");
}
