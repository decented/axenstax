#!/usr/bin/env node
// Forbidden-symbol gate: prove the web bundle carries none of the native-only
// chat surface, and none of the feedback channel (mailbox / NIP-59 / /bug /
// /idea) the browser build no longer has (owner decision, 2026-10-01).
//
// WHY THIS EXISTS
//
//   The web build is the anonymous local taster: no login, no multiplayer, no
//   Stash, no analytics, and (T0-4, 2026-10-05) OFFLINE: the page may talk only
//   to its own origin (CSP `connect-src 'self'`, pinned in the game site's
//   tests/test_offline_taster.py), so no relay / Blossom / Stash / Beacon code
//   and no `relay.trotters.cc` may be on the page or in its runtime JS. World chat would make it a service, and a service that
//   carries a child's words is the regulated thing this project is built not to
//   be (CLAUDE.md, red line 3). Compile-time `cfg` is what keeps chat out; this
//   gate is what proves the `cfg` is still doing its job after somebody
//   refactors in six months.
//
// WHAT IT CHECKS, AND WHAT IT DELIBERATELY DOES NOT
//
//   It checks the surface that is unambiguously native-only: the room plug (a
//   Node subprocess), guardian copy (NIP-17 over the family's relays), the
//   guardian policy file, and the custom URL scheme.
//
//   It also checks the feedback channel: the browser build has NO /bug, /idea
//   or mailbox. Markers are specific (never the bare word "mailbox" — wgpu's
//   `PresentMode::Mailbox` contains it). A second pass looks at the game site's
//   static JS (what index.html loads at runtime, which is NOT in the trunk dist)
//   for the retired mailbox/NIP-59 files and globals.
//
//   It does NOT check for the permission rule itself (`comms.rs`). That module
//   compiles on both targets ON PURPOSE — a permission rule compiled on one
//   target is a permission rule tested on one target — so its presence in the
//   bundle is correct, not a leak. Do not "tighten" this gate by adding it.
//
// THE CANARY
//
//   A grep that finds nothing is indistinguishable from a grep that cannot see.
//   If a future bundler minifies, mangles or compresses the payload out of
//   reach, every forbidden marker would silently stop matching and this gate
//   would print a confident pass forever. So it also looks for a string it
//   KNOWS is in the bundle, and fails if that is missing — the same principle
//   as check.sh's own header: a gate that skips itself is worse than no gate.

import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { resolve } from "node:path";

const DIST = resolve(
  new URL(".", import.meta.url).pathname,
  "..",
  "..",
  "game",
  "engine",
  "dist",
);

// Markers that must NOT appear in a web build. Each carries the reason, which
// is printed on a hit so whoever broke it knows what boundary they crossed.
const FORBIDDEN = [
  ["kithmoot", "the room plug spawns a Node process; there is no such thing on web"],
  ["kithmoot-agent", "room keeper subprocess — native only"],
  ["guardian-policy", "the guardian-signed policy file is read from a native config dir"],
  ["axenstax-contacts:v1", "contacts import is native only"],
  ["ChatSayPacket", "chat on the transport is native only"],
  ["ChatDeliverPacket", "chat on the transport is native only"],
  ["/room invite", "room commands are native only"],
  ["haft-owner-proof", "agent ownership proof — native only"],
  ["axenstax://invite/", "online play by contact is native only — the web build is the anonymous local taster"],
  ["signet:contacts:proj:", "Signet contacts sync (signet/contacts_wire) is native only"],
  ["signet-contacts:pairing-code:v1", "Signet contacts pairing is native only"],
  // The web feedback channel — removed 2026-10-01. Native keeps its mailbox.
  ["native_mailbox", "the native mailbox module must not be compiled into the web build"],
  ["AxeMailbox", "the web lobby mailbox (mailbox.js) was removed — no feedback channel on web"],
  ["AxeNip59", "the web NIP-59 gift-wrap helper (nip59.js) was removed — no feedback channel on web"],
  ["axenstax_mailbox", "the engine<->JS mailbox bridge was removed — no feedback channel on web"],
  ["__axenstax_open_mailbox", "the engine<->JS mailbox bridge was removed — no feedback channel on web"],
  ["__axenstax_feedback_context", "the web feedback context hook (wasm_feedback.rs) was removed"],
  ["static/mailbox.js", "index.html must not load the removed mailbox script"],
  ["static/nip59.js", "index.html must not load the removed NIP-59 script"],
  ["Report a bug to the makers", "/bug is native only — it must not be compiled into the web build"],
  ["sending to the makers", "/bug and /idea are native only — their text must not be in the web build"],
  ["test-verdict", "Test Lab verdicts rode the removed web mailbox"],
];

// The game site's runtime JS (served at /static/*, loaded by index.html). Not in
// the trunk dist, so scanned separately. Vendored bundles are skipped.
const SITE_STATIC = resolve(
  new URL(".", import.meta.url).pathname,
  "..",
  "sites",
  "game",
  "static",
);
const FORBIDDEN_STATIC_FILES = [
  "mailbox.js",
  "nip59.js",
  // T0-4 offline taster: the Stash/relay/Beacon/identity glue, all deleted.
  "beacon.js",
  "relay-query.js",
  "persona-handle.js",
  "noble-curves.js",
  "_debug-monitor.js",
];
// Paths (relative to the game site's static dir) that must not exist, even in a
// subdirectory — the vendored Stash / relay / Beacon / NIP-46 bundles.
const FORBIDDEN_STATIC_PATHS = [
  "vendor/stash.iife.js",
  "vendor/relay.iife.js",
  "vendor/beacon.iife.js",
  "vendor/nostr-tools-nip46.iife.js",
];
// Unloaded legacy file kept only because tools/feedback-tests/gamestr.test.mjs
// still unit-tests it. index.html does not load it, so it is exempt from the
// offline markers below. Delete both together, then drop this entry.
const OFFLINE_EXEMPT_STATIC = new Set(["gamestr.js"]);
const FORBIDDEN_STATIC_MARKERS = [
  "AxeMailbox",
  "AxeNip59",
  "axenstax_mailbox",
  "__axenstax_mailbox_enqueue",
  "__axenstax_open_mailbox",
];

// Offline-taster markers (T0-4). Checked in the page (trunk's dist/index.html AND
// the game/engine/index.html source) and in the site's runtime static JS. NOT in
// the .wasm: `relay.trotters.cc` legitimately survives there as the
// FORBIDDEN_RELAY_HOST lint constant (world_room.rs), which exists to REFUSE it.
const OFFLINE_MARKERS = [
  ["relay.trotters.cc", "the web taster is offline — it must not name any AxeNStax relay (red line 2)"],
  ["axenstax-relay", "the relay <meta> tag fed the removed Stash/Beacon relay clients"],
  ["AxeStash", "the Stash cloud-save SDK was removed from the web build"],
  ["AxeRelay", "the Nostr relay client bundle was removed from the web build"],
  ["AxeBeacon", "the Beacon sharing SDK was removed from the web build"],
  ["AxeCloud", "the web cloud-save client (cloud.js AxeCloud) was removed"],
  ["AxeNostrNip46", "the NIP-46 bunker-signer bundle was removed — no web sign-in"],
  ["AxeHandle", "the persona-handle relay lookup was removed — no web identity"],
  ["AxeNoble", "noble-curves only served the removed Beacon/persona verification"],
  ["__axenstax_get_signer", "no signer exists on web — nothing defines or may call it"],
  ["stash.iife", "index.html must not load the Stash bundle"],
  ["relay.iife", "index.html must not load the relay-client bundle"],
  ["beacon.iife", "index.html must not load the Beacon bundle"],
  ["nostr-tools-nip46", "index.html must not load the NIP-46 bundle"],
  ["static/beacon.js", "index.html must not load the removed Beacon glue"],
  ["static/relay-query.js", "index.html must not load the removed relay-query bridge"],
  ["static/persona-handle.js", "index.html must not load the removed persona-handle lookup"],
  ["static/noble-curves.js", "index.html must not load the removed noble-curves loader"],
];
// The game page SOURCE (trunk input). dist/index.html is scanned with the bundle.
const PAGE_SOURCE = resolve(
  new URL(".", import.meta.url).pathname,
  "..",
  "..",
  "game",
  "engine",
  "index.html",
);

// Strings we know are in any real bundle. If none is found, the gate cannot see
// into the payload and must fail rather than pass.
const CANARIES = ["axenstax", "wasm"];

function bundleFiles() {
  let names;
  try {
    names = readdirSync(DIST);
  } catch {
    console.error(
      `FAIL: no trunk output at ${DIST} — run \`trunk build\` first.\n` +
        "This gate cannot pass without a bundle to inspect.",
    );
    process.exit(1);
  }
  const files = [];
  for (const name of names.sort()) {
    const path = resolve(DIST, name);
    if (!statSync(path).isFile()) continue;
    if (!/\.(wasm|js|html|css)$/.test(name)) continue;
    files.push({ name, path });
  }
  if (files.length === 0) {
    console.error(`FAIL: ${DIST} holds no .wasm/.js/.html/.css to inspect.`);
    process.exit(1);
  }
  return files;
}

const files = bundleFiles();
let canaryFound = false;
const hits = [];
let totalBytes = 0;

for (const { name, path } of files) {
  const buf = readFileSync(path);
  totalBytes += buf.length;
  // latin1 keeps every byte addressable as a character, so a marker embedded in
  // a .wasm data section matches just as it would in .js.
  const text = buf.toString("latin1").toLowerCase();

  for (const canary of CANARIES) {
    if (text.includes(canary)) canaryFound = true;
  }
  for (const [marker, why] of FORBIDDEN) {
    if (text.includes(marker.toLowerCase())) {
      hits.push({ name, marker, why });
    }
  }
  if (name.endsWith(".html")) {
    for (const [marker, why] of OFFLINE_MARKERS) {
      if (text.includes(marker.toLowerCase())) {
        hits.push({ name, marker, why });
      }
    }
  }
}

// --- Page source (what trunk builds the dist page from) ---------------------
if (existsSync(PAGE_SOURCE)) {
  const text = readFileSync(PAGE_SOURCE, "latin1").toLowerCase();
  for (const [marker, why] of OFFLINE_MARKERS) {
    if (text.includes(marker.toLowerCase())) {
      hits.push({ name: "game/engine/index.html", marker, why });
    }
  }
}

// --- Second pass: the game site's static JS -------------------------------
for (const name of readdirSync(SITE_STATIC).sort()) {
  const path = resolve(SITE_STATIC, name);
  if (!statSync(path).isFile()) continue;
  if (FORBIDDEN_STATIC_FILES.includes(name)) {
    hits.push({
      name: `sites/game/static/${name}`,
      marker: name,
      why: "this retired web feedback file must not exist — the browser build has no feedback channel",
    });
    continue;
  }
  if (!name.endsWith(".js")) continue;
  const text = readFileSync(path, "latin1");
  if (!OFFLINE_EXEMPT_STATIC.has(name)) {
    for (const [marker, why] of OFFLINE_MARKERS) {
      if (text.includes(marker)) {
        hits.push({ name: `sites/game/static/${name}`, marker, why });
      }
    }
  }
  for (const marker of FORBIDDEN_STATIC_MARKERS) {
    if (text.includes(marker)) {
      hits.push({
        name: `sites/game/static/${name}`,
        marker,
        why: "web feedback/mailbox bridge — removed, the browser build has no feedback channel",
      });
    }
  }
}

for (const rel of FORBIDDEN_STATIC_PATHS) {
  if (existsSync(resolve(SITE_STATIC, rel))) {
    hits.push({
      name: `sites/game/static/${rel}`,
      marker: rel,
      why: "this retired Stash/relay/Beacon/identity bundle must not exist — the web taster is offline",
    });
  }
}

const mib = (totalBytes / 1024 / 1024).toFixed(2);
console.log(
  `Scanned ${files.length} bundle file(s), ${mib} MiB, for ${FORBIDDEN.length} forbidden markers (+${OFFLINE_MARKERS.length} offline-taster markers on the page and its JS).`,
);

if (!canaryFound) {
  console.error(
    "\nFAIL: canary string not found in the bundle.\n" +
      "This gate could not see into the payload, so its 'no forbidden markers'\n" +
      "result means nothing. Something changed about how the bundle is built or\n" +
      "encoded — fix the gate before trusting a pass.",
  );
  process.exit(1);
}

if (hits.length > 0) {
  console.error("\nFAIL: native-only or networked surface (chat / feedback / relay / Stash) found in the web build:\n");
  for (const h of hits) {
    console.error(`  ${h.name}: "${h.marker}"`);
    console.error(`    ${h.why}`);
  }
  console.error(
    "\nThe web build is the anonymous local taster. Chat, the room plug,\n" +
      "guardian copy and the feedback mailbox (/bug, /idea) are native only, by\n" +
      "cfg. Something is no longer gated — see docs/foundations/2026-09-05-world-chat.md §6.",
  );
  process.exit(1);
}

console.log("OK: no native-only chat/feedback surface and no relay/Stash/Beacon code in the web build (canary seen).");
