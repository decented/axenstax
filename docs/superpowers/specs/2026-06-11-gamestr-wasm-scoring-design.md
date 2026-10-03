# gamestr leaderboard scoring — WASM v1 design

**Date:** 2026-06-11
**Status:** Approved (owner, 2026-06-11) — build to live
**Scope:** Publish Hash Dash scores to the gamestr (NIP-133, kind 33334) leaderboard
from the PWA/WASM build. Opt-in, default OFF. Honor-system integrity.

## 1. Background

gamestr.io (github.com/nosdav/gamestr) is an open Nostr leaderboard. Score events
are **NIP-133, kind 33334** parameterized-replaceable events:

- `d` tag = a unique string for the game (the game id).
- `content` = either a stringified number (sats by default) **or** stringified
  JSON by category — the spec's own example is `{"steps":10000,"water":5000,"work":20000}`.
- Replaceable: relays keep only the latest event per `(pubkey, d)`.
- Identity = npub. Names resolve from kind-0 profiles (gamestr's concern, not specced).
- Optional future tags `p` (witness) / `c` (commitment) — NOT used here.

We are already Nostr-native: the PWA has a signer (`window.__axenstax_get_signer`),
a relay client (`window.AxeRelay`), and a proven publish pattern (`beacon.js` +
`open_stash.rs`). Publishing a kind-33334 event is the same machinery, minus
NIP-44 (scores are public).

**Decisions (owner, 2026-06-11):**
- **Hash Dash only** for v1. Its score *is* work (higher = better), mapping 1:1
  onto NIP-133 `content {"work": n}`. Satori Rush (time-to-genesis, lower = better)
  clashes with the higher-is-better model and is deferred until we pick a derived
  score.
- **Opt-in, default OFF, with a handle field.** Standing rule: identity defaults
  NON-public; going public is always an explicit opt-in. There is no guest/burner
  sign-in yet, so a published score is signed by the player's real Signet npub —
  the default-OFF gate is the privacy guard.
- **Honor-system integrity.** Client-signed scores are forgeable. Acceptable for a
  staffed booth; server-validated proof-of-play is the long-run answer (out of scope).

## 2. Architecture

All gamestr concerns live in **JS** (`gamestr.js`); the engine only *triggers* the
offer at round-end. This mirrors `beacon.js`/`open_stash.rs` and the existing
HTML-overlay-over-canvas pattern (`__axenstax_feedback_overlay_open`, `input.rs:20`),
so egui is untouched.

### 2.1 `tools/sites/game/static/gamestr.js` (new)

Self-contained IIFE, loaded after `auth.js` (needs the signer hook). Exposes two
wasm-bindgen extern targets:

- `window.axenstax_gamestr_available()` → `bool` — true iff a signer and
  `window.AxeRelay` are present.
- `window.axenstax_gamestr_offer(gameId, work)` → shows an **HTML overlay** over the
  canvas and sets `window.__axenstax_gamestr_overlay_open = true` (engine gates game
  input while set, same as the feedback overlay). The overlay shows:
  - the score (*"You did N work."*),
  - a checkbox **"Show my score on the leaderboard"** — **default OFF**, last choice
    remembered in `localStorage`,
  - a **handle** text field — last value remembered in `localStorage`,
  - **Post** and **Not now** buttons.

  On **Post** (only if the checkbox is ticked):
  1. if the handle is non-empty, sign + publish a **minimal kind-0** profile
     `content: {"name": handle}` on the current pubkey (so the board shows a name);
  2. sign + publish the **kind-33334** score event;
  3. close the overlay (clear the flag).

  On **Not now / close**, or if the checkbox is unticked: nothing is signed or
  published — the score stays on the device. Always clears the flag.

**Event construction (the only wire contract):** — *content form revised, see
Revision 6.*
```
{ kind: 33334, created_at: <now>, tags: [["d", gameId]],
  content: String(N) }   // plain stringified number (NIP-133 default).
                         // WAS JSON.stringify({work:N}); the board couldn't render
                         // the category-JSON form — see Revision 6.
```

**Relay:** new `<meta name="gamestr-relay">`, default `wss://relay.trotters.cc`
(our own infra — so the publish loop is verifiable today). Overridable to the
gamestr booth relay once their team supplies it; publishing to several relays is a
one-line array change in `makeRelayClient`.

**Signer:** `window.__axenstax_get_signer().signEvent(template)` — identical to the
BUD-02 auth path in `beacon.js`.

### 2.2 `game/engine/src/gamestr.rs` (new, `#[cfg(target_arch = "wasm32")]`)

Mirrors `open_stash.rs`:

- `extern "C"` decls: `axenstax_gamestr_available() -> bool`,
  `axenstax_gamestr_offer(game_id: String, work: f64)` (catch).
- `pub fn gamestr_game_id(def: &ScenarioDef) -> Option<String>` — returns
  `Some("axenstax-hash-dash")` for the Hash Dash def, `None` otherwise. This is the
  single point that enforces the "Hash Dash only" scope. **Pure + unit-tested,
  available on all targets** (so native tests cover it).
- `pub fn offer_score(def: &ScenarioDef, score: u64)` — wasm-only; if
  `gamestr_game_id(def)` is `Some` and `available()`, `spawn_local`s the JS offer.
  Native build = no-op stub.

### 2.3 `game/engine/src/game_loop.rs` — the trigger

At the per-tick scenario site (~3826, after `scenario.tick()`), detect the
**not-ended → ended edge** with a one-shot latch (`scenario_offered: bool` on the
game state, reset when a new scenario starts). On the edge, call
`gamestr::offer_score(def, score)` **exactly once**. Wrapped in
`#[cfg(target_arch = "wasm32")]`; native is unaffected.

### 2.4 Template + input gate

- `tools/sites/game/templates/*` (the `/play` page that already loads `beacon.js`):
  add `<script src="…/gamestr.js">` after `auth.js`/`beacon.js`, and a
  `<meta name="gamestr-relay" content="wss://relay.trotters.cc">`.
- Input gate: the engine reads `window.__axenstax_gamestr_overlay_open` (mirror
  `input.rs:20`) and suppresses gameplay input while the overlay is open, so a click
  on the overlay doesn't also swing the pickaxe.

## 3. Data flow

```
play Hash Dash → timer ends → scenario.is_ended() flips true
  → game_loop edge latch fires once
  → gamestr::offer_score(def, score())            [wasm]
  → axenstax_gamestr_offer("axenstax-hash-dash", N)  [JS]
  → overlay: checkbox (OFF) + handle
  → Post (ticked) → [kind-0 handle?] + kind-33334 → relay
  → Not now / unticked → nothing leaves the device
```

## 4. Testing

- **Rust unit (all targets):** `gamestr_game_id` returns `Some` for Hash Dash and
  `None` for Satori Rush / the builtin test def / arbitrary defs.
- **Rust unit (logic):** the one-shot latch — given a scenario that flips ended,
  the offer is attempted exactly once across repeated ticks, and the latch resets
  for a fresh scenario. (Tested via a small seam that records "offered" instead of
  calling JS, so it runs on the native test target.)
- **JS (node, fake relay/signer like `mailbox.js`):** event shape is exactly
  `kind 33334`, `tags:[["d", gameId]]`, `content == {"work":N}`; and the **consent
  invariant** — checkbox OFF ⇒ `signEvent` is never called, nothing published.
- **Gate:** `check.sh` green (clippy + engine tests + `trunk build` + bundle size).

## 5. "Live and testable" boundary

Build → `check.sh` green → merge to main → auto-deploy. The owner can then play
Hash Dash on the live site, tick the box, and confirm a kind-33334 event publishes
to `wss://relay.trotters.cc` (verifiable with any Nostr client).

**The one external dependency (cannot be closed solo):** the actual gamestr **booth
relay URL** and their board reading our `d`-tagged events. Defaulting to our relay
makes the publish loop fully testable now; pointing at gamestr's board is a one-line
`<meta name="gamestr-relay">` change once their team supplies the URL (and a courtesy
check that they read `d` as the game id).

## 6. Out of scope (v1)

- Satori Rush boarding (needs a time→score model).
- Guest/burner sign-in + the firebreak npub (publish currently rides the real npub;
  the default-OFF gate covers the privacy requirement until a burner flow exists).
- Server-validated score integrity.
- A native build of the publish path (re-implemented in Rust later, sharing only the
  kind-33334 event shape; gated on the native NIP-46 signer being proven live).

## Revision 2 (2026-06-11) — always-show overlay + lazy signer on Post

Playtest surfaced two issues, both now fixed:

1. **Cursor stayed captured at the scenario end-card** (pre-existing Goal-1 bug):
   the engine never released the winit pointer lock when the blocking end-card
   appeared, so neither "Back to Menu" nor the gamestr overlay was clickable.
   Fixed engine-side: `ScenarioState::shows_blocking_end_card()` drives a
   `release_cursor()` on the not-ended→ended edge plus a re-capture gate in the
   click handler (mirrors the crafting-UI modal). Commit `6d5ade0`.

2. **Overlay never appeared because the signer was null at round-end.** Console
   confirmed `gamestr: signer/relay unavailable at scenario end`. A phone bunker
   reconnects in the **background** at boot (auth.js `Signet.restoreSession`),
   which can take >15s or fail if the app is asleep — so it often isn't connected
   at the instant a round ends (same reason cloud save shows `capable=false
   (no-signer)` until you sync). The original design gated the overlay on a live
   signer, so it silently no-showed.

   **Fix:** the overlay now **always appears** for an in-scope scenario (the
   engine-side `available()` pre-check and the JS early-return are removed). The
   signer is acquired **lazily at Post time** via `ensureSigner()`, which reuses
   the canonical reconnect (`window.Signet.restoreSession` →
   `window.__axenstax_set_signer`, bounded to 30s) — the explicit Post click is
   the user gesture that wakes the bunker, exactly like the lobby Sync button.
   On failure it shows an actionable hint ("make sure your signer app is awake").
   This makes the prompt independent of bunker timing, which is the right model
   for a booth.

## Revision 3 (2026-06-11) — prefill handle from the signed-in persona

Playtest feedback: at Hash Dash end the handle field was blank; it should
default to the persona the player already signed in with (editable before Post).

**Fix (JS-only, `gamestr.js`):** when the handle field would otherwise be empty
(no remembered handle), prefill it from the player's Signet persona via
`window.AxeHandle.fetchPersonaHandle(pubkey)` — the same kind-31000 display-name
credential the entrance card shows ("Signed in as …"). The pubkey comes from
`window.__axenstax_pubkey` (set after a verified sign-in, available even when
the bunker isn't connected), falling back to the live signer's pubkey. The fetch
is a relay round-trip, so the overlay appears immediately and the name lands when
it arrives — and only if the player hasn't started typing. A remembered handle
(their last explicit choice in `localStorage`) still wins over the persona.

Read-only and private: prefilling touches nothing on the wire; the handle is
only published if the player ticks consent and clicks Post. The game page already
loads `noble-curves.js` + `persona-handle.js`, so `AxeHandle` is available there.
This is a static-JS change (not compiled into the WASM bundle), so the gate is the
node tests + syntax check, not the full `check.sh` Rust/WASM build.

## Revision 4 (2026-06-11) — persona-handle lookup fans out across signet-app's relay set

The Revision-3 prefill (and the entrance "Signed in as…" card) read the persona
handle via `AxeHandle.fetchPersonaHandle`, which queried only our primary relay
(`wss://relay.trotters.cc`). But signet-app publishes the kind-31000 display-name
credential across its **default relay set** (`src/lib/relay-service.ts`): the
primary trotters + the 5 public defaults `nos.lol`, `relay.damus.io`,
`relay.nostr.band` (indexer, read-only), `relay.primal.net`, `relay.ditto.pub`.
A single-relay read can miss a credential that only reached the public relays.

**Fix (`persona-handle.js`, general infra — helps lobby + /play + gamestr):**
`fetchPersonaHandle` now defaults to a **multi-relay fan-out** across that whole
set — opens one socket per relay, merges all returned events, and runs the
existing verify + newest-wins + expiry selection over the union. Each relay
settles at most once (per-relay guard); the call resolves on all-relays-done or
timeout, whichever first. `opts.relayUrl` still forces a single relay (back-compat)
and a new `opts.relayUrls` takes an explicit list. The score-PUBLISH relay is a
separate concern (still the booth relay / trotters until the gamestr team supplies
their board's URL).

## Revision 5 (2026-06-12) — name field ALWAYS defaults to the signed-in handle + bunker nudge

Playtest (owner): the end-card name box should re-seed from the handle you
**signed in with** on *every* game, editable, and should NOT let a one-off edit
stick as the new default. Revisions 3–4 only filled from the persona when no
remembered name existed, so the last *typed* name won (`LS_HANDLE` seeded the
field; persona was the fallback) — the inverse of the desired precedence.

This is the **same class of bug** the feedback fix (`9e45dd3`) had just resolved
on the maker side: the lobby feedback reader was logging the player-edited Hash
Dash kind-0 instead of who-they-signed-in-as. That fix established the canonical
signed-in handle source: the synchronous Signet **session display name**
(`localStorage['signet:login.displayName']`, what the entrance card shows), with
the kind-31000 persona credential as the async relay fallback. Never a kind-0.

**Fix (`gamestr.js`, DOM layer only — pure core + its 8 node tests unchanged):**
- New `signedInHandle()` reads `signet:login.displayName` synchronously (capped to
  the field's 24 chars) — mirrors `cloud.js signedInHandle()` so the leaderboard
  and the feedback log agree on "who you are".
- The name field now seeds from `signedInHandle() || rememberedFallback` and, when
  there's no synchronous session name, fires `prefillPersonaHandle()` (kind-31000)
  which **overwrites** the remembered fallback unless the player has started
  typing (a `userEdited` flag replaces the old `value === ''` guard, so the
  signed-in persona is authoritative — it can win over a fallback already shown).
- `LS_HANDLE` is demoted to a **fallback-only** seed used when there is no
  signed-in handle at all (e.g. a guest who hasn't set one); for a signed-in
  player it is never read, so an edit no longer sticks across games.
- Added a bunker nudge under the field — *"⚡ Make sure your Signet bunker is on,
  so your score can post."* — shown only while the opt-in box is ticked, because
  Post connects the (often-asleep) phone bunker lazily and that can otherwise look
  like a silent failure.

Privacy invariant intact: nothing is signed/published unless the opt-in box is
ticked and Post is pressed; the prefill is read-only on-device.

## Revision 6 (2026-06-13) — close the §5 gap: publish to gamestr's OWN relays + plain-number content

**Symptom (owner, live):** a Hash Dash score posted, the in-app read-back logged
`CONFIRMED on relay ✓`, the gamestr sys-admin reported the board healthy — yet the
score never appeared on the leaderboard. This is exactly the §5 "cannot be closed
solo" external dependency finally biting in production.

**Root cause (proven, not inferred):** two independent gaps, both now fixed.

1. **Relay mismatch (primary).** We published *only* to `wss://relay.trotters.cc`
   (our own infra — the §2.1 default chosen so the publish loop was verifiable
   before the gamestr team supplied their URL). Reading gamestr.io's deployed
   bundle (`/assets/index-*.js`, 2026-06-13) showed its board reads
   `relay.gamestr.io`, `test.gamestr.io`, `nos.lol`, `relay.damus.io`,
   `relay.primal.net`, `relay.ditto.pub` — **trotters is not in the set.** A live
   probe of those relays (`node`, built-in `WebSocket`, REQ `{kinds:[33334]}`)
   returned 42 real score events across 15 games and **zero** `axenstax-hash-dash`
   events. The board literally never received ours — and the original read-back's
   ✓ was a false comfort: it queried the SAME relay we published to (trotters), so
   it only ever confirmed "trotters has it", never "the board has it".

2. **Content format (secondary).** §2.1 specced `content: {"work":N}` (the NIP-133
   category-JSON form). The same live probe showed every working single-score
   board uses a **plain stringified number** (`2048-nostrapps`, `asteroids-nostrapps`,
   `melrise`); only `wordswithzaps` uses category-JSON. Hash Dash's score IS a single
   number (work, higher = better), so the plain form maps 1:1 and is the lowest-risk
   form for an arbitrary board to render.

**Fix (`gamestr.js` + `index.html`, no engine/WASM change):**
- `buildScoreEvent` content is now `String(work)` (clamped non-negative integer),
  not `JSON.stringify({work})`. The wire contract in §2.1 is updated accordingly.
- New relay model: `boardRelays()` (where the board reads — **gamestr's own relays**),
  `verifyRelay()` (our trotters relay, for read-back), `publishRelays()` (both). The
  score now publishes to `relay.gamestr.io` + `test.gamestr.io` + `trotters.cc`.
- `<meta name="gamestr-relay">` in `game/engine/index.html` now carries gamestr's
  relays (comma-separated) — it previously (wrongly) pointed at trotters, which
  would have overridden the new default.
- The post-publish read-back now queries **gamestr's relays specifically**, so a
  `CONFIRMED ✓` genuinely means "on the board's infra" (the old false-positive is
  gone).

**Deliberate scope choice (privacy):** we publish to gamestr's *own* relays
(`relay.gamestr.io` / `test.gamestr.io`) and let gamestr's aggregator fan out to
the big public relays — we do **not** seed `nos.lol`/`damus`/`primal` ourselves, so
a child's name isn't blasted onto public infra by us (standing rule: identity
defaults non-public). Override via the meta tag if a future booth needs a wider set.

**Gate:** 8 `gamestr.js` node tests updated to the plain-number shape and green;
`check.sh` WASM build + bundle gate green. (The one unrelated red — a pre-existing
parallel-execution flake in `menu::tests::import_picked_world_garbage_bytes_fails_gracefully`,
which shares the process-wide `./worlds` dir — passes in isolation; not touched by
this change.)

**Still external:** final confirmation is the score *rendering* on gamestr.io's
board after a live replay; the new read-back log will print `CONFIRMED on a gamestr
relay ✓` once it lands there. §5's gap is now closed on our side.
