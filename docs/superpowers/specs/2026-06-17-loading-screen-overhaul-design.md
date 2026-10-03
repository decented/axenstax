# Loading Screen Overhaul — Design

**Date:** 2026-06-17
**Status:** BUILT 2026-06-17 (branch `feat/loading-screen-overhaul`, 6 tasks, 2723 engine tests green; awaiting Axolittle feel playtest). Approved (owner: "smash it all", 2026-06-17)

**Build deltas vs. design:**
- `begin_load()` takes no params (load centre derived from the restored/placed
  player), and `initial_load(_pcx, _pcz)` is the blocking wrapper kept as the
  `stream_chunks` fallback.
- The in-engine load is made incremental by deferring **all** heavy per-column
  work (gen/light/water/scatter/mesh) to `step_load`, unifying the save + fresh
  paths (the save path no longer early-returns). The save path's "mesh every
  loaded chunk" is now incremental too — fixing the freeze on big saved worlds.
- WASM keeps its existing async data-fetch overlay (`menu::draw_loading_overlay`,
  now WASM-only) for the brief pre-data phase, then enters the same animated
  `GameMode::Loading` for the chunk build — so phase 1 (fetch) and phase 2
  (mesh) both have feedback. `begin_load` is identical on both platforms.
- Loading paints one frame BEFORE running `begin_load` (instant feedback), via a
  `painted` flag on `LoadingState`.
**Surfaces:** web bundle loader (`index.html`) + in-engine world-entry (egui)

## Problem

Two loading moments are weak:

1. **"Enter the game"** — the WASM bundle download page (`game/engine/index.html`)
   is a bare `#loading` div: the word "Axe'n'Stax" in Georgia serif + grey
   "Loading…" on a dark background. No art, no animation, no progress. Looks
   unfinished.
2. **Entering a world** (New World / Load / Workshop / Scenario) — *feels stuck*.
   Root cause: `chunk_stream::initial_load` (file read + world-gen + meshing) runs
   **synchronously on a single frame and freezes the thread**. The current code
   paints one "Loading <name>…" frame then blocks — on **both** native and WASM
   (on WASM only the data *fetch* is async; world-gen + meshing still freeze the
   tab). The frozen frame reads as a hang.

## Goals

- Both loading moments share **one brand look** and feel like one product.
- The world-entry load **never freezes** — the screen stays alive (animated) the
  whole time, with a **real progress bar**.
- The player always gets **enough time to read a tip** (minimum display time).
- Loading content is **useful**: how-to-play Tips + "What's new — needs testing"
  notes, rotating.

## Approved visual direction

"Voxel World hero + rotating card beneath" (companion mockup `combined-bc.html`):

- Floating voxel diorama (hero), `AXE'N'STAX` wordmark, `PROOF OF PLAY` tagline.
- Animated progress bar.
- A **rotating content card** beneath the bar, cycling through cards every ~6s.
- Dark navy background (`#0d1220` / radial to `#1a2746`), gold/amber accents
  (`#d4a044` / `#f0c674`), green highlight for "new" (`#6fae5a`).

## Design

### 1. Shared content source (one file, baked two ways)

Canonical content file in the engine asset tree (e.g.
`game/engine/assets/loading_tips.json`). Each entry:

```json
{ "kind": "tip" | "new", "title": "…", "body": "…" }
```

- ~15–25 starter cards: how-to-play **Tips** and "What's new — needs testing"
  notes (**no** Challenges/Tutorials yet — those systems aren't built).
- **Engine (Surface 2):** `include_str!` the JSON at build time → parsed into a
  static `Vec<LoadingCard>`. Works offline, native + WASM, no fetch.
- **Web bundle loader (Surface 1):** the same file is copied into the trunk
  `dist` (via `data-trunk rel="copy-file"`) and fetched by the loader JS at a
  static URL. One source of truth, no drift.
- **Card picker:** shuffled order, never the same card index twice in a row.
  Pure function (testable).

### 2. Surface 1 — web bundle loader (`index.html`)

Replace the bare `#loading` div with the B+C layout in plain HTML/CSS + a small
loader JS module (CSP-friendly external script, like the other `/static/*.js`):

- Floating CSS voxel-cube hero, wordmark, tagline, animated progress bar.
- Rotating card beneath; JS fetches `loading_tips.json`, shuffles, swaps every ~6s.
- Bar is a smooth eased fill (real bundle download-progress is a **stretch goal**;
  default is an indeterminate/eased animation).
- Hides on the existing `window.__axenstax_start` hook (unchanged).
- Falls back gracefully if the JSON fails to load (hero + bar still show; no card).

### 3. Surface 2 — in-engine world entry (egui) — the real fix

**Make the load incremental** so the loop keeps rendering:

- Split `chunk_stream::initial_load` into:
  - **one-shot setup** — save read + player restore + per-world resets (fast); and
  - an **incremental gen+mesh queue** — the spawn-area columns to generate + mesh,
    drained at a per-frame budget. Reuse the existing per-frame budgeting pattern
    from `stream_chunks` / the mesh-rebuild queue (Spec 39 A4).
- `GameMode::Loading` becomes a **real repainting state on both native AND WASM**
  (removes the current native/WASM divergence). Each frame, in this state:
  1. advance the load queue by the budget,
  2. render the egui loading screen (voxel hero + wordmark + bar + rotating card),
  3. progress bar = `columns_done / columns_total` (real),
  4. rotate the card every ~6s.
- **Minimum display ~5s:** if the world finishes loading sooner, keep the animated
  screen up (still rotating) until the minimum elapses, so the tip is readable.
  If loading is slower, real progress drives the bar.
- Transition to `Playing` only when **both** the queue is drained **and** the
  minimum time has elapsed.
- **Retire** the old one-frame `draw_loading_overlay` flash and the WASM
  "straight to Playing then block on first frame" path.

The egui screen mirrors the HTML look within egui's painter: filled rects for the
voxel cubes (with the same shaded look), text for wordmark/tagline, a rounded
progress rect, and a card rect with a kind label + title + body.

### 4. State machine (Surface 2)

```
Menu --(world chosen)--> Loading { setup done?; queue; progress; started_at; card_state }
Loading (each frame): drain budget; repaint; rotate card
Loading --(queue empty AND elapsed >= MIN_DISPLAY)--> Playing
```

### 5. Tunable constants

- `CARD_HOLD` ≈ 6s (per-card display before rotating)
- `MIN_DISPLAY` ≈ 5s (floor on total loading-screen time)
- `LOAD_BUDGET_PER_FRAME` — columns generated+meshed per frame (tune for smoothness)

All easy to adjust in playtest.

## Testing

- **Pure-function units:**
  - card picker — no immediate repeat; respects `kind` filtering; deterministic
    given a seed/index sequence.
  - load state machine — queue drains to empty; progress is monotonic in
    `[0,1]`; never completes before `MIN_DISPLAY`; completes once both gates pass.
  - JSON parse — the bundled `loading_tips.json` parses into ≥1 card and every
    card has non-empty title/body and a valid `kind`.
- **Feel** (voxel art, animation smoothness, whether 5s/6s feel right, queasiness)
  is the **Axolittle playtest boundary** — can't be verified solo.

## Risk

Surface 3 (incremental load) touches the chunk-load path — the highest-risk piece.
It reuses existing budgeting machinery rather than inventing new, but it is the
part to build carefully and test hardest. Surfaces 1 + content are low-risk wins.

## Non-goals (deferred)

- Challenges / Tutorials cards (gated on those systems existing).
- Real bundle-download progress on Surface 1 (eased bar is fine for alpha).
- Per-block or themed loading art beyond the voxel hero.
- Wiring the card list to any live data source (hand-curated for now).
```
