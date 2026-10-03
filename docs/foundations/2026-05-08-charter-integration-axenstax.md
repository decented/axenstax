# Charter integration in AxeNStax — feasibility audit

**Status**: DELIVERED 2026-05-08 as the AxeNStax-side feasibility input under Charter spec **rev. 3**. Charter spec subsequently pivoted: **rev. 6 (2026-05-08)** moved the Schedule clause from bunker-evaluator (mechanism B-shaped) to relay-publication (mechanism A — static-data); **rev. 7 (2026-05-09)** closed the Consumer-app-keypair / `charter_relays` plurality / mid-session-revocation / portability gaps surfaced by AxeNStax's round-4 review. The audit content below remains historically accurate for rev. 3 but parts are no longer the live contract — see "What rev. 7 changed for AxeNStax" below for the deltas. Implementation prereqs that fell out of rev. 3 (Specs 7, 8, 9) are partially superseded: Spec 7 vendor pattern still applies (with a different entrypoint shim), Spec 8 (bunker pairing) is preserved as future mechanism-B/C/D groundwork, Spec 9 (consumer-side pairing UX) is fully superseded. A new Spec 10 (rev. 7 integration) is queued in the foundations README.
**Date**: 2026-05-08 (status updated 2026-05-09).
**Source brief**: Forgesworn-internal Charter brief (`<workspace>/forgesworn/charter/README.md` + `<workspace>/forgesworn/signet-plans/docs/plans/2026-05-08-charter-positioning.md`).
**Charter spec (live)**: `<workspace>/forgesworn/signet-plans/docs/plans/2026-05-08-charter-schedule-clause-spec.md` (rev. 7).
**Memory rules in scope**: signet boundary (Charter sits on Signet — engine-side adoption only), shared infra strategy (Charter primitives must lift to other games), autonomy to playtest boundary (this is research, not implementation).

## What rev. 7 changed for AxeNStax (added 2026-05-09)

Headline: Phase 1 architecture moved from bunker-evaluator to relay-read for static-data clauses (Schedule). The integration simplifies materially. Specifically:

- **No NIP-46 client.** No `BunkerSigner`, `sendRequest`, or `nostr-tools/nip46`. Just a plain Nostr relay client (`nostr-tools/relay` + `nip44.v2.decrypt`).
- **No `bunker://` pairing flow.** Pairing is implicit in the existing URL-auth handshake — when the kid signs in via Signet, the URL-auth response carries a `charter_*` extension block (`charter_authors`, `charter_relays` ≥2, `dep_canonical_pubkey`).
- **No `charter_check` method call.** Consumer subscribes to relays (kind 31000 + `["t","charter-clause"]` + `["#p", "<axenstax-app-pubkey>"]`), decrypts with the AxeNStax app private key, evaluates the cached clause client-side at session start.
- **AxeNStax needs a persistent app keypair.** Bundle-embedded, public-shaped secret. P1 discovery is a hardcoded registry in signet-app (AxeNStax-only at alpha).
- **Audit is reframed.** Per-check audit is opt-in via `kind-30471` self-reports (gift-wrapped to the clause author). AxeNStax defaults this **ON** for alpha.

§§A–F below were written under rev. 3 framing (bunker-evaluator). The architectural concerns surfaced there mostly resolved positively under rev. 7 — bunker reachability is no longer in the per-check hot path; pairing UX disappears for mechanism A; the cross-platform NIP-46 crate (Phase 2) is dropped from the rev. 7 spec entirely. Remaining concerns specific to mechanism A (relay reachability at cold-start, app-keypair rotation) are addressed in rev. 7 Q1 + Q5.

---

## TL;DR

Charter is a parent-led permission product that pairs into Signet's bunker and gates a child's gameplay via revocable clauses (schedule, budget, spend, content, comms). For a marquee P1 integration, AxeNStax needs:

- **(a)** A Signet pubkey wired through to the engine session (already speced as Spec 1 Phase 4 — `JoinRequestPacket` carrying a verified `auth_event`). Currently blocked on a missing engine-side signing bridge — see `2026-04-20-engine-signet-auth.md` §"Phase 4 prerequisite".
- **(b)** A session-start hook calling `charter_check(schedule)` against the Signet bunker.
- **(c)** A child-friendly deny-screen UI when the bunker refuses.
- **(d)** An audit-log emit on deny so the parent dashboard sees the attempt.

Everything else from the Charter clause set (budget polling, spend approvals, content gating, comms allowlist) is **post-P1** — AxeNStax should not build for it speculatively. The single schedule-clause loop is the smallest thing that proves the Charter contract end-to-end.

The biggest open question for Charter's spec phase: **how does the engine WASM client talk to the Signet bunker?** The same gap that blocks Spec 1 Phase 4 (no `__axenstax_sign_*` JS bridge) blocks Charter — both want the engine to make NIP-46 calls. Resolving once unlocks both.

---

## Six surfaces — where Charter calls would land

The Charter brief asked for an audit of six AxeNStax surfaces. Mapped to current code:

### 1. Player identity / account model

**Today**: client-asserted `JoinRequestPacket.player_name: String` for remote players (Spec 1 BRIDGE, gated `USE_SIGNET_AUTH=false`). Local single-player has no identity at all on the engine side — `save::WASM_PUBKEY` (web) is used only for save-file namespacing in `save.rs:28` and never reaches `GameServer`.

**Charter shape**: pubkey is the account, full stop. No "AxeNStax accounts" abstraction (per Charter brief's "Don'ts").

**Action**: Spec 1 Phase 4 + missing signing bridge. Once that lands, `ServerPlayer` gains `pubkey: [u8; 32]` and `handle: String` and Charter has its identity primitive for free.

### 2. Session lifecycle (where to fire `charter_check`)

**Today**:

- **Multiplayer host path** — `HostedServer::start` (`hosted_server.rs:144`) → `GameServer::initial_load` (`server.rs:146`) → first `tick()` (`hosted_server.rs:257`).
- **Multiplayer join path** — `RemoteClient::connect` (`remote_client.rs:43`) → server-side `JoinRequest` handler (`hosted_server.rs:347`).
- **Single-player path** — bypasses `GameServer` entirely (CLAUDE.md known debt bullet 1). `chunk_stream::initial_load` runs in `GameState::tick` on the client. **A Charter check fired only on `HostedServer::start` would be bypassed by single-player.**

**Charter shape**: one check per session start, per pubkey. For split-screen, one check per local pubkey (parallel calls, not one device-wide call).

**Action**: Defer the location decision until Spec 2 Phase 0b (`ENABLE_SINGLEPLAYER_HOSTED_SERVER=true`) lands and single-player routes through `HostedServer`. Until then, any Charter check site is incomplete by construction. Resolution: gate Charter's P1 integration on Spec 2 Phase 0b OR build the check at a layer above both paths (e.g., `game_loop.rs` `MenuAction::CreateWorld` / `LoadWorld` / `JoinGame` handlers).

### 3. Time tracking (budget warnings)

**Today**: `GameState.world_time` (advanced 4×/tick — folded into `/time speed` per the engine commands feature) is **in-game** time, not wall-clock session duration. No session-duration counter exists.

**Charter shape**: Charter clauses include daily/weekly play-time budgets. Two enforcement modes:

- **Hard** (bunker-authoritative) — bunker refuses to sign at lockout time; the engine sees a deny on the next periodic check.
- **Soft** (engine-cooperative) — engine polls `charter_remaining_minutes` and shows a 5-min/1-min warning UI before lockout.

**Action**: Both modes require a wall-clock session-start timestamp on `GameServer` and a periodic re-poll (not on the 20 TPS hot path — every ~30 s is plenty). Defer until Charter's contract names the method. **Out of scope for P1** per the brief — schedule clause is the smallest loop.

### 4. In-game purchases (`charter_spend_request` hook)

**Today**: no IAP. Bitcoin flow is hash-on-mine *earning*, not spending — the spec ([06-bitcoin-integration.md](../spec/06-bitcoin-integration.md)) is about player payouts, not player spend.

**Charter shape**: when Lightning spend lands (creator-server tipping, block-pack cosmetics, etc.), every spend is a `charter_spend_request(amount, reason)` to the bunker. Parent approval gates the sign. AxeNStax never touches funds (existing platform constraint — see CLAUDE.md "Not a money transmitter").

**Action**: **No P1 work.** Flag as a design constraint when IAP design begins: "every spend hooks the Charter spend method" — same place where the Lightning UX lands.

### 5. Multiplayer / chat (comms allowlist)

**Today**: chat overlay shipped 2026-05-08 (engine commands feature, `chat_ui.rs`). Single-player only — no chat routing, no friend list, no addressing model. Multiplayer-aware chat is post-Spec-2.

**Charter shape**: `charter_comms_check(target_pubkey)` before adding a friend or sending a DM. Group chat in shared worlds is a separate gate (allow-by-server-policy, not per-friend).

**Action**: **No P1 work.** Comms are post-P1 per the brief. When multiplayer chat ships, design with a `charter_comms_check` hook in the friend-add and DM-send paths.

### 6. Save / state (Dominion composability)

**Today**: per-pubkey save namespacing (`save.rs` uses `WASM_PUBKEY` to scope local IDB world list). Multi-player saves restore Player 2 correctly (resolved 2026-04-30).

**Charter shape**: orthogonal to Charter — saves are about access, Charter is about permission. **Composes** with Dominion (epoch-encrypted save shares for cross-device portability) but no Charter call sites.

**Action**: **No Charter work.** Note Dominion as a separate cross-game primitive when the design phase opens.

---

## Smallest loop that proves the contract (Charter P1)

**One** `charter_check(schedule)` call at session start. Concretely:

1. **Hook site**: `game_loop.rs` — in the `MenuAction::CreateWorld` / `LoadWorld` / `JoinGame` handlers, **before** `HostedServer::start` / `RemoteClient::connect`. This site sits above the single-player vs. hosted-server split, so it covers all paths today and survives the Spec 2 Phase 0b unification.
2. **Call shape** (Charter contract TBD): `bunker.charter_check(pubkey, "axenstax", "schedule")` → returns `{ allowed: bool, reason?: string, deny_screen?: string }`.
3. **Deny path**: render a child-friendly screen ("Your Charter says playtime is over. Come back at 4 PM.") instead of transitioning to `GameMode::Playing`. UI lives in `menu.rs` — same paint as the existing world-loading-failed states.
4. **Audit emit**: on deny, emit a kind-XXXXX (Charter spec names it) event via the same NIP-46 pairing. The parent dashboard reads from the audit relay.

That's the loop. Anything more (re-checks during play, time warnings, spend, comms) is out of scope for P1.

---

## Architectural concerns to feed back to Charter spec

These are open questions / constraints AxeNStax surfaces — Charter's spec phase needs to answer them.

### A. WASM target — engine cannot call Signet bunker today

**Two distinct gaps live here, often conflated.** Both involve "engine wants the Signet bunker to do something via NIP-46," but they need different artefacts:

| Gap | Artefact | Status |
|---|---|---|
| **A1. Sign arbitrary kind events on demand** (kind 21236 for Spec 1 Phase 4 multiplayer auth, 31001 for voice-feedback Phase 1e Nostr-signed manifests, 31002 cloud-save manifests, etc.) | NIP-46 `sign_event` primitive in mysignet.app | **Documented + deferred.** See `docs/integrations/signet/2026-05-05-nip46-signing-bunker-upstream.md` (DRAFT). Sentinel **D-003** decided 2026-05-07: "wait for Phase 5 (Bitcoin) lead-in." Higher-priority cross-game-shared infra (sign-in, age gating, Blossom cloud save, QR multi-screen sign-in, social, voice) takes precedence per state.yaml. |
| **A2. Charter `charter_check` method** | NIP-46 vendor-method handler (specific to Charter, returns structured result rather than a signed event for the consumer) | **Charter Phase 1 work**, ~1 week signet-app-side per Charter spec rev. 3 §Q5. Sidesteps A1 by being its own narrow method, callable via `nostr-tools/nip46`'s generic `sendRequest` — no need to wait for the broad signing primitive. |

The engine WASM has `wasm_auth.rs` that *receives* a pubkey but no JS bridge for either kind of NIP-46 call. Charter Phase 1 closes A2 with a small `__axenstax_charter_check` JS hook in `auth.js` embedding `nostr-tools/nip46` directly. A1 stays open behind D-003.

**Resolution for Charter** (rev. 3 settled):
- **Phase 1** (alpha-blocking): WASM-only JS bridge using `nostr-tools/nip46` for `charter_check`. ~2 days AxeNStax-side, ~1 week signet-app-side. Doesn't touch A1.
- **Phase 2** (post-alpha): cross-platform `signet-nip46-client` Rust crate, Forgesworn-leading. *This* artefact also closes A1 — its generic `sign_event` method unblocks Spec 1 Phase 4 + voice-feedback Phase 1e Nostr-signed manifests + future kind-31002/31003 use cases. The crate's API surface (`KeyCustody` trait, `sign_event`, `nip44_encrypt`, plus `charter_check` helper) is exactly what AS-005's upstream doc was asking for, just packaged on the consumer side.

**So Phase 2 = AS-005 capability + Charter helper**, delivered together. When Charter Phase 2 lands, D-003 effectively resolves through it, and AxeNStax adopts at ~6–8 weeks post-alpha. AxeNStax acts as API-design sounding board during Forgesworn's scoping.

### B. Native target — same gap, different transport

Native engine builds have no Signet integration at all. `RemoteClient::connect(addr, "Player")` (`game_loop.rs:520`) hardcodes a string. Native Charter consumers need a paired Signet-app on the same host (or a relay-only pairing). Out of scope until cross-platform NIP-46 client lands.

### C. Single-player bypasses GameServer (CLAUDE.md debt #1)

If Charter check goes in `HostedServer::start`, single-player sessions are a Charter bypass. Two ways out: gate Charter's P1 on Spec 2 Phase 0b (`ENABLE_SINGLEPLAYER_HOSTED_SERVER=true`), or fire the check above the dual-sim split in `game_loop.rs`. **Recommend the latter** — decouples Charter from a separately-blocked spec.

### D. Split-screen — one device, multiple pubkeys

Two children sharing a couch + two gamepads = two Charter checks, one per pubkey, at session start. Charter contract should make per-pubkey calls explicit and cheap. AxeNStax already supports split-screen up to 4 local players (`PlayerSlot` / `chunk_stream::initial_load` resolved 2026-04-30); each slot needs its own Charter check.

### E. 20 TPS tick budget

Synchronous Charter checks at session start are fine (multi-second relay round-trip is acceptable when the player hasn't entered the world yet). Mid-session re-checks **must not** block the tick. Charter spec should commit to either (a) push-only deny notifications from the bunker, or (b) a polling cadence ≥ 1 s the engine can run off the main tick.

### F. Audit log — Nostr event, not a custom format

AxeNStax already has a `kind-31000` precedent (handle credential). The Charter audit event should be a kind-XXXXX Nostr event, signed by the bunker, published to `wss://relay.trotters.cc` (own infra — see memory trotters relay). Don't invent a parallel HTTP audit endpoint.

---

## Briefing back to Charter

> To be Charter's marquee P1 integration, AxeNStax needs:
> - **(a)** Signet pubkey wired through to engine session (Spec 1 Phase 4 + missing signing bridge — both AxeNStax-internal, but the bridge should be built as cross-platform Rust NIP-46 client per signet boundary).
> - **(b)** Session-start hook in `game_loop.rs` (above the dual-sim split — covers single-player + multiplayer + split-screen with one site).
> - **(c)** Deny-screen UI in `menu.rs`.
> - **(d)** Audit emit via the cross-platform NIP-46 client.
>
> Everything else (budget polling, spend, comms) is post-P1.
>
> Open questions Charter's spec must answer: cross-platform NIP-46 transport (engine WASM ↔ Signet bunker), audit event kind, mid-session re-check cadence (push vs. poll), per-pubkey check semantics for split-screen.

---

## What's NOT in this doc

- **No Charter-side asks beyond contract clarity.** Charter's spec is its own work; this doc is consumer-side feasibility input.
- **No NIP-46 method draft.** Charter brief explicitly defers NIP discipline ("Use vendor-prefixed `charter_*` names locally"); this doc respects that.
- **No "AxeNStax accounts" abstraction.** Per Charter brief Don'ts.
- **No Charter UI in AxeNStax.** Charter's parent dashboard is a separate native app; the game just consumes the contract.
- **No build steps.** Promote this to a build spec only after Charter's contract lands.

---

## Memory rules check

- signet boundary — Charter sits on Signet, but the *NIP-46 client crate* and *signing bridge* are general-purpose Signet ecosystem upgrades, not AxeNStax-specific work. The engine-side adoption (Charter check in `game_loop.rs`, deny screen in `menu.rs`) is consumer-side and AxeNStax-specific by design.
- shared infra strategy — the cross-platform NIP-46 client is exactly the kind of primitive that lifts to other games on the same primitives Build it shared.
- autonomy to playtest boundary — this doc is research, not implementation. No code changed.
- pretest check — surfaced the Spec 1 Phase 4 signing-bridge gap by reading real code (`wasm_auth.rs`, `auth.js`, `game_loop.rs:520`) instead of trusting the test sheet's "mechanical pre-flight" framing.

---

## Status — what to do with this doc

- **If Charter spec writers**: input is delivered. Charter spec rev. 3 is settled — see `signet-plans/docs/plans/2026-05-08-charter-schedule-clause-spec.md`.
- **If AxeNStax-side**: this audit is now a historical reference. The buildable next steps are Specs 7 → 8 → 9 in this folder, READY TO BUILD when alpha-launch crunch eases.
- **If a future Claude session**: read this doc to understand *why* Specs 7–9 exist (they're Charter Phase 1 prerequisites, not standalone work). The signing-bridge gap that originally blocked both Spec 1 Phase 4 and Charter is now Forgesworn's `signet-nip46-client` Phase 2 work; AxeNStax adopts when it lands. Don't start Charter Phase 1 implementation until Specs 7 → 8 → 9 are scheduled in the right order.
