# Satoshi Onboarding v2 — agreed design refinements

**Captures the owner's refinements (2026-06-23) over the built v1 MVP. Supersedes the relevant parts of v1 (`2026-06-22-satoshi-onboarding-foundation-spec.md`) for the areas below. DESIGN ONLY — not yet built.**

- **Date:** 2026-06-23
- **Status:** 🟢 Agreed, ready to build on owner's go. One open decision (hut build animation) + one content piece to author (starter plan).
- **Built v1 (on `main`):** instant cobblestone hut, food gift, Fetch-quest nudges (wheat/berries), spawns in *any* non-Workshop world (incl. existing). **v2 reworks all of that.**

---

## 1. Placement (CHANGED)
- **New worlds ONLY** — tied to the explicit **"Create World"** action, *not* the load path (so the WASM reload bug can never sneak him into an existing world). **Never** existing worlds (the owner's core rule: don't intrude on a world someone has invested in).
- **Normal world type ONLY** — skip **Workshop, Flat/Blank-Canvas, and Gallery**.
- **Game mode Survival or Creative ONLY** — **absent in Adventure & Spectator**.

## 2. His hut (CHANGED)
- A humble **wooden** hut — wood frame + plank walls + a door. "Start with wood" = humble beginnings (and sets up *upgrading* his place as the world grows — a nice future founding-myth thread, not now).
- **Built, not spawned** — it should feel constructed, not pop in fully-formed. **OPEN DECISION:**
  - **(a)** Satoshi visibly **builds it** (animated, wood-frame first) via the engine's existing animated builder — charming, on-theme, more work, pacing is a playtest-feel thing.
  - **(b)** A proper **wooden hut placed cleanly** at creation, with the "watch it build" animation as a fast-follow. *(Recommended for the first cut — lower risk, faster; honours "wooden hut, not a cobblestone box" now.)*
- Same hut in both modes.

## 3. The gift = a SCHEMATIC (CHANGED — this is the centrepiece)
Satoshi **gifts a starter wooden hut/shelter PLAN** (`Item::Plan`). This unifies both modes and showcases a genuine differentiator (built-in blueprints; see §5).
- **Survival flow:** greet → *"you look peckish, here's a bite"* (food — the care touch) → **then gift the schematic** → the build-guide shows the materials → gather them + build it (the animated builder consumes them). *The gather-and-build is the onboarding task.*
- **Creative flow:** greet → **straight to the schematic** → place it and build anywhere, free. **No material reward** (a plan isn't materials, so this stays consistent with the Creative "no material reward" rule).

## 4. Content (per mode)
- **Survival corpus:** warm greeting + food gift + schematic gift. (The earlier grow/forage Fetch nudges become *optional extras*, not the centrepiece.)
- **Creative corpus:** own greeting + schematic gift. Optional build/Workshop/"build tall" challenges = **offer + encourage, no tracked completion, no material reward** (Creative builds can't be auto-detected — accepted consequence of "no reward").
- **Compliance test** scans **both** corpora: never any money/earning word, UK English, kid-safe (unchanged, load-bearing).

## 5. Schematics vs Minecraft (why this is worth showcasing)
Vanilla Minecraft has **no built-in player-facing blueprint/build-along system** — only niche Structure Blocks. The schematic experience players know is **mod-only** (Litematica, WorldEdit `.schem`, Create). AxeNStax's blueprint pipeline is **built into the base game** — a real differentiator, and introducing it via Satoshi's gift shows it off immediately.

## 6. Feasibility (honest)
- The Plan/blueprint pipeline already exists and works in both modes: capture (Drafting Stamp) → ghost preview → **build-guide material list** → **animated builder** → `.schem` import. Gifting an `Item::Plan` and building it is fully supported.
- **Gap:** the engine-bundled plan library is **empty** (curated plan content was deferred, `plan_registry.rs` Phase 5). So the **one new content piece is authoring the starter wooden-hut plan** for him to gift. Everything else is wiring existing systems.

## 7. Build scope when greenlit (one batch)
1. Placement gating: new‑world flag set on the explicit Create action; gate on it + normal world type + Survival/Creative mode.
2. Wooden hut (option a or b per the open decision).
3. Author the starter wooden‑hut **plan** + have Satoshi gift it (`Item::Plan`).
4. Mode‑aware content: Survival (food → schematic) vs Creative (schematic), selected by current game mode at dialogue time.
5. `SatoshiState` gains the `enabled` (new‑world) flag; persistence + byte‑layout tests already self‑adjust.
6. Keep `check.sh` green; compliance test covers both corpora.

## 8. Open / to confirm
- **Hut build:** animated (a) vs instant‑then‑animate (b). *Recommend (b).*
- **Author the starter hut plan:** confirmed needed (no bundled plans) — this is the new content.
