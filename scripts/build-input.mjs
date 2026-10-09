// Builds omnipulse-offset.exe (crates/input) and puts it where Tauri bundles it from
// (src-tauri/binaries, named for the target as Tauri expects); the installer puts it next to OmniPulse.exe.
import { execSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";

const triple = execSync("rustc -vV").toString().match(/host: (\S+)/)[1];
execSync("cargo build -p omnipulse-offset --release", { stdio: "inherit" });
mkdirSync("src-tauri/binaries", { recursive: true });
copyFileSync("target/release/omnipulse-offset.exe", `src-tauri/binaries/omnipulse-offset-${triple}.exe`);
console.log("omnipulse-offset ready for " + triple);
