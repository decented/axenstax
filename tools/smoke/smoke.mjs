#!/usr/bin/env node
// Playwright smoke test: does /game serve a valid WASM bundle that the
// trunk-emitted loader can fetch? Confirms:
//   1. / returns 200 (website is up)
//   2. /game returns 200 (serves the game index — open access since 2026-05-09)
//   3. The WASM asset 200s with Content-Type application/wasm
//   4. The JS loader 200s
// Saves a screenshot of the /game landing page to tools/smoke/out/.
//
// Path history:
//   pre-2026-04-30: /play/ + WASM mounted at /play/<asset>
//   post-three-site-split (2026-04-30): /game (root-mounted WASM at /<asset>)
//   post-open-access (2026-05-09): no waitlist; /game serves engine directly

import { chromium } from "playwright";
import { mkdirSync, readdirSync } from "node:fs";
import { resolve } from "node:path";

const BASE = process.env.AXE_SMOKE_BASE ?? "https://localhost:8094";
const OUT_DIR = resolve(new URL(".", import.meta.url).pathname, "out");
const DIST = resolve(new URL(".", import.meta.url).pathname, "..", "..", "game", "engine", "dist");

mkdirSync(OUT_DIR, { recursive: true });

// Find the hashed WASM + JS filenames trunk emitted.
let wasmName = null;
let jsName = null;
for (const n of readdirSync(DIST)) {
  if (n.endsWith(".wasm")) wasmName = n;
  if (n.endsWith(".js") && n.startsWith("axenstax-engine-")) jsName = n;
}
if (!wasmName || !jsName) {
  console.error(`FAIL: trunk dist at ${DIST} missing .wasm or .js — run \`trunk build\` first`);
  process.exit(1);
}

const failures = [];
const browser = await chromium.launch({ headless: true });
const ctx = await browser.newContext({ ignoreHTTPSErrors: true });
const page = await ctx.newPage();

async function expectStatus(path, label, allowed) {
  // Don't follow redirects — we want to assert on the gate's initial response,
  // not whatever the redirect chain ends at.
  const res = await page.request.get(`${BASE}${path}`, { maxRedirects: 0 });
  const status = res.status();
  if (!allowed.includes(status)) {
    failures.push(`${label}: ${path} → ${status} (wanted one of ${allowed.join(", ")})`);
    return null;
  }
  return res;
}

// Restructure 2026-06-27: "/" IS the game (serves the WASM engine, 200). Legacy
// "/game" permanently redirects to "/" (301; tolerate 200 if the client follows
// it). WASM + JS assets mount at root — gate-public per Spec 2 Task 5 (HTML gate
// is the control, not asset ACL).
const rootRes = await expectStatus("/", "root (game)", [200]);
await expectStatus("/game", "legacy /game → / (301)", [200, 301]);
const wasmRes = await expectStatus(`/${wasmName}`, "wasm asset", [200]);
await expectStatus(`/${jsName}`, "js loader", [200]);

// Feedback admin gate (spec §Phase 1d). Anonymous hits should either
// redirect to /game / / or get a 403 — never a 200 that leaks the board.
await expectStatus("/feedback/board", "feedback board gate", [302, 303, 403]);
await expectStatus("/api/feedback/board.json", "feedback board JSON gate", [401, 403]);

if (rootRes && rootRes.status() === 200) {
  // "/" serves the game index now — validate it looks like the engine bundle.
  const body = await rootRes.text();
  const looksLikeGame = /data-trunk|wasm|canvas/i.test(body);
  if (!looksLikeGame) {
    failures.push(`/game body did not look like game index (no data-trunk/wasm/canvas marker)`);
  }
}

if (wasmRes) {
  const ct = wasmRes.headers()["content-type"] ?? "";
  if (!ct.includes("wasm")) {
    failures.push(`wasm content-type was "${ct}", expected application/wasm`);
  }
  const body = await wasmRes.body();
  if (body.length < 100 || body.subarray(0, 4).toString("hex") !== "0061736d") {
    failures.push("wasm body is not a valid WebAssembly module (missing magic 0061736d)");
  }
}

await page.goto(`${BASE}/game`, { waitUntil: "domcontentloaded" });
await page.screenshot({ path: resolve(OUT_DIR, "play-landing.png"), fullPage: true });

await browser.close();

if (failures.length > 0) {
  console.error("FAIL:");
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}

console.log("OK: root + /game + wasm + js asset all served cleanly");
console.log(`Screenshot: ${resolve(OUT_DIR, "play-landing.png")}`);
