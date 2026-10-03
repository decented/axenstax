#!/usr/bin/env node
// tools/bake-beacon.js — BUILD-TIME: embed the official npub's published override
// sets into a static catalogue the engine ships (offline, both platforms; no
// runtime Rust Beacon). Re-run when official content changes. beacon/CONSUMING.md §4.
// Usage: node tools/bake-beacon.js <official_pubkey_hex> [relay_wss] [blossom_url] > game/engine/assets/official_overrides.json
//
// LIVE RUN (owner boundary — needs a real relay + Blossom; can't run on a CI build
// host with no network). Example:
//   node tools/bake-beacon.js <OFFICIAL_HEX> wss://relay.trotters.cc https://blossom.primal.net \
//     > game/engine/assets/official_overrides.json
//
// Output shape (Task 11's Rust loader serde-decodes this verbatim; bytes are a u8
// JSON array, NOT base64 — keeps a base64 crate out of the engine):
//   { "version": 1, "items": [ { "name", "contentType": "override-set", "bytes": [<u8>...] } ] }
// Each item's bytes[0] is the OVERRIDE_SET_VERSION prefix byte (= 2 on a Phase-5+
// build; the engine loader accepts both v1 and v2, migrating v1 → v2 on read).
//
// Bounds (the §9 consumer obligation for the offline booth — the build host must
// never embed unbounded content from a followed publisher):
//   MAX_ITEMS = 64        — at most this many override-set items are baked
//   MAX_BYTES = 2 MiB     — any blob larger than this is skipped, never embedded
// Every blob is sha256-verified against its advertised hash before it is embedded;
// a mismatch is FATAL (non-zero exit) — we never ship unverified bytes.
//
// READ-ONLY signer note: Beacon's read path — list(pubkeyHex) + fetch(item) — never
// calls signer.signEvent (verified against @forgesworn/beacon src/index.ts: list ->
// manifest.listForPubkey -> relay.query + schnorr-verify; fetch -> Blossom GET).
// getPublicKey is only touched by the no-arg list() branch, which we don't use. So a
// read-only stub signer is sufficient; signEvent throws to make any accidental write
// loud. (If a future SDK made reads require signing, the raw-relay-REQ + raw-Blossom
// fallback below would replace createBeacon — but today the SDK path is used.)

import { createHash } from 'node:crypto';

const APP = 'axenstax'; // AxeNStax namespace → the manifest/follow-set d-tag (SDK-derived).
const OVERRIDE_CONTENT_TYPE = 'override-set';
const MANIFEST_KIND = 30820; // BEACON_MANIFEST_KIND (used only by the raw fallback).
const MAX_ITEMS = 64;
const MAX_BYTES = 2 * 1024 * 1024; // 2 MiB per blob.
const RELAY_TIMEOUT_MS = 15000;

// Resolve the SDK whether the tool runs from the AxeNStax repo (sibling install) or
// against a Forgesworn checkout directly. Try the package name first; if it isn't
// installed, fall back to BEACON_SDK_PATH (point it at the built dist, e.g.
// BEACON_SDK_PATH=../forgesworn/beacon/dist/index.js). No machine-specific path is
// baked in — the owner runs this tool and supplies the checkout location.
async function loadBeaconSdk() {
  try {
    return await import('@forgesworn/beacon');
  } catch {
    const fallback = process.env.BEACON_SDK_PATH;
    if (!fallback) {
      die('cannot resolve @forgesworn/beacon — install it or set BEACON_SDK_PATH=<path-to>/beacon/dist/index.js');
    }
    return await import(fallback);
  }
}

function die(msg) {
  process.stderr.write(`bake-beacon: ${msg}\n`);
  process.exit(1);
}

function usage() {
  process.stderr.write(
    'Usage: node tools/bake-beacon.js <official_pubkey_hex> [relay_wss] [blossom_url] ' +
      '> game/engine/assets/official_overrides.json\n' +
      '  (no args = print this usage and exit non-zero — never silently emit an empty catalogue)\n',
  );
}

function sha256Hex(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

function isHex64(s) {
  return typeof s === 'string' && /^[0-9a-f]{64}$/i.test(s);
}

// --- Injected transports -----------------------------------------------------

// Read-only signer: reads need no signing; any write attempt is loud.
function readOnlySigner(officialHex) {
  return {
    getPublicKey: () => officialHex,
    async signEvent() {
      throw new Error('bake is read-only — refusing to sign');
    },
  };
}

// Blossom GET only (public, no auth). PUT is refused — the bake never writes.
function makeBlossom(blossomUrl) {
  const base = blossomUrl.replace(/\/+$/, '');
  return {
    async put() {
      throw new Error('bake is read-only — refusing to PUT to Blossom');
    },
    async get(blobHash) {
      const res = await fetch(`${base}/${blobHash}`);
      if (!res.ok) throw new Error(`Blossom GET ${blobHash} -> HTTP ${res.status}`);
      return new Uint8Array(await res.arrayBuffer());
    },
  };
}

// Minimal relay client over a single relay socket (Node 22 has a global WebSocket).
// Implements the RelayClient.query the SDK's list() needs; publish() is refused.
function makeRelayClient(relayUrl) {
  return {
    async publish() {
      throw new Error('bake is read-only — refusing to publish to relay');
    },
    // Fetch replaceable events for (pubkey, kind, dTag) via a one-shot REQ.
    query(pubkey, kind, dTag) {
      return new Promise((resolve, reject) => {
        let ws;
        try {
          ws = new WebSocket(relayUrl);
        } catch (e) {
          reject(new Error(`relay connect failed: ${e.message}`));
          return;
        }
        const subId = `bake-${Math.random().toString(36).slice(2)}`;
        const events = [];
        const filter = { kinds: [kind], authors: [pubkey] };
        if (dTag !== undefined) filter['#d'] = [dTag];
        const timer = setTimeout(() => {
          try {
            ws.close();
          } catch {}
          resolve(events); // EOSE not seen in time → return what we have.
        }, RELAY_TIMEOUT_MS);
        ws.addEventListener('open', () => {
          ws.send(JSON.stringify(['REQ', subId, filter]));
        });
        ws.addEventListener('message', (ev) => {
          let msg;
          try {
            msg = JSON.parse(typeof ev.data === 'string' ? ev.data : ev.data.toString());
          } catch {
            return;
          }
          if (!Array.isArray(msg)) return;
          if (msg[0] === 'EVENT' && msg[1] === subId && msg[2]) {
            events.push(msg[2]);
          } else if (msg[0] === 'EOSE' && msg[1] === subId) {
            clearTimeout(timer);
            try {
              ws.send(JSON.stringify(['CLOSE', subId]));
              ws.close();
            } catch {}
            resolve(events);
          }
        });
        ws.addEventListener('error', (e) => {
          clearTimeout(timer);
          reject(new Error(`relay socket error: ${e?.message ?? 'unknown'}`));
        });
      });
    },
  };
}

// --- Main --------------------------------------------------------------------

async function main() {
  const [officialHex, relayUrl = 'wss://relay.trotters.cc', blossomUrl = 'https://blossom.primal.net'] =
    process.argv.slice(2);

  if (!officialHex) {
    usage();
    process.exit(2); // no args: never silently emit an empty catalogue.
  }
  if (!isHex64(officialHex)) {
    die(`official pubkey must be 64 hex chars, got: ${officialHex}`);
  }

  const { createBeacon } = await loadBeaconSdk();
  const beacon = createBeacon({
    app: APP,
    signer: readOnlySigner(officialHex),
    blossom: makeBlossom(blossomUrl),
    relay: makeRelayClient(relayUrl),
    official: officialHex,
  });

  // 1. Read the official publisher's manifest (SDK schnorr-verifies it).
  let items;
  try {
    items = await beacon.list(officialHex);
  } catch (e) {
    die(`reading official manifest failed: ${e.message}`);
  }

  // 2. Keep only override-set items, capped at MAX_ITEMS.
  const overrideItems = items.filter((it) => it && it.contentType === OVERRIDE_CONTENT_TYPE);
  if (overrideItems.length > MAX_ITEMS) {
    process.stderr.write(
      `bake-beacon: capping ${overrideItems.length} override-set items to MAX_ITEMS=${MAX_ITEMS}\n`,
    );
  }
  const kept = overrideItems.slice(0, MAX_ITEMS);

  // 3. Fetch + verify each blob; embed verified bytes only.
  const out = [];
  for (const item of kept) {
    // Advisory size from the manifest — skip obvious oversize before downloading.
    if (typeof item.size === 'number' && item.size > MAX_BYTES) {
      process.stderr.write(
        `bake-beacon: skip "${item.name}" — advertised size ${item.size} > MAX_BYTES=${MAX_BYTES}\n`,
      );
      continue;
    }
    let bytes;
    try {
      bytes = await beacon.fetch(item); // GET + the SDK's own sha256 check.
    } catch (e) {
      die(`fetching blob for "${item.name}" failed: ${e.message}`);
    }
    // Authoritative size + integrity gate (don't trust the advisory size).
    if (bytes.length > MAX_BYTES) {
      process.stderr.write(
        `bake-beacon: skip "${item.name}" — actual size ${bytes.length} > MAX_BYTES=${MAX_BYTES}\n`,
      );
      continue;
    }
    if (item.blobHash) {
      const got = sha256Hex(bytes);
      if (got !== item.blobHash.toLowerCase()) {
        // FATAL: never embed unverified bytes.
        die(`sha256 mismatch for "${item.name}": expected ${item.blobHash}, got ${got}`);
      }
    }
    out.push({
      name: String(item.name ?? ''),
      contentType: OVERRIDE_CONTENT_TYPE,
      bytes: Array.from(bytes),
    });
  }

  process.stdout.write(JSON.stringify({ version: 1, items: out }) + '\n');
  process.stderr.write(`bake-beacon: baked ${out.length} override-set item(s)\n`);
}

main().catch((e) => die(e?.stack ?? String(e)));
