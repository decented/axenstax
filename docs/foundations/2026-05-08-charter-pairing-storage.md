# Charter pairing storage in `auth.js` — IDB-backed, multi-subject

**Status**: DELIVERED 2026-05-08 on `main` as **rev. 5 contract groundwork**. Then **SUPERSEDED 2026-05-09 by Charter spec rev. 7** (mechanism A — static-data evaluation): bunker pairing is *not* in the rev. 7 Phase 1 hot path, so the `axenstax_pair_charter` / `axenstax_get_charter_signer` / `BunkerSigner.fromBunker` surface this spec built has **no caller in the rev. 7 contract**. The module, IDB schema, and smoke harness are intentionally preserved on `main` as **referenceable groundwork for future mechanism B/C/D specs** (cumulative-state budgets, live-request extension flows, triggered clauses) where bunker pairing remains the right primitive. Rev. 5 review-round groundwork is also preserved on branch `preserved/charter-rev5-groundwork`.
**Date**: 2026-05-08 (status updated 2026-05-09).
**Memory rules in scope**: npub only display (any UI surface that shows a pairing record uses npub, hex stays internal), signet boundary (this is AxeNStax-internal storage, no Signet asks).
**See also**: rev. 7 spec at `<workspace>/forgesworn/signet-plans/docs/plans/2026-05-08-charter-schedule-clause-spec.md` and the architecture-broadening holodeck at `<workspace>/forgesworn/signet-plans/docs/reports/2026-05-08-charter-architecture-broadening-holodeck.md`. AxeNStax-side rev. 7 integration plan: pending (drafted as part of the rev. 7 round-4 acceptance).

---

## TL;DR

A parent uses signet-app to pair AxeNStax with each of their kids' Signet personas via `bunker://` URIs (per the Charter spec rev. 3 §Q5 Phase 1). AxeNStax's `auth.js` needs to:

1. **Accept** the URI when the parent pastes / scans it.
2. **Persist** the pairing (URI + metadata) across page reloads — the bunker connection token must survive a refresh.
3. **Look up** the right pairing by *kid pubkey* when `__axenstax_charter_check(subject, ...)` fires — a parent who's paired AxeNStax for both Tom and Sally should get the right pairing for whoever's signed in.
4. **Rehydrate** a `BunkerSigner` from the stored URI on demand (lazy — don't pre-connect at page load).

Storage layer: IndexedDB, matching the `world_store.js` pattern. Schema = one record per kid pubkey, holding the `bunker://` URI plus pairing metadata.

---

## Context pointers

- Existing IDB pattern: `tools/sites/game/static/world_store.js:1-30` (DB open, version, store creation, by-pubkey index).
- Existing localStorage pattern (cached pubkey): `tools/sites/game/static/auth.js:21,148-154`. **Don't** put the pairing here — pairings need IDB's structured storage and per-subject keying.
- Charter spec authoritative on the pairing UX side: `~/Documents/<workspace>/forgesworn/signet-plans/docs/plans/2026-05-08-charter-schedule-clause-spec.md` §Q5 Phase 1 "Pairing UX flow" rev. 3 answer.
- `BunkerSigner.fromBunker(localSecret, pointer, opts)` is the rehydrate primitive (per `nostr-tools/nip46` v2.23.3). Note that this *takes a parsed `{pubkey, relays, secret}` pointer plus a per-pairing consumer secret*, **not** a URI directly — the original spec assumed a `fromBunker(uri)` shape; that turned out to be wrong on inspection of the vendored bundle. The implementation generates a fresh 32-byte `local_secret_hex` at pair time and persists it alongside the URI so rehydrate is deterministic.
- `parseBunkerInput` is `async` in v2.23.3 — it falls through to a NIP-05 lookup if the bunker:// regex doesn't match. Always `await` it.

---

## Data model

```ts
interface CharterPairingRecord {
  // PRIMARY KEY — kid (subject) pubkey, hex.
  // One record per kid, so a parent who pairs AxeNStax for two kids has two records.
  subject_pubkey: string;        // 64-char lowercase hex

  // The bunker:// URI as accepted from signet-app's pairing flow.
  // Stored verbatim for diagnostic + future re-parse if the schema changes.
  bunker_uri: string;

  // Parsed pointer extracted from bunker_uri at pair time. Stored separately
  // so rehydrate doesn't have to re-parse + the URI string can be treated as
  // a credential to redact in UX.
  bunker_pointer: {
    pubkey: string;              // bunker pubkey (hex, NOT the subject)
    relays: string[];            // at least one
    secret: string | null;       // bunker connection secret, if URI carried one
  };

  // Per-pairing consumer secret, generated at pair time. Used as the first
  // argument to BunkerSigner.fromBunker on rehydrate so the consumer side
  // of the NIP-46 conversation has a stable identity.
  local_secret_hex: string;      // 64-char lowercase hex (32 bytes)

  // Pairing metadata.
  paired_at: number;             // unix ms — for parent UX ("paired on May 12")
  last_used_at: number;          // unix ms — touched on every successful charter_check
  consumer_app_pubkey: string | null;  // hex — schnorr pubkey derived from
                                       // local_secret_hex; diagnostic only.
}
```

Storage:
- IDB database name: `axenstax_charter` (separate from `axenstax_worlds`).
- Object store: `pairings`, keyPath: `subject_pubkey`.
- Optional index on `paired_at` (for "show me all pairings, newest first" parent UX). Not required for Phase 1.

In-memory cache:
- One module-scope `Map<string, BunkerSigner>` keyed by `subject_pubkey`. Populated lazily on first `__axenstax_charter_check` for a given subject. Lives for the page lifetime; cleared on sign-out.

---

## API surface (added to `auth.js` or a new `charter-pairing.js` IIFE)

```js
// Pairing intake — called when the parent pastes / scans a bunker:// URI.
// Returns a Promise<{ ok: true, subject_pubkey } | { ok: false, error }>.
window.axenstax_pair_charter(bunker_uri, subject_pubkey_hex)

// Lookup — does this subject have a pairing? Used by __axenstax_charter_check
// to decide the no_pairing fast-path before opening any connection.
// Returns a Promise<boolean>.
window.axenstax_has_charter_pairing(subject_pubkey_hex)

// Rehydrate — get a BunkerSigner for this subject, opening the connection
// if not in the in-memory cache. Throws if no pairing record exists.
// Returns a Promise<BunkerSigner>.
window.axenstax_get_charter_signer(subject_pubkey_hex)

// Cleanup — called on sign-out and on explicit "Unpair AxeNStax" UX.
// Drops both the IDB record and the in-memory BunkerSigner.
// Returns a Promise<void>.
window.axenstax_unpair_charter(subject_pubkey_hex)
```

`__axenstax_charter_check(subject, clauses)` — defined in the Charter Phase 1 work, not this spec — uses `axenstax_has_charter_pairing` for the `no_pairing` fast-path and `axenstax_get_charter_signer` for the live call.

---

## Phased plan

### Phase 1 — IDB schema + bare API (~3 hours)

- New file: `tools/sites/game/static/charter-pairing.js`. IIFE pattern matching `world_store.js`. Implements `openDb`, `pair`, `has`, `get` (returns the *record*, not a `BunkerSigner` yet), `unpair`.
- Unit-style smoke in a scratch HTML page: pair → has → get → unpair.

### Phase 2 — `BunkerSigner` rehydrate + in-memory cache (~3 hours)

- Add the in-memory `Map<subject_pubkey, BunkerSigner>` cache.
- `axenstax_get_charter_signer` — checks the cache first; if miss, reads the IDB record, calls `BunkerSigner.fromBunker(hexToBytes(record.local_secret_hex), record.bunker_pointer, {})`, populates the cache. `fromBunker` is sync — it sets up the relay subscription but doesn't await a `connect` request; that happens lazily on the first `sendRequest`. Caller decides whether to `await signer.connect()` for fail-fast behaviour.
- Failure modes: surface `BunkerSigner` construction errors (missing relays, etc.) to the caller as structured rejections. Do NOT cache failed signers.

### Phase 3 — Lifecycle integration (~2 hours)

- On sign-out (`auth.js:158-177`), call `axenstax_unpair_charter` for the current subject? **No** — sign-out should preserve the pairing (parent's intent persists across kid's sign-outs). Instead, add an explicit `axenstax_unpair_charter` UX hook for "Remove pairing on this device" if/when we want it.
- Touch `last_used_at` on every successful `axenstax_get_charter_signer` call.

### Phase 4 — Verification gate

- Scratch HTML harness pairs against a mock `bunker://` URI, exercises `has`, `get` (mock-friendly — uses a stub `BunkerSigner`), `unpair`. Lives outside `static/` so it's not shipped.
- `./check.sh` green.
- Manual: pair AxeNStax with a real signet-app dev instance via the actual `bunker://` flow. Verify `axenstax_get_charter_signer` returns a usable signer that survives a page reload.

**Delivered as `tools/smoke/charter-pairing-check.mjs`**: stubs `window.AxeNostrNip46` (Playwright), navigates to the game site for same-origin script load, exercises six phases — basic round-trip, re-pair invalidation, multi-subject independence, IDB persistence across reload, unpair contract, input validation. Doesn't require a real signet-app dev instance.

---

## Multi-subject scenarios

| Scenario | Expected behaviour |
|---|---|
| Parent pairs AxeNStax for Tom only. Tom signs in, then Sally signs in. | Tom's `charter_check` rides the bunker connection. Sally's hits `no_pairing` — fail-open per Charter spec. |
| Parent pairs AxeNStax for Tom on iPad, Sally signs in on iPad. | Same as above — pairings are per-subject, not per-device-account. |
| Parent pairs AxeNStax for Tom on iPad. Tom switches to desktop (different browser). | Desktop has no IDB record → `no_pairing` → kid plays. Per-device pairing model is the Charter spec's deliberate choice. UX onboarding handles this (`2026-05-08-alpha-onboarding-charter-pairing.md`). |
| Parent pairs AxeNStax for Tom, then re-pairs (e.g., bunker key rotated). | The new `bunker://` URI overwrites the existing record (same `subject_pubkey` key). In-memory cache must be invalidated on overwrite. |

---

## Acceptance

- IDB schema lives in `axenstax_charter` DB, separate from `axenstax_worlds`.
- All four `window.axenstax_*` functions implemented + smoke-tested.
- Pairing survives page reload.
- Multi-subject lookups return correct per-kid pairings.
- Sign-out preserves pairings (parent intent persists); explicit `unpair` is the only way to remove.
- `./check.sh` green; smoke harness round-trips end-to-end.

---

## What this spec does *not* cover

- The pairing-intake **UX** — paste box vs camera-scan, where the surface lives in `lobby.html`. That's `2026-05-08-alpha-onboarding-charter-pairing.md`.
- The `__axenstax_charter_check` JS function itself — that's the Charter Phase 1 implementation, on top of this storage layer.
- Cross-device pairing portability — out of Charter Phase 1 scope (each device pairs separately, by spec).
- Encryption-at-rest of stored `bunker://` URIs — for alpha, IDB scoping is the only isolation. Same posture as `world_store.js`. If a future spec hardens `world_store.js` (encryption-at-rest), this store inherits the same treatment.

---

## Memory rules check

- npub only display — internal storage uses hex `subject_pubkey`. Any future UI surface (parent dashboard within AxeNStax, "manage pairings" screen) renders npub. ✓
- signet boundary — entirely AxeNStax-internal. No Signet asks. ✓
- pretest check — the build-pipeline spec must land first; verify `BunkerSigner` is actually loadable before claiming this spec's Phase 2 is testable. ✓
