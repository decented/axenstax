# Satoshi Onboarding — Buildable Foundation Spec

**Turn the Satoshi *design* into concrete, ordered engine tasks a supervised coding session can execute fast — leverage, not new engine.**

- **Date:** 2026-06-22
- **Status:** 🟢 **READY TO BUILD** — foundation spec. Derived from the design (`docs/superpowers/specs/2026-06-22-guided-onboarding-satoshi-design.md`) and the evidence base (`docs/research/2026-06-22-guided-onboarding-precedents.md`).
- **Posture:** Concrete, data-driven, append-only-save-safe. Reuses the **villager + dialogue + quest + challenge-board** assets already in the engine. No live AI. Minimal new systems.
- **Companions:** `docs/vision/genesis-founding-myth-long-run.md` (Satoshi as founder figure — fold lore in *later*, not in the MVP).

---

## 0. Compliance guardrails (ABSOLUTE — embed in every phase)

These are not aspirational; they are acceptance gates. A build that violates any of these is **not done**.

1. **Kid-safe, fully scripted.** Every line Satoshi speaks comes from an authored, reviewed data table (`SATOSHI_DIALOGUE` / `SATOSHI_NUDGES`). **No live AI, no network call, no free-text input.** The AI-driven Satoshi is **deferred / do-not-build** (§9).
2. **Never push Bitcoin or earning.** Satoshi must not mention sats, earning, payouts, wallets, or rewards-as-money. His currency is **fun and care**. The welcome gift is framed as *a kindness* ("here, this'll keep you going"), never as a payout. A unit test asserts the dialogue/nudge corpus contains none of a banned-substring set (`sat`, `bitcoin`, `earn`, `payout`, `wallet`, `money`, `reward` — case-insensitive, word-boundary).
3. **Defer Bitcoin to a parent.** If a player ever asks Satoshi about Bitcoin/money (a future branch — NOT in MVP), the only scripted answer defers to a guardian. MVP simply has no such branch. Document the rule so the later branch can't drift.
4. **Never over-claim safety.** No "you're safe here", "nothing can hurt you", or protective guarantees. Warmth without false promises.
5. **Lead with fun / sovereignty.** The nudges point at *play* (meet the animals, grow something, explore) — autonomy, competence, relatedness (SDT). Extrinsic reward is never the hook.
6. **UK English** throughout (authored strings, comments, doc).
7. **Always dismissable, never seizes input.** Single-input dismissal everywhere; Satoshi never locks movement or grabs the camera (§ anti-Navi/Fi guardrails).

---

## 1. Goal & MVP scope

**The smallest shippable Satoshi:** a special, named villager who

- appears **once** near the player's first-session spawn and **lives in a findable house** (so guidance is pull-on-demand: knock when *you* want him);
- on first talk, **greets warmly and offers a "hungry?" gift** (a little food), framed as care;
- offers a **2–3 option nudge** ("meet the animals", "try growing something", "I'm good, I'll explore") — each a tiny, optional, concrete goal;
- on completing a tiny goal, hands a **welcome gift** (seeds / a tool) as a kindness — **no "tutorial complete" pop-up**;
- **demonstrates ONE activity beside the player** (co-presence: he tills/plants nearby, you watch, you try) — **never seizing input**;
- is **fully dismissable** at every step and **goes quiet once waved off** (frequency budget);
- **persists** "has greeted / has gifted / nudge done" per-world so he doesn't repeat himself across reloads.

**What it reuses (verified in code, §4):** the Villager mob + `VillagerComponent`, the `draw_villager_dialogue` overlay, the `quest.rs` data model for the tiny goal + gift, `village_gen` house/spawn placement, and the append-only `WorldSave` persistence convention. The only genuinely new pieces are: a Satoshi **identity marker** (component), a small **dialogue/nudge data corpus**, a tiny **state machine** (greeted → gifted → nudged → demo'd), and **one persistence field**.

**Out of MVP:** richer branching, deep founding-myth lore, multiple activities, "show the locks" teases, voiced lines, multiplayer sync (single-player / local split-screen only — matches `quest.rs`'s `accepted_by: u32` slot-index identity).

---

## 2. Phases (ordered, each independently shippable + verifiable)

Each phase compiles, tests green, and is mergeable on its own. **Solo-verifiable** = check.sh + a manual single-player run by the builder. **Playtest-gated** = needs the voluntary-player fun test (un-coerced kid).

### P0 — Satoshi identity + spawn + house (solo-verifiable)
- Add a **`SatoshiMarker`** ECS component (zero-size or a tiny struct) so exactly one villager in a world is Satoshi. Do **NOT** add a new `MobType` variant — Satoshi is a `MobType::Villager` *plus* a marker. This keeps him on every existing villager code path (dialogue, model, anti-grief, find-attack-target) for free and avoids touching `mob_def`/`entity_model`/the positional `MobType` bincode order.
- **Spawn rule:** on first entry to a fresh world (single-player), place **one Satoshi** at a deterministic, findable spot near the player's spawn. Two viable hosts (pick in build, lean toward A):
  - **A (preferred, lowest risk):** spawn a small dedicated "Satoshi's house" near the world spawn point (reuse `village_gen`'s `build_hardcoded_house` shape + a door + a bed), and spawn the Satoshi villager inside it. A signpost/torch marks it.
  - **B:** if the spawn lands near a procedurally-placed village (`world.village_anchors`), promote one of that village's villagers to Satoshi instead of building a house. More "diegetic" but depends on a village being near spawn — flag as a follow-up, not MVP.
- Attach `SatoshiMarker` + `VillagerComponent` (already auto-attached for `MobType::Villager` in `entity::spawn_mob`). Give him `Profession::None` so he never claims a workstation and never reads as "Farmer".
- **Verify:** unit test that exactly one entity carries `SatoshiMarker` after the spawn pass; manual run shows Satoshi standing in a findable house near spawn.

### P1 — Greet + "hungry?" gift (solo-verifiable)
- When the player right-clicks Satoshi (existing dialogue trigger path, `game_loop.rs:~7981`), branch on `SatoshiMarker` **before** the generic villager-dialogue branch (mirror the existing Builder special-case at `game_loop.rs:~8014`). Open a **Satoshi dialogue** instead of the quest dialogue.
- First-ever talk (`!world.satoshi.has_greeted`): show the warm greeting line + a **single "Thanks!" gift action**. Accepting gives a small food stack (e.g. `ItemStack::new_material(MaterialId::Bread, 2)` or cooked beef) via `inventory.add_item`, fires the existing toast + pickup juice, and sets `has_greeted = true` + `has_gifted = true`.
- A **"Thanks, I'm good — I'll explore"** action is always present and dismisses on one click (records `waved_off` for the frequency budget, §P4).
- **Verify:** unit test on the gift helper (inventory gains the food once, not twice on re-talk); manual run shows greeting → gift → toast.

### P2 — The 2–3 option nudge → tiny goal → welcome gift (solo-verifiable for the loop; charm is playtest-gated)
- After the greeting (or on a later talk), show a **nudge menu** of 2–3 authored options drawn from `SATOSHI_NUDGES` (data-driven). Each option maps to a tiny goal expressed as an **existing `quest::Quest`** (reuse the data model — do **not** invent a parallel goal type):
  - "Meet the animals" → a trivial **`QuestFlavour::Fetch`/proximity-style** goal the world already supports, OR (simplest) a no-op "go say hello" that completes on the next talk. Keep it tiny and unmissable.
  - "Try growing something" → e.g. plant a seed / harvest 1 crop, expressed as a `Fetch { item: <crop>, count: 1 }` against obtainable materials (follow `quest.rs`'s "obtainable-materials-only" rule — see `hp7_quest_pools_use_obtainable_materials_only`).
  - "I'll explore" → dismiss (no goal).
- On completion (checked with the **existing** `quest::is_complete` / `may_turn_in` gate — reuse, don't reimplement), hand a **welcome gift** (`QuestReward.items`, e.g. a few seeds or a wooden tool). **Sats = 0 always** for Satoshi (compliance). **No "tutorial complete" screen** — just the gift + a warm line + the existing reward juice.
- Reuse the existing accept/turn-in plumbing in `game_loop.rs` (`active_quests`, `kill_quest_baseline`, the re-check-before-pay gate) so Satoshi's goal can't drift from the villager-quest path.
- **Verify (solo):** the nudge options render, selecting one creates the goal, completing it pays the gift exactly once (re-check gate holds — a dropped-item exploit must not pay). **Playtest-gated:** do the options read as *inviting* not *homework* to a kid.

### P3 — Co-presence demo of ONE activity (solo-verifiable for mechanics; "feels collaborative" is THE playtest question)
- When the player picks "try growing something", Satoshi **demonstrates beside them**: drive his `MobAi` to walk to a nearby tilled patch and place a seed/till a block himself (a scripted one-shot, NOT a new AI behaviour tree). Implement as a tiny **`AiState::SatoshiDemo { step, target }`** added to `mob_ai.rs`'s `AiState` enum (it's already an open enum with `Wander`/`GolemGuard` — additive), or as an even-simpler scripted sequence driven from a `satoshi::tick_demo` free function that nudges his position/facing. **Lean toward the free-function approach** to avoid widening the AI state machine.
- **Hard guardrail:** the demo **never seizes the player's input, camera, or movement**. The player can walk away mid-demo; Satoshi finishes (or abandons) gracefully. No cutscene, no lock.
- Keep it to **one** activity for MVP (planting). The "watch, then you try" framing is conveyed by a single short line + the visible action.
- **Verify (solo):** Satoshi walks over and the demo block-change lands; player movement is provably unaffected (manual). **Playtest-gated (THE key question):** does an 11-year-old read this as *collaborative and charming*, not a cutscene or a nag? (Design §9, research open-question #4.)

### P4 — Frequency budget + dismissal + persistence (solo-verifiable)
- **Persistence:** add **one append-only field** `satoshi: SatoshiState` to `WorldSave` (§3). It carries `has_greeted`, `has_gifted`, `nudge_done`, `demo_done`, `waved_off`, and `last_initiated_tick`. Restored on load so Satoshi never re-greets or re-gifts across reloads.
- **Frequency budget (anti-Navi):** Satoshi **initiates** contact at most once, gently, and **only if not yet greeted and not waved off**. After `waved_off` he is **pull-only** — he never initiates again; the player must come to his house. A min-interval (`SATOSHI_INITIATE_COOLDOWN_TICKS`, e.g. one in-game day) guards any optional "wave as you pass once" cue so it can't nag. **Never repeat a tip the player has acted on** (gate on the persisted flags).
- **Dismissal:** every Satoshi dialogue/nudge closes on a single input (reuse `DialogueAction::Close`); closing is never penalised.
- **Verify (solo):** save → reload → talk to Satoshi: he does NOT re-greet or re-gift (flags survived). A test round-trips `SatoshiState` through the save format and asserts the post-load dialogue state. Frequency-budget unit test: once `waved_off`, the initiate check returns false.

---

## 3. Data structures (data-driven; append-only-safe persistence)

### 3.1 Identity marker (ECS component) — new, in `satoshi.rs`
```rust
/// Marks the single villager who is Satoshi in a given world. Satoshi is a
/// MobType::Villager + this marker (NOT a new MobType) so he rides every
/// existing villager code path. Profession stays None (no workstation claim).
pub struct SatoshiMarker;   // zero-size tag
```

### 3.2 Dialogue + nudge corpus — new, authored tables in `satoshi.rs`
```rust
/// One scripted Satoshi line. Authored + reviewed; NEVER generated.
pub struct SatoshiLine { pub text: &'static str }

/// Greeting / gift / farewell lines. UK English. No Bitcoin/earning words.
pub static SATOSHI_GREETING: &[&str] = &[ /* warm, care-first */ ];
pub static SATOSHI_GIFT_LINE: &[&str] = &[ /* "here, this'll keep you going" */ ];

/// One nudge option: a label + the tiny goal it creates + the welcome gift.
pub struct SatoshiNudge {
    pub label: &'static str,                 // "Meet the animals"
    pub goal: NudgeGoal,                      // → builds a quest::Quest (sats=0)
    pub gift: &'static [(MaterialId, u8)],    // welcome gift, framed as kindness
}
pub static SATOSHI_NUDGES: &[SatoshiNudge] = &[ /* 2–3 entries */ ];
```
- `NudgeGoal` is a thin authored descriptor that **builds an existing `quest::Quest`** (reusing `QuestFlavour::Fetch`/`Make`) with `reward.sats = 0`. It is **not** a parallel goal engine.
- A `#[cfg(test)]` **compliance test** scans every string in these tables for the banned-substring set (§0.2) and asserts none are present.

### 3.3 Frequency-budget + per-world state — new, persisted
```rust
/// Per-world Satoshi progress. Append-only LAST field on WorldSave.
#[derive(Default, Serialize, Deserialize, Clone)]
pub struct SatoshiState {
    pub has_greeted: bool,
    pub has_gifted: bool,
    pub nudge_done: bool,
    pub demo_done: bool,
    pub waved_off: bool,
    pub last_initiated_tick: u64,
}
pub const SATOSHI_INITIATE_COOLDOWN_TICKS: u64 = 24_000; // ~1 in-game day
```

### 3.4 Persistence slot (append-only WorldSave convention)
- Add **`#[serde(default)] pub satoshi: SatoshiState`** to `WorldSave` (`save.rs`) as **the NEWEST appended field** — it must go **AFTER** `saved_mobs` (currently the last field) in declaration/wire order. bincode is positional; pre-Satoshi saves load it via `#[serde(default)]` / `read_tail` (the established pattern documented on `saved_mobs`, `composters`, `power_devices`).
- The Satoshi *villager entity itself* is **not** serialised (he is procedural — re-spawned deterministically near spawn like `spawn_initial_villagers` does for village cohorts, gated on `!has_greeted`-style logic so we don't spawn a second one). Only `SatoshiState` persists. This mirrors how villagers are re-derived rather than saved.
- **Transient** (NOT persisted, mirror the existing `dialogue_villager` rule at `player_slot.rs:91`): which player has the Satoshi dialogue open, and any in-flight demo step.

---

## 4. Hook points / file touch-list (real names from the code)

| File | Type / fn (verified) | Change |
|------|----------------------|--------|
| **`satoshi.rs`** *(new)* | — | `SatoshiMarker`, `SatoshiState`, `SatoshiLine`, `SATOSHI_*` corpus, `NudgeGoal`→`quest::Quest` builder, `tick_demo` free fn, `should_initiate`, compliance test. One responsibility, well under 500 lines. |
| `entity.rs` | `spawn_mob(ecs, MobType::Villager, pos)` (l.347; auto-attaches `VillagerComponent` via `is_villager_kind`, l.364) | New `satoshi::spawn_satoshi(ecs, pos)` helper that spawns a Villager then `insert_one(id, SatoshiMarker)`. |
| `village_gen.rs` | `build_hardcoded_house` (l.421), `spawn_initial_villagers` (l.534) | Add `spawn_satoshi_house_and_npc(world, ecs, spawn_xz)` modelled on these — build a small house near world spawn + spawn Satoshi inside, idempotent via a `world.satoshi.*` guard. |
| `game_loop.rs` | villager right-click branch (l.~7981); the **Builder special-case** at l.~8014–8031 is the exact pattern to copy | Branch on `SatoshiMarker` before the generic `dialogue_villager` assignment; open the Satoshi dialogue path. Also: the spawn/initiate tick (call `satoshi::should_initiate` + the spawn helper on the same village-tick cadence as `spawn_initial_villagers`). |
| `game_loop.rs` | quest accept / turn-in plumbing (l.~11588–11700: `active_quests`, `kill_quest_baseline`, `may_turn_in` re-check, reward drop + juice) | Reuse verbatim for the nudge goal + welcome gift (sats forced 0). |
| `villager_ui.rs` | `draw_villager_dialogue` (l.59), `DialogueAction` (l.26), `DialogueMode` (l.37) | Add a sibling `draw_satoshi_dialogue` (or extend with a Satoshi mode) returning the same `DialogueAction` enum — warm styling, 2–3 nudge buttons + always a Close/"I'll explore". Pure UI, no state. |
| `quest.rs` | `Quest`, `QuestFlavour::{Fetch,Make}`, `QuestReward` (sats=0), `is_complete`, `may_turn_in`, `summarise` | Build the tiny goal + gift from `NudgeGoal`. **No new quest types.** Honour the obtainable-materials rule (`hp7_quest_pools_use_obtainable_materials_only`). |
| `mob_ai.rs` *(optional, P3)* | `AiState` enum (l.28, open: `Wander`/`GolemGuard`) | Either add an additive `SatoshiDemo { step, target }` variant **or** prefer driving the demo from `satoshi::tick_demo` without touching the enum (lower risk). |
| `player_slot.rs` | `dialogue_villager: Option<hecs::Entity>` (l.91, transient) | Reuse as-is to track the open Satoshi dialogue (he is a villager entity). Optionally a transient `satoshi_nudge_open: bool` if the UI needs a separate flag. |
| `save.rs` | `WorldSave` (l.66), append-only tail after `saved_mobs` (l.282); `read_tail` (l.1780) | Add `#[serde(default)] pub satoshi: SatoshiState` as the new LAST field; initialise to `default()` in every `WorldSave { .. }` constructor site (the build will hit several — `save_world`, the WASM path, tests). |
| `world.rs` | `World` struct (carries `village_anchors`, `populated_villages`, etc.) | Add a runtime `pub satoshi: SatoshiState` mirror on `World` (loaded from / written to `WorldSave.satoshi`), matching how `village_anchors`/`bounties` live on `World` and round-trip through save. |

**Not touched (important):** `MobType` enum (`mob.rs`) — no new variant, so no positional bincode change and no `mob_def`/`entity_model` registry edits. `narration.rs` is available if accessibility narration of Satoshi's lines is wanted later, but is **not** required for MVP.

---

## 5. Acceptance criteria

**Solo-verifiable (the bulk — gate every phase on these):**
- [ ] `./check.sh` green (clippy, build, `cargo test --bin axenstax-engine`, trunk WASM build, bundle-size gate). Run with `CARGO_INCREMENTAL=0` to dodge the known flaky LLVM linker.
- [ ] **Compliance test passes:** the `SATOSHI_*` corpus contains none of the banned substrings (`sat`, `bitcoin`, `earn`, `payout`, `wallet`, `money`, `reward`), case-insensitive, word-boundary. (This is the load-bearing guardrail — it must exist and pass.)
- [ ] Exactly **one** entity carries `SatoshiMarker` after the spawn pass (test), and he stands in a findable house near spawn (manual).
- [ ] **Greets once:** first talk shows the greeting + gift; the food lands in inventory exactly once; re-talk does NOT re-greet or re-gift (flags hold).
- [ ] **Nudge options work:** 2–3 options render; selecting one creates a `quest::Quest` (sats=0); "I'll explore" dismisses cleanly.
- [ ] **Gift lands once:** completing the tiny goal pays the welcome gift exactly once; the **re-check-before-pay** gate (`may_turn_in`) rejects the drop-items-after-dialogue exploit (mirror `may_turn_in_blocks_after_required_items_dropped`).
- [ ] **No "tutorial complete" screen** anywhere in the flow (manual + code review).
- [ ] **Dismissal:** every Satoshi panel closes on a single input; closing is never penalised.
- [ ] **Co-presence never seizes input:** during the P3 demo, player movement/camera are provably unaffected (manual; assert no input-lock code path is entered).
- [ ] **Persistence survives reload:** save → reload → talk → Satoshi does not repeat greeting/gift; `SatoshiState` round-trips through the save format (test). Pre-Satoshi saves still load (append-only / `read_tail` test, following the `saved_mobs`/`composters` byte-layout test pattern).
- [ ] **Frequency budget:** once `waved_off`, `should_initiate` returns false forever (pull-only); the initiate cooldown gates any pass-by cue.

**Playtest-gated (flag clearly — needs the voluntary-player fun test, an un-coerced kid):**
- [ ] **THE key question:** does the **co-presence demo feel collaborative and charming**, not a cutscene and not a nag? (Design §9, research open-Q #4.) This is the one thing the solo build *cannot* validate — Satoshi can be correct and still be annoying. Tune voice/personality, demo pacing, and the pass-by-cue frequency from this test, not from the build.
- [ ] Secondary playtest read: do the nudge labels invite (autonomy/competence/relatedness) rather than read as homework?

---

## 6. Build sequencing notes (for the supervised session)

- Build **P0 → P4 in order**; each is a clean commit + green check.sh. P0–P2 + P4 are fully solo-shippable; **P3 is where you stop and book the playtest** (the co-presence charm question).
- **Reuse before adding.** Before writing any goal logic, confirm it routes through `quest.rs`. Before adding an `AiState` variant, try the `satoshi::tick_demo` free-function route. Before adding a MobType, remember: **don't** — use the marker.
- **Append-only discipline:** `WorldSave.satoshi` goes LAST, after `saved_mobs`. Add a byte-layout/`read_tail` test in the same style as the existing tail tests so a future field can't silently shift it.
- Keep `satoshi.rs` under ~500 lines (CLAUDE.md decomposition rule). If the corpus grows, move authored strings to a bundled JSON asset (mirror `assets/scenarios/onboarding.json`) loaded once — still fully scripted.

---

## 7. What the code already supports (verified) vs. gaps

**Fully supported — reuse directly:**
- Villager mob + `VillagerComponent` auto-attach (`entity.rs:347/364`), profession=None path, anti-grief grace, `find_attack_target` interaction (`game_loop.rs:~7993`).
- Modal dialogue overlay with Accept/Decline/Close/TurnIn + dim-background (`villager_ui.rs:draw_villager_dialogue`), and the per-special-villager branch precedent (the **Builder** commission special-case, `game_loop.rs:~8014`).
- Quest data model + deterministic generation + completion/turn-in re-check gate + reward drop + juice (`quest.rs`, `game_loop.rs:~11588`).
- Procedural house build + villager spawn near a position, idempotent via a `populated_*` guard (`village_gen.rs:build_hardcoded_house`, `spawn_initial_villagers`).
- Append-only `WorldSave` persistence with `read_tail` graceful-default (`save.rs`), and the "re-derive procedural entities on load rather than serialise them" pattern (villagers).
- `Inventory::add_item` + `ItemStack::new_material`/`new_block` for the gift; toast + pickup juice already fire.

**Gaps the build creates (small, additive — none block MVP):**
1. **No "special unique villager" primitive.** There's the Builder profession but no per-world singleton NPC. → The new `SatoshiMarker` + a spawn guard fill this; ~trivial, additive.
2. **No villager-initiated contact / co-presence demo today.** Villagers only react to right-click; AI is `Wander`/`GolemGuard`/raid states — none walks-over-and-demonstrates. → P3 adds a scripted demo (free-fn preferred). This is the **only** behaviour genuinely new to the engine, and it's the playtest-gated piece.
3. **Quest state is transient** (`active_quests` is not persisted; `player_slot.rs:100` notes "Phase 11 may persist"). Satoshi's *progress* must therefore live on the persisted **`SatoshiState`** (booleans), not on the transient quest map — which is exactly what §3.3 does. No need to make quests persistent for MVP.
4. **`quest::summarise` / dialogue copy is reward-and-rep flavoured** ("Reward: … sats … rep"). Satoshi must NOT show sats/rep. → Use a **separate `draw_satoshi_dialogue`** (or a Satoshi mode) with kindness-framed copy, not `summarise`. Compliance test guards the corpus.
5. **The bundled `onboarding.json` scenario arc exists** (`scenario.rs:ONBOARDING_JSON`, surfaced by the challenge board). It is a *checklist* arc, not a guide character. MVP Satoshi does **not** depend on it; a later phase could let Satoshi *voice* that arc, but keep them decoupled for now (the design explicitly wants no checklist feel).

---

## 8. Anti-pattern guardrails (from the research — encode as review gates)

- **Not Navi:** Satoshi initiates **rarely** (once, gated by `should_initiate` + cooldown), never repeats an acted-on tip, goes quiet on wave-off. Reviewer checks the initiate logic can't fire repeatedly.
- **Not Fi:** nothing unskippable; never re-explains what the player just did. Every panel `Close`-able on one input.
- **Not Flowey/Clippy:** authored lines are warm and respectful — no condescension. Corpus review by a human (kid-safety pass) in addition to the substring test.

---

## 9. Deferred / do-not-build (explicit)

- **AI-driven Satoshi (live AI conversation via a player-supplied API key).** **DO NOT BUILD.** Unbounded AI talking to children is a child-safeguarding nightmare (design §8). Parked as a far-future possibility *only* behind serious safeguarding review. The MVP is 100% authored/scripted; there is no code path that sends a Satoshi line to, or receives one from, any model or network.
- **Rich branching dialogue trees** (beyond greet + 2–3 nudges + demo). Later.
- **Deep founding-myth lore tie-in.** Satoshi *is* the founder figure (`genesis-founding-myth-long-run.md`), but the MVP seeds no lore beyond tone. Fold in after the fun test passes.
- **Multiple co-presence activities / "show the locks" teases / pass-by wandering re-engagement beyond the single gated cue.** All post-MVP, post-playtest.
- **Multiplayer Satoshi sync.** Single-player / local split-screen only for MVP (matches quest identity model).

---

## 10. One-paragraph summary for the builder

Spawn one `MobType::Villager + SatoshiMarker` in a small findable house near spawn (P0). Right-click branches to a warm, scripted dialogue that greets + gifts food once (P1), offers 2–3 authored nudges that each build an existing `quest::Quest` with `sats=0` and pay a kindness-framed welcome gift through the existing turn-in gate — no completion screen (P2). One activity is demonstrated beside the player without ever seizing input (P3). All progress persists via a single append-only `SatoshiState` field after `saved_mobs`, and a frequency budget keeps him pull-first and un-naggy (P4). A compliance test guarantees the corpus never says "sats/earn/bitcoin/reward". Everything but the **does-it-feel-charming co-presence demo** is solo-verifiable; that one question is the playtest gate.
