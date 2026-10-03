#!/usr/bin/env node
// Browser-load smoke for the vendored nostr-tools/nip46 IIFE bundle (Spec 7).
// The bundle was only node-sandbox verified at delivery; this confirms it
// actually parses + executes in Chromium and populates the expected globals.
//
// Run from tools/smoke after the game site is up on :8094:
//   node nip46-vendor-check.mjs

import { chromium } from "playwright";

const BASE = process.env.AXE_SMOKE_BASE ?? "https://localhost:8094";
const failures = [];

const browser = await chromium.launch({ headless: true });
const ctx = await browser.newContext({ ignoreHTTPSErrors: true });
const page = await ctx.newPage();

const consoleErrors = [];
page.on("pageerror", (e) => consoleErrors.push(`pageerror: ${e.message}`));
page.on("console", (m) => {
  if (m.type() === "error") consoleErrors.push(`console.error: ${m.text()}`);
});

// Load a blank-origin page on the same host so the relative <script src>
// resolves and same-origin policies are satisfied. The /static/ path serves
// the vendor file with a permissive MIME.
await page.setContent(`<!doctype html>
<html><head><meta charset="utf-8"><title>nip46 vendor smoke</title></head>
<body>
  <script src="${BASE}/static/vendor/nostr-tools-nip46.iife.js"></script>
</body></html>`, { waitUntil: "load" });

const probe = await page.evaluate(async () => {
  const out = {
    hasGlobal: typeof window.AxeNostrNip46 === "object" && window.AxeNostrNip46 !== null,
    bunkerSignerType: typeof (window.AxeNostrNip46 && window.AxeNostrNip46.BunkerSigner),
    parseBunkerInputType: typeof (window.AxeNostrNip46 && window.AxeNostrNip46.parseBunkerInput),
    fromBunkerType: typeof (
      window.AxeNostrNip46 && window.AxeNostrNip46.BunkerSigner && window.AxeNostrNip46.BunkerSigner.fromBunker
    ),
  };
  // parseBunkerInput is async (per nostr-tools v2.23.3) — it falls through
  // to a NIP-05 lookup if the bunker:// regex doesn't match, hence the async
  // wrapper. For a well-formed bunker:// URI it resolves synchronously
  // without hitting the network.
  try {
    const uri = "bunker://000000000000000000000000000000000000000000000000000000000000abcd?relay=wss%3A%2F%2Frelay.example";
    const parsed = await window.AxeNostrNip46.parseBunkerInput(uri);
    out.parseValidReturn = {
      ok: !!parsed,
      pubkey: parsed && parsed.pubkey || null,
      relayCount: parsed && Array.isArray(parsed.relays) ? parsed.relays.length : -1,
      relayFirst: parsed && Array.isArray(parsed.relays) && parsed.relays.length > 0 ? parsed.relays[0] : null,
    };
  } catch (e) {
    out.parseValidReturn = { ok: false, reason: `THREW: ${e && e.message}` };
  }
  return out;
});

if (!probe.hasGlobal) failures.push("window.AxeNostrNip46 missing");
if (probe.bunkerSignerType !== "function") {
  failures.push(`window.AxeNostrNip46.BunkerSigner type=${probe.bunkerSignerType}, expected function`);
}
if (probe.parseBunkerInputType !== "function") {
  failures.push(`window.AxeNostrNip46.parseBunkerInput type=${probe.parseBunkerInputType}, expected function`);
}
if (probe.fromBunkerType !== "function") {
  failures.push(`BunkerSigner.fromBunker type=${probe.fromBunkerType}, expected function`);
}
// parseBunkerInput on a valid-shape URI should return pubkey ending in the
// synthesized 'abcd' suffix and one parsed relay. The bundle returns a
// {pubkey, relays, secret} shape per nostr-tools v2.x.
if (!probe.parseValidReturn || probe.parseValidReturn.ok !== true) {
  failures.push(`parseBunkerInput on valid URI failed: ${JSON.stringify(probe.parseValidReturn)}`);
} else {
  if (probe.parseValidReturn.isPromise) {
    failures.push(`parseBunkerInput unexpectedly returned a Promise (NIP-05 fallback path) — bunker:// regex didn't match`);
  }
  if (!probe.parseValidReturn.pubkey || !probe.parseValidReturn.pubkey.endsWith("abcd")) {
    failures.push(`parseBunkerInput pubkey unexpected: ${JSON.stringify(probe.parseValidReturn)}`);
  }
  if (probe.parseValidReturn.relayCount !== 1) {
    failures.push(`parseBunkerInput relayCount=${probe.parseValidReturn.relayCount}, expected 1; full=${JSON.stringify(probe.parseValidReturn)}`);
  }
}

if (consoleErrors.length > 0) {
  failures.push(`${consoleErrors.length} console error(s):\n    ${consoleErrors.join("\n    ")}`);
}

await browser.close();

if (failures.length > 0) {
  console.error("FAIL:");
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}

console.log("OK: nostr-tools/nip46 vendor bundle loads in Chromium");
console.log(`  window.AxeNostrNip46.BunkerSigner is a function ✓`);
console.log(`  window.AxeNostrNip46.parseBunkerInput is a function ✓`);
console.log(`  BunkerSigner.fromBunker is reachable ✓`);
console.log(`  parseBunkerInput round-trip → pubkey=…${probe.parseValidReturn.pubkey.slice(-8)}, relays=[${probe.parseValidReturn.relayFirst}] ✓`);
