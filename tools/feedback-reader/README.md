# feedback-reader — local dev reader for the lobby mailbox

A **local, trusted** tool that decrypts player feedback (NIP-17 DMs to the
AxeNStax npub) into a parseable ledger, and publishes the anonymous status
board (`status.mjs`). It never messages a player. It holds the
**AxeNStax secret key**, so it is **never served publicly** and the ledger is
**never committed**.

Spec: `docs/foundations/2026-06-07-lobby-mailbox-feedback.md`. The kid side that
produces these DMs is the native mailbox (`game/engine/src/native_mailbox/`). The
browser build no longer has a feedback channel (removed 2026-10-01); older web
reports may still be on the old relay. The gift-wrap helper is `lib/nip59.cjs`
(loaded via `lib/nostr.mjs`).

**Relays.** Reports land on the project's **inbox relays** — `wss://nos.lol`,
`wss://relay.primal.net`, `wss://offchain.pub` (`INBOX_RELAYS` in
`lib/board.mjs`, the same fixed set as `FEEDBACK_INBOX_RELAYS` in
`game/engine/src/native_mailbox/mod.rs`). They were chosen (2026-10-02) because
each serves kind 1059 without NIP-42 auth; `relay.damus.io` demands auth for
1059, so a report sent there could not be read back. Every tool here reads and
publishes to that set by default; `RELAY=url` or `RELAYS=a,b` overrides it.
Builds up to v0.2.27 still send to `relay.trotters.cc`, so `read.mjs` and
`live.mjs` also listen there by default (`LEGACY_RELAYS`) until those installs
age out; `RELAY=`/`RELAYS=` overrides the whole set.

## Setup

```bash
cd tools/feedback-reader
npm install
```

The secret lives **outside the repo** at
`~/.config/axenstax/axenstax-official.json` (mode 600). Override with
`KEY_FILE=…` if relocated.

## Read inbound reports → ledger

```bash
node read.mjs            # catch up on stored DMs, then subscribe live (Ctrl-C to stop)
node read.mjs --once     # catch up + exit
```

Each report becomes a JSONL row in `feedback-ledger.jsonl` (gitignored). The row
records the **npub, the handle, and the bug/idea** (owner directive 2026-06-11):

```json
{"id":"…","fromNpub":"npub1…","handle":"Secret Pete","type":"bug",
 "body":"doors too tall","ts":…,"status":"new","verdict":null,"wrapId":"…"}
```

`handle` is the reporter's **signed-in** persona handle — the display name they
signed in with. The client embeds it on the report at send time (from the Signet
session); if it's missing we fall back to resolving their Signet persona
credential (kind-31000) from the relay. We never read **kind-0** for this: the
Hash Dash leaderboard lets a player publish a kind-0 with a *typed* name, which
is not "who they signed in as". `null` if neither source has a name. Since
2026-10-01 native reports carry **no persona or handle** (each is sealed with a
one-time key), so those fields are only ever filled for older native rows and web
rows. `status` lifecycle: `new → triaged → resolved`.

### Repair old rows

Rows logged before the signed-in-handle fix resolved the handle from kind-0 and
may show a Hash-Dash-typed name. Re-resolve them from the persona credential
(kind-31000) in place — embedded send-time handles are left untouched:

```bash
node read.mjs --reresolve-handles    # fix existing rows, then exit
```

## Triage (Claude-assisted, on demand — INTERNAL ONLY)

**Feedback is NEVER filed to GitHub** (owner directive). Bugs and ideas live only
in this local ledger — npub + handle + content — and are triaged in place. Point
a Claude Code session at the ledger:

```bash
node list.mjs --new      # the reports awaiting triage
```

For each `new` report, Claude decides bug / idea / discard and records the
verdict on the row — no external system, nothing published:

```js
import { Ledger } from './lib/ledger.mjs';
new Ledger('feedback-ledger.jsonl').update(reportId, { status: 'triaged', verdict: 'bug' });
```

## Report status — the public status board (no replies)

**Nobody is ever replied to or messaged** (owner decision 2026-10-01, Children's
Code posture; spec `docs/foundations/2026-10-01-feedback-status-board.md`). The
only thing a reporter gets back is "we got it" / "it's fixed in vX", delivered as
ONE public, signed, addressable event (kind 30078, `d = axenstax-feedback-status`)
that the game reads. Each entry is keyed by
`sha256("axenstax-feedback-ticket:" + report-id)` (first 32 hex chars) — the
report-id (the "ticket") only ever travels inside the encrypted report and is
kept by the sender's game, so no entry is linkable to a person. No free text, no
npubs, no titles.

```bash
node status.mjs <report-id> received|fixed|wontfix [--version 0.2.28] [--dry-run]
```

`status.mjs` updates the local board file (`BOARD`, default
`./feedback-status-board.json`, mode 0600, gitignored), prunes it (≤180 days old,
≤2000 entries, oldest first), signs the event with the official key and
publishes it to the inbox relays the game reads the board from (`RELAYS=a,b`
to override). A failed publish leaves the board file untouched. `fixed` and
`wontfix` also mark the ledger row `resolved` (owner-local; the ledger is never
published).

Optionally let the reader acknowledge new reports for you:

```bash
node read.mjs --auto-received      # also: node live.mjs --auto-received
```

marks each newly ingested report `received` on the board (default off; never
overrides a status already set; one publish per burst).

The game side: on boot, when `/mailbox` opens (≥60 s apart) and hourly, the
native client fetches the board, checks the author is the pinned official key
and the signature is valid, and matches entries against its own local tickets.
`/mailbox` shows each ticket as Sent / Received / Fixed in vX / Won't fix.

> Old `reply.mjs` and its native-only reply policy were removed with the reply
> path; there is no way to send a player a message from this tool.

## Boundaries

- The **live relay run with the real AxeNStax key** is owner-only (an unattended
  session must not handle the secret). The crypto + ledger + privacy logic is
  unit-tested with throwaway keys in `test/` (`npm test`).
- The board is public by design: it holds only scrambled ticket hashes, a status,
  an optional version and a timestamp. Never add free text, npubs or titles to it.
