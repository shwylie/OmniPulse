// The native OmniPulse's release (0.3.x):  node scripts/release-native.mjs <version> ["What's new"] [--publish]
//
// 1. Sets the version (crates/app, and crates/core so the User-Agent says it too)
// 2. Builds the native app and OmniPulse Offset (release)
// 3. Checks the SoundCloud key isn't in the build
// 4. Builds the installer (installer/omnipulse-native.nsi, with the NSIS Tauri keeps), and signs it with
//    the updater's key in %USERPROFILE%\.tauri (the same key: the web app's updater accepts it too)
// 5. Only with --publish: uploads it and latest.json as a GitHub release on the public releases repo.
//    NOTE: the web app (0.2.x) reads that same latest.json, so publishing moves everyone on the web app
//    to the native one at their next update. Publish only once that's what's wanted.
//
// Needs the GitHub CLI, signed in (gh auth login), to publish.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";

const REPO = "shwylie/omnipulse-releases";
const KEY = join(homedir(), ".tauri", "omnipulse-updater.key");
const MAKENSIS = join(process.env.LOCALAPPDATA || "", "tauri", "NSIS", "makensis.exe");
const publish = process.argv.includes("--publish");
const [version, notes = ""] = process.argv.slice(2).filter((a) => !a.startsWith("--"));
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
if (readFileSync("target/release/OmniPulse-native.exe").includes("gk1rHbyz5QpT4")) { console.error("The SoundCloud key is in the build: not releasing."); process.exit(1); }

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
  console.log("Not published (add --publish to). Publishing moves the web app's users to this one at their next update.");
  process.exit(0);
}
const tag = "v" + version;
const latest = join(tmpdir(), "latest.json");
writeFileSync(latest, JSON.stringify({
  version, notes, pub_date: new Date().toISOString(),
  platforms: { "windows-x86_64": { signature: sig, url: `https://github.com/${REPO}/releases/download/${tag}/${name}` } },
}, null, 2));
run("gh", ["release", "create", tag, exe, latest, "--repo", REPO, "--title", "OmniPulse " + version, "--notes", notes || "OmniPulse " + version]);
console.log("Released " + tag + ".");
