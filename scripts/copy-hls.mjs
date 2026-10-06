import { copyFileSync, existsSync } from "node:fs";

const src = "node_modules/hls.js/dist/hls.min.js";
if (!existsSync(src)) {
  console.error("hls.js was not found. Run: npm install");
  process.exit(1);
}
copyFileSync(src, "dist/hls.min.js");
console.log("Copied hls.js into dist/");
