# Native Mailbox — feedback send + maker replies on the native client

> **SUPERSEDED (reply half) by `docs/foundations/2026-10-01-feedback-status-board.md`.**
> The reply path described below (device mailbox key, reply inbox, `/mailbox` reading
> replies, `reply.mjs`, the `persona` tag) was REMOVED 2026-10-01 (Children's Code
> posture: no two-way channel with players). Native reports are now sealed with a
> fresh per-report burner key and carry no persona; `/mailbox` shows each local
> ticket's status read off a public, anonymous, signed status board. The sending
> half (outbox, wire format minus `persona`, relay worker) still stands.

**Date:** 2026-07-24 · **Status:** approved (owner: "smash it") · **Owner decisions baked in:**
web-origin reports never get replies (anonymity-first); native reports are replyable when
sent signed-in; native signed-out sends are allowed but anonymous (never replied to);
replies surface as chat line + `/mailbox`; trotters kind-1059 retention gets bumped now.

Parent spec: `docs/foundations/2026-06-07-lobby-mailbox-feedback.md` (wire format, ledger,
privacy posture). This spec makes the native client a first-class mailbox participant;
the web path (`mailbox.js` / `wasm_feedback.rs`) is untouched.

## Why

`/bug` and `/idea` on native are today a no-op pointing at the web client
(`commands/builtins/feedback.rs`). Policy (2026-07-24) says native logged-in players get
the reply loop — so native needs to send reports and receive replies.

## Architecture decision: device mailbox key

The persona secret key never lives on the device (NIP-46 bunker signs remotely). Sealing
reports with the persona key would mean a phone round-trip per send **and per inbox
decrypt** — rejected. Instead:

- Native mints a **local mailbox keypair** on first use → `profile/mailbox_key.json`
  (0600, same posture as `signet_session.json`). It is a transport identity, nothing more.
- Reports are **sealed with the device key** (instant, offline-capable) and gift-wrapped
  to the official AxeNStax pubkey (`0bb8a9199a3e3c240a378cf1b5a977945decf05fc7d3021f66faa6b049cb55fd`,
  same committed constant as `mailbox.js`).
- **Who-you-are travels inside the rumor as tags** (like web's `handle` tag): when signed
  in, a `persona` tag carries the persona pubkey. The reader resolves the display handle
  from the persona's kind-31000 credential (existing fallback path) — native does not
  need to know the handle.
- **Replies target the device key**: the reader replies NIP-17 to the seal pubkey; the
  native inbox subscribes `#p = device pubkey` and decrypts locally. No bunker involved
  anywhere in the mailbox.

## Wire format (native additions in bold)

Rumor identical to web: `kind 14`, `content` = report body, tags:
`['t', 'bug'|'idea']`, `['report-id', <uuid>]`, **`['client', 'axenstax-native']`**
(web keeps `'axenstax'`), **`['persona', <persona pubkey hex>]` when signed in** (absent
when signed out ⇒ anonymous ⇒ never replied to). Wrapped with rust-nostr's NIP-59
(`nostr = 0.44.3`, `gift_wrap` / `extract_rumor` — no hand-rolled crypto).

## Engine components (all native-only, `#[cfg(not(target_arch = "wasm32"))]`)

1. **`native_mailbox.rs`** — the module. Owns:
   - Device key mint/load (`profile/mailbox_key.json`).
   - **Outbox** `profile/mailbox_outbox.json`: queued reports `{id, type, body,
     persona_hex?, created_at, status: queued|sent, event_id?}`. Enqueue is offline-safe;
     flush marks `sent` by event id; failures stay `queued` and retry — never double-send
     (mirror web semantics).
   - **Inbox** `profile/mailbox_inbox.json`: `{id, body, report_id?, ts, read}` +
     receive watermark. Only replies whose **seal author == official AxeNStax pubkey**
     are accepted (stranger wraps dropped with a warn, same rule as web).
   - **Flush worker**: background thread + small tokio runtime (same pattern as
     `native_join_sign_driver`), talking to the game thread via channels. Runs at
     launch, after each enqueue, and on a ~5-minute poll. Relay I/O =
     `tokio-tungstenite` to `wss://relay.trotters.cc` using **raw
     `["REQ",…]`/`["EVENT",…]` framing** (trotters rejects nostr-tools' framing —
     mirror `tools/feedback-reader/live.mjs`).
   - Offline-first contract (binding, from `2026-06-10-offline-first-login…`): nothing
     here ever blocks play; a dead relay degrades to "queued".
2. **`commands/builtins/feedback.rs`** native arm: enqueue (reads
   `signet::native_signer::load_identity()` for the persona), kick flush, reply
   "Queued — sending to the makers." Signed-out: still queues, no persona tag.
   Empty-arg native message changes from "web client" pointer to a short usage hint.
3. **`/mailbox` builtin** (new): lists inbox newest-first in the chat overlay, marks
   read. Register in `commands/builtins/mod.rs` (+ help).
4. **Chat notification**: game loop drains `native_mailbox::take_arrivals()` each tick →
   "📬 Message from the makers — /mailbox" chat line; on world join, unread count line
   if any.

## Reader-side (`tools/feedback-reader`)

- `read.mjs` / `live.mjs`: record `origin` (from `client` tag; absent ⇒ `web`) and
  `personaNpub` (from `persona` tag, rendered bech32 — npub-only display rule) on each
  ledger row.
- **`reply.mjs` gate (the policy enforcement point)**: refuse to send unless the row has
  `origin: 'axenstax-native'` **and** a persona. Reply is addressed to the row's seal
  pubkey (the device key).
- `README.md` + parent foundations spec updated to match.

## Infra (parallel, not blocking)

Ask (via `<workspace>/forgesworn/signet-plans/MESSAGE-FROM-AXENSTAX.md`, the established
trotters/Signet channel): bump kind-1059 retention on `relay.trotters.cc` from ~2h to
≥30 days. Fixes (a) replies sent while the player is offline, (b) the long-standing
inbound drop (wraps are backdated up to 48h; catch-up reads miss them — 2026-06-20
runbook). Content is E2E ciphertext; retention holds ciphertext only.

## Privacy / red-lines check

- Relay carries only feedback (explicitly allowed use of trotters; no game traffic).
- Reports remain E2E: ciphertext on the relay, plaintext only in the owner-local ledger.
- No new data collection; no age data; anonymous native send preserved (no persona tag).
- Device mailbox key is local-only, transport-scoped, never displayed as an identity.

## Testing

- Wrap → unwrap round-trip with throwaway keys (offline, rust-nostr).
- Tag policy: signed-in ⇒ `persona` tag present; signed-out ⇒ absent; `client` is
  `axenstax-native`.
- Seal-author verification: stranger reply dropped, official accepted.
- Outbox: enqueue/flush/mark-sent idempotence; queued survives restart; never double-send.
- Inbox: dedup by event id; watermark; read flags survive restart.
- Command arms: native `/bug` queues (signed in and out); `/mailbox` lists + marks read.
- Reader: `npm test` additions — origin/persona recorded; `reply.mjs` refuses web rows
  and persona-less native rows.
- **Owner playtest boundary:** live relay round-trip (native `/bug` → ledger →
  `reply.mjs` → native chat notification) needs the real key + a second machine.

## Explicitly out of scope

- Web client changes (its mailbox and no-reply policy are already correct).
- Title-screen inbox panel (chat surface only, per owner pick).
- Handle lookup on native (reader resolves kind-31000).
- Relay HA / fallback relays.
