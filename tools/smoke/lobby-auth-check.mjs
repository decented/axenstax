#!/usr/bin/env node
// One-off verification for the lobby/request-access QR refresh + countdown UX.
// Run from tools/smoke after the game site is up on :8094:
//   node lobby-auth-check.mjs
// Exits non-zero on any failure.

import { chromium } from "playwright";

const BASE = process.env.AXE_SMOKE_BASE ?? "https://localhost:8094";
const failures = [];

const browser = await chromium.launch({ headless: true });
const ctx = await browser.newContext({ ignoreHTTPSErrors: true });
const page = await ctx.newPage();

const consoleErrors = [];
page.on("pageerror", (e) => consoleErrors.push(`pageerror: ${e.message}`));
page.on("console", (m) => {
  if (m.type() !== "error") return;
  const t = m.text();
  // Browsers log all non-2xx fetches as console.error even when the JS
  // handles them (e.g. /auth/whoami returns 401 when no cookie — request-auth.js
  // treats !res.ok as "not signed in" and runs the QR flow). Filter those out;
  // we want real JS errors only.
  if (/Failed to load resource.*40\d/.test(t)) return;
  consoleErrors.push(`console.error: ${t}`);
});

const challengeHits = [];
page.on("request", (req) => {
  if (req.url().endsWith("/auth/challenge") && req.method() === "POST") {
    challengeHits.push(Date.now());
  }
});

for (const path of ["/", "/request-access"]) {
  console.log(`\n=== ${path} ===`);
  challengeHits.length = 0;
  consoleErrors.length = 0;

  await page.goto(`${BASE}${path}`, { waitUntil: "networkidle" });

  // Wait for the QR (svg) to appear — proves auth flow ran.
  try {
    await page.waitForSelector("#qr-slot svg", { timeout: 5000 });
  } catch {
    failures.push(`${path}: QR <svg> never rendered`);
    continue;
  }

  // Countdown should be visible and match m:ss.
  const countdownText = await page.textContent("#qr-countdown");
  const countdownVisible = await page.isVisible("#qr-countdown");
  if (!countdownVisible || !/Expires in \d+:\d{2}/.test(countdownText ?? "")) {
    failures.push(`${path}: countdown not visible / wrong format: ${JSON.stringify(countdownText)}`);
  } else {
    console.log(`  countdown: "${countdownText}" ✓`);
  }

  // Refresh button should be visible.
  const refreshVisible = await page.isVisible("#qr-refresh");
  if (!refreshVisible) {
    failures.push(`${path}: #qr-refresh not visible`);
  } else {
    console.log(`  refresh button: visible ✓`);
  }

  // Should have hit /auth/challenge exactly once on first render.
  if (challengeHits.length !== 1) {
    failures.push(`${path}: expected 1 /auth/challenge hit on load, got ${challengeHits.length}`);
  } else {
    console.log(`  /auth/challenge on load: 1 hit ✓`);
  }

  // Click refresh — should fire a second /auth/challenge.
  const beforeClick = challengeHits.length;
  await page.click("#qr-refresh");
  await page.waitForFunction(
    (n) => true, // dummy — we'll wait for the network event instead
    beforeClick,
  ).catch(() => {});
  // Wait up to 3s for the next challenge POST.
  for (let i = 0; i < 30 && challengeHits.length === beforeClick; i++) {
    await new Promise((r) => setTimeout(r, 100));
  }
  if (challengeHits.length !== beforeClick + 1) {
    failures.push(`${path}: refresh click did not fire a new /auth/challenge (had ${beforeClick}, now ${challengeHits.length})`);
  } else {
    console.log(`  refresh click → new /auth/challenge ✓`);
  }

  // Diagnostic logs should be in console (info-level, not errors).
  const allLogs = await page.evaluate(() => "console.log diagnostics are best-effort, not asserted here");
  void allLogs;

  // No page errors / console.errors.
  if (consoleErrors.length > 0) {
    failures.push(`${path}: ${consoleErrors.length} console error(s):\n    ${consoleErrors.join("\n    ")}`);
  } else {
    console.log(`  no console errors ✓`);
  }
}

await browser.close();

if (failures.length > 0) {
  console.error("\nFAIL:");
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}
console.log("\nOK: lobby + request-access QR refresh UX all wired");
