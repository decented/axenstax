# Blank-Canvas Worlds + Web↔Native World-File Interchange — Design

**Date:** 2026-06-09
**Status:** Approved (Staxolottle, 2026-06-09 — "go with your options, keep water")

Two **independent** features, built and shipped in order. Feature A first (cleaner-scoped, owner named it first), then Feature B.

---

## Feature A — Web↔Native World-File Interchange

**Goal:** A world created on either platform opens on the other. The `.axeworld` file (a tar+gzip of the world) is the single universal interchange format.

**Current state (verified):**
- The world *data* is identical across platforms — shared `WorldSave` + `WorldMeta` + chunk serialisation in `save.rs`.
- `pack_world` / `unpack_world` (world ⇄ `.axeworld` tar+gzip bytes) exist in `wasm_save.rs` but are `#[cfg(target_arch = "wasm32")]`-gated. So is the Export/Import UI.
- The `flate2` + `tar` crates are under the `[target.'cfg(target_arch = "wasm32")'.dependencies]` section — **wasm-only today**.
- Native persists worlds as raw folders at `worlds/<name>/`; it has **no `.axeworld` path** and no file-dialog dependency.

**Design:**
1. **Make the packing deps cross-platform.** Move `flate2` + `tar` to the general `[dependencies]` (or add to the `not(wasm32)` target). Confirm `pack_world`/`unpack_world` use no wasm-bindgen/JS types internally (they shouldn't — they're tar+gzip over byte buffers).
2. **Un-gate `pack_world` / `unpack_world`** — remove the `#[cfg(wasm32)]` so both compile on native. Add a **native round-trip unit test**: build a small `World` → `pack_world` → `unpack_world` → assert blocks + meta survive. (Byte-compatibility with web is automatic: same code, same bytes.)
3. **Native Export** — a Lobby "Export world" action: pick the target world → `rfd` *save* dialog → write `<name>.axeworld` (= `pack_world` bytes).
4. **Native Import** — "Import world": `rfd` *open* dialog → read `.axeworld` → `unpack_world` → write to `worlds/<dedup-name>/` (reuse `dedupe_world_name`, never overwrite) → appears in the world list.
5. **Dependency decision:** add **`rfd`** (Rust File Dialog) as a **native-only** dep for the open/save dialogs. It integrates cleanly with the existing winit/egui stack. *(Flagged: this is a new dependency. If the owner prefers zero new deps, the fallback is a fixed `exports/` + `imports/` folder convention with no picker — uglier UX, kept as a backup, not the plan.)*
6. **(Nice-to-have, same feature):** a native **"Reveal worlds folder"** button so the on-disk `worlds/<name>/` location is never a mystery (helps the repo-handoff workflow).

**Result:** a `.axeworld` exported on web imports on native and vice versa; the round-trip test proves the format is shared. Native worlds remain folders, but can now be packed to / unpacked from `.axeworld` for portability.

**Out of scope:** cross-player cloud sharing (separate concern); the `.axeworld` format itself is unchanged (we're just making it cross-platform).

---

## Feature B — Blank-Canvas Worlds

**Goal:** "New World" offers a flat **blank canvas** to build on (parkour and anything else), with options, persisted in the world so they survive reload and travel inside `.axeworld`.

**The dialog:** the New World screen gains a **World type** choice:
- **Normal** — today's procedural terrain (unchanged default).
- **Blank Canvas** — a flat world. Selecting it reveals the options below.

**Blank-Canvas options:**

| Option | Choices | Default | Notes |
|---|---|---|---|
| **Ground surface** | None (bare bedrock) · Grass · Sand · Stone · Dirt · Snow · **Water** | Grass | The visible top layer. Always sits on a **bedrock base** (unbreakable — you can't fall into the void). |
| **Water depth** *(only if Ground = Water)* | 1–8 | 3 | N water-source layers over a **sand bottom** over bedrock. Water needs a solid base or it drains into the void — hence the bottom + depth. |
| **Time** | Day-night cycle · Always Day · Always Night | Cycle | A world-time lock. |
| **Mobs** | Off (peaceful) · On | Off | Build undisturbed by default. |
| **Weather** | Clear · On | Clear | Clear skies for a clean canvas. |
| **Start mode** | Creative · Survival | Creative | Build in Creative; flip to Adventure via `/gamemode adventure` when ready to publish (ties into the play-modes work). |

**Generation:** generalise the existing `generate_workshop_column` (which already builds a flat sand-over-bedrock floor) into a parameterised `generate_flat_column(cfg)`:
- Bedrock base layer at the bottom (always).
- For most grounds: one surface layer of the chosen block directly above bedrock (None = bare bedrock showing — no surface layer).
- For Water: bedrock base → one **sand** bottom layer → **depth** water-source layers; the water surface is the top.
- Everything else: air. No terrain/trees/villages/ores (mirrors the Workshop void preset).

**Persistence:** add the canvas config to `WorldMeta` (new `#[serde(default)]` fields — tolerant decode, so old saves load fine):
- `world_type: "normal" | "flat"` (default normal)
- `ground: String` (e.g. "grass"; default "grass") + `water_depth: u8` (default 3)
- `time_lock: "cycle" | "day" | "night"` (default cycle)
- `mobs_enabled: bool` (default for flat = false; normal = true)
- `weather_enabled: bool` (default false for flat)

These ride inside `WorldSave`/`WorldMeta`, so they persist on reload **and travel in `.axeworld`** (Feature A) automatically.

**Consumption:**
- World-gen reads `world_type`/`ground`/`water_depth` to pick the generator.
- The world-time tick honours `time_lock` (skip advancing time when locked; pin to a day/night value).
- Spawning honours `mobs_enabled` (no hostile spawns when off — reuse the peaceful-difficulty path).
- Weather (if a weather system exists) honours `weather_enabled`; if there's no weather system yet, this field is reserved and a no-op (note it).

**Decision — "None" ground:** "None" means *no surface block* — the bedrock base is the visible floor. (Per the owner: "there should be a bedrock base, plus the option.") There is no pure-void/skyblock option in v1 (that would contradict the always-present bedrock base); it can be added later if wanted.

---

## Build order & verification
1. **Feature A** → check.sh green → deploy. (Round-trip test is the key gate.)
2. **Feature B** → check.sh green → deploy. (Generation unit tests per ground type; meta round-trip test.)

Each ships independently. Feel/UX (does the dialog read well for a kid, does a water canvas feel right) is the Axolittle playtest boundary.

## Out of scope / deferred (YAGNI for v1)
- Finite platform size / custom sky colour / configurable water-bottom block.
- Pure-void (skyblock) ground.
- A `world` content type on Beacon / publishing worlds as experiences (separate future spec — the "user-generated experiences" work).
