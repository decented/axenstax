# Guided Build-Along for Schematics — Build Spec

**Upgrade the schematic/blueprint system so laying a plan becomes a *guide*, not just an insta-build: in Survival without the materials it shows you what to go and mine; with the materials you choose insta-build ("automagically") or a step-by-step guided build (paint-by-numbers / Lego instructions). Its own feature, then applied to Satoshi's gift.**

- **Date:** 2026-06-23
- **Status:** ✅ **DELIVERED (P1–P3) 2026-09-06** — branch `solo-queue-2026-09-06`. P4 stays deferred. `check.sh` **ALL GREEN** (4137 tests, 13 new). New pure module `game/engine/src/build_steps.rs` (step sequencer), `BuildGuide` gains `mode`/`steps`/`current_step`, the lay dialog offers **Insta-build · block-by-block · layer-by-layer**, the ghost focuses the current step, and the HUD panel shows `Step N of M` + a Survival "Go and gather" call-out.
  - **Deviations from this spec (and why):**
    1. **The step model lives in a NEW module `build_steps.rs`, not in `build_guide.rs`** (§7 said the latter). `build_guide.rs` keeps the world-facing verifier; the sequencer is a separate pure module so neither file grows past the project's ~500-line rule. `BuildGuide` itself (the state struct) still lives in `build_guide.rs` and holds the cursor.
    2. **Completed steps draw nothing** rather than a "solid/done" ghost. The block you placed *is* the done state — a ghost on top of it would z-fight the real block and re-clutter the very thing the focus render is meant to clear. Correct cells have always vanished; that behaviour is kept.
    3. **Survival-without-materials lays the guide in layer-by-layer mode**, not the whole-plan ghost. It costs nothing and scopes the "go and gather" list to the course in front of you — a shopping list you can act on — which is the whole point of that branch.
    4. **The mode picker is the lay-time dialog plus `/buildguide mode <block|layer|whole>`**, not an in-HUD control. The HUD panel is `interactable(false)` and the cursor is grabbed during play, so a button there could never be clicked; re-laying (the plan stays in your bag) or the chat command are the two real switches. `/buildguide <plan> [mode]` also takes a trailing mode word.
    5. **No `TestHost`/game-loop integration test.** The guide lives on the client `GameState`, which has no non-GPU-gated harness (`HeadlessGame` is `--ignored`). Instead there are two walk-through tests: one against a stand-in world in `build_steps.rs` and one against a **real `World`** in `build_guide.rs` (lay → build → advance → wrong block holds → mode switch mid-build → complete).
    6. **Back-a-step** is not implemented (spec P4).
  - **Not verified solo (feel, playtest-gated):** whether the current-step focus reads as clear rather than overwhelming; whether layer grouping paces well on a big plan; whether block-by-block is too slow to be fun on anything larger than a hut; and whether the 4-button lay dialog is the right number of choices for a 9-year-old.
- **Original status:** 🟢 READY TO BUILD — design + feasibility done; sits on the existing build-guide + animated-builder (mostly surfacing what's there).
- **Owner ask (2026-06-23):** "When you lay a schematic, if you don't have the materials and you're in Survival, highlight what you don't have so you know to go mine it. If you have all the materials, ask: do you want it built automagically or a guided build?" Build it as its own feature/upgrade, then apply it to the Satoshi gift — because we don't know yet how people will want to use it.
- **Companions:** `docs/foundations/2026-06-16-blueprint-build-guide.md` (the existing build-guide), `docs/superpowers/specs/2026-06-23-satoshi-onboarding-v2-design.md` (the schematic gift that consumes this).

---

## 1. What already exists (REUSE — do not rebuild)
The hard parts are built; this feature mostly *sequences + focuses + routes* them.

| Piece | Where | What it gives us |
|---|---|---|
| Build-guide core (pure + tested) | `build_guide.rs` | `BuildGuide` state, `material_list`, `verify`/`verify_cell` → **Correct/Missing/Wrong** per cell, `summarize`, `remaining_materials` |
| Build-along ghost command | `commands/builtins/buildguide.rs` (`/buildguide <plan>\|off\|list`) | Lays a plan as a **"build-along ghost guide"** (no materials needed) → sets `GameState.build_guide` |
| Ghost render | `renderer.rs` (`build_guide_buffer`, `build_guide_vertex_count`) | Per-cell wireframe cubes **coloured by status** |
| Guide HUD panel | `hud_ui::draw_build_guide_panel` | Material list + (correct/missing/wrong) + **remaining materials** |
| Animated auto-builder | `plan.rs` (`order_cells_for_build` Y-ascending, `tick_build` consumes locked materials in Survival / free in Creative, cells-per-tick pacing) | The **insta-build ("automagically")** path |
| Plan item + lay/ghost/commit | `plan.rs` + `game_loop` (Plan-item ghost preview, commit) | Laying an `Item::Plan` in the world |

**So today:** `/buildguide` lays a colour-coded full-grid ghost + shows what's missing (no materials needed), and placing a Plan item animated-builds it (consuming materials in Survival). The two live *separately*, and the guide shows the **whole grid at once** — a static reference, not a step-by-step guide.

## 2. The gap to close
1. The guide is a **dev command** keyed off the plan *registry*, not integrated with **laying a Plan item** the player holds (e.g. Satoshi's gift).
2. It shows the **entire plan at once** ("just a wire grid") — overwhelming, not a *guided* walk-through.
3. No **mode choice** when laying (insta vs guided); no **step sequencing**; no **Survival missing-materials-led** flow on a Plan lay.

## 3. The unified "lay a schematic" experience (the goal)
When the player lays an `Item::Plan`, branch on mode + materials:

- **A. Survival, missing some materials** → lay it as a **guide** (ghost coloured by status; the panel **emphasises what's still missing** so they go mine/retrieve it). No insta-build. Build along as materials arrive — cells flip to *Correct* as you place them. (Reuses `build_guide` + `remaining_materials`.)
- **B. Have all the materials (or Creative)** → a **choice**:
  - **Insta-build ("automagically")** → the existing animated builder (consumes in Survival / free in Creative).
  - **Guided build** → the new **step-by-step build-along** (§4).
- The player can pick **Guided even when they have the materials** (the whole point — a tool to build *along*, not just fast).

## 4. The new facility: step-by-step guided build-along
Built on `build_guide.rs` (which already does ghost + per-cell verify):

- **Step model** — split the plan's `cells` into ordered steps:
  - **Block-by-block** — one cell per step, in a *buildable* order (Y-ascending; reuse `plan::order_cells_for_build`).
  - **Grouped ("Lego instructions")** — group cells into logical steps. **MVP heuristic: layer-by-layer** (group by `ry` — build the floor, then each layer up). Reads naturally like Lego steps and is cheap. *Smarter grouping (connected wall sections, roof as a unit) is deferred — it's the one genuinely subjective/hard bit.*
- **Current-step focus render** — extend the `build_guide_buffer` colouring: **current step bright/pulsing**, completed steps **solid/done**, future steps **dimmed or hidden**. (The main *visual* work; feel is playtest-gated.)
- **Advance** — when the current step's cells all `verify` as **Correct**, advance to the next step (reuses the verifier — no new validation). Optional **back-a-step**.
- **Missing-material call-out** — in Survival, the panel + ghost flag which blocks you lack for the current step (and overall), so the guide doubles as a shopping list.
- **Mode picker** — a small dialog when laying: **Insta-build · Block-by-block · Step-by-step (grouped)**.

## 5. Phases
- **P1 — Route + Survival-missing flow.** Lay an `Item::Plan` → branch (materials/mode). Survival-without-materials → lay as a build-guide with the **missing-materials highlight** (reuses existing build-guide + `remaining_materials`); full-materials/Creative → the **insta-vs-guided choice dialog** (insta = existing animated builder).
- **P2 — Guided build-along (block-by-block).** Step sequencing (block-by-block) + current-step focus render + advance-on-Correct. Tests for the sequencer + advance logic.
- **P3 — Lego grouping + mode picker.** Layer-by-layer grouped steps + the block-vs-grouped picker + polish.
- **P4 (deferred).** Smarter logical grouping (connected sections), back-a-step, richer per-step material call-outs, animation pacing tuning.

## 6. Apply to the Satoshi gift
Satoshi gifts a Plan (the starter wooden hut, per the v2 design). Laying it runs this exact flow:
- **Creative** → the insta-vs-guided choice (build free, or build-along for the fun of it).
- **Survival** → if short on materials, the guide + "go mine these" highlight; build along as you gather; or if you have it all, the choice.
This makes Satoshi's gift a *teach-you-to-build* moment, not an insta-house — which is the whole point of the guide. (Satoshi's onboarding should *default-suggest* the guided build, but never force it.)

## 7. File hook-list
- `build_guide.rs` — add the **step model** (sequencer: block-by-block via `order_cells_for_build`; layer-grouped by `ry`) + a `current_step` + step-status helpers; reuse `verify`/`remaining_materials`. Keep it pure-core.
- `renderer.rs` (`build_guide_buffer`) — current-step **focus colouring** (bright/solid/dim).
- `hud_ui::draw_build_guide_panel` — **step indicator** ("Step 3/12"), **missing-material emphasis**, mode hint.
- `plan.rs` — the animated builder stays the **insta** path (reuse `tick_build`); the guided path validates player placement rather than auto-placing.
- `game_loop` — the **Plan-item lay handler**: branch on materials + mode → guide / insta / guided; the **mode-choice dialog**; per-tick advance-on-Correct while a guided build is active.
- `commands/builtins/buildguide.rs` — optionally gains a step mode; otherwise stays the dev/registry path (the *player* path is laying a Plan item).
- `GameState` — `build_guide` already lives here; add `current_step` + the active build mode.

## 8. Acceptance criteria
**Solo-verifiable:**
- Lay a Plan in Survival **without** the materials → ghost appears + the panel lists **what's still needed** (no insta-build, no crash). Place the right blocks → they verify **Correct** as you go.
- Lay a Plan with **all** materials (or in Creative) → the **choice dialog** appears → Insta builds it (existing path) / Guided steps through it.
- **Block-by-block:** the current block is highlighted; placing it correctly advances; a wrong block reads Wrong and doesn't advance.
- **Layer-grouped:** each step is one horizontal layer; completing a layer advances.
- The **mode picker** switches block-by-block ↔ grouped.
- Unit tests for the **sequencer** (block-by-block order; layer grouping) + **advance-on-Correct** logic.
- `check.sh` green.

**Playtest-gated (feel):**
- Does the current-step focus feel **clear and helpful**, not overwhelming?
- Is **layer-grouping intuitive** (the Lego-instructions feel), and is the pacing right?

## 9. Conventions
- **UK English.** **Reuse before adding** (`build_guide`/`verify`/`remaining_materials`/the animated builder). Keep `build_guide.rs` **pure-core**; render + UI call in. `check.sh` green; tests for the new pure logic.

## 10. Recommended sequencing
This benefits **every** schematic, not just Satoshi's. Suggested order: land **Satoshi v2 part 2** (the schematic gift) using insta-build + the existing guide first, then build **this** (P1→P3) as the upgrade that makes laying *any* schematic a guided experience — and have Satoshi's gift adopt it.
