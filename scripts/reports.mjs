// Reads the bug reports and feature requests the app's sent (kept by the relay), newest last:
//   REPORTS_KEY=... node scripts/reports.mjs [after]
// (REPORTS_KEY is the secret set in the Cloudflare dashboard; RATINGS_KEY works too while there's no REPORTS_KEY)
const key = process.env.REPORTS_KEY || process.env.RATINGS_KEY;
if (!key) { console.error("Set REPORTS_KEY first (the secret from the Cloudflare dashboard)."); process.exit(1); }
const after = process.argv[2] || "0";
const r = await fetch("https://omnipulse-jam.shwylie101.workers.dev/api/reports?after=" + encodeURIComponent(after), { headers: { "x-reports-key": key } });
if (!r.ok) { console.error("The relay said " + r.status + ": " + (await r.text())); process.exit(1); }
for (const line of (await r.text()).split("\n").filter(Boolean)) {
  const x = JSON.parse(line);
  console.log(`#${x.n}  ${new Date(x.got).toLocaleString()}  ${x.kind === "bug" ? "BUG" : "FEATURE"}  ${x.title || ""}`);
  console.log(`   ${x.version || "?"} on ${x.os || "?"}${x.contact ? ", contact: " + x.contact : ""}`);
  console.log("   " + String(x.text).replace(/\n/g, "\n   "));
  if (x.extra) console.log("   (" + x.extra + ")");
  console.log("");
}
