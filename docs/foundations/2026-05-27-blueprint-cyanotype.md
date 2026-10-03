# Blueprint / Cyanotype — capture by contact, develop in the sun

**Date:** 2026-05-27
**Status:** BUILT — superseded at the representation layer by
[`2026-06-04-blueprint-face-attachment.md`](2026-06-04-blueprint-face-attachment.md).
See note below.
**Driver:** Staxolottle + Axolittle. Grew out of "make the blueprint blue" →
the real history of *why* blueprints are blue. Text-only design.

> **Representation change — 2026-06-04.** As of the `feat/blueprint-face-attachment`
> build the laid-paper representation has **moved from full-cube blocks to paper-thin
> face attachments**. Blueprint Paper (`BLUEPRINT_PAPER`, id 56), Latent Print
> (`LATENT_PRINT`, id 158), and Cyanotype Print (`CYANOTYPE_PRINT`, id 172) as **placed
> solid cubes** are retired — a held Blueprint Paper now lays a `BlueprintBlank` face
> attachment (zero height, zero collision) on the floor's top face; a retrieved Latent
> plan re-lays as a `Blueprint(Latent)` attachment for sun-develop. The block ids and
> definitions are retained for decode compatibility with legacy saves (Phase G migration
> is deferred — see the 2026-06-04 foundation doc). The capture / develop / PlanData /
> Construction Anchor mechanics described below are otherwise **unchanged**; only *where
> the paper lives in the world* has changed.
>
> Full data model, the capture rework, mode-split, two-stage peel, and deferred
> migration are documented in
> [`2026-06-04-blueprint-face-attachment.md`](2026-06-04-blueprint-face-attachment.md)
> §"Implementation outcome".

## Why

Blueprints are blue because of the **cyanotype** (John Herschel, 1842): paper
coated in iron salts, a drawing/object laid on top, exposed to **sunlight** →
the lit areas turn Prussian blue, the masked lines stay white → white-on-blue.
It was the cheap-copy method for plans for a century; the *first* use was
actually **art** (Anna Atkins' 1843 cyanotype book of algae — the first
photographic book). So a blueprint is blue because it's a **sun-printed copy**,
not because it's "dyed paper". That distinction drives the whole mechanic and
cleanly separates blueprints from the decorative dyed-paper line (Spec 35).

## The loop (capture anywhere → develop in light)

The real process is two stages — *prepare/capture*, then *expose* — and that's
exactly what lets you blueprint a cave build:

1. **Blueprint Paper** ← `Papyrus Sheet + Iron + Salt` (the iron-salt-coated,
   light-sensitive sheet — pale greenish-yellow, unexposed). **No stick** (the
   old Plan-Tile stick is dropped — it never made sense; see Spec 24 note).

2. **Capture — anywhere, no sunlight needed.** Lay the Blueprint Paper flat
   (on the floor under a build, or against a wall for artwork) so its **edge is
   exposed**. Build on/over it, or lay it against an existing build. **Lift it
   by the exposed edge** → you get a **Latent Print**: pale, holding the
   captured form, undeveloped. The build is **untouched** — you only pull the
   paper out. This works in a cave, a mine, anywhere.

3. **Develop — sunlight only.** Lay the Latent Print **flat, under open sky, in
   daytime**. It develops over **~90 s of cumulative direct sunlight**, visibly
   deepening pale → faint blue → blue → deep blueprint-blue (~4 stages). The
   timer **only advances in real daylight on the sheet** — shade, a roof, or
   night **pauses** it (no UV at night) and it resumes at sunrise. Result: the
   finished **Blueprint** (white-on-blue). Carry a latent print around in the
   dark indefinitely; nothing's lost — you just haven't taken it to the sun yet.

So the journey is **lay → build → lift → carry to daylight → lay out → wait
~90 s of sun → collect.** Nobody's blocked from building underground; they just
surface to develop. (Sun-only is final — a flame lantern emits essentially no
UV, so artificial developing was dropped; magnesium got its own spec instead.)

## Buildings *and* artwork

Same paper, same sun, two destinations (cyanotype did both historically):
- Capture a **building** → a **Blueprint** you can re-build (the Spec 24 Plan).
- Capture **artwork** (a wall of coloured/dyed blocks) → a **Cyanotype Print**
  you **hang on a wall** as décor (and trade — player sun-print galleries).

The captured item is generally a **Cyanotype**; it's a *Blueprint* when it
holds a building and *art* when it holds a picture. (Naming: keep "Blueprint"
as the player-facing word for the buildable kind.)

## Relationship to Spec 24
Spec 24 delivered the Plan item, the capture-a-16³-box, the Construction Anchor
re-build, and the Plaque/attribution. This spec **revises the front-end**:
- The capture interaction becomes **lay-and-lift** (replacing "place a Plan
  Tile at the corner, right-click"). The old `Stick + Papyrus → Plan Tiles`
  recipe is **retired**; Blueprint Paper replaces it.
- The Plan gains a **develop state** (Latent → Developed, driven by sunlight on
  the laid-out sheet) and renders **blue** when developed.
- Plaque, Construction Anchor re-build, licensing, and the captured building
  data are **unchanged** — they sit downstream of the developed Blueprint.

## Data model (build notes)
- **Item (`MaterialId` or a Plan variant):** `BlueprintPaper` (sensitised
  blank). The captured object stays the existing `Item::Plan(PlanData)`, with
  **two new fields on `PlanData`** (`#[serde(default)]` for save-compat):
  `develop_progress: u16` (sun-ticks accrued) and a `kind`/`developed` flag
  (Latent vs Blueprint vs Cyanotype-art).
- **Recipe (`crafting.rs`):** `Papyrus + Iron + Salt → Blueprint Paper`.
- **Capture:** lay-and-lift handler — place Blueprint Paper as a flat block,
  sample the adjacent build volume on lift, hand back a Latent `Plan`.
- **Develop tick:** when a Latent Print is laid flat, accrue progress while
  `effective_sky_light` is full + it's daytime + open sky above; advance the
  visible stage; at full progress mark Developed. Reuses the lighting/day
  systems shipped in Spec 30.
- **Render:** developed Plans render blueprint-blue (white-on-blue); the
  hangable Cyanotype-art variant is a wall-mounted decorative block.

## Verification & boundary
- **Unit-testable:** Blueprint Paper recipe; capture produces a Latent Plan
  holding the right blocks with the build intact; develop progresses only under
  full daylight and pauses at night; reaches Developed at the threshold;
  save/load round-trips the new PlanData fields.
- **Playtest boundary (Axolittle):** the lay/build/lift feel, the ~90 s sun
  timer (drag vs snappy), the "take it to the light" journey, and the blue
  reading like a blueprint.
- `check.sh` green; engine + WASM build.

## Memory-rule check
- **`project_shared_infra_strategy`:** capture/print is generic; lifts to other
  voxel games. ✅
- **`feedback_uk_english_naming`:** UK spelling. ✅
- Separates cleanly from dyed paper (Spec 35) — pigment décor vs sun-printed
  copy. Magnesium (sibling spec) is fully decoupled (sun-only develop).
