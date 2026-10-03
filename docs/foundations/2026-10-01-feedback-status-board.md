# Feedback status board (native) — replaces mailbox replies

**Status:** READY TO BUILD (2026-10-01). Native only. Supersedes the reply half of
`docs/foundations/2026-06-07-lobby-mailbox-feedback.md` and the native-mailbox
reply inbox (2026-07-24).

## 1. Why

Owner decision 2026-10-01 (Children's Code posture): AxeNStax must not run a
two-way channel with players, who may be children. The only thing a reporter
needs back is "we got it" / "it's fixed in vX". That is delivered as a **public,
anonymous status board** the game reads — nobody is ever messaged, and no
report is linkable to a person.

## 2. Decisions

| # | Decision |
|---|---|
| S1 | **Ticket = the report's `report-id`**, provided it is ≥128 bits from a CSPRNG (if the current id is weaker, make it so). It travels only inside the encrypted report; the game keeps it locally. |
| S2 | **Board entry key** = `sha256("axenstax-feedback-ticket:" + ticket)` hex, first 32 chars. Unlinkable without the ticket. |
| S3 | **Board event:** one addressable kind-30078 event, `d = "axenstax-feedback-status"`, signed by the **existing official feedback key** (`OFFICIAL_AXENSTAX_PUBKEY_HEX`, already pinned in the binary — the reader already holds its secret). Content (plain JSON, public): `{"v":1,"updated":<unix>,"t":{"<hash>":{"s":"received"\|"fixed"\|"wontfix","v":"<version or omitted>","at":<unix>}}}`. Pruned to entries ≤180 days old and ≤2000 entries, oldest first. No free text, no npubs, no titles. |
| S4 | **Relays (updated 2026-10-02):** the project's fixed public inbox relays, `native_mailbox::FEEDBACK_INBOX_RELAYS` = `wss://nos.lol`, `wss://relay.primal.net`, `wss://offchain.pub` (each served kind 1059 without NIP-42 auth in a read-only probe; `relay.damus.io` demanded auth). Reports are published to all of them (sent once any one accepts); the board is published to and read from the same set; `tools/feedback-reader` defaults to it. Not the player's own relay list, so custom relays never cut a player off. Trotters is no longer a default. Client verifies the event's author == the pinned key and its signature. |
| S5 | **Sending becomes anonymous:** every native report is sealed with a **fresh per-report burner key** (as web did), and the `persona` tag is dropped. The long-lived device mailbox key is no longer used for sending; delete its use for replies. The `build` and `client` tags stay. |
| S6 | **Remove the reply path entirely:** native reply inbox (`native_mailbox/inbox.rs` receive/decrypt of replies), its UI, and `tools/feedback-reader/reply.mjs` + any reply code in the reader. The device key file is deleted on first launch of the new build (and its module removed if nothing else uses it). |
| S7 | **Client reads the board:** on boot (off the frame thread), when `/mailbox` opens (≥60 s debounce), then hourly. `/mailbox` becomes "Your reports": each local ticket (kind, first ~60 chars of the body, sent date — all LOCAL only) with its status: Sent / Received / Fixed in vX / Won't fix. Local ticket list in `profile/` (0600), pruned after 180 days. |
| S8 | **Reader tool:** `node status.mjs <report-id> received\|fixed\|wontfix [--version X]` updates a local board file and publishes the signed event; `read.mjs`/`live.mjs` may auto-mark new reports `received` behind a `--auto-received` flag (default off). Ledger stays owner-local. |
| S9 | Web: nothing (web has no feedback channel since 2026-10-01). |

## 3. Privacy statement (for the privacy page)

Native bug/idea reports are end-to-end encrypted to AxeNStax and sent from a
one-time key with no account or identity attached. We never reply to or message
players. Report status is published as an anonymous list of scrambled ticket
numbers that only the sender's own game can recognise.

## 4. Acceptance

- Unit tests: ticket hash; board parse (wrong author/sig rejected, oversize/ junk
  tolerated, unknown status ignored); status matching against local tickets;
  per-report burner (two reports → two different seal pubkeys, no persona tag).
- Reader: test for status.mjs board update + pruning.
- `./check.sh` green; web bundle unchanged by this work.
- Owner live test: `/bug` in the AppImage → `read.mjs` shows it → `status.mjs <id> fixed --version 0.2.28` → the game's `/mailbox` shows "Fixed in 0.2.28".

## 5. Tester gate (2026-10-03)

The whole status-board feature — `/bug`, `/idea`, `/mailbox` and the board refresh that
`/mailbox` triggers — is an **alpha-tester feature, off by default** (hidden unlock:
Settings, tap the version line 7 times). When off, the commands are hidden and answer
as unknown commands, so no report can be queued and `/mailbox` shows nothing. The board
worker already makes no network call while there are no local tickets, so a player who
never unlocks it generates no feedback traffic at all. Full rules, the single gate
function (`native_mailbox::feedback_enabled`, where a future Signet age boolean is
ANDed in) and the unlock UX: `2026-06-07-lobby-mailbox-feedback.md` § "Tester gate".
