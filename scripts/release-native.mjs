// The native OmniPulse's release (0.3.x):  node scripts/release-native.mjs <version> ["What's new"] [--publish] [--channel beta]
//
// 1. Sets the version (crates/app, and crates/core so the User-Agent says it too)
// 2. Builds the native app and OmniPulse Offset (release)
// 3. Checks the SoundCloud key isn't in the build
// 4. Builds the installer (installer/omnipulse-native.nsi, with the NSIS Tauri keeps), and signs it with
//    the updater's key in %USERPROFILE%\.tauri (the same key: the web app's updater accepts it too)
// 5. Only with --publish: a stable release (the default) goes to Cloudflare R2 (the omnipulse-updates
//    bucket, served by the relay at /updates/...) with its latest.json, for everyone; --channel beta goes
//    to GitHub instead, a release on the native releases repo (shwylie/omnipulse-native-releases), for
//    those with Beta Updates on. --also-github puts a stable one on GitHub as well: versions up to 0.3.21
//    only look there, so the first stable on R2 (0.3.22) goes to both, to move them over.
//
// Needs wrangler signed in (it is, for the relay) for stable, and the GitHub CLI (gh auth login) for beta.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";

const REPO = "shwylie/omnipulse-native-releases";
const RELAY = "https://omnipulse-jam.shwylie101.workers.dev";
const KEY = join(homedir(), ".tauri", "omnipulse-updater.key");
const MAKENSIS = join(process.env.LOCALAPPDATA || "", "tauri", "NSIS", "makensis.exe");
const publish = process.argv.includes("--publish");
const ci = process.argv.indexOf("--channel");
const channel = ci > 0 ? process.argv[ci + 1] : "stable";
if (channel !== "stable" && channel !== "beta") { console.error("--channel is stable or beta."); process.exit(1); }
const [version, notes = ""] = process.argv.slice(2).filter((a, i, all) => !a.startsWith("--") && all[i - 1] !== "--channel");
if (!/^\d+\.\d+\.\d+$/.test(version || "")) { console.error("Usage: node scripts/release-native.mjs <version, like 0.3.1> [notes] [--publish]"); process.exit(1); }
if (!existsSync(KEY)) { console.error("The updater's signing key isn't at " + KEY + "."); process.exit(1); }
if (!existsSync(MAKENSIS)) { console.error("NSIS isn't at " + MAKENSIS + " (a Tauri build puts it there)."); process.exit(1); }
const run = (cmd, args, opts = {}) => execFileSync(cmd, args, { stdio: "inherit", shell: (cmd === "npm" || cmd === "npx") && process.platform === "win32", ...opts });

// 1. the version
for (const toml of ["crates/app/Cargo.toml", "crates/core/Cargo.toml"]) {
  writeFileSync(toml, readFileSync(toml, "utf8").replace(/^version = "[^"]*"/m, `version = "${version}"`));
}

// 2. the build
run("cargo", ["build", "--release", "-p", "omnipulse", "-p", "omnipulse-offset"]);

// 3. no SoundCloud key in it
// (its secret, as keys.local.json has it: never in the code, and never in a build, as the relay keeps it)
const scSecret = (() => { try { return String(JSON.parse(readFileSync("crates/core/keys.local.json", "utf8")).soundcloud_client_secret || ""); } catch (e) { return ""; } })();
if (scSecret && readFileSync("target/release/OmniPulse-native.exe").includes(scSecret)) { console.error("The SoundCloud secret is in the build: not releasing."); process.exit(1); }

// 4. the installer, signed
const outDir = resolve("target/release/bundle/native");
mkdirSync(outDir, { recursive: true });
const name = `OmniPulse_${version}_x64-setup.exe`;
const exe = join(outDir, name);
run(MAKENSIS, ["/V2", `/DVERSION=${version}`, `/DOUT=${exe}`, "installer\\omnipulse-native.nsi"]);
// (no password: an empty one goes in the environment, as an empty argument wouldn't survive the shell)
run("npx", ["tauri", "signer", "sign", "-f", KEY, exe], { env: { ...process.env, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "" } });
const sig = readFileSync(exe + ".sig", "utf8").trim();
console.log("Built and signed " + exe);

// 5. the release (only when asked)
if (!publish) {
  console.log("Not published (add --publish to).");
  process.exit(0);
}
const tag = "v" + version;
const latestFor = (url) => JSON.stringify({ version, notes, channel, pub_date: new Date().toISOString(), platforms: { "windows-x86_64": { signature: sig, url } } }, null, 2);

// Cloudflare: stable releases, the installer and then its list
if (channel === "stable") {
  const r2 = (key, file, type) => run("npx", ["wrangler", "r2", "object", "put", "omnipulse-updates/" + key, "--file", file, "--content-type", type, "--remote"], { cwd: "jam-server" });
  r2(`download/${version}/${name}`, exe, "application/octet-stream");
  const cfLatest = join(tmpdir(), "latest-cf.json");
  writeFileSync(cfLatest, latestFor(`${RELAY}/updates/download/${version}/${name}`));
  r2("stable/latest.json", cfLatest, "application/json");
  console.log("On Cloudflare.");
}

// GitHub: beta releases (and a stable one with --also-github)
if (channel === "beta" || process.argv.includes("--also-github")) {
  const latest = join(tmpdir(), "latest.json");
  writeFileSync(latest, latestFor(`https://github.com/${REPO}/releases/download/${tag}/${name}`));
  run("gh", ["release", "create", tag, exe, latest, "--repo", REPO, "--title", "OmniPulse " + version, "--notes", notes || "OmniPulse " + version]);
}
console.log("Released " + tag + " (" + channel + ").");
