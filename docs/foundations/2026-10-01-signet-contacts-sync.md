# Signet contacts sync (native)

**Status:** BUILT (2026-10-01) — wire port + relay I/O, storage, book assembly and Friends row; owner live phone test outstanding. Native only.
**Upstream contract:** `forgesworn/signet-contacts` `docs/WIRE.md` (v2, frozen) +
`vectors/*.json`. This doc only records AxeNStax's choices on top of it; where
the two disagree, WIRE.md wins and this doc is the bug.

## 1. What and why

The native address book (`contacts.rs`: Kenspeckle export ∪ `profile/contacts.json`
mirror) is fed today by invites, pasted npubs and a hand-dropped Kenspeckle file.
mySignet now offers an app-access rail: the player pairs AxeNStax once, approves
it on their phone, and Signet publishes an encrypted, replaceable snapshot of
their contacts that only AxeNStax's app key can read. This feature consumes it,
so Kin/Kith from Signet admit to online play (`online_admission.rs`) and world
chat tiers (`comms.rs`) without any manual import.

Red lines: the data goes from the player's own Signet to the player's own disk
over a relay; AxeNStax runs no service in the path and keeps no copy (lines 2/3).
Nothing here makes a directory (line 1).

## 2. Decisions

| # | Decision |
|---|---|
| D1 | **Capabilities requested:** `signet.contacts.read:directory`, `signet.contacts.read:tier`, `signet.contacts.blocks.read`. Nothing else (no methods, roles, checks, proposals, invites). `requestedCapabilities` is passed to ack validation so a wider ack is refused. |
| D2 | **`dir=owner`** only. Guardian-pairs-for-child (`dir=dependant`) and the child flag are **deliberately out of scope** (owner call 2026-10-01) — Signet-sourced contacts get `is_child: false`. Paired-child Signet installs refuse this pairing upstream anyway. |
| D3 | **Carrier:** the QR shows the web carrier `https://mysignet.app/?pair=1&<query string from the v2 builder>` (desktop→phone hand-off, WIRE.md §3 carriers). Host is a single constant, not scattered. Also show a "copy link" button with the same string. |
| D4 | **Pairing relay (updated 2026-10-02):** the **first relay of the player's "Your relays" list** (default `wss://relay.damus.io`, a public relay; trotters is no longer a default). The v2 pairing URI carries exactly one `relay=`, which mySignet reads (`signet-contacts/src/wire/pairing.ts`, `params.get('relay')`) and publishes its ack to (`signet-app/src/App.tsx`, `new RelayClient(req.rendezvousRelay)`) — so the app chooses it. After the ack, all fetches use the **ack's `relay`**, never the pairing relay by assumption. |
| D5 | **App key:** a fresh secp256k1 key per grant, generated locally, never the persona key. Stored with the grant (§4). |
| D6 | **Verification code is mandatory** (WIRE.md §3 B1/F1): after the ack, show `formatPairingCode(...)` large, with "Type this code into Signet. Press Continue only when Signet says the code matches. If it doesn't, press Cancel." The grant is not persisted, fetched or used until Continue. Cancel/mismatch discards it and a retry mints a new challenge. Never hide the code. |
| D7 | **Snapshot, kept separate:** the latest accepted projection is written to `profile/signet-contacts.json` (replaced wholesale per newer projection, newest-wins by `(publishedAt, maxClock)` per WIRE.md §5). It is **not** merged into `contacts.json`. |
| D8 | **Book assembly:** `load_local_book()` = Kenspeckle ∪ mirror ∪ Signet snapshot (via existing `upsert`, so the closest tier wins and nothing the player set is loosened), **then** every pubkey on a Signet `blocked: true` contact is removed from the book entirely (→ `Stranger`). Blocks beat every other source. |
| D9 | **Tier map:** `kin→Kin`, `kith→Kith`, `ken→Ken`, `none`/absent → contact skipped (not added). Each contact's `identities[].pubkey` all map to the same tier/display name (one `Contact` per pubkey). `AddedVia::Import` (already reserved for this). `added_at` = first time seen (preserved by `upsert`). |
| D10 | **Freshness, fail closed:** if `now > expiresAt` of the stored snapshot, its contacts are **not** added to the book (no admission from stale data) but its **blocks still apply**. `revoked: true` → delete grant + snapshot; UI says "Signet disconnected this game". |
| D11 | **When to fetch:** on boot (off the frame thread), when the Friends column opens (debounced, ≥60 s), and every 15 min while running. Never on the frame thread. Failure is silent except a "last synced" line. |
| D12 | **Disconnect:** a button deletes grant + snapshot locally. (No upstream revoke message exists in v2; the copy tells the player to remove "Axe'n'Stax" in Signet too.) |
| D13 | **`truncated: true`** → Friends column shows "Your Signet list may be incomplete". `awaitPairingAck` timing out → "No answer from Signet. If you've connected lots of apps, Signet may be at its limit (10)." (WIRE.md §8). |

## 3. Wire port (pure, generic)

A Rust port of the **consumer** half of WIRE.md, in `game/engine/src/signet/contacts_wire/`
(no AxeNStax types, no game deps — liftable to a standalone crate later per the
shared-infra strategy). Needed pieces, each conformance-tested against the
upstream frozen vectors (copy the needed `vectors/*.json` into
`game/engine/src/signet/contacts_wire/vectors/` with a README naming the upstream commit):

- `build_pairing_uri` — byte-exact vs `pairing.v2.json` (parameter order binding).
- `ack_tag`, `projection_tag`, `scoped_contact_id` — §4 derivations.
- `parse_ack` — §5 shape, `v==2`, challenge echoed case-preserved, caps clamped and ⊆ requested, `maxStalenessSeconds` clamp.
- `pairing_code` + `format_pairing_code` — vs `pairing-code.json` (algorithm from upstream `src/wire/pairing-code.ts`).
- `open_vault_envelope` — §1 (NIP-44 unwrap of `k` against rail pubkey, AES-256-GCM, 4-byte BE length prefix) vs `envelope.v2.json`; every failure → `None`.
- `parse_projection` — §5/§6/§10: field-coverage refusal (whole projection `None`), item-level drop, scopes ⊆ granted, `expiresAt − issuedAt ≤ maxStalenessSeconds`, caps from §8, `sanitize_wire_text` vs `sanitise.json`; vs `projection.v2.json` incl. every `uncovered` case.
- `newer_than(a, b)` — `(publishedAt, maxClock)` order; `revoked` exempt.

Relay I/O sits outside the pure module: an ack waiter implementing WIRE.md §3
"Ack delivery" (21237 `#p` filter AND the separate 30078 `#d=ackTag` filter,
up to 10 candidates newest-first, decrypt+challenge gate, ≤32 attempts, paging
rule, poll-before-deadline, 600 s default) and a projection fetch (`kinds:[30078]`,
`authors:[railPubkey]`, `#d:[projectionTag]`). Use the existing `nostr`/`nostr-sdk`
stack the engine already uses for the mailbox / rendezvous.

## 4. Storage

`profile/signet-contacts-grant.json` (0600): `{v:1, app_secret_hex, grant_id,
rail_pubkey, projection_tag, relay, granted_capabilities, max_staleness_seconds,
paired_at}`. `profile/signet-contacts.json`: the last accepted projection body
(already parsed/sanitised) + `fetched_at`. Both append-only formats per the
codebase convention; a corrupt file is quarantined the way saves are, never a panic.

## 5. UI (Friends column)

A "Signet contacts" row: **Connect Signet contacts** → QR + link + "Waiting for
Signet…" (cancel) → code screen (D6) → "Connected · N contacts · synced 2 min ago"
+ **Sync now** + **Disconnect**. Copy is plain and contains no social-network
framing (red line 4).

## 6. Out of scope

`dir=dependant`, `is_child` from Signet, proposals (`add-ken` on invite accept is
a good follow-up), app introductions, avatars, web build (forbidden-symbol check
must stay green).

## 7. Acceptance

- Conformance tests green against every copied vector; `trials_lint`-style loud
  failures for refusal cases.
- Unit tests: book assembly (D8 block beats local Kin; D9 `none` skipped; D10
  stale snapshot adds nobody but blocks still apply; revoked clears).
- `./check.sh` green, `tools/smoke/forbidden-symbol.mjs` still proves no contacts
  symbols in the web bundle.
- Owner live test: pair with mySignet on a phone, type the code, see Kin/Kith
  appear, block someone in Signet → they vanish on next sync.

## 8. As built (2026-10-01)

- Wire port: `game/engine/src/signet/contacts_wire/` (§3). Everything else:
  `game/engine/src/signet_contacts/` — `store.rs` (§4 files, atomic 0600
  writes, corrupt → `save::quarantine_corrupt`), `book.rs` (D2/D8–D10),
  `ack_wait.rs` (WIRE §3 ack delivery, relay behind a trait, unit-tested with
  fakes), `relay.rs` (raw REQ/EOSE sockets, worker threads only), `sync.rs`
  (projection filter + acceptance + `sync_once`), `pairing_flow.rs` (D5/D6
  state machine), `ui.rs` (§5 row), `mod.rs` (worker: boot sync, 15-min loop,
  column-open trigger debounced to 60 s).
- Ack waiter: each poll opens one socket with the two filters as separate
  subscriptions, reads to EOSE, then listens live up to 2 s (the ephemeral
  21237 only reaches a live subscriber) before the next pass.
- `contacts::load_local_book()` = `assemble_book(Kenspeckle, mirror, Signet
  part)`; blocks removed last. **Mirror writes now go through
  `contacts::record_in_mirror` / `record_visit` (one row, mirror-only)** — the
  three game-loop writers and the online host used to save the whole merged
  book, which would have copied Signet rows into `contacts.json` (breaking D7,
  D10, D12). A visit is recorded with the row the join itself built
  (`online_join`: `Kith`, `AddedVia::Invite` — being let in earns Kith, the
  pre-Signet rule), so the mirror gets what the join earned and never Signet's
  own tier (a Signet Kin visited is written as Kith; a Signet block still
  beats it). `record_visit`'s `Import` → `Stranger` branch is a guard for a
  future caller that passes a book row; no production caller does today.
- A book change from sync (newer snapshot, revocation, disconnect, **or a
  staleness flip**) sets `contacts_dirty` and pushes the fresh book into a
  hosted world. The push carries the Signet blocked set read in the same
  snapshot read (`contacts::load_local_book_with_blocks`): `OnlineHost::set_book`
  drops blocked pubkeys from the book, from its own invite rows and from this
  session's admissions; `OnlineHost::access_policy` gives the whitelist minus
  blocks and the blocks as `HostedServer`'s blocklist; and
  `HostedServer::kick_pubkeys` (the operator-kick teardown) disconnects a
  blocked player already in the world. The host's offer loop refuses a
  blocked persona in silence before the bearer check. Blocks are seeded at
  host start, before the first offer is read.
- Staleness is time-driven (D10): the worker re-reads the snapshot every 60 s
  between 15-minute syncs, and any flip in staleness re-pushes the book, so
  expired Kin/Kith leave a hosted world's whitelist without a fetch (a
  `Failed` fetch included). That refuses their next join; a player already in
  the world is not removed for staleness — only a block disconnects.
- Relay I/O is bounded: each socket open has a 10 s cap and each whole
  query/ack poll an overall cap, so a hung relay cannot hold the worker.
- Disconnect deletes both files on the calling thread at once, bumps a
  generation and re-pushes the book; a sync in flight under the old
  generation has its result discarded (no write, no "last synced"). The
  queued worker Disconnect deletes again in case that sync wrote first.
- A snapshot counts only when its `grant_id` equals the stored grant's (no
  grant → nothing): a leftover from an older grant, or one written after a
  disconnect, is ignored by both the book and the Friends row.
- On Continue the old snapshot (if any) is removed with the new grant saved; a
  failed removal is logged and the file moved aside (`quarantine_corrupt`),
  and is ignored anyway by the `grant_id` rule.
