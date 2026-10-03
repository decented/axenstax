# Feature-coverage challenges — tiered, in-game, scenario-driven

**Status**: FUNCTIONALLY COMPLETE IN-GAME — Phases 2, 3, 4, 5(logic), **6**, **7**
DELIVERED to `main`. The onboarding arc + every explorer card progress
end-to-end (all 9 completion hooks fire); **Phase 7 (feedback anchor) shipped
2026-06-18** (`d298c65`). **Phase 6 — the challenge board — BUILT 2026-06-19**
(goal `2026-06-19-solo-buildout-wave-2`, Wave 6): press **J** for a board listing
every bundled challenge (onboarding + explorer cards), Start one with a click
(routes through the same `GameState::start_scenario` as `/scenario`), and see the
active challenge's progress (`ScenarioState::progress_summary` → X/Y leaves +
elapsed). `challenge_board_ui.rs` + the J toggle + modal gating (mirrors the map /
wardrobe). The chat `/scenario list` stays as the equivalent text path. **Launch routing (audit fix 2026-09-28):** a scenario that clears the inventory (every non-Challenge kind, and a `Timed` Challenge such as Scavenger) or carries a kit (most explorer cards) never runs in the player's current world — the J board and `/scenario` save and leave (`leave_world`), then launch it into its arena through the Trials menu's own `PlayScenario` path (`world_exit::challenge_launch_mode`). Before, Scavenger wiped real gear in-world and kit Challenges were a repeatable free-items tap. Only a Challenge with no kit and no clear overlays the current world. In a hosted or joined session an arena launch is refused with a message. Remaining:
**Phase 8** (Axolittle playtest gate — the board's *feel* + completion surfacing;
test sheet `docs/test-sheets/2026-06-18-feature-coverage-challenges.md`,
Test Board ref `TB-49-challenges`).
**Date**: 2026-06-06 (drafted) · 2026-06-14 (built)
**Branch (when built)**: built directly on `main` (`0eab0df`, `917a388`, `e255a4a`, `3bc0a39`).
**Owner decisions captured**: tiered (fun onboarding arc + optional coverage "explorer" set); delivered **in-game**, authored as `ScenarioDef` data on the existing scenario runner.

---

## TL;DR

We have two *game* challenges (Hash Dash, Satori Rush) but no mechanism that **steers an alpha tester to exercise a specific feature**, so feature coverage during playtests is incidental. This spec adds a **challenge pack**: small objective-driven prompts authored as scenario data that walk a tester through the alpha feature surface, tied to the `/bug`–`/idea` loop so feedback is anchored to "what they were trying to do."

The pack is **tiered**:

1. **Onboarding arc** — a short, fun, *sequenced* "first session" (move → mine → craft → cook → build) that happens to cover the core loop. Goal: delight + retention; coverage is a side effect.
2. **Explorer set** — an optional, *unordered* board of single-feature challenges (tame a wolf, reskin a block in the Workshop, sell at a Vendor, claim a Plot, …). Goal: intentional coverage — no shipped feature goes unplaytested.

The engineering core is **not** the content — it's that the scenario runner's objective vocabulary is currently too small. Today `Objective` has exactly two variants (`Timed`, `FirstSatori`) and two completion hooks (`on_block_broken`, `on_material_gained`). A coverage challenge needs *action* objectives ("tame a wolf", "publish a Workshop reskin") and *composite* objectives ("plant, then cook, then build"). This spec extends the objective model and wires new completion hooks into the relevant subsystems — both of which lift cross-game (a challenge is just declarative data, no AxeNStax coupling).

---

## Why this lives here

- Per launch scope axenstax only + alpha launch posture: AxeNStax is live (5 Jun) and the alpha is actively being tested — directed coverage is the missing half of the feedback loop.
- Per pretest check: playtests surface regressions, but only for features the tester happens to touch. Challenges make coverage deliberate.
- Builds directly on **Goal 1's scenario runner** (`scenario.rs`, PR #109) — challenges are `ScenarioDef`s, the same artifact format Hash Dash / Satori Rush already use.
- Pairs with the **`/bug`–`/idea` feedback loop** (`docs/foundations/2026-05-28-alpha-feedback-loop.md`): the active challenge id becomes a `client_intent`-style soft hint on every submission, so a bug report is automatically anchored to the feature under test.
- Per shared infra strategy: the objective vocabulary + hooks are game-agnostic primitives. Any game on the engine authors its own pack; nothing here hard-codes AxeNStax.
- Per uk english naming: UK English throughout.

---

## The core gap (grounded in the real seam)

`game/engine/src/scenario.rs` today:

```rust
pub enum Objective {
    Timed { ticks: u32 },   // Hash Dash
    FirstSatori,            // Satori Rush
}
```

Completion is driven by exactly two hooks on `ScenarioState`:
- `on_block_broken(work: u64)` — called from the break path.
- `on_material_gained(material: MaterialId)` — arms `FirstSatori`.

That's enough for "mine for time" and "get a Satori", but a coverage challenge needs to observe events that **no hook currently surfaces**: a wolf was tamed, a Vendor sale completed, a Workshop reskin was published, an item was crafted, a specific block was placed. So the work is two-sided:

1. **Richer objectives** (the *what-counts* vocabulary).
2. **More completion hooks** at the right subsystem call sites (the *when-it-fires* events).

Both are additive and serde-default-safe — every existing `ScenarioDef` (Hash Dash, Satori Rush) keeps deserialising unchanged.

---

## Objective model extension

Add **action** and **composite** objective variants. Sketch (final shape decided in build, kept minimal):

```rust
pub enum Objective {
    Timed { ticks: u32 },
    FirstSatori,

    // NEW — count a tagged action N times.
    Action { event: ChallengeEvent, count: u32 },

    // NEW — an ordered arc (onboarding): each step completes before the next
    // is shown. The challenge ends when the last step completes.
    Sequence { steps: Vec<Objective> },

    // NEW — an unordered checklist (explorer multi-part): all must complete,
    // any order. (Often a single Action; Checklist is for "do A and B".)
    Checklist { items: Vec<Objective> },
}

/// Hookable, feature-tagged completion events. Each maps to a built subsystem.
pub enum ChallengeEvent {
    BreakBlock { block: Option<BlockTag> },   // any / specific
    PlaceBlock { block: Option<BlockTag> },
    CraftItem  { item: Option<ItemTag> },
    CookAtCampfire,
    TameMob    { kind: Option<MobTag> },      // wolf for alpha
    VendorSale,                               // a Vendor purchase settled
    WorkshopPublish,                          // a reskin/reshape published
    ClaimPlot,
    GainMaterial { material: MaterialId },    // generalises FirstSatori
}
```

`Sequence`/`Checklist` make `FirstSatori` and `Timed` composable too, but they stay as-is for back-compat.

`ScenarioState` gains a per-objective progress vector (current counts / which steps done) so the HUD can render "2 / 3". `is_resumable()` extends to "any challenge with progress worth persisting" (the onboarding arc should survive a reload); progress rides on `WorldMeta` alongside the existing `total_work`/`genesis` stats, same pattern.

---

## Completion hooks to add (event → call site)

Each new `ChallengeEvent` needs one call into `ScenarioState::on_event(...)` at the subsystem where the action *completes* (not where it's attempted). Verified-built modules:

| Event | Fires at | Module |
|---|---|---|
| `BreakBlock` | already hooked (`on_block_broken`) — extend with the block tag | break path |
| `PlaceBlock` | block-place commit | `world` / placement path |
| `CraftItem` | successful craft commit | `crafting.rs` |
| `CookAtCampfire` | campfire output taken | `campfire.rs` |
| `TameMob` | taming success transition | `wolf.rs` (tameable-mob seam) |
| `VendorSale` | `vendor::try_buy` returns success | `vendor.rs` |
| `WorkshopPublish` | publish driver success | `workshop` + `/ws publish` path |
| `ClaimPlot` | plot anchored | `plot.rs` |
| `GainMaterial` | already hooked (`on_material_gained`) | inventory gain |

Each hook is a one-line dispatch guarded by "is a scenario active and does it care about this event" — zero cost when no challenge is running. Farming is **intentionally excluded** until a `farming.rs`-level feature is confirmed shipped (no top-level module exists as of 2026-06-06); add `PlantCrop`/`HarvestCrop` events when it lands.

---

## Content — the two tiers (authored as JSON in `assets/challenges/`)

### Tier 1 — onboarding arc (`onboarding.json`, `Objective::Sequence`)
A guided first ~15 minutes. Each step is a single `Action`; the HUD shows only the current step + a "Challenge complete!" on the last.

1. Break 5 blocks → 2. Craft a tool → 3. Place 10 blocks (build something) → 4. Cook at a campfire → 5. (capstone) mine an ore.

Fun-first: framed as a friendly "let's get started", not a QA list. Skippable from the menu.

### Tier 2 — explorer set (`explorer/*.json`, one `Action`/`Checklist` each)
An unordered board of single-feature challenges, each mapped to a built feature so a tester can be pointed at exactly the surface we need data on:

- **Tame a wolf** (`wolf.rs`)
- **Reskin a block in the Workshop and publish it** (`Checklist`: WorkshopPublish)
- **Sell something at a Vendor** (`vendor.rs`)
- **Claim a Plot** (`plot.rs`)
- **Cook three different foods** (`Checklist` of `CookAtCampfire ×3`)
- **Trade with a villager / earn reputation** (`village` + `reputation.rs`)
- **Build with a blueprint / Drafting Stamp** (`blueprint` capture)

Each card names the feature and gives a one-line "why we want you to try this." Balance/feel is a **playtest boundary** — counts and copy are tuned with Axolittle, not solo.

---

## Delivery — in-game

- **Objective HUD**: extend the existing scenario HUD (timer/score today) to render checklist/sequence progress ("Tame a wolf — 0 / 1", "Step 2 of 5"). Reuses the runner's per-frame draw.
- **Onboarding trigger (owner decision, 2026-06-15)**: the onboarding arc is **NOT** an unconditional auto-start, and login never dives past the lobby into a world — the lobby stays the landing screen. The arc is **offered only to a genuine first-timer**: a player with **no local saved worlds AND no Stash worlds** (no evidence they've played before). Most people who log in already have a Stash / saved worlds → they just see their normal lobby, no prompt. A true newcomer gets a **non-forcing popup** ("New here? Want the guided start?") with a skip. The trigger predicate is built + unit-tested: `scenario::should_offer_onboarding(local_world_count, stash_world_count)` (pure; the lobby passes its real counts). The popup UI + wiring is the Phase 6 (app-verified) part.
- **Challenge board**: a menu section ("Challenges") listing the onboarding arc + explorer cards; clicking one starts that `ScenarioDef` in the current world (no fresh-world needed — these are *do-an-action* objectives, not fixed-arena score-attacks, so `arena_seed` stays `None`). This is the same create-or-start lifecycle the deferred Goal 5 "Official Challenges" menu needs — **build them together**; this spec subsumes that bounded gap.
- **Feedback anchor**: while a challenge is active, `/bug` and `/idea` submissions attach the active challenge id as context (extends the `client_intent` hint in `2026-05-28-alpha-feedback-loop.md`) so the admin board shows "reported while attempting: Tame a wolf."

---

## Phases (each solo-verifiable, `check.sh`-green before the next)

| # | Phase | Autonomous? | State (2026-06-14) |
|---|---|:--:|---|
| 1 | This spec | ✓ | ✅ |
| 2 | Objective model: `Action`/`Sequence`/`Checklist` + `ChallengeEvent` + progress state + serde-default back-compat + unit tests | ✓ | ✅ **DELIVERED** (`0eab0df`) — 14 tests; `Objective` lost `Copy`; `ScenarioKind::Challenge` added |
| 3 | Completion hooks wired at each verified call site (place/craft/cook/tame/vendor/publish/plot) + per-hook tests | ✓ | ✅ **DELIVERED** (`e255a4a` + `3bc0a39`) — all 9 events fire at their real commit sites: BreakBlock (both paths), GainMaterial, PlaceBlock, CraftItem (guarded on a real craft), CookAtCampfire, TameMob, VendorSale (both buy paths), WorkshopPublish, ClaimPlot. Additive + guarded by `scenario.is_some()`. Firing correctness = playtest (Phase 8); on_event LOGIC unit-tested (Phase 2) |
| 4 | Content: `onboarding.json` + explorer pack, embedded like the existing scenarios | ✓ | ✅ **DELIVERED** (`917a388`) — onboarding arc (5-step Sequence) + 5 explorer cards; `challenge_pack()` + `named_builtin_def` |
| 5 | HUD: sequence/checklist progress render | ✓ (logic) / playtest (feel) | ✅ **logic DELIVERED** (`0eab0df`) — "name — done / total" + "complete!"; feel = playtest |
| 6 | Challenge board menu + create-or-start lifecycle (closes the Goal 5 menu gap) — **needs the running app to verify** | supervised | ☐ remaining — *interim discovery shipped*: `/scenario list` (`d298c65`, `scenario::challenge_listing()`) lists every challenge + start command in chat; the GUI board is the remaining app-supervised piece |
| 7 | Feedback anchor: active-challenge id on `/bug`–`/idea` | ✓ | ✅ **DELIVERED** (`d298c65`) — `ChallengeSnap {name, done, total}` on the `wasm_feedback` Snapshot (the blob already attached to every report); rides the existing JSON, no JS change. Unit-tested (serialises when a challenge runs, omitted otherwise) |
| 8 | Test sheet for Axolittle + the playtest gate (copy, counts, fun) | gate | ☐ gate — **test sheet written** (`docs/test-sheets/2026-06-18-feature-coverage-challenges.md`, `TB-49-challenges` on the lobby board); the play itself is the gate |

Phases 2–4 + 7 are fully solo-verifiable (data + unit tests). Phase 6 is the one ordering-sensitive piece that wants a live app (same reason Goal 5's menu was bounded). Phase 8 is the feel gate.

---

## Hook coverage boundaries (as built — for challenge authors)

The Phase 3 hooks (`3bc0a39`, refined post-review) fire at specific commit sites.
Authoring an objective outside these boundaries yields an *unreachable* challenge:

- **`GainMaterial` fires only for the gem/Satori drop path**, NOT ordinary
  `mine_drop`/`bonus_mine_drop` materials (Salt, Flint, …). `GainMaterial { Coal }`
  works (coal is a gem-path drop / the onboarding capstone); `GainMaterial { Salt }`
  would never complete. (Matches the spec's "generalises FirstSatori".)
- **`PlaceBlock` fires for EVERY placement through `place_player_block`**, including
  special blocks — placing a Vendor Block fires `PlaceBlock { VENDOR_BLOCK }`, a Plot
  Marker fires both `PlaceBlock { PLOT_MARKER }` *and* `ClaimPlot`. A reverted
  plot-marker placement (foreign-plot conflict) is correctly NOT counted (review fix).
  Plant/mount commits (crop, papyrus, cyanotype) go through `place_player_block` too
  but are deliberately NOT hooked — a "place any block" challenge won't progress on those.
- **`CookAtCampfire`** counts the empty-hand pickup of a cooked item off the fire
  (success branch only — an inventory-full pickup does not count).
- **`CraftItem`** counts only a real craft (guarded on `click_result() == true`), not
  an empty result-slot click.
- **`WorkshopPublish`** fires when the player confirms a share (the `PublishOverrideSet`
  command) on BOTH targets — so on native it completes even though publishing is
  web-only (counts the player's confirmed action, not the network outcome).
- **`VendorSale`** fires once per settled purchase (both the `try_buy` and
  `try_buy_plan` paths; exactly one fires per buy — no double-count).

Firing correctness is verified by code-reading + the Phase 8 playtest (the hook sites
live in the client game-loop and aren't reachable by the unit-test harness); the
`on_event` LOGIC is fully unit-tested (Phase 2).

---

## Out of scope (deferred — not bugs)

- Rewards/economy for completing challenges (cosmetics, Satori) — a separate design once the loop is proven.
- Networked / co-op challenges (multiplayer fleet is deferred per alpha launch posture).
- Farming challenges — until a shipped farming feature exists to hook.
- Community-authored challenge packs over Beacon/open-stash — structurally supported (it's just `ScenarioDef` JSON, same as Official Challenges) but not wired here.
- Per-tester progress analytics dashboard.

---

## Risks

| # | Risk | Mitigation |
|---|---|---|
| 1 | Objective enum churn breaks the two shipped scenarios | All new variants additive + `#[serde(default)]`; round-trip test asserts Hash Dash / Satori Rush JSON still loads byte-identical. |
| 2 | A hook fires at the wrong moment (attempt vs completion) | Hook at the success/commit site only; unit-test each with a fake event. |
| 3 | Onboarding arc feels like homework | Fun-first copy; skippable; Phase 8 feel gate owns it. |
| 4 | Challenge board create-or-start lifecycle is the same multi-frame hazard that bounded Goal 5 | Build supervised (Phase 6), not in an unattended run. |
| 5 | Scope creep into a full quest system | Hard line: challenges are stateless `ScenarioDef`s with an objective + completion. No branching, no NPC dialogue, no rewards in v1. |

---

## Relationship to existing work

- **Extends**: `scenario.rs` (Goal 1) — same artifact, richer objectives.
- **Subsumes**: the deferred Goal 5 "Official Challenges" in-game menu render (the create-world→start-scenario lifecycle) — build the board once, here.
- **Pairs with**: `2026-05-28-alpha-feedback-loop.md` (`/bug`–`/idea`) — challenges give the feedback a subject.
- **Distinct from**: the `docs/test-sheets/` checklists, which stay as *our* supervised QA scripts; this is the *tester-facing*, in-game counterpart.
