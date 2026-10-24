// The Stream Deck plugin, packed for the app to carry:  node scripts/streamdeck-pack.mjs
//
// Its code and the Elgato library it uses made into one file (esbuild), so it needs no npm install; then the
// plugin's folder (manifest, pictures, its settings pages, that file) zipped as a .streamDeckPlugin, the file
// Stream Deck installs or updates from when it's opened. Written to crates/app/streamdeck/, where the app
// takes it in (and gives it to Stream Deck when the one there is older; see crates/app/src/streamdeck.rs).
// The release script runs this first.

import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { readdirSync, statSync } from "node:fs";
import { deflateRawSync } from "node:zlib";
import { join, resolve } from "node:path";

const SRC = resolve("streamdeck/com.shwylie.scclient.sdPlugin");
const OUT = resolve("crates/app/streamdeck");
const stage = join(tmpdir(), "omnipulse-sd-pack");
const dir = join(stage, "com.shwylie.scclient.sdPlugin");
rmSync(stage, { recursive: true, force: true });
mkdirSync(join(dir, "bin"), { recursive: true });

// 1. its code, with the library in it
if (!existsSync(join(SRC, "node_modules", "@elgato", "streamdeck"))) {
  execFileSync("npm", ["install", "--no-audit", "--no-fund"], { cwd: SRC, stdio: "inherit", shell: true });
}
execFileSync("npx", ["--yes", "esbuild@0.24.0", join(SRC, "bin", "plugin.js"), "--bundle", "--platform=node", "--format=esm", "--target=node20",
  "--outfile=" + join(dir, "bin", "plugin.js"),
  // (an ES module that's had require() put in it by a library: given one)
  // (quoted whole: it goes through the shell, which would split it at its spaces)
  `"--banner:js=import { createRequire } from 'module'; const require = createRequire(import.meta.url);"`], { stdio: "inherit", shell: true });

// 2. the rest of its folder
for (const part of ["manifest.json", "imgs", "ui"]) cpSync(join(SRC, part), join(dir, part), { recursive: true });

// 3. zipped, its folder at the top (Stream Deck's own format)
mkdirSync(OUT, { recursive: true });
const zip = join(stage, "OmniPulse.zip");
writeFileSync(zip, zipOf(dir, "com.shwylie.scclient.sdPlugin"));
const out = join(OUT, "OmniPulse.streamDeckPlugin");
writeFileSync(out, readFileSync(zip));
const version = JSON.parse(readFileSync(join(SRC, "manifest.json"), "utf8")).Version;
writeFileSync(join(OUT, "version.txt"), version + "\n");
console.log("Packed the Stream Deck plugin " + version + " into " + out);

// A zip of a folder, written here (each entry's path with forward slashes, as zips want: Windows PowerShell's
// Compress-Archive writes backslashes, which Stream Deck may not take)
function zipOf(folder, top) {
  const files = [];
  const walk = (d, rel) => { for (const n of readdirSync(d)) { const p = join(d, n), r = rel ? rel + "/" + n : n; if (statSync(p).isDirectory()) walk(p, r); else files.push([p, top + "/" + r]); } };
  walk(folder, "");
  const crcTable = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc32 = (b) => { let c = 0xffffffff; for (const x of b) c = crcTable[(c ^ x) & 0xff] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
  const parts = [], central = [];
  let at = 0;
  for (const [path, name] of files) {
    const data = readFileSync(path), packed = deflateRawSync(data), nm = Buffer.from(name, "utf8"), crc = crc32(data);
    const head = Buffer.alloc(30);
    head.writeUInt32LE(0x04034b50, 0); head.writeUInt16LE(20, 4); head.writeUInt16LE(0x0800, 6); head.writeUInt16LE(8, 8);
    head.writeUInt32LE(crc, 14); head.writeUInt32LE(packed.length, 18); head.writeUInt32LE(data.length, 22); head.writeUInt16LE(nm.length, 26);
    parts.push(head, nm, packed);
    const c = Buffer.alloc(46);
    c.writeUInt32LE(0x02014b50, 0); c.writeUInt16LE(20, 4); c.writeUInt16LE(20, 6); c.writeUInt16LE(0x0800, 8); c.writeUInt16LE(8, 10);
    c.writeUInt32LE(crc, 16); c.writeUInt32LE(packed.length, 20); c.writeUInt32LE(data.length, 24); c.writeUInt16LE(nm.length, 28); c.writeUInt32LE(at, 42);
    central.push(c, nm);
    at += head.length + nm.length + packed.length;
  }
  const cd = Buffer.concat(central), end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0); end.writeUInt16LE(files.length, 8); end.writeUInt16LE(files.length, 10); end.writeUInt32LE(cd.length, 12); end.writeUInt32LE(at, 16);
  return Buffer.concat([...parts, cd, end]);
}
