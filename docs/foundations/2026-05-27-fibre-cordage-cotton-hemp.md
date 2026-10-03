# Fibre & Cordage — Cotton, Hemp, Wool → String + Rope

**Date:** 2026-05-27
**Status:** PHASE 1 DELIVERED 2026-05-27 (coordinated with Spec 35). Cotton
(block 134, wild in Savanna) breaks to Cotton → **String** (2 per), and the
`Wool → String` stopgap is **retired**. Hemp (block 135, wild in Plains/Forest)
breaks to Hemp Fibre → **Rope**. Textures 223-224 (plants) + 228-230 (cotton/
hemp-fibre/rope items). Wool unchanged. **Deferred:** **farmable** planting
(seeds + growth stages — currently wild-harvest only); the **3-fibre→1-rope**
twist recipe (currently 1:1, tunable); Rope's **Lead/tether** consumer + Cloth/
Canvas (Phase 2). PWA/WASM + native.
**Driver:** Staxolottle. Sibling to the dye system (came out of the same
2026-05-27 conversation). Text-only design.

## Origin

When the Spider was removed (fantasy-roster excision, 2026-05-24), **String**
lost its source and was patched onto a `Wool → String` recipe — always a
stopgap, never an honest one. The fix: give fibre its own crop line. Two new
fibre crops join Wool to form a **fibre triangle**, each with a distinct
character and product, and **Rope** arrives as a new heavier cordage item.

## The fibre triangle

| Fibre | Source | Character | Primary product |
|---|---|---|---|
| **Cotton** | Crop (warm biomes) | Soft, fine | **String** (+ cloth later) |
| **Hemp** | Crop (temperate biomes) | Strong, coarse | **Rope** (+ canvas later) |
| **Wool** | Sheep (existing) | Warm, animal | Wool blocks + dyeing (unchanged) |

They don't overlap: soft vs strong vs warm; plant vs plant vs animal; warm
biome vs temperate biome vs anywhere sheep graze. Cotton becomes the honest
String source (**the `Wool → String` patch is retired**); Hemp owns Rope;
Wool goes back to being just wool.

**Climate-authenticity is explicitly NOT a constraint** (owner call,
2026-05-27): the world already has savannas, ostriches and hyenas, so
real-world "you can't grow cotton in Britain" purity doesn't apply. Crops are
themed to the game's biomes, not to the UK. (UK *spelling* still applies —
"fibre", "colour".) This is why we chose cotton over flax/linen; flax stays
available as a future temperate fibre if ever wanted.

## Crops

Both are proper farmable crops, mirroring the existing wheat/sugarcane
pattern, and also generate wild so the player can find their first seeds:

- **Cotton** — seed + 4 growth stages → mature bolls drop **Cotton** (fibre)
  + 1 seed. Grows best in / wild-scatters in **warm biomes (Savanna)** via
  the `place_vegetation` pass.
- **Hemp** — seed + 4 growth stages → mature stalk drops **Hemp Fibre** +
  1 seed. Grows best in / wild-scatters in **temperate biomes (Plains,
  Forest)**.

Biome-gating the two crops makes fibre a reason to travel: you can't pull
fine string and strong rope out of the same field.

## Products & recipes

- **Cotton → String.** Shapeless: 1 Cotton → 1 String. (Replaces the
  `Wool → String` recipe — remove it; Wool no longer crafts to String.)
- **Hemp Fibre → Rope.** Rope is substantial cordage: 3 Hemp Fibre in a
  vertical column → 1 Rope (twisting fibres into a rope), or a tuned ratio
  at build. Quantity > String so rope feels "heavier".
- **Later (Phase 2):** Cotton → **Cloth** (fine textile) and Hemp →
  **Canvas** (coarse) — feed future bags/sails/banners. Out of scope here.

### Rope's uses
- **Leads / tethering** — tie a tamed Wolf (and future tameable animals) to
  a post so it stays put; the immediate near-term consumer. *(A `Lead` item
  + tether mechanic is a small follow-on — flag if it should land in this
  spec's Phase 1 or its own.)*
- **Future:** scaffolding / climbing, fences-with-gates, decorative rigging.
- Rope is a **cordage primitive** — like String, its value compounds as
  consumers land. Phase 1 ships the fibre→rope chain; the first rope
  consumer (Lead) follows close behind so rope isn't inert on arrival.

## Data model (build notes)

- **Blocks:** cotton crop stages (×4) + hemp crop stages (×4) as
  transparent/non-solid crop blocks; append after the current highest
  BlockId (preserve bincode order — same discipline as every block wave).
  Wild-scatter mature stages in `World::place_vegetation`, biome-gated.
- **Items (`MaterialId`):** `CottonSeeds`, `Cotton`, `HempSeeds`,
  `HempFibre`, `Rope`. `String` already exists. Trade/complexity/food values
  per the farming-economy ladder (fibre = non-food, `food_value: None`).
- **Recipes (`crafting.rs`):** Cotton→String; HempFibre→Rope; **remove**
  the `Wool → String` arm. Wire growth + planting through the existing crop
  framework (tilled soil, light-gated growth).
- **Textures:** crop stages (×8) + Cotton + Hemp Fibre + Rope + 2 seeds,
  appended at the end of `texture_gen` (after the dye-system textures if that
  ships first — coordinate the index ranges).
- **Retire:** the `Wool → String` recipe + its test; add a migration note
  (alpha string in old saves is fine — it's still a valid item, just no
  longer wool-craftable).

## Verification & boundary

- **Unit-testable:** Cotton→String and HempFibre→Rope recipes match; the
  `Wool → String` recipe no longer matches; crop growth stages advance;
  wild scatter places cotton only in warm biomes and hemp only in temperate.
- **Playtest boundary (Axolittle):** crop spawn density, growth feel, whether
  the cotton/hemp/wool split reads clearly, and rope's first real use.
- `check.sh` green; engine + WASM build.

## Build phasing

- **Phase 1 — the fibre triangle.** Cotton + Hemp crops (blocks, seeds,
  growth, wild scatter, biome gating) + Cotton→String + HempFibre→Rope +
  retire Wool→String + textures.
- **Phase 2 — consumers + textiles.** Lead/tether item for Rope; Cloth
  (cotton) + Canvas (hemp) and their downstream uses.

## Memory-rule check

- **`feedback_uk_english_naming`:** UK spelling (fibre, colour); crop *theme*
  is global-biome, not UK-climate — explicit owner call, see Origin. ✅
- **`project_axenstax_has_farming`:** cotton + hemp slot into the farming
  tiers as fibre crops; biome-gated growth fits the tier system. ✅
- **`project_shared_infra_strategy`:** fibre/cordage is generic content,
  lifts cleanly to other voxel games. ✅
- Sibling to [`2026-05-27-flowers-dyes-colour-mixing.md`](2026-05-27-flowers-dyes-colour-mixing.md)
  (both came from the same conversation; both extend `place_vegetation` +
  the crop framework). Sequence them so their block/texture index ranges
  don't collide.
