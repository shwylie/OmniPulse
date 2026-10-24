// Testers' Seamless Mix ratings, from the relay: appended to state/remote-mix-feedback.jsonl in your data
// folder (each line: its number, the install it came from, when it arrived, and the rating as the app wrote
// it). Carries on after the last one fetched. Needs the relay's RATINGS_KEY:
//   RATINGS_KEY=... node scripts/mix-ratings.mjs
import fs from "node:fs";
import path from "node:path";

const relay = process.env.RELAY_URL || "https://omnipulse-jam.shwylie101.workers.dev";
const key = process.env.RATINGS_KEY;
if (!key) { console.error("Set RATINGS_KEY first (the one set on the relay with wrangler secret put)."); process.exit(1); }
const dir = path.join(process.env.APPDATA, "OmniPulse", "state");
fs.mkdirSync(dir, { recursive: true });
const file = path.join(dir, "remote-mix-feedback.jsonl");
const have = fs.existsSync(file) ? fs.readFileSync(file, "utf8").trim().split("\n").filter(Boolean) : [];
let after = have.length ? JSON.parse(have[have.length - 1]).n : 0;
let got = 0;
for (;;) {
  const r = await fetch(relay + "/api/mix-ratings?after=" + after, { headers: { "x-ratings-key": key } });
  if (!r.ok) { console.error("The relay said " + r.status + ": " + (await r.text())); process.exit(1); }
  const lines = (await r.text()).split("\n").filter(Boolean);
  if (!lines.length) break;
  fs.appendFileSync(file, lines.join("\n") + "\n");
  got += lines.length;
  after = JSON.parse(lines[lines.length - 1]).n;
}
console.log(got + " new ratings, in " + file);
