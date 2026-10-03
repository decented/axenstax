# `begin_load` WASM/PWA Resume Position-Fix — Spec

**On a PWA page reload of a *saved* world, the player can be teleported back to spawn and have the creative starter hotbar re-dumped over their inventory. This spec pins the root cause and the `world_preloaded` guard that fixes it.**

- **Date:** 2026-06-22
- **Status:** ✅ **SHIPPED 2026-07-10** — built + verified end-to-end in the live PWA runtime by a headless-browser session (the "browser-gated" constraint was lifted by the headless WebGPU verify recipe from the 256-layer work). Build notes vs this spec: (a) §3.3's "pre-stage the bytes" half had already been built in the interim — the WASM resume restore now lives in `game_loop.rs`'s IndexedDB poll branch, which fully restores world + players *before* entering `GameMode::Loading`; only the guard was missing. (b) The creative-starter-kit half of the symptom was obsolete (kit removed 2026-07-02), but the Workshop Bellows and Test-Lab kit dumps in the FRESH branch were still live re-dump hazards and are now guarded too. (c) Implemented as a one-shot `GameState.world_preloaded` flag (`take()`-consumed by `begin_load`, following the `pending_workshop_reset` idiom), routed through a pure, unit-tested `fresh_setup_wanted(load_ok, world_preloaded)` decision fn. Verified: full headless flow (create → move → Save and Quit → **page reload** → reopen) shows "Resuming preloaded world … skipping fresh-world setup", zero "Fresh world spawn" on resume, exactly one on genuine create, and a pixel-identical resumed vista/minimap.
- **Severity:** High for the alpha (web PWA is the alpha target; losing your spot + having your hotbar overwritten on every reload is a bad first impression).
- **Scope:** `game/engine/src/chunk_stream.rs` `begin_load`, plus the web resume entry that drives it. **No save-format change.**

---

## 1. The symptom
On the **web/PWA** build, reload the page while in a saved world (or let the PWA be evicted + relaunched) and:
- the player **respawns at the world spawn point** instead of where they logged out, and
- in **creative** worlds, the **starter hotbar is dumped back into slots 0–8**, overwriting whatever the player had there.

Native does **not** exhibit this — native `save::load_world` reads the filesystem synchronously, so the load always succeeds before placement.

## 2. Root cause (verified against code, 2026-06-22)
`begin_load` (`chunk_stream.rs:145`) does a **synchronous** load and branches on its result:

- **Line 170–186:** `let load_result = … crate::save::load_world(&self.world_name, &mut self.world); if let Ok((save_data, _)) = load_result { /* RESTORE */ } else { /* FRESH */ }`.
- **RESTORE branch (`if`)** restores player position, inventory, pets, carts, bed-spawn, etc. from `save_data`.
- **FRESH branch (`else`, lines 366–419)** assumes a brand-new world: it
  - **resets the player position to a freshly-computed spawn** (`chunk_stream.rs:396–397`), and
  - **re-dumps the creative starter kit** into the hotbar (`chunk_stream.rs:402–419`).
  - Its own comment (line 400–401) asserts *"this `else` is never reached on re-entry, so the kit is never re-dumped"* — **that assumption holds on native but is false on WASM.**

**Why the assumption breaks on WASM:** the actual world bytes live in **IndexedDB** and are fetched through an **async** JS bridge (`wasm_save.rs::load_world_wasm` → `js_load_world`, both `async`). `begin_load` is synchronous and calls the synchronous `save::load_world`, which can only succeed if the bytes were **pre-staged into the synchronous store before `begin_load` ran**. On a PWA reload the async IndexedDB read may not have completed (or wasn't kicked off / awaited) by the time `begin_load` runs, so `load_world` returns `Err` → the **FRESH branch runs on a world that actually has a save** → position reset + kit re-dump.

So the bug is a **timing/branch-selection** bug, not a data bug: the save is fine; `begin_load` just picks the wrong branch because it infers "new vs resume" from a synchronous load that can spuriously fail on web.

## 3. The fix — a `world_preloaded` guard
**Principle:** stop inferring "brand-new world" from `load_result.is_ok()`. Carry an explicit intent flag, set by the entry point that knows whether this is a *new* world or a *resume*, and never run the position-reset + kit-dump on a resume.

### 3.1 Add the flag
- Add a boolean to the loader state (the `GameState`/world-loader struct that owns `begin_load`), e.g. **`world_preloaded: bool`** (or, equivalently framed, `is_new_world: bool`). Default such that the *new-world* creation path is the only thing that sets "this is genuinely new".
- Set it from the two entry points:
  - **New World** (menu → create): mark as a brand-new world (FRESH placement + kit are wanted **once**).
  - **Load/Resume World** (menu → open existing, and the PWA auto-resume path): mark as a resume (`world_preloaded = true`) — FRESH placement + kit must **never** fire.

### 3.2 Gate the FRESH-only side effects
In `begin_load`'s `else` branch, guard the two destructive actions on the resume flag:
- **Position reset (`chunk_stream.rs:396–397`)** — only place the player at a fresh spawn when this is a genuinely new world. On a resume that fell into `else` (because the async load wasn't ready), **do not** overwrite `players[0].player.pos` / `velocity`; leave the restored/last-known position (see §3.3 on ordering).
- **Creative starter kit (`chunk_stream.rs:402–419`)** — only dump the kit for a genuinely new world. Never on resume.

### 3.3 Make the web path pre-stage the bytes (the real correctness half)
The guard above stops the *damage*, but the **world should still actually load on resume**. The web resume entry must **await the async IndexedDB read and pre-stage the bytes into the synchronous store `save::load_world` consults, before calling `begin_load`** — so the RESTORE (`if`) branch is taken on web exactly as on native. Concretely:
- The web resume flow (`web_main.rs` / the wasm entry that transitions into `GameMode::Loading`) should `await load_world_wasm(pubkey, name)` and stage the returned bytes where `save::load_world` reads them, *then* enter the load state that runs `begin_load`.
- With bytes pre-staged, `load_world` succeeds → RESTORE branch → the `world_preloaded` guard is belt-and-braces for the residual race (eviction mid-session, partial IndexedDB, etc.).

**Net:** `world_preloaded` guarantees we never *vandalise* a resume even if the load momentarily fails; pre-staging guarantees the resume actually *restores*. Ship both.

## 4. What NOT to change
- **No save-format change.** This is purely loader control-flow + an entry-point intent flag.
- **Do not** touch the RESTORE branch's restore logic (pets/carts/bed-spawn/etc. were fixed separately on `3105b6cb` and are correct).
- **Do not** remove the FRESH branch — a genuinely new world still needs spawn placement + the creative kit exactly once.
- Keep native behaviour byte-for-byte identical (native already takes RESTORE on resume; the flag just makes the intent explicit).

## 5. Acceptance criteria
**Solo-verifiable (native + tests):**
- New world (native): player spawns on ground, creative kit present once. Unchanged.
- A unit/integration test asserting that, given a resume intent (`world_preloaded = true`) and a *failed* synchronous load, `begin_load` does **not** overwrite a pre-set player position and does **not** populate the creative hotbar. (This isolates the branch logic without the browser.)

**Browser-gated (the actual bug — needs the PWA runtime):**
- Web/PWA: enter a saved creative world, move away from spawn, put custom items in the hotbar, **reload the page** → player is at their last position, hotbar is intact, world is fully restored.
- Repeat in survival. Repeat after a hard PWA relaunch (app evicted).
- Confirm a brand-new web world still spawns correctly + gets the kit exactly once.

## 6. Why this is spec-only tonight
The defect only manifests under the **WASM/PWA async-load timing**; a native/CLI session cannot reproduce it (native load is synchronous) and cannot drive a browser reload to verify the fix. The branch-logic unit test in §5 is solo-buildable, but the **fix is not complete or trustworthy without the live PWA reload test**, so per the overnight guardrails this is captured as a spec, not built. Hand to a supervised browser-capable session.

## 7. File touch-list (for the builder)
- `game/engine/src/chunk_stream.rs` — add the `world_preloaded` field read; guard lines `396–397` (position) and `402–419` (creative kit) on it.
- The loader-state struct definition (same module / `game_loop.rs`) — add the `world_preloaded: bool` field + default.
- The two world-entry points (New World vs Load/Resume) in the menu/entry flow (`menu.rs` / `web_main.rs` / native entry) — set the flag.
- `game/engine/src/web_main.rs` (+ `wasm_save.rs`) — await + pre-stage the IndexedDB bytes before entering the load state (§3.3).
- Add the §5 branch-logic test under `src/test_integration/` (e.g. extend `save_load.rs`).
