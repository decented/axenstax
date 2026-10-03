# Charter rev. 7 integration — relay-read evaluator + app keypair + engine bridge

**Status**: **DELIVERED 2026-05-09 (solo parts) on `main`.** Phases 1-7 all in: app keypair generated + published, vendor bundle + browser-load smoke green, `charter-evaluator.js` (subscribe + decrypt + cache + evaluate) implemented, 18-case synthetic-clause smoke green (basic flows, defaults+edges, multi-relay dedup, supersedence, kind-30471 self-reports, plus 5 review-driven gap closures), `auth.js` wires `charter_*` from `/auth/whoami`, engine-side `charter::check()` gates `MenuAction::CreateWorld | LoadWorld | HostWorld | JoinGame` with a child-friendly deny overlay, and kind-30471 self-reports default-on for alpha (gift-wrapped to charter author via NIP-59, multi-relay redundancy). Full end-to-end real-bunker round-trip waits for signet-app's rev. 7/8 punch list (~7-8 days their side: editor UI + clause-publish + URL-auth `charter_*` extensions + dashboard).
**Charter spec at delivery:** rev. 7 (2026-05-09). **Updated to rev. 8** later same day (no-semantic-change ambiguity removal): rev. 8 pins the URL-auth wire shape across three transports (JSON body = native arrays, URL query string = repeated keys, Nostr event tags = repeated single-value) and confirms the cold-start fail-open semantics this implementation matches. AxeNStax's existing defensive parser handles both repeated-keys and list-in-one-tag input shapes — confirmed correct under rev. 8 by Forgesworn, no rework required.
**Date**: 2026-05-09
**Charter spec (live)**: `<workspace>/forgesworn/signet-plans/docs/plans/2026-05-08-charter-schedule-clause-spec.md` (rev. 7).
**Architecture-broadening report**: `<workspace>/forgesworn/signet-plans/docs/reports/2026-05-08-charter-architecture-broadening-holodeck.md`.
**Memory rules in scope**: signet boundary (this is AxeNStax-internal — Signet-side asks live in their repo), npub only display (any displayed pubkey renders as npub), charter rev7 pivot (mechanism A — relay-read, no NIP-46), autonomy to playtest boundary (solo parts land now; full round-trip waits for signet-app rev. 7 ship).

---

## TL;DR

Charter spec rev. 7 (mechanism A — static-data evaluation) replaces the previously-planned bunker-evaluator integration. AxeNStax becomes a plain Nostr relay consumer:

1. Subscribe to ≥2 `charter_relays` for kind-31000 events tagged `["t","charter-clause"]` and `["#p","<axenstax-app-pubkey>"]`.
2. Decrypt incoming events with the AxeNStax app private key (NIP-44 v2).
3. Validate, cache the latest clause per `(subject, consumer)` in IDB.
4. Expose `window.__axenstax_charter_check(subject_pubkey)` → `{allow, reason?}`. Engine WASM calls via `Reflect`.
5. (Optional, default-on) Publish kind-30471 ephemeral self-reports gift-wrapped to the clause author for parent-side per-session visibility.

No NIP-46 client. No `bunker://` paste-box. No `charter_check` method. Pairing is implicit in URL-auth.

Estimate: ~2 days solo-buildable; ~3 with kind-30471.

---

## Context pointers

- Charter spec rev. 7: `<workspace>/forgesworn/signet-plans/docs/plans/2026-05-08-charter-schedule-clause-spec.md` Q1 + Q2 + Q4 + Q5.
- AxeNStax-side audit (rev. 3 framing, rev. 7 delta header): `docs/foundations/2026-05-08-charter-integration-axenstax.md`.
- Spec 7 vendor pattern (esbuild → IIFE → window globals): `docs/foundations/2026-05-08-site-build-pipeline-for-npm.md`. Reused with a different entrypoint shim.
- Dormant Spec 8 IDB layer (referenceable groundwork): `tools/sites/game/static/charter-pairing.js` + `tools/smoke/charter-pairing-check.mjs`. New `charter-cache.js` cribs the open-DB / per-record-key / smoke-harness shape.
- App-keypair coordination doc (publishes the consumer pubkey for signet-app's hardcoded registry): `docs/integrations/signet/2026-05-09-charter-app-keypair.md`.

---

## Constraints

- **Vanilla / IIFE discipline preserved** — same as Spec 7. `auth.js` stays plain browser JS; the new `charter-evaluator.js` is an IIFE matching `world_store.js` / `charter-pairing.js` shape.
- **No mid-session enforcement.** Per rev. 7 Q4: cache update mid-session does NOT terminate the running session. Re-evaluate on next session start only.
- **`charter_relays` MUST contain ≥2 entries.** Subscribe to all in parallel; deduplicate events by ID. Trivial code lift; meaningful resilience.
- **Default-allow on absent state.** `no_clause` and `no_pairing` are `allow`, not `deny`. Charter is opt-in per `(dep, consumer)`.
- **Fail-closed at session start, fail-open mid-session.** A network blip mid-session shouldn't kick a kid out, but couldn't-reach-relay-at-cold-start MUST NOT silently allow (per rev. 7 Q2).
- **Defensive on URL-auth extension absence.** Until signet-app ships its rev. 7 punch list, `charter_*` fields won't be present in the URL-auth response. Consumer treats absence as `no_pairing` (allow). Code paths exercise cleanly with synthetic events for solo verification.

---

## Phased plan

### Phase 1 — App keypair + coordination doc (~half day)

Generate a persistent secp256k1 BIP-340 keypair for the AxeNStax consumer. Bundle-embed the private half; publish the public half to a discoverable doc so signet-app's hardcoded registry can pick it up unambiguously.

- **Generation:** offline one-shot using `noble-curves` / `nostr-tools` `generateSecretKey()` + `getPublicKey()`. Run via a small helper script committed to `tools/scripts/`.
- **Storage:** private key hex → `tools/sites/game/static/charter-app-key.js` as a single IIFE-wrapped const exposed on `window.__axenstax_charter_app_priv` (or similar — kept short, scoped, easy to grep). Treated as a public-shaped secret per rev. 7 Q1, NOT as a credential.
- **Coordination:** `docs/integrations/signet/2026-05-09-charter-app-keypair.md` carries the public hex + a short note ("This is the value to hardcode in signet-app's `charter-consumer-registry.ts` for the AxeNStax consumer").

### Phase 2 — Vendor `nostr-tools/relay` + `nip44.v2` IIFE (~half day)

Spec 7 recipe with new entrypoint shim. Output `tools/sites/game/static/vendor/nostr-tools-relay-nip44.iife.js`. Globals: `window.AxeNostr.{Relay, nip44, finalizeEvent, verifyEvent, getPublicKey, generateSecretKey}` (whatever subset the evaluator + smoke need).

- Browser-load smoke verifies the globals + a parse round-trip on a synthetic event.
- REGENERATE.md picks up the new entrypoint shim.
- Bundle size delta < 100 KB minified per Spec 7 acceptance criterion (re-checked in `check.sh --smoke`).

### Phase 3 — `charter-evaluator.js` (~half day)

New `tools/sites/game/static/charter-evaluator.js`. Module-scope state: subscriptions to ≥2 relays (parallel), in-memory cache of latest clause per `(subject, consumer)`, IDB persistence (`charterClauseCache` DB, separate from `axenstax_charter` Spec 8 dormant DB).

API:

```js
window.axenstax_charter_init(session_state)  // session_state = {charter_authors[], charter_relays[], dep_canonical_pubkey, subject_pubkey}
window.__axenstax_charter_check(subject_pubkey_hex)  // sync; returns {allow: boolean, reason?: string}
window.axenstax_charter_shutdown()  // closes subscriptions; called on sign-out
```

Internals:
- Multi-relay subscribe (all in `charter_relays` in parallel; dedupe by event ID).
- Filter: `kind: [31000]`, `#t: ['charter-clause']`, `#p: [<app-pubkey>]`, optional `#d: [<subject>:<consumer>]` narrowing.
- For each event: verify `event.pubkey` ∈ `charter_authors`; NIP-44 v2 decrypt; validate payload (`schemaVersion`=1, `kind`='schedule', `mechanism`='static-data', windows well-formed, timezone valid); cache latest per `(subject, consumer)`.
- Cold-start: 5-second timeout, then IDB fallback, then `no_pairing` allow.
- Evaluator: per rev. 7 §"Evaluation at the consumer". Window-crosses-midnight branch handled.

### Phase 4 — Synthetic-clause smoke (~half day)

Playwright smoke that publishes synthetic signed + NIP-44-encrypted clauses to a stub relay (or in-page intercept) and exercises the evaluator. Cases:

- in-window allow
- out-of-window deny (`schedule_locked`)
- revoked
- expired (`endDate` in the past)
- always-allow (`windows: []`)
- unparseable payload (treat as `no_clause` allow per rev. 7)
- no event at all → `no_clause` allow
- relay timeout → `no_pairing` allow
- multi-relay dedup (same event from two relays → one cache write)

Hosted as `tools/smoke/charter-evaluator-check.mjs`.

### Phase 5 — `auth.js` wiring (~half day)

Server-side: parse `charter_authors` / `charter_relays` / `dep_canonical_pubkey` from the URL-auth response (when present). Stash on the session cookie or expose via `/auth/whoami`. Defensive — absence of any field treats the session as un-Chartered.

Client-side: on signed-in lobby boot, read the session-state, call `window.axenstax_charter_init(session_state)`. Charter-evaluator subscribes per the relays; engine bridge becomes hot.

### Phase 6 — Engine-side bridge (~half day)

Rust side: `MenuAction::CreateWorld | LoadWorld | JoinGame` consults the JS bridge before transitioning into world state. WASM only (native skip). Use existing `Reflect`-style call pattern from `wasm_auth.rs`. On `deny`, render an in-engine deny screen with reason-keyed copy:

- `schedule_locked` → "Time's up — try again at the next allowed window"
- `clause_revoked` → "Charter has been turned off for AxeNStax. Ask your guardian."
- `clause_expired` → "Charter's end date passed. Ask your guardian to extend."
- `relay_unavailable` → "Can't reach your Charter — try again in a moment."

Deny screen is a single-screen overlay; OK button returns to the lobby. Visual UX validated by Axolittle later (separate test sheet).

### Phase 7 — Optional kind-30471 self-reports (~1 day, defaults ON for alpha)

Per rev. 7 Q3: publish kind-30471 ephemeral session-start/end events gift-wrapped to the clause author. Default ON for alpha.

- Publishing logic: `tools/sites/game/static/charter-self-report.js` (or fold into the evaluator if cleaner).
- Tags: `["t","charter-session"]`, `["d","<subject>:<consumer>"]`, `["clause","schedule"]`, `["consumer","<axenstax-app-pubkey>"]`, `["result","allow"|"deny"]`, optional `["reason",<enum>]`. Content: empty string (metadata-only rumor pattern, like #90 v2).
- Gift-wrapped via NIP-17 (kind-1059) to the first entry of `charter_authors` (parent-as-sole-author Phase 1).
- Smoke confirms the publish goes out after a session-start gating check; visible to the parent's signet-app-side dashboard once they ship it.

### Phase 8 — Verification gate

- `./check.sh` ALL GREEN.
- `./check.sh --smoke` ALL GREEN (existing smokes plus the two new ones: vendor bundle + evaluator).
- Bundle-size gate: brotli total stays under 5 MiB.
- Solo-verifiable end-to-end via synthetic events. Real-bunker round-trip blocked on signet-app rev. 7 ship; on the day they're ready, swap the synthetic publisher for theirs.

---

## Acceptance

- Persistent AxeNStax app keypair generated; private half bundled; public half published to `docs/integrations/signet/2026-05-09-charter-app-keypair.md`.
- Vendor bundle present at `tools/sites/game/static/vendor/nostr-tools-relay-nip44.iife.js`; browser-load smoke green.
- `charter-evaluator.js` implemented per the API above; synthetic-clause smoke covers nine cases.
- `auth.js` reads `charter_*` defensively; absence does not break signed-in flow.
- Engine bridge calls `__axenstax_charter_check` at session start; deny screen renders with reason-keyed copy.
- (Optional) kind-30471 self-reports publish on session start.
- All four existing smokes plus two new ones green; `check.sh` ALL GREEN.
- Spec 8 (`charter-pairing.js`) remains dormant on main; this spec does NOT touch it.

---

## What this spec does *not* cover

- Signet-app-side rev. 7 punch list (editor UI + clause-publish + URL-auth `charter_*` extensions + dashboard). That's Forgesworn's ~7-8 days.
- Future mechanism B/C/D consumer work (cumulative-state budgets, live-request, triggered). When those land, the dormant Spec 8 layer becomes live again.
- Rev. 7-shaped onboarding doc (replacement for the superseded Spec 9). One short FAQ-style page, drafted as a side-task during this spec's Phase 5/6.
- App-keypair rotation procedure beyond what rev. 7 already documents. If a leak is suspected, regenerate, push new bundle, update the coordination doc, ping signet-app to update the registry, ask alpha-tester parents to republish their clauses.

---

## Memory rules check

- signet boundary — entirely AxeNStax-internal consumer work. ✓
- npub only display — any displayed pubkey (deny screen, future "manage Charter" UX) renders as npub. Hex stays internal. ✓
- autonomy to playtest boundary — Phases 1–7 solo-verifiable via synthetic events; real-bunker round-trip waits for signet-app + an Axolittle session. ✓
- charter rev7 pivot — Spec 8 IDB layer stays dormant; this spec doesn't reach into it. ✓
