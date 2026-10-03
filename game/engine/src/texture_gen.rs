//! Procedural 16x16 block texture generation.
//!
//! Generates RGBA pixel data for each block texture layer.
//! Textures are uploaded as a 2D texture array to the GPU.
//! Layer order must match block::TEX_* constants.

const SIZE: u32 = 16;
const PIXELS: usize = (SIZE * SIZE * 4) as usize; // 1024 bytes per texture

/// Generate all block textures. Returns Vec of RGBA pixel data, one per layer.
/// Index = texture array layer index (matches block::TEX_* constants).
pub fn generate_textures() -> Vec<Vec<u8>> {
    let mut textures = vec![
        gen_stone(),              // 0: TEX_STONE
        gen_dirt(),               // 1: TEX_DIRT
        gen_grass_top(),          // 2: TEX_GRASS_TOP
        gen_grass_side(),         // 3: TEX_GRASS_SIDE
        gen_bedrock(),            // 4: TEX_BEDROCK
        gen_sand(),               // 5: TEX_SAND
        gen_oak_log_side(),       // 6: TEX_OAK_LOG_SIDE
        gen_oak_log_top(),        // 7: TEX_OAK_LOG_TOP
        gen_oak_leaves(),         // 8: TEX_OAK_LEAVES
        gen_oak_planks(),         // 9: TEX_OAK_PLANKS
        gen_cobblestone(),        // 10: TEX_COBBLESTONE
        gen_water(),              // 11: TEX_WATER
        gen_gravel(),             // 12: TEX_GRAVEL
        gen_sandstone(),          // 13: TEX_SANDSTONE
        gen_snow(),               // 14: TEX_SNOW
        gen_crafting_table_top(), // 15: TEX_CRAFTING_TABLE_TOP
        // --- Entity textures (layers 16+) ---
        // Cow (layers 16-22)
        gen_cow_head_front(),     // 16
        gen_cow_head_side(),      // 17
        gen_cow_head_top(),       // 18
        gen_cow_body_side(),      // 19
        gen_cow_body_top(),       // 20
        gen_cow_body_end(),       // 21
        gen_cow_leg(),            // 22
    ];

    // Retired layers 23-29 — neutral grey fillers (see gen_retired_layer).
    // Order/count preserved so all later layer indices stay stable.
    for _ in 23..=29 { textures.push(gen_retired_layer()); }

    // Chicken (layers 30-33)
    textures.push(gen_chicken_head());      // 30
    textures.push(gen_chicken_body());      // 31
    textures.push(gen_chicken_leg());       // 32
    textures.push(gen_chicken_wing());      // 33

    // Pig (layers 34-40)
    textures.push(gen_pig_head_front());    // 34
    textures.push(gen_pig_head_side());     // 35
    textures.push(gen_pig_head_top());      // 36
    textures.push(gen_pig_body_side());     // 37
    textures.push(gen_pig_body_top());      // 38
    textures.push(gen_pig_body_end());      // 39
    textures.push(gen_pig_leg());           // 40

    // Sheep (layers 41-47)
    textures.push(gen_sheep_head_front());  // 41
    textures.push(gen_sheep_head_side());   // 42
    textures.push(gen_sheep_head_top());    // 43
    textures.push(gen_sheep_body_side());   // 44
    textures.push(gen_sheep_body_top());    // 45
    textures.push(gen_sheep_body_end());    // 46
    textures.push(gen_sheep_leg());         // 47

    // Retired layers 48-66 — neutral grey fillers (see gen_retired_layer).
    // Order/count preserved so all later layer indices stay stable.
    for _ in 48..=66 { textures.push(gen_retired_layer()); }

    // Item-drop textures (Wave 2b + Gunpowder + per-tool — layers 67-83)
    textures.push(gen_item_stick());          // 67
    textures.push(gen_item_leather());        // 68
    textures.push(gen_item_feather());        // 69
    textures.push(gen_item_wool());           // 70
    textures.push(gen_item_bone());           // 71
    textures.push(gen_retired_layer());       // 72 (retired item drop)
    textures.push(gen_item_raw_beef());       // 73
    textures.push(gen_item_raw_porkchop());   // 74
    textures.push(gen_item_raw_chicken());    // 75
    textures.push(gen_item_raw_mutton());     // 76
    textures.push(gen_item_string());         // 77
    textures.push(gen_retired_layer());       // 78 (retired item drop)
    textures.push(gen_item_grey_powder());    // 79: TEX_ITEM_GREY_POWDER
    textures.push(gen_item_tool_wood());      // 80
    textures.push(gen_item_tool_stone());     // 81
    textures.push(gen_item_tool_iron());      // 82
    textures.push(gen_item_tool_diamond());   // 83

    // Bed textures (Wave 9 — layers 84-85)
    textures.push(gen_bed_top());             // 84
    textures.push(gen_bed_side());            // 85

    // Ore textures (Wave 13 — layers 86-88)
    textures.push(gen_coal_ore());            // 86
    textures.push(gen_iron_ore());            // 87
    textures.push(gen_diamond_ore());         // 88

    // Retired layers 89-92 (formerly a fantasy mob + its drop) — neutral grey
    // fillers. Order/count preserved so all later layer indices stay stable.
    for _ in 89..=92 { textures.push(gen_retired_layer()); }

    // Glass block (Wave 16 — layer 93)
    textures.push(gen_glass());               // 93

    // Storage blocks (Wave 17 — layers 94-96)
    textures.push(gen_coal_block());          // 94
    textures.push(gen_iron_block());          // 95
    textures.push(gen_diamond_block());       // 96

    // Torch (Wave 18 — layer 97)
    textures.push(gen_torch());               // 97

    // Smelting outputs (Wave 6 — layers 98-102)
    textures.push(gen_item_iron_ingot());     // 98
    textures.push(gen_item_cooked_beef());    // 99
    textures.push(gen_item_cooked_porkchop()); // 100
    textures.push(gen_item_cooked_chicken()); // 101
    textures.push(gen_item_cooked_mutton());  // 102

    // Tall Grass block (Wave 22 — layer 103)
    textures.push(gen_tall_grass());          // 103

    // Reserved for Wave 23 sequencing — arrow texture sits at layer 105;
    // layer 104 left for a future polish slot. (Adding 105 directly without
    // a 104 entry would mis-align indices, so keep the gap explicit by
    // pushing a placeholder. Replaced when Wave 24 / 25 wants the slot.)
    textures.push(gen_placeholder_104());      // 104

    // Arrow item + projectile (Wave 23 — layer 105)
    textures.push(gen_item_arrow());          // 105

    // Deepslate layer (Wave 25 — layers 106..=110, plus Satori item 111).
    // Spec 2 §5.3.1a deepslate world-gen + Spec 5 §3.8 / Spec 6 §2.2c Satori.
    textures.push(gen_pure_deepslate());        // 106
    textures.push(gen_deepslate_coal_ore());    // 107
    textures.push(gen_deepslate_iron_ore());    // 108
    textures.push(gen_deepslate_diamond_ore()); // 109
    textures.push(gen_satori_block());          // 110
    textures.push(gen_item_satori());           // 111

    // Farming Tier 1 (Wave 26 — layers 112..=129).
    textures.push(gen_tilled_soil());           // 112
    textures.push(gen_item_wheat_seeds());      // 113
    textures.push(gen_item_wheat());            // 114
    textures.push(gen_item_bread());            // 115
    textures.push(gen_item_carrot());           // 116
    textures.push(gen_item_potato());           // 117
    // Crop-stage textures (118-129) — visual progression: stage 0 sprouts
    // at the base, stage 3 fills the block.
    textures.push(gen_crop_stage(118, [60, 105, 30], 0));   // 118 wheat_stage_0
    textures.push(gen_crop_stage(119, [80, 120, 30], 1));   // 119 wheat_stage_1
    textures.push(gen_crop_stage(120, [160, 145, 50], 2));  // 120 wheat_stage_2
    textures.push(gen_crop_stage(121, [220, 190, 60], 3));  // 121 wheat_stage_3 mature
    textures.push(gen_crop_stage(122, [50, 100, 30], 0));   // 122 carrot_stage_0
    textures.push(gen_crop_stage(123, [60, 115, 30], 1));   // 123 carrot_stage_1
    textures.push(gen_crop_stage(124, [80, 135, 35], 2));   // 124 carrot_stage_2
    textures.push(gen_crop_stage(125, [110, 155, 40], 3));  // 125 carrot_stage_3 mature
    textures.push(gen_crop_stage(126, [45, 95, 30], 0));    // 126 potato_stage_0
    textures.push(gen_crop_stage(127, [55, 110, 30], 1));   // 127 potato_stage_1
    textures.push(gen_crop_stage(128, [75, 130, 35], 2));   // 128 potato_stage_2
    textures.push(gen_crop_stage(129, [95, 145, 40], 3));   // 129 potato_stage_3 mature

    // Campfire textures (Wave 27 — layers 130-133).
    textures.push(gen_campfire_lit_top());    // 130
    textures.push(gen_campfire_lit_side());   // 131
    textures.push(gen_campfire_unlit_top());  // 132
    textures.push(gen_campfire_unlit_side()); // 133
    // Flint item + flint-and-steel tool (Wave 27 — layers 134-135).
    textures.push(gen_item_flint());          // 134
    textures.push(gen_item_flint_and_steel());// 135

    // Campfire smoke pillar (Wave 28 — layer 136).
    textures.push(gen_campfire_smoke());      // 136
    // Corn crop stages (Wave 28 — layers 137-140). Yellow/green palette
    // progression so mature corn reads at a distance.
    textures.push(gen_crop_stage(2800, [70, 130, 35], 0));   // 137 corn_stage_0
    textures.push(gen_crop_stage(2801, [90, 145, 35], 1));   // 138 corn_stage_1
    textures.push(gen_crop_stage(2802, [140, 165, 50], 2));  // 139 corn_stage_2
    textures.push(gen_crop_stage(2803, [230, 195, 60], 3));  // 140 corn_stage_3 mature
    // Corn item drops (Wave 28 — layers 141-143).
    textures.push(gen_farming_item(2810, 220, 200, 80));     // 141 corn_seeds
    textures.push(gen_farming_item(2811, 235, 195, 60));     // 142 corn (ear)
    textures.push(gen_farming_item(2812, 245, 175, 50));     // 143 baked_corn
    // Baked potato + baked carrot (Wave 28 — layers 144-145).
    textures.push(gen_farming_item(2813, 175, 135, 80));     // 144 baked_potato
    textures.push(gen_farming_item(2814, 215, 110, 35));     // 145 baked_carrot
    // Spec 19 — Villager / Iron Golem / Wandering Villager (layers 146-157).
    textures.push(gen_villager_head_front());     // 146
    textures.push(gen_villager_head_side());      // 147
    textures.push(gen_villager_body(false));      // 148 brown robe
    textures.push(gen_villager_leg());            // 149
    // Retired layers 150-153 (formerly a fantasy construct mob) — neutral grey
    // fillers. Order/count preserved so all later layer indices stay stable.
    for _ in 150..=153 { textures.push(gen_retired_layer()); }
    textures.push(gen_wandering_head_front());    // 154
    textures.push(gen_wandering_head_side());     // 155
    textures.push(gen_villager_body(true));       // 156 purple robe
    textures.push(gen_villager_leg());            // 157 reuses dirt-brown leg

    // Drying Rack (Wave 29 — layers 158-159). Two textures: top shows a
    // stack of horizontal logs visible from above; sides show a vertical
    // post-and-rail frame so the block reads as a workstation.
    textures.push(gen_drying_rack_top());         // 158 drying_rack_top
    textures.push(gen_drying_rack_side());        // 159 drying_rack_side

    // Papyrus Reed (Spec 23 — layers 160-163 block stages + 164-165 item
    // drops). Mirror the wheat/carrot/potato/corn approach by reusing
    // `gen_crop_stage` with a green-yellow palette progressing toward
    // papyrus-gold. Item drops use `gen_farming_item` with a green-tan
    // reed colour + parchment-cream sheet colour.
    textures.push(gen_crop_stage(3000, [80, 130, 45], 0));   // 160 papyrus_stage_0
    textures.push(gen_crop_stage(3001, [110, 155, 50], 1));  // 161 papyrus_stage_1
    textures.push(gen_crop_stage(3002, [160, 175, 55], 2));  // 162 papyrus_stage_2
    textures.push(gen_crop_stage(3003, [220, 200, 90], 3));  // 163 papyrus_stage_3 mature
    textures.push(gen_farming_item(3010, 140, 175, 80));     // 164 papyrus_reed item
    textures.push(gen_farming_item(3011, 235, 220, 158));    // 165 papyrus_sheet item

    // Build Schematics Core (Spec 24 — layers 166-170). Blueprint Paper uses
    // a parchment-cream top with a dark walnut border on the sides so
    // a row of tiles reads as a drafting board from any angle. The
    // Construction Anchor is a red surveyor's flag silhouette on a
    // shadowed background (single texture across all faces — it's a
    // marker, not a workstation). The Architect's Plaque has a
    // parchment-on-wood look with a small grid pattern on the top
    // hinting at "structured drawing".
    textures.push(gen_blueprint_paper_top());          // 166 blueprint_paper_top
    textures.push(gen_blueprint_paper_side());         // 167 blueprint_paper_side
    textures.push(gen_construction_anchor());    // 168 construction_anchor
    textures.push(gen_architect_plaque_top());   // 169 architect_plaque_top
    textures.push(gen_architect_plaque_side());  // 170 architect_plaque_side

    // Spec 20 (Furnace) — top (banded stone slab), side unlit (cobblestone
    // with dark hollow), side lit (same with orange-glow chamber).
    textures.push(gen_furnace_top());            // 171 furnace_top
    textures.push(gen_furnace_side_unlit());     // 172 furnace_side_unlit
    textures.push(gen_furnace_side_lit());       // 173 furnace_side_lit

    // Spec 16 Phase 3b — deepslate visual variants. Tier-ascending
    // richness: THIN sprinkles blue specks, HEALTHY adds visible
    // blue-orange veins, FAT glows densely. All keep the canonical
    // dark-grey base so the variant family reads as a unit.
    textures.push(gen_deepslate_thin());        // 174 deepslate_thin
    textures.push(gen_deepslate_healthy());     // 175 deepslate_healthy
    textures.push(gen_deepslate_fat());         // 176 deepslate_fat

    // Spec 21 Vendor Block — chest-style wood face with an iron
    // coin-slot rectangle on the front. Single texture used for all
    // faces on alpha; front-vs-back face distinction is post-MVP polish.
    textures.push(gen_vendor_block());          // 177 vendor_block

    // Spec 26 Drafting Table — parchment-on-wood top with a faint
    // grid hint (drafting paper). Single texture for all faces.
    textures.push(gen_drafting_table());        // 178 drafting_table

    // Spec 22 Phase 7 — red-tinted smoke variant used when the source
    // campfire's raid_warning_active flag is set. Mesh swaps to this
    // layer per-cell; no new BlockId.
    textures.push(gen_campfire_smoke_warning()); // 179 campfire_smoke_warning

    // HP-2 (2026-05-22) — Chest. Plain oak base with an iron clasp +
    // keyhole on the side and a horizontal lid line across the top.
    textures.push(gen_chest_top());              // 180 chest_top
    textures.push(gen_chest_side());             // 181 chest_side

    // HP-3 (2026-05-23) — Brigand Hideout Banner. Purple-red banner on
    // a dark wooden post. Decorative-only block; pure procedural.
    textures.push(gen_brigand_hideout_banner()); // 182 brigand_hideout_banner

    // HP-3 v2 (2026-05-23) — Trophy Wall. Wood-grain plaque with a
    // stylised purple-and-red Chieftain trophy mounted at the centre.
    textures.push(gen_trophy_wall()); // 183 trophy_wall

    // Salt feature (2026-05-23) — 5 blocks, 6 textures (SALT_PATH has
    // distinct top + side faces).
    textures.push(gen_rock_salt());      // 184 rock_salt
    textures.push(gen_salt_lick());      // 185 salt_lick
    textures.push(gen_salt_lamp());      // 186 salt_lamp
    textures.push(gen_salt_block());     // 187 salt_block
    textures.push(gen_salt_path_top());  // 188 salt_path_top
    textures.push(gen_salt_path_side()); // 189 salt_path_side

    // Rubber feature (2026-05-23) — 4 blocks, 4 textures.
    textures.push(gen_rubber_log());        // 190 rubber_log
    textures.push(gen_rubber_planks());     // 191 rubber_planks
    textures.push(gen_rubber_leaves());     // 192 rubber_leaves
    textures.push(gen_rubber_log_tapped()); // 193 rubber_log_tapped

    // Mob Bounty Board (Spec 33, 2026-05-23).
    textures.push(gen_bounty_board());      // 194 bounty_board

    // Tip Jar (Spec 34, 2026-05-23).
    textures.push(gen_tip_jar());           // 195 tip_jar

    // Repair Bench (Spec 35, 2026-05-23).
    textures.push(gen_repair_bench());      // 196 repair_bench

    // Plot Marker (Spec 36, 2026-05-23).
    textures.push(gen_plot_marker());       // 197 plot_marker

    // Market Bell (Spec 37, 2026-05-23).
    textures.push(gen_market_bell());       // 198 market_bell

    // Auction Block (Spec 38, 2026-05-23).
    textures.push(gen_auction_block());     // 199 auction_block

    // Bazaar Block (Spec 39, 2026-05-23).
    textures.push(gen_bazaar_block());      // 200 bazaar_block

    // Player skin moved to the 64x64 avatar path (skin_texture); layers 201-206
    // retired as fillers to keep later indices stable (see skin_uv.rs /
    // build_skin_part_*). The 16x16 player layers are no longer sampled by any
    // renderer — both 3rd-person avatar and 1st-person viewmodel use the 64x64
    // skin atlas + skin_uv tables, which ignore part.tex_faces.
    for _ in 201..=206 { textures.push(gen_retired_layer()); } // 201-206 retired player skin

    // Block-break crack overlay (Spec 05 §2.2 — layers 207-216, stages 0-9).
    // Drawn over the targeted block as mining progresses; see TEX_CRACK_BASE.
    for stage in 0u8..10 {
        textures.push(gen_crack_stage(stage)); // 207-216 crack_stage_0..9
    }

    // Bespoke Bee + Squid coats (2026-05-27 — layers 217-219). Replace the
    // chicken/sheep stand-in textures the first mob-model pass borrowed.
    textures.push(gen_bee_body());   // 217 bee_body (yellow + black stripes)
    textures.push(gen_bee_head());   // 218 bee_head (near-black)
    textures.push(gen_squid_body()); // 219 squid_body (ocean blue)

    // Dye flowers (Spec 35) + fibre plants (Spec 36) — block textures
    // 220-224 (X-mesh plants), item icons 225-230. 2026-05-27.
    textures.push(gen_cornflower());                      // 220 cornflower
    textures.push(gen_field_poppy());                     // 221 field_poppy
    textures.push(gen_buttercup());                       // 222 buttercup
    textures.push(gen_cotton_plant());                    // 223 cotton_plant
    textures.push(gen_hemp_plant());                      // 224 hemp_plant
    textures.push(gen_farming_item(4310, 48, 80, 210));   // 225 blue_dye
    textures.push(gen_farming_item(4311, 212, 46, 40));   // 226 red_dye
    textures.push(gen_farming_item(4312, 246, 210, 56));  // 227 yellow_dye
    textures.push(gen_farming_item(4313, 240, 240, 232)); // 228 cotton item
    textures.push(gen_farming_item(4314, 150, 165, 100)); // 229 hemp_fibre
    textures.push(gen_farming_item(4315, 180, 145, 85));  // 230 rope

    // Magnesium (Spec 37 — ore block 231 + item icons 232-236). 2026-05-27.
    textures.push(gen_magnesium_ore());                   // 231 magnesium_ore block
    textures.push(gen_farming_item(4320, 210, 210, 220)); // 232 magnesium (silver)
    textures.push(gen_farming_item(4321, 200, 205, 140)); // 233 fertiliser (pale green)
    textures.push(gen_farming_item(4322, 245, 235, 170)); // 234 sparkler (gold spark)
    textures.push(gen_farming_item(4323, 252, 244, 220)); // 235 flare (brilliant white)
    textures.push(gen_farming_item(4324, 140, 140, 150)); // 236 firestarter (grey)

    // Dye Phase 2 (Spec 35 — item icons 237-246). 2026-05-27.
    textures.push(gen_farming_item(4330, 30, 30, 36));    // 237 black_dye
    textures.push(gen_farming_item(4331, 242, 242, 242)); // 238 white_dye
    textures.push(gen_farming_item(4332, 230, 128, 38));  // 239 orange_dye
    textures.push(gen_farming_item(4333, 76, 158, 56));   // 240 green_dye
    textures.push(gen_farming_item(4334, 140, 64, 178));  // 241 purple_dye
    textures.push(gen_farming_item(4335, 237, 140, 178)); // 242 pink_dye
    textures.push(gen_farming_item(4336, 140, 216, 76));  // 243 lime_dye
    textures.push(gen_farming_item(4337, 114, 178, 235)); // 244 light_blue_dye
    textures.push(gen_farming_item(4338, 114, 114, 122)); // 245 grey_dye
    textures.push(gen_farming_item(4339, 178, 178, 184)); // 246 light_grey_dye

    // Fibre Phase 2 (Spec 36 — crop stages 247-254, seed icons 255-256).
    for stage in 0u8..4 {
        textures.push(gen_crop_stage(4350 + stage as u32, [232, 232, 222], stage)); // 247-250 cotton
    }
    for stage in 0u8..4 {
        textures.push(gen_crop_stage(4360 + stage as u32, [60, 130, 50], stage)); // 251-254 hemp
    }
    textures.push(gen_farming_item(4370, 205, 208, 150)); // 255 cotton_seeds
    textures.push(gen_farming_item(4371, 150, 165, 95));  // 256 hemp_seeds

    // Coloured wallpaper (dyed-paper décor — layers 257-269). 2026-05-27.
    // The first 13 entries of WALLPAPER_PALETTE (white..light_grey) map to
    // these layers in order; the seed is palette-index + 4380.
    for (i, (_, rgb)) in WALLPAPER_PALETTE[..13].iter().enumerate() {
        textures.push(gen_wallpaper(4380 + i as u32, *rgb));
    }
    // 257 white, 258 black, 259 red, 260 blue, 261 yellow, 262 orange,
    // 263 green, 264 purple, 265 pink, 266 lime, 267 light_blue,
    // 268 grey, 269 light_grey

    // Spec 35 Phase 2 completion (2026-05-28) — Brown / Cyan / Magenta
    // dye item icons + matching wallpapers. Layers 270-275.
    textures.push(gen_farming_item(4340, 107, 69, 41));   // 270 brown_dye
    textures.push(gen_farming_item(4341, 51, 184, 199));  // 271 cyan_dye
    textures.push(gen_farming_item(4342, 209, 64, 158));  // 272 magenta_dye
    // The last 3 entries of WALLPAPER_PALETTE (brown, cyan, magenta) map to
    // layers 273-275; seeds 4393-4395 preserve the pre-refactor values.
    for (i, (_, rgb)) in WALLPAPER_PALETTE[13..].iter().enumerate() {
        textures.push(gen_wallpaper(4393 + i as u32, *rgb));
    }
    // 273 wallpaper_brown, 274 wallpaper_cyan, 275 wallpaper_magenta

    // Spec 36 Phase 2 (2026-05-28) — Rope's first consumer + textile
    // item icons. Reuse the farming-item swatch painter — these are
    // crafted goods, not crops, so a coloured swatch is the right v1.
    textures.push(gen_farming_item(4400, 184, 148, 92));  // 276 lead — braided tan
    textures.push(gen_farming_item(4401, 245, 240, 220)); // 277 cloth — soft cotton off-white
    textures.push(gen_farming_item(4402, 204, 188, 128)); // 278 canvas — coarse beige sailcloth

    // Spec 35 farmable-flower follow-on (2026-05-28). 9 crop-stage
    // block textures (279-287) + 3 seed icons (288-290). Per-stage
    // colour steps from sprout-green at 0 → mature-flower tint at 2,
    // so the player can read maturity at a glance on the X-mesh
    // plant shape.
    let stage_color = |stage: u8, mature: [u8; 3]| -> [u8; 3] {
        let sprout = [140u8, 168u8, 100u8];
        let t: f32 = match stage { 0 => 0.0, 1 => 0.55, _ => 0.85 };
        let lerp = |a: u8, b: u8| -> u8 {
            (a as f32 + (b as f32 - a as f32) * t).clamp(0.0, 255.0) as u8
        };
        [lerp(sprout[0], mature[0]), lerp(sprout[1], mature[1]), lerp(sprout[2], mature[2])]
    };
    for stage in 0u8..3 {
        let c = stage_color(stage, [78, 102, 217]); // cornflower blue
        textures.push(gen_crop_stage(4500 + stage as u32, c, stage)); // 279-281
    }
    for stage in 0u8..3 {
        let c = stage_color(stage, [199, 46, 41]); // poppy red
        textures.push(gen_crop_stage(4503 + stage as u32, c, stage)); // 282-284
    }
    for stage in 0u8..3 {
        let c = stage_color(stage, [237, 214, 56]); // buttercup yellow
        textures.push(gen_crop_stage(4506 + stage as u32, c, stage)); // 285-287
    }
    textures.push(gen_farming_item(4510, 142, 168, 199)); // 288 cornflower_seeds
    textures.push(gen_farming_item(4511, 199, 142, 108)); // 289 field_poppy_seeds
    textures.push(gen_farming_item(4512, 220, 200, 108)); // 290 buttercup_seeds

    // Spec 38 cyanotype-art (v1, 2026-05-28). Blueprint-blue square
    // with a white border — the "framed picture" look. Direct fill
    // (no procedural noise) so the frame reads sharp.
    textures.push(gen_cyanotype_print()); // 291

    // Spec 35 dyed-décor — Bunting (2026-05-28). 16 layers (292-307),
    // one per dye colour. Pattern: row of triangular flags on a
    // dark cord, with the dye colour filling the triangles.
    let bunting_colours: [[u8; 3]; 16] = [
        [238, 238, 238], // white
        [40, 40, 48],    // black
        [212, 60, 56],   // red
        [60, 92, 209],   // blue
        [237, 214, 56],  // yellow
        [224, 128, 41],  // orange
        [82, 158, 61],   // green
        [133, 66, 173],  // purple
        [234, 148, 184], // pink
        [143, 214, 82],  // lime
        [122, 184, 235], // light_blue
        [115, 115, 122], // grey
        [184, 184, 190], // light_grey
        [115, 75, 45],   // brown
        [56, 184, 196],  // cyan
        [209, 70, 166],  // magenta
    ];
    for (i, rgb) in bunting_colours.iter().enumerate() {
        textures.push(gen_bunting(4600 + i as u32, *rgb)); // 292-307
    }

    // Spec 35 dyed-décor — Paper Lantern (2026-05-28). 16 layers
    // (308-323). Lantern tints are paler/warmer than the dye-pure
    // bunting colours — the glow lifts the tint toward white so a
    // yellow lantern reads as sun-warm-yellow, not dye-yellow.
    let lantern_colours: [[u8; 3]; 16] = [
        [246, 246, 240], // white — barely-tinted ivory
        [76, 76, 86],    // black — dark grey-paper
        [232, 130, 122], // red — warm coral
        [136, 168, 232], // blue — sky
        [248, 232, 142], // yellow — sun
        [240, 184, 130], // orange — soft pumpkin
        [156, 210, 138], // green — leafy
        [188, 148, 218], // purple — lavender
        [248, 200, 218], // pink — pale rose
        [200, 232, 148], // lime — pastel
        [188, 222, 246], // light_blue — pale sky
        [168, 168, 174], // grey
        [218, 218, 224], // light_grey
        [184, 142, 110], // brown — kraft paper
        [148, 226, 230], // cyan — sea
        [232, 148, 208], // magenta — bright rose
    ];
    for (i, rgb) in lantern_colours.iter().enumerate() {
        textures.push(gen_paper_lantern(4700 + i as u32, *rgb)); // 308-323
    }

    // Spec 35 dyed-décor — Kite (2026-05-28). 16 layers (324-339).
    // Kite colours match the bunting palette so a matching set
    // reads across décor families.
    let kite_colours: [[u8; 3]; 16] = [
        [238, 238, 238], [40, 40, 48], [212, 60, 56], [60, 92, 209],
        [237, 214, 56], [224, 128, 41], [82, 158, 61], [133, 66, 173],
        [234, 148, 184], [143, 214, 82], [122, 184, 235], [115, 115, 122],
        [184, 184, 190], [115, 75, 45], [56, 184, 196], [209, 70, 166],
    ];
    for (i, rgb) in kite_colours.iter().enumerate() {
        textures.push(gen_kite(4800 + i as u32, *rgb)); // 324-339
    }

    // Banner — 16 layers (340-355). Same palette as kite + bunting.
    let banner_colours: [[u8; 3]; 16] = [
        [238, 238, 238], [40, 40, 48], [212, 60, 56], [60, 92, 209],
        [237, 214, 56], [224, 128, 41], [82, 158, 61], [133, 66, 173],
        [234, 148, 184], [143, 214, 82], [122, 184, 235], [115, 115, 122],
        [184, 184, 190], [115, 75, 45], [56, 184, 196], [209, 70, 166],
    ];
    for (i, rgb) in banner_colours.iter().enumerate() {
        textures.push(gen_banner(4900 + i as u32, *rgb)); // 340-355
    }

    // Sail — 16 layers (356-371). Same palette as banner.
    let sail_colours: [[u8; 3]; 16] = [
        [238, 238, 238], [40, 40, 48], [212, 60, 56], [60, 92, 209],
        [237, 214, 56], [224, 128, 41], [82, 158, 61], [133, 66, 173],
        [234, 148, 184], [143, 214, 82], [122, 184, 235], [115, 115, 122],
        [184, 184, 190], [115, 75, 45], [56, 184, 196], [209, 70, 166],
    ];
    for (i, rgb) in sail_colours.iter().enumerate() {
        textures.push(gen_sail(5000 + i as u32, *rgb)); // 356-371
    }

    // Tent — 1 layer (372). Canvas dome shape.
    textures.push(gen_tent(5100));

    // Track — 1 layer (373). Two steel rails over a sleeper base.
    textures.push(gen_track(5200));

    // #30 building-detail blocks (2026-06-16).
    // Ladder — 1 layer (374). Two oak side-rails + horizontal rungs.
    textures.push(gen_ladder(5300));
    // Carpet — 1 layer (375). Soft woven wool weave (neutral, tintable).
    textures.push(gen_carpet(5400));

    // #47 — Grave — 1 layer (376). Weathered stone headstone slab with a mound.
    textures.push(gen_grave(5500));

    // #15 — tiered-storage chests — 8 layers (377-384): top+side per tier,
    // tinted from the shared chest gen (iron clasp stays metal across tiers).
    textures.push(gen_chest_top_tinted(175, 108, 68)); // 377 copper top
    textures.push(gen_chest_side_tinted(170, 104, 64)); // 378 copper side
    textures.push(gen_chest_top_tinted(152, 154, 160)); // 379 iron top
    textures.push(gen_chest_side_tinted(148, 150, 156)); // 380 iron side
    textures.push(gen_chest_top_tinted(112, 198, 205)); // 381 diamond top
    textures.push(gen_chest_side_tinted(108, 192, 200)); // 382 diamond side
    textures.push(gen_chest_top_tinted(220, 150, 55)); // 383 satori top
    textures.push(gen_chest_side_tinted(214, 145, 52)); // 384 satori side

    // Spec 48 (Electricity) — power-block textures.
    textures.push(gen_cable());                 // 385: TEX_CABLE
    textures.push(gen_cable_lit());             // 386: TEX_CABLE_LIT
    textures.push(gen_lamp());                  // 387: TEX_ELECTRIC_LAMP
    textures.push(gen_lamp_lit());              // 388: TEX_ELECTRIC_LAMP_LIT
    textures.push(gen_lever());                 // 389: TEX_LEVER
    textures.push(gen_button());                // 390: TEX_BUTTON
    textures.push(gen_pressure_plate());        // 391: TEX_PRESSURE_PLATE
    textures.push(gen_logic_gate());            // 392: TEX_LOGIC_GATE
    textures.push(gen_hand_crank());            // 393: TEX_HAND_CRANK
    textures.push(gen_steam_generator());       // 394: TEX_STEAM_GENERATOR
    textures.push(gen_steam_generator_lit());   // 395: TEX_STEAM_GENERATOR_LIT
    textures.push(gen_battery());               // 396: TEX_BATTERY
    // Spec 48 Phase 2 — sensors.
    textures.push(gen_beam_sensor());           // 397: TEX_BEAM_SENSOR
    textures.push(gen_mirror());                // 398: TEX_MIRROR
    textures.push(gen_motion_sensor());         // 399: TEX_MOTION_SENSOR

    // Spec 49 (Explosives) — 400..=404.
    textures.push(gen_brimstone());             // 400: TEX_BRIMSTONE
    textures.push(gen_nitre_ore());             // 401: TEX_NITRE_ORE
    textures.push(gen_composter());             // 402: TEX_COMPOSTER
    textures.push(gen_blasting_keg());          // 403: TEX_BLASTING_KEG
    textures.push(gen_plunger_detonator());     // 404: TEX_PLUNGER_DETONATOR
    // P-bugfix (Workshop paint): 16 flat solid-colour layers (405..=420), one per
    // dye/wallpaper colour in `block::PAINT_WALLPAPERS` order. The Workshop paints
    // a dye as a SOLID colour by mapping its wallpaper block to its layer here
    // (`block::wallpaper_solid_layer`) instead of the patterned wallpaper texture.
    for &wp in &crate::block::PAINT_WALLPAPERS {
        let [r, g, b, _a] = wallpaper_rgba(wp);
        textures.push(gen_solid_colour(r, g, b)); // 405..=420
    }

    // #130 — per-species leaf textures, 421..=425 (lock-step with TEXTURE_KEYS +
    // the TEX_*_LEAVES constants). Retinted oak-leaf noise; distinct seeds so the
    // speckle differs, base hues echo each BlockDef.color.
    textures.push(gen_species_leaves(901, 105, 150, 72)); // 421 birch  — light yellow-green
    textures.push(gen_species_leaves(902, 38, 84, 60));   // 422 spruce — dark blue-green conifer
    textures.push(gen_species_leaves(903, 40, 145, 32));  // 423 jungle — vivid green
    textures.push(gen_species_leaves(904, 96, 122, 44));  // 424 acacia — olive
    textures.push(gen_species_leaves(905, 32, 74, 28));   // 425 dark oak — deep green

    // #132 — per-species WOOD textures, 426..=440 (3 per species: log_side,
    // log_top, planks; lock-step with TEXTURE_KEYS + the TEX_* consts). Mirrors
    // the leaf retint above; finishes the per-species trees (#130 did canopies).
    textures.push(gen_species_log_side(906, 205, 200, 185)); // 426 birch log side  — pale cream bark
    textures.push(gen_species_log_top(907, 210, 195, 160));  // 427 birch log top
    textures.push(gen_species_planks(908, 225, 215, 180));   // 428 birch planks
    textures.push(gen_species_log_side(909, 115, 70, 48));   // 429 spruce log side — reddish-brown
    textures.push(gen_species_log_top(910, 165, 110, 75));   // 430 spruce log top
    textures.push(gen_species_planks(911, 165, 110, 78));    // 431 spruce planks
    textures.push(gen_species_log_side(912, 110, 92, 50));   // 432 jungle log side — olive-brown
    textures.push(gen_species_log_top(913, 160, 140, 85));   // 433 jungle log top
    textures.push(gen_species_planks(914, 170, 150, 95));    // 434 jungle planks
    textures.push(gen_species_log_side(915, 140, 80, 45));   // 435 acacia log side — orange-brown
    textures.push(gen_species_log_top(916, 185, 120, 70));   // 436 acacia log top
    textures.push(gen_species_planks(917, 200, 130, 70));    // 437 acacia planks
    textures.push(gen_species_log_side(918, 70, 48, 30));    // 438 dark oak log side — deep brown
    textures.push(gen_species_log_top(919, 105, 80, 50));    // 439 dark oak log top
    textures.push(gen_species_planks(920, 110, 80, 52));     // 440 dark oak planks

    // Satoshi the founder-sage (2026-06-25 — layers 441-443).
    textures.push(gen_satoshi_head_front()); // 441 TEX_SATOSHI_HEAD_FRONT
    textures.push(gen_satoshi_head_side());  // 442 TEX_SATOSHI_HEAD_SIDE
    textures.push(gen_satoshi_trim());       // 443 TEX_SATOSHI_TRIM

    // Rail auto-connect (2026-07-02) — east–west rail (gen_track rotated 90°).
    textures.push(gen_track_ew(5201));       // 444: TEX_TRACK_EW

    // Rail/cable connecting-geometry v2 (2026-07-02) — clean solid metal colours
    // (not the pixel rail texture). 445=steel rail, 446=copper cable, 447=lit cable.
    textures.push(gen_solid_colour(150, 152, 158)); // 445: TEX_RAIL_STEEL
    textures.push(gen_solid_colour(176, 110, 66));  // 446: TEX_CABLE_COPPER
    textures.push(gen_solid_colour(255, 196, 120)); // 447: TEX_CABLE_COPPER_LIT
    // Sleeper/ballast bed under the rails (dark brown), restored after the
    // clean-geometry rails dropped the old baked-in texture bed.
    textures.push(gen_solid_colour(82, 64, 46)); // 448: TEX_RAIL_BASE

    // Fire (2026-07-04 gap-fill wave) — licking flames, partly transparent.
    textures.push(gen_fire()); // 449: TEX_FIRE

    // Dispenser/Dropper faces (2026-07-04) — furnace metal + a round muzzle.
    textures.push(gen_muzzle_face(18)); // 450: TEX_DISPENSER_SIDE (dark bore)
    textures.push(gen_muzzle_face(70)); // 451: TEX_DROPPER_SIDE (shallow dish)

    // Sapling (2026-07-04) — shared sprout silhouette, tinted per species by
    // the block colour.
    textures.push(gen_sapling()); // 452: TEX_SAPLING

    // Particle billboards (2026-07-05). White/alpha masks — the per-instance
    // colour tints them, so one soft blob serves smoke, splash and snow.
    textures.push(gen_particle_soft()); // 453: TEX_PARTICLE_SOFT
    textures.push(gen_particle_spark()); // 454: TEX_PARTICLE_SPARK
    textures.push(gen_particle_streak()); // 455: TEX_PARTICLE_STREAK
    textures.push(gen_particle_chip()); // 456: TEX_PARTICLE_CHIP

    // Pet Bed (2026-07-06 pets wave) — wool cushion in a wood-crate frame.
    textures.push(gen_pet_bed_top());  // 457: TEX_PET_BED_TOP
    textures.push(gen_pet_bed_side()); // 458: TEX_PET_BED_SIDE

    // Species-tint run (mob-species-tint foundation, 2026-07-11) — per-species
    // tinted copies of donor coat layers, 459.. in SPECIES_TINTS order. The
    // tint colour is `MobDef.color` (the per-mob TOML) so data stays the
    // single source of truth. MUST remain the LAST run: `tinted_layer` indexes
    // from PRE_TINT_LAYER_COUNT.
    debug_assert_eq!(textures.len() as u32, PRE_TINT_LAYER_COUNT);
    for st in SPECIES_TINTS {
        let colour = crate::mob::mob_def(st.kind).color;
        for &donor in st.donors {
            let tinted = tint_rgba(&textures[donor as usize], colour);
            textures.push(tinted);
        }
    }

    // Post-tint block layers (506.., Spec 48 Phase 4 Water Wheel, 2026-09-06).
    // Block layers that arrive AFTER the species-tint bake append here rather
    // than in the 400-block run, because the tint run is index-derived from
    // PRE_TINT_LAYER_COUNT and has to stay contiguous.
    debug_assert_eq!(textures.len() as u32, POST_TINT_LAYER_START);
    textures.push(gen_water_wheel());         // 506: TEX_WATER_WHEEL
    textures.push(gen_water_wheel_turning()); // 507: TEX_WATER_WHEEL_TURNING

    // Copper Ore (2026-09-07, Wind, Copper & Electricity wave, Spec 02 §1) —
    // appended after the Water Wheel layers, same post-tint reasoning.
    textures.push(gen_copper_ore()); // 508: TEX_COPPER_ORE

    // Windmill idle/turning (2026-09-07, Wind, Copper & Electricity wave §2.2)
    // — same post-tint reasoning again. Top/bottom reuse the plank cap, so the
    // pair costs two layers and lands the atlas on 511 of the 512 floor.
    textures.push(gen_windmill());         // 509: TEX_WINDMILL
    textures.push(gen_windmill_turning()); // 510: TEX_WINDMILL_TURNING

    textures
}

/// Water Wheel (506) — a paddle wheel seen side-on: eight wooden paddles on an
/// iron axle inside a plank rim, still.
fn gen_water_wheel() -> Vec<u8> {
    water_wheel_face(4820, 0.0, false)
}

/// Water Wheel turning (507) — the same wheel mid-spin: the paddles sit half a
/// segment round and smear toward a ghosted trailing position, with a few
/// thrown water flecks, so the face reads as motion rather than a second wheel.
fn gen_water_wheel_turning() -> Vec<u8> {
    water_wheel_face(4821, std::f32::consts::FRAC_PI_8, true)
}

/// Windmill (509) — four canvas sails on a plank tower face, still.
fn gen_windmill() -> Vec<u8> {
    windmill_face(6110, 0.0, false)
}

/// Windmill turning (510) — the same sails a third of a quadrant round, each
/// trailing a ghost of where it just was, so the face reads as motion rather
/// than as a second, differently-aimed mill.
fn gen_windmill_turning() -> Vec<u8> {
    windmill_face(6111, std::f32::consts::FRAC_PI_8, true)
}

/// The shared Windmill face. `spin` rotates the sails; `blurred` adds the
/// motion ghost behind each one. Same polar construction as
/// [`water_wheel_face`], but four broad canvas sails on a plank tower instead
/// of eight paddles in a rim — the two blocks must not read as the same thing
/// at a glance.
fn windmill_face(seed: u32, spin: f32, blurred: bool) -> Vec<u8> {
    let seg = std::f32::consts::TAU / 4.0;
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, seed) % 9) as i32 - 4;
            let dx = x as f32 - 7.5;
            let dy = y as f32 - 7.5;
            let r = (dx * dx + dy * dy).sqrt();
            let a = dy.atan2(dx);

            // Backdrop — the plank tower the sails are mounted on, with a
            // darker line every fourth row for the plank courses.
            let course = if y % 4 == 0 { -22 } else { 0 };
            let mut cr = clamp_u8(148 + n + course);
            let mut cg = clamp_u8(110 + n + course);
            let mut cb = clamp_u8(68 + n + course);

            // One sail = a broad angular wedge from the hub out to the tip.
            let sail = |offset: f32| -> bool {
                let t = ((a - spin - offset).rem_euclid(seg) / seg - 0.5).abs();
                t < 0.13 && (1.8..=7.4).contains(&r)
            };
            if blurred && sail(-seg * 0.30) {
                // Motion ghost — where the sail was a moment ago.
                cr = clamp_u8(170 + n);
                cg = clamp_u8(158 + n);
                cb = clamp_u8(120 + n);
            }
            if sail(0.0) {
                cr = clamp_u8(214 + n);
                cg = clamp_u8(200 + n);
                cb = clamp_u8(150 + n);
            }
            // Sail spar — the darker batten running down the middle of each
            // sail, so the canvas doesn't read as a flat cross.
            let spar = ((a - spin).rem_euclid(seg) / seg - 0.5).abs() < 0.03;
            if spar && (1.8..=7.4).contains(&r) {
                cr = clamp_u8(126 + n);
                cg = clamp_u8(96 + n);
                cb = clamp_u8(58 + n);
            }
            // Iron hub the sails turn on, with a darker bore.
            if r <= 1.8 {
                cr = clamp_u8(150 + n);
                cg = clamp_u8(152 + n);
                cb = clamp_u8(160 + n);
            }
            if r <= 0.9 {
                cr = clamp_u8(84 + n);
                cg = clamp_u8(86 + n);
                cb = clamp_u8(92 + n);
            }
            set_px(&mut px, x, y, cr, cg, cb, 255);
        }
    }
    px
}

/// Shared painter for both Water Wheel faces. `spin` rotates the eight paddles
/// about the axle; `blurred` adds the motion ghost + water flecks.
fn water_wheel_face(seed: u32, spin: f32, blurred: bool) -> Vec<u8> {
    let seg = std::f32::consts::TAU / 8.0;
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, seed) % 9) as i32 - 4;
            let dx = x as f32 - 7.5;
            let dy = y as f32 - 7.5;
            let r = (dx * dx + dy * dy).sqrt();
            let a = dy.atan2(dx);

            // Backdrop — the shaded gap you see between the paddles.
            let mut cr = clamp_u8(58 + n / 2);
            let mut cg = clamp_u8(62 + n / 2);
            let mut cb = clamp_u8(70 + n / 2);

            // One paddle = an angular wedge running from the hub to the rim.
            let paddle = |offset: f32| -> bool {
                let t = ((a - spin - offset).rem_euclid(seg) / seg - 0.5).abs();
                t < 0.17 && (2.4..=7.3).contains(&r)
            };
            if blurred && paddle(-seg * 0.35) {
                // Motion ghost — where the paddle was a moment ago.
                cr = clamp_u8(112 + n);
                cg = clamp_u8(92 + n);
                cb = clamp_u8(66 + n);
            }
            if paddle(0.0) {
                cr = clamp_u8(158 + n);
                cg = clamp_u8(118 + n);
                cb = clamp_u8(72 + n);
            }
            // Plank rim around the outside.
            if (6.4..=7.6).contains(&r) {
                cr = clamp_u8(126 + n);
                cg = clamp_u8(92 + n);
                cb = clamp_u8(56 + n);
            }
            // Iron axle hub, with a darker bore at its centre.
            if r <= 2.4 {
                cr = clamp_u8(150 + n);
                cg = clamp_u8(152 + n);
                cb = clamp_u8(160 + n);
            }
            if r <= 1.1 {
                cr = clamp_u8(84 + n);
                cg = clamp_u8(86 + n);
                cb = clamp_u8(92 + n);
            }
            // Thrown water — a few pale flecks, only on the turning face.
            if blurred && r > 5.0 && px_hash(x, y, seed ^ 0x5EED).is_multiple_of(17) {
                cr = clamp_u8(150 + n);
                cg = clamp_u8(196 + n);
                cb = clamp_u8(226 + n);
            }
            set_px(&mut px, x, y, cr, cg, cb, 255);
        }
    }
    px
}

/// The mean luminance every donor layer is normalized to before the species
/// colour is applied. Bright enough that a near-white tint (polar bear)
/// actually reads white, low enough that saturated tints (fox orange) keep
/// their punch instead of clamping.
const TINT_TARGET_LUM: f32 = 200.0;

/// Per-pixel species recolour (alpha untouched) — the species-tint bake.
/// Donor grayscale (Rec. 601 luma), normalized so the layer's MEAN luminance
/// hits [`TINT_TARGET_LUM`], then multiplied by the species colour.
///
/// Why not simpler formulas (both tried, both failed the comparison sheet):
/// a straight RGB multiply can never brighten a dark donor — the white polar
/// bear stayed cow-brown; an un-normalized luminance recolour preserves the
/// donor's brightness (bear mean 127 vs donor 128) so the bear read grey-
/// brown. Normalize-then-multiply keeps coat detail, keeps the fox saturated,
/// and lifts the bear to white.
fn tint_rgba(donor: &[u8], c: [f32; 3]) -> Vec<u8> {
    let lum = |p: &[u8]| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
    let mean = donor.chunks(4).map(lum).sum::<f32>() / (donor.len() as f32 / 4.0);
    let scale = TINT_TARGET_LUM / mean.max(1.0);
    let mut out = donor.to_vec();
    for px in out.chunks_mut(4) {
        let l = lum(px) * scale;
        for ch in 0..3 {
            px[ch] = (l * c[ch]).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// Soft radial alpha blob: white, alpha falls off with radius².
fn gen_particle_soft() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..16i32 {
        for x in 0..16i32 {
            let (dx, dy) = (x as f32 - 7.5, y as f32 - 7.5);
            let d = (dx * dx + dy * dy) / (7.5 * 7.5);
            let a = ((1.0 - d).max(0.0) * 255.0) as u8;
            let idx = ((y * 16 + x) * 4) as usize;
            px[idx] = 255;
            px[idx + 1] = 255;
            px[idx + 2] = 255;
            px[idx + 3] = a;
        }
    }
    px
}

/// Hot spark: tiny intense core, quick falloff.
fn gen_particle_spark() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..16i32 {
        for x in 0..16i32 {
            let (dx, dy) = (x as f32 - 7.5, y as f32 - 7.5);
            let d = (dx * dx + dy * dy).sqrt();
            let a = ((1.0 - d / 4.5).max(0.0) * 255.0) as u8;
            let idx = ((y * 16 + x) * 4) as usize;
            px[idx] = 255;
            px[idx + 1] = 255;
            px[idx + 2] = 240;
            px[idx + 3] = a;
        }
    }
    px
}

/// Rain streak: a 2-px vertical line, alpha fading toward both ends.
fn gen_particle_streak() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..16i32 {
        let edge = (y.min(15 - y) as f32 / 7.5).min(1.0);
        let a = (edge * 220.0) as u8;
        for x in 7..=8i32 {
            let idx = ((y * 16 + x) * 4) as usize;
            px[idx] = 255;
            px[idx + 1] = 255;
            px[idx + 2] = 255;
            px[idx + 3] = a;
        }
    }
    px
}

/// Debris chip: hard-edged 10×10 square with light per-pixel speckle.
fn gen_particle_chip() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 3..13i32 {
        for x in 3..13i32 {
            let n = ((x * 7 + y * 13) % 4) as i16 * 10;
            let v = (235 - n).clamp(0, 255) as u8;
            let idx = ((y * 16 + x) * 4) as usize;
            px[idx] = v;
            px[idx + 1] = v;
            px[idx + 2] = v;
            px[idx + 3] = 255;
        }
    }
    px
}

/// Pet Bed top (2026-07-06 pets wave) — a round wool cushion sitting inside
/// a wood-crate frame border. Deliberately distinct from the player BED's
/// pillow-and-mattress read (`gen_bed_top`): no pillow, a rounder cushion,
/// and a visible wood frame on all four edges (a basket, not a mattress).
fn gen_pet_bed_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let (cx, cy) = (7.5f32, 7.5f32);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 5301) % 9) as i32 - 4;
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let d2 = dx * dx + dy * dy;
            let (r, g, b) = if x == 0 || x == SIZE - 1 || y == 0 || y == SIZE - 1 {
                // Wood crate frame border.
                (clamp_u8(120 + n), clamp_u8(85 + n), clamp_u8(45 + n))
            } else if d2 <= 36.0 {
                // Round wool cushion, warm rust-red, quilted with darker flecks.
                (clamp_u8(175 + n), clamp_u8(45 + n), clamp_u8(35 + n))
            } else {
                // Wood crate base peeking around the cushion's corners.
                (clamp_u8(140 + n), clamp_u8(100 + n), clamp_u8(55 + n))
            };
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Pet Bed side (2026-07-06 pets wave) — a wood-crate wall with the wool
/// cushion peeking over the rim as a red band along the top few rows.
fn gen_pet_bed_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 5302) % 11) as i32 - 5;
            let (r, g, b) = if y < 4 {
                // Cushion band along the rim.
                (clamp_u8(175 + n), clamp_u8(45 + n), clamp_u8(35 + n))
            } else {
                // Wood crate plank wall, with a darker seam every 4 rows.
                let plank_line = (y - 4) % 4 == 0;
                if plank_line {
                    (clamp_u8(95 + n), clamp_u8(65 + n), clamp_u8(30 + n))
                } else {
                    (clamp_u8(140 + n), clamp_u8(100 + n), clamp_u8(55 + n))
                }
            };
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Sapling sprout — a thin stem with a small leaf tuft, transparent
/// background (non-solid small-cube block; the tuft reads at 16px).
fn gen_sapling() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let mut set = |x: i32, y: i32, r: u8, g: u8, b: u8| {
        if (0..16).contains(&x) && (0..16).contains(&y) {
            let idx = ((y * 16 + x) * 4) as usize;
            px[idx] = r;
            px[idx + 1] = g;
            px[idx + 2] = b;
            px[idx + 3] = 255;
        }
    };
    // Stem (texture bottom rows y=11..16).
    for y in 8..16 {
        set(7, y, 96, 70, 40);
        set(8, y, 110, 80, 48);
    }
    // Leaf tuft — diamond around (7.5, 5).
    for y in 1..9i32 {
        let half = 4 - (y - 5).abs();
        for x in (8 - half)..(8 + half) {
            let n = ((x * 5 + y * 11) % 4) as u8 * 12;
            set(x, y, 60 + n / 2, 150 + n, 55);
        }
    }
    px
}

/// Furnace-grey face with a centred round muzzle. `bore` is the muzzle's
/// brightness (dark = a dispenser's barrel, lighter = a dropper's dish).
fn gen_muzzle_face(bore: u8) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..16i32 {
        for x in 0..16i32 {
            let n = ((x * 7 + y * 13) % 5) as i16 - 2; // stone-ish speckle
            let base = (110 + n * 4).clamp(0, 255) as u8;
            let (dx, dy) = (x - 8, y - 8);
            let r2 = dx * dx + dy * dy;
            let (r, g, b) = if r2 <= 9 {
                (bore, bore, bore) // muzzle
            } else if r2 <= 16 {
                (60u8, 60u8, 62u8) // muzzle rim
            } else {
                (base, base, (base as u16 + 4).min(255) as u8)
            };
            let idx = ((y * 16 + x) * 4) as usize;
            px[idx] = r;
            px[idx + 1] = g;
            px[idx + 2] = b;
            px[idx + 3] = 255;
        }
    }
    px
}

/// Fire — vertical flame tongues: opaque orange/yellow near the bottom fading
/// to transparent at the tips, deterministic per-pixel hash so the tongues are
/// ragged rather than banded. (Animation can arrive later via the disk-pack
/// strip + `.anim` sidecar path like water.)
fn gen_fire() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..16u32 {
        for x in 0..16u32 {
            // y=15 is the texture top; flames rise, so density falls with
            // height. Ragged tongue edge from a position hash.
            let h = x.wrapping_mul(31).wrapping_add(y.wrapping_mul(17)) ^ (x * y).wrapping_add(7);
            let tongue = h % 5; // 0..4 raggedness
            let height_from_base = 15 - y; // 0 at the texture bottom row
            let cutoff = 9 + tongue; // tongues reach 9..13 px tall
            if height_from_base > cutoff {
                continue; // transparent above the tongue tip
            }
            // Hot core → yellow low, orange mid, red tips.
            let (r, g, b) = if height_from_base < 3 {
                (255, 220, 90)
            } else if height_from_base < 7 {
                (255, 150, 40)
            } else {
                (220, 70, 20)
            };
            let idx = ((y * 16 + x) * 4) as usize;
            px[idx] = r;
            px[idx + 1] = g;
            px[idx + 2] = b;
            px[idx + 3] = 255;
        }
    }
    px
}

/// Flat single-colour 16×16 RGBA fill (fully opaque). Used for the Workshop's
/// solid paint-colour layers so a dye paints as one flat colour.
fn gen_solid_colour(r: u8, g: u8, b: u8) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for texel in px.chunks_exact_mut(4) {
        texel.copy_from_slice(&[r, g, b, 255]);
    }
    px
}

/// #47 — Grave face: a weathered grey headstone — a rounded-top slab on a
/// darker earth mound, faintly mottled so it reads as old stone.
fn gen_grave(seed: u32) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, seed);
            let n = (h % 13) as i32 - 6;
            // Bottom 4px = earth mound; above = stone headstone.
            let (r, g, b) = if y >= SIZE - 4 {
                (clamp_u8(86 + n), clamp_u8(64 + n), clamp_u8(40 + n))
            } else {
                (clamp_u8(150 + n), clamp_u8(150 + n), clamp_u8(156 + n))
            };
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// #30 — Ladder face: two vertical oak rails down the edges with regularly
/// spaced horizontal rungs, transparent gaps between, so it reads as a ladder
/// against whatever's behind it.
fn gen_ladder(seed: u32) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let rail = |n: i32| (clamp_u8(120 + n), clamp_u8(86 + n), clamp_u8(48 + n));
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, seed);
            let n = (h % 7) as i32 - 3;
            // Side rails: leftmost/rightmost 3px columns. Rungs: 3px bands every 6px.
            let is_rail = !(3..SIZE - 3).contains(&x);
            let is_rung = (y % 6) < 3 && (3..SIZE - 3).contains(&x);
            if is_rail || is_rung {
                let (r, g, b) = rail(n);
                set_px(&mut px, x, y, r, g, b, 255);
            } else {
                set_px(&mut px, x, y, 0, 0, 0, 0); // transparent gap
            }
        }
    }
    px
}

/// #30 — Carpet face: a soft, slightly-noisy wool weave. Near-white so the
/// per-block `color` tint reads cleanly (matches the dyed-décor pattern).
fn gen_carpet(seed: u32) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, seed);
            let n = (h % 11) as i32 - 5;
            // Subtle 2px weave checker so it isn't a flat fill.
            let weave = ((x / 2) + (y / 2)) % 2 == 0;
            let base = if weave { 224 } else { 212 };
            set_px(&mut px, x, y, clamp_u8(base + n), clamp_u8(base + n), clamp_u8(base + n), 255);
        }
    }
    px
}

/// Tent block face — beige canvas dome with a darker ridgepole down
/// the centre + two guy-line stays trailing from the top corners.
/// Fully opaque. Cross-hatch weave like the Sail painter so it reads
/// as the same material family. Single texture — every face of the
/// tent uses this same view, which is the v1 single-block tent feel.
fn gen_tent(seed: u32) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let canvas = [198, 178, 130]; // beige sailcloth
    let ridge = [104, 84, 52];    // dark ridgepole
    let guy = [60, 50, 38];       // dark guy-line
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, seed);
            let noise = (h % 9) as i32 - 4;
            let weave = if x % 4 == 0 || y % 4 == 0 { -10 } else { 0 };
            set_px(
                &mut px,
                x,
                y,
                clamp_u8(canvas[0] + noise + weave),
                clamp_u8(canvas[1] + noise + weave),
                clamp_u8(canvas[2] + noise + weave),
                255,
            );
        }
    }
    // Ridgepole — vertical 2px-wide bar at the centre.
    for y in 0..SIZE {
        for dx in -1..=0 {
            let x = (SIZE as i32 / 2 + dx) as u32;
            set_px(&mut px, x, y, ridge[0], ridge[1], ridge[2], 255);
        }
    }
    // Two guy-line stays — from top corners slanting outward (the
    // anchor ropes that hold the dome to the ground).
    for t in 0..6 {
        let lx = t as u32;
        let ly = (t * 2) as u32;
        if lx < SIZE && ly < SIZE {
            set_px(&mut px, lx, ly, guy[0], guy[1], guy[2], 255);
        }
        let rx = SIZE - 1 - t as u32;
        let ry = (t * 2) as u32;
        if rx < SIZE && ry < SIZE {
            set_px(&mut px, rx, ry, guy[0], guy[1], guy[2], 255);
        }
    }
    px
}

/// Track block face — two parallel steel rails running top-to-bottom over a
/// dark-brown sleeper (railroad-tie) base, with lighter cross-ties every few
/// rows so it reads as laid track from above. Single texture used on every
/// face; the block renders as a thin ground slab (`mesh.rs`) so only the top
/// view matters in practice. Fully opaque pixels.
fn gen_track(seed: u32) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let base = [74, 56, 38];  // dark-brown sleeper/ballast
    let tie = [96, 72, 48];   // lighter wooden cross-tie
    let rail = [150, 152, 158]; // steel rail
    // Sleeper base with faint per-pixel noise + lighter cross-ties every
    // 5th row so the bed reads as a run of railroad ties.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, seed);
            let noise = (h % 7) as i32 - 3;
            let c = if y % 5 == 0 { tie } else { base };
            set_px(
                &mut px,
                x,
                y,
                clamp_u8(c[0] + noise),
                clamp_u8(c[1] + noise),
                clamp_u8(c[2] + noise),
                255,
            );
        }
    }
    // Two parallel rails — 2px-wide steel bars, inset symmetrically from the
    // edges so a cart (later tasks) reads as running between them.
    for y in 0..SIZE {
        let h = px_hash(0, y, seed.wrapping_add(1));
        let noise = (h % 5) as i32 - 2;
        for x in [4u32, 5, 10, 11] {
            set_px(
                &mut px,
                x,
                y,
                clamp_u8(rail[0] + noise),
                clamp_u8(rail[1] + noise),
                clamp_u8(rail[2] + noise),
                255,
            );
        }
    }
    px
}

/// East–west rail: `gen_track` rotated 90° so the two steel rails run along X.
/// Used for `StraightEW` and the horizontal leg of corner rails.
fn gen_track_ew(seed: u32) -> Vec<u8> {
    rotate90(gen_track(seed))
}

/// Rotate a `SIZE`×`SIZE` RGBA buffer 90° clockwise.
fn rotate90(src: Vec<u8>) -> Vec<u8> {
    let mut dst = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let sx = y;
            let sy = SIZE - 1 - x;
            let d = ((y * SIZE + x) * 4) as usize;
            let s = ((sy * SIZE + sx) * 4) as usize;
            dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
    dst
}

/// Sail block face — full-face dyed canvas with a cross-hatch weave
/// (every 4th row + column darkened by 14) so the surface reads as
/// sailcloth, not flat wallpaper. Fully opaque. Mid-row reinforcement
/// stripe so the texture has a horizontal "boom" line that orients
/// the player when stacking sails.
fn gen_sail(seed: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, seed);
            let noise = (h % 9) as i32 - 4;
            // Cross-hatch weave: darken on every 4th row + column.
            let weave = if x % 4 == 0 || y % 4 == 0 { -14 } else { 0 };
            // Mid-row darker reinforcement band (a single pixel wide
            // at y=8) — a visible "seam" along the sail's middle.
            let seam = if y == 8 { -22 } else { 0 };
            set_px(
                &mut px,
                x,
                y,
                clamp_u8(rgb[0] as i32 + noise + weave + seam),
                clamp_u8(rgb[1] as i32 + noise + weave + seam),
                clamp_u8(rgb[2] as i32 + noise + weave + seam),
                255,
            );
        }
    }
    px
}

/// Banner block face — dyed flag in the top ~⅔ + wooden pole shaft
/// below + a thin dark trim where the flag meets the pole. Fully
/// opaque (solid block). Stands as a flag-on-pole motif from
/// distance + reads with patterning up close via per-pixel noise.
fn gen_banner(seed: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let flag_bottom = 11u32; // top 11 rows = flag, bottom 5 = pole
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, seed);
            let noise = (h % 9) as i32 - 4;
            if y < flag_bottom {
                // Dye-coloured flag.
                set_px(
                    &mut px,
                    x,
                    y,
                    clamp_u8(rgb[0] as i32 + noise),
                    clamp_u8(rgb[1] as i32 + noise),
                    clamp_u8(rgb[2] as i32 + noise),
                    255,
                );
            } else if y == flag_bottom {
                // Dark trim — the rope cinching flag to pole.
                set_px(&mut px, x, y, 38, 26, 18, 255);
            } else {
                // Wooden pole — narrower in the centre of the face,
                // surrounded by neutral grey ground so the pole reads.
                let centre_dist = ((x as i32) - (SIZE as i32 / 2)).unsigned_abs();
                if centre_dist <= 2 {
                    set_px(
                        &mut px,
                        x,
                        y,
                        clamp_u8(140 + noise),
                        clamp_u8(96 + noise),
                        clamp_u8(54 + noise),
                        255,
                    );
                } else {
                    // Grass / ground neutral — softer green-brown so
                    // the pole reads as planted in earth.
                    set_px(
                        &mut px,
                        x,
                        y,
                        clamp_u8(120 + noise),
                        clamp_u8(140 + noise),
                        clamp_u8(90 + noise),
                        255,
                    );
                }
            }
        }
    }
    px
}

/// Spec 35 dyed-décor — Kite. Diamond-shape body with a cross-spar
/// in dark wood + a thin string tail trailing down from the bottom
/// corner. Transparent background so the X-mesh render shows empty
/// cells. The diamond is centred on the 16×16 face; the cross-spar
/// reads as wooden sticks holding the cloth taut.
fn gen_kite(seed: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Default: fully transparent.
    for i in 0..PIXELS / 4 {
        px[i * 4 + 3] = 0;
    }
    // Diamond body: centred at (8, 7), half-width 6, half-height 5.
    // The body is widest at row 7 and tapers to points top + bottom.
    let cx = 8i32;
    let cy = 7i32;
    let hw = 6i32;
    let hh = 5i32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = (x as i32 - cx).unsigned_abs() as i32;
            let dy = (y as i32 - cy).unsigned_abs() as i32;
            // Diamond inequality |dx|/hw + |dy|/hh <= 1.
            // Cross-multiply to integer: dx*hh + dy*hw <= hw*hh.
            if dx * hh + dy * hw <= hw * hh {
                let h = px_hash(x, y, seed);
                let noise = (h % 9) as i32 - 4;
                set_px(
                    &mut px,
                    x,
                    y,
                    clamp_u8(rgb[0] as i32 + noise),
                    clamp_u8(rgb[1] as i32 + noise),
                    clamp_u8(rgb[2] as i32 + noise),
                    255,
                );
            }
        }
    }
    // Cross-spar — dark wood. Horizontal beam at y=7 across full
    // diamond width, vertical beam at x=8 from top to bottom of diamond.
    let spar_r = 70u8;
    let spar_g = 48u8;
    let spar_b = 26u8;
    for x in (cx - hw)..=(cx + hw) {
        if x >= 0 && (x as u32) < SIZE {
            set_px(&mut px, x as u32, cy as u32, spar_r, spar_g, spar_b, 255);
        }
    }
    for y in (cy - hh)..=(cy + hh) {
        if y >= 0 && (y as u32) < SIZE {
            set_px(&mut px, cx as u32, y as u32, spar_r, spar_g, spar_b, 255);
        }
    }
    // String tail trailing from the bottom corner (cx, cy+hh) downward.
    let tail_y_max = SIZE.min((cy + hh + 4) as u32);
    for y in (cy + hh + 1) as u32..tail_y_max {
        // Slight zig-zag so it reads as a string, not a straight line.
        let offset = ((y as i32 % 2) - 1).max(-1);
        let x = (cx + offset).max(0) as u32;
        if x < SIZE {
            set_px(&mut px, x, y, 220, 218, 210, 255);
        }
    }
    px
}

/// Spec 35 dyed-décor — Paper Lantern. Dye-tinted paper face with a
/// soft glow lift toward the centre + faint vertical seams (the
/// folded-paper construction). Fully opaque (covers the full 16×16
/// face).
fn gen_paper_lantern(seed: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let mid = (SIZE / 2) as f32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            // Soft glow gradient — brighter at the centre, dimmer at
            // the edges. Distance from centre normalised against
            // half the face.
            let dx = x as f32 - mid;
            let dy = y as f32 - mid;
            let dist = (dx * dx + dy * dy).sqrt() / mid;
            let glow = ((1.0 - dist) * 26.0).max(-18.0) as i32;
            // Vertical paper-fold seam every 4 px — slight darken.
            let seam = if x % 4 == 0 { -10 } else { 0 };
            let h = px_hash(x, y, seed);
            let noise = (h % 7) as i32 - 3;
            set_px(
                &mut px,
                x,
                y,
                clamp_u8(rgb[0] as i32 + glow + seam + noise),
                clamp_u8(rgb[1] as i32 + glow + seam + noise),
                clamp_u8(rgb[2] as i32 + glow + seam + noise),
                255,
            );
        }
    }
    px
}

/// Spec 35 dyed-décor — Bunting. Row of triangular flags strung on a
/// dark cord. The face is transparent except where the flags sit;
/// rendered via the existing X-mesh small-cube path (non-solid +
/// transparent). `rgb` colours the flag triangles. v1 = 4 flags
/// per face, alternating up-down so the row reads as bunting at a
/// glance.
fn gen_bunting(seed: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Default: fully transparent.
    for i in 0..PIXELS / 4 {
        px[i * 4 + 3] = 0;
    }
    // Cord across the top (rows 1-2, dark brown).
    for y in 1..3 {
        for x in 0..SIZE {
            set_px(&mut px, x, y, 60, 40, 28, 255);
        }
    }
    // 4 triangular flags below the cord, each 4 px wide × 6 px tall.
    // Apex at (cx, 3), base at y=8, half-width 2.
    for flag in 0..4 {
        let cx = flag * 4 + 2;
        for ly in 0..6 {
            let half = ((ly + 1) * 2) / 6; // grows 0..2 from apex to base
            for dx in -half..=half {
                let x = (cx + dx) as u32;
                let y = (3 + ly) as u32;
                if x >= SIZE || y >= SIZE {
                    continue;
                }
                let h = px_hash(x, y, seed);
                let noise = (h % 9) as i32 - 4;
                set_px(
                    &mut px,
                    x,
                    y,
                    clamp_u8(rgb[0] as i32 + noise),
                    clamp_u8(rgb[1] as i32 + noise),
                    clamp_u8(rgb[2] as i32 + noise),
                    255,
                );
            }
        }
    }
    px
}

/// Spec 38 cyanotype-art — wall-mounted framed print. Blueprint-blue
/// interior (matches the `Item::Plan` Developed colour) wrapped in a
/// 2-pixel ivory border on a 16×16 face. Simple + readable v1; the
/// proper "captured wall slice rendered on the face" v2 needs a
/// per-instance block-entity carrying the captured 2D grid.
fn gen_cyanotype_print() -> Vec<u8> {
    let mut out = vec![0u8; 16 * 16 * 4];
    // Blueprint blue [40, 82, 158] (~Item::Plan Developed colour 0x294EA0).
    // Frame ivory [232, 226, 200] — a parchment-warm white-ish.
    for y in 0..16 {
        for x in 0..16 {
            let i = (y * 16 + x) * 4;
            let in_frame = !(2..14).contains(&x) || !(2..14).contains(&y);
            if in_frame {
                out[i] = 232; out[i + 1] = 226; out[i + 2] = 200;
            } else {
                out[i] = 40; out[i + 1] = 82; out[i + 2] = 158;
            }
            out[i + 3] = 255;
        }
    }
    out
}

// Number of texture layers generated.
//
// 16 block (0-15) + 7 cow (16-22) + 7 retired-mob filler (23-29) + 4 chicken (30-33)
// + 7 pig (34-40) + 7 sheep (41-47) + 7 retired-mob filler (48-54) + 6 retired-mob filler (55-60)
// + 6 retired-mob filler (61-66) + 13 misc item-drops (67-79) + 4 per-tool (80-83)
// + 2 bed (84-85) + 3 ore (86-88) + 3 retired-mob filler (89-91) + 1 retired-drop filler (92)
// + 1 glass (93) + 3 storage blocks (94-96) + 1 torch (97)
// + 5 smelting outputs (98-102) + 1 tall grass (103) + 1 placeholder (104)
// + 1 arrow (105) + 4 deepslate (106-109) + 1 Satori block (110)
// + 1 Satori item (111) + 1 tilled soil (112)
// + 5 farming items (113-117: seeds/wheat/bread/carrot/potato)
// + 12 crop-stage textures (118-129: 4 each for wheat/carrot/potato)
// + 4 campfire textures (130-133: lit/unlit × top/side)
// + 2 flint textures (134: flint item, 135: flint-and-steel tool)
// + 1 campfire smoke (136) + 4 corn-stage (137-140) + 3 corn items (141-143)
// + 2 baked-veg items (144: baked_potato, 145: baked_carrot)
// + 12 villager-family textures (146-157: 4× villager + 4× retired-mob filler
//   + 4× wandering villager — Spec 19)
// + 2 drying-rack textures (158: top, 159: side — Wave 29 log seasoning,
//   2026-05-19)
// + 4 papyrus stage textures (160-163) + 2 papyrus item drops (164: reed,
//   165: sheet — Spec 23 Foundation A of Build Schematics, 2026-05-19)
// + 2 plan-tile textures (166: top, 167: side) + 1 construction-anchor
//   texture (168) + 2 architect-plaque textures (169: top, 170: side —
//   Spec 24 Foundation B, 2026-05-19)
// + 3 furnace textures (171: top, 172: side-unlit, 173: side-lit —
//   Spec 20 Foundation, 2026-05-20)
// + 3 deepslate variant textures (174: thin, 175: healthy, 176: fat
//   — Spec 16 Phase 3b, 2026-05-20)
// + 1 vendor block texture (177 — Spec 21, 2026-05-20)
// + 1 drafting table texture (178 — Spec 26, 2026-05-20)
// + 1 campfire smoke warning variant (179 — Spec 22 Phase 7, 2026-05-22)
// + 2 chest textures (180: top, 181: side — HP-2, 2026-05-22)
// + 1 brigand hideout banner texture (182 — HP-3, 2026-05-23)
// + 1 trophy wall texture (183 — HP-3 v2, 2026-05-23)
// + 6 salt-feature textures (184-189 — Salt, 2026-05-23)
// + 4 rubber-feature textures (190-193 — Rubber, 2026-05-23)
// + 1 mob bounty board texture (194 — Spec 33, 2026-05-23)
// + 1 tip jar texture (195 — Spec 34, 2026-05-23)
// + 1 repair bench texture (196 — Spec 35, 2026-05-23)
// + 1 plot marker texture (197 — Spec 36, 2026-05-23)
// + 1 market bell texture (198 — Spec 37, 2026-05-23)
// + 1 auction block texture (199 — Spec 38, 2026-05-23)
// + 1 bazaar block texture (200 — Spec 39, 2026-05-23)
// + 6 retired-player-skin fillers (201-206 — the player moved to the 64x64
//   skin path; layers kept as grey fillers to hold later indices, 2026-06-02)
// + 10 block-break crack-overlay stages (207-216 — Spec 05 §2.2,
//   2026-05-24)
// + 3 bee/squid coats (217: bee body, 218: bee head, 219: squid body —
//   2026-05-27, replacing the chicken/sheep stand-ins)
// + 11 dye/fibre textures (220-224: cornflower/field-poppy/buttercup/
//   cotton-plant/hemp-plant blocks; 225-230: blue/red/yellow dye +
//   cotton/hemp-fibre/rope items — Spec 35 + 36, 2026-05-27)
// + 6 magnesium textures (231: ore block; 232-236: magnesium/fertiliser/
//   sparkler/flare/firestarter items — Spec 37, 2026-05-27)
// + 10 dye-Phase-2 item icons (237-246: black/white/orange/green/purple/
//   pink/lime/light-blue/grey/light-grey — Spec 35, 2026-05-27)
// + 10 fibre-Phase-2 textures (247-254: cotton/hemp crop stages; 255-256:
//   cotton/hemp seed icons — Spec 36, 2026-05-27)
// + 13 coloured-wallpaper textures (257-269 — dyed-paper décor, 2026-05-27)
// + 6 dye-Phase-2-completion textures (270-272: brown/cyan/magenta dye
//   items; 273-275: matching wallpapers — Spec 35 Phase 2 completion,
//   2026-05-28)
// + 3 fibre-Phase-2 consumer textures (276-278: lead / cloth / canvas
//   — Spec 36 Phase 2, 2026-05-28)
// + 12 farmable-flower textures (279-287: 3×3 crop-stages
//   cornflower/field-poppy/buttercup; 288-290: matching seeds
//   — Spec 35 farmable-flower follow-on, 2026-05-28)
// + 1 cyanotype-print texture (291 — Spec 38 cyanotype-art v1,
//   2026-05-28)
// + 16 bunting textures (292-307 — Spec 35 dyed-décor bunting,
//   2026-05-28)
// + 16 paper-lantern textures (308-323 — Spec 35 dyed-décor paper
//   lantern, 2026-05-28)
// + 16 kite textures (324-339 — Spec 35 dyed-décor kite,
//   2026-05-28)
// + 16 banner textures (340-355 — Banner block / first Cloth
//   consumer, 2026-05-28)
// + 16 sail textures (356-371 — Sail block / first Canvas
//   consumer, 2026-05-28)
// + 1 tent texture (372 — Tent block / second Canvas consumer,
//   2026-05-28)
// + 1 track texture (373 — Track block / Rail freight Phase 1)
// + 1 ladder texture (374 — #30 building-detail blocks)
// + 1 carpet texture (375 — #30 building-detail blocks)
// + 1 grave texture (376 — #47 graves)
// + 8 tiered-chest textures (377-384 — #15 tiered storage)
// = 385.
// ─────────────────────────────────────────────────────────────────────────
// Spec 48 (Electricity) — procedural textures for the power blocks (indices
// 385..=396). Each block id gets one identifiable texture (single texture for
// all faces; per-face directional orientation is the deferred #5B mesh pass).
// Aesthetic fine-tuning is a playtest-boundary call; these give each block a
// distinct, readable look in place of the earlier iron/stone placeholders.
// ─────────────────────────────────────────────────────────────────────────

/// Cable (385) — a dark rubber sheath with a copper core stripe down the middle.
fn gen_cable() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4801) % 9) as i32 - 4;
            // Rubber sheath (near-black charcoal).
            let (mut r, mut g, mut b) = (clamp_u8(38 + n), clamp_u8(38 + n), clamp_u8(42 + n));
            // Copper core band across the centre two rows.
            if y == SIZE / 2 - 1 || y == SIZE / 2 {
                r = clamp_u8(176 + n);
                g = clamp_u8(108 + n);
                b = clamp_u8(58 + n);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Cable lit (386) — the energised cable: copper core glows, faint orange bloom.
fn gen_cable_lit() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4802) % 9) as i32 - 4;
            let dist = ((y as i32) - (SIZE as i32 / 2)).abs();
            let (mut r, mut g, mut b) = (clamp_u8(46 + n), clamp_u8(42 + n), clamp_u8(40 + n));
            if dist <= 1 {
                // Bright energised core.
                r = clamp_u8(245 + n);
                g = clamp_u8(170 + n);
                b = clamp_u8(70 + n);
            } else if dist <= 3 {
                // Orange bloom either side of the core.
                r = clamp_u8(150 + n);
                g = clamp_u8(80 + n);
                b = clamp_u8(40 + n);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Electric Lamp unlit (387) — grey frame around a dull domed bulb.
fn gen_lamp() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let c = SIZE as i32 / 2;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4803) % 9) as i32 - 4;
            let edge = x == 0 || y == 0 || x == SIZE - 1 || y == SIZE - 1;
            let d = ((x as i32) - c).pow(2) + ((y as i32) - c).pow(2);
            let (r, g, b) = if edge {
                (clamp_u8(72 + n), clamp_u8(74 + n), clamp_u8(80 + n)) // dark frame
            } else if d <= 30 {
                (clamp_u8(150 + n), clamp_u8(146 + n), clamp_u8(110 + n)) // dull bulb
            } else {
                (clamp_u8(108 + n), clamp_u8(110 + n), clamp_u8(116 + n)) // grey body
            };
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Electric Lamp lit (388) — a hot glowing bulb, bright at the centre.
fn gen_lamp_lit() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let c = SIZE as i32 / 2;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4804) % 7) as i32 - 3;
            let edge = x == 0 || y == 0 || x == SIZE - 1 || y == SIZE - 1;
            let d = ((x as i32) - c).pow(2) + ((y as i32) - c).pow(2);
            let (r, g, b) = if edge {
                (clamp_u8(120 + n), clamp_u8(108 + n), clamp_u8(70 + n))
            } else if d <= 12 {
                (clamp_u8(255), clamp_u8(252 + n), clamp_u8(214 + n)) // white-hot core
            } else if d <= 36 {
                (clamp_u8(255), clamp_u8(224 + n), clamp_u8(130 + n)) // warm glow
            } else {
                (clamp_u8(235 + n), clamp_u8(196 + n), clamp_u8(110 + n))
            };
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Lever (389) — wooden base with a diagonal handle.
fn gen_lever() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4805) % 9) as i32 - 4;
            // Cobble-grey base plate.
            let (mut r, mut g, mut b) = (clamp_u8(120 + n), clamp_u8(120 + n), clamp_u8(124 + n));
            // Diagonal handle (dark wood) from bottom-left to top-right.
            if ((x as i32) - (SIZE as i32 - 1 - y as i32)).abs() <= 1 {
                r = clamp_u8(96 + n);
                g = clamp_u8(64 + n);
                b = clamp_u8(34 + n);
            }
            // Knob at the top end.
            if x >= SIZE - 4 && y <= 3 {
                r = clamp_u8(150 + n);
                g = clamp_u8(40 + n);
                b = clamp_u8(36 + n);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Button (390) — stone plate with a small raised centre square.
fn gen_button() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4806) % 9) as i32 - 4;
            let inner = (5..=10).contains(&x) && (5..=10).contains(&y);
            let (r, g, b) = if inner {
                (clamp_u8(150 + n), clamp_u8(150 + n), clamp_u8(156 + n)) // raised pad
            } else {
                (clamp_u8(112 + n), clamp_u8(112 + n), clamp_u8(116 + n)) // stone
            };
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Pressure Plate (391) — flat stone tile with a bevelled lip.
fn gen_pressure_plate() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4807) % 9) as i32 - 4;
            let lip = x <= 1 || y <= 1 || x >= SIZE - 2 || y >= SIZE - 2;
            let (r, g, b) = if lip {
                (clamp_u8(140 + n), clamp_u8(140 + n), clamp_u8(146 + n)) // bright bevel
            } else {
                (clamp_u8(104 + n), clamp_u8(104 + n), clamp_u8(110 + n)) // sunken centre
            };
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Logic Gate (392) — iron relay plate, copper contacts, a centre indicator.
fn gen_logic_gate() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4808) % 9) as i32 - 4;
            // Iron plate.
            let (mut r, mut g, mut b) = (clamp_u8(150 + n), clamp_u8(152 + n), clamp_u8(158 + n));
            // Two copper contact rails.
            if x == 3 || x == SIZE - 4 {
                r = clamp_u8(180 + n);
                g = clamp_u8(110 + n);
                b = clamp_u8(58 + n);
            }
            // Centre indicator lamp (amber).
            if (6..=9).contains(&x) && (6..=9).contains(&y) {
                r = clamp_u8(230 + n);
                g = clamp_u8(170 + n);
                b = clamp_u8(60 + n);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Hand Crank (393) — wooden body with a copper crank handle.
fn gen_hand_crank() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let c = SIZE as i32 / 2;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4809) % 9) as i32 - 4;
            // Plank body with horizontal grain.
            let mut r = clamp_u8(150 + n);
            let mut g = clamp_u8(110 + n);
            let mut b = clamp_u8(66 + n);
            if y % 4 == 0 {
                r = clamp_u8(r as i32 - 26);
                g = clamp_u8(g as i32 - 20);
                b = clamp_u8(b as i32 - 12);
            }
            // Copper crank ring + handle.
            let d = ((x as i32) - c).pow(2) + ((y as i32) - c).pow(2);
            if (20..=40).contains(&d) || (x >= SIZE - 4 && y >= c as u32 && y <= c as u32 + 2) {
                r = clamp_u8(186 + n);
                g = clamp_u8(116 + n);
                b = clamp_u8(62 + n);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Steam Generator unlit (394) — a riveted iron boiler with a dark hatch.
fn gen_steam_generator() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4810) % 9) as i32 - 4;
            // Dark iron shell.
            let (mut r, mut g, mut b) = (clamp_u8(96 + n), clamp_u8(98 + n), clamp_u8(104 + n));
            let band = x == 0 || x == SIZE - 1 || y == 0 || y == SIZE - 1;
            if band {
                r = clamp_u8(62 + n / 2);
                g = clamp_u8(64 + n / 2);
                b = clamp_u8(70 + n / 2);
            }
            // Rivets in the corners.
            let rivet = (x == 2 || x == SIZE - 3) && (y == 2 || y == SIZE - 3);
            if rivet {
                r = clamp_u8(168 + n);
                g = clamp_u8(170 + n);
                b = clamp_u8(176 + n);
            }
            // Dark combustion hatch (centre).
            if (5..=10).contains(&x) && (6..=11).contains(&y) {
                r = clamp_u8(40 + n / 2);
                g = clamp_u8(38 + n / 2);
                b = clamp_u8(40 + n / 2);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Steam Generator lit (395) — the boiler running: the hatch glows orange.
fn gen_steam_generator_lit() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4811) % 9) as i32 - 4;
            let (mut r, mut g, mut b) = (clamp_u8(100 + n), clamp_u8(100 + n), clamp_u8(106 + n));
            let band = x == 0 || x == SIZE - 1 || y == 0 || y == SIZE - 1;
            if band {
                r = clamp_u8(64 + n / 2);
                g = clamp_u8(66 + n / 2);
                b = clamp_u8(72 + n / 2);
            }
            let rivet = (x == 2 || x == SIZE - 3) && (y == 2 || y == SIZE - 3);
            if rivet {
                r = clamp_u8(170 + n);
                g = clamp_u8(172 + n);
                b = clamp_u8(178 + n);
            }
            // Glowing chamber — brighter toward its centre.
            if (5..=10).contains(&x) && (6..=11).contains(&y) {
                let glow = 9 - (((x as i32) - 7).abs() + ((y as i32) - 8).abs()).min(9);
                r = clamp_u8(180 + glow * 8 + n);
                g = clamp_u8(90 + glow * 5 + n);
                b = clamp_u8(30 + n);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Battery (396) — a voltaic pile: copper body with stacked cell bands + a
/// "+" terminal up top.
fn gen_battery() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4812) % 9) as i32 - 4;
            // Copper body.
            let mut r = clamp_u8(178 + n);
            let mut g = clamp_u8(112 + n);
            let mut b = clamp_u8(60 + n);
            // Darker bands every 3 rows = stacked cells.
            if y % 3 == 0 {
                r = clamp_u8(120 + n);
                g = clamp_u8(74 + n);
                b = clamp_u8(40 + n);
            }
            // "+" terminal near the top.
            if (y == 2 && (5..=10).contains(&x)) || (x == 7 && (1..=4).contains(&y)) {
                r = clamp_u8(220 + n);
                g = clamp_u8(220 + n);
                b = clamp_u8(228 + n);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Beam Sensor (397) — a dark housing with a glowing red emitter eye.
fn gen_beam_sensor() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let c = SIZE as i32 / 2;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4901) % 9) as i32 - 4;
            // Dark metal housing.
            let (mut r, mut g, mut b) = (clamp_u8(70 + n), clamp_u8(66 + n), clamp_u8(72 + n));
            let d = ((x as i32) - c).pow(2) + ((y as i32) - c).pow(2);
            if d <= 6 {
                // Glowing red emitter eye.
                r = clamp_u8(235 + n);
                g = clamp_u8(40 + n);
                b = clamp_u8(34 + n);
            } else if d <= 16 {
                // Lens ring.
                r = clamp_u8(120 + n);
                g = clamp_u8(50 + n);
                b = clamp_u8(48 + n);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Mirror (398) — a bright reflective face with a bevelled frame + a diagonal
/// glint so its angle reads at a glance.
fn gen_mirror() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4902) % 7) as i32 - 3;
            let frame = x <= 1 || y <= 1 || x >= SIZE - 2 || y >= SIZE - 2;
            let (mut r, mut g, mut b) = if frame {
                (clamp_u8(96 + n), clamp_u8(100 + n), clamp_u8(108 + n)) // dark frame
            } else {
                (clamp_u8(196 + n), clamp_u8(208 + n), clamp_u8(220 + n)) // silvered glass
            };
            // Diagonal glint highlight.
            if !frame && ((x as i32) - (y as i32)).abs() <= 1 {
                r = clamp_u8(245);
                g = clamp_u8(250);
                b = clamp_u8(255);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Motion Sensor (399) — a teal housing with a faceted PIR dome.
fn gen_motion_sensor() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let c = SIZE as i32 / 2;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4903) % 9) as i32 - 4;
            let (mut r, mut g, mut b) = (clamp_u8(64 + n), clamp_u8(72 + n), clamp_u8(82 + n));
            let d = ((x as i32) - c).pow(2) + ((y as i32) - c).pow(2);
            if d <= 24 {
                // Pale faceted dome; grid lines for the "fresnel" look.
                let facet = (x % 4 == 0 || y % 4 == 0) as i32 * 20;
                r = clamp_u8(150 + n - facet);
                g = clamp_u8(180 + n - facet);
                b = clamp_u8(185 + n - facet);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

// ── Spec 49 (Explosives) textures (400..=404) ──

/// Brimstone (400) — sulphur ore: grey rock studded with sulphur-yellow crystals.
fn gen_brimstone() -> Vec<u8> {
    gen_ore((170, 160, 30), (220, 210, 70), 4900)
}

/// Nitre ore (401) — pale white-grey niter crust speckling grey stone.
fn gen_nitre_ore() -> Vec<u8> {
    gen_ore((208, 208, 202), (240, 240, 236), 4901)
}

/// Composter (402) — a slatted wooden bin: vertical staves with dark gaps and a
/// dark composting interior showing over the rim.
fn gen_composter() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4902) % 9) as i32 - 4;
            // Vertical slats: light stave / dark gap every 4px.
            let gap = x % 4 == 3;
            let (mut r, mut g, mut b) = if gap {
                (clamp_u8(58 + n), clamp_u8(42 + n), clamp_u8(24 + n))
            } else {
                (clamp_u8(120 + n), clamp_u8(86 + n), clamp_u8(46 + n))
            };
            // Dark composting fill visible over the top rim.
            if (2..=13).contains(&x) && (2..=5).contains(&y) {
                r = clamp_u8(48 + n);
                g = clamp_u8(40 + n);
                b = clamp_u8(28 + n);
            }
            // Top + bottom rim band.
            if y == 0 || y == SIZE - 1 {
                r = clamp_u8(92 + n);
                g = clamp_u8(64 + n);
                b = clamp_u8(34 + n);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Blasting Keg (403) — a banded wooden barrel (NEVER the red TNT cube, no
/// lettering) with dark iron hoops and a short fuse poking out the top.
fn gen_blasting_keg() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4904) % 9) as i32 - 4;
            // Wooden staves — warm barrel brown, faintly vertical-grained.
            let stave = (x % 4 == 0) as i32 * -8;
            let (mut r, mut g, mut b) = (
                clamp_u8(124 + n + stave),
                clamp_u8(84 + n + stave),
                clamp_u8(44 + n + stave),
            );
            // Iron hoops — three dark horizontal bands.
            if y == 2 || y == 7 || y == 12 {
                r = clamp_u8(58 + n);
                g = clamp_u8(58 + n);
                b = clamp_u8(64 + n);
            }
            // Fuse — a short pale-grey wick rising from the centre top.
            if (x == 7 || x == 8) && y <= 2 {
                r = clamp_u8(186 + n);
                g = clamp_u8(180 + n);
                b = clamp_u8(150 + n);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Plunger Detonator (404) — a boxed T-handle blasting machine: a dark casing
/// with a metal plunger shaft and a cross-bar handle on top.
fn gen_plunger_detonator() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let c = SIZE / 2;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let n = (px_hash(x, y, 4905) % 9) as i32 - 4;
            // Dark casing box with a border.
            let border = x == 0 || x == SIZE - 1 || y == SIZE - 1 || y == 0;
            let (mut r, mut g, mut b) = if border {
                (clamp_u8(42 + n), clamp_u8(34 + n), clamp_u8(30 + n))
            } else {
                (clamp_u8(86 + n), clamp_u8(62 + n), clamp_u8(48 + n))
            };
            // The box body sits in the lower two-thirds.
            if y >= 6 {
                r = clamp_u8(74 + n);
                g = clamp_u8(52 + n);
                b = clamp_u8(40 + n);
            }
            // Vertical plunger shaft up the centre.
            if (x == c || x == c - 1) && y >= 2 {
                r = clamp_u8(150 + n);
                g = clamp_u8(150 + n);
                b = clamp_u8(158 + n);
            }
            // T-handle cross-bar near the top (the classic red detonator handle).
            if y <= 2 && (3..=12).contains(&x) {
                r = clamp_u8(168 + n);
                g = clamp_u8(60 + n);
                b = clamp_u8(40 + n);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Layer count BEFORE the species-tint run (mob-species-tint foundation,
/// 2026-07-11). The tinted layers append after every other layer; this is
/// their base index.
pub const PRE_TINT_LAYER_COUNT: u32 = 459;

pub fn texture_count() -> u32 {
    // 405 block/item layers + 16 flat solid paint-colour layers (Workshop paint)
    // + 5 per-species leaf layers (#130, 421..=425)
    // + 15 per-species wood layers (#132, 426..=440: log_side/top + planks × 5).
    // + 3 Satoshi founder-sage layers (441..=443: hood front/side + glow trim).
    // + 1 rail auto-connect east-west track layer (444: TEX_TRACK_EW, gen_track rotated 90°).
    // + 3 rail/cable v2 solid metal layers (445 steel, 446 copper, 447 copper-lit).
    // + 1 rail sleeper/ballast bed layer (448 TEX_RAIL_BASE).
    // + 1 fire layer (449 TEX_FIRE, 2026-07-04 gap-fill wave).
    // + 2 dispenser/dropper muzzle faces (450..=451, 2026-07-04).
    // + 1 shared sapling sprout (452, 2026-07-04).
    // + 4 particle billboards (453..=456, 2026-07-05).
    // + 2 Pet Bed top/side layers (457..=458, 2026-07-06 pets wave).
    //   (= PRE_TINT_LAYER_COUNT, 459)
    // + the species-tint run (459.., 2026-07-11): per-species tinted copies of
    //   donor coat layers, driven by SPECIES_TINTS below. MUST stay under the
    //   512 JS pre-flight floor (`texture_count_stays_under_webgpu_check_floor`).
    //   (= POST_TINT_LAYER_START, 506)
    // + 2 Water Wheel layers (506..=507, Spec 48 Phase 4, 2026-09-06) — block
    //   layers appended AFTER the tint run so the tint indices stay put.
    // + 1 Copper Ore layer (508, Wind, Copper & Electricity wave, 2026-09-07,
    //   Spec 02 §1) — same post-tint reasoning as the Water Wheel.
    // + 2 Windmill layers (509..=510, Wind, Copper & Electricity wave §2.2,
    //   2026-09-07) — 511 total, one layer under the 512 floor.
    POST_TINT_LAYER_START + POST_TINT_LAYER_COUNT
}

/// First layer index AFTER the species-tint run — where block layers minted
/// after the 2026-07-11 tint bake append. The tint run is index-derived from
/// [`PRE_TINT_LAYER_COUNT`], so it must stay contiguous; anything newer goes
/// here. Asserted against the real push order in `generate_textures`.
pub const POST_TINT_LAYER_START: u32 = PRE_TINT_LAYER_COUNT + tint_run_len();

/// How many layers live at/after [`POST_TINT_LAYER_START`]: the Water Wheel
/// idle + turning faces (Spec 48 Phase 4), the Copper Ore layer, and the
/// Windmill idle + turning faces (both Wind, Copper & Electricity wave,
/// 2026-09-07). Five layers → `texture_count()` = 511, ONE under the 512
/// device floor — the next block layer has to buy its room somewhere.
pub const POST_TINT_LAYER_COUNT: u32 = 5;

/// Total layers the species-tint bake produces (47 today). `const fn` so
/// [`POST_TINT_LAYER_START`] is a compile-time constant — iterators aren't
/// const, hence the index loop.
pub const fn tint_run_len() -> u32 {
    let mut n = 0;
    let mut i = 0;
    while i < SPECIES_TINTS.len() {
        n += SPECIES_TINTS[i].donors.len() as u32;
        i += 1;
    }
    n
}

/// One mesh-reuse species' tint entry (mob-species-tint foundation,
/// 2026-07-11). `MobDef.color` (the per-mob TOML) is the tint source at
/// generation time — this table only says WHICH donor layers to clone.
pub struct SpeciesTint {
    pub kind: crate::mob::MobType,
    /// Donor layers to clone + tint, in append order.
    pub donors: &'static [u32],
    /// `tex_faces` remap: (donor_layer → offset into this species' generated
    /// run). Includes the leg alias mapping onto the tinted body_side — legs
    /// are 0.15-block posts, and folding them onto body_side keeps the whole
    /// run at 47 layers (506 total), under the 512 device floor with headroom.
    pub map: &'static [(u32, u32)],
}

// Shared donor coat sets. Order within each set defines the generated run
// (body_end, body_side, body_top, head_front, head_side, head_top) and must
// match the registry keys appended in texture_registry.rs.
const SHEEP_COAT: &[u32] = &[
    crate::entity_model::TEX_SHEEP_BODY_END,
    crate::entity_model::TEX_SHEEP_BODY_SIDE,
    crate::entity_model::TEX_SHEEP_BODY_TOP,
    crate::entity_model::TEX_SHEEP_HEAD_FRONT,
    crate::entity_model::TEX_SHEEP_HEAD_SIDE,
    crate::entity_model::TEX_SHEEP_HEAD_TOP,
];
const SHEEP_COAT_MAP: &[(u32, u32)] = &[
    (crate::entity_model::TEX_SHEEP_BODY_END, 0),
    (crate::entity_model::TEX_SHEEP_BODY_SIDE, 1),
    (crate::entity_model::TEX_SHEEP_BODY_TOP, 2),
    (crate::entity_model::TEX_SHEEP_HEAD_FRONT, 3),
    (crate::entity_model::TEX_SHEEP_HEAD_SIDE, 4),
    (crate::entity_model::TEX_SHEEP_HEAD_TOP, 5),
    // Leg alias → tinted body_side (layer-budget, see SpeciesTint.map doc).
    (crate::entity_model::TEX_SHEEP_LEG, 1),
];
const COW_COAT: &[u32] = &[
    crate::entity_model::TEX_COW_BODY_END,
    crate::entity_model::TEX_COW_BODY_SIDE,
    crate::entity_model::TEX_COW_BODY_TOP,
    crate::entity_model::TEX_COW_HEAD_FRONT,
    crate::entity_model::TEX_COW_HEAD_SIDE,
    crate::entity_model::TEX_COW_HEAD_TOP,
];
// bear_model legs wear TEX_PIG_LEG; horse_model legs wear TEX_COW_LEG.
const COW_COAT_MAP_PIG_LEG: &[(u32, u32)] = &[
    (crate::entity_model::TEX_COW_BODY_END, 0),
    (crate::entity_model::TEX_COW_BODY_SIDE, 1),
    (crate::entity_model::TEX_COW_BODY_TOP, 2),
    (crate::entity_model::TEX_COW_HEAD_FRONT, 3),
    (crate::entity_model::TEX_COW_HEAD_SIDE, 4),
    (crate::entity_model::TEX_COW_HEAD_TOP, 5),
    (crate::entity_model::TEX_PIG_LEG, 1),
];
const COW_COAT_MAP_COW_LEG: &[(u32, u32)] = &[
    (crate::entity_model::TEX_COW_BODY_END, 0),
    (crate::entity_model::TEX_COW_BODY_SIDE, 1),
    (crate::entity_model::TEX_COW_BODY_TOP, 2),
    (crate::entity_model::TEX_COW_HEAD_FRONT, 3),
    (crate::entity_model::TEX_COW_HEAD_SIDE, 4),
    (crate::entity_model::TEX_COW_HEAD_TOP, 5),
    (crate::entity_model::TEX_COW_LEG, 1),
];
const CHICKEN_COAT: &[u32] = &[
    crate::entity_model::TEX_CHICKEN_BODY,
    crate::entity_model::TEX_CHICKEN_HEAD,
    crate::entity_model::TEX_CHICKEN_LEG,
    crate::entity_model::TEX_CHICKEN_WING,
];
const CHICKEN_COAT_MAP: &[(u32, u32)] = &[
    (crate::entity_model::TEX_CHICKEN_BODY, 0),
    (crate::entity_model::TEX_CHICKEN_HEAD, 1),
    (crate::entity_model::TEX_CHICKEN_LEG, 2),
    (crate::entity_model::TEX_CHICKEN_WING, 3),
];
const SQUID_COAT: &[u32] = &[crate::entity_model::TEX_SQUID_BODY];
const SQUID_COAT_MAP: &[(u32, u32)] = &[(crate::entity_model::TEX_SQUID_BODY, 0)];

/// The species-tint table, in layer-append order (459..): fox, cat, crab,
/// polar bear, reindeer, donkey, mule, parrot, glow squid — 47 layers,
/// `texture_count()` = 506, six layers of headroom under the 512 floor.
/// The four villager-tier humans (Brigand/Marauder/Berserker/Knight) are
/// deliberately absent: they get a bespoke armoured-human mesh post-playtest,
/// and skipping them keeps the run inside the 512-floor budget.
pub const SPECIES_TINTS: &[SpeciesTint] = &[
    SpeciesTint { kind: crate::mob::MobType::Fox, donors: SHEEP_COAT, map: SHEEP_COAT_MAP },
    SpeciesTint { kind: crate::mob::MobType::Cat, donors: SHEEP_COAT, map: SHEEP_COAT_MAP },
    SpeciesTint { kind: crate::mob::MobType::Crab, donors: SHEEP_COAT, map: SHEEP_COAT_MAP },
    SpeciesTint { kind: crate::mob::MobType::PolarBear, donors: COW_COAT, map: COW_COAT_MAP_PIG_LEG },
    SpeciesTint { kind: crate::mob::MobType::Reindeer, donors: COW_COAT, map: COW_COAT_MAP_COW_LEG },
    SpeciesTint { kind: crate::mob::MobType::Donkey, donors: COW_COAT, map: COW_COAT_MAP_COW_LEG },
    SpeciesTint { kind: crate::mob::MobType::Mule, donors: COW_COAT, map: COW_COAT_MAP_COW_LEG },
    SpeciesTint { kind: crate::mob::MobType::Parrot, donors: CHICKEN_COAT, map: CHICKEN_COAT_MAP },
    SpeciesTint { kind: crate::mob::MobType::GlowSquid, donors: SQUID_COAT, map: SQUID_COAT_MAP },
];

/// Resolve a donor-coat layer to `kind`'s tinted copy, if `kind` is a
/// mesh-reuse species with a tint run. Donor species themselves return None.
pub fn tinted_layer(kind: crate::mob::MobType, donor: u32) -> Option<u32> {
    let mut base = PRE_TINT_LAYER_COUNT;
    for st in SPECIES_TINTS {
        if st.kind == kind {
            return st
                .map
                .iter()
                .find(|(d, _)| *d == donor)
                .map(|(_, off)| base + off);
        }
        base += st.donors.len() as u32;
    }
    None
}

/// Bee body (2026-05-27) — golden-yellow coat banded with near-black
/// stripes (3-px bands), the classic bee read. Faint per-pixel noise so
/// it isn't a flat fill.
fn gen_bee_body() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 4200);
            let noise = (h % 7) as i32 - 3;
            let stripe = (y / 3) % 2 == 1;
            let (r, g, b) = if stripe {
                (clamp_u8(40 + noise), clamp_u8(34 + noise), clamp_u8(22 + noise))
            } else {
                (clamp_u8(243 + noise), clamp_u8(196 + noise), clamp_u8(40 + noise))
            };
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Bee head (2026-05-27) — near-black, slightly warmer than the stripe
/// so the head reads as a distinct segment.
fn gen_bee_head() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 4201);
            let noise = (h % 6) as i32 - 3;
            set_px(&mut px, x, y, clamp_u8(46 + noise), clamp_u8(40 + noise), clamp_u8(30 + noise), 255);
        }
    }
    px
}

/// Squid body (2026-05-27) — deep ocean blue with a gentle vertical
/// gradient (darker mantle top, lighter underside) and soft mottling.
fn gen_squid_body() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 4202);
            let noise = (h % 9) as i32 - 4;
            // Top rows darker, lower rows lighter — a subtle gradient.
            let lift = (y as i32 * 3) / SIZE as i32; // 0..3
            set_px(
                &mut px,
                x,
                y,
                clamp_u8(40 + lift * 6 + noise),
                clamp_u8(78 + lift * 8 + noise),
                clamp_u8(165 + lift * 6 + noise),
                255,
            );
        }
    }
    px
}

/// Shared flower texture (Spec 35) — a green stem + two small leaves and a
/// round petal head with a yellow eye, on a transparent background so it
/// renders as an X-mesh plant. `petal` tints the head.
fn gen_flower(seed: u32, petal: [u8; 3]) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let (sr, sg, sb) = (60u8, 110u8, 48u8); // stem green
    // Stem column (x = 7..9, from the head down to the ground).
    for y in 5..SIZE {
        for x in 7..9 {
            set_px(&mut px, x, y, sr, sg, sb, 255);
        }
    }
    // A couple of leaves off the stem.
    set_px(&mut px, 6, 10, sr, sg, sb, 255);
    set_px(&mut px, 9, 12, sr, sg, sb, 255);
    // Round petal head centred on (7.5, 3), radius ~3.
    for y in 0..7i32 {
        for x in 4..12i32 {
            let dx = x - 7;
            let dy = y - 3;
            if dx * dx + dy * dy <= 9 {
                let h = px_hash(x as u32, y as u32, seed);
                let n = (h % 7) as i32 - 3;
                set_px(
                    &mut px,
                    x as u32,
                    y as u32,
                    clamp_u8(petal[0] as i32 + n),
                    clamp_u8(petal[1] as i32 + n),
                    clamp_u8(petal[2] as i32 + n),
                    255,
                );
            }
        }
    }
    // Yellow eye.
    for x in 7..9 {
        set_px(&mut px, x, 3, 250, 220, 90, 255);
    }
    px
}

fn gen_cornflower() -> Vec<u8> {
    gen_flower(4300, [50, 90, 210])
}
fn gen_field_poppy() -> Vec<u8> {
    gen_flower(4301, [212, 44, 40])
}
fn gen_buttercup() -> Vec<u8> {
    gen_flower(4302, [250, 212, 50])
}

/// Cotton plant (Spec 36) — a short stalk hung with off-white bolls.
fn gen_cotton_plant() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let (sr, sg, sb) = (96u8, 120u8, 72u8);
    for y in 4..SIZE {
        for x in 7..9 {
            set_px(&mut px, x, y, sr, sg, sb, 255);
        }
    }
    // Bolls — small 2×2 white tufts at a few stalk nodes.
    for (cx, cy) in [(6u32, 5u32), (9, 7), (7, 2), (10, 4), (5, 9)] {
        for dy in 0..2 {
            for dx in 0..2 {
                let h = px_hash(cx + dx, cy + dy, 4303);
                let n = (h % 9) as i32 - 4;
                set_px(
                    &mut px,
                    cx + dx,
                    cy + dy,
                    clamp_u8(238 + n),
                    clamp_u8(238 + n),
                    clamp_u8(230 + n),
                    255,
                );
            }
        }
    }
    px
}

/// Hemp plant (Spec 36) — a tall stalk with paired green leaf fronds.
fn gen_hemp_plant() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let (sr, sg, sb) = (72u8, 110u8, 56u8);
    let (lr, lg, lb) = (58u8, 132u8, 50u8);
    for y in 1..SIZE {
        for x in 7..9 {
            set_px(&mut px, x, y, sr, sg, sb, 255);
        }
    }
    // Leaf fronds stepping out left + right at intervals up the stalk.
    for y in (3..15).step_by(3) {
        for k in 1..4u32 {
            let yy = y;
            if 7 >= k {
                set_px(&mut px, 7 - k, yy.min(SIZE - 1), lr, lg, lb, 255);
            }
            if 8 + k < SIZE {
                set_px(&mut px, 8 + k, yy.min(SIZE - 1), lr, lg, lb, 255);
            }
        }
    }
    px
}

/// Magnesium ore (Spec 37) — stone-grey base flecked with bright
/// silver-white speckles (the mineral showing through the rock).
fn gen_magnesium_ore() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 4330);
            // Stone base.
            let base = (h % 11) as i32 - 5;
            let (mut r, mut g, mut b) = (clamp_u8(128 + base), clamp_u8(128 + base), clamp_u8(132 + base));
            // ~14% silver-white mineral flecks.
            if h.is_multiple_of(7) {
                let f = (px_hash(x, y, 4331) % 20) as i32;
                r = clamp_u8(215 + f / 2);
                g = clamp_u8(215 + f / 2);
                b = clamp_u8(225 + f / 2);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// The canonical RGB for each of the 16 `WALLPAPER_*` blocks — the single source
/// of truth for both texture generation and the Workshop pin paint-path
/// ([`wallpaper_rgba`]). Keep in sync with `block::WALLPAPER_*` constants.
///
/// Scope: this is the source of truth for the 16 **wallpaper blocks** only. The
/// bunting / kite / banner / sail décor colour tables below are a SEPARATE
/// (slightly tonally-tuned) 16-colour set and are intentionally not unified with
/// this palette — don't assume their reds/blacks match these.
///
/// Order matches the generation order in `generate_textures()` (layers 257-269,
/// then 273-275); do not reorder without also reordering the generation loop.
pub(crate) const WALLPAPER_PALETTE: &[(crate::block::BlockId, [u8; 3])] = &[
    (crate::block::WALLPAPER_WHITE,      [238, 238, 238]),
    (crate::block::WALLPAPER_BLACK,      [34,  34,  40 ]),
    (crate::block::WALLPAPER_RED,        [199, 46,  41 ]),
    (crate::block::WALLPAPER_BLUE,       [62,  92,  209]),
    (crate::block::WALLPAPER_YELLOW,     [237, 214, 56 ]),
    (crate::block::WALLPAPER_ORANGE,     [224, 128, 41 ]),
    (crate::block::WALLPAPER_GREEN,      [82,  158, 61 ]),
    (crate::block::WALLPAPER_PURPLE,     [133, 66,  173]),
    (crate::block::WALLPAPER_PINK,       [234, 148, 184]),
    (crate::block::WALLPAPER_LIME,       [143, 214, 82 ]),
    (crate::block::WALLPAPER_LIGHT_BLUE, [122, 184, 235]),
    (crate::block::WALLPAPER_GREY,       [115, 115, 122]),
    (crate::block::WALLPAPER_LIGHT_GREY, [184, 184, 190]),
    (crate::block::WALLPAPER_BROWN,      [115, 75,  45 ]),
    (crate::block::WALLPAPER_CYAN,       [56,  184, 196]),
    (crate::block::WALLPAPER_MAGENTA,    [209, 70,  166]),
];

/// Representative RGBA for a wallpaper block — its flat palette colour, opaque.
/// Falls back to `block_average_rgba` for non-wallpaper ids so the pin paint-path
/// always has a colour (an un-recoloured source cell uses its block's average).
pub fn wallpaper_rgba(block_id: crate::block::BlockId) -> [u8; 4] {
    if let Some((_, rgb)) = WALLPAPER_PALETTE.iter().find(|(b, _)| *b == block_id) {
        return [rgb[0], rgb[1], rgb[2], 255];
    }
    block_average_rgba(block_id)
}

/// The base block-texture set, generated once and cached. The set is deterministic
/// + immutable, so the per-cell `block_average_rgba` sampling done while baking a
///   pin (hundreds of calls per pin) must NOT regenerate it each time (that was a
///   multi-second freeze). `rebuild_block_textures` still calls `generate_textures()`
///   directly — it owns the GPU upload and runs rarely.
fn cached_textures() -> &'static [Vec<u8>] {
    static CACHE: std::sync::OnceLock<Vec<Vec<u8>>> = std::sync::OnceLock::new();
    CACHE.get_or_init(generate_textures)
}

/// Representative RGBA for any block: the mean of its stock side texture, opaque.
/// Used by the Workshop pin paint-path for cells the player never recoloured.
/// Cheap to call repeatedly — reads the cached texture set ([`cached_textures`]).
pub fn block_average_rgba(block_id: crate::block::BlockId) -> [u8; 4] {
    let reg = crate::block::BlockRegistry::new();
    let layer = reg.tex_side(block_id) as usize;
    let textures = cached_textures();
    let Some(tex) = textures.get(layer) else {
        return [120, 120, 120, 255];
    };
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for px in tex.chunks_exact(4) {
        if px[3] == 0 {
            continue;
        }
        r += px[0] as u64;
        g += px[1] as u64;
        b += px[2] as u64;
        n += 1;
    }
    if n == 0 {
        return [120, 120, 120, 255];
    }
    [(r / n) as u8, (g / n) as u8, (b / n) as u8, 255]
}

/// Coloured wallpaper (dyed-paper décor) — a flat dyed field with faint
/// vertical stripes every 4 px + light per-pixel noise, so it reads as
/// patterned paper rather than a solid block. `rgb` is the dye colour.
fn gen_wallpaper(seed: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, seed);
            let noise = (h % 9) as i32 - 4;
            let stripe = if x % 4 == 0 { -12 } else { 0 };
            set_px(
                &mut px,
                x,
                y,
                clamp_u8(rgb[0] as i32 + noise + stripe),
                clamp_u8(rgb[1] as i32 + noise + stripe),
                clamp_u8(rgb[2] as i32 + noise + stripe),
                255,
            );
        }
    }
    px
}

// --- Pixel helpers ---

fn px_hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h = x.wrapping_mul(374761393)
        .wrapping_add(y.wrapping_mul(668265263))
        .wrapping_add(seed.wrapping_mul(1274126177));
    h = (h ^ (h >> 13)).wrapping_mul(1103515245);
    h ^ (h >> 16)
}

fn set_px(buf: &mut [u8], x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
    let i = (y * SIZE + x) as usize * 4;
    buf[i] = r;
    buf[i + 1] = g;
    buf[i + 2] = b;
    buf[i + 3] = a;
}

fn clamp_u8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

// --- Texture generators ---

/// Neutral solid-grey 16x16 filler used for retired texture layers.
///
/// Several mid-array layers once held mob/item skin art that has been removed
/// for IP cleanliness. The layers are kept (same count + order) so every later
/// layer index stays stable and the atlas isn't corrupted; their art is now a
/// flat grey placeholder. No model or material references these layers, so the
/// fill colour is never seen in-game.
fn gen_retired_layer() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            set_px(&mut px, x, y, 128, 128, 128, 255);
        }
    }
    px
}

fn gen_stone() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 100);
            let base = 128i32;
            let noise = (h % 25) as i32 - 12;
            let mut v = base + noise;
            if h.is_multiple_of(13) { v -= 25; } // dark speckle
            let v = clamp_u8(v);
            set_px(&mut px, x, y, v, v, v, 255);
        }
    }
    px
}

fn gen_dirt() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 200);
            let noise = (h % 21) as i32 - 10;
            let r = clamp_u8(134 + noise);
            let g = clamp_u8(96 + noise * 7 / 10);
            let b = clamp_u8(67 + noise / 2);
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

fn gen_grass_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 300);
            let noise = (h % 31) as i32 - 15;
            let r = clamp_u8(85 + noise / 3);
            let g = clamp_u8(148 + noise);
            let b = clamp_u8(43 + noise / 4);
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

fn gen_grass_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 400);
            // Top rows: green with jagged edge
            let green_depth = 3 + (h % 3); // jagged between row 3-5
            if y < green_depth {
                let noise = (h % 21) as i32 - 10;
                let r = clamp_u8(85 + noise / 3);
                let g = clamp_u8(140 + noise);
                let b = clamp_u8(43 + noise / 4);
                set_px(&mut px, x, y, r, g, b, 255);
            } else {
                // Dirt part
                let noise = (h % 21) as i32 - 10;
                let r = clamp_u8(134 + noise);
                let g = clamp_u8(96 + noise * 7 / 10);
                let b = clamp_u8(67 + noise / 2);
                set_px(&mut px, x, y, r, g, b, 255);
            }
        }
    }
    px
}

fn gen_bedrock() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 500);
            let base = 50i32;
            let noise = (h % 31) as i32 - 15;
            let v = clamp_u8(base + noise);
            // Lighter patches
            let v = if h.is_multiple_of(7) { clamp_u8(v as i32 + 30) } else { v };
            set_px(&mut px, x, y, v, v, v, 255);
        }
    }
    px
}

fn gen_sand() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 600);
            let noise = (h % 17) as i32 - 8;
            let r = clamp_u8(219 + noise);
            let g = clamp_u8(211 + noise);
            let b = clamp_u8(160 + noise);
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

fn gen_oak_log_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 700);
            let noise = (h % 15) as i32 - 7;
            // Vertical bark lines every 4 pixels
            let bark_line = (x % 4 == 0) || (x % 4 == 1 && h.is_multiple_of(3));
            let base_r = if bark_line { 75 } else { 101 };
            let base_g = if bark_line { 56 } else { 76 };
            let base_b = if bark_line { 36 } else { 48 };
            let r = clamp_u8(base_r + noise);
            let g = clamp_u8(base_g + noise * 7 / 10);
            let b = clamp_u8(base_b + noise / 2);
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

fn gen_oak_log_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let cx = 7.5_f32;
    let cy = 7.5_f32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 800);
            let noise = (h % 11) as i32 - 5;
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            // Concentric rings
            let ring = (dist * 1.5) as u32 % 3;
            let (base_r, base_g, base_b) = if ring == 0 {
                (160, 130, 80)  // light wood
            } else if ring == 1 {
                (130, 100, 60)  // medium wood
            } else {
                (110, 85, 50)   // dark ring
            };
            let r = clamp_u8(base_r + noise);
            let g = clamp_u8(base_g + noise);
            let b = clamp_u8(base_b + noise);
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// #131 fancy-leaf cutout density — percent of leaf texels punched fully
/// transparent (alpha 0) so the canopy reads as see-through. Conservative
/// default (subtle gaps, trees still read full); tune up for an airier look.
/// The `fs_transparent` shader discards `alpha < 0.04`, so these are real holes.
const LEAF_CUTOUT_PCT: u32 = 20;

fn gen_oak_leaves() -> Vec<u8> {
    // Oak = the species generator at oak's seed + hue (byte-identical colour to
    // the old inline version, now with the shared cutout holes — #131).
    gen_species_leaves(900, 48, 110, 25)
}

/// Per-species leaf texture (#130 colour + #131 cutout). The oak-leaf noise
/// pattern retinted to a species' canopy colour (birch / spruce / jungle /
/// acacia / dark-oak read distinct), with ~`LEAF_CUTOUT_PCT`% of texels punched
/// transparent for the see-through "fancy" look. `seed` varies both the colour
/// speckle and the hole layout so species don't share an identical pattern.
fn gen_species_leaves(seed: u32, base_r: i32, base_g: i32, base_b: i32) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, seed);
            let noise = (h % 31) as i32 - 15;
            let r = clamp_u8(base_r + noise / 3);
            let g = clamp_u8(base_g + noise);
            let b = clamp_u8(base_b + noise / 4);
            // Cutout holes — a separate hash stream so they don't track the
            // colour speckle. alpha 0 → discarded by fs_transparent → see-through.
            let hole = (px_hash(x, y, seed.wrapping_add(0x1eaf)) % 100) < LEAF_CUTOUT_PCT;
            let a = if hole { 0 } else { 255 };
            set_px(&mut px, x, y, r, g, b, a);
        }
    }
    px
}

/// Per-species log SIDE (#132) — oak bark-grain noise retinted to a species'
/// wood hue. `base_*` is the main bark shade; the recessed bark lines derive a
/// ~0.74× darker shade (oak's own ratio), so birch's pale base gives white bark
/// with dark flecks, dark-oak a deep brown, etc. Distinct `seed` per species.
fn gen_species_log_side(seed: u32, base_r: i32, base_g: i32, base_b: i32) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, seed);
            let noise = (h % 15) as i32 - 7;
            let bark_line = (x % 4 == 0) || (x % 4 == 1 && h.is_multiple_of(3));
            let (br, bg, bb) = if bark_line {
                (base_r * 74 / 100, base_g * 74 / 100, base_b * 75 / 100)
            } else {
                (base_r, base_g, base_b)
            };
            let r = clamp_u8(br + noise);
            let g = clamp_u8(bg + noise * 7 / 10);
            let b = clamp_u8(bb + noise / 2);
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Per-species log TOP (#132) — oak's concentric growth-rings retinted. `base_*`
/// is the lightest ring; the two darker rings derive from oak's ring ratios.
fn gen_species_log_top(seed: u32, base_r: i32, base_g: i32, base_b: i32) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let cx = 7.5_f32;
    let cy = 7.5_f32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, seed);
            let noise = (h % 11) as i32 - 5;
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let ring = (dist * 1.5) as u32 % 3;
            let (br, bg, bb) = match ring {
                0 => (base_r, base_g, base_b),
                1 => (base_r * 81 / 100, base_g * 77 / 100, base_b * 75 / 100),
                _ => (base_r * 69 / 100, base_g * 65 / 100, base_b * 62 / 100),
            };
            let r = clamp_u8(br + noise);
            let g = clamp_u8(bg + noise);
            let b = clamp_u8(bb + noise);
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Per-species planks (#132) — oak's horizontal plank-line noise retinted.
/// `base_*` is the board face; the seam lines derive a ~0.78× darker shade.
fn gen_species_planks(seed: u32, base_r: i32, base_g: i32, base_b: i32) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, seed);
            let noise = (h % 15) as i32 - 7;
            let plank_line = y % 4 == 0;
            let (pr, pg, pb) = if plank_line {
                (base_r * 78 / 100, base_g * 78 / 100, base_b * 78 / 100)
            } else {
                (base_r, base_g, base_b)
            };
            let r = clamp_u8(pr + noise);
            let g = clamp_u8(pg + noise);
            let b = clamp_u8(pb + noise);
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

fn gen_oak_planks() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 1000);
            let noise = (h % 15) as i32 - 7;
            // Horizontal plank lines every 4 pixels
            let plank_line = y % 4 == 0;
            let base_r = if plank_line { 145 } else { 185 };
            let base_g = if plank_line { 115 } else { 148 };
            let base_b = if plank_line { 75 } else { 96 };
            let r = clamp_u8(base_r + noise);
            let g = clamp_u8(base_g + noise);
            let b = clamp_u8(base_b + noise);
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

fn gen_cobblestone() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 1100);
            let noise = (h % 31) as i32 - 15;
            // Irregular stone pattern using hash-based regions
            let region = px_hash(x / 4, y / 4, 1150);
            let shade = 110 + (region % 40) as i32;
            // Mortar lines at 4-pixel boundaries
            let mortar = (x % 4 == 0) || (y % 4 == 0);
            let base = if mortar { shade - 30 } else { shade };
            let v = clamp_u8(base + noise / 2);
            set_px(&mut px, x, y, v, v, v, 255);
        }
    }
    px
}

fn gen_water() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 1200);
            let noise = (h % 31) as i32 - 15;
            let r = clamp_u8(40 + noise);
            let g = clamp_u8(100 + noise);
            let b = clamp_u8(200 + noise);
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

fn gen_gravel() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 1300);
            let noise = (h % 21) as i32 - 10;
            // 4x4 cell regions with random shade offsets to simulate pebbles
            let cell = px_hash(x / 4, y / 4, 1350);
            let shade_offset = (cell % 41) as i32 - 20;
            let base = 140i32 + shade_offset;
            let r = clamp_u8(base + noise);
            let g = clamp_u8((base - 5) + noise);
            let b = clamp_u8((base - 10) + noise);
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

fn gen_sandstone() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 1400);
            let noise = (h % 15) as i32 - 7;
            // Subtle horizontal band lines every 4 pixels (slightly darker)
            let band = y % 4 == 0;
            let base_r = if band { 200 } else { 210 };
            let base_g = if band { 185 } else { 195 };
            let base_b = if band { 130 } else { 140 };
            let r = clamp_u8(base_r + noise);
            let g = clamp_u8(base_g + noise);
            let b = clamp_u8(base_b + noise);
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

fn gen_snow() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 1500);
            let noise = (h % 11) as i32 - 5;
            let r = clamp_u8(240 + noise);
            let g = clamp_u8(245 + noise);
            let b = clamp_u8(255 + noise);
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

// Magenta/black checkerboard — the conventional "missing texture" indicator.
// Never wired into `generate_textures()`'s push-chain or any missing-texture
// fallback path anywhere in the renderer; zero callers, including tests. Kept
// as a ready-made dev utility for a future "texture key resolved to nothing"
// fallback.
#[allow(dead_code)]
fn gen_debug() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let checker = ((x / 4) + (y / 4)) % 2 == 0;
            if checker {
                set_px(&mut px, x, y, 200, 0, 200, 255); // magenta
            } else {
                set_px(&mut px, x, y, 0, 0, 0, 255); // black
            }
        }
    }
    px
}

// ─── Cow Textures ───

fn gen_crafting_table_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Dark wood base with 3x3 grid lines
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            // Grid lines at 5, 10 (dividing 16 into ~thirds)
            let on_grid = x == 5 || x == 10 || y == 5 || y == 10;
            // Border
            let on_border = x == 0 || x == 15 || y == 0 || y == 15;
            let (r, g, b) = if on_border {
                (90, 60, 30)
            } else if on_grid {
                (100, 70, 35)
            } else {
                // Wood planks with slight variation
                let h = pixel_hash(x, y, 555);
                (160 + (h % 20) as u8, 120 + (h % 15) as u8, 70 + (h % 10) as u8)
            };
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

fn gen_cow_head_front() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Brown face with white patch, dark eyes, pink nose
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            // Base: brown
            let (mut r, mut g, mut b) = (139, 90, 43);
            // White patch on forehead (top half, centre)
            if y < 8 && (4..12).contains(&x) {
                (r, g, b) = (230, 220, 200);
            }
            // Eyes (row 8-9, col 3-4 and 11-12)
            if (y == 8 || y == 9) && ((x == 4 || x == 5) || (x == 10 || x == 11)) {
                (r, g, b) = (30, 20, 10);
            }
            // Nose/mouth area (bottom centre)
            if (12..=14).contains(&y) && (5..11).contains(&x) {
                (r, g, b) = (180, 140, 120);
            }
            // Nostrils
            if y == 13 && (x == 6 || x == 9) {
                (r, g, b) = (60, 40, 30);
            }
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

fn gen_cow_head_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let (mut r, mut g, mut b) = (139, 90, 43);
            // White patches
            if y < 6 && x > 4 {
                (r, g, b) = (230, 220, 200);
            }
            // Ear area (top-back)
            if y < 3 && x < 4 {
                (r, g, b) = (120, 75, 35);
            }
            // Eye
            if (y == 7 || y == 8) && (x == 10 || x == 11) {
                (r, g, b) = (30, 20, 10);
            }
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

fn gen_cow_head_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_color(&mut px, 139, 90, 43);
    // White centre patch
    for y in 4..12 {
        for x in 4..12 {
            set_pixel(&mut px, x, y, 230, 220, 200);
        }
    }
    // Horns (small bumps at edges)
    for y in 0..3 {
        set_pixel(&mut px, 2, y, 200, 190, 160);
        set_pixel(&mut px, 13, y, 200, 190, 160);
    }
    px
}

fn gen_cow_body_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Brown base with white splotches (classic cow pattern)
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 42);
            let (r, g, b) = if (h.is_multiple_of(3) && x > 2 && x < 14) || (y > 4 && y < 12 && h.is_multiple_of(5)) {
                (230, 220, 200) // White patches
            } else {
                (139u8.wrapping_add((h % 15) as u8), 90u8.wrapping_add((h % 10) as u8), 43)
            };
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

fn gen_cow_body_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Brown with white stripe
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let (r, g, b) = if (5..=10).contains(&x) {
                (230, 220, 200) // White stripe
            } else {
                (139, 90, 43)
            };
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

fn gen_cow_body_end() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_color(&mut px, 139, 90, 43);
    // Some lighter brown variation
    for y in 4..12 {
        for x in 4..12 {
            set_pixel(&mut px, x, y, 155, 105, 55);
        }
    }
    px
}

fn gen_cow_leg() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Dark brown leg, lighter at bottom (hooves)
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let (r, g, b) = if y >= 12 {
                (80, 55, 30) // Dark hooves
            } else {
                (120, 80, 40)
            };
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

// ─── Chicken Textures ───

fn gen_chicken_head() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // White head with red comb, orange beak, dark eyes
    fill_color(&mut px, 230, 225, 215);
    // Red comb on top
    for x in 5..11 {
        set_pixel(&mut px, x, 0, 200, 30, 30);
        set_pixel(&mut px, x, 1, 210, 40, 35);
    }
    for x in 6..10 { set_pixel(&mut px, x, 2, 190, 25, 25); }
    // Eyes
    set_pixel(&mut px, 4, 7, 15, 10, 10);
    set_pixel(&mut px, 11, 7, 15, 10, 10);
    // Beak (orange, centre bottom)
    for x in 6..10 {
        set_pixel(&mut px, x, 10, 230, 160, 40);
        set_pixel(&mut px, x, 11, 220, 150, 30);
    }
    // Red wattle below beak
    set_pixel(&mut px, 7, 12, 200, 30, 30);
    set_pixel(&mut px, 8, 12, 200, 30, 30);
    set_pixel(&mut px, 7, 13, 190, 25, 25);
    px
}

fn gen_chicken_body() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // White feathered body with slight texture variation
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 300);
            let v = 215 + (h % 25) as u8;
            px[i] = v; px[i+1] = v; px[i+2] = v.saturating_sub(10); px[i+3] = 255;
        }
    }
    px
}

fn gen_chicken_leg() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Orange/yellow thin legs
    fill_color(&mut px, 220, 160, 40);
    // Darker joint lines
    for x in 0..SIZE {
        set_pixel(&mut px, x, 4, 190, 130, 30);
        set_pixel(&mut px, x, 10, 190, 130, 30);
    }
    px
}

fn gen_chicken_wing() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // White-gray feathered wing
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 333);
            let v = 200 + (h % 30) as u8;
            px[i] = v; px[i+1] = v; px[i+2] = v.saturating_sub(15); px[i+3] = 255;
        }
    }
    px
}

// ─── Pig Textures ───

fn gen_pig_head_front() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_color(&mut px, 240, 170, 170);
    // Eyes
    for x in 3..5 { set_pixel(&mut px, x, 6, 30, 20, 20); set_pixel(&mut px, x, 7, 30, 20, 20); }
    for x in 11..13 { set_pixel(&mut px, x, 6, 30, 20, 20); set_pixel(&mut px, x, 7, 30, 20, 20); }
    // Snout — flat pink disc with two nostrils
    for y in 10..14 {
        for x in 5..11 {
            set_pixel(&mut px, x, y, 220, 145, 145);
        }
    }
    set_pixel(&mut px, 7, 12, 90, 50, 50);
    set_pixel(&mut px, 8, 12, 90, 50, 50);
    px
}

fn gen_pig_head_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_color(&mut px, 240, 170, 170);
    // Triangle ear (top-back)
    for y in 0..3 {
        for x in 0..(3 - y) { set_pixel(&mut px, x, y, 215, 145, 145); }
    }
    // Eye from side
    set_pixel(&mut px, 12, 6, 30, 20, 20);
    set_pixel(&mut px, 12, 7, 30, 20, 20);
    // Subtle freckles
    for y in 0..SIZE { for x in 0..SIZE {
        let h = pixel_hash(x, y, 410);
        if h.is_multiple_of(11) { set_pixel(&mut px, x, y, 230, 160, 160); }
    }}
    px
}

fn gen_pig_head_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_color(&mut px, 240, 170, 170);
    // Ears at the back corners
    for y in 0..3 { set_pixel(&mut px, 1, y, 215, 145, 145); set_pixel(&mut px, 14, y, 215, 145, 145); }
    px
}

fn gen_pig_body_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 411);
            let r = 240u8.wrapping_sub((h % 12) as u8);
            let g = 170u8.wrapping_sub((h % 10) as u8);
            let b = 170u8.wrapping_sub((h % 10) as u8);
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

fn gen_pig_body_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_color(&mut px, 235, 165, 165);
    // Spine line
    for y in 0..SIZE { set_pixel(&mut px, 7, y, 215, 145, 145); set_pixel(&mut px, 8, y, 215, 145, 145); }
    px
}

fn gen_pig_body_end() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_color(&mut px, 240, 170, 170);
    // Curly tail hint (back face)
    for y in 5..8 { set_pixel(&mut px, 7, y, 220, 150, 150); set_pixel(&mut px, 8, y, 220, 150, 150); }
    px
}

fn gen_pig_leg() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let (r, g, b) = if y >= 13 { (90, 60, 50) } else { (230, 160, 160) };
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

// ─── Sheep Textures ───

fn gen_sheep_head_front() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Tan/cream face
    fill_color(&mut px, 215, 195, 165);
    // Eyes
    for x in 4..6 { set_pixel(&mut px, x, 7, 25, 18, 12); }
    for x in 10..12 { set_pixel(&mut px, x, 7, 25, 18, 12); }
    // Nose dark patch
    for y in 11..13 { for x in 6..10 { set_pixel(&mut px, x, y, 60, 45, 35); } }
    px
}

fn gen_sheep_head_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_color(&mut px, 215, 195, 165);
    // Eye from side
    set_pixel(&mut px, 12, 7, 25, 18, 12);
    // Floppy ear
    for y in 4..8 { set_pixel(&mut px, 0, y, 195, 175, 145); set_pixel(&mut px, 1, y, 195, 175, 145); }
    px
}

fn gen_sheep_head_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_color(&mut px, 200, 180, 150);
    // Wool tuft on top of head
    for y in 4..12 { for x in 4..12 {
        let h = pixel_hash(x, y, 510);
        let v = 235 + (h % 20) as u8;
        set_pixel(&mut px, x, y, v, v, v.saturating_sub(8));
    }}
    px
}

fn gen_sheep_body_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // White woolly body with bumpy texture
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 511);
            let v = 230 + (h % 22) as u8;
            px[i] = v; px[i+1] = v; px[i+2] = v.saturating_sub(8); px[i+3] = 255;
        }
    }
    // Random darker tufts
    for y in 0..SIZE { for x in 0..SIZE {
        let h = pixel_hash(x, y, 512);
        if h.is_multiple_of(13) { set_pixel(&mut px, x, y, 210, 210, 200); }
    }}
    px
}

fn gen_sheep_body_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 513);
            let v = 235 + (h % 18) as u8;
            px[i] = v; px[i+1] = v; px[i+2] = v.saturating_sub(8); px[i+3] = 255;
        }
    }
    px
}

fn gen_sheep_body_end() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_color(&mut px, 240, 240, 232);
    // Slight tufts
    for y in 0..SIZE { for x in 0..SIZE {
        let h = pixel_hash(x, y, 514);
        if h.is_multiple_of(9) { set_pixel(&mut px, x, y, 220, 220, 210); }
    }}
    px
}

fn gen_sheep_leg() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Tan upper leg (skin), darker hoof bottom
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let (r, g, b) = if y >= 13 { (60, 45, 35) } else { (200, 180, 150) };
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

// ─── Item-drop Textures (Wave 2b) ───
//
// Each is a 16x16 fill-plus-detail giving non-block items a recognisable
// look on the ground (vs the earlier oak-planks fallback). Style is "tiny
// painted icon" — solid base + a couple of accent strokes per item.

fn gen_item_stick() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_color(&mut px, 90, 70, 40);
    // Wood-grain stripe down the middle.
    for y in 0..SIZE {
        set_pixel(&mut px, 7, y, 140, 105, 60);
        set_pixel(&mut px, 8, y, 140, 105, 60);
    }
    px
}

fn gen_item_leather() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 810);
            let v = 110 + (h % 18) as u8;
            px[i] = v.saturating_add(30); px[i+1] = v - 30; px[i+2] = v / 3; px[i+3] = 255;
        }
    }
    // Stitching marks
    for x in (1..SIZE).step_by(3) { set_pixel(&mut px, x, 1, 60, 35, 15); }
    px
}

fn gen_item_feather() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_color(&mut px, 240, 240, 235);
    // Quill spine
    for y in 1..15 { set_pixel(&mut px, 7, y, 200, 195, 170); set_pixel(&mut px, 8, y, 200, 195, 170); }
    // Barbs flicking out
    for y in 2..14 {
        let off = (y as i32 - 8).abs() / 2;
        let l = 5 - off.min(5);
        for x in (8 - l as u32)..8 { set_pixel(&mut px, x, y, 230, 230, 220); }
        for x in 9..(9 + l as u32) { set_pixel(&mut px, x, y, 230, 230, 220); }
    }
    px
}

fn gen_item_wool() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 811);
            let v = 230 + (h % 22) as u8;
            px[i] = v; px[i+1] = v; px[i+2] = v.saturating_sub(10); px[i+3] = 255;
        }
    }
    px
}

fn gen_item_bone() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_color(&mut px, 240, 235, 215);
    // End caps darker (rounded bone tips)
    for x in 0..3 { for y in 0..SIZE { set_pixel(&mut px, x, y, 215, 210, 190); } }
    for x in 13..SIZE { for y in 0..SIZE { set_pixel(&mut px, x, y, 215, 210, 190); } }
    // Marrow line
    for x in 4..12 { set_pixel(&mut px, x, 7, 200, 195, 170); set_pixel(&mut px, x, 8, 200, 195, 170); }
    px
}

fn gen_item_raw_beef() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 814);
            let r = 200 + (h % 25) as u8;
            let g = 70 + (h % 15) as u8;
            let b = 65 + (h % 10) as u8;
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    // Darker marbling
    for y in 0..SIZE { for x in 0..SIZE {
        let h = pixel_hash(x, y, 815);
        if h.is_multiple_of(9) { set_pixel(&mut px, x, y, 150, 35, 35); }
    }}
    px
}

fn gen_item_raw_porkchop() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 816);
            let r = 235 + (h % 18) as u8;
            let g = 140 + (h % 18) as u8;
            let b = 130 + (h % 12) as u8;
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    // Fat streaks (white)
    for y in [3u32, 9, 13].iter().copied() {
        for x in 2..14 { set_pixel(&mut px, x, y, 245, 230, 220); }
    }
    px
}

fn gen_item_raw_chicken() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 817);
            let r = 235 + (h % 18) as u8;
            let g = 200 + (h % 22) as u8;
            let b = 170 + (h % 18) as u8;
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

fn gen_item_raw_mutton() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 818);
            let r = 215 + (h % 22) as u8;
            let g = 100 + (h % 18) as u8;
            let b = 85 + (h % 12) as u8;
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    // Slight white fat at edges
    for x in 0..3 { for y in 0..SIZE { set_pixel(&mut px, x, y, 235, 220, 200); } }
    for x in 13..SIZE { for y in 0..SIZE { set_pixel(&mut px, x, y, 235, 220, 200); } }
    px
}

fn gen_item_string() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_color(&mut px, 210, 205, 195);
    // Looped fibres
    for y in (0..SIZE).step_by(3) {
        for x in 0..SIZE {
            let off = ((x as i32 + y as i32) % 6).abs();
            if off < 2 { set_pixel(&mut px, x, y, 175, 170, 160); }
        }
    }
    px
}

/// Generic grey-powder item texture (layer 79). Reused by Sulphur, InkSac
/// and BeeStinger via entity_model::TEX_ITEM_GREY_POWDER.
fn gen_item_grey_powder() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Dark grey with sparkly grain — speckled black + subtle highlights.
    for y in 0..SIZE { for x in 0..SIZE {
        let i = ((y * SIZE + x) * 4) as usize;
        let h = pixel_hash(x, y, 819);
        let v = 60 + (h % 25) as u8;
        px[i] = v; px[i+1] = v; px[i+2] = v + 5; px[i+3] = 255;
    }}
    // Sparks
    for y in 0..SIZE { for x in 0..SIZE {
        let h = pixel_hash(x, y, 820);
        if h.is_multiple_of(17) { set_pixel(&mut px, x, y, 200, 200, 210); }
        if h.is_multiple_of(19) { set_pixel(&mut px, x, y, 25, 25, 25); }
    }}
    px
}

/// Build a tool-tier texture: tier-coloured background with a wooden handle
/// running diagonally and a "head" wedge in the tier colour. Same silhouette
/// for all four tiers so they read as a family at a glance; tier itself is
/// coded in the head colour.
fn gen_tool_for_tier(head_rgb: (u8, u8, u8), bg_rgb: (u8, u8, u8)) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_color(&mut px, bg_rgb.0, bg_rgb.1, bg_rgb.2);
    // Wooden handle from bottom-left to top-right.
    for i in 0..14 {
        let x = 1 + i;
        let y = 14 - i;
        set_pixel(&mut px, x, y, 110, 75, 35);
        if x + 1 < SIZE { set_pixel(&mut px, x + 1, y, 90, 60, 25); }
    }
    // Tool head — diamond-ish wedge at the top-right.
    for d in 0..4 {
        for off in -2..=2i32 {
            let x = (10 + d) + off;
            let y = (4 + d) - off;
            if x >= 0 && x < SIZE as i32 && y >= 0 && y < SIZE as i32 {
                set_pixel(&mut px, x as u32, y as u32, head_rgb.0, head_rgb.1, head_rgb.2);
            }
        }
    }
    px
}

fn gen_item_tool_wood() -> Vec<u8> {
    gen_tool_for_tier((155, 110, 65), (95, 75, 45))
}

fn gen_item_tool_stone() -> Vec<u8> {
    gen_tool_for_tier((130, 130, 130), (85, 85, 85))
}

fn gen_item_tool_iron() -> Vec<u8> {
    gen_tool_for_tier((215, 215, 215), (130, 130, 130))
}

fn gen_item_tool_diamond() -> Vec<u8> {
    gen_tool_for_tier((90, 230, 220), (50, 130, 130))
}

// ─── Arrow (Wave 23) ───

/// Placeholder for layer 104 — keeps the layer-number bookkeeping aligned
/// while reserving the slot for future polish. Renders as a magenta "missing
/// texture" pattern so any accidental usage is visually obvious.
fn gen_placeholder_104() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let on = (x / 4 + y / 4) % 2 == 0;
            let (r, g, b) = if on { (255, 0, 255) } else { (40, 40, 40) };
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

fn gen_item_arrow() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Light wooden background; the arrow itself reads as a horizontal stripe.
    fill_color(&mut px, 35, 28, 22);
    // Shaft (light wood) running across centre rows
    for y in 7..9 {
        for x in 1..13 {
            set_pixel(&mut px, x, y, 200, 175, 130);
        }
    }
    // Iron arrowhead at one end (right)
    set_pixel(&mut px, 13, 7, 195, 195, 200);
    set_pixel(&mut px, 13, 8, 195, 195, 200);
    set_pixel(&mut px, 14, 7, 230, 230, 235);
    set_pixel(&mut px, 14, 8, 230, 230, 235);
    set_pixel(&mut px, 15, 7, 255, 255, 255);
    set_pixel(&mut px, 15, 8, 255, 255, 255);
    // Feather fletching at the other end (left)
    for y in 5i32..11 {
        let dy = (y - 7).abs();
        let half = (3 - dy) as u32;
        for x in 0..half {
            set_pixel(&mut px, x, y as u32, 230, 230, 220);
        }
    }
    px
}

// ─── Tall Grass (Wave 22) ───

fn gen_tall_grass() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Dark green-ish background reads as "grass tuft on a face"; vertical
    // bright-green strands suggest the grass blades.
    fill_color(&mut px, 32, 60, 24);
    // Vertical strands at x = 3, 7, 11 of varying height.
    let strands = [(3u32, 5u32, 14u32), (7, 2, 14), (11, 4, 14)];
    for (sx, top, bot) in strands.iter().copied() {
        for y in top..bot {
            let h = pixel_hash(sx, y, 1510);
            let g = 145 + (h % 30) as u8;
            let r = 60 + (h % 22) as u8;
            let b = 40 + (h % 18) as u8;
            set_pixel(&mut px, sx, y, r, g, b);
            // Stem highlight on the right side.
            if sx + 1 < SIZE {
                set_pixel(&mut px, sx + 1, y, r.saturating_sub(20), g.saturating_sub(20), b.saturating_sub(15));
            }
        }
    }
    // A few stray flecks at the base.
    for x in 1..15 {
        let h = pixel_hash(x, 14, 1511);
        if h.is_multiple_of(4) { set_pixel(&mut px, x, 14, 100, 175, 60); }
    }
    px
}

// ─── Smelting Outputs (Wave 6) ───

fn gen_item_iron_ingot() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Silver ingot — bevelled rectangle in the centre.
    fill_color(&mut px, 60, 60, 65);
    for y in 4..12 {
        for x in 3..13 {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 1410);
            let v = 195 + (h % 25) as u8;
            px[i] = v; px[i+1] = v; px[i+2] = v.saturating_add(2); px[i+3] = 255;
        }
    }
    // Top highlight band
    for x in 4..12 { set_pixel(&mut px, x, 4, 230, 230, 235); }
    // Bottom shadow band
    for x in 4..12 { set_pixel(&mut px, x, 11, 145, 145, 150); }
    px
}

fn gen_item_cooked_beef() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Darker than raw beef, with charred edges.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 1411);
            let r = 130 + (h % 25) as u8;
            let g = 60 + (h % 18) as u8;
            let b = 45 + (h % 12) as u8;
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    // Charred dark spots
    for y in 0..SIZE { for x in 0..SIZE {
        let h = pixel_hash(x, y, 1412);
        if h.is_multiple_of(11) { set_pixel(&mut px, x, y, 60, 30, 22); }
    }}
    px
}

fn gen_item_cooked_porkchop() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Tan-pink with seared streaks.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 1413);
            let r = 195 + (h % 22) as u8;
            let g = 140 + (h % 18) as u8;
            let b = 105 + (h % 14) as u8;
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    // Sear streaks
    for y in [4u32, 9, 12].iter().copied() {
        for x in 2..14 {
            if (x + y) % 3 != 0 { set_pixel(&mut px, x, y, 130, 80, 55); }
        }
    }
    px
}

fn gen_item_cooked_chicken() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Golden brown.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 1414);
            let r = 200 + (h % 25) as u8;
            let g = 165 + (h % 20) as u8;
            let b = 105 + (h % 18) as u8;
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    // Crispy spots
    for y in 0..SIZE { for x in 0..SIZE {
        let h = pixel_hash(x, y, 1415);
        if h.is_multiple_of(13) { set_pixel(&mut px, x, y, 140, 100, 55); }
    }}
    px
}

fn gen_item_cooked_mutton() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Darker red-brown than cooked beef.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 1416);
            let r = 145 + (h % 22) as u8;
            let g = 70 + (h % 18) as u8;
            let b = 55 + (h % 14) as u8;
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    // Bone bits at the edges
    for y in 0..SIZE {
        set_pixel(&mut px, 0, y, 215, 200, 175);
        set_pixel(&mut px, 15, y, 215, 200, 175);
    }
    px
}

// ─── Torch (Wave 18) ───

fn gen_torch() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Dark transparent-ish background so the torch reads as a slim shape on
    // a cube face. Using full-alpha here because the renderer's block path
    // doesn't blend per-pixel; future polish: switch to a cross-mesh shape.
    fill_color(&mut px, 18, 14, 10);
    // Wooden handle column (centre 2 pixels wide, lower 8 rows)
    for y in 8..SIZE {
        for x in 7..9 {
            set_pixel(&mut px, x, y, 110, 75, 35);
        }
    }
    // Flame head — bright orange/yellow blob in upper portion
    for y in 2..8 {
        for x in 6..10 {
            let dx = x as i32 - 7;
            let dy = y as i32 - 5;
            let r2 = dx * dx + dy * dy;
            if r2 <= 6 {
                let bright = 255 - (r2.abs() * 20) as u8;
                set_pixel(&mut px, x, y, bright, bright.saturating_sub(40), 50);
            }
        }
    }
    // Hot core (bright white at the centre)
    set_pixel(&mut px, 7, 4, 255, 240, 200);
    set_pixel(&mut px, 8, 4, 255, 240, 200);
    set_pixel(&mut px, 7, 5, 255, 220, 150);
    set_pixel(&mut px, 8, 5, 255, 220, 150);
    px
}

// ─── Storage Blocks (Wave 17) ───

fn gen_coal_block() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Solid coal — black with a few subtle highlights.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 1310);
            let v = 18 + (h % 16) as u8;
            px[i] = v; px[i+1] = v; px[i+2] = v; px[i+3] = 255;
        }
    }
    for y in 0..SIZE { for x in 0..SIZE {
        let h = pixel_hash(x, y, 1311);
        if h.is_multiple_of(23) { set_pixel(&mut px, x, y, 60, 60, 65); }
    }}
    px
}

fn gen_iron_block() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Polished iron — silvery with mottled highlights and a riveted look.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 1312);
            let v = 195 + (h % 30) as u8;
            px[i] = v; px[i+1] = v; px[i+2] = v.saturating_sub(8); px[i+3] = 255;
        }
    }
    // Rivet dots at corners
    for &(x, y) in &[(2u32, 2u32), (13, 2), (2, 13), (13, 13)] {
        set_pixel(&mut px, x, y, 130, 130, 130);
    }
    // Border framing
    for x in 0..SIZE { set_pixel(&mut px, x, 0, 165, 165, 165); set_pixel(&mut px, x, 15, 165, 165, 165); }
    for y in 0..SIZE { set_pixel(&mut px, 0, y, 165, 165, 165); set_pixel(&mut px, 15, y, 165, 165, 165); }
    px
}

fn gen_diamond_block() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Cyan-blue with crystalline facets.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 1313);
            let g = 200 + (h % 30) as u8;
            let b = 220 + (h % 25) as u8;
            px[i] = 100 + (h % 20) as u8;
            px[i+1] = g;
            px[i+2] = b;
            px[i+3] = 255;
        }
    }
    // Diamond-shaped facet highlight in the centre
    for d in 0i32..6 {
        let y = (5 + d / 2) as u32;
        let x_mid = 7;
        let half = (3 - (d - 3).abs()) as u32;
        for off in 0..=half {
            set_pixel(&mut px, x_mid - off, y, 230, 250, 250);
            set_pixel(&mut px, x_mid + off, y, 230, 250, 250);
        }
    }
    px
}

// ─── Glass Texture (Wave 16) ───

fn gen_glass() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Light blue tint with white highlights at the edges to read as a pane.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 1210);
            let v = 220 + (h % 25) as u8;
            // Slight blue cast
            px[i] = v.saturating_sub(10);
            px[i+1] = v.saturating_sub(5);
            px[i+2] = v;
            // Translucent corner suggestion: lower alpha at edges
            let edge = x == 0 || x == 15 || y == 0 || y == 15;
            px[i+3] = if edge { 240 } else { 200 };
        }
    }
    // White highlight stripe (corner glint)
    for i in 0..3u32 {
        set_pixel(&mut px, 1 + i, 1 + i, 255, 255, 255);
    }
    px
}

// ─── Bed Textures (Wave 9) ───

fn gen_bed_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Red mattress with a white pillow at one end.
    fill_color(&mut px, 195, 35, 35);
    // Pillow at the back (top half of texture)
    for y in 0..6 {
        for x in 1..15 {
            set_pixel(&mut px, x, y, 240, 240, 232);
        }
    }
    // Pillow shadow line
    for x in 1..15 { set_pixel(&mut px, x, 6, 175, 175, 165); }
    // Mattress quilting pattern — light red diamond grid
    for y in 7..16 {
        for x in 0..SIZE {
            if (x + y) % 4 == 0 { set_pixel(&mut px, x, y, 220, 60, 60); }
        }
    }
    // Wood frame edge along the side
    for x in 0..SIZE { set_pixel(&mut px, x, 15, 110, 75, 35); }
    px
}

fn gen_bed_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Wood plank base for the lower 8 rows; red mattress for the upper 8.
    for y in 0..8 {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 920);
            let r = 195u8.wrapping_sub((h % 8) as u8);
            let g = 35 + (h % 6) as u8;
            let b = 35 + (h % 5) as u8;
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    for y in 8..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 921);
            let r = 110u8.wrapping_add((h % 22) as u8);
            let g = 75 + (h % 15) as u8;
            let b = 35 + (h % 10) as u8;
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    // Mattress/wood seam
    for x in 0..SIZE { set_pixel(&mut px, x, 7, 80, 18, 18); }
    px
}

// ─── Ore Textures (Wave 13) ───

/// Stone base + coloured flecks. Helper used by all three ore textures.
fn gen_ore(fleck_rgb: (u8, u8, u8), accent_rgb: (u8, u8, u8), seed: u32) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Mottled stone background — same look as gen_stone but inlined so we
    // don't depend on the order block textures are registered.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 1);
            let v = 110 + (h % 30) as u8;
            px[i] = v; px[i+1] = v; px[i+2] = v.saturating_add(5); px[i+3] = 255;
        }
    }
    // Ore flecks scattered across the face, with brighter accent specks.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = pixel_hash(x, y, seed);
            if h.is_multiple_of(6) {
                set_pixel(&mut px, x, y, fleck_rgb.0, fleck_rgb.1, fleck_rgb.2);
            } else if h.is_multiple_of(23) {
                set_pixel(&mut px, x, y, accent_rgb.0, accent_rgb.1, accent_rgb.2);
            }
        }
    }
    px
}

fn gen_coal_ore() -> Vec<u8> {
    gen_ore((25, 25, 25), (60, 60, 60), 1010)
}

fn gen_iron_ore() -> Vec<u8> {
    gen_ore((175, 130, 90), (215, 175, 120), 1011)
}

fn gen_diamond_ore() -> Vec<u8> {
    gen_ore((110, 220, 215), (200, 245, 240), 1012)
}

/// Copper Ore (Wind, Copper & Electricity wave, 2026-09-07) — same
/// procedural family as `gen_iron_ore`/`gen_diamond_ore`, copper-orange
/// flecks with a lighter accent speck. Registered post-tint (508); see
/// `block::TEX_COPPER_ORE`.
fn gen_copper_ore() -> Vec<u8> {
    gen_ore((199, 115, 64), (225, 150, 90), 1013)
}

// ─── Texture helpers ───

fn fill_color(px: &mut [u8], r: u8, g: u8, b: u8) {
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
}

fn set_pixel(px: &mut [u8], x: u32, y: u32, r: u8, g: u8, b: u8) {
    if x < SIZE && y < SIZE {
        let i = ((y * SIZE + x) * 4) as usize;
        px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
    }
}

fn pixel_hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h = x.wrapping_mul(374761393) ^ y.wrapping_mul(668265263) ^ seed.wrapping_mul(1274126177);
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^ (h >> 16)
}

// ─── Deepslate Textures (Wave 25 — Spec 2 §5.3.1a) ───

/// Dark mottled background — the deepslate equivalent of `gen_stone`.
/// Inlined into the ore helpers so we don't depend on registration order.
fn fill_deepslate_base(px: &mut [u8], base_seed: u32) {
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, base_seed);
            // Range 55..=82 — clearly darker than the stone base (110..=140).
            let v = 55 + (h % 28) as u8;
            // Slight blue tint to distinguish from plain dark stone.
            px[i] = v;
            px[i + 1] = v;
            px[i + 2] = v.saturating_add(8);
            px[i + 3] = 255;
        }
    }
}

fn gen_pure_deepslate() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_deepslate_base(&mut px, 4101);
    // Subtle veining lines (vertical grain) to give it character distinct
    // from stone.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = pixel_hash(x, y, 4102);
            if h.is_multiple_of(47) {
                let i = ((y * SIZE + x) * 4) as usize;
                let v = 35 + (h % 12) as u8;
                px[i] = v;
                px[i + 1] = v;
                px[i + 2] = v.saturating_add(6);
            }
        }
    }
    px
}

fn gen_deepslate_ore(fleck_rgb: (u8, u8, u8), accent_rgb: (u8, u8, u8), seed: u32) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_deepslate_base(&mut px, 4101);
    // Ore flecks — same density as stone-tier but on the darker substrate
    // so they pop more.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = pixel_hash(x, y, seed);
            if h.is_multiple_of(6) {
                set_pixel(&mut px, x, y, fleck_rgb.0, fleck_rgb.1, fleck_rgb.2);
            } else if h.is_multiple_of(23) {
                set_pixel(&mut px, x, y, accent_rgb.0, accent_rgb.1, accent_rgb.2);
            }
        }
    }
    px
}

fn gen_deepslate_coal_ore() -> Vec<u8> {
    gen_deepslate_ore((15, 15, 15), (45, 45, 45), 4201)
}

fn gen_deepslate_iron_ore() -> Vec<u8> {
    gen_deepslate_ore((175, 130, 90), (215, 175, 120), 4202)
}

fn gen_deepslate_diamond_ore() -> Vec<u8> {
    gen_deepslate_ore((110, 220, 215), (200, 245, 240), 4203)
}

// ─── Satori Storage Block + Item Drop (Wave 25 — Spec 5 §3.8) ───

fn gen_satori_block() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Vivid orange base with golden highlights — the in-world face of
    // Bitcoin (which itself uses an orange brand-mark).
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = pixel_hash(x, y, 4301);
            let r = 220 + (h % 30) as u8;
            let g = 130 + (h % 25) as u8;
            let b = 30 + (h % 20) as u8;
            px[i] = r;
            px[i + 1] = g;
            px[i + 2] = b;
            px[i + 3] = 255;
        }
    }
    // Diagonal highlight lines — gem-facet feel without going overboard.
    for x in 0..SIZE {
        let y = x;
        set_pixel(&mut px, x, y, 255, 220, 140);
        if x + 1 < SIZE {
            set_pixel(&mut px, x + 1, y, 255, 230, 160);
        }
    }
    for x in 0..SIZE {
        let y = (SIZE - 1).saturating_sub(x);
        set_pixel(&mut px, x, y, 255, 200, 110);
    }
    px
}

fn gen_item_satori() -> Vec<u8> {
    // 16×16 transparent canvas with a small faceted gem in the centre.
    let mut px = vec![0u8; PIXELS];
    // Diamond-ish shape: rows 4..12, columns proportional to distance from
    // vertical centre.
    let cx = SIZE as i32 / 2;
    let cy = SIZE as i32 / 2;
    for y in 0..SIZE as i32 {
        let dy = (y - cy).abs();
        // Half-width at this row: max(0, 6 - dy)
        let hw = (6 - dy).max(0);
        for x in 0..SIZE as i32 {
            let dx = (x - cx).abs();
            if dx <= hw {
                // Vivid orange core, brighter on the upper-left for fake
                // light direction.
                let lit = dy <= 2 && (x - cx) <= 0;
                let (r, g, b) = if lit {
                    (255, 200, 130)
                } else {
                    (235, 140, 40)
                };
                let i = ((y as u32 * SIZE + x as u32) * 4) as usize;
                px[i] = r;
                px[i + 1] = g;
                px[i + 2] = b;
                px[i + 3] = 255;
            }
        }
    }
    // Dark outline for contrast on backgrounds.
    let outline_pts: [(i32, i32); 8] = [
        (cx, cy - 6),
        (cx, cy + 6),
        (cx - 6, cy),
        (cx + 6, cy),
        (cx - 4, cy - 4),
        (cx + 4, cy - 4),
        (cx - 4, cy + 4),
        (cx + 4, cy + 4),
    ];
    for (x, y) in outline_pts {
        if x >= 0 && y >= 0 && x < SIZE as i32 && y < SIZE as i32 {
            set_pixel(&mut px, x as u32, y as u32, 90, 40, 10);
        }
    }
    px
}

// --- Farming Tier 1 textures (Wave 26, 2026-05-17) ---

/// Tilled-soil top face. Darker than plain dirt with parallel furrow
/// lines that read as "hoed earth" at a glance. Sides + bottom of the
/// block still render as plain dirt (per BlockRegistry config).
fn gen_tilled_soil() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 2600);
            let noise = (h % 17) as i32 - 8;
            // Darker base than dirt (which uses 134/96/67); furrows
            // shave another step off in the trough rows.
            let furrow_row = (y / 4) % 2 == 0;
            let row_bias: i32 = if furrow_row { -18 } else { 0 };
            let r = clamp_u8(96 + noise + row_bias);
            let g = clamp_u8(64 + noise * 6 / 10 + row_bias);
            let b = clamp_u8(40 + noise / 2 + row_bias);
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Simple icon-style item texture — solid base colour with noise +
/// a darker outline ring. Shared shape for the new farming item
/// drops (seeds, wheat, bread, carrot, potato). Each caller passes
/// its own RGB so the items remain visually distinct in the hotbar.
fn gen_farming_item(seed: u32, r: u8, g: u8, b: u8) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let cx = SIZE as i32 / 2;
    let cy = SIZE as i32 / 2;
    let radius_sq = (SIZE as i32 / 2 - 2).pow(2);
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let dx = x - cx;
            let dy = y - cy;
            let d2 = dx * dx + dy * dy;
            if d2 > (SIZE as i32 / 2).pow(2) {
                // Outside the icon — fully transparent.
                continue;
            }
            let h = px_hash(x as u32, y as u32, seed);
            let noise = (h % 31) as i32 - 15;
            let rr = clamp_u8(r as i32 + noise / 3);
            let gg = clamp_u8(g as i32 + noise / 3);
            let bb = clamp_u8(b as i32 + noise / 3);
            // Outline ring near the boundary. `alpha` is unconditionally opaque
            // (both branches were 255 — collapsed here; `darken` is the part that
            // actually varies with the boundary check).
            let alpha = 255;
            let darken = if d2 > radius_sq { 60 } else { 0 };
            set_px(
                &mut px,
                x as u32,
                y as u32,
                clamp_u8(rr as i32 - darken),
                clamp_u8(gg as i32 - darken),
                clamp_u8(bb as i32 - darken),
                alpha,
            );
        }
    }
    px
}

fn gen_item_wheat_seeds() -> Vec<u8> {
    gen_farming_item(2613, 217, 192, 115)
}

fn gen_item_wheat() -> Vec<u8> {
    gen_farming_item(2614, 242, 209, 89)
}

fn gen_item_bread() -> Vec<u8> {
    gen_farming_item(2615, 199, 140, 76)
}

fn gen_item_carrot() -> Vec<u8> {
    gen_farming_item(2616, 242, 140, 46)
}

fn gen_item_potato() -> Vec<u8> {
    gen_farming_item(2617, 217, 191, 140)
}

// --- Campfire textures (Wave 27 — 2026-05-18) ---

/// Lit campfire's top face — central flame on a log-cross.
fn gen_campfire_lit_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Dark wood base over the whole tile.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 2700);
            let noise = (h % 11) as i32 - 5;
            set_px(&mut px, x, y,
                clamp_u8(70 + noise),
                clamp_u8(48 + noise / 2),
                clamp_u8(32 + noise / 3),
                255,
            );
        }
    }
    // Central flame — orange-yellow oval, ~6×8 px.
    let cx = SIZE as i32 / 2;
    let cy = SIZE as i32 / 2;
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let dx = x - cx;
            let dy = y - cy;
            // Vertical-ish oval flame.
            let d2 = dx * dx * 4 + dy * dy * 2;
            if d2 < 36 {
                let h = px_hash(x as u32, y as u32, 2701);
                let intensity = ((d2 as u32 * 100) / 36) as u8;
                set_px(&mut px, x as u32, y as u32,
                    clamp_u8(240 - intensity as i32 / 4),
                    clamp_u8(180 - intensity as i32 / 2 + (h % 30) as i32 - 15),
                    clamp_u8(40 + (h % 20) as i32),
                    255,
                );
            }
        }
    }
    px
}

/// Lit campfire's side face — stacked logs with a glowing red gap.
fn gen_campfire_lit_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 2710);
            let noise = (h % 13) as i32 - 6;
            // Logs occupy top + bottom thirds; middle has the glow.
            let in_middle = (7..=9).contains(&y);
            let (r, g, b) = if in_middle {
                (clamp_u8(220 + noise),
                 clamp_u8(80 + noise * 2),
                 clamp_u8(20 + noise))
            } else {
                (clamp_u8(85 + noise),
                 clamp_u8(58 + noise / 2),
                 clamp_u8(36 + noise / 3))
            };
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Unlit campfire's top — cold ash + charred logs.
fn gen_campfire_unlit_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 2720);
            let noise = (h % 13) as i32 - 6;
            // Dark grey/brown over everything.
            set_px(&mut px, x, y,
                clamp_u8(55 + noise),
                clamp_u8(48 + noise),
                clamp_u8(40 + noise),
                255,
            );
        }
    }
    // Central ash patch — slightly lighter grey.
    let cx = SIZE as i32 / 2;
    let cy = SIZE as i32 / 2;
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let dx = x - cx;
            let dy = y - cy;
            if dx * dx + dy * dy < 16 {
                let h = px_hash(x as u32, y as u32, 2721);
                let noise = (h % 21) as i32 - 10;
                set_px(&mut px, x as u32, y as u32,
                    clamp_u8(110 + noise),
                    clamp_u8(105 + noise),
                    clamp_u8(98 + noise),
                    255,
                );
            }
        }
    }
    px
}

/// Flint item — small grey shard.
fn gen_item_flint() -> Vec<u8> {
    gen_farming_item(2740, 115, 108, 102)
}

/// Flint and steel — grey shard + iron-coloured striker. Reuses the
/// generic farming-item shape but with steel-tinged colour.
fn gen_item_flint_and_steel() -> Vec<u8> {
    gen_farming_item(2741, 165, 155, 145)
}

/// Campfire smoke pillar — grey wispy texture with partial transparency.
/// Designed to read as a hazy column at any distance. Soft edges via per-
/// pixel alpha falloff toward the sides + sparse noise so it looks like
/// moving smoke (it doesn't actually move — just visually noisy).
fn gen_campfire_smoke() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let cx = SIZE as i32 / 2;
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let h = px_hash(x as u32, y as u32, 2820);
            // Horizontal falloff from centre to edges.
            let dx = (x - cx).abs();
            let edge = ((dx as u32 * 32) / (SIZE / 2)) as i32;
            // Per-pixel "puff" noise — speckled lighter / darker pixels.
            let noise = (h % 31) as i32 - 15;
            // Base grey tones.
            let lum = 175 - edge - noise / 2;
            // Alpha falls off near the edges and gets a little speckle.
            let alpha = clamp_u8(190 - edge * 4 - noise.abs() / 2);
            set_px(&mut px, x as u32, y as u32,
                clamp_u8(lum),
                clamp_u8(lum),
                clamp_u8(lum),
                alpha,
            );
        }
    }
    px
}

/// Spec 22 Phase 7 — red/ember-tinted smoke pillar variant. Mirror of
/// `gen_campfire_smoke` but the grey ramp is swapped for an ember-red
/// palette so the pillar reads as "warning" from a distance without
/// being lurid. Used when the source campfire's `raid_warning_active`
/// flag is true; the mesh builder selects the layer per-cell.
fn gen_campfire_smoke_warning() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let cx = SIZE as i32 / 2;
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let h = px_hash(x as u32, y as u32, 2821);
            let dx = (x - cx).abs();
            let edge = ((dx as u32 * 32) / (SIZE / 2)) as i32;
            let noise = (h % 31) as i32 - 15;
            // Ember-red base — red dominant, a touch of orange in the
            // green channel, very little blue. Same falloff curve as
            // the grey variant so the silhouette stays identical.
            let r = clamp_u8(205 - edge - noise / 2);
            let g = clamp_u8(85 - edge - noise / 2);
            let b = clamp_u8(50 - edge - noise / 2);
            let alpha = clamp_u8(190 - edge * 4 - noise.abs() / 2);
            set_px(&mut px, x as u32, y as u32, r, g, b, alpha);
        }
    }
    px
}

/// Unlit campfire's side — same stacked logs, no glow.
fn gen_campfire_unlit_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 2730);
            let noise = (h % 11) as i32 - 5;
            set_px(&mut px, x, y,
                clamp_u8(75 + noise),
                clamp_u8(52 + noise / 2),
                clamp_u8(34 + noise / 3),
                255,
            );
        }
    }
    px
}

/// Procedural crop-stage texture. `stage` ∈ 0..=3 determines how high
/// up the texture the plant reaches:
///   stage 0 — bottom 4 rows only (sprouts)
///   stage 1 — bottom 8 rows (young)
///   stage 2 — bottom 12 rows (filling out)
///   stage 3 — full block (mature)
/// The rest of the texture is fully transparent so neighbouring blocks
/// (tilled soil below, air above) read through.
fn gen_crop_stage(seed: u32, leaf: [u8; 3], stage: u8) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let fill_rows: u32 = match stage {
        0 => 4,
        1 => 8,
        2 => 12,
        _ => SIZE,
    };
    for y in (SIZE - fill_rows)..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, seed);
            // Vertical strands — pixel is "stalk" if its column's hash
            // bit matches; otherwise transparent. Higher stages have
            // more strands.
            let strand_chance: u32 = match stage {
                0 => 30,
                1 => 45,
                2 => 60,
                _ => 80,
            };
            let is_strand = (h % 100) < strand_chance;
            if !is_strand {
                continue;
            }
            let noise = ((h >> 8) % 31) as i32 - 15;
            let r = clamp_u8(leaf[0] as i32 + noise / 4);
            let g = clamp_u8(leaf[1] as i32 + noise / 4);
            let b = clamp_u8(leaf[2] as i32 + noise / 4);
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Block-break crack overlay, stage 0-9 (Spec 05 §2.2). Transparent 16×16 with
/// near-black crack lines that grow in number, length, thickness and opacity as
/// the stage rises — hairline at stage 0, heavy fracturing at stage 9. Drawn
/// alpha-blended over the targeted block, so only the crack pixels darken the
/// face. Deterministic (seeded by `px_hash`) so the pattern is stable across
/// frames while mining the same block.
fn gen_crack_stage(stage: u8) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS]; // transparent background (a = 0)
    let s = stage as u32;
    let crack_rgb = 18u8;
    let alpha = clamp_u8(80 + stage as i32 * 17); // 80 (faint) .. 233 (dense)
    let num_cracks = 2 + s; // 2 cracks at stage 0 .. 11 at stage 9
    let center = (SIZE / 2) as i32;
    // 8-way step directions for the random walk.
    const DIRS: [(i32, i32); 8] =
        [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];

    let plot = |buf: &mut [u8], x: i32, y: i32| {
        if x >= 0 && x < SIZE as i32 && y >= 0 && y < SIZE as i32 {
            set_px(buf, x as u32, y as u32, crack_rgb, crack_rgb, crack_rgb, alpha);
        }
    };

    for c in 0..num_cracks {
        let seed = s.wrapping_mul(1000).wrapping_add(c.wrapping_mul(37)).wrapping_add(7);
        // Start near the block centre, jittered.
        let mut x = center + (px_hash(c, 0, seed) % 5) as i32 - 2;
        let mut y = center + (px_hash(0, c, seed) % 5) as i32 - 2;
        let mut dir = (px_hash(c, c, seed) % 8) as i32;
        let len = 6 + stage as i32; // longer fractures at higher stages
        for step in 0..len {
            plot(&mut px, x, y);
            if stage >= 6 {
                // Thicken late-stage cracks by one pixel.
                plot(&mut px, x + 1, y);
            }
            let (dx, dy) = DIRS[dir as usize % 8];
            x += dx;
            y += dy;
            if x < 0 || x >= SIZE as i32 || y < 0 || y >= SIZE as i32 {
                break;
            }
            // Occasional jagged turn.
            if px_hash(step as u32, c, seed).is_multiple_of(3) {
                let turn = if px_hash(c, step as u32, seed).is_multiple_of(2) { 1 } else { 7 };
                dir = (dir + turn) % 8;
            }
        }
    }
    px
}

// --- Spec 19 villager-family textures ---

/// Villager head (front face). Tan skin + dark eyes + prominent nose
/// nub that reads at distance — recognisable as a person, not a mob.
fn gen_villager_head_front() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            // Base skin tone — warm tan.
            let (mut r, mut g, mut b) = (210, 175, 145);
            // Hair line at top.
            if y < 3 {
                (r, g, b) = (90, 60, 35);
            }
            // Eyes — row 7-8, cols 4-5 and 10-11.
            if (y == 7 || y == 8) && ((x == 4 || x == 5) || (x == 10 || x == 11)) {
                (r, g, b) = (35, 25, 15);
            }
            // Nose — central column, slightly protruding shade.
            if (8..=12).contains(&y) && (x == 7 || x == 8) {
                (r, g, b) = (175, 135, 105);
            }
            // Mouth.
            if y == 13 && (6..=9).contains(&x) {
                (r, g, b) = (110, 60, 50);
            }
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

/// Villager head (side). Same skin tone, no eyes, hair on top + back.
fn gen_villager_head_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let (mut r, mut g, mut b) = (210, 175, 145);
            // Hair top + back.
            if y < 3 || (y < 6 && x < 4) {
                (r, g, b) = (90, 60, 35);
            }
            // Ear nub.
            if (7..=9).contains(&y) && (2..=3).contains(&x) {
                (r, g, b) = (190, 155, 130);
            }
            // Eye visible on side view (face direction = +x).
            if (y == 7 || y == 8) && (x == 11 || x == 12) {
                (r, g, b) = (35, 25, 15);
            }
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

/// Villager torso (robe). Brown by default, purple for wandering variant.
fn gen_villager_body(wandering: bool) -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let (base_r, base_g, base_b) = if wandering {
        (95, 55, 130) // muted purple
    } else {
        (120, 75, 45) // dark brown
    };
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = px_hash(x, y, if wandering { 4101 } else { 4100 });
            let noise = (h % 21) as i32 - 10;
            let (mut r, mut g, mut b) = (
                clamp_u8(base_r + noise / 2),
                clamp_u8(base_g + noise / 2),
                clamp_u8(base_b + noise / 2),
            );
            // Belt mid-torso.
            if y == 8 || y == 9 {
                (r, g, b) = (60, 40, 25);
            }
            // Lighter cuffs on the trim.
            if y == 0 || y == SIZE - 1 {
                (r, g, b) = (
                    clamp_u8(r as i32 + 25),
                    clamp_u8(g as i32 + 25),
                    clamp_u8(b as i32 + 25),
                );
            }
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

/// Villager legs — simple dark-tan trousers.
fn gen_villager_leg() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = px_hash(x, y, 4102);
            let noise = (h % 17) as i32 - 8;
            let r = clamp_u8(85 + noise / 2);
            let g = clamp_u8(55 + noise / 2);
            let b = clamp_u8(35 + noise / 2);
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

// --- Satoshi the founder-sage (2026-06-25 — hooded head + glow trim) ---
//
// A warm sage in a brown hood (the cloak read is textural, like the Peddler's
// wandering head). Kid-safe: the face stays visible and kindly — never a
// shadowed-creepy stranger. The trim is a soft pale glow for his throat amulet,
// rendered unlit via the emissive sentinel (entity_model::SATOSHI_GLOW_LIGHT).

/// Satoshi's hooded head (front): a brown hood framing a warm face with kind
/// eyes and a soft grey beard.
fn gen_satoshi_head_front() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            // Brown hood (frames the face).
            let h = px_hash(x, y, 4150);
            let n = (h % 19) as i32 - 9;
            let (mut r, mut g, mut b) =
                (clamp_u8(96 + n / 2), clamp_u8(63 + n / 2), clamp_u8(40 + n / 2));
            // Face opening — a centred panel, corners softened so the hood reads
            // as rounded.
            let in_face = (4..12).contains(&x) && (3..14).contains(&y)
                && !((x == 4 || x == 11) && (y <= 4 || y >= 12));
            if in_face {
                (r, g, b) = (212, 178, 148); // warm tan skin
                if (y == 6 || y == 7) && (x == 6 || x == 9) {
                    (r, g, b) = (44, 30, 20); // kind eyes
                }
                if (8..=9).contains(&y) && (x == 7 || x == 8) {
                    (r, g, b) = (184, 144, 112); // nose
                }
                if y >= 11 {
                    let bh = px_hash(x, y, 4151);
                    let bn = (bh % 23) as i32 - 11;
                    (r, g, b) = (
                        clamp_u8(186 + bn / 2),
                        clamp_u8(184 + bn / 2),
                        clamp_u8(180 + bn / 2),
                    ); // soft grey beard
                }
            }
            px[i] = r; px[i + 1] = g; px[i + 2] = b; px[i + 3] = 255;
        }
    }
    px
}

/// Satoshi's hooded head (side): mostly brown hood with a sliver of face toward
/// the front (high-x) edge.
fn gen_satoshi_head_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = px_hash(x, y, 4152);
            let n = (h % 19) as i32 - 9;
            let (mut r, mut g, mut b) =
                (clamp_u8(96 + n / 2), clamp_u8(63 + n / 2), clamp_u8(40 + n / 2));
            if x >= 11 && (4..13).contains(&y) {
                (r, g, b) = (208, 173, 143); // face sliver
                if (y == 6 || y == 7) && x == 12 {
                    (r, g, b) = (44, 30, 20); // eye
                }
                if y >= 11 {
                    (r, g, b) = (186, 184, 180); // beard
                }
            }
            px[i] = r; px[i + 1] = g; px[i + 2] = b; px[i + 3] = 255;
        }
    }
    px
}

/// Satoshi's glow trim — a soft, pale luminescence for his throat amulet
/// (rendered unlit via the emissive sentinel, so it shines warmly in a dim hut).
fn gen_satoshi_trim() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let h = px_hash(x, y, 4153);
            let n = (h % 21) as i32 - 10;
            // Warm-white with a faint cyan cast → gentle, magical, not harsh.
            let r = clamp_u8(212 + n / 2);
            let g = clamp_u8(228 + n / 2);
            let b = clamp_u8(246 + n / 2);
            px[i] = r; px[i + 1] = g; px[i + 2] = b; px[i + 3] = 255;
        }
    }
    px
}

// --- Player avatar skin (Phase 4) ---
//
// Default humanoid skin. Mirrors the villager generators' structure but with
// a friendlier, more "player-character" palette: warm skin tone, brown hair,
// a bright blue shirt, and brown trousers. Kept deliberately simple +
// recognisable. Per-player tint is a Phase 6 concern; nothing tints here.

// The 16x16 player skin generators (gen_player_head_front/side/top, _body,
// _arm, _leg) and their PLAYER_SKIN/HAIR/SHIRT/TROUSER colour palette were
// removed when the player moved to the 64x64 skin path (skin_texture +
// skin_uv). Layers 201-206 are now grey fillers (see the push loop above and
// gen_retired_layer). The original art is preserved in git history.

/// Our ORIGINAL default player skin, 64x64 RGBA (NOT Steve/Alex — see spec §IP).
/// Painted procedurally into the standard classic-layout regions. The avatar's
/// FACE is the -Z head face, which the skin_uv table maps to the (8,8)-(16,16)
/// tile — that's where the eyes go.
pub fn default_skin_rgba() -> Vec<u8> {
    let mut px = vec![0u8; 64 * 64 * 4]; // transparent base (overlay layer stays clear in v1)
    // Single fill closure (avoids two-mutable-closure borrow conflicts). NLL
    // releases the borrow after the last call, so `px` can be returned.
    let mut fill = |x0: u32, y0: u32, w: u32, h: u32, c: [u8; 4]| {
        for yy in y0..y0 + h {
            for xx in x0..x0 + w {
                let i = ((yy * 64 + xx) * 4) as usize;
                px[i..i + 4].copy_from_slice(&c);
            }
        }
    };
    let skin = [224u8, 172, 132, 255];
    let shirt = [70u8, 130, 180, 255]; // steel-blue shirt (our look, not MC)
    let trousers = [60u8, 60, 80, 255];
    let eye = [40u8, 40, 60, 255];
    let hair = [90u8, 60, 40, 255];

    // Head base: all 6 head faces live in the (0,0)-(32,16) atlas block.
    fill(0, 8, 32, 8, skin); // the side/front/back row (y 8..16)
    fill(8, 0, 16, 8, skin); // the top + bottom tiles (y 0..8)
    fill(8, 0, 8, 2, hair);  // a little hair across the head-top tile
    // Body base (16,16)-(40,32): shirt.
    fill(16, 16, 24, 16, shirt);
    // Arms: right arm block (40,16), left arm block (32,48): skin.
    fill(40, 16, 16, 16, skin);
    fill(32, 48, 16, 16, skin);
    // Legs: right leg block (0,16), left leg block (16,48): trousers.
    fill(0, 16, 16, 16, trousers);
    fill(16, 48, 16, 16, trousers);
    // Eyes on the head FRONT tile (-Z face, atlas px (8,8)-(16,16)): two dark pixels.
    fill(10, 12, 1, 1, eye);
    fill(13, 12, 1, 1, eye);

    px
}

/// Per-player tinted variants of the default skin so split-screen players are
/// visually distinct. `variant 0` = the untouched default; others multiply RGB
/// by a distinct hue (shading + alpha preserved). Cycles through 8 tints.
pub fn tinted_default_skin_rgba(variant: u32) -> Vec<u8> {
    let mut px = default_skin_rgba();
    // [white(=identity), red, green, blue, yellow, magenta, cyan, orange]
    const TINTS: [[u32; 3]; 8] = [
        [255, 255, 255], [255, 140, 140], [150, 230, 150], [150, 180, 255],
        [240, 230, 140], [240, 160, 240], [150, 230, 230], [250, 190, 120],
    ];
    let t = TINTS[(variant as usize) % TINTS.len()];
    for c in px.chunks_mut(4) {
        c[0] = ((c[0] as u32 * t[0]) / 255) as u8;
        c[1] = ((c[1] as u32 * t[1]) / 255) as u8;
        c[2] = ((c[2] as u32 * t[2]) / 255) as u8;
        // c[3] (alpha) untouched
    }
    px
}

/// Wandering Villager head (front) — purple hood + face.
fn gen_wandering_head_front() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let (mut r, mut g, mut b) = (210, 175, 145);
            // Purple hood wraps the top + sides.
            if y < 4 || !(2..=13).contains(&x) {
                (r, g, b) = (80, 50, 115);
            }
            // Eyes.
            if (y == 7 || y == 8) && ((x == 4 || x == 5) || (x == 10 || x == 11)) {
                (r, g, b) = (35, 25, 15);
            }
            // Nose.
            if (8..=12).contains(&y) && (x == 7 || x == 8) {
                (r, g, b) = (175, 135, 105);
            }
            // Mouth.
            if y == 13 && (6..=9).contains(&x) {
                (r, g, b) = (110, 60, 50);
            }
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

/// Wandering Villager head (side).
fn gen_wandering_head_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let (mut r, mut g, mut b) = (210, 175, 145);
            // Hood wraps top + back.
            if y < 4 || (y < 8 && x < 4) {
                (r, g, b) = (80, 50, 115);
            }
            // Eye.
            if (y == 7 || y == 8) && (x == 11 || x == 12) {
                (r, g, b) = (35, 25, 15);
            }
            px[i] = r; px[i+1] = g; px[i+2] = b; px[i+3] = 255;
        }
    }
    px
}

/// Drying Rack top (Wave 29). Three horizontal log-bars laid across the
/// frame, with darker gaps showing where seasoning happens. The eye
/// reads it as "logs stacked on a rack" from above.
fn gen_drying_rack_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    // Frame outline first — a darker wood border so the rack reads as
    // bounded.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 2900);
            let noise = (h % 9) as i32 - 4;
            // Default background — shadowed wood between the bars.
            let mut r = clamp_u8(50 + noise);
            let mut g = clamp_u8(35 + noise);
            let mut b = clamp_u8(22 + noise);
            // Three horizontal log-bars at rows 2-3, 7-8, 12-13.
            let on_bar = matches!(y, 2..=3 | 7..=8 | 12..=13);
            if on_bar {
                r = clamp_u8(115 + noise);
                g = clamp_u8(82 + noise * 7 / 10);
                b = clamp_u8(48 + noise / 2);
            }
            // Frame edges (top/bottom row + left/right column) darker.
            let on_frame = x == 0 || x == SIZE - 1 || y == 0 || y == SIZE - 1;
            if on_frame {
                r = clamp_u8(75 + noise / 2);
                g = clamp_u8(55 + noise / 2);
                b = clamp_u8(35 + noise / 2);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Blueprint Paper top (Spec 24). Parchment-cream face with a faint grid
/// pattern so a row of tiles reads as a drafting board from above.
/// 4-px grid lines slightly darker than the base parchment.
fn gen_blueprint_paper_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 3100);
            let noise = (h % 7) as i32 - 3;
            let mut r = clamp_u8(235 + noise);
            let mut g = clamp_u8(220 + noise);
            let mut b = clamp_u8(170 + noise);
            // Faint grid every 4 pixels — drafting-board feel.
            if x % 4 == 0 || y % 4 == 0 {
                r = clamp_u8(r as i32 - 20);
                g = clamp_u8(g as i32 - 20);
                b = clamp_u8(b as i32 - 15);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Blueprint Paper side (Spec 24). Dark walnut border with a thin
/// parchment-edge line at the top so the tile reads as a flat
/// drafting board lying on the ground.
fn gen_blueprint_paper_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 3101);
            let noise = (h % 9) as i32 - 4;
            let mut r = clamp_u8(70 + noise);
            let mut g = clamp_u8(48 + noise);
            let mut b = clamp_u8(30 + noise);
            // Thin parchment edge on the top row so the side
            // suggests "drafting board lying on the ground".
            if y == 0 {
                r = clamp_u8(230 + noise);
                g = clamp_u8(215 + noise);
                b = clamp_u8(165 + noise);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Construction Anchor (Spec 24). Small red surveyor's flag silhouette
/// on a shadow-grey background. Single texture used for every face —
/// the marker reads from any angle.
fn gen_construction_anchor() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 3110);
            let noise = (h % 7) as i32 - 3;
            // Shadow background.
            let mut r = clamp_u8(55 + noise);
            let mut g = clamp_u8(45 + noise);
            let mut b = clamp_u8(38 + noise);
            // Vertical pole — column 7 from y=2 to y=14.
            if x == 7 && (2..=14).contains(&y) {
                r = clamp_u8(140 + noise);
                g = clamp_u8(100 + noise);
                b = clamp_u8(60 + noise);
            }
            // Flag triangle — pixels from (8, 2) to (12, 5).
            let in_flag = (8..=12).contains(&x) && (2..=5).contains(&y)
                && (x - 8) <= (5 - y) + 4;
            if in_flag {
                r = clamp_u8(220 + noise);
                g = clamp_u8(60 + noise);
                b = clamp_u8(60 + noise);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Architect's Plaque top (Spec 24). Parchment face with a small grid
/// hint — suggests "this contains a structured drawing" without being
/// busy. Slightly warmer than Blueprint Paper so the two are visually
/// distinguishable side-by-side.
fn gen_architect_plaque_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 3120);
            let noise = (h % 7) as i32 - 3;
            let mut r = clamp_u8(240 + noise);
            let mut g = clamp_u8(215 + noise);
            let mut b = clamp_u8(155 + noise);
            // Centred "drawing" — three darker squares in a 3x3
            // arrangement at rows 5-10, cols 5-10.
            if (5..=10).contains(&x) && (5..=10).contains(&y)
                && (x + y) % 2 == 0
            {
                r = clamp_u8(r as i32 - 60);
                g = clamp_u8(g as i32 - 50);
                b = clamp_u8(b as i32 - 35);
            }
            // Wood-frame edge — outer ring.
            if x == 0 || x == SIZE - 1 || y == 0 || y == SIZE - 1 {
                r = clamp_u8(100 + noise);
                g = clamp_u8(70 + noise);
                b = clamp_u8(40 + noise);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Architect's Plaque side (Spec 24). Plain wood frame texture with
/// a thin parchment band at the centre so the sides hint at the
/// plaque's content without being noisy.
fn gen_architect_plaque_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 3121);
            let noise = (h % 9) as i32 - 4;
            let mut r = clamp_u8(95 + noise);
            let mut g = clamp_u8(65 + noise);
            let mut b = clamp_u8(38 + noise);
            // Parchment band at rows 6-9.
            if (6..=9).contains(&y) {
                r = clamp_u8(230 + noise);
                g = clamp_u8(208 + noise);
                b = clamp_u8(150 + noise);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Drying Rack side (Wave 29). Vertical post-and-rail frame: two
/// uprights at columns 1-2 and 13-14, two horizontal rails connecting
/// them, open gap-spaces between so the rack reads as airy / breathable.
fn gen_drying_rack_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 2901);
            let noise = (h % 9) as i32 - 4;
            // Default: shadow between frame members.
            let mut r = clamp_u8(45 + noise);
            let mut g = clamp_u8(30 + noise);
            let mut b = clamp_u8(20 + noise);
            // Vertical uprights at columns 1-2 and 13-14.
            let on_upright = matches!(x, 1..=2 | 13..=14);
            // Horizontal rails at rows 4-5 and 11-12.
            let on_rail = matches!(y, 4..=5 | 11..=12);
            if on_upright || on_rail {
                r = clamp_u8(110 + noise);
                g = clamp_u8(78 + noise * 7 / 10);
                b = clamp_u8(46 + noise / 2);
            }
            // Frame outline.
            let on_frame = x == 0 || x == SIZE - 1 || y == 0 || y == SIZE - 1;
            if on_frame {
                r = clamp_u8(72 + noise / 2);
                g = clamp_u8(52 + noise / 2);
                b = clamp_u8(32 + noise / 2);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Furnace top (Spec 20). Iron-banded stone slab — cobblestone base
/// with a darker iron band around the perimeter, and a thin centre
/// channel hinting at the chimney. Same texture for top + bottom.
fn gen_furnace_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 3200);
            let noise = (h % 11) as i32 - 5;
            // Cobblestone-grey base.
            let mut r = clamp_u8(115 + noise);
            let mut g = clamp_u8(112 + noise);
            let mut b = clamp_u8(108 + noise);
            // Iron band — outer 2-pixel ring darker / cooler.
            let on_band = x <= 1 || x >= SIZE - 2 || y <= 1 || y >= SIZE - 2;
            if on_band {
                r = clamp_u8(70 + noise / 2);
                g = clamp_u8(72 + noise / 2);
                b = clamp_u8(78 + noise / 2);
            }
            // Centre chimney channel — 2x2 darker pixels at the centre.
            if (7..=8).contains(&x) && (7..=8).contains(&y) {
                r = clamp_u8(45 + noise / 2);
                g = clamp_u8(40 + noise / 2);
                b = clamp_u8(38 + noise / 2);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Furnace side unlit (Spec 20). Cobblestone with a dark hollow
/// rectangle in the middle — the closed chamber. Reads as "this is
/// a workstation block with an opening you'll feed materials into".
fn gen_furnace_side_unlit() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 3201);
            let noise = (h % 11) as i32 - 5;
            // Cobblestone base.
            let mut r = clamp_u8(115 + noise);
            let mut g = clamp_u8(112 + noise);
            let mut b = clamp_u8(108 + noise);
            // Iron-band frame (matches top).
            let on_band = x <= 1 || x >= SIZE - 2 || y <= 1 || y >= SIZE - 2;
            if on_band {
                r = clamp_u8(70 + noise / 2);
                g = clamp_u8(72 + noise / 2);
                b = clamp_u8(78 + noise / 2);
            }
            // Chamber opening — dark hollow rectangle at rows 5-12, cols 4-11.
            let in_chamber = (4..=11).contains(&x) && (5..=12).contains(&y);
            if in_chamber {
                r = clamp_u8(28 + noise / 2);
                g = clamp_u8(24 + noise / 2);
                b = clamp_u8(22 + noise / 2);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Furnace side lit (Spec 20). Same as unlit but the chamber glows
/// orange. Strong contrast vs the cool grey frame so the player can
/// tell at a glance which furnaces are working.
fn gen_furnace_side_lit() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 3202);
            let noise = (h % 11) as i32 - 5;
            let mut r = clamp_u8(115 + noise);
            let mut g = clamp_u8(112 + noise);
            let mut b = clamp_u8(108 + noise);
            let on_band = x <= 1 || x >= SIZE - 2 || y <= 1 || y >= SIZE - 2;
            if on_band {
                r = clamp_u8(70 + noise / 2);
                g = clamp_u8(72 + noise / 2);
                b = clamp_u8(78 + noise / 2);
            }
            let in_chamber = (4..=11).contains(&x) && (5..=12).contains(&y);
            if in_chamber {
                // Orange-yellow glow, slightly brighter in centre.
                let cx = 7i32; let cy = 8i32;
                let d = ((x as i32) - cx).abs() + ((y as i32) - cy).abs();
                let glow = (8 - d.min(8)) * 8;
                r = clamp_u8(180 + glow + noise);
                g = clamp_u8(90 + glow / 2 + noise);
                b = clamp_u8(30 + noise / 2);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Spec 16 Phase 3b — deepslate THIN visual variant. Same dark-grey
/// substrate as PURE_DEEPSLATE plus a sparse blue speckle (Reserve-fat
/// hint at low density). Reads as "there's something here but it's
/// thin" — the lowest of the three richness tiers.
fn gen_deepslate_thin() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_deepslate_base(&mut px, 4101);
    // Subtle blue specks at ~1 in 30 pixels.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = pixel_hash(x, y, 4150);
            if h.is_multiple_of(30) {
                let i = ((y * SIZE + x) * 4) as usize;
                let v = 50 + (h % 20) as u8;
                px[i] = v.saturating_sub(20);     // slight blue tint
                px[i + 1] = v.saturating_sub(10);
                px[i + 2] = v.saturating_add(45);
            }
        }
    }
    px
}

/// Deepslate HEALTHY — denser specks plus a warmer secondary vein
/// (Satori-orange-adjacent palette per the spec). Visible from a
/// few blocks away; reads as "the Reserve has substance".
fn gen_deepslate_healthy() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_deepslate_base(&mut px, 4101);
    // Blue veining ~1 in 14 pixels.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = pixel_hash(x, y, 4151);
            if h.is_multiple_of(14) {
                let i = ((y * SIZE + x) * 4) as usize;
                let v = 70 + (h % 20) as u8;
                px[i] = v.saturating_sub(30);
                px[i + 1] = v.saturating_sub(5);
                px[i + 2] = v.saturating_add(50);
            }
            // Orange warm-veins ~1 in 40 pixels.
            let h2 = pixel_hash(x, y, 4152);
            if h2.is_multiple_of(40) {
                let i = ((y * SIZE + x) * 4) as usize;
                px[i] = 140;
                px[i + 1] = 80;
                px[i + 2] = 35;
            }
        }
    }
    px
}

/// Deepslate FAT — dense glowing veins; the rock looks *alive* with
/// potential. The top tier — chunk-gen places these when richness is
/// near 1.0. Spec 16 §"What the alpha gets" calls this the
/// "rock-is-full-of-sats" beat for the kid player.
fn gen_deepslate_fat() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    fill_deepslate_base(&mut px, 4101);
    // Dense blue glow ~1 in 7 pixels.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = pixel_hash(x, y, 4153);
            if h.is_multiple_of(7) {
                let i = ((y * SIZE + x) * 4) as usize;
                let v = 95 + (h % 30) as u8;
                px[i] = v.saturating_sub(40);
                px[i + 1] = v.saturating_sub(5);
                px[i + 2] = v.saturating_add(75);
            }
            // Orange warmth ~1 in 18 pixels — more frequent than HEALTHY.
            let h2 = pixel_hash(x, y, 4154);
            if h2.is_multiple_of(18) {
                let i = ((y * SIZE + x) * 4) as usize;
                px[i] = 195;
                px[i + 1] = 115;
                px[i + 2] = 55;
            }
        }
    }
    px
}

/// Spec 21 Vendor Block — chest-style wood base with an iron coin-slot
/// motif on the front face. Wood grain runs vertically; the coin slot
/// is a horizontal grey rectangle slightly above centre with a thin
/// dark line through the middle (the slit).
fn gen_vendor_block() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 3300);
            let noise = (h % 11) as i32 - 5;
            // Wood-base — warm oak-plank-ish.
            let mut r = clamp_u8(155 + noise);
            let mut g = clamp_u8(120 + noise);
            let mut b = clamp_u8(75 + noise);
            // Vertical grain — every 3rd column slightly darker.
            if x % 3 == 0 {
                r = clamp_u8(r as i32 - 25);
                g = clamp_u8(g as i32 - 20);
                b = clamp_u8(b as i32 - 12);
            }
            // Frame outline.
            if x == 0 || x == SIZE - 1 || y == 0 || y == SIZE - 1 {
                r = clamp_u8(75 + noise / 2);
                g = clamp_u8(55 + noise / 2);
                b = clamp_u8(35 + noise / 2);
            }
            // Coin-slot rectangle — rows 6-7, cols 4-11. Iron-grey.
            if (4..=11).contains(&x) && (6..=7).contains(&y) {
                r = clamp_u8(160 + noise);
                g = clamp_u8(160 + noise);
                b = clamp_u8(165 + noise);
            }
            // Slot slit — single dark row across the middle of the slot.
            if (5..=10).contains(&x) && y == 6 {
                r = clamp_u8(40 + noise / 2);
                g = clamp_u8(40 + noise / 2);
                b = clamp_u8(45 + noise / 2);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// HP-2 — Chest top. Plank-wood base with a horizontal lid line across
/// the middle and a small iron clasp band the side texture aligns
/// against.
fn gen_chest_top() -> Vec<u8> {
    gen_chest_top_tinted(140, 95, 50)
}

/// #15 — tinted chest lid. `(br,bg,bb)` is the plank base; edges/lid-join derive
/// from it, the iron clasp stays grey. Wood passes (140,95,50); tiers tint.
fn gen_chest_top_tinted(br: i32, bg: i32, bb: i32) -> Vec<u8> {
    let half = |c: i32| c / 2;
    let dark = |c: i32| c * 2 / 5;
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 3500);
            let noise = (h % 11) as i32 - 5;
            let mut r = clamp_u8(br + noise);
            let mut g = clamp_u8(bg + noise);
            let mut b = clamp_u8(bb + noise);
            if y % 4 == 0 {
                r = clamp_u8(r as i32 - 22);
                g = clamp_u8(g as i32 - 18);
                b = clamp_u8(b as i32 - 10);
            }
            if x == 0 || x == SIZE - 1 || y == 0 || y == SIZE - 1 {
                r = clamp_u8(half(br) + noise / 2);
                g = clamp_u8(half(bg) + noise / 2);
                b = clamp_u8(half(bb) + noise / 2);
            }
            if y == SIZE / 2 || y == SIZE / 2 - 1 {
                r = clamp_u8(dark(br) + noise / 2);
                g = clamp_u8(dark(bg) + noise / 2);
                b = clamp_u8(dark(bb) + noise / 2);
            }
            if (6..=9).contains(&x) && (y == SIZE / 2 - 2 || y == SIZE / 2 + 1) {
                r = clamp_u8(165 + noise);
                g = clamp_u8(165 + noise);
                b = clamp_u8(170 + noise);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// HP-2 — Chest side. Vertical plank-wood grain with an iron clasp band
/// across the lid join + a small keyhole in the centre of the front
/// face. Single side texture across all four sides on alpha; per-face
/// front-vs-back distinction is post-MVP polish.
fn gen_chest_side() -> Vec<u8> {
    gen_chest_side_tinted(135, 92, 48)
}

/// #15 — tinted chest side. `(br,bg,bb)` is the plank base; edges derive from
/// it, the iron clasp band + keyhole stay metal/dark. Wood passes (135,92,48).
fn gen_chest_side_tinted(br: i32, bg: i32, bb: i32) -> Vec<u8> {
    let half = |c: i32| c / 2;
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 3600);
            let noise = (h % 11) as i32 - 5;
            let mut r = clamp_u8(br + noise);
            let mut g = clamp_u8(bg + noise);
            let mut b = clamp_u8(bb + noise);
            if x % 4 == 0 {
                r = clamp_u8(r as i32 - 22);
                g = clamp_u8(g as i32 - 18);
                b = clamp_u8(b as i32 - 10);
            }
            if x == 0 || x == SIZE - 1 || y == 0 || y == SIZE - 1 {
                r = clamp_u8(half(br) + noise / 2);
                g = clamp_u8(half(bg) + noise / 2);
                b = clamp_u8(half(bb) + noise / 2);
            }
            if y == SIZE / 2 - 2 || y == SIZE / 2 - 1 {
                r = clamp_u8(160 + noise);
                g = clamp_u8(160 + noise);
                b = clamp_u8(165 + noise);
            }
            if (7..=8).contains(&x) && (y == SIZE / 2 || y == SIZE / 2 + 1) {
                r = clamp_u8(35 + noise / 2);
                g = clamp_u8(28 + noise / 2);
                b = clamp_u8(20 + noise / 2);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// HP-3 — Brigand Hideout Banner. Central wooden post with a tattered
/// purple-and-red banner. Reads as "this is a bandit camp" from a
/// distance. Decorative; transparent at the corners so the model
/// renders as a banner rather than a solid cube.
fn gen_brigand_hideout_banner() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 3800);
            let noise = (h % 9) as i32 - 4;
            // Default to fully transparent.
            let mut r = 0u8;
            let mut g = 0u8;
            let mut b = 0u8;
            let mut a = 0u8;
            // Central wooden post (2-wide vertical strip).
            if (x == SIZE / 2 || x == SIZE / 2 - 1) && y >= 2 {
                r = clamp_u8(75 + noise);
                g = clamp_u8(45 + noise);
                b = clamp_u8(22 + noise);
                a = 255;
            }
            // Banner — covers x ∈ [3, 13), y ∈ [4, 12). Tattered bottom
            // edge: skip a few cells to suggest fraying.
            let in_banner_x = (3..13).contains(&x);
            let in_banner_y = (4..12).contains(&y);
            if in_banner_x && in_banner_y {
                // Split into two horizontal halves — top purple, bottom red.
                let dark = (h % 18) as i32 - 9;
                if y < 8 {
                    r = clamp_u8(110 + dark);
                    g = clamp_u8(40 + dark / 2);
                    b = clamp_u8(110 + dark);
                } else {
                    r = clamp_u8(135 + dark);
                    g = clamp_u8(40 + dark / 2);
                    b = clamp_u8(45 + dark / 2);
                }
                // Tatter holes near the bottom edge (y == 11).
                if y == 11 && (h >> 4).is_multiple_of(3) {
                    a = 0;
                } else {
                    a = 255;
                }
            }
            set_px(&mut px, x, y, r, g, b, a);
        }
    }
    px
}

/// HP-3 v2 — Trophy Wall. Wood-grain plaque (oak-plank base) with a
/// stylised purple-and-red Chieftain trophy mounted at the centre.
/// Reads as "I killed a Berserker" from across the room.
fn gen_trophy_wall() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 3900);
            let noise = (h % 9) as i32 - 4;
            // Oak-plank base — horizontal-grain feel.
            let mut r = clamp_u8(150 + noise);
            let mut g = clamp_u8(110 + noise);
            let mut b = clamp_u8(70 + noise);
            if y % 4 == 0 {
                r = clamp_u8(r as i32 - 22);
                g = clamp_u8(g as i32 - 18);
                b = clamp_u8(b as i32 - 12);
            }
            if x == 0 || x == SIZE - 1 || y == 0 || y == SIZE - 1 {
                r = clamp_u8(72 + noise / 2);
                g = clamp_u8(48 + noise / 2);
                b = clamp_u8(30 + noise / 2);
            }
            // Trophy motif: centred 6×8 plaque area, purple top half,
            // red bottom half (mirrors BrigandChieftainTrophy palette).
            let in_trophy_x = (5..11).contains(&x);
            let in_trophy_y = (4..12).contains(&y);
            if in_trophy_x && in_trophy_y {
                let dark = (h % 18) as i32 - 9;
                if y < 8 {
                    r = clamp_u8(115 + dark);
                    g = clamp_u8(45 + dark / 2);
                    b = clamp_u8(115 + dark);
                } else {
                    r = clamp_u8(125 + dark);
                    g = clamp_u8(35 + dark / 2);
                    b = clamp_u8(40 + dark / 2);
                }
                // Iron mounting pegs: top-left + top-right corners.
                if (x == 5 || x == 10) && y == 4 {
                    r = 170; g = 170; b = 175;
                }
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Spec 26 Drafting Table — parchment top with a faint blue grid + a
/// dark wood frame. Reads as "this is where plans get drawn" without
/// being noisy.
fn gen_drafting_table() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 3400);
            let noise = (h % 9) as i32 - 4;
            let mut r = clamp_u8(225 + noise);
            let mut g = clamp_u8(210 + noise);
            let mut b = clamp_u8(160 + noise);
            if x % 4 == 0 || y % 4 == 0 {
                r = clamp_u8(r as i32 - 30);
                g = clamp_u8(g as i32 - 20);
                b = clamp_u8(b as i32 - 5);
            }
            if x == 0 || x == SIZE - 1 || y == 0 || y == SIZE - 1 {
                r = clamp_u8(95 + noise / 2);
                g = clamp_u8(65 + noise / 2);
                b = clamp_u8(38 + noise / 2);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Salt feature — ROCK_SALT ore. Pale stone base with white-pink
/// crystalline veining. Reads as "salt deposit" on first sight.
fn gen_rock_salt() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 4000);
            let noise = (h % 11) as i32 - 5;
            let mut r = clamp_u8(195 + noise);
            let mut g = clamp_u8(185 + noise);
            let mut b = clamp_u8(180 + noise);
            // Diagonal salt veins — pink-white streaks.
            let band = ((x + y) % 5) as i32;
            if band == 0 || band == 1 {
                let v = (px_hash(x, y, 4001) % 10) as i32 - 5;
                r = clamp_u8(240 + v);
                g = clamp_u8(225 + v);
                b = clamp_u8(225 + v);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Salt feature — SALT_LICK. Chunky off-white block with greenish-
/// brown mineral inclusions. Reads as the "cattle mineral block"
/// reference.
fn gen_salt_lick() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 4100);
            let noise = (h % 9) as i32 - 4;
            let mut r = clamp_u8(225 + noise);
            let mut g = clamp_u8(218 + noise);
            let mut b = clamp_u8(200 + noise);
            // Mineral flecks — scattered olive + brown at low percentages.
            let fleck = px_hash(x, y, 4101) % 100;
            if fleck < 6 {
                r = clamp_u8(120 + noise);
                g = clamp_u8(95 + noise);
                b = clamp_u8(45 + noise);
            } else if fleck < 12 {
                r = clamp_u8(95 + noise);
                g = clamp_u8(100 + noise);
                b = clamp_u8(65 + noise);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Salt feature — SALT_LAMP. Amber/pink crystal cluster with a faint
/// painted glow halo. No shader work; the texture sells the warm feel.
fn gen_salt_lamp() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    let cx = SIZE as i32 / 2;
    let cy = SIZE as i32 / 2;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 4200);
            let noise = (h % 11) as i32 - 5;
            let dx = x as i32 - cx;
            let dy = y as i32 - cy;
            let dist2 = (dx * dx + dy * dy) as f32;
            let radial = (dist2.sqrt() / (SIZE as f32 / 2.0)).min(1.0);
            // Warm amber centre fading to dim orange at edges.
            let r_base = 245.0 - 60.0 * radial;
            let g_base = 175.0 - 65.0 * radial;
            let b_base = 90.0 - 50.0 * radial;
            let mut r = clamp_u8(r_base as i32 + noise);
            let mut g = clamp_u8(g_base as i32 + noise);
            let mut b = clamp_u8(b_base as i32 + noise);
            // Crystal facets — diagonal highlights brighter than base.
            if ((x + y) % 4) == 0 && radial < 0.7 {
                r = clamp_u8(r as i32 + 20);
                g = clamp_u8(g as i32 + 15);
                b = clamp_u8(b as i32 + 10);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Salt feature — SALT_BLOCK. Solid white-pink storage block, fine
/// crystal grain. Distinguishable from SALT_LICK by uniform colour
/// (no mineral inclusions).
fn gen_salt_block() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 4300);
            let noise = (h % 9) as i32 - 4;
            let r = clamp_u8(245 + noise);
            let g = clamp_u8(232 + noise);
            let b = clamp_u8(230 + noise);
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Salt feature — SALT_PATH (top face). Dirt base with heavy white
/// salt-crystal sprinkle.
fn gen_salt_path_top() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 4400);
            let noise = (h % 9) as i32 - 4;
            let mut r = clamp_u8(125 + noise);
            let mut g = clamp_u8(95 + noise);
            let mut b = clamp_u8(60 + noise);
            // Salt sprinkle — ~35 % of pixels bright white.
            if (px_hash(x, y, 4401) % 100) < 35 {
                r = clamp_u8(240 + noise);
                g = clamp_u8(232 + noise);
                b = clamp_u8(225 + noise);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Salt feature — SALT_PATH (side face). Dirt base with a lighter
/// sprinkle on the upper quarter only — salt sits on top of the path.
fn gen_salt_path_side() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 4500);
            let noise = (h % 9) as i32 - 4;
            let mut r = clamp_u8(125 + noise);
            let mut g = clamp_u8(95 + noise);
            let mut b = clamp_u8(60 + noise);
            // Sprinkle on the top quarter only.
            if y < SIZE / 4 && (px_hash(x, y, 4501) % 100) < 25 {
                r = clamp_u8(235 + noise);
                g = clamp_u8(228 + noise);
                b = clamp_u8(220 + noise);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Rubber feature — RUBBER_LOG. Pale amber bark with subtle vertical
/// grain. Hevea brasiliensis colouring.
fn gen_rubber_log() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 5000);
            let noise = (h % 11) as i32 - 5;
            let mut r = clamp_u8(195 + noise);
            let mut g = clamp_u8(170 + noise);
            let mut b = clamp_u8(125 + noise);
            // Vertical grain — darker streaks every 3 columns.
            if x % 3 == 0 {
                r = clamp_u8(r as i32 - 28);
                g = clamp_u8(g as i32 - 24);
                b = clamp_u8(b as i32 - 18);
            }
            // Top + bottom ring darker for the end-grain feel.
            if y == 0 || y == SIZE - 1 {
                r = clamp_u8(150 + noise / 2);
                g = clamp_u8(130 + noise / 2);
                b = clamp_u8(95 + noise / 2);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Rubber feature — RUBBER_PLANKS. Warm pale-amber plank with the
/// horizontal-grain rhythm matching other species' plank textures.
fn gen_rubber_planks() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 5100);
            let noise = (h % 9) as i32 - 4;
            let mut r = clamp_u8(218 + noise);
            let mut g = clamp_u8(190 + noise);
            let mut b = clamp_u8(140 + noise);
            // Horizontal plank boundaries every 4 rows.
            if y % 4 == 0 {
                r = clamp_u8(r as i32 - 32);
                g = clamp_u8(g as i32 - 26);
                b = clamp_u8(b as i32 - 18);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Rubber feature — RUBBER_LEAVES. Slightly darker green than oak,
/// hinting at the broad lobed leaves of a Hevea.
fn gen_rubber_leaves() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 5200);
            let noise = (h % 13) as i32 - 6;
            let mut r = clamp_u8(75 + noise);
            let mut g = clamp_u8(130 + noise);
            let mut b = clamp_u8(60 + noise);
            // 15% lighter speckle.
            if (px_hash(x, y, 5201) % 100) < 15 {
                r = clamp_u8(r as i32 + 28);
                g = clamp_u8(g as i32 + 30);
                b = clamp_u8(b as i32 + 22);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Rubber feature — RUBBER_LOG_TAPPED side face. RUBBER_LOG base with
/// a small iron-grey spout + dark cup overlay near the centre-bottom.
/// Other faces use plain RUBBER_LOG (via BlockDef tex_top/tex_bottom).
fn gen_rubber_log_tapped() -> Vec<u8> {
    let mut px = vec![0u8; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 5300);
            let noise = (h % 11) as i32 - 5;
            let mut r = clamp_u8(195 + noise);
            let mut g = clamp_u8(170 + noise);
            let mut b = clamp_u8(125 + noise);
            if x % 3 == 0 {
                r = clamp_u8(r as i32 - 28);
                g = clamp_u8(g as i32 - 24);
                b = clamp_u8(b as i32 - 18);
            }
            if y == 0 || y == SIZE - 1 {
                r = clamp_u8(150 + noise / 2);
                g = clamp_u8(130 + noise / 2);
                b = clamp_u8(95 + noise / 2);
            }
            // Spout — small diagonal cut at (4..=8, 8..=10).
            let xs = x as i32;
            let ys = y as i32;
            let on_spout = (xs == 5 || xs == 4) && ys == 8
                || (xs == 6 && ys == 9)
                || (xs == 7 && ys == 10)
                || (xs == 8 && ys == 10);
            if on_spout {
                r = 110;
                g = 110;
                b = 115;
            }
            // Cup beneath spout — dark crescent.
            let in_cup = (6..=9).contains(&xs) && (11..=13).contains(&ys);
            if in_cup {
                r = clamp_u8(50 + noise / 3);
                g = clamp_u8(40 + noise / 3);
                b = clamp_u8(30 + noise / 3);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Mob Bounty Board — cork-board base + two parchment notes pinned.
/// Spec 33 (2026-05-23).
fn gen_bounty_board() -> Vec<u8> {
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            // Cork base — warm brown with grain noise.
            let h = px_hash(x, y, 0xB07D);
            let noise = (h & 0x1F) as i32 - 16;
            let mut r = clamp_u8(140 + noise);
            let mut g = clamp_u8(95 + noise);
            let mut b = clamp_u8(55 + noise);

            // Two parchment rectangles — top-left + bottom-right.
            let in_note1 = (2..=7).contains(&x) && (2..=6).contains(&y);
            let in_note2 = (8..=13).contains(&x) && (9..=13).contains(&y);
            if in_note1 || in_note2 {
                // Cream parchment with subtle off-white grain.
                let pn = (px_hash(x, y, 0xC0FE) & 0x0F) as i32 - 8;
                r = clamp_u8(230 + pn);
                g = clamp_u8(218 + pn);
                b = clamp_u8(180 + pn);
                // Faint horizontal "text" lines every other row.
                if (y % 2 == 0) && x > if in_note1 { 3 } else { 9 } && x < if in_note1 { 7 } else { 13 } {
                    r = clamp_u8(r as i32 - 50);
                    g = clamp_u8(g as i32 - 50);
                    b = clamp_u8(b as i32 - 50);
                }
            }
            // Pin dots at the top-centre of each note.
            let is_pin1 = x == 4 && y == 2;
            let is_pin2 = x == 10 && y == 9;
            if is_pin1 || is_pin2 {
                r = 200; g = 30; b = 30;
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Tip Jar — central jar silhouette with gold rim + a coin slot at
/// top. Spec 34 (2026-05-23). Background is warm wood; jar body is
/// muted cream; rim + coin slot pop in saturated gold.
fn gen_tip_jar() -> Vec<u8> {
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 0x71_AB);
            let noise = (h & 0x1F) as i32 - 16;
            let mut r = clamp_u8(120 + noise);
            let mut g = clamp_u8(82 + noise);
            let mut b = clamp_u8(48 + noise);

            // Jar body 12×10, corners dropped.
            let in_jar_body = (2..=13).contains(&x) && (4..=13).contains(&y);
            let in_jar = in_jar_body
                && !((x == 2 || x == 13) && (y == 4 || y == 13));
            if in_jar {
                let n = (px_hash(x, y, 0xCAFE) & 0x0F) as i32 - 8;
                r = clamp_u8(232 + n);
                g = clamp_u8(220 + n);
                b = clamp_u8(180 + n);
            }
            // Gold rim — top row of jar body.
            if in_jar && y == 4 {
                r = 220; g = 175; b = 30;
            }
            // Coin slot — two dark pixels just above the rim.
            if (x == 7 || x == 8) && y == 3 {
                r = 40; g = 30; b = 10;
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Repair Bench — dark worn-metal block with a lighter scuffed top
/// band + four rivet dots. Spec 35 (2026-05-23).
fn gen_repair_bench() -> Vec<u8> {
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 0x5EA1);
            let noise = (h & 0x1F) as i32 - 16;
            // Dark iron base.
            let mut r = clamp_u8(78 + noise);
            let mut g = clamp_u8(80 + noise);
            let mut b = clamp_u8(90 + noise);
            // Lighter scuffed work-surface band across the top third.
            if y < 5 {
                r = clamp_u8(120 + noise);
                g = clamp_u8(122 + noise);
                b = clamp_u8(130 + noise);
            }
            // Rivet dots at the four "corners" of the face.
            if (x == 2 || x == 13) && (y == 7 || y == 13) {
                r = 40; g = 40; b = 46;
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Plot Marker — a hazard-striped survey stake: a vertical post with
/// diagonal yellow/black warning bands. Spec 36 (2026-05-23).
fn gen_plot_marker() -> Vec<u8> {
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 0x9107);
            let noise = (h & 0x0F) as i32 - 8;
            // Grass-ish ground backdrop.
            let mut r = clamp_u8(72 + noise);
            let mut g = clamp_u8(100 + noise);
            let mut b = clamp_u8(54 + noise);
            // Central post (x ∈ [6, 9]).
            if (6..=9).contains(&x) {
                // Diagonal hazard stripes: alternate yellow / black on
                // (x + y) bands.
                if ((x + y) / 2) % 2 == 0 {
                    r = clamp_u8(220 + noise); g = clamp_u8(180 + noise); b = 20;
                } else {
                    r = 30; g = 30; b = 30;
                }
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Market Bell — a brass bell silhouette: a rounded dome with a
/// flared lip + a clapper dot, on a dark post. Distinct from the
/// Village Bell (which is rendered as its own block model).
/// Spec 37 (2026-05-23).
fn gen_market_bell() -> Vec<u8> {
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];
    let cx = 8i32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 0xBE11);
            let noise = (h & 0x0F) as i32 - 8;
            // Dark wood backdrop.
            let mut r = clamp_u8(70 + noise);
            let mut g = clamp_u8(52 + noise);
            let mut b = clamp_u8(34 + noise);
            let dx = (x as i32 - cx).abs();
            // Bell dome: rows 3..=11, width tapering from ~2 at top to
            // ~6 at the lip.
            let in_dome = (3..=11).contains(&y) && dx <= (2 + (y as i32 - 3) / 2);
            // Flared lip row.
            let in_lip = y == 12 && dx <= 6;
            if in_dome || in_lip {
                r = clamp_u8(210 + noise); // brass
                g = clamp_u8(160 + noise);
                b = clamp_u8(55 + noise);
            }
            // Clapper dot just under the lip.
            if y == 13 && dx == 0 {
                r = 40; g = 30; b = 15;
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Auction Block — a polished wood podium with an iron-banded top
/// edge + a small gavel mark. Spec 38 (2026-05-23).
fn gen_auction_block() -> Vec<u8> {
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 0xAC10);
            let noise = (h & 0x1F) as i32 - 16;
            // Polished wood body.
            let mut r = clamp_u8(150 + noise);
            let mut g = clamp_u8(108 + noise);
            let mut b = clamp_u8(62 + noise);
            // Iron band across the top 2 rows.
            if y < 2 {
                r = clamp_u8(150 + noise);
                g = clamp_u8(150 + noise);
                b = clamp_u8(158 + noise);
            }
            // Vertical plank seams.
            if x % 4 == 0 {
                r = clamp_u8(r as i32 - 30);
                g = clamp_u8(g as i32 - 30);
                b = clamp_u8(b as i32 - 30);
            }
            // Small gavel head (dark) near the centre.
            if (5..=8).contains(&x) && (6..=8).contains(&y) {
                r = clamp_u8(70 + noise);
                g = clamp_u8(48 + noise);
                b = clamp_u8(28 + noise);
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

/// Bazaar Block — a striped market-stall canopy (teal/cream awning)
/// over a wood counter, with a small coin glint. Spec 39 (2026-05-23).
fn gen_bazaar_block() -> Vec<u8> {
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let h = px_hash(x, y, 0xBA2A);
            let noise = (h & 0x1F) as i32 - 16;
            // Lower two-thirds: wood counter.
            let mut r = clamp_u8(140 + noise);
            let mut g = clamp_u8(100 + noise);
            let mut b = clamp_u8(58 + noise);
            // Top third: striped awning (teal / cream by column).
            if y < 6 {
                if (x / 2) % 2 == 0 {
                    r = clamp_u8(45 + noise); g = clamp_u8(140 + noise); b = clamp_u8(120 + noise);
                } else {
                    r = clamp_u8(230 + noise); g = clamp_u8(225 + noise); b = clamp_u8(200 + noise);
                }
            }
            // Coin glint on the counter (gold dot).
            if x == 11 && y == 10 {
                r = 230; g = 190; b = 50;
            }
            set_px(&mut px, x, y, r, g, b, 255);
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallpaper_rgba_matches_known_colours() {
        assert_eq!(wallpaper_rgba(crate::block::WALLPAPER_RED), [199, 46, 41, 255]);
        assert_eq!(wallpaper_rgba(crate::block::WALLPAPER_BLUE), [62, 92, 209, 255]);
        assert_eq!(wallpaper_rgba(crate::block::WALLPAPER_WHITE), [238, 238, 238, 255]);
        assert_eq!(wallpaper_rgba(crate::block::WALLPAPER_MAGENTA), [209, 70, 166, 255]);
    }

    #[test]
    fn wallpaper_rgba_non_wallpaper_is_opaque_fallback() {
        let c = wallpaper_rgba(crate::block::STONE);
        assert_eq!(c[3], 255, "fallback must be opaque");
    }

    #[test]
    fn block_average_rgba_is_opaque_and_plausible() {
        let c = block_average_rgba(crate::block::STONE);
        assert_eq!(c[3], 255);
        assert!(c[0] > 40 && c[0] < 220, "stone avg red {} should be mid", c[0]);
    }

    /// The texture array is created with `texture_count()` layers and then
    /// each generated texture is uploaded by index — the two MUST agree or the
    /// upload writes past the array (panic) or leaves dead layers. Lock it.
    #[test]
    fn generated_count_matches_texture_count() {
        assert_eq!(generate_textures().len(), texture_count() as usize);
    }

    /// DRIFT GUARD (renderer-fix hardening wave): the JS pre-flight gates
    /// hard-code a floor of 512 array layers in FOUR places —
    /// `REQUIRED_TEXTURE_ARRAY_LAYERS` in tools/sites/game/static/webgpu-check.js,
    /// mirrored in tools/sites/marketing/static/webgpu-probe.js and
    /// game/engine/index.dedicated.html, plus the defensive fallback in
    /// tools/sites/game/static/auth.js (the ONLY gate if webgpu-check.js fails
    /// to load, so a stale value there silently reproduces the original crash).
    /// Below the floor, a device is refused the friendly "can't run yet" gate
    /// BEFORE the WASM loads, instead of letting `Renderer::new` hard-panic
    /// trying to allocate the block-texture atlas (`assert_block_texture_layers_fit`).
    /// That floor is only safe while it stays above `texture_count()`. If a
    /// future block-registry wave grows the atlas past 512, this test fails
    /// loudly — bump the floor in ALL FOUR JS sites above and update this
    /// assertion's constant together.
    // --- species-tint run (mob-species-tint foundation, 2026-07-11) ------

    #[test]
    fn species_tint_lookup_maps_donor_faces_into_the_tinted_run() {
        use crate::entity_model::{TEX_SHEEP_BODY_SIDE, TEX_SHEEP_LEG, TEX_SQUID_BODY};
        use crate::mob::MobType;
        // Fox: body_side is the second layer of the first species run.
        assert_eq!(
            tinted_layer(MobType::Fox, TEX_SHEEP_BODY_SIDE),
            Some(PRE_TINT_LAYER_COUNT + 1),
            "fox body_side must map into the tinted run"
        );
        // The leg alias folds onto the tinted body_side (layer-budget:
        // the whole run must stay under the 512 device floor).
        assert_eq!(
            tinted_layer(MobType::Fox, TEX_SHEEP_LEG),
            tinted_layer(MobType::Fox, TEX_SHEEP_BODY_SIDE),
            "leg tints onto body_side"
        );
        assert!(
            tinted_layer(MobType::GlowSquid, TEX_SQUID_BODY).is_some(),
            "glow squid gets its cyan body"
        );
        // Donor species themselves are never remapped.
        assert_eq!(tinted_layer(MobType::Wolf, TEX_SHEEP_BODY_SIDE), None);
        assert_eq!(tinted_layer(MobType::Squid, TEX_SQUID_BODY), None);
    }

    #[test]
    fn tinted_layers_recolour_the_donor_by_normalized_luminance() {
        // The bake: donor grayscale (Rec. 601 luma), normalized so the
        // layer's MEAN luminance hits TINT_TARGET_LUM, then multiplied by the
        // species colour. A straight multiply can never brighten a dark donor
        // (the white polar bear stayed cow-brown); un-normalized luminance
        // keeps the donor's brightness (bear mean 127 vs donor 128). The
        // normalize-then-multiply keeps the fox saturated AND lifts the bear.
        use crate::entity_model::{TEX_COW_BODY_SIDE, TEX_SHEEP_BODY_SIDE};
        use crate::mob::MobType;
        let layers = generate_textures();
        for (kind, donor_layer) in [
            (MobType::Fox, TEX_SHEEP_BODY_SIDE),
            (MobType::PolarBear, TEX_COW_BODY_SIDE),
        ] {
            let donor = &layers[donor_layer as usize];
            let idx = tinted_layer(kind, donor_layer)
                .unwrap_or_else(|| panic!("{kind:?} tinted layer exists")) as usize;
            let tinted = &layers[idx];
            let c = crate::mob::mob_def(kind).color;
            assert_eq!(donor.len(), tinted.len());
            let lum = |p: &[u8]| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
            let mean = donor.chunks(4).map(|p| lum(p)).sum::<f32>() / (donor.len() as f32 / 4.0);
            let scale = TINT_TARGET_LUM / mean;
            for (i, (d, t)) in donor.chunks(4).zip(tinted.chunks(4)).enumerate() {
                for ch in 0..3 {
                    let want = (lum(d) * scale * c[ch]).round().clamp(0.0, 255.0) as u8;
                    assert_eq!(t[ch], want, "{kind:?} pixel {i} channel {ch}");
                }
                assert_eq!(t[3], d[3], "{kind:?} alpha preserved at pixel {i}");
            }
        }
    }

    #[test]
    fn fox_coat_stays_saturated_orange_not_pastel() {
        // Companion property to the bear-brightness test: normalization must
        // not wash the fox out — its coat keeps a clear R > G > B ordering.
        use crate::entity_model::TEX_SHEEP_BODY_SIDE;
        use crate::mob::MobType;
        let layers = generate_textures();
        let fox = &layers[tinted_layer(MobType::Fox, TEX_SHEEP_BODY_SIDE).unwrap() as usize];
        let n = fox.len() as u32 / 4;
        let mean_ch = |ch: usize| fox.chunks(4).map(|p| p[ch] as u32).sum::<u32>() / n;
        let (r, g, b) = (mean_ch(0), mean_ch(1), mean_ch(2));
        assert!(
            r > g + 40 && g > b + 30,
            "fox coat must read orange (R {r} > G {g} > B {b} with margin)"
        );
    }

    #[test]
    fn polar_bear_coat_actually_reads_white_not_donor_brown() {
        // The visual regression the comparison sheet caught: mean brightness
        // of the polar bear's tinted coat must comfortably exceed the brown
        // cow donor's.
        use crate::entity_model::TEX_COW_BODY_SIDE;
        use crate::mob::MobType;
        let layers = generate_textures();
        let mean = |px: &[u8]| {
            px.chunks(4).map(|p| (p[0] as u32 + p[1] as u32 + p[2] as u32) / 3).sum::<u32>()
                / (px.len() as u32 / 4)
        };
        let donor = mean(&layers[TEX_COW_BODY_SIDE as usize]);
        let bear = mean(
            &layers[tinted_layer(MobType::PolarBear, TEX_COW_BODY_SIDE).unwrap() as usize],
        );
        assert!(
            bear > donor + 30,
            "polar bear coat (mean {bear}) must read clearly lighter than the cow donor (mean {donor})"
        );
    }

    #[test]
    fn tinted_registry_keys_line_up_with_the_generated_run() {
        use crate::entity_model::TEX_SHEEP_BODY_SIDE;
        use crate::mob::MobType;
        // Every tinted layer resolves to an `entity/<species>/…` key…
        for idx in PRE_TINT_LAYER_COUNT..POST_TINT_LAYER_START {
            let key = crate::texture_registry::texture_key(idx)
                .unwrap_or_else(|| panic!("no registry key for tinted layer {idx}"));
            assert!(
                key.starts_with("entity/"),
                "tinted layer {idx} has a non-entity key {key:?}"
            );
        }
        // …and the fox body_side spot-check pins the key↔index bijection.
        assert_eq!(
            crate::texture_registry::texture_index("entity/fox/body_side"),
            tinted_layer(MobType::Fox, TEX_SHEEP_BODY_SIDE),
            "registry key and tint lookup must agree on the fox body_side index"
        );
    }

    /// Spec 48 Phase 4 — the two Water Wheel faces are real, distinct, opaque
    /// layers sitting exactly where `block::TEX_WATER_WHEEL*` says they do. If
    /// a future layer is appended without bumping `POST_TINT_LAYER_COUNT`, the
    /// index assertions below move off the wheel and this fails loudly.
    #[test]
    fn water_wheel_layers_sit_at_their_constants_and_differ() {
        assert_eq!(crate::block::TEX_WATER_WHEEL, POST_TINT_LAYER_START);
        assert_eq!(crate::block::TEX_WATER_WHEEL_TURNING, POST_TINT_LAYER_START + 1);
        let layers = generate_textures();
        let idle = &layers[crate::block::TEX_WATER_WHEEL as usize];
        let turning = &layers[crate::block::TEX_WATER_WHEEL_TURNING as usize];
        assert_eq!(idle.len(), PIXELS, "idle wheel is a 16x16 RGBA layer");
        assert_eq!(turning.len(), PIXELS, "turning wheel is a 16x16 RGBA layer");
        assert!(
            idle.chunks(4).all(|p| p[3] == 255),
            "an opaque block face must be fully opaque"
        );
        assert_ne!(idle, turning, "the turning face must read differently from the idle one");
        // …and both resolve through the pack registry by name.
        assert_eq!(
            crate::texture_registry::texture_index("blocks/water_wheel"),
            Some(crate::block::TEX_WATER_WHEEL)
        );
        assert_eq!(
            crate::texture_registry::texture_index("blocks/water_wheel_turning"),
            Some(crate::block::TEX_WATER_WHEEL_TURNING)
        );
    }

    /// Wind, Copper & Electricity wave (2026-09-07), Spec 02 §1 — the Copper
    /// Ore layer is a real, opaque layer sitting exactly where
    /// `block::TEX_COPPER_ORE` says it does, distinct from plain stone and
    /// from the iron-ore layer it shares its `gen_ore` family with.
    #[test]
    fn copper_ore_layer_sits_at_its_constant_and_differs_from_neighbours() {
        assert_eq!(crate::block::TEX_COPPER_ORE, POST_TINT_LAYER_START + 2);
        let layers = generate_textures();
        let copper = &layers[crate::block::TEX_COPPER_ORE as usize];
        assert_eq!(copper.len(), PIXELS, "copper ore is a 16x16 RGBA layer");
        assert!(
            copper.chunks(4).all(|p| p[3] == 255),
            "an opaque block face must be fully opaque"
        );
        let stone = &layers[crate::block::TEX_STONE as usize];
        let iron = &layers[crate::block::TEX_IRON_ORE as usize];
        assert_ne!(copper, stone, "copper ore must read differently from plain stone");
        assert_ne!(copper, iron, "copper ore must read differently from iron ore");
        assert_eq!(
            crate::texture_registry::texture_index("blocks/copper_ore"),
            Some(crate::block::TEX_COPPER_ORE)
        );
    }

    /// Wind, Copper & Electricity wave §2.2 — the two Windmill faces are real,
    /// distinct, opaque layers at their constants, and they must not read as
    /// the Water Wheel (the two sources sit side by side in the recipe book).
    #[test]
    fn windmill_layers_sit_at_their_constants_and_differ() {
        assert_eq!(crate::block::TEX_WINDMILL, POST_TINT_LAYER_START + 3);
        assert_eq!(crate::block::TEX_WINDMILL_TURNING, POST_TINT_LAYER_START + 4);
        let layers = generate_textures();
        let idle = &layers[crate::block::TEX_WINDMILL as usize];
        let turning = &layers[crate::block::TEX_WINDMILL_TURNING as usize];
        assert_eq!(idle.len(), PIXELS, "idle windmill is a 16x16 RGBA layer");
        assert_eq!(turning.len(), PIXELS, "turning windmill is a 16x16 RGBA layer");
        assert!(
            idle.chunks(4).all(|p| p[3] == 255),
            "an opaque block face must be fully opaque"
        );
        assert_ne!(idle, turning, "the turning sails must read differently from the still ones");
        assert_ne!(
            idle, &layers[crate::block::TEX_WATER_WHEEL as usize],
            "the Windmill must not read as the Water Wheel"
        );
        assert_eq!(
            crate::texture_registry::texture_index("blocks/windmill"),
            Some(crate::block::TEX_WINDMILL)
        );
        assert_eq!(
            crate::texture_registry::texture_index("blocks/windmill_turning"),
            Some(crate::block::TEX_WINDMILL_TURNING)
        );
    }

    #[test]
    fn texture_count_stays_under_webgpu_check_floor() {
        const JS_REQUIRED_TEXTURE_ARRAY_LAYERS_FLOOR: u32 = 512;
        assert!(
            texture_count() <= JS_REQUIRED_TEXTURE_ARRAY_LAYERS_FLOOR,
            "texture_count() ({}) has grown past the JS pre-flight gate's floor ({}) — \
             bump the 512 floor in ALL FOUR sites: tools/sites/game/static/webgpu-check.js, \
             tools/sites/marketing/static/webgpu-probe.js, \
             game/engine/index.dedicated.html, and tools/sites/game/static/auth.js \
             (defensive fallback), then raise this constant to match",
            texture_count(),
            JS_REQUIRED_TEXTURE_ARRAY_LAYERS_FLOOR
        );
    }

    /// All ten crack-overlay stages exist, sit at TEX_CRACK_BASE..+10, are the
    /// right size, and carry transparency (a fully-opaque layer would paint a
    /// solid box over the block instead of cracks).
    #[test]
    fn crack_overlay_layers_present_and_transparent() {
        let texs = generate_textures();
        for stage in 0u32..10 {
            let layer = (crate::block::TEX_CRACK_BASE + stage) as usize;
            assert!(layer < texs.len(), "crack stage {stage} layer {layer} out of range");
            assert_eq!(texs[layer].len(), PIXELS, "crack stage {stage} wrong size");
            let has_transparent = texs[layer].chunks_exact(4).any(|p| p[3] == 0);
            assert!(has_transparent, "crack stage {stage} has no transparent pixels");
            // Must actually draw some cracks, or nothing renders.
            let crack_px = texs[layer].chunks_exact(4).filter(|p| p[3] > 0).count();
            assert!(crack_px >= 5, "crack stage {stage} drew too few crack pixels: {crack_px}");
        }
    }

    /// Heavier stages must be visibly denser than the hairline stage, so the
    /// overlay communicates progress rather than looking identical throughout.
    #[test]
    fn crack_density_increases_with_stage() {
        let texs = generate_textures();
        let count_px = |stage: u32| {
            let layer = (crate::block::TEX_CRACK_BASE + stage) as usize;
            texs[layer].chunks_exact(4).filter(|p| p[3] > 0).count()
        };
        assert!(
            count_px(9) > count_px(0),
            "stage 9 ({}) should have more crack pixels than stage 0 ({})",
            count_px(9),
            count_px(0)
        );
    }

    /// Rotating the EW texture back 90° (i.e. 270° CW) reproduces the NS track.
    #[test]
    fn ew_track_is_a_rotation_of_the_ns_track() {
        let ns = super::gen_track(5200);
        let ew = super::gen_track_ew(5200);
        assert_ne!(ns, ew, "EW rail must differ from NS rail");
        assert_eq!(ew.len(), ns.len());
    }
}

#[cfg(test)]
mod skin_tests {
    use super::*;

    #[test]
    fn default_skin_is_64x64_rgba() {
        let px = default_skin_rgba();
        assert_eq!(px.len(), 64 * 64 * 4, "skin must be 64x64 RGBA");
        assert!(px.chunks(4).any(|c| c[3] > 0), "skin must not be fully transparent");
        let idx = (8 * 64 + 8) * 4;
        assert_eq!(px[idx + 3], 255, "head-front base pixel must be opaque");
    }

    /// Cross-check painter <-> UV table: the eyes are drawn on the head's FRONT
    /// face, and the table maps that face to -Z (index 5). If either side flips
    /// front/back, this fails headless instead of putting eyes on the back of
    /// the head on screen.
    #[test]
    fn eyes_fall_inside_head_front_uv_rect() {
        let px = default_skin_rgba();
        let front = crate::skin_uv::base_faces(crate::skin_uv::ArmModel::Classic)[0][5]; // head, -Z (front)
        let (x0, y0) = ((front[0] * 64.0).round() as u32, (front[1] * 64.0).round() as u32);
        let (x1, y1) = ((front[2] * 64.0).round() as u32, (front[3] * 64.0).round() as u32);
        assert_eq!((x0, y0, x1, y1), (8, 8, 16, 16), "head front rect must be the (8,8)-(16,16) tile");
        for (ex, ey) in [(10u32, 12u32), (13, 12)] {
            assert!(ex >= x0 && ex < x1 && ey >= y0 && ey < y1, "eye ({ex},{ey}) outside head-front rect");
            let eye = px[((ey * 64 + ex) * 4) as usize..][..4].to_vec();
            let cheek = px[((y0 * 64 + x0) * 4) as usize..][..4].to_vec(); // a corner skin pixel
            assert_ne!(eye, cheek, "eye pixel must differ from face skin (eyes actually painted)");
            assert_eq!(eye[3], 255, "eye pixel must be opaque");
        }
    }

    #[test]
    fn tinted_variant0_is_the_default() {
        assert_eq!(tinted_default_skin_rgba(0), default_skin_rgba());
    }
    #[test]
    fn tinted_variant_differs_and_preserves_size_and_alpha() {
        let def = default_skin_rgba();
        let t1 = tinted_default_skin_rgba(1);
        assert_eq!(t1.len(), 64 * 64 * 4);
        assert_ne!(t1, def, "a non-zero variant must visibly differ");
        // Alpha channel preserved (tint only touches RGB).
        for i in (3..def.len()).step_by(4) {
            assert_eq!(t1[i], def[i], "alpha must be preserved at byte {i}");
        }
    }
}
