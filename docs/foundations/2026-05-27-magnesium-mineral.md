# Magnesium — mineral, fertiliser, pyrotechnics, firestarter

**Date:** 2026-05-27
**Status:** DESIGN APPROVED — ready to build. PWA/WASM + native.
**Driver:** Staxolottle. Came out of the cyanotype/blueprint conversation
(magnesium was the "UV lamp" idea), but **decoupled** from it — blueprints
develop in sunlight only (see the Blueprint spec), so magnesium stands on its
own as a mineable mineral with its own uses. Text-only design.

## Why magnesium earns a slot

Magnesium isn't filler — it's the atom at the **centre of chlorophyll** (why
plants are green), it's what makes **fireworks** brilliant white, and it's a
real **firestarter**. Three honest, teachable uses from one mineral. The
lightweight-alloy angle is **deferred** (owner call, 2026-05-27).

## Source — a new thing to mine

- **Magnesium Ore** — a new mineable block. Mined with a **stone pickaxe or
  better**, drops 1 **Magnesium**.
- **Where:** a mid-depth band, **more common in Desert + Mountains**. Grounded:
  real magnesium comes from evaporite/salt deposits, so it pairs thematically
  with the existing Rock Salt veins (Salt, 2026-05-23) in those biomes. Exact
  depth band + frequency tuned at build (start ~ iron-band rarity).
- **Magnesium** (the raw mineral) is usable directly — no smelting step for
  v1 (real extraction is electrolysis; we abstract it away).

## Uses & recipes

### 🌱 Fertiliser (the core use)
- **Epsom Salt (Fertiliser)** ← `Magnesium + Sulphur`. Grounded: Epsom salt
  *is* magnesium sulphate, a real garden fertiliser — and we already have
  Sulphur. So the recipe itself teaches the chemistry.
- **Mechanic:** right-click a planted crop with Fertiliser → boosts growth.
  Exact relationship to **Bonemeal** is a build decision — proposal: Bonemeal
  jumps a crop **+1 stage instantly** (quick nudge); Fertiliser **speeds the
  growth *rate*** of crops in a small radius for a while (a "feed the soil"
  effect), so they're complementary not redundant. Works on the whole crop
  family incl. the new cotton/hemp/flowers.

### 🎆 Pyrotechnics — sparklers & flares (no propellant needed)
- **Sparkler** ← `Magnesium + Stick`. Handheld; right-click → a few seconds of
  bright white sparkle particles. Pure celebration — pairs with the dyed-paper
  **bunting/party** line. Magnesium burns white on its own, so **no propellant
  required** (a sparkler really is just metal on a wire).
- **Flare / Signal Light** ← `Magnesium + Papyrus/cloth` (a bound torch).
  Burns brilliant white for a while → a bright temporary light / signal beacon
  (find-your-way-back, mark a spot).
- **Coloured sparkles (Phase 2):** real flame-test chemistry — Copper → green,
  etc. Magnesium + a metal/dye → coloured sparkle. Deferred; lovely teachable
  follow-on.
- **Launching fireworks (deferred — propellant gap):** true fireworks need a
  black-powder propellant. **Gunpowder was removed** in the fantasy-roster
  excision, so launching fireworks need a new propellant first (e.g. a
  **Black Powder** from `Sulphur + Coal + (a saltpetre stand-in)` — needs its
  own design + the saltpetre question answered). Sparklers + flares deliver
  the celebration payoff now **without** reopening explosives; launching
  fireworks are a flagged follow-on.

### 🔥 Firestarter
- **Magnesium Firestarter** ← `Magnesium + Iron Ingot`. A reusable tool that
  lights campfires (and future fire-needing stations) like flint-and-steel —
  an alternative/upgrade. Real magnesium fire-starting rods are a camping
  staple.

## Deferred (explicit)
- **Lightweight alloy** (magnesium = strong-but-light gear) — parked per owner.
- **Launching fireworks** — ~~blocked on a propellant design (gunpowder gone)~~
  **UNBLOCKED 2026-06-20:** the Explosives spec
  (`2026-06-20-explosives-blasting-keg.md`) adds **Black Powder**
  (`Sulphur + Charcoal + Saltpetre`) + sources Saltpetre (mine Nitre / farm via
  Composter). Launching fireworks remain *this* spec's follow-on, but the
  propellant + saltpetre question are now answered there.
- **Coloured sparkles** (flame-test metal salts) — Phase 2.

## Data model (build notes)
- **Block:** `MAGNESIUM_ORE` (append after the current highest BlockId — bincode
  order). Worldgen via the ore-placement path (`biome.rs::ore_at`-style band,
  Desert/Mountains weighting) — coordinate the texture index range with any
  sibling specs building concurrently.
- **Items (`MaterialId`):** `Magnesium`, `Fertiliser` (Epsom salt), `Sparkler`,
  `Flare`, plus the `MagnesiumFirestarter` (likely a Tool, mirroring
  flint-and-steel). Append-only; extend the bincode map + name/colour/
  texture/complexity arms + `ALL_MATERIAL_IDS`.
- **Recipes (`crafting.rs`):** Magnesium+Sulphur→Fertiliser; Magnesium+Stick→
  Sparkler; Magnesium+Papyrus→Flare; Magnesium+Iron→Firestarter.
- **Fertiliser mechanic:** hook into `growth.rs` (crop growth) — either a
  right-click "+stage" like bonemeal or a timed rate-boost in a radius.
- **Sparkler/flare effects:** particle + temporary light; reuse the lighting
  system. Visual, so the *feel* is a playtest item.

## Verification & boundary
- **Unit-testable:** Magnesium Ore drops Magnesium; all four recipes match;
  Fertiliser advances/speeds crop growth; Firestarter lights a campfire.
- **Playtest boundary (Axolittle):** ore find-rate, fertiliser feel vs
  bonemeal, sparkler/flare visuals.
- `check.sh` green; engine + WASM build.

## Memory-rule check
- **`project_axenstax_has_farming`:** Fertiliser ties magnesium to the farming
  tiers (chlorophyll chemistry). ✅
- **`feedback_uk_english_naming`:** UK spelling (fertiliser, colour). ✅
- **`project_shared_infra_strategy`:** generic mineral + mechanics; lifts to
  other voxel games. ✅
- Sibling to the dyed-paper **bunting/party** line (sparklers) and the
  Blueprint/Cyanotype spec (shared origin, now decoupled).
