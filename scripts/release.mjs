// Publishes a release:  npm run release -- 0.2.0 ["What's new"]
//
// 1. Sets the version (tauri.conf.json, and the Rust crates, so the User-Agent says it too)
// 2. Builds the installer, signed for the updater with the key in %USERPROFILE%\.tauri (never in the
//    repo: without it no update can be made, so keep a copy somewhere safe)
// 3. Checks the SoundCloud key isn't in the build
// 4. Copies plugins/README.md to the public releases repo as PLUGINS.md (the cheat sheet), then
//    uploads the installer and latest.json (what the app's update check reads) as a GitHub release
//    there; that repo holds no source, only releases and the cheat sheet
//
// Needs the GitHub CLI, signed in (gh auth login).

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync, copyFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";

const REPO = "shwylie/omnipulse-releases";
const KEY = join(homedir(), ".tauri", "omnipulse-updater.key");
// (--publish-only: the build is done already, just publish it: a retry after a failed upload)
const publishOnly = process.argv.includes("--publish-only");
const [version, notes = ""] = process.argv.slice(2).filter((a) => a !== "--publish-only");
if (!/^\d+\.\d+\.\d+$/.test(version || "")) { console.error("Usage: npm run release -- <version, like 0.2.0> [notes]"); process.exit(1); }
if (!existsSync(KEY)) { console.error("The updater's signing key isn't at " + KEY + "."); process.exit(1); }
// (only npm goes through the shell, which Windows needs for it; gh and git get their arguments as they are)
const run = (cmd, args, opts = {}) => execFileSync(cmd, args, { stdio: "inherit", shell: cmd === "npm" && process.platform === "win32", ...opts });
const out = (cmd, args) => execFileSync(cmd, args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();

// 1. the version
if (!publishOnly) {
const conf = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
conf.version = version;
writeFileSync("src-tauri/tauri.conf.json", JSON.stringify(conf, null, 2) + "\n");
for (const toml of ["src-tauri/Cargo.toml", "crates/core/Cargo.toml"]) {
  writeFileSync(toml, readFileSync(toml, "utf8").replace(/^version = "[^"]*"/m, `version = "${version}"`));
}

// 2. the build, signed
// (the updater files are only made here: an everyday build has no key to sign them with)
const extra = join(tmpdir(), "omnipulse-release.conf.json");
writeFileSync(extra, JSON.stringify({ bundle: { createUpdaterArtifacts: true } }));
run("npm", ["run", "build", "--", "--config", JSON.stringify(extra)], { env: { ...process.env, TAURI_SIGNING_PRIVATE_KEY: readFileSync(KEY, "utf8"), TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "" } });

// 3. no SoundCloud key in it
// (its secret, as keys.local.json has it: never in the code, and never in a build, as the relay keeps it)
const scSecret = (() => { try { return String(JSON.parse(readFileSync("crates/core/keys.local.json", "utf8")).soundcloud_client_secret || ""); } catch (e) { return ""; } })();
if (scSecret && readFileSync("target/release/OmniPulse.exe").includes(scSecret)) { console.error("The SoundCloud secret is in the build: not publishing."); process.exit(1); }

}

// 4. the release
const name = `OmniPulse_${version}_x64-setup.exe`;
const exe = join("target/release/bundle/nsis", name);
const sig = readFileSync(exe + ".sig", "utf8").trim();
const tag = "v" + version;
const latest = join(tmpdir(), "latest.json");
writeFileSync(latest, JSON.stringify({
  version, notes, pub_date: new Date().toISOString(),
  platforms: { "windows-x86_64": { signature: sig, url: `https://github.com/${REPO}/releases/download/${tag}/${name}` } },
}, null, 2));
try { out("gh", ["repo", "view", REPO]); } catch (e) {
  run("gh", ["repo", "create", REPO, "--public", "--description", "OmniPulse releases and the plugin cheat sheet (no source here)"]);
}
// 5. the cheat sheet first: a release needs the repo to have something in it
const dir = join(tmpdir(), "omnipulse-releases");
if (!existsSync(join(dir, ".git"))) { mkdirSync(dir, { recursive: true }); run("gh", ["repo", "clone", REPO, dir]); }
else run("git", ["-C", dir, "pull", "--ff-only"]);
copyFileSync("plugins/README.md", join(dir, "PLUGINS.md"));
if (!existsSync(join(dir, "README.md"))) {
  writeFileSync(join(dir, "README.md"), "# OmniPulse\n\nReleases of OmniPulse (see Releases), which updates itself from here.\n\nWriting a plugin? See [PLUGINS.md](PLUGINS.md).\n");
}
run("git", ["-C", dir, "add", "PLUGINS.md", "README.md"]);
if (out("git", ["-C", dir, "status", "--porcelain"])) {
  run("git", ["-C", dir, "commit", "-m", "Plugin cheat sheet for " + version]);
  run("git", ["-C", dir, "push", "-u", "origin", "HEAD"]);
}
// then the release
run("gh", ["release", "create", tag, exe, latest, "--repo", REPO, "--title", "OmniPulse " + version, "--notes", notes || "OmniPulse " + version]);

console.log("Released " + tag + ".");
