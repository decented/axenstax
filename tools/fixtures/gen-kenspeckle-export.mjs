#!/usr/bin/env node
// Regenerate the frozen Kenspeckle contacts-export fixture.
//
// WHY A FROZEN FIXTURE
//
//   The Rust contacts importer must parse a format produced by a TypeScript
//   library in another repo. Kenspeckle ships NO test vector for this format —
//   its vectors/ cover the bond ECDH and the companion rail only, and there is
//   no generator for a backup vector. Without a frozen artefact the two
//   implementations drift silently: the TS side changes a field, the Rust side
//   keeps parsing last year's shape, and nothing goes red until a real roster
//   fails to import on somebody's kitchen table.
//
//   So this generates the blob using KENSPECKLE'S OWN CODE, round-trips it back
//   through Kenspeckle's own importer before writing anything, and freezes the
//   bytes. The Rust test parses those exact bytes.
//
// THE FORMAT (verified 2026-09-05, and not what you would guess)
//
//   It is NOT NIP-44. `exportEntriesEncrypted(entries, key)` is a symmetric
//   self-backup: XChaCha20-Poly1305 under a raw 32-byte caller-supplied key,
//   laid out as nonce(24) || ciphertext || poly1305 tag(16), with no envelope
//   object. The plaintext is a bare JSON array of entries. Kenspeckle performs
//   no key derivation at all — the 32 bytes are the consumer's problem.
//
// USAGE
//
//   KENSPECKLE=<path-to-kenspeckle-checkout> node tools/fixtures/gen-kenspeckle-export.mjs
//
//   The checkout must have been built (`npm install && npm run build`), since
//   this imports from its dist/.
//
// WHAT THE ROSTER DELIBERATELY CONTAINS
//
//   One of each tier; a kin/child entry, because `relationship == "child"` on a
//   kin entry is the ONLY place `is_child` can be derived from (Kenspeckle has
//   no child flag of its own); a ken with no displayName, so the null case is
//   covered; and a secret in every field a consumer must strip — sharedSecret,
//   annotations including a private note, ownerPubkey, and a bondAssertion. The
//   Rust stripping test greps the persisted form for exactly those, so they
//   have to be present here or that test proves nothing.
//
//   The key is bytes 0x00..0x1f: deterministic, and obviously a test key at a
//   glance. It is NOT written to a file — this repo gitignores *.key so that a
//   real key can never be committed by accident, and forcing a test key past
//   that rule would blunt it. The Rust test builds the same 32 bytes inline.
//
// REGENERATION IS NOT BYTE-IDENTICAL
//
//   The nonce is random per call, so running this again produces a different
//   blob that decrypts to the same roster. That is correct, not a bug — but it
//   means the committed .bin is a SNAPSHOT, and regenerating it will show up as
//   a changed binary file in git. Only regenerate when the format or the roster
//   actually needs to change, and say which in the commit message.

import { writeFileSync } from "node:fs";
import { resolve } from "node:path";

const KENSPECKLE = process.env.KENSPECKLE;
if (!KENSPECKLE) {
  console.error(
    "Set KENSPECKLE to your kenspeckle checkout, e.g.\n" +
      "  KENSPECKLE=$HOME/src/kenspeckle node tools/fixtures/gen-kenspeckle-export.mjs",
  );
  process.exit(2);
}

const { exportEntriesEncrypted, importEntries } = await import(
  `${KENSPECKLE}/dist/backup.js`
);

const now = 1757100000;
const entries = [
  {
    pubkey: "a".repeat(64),
    ownerPubkey: "f".repeat(64),
    tier: "kin",
    displayName: "Mum",
    addedAt: now,
    relationship: "parent",
    sharedSecret: "11".repeat(32),
    verifiedAt: now,
    annotations: { label: "home", note: "PRIVATE NOTE MUST NOT PERSIST", blocked: false },
  },
  {
    pubkey: "b".repeat(64),
    ownerPubkey: "f".repeat(64),
    tier: "kin",
    displayName: "Wee Yin",
    addedAt: now,
    relationship: "child",
    sharedSecret: "22".repeat(32),
    verifiedAt: now,
  },
  {
    pubkey: "c".repeat(64),
    ownerPubkey: "f".repeat(64),
    tier: "kith",
    displayName: "Pal",
    addedAt: now,
    sharedSecret: "33".repeat(32),
    verifiedAt: now,
    bondAssertion: { mineId: "d".repeat(64), theirsId: "e".repeat(64) },
  },
  {
    pubkey: "0".repeat(64),
    ownerPubkey: "f".repeat(64),
    tier: "ken",
    addedAt: now,
    provenance: { source: "nip05", locator: "host@example.org", confirmedAt: now },
  },
];

const key = new Uint8Array(32);
for (let i = 0; i < 32; i++) key[i] = i;

const blob = exportEntriesEncrypted(entries, key);

// Round-trip through Kenspeckle's own importer before freezing anything. If the
// library cannot read what it just wrote, the fixture is worthless and we should
// find that out here rather than in a Rust test failure that looks like our bug.
const back = importEntries(blob, key);
if (back.length !== entries.length) {
  throw new Error(
    `round-trip length mismatch: wrote ${entries.length}, read back ${back.length}`,
  );
}

const OUT = resolve(
  new URL(".", import.meta.url).pathname,
  "..",
  "..",
  "game",
  "engine",
  "assets",
  "test",
);

writeFileSync(`${OUT}/kenspeckle-export.v1.bin`, Buffer.from(blob));
writeFileSync(
  `${OUT}/kenspeckle-export.v1.expected.json`,
  JSON.stringify(
    back.map((e) => ({
      pubkey: e.pubkey,
      display_name: e.displayName ?? null,
      tier: e.tier,
      is_child: e.tier === "kin" && e.relationship === "child",
    })),
    null,
    2,
  ) + "\n",
);

console.log(`blob: ${blob.length} bytes (nonce 24 + ct/tag ${blob.length - 24})`);
console.log(`entries: ${back.length}`);
console.log("key: bytes 0x00..0x1f (built inline by the Rust test; not written — *.key is gitignored)");
console.log(`written to ${OUT}`);
