// The app's own SoundCloud keys, built in: read from keys.local.json beside this file (never in git:
// see .gitignore) and written into the build scrambled, so neither key shows up as text in the
// program. Without the file the build still works, and SoundCloud needs keys in config.json instead.
//
// keys.local.json: { "soundcloud_client_id": "...", "soundcloud_client_secret": "...",
//                    "soundcloud_redirect_uri": "http://localhost:8080/callback" }
//
// Scrambling only makes the keys harder to find: a program has to send its secret to SoundCloud to
// sign in, so anyone determined (watching their own traffic, or stepping through the program) can
// still get it. A server that keeps the secret and does the sign-in exchange is the real fix.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

fn field(json: &str, name: &str) -> String {
    let key = format!("\"{name}\"");
    let Some(at) = json.find(&key) else { return String::new() };
    let rest = &json[at + key.len()..];
    let Some(open) = rest.find('"') else { return String::new() };
    let rest = &rest[open + 1..];
    rest.find('"').map(|close| rest[..close].to_string()).unwrap_or_default()
}

// A fresh pad for every build, so two builds never scramble the same way
fn pad(len: usize) -> Vec<u8> {
    let s = RandomState::new();
    (0..len)
        .map(|i| {
            let mut h = s.build_hasher();
            h.write_usize(i);
            h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
            (h.finish() >> 17) as u8
        })
        .collect()
}

fn scramble(name: &str, value: &str) -> String {
    let b = value.as_bytes();
    let k = pad(b.len());
    // each byte mixed with its pad byte and its place, stored back to front
    let enc: Vec<u8> = b.iter().enumerate().map(|(i, &c)| c ^ k[i] ^ ((i as u8).wrapping_mul(0x5d).wrapping_add(0xa7))).rev().collect();
    format!("const {name}_E: [u8; {n}] = {enc:?};\nconst {name}_K: [u8; {n}] = {k:?};\n", n = b.len())
}

fn main() {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("keys.local.json");
    println!("cargo:rerun-if-changed={}", here.display());
    let json = std::fs::read_to_string(&here).unwrap_or_default();
    let out = [
        scramble("SC_ID", &field(&json, "soundcloud_client_id")),
        scramble("SC_SECRET", &field(&json, "soundcloud_client_secret")),
        scramble("SC_REDIRECT", &field(&json, "soundcloud_redirect_uri")),
    ]
    .concat();
    let dest = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("bundled.rs");
    std::fs::write(dest, out).unwrap();
}
