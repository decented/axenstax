# Biomes

A biome is a chunk of the world with its own look, surface block, and tree mix. Walk far enough in any direction and you'll cross a biome border.

> **Status (2026-05-27):** the classifier is now **live in world generation** — new worlds use all 10 biomes. The 8 climate biomes below come from the Whittaker classifier; **Mountains** and **Ocean** are decided separately by elevation (see "How biome selection works"). Make a *new* world to see them — existing saves keep their already-generated terrain.

## The 8 launch biomes

| Biome | Surface | Trees | Vibe |
|---|---|---|---|
| **Plains** | Grass | Sparse Oak | Open, sunny, low-density. The default starting biome. |
| **Forest** | Grass | Oak + Birch mix | Dense canopy, mixed-species — feels lush. |
| **Birch Forest** | Grass | Birch only | Tall pale trees, light canopy. |
| **Taiga** | Grass | Spruce | Cool, conifer-heavy, darker. |
| **Jungle** | Grass | Jungle + Rubber (50/50) | Hot + wet + densest tree cover in the game. |
| **Savanna** | Grass | Acacia (sparse) | Hot + dry, golden-tinted grass, umbrella trees. |
| **Desert** | Sand | None | Hot, treeless, sandstone underneath. |
| **Snowy Tundra** | Snow | Sparse Spruce | Cold, treeless except for tough spruces. |

## How biome selection works

First the engine checks **elevation** (a broad "continentalness" noise):
very low → **Ocean**, very high → **Mountains**. These two are
terrain-shape biomes — decided by the lie of the land, not the climate.
Keeping them separate is what frees the cold climates to become Taiga /
Snowy Tundra instead of every cold spot turning into mountains.

Everywhere in between is normal land, where **climate** decides. The
engine samples two noise values at the position:

- **Temperature** — cold (below -0.4) / cool / warm / hot (above 0.5)
- **Humidity** — dry (below -0.2) / mid / wet (above 0.3)

It then looks up the (temperature, humidity) pair in a 4×3 Whittaker climate grid:

```
              dry         mid          wet
cold      Snowy Tundra   Taiga        Taiga
cool      Plains         Forest       Birch Forest
warm      Savanna        Forest       Jungle
hot       Desert         Savanna      Jungle
```

The Whittaker grid is a real climate-science model — same approach used to map Earth biomes. We've simplified it down to 8 buckets to keep the surface manageable on alpha.

## Mixed forests (biome blending)

The engine has blending code that samples 5 positions (yours + 4 neighbours at ±16 blocks) and weights trees across a biome border — but **it isn't wired into live world generation yet**, so right now biome borders are hard: trees snap from one biome's pool to the other's exactly at the border, no gradient. The blending logic only runs in its own unit tests today. "Mixed forests" inside a single biome (like Forest's Oak + Birch mix) still work fine — it's just the cross-border blend that isn't live.

The visual surface block (grass / sand / snow) doesn't blend either — that would look like checkerboard noise. That part's by design.

## Tree species per biome

This connects to Spec 28b Wood Species (6 species coded, live tree placement in the next update):

- **Plains, Forest** → Oak (familiar default)
- **Forest** also → Birch (mixed)
- **Birch Forest** → Birch only
- **Taiga, Snowy Tundra** → Spruce (cold-tolerant conifers)
- **Jungle** → Jungle (tall, dense) + Rubber, a deliberate 50/50 mix (Rubber trees are tappable, so this makes sure you'll run into some)
- **Savanna** → Acacia (umbrella canopy)
- **Desert** → no trees

Dark Oak doesn't have a biome yet — it'll land with a future "Dark Forest" / Roofed Forest biome.

## Now live (2026-05-27)

- All 10 biomes generate — climate biomes via the Whittaker classifier, Mountains + Ocean by elevation
- Per-biome surface blocks placed (grass / sand / snow)
- Trees placed per biome from each biome's species pool (borders are currently hard, not blended — see "Mixed forests" above)
- Mob spawn weights active — animals appear in their biomes (Nostrich in Savanna, etc.)
- Ground vegetation scatters: tall grass + berry bushes on grassland, papyrus reeds along the waterline

## Underground & rare features

Worldgen also scatters a handful of structures you can stumble into while exploring or mining:

- **Ravines** — rare canyon-like gashes cut into the terrain, sometimes with a lava floor. Watch your footing near the edges.
- **Abandoned Mineshafts** — underground tunnel structures with loot chests tucked inside. Worth a detour if you spot one while caving.
- **Brigand Hideouts** — found away from villages in Plains, Forest, Savanna, and Taiga biomes. Some are tougher "boss" hideouts.
- **Rock Salt veins** — a mineable ore found only in Mountains and Desert biomes.
- **Deepslate** — below Y30 the stone substrate transitions to deepslate, a tougher variant of regular stone.

## Still coming

- Per-biome sky/fog tint
- A Dark Forest biome to give Dark Oak a home
- Animals respawning over time (today they appear as you explore fresh ground)

See `docs/foundations/2026-05-20-biomes-and-mixed-forests.md` for the full spec.
