# Terrain overhaul — generator v3 ("dramatic terrain")

Status: §1–§6 and §8 SHIPPED as generator v3 (2026-10-24), **frozen** with v2's caves and decoration
(Spec 02 §5.2, §5.3.8). §7 (dry caves) and §9–§10 (trees, structures, spawn) are NOT built and will land as
the next generator (v4), never as a change to v3. A partial §7 attempt is on branch `overnight/terrain-caves`
(WIP, untested; it edits v3 in place and must be re-targeted at v4). Owner ask (2026-10-23): terrain should feel
closer to modern Minecraft — cliffs, mountains, savannas, deep oceans, rivers, caves; "at the moment AxeNStax
just seems to be a lot of trees".

As-built audit this replaces: `.claude/overnight/worldgen-audit.md` (local, untracked). Summary of today (v2):
one 2D heightmap `62 + amp*noise` with amp 5–14 (p1 50 / p50 62 / p99 70), a 96-block world (y 0..95), biome
chosen by a 2-axis Whittaker grid whose noise spread is too narrow (Forest ≈ 62 % of the world, Desert ≈ 0.1 %),
trees rolled per column with no spacing or clearings, every noise cave flooded (water y 11..54, lava ≤ 10), the
only cliffs are formula switches at biome borders, and no per-world generator version.

## 0. Principles

1. **Old worlds never change.** A world records the generator that made it. v2 worlds keep v2 forever,
   byte-identical (the existing golden hash must not move). New worlds get v3. No seams.
2. **Deterministic everywhere.** Host, joiner and web regenerate the same terrain from the seed: f64 only,
   only `+ - * /`, `sqrt`, `abs`, `floor`, `min/max`, comparisons; no `powf/exp/ln/sin/cos` (use `libm` if trig is
   ever needed); no HashMap iteration feeding block writes; generation stays column-clipped and order-independent.
3. **Cheap.** Noise is sampled on a coarse lattice and interpolated (Minecraft does the same). Target: a v3
   column in ≤ 3 ms release on this laptop and no more than ~2× the v2 cost; measured by an `#[ignore]` bench.
4. **Biome is a label, not a shape.** Height comes from continuous climate fields; biome is derived from the
   same fields afterwards. So there are no artificial walls at biome borders; cliffs come from relief and 3D noise.

## 1. Vertical layout

- World height raised **0..255** (`MAX_CHUNK_Y` 5 → 15, 16 sections; `WORLD_HEIGHT` 256). Global, not per world:
  old worlds just gain sky (and players can build higher). Empty sections are already never stored.
- **Sea level stays 62.** Every underground constant (ore bands, `Y_DP`, `LAVA_CAVE_LEVEL` 10, mineshafts y16..40,
  ravine floors) stays valid. No negative Y (out of scope: signed-cy assumptions across lighting/physics/saves).
- Peaks reach ~200 (cap 240), deep ocean floors ~30, rivers 5 deep.

## 2. Version plumbing (lane T1)

- `WorldMeta` / `WorldSave` gain `worldgen_version: u32`, serde default **2** (append-only invariant: add at the
  end). Every new world is stamped `NEW_WORLD_GENERATOR = 3`; loading an old save yields 2.
- `BiomeGenerator::new(seed)` becomes `BiomeGenerator::new(seed, generator)` (or a `with_generator`); everything
  that already takes `&BiomeGenerator` (terrain_height, biome_at, structures, spawn, snowfall, mobs) dispatches
  inside it. `World::generate_column` dispatches its fill/decoration stages on the same selector.
- `worldgen_fingerprint()` hashes the selected generator too, so v2 and v3 worlds never agree by accident.
- Multiplayer: `JoinAcceptPacket` gains the generator selector (append; protocol bump 93 → 94, logged in the
  protocol history comment and Spec 04). The joiner builds its `BiomeGenerator` with the host's selector.
  The dedicated server reads it from the world's meta.
- Workshop / flat worlds are unaffected (they short-circuit before terrain).

## 3. Climate fields (v3)

All 2D fields use `noise::OpenSimplex` (already a dependency, already accepted for determinism) with new salts,
summed as fBm by hand (`Σ amp_i · n(x·f_i, z·f_i)`, lacunarity 2, gain 0.5), then **normalised** by a constant so
each field's sampled std ≈ 0.35, then clamped to [-1, 1]. (Measure the constants once with a test; hard-code them.)

| field | meaning | octaves | base wavelength (blocks) |
|---|---|---|---|
| C | continentalness (ocean ↔ inland) | 4 | 2048 |
| E | erosion (low = mountainous, high = flat) | 3 | 1024 |
| W | weirdness; PV = 1 − \|3·\|W\| − 2\| (peaks & valleys) | 3 | 512 |
| T | temperature | 2 | 1536 |
| H | humidity | 2 | 1536 |
| R | river field | 2 | 768 |

**Lattice:** 2D fields (and the derived height) are computed at every 4th block (x, z multiples of 4, globally
aligned) and bilinearly interpolated in between. One pure function `height2d(x, z)` does lattice lookup +
interpolation; the column generator uses a per-column cache of the same lattice values, and both paths must
give bit-identical f64 (same formula, same operation order). A small `thread_local` memo of lattice samples is
allowed (lookup only, never iterated).

## 4. Height (2D) — `height2d`, also the v3 `terrain_height`

Piecewise-linear splines (linear interpolation between knots; smoothstep only via polynomials).

- `base(C)` knots: −1 → 28, −0.6 → 34, −0.45 → 44, −0.25 → 52, −0.15 → 58, −0.08 → 62, −0.02 → 65,
  0.2 → 70, 0.6 → 78, 1 → 86.
- `inland = smoothstep(−0.12, 0.05, C)` (0 at sea, 1 inland).
- relief amplitude `A(E)` knots: −1 → 120, −0.6 → 90, −0.3 → 45, 0 → 22, 0.4 → 8, 1 → 4.
- `shape(PV)`: PV ≥ 0 → `PV·sqrt(PV)`; PV < 0 → `0.35·PV`.
- `h = base(C) + inland · A(E) · shape(PV)`, clamped to [6, 240].
- **Rivers:** with `r = |R|`, where `inland > 0` and `r < 0.06` and `h < 110`: `t = r / 0.06`,
  `h = lerp(57, h, smoothstep(0, 1, t))` (bed at 57, water to 62 fills naturally). Width shrinks to 0 between
  h 95 and 110, so rivers don't saw through mountains.

Knots are a starting point. Tune them so the acceptance numbers in §11 hold.

**As tuned (lane T2a, 2026-10-24; full as-built in Spec 02 §5.3.8):** field scale `NORM` 1.539 /
1.378 / 1.345 / 1.336 (1–4 octaves, measured). `base(C)`: −1 → 28, −0.6 → 32, −0.5 → 37, −0.45 → 44,
−0.25 → 52, −0.15 → 58, −0.08 → 62, −0.02 → 65, 0.2 → 67, 0.6 → 72, 1 → 78. `A(E)`: −1 → 160,
−0.6 → 140, −0.3 → 70, 0 → 28, 0.4 → 9, 1 → 4. **Added: escarpments.** A seventh field S (2 octaves,
wavelength 256) adds `44 · clamp01((0.3 − E)/0.5) · smoothstep(−0.008, 0.008, S)` inside the
`inland ·` term: cliff lines along S's zero contour, none on eroded plains. Without it the 2D slope
≥ 6 share was 0.06–0.1 % (a 2D slope of 6 needs a 24-block step between neighbouring lattice
points, which smooth splines never make). River width, bed and the 95/110 fade are as above.

## 5. 3D shape — overhangs and cliffs

`density(x, y, z) = (h − y) + J · n3(x/32, y/24, z/32)`; solid where `density > 0`.
Jaggedness (blocks): `J = inland · (18 · clamp01((−E − 0.2)/0.5) · clamp01(PV) + 2)`, so it is near 18 in mountains and 2 in plains.
`n3` (one OpenSimplex 3D, ×2.6, clamped to [-1, 1]) is sampled on a 4×8×4 lattice (x, z every 4, y every 8, globally aligned) and trilinearly
interpolated. Only the y-band `[min_h − J_max − 2, max_h + J_max + 2]` of a column needs density; below is solid,
above is air. A pure `solid_at(x, y, z)` (lattice lookup + interpolation) must equal the column result
(`terrain_block_at` purity contract).

## 6. Surface rules (top solid block of each column, downward)

Slope `s` = max |height2d(x±1, z) − h|, |height2d(x, z±1) − h| (pure 2D, cheap from the cache).

1. top ≥ snow line (165 + 8·n) → SNOW top over stone.
2. top ≥ 140, or `s ≥ 3` (cliff face) → stone (gravel patches by noise). Exposed cliffs read as rock.
3. Beach: `−0.15 ≤ C ≤ −0.02` (tuned to `−0.12 ≤ C ≤ −0.08`: the wide band made 11 % of land beach) and top in 59..=65 and not mountainous → SAND ×3 over sandstone; cold (T < −0.45)
   → GRAVEL.
4. Underwater (top < 62): ocean/river floor SAND, GRAVEL patches, occasional CLAY if that block exists.
5. Biome default: Desert SAND/sandstone, SnowyTundra SNOW over dirt, else GRASS over 3–4 DIRT.
Ores keep `ore_at` / `base_rock_at` unchanged (bands are absolute Y and sea level did not move).

## 7. Caves (v3) — dry, walkable, a few mouths

- **Cheese** (caverns): `fbm3(x/96, y/64, z/96, 2 oct) > 0.62`, y in 8..min(top − 10, 120).
- **Spaghetti** (tunnels): two 3D fields a, b at (x/48, y/32, z/48); carve where `a² + b² < 0.012`.
  Spaghetti may break the surface on dry land (top ≥ 65), which makes cave mouths.
- Sampled on a 4×4×4 lattice and trilinearly interpolated.
- Never carve within 6 blocks below the top of a wet column (top < 63: ocean, river, lake beds), so the sea
  can't drain into caves.
- Carved cells at y ≤ 10 → LAVA (lava lakes at the bottom); everything else → AIR. No flooded caves in v3.
- Target volume: 5–8 % of the underground (v2 floods ~28 %). The v2 `is_cave` is not used by v3.
- Ravines and mineshafts stay (they key off `terrain_height`, which is `height2d` in v3).

## 8. Biomes (v3 labels)

Order of decision:
1. C < −0.5 → **DeepOcean** (new variant). C < −0.15 → Ocean.
2. River mask active and h < 63 → **River** (new).
3. Beach band (§6.3) → **Beach** (new).
4. h > 120, or (inland and E < −0.35 and PV > 0.4) → Mountains.
5. Otherwise a Whittaker table on T/H. Thresholds are tuned so land (excluding mountains) splits roughly
   Plains 22 %, Forest 18 %, Savanna 12 %, Desert 10 %, Taiga 10 %, SnowyTundra 7 %, BirchForest 6 %,
   Jungle 6 % (±5 points each). As tuned: T edges −0.32 / 0.02 / 0.27; cold: SnowyTundra if H < −0.20
   else Taiga; cool: Plains < −0.13 ≤ Forest < 0.22 ≤ BirchForest; warm: Savanna < −0.25 ≤ Plains
   < 0.07 ≤ Forest < 0.40 ≤ Jungle; hot: Desert < 0 ≤ Savanna < 0.30 ≤ Jungle. Measured (3 seeds,
   8192², every 8): Plains 19.5, Forest 15.9, Savanna 12.3, Desert 10.3, Taiga 10.4, SnowyTundra 7.0,
   BirchForest 6.6, Jungle 6.8, River 6.1, Beach 5.1 % of land excluding mountains.
New variants map onto existing behaviour: DeepOcean and River use Ocean's mobs (fish/squid; sharks only in
DeepOcean if that's a one-line change), Beach uses no trees and Plains' passive table minus large animals.
v2 never produces the new variants.

## 9. Trees and vegetation (v3)

- **Jittered-grid candidates**, not per-column dice. The world is split into square cells of size `S(biome)`;
  each cell has one candidate at a hash-jittered position, accepted with probability `p(biome) · clump`, where
  `clump = 2 · smoothstep(−0.3, 0.5, fbm2(x/96, z/96))` (0..2, giving groves and clearings). Spacing is
  guaranteed, and only candidates evaluate the biome (cheap, and pure for neighbour columns).
- S / p: Jungle 5 / 0.9, Forest 6 / 0.8, BirchForest 6 / 0.8, Taiga 7 / 0.75, Savanna 12 / 0.25 (acacia),
  Plains 12 / 0.12, SnowyTundra 12 / 0.08, Mountains 9 / 0.15 below y 135 (spruce), none above.
  Desert, Ocean, DeepOcean, River and Beach: none.
- No trees on stone, sand or snow surfaces, underwater, or where slope ≥ 3.
- Ground flora stays as-is, on grass only.

## 10. Structures and spawn (v3)

- **Villages:** biome gate unchanged plus a flatness test: sample `height2d` at the anchor and 8 points on
  radius 24. Need max − min ≤ 5 and all ≥ 64, else no village. Each building sits on a levelled pad: origin y =
  `height2d` at the building centre; fill DIRT (or the cobble/planks the plans use) from the ground up to
  floor − 1 inside the footprint; clear AIR from the floor up to plan height + 2. Column-clipped like the plans.
- **Brigand hideouts:** the same flatness test (radius 12); palisade posts at their own cell's y.
- **Spawn:** for v3 worlds, spiral outward from the origin (up to 64 chunks) to the first column that is dry,
  not Ocean/River/Beach, has h in 63..=110, slope < 2, and real ground with headroom. Raise
  `ground_with_headroom`'s scan start to `terrain_height + 32` (overhangs).
- `SpawnPref` finders work unchanged on labels. Their tests may need larger radii.

## 11. Acceptance (measured by tests; numbers are for v3, over ≥ 3 seeds)

- Legacy: the existing v2 golden hash and every v2 test are unchanged.
- v3 golden plus order-independence tests (row-major / reversed / scattered), same pattern as v2.
- Height (dry land): p50 66–78, p99 ≥ 150, max ≥ 180. ≥ 25 % of the world by area is Ocean or DeepOcean;
  deep-ocean floors ≤ 40.
- Steep: ≥ 1.5 % of land columns have slope ≥ 6 (cliffs).
- Biome shares within the §8 targets ±5 points; no land biome above 30 %.
- Trees: Forest 4–9 per chunk, Plains ≤ 1.5, Savanna ≤ 2.5; ≥ 15 % of Forest chunks have ≤ 2 trees (clearings).
- Caves: 5–8 % of underground volume carved; 0 water cells inside caves; at least one cave mouth per
  16×16-chunk area on average.
- A spawn is found for every test seed in ≤ 64 chunks.
- Bench: v3 column ≤ 3 ms release (report the measured v2 and v3 numbers).
- A map tool writes PNG biome and height maps for a seed (`#[ignore]` test using the `image` crate), so a person
  can see the result: `.claude/overnight/maps/`.

## 12. Out of scope tonight (follow-ups)

Negative Y; aquifers (water pockets in caves); Badlands, Meadow, cherry and other new biomes needing new blocks;
noodle caves; biome-tinted grass (the `grass_tint` field exists with no consumer); threaded generation (Spec 02
§5.4); ravines and mineshafts respecting caves/fluids; ore veins.
