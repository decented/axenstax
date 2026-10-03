# Consuming Beacon in AxeNStax — pointer brief

**Status:** ✅ **CONSUMED 2026-06-05** — the migration in this brief is **built** on branch
`feat/workshop-beacon-share` (Spec 40 Phases D/E, `docs/goals/the-workshop-sharing.md`, `./check.sh`-green
at 2145 engine tests). `beacon.iife.js` is vendored → `window.AxeBeacon`; `tools/sites/game/static/beacon.js`
is the glue (capped Blossom transport + relay + signer); `open_stash.rs`'s manifest logic + `openstash.js`
+ `official_content.rs` are **retired**; the engine marshals bytes to the SDK; `tools/bake-beacon.js` +
`game/engine/src/official_overrides.rs` deliver the offline official catalogue. **Owner-only remainder:**
the live publish→adopt round-trip, the real official npub + live bake, and the polished Share/Browse UI.
The `@forgesworn/beacon` primitive itself is built (private repo `forgesworn/beacon`, mirrors
`@forgesworn/stash`). See [[project_beacon_public_distribution]].

## What Beacon is

The **public** sibling of Stash: signed, plaintext, content-addressed items a persona publishes for
anyone to follow + adopt. It **generalises** our local `open_stash` prototype
(`game/engine/src/open_stash.rs` kind 30820, `tools/sites/game/static/openstash.js`) into a standalone
Forgesworn primitive. Proposal: `docs/integrations/beacon/2026-06-04-beacon-public-content-primitive.md`.

## Canonical guide (read this to do the work)

`<workspace>/forgesworn/beacon/CONSUMING.md` — the full, concrete migration playbook. It lives with the
package so it stays in sync with the code. This brief is the AxeNStax-side index into it.

## The migration in one screen (per CONSUMING.md)

1. **Vendor** `beacon.iife.js` → `window.AxeBeacon` (esbuild `--global-name=AxeBeacon`, exactly like
   `stash.iife.js`; add a section to `tools/sites/game/static/vendor/REGENERATE.md`).
2. **Swap** `openstash.js`'s manifest role for a `beacon.js` glue that calls
   `window.AxeBeacon.createBeacon({ app:'axenstax', signer, blossom, relay, official })` — mirrors how
   `cloud.js` wires `window.AxeStash`. Keep the plaintext Blossom PUT/GET transport from `openstash.js`.
3. **Retire** `open_stash.rs`'s runtime manifest logic (`OpenStashManifest` / `build`/`parse_manifest_event`
   / `FollowedNpubs`) — the SDK now builds, parses, **and schnorr-verifies** the manifest in JS. **No
   parallel Rust implementation.** The engine keeps only thin marshalling to the JS Beacon externs.
4. **Bake** the offline booth's official content at BUILD time (a small Node tool reads the official
   npub's Beacon → embeds `{name,contentType,bytes}` JSON in the binary), so the booth works on **both**
   platforms with no runtime Rust Beacon impl.
5. **Posture:** live Beacon (publish/discover/adopt over the network) is **PWA-only**, like cloud save;
   native ships only the baked official catalogue.
6. **Kind/d-tag:** kind stays **30820** (Beacon adopts the kind `open_stash` already uses → no-op);
   `d` tag moves from the hardcoded `"axenstax-open-stash"` to the bare app namespace `"axenstax"`. **No
   live data to migrate** (the official npub is still a placeholder; nothing real is published yet).
7. **Safety UX:** `publish()` is a **deliberate, distinct, clearly-labelled public act** — separate from
   the private "Stash" save, with explicit "anyone can see your name and download this" copy and a
   first-publish confirm. Most important consumer obligation (kids' platform).

## What stays unchanged

- **Blossom** (same BUD-02 PUT + public GET; open-stash already uploaded plaintext).
- **Relay** `wss://relay.trotters.cc` ([[reference_trotters_relay]]) via the vendored `window.AxeRelay`
  (Beacon's `RelayClient` shape matches Stash's — one adapter serves both).
- **Signet signer** (`window.__axenstax_get_signer`) — Beacon needs only `signEvent` + `pubkey`, **no
  NIP-44**, so it works even when the cloud-save `capable()` gate doesn't. ([[feedback_npub_only_display]]:
  render npub; Beacon keys on hex — decode at the display boundary.)

## Boundaries

- **Forgesworn owns the primitive + the NIP** ([[feedback_signet_boundary]]); AxeNStax is the first
  reference consumer and drives the use case. Beacon is general-purpose (any app wanting public
  user-owned content distribution), not AxeNStax work repackaged.
- The official AxeNStax content npub is still a **placeholder** — wire the real hex pubkey into `official`
  + the bake tool when the owner provides it (the `official_axenstax_pubkey()` seam in `open_stash.rs`).
- **Live round-trip = owner-verified** (real relay + Blossom + a second identity). Exact steps in
  `CONSUMING.md` §"Ready to verify live". Same boundary as the Goal-2 open-stash live check.
