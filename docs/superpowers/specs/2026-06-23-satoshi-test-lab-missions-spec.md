# Satoshi's Test Lab — in-world missions that turn the test sheet into play

**Date:** 2026-06-23
**Status:** BUILT (P0–P2 + P4) 2026-06-23 — owner said "build it". Discovery: the
verdict transport (`test_board::Verdict`/`queue_verdict`) and the lobby test
list (Community Test Board) **already existed**, so the build reduced to the
Test Lab world + Satoshi's in-world mission dialogue reusing them. P3
(auto-detect of the tested action) deferred — self-report is the floor and
covers every mission. See §8 for what shipped.
**Owner ask:** "I need test mode, where Satoshi asks you to test a specific
thing. We can still use `/bug` to report it works, or the test list in the
lobby. Axo is not testing, as it is boring."

Related: [[project_guided_onboarding_satoshi]] (Satoshi guide),
`2026-06-23-satoshi-onboarding-v2-design.md`, the daily build→test workflow
(`docs/workflow/daily-build-test.md`), the lobby mailbox feedback path
(`docs/foundations/2026-06-07-lobby-mailbox-feedback.md`), and the
relay-retention runbook ([[project_feedback_relay_retention_runbook]]).

---

## 1. The problem, stated honestly

The current test loop is a **markdown test sheet a human reads and works
through**. Axolottle finds it boring — and per our own research that's a real
signal, not something to paper over ([[feedback_axolittle_coerced_not_fun_signal]]):
a coerced tester is a *bug oracle*, not a *fun oracle*. So this feature is **not**
"trick a bored kid into testing." It is: **make a voluntary tester's session
both more playful and structured**, so the feedback we get is actionable and the
chore-feel is removed. Target audience = the voluntary, non-family testers the
owner has lined up ([[project_platform_priority_pc_tablet_gamepad_parked]]).

The core move: **the test sheet becomes a list of in-world missions, delivered
by Satoshi, where doing the test IS playing.** Reuse what exists — Satoshi (the
charming guide), the quest system (objective → completion → reward), and the
lobby `/bug` mailbox (report channel). Mostly wiring + content, not new engine.

## 2. Decisions locked (owner, 2026-06-23)

1. **Entry = a dedicated "Test Lab" world type.** A special world pre-loaded
   with the features under test, with Satoshi running the mission list. Cleanest
   isolation; normal worlds are never pestered with missions.
2. **Reporting = reuse `/bug`, tagged.** A mission report is a structured `/bug`
   carrying `{mission_id, verdict, build_version, world_type, optional note}`.
   **Caveat to honour (not defer):** the lobby mailbox can silently drop messages
   unless the live feedback-reader is subscribed at sync time
   ([[project_feedback_relay_retention_runbook]]). So a test session needs
   `tools/feedback-reader/live.mjs` running, OR we add a local on-disk copy as a
   fast follow. Flag this in the build goal's "owner boundary."

## 3. The reframe (load-bearing)

Never surface the word "test" to the player — *that* is the boring homework.
Satoshi **asks for help**: *"I just finished a new kind of door — would you try
one and tell me if it opens right?"* Same principles as onboarding:

- **Pull-first + always skippable** ("not now" is always valid).
- **Minimise reading** — one warm sentence + a clear tiny goal.
- **The mission IS the fun thing** — go build/fight/plant/lay-a-schematic; the
  "test" is invisible.
- **No "TEST 3/12 COMPLETE" UI.** Progress is Satoshi being pleased, not a
  checklist bar.

## 4. The Test Lab world

- New `world_type = "testlab"`, offered in New World as **"🧪 Help Satoshi
  (Test Lab)"**.
- **Generation: normal terrain** (most features under test — terrain integrity,
  mobs, build-along, schematics — need real ground), with Satoshi spawned near
  spawn in **mission mode**, and a **starter kit** handed over so the tester is
  never blocked grinding for materials before a mission.
  - *Alternative considered:* flat/curated arena. Rejected as the default
    because it can't exercise terrain/worldgen/mob missions. A mission may still
    teleport the player to a curated spot if it needs one (see auto-stage, §5).
- `satoshi_enabled` is forced on; Satoshi's **mission corpus** replaces the
  onboarding corpus in this world type.
- Day/weather/mob settings: default normal, but a mission can request a specific
  condition (e.g. "make it rain") via the existing commands.

## 5. Mission data model (data-driven)

Missions are authored per build in a `missions.json` (the test sheet, in data).
This is the **single source** rendered both in-world (Satoshi) and in the lobby
test list — keep them in sync from one file.

```jsonc
{
  "build_version": "2026-06-23",          // stamped into every report
  "missions": [
    {
      "id": "void-floor",                  // stable; goes in the /bug report
      "title": "Solid ground",
      "satoshi_line": "I patched the ground. Walk about and dig down a bit — does it hold, or do you fall through?",
      "hint": "Walk ~50 blocks and dig straight down.",
      "auto_detect": { "kind": "moved_distance", "value": 50 },  // optional; null = self-report only
      "reward": "thanks"                  // intrinsic; never sats/materials
    }
  ]
}
```

- **Completion = self-report by default** (universal, cheap): after the player
  has had a go (or taps "I tried it"), Satoshi asks **"Did it work?"** → 👍 / 👎
  (+ optional one-line note via the chat field).
- **Auto-detect where cheap** (magical, removes a tap): hook the action the
  mission is about (placed block X, opened inventory, laid a schematic, etc.).
  When detected, Satoshi notices ("you've got one in — how'd it feel?") and goes
  straight to the verdict prompt. Auto-detect is *additive*; self-report is the
  floor.
- **Reward = Satoshi's gratitude + a small cosmetic/progress nudge, NEVER sats or
  materials** (matches no-material-reward-in-Creative + [[feedback_pop_educational_not_earning_now]]).
- **Per-world progress** persisted (which missions attempted/verdicted) so a
  tester can resume; append-only `WorldSave` field, like Satoshi's state.

## 6. Reporting (tagged /bug)

On verdict, fire a `/bug` through the lobby mailbox with a structured payload:

```
[test-report] build=2026-06-23 mission=void-floor verdict=pass
note: "dug to bedrock, no holes"
world=testlab npub=<signed-in> 
```

- Reuse the existing `/bug` NIP-17 path + `tools/feedback-reader/` triage — just
  a recognised prefix/tag so reports sort into a "test reports" view.
- 👎 (fail) reports are the valuable ones; 👍 still logged (confirms coverage).
- **Reliability:** run `tools/feedback-reader/live.mjs` during test sessions, or
  add a local on-disk mirror (fast follow) so a relay miss can't lose a report.

## 7. Lobby test list (kept)

The lobby/docs site renders the same `missions.json` as a human-readable test
list (what the owner already wanted to keep). One file → two surfaces. A tester
can glance at the lobby for the full list; Satoshi walks them through it in-world.

## 8. Phased build plan — what shipped

- **P0 — Test Lab world type. ✓ BUILT.** `world_type="testlab"`, New World
  "🧪 Test Lab" entry, normal terrain (falls through `generate_column` to the
  normal path), `satoshi_enabled` forced on, and a generous starter kit (blocks
  + iron tools + bread + the starter-hut Plan) so a tester is never blocked.
- **P1 — Mission engine + Satoshi dialogue. ✓ BUILT.** Reuses the existing
  `test_board` registry (`assets/test_board.json`) as the mission list — no new
  `missions.json`. `villager_ui::draw_satoshi_mission` shows one mission at a
  time (title + "what to test") with ✓/✗/Skip/Close + an optional note;
  pull-first + skippable; session-only `mission_idx` on `GameState` (a verdict
  hits the mailbox immediately, so no persistence needed). A HUD banner keeps
  "what to test" visible after the dialogue closes.
- **P2 — Tagged /bug report. ✓ BUILT (transport pre-existed).**
  `test_board::queue_verdict` (now `pub`, shared with the lobby) sends a
  `"test-verdict"` DM `{ref, verdict, note, build}` via the lobby mailbox.
- **P4 — Lobby test list. ✓ ALREADY EXISTED** (Community Test Board). The new
  build's features were appended to `assets/test_board.json` (one source → both
  in-world missions + lobby list); test sheet
  `docs/test-sheets/2026-06-23-test-lab-and-fixes.md`.
- **P3 — Auto-detect hooks. DEFERRED.** Self-report is the floor and covers
  every mission; auto-detect ("Satoshi noticed you placed one!") is an additive
  polish for cheap cases — add when a mission clearly benefits.
- **Fast follow (reliability):** local on-disk report mirror so a verdict
  survives a relay miss (the relay-retention gotcha). Still open.

## 9. Open questions

1. **Starter kit contents** per Test Lab session — generic (blocks + tools) or
   mission-driven (only what the current missions need)? Default: generous
   generic kit so nothing blocks a mission.
2. **Mission ordering** — fixed sequence, or Satoshi offers a menu? Default:
   sequence (pull-first), with "skip this one" always available.
3. **Who authors `missions.json`** each build — Claude generates it from the
   build's test sheet automatically (ties into daily-build-test), confirmed by
   the owner. Default: Claude drafts, owner trims.

## 10. The honest guardrail

If a mission can only land as a chore, don't ship it — cut it or make the
underlying feature more fun first. Missions structure *voluntary* testers'
feedback; they are not a lever to make an unfun thing feel fun. "Axo's bored"
remains a signal we listen to, not one we engineer around.
