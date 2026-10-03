# Mob species tint via baked texture layers — ✅ DELIVERED

**Status: ✅ DELIVERED 2026-07-11**, same day as scoping. 47 tinted layers
(`texture_count()` 459 → 506, six layers of headroom under the 512 floor —
no JS-gate change), 9 species (fox, cat, crab, polar bear, reindeer, donkey,
mule, parrot, glow squid), TDD'd (7 new tests) + verified visually via
`--dump-textures` comparison sheets. Deltas vs the plan below:

- **The bake is NOT a straight multiply.** Two formulas failed the visual
  check before the third stuck: (1) RGB multiply can never brighten a dark
  donor — the white polar bear stayed cow-brown; (2) plain luminance
  recolour preserves the donor's brightness (bear mean 127 vs donor 128) —
  the bear read grey-brown. Shipped: **donor grayscale normalized to mean
  luminance 200, then multiplied by `MobDef.color`**
  (`texture_gen::tint_rgba`, `TINT_TARGET_LUM`). Property tests pin both
  failure modes (bear reads white, fox stays saturated orange).
- Legs fold onto the tinted body_side layer (budget); the four villager-tier
  humans are skipped as planned.
- The build-plan steps below were executed as written otherwise.

## TL;DR

`MobDef.color` is loaded from every `data/mobs/*.toml` but **never read by any
render code** (`mob.rs:~225`, flagged `#[allow(dead_code)]`). Every mesh-reuse
species therefore renders in its donor mesh's hardcoded coat: Fox and Cat look
like Wolves, Polar Bear like a brown Bear, Reindeer/Donkey/Mule like Horses,
Crab like a Rabbit, Glow Squid like a Squid, Parrot like a Chicken, and
Brigand/Marauder/Berserker/Knight all like the Villager. The fix: **bake
per-species tinted copies of the donor texture layers** at texture-array build
time and remap each reuse-species' `ModelPart.tex_faces` to them. **No shader,
vertex-format, or pipeline change.**

## Why baked layers (decision, considered alternatives)

The shared `Vertex` (`mesh.rs:20`) has no RGB channel — only scalar
`light`/`sky_light` — and terrain + entities share `shader.wgsl`. The options:

1. **Widen `Vertex` with a `[f32;3]` tint** — pays memory + bandwidth on every
   terrain vertex for a feature only entities use. Rejected.
2. **Separate entity pipeline + vertex format** — proper long-term shape
   (would also unlock dynamic tints, e.g. dyed pets), but a big renderer.rs
   surgery with WASM/WebGPU compat risk. Defer until a feature actually needs
   *dynamic* per-entity tint; `MobDef.color` is static per species.
3. **Bake tinted layers** (CHOSEN) — generate tinted copies of the donor
   layers (multiply RGB by `mob_def(kind).color`), append them to the texture
   array with stable registry keys, and point each reuse-species' model at
   them. Zero shader work, fully unit-testable, resource-pack addressable
   (each tinted layer gets its own key a pack can override with a bespoke
   texture — strictly better for modders than a runtime multiply).

**Device-limit check (do NOT skip):** `texture_count()` is currently **459**
(`texture_gen.rs:~1903`; ledger comment lists the append history). The engine
already exceeds the 256 `max_texture_array_layers` floor — 256-capped devices
get the friendly boot gate (`renderer.rs:~778/~2873`, 2026-07-08 fix). Devices
that pass the gate have ≥ the requested limit (typically 2048), so ~30-40 more
layers introduces **no new compatibility cliff**. Update the drift-guard
comments' `texture_count() == 459` mentions (all four sites were named on
main@f8072150) when the count changes.

## Current facts (verified in code 2026-07-11)

- Mesh-reuse map: `entity_model.rs` `MODEL_CACHE` (~line 230):
  Fox→wolf, Cat→wolf, PolarBear→bear, Reindeer/Donkey/Mule→horse,
  Parrot→chicken, GlowSquid→squid, Crab→rabbit,
  Brigand/Marauder/Berserker/Knight→villager. Each species already gets its
  **own `Vec<ModelPart>` entry** (built by the donor's builder fn), so
  per-species face remapping is a local post-process — no donor is affected.
- Tint values: `data/mobs/<id>.toml` `color = [r, g, b]` (e.g. fox
  `[0.85,0.45,0.15]`, polar_bear `[0.92,0.93,0.95]`, crab `[0.85,0.3,0.2]`,
  glow_squid `[0.20,0.65,0.70]`), reachable via `mob::mob_def(kind).color`.
- `GlowSquid` also has the stay-full-bright carve-out in
  `build_entity_model_vertices` — tint composes with that (tinted layer +
  no light dimming), keep both.
- Registry invariant: `texture_registry.rs` keys are **append-only**, index
  == GPU layer; tests `keys_len_matches_texture_count` +
  `tex_constants_resolve_to_their_keys` enforce the bijection. New layers
  append at the end with keys like `mobs/fox/wolf_body_tinted` (follow the
  existing key naming in the file).

## Build plan (ordered)

1. **Inventory donor layers per reuse species** — collect the distinct
   `TEX_*` constants each donor model's `tex_faces` reference (wolf ~4,
   bear ~4, horse ~4, chicken ~4, squid ~2, rabbit ~3, villager ~5).
   Distinct (species, donor-layer) pairs ≈ 30-40.
2. **texture_gen**: add a `tinted(donor_px, [r,g,b])` helper (per-pixel RGB
   multiply, alpha untouched) + generate the new layers; bump
   `texture_count()` and extend its ledger comment (append-only).
3. **texture_registry**: append the new keys in layer order + `TEX_` consts.
   Existing bijection tests catch any off-by-one loudly.
4. **entity_model**: in `MODEL_CACHE`, wrap the reuse entries with a
   `retint(model, &[(donor_layer, tinted_layer), …])` post-process that
   remaps `tex_faces`. TDD: a test per reuse species asserting its model
   references **no donor coat layer** and **at least one tinted layer**
   (pure — no GPU needed); plus one donor-unchanged guard (Wolf still uses
   wolf layers).
5. **`mob.rs`**: drop the `#[allow(dead_code)]` + stale doc on
   `MobDef.color` (it's read now); correct the `entity_model.rs` comments
   that claimed tint already worked.
6. **Visual verify headless** (the ONLY part needing a GPU): drive the PWA
   via the headless recipe (`reference_headless_gpu_verify_recipe` memory /
   `--use-angle=vulkan`), `/give`-spawn or locate a Fox + Polar Bear,
   screenshot, and eyeball orange-vs-white. Then update the wiki mob pages
   if they hedge about colours.

## Acceptance criteria

- All reuse species' models reference tinted layers; donors unchanged.
- `check.sh` ALL GREEN (clippy -D warnings, bijection tests, bundle gate —
  note: +30 16×16 layers ≈ negligible brotli growth, but confirm).
- Headless screenshot shows Fox ≠ Wolf colour, PolarBear ≠ Bear colour.
- `texture_count()` ledger + drift-guard comments updated consistently.

## Memory-rule check

- Append-only texture registry — never reorder existing keys (resource-pack
  addressing depends on stable indices).
- 256-layer floor already handled by the boot gate; no new gate needed, but
  keep the four drift-guard sites in sync (main@f8072150).
- UK English in any new player-facing text.
