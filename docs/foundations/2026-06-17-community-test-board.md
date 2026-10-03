# Community Test Board — lobby playtest tracker

**Status**: ✅ BUILT 2026-06-17 (worktree `worktree-community-features`). Solo
vertical slice complete + `check.sh` green; live multi-tester round-trip + visual
feel = owner/playtest boundary.
**Date**: 2026-06-17
**Owner ask**: a lobby column listing features that need testing, each with a
searchable reference number → its test sheet, that **any signed-in user** can
mark with a traffic-light verdict (+ optional note), aggregated by the backend so
"confirmed by multiple people" is real, not self-asserted. Ordered by importance
(things gating pipelined work rank above no-dependency tweaks).

---

## TL;DR

A new **left lobby column, "Needs testing"**, lists features awaiting playtest,
importance-ordered. Each row has a four-state dot:

- ⚪ **grey** — untested (no verdicts)
- 🟠 **amber** — one tester confirms it works
- 🟢 **green** — `GREEN_THRESHOLD` (= **2**) distinct testers confirm it works
- 🔴 **red** — tested, doesn't work (failures **≥** goods; failure-leaning on ties)

Click a feature → its **ref** (a searchable token also present in the test
sheet), the test-sheet link, the current tally, a **note** box, and **✓ Works** /
**✗ Doesn't work** buttons. A verdict is queued as a `test-verdict` DM and sent
at the next stash sync; the backend tallies **distinct testers** and serves the
counts that colour the dots. Owner decisions (2026-06-17): green at **2**; red
when failures **≥** goods.

## Architecture (reuses existing rails)

| Concern | Where | Note |
|---|---|---|
| Status rule + ordering + registry | `game/engine/src/test_board.rs` (pure, 13 tests) | `board_status(good,broken)`, `rank_items`, embedded `assets/test_board.json` |
| Lobby column UI | `menu.rs::draw_test_board_column` (left `SidePanel`, wide screens) | mirrors the feedback/stash columns |
| Verdict transport | `wasm_feedback::enqueue_feedback("test-verdict", json)` → `mailbox.js` | **zero JS change** — the `/bug`·`/idea` rail is generic; type passthrough |
| Status read-back | raw web-sys `GET /api/test-board` in `menu.rs` (no JS bridge) | tolerant: offline/404 → all grey |
| Aggregation | `tools/feedback-reader/lib/test_board.mjs::tallyVerdicts` (pure, 7 tests) | distinct-npub, latest-verdict-wins, optional build filter |
| Runner | `tools/feedback-reader/test-board.mjs` | ledger → `{ ref: {good,broken} }` file |
| Serve | `tools/sites/game/app.py` `GET /api/test-board` | reads `data/test-board-status.json`; `{}` until published |

**The status rule lives once** (Rust `board_status`); the Node side only *counts
distinct testers* and never decides colour — so there's no two-language rule to
drift. Counts only are published: **notes + npubs never leave the local ledger**
(owner directive — feedback is internal-only). The verdict note rides inside the
same NIP-17 DM as `/bug`·`/idea`, so the makers see it beside the reporter's npub
+ signed-in handle in the existing ledger.

## Status rule (canonical)

```text
grey   : good == 0 && broken == 0
red    : broken >= good            (and not all-zero)   — failure-leaning on ties
green  : good   >= 2 && good > broken
amber  : good   == 1 && broken == 0
```

`good`/`broken` = counts of **distinct** testers whose **latest** verdict on the
feature is good/broken. Two goods overcome one stale broken (`2 > 1` → green), so
the board self-corrects as testers pile on.

## Importance ordering

`rank_items` sorts by *(number of downstream items the feature `blocks`, desc)*
then *curated `priority`, desc* then `ref`. So WorldEdit / schematic-import /
build-guide (which the building family leans on) sort above leaf tweaks like
narration. Data-driven in `assets/test_board.json` — append a row when a feature
ships, remove it once green.

## Solo boundary → playtest gate

**Solo (done, tested headless):** the status rule, importance ordering, registry
parse, verdict serialisation (Rust); the distinct-tester tally + build filter +
PII-free output (Node); both targets build; `check.sh` green.

**Playtest / owner (needs a display + multiple real testers + the reader
running):** the column's *feel*; a live verdict round-trip (DM → ledger →
`test-board.mjs` → served JSON → dots); confirming green needs two real npubs.
This mirrors the existing lobby-mailbox boundary
([2026-06-07-lobby-mailbox-feedback.md](2026-06-07-lobby-mailbox-feedback.md)).

**Ops:** run `node read.mjs --once` then `node test-board.mjs` (or on a timer) to
refresh dots. Point `TEST_BOARD_STATUS` (site) + the runner `--out` at one shared
path in production.

## Deferred (named)

- **Per-deploy auto-clear of red.** Verdicts already carry `build` (currently the
  crate version) and `tallyVerdicts` accepts a `--build` filter, but there's no
  per-deploy build id yet (no `build.rs`/git-sha). Until then red clears when a
  tester **re-tests good** (or 2 goods outweigh it), not automatically on
  redeploy. Add a git-sha build id → pass `--build` in the deploy → full
  self-clear.
- **Mobile / narrow surfacing.** The column shows on wide screens (≥ 900 px); a
  tab/drawer for phones is deferred.
- **Live-served registry.** The registry is compiled in (`include_str!`) so it
  renders offline; a relay/served overlay would let us add features without an
  engine redeploy.
- **Anti-sybil.** "Distinct" = distinct signed-in npub; burner npubs could pad a
  count. Acceptable for alpha; revisit if it's abused.
- **Per-feature dedicated test sheets** beyond the shared mega-test sheet, and a
  "history / who tested" view for the makers.

## Spec maintenance

This doc is the spec of record. Cross-referenced from the lobby-mailbox feedback
foundation (shared transport). The Phase-1 registry + the mega-test sheet
(`docs/test-sheets/2026-06-17-community-features-mega-test.md`) are the live data.
