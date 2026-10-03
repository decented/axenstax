//! Texture-key registry — the name↔index authority for the block texture array.
//!
//! Spec 03 §2: "Block textures are referenced by **name**, never by pixel
//! coordinate." This module is the foundation that makes that true. Every layer
//! produced by [`crate::texture_gen::generate_textures`] gets a **stable string
//! key** here, and the bijection key↔layer-index is the address packs use:
//! a texture pack says "override `blocks/stone`" and the loader resolves that to
//! the layer whose pixels to replace ([`texture_index`]); the `--dump-textures`
//! dev tool names each PNG by its key ([`texture_key`]).
//!
//! This is texture-pack spec PHASE 1 (`docs/foundations/2026-06-18-texture-pack-authoring.md`).
//! It is **behaviour-neutral**: it adds a naming layer over the existing layer
//! indices and changes nothing the renderer does today.
//!
//! ## BRIDGE — parallel key list
//! [`TEXTURE_KEYS`] is authored in lock-step with the push order in
//! `texture_gen::generate_textures()`. The two are bound by tests
//! (`keys_len_matches_texture_count`, `tex_constants_resolve_to_their_keys`) so
//! drift is caught, but they are two lists. **Unify** them into one keyed table
//! (the generator yields `(key, pixels)` pairs) when P2 restructures the atlas
//! build to resolve-then-upload — that pass already rewrites this region.

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Stable string key for every texture array layer, in layer-index order.
///
/// Index into this slice == the GPU texture-array layer index. Keys follow the
/// Spec 03 §11 directory taxonomy (`blocks/…`, `item/…`, `entity/<mob>/…`,
/// `overlay/…`, `decor/<family>/<colour>`); `_retired/<n>` marks a vestigial
/// filler layer kept only to hold later indices stable.
pub fn texture_keys() -> &'static [&'static str] {
    TEXTURE_KEYS
}

/// Resolve a texture key to its layer index (the address packs override).
pub fn texture_index(key: &str) -> Option<u32> {
    TEXTURE_KEYS.iter().position(|k| *k == key).map(|i| i as u32)
}

/// Resolve a layer index to its stable key. Despite the module doc's original
/// claim, `dump_textures` below doesn't actually call this — it already has the
/// key string in hand from `keys.iter().zip(pixels.iter())` — so this pure
/// inverse of [`texture_index`] is currently exercised only by the round-trip
/// tests. No production caller yet.
#[cfg_attr(not(test), allow(dead_code))]
pub fn texture_key(index: u32) -> Option<&'static str> {
    TEXTURE_KEYS.get(index as usize).copied()
}

/// Dump every procedural layer to `<out_dir>/<key>.png` (16×16 RGBA), creating
/// `key`'s sub-directories. This produces the canonical **default pack** — the
/// editable PNG set an author edits and reloads (texture-pack spec P1/P2). Native
/// dev tool; returns the number of PNGs written.
#[cfg(not(target_arch = "wasm32"))]
pub fn dump_textures(out_dir: &Path) -> std::io::Result<usize> {
    use std::io::{Error, ErrorKind};
    let pixels = crate::texture_gen::generate_textures();
    let keys = texture_keys();
    if keys.len() != pixels.len() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("key count {} != layer count {}", keys.len(), pixels.len()),
        ));
    }
    for (key, buf) in keys.iter().zip(pixels.iter()) {
        let path = out_dir.join(format!("{key}.png"));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Default pack is 16×16 RGBA (Spec 03 §11 v1 resolution). `from_raw`
        // returns None if the buffer isn't exactly 16·16·4 bytes.
        let img = image::RgbaImage::from_raw(16, 16, buf.clone())
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, format!("layer {key} not 16×16 RGBA")))?;
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| Error::other(e.to_string()))?;
        std::fs::write(&path, png)?;
    }
    // Make the dumped set a self-describing pack (Spec 03 §11.2): a default
    // 16×16 manifest the author can rename / re-resolution.
    let manifest = PackManifest::fallback(pack_name_from_dir(out_dir));
    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| Error::other(e.to_string()))?;
    std::fs::write(out_dir.join("pack.json"), json)?;
    Ok(keys.len())
}

// BRIDGE: this list is authored in lock-step with `texture_gen::generate_textures`'s
// push order. Unify into a single keyed table in P2 (the atlas-rebuild rewrite).
// The 16-colour décor order is the canonical dye order:
//   white, black, red, blue, yellow, orange, green, purple, pink, lime,
//   light_blue, grey, light_grey, brown, cyan, magenta.
#[rustfmt::skip]
static TEXTURE_KEYS: &[&str] = &[
    // 0-15 — core terrain blocks
    "blocks/stone", "blocks/dirt", "blocks/grass_top", "blocks/grass_side",
    "blocks/bedrock", "blocks/sand", "blocks/oak_log_side", "blocks/oak_log_top",
    "blocks/oak_leaves", "blocks/oak_planks", "blocks/cobblestone", "blocks/water",
    "blocks/gravel", "blocks/sandstone", "blocks/snow", "blocks/crafting_table_top",
    // 16-22 — cow
    "entity/cow/head_front", "entity/cow/head_side", "entity/cow/head_top",
    "entity/cow/body_side", "entity/cow/body_top", "entity/cow/body_end", "entity/cow/leg",
    // 23-29 — retired fillers
    "_retired/23", "_retired/24", "_retired/25", "_retired/26", "_retired/27",
    "_retired/28", "_retired/29",
    // 30-33 — chicken
    "entity/chicken/head", "entity/chicken/body", "entity/chicken/leg", "entity/chicken/wing",
    // 34-40 — pig
    "entity/pig/head_front", "entity/pig/head_side", "entity/pig/head_top",
    "entity/pig/body_side", "entity/pig/body_top", "entity/pig/body_end", "entity/pig/leg",
    // 41-47 — sheep
    "entity/sheep/head_front", "entity/sheep/head_side", "entity/sheep/head_top",
    "entity/sheep/body_side", "entity/sheep/body_top", "entity/sheep/body_end", "entity/sheep/leg",
    // 48-66 — retired fillers
    "_retired/48", "_retired/49", "_retired/50", "_retired/51", "_retired/52", "_retired/53",
    "_retired/54", "_retired/55", "_retired/56", "_retired/57", "_retired/58", "_retired/59",
    "_retired/60", "_retired/61", "_retired/62", "_retired/63", "_retired/64", "_retired/65",
    "_retired/66",
    // 67-83 — item drops (72, 78 retired)
    "item/stick", "item/leather", "item/feather", "item/wool", "item/bone", "_retired/72",
    "item/raw_beef", "item/raw_porkchop", "item/raw_chicken", "item/raw_mutton", "item/string",
    "_retired/78", "item/grey_powder", "item/tool_wood", "item/tool_stone", "item/tool_iron",
    "item/tool_diamond",
    // 84-85 — bed
    "blocks/bed_top", "blocks/bed_side",
    // 86-88 — ores
    "blocks/coal_ore", "blocks/iron_ore", "blocks/diamond_ore",
    // 89-92 — retired fillers
    "_retired/89", "_retired/90", "_retired/91", "_retired/92",
    // 93 — glass
    "blocks/glass",
    // 94-96 — storage blocks
    "blocks/coal_block", "blocks/iron_block", "blocks/diamond_block",
    // 97 — torch
    "blocks/torch",
    // 98-102 — smelting outputs
    "item/iron_ingot", "item/cooked_beef", "item/cooked_porkchop", "item/cooked_chicken",
    "item/cooked_mutton",
    // 103 — tall grass
    "blocks/tall_grass",
    // 104 — reserved placeholder slot
    "_placeholder/104",
    // 105 — arrow (item + projectile)
    "item/arrow",
    // 106-111 — deepslate + satori
    "blocks/pure_deepslate", "blocks/deepslate_coal_ore", "blocks/deepslate_iron_ore",
    "blocks/deepslate_diamond_ore", "blocks/satori_block", "item/satori",
    // 112-129 — farming T1 (soil, item icons, crop stages)
    "blocks/tilled_soil", "item/wheat_seeds", "item/wheat", "item/bread", "item/carrot",
    "item/potato",
    "blocks/wheat_stage_0", "blocks/wheat_stage_1", "blocks/wheat_stage_2", "blocks/wheat_stage_3",
    "blocks/carrot_stage_0", "blocks/carrot_stage_1", "blocks/carrot_stage_2", "blocks/carrot_stage_3",
    "blocks/potato_stage_0", "blocks/potato_stage_1", "blocks/potato_stage_2", "blocks/potato_stage_3",
    // 130-135 — campfire + flint
    "blocks/campfire_lit_top", "blocks/campfire_lit_side", "blocks/campfire_unlit_top",
    "blocks/campfire_unlit_side", "item/flint", "item/flint_and_steel",
    // 136-145 — campfire smoke + corn
    "blocks/campfire_smoke",
    "blocks/corn_stage_0", "blocks/corn_stage_1", "blocks/corn_stage_2", "blocks/corn_stage_3",
    "item/corn_seeds", "item/corn", "item/baked_corn", "item/baked_potato", "item/baked_carrot",
    // 146-157 — villager + wandering villager (150-153 retired)
    "entity/villager/head_front", "entity/villager/head_side", "entity/villager/body",
    "entity/villager/leg",
    "_retired/150", "_retired/151", "_retired/152", "_retired/153",
    "entity/wandering_villager/head_front", "entity/wandering_villager/head_side",
    "entity/wandering_villager/body", "entity/wandering_villager/leg",
    // 158-159 — drying rack
    "blocks/drying_rack_top", "blocks/drying_rack_side",
    // 160-165 — papyrus
    "blocks/papyrus_stage_0", "blocks/papyrus_stage_1", "blocks/papyrus_stage_2",
    "blocks/papyrus_stage_3", "item/papyrus_reed", "item/papyrus_sheet",
    // 166-170 — build schematics
    "blocks/blueprint_paper_top", "blocks/blueprint_paper_side", "blocks/construction_anchor",
    "blocks/architect_plaque_top", "blocks/architect_plaque_side",
    // 171-173 — furnace
    "blocks/furnace_top", "blocks/furnace_side_unlit", "blocks/furnace_side_lit",
    // 174-176 — deepslate variants
    "blocks/pure_deepslate_thin", "blocks/pure_deepslate_healthy", "blocks/pure_deepslate_fat",
    // 177-179 — vendor, drafting table, smoke warning
    "blocks/vendor_block", "blocks/drafting_table", "blocks/campfire_smoke_warning",
    // 180-181 — chest
    "blocks/chest_top", "blocks/chest_side",
    // 182-183 — brigand banner, trophy wall
    "blocks/brigand_hideout_banner", "blocks/trophy_wall",
    // 184-189 — salt
    "blocks/rock_salt", "blocks/salt_lick", "blocks/salt_lamp", "blocks/salt_block",
    "blocks/salt_path_top", "blocks/salt_path_side",
    // 190-193 — rubber
    "blocks/rubber_log", "blocks/rubber_planks", "blocks/rubber_leaves", "blocks/rubber_log_tapped",
    // 194-200 — economy blocks
    "blocks/bounty_board", "blocks/tip_jar", "blocks/repair_bench", "blocks/plot_marker",
    "blocks/market_bell", "blocks/auction_block", "blocks/bazaar_block",
    // 201-206 — retired player skin fillers
    "_retired/201", "_retired/202", "_retired/203", "_retired/204", "_retired/205", "_retired/206",
    // 207-216 — block-break crack overlay (stages 0-9)
    "overlay/crack_0", "overlay/crack_1", "overlay/crack_2", "overlay/crack_3", "overlay/crack_4",
    "overlay/crack_5", "overlay/crack_6", "overlay/crack_7", "overlay/crack_8", "overlay/crack_9",
    // 217-219 — bee + squid
    "entity/bee/body", "entity/bee/head", "entity/squid/body",
    // 220-224 — dye flowers + fibre plants
    "blocks/cornflower", "blocks/field_poppy", "blocks/buttercup", "blocks/cotton_plant",
    "blocks/hemp_plant",
    // 225-230 — dye + fibre item icons
    "item/blue_dye", "item/red_dye", "item/yellow_dye", "item/cotton", "item/hemp_fibre", "item/rope",
    // 231-236 — magnesium ore + item icons
    "blocks/magnesium_ore", "item/magnesium", "item/fertiliser", "item/sparkler", "item/flare",
    "item/firestarter",
    // 237-246 — dye phase 2 item icons
    "item/black_dye", "item/white_dye", "item/orange_dye", "item/green_dye", "item/purple_dye",
    "item/pink_dye", "item/lime_dye", "item/light_blue_dye", "item/grey_dye", "item/light_grey_dye",
    // 247-254 — fibre phase 2 crop stages (cotton, hemp)
    "blocks/cotton_stage_0", "blocks/cotton_stage_1", "blocks/cotton_stage_2", "blocks/cotton_stage_3",
    "blocks/hemp_stage_0", "blocks/hemp_stage_1", "blocks/hemp_stage_2", "blocks/hemp_stage_3",
    // 255-256 — fibre seeds
    "item/cotton_seeds", "item/hemp_seeds",
    // 257-269 — wallpaper (first 13 dye colours)
    "decor/wallpaper/white", "decor/wallpaper/black", "decor/wallpaper/red", "decor/wallpaper/blue",
    "decor/wallpaper/yellow", "decor/wallpaper/orange", "decor/wallpaper/green",
    "decor/wallpaper/purple", "decor/wallpaper/pink", "decor/wallpaper/lime",
    "decor/wallpaper/light_blue", "decor/wallpaper/grey", "decor/wallpaper/light_grey",
    // 270-272 — dye phase 2 completion (brown, cyan, magenta)
    "item/brown_dye", "item/cyan_dye", "item/magenta_dye",
    // 273-275 — wallpaper (last 3 dye colours)
    "decor/wallpaper/brown", "decor/wallpaper/cyan", "decor/wallpaper/magenta",
    // 276-278 — textile items
    "item/lead", "item/cloth", "item/canvas",
    // 279-290 — farmable-flower crop stages (3 each) + seeds
    "blocks/cornflower_stage_0", "blocks/cornflower_stage_1", "blocks/cornflower_stage_2",
    "blocks/field_poppy_stage_0", "blocks/field_poppy_stage_1", "blocks/field_poppy_stage_2",
    "blocks/buttercup_stage_0", "blocks/buttercup_stage_1", "blocks/buttercup_stage_2",
    "item/cornflower_seeds", "item/field_poppy_seeds", "item/buttercup_seeds",
    // 291 — cyanotype print
    "blocks/cyanotype_print",
    // 292-307 — bunting (16 dye colours)
    "decor/bunting/white", "decor/bunting/black", "decor/bunting/red", "decor/bunting/blue",
    "decor/bunting/yellow", "decor/bunting/orange", "decor/bunting/green", "decor/bunting/purple",
    "decor/bunting/pink", "decor/bunting/lime", "decor/bunting/light_blue", "decor/bunting/grey",
    "decor/bunting/light_grey", "decor/bunting/brown", "decor/bunting/cyan", "decor/bunting/magenta",
    // 308-323 — paper lantern (16 dye colours)
    "decor/lantern/white", "decor/lantern/black", "decor/lantern/red", "decor/lantern/blue",
    "decor/lantern/yellow", "decor/lantern/orange", "decor/lantern/green", "decor/lantern/purple",
    "decor/lantern/pink", "decor/lantern/lime", "decor/lantern/light_blue", "decor/lantern/grey",
    "decor/lantern/light_grey", "decor/lantern/brown", "decor/lantern/cyan", "decor/lantern/magenta",
    // 324-339 — kite (16 dye colours)
    "decor/kite/white", "decor/kite/black", "decor/kite/red", "decor/kite/blue",
    "decor/kite/yellow", "decor/kite/orange", "decor/kite/green", "decor/kite/purple",
    "decor/kite/pink", "decor/kite/lime", "decor/kite/light_blue", "decor/kite/grey",
    "decor/kite/light_grey", "decor/kite/brown", "decor/kite/cyan", "decor/kite/magenta",
    // 340-355 — banner (16 dye colours)
    "decor/banner/white", "decor/banner/black", "decor/banner/red", "decor/banner/blue",
    "decor/banner/yellow", "decor/banner/orange", "decor/banner/green", "decor/banner/purple",
    "decor/banner/pink", "decor/banner/lime", "decor/banner/light_blue", "decor/banner/grey",
    "decor/banner/light_grey", "decor/banner/brown", "decor/banner/cyan", "decor/banner/magenta",
    // 356-371 — sail (16 dye colours)
    "decor/sail/white", "decor/sail/black", "decor/sail/red", "decor/sail/blue",
    "decor/sail/yellow", "decor/sail/orange", "decor/sail/green", "decor/sail/purple",
    "decor/sail/pink", "decor/sail/lime", "decor/sail/light_blue", "decor/sail/grey",
    "decor/sail/light_grey", "decor/sail/brown", "decor/sail/cyan", "decor/sail/magenta",
    // 372-376 — tent, track, ladder, carpet, grave
    "blocks/tent", "blocks/track", "blocks/ladder", "blocks/carpet", "blocks/grave",
    // 377-384 — tiered storage chests
    "blocks/copper_chest_top", "blocks/copper_chest_side", "blocks/iron_chest_top",
    "blocks/iron_chest_side", "blocks/diamond_chest_top", "blocks/diamond_chest_side",
    "blocks/satori_chest_top", "blocks/satori_chest_side",
    // 385-399 — electricity / power blocks
    "blocks/cable", "blocks/cable_lit", "blocks/electric_lamp", "blocks/electric_lamp_lit",
    "blocks/lever", "blocks/button", "blocks/pressure_plate", "blocks/logic_gate",
    "blocks/hand_crank", "blocks/steam_generator", "blocks/steam_generator_lit", "blocks/battery",
    "blocks/beam_sensor", "blocks/mirror", "blocks/motion_sensor",
    // Spec 49 (Explosives) — 400..=404, lock-step with generate_textures.
    "blocks/brimstone", "blocks/nitre_ore", "blocks/composter",
    "blocks/blasting_keg", "blocks/plunger_detonator",
    // P-bugfix (Workshop paint) — 405..=420, the 16 flat solid paint-colour
    // layers in `block::PAINT_WALLPAPERS` order (lock-step with generate_textures).
    "paint/white", "paint/black", "paint/red", "paint/blue",
    "paint/yellow", "paint/orange", "paint/green", "paint/purple",
    "paint/pink", "paint/lime", "paint/light_blue", "paint/grey",
    "paint/light_grey", "paint/brown", "paint/cyan", "paint/magenta",
    // #130 — per-species leaf textures, 421..=425 (lock-step with generate_textures).
    "blocks/birch_leaves", "blocks/spruce_leaves", "blocks/jungle_leaves",
    "blocks/acacia_leaves", "blocks/dark_oak_leaves",
    // #132 — per-species wood textures, 426..=440 (log_side, log_top, planks per
    // species; lock-step with generate_textures).
    "blocks/birch_log_side", "blocks/birch_log_top", "blocks/birch_planks",
    "blocks/spruce_log_side", "blocks/spruce_log_top", "blocks/spruce_planks",
    "blocks/jungle_log_side", "blocks/jungle_log_top", "blocks/jungle_planks",
    "blocks/acacia_log_side", "blocks/acacia_log_top", "blocks/acacia_planks",
    "blocks/dark_oak_log_side", "blocks/dark_oak_log_top", "blocks/dark_oak_planks",
    // Satoshi the founder-sage (441..=443) — hooded head + glow trim.
    "entity/satoshi/head_front", "entity/satoshi/head_side", "entity/satoshi/trim",
    // Rail auto-connect (2026-07-02) — east-west rail (444, lock-step with
    // generate_textures / TEX_TRACK_EW).
    "blocks/track_ew",
    // Rail/cable v2 solid metal layers (445..=447), lock-step with generate_textures.
    "blocks/rail_steel", "blocks/cable_copper", "blocks/cable_copper_lit",
    // Rail sleeper/ballast bed (448), lock-step with generate_textures.
    "blocks/rail_base",
    // Fire (449), lock-step with generate_textures / TEX_FIRE.
    "blocks/fire",
    // Dispenser/Dropper muzzle faces (450..=451), lock-step with generate_textures.
    "blocks/dispenser_side", "blocks/dropper_side",
    // Sapling (452), lock-step with generate_textures / TEX_SAPLING.
    "blocks/sapling",
    // Particle billboards (453..=456), lock-step with generate_textures.
    "particles/soft", "particles/spark", "particles/streak", "particles/chip",
    // Pet Bed top/side (457..=458), lock-step with generate_textures.
    "blocks/pet_bed_top", "blocks/pet_bed_side",
    // Species-tint run (459..=505, mob-species-tint foundation 2026-07-11),
    // lock-step with `texture_gen::SPECIES_TINTS` — per-species tinted donor
    // coats so mesh-reuse mobs stop rendering in their donor's colours. A
    // resource pack can override any of these with a fully bespoke texture.
    // fox (459..=464)
    "entity/fox/body_end", "entity/fox/body_side", "entity/fox/body_top",
    "entity/fox/head_front", "entity/fox/head_side", "entity/fox/head_top",
    // cat (465..=470)
    "entity/cat/body_end", "entity/cat/body_side", "entity/cat/body_top",
    "entity/cat/head_front", "entity/cat/head_side", "entity/cat/head_top",
    // crab (471..=476)
    "entity/crab/body_end", "entity/crab/body_side", "entity/crab/body_top",
    "entity/crab/head_front", "entity/crab/head_side", "entity/crab/head_top",
    // polar bear (477..=482)
    "entity/polar_bear/body_end", "entity/polar_bear/body_side", "entity/polar_bear/body_top",
    "entity/polar_bear/head_front", "entity/polar_bear/head_side", "entity/polar_bear/head_top",
    // reindeer (483..=488)
    "entity/reindeer/body_end", "entity/reindeer/body_side", "entity/reindeer/body_top",
    "entity/reindeer/head_front", "entity/reindeer/head_side", "entity/reindeer/head_top",
    // donkey (489..=494)
    "entity/donkey/body_end", "entity/donkey/body_side", "entity/donkey/body_top",
    "entity/donkey/head_front", "entity/donkey/head_side", "entity/donkey/head_top",
    // mule (495..=500)
    "entity/mule/body_end", "entity/mule/body_side", "entity/mule/body_top",
    "entity/mule/head_front", "entity/mule/head_side", "entity/mule/head_top",
    // parrot (501..=504)
    "entity/parrot/body", "entity/parrot/head", "entity/parrot/leg", "entity/parrot/wing",
    // glow squid (505)
    "entity/glow_squid/body",
    // Water Wheel idle/turning (506..=507, Spec 48 Phase 4, 2026-09-06) — block
    // layers minted after the tint bake, so they append past it (lock-step with
    // generate_textures / `texture_gen::POST_TINT_LAYER_START`).
    "blocks/water_wheel", "blocks/water_wheel_turning",
    // Copper Ore (508, Wind, Copper & Electricity wave, 2026-09-07, Spec 02
    // §1), lock-step with generate_textures / `block::TEX_COPPER_ORE`.
    "blocks/copper_ore",
    // Windmill idle/turning (509..=510, Wind, Copper & Electricity wave §2.2,
    // 2026-09-07), lock-step with generate_textures / `block::TEX_WINDMILL*`.
    "blocks/windmill", "blocks/windmill_turning",
];

/// Override base layers in place from a disk pack: for each layer whose key has
/// a matching `<pack_dir>/<key>.png` (16×16 RGBA), replace that layer's pixels.
/// Missing / malformed / wrong-size files are skipped (a bad PNG never bricks
/// startup). A missing `pack_dir` is a no-op. Returns the number of layers
/// overridden. This is the inverse of [`dump_textures`] — the loader half of the
/// dump→edit→reload loop (texture-pack spec P2). Reload = restart for now;
/// live hot-swap is P3 (§11.5).
/// Side length of a square RGBA buffer (`len == side·side·4`). Used to infer the
/// atlas resolution from the layer bytes, so the renderer needs no extra param.
pub fn square_side(byte_len: usize) -> u32 {
    ((byte_len / 4) as f64).sqrt() as u32
}

/// One already-decoded pack texture: a named, raw-RGBA image. The
/// platform-independent unit the resolver composites onto the base layers.
/// Native fills this from disk PNGs; WASM (P4) from fetched + decoded PNGs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedTexture {
    pub key: String,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Composite already-decoded RGBA overrides onto base `layers` **by texture key**
/// — the cross-platform heart of the pack loader (Spec 03 §3.2/§11.4). For each
/// entry:
///   - resolve `key` → layer index (unknown keys are skipped, never bricking);
///   - skip non-square images (`width != height`);
///   - scale the image to the layer's current resolution (identity when equal;
///     nearest-up to keep pixel art, bilinear-down per §11.3);
///   - replace that layer's pixels **in place** (no new array layers).
///
/// Returns the number of layers overridden. Native [`apply_pack_overrides`] and
/// the WASM fetch path both funnel through this, so a pack composites identically
/// on every platform.
pub fn apply_pack_layers(layers: &mut [Vec<u8>], decoded: &[DecodedTexture]) -> usize {
    let mut count = 0;
    for tex in decoded {
        let Some(idx) = texture_index(&tex.key) else {
            log::warn!("texture pack: unknown texture key '{}' — skipping", tex.key);
            continue;
        };
        let i = idx as usize;
        if i >= layers.len() {
            continue;
        }
        if tex.width != tex.height {
            log::warn!(
                "texture pack: '{}' is {}×{} (must be square) — skipping",
                tex.key,
                tex.width,
                tex.height
            );
            continue;
        }
        // Target = the resolution this layer is already at, so the override is
        // scaled to match the atlas (the procedural base was pre-scaled to the
        // pack's declared resolution by `base_textures_from`).
        let target = square_side(layers[i].len());
        layers[i] = scale_layer(&tex.rgba, tex.width, target);
        count += 1;
    }
    count
}

/// Decode a disk pack dir's `<key>.png` files into [`DecodedTexture`]s — the
/// native half of the override list `apply_pack_layers` consumes. A missing dir
/// yields an empty list; unreadable/undecodable files are skipped with a warning.
#[cfg(not(target_arch = "wasm32"))]
pub fn decode_pack_dir(pack_dir: &Path) -> Vec<DecodedTexture> {
    if !pack_dir.is_dir() {
        return Vec::new();
    }
    let mut decoded = Vec::new();
    for key in texture_keys().iter() {
        let path = pack_dir.join(format!("{key}.png"));
        let Ok(bytes) = std::fs::read(&path) else { continue };
        match image::load_from_memory(&bytes) {
            Ok(img) => {
                let img = img.to_rgba8();
                let (width, height) = img.dimensions();
                // An animated strip (height = k·width, k ≥ 2) with a `.anim`
                // sidecar contributes its **first frame** as the static base
                // layer; the strip itself is collected by collect_pack_animations.
                let is_strip = width > 0 && height > width && height % width == 0;
                if is_strip && pack_dir.join(format!("{key}.png.anim")).is_file() {
                    decoded.push(DecodedTexture {
                        key: (*key).to_string(),
                        rgba: crate::texture_anim::first_frame(&img.into_raw(), width),
                        width,
                        height: width,
                    });
                } else {
                    decoded.push(DecodedTexture {
                        key: (*key).to_string(),
                        rgba: img.into_raw(),
                        width,
                        height,
                    });
                }
            }
            Err(e) => log::warn!("texture pack: {key}.png failed to decode ({e}) — skipping"),
        }
    }
    decoded
}

/// Maximum animated textures the atlas updates per frame (Spec 03 §3.4 — bounds
/// per-frame `write_texture` upload cost).
pub const MAX_ANIMATED_TEXTURES: usize = 32;

/// Build the [`crate::texture_anim::AnimatedTexture`]s for a disk pack at
/// `atlas_res`: every `<key>.png` that is a vertical strip (height = k·width,
/// k ≥ 2) **with** a `<key>.png.anim` sidecar becomes one animated layer — frames
/// sliced from the strip and scaled to the atlas resolution, schedule parsed from
/// the sidecar (Spec 03 §3.4 / §11.3). Capped at [`MAX_ANIMATED_TEXTURES`].
/// Native; the WASM fetch path builds the same structures in P4.
#[cfg(not(target_arch = "wasm32"))]
pub fn collect_pack_animations(
    pack_dir: &Path,
    atlas_res: u32,
) -> Vec<crate::texture_anim::AnimatedTexture> {
    use crate::texture_anim;
    if !pack_dir.is_dir() {
        return Vec::new();
    }
    let mut anims = Vec::new();
    for key in texture_keys().iter() {
        let sidecar = pack_dir.join(format!("{key}.png.anim"));
        if !sidecar.is_file() {
            continue;
        }
        let Some(layer_index) = texture_index(key) else { continue };
        let Ok(bytes) = std::fs::read(pack_dir.join(format!("{key}.png"))) else { continue };
        let img = match image::load_from_memory(&bytes) {
            Ok(img) => img.to_rgba8(),
            Err(e) => {
                log::warn!("texture pack: {key}.png failed to decode ({e}) — skipping");
                continue;
            }
        };
        let (w, h) = img.dimensions();
        let count = texture_anim::frame_count(h, w);
        if count < 2 {
            continue; // square / ragged strip — declared animated but only one frame
        }
        let frames: Vec<Vec<u8>> = texture_anim::slice_vertical_strip(&img.into_raw(), w, count)
            .iter()
            .map(|f| scale_layer(f, w, atlas_res))
            .collect();
        let meta = texture_anim::AnimMeta::parse(
            &std::fs::read_to_string(&sidecar).unwrap_or_default(),
        );
        let schedule = texture_anim::build_schedule(&meta, count);
        anims.push(texture_anim::AnimatedTexture { layer_index, frames, schedule });
        if anims.len() >= MAX_ANIMATED_TEXTURES {
            log::warn!(
                "texture pack: animated-texture cap ({MAX_ANIMATED_TEXTURES}) reached — stopped scanning"
            );
            break;
        }
    }
    anims
}

#[cfg(not(target_arch = "wasm32"))]
pub fn apply_pack_overrides(layers: &mut [Vec<u8>], pack_dir: &Path) -> usize {
    apply_pack_layers(layers, &decode_pack_dir(pack_dir))
}

/// Discover installed packs: sub-directories of `packs_root` that contain a
/// `pack.json`, by folder name, sorted. The picker shows these plus a built-in
/// "Default". (Loose files / dirs without a manifest are ignored.)
#[cfg(not(target_arch = "wasm32"))]
pub fn discover_packs(packs_root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(packs_root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().join("pack.json").is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// Root directory packs are installed in — a `texturepacks/` folder alongside
/// `worlds/` (Spec 03 §11.7 cache lives here too, later).
#[cfg(not(target_arch = "wasm32"))]
pub fn texturepacks_root() -> std::path::PathBuf {
    crate::save::worlds_root()
        .parent()
        .map(|p| p.join("texturepacks"))
        .unwrap_or_else(|| std::path::PathBuf::from("texturepacks"))
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    /// The picker's current selection (resolved dir, or `None` for the built-in
    /// default). Set at startup from the persisted setting and on picker change.
    static ACTIVE_PACK: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Set the active pack directory (`None` = built-in default). Called by the
/// picker and at startup from the persisted setting.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_active_pack(dir: Option<std::path::PathBuf>) {
    ACTIVE_PACK.with(|c| *c.borrow_mut() = dir);
}

/// The active disk texture pack, if any. Precedence: `AXENSTAX_TEXTURE_PACK`
/// (dev override) > picker selection ([`set_active_pack`]) > none (procedural
/// default). P4 adds the WASM HTTP-fetch source.
#[cfg(not(target_arch = "wasm32"))]
pub fn active_pack_dir() -> Option<std::path::PathBuf> {
    if let Some(v) = std::env::var_os("AXENSTAX_TEXTURE_PACK") {
        let p = std::path::PathBuf::from(v);
        if p.is_dir() {
            return Some(p);
        }
    }
    ACTIVE_PACK.with(|c| c.borrow().clone())
}

/// Resolve a picker selection to a pack dir. `"Default"`/empty → `None`; an
/// existing manifest-bearing subdir of `root` → `Some`; otherwise `None`.
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve_pack_dir_in(root: &Path, name: &str) -> Option<std::path::PathBuf> {
    if name.is_empty() || name == "Default" {
        return None;
    }
    let dir = root.join(name);
    dir.join("pack.json").is_file().then_some(dir)
}

#[cfg(not(target_arch = "wasm32"))]
fn active_pack_file() -> std::path::PathBuf {
    texturepacks_root().join("active.txt")
}

/// The persisted picker selection (`"Default"`/absent → `None`).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_active_pack_name() -> Option<String> {
    let name = std::fs::read_to_string(active_pack_file()).ok()?;
    let name = name.trim().to_string();
    (!name.is_empty() && name != "Default").then_some(name)
}

/// Display name of the active pack (`"Default"` when none) — what the picker shows.
#[cfg(not(target_arch = "wasm32"))]
pub fn active_pack_name() -> String {
    load_active_pack_name().unwrap_or_else(|| "Default".to_string())
}

/// Picker on-change: resolve, activate, and persist a selection by name.
#[cfg(not(target_arch = "wasm32"))]
pub fn select_pack(name: &str) {
    set_active_pack(resolve_pack_dir_in(&texturepacks_root(), name));
    let path = active_pack_file();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, name);
}

/// Activate the persisted pack selection — call once at startup, **before** the
/// texture array is first built (so the saved pack is in effect on first frame).
#[cfg(not(target_arch = "wasm32"))]
pub fn init_active_pack_from_disk() {
    if let Some(name) = load_active_pack_name() {
        set_active_pack(resolve_pack_dir_in(&texturepacks_root(), &name));
    }
}

/// The active WASM pack: `(resolution, decoded overrides)`. On WASM there is no
/// filesystem, so a fetched pack (P4) is decoded into memory here and applied by
/// [`base_textures`] exactly like a native disk pack. Empty = procedural default.
#[cfg(target_arch = "wasm32")]
thread_local! {
    static WASM_PACK: std::cell::RefCell<(u32, Vec<DecodedTexture>)> =
        const { std::cell::RefCell::new((16, Vec::new())) };
}

/// Install the fetched + decoded web pack (P4) so the next atlas (re)build applies
/// it. `resolution` is the pack's declared atlas resolution; `textures` the
/// decoded per-key overrides. Pass `(16, vec![])` to revert to the default.
#[cfg(target_arch = "wasm32")]
pub fn set_wasm_pack(resolution: u32, textures: Vec<DecodedTexture>) {
    let resolution = if VALID_RESOLUTIONS.contains(&resolution) { resolution } else { 16 };
    WASM_PACK.with(|p| *p.borrow_mut() = (resolution, textures));
}

/// True when a web pack is currently installed (WASM).
#[cfg(target_arch = "wasm32")]
pub fn has_wasm_pack() -> bool {
    WASM_PACK.with(|p| !p.borrow().1.is_empty())
}

/// The base texture layers as the renderer should see them: the procedural set
/// with the active pack's named overrides applied. Native reads a disk pack; WASM
/// applies the in-memory pack fetched over HTTP (P4). Both scale the procedural
/// base to the pack's resolution first, then composite the overrides (§11.3).
///
/// Every renderer stock-layer source routes through this (initial upload +
/// Workshop rebuild) so a configured pack is reflected consistently everywhere.
pub fn base_textures() -> Vec<Vec<u8>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        base_textures_from(active_pack_dir().as_deref())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let mut layers = crate::texture_gen::generate_textures(); // procedural, 16×16
        WASM_PACK.with(|p| {
            let (res, ref textures) = *p.borrow();
            if textures.is_empty() {
                return;
            }
            if res != 16 {
                for layer in layers.iter_mut() {
                    *layer = scale_layer(layer, 16, res);
                }
            }
            let n = apply_pack_layers(&mut layers, textures);
            log::info!("texture pack (web): applied {n} override(s) at {res}×{res}");
        });
        layers
    }
}

/// Procedural set with `pack`'s named overrides applied (when `Some`). Split out
/// from [`base_textures`] so the override composition is testable without the
/// `AXENSTAX_TEXTURE_PACK` env var. (P4 adds a WASM HTTP-fetch source here.)
#[cfg(not(target_arch = "wasm32"))]
pub fn base_textures_from(pack: Option<&Path>) -> Vec<Vec<u8>> {
    let mut layers = crate::texture_gen::generate_textures(); // procedural, 16×16
    let Some(dir) = pack else { return layers };
    // The pack's declared resolution sets the atlas resolution. Upscale the
    // procedural base to it first (nearest-neighbour), then overlay the pack's
    // PNGs (which `apply_pack_overrides` scales to the same resolution).
    let res = load_manifest(dir).texture_resolution;
    if res != 16 {
        for layer in layers.iter_mut() {
            *layer = scale_layer(layer, 16, res);
        }
    }
    let n = apply_pack_overrides(&mut layers, dir);
    log::info!(
        "texture pack: applied {n} override(s) at {res}×{res} from {}",
        dir.display()
    );
    layers
}

/// Square texture resolutions a pack may declare (Spec 03 §11.3).
pub const VALID_RESOLUTIONS: [u32; 4] = [16, 32, 64, 128];

/// Scale a square `from`×`from` RGBA layer to `to`×`to` (Spec 03 §11.3):
/// **nearest-neighbour** upscale to keep pixel art crisp, **bilinear** (triangle)
/// downscale. Identity (cheap clone) when the sizes already match — which makes
/// the whole resolution path a no-op for a default 16×16 pack. Cross-platform so
/// P4's WASM-fetched packs reuse it.
pub fn scale_layer(pixels: &[u8], from: u32, to: u32) -> Vec<u8> {
    if from == to {
        return pixels.to_vec();
    }
    let Some(img) = image::RgbaImage::from_raw(from, from, pixels.to_vec()) else {
        // Not a from×from RGBA buffer — leave it untouched rather than panic.
        log::warn!("scale_layer: buffer is not {from}×{from} RGBA — left unscaled");
        return pixels.to_vec();
    };
    let filter = if to > from {
        image::imageops::FilterType::Nearest // preserve pixel art on upscale
    } else {
        image::imageops::FilterType::Triangle // bilinear downscale
    };
    image::imageops::resize(&img, to, to, filter).into_raw()
}

fn default_resolution() -> u32 {
    16
}

/// `pack.json` — Spec 03 §11.2. Only `name` is conceptually required; every
/// other field defaults, and an absent or unparseable file yields a sensible
/// default manifest (16×16, name from the directory). `texture_resolution`
/// outside {16,32,64,128} falls back to 16 — a bad manifest never bricks load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackManifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default = "default_resolution")]
    pub texture_resolution: u32,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub license: String,
}

impl PackManifest {
    /// Default manifest for a pack dir lacking a (valid) `pack.json`: 16×16,
    /// name taken from the directory.
    pub fn fallback(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            version: String::new(),
            texture_resolution: 16,
            authors: Vec::new(),
            license: String::new(),
        }
    }

    /// Clamp an out-of-set `texture_resolution` back to 16 (lenient).
    fn sanitised(mut self) -> Self {
        if !VALID_RESOLUTIONS.contains(&self.texture_resolution) {
            log::warn!(
                "texture pack '{}': texture_resolution {} not in {{16,32,64,128}} — using 16",
                self.name,
                self.texture_resolution
            );
            self.texture_resolution = 16;
        }
        self
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn pack_name_from_dir(pack_dir: &Path) -> String {
    pack_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("pack")
        .to_string()
}

/// Load `<pack_dir>/pack.json`, or a default manifest if it's absent or
/// unparseable. Resolution is sanitised to a valid value. (P4 adds the WASM
/// fetch path; this is the native disk read.)
#[cfg(not(target_arch = "wasm32"))]
pub fn load_manifest(pack_dir: &Path) -> PackManifest {
    match std::fs::read_to_string(pack_dir.join("pack.json")) {
        Ok(text) => match serde_json::from_str::<PackManifest>(&text) {
            Ok(manifest) => manifest.sanitised(),
            Err(e) => {
                log::warn!("texture pack: pack.json parse error ({e}) — using defaults");
                PackManifest::fallback(pack_name_from_dir(pack_dir))
            }
        },
        Err(_) => PackManifest::fallback(pack_name_from_dir(pack_dir)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::texture_gen::texture_count;

    #[cfg(not(target_arch = "wasm32"))]
    fn write_png(path: &Path, w: u32, h: u32, rgba: [u8; 4]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba(rgba));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(path, png).unwrap();
    }

    fn solid(side: u32, rgba: [u8; 4]) -> Vec<u8> {
        std::iter::repeat(rgba).take((side * side) as usize).flatten().collect()
    }

    #[test]
    fn apply_pack_layers_composites_decoded_overrides_by_key() {
        let mut layers = crate::texture_gen::generate_textures();
        let dirt_before = layers[block::TEX_DIRT as usize].clone();
        let decoded = vec![DecodedTexture {
            key: "blocks/stone".to_string(),
            rgba: solid(16, [255, 0, 0, 255]),
            width: 16,
            height: 16,
        }];

        let n = apply_pack_layers(&mut layers, &decoded);

        assert_eq!(n, 1, "one in-memory override applied");
        assert!(
            layers[block::TEX_STONE as usize]
                .chunks_exact(4)
                .all(|p| p == [255, 0, 0, 255]),
            "stone replaced by the decoded override"
        );
        assert_eq!(
            layers[block::TEX_DIRT as usize], dirt_before,
            "an un-overridden key stays procedural"
        );
    }

    #[test]
    fn apply_pack_layers_skips_unknown_key_and_non_square() {
        let mut layers = crate::texture_gen::generate_textures();
        let snapshot = layers.clone();
        let decoded = vec![
            DecodedTexture {
                key: "blocks/does_not_exist".to_string(),
                rgba: solid(16, [1, 2, 3, 4]),
                width: 16,
                height: 16,
            },
            DecodedTexture {
                key: "blocks/sand".to_string(),
                rgba: vec![0; 16 * 8 * 4],
                width: 16,
                height: 8,
            },
        ];

        assert_eq!(
            apply_pack_layers(&mut layers, &decoded),
            0,
            "unknown key + non-square are both skipped"
        );
        assert_eq!(layers, snapshot, "no valid override → atlas byte-identical");
    }

    #[test]
    fn apply_pack_layers_scales_override_to_layer_resolution() {
        let mut layers = crate::texture_gen::generate_textures(); // 16×16
        let decoded = vec![DecodedTexture {
            key: "blocks/stone".to_string(),
            rgba: solid(32, [0, 255, 0, 255]),
            width: 32,
            height: 32,
        }];

        assert_eq!(apply_pack_layers(&mut layers, &decoded), 1);
        assert_eq!(
            layers[block::TEX_STONE as usize].len(),
            16 * 16 * 4,
            "32×32 override downscaled to the 16×16 layer"
        );
        assert!(
            layers[block::TEX_STONE as usize]
                .chunks_exact(4)
                .all(|p| p == [0, 255, 0, 255]),
            "solid colour survives the downscale"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn write_strip_png(path: &Path, frame_w: u32, frames: &[[u8; 4]]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut img = image::RgbaImage::new(frame_w, frame_w * frames.len() as u32);
        for (fi, c) in frames.iter().enumerate() {
            for y in 0..frame_w {
                for x in 0..frame_w {
                    img.put_pixel(x, fi as u32 * frame_w + y, image::Rgba(*c));
                }
            }
        }
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        std::fs::write(path, png).unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn decode_pack_dir_crops_animated_strip_to_first_frame() {
        let dir = std::env::temp_dir().join("axenstax_anim_decode_test");
        let _ = std::fs::remove_dir_all(&dir);
        // 16×32 strip: red frame, green frame; with a sidecar → animated.
        write_strip_png(&dir.join("blocks/water.png"), 16, &[[255, 0, 0, 255], [0, 255, 0, 255]]);
        std::fs::write(dir.join("blocks/water.png.anim"), "{}").unwrap();

        let decoded = decode_pack_dir(&dir);
        let water = decoded.iter().find(|d| d.key == "blocks/water").expect("water decoded");
        assert_eq!((water.width, water.height), (16, 16), "cropped to a square base layer");
        assert!(
            water.rgba.chunks_exact(4).all(|p| p == [255, 0, 0, 255]),
            "base layer holds the strip's first frame"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn collect_pack_animations_builds_from_strip_and_sidecar() {
        let dir = std::env::temp_dir().join("axenstax_anim_collect_test");
        let _ = std::fs::remove_dir_all(&dir);
        write_strip_png(&dir.join("blocks/water.png"), 16, &[[255, 0, 0, 255], [0, 255, 0, 255]]);
        std::fs::write(dir.join("blocks/water.png.anim"), r#"{ "frame_time": 3 }"#).unwrap();

        let anims = collect_pack_animations(&dir, 16);
        assert_eq!(anims.len(), 1, "one animated texture collected");
        let a = &anims[0];
        assert_eq!(a.layer_index, texture_index("blocks/water").unwrap());
        assert_eq!(a.frames.len(), 2);
        assert!(a.frames[0].chunks_exact(4).all(|p| p == [255, 0, 0, 255]), "frame 0 red");
        assert!(a.frames[1].chunks_exact(4).all(|p| p == [0, 255, 0, 255]), "frame 1 green");
        assert_eq!(a.schedule, vec![(0, 3), (1, 3)]);
        assert!(a.is_animated());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn collect_pack_animations_scales_frames_to_atlas_resolution() {
        let dir = std::env::temp_dir().join("axenstax_anim_scale_test");
        let _ = std::fs::remove_dir_all(&dir);
        write_strip_png(&dir.join("blocks/water.png"), 16, &[[1, 2, 3, 255], [4, 5, 6, 255]]);
        std::fs::write(dir.join("blocks/water.png.anim"), "{}").unwrap();

        let anims = collect_pack_animations(&dir, 32);
        assert_eq!(anims.len(), 1);
        assert!(
            anims[0].frames.iter().all(|f| f.len() == 32 * 32 * 4),
            "frames upscaled to the 32×32 atlas resolution"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn collect_pack_animations_ignores_strip_without_sidecar() {
        let dir = std::env::temp_dir().join("axenstax_anim_nosidecar_test");
        let _ = std::fs::remove_dir_all(&dir);
        // A strip but NO .anim sidecar → just a tall texture, not animated.
        write_strip_png(&dir.join("blocks/water.png"), 16, &[[1, 1, 1, 255], [2, 2, 2, 255]]);
        assert!(collect_pack_animations(&dir, 16).is_empty(), "no sidecar → not animated");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pack_overrides_replace_layer_by_name() {
        let dir = std::env::temp_dir().join("axenstax_pack_override_test");
        let _ = std::fs::remove_dir_all(&dir);
        // Only stone is overridden — to solid red.
        write_png(&dir.join("blocks/stone.png"), 16, 16, [255, 0, 0, 255]);

        let mut layers = crate::texture_gen::generate_textures();
        let dirt_before = layers[block::TEX_DIRT as usize].clone();

        let n = apply_pack_overrides(&mut layers, &dir);

        assert_eq!(n, 1, "exactly one layer overridden");
        assert!(
            layers[block::TEX_STONE as usize]
                .chunks_exact(4)
                .all(|p| p == [255, 0, 0, 255]),
            "stone layer should now be solid red from the pack"
        );
        assert_eq!(
            layers[block::TEX_DIRT as usize], dirt_before,
            "layers with no pack file stay procedural"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn base_textures_from_applies_pack_else_procedural() {
        // None → byte-identical to the procedural set.
        assert_eq!(
            base_textures_from(None),
            crate::texture_gen::generate_textures(),
            "no pack → procedural set unchanged"
        );
        // Some(dir) → matching layers overridden, count preserved.
        let dir = std::env::temp_dir().join("axenstax_base_textures_test");
        let _ = std::fs::remove_dir_all(&dir);
        write_png(&dir.join("blocks/stone.png"), 16, 16, [255, 0, 0, 255]);
        let layers = base_textures_from(Some(&dir));
        assert_eq!(layers.len(), texture_count() as usize, "layer count preserved");
        assert!(
            layers[block::TEX_STONE as usize]
                .chunks_exact(4)
                .all(|p| p == [255, 0, 0, 255]),
            "base_textures_from should apply the pack's stone override"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scale_layer_identity_upscale_downscale() {
        // Identity when sizes match (the default-16×16 no-op path).
        let p: Vec<u8> = (0..16 * 16 * 4).map(|i| i as u8).collect();
        assert_eq!(scale_layer(&p, 16, 16), p, "equal sizes → identity");

        // Nearest upscale 1×1 → 2×2: one red texel becomes four red texels.
        let one = vec![255u8, 0, 0, 255];
        let up = scale_layer(&one, 1, 2);
        assert_eq!(up.len(), 2 * 2 * 4, "upscaled byte length");
        assert!(
            up.chunks_exact(4).all(|px| px == [255, 0, 0, 255]),
            "nearest upscale replicates the source texel exactly"
        );

        // Downscale changes dimensions.
        let big = vec![128u8; 4 * 4 * 4];
        assert_eq!(scale_layer(&big, 4, 2).len(), 2 * 2 * 4, "downscaled byte length");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn manifest_defaults_when_absent() {
        let dir = std::env::temp_dir().join("axenstax_manifest_absent_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let m = load_manifest(&dir);
        assert_eq!(m.texture_resolution, 16, "absent pack.json → 16×16 default");
        assert_eq!(m.name, "axenstax_manifest_absent_test", "name from dir");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn manifest_parses_declared_fields() {
        let dir = std::env::temp_dir().join("axenstax_manifest_parse_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("pack.json"),
            r#"{ "name": "Crisp 32", "texture_resolution": 32, "authors": ["axo"], "license": "CC-BY-4.0" }"#,
        )
        .unwrap();
        let m = load_manifest(&dir);
        assert_eq!(m.name, "Crisp 32");
        assert_eq!(m.texture_resolution, 32);
        assert_eq!(m.authors, vec!["axo".to_string()]);
        assert_eq!(m.license, "CC-BY-4.0");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn dump_writes_a_default_manifest() {
        let dir = std::env::temp_dir().join("axenstax_dump_manifest_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dump_textures(&dir).expect("dump should succeed");
        assert!(dir.join("pack.json").exists(), "dump writes a default pack.json");
        let m = load_manifest(&dir);
        assert_eq!(m.texture_resolution, 16, "default pack is 16×16");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn manifest_bad_resolution_falls_back_to_16() {
        let dir = std::env::temp_dir().join("axenstax_manifest_badres_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pack.json"), r#"{ "name": "Odd", "texture_resolution": 17 }"#).unwrap();
        let m = load_manifest(&dir);
        assert_eq!(m.texture_resolution, 16, "invalid resolution clamped to 16");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pack_overrides_skip_bad_and_missing() {
        // Missing dir → no-op.
        let missing = std::env::temp_dir().join("axenstax_pack_missing_xyz");
        let _ = std::fs::remove_dir_all(&missing);
        let mut layers = crate::texture_gen::generate_textures();
        let snapshot = layers.clone();
        assert_eq!(apply_pack_overrides(&mut layers, &missing), 0);
        assert_eq!(layers, snapshot, "missing pack dir leaves layers untouched");

        // Non-square file → skipped, layer untouched (off-size *square* files are
        // scaled instead, covered by base_textures_from_scales_to_pack_resolution).
        let dir = std::env::temp_dir().join("axenstax_pack_nonsquare_test");
        let _ = std::fs::remove_dir_all(&dir);
        write_png(&dir.join("blocks/sand.png"), 16, 8, [0, 255, 0, 255]);
        let sand_before = layers[block::TEX_SAND as usize].clone();
        let n = apply_pack_overrides(&mut layers, &dir);
        assert_eq!(n, 0, "non-square PNG is not applied");
        assert_eq!(
            layers[block::TEX_SAND as usize], sand_before,
            "non-square pack file leaves the layer procedural"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn resolve_pack_dir_in_maps_default_and_real_packs() {
        let root = std::env::temp_dir().join("axenstax_resolve_test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("aqua")).unwrap();
        std::fs::write(root.join("aqua/pack.json"), r#"{ "name": "Aqua" }"#).unwrap();

        assert_eq!(resolve_pack_dir_in(&root, "Default"), None, "Default → no pack");
        assert_eq!(resolve_pack_dir_in(&root, ""), None, "empty → no pack");
        assert_eq!(resolve_pack_dir_in(&root, "nope"), None, "missing pack → None");
        assert_eq!(
            resolve_pack_dir_in(&root, "aqua"),
            Some(root.join("aqua")),
            "real manifest-bearing pack → its dir"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn discover_packs_lists_only_subdirs_with_a_manifest() {
        let root = std::env::temp_dir().join("axenstax_discover_test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("aqua")).unwrap();
        std::fs::write(root.join("aqua/pack.json"), r#"{ "name": "Aqua" }"#).unwrap();
        std::fs::create_dir_all(root.join("retro")).unwrap();
        std::fs::write(root.join("retro/pack.json"), r#"{ "name": "Retro" }"#).unwrap();
        std::fs::create_dir_all(root.join("notapack")).unwrap(); // dir, no manifest
        std::fs::write(root.join("loose.txt"), "x").unwrap(); // not a dir

        let packs = discover_packs(&root);
        assert_eq!(
            packs,
            vec!["aqua".to_string(), "retro".to_string()],
            "only manifest-bearing subdirs, sorted"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn base_textures_from_scales_to_pack_resolution() {
        let dir = std::env::temp_dir().join("axenstax_res32_pack_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pack.json"), r#"{ "name": "hi", "texture_resolution": 32 }"#).unwrap();
        // A native-32×32 override for stone.
        write_png(&dir.join("blocks/stone.png"), 32, 32, [255, 0, 255, 255]);

        let layers = base_textures_from(Some(&dir));

        // Every layer is now 32×32 RGBA (procedural ones upscaled, pack one native).
        assert!(
            layers.iter().all(|l| l.len() == 32 * 32 * 4),
            "whole atlas scaled to the pack's 32×32 resolution"
        );
        assert!(
            layers[block::TEX_STONE as usize]
                .chunks_exact(4)
                .all(|p| p == [255, 0, 255, 255]),
            "32×32 stone override applied"
        );
        // Dirt has no pack file → procedural, upscaled (not blank, full size).
        assert_eq!(layers[block::TEX_DIRT as usize].len(), 32 * 32 * 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys_len_matches_texture_count() {
        assert_eq!(
            texture_keys().len(),
            texture_count() as usize,
            "every generated layer must have exactly one key"
        );
    }

    #[test]
    fn keys_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for k in texture_keys() {
            assert!(seen.insert(*k), "duplicate texture key: {k}");
        }
    }

    #[test]
    fn keys_round_trip() {
        for i in 0..texture_keys().len() as u32 {
            let k = texture_key(i).expect("key for in-range index");
            assert_eq!(
                texture_index(k),
                Some(i),
                "round-trip failed at index {i} (key {k}) — likely a duplicate key"
            );
        }
    }

    #[test]
    fn tex_constants_resolve_to_their_keys() {
        use crate::entity_model as em;
        // Representative + boundary bindings spanning the whole 0..400 range.
        // Locks the key list to the existing layer indices at every point a
        // named constant exists — the behaviour-neutrality guarantee.
        let pairs: &[(&str, u32)] = &[
            ("blocks/stone", block::TEX_STONE),                 // 0 (low boundary)
            ("blocks/dirt", block::TEX_DIRT),                   // 1
            ("blocks/grass_top", block::TEX_GRASS_TOP),         // 2
            ("blocks/grass_side", block::TEX_GRASS_SIDE),       // 3
            ("blocks/bedrock", block::TEX_BEDROCK),             // 4
            ("blocks/oak_log_side", block::TEX_OAK_LOG_SIDE),   // 6
            ("blocks/crafting_table_top", block::TEX_CRAFTING_TABLE_TOP), // 15
            ("entity/cow/head_front", em::TEX_COW_HEAD_FRONT),  // 16 (entity base)
            ("entity/cow/leg", em::TEX_COW_LEG),                // 22
            ("entity/chicken/head", em::TEX_CHICKEN_HEAD),      // 30
            ("entity/pig/head_front", em::TEX_PIG_HEAD_FRONT),  // 34
            ("entity/sheep/leg", em::TEX_SHEEP_LEG),            // 47
            ("item/stick", em::TEX_ITEM_STICK),                 // 67
            ("item/tool_diamond", em::TEX_ITEM_TOOL_DIAMOND),   // 83
            ("blocks/coal_ore", block::TEX_COAL_ORE),           // 86
            ("blocks/diamond_ore", block::TEX_DIAMOND_ORE),     // 88
            ("blocks/glass", block::TEX_GLASS),                 // 93
            ("blocks/torch", block::TEX_TORCH),                 // 97
            ("item/arrow", em::TEX_ITEM_ARROW),                 // 105
            ("blocks/pure_deepslate", block::TEX_PURE_DEEPSLATE), // 106
            ("blocks/satori_block", block::TEX_SATORI_BLOCK),   // 110
            ("item/satori", block::TEX_ITEM_SATORI),            // 111
            ("blocks/tilled_soil", block::TEX_TILLED_SOIL),     // 112
            ("blocks/wheat_stage_0", block::TEX_WHEAT_STAGE_0), // 118
            ("entity/villager/head_front", em::TEX_VILLAGER_HEAD_FRONT), // 146
            ("blocks/chest_top", block::TEX_CHEST_TOP),         // 180
            ("overlay/crack_0", block::TEX_CRACK_BASE),         // 207
            ("blocks/copper_chest_top", block::TEX_COPPER_CHEST_TOP), // 377
            ("blocks/cable", block::TEX_CABLE),                 // 385
            ("blocks/motion_sensor", block::TEX_MOTION_SENSOR), // 399
            // #130 — per-species leaf layers, the new high boundary (421..=425).
            ("blocks/birch_leaves", block::TEX_BIRCH_LEAVES),     // 421
            ("blocks/spruce_leaves", block::TEX_SPRUCE_LEAVES),   // 422
            ("blocks/jungle_leaves", block::TEX_JUNGLE_LEAVES),   // 423
            ("blocks/acacia_leaves", block::TEX_ACACIA_LEAVES),   // 424
            ("blocks/dark_oak_leaves", block::TEX_DARK_OAK_LEAVES), // 425
            // #132 — per-species wood layers, 426..=440 (new high boundary).
            ("blocks/birch_log_side", block::TEX_BIRCH_LOG_SIDE),     // 426
            ("blocks/birch_log_top", block::TEX_BIRCH_LOG_TOP),       // 427
            ("blocks/birch_planks", block::TEX_BIRCH_PLANKS),         // 428
            ("blocks/spruce_log_side", block::TEX_SPRUCE_LOG_SIDE),   // 429
            ("blocks/spruce_log_top", block::TEX_SPRUCE_LOG_TOP),     // 430
            ("blocks/spruce_planks", block::TEX_SPRUCE_PLANKS),       // 431
            ("blocks/jungle_log_side", block::TEX_JUNGLE_LOG_SIDE),   // 432
            ("blocks/jungle_log_top", block::TEX_JUNGLE_LOG_TOP),     // 433
            ("blocks/jungle_planks", block::TEX_JUNGLE_PLANKS),       // 434
            ("blocks/acacia_log_side", block::TEX_ACACIA_LOG_SIDE),   // 435
            ("blocks/acacia_log_top", block::TEX_ACACIA_LOG_TOP),     // 436
            ("blocks/acacia_planks", block::TEX_ACACIA_PLANKS),       // 437
            ("blocks/dark_oak_log_side", block::TEX_DARK_OAK_LOG_SIDE), // 438
            ("blocks/dark_oak_log_top", block::TEX_DARK_OAK_LOG_TOP),   // 439
            ("blocks/dark_oak_planks", block::TEX_DARK_OAK_PLANKS),   // 440 (high boundary)
        ];
        for (key, idx) in pairs {
            assert_eq!(
                texture_index(key),
                Some(*idx),
                "key {key:?} should resolve to layer {idx}"
            );
            assert_eq!(
                texture_key(*idx),
                Some(*key),
                "layer {idx} should resolve to key {key:?}"
            );
        }
    }

    #[test]
    fn loop_family_boundaries_pinned() {
        // The décor/wallpaper colour families are generated in loops, so no
        // TEX_* constant covers them. Pin each range's first + last layer by
        // direct index to catch an off-by-one in the 16-colour authoring.
        let bounds: &[(u32, &str)] = &[
            (257, "decor/wallpaper/white"),
            (269, "decor/wallpaper/light_grey"),
            (273, "decor/wallpaper/brown"),
            (275, "decor/wallpaper/magenta"),
            (292, "decor/bunting/white"),
            (307, "decor/bunting/magenta"),
            (308, "decor/lantern/white"),
            (323, "decor/lantern/magenta"),
            (324, "decor/kite/white"),
            (339, "decor/kite/magenta"),
            (340, "decor/banner/white"),
            (355, "decor/banner/magenta"),
            (356, "decor/sail/white"),
            (371, "decor/sail/magenta"),
        ];
        for (idx, key) in bounds {
            assert_eq!(texture_key(*idx), Some(*key), "layer {idx} should be {key:?}");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn dump_writes_one_decodable_png_per_key() {
        let dir = std::env::temp_dir().join("axenstax_texdump_p1_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let n = dump_textures(&dir).expect("dump should succeed");
        assert_eq!(n, texture_keys().len(), "one PNG per key");

        // A flat block key.
        let stone = dir.join("blocks/stone.png");
        assert!(stone.exists(), "blocks/stone.png should exist");
        let bytes = std::fs::read(&stone).unwrap();
        let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (16, 16), "default pack is 16×16");

        // A nested entity key must create its sub-directories.
        assert!(dir.join("entity/cow/leg.png").exists(), "nested key dirs created");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
