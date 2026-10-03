# Lobby Mailbox — NIP-17 player↔dev feedback (store-and-forward)

**Status**: DRAFT — pending owner review, then implementation plan.
**2026-10-01 — WEB BUILD REMOVED (owner decision):** the browser build no longer has
any feedback channel — `mailbox.js`, the `cloud.js` mailbox bridge, `wasm_feedback.rs`,
the lobby "Tell the makers" panel, web `/bug` `/idea` `/mailbox` and Test Lab verdicts
are gone. This spec now describes the **native** mailbox (`game/engine/src/native_mailbox/`)
and the dev reader only; every "web"/"lobby"/"stash sync" statement below is historical.
The shared gift-wrap helper moved to `tools/feedback-reader/lib/nip59.cjs`.
**2026-10-02 — INBOX RELAYS:** reports no longer go to our own relay. The native
mailbox publishes to, and reads the status board from, the fixed public
`FEEDBACK_INBOX_RELAYS` (`wss://nos.lol`, `wss://relay.primal.net`,
`wss://offchain.pub` — `game/engine/src/native_mailbox/mod.rs`), chosen because
each serves kind 1059 without NIP-42 auth (`relay.damus.io` demands it). It is the
project's inbox, independent of the player's own relay list, so custom relays
never cut a player off. `tools/feedback-reader/` reads the same set by default
(`RELAY`/`RELAYS` override). Every "our relay" below is historical.
**Date**: 2026-06-07 (drafted)
**Branch**: `docs/lobby-mailbox-feedback` (spec) → implementation branches off `main` per phase.
**Reply sections superseded 2026-10-01** by `docs/foundations/2026-10-01-feedback-status-board.md`:
no player is ever replied to or messaged (web has no feedback channel; native shows status from a
public anonymous board). Every "reply" / inbox / `reply.mjs` section below is historical.
**Supersedes**: the *transport* of `docs/foundations/2026-05-28-alpha-feedback-loop.md`
(HTTP POST → voice-server). That doc's `/bug` & `/idea` command UX is **kept**; its
voice-server intake, LLM triage, dedup and web admin board are **retired** along with
the voice-server itself (owner decision, 2026-06-07).

---

## TL;DR

Feedback becomes a **serverless, store-and-forward mailbox** that lives in the lobby.

- A kid reports a bug or idea in-game (`/bug …` / `/idea …`) or from the lobby. The
  message is **queued locally** — nothing is sent or signed yet.
- When the kid **syncs their stash** (the existing, deliberate cloud-save moment where
  the phone bunker is intentionally turned on), the queued reports are **batch-signed**
  as **NIP-17 private DMs** to the AxeNStax npub and published to our relay. The same
  flush **pulls and decrypts any replies** ("✅ fixed!") and drops them in the lobby inbox.
- The dev side reads/replies two ways ("both"): in **any modern Nostr client**, and in a
  **thin, local, trusted dev reader** that subscribes, decrypts with the AxeNStax key,
  threads by status, and sends replies.

This is built as a **general `mailbox.js` primitive** (outbox + inbox over NIP-17).
Feedback is the first — and for now only — message *type*; player↔player messaging can
reuse the same primitive later with no rebuild.

The whole point: **no per-message signing.** One bunker approval at sync covers the
world blob *and* all queued reports. This is robust on the flaky phones we're testing on,
because there is no live per-message NIP-46 round-trip — the bunker is only needed during
the deliberate sync window.

---

## Why this shape

- **The signing problem drives it.** We have no engine→signer bridge for per-message
  signing (the documented Spec 1 Phase 4 / `engine-signing-bridge` gap), no always-on
  remote signer, and test phones that suspend/throttle JS. A per-message NIP-46 handshake
  would be fragile exactly there. Batching all signing to the deliberate stash-sync window
  side-steps all three. (See charter phase4 shared gap.)
- **It rides infrastructure that already ships.** `tools/sites/game/static/cloud.js`
  already retains a capability-gated Signet signer (`window.__axenstax_get_signer`, gated
  on `hasNip44` + `nip44.encrypt/decrypt` + `signEvent`), already holds a relay client
  (`window.AxeRelay`) and the Stash SDK (`window.AxeStash`), and "sync your stash" is
  already the bunker-approve moment. The mailbox is a new *payload* on an existing signing
  event, not new plumbing.
- **Serverless, like cloud save.** `cloud.js` is explicitly "serverless (destination
  tier)". Retiring the voice-server keeps the whole feedback path serverless and removes a
  moving part (and the three site-split regressions the 2026-05-28 review found in it).
- **Privacy for kids.** NIP-17 gift-wrap hides *who is talking to whom* from the public
  relay graph (metadata privacy), in line with identity default nonpublic.
  NIP-04 (kind-4) would leak the sender/recipient/timing graph publicly — rejected.
- **Cross-game lift.** The mailbox primitive is game-agnostic — an AxeNStax-side consumer
  of generic Nostr DM + the retained signer's generic nip44/signEvent. No Signet-internal
  work (signet boundary); the "pre-approved signing" idea below is a
  bunker-side capability we *consume if present*, never something we ask Signet to build
  for us specifically.

---

## Context pointers (real code)

- **Retained signer + capability gate**: `tools/sites/game/static/cloud.js` —
  `rawSigner()` / `capable()` (`hasNip44` + `nip44` + `signEvent`) / `stashSigner()`.
  The mailbox reuses this exact gate.
- **Relay client**: `window.AxeRelay` (vendored `relay.iife.js`).
- **Stash SDK**: `window.AxeStash` (vendored `stash.iife.js`) + `nostrManifestStore({signer, relay})`.
- **NIP-46 bunker**: vendored `nostr-tools-nip46.iife.js`. nip44 lives on the retained signer.
- **Sync trigger**: the cloud-save flow in `cloud.js` (`AxeCloud.save()` and the
  cosmetics path) — the mailbox flush hooks in next to the world-blob save.
- **Engine commands**: `game/engine/src/commands/{mod,parser,registry,dispatch}.rs` +
  `builtins/`. Commands are trait objects: `impl Command for X` + `reg.register(Box::new(X))`
  (NB: the 2026-05-28 doc's `registry.add(Command{…})` sketch does **not** match the real
  API — adapt to the trait-object pattern).
- **WASM bridge**: `game/engine/src/wasm_feedback.rs` (today exposes
  `__axenstax_feedback_context()` + `register_boot_hook`). We add a Rust→JS enqueue import.
- **AxeNStax identity**: official npub + secret at `~/.config/axenstax/axenstax-official.json`
  (mode 600, outside the repo — axenstax identity keys). The dev reader uses it.
- **Relays** (2026-10-02): the public inbox set above. `wss://relay.trotters.cc`
  (our own infra) is no longer a default; reports sent before then sit there.

---

## Components (each isolated, one responsibility, independently testable)

| # | Unit | New? | Does | Depends on |
|---|------|:--:|------|------------|
| 1 | `nip59` wrap helper (`tools/feedback-reader/lib/nip59.cjs` — moved off the web 2026-10-01) | ✓ | `wrap(rumor, toPubkey, signer)` → kind-1059 gift-wrap; `unwrap(giftWrap, signer)` → rumor | signer nip44 + signEvent; ephemeral key for the outer wrap |
| 2 | `mailbox.js` (`static/`) | ✓ | The primitive: `enqueue(type, body, {to})`, `flush({signer, relay})`, `list()`, `markRead(id)`, `unsentCount()`. Outbox/inbox in IndexedDB, type-tagged | nip59 helper, `AxeRelay`, retained signer |
| 3 | `/bug` & `/idea` commands (`commands/feedback.rs`) | ✓ | Enqueue to the JS outbox. Inline `<msg>` queues; no-arg opens the lobby mailbox; >2000 chars rejected; native = stub | wasm bridge |
| 4 | wasm bridge (`wasm_feedback.rs`) | edit | `enqueue_feedback(kind, text)` Rust→JS into `mailbox.js` | — |
| 5 | Lobby mailbox panel | edit | Outbox (queued/sent), inbox (replies, resolved ✓), "N unsent — sync to send" nudge | `mailbox.js` |

> **Relocation (2026-06-15).** The full "Tell the makers" panel moved **off the web
> Entrance page** (`lobby.html`/`lobby.js`) and **into the in-game Lobby** (the engine
> world-list menu — `menu.rs` `draw_feedback_panel`, replacing the old Friends
> placeholder), so the Entrance is sign-in only (owner request, owner-inbox "Move 'Tell
> the makers' messaging: Entrance → Lobby"). The engine panel has the **composer**
> (bug/idea + message → `wasm_feedback::enqueue_feedback`) **and the full history**:
> "Your reports" (queued/sent badges) + "Replies" (with mark-on-view unread dots).
> The history reads a new cached JSON snapshot `cloud.js` exposes as
> `axenstax_mailbox_snapshot` (refreshed on enqueue/flush/load alongside the unsent
> count); `axenstax_mailbox_mark_read` flags a reply read. The queue (`mailbox.js`) +
> sync-flush remain the source of truth. Wide-screen lobby only (mirrors where the
> Friends panel lived); narrow/mobile still uses `/bug`·`/idea`.
| 6 | Sync integration (`cloud.js`) | edit | At stash-sync, after the world blob, call `mailbox.flush({signer, relay})` on the same signer + relay | `mailbox.js`, existing sync |
| 7 | Thin dev reader + ledger (new local tool, e.g. `tools/feedback-reader/`) | ✓ | Subscribe to relay, decrypt reports with the **AxeNStax key**, write each to a **parseable local ledger** (id, persona npub, type, text, ts, status), and send NIP-17 replies it's handed. No triage logic of its own — triage is the Claude-assisted step below | AxeNStax secret key, relay, nip59 helper |

**Trust boundary:** component 7 holds the AxeNStax secret and therefore is **not** a
public web page — it runs on a trusted machine (Staxolottle's). Confirmed (a).

## Data flow

**Report out**
1. `/bug doors too tall` → `wasm_feedback::enqueue_feedback("bug", text)` → JS
   `mailbox.enqueue("bug", text, {to: AXENSTAX_NPUB})` with a stable client-generated id → IndexedDB outbox.
2. At next **stash-sync** → `mailbox.flush`: for each queued item build a NIP-17 message —
   inner rumour (kind 14) → **seal (kind 13) signed by the bunker** + nip44 → **gift-wrap
   (kind 1059) signed by a throwaway ephemeral key** + nip44 to the recipient → publish to
   the relay → mark sent (by event id). The bunker signs the whole batch of seals in one
   approval window (alongside the world-blob save).

**Reply in** *(superseded 2026-10-01 by `2026-10-01-feedback-status-board.md` — replies removed; status is a public anonymous board)*
3. Same flush queries the relay for kind-1059 wraps addressed to the **kid's persona
   pubkey** since the last-seen watermark → `unwrap` + decrypt (bunker nip44) → store in
   the inbox → lobby shows "✅ 'doors too tall' — fixed!".

**Dev side**
4. The reader subscribes for wraps to `AXENSTAX_NPUB` → decrypt with the AxeNStax key →
   append each report to the **parseable local ledger** (`status: new`). Triage is a
   separate, on-demand Claude-assisted step (next section), not done by the reader. Replies
   the triage step produces are handed back to the reader → wrapped to the kid's persona
   pubkey → published. (Reports are also readable/repliable in any modern Nostr DM client =
   the "native" half of *both*.)

## Dev-side triage workflow

Owner-side process; lives in the internal repo, not in this tree.

## Signing & the sync window (the core requirement)

- **No per-message signing.** One bunker session at sync signs the batch of seals.
- **"Pre-approved"** = a NIP-46 permission grant for the report's event kind so the parent
  isn't prompted per message. This is **bunker-dependent** — we consume it if the signer
  offers it; we don't gate v1 on it (default: one approval prompt per sync covers the batch).
- **Capable-signer gate, reused.** Same `capable()` check cloud save uses. A QR-only /
  ephemeral session (no nip44) → the outbox simply **queues and waits**; the lobby shows
  "sign in with your phone bunker to send" — identical to today's cloud-save-unavailable
  degrade. This is the consumer side of the owner's "start-of-game signing" change
  (establish a capable bunker connection at the lobby); that change itself is Signet-side.

## Error handling

- No capable signer / bunker declines / timeout → nothing sent; items stay queued; retry
  next sync. **Idempotent** — sent items are marked by published event id, never re-sent.
- Relay unreachable → unsent items stay queued; partial batches are fine (per-item marking).
- Bad inbound event (decrypt/parse failure) → skip + log; never crash the lobby.
- **Outbox durability (caveat).** IndexedDB can be evicted on iOS PWAs that aren't added
  to the home screen. v1 uses IndexedDB (confirmed (b)) and prompts add-to-home-screen.
  *Later option:* mirror the unsent queue into the stash blob so it survives a device wipe.

## Identity caveat *(superseded 2026-10-01 — there are no replies, so no reply-addressing identity)*

Replies are addressed to the **kid's persona npub**. If that persona is a rotating
guest-burner (the gamestr pattern in identity default nonpublic), replies to
a retired key are undeliverable. v1 assumes a stable persona; burner-rotation reply-loss is
a known, documented limit, not a v1 blocker.

## Testing

- **`nip59` helper**: `wrap` → `unwrap` round-trip with test keys returns the original rumour.
- **`mailbox.js`**: enqueue / dedup / flush-marks-sent / idempotent-resend / inbox-dedup /
  read-state (node test runner — add a small harness; the repo previously ran jest for the
  voice-server).
- **Engine commands**: Rust unit tests in `commands/feedback.rs#[cfg(test)]` — enqueue
  invoked with the right type + text, length cap rejects >2000, native target hits the stub.
  Mirrors the existing command tests. Runs under `check.sh`.
- **Dev reader**: wrap→publish→subscribe→unwrap round-trip against a local/mock relay with
  test keys; plus ledger append + status-transition (`new → triaged → replied`) and
  "npub never written into the issue body" unit tests.
- **Real-bunker end-to-end** (kid sends → dev reads → replies → kid receives) = **playtest
  boundary**; not solo-verifiable.

## Phasing (each phase solo-verifiable; outbound-first, confirmed (c))

1. **Outbound** — `nip59` helper + `mailbox.js` outbox + `/bug`/`/idea` + sync integration.
   Verify: send a report, decrypt it with the AxeNStax key (script or Nostr client).
2. **Dev reader** — subscribe + decrypt + thread + reply.
3. **Inbound** — lobby inbox pulls + decrypts replies + shows them threaded.
4. **Lobby UI polish** — outbox/inbox panel, unsent nudge, read-state.

Each phase lands on its own branch off `main` after `check.sh` is green. Phase 1 is the
minimum that proves the signing-at-sync loop.

## Out of scope for v1 (deferred — not bugs)

- **Screenshot-on-`P`** (carried over from the 2026-05-28 spec; add as a later phase).
- **Player↔player messaging** (the primitive supports it; no second channel wired in v1).
- **Automated LLM triage / dedup / priority board** (died with the voice-server). Triage is
  now **Claude-assisted on demand, internal-only** (see the triage-workflow section) — a
  human-in-the-loop session updating the local ledger, never an external system. Auto-dedup of
  near-duplicate reports is deferred; Claude merges by eye at triage time.
- **Voice/audio capture** (text-first; Whisper went with the voice-server).
- **Stash-mirrored outbox** (IndexedDB only in v1; mirror later if eviction bites).
- **NIP-46 permission-grant ("pre-approved") UX** beyond consuming it if the bunker offers it.

## Success criteria

1. A kid can `/bug …` / `/idea …` (or compose in the lobby); it appears in the lobby outbox
   immediately, unsent.
2. On stash-sync with a capable bunker, queued reports are published as NIP-17 DMs and
   marked sent; re-syncing does not double-send.
3. The dev reader (and a generic Nostr client) decrypt and display the reports.
4. A dev reply is received, decrypted, and shown threaded in the lobby inbox on the next sync.
5. With no capable signer, reports queue and the lobby explains why; nothing is lost.
6. A Claude session can read the ledger (npub + handle + content), mark `new` reports
   `triaged` with an internal `verdict` (bug/idea/discard) — never filing to GitHub — and
   queue resolution replies that reach the kid on their next sync.
7. `check.sh` green after the engine changes.

## Docs to update on implementation

- `docs/foundations/2026-05-28-alpha-feedback-loop.md` — header note: transport superseded
  by this doc; voice-server retired; `/bug`/`/idea` UX retained.
- `docs/foundations/README.md` — queue entry for this spec.
- `docs/spec/05-gameplay-systems.md` — `/bug` & `/idea` under the commands section.
- `CLAUDE.md` — remove the voice-server spin-up section; note the serverless mailbox.
- `MEMORY.md` (user-side) — a feedback mailbox memory pointing here.
- **No GitHub** (owner directive 2026-06-11): feedback is logged internally only. Add the
  reader's ledger path to the relevant `.gitignore` (local-only, never committed).

## Tester gate (2026-10-03)

**Decision (owner):** `/bug`, `/idea` and `/mailbox` are an **alpha-tester feature,
OFF by default**. The code stays compiled and tested; only the entry points are gated.
Native only (the web build still carries none of it).

- **Flag.** `GraphicsSettings.tester_feedback: bool` (`#[serde(default)]` = false, so
  every existing settings file loads with it off; a quality-preset click preserves it).
- **ONE gate function.** `native_mailbox::feedback_enabled(&settings) -> bool`. Every
  entry point asks it; nothing reads the flag directly. **This is where a future Signet
  adult/age boolean gets ANDed in** (age-gate later, not now). The boolean would come
  from third-party Signet, never a birth date we hold (CLAUDE.md red line 3).
- **Off = invisible.** The three commands are registered but hidden
  (`CommandRegistry::set_hidden`): not in `/help`, `/help bug` and typing `/bug` answer
  exactly as for any unknown command. Nothing new is queued. The "Suggestion Box" trial
  (objective `SendFeedback`) is hidden from the lobby Trials list, the J board,
  `/trial list` and `/scenario list`, and is not startable by name. The Your-relays blurb
  only mentions bug reports while the gate is on. (A report already queued before the
  gate was switched off still flushes — it was composed deliberately.)
- **Unlock.** Settings panel ("Graphics") — opened from the pause menu in a world, or from
  the **Settings** button in the lobby header (the same panel and state, so the unlock is
  reachable without entering a world; the lobby applies only world-independent effects —
  see `menu::plan_lobby_settings`): the version line
  "AxeNStax v<version>" at the bottom. Tap it **7 times**, each tap within **3 s** of the
  previous (a longer gap resets the count). From tap 3 a small hint reads "N more taps to
  turn on tester feedback"; at tap 7 it reads "Tester feedback on — /bug and /idea are
  now available" and settings are saved. Once on, a checkbox "Tester feedback (/bug,
  /idea, /mailbox)" appears; unticking turns it off and hides the checkbox again
  (re-enabling takes 7 taps again). The tap counter is a pure struct
  (`tester_gate::TapCounter`, time passed in) with unit tests.
- **Tests.** Tap counter (7 in window unlocks; a gap over 3 s resets; hint countdown);
  gate (hidden from `/help` and answers like an unknown command when off; listed and
  working when on; nothing queued when off); settings default and old-file load = off.
