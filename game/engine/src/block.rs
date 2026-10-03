//! Block type definitions, registry, and per-face texture mapping.
//!
//! Spec 02 defines BlockId as u16, 0 = air, 1-1023 reserved for genesis:* blocks.

pub type BlockId = u16;

pub const AIR: BlockId = 0;
pub const STONE: BlockId = 1;
pub const DIRT: BlockId = 2;
pub const GRASS: BlockId = 3;
pub const BEDROCK: BlockId = 4;
pub const SAND: BlockId = 5;
pub const WATER: BlockId = 6;
pub const OAK_LOG: BlockId = 7;
pub const OAK_LEAVES: BlockId = 8;
pub const OAK_PLANKS: BlockId = 9;
pub const COBBLESTONE: BlockId = 10;
pub const GRAVEL: BlockId = 11;
pub const SANDSTONE: BlockId = 12;
pub const SNOW: BlockId = 13;
pub const CRAFTING_TABLE: BlockId = 14;
pub const BED: BlockId = 15;
pub const COAL_ORE: BlockId = 16;
pub const IRON_ORE: BlockId = 17;
pub const DIAMOND_ORE: BlockId = 18;
pub const GLASS: BlockId = 19;
pub const COAL_BLOCK: BlockId = 20;
pub const IRON_BLOCK: BlockId = 21;
pub const DIAMOND_BLOCK: BlockId = 22;
pub const TORCH: BlockId = 23;
pub const TALL_GRASS: BlockId = 24;
// Deepslate layer (Spec 2 §5.3.1a). Pure deepslate replaces stone below Y_dp;
// Satori veins (Spec 6 §2.2c) only spawn in any PURE_DEEPSLATE-family block.
pub const PURE_DEEPSLATE: BlockId = 25;
pub const DEEPSLATE_COAL_ORE: BlockId = 26;
pub const DEEPSLATE_IRON_ORE: BlockId = 27;
pub const DEEPSLATE_DIAMOND_ORE: BlockId = 28;
// Spec 16 Phase 3b — visual variants of PURE_DEEPSLATE. Same gameplay
// identity (mining time, tool gate, proof-of-play roll); only the
// texture differs. Chunk-gen picks variant from the server's
// `reserve.richness` snapshot at gen time via
// `biome::pick_deepslate_variant`. Mining any variant drops the
// canonical PURE_DEEPSLATE item (see `mine_drop` normalisation arm).
pub const PURE_DEEPSLATE_THIN: BlockId = 61;
pub const PURE_DEEPSLATE_HEALTHY: BlockId = 62;
pub const PURE_DEEPSLATE_FAT: BlockId = 63;

// Spec 21 — Vendor Block. Player-placed shop. Owner sets a mode +
// item + price; buyers right-click to trade. Block-entity state
// (mode, slot, stock, escrow) lives in `World::block_entities` as
// `BlockEntityData::Vendor(VendorData)`. Crafted from 8 planks
// ringing 1 iron ingot (chest + coin-slot motif).
pub const VENDOR_BLOCK: BlockId = 64;

// Spec 26 Drafting Table — workstation for the Builder profession.
// Villager bound to this block reads as Builder; v1 commission UI is
// DEFERRED post-MVP (the workstation just confers the profession label).
pub const DRAFTING_TABLE: BlockId = 65;

// Spec 28c Materials Expansion (2026-05-20). Stone variants + Copper Ore
// + decorative blocks. Bincode-positional: appended after DRAFTING_TABLE.
pub const LIMESTONE: BlockId = 66;
pub const MARBLE: BlockId = 67;
pub const GRANITE: BlockId = 68;
pub const SLATE: BlockId = 69;
pub const COPPER_ORE: BlockId = 70;
pub const BONE_BLOCK: BlockId = 71;
pub const HAY_BALE: BlockId = 72;
pub const AMETHYST_BLOCK: BlockId = 73;

// Spec 28b Wood Species (2026-05-20). Five new species × {log, leaves,
// planks}. Bincode-positional. Oak stays at its original ids 7/8/9; the
// new species mine-drop to GreenLog (species-neutral) just like Oak,
// preserving the Wave 29 log-seasoning pipeline.
pub const BIRCH_LOG: BlockId = 74;
pub const BIRCH_LEAVES: BlockId = 75;
pub const BIRCH_PLANKS: BlockId = 76;
pub const SPRUCE_LOG: BlockId = 77;
pub const SPRUCE_LEAVES: BlockId = 78;
pub const SPRUCE_PLANKS: BlockId = 79;
pub const JUNGLE_LOG: BlockId = 80;
pub const JUNGLE_LEAVES: BlockId = 81;
pub const JUNGLE_PLANKS: BlockId = 82;
pub const ACACIA_LOG: BlockId = 83;
pub const ACACIA_LEAVES: BlockId = 84;
pub const ACACIA_PLANKS: BlockId = 85;
pub const DARK_OAK_LOG: BlockId = 86;
pub const DARK_OAK_LEAVES: BlockId = 87;
pub const DARK_OAK_PLANKS: BlockId = 88;

// Spec T1.5 Processed Economy Base (2026-05-14 spec, 2026-05-20 block-id
// layer landed). Bincode-positional. Crops + workstations.
// Sugarcane is a single-block-tall placeholder until vertical-stack
// growth lands (water-adjacent multi-block tall like Minecraft is
// playtest-gated). Crop-stage blocks follow the wheat/carrot/potato/
// corn 4-stage convention. Pumpkin Stem is 5 stages — stages 0-3 are
// growth, stage 4 is "mature and producing pumpkins on a free side
// tile". Berry Bush is 4-stage replenishing.
pub const SUGARCANE: BlockId = 89;
pub const SUGAR_BEET_STAGE_0: BlockId = 90;
pub const SUGAR_BEET_STAGE_1: BlockId = 91;
pub const SUGAR_BEET_STAGE_2: BlockId = 92;
pub const SUGAR_BEET_STAGE_3: BlockId = 93; // mature
pub const BEETROOT_STAGE_0: BlockId = 94;
pub const BEETROOT_STAGE_1: BlockId = 95;
pub const BEETROOT_STAGE_2: BlockId = 96;
pub const BEETROOT_STAGE_3: BlockId = 97; // mature
pub const PUMPKIN: BlockId = 98;
pub const PUMPKIN_STEM_0: BlockId = 99;
pub const PUMPKIN_STEM_1: BlockId = 100;
pub const PUMPKIN_STEM_2: BlockId = 101;
pub const PUMPKIN_STEM_3: BlockId = 102;
pub const PUMPKIN_STEM_4: BlockId = 103; // mature — produces pumpkins
pub const BERRY_BUSH_0: BlockId = 104;
pub const BERRY_BUSH_1: BlockId = 105;
pub const BERRY_BUSH_2: BlockId = 106;
pub const BERRY_BUSH_3: BlockId = 107; // mature — right-click harvest, regrows
pub const MILL: BlockId = 108;
pub const OVEN: BlockId = 109;
pub const AGING_RACK: BlockId = 110;

/// Spec 28d chunk 8 — Bee Hive. Block-entity stores `HiveData` (bees
/// inside + honey level 0..=5). Right-click matrix lives in
/// `bee_hive::resolve_right_click`. Bees deposit honey on return from
/// pollination (playtest-gated for live wire-up). Crafted from 6 planks
/// + 3 honeycomb on a 3x3 grid; surface texture reuses oak planks
///   (BRIDGE: dedicated hive texture lands in texture_gen.rs phase 9).
pub const BEE_HIVE: BlockId = 111;

/// Historical Pivot Sub-Foundation 2 (HP-2, 2026-05-22). Chest — 27-slot
/// storage block-entity. Right-click opens a 3×9 inventory dialog
/// (`chest_ui.rs`). Mined drops contents at the chest's world position
/// plus 1 Chest item. Block-entity state lives in `World::block_entities`
/// as `BlockEntityData::Chest(ChestData)`. Crafted from 8 oak planks
/// ringing an empty centre (mirrors the Furnace recipe shape; accepts
/// any wood species via the `is_any_planks` predicate). Also the
/// Brigand Hideout stockpile primitive for HP-3.
pub const CHEST: BlockId = 112;

/// Historical Pivot Sub-Foundation 3 (HP-3, 2026-05-23). Brigand Hideout
/// banner — decorative tall block placed at the palisade gate of a
/// worldgen Brigand Hideout, visible from a distance as the "this is a
/// bandit camp" cue. Pure decoration: no block-entity, no recipe, no
/// drop. Procedurally textured (`texture_gen.rs`) — rough purple-and-red
/// banner motif on a stick. Transparent so the model reads as a banner
/// rather than a cube. Removing the banner does NOT remove the hideout
/// from the world.brigand_hideouts side-table — the structure is
/// identified by its grid cell, not by this block.
pub const BRIGAND_HIDEOUT_BANNER: BlockId = 113;

/// HP-3 v2 (2026-05-23). Trophy Wall — a kept-trophy decorative block
/// crafted from a BrigandChieftainTrophy (HP-1-reserved material) plus
/// two oak planks (vertical 3x1 grid: trophy / plank / plank). Trophy
/// is consumed at craft; the block becomes the kept artefact a player
/// can mount on their base wall as a visible kill counter.
///
/// Pure decoration: no block-entity, no inventory; mining drops the
/// Trophy Wall block itself so a player can move it. Lifts cross-game
/// (any survival game with "trophy from a defeated boss" content).
pub const TROPHY_WALL: BlockId = 114;

/// Salt feature (2026-05-23). See docs/foundations/2026-05-23-salt.md.
/// ROCK_SALT is the mineable ore; SALT_LICK is the husbandry aura block;
/// SALT_LAMP is a torch-equivalent light source; SALT_BLOCK is 9-salt
/// storage; SALT_PATH is the placeable salted-tile that suppresses
/// snowfall.
pub const ROCK_SALT: BlockId = 115;
pub const SALT_LICK: BlockId = 116;
pub const SALT_LAMP: BlockId = 117;
pub const SALT_BLOCK: BlockId = 118;
pub const SALT_PATH: BlockId = 119;

/// Rubber feature (2026-05-23). See docs/foundations/2026-05-23-rubber.md.
/// RUBBER_LOG is the tappable living tree; RUBBER_LOG_TAPPED is its
/// post-tap cooldown variant (returns to RUBBER_LOG after 24 000 ticks).
/// Mining either drops 1 RUBBER_LOG (timber only — cooldown lost).
pub const RUBBER_LOG: BlockId = 120;
pub const RUBBER_PLANKS: BlockId = 121;
pub const RUBBER_LEAVES: BlockId = 122;
pub const RUBBER_LOG_TAPPED: BlockId = 123;

/// Mob Bounty Board (Spec 33, 2026-05-23). Right-click to open the
/// daily bounty dialog. Server-issued bounties rotate every
/// BOUNTY_REFRESH_TICKS via `bounty::tick_bounty_refresh`. Recipe:
/// 8 planks ring around 1 PapyrusSheet centre (PapyrusSheet avoids
/// the Vendor-Block iron-centre collision + is thematically apt).
pub const BOUNTY_BOARD: BlockId = 124;

/// Tip Jar (Spec 34, 2026-05-23). Right-click to open the tip
/// dialog: non-owner sees preset-amount buttons (1/5/25/100 sats),
/// owner sees the Withdraw button + lifetime tip counter. Block-
/// entity state lives in `BlockEntityData::TipJar(TipJarData)`.
/// Recipe: 8 planks ring + 2 IronIngot stacked at centre (the
/// stack distinguishes it from Vendor Block's single-iron centre).
pub const TIP_JAR: BlockId = 125;

/// Repair Bench (Spec 35, 2026-05-23). Stateless station — right-
/// click opens a 2-slot repair dialog (damaged tool + tier material)
/// that restores durability + charges a sats repair tax (the
/// economy's first sink). No block-entity. Recipe: 3 IronIngot top
/// row + 1 Stone centre + 3 Stone bottom row (anvil-ish; distinct
/// from the pickaxe pattern which needs sticks in the handle column).
pub const REPAIR_BENCH: BlockId = 126;

/// Plot Marker (Spec 36, 2026-05-23). Placing it claims a 32×32
/// region owned by the placer; non-owners can't place/break inside
/// (anti-grief). Breaking your own marker releases the plot. Plot
/// state lives in `World.plots: Vec<PlotData>`. Recipe: 4 IronIngot
/// corners + 1 OAK_PLANKS centre (boundary-post shape).
pub const PLOT_MARKER: BlockId = 127;

/// Market Bell (Spec 37, 2026-05-23). Placing it designates a 32-
/// block Market Hub — a vendor-discovery zone owned by the placer.
/// Right-click opens a read-only directory of every Vendor Block in
/// radius; `/market` gives a compass hint to the nearest hub. Hub
/// state lives in `World.market_hubs: Vec<MarketHubData>`. Recipe:
/// gold-nugget cap over 4 iron (distinct from Village Bell).
pub const MARKET_BELL: BlockId = 128;

/// Auction Block (Spec 38, 2026-05-23). A timed auction podium —
/// owner configures a lot + reserve + duration; anyone bids; the
/// deadline-settlement driver awards the lot to the high bidder at
/// expiry (anti-snipe extends). State lives in
/// `BlockEntityData::Auction(AuctionData)`. Recipe: 4 IronIngot
/// corners + 5 OAK_PLANKS (edges + centre).
pub const AUCTION_BLOCK: BlockId = 129;

/// Bazaar Block (Spec 39, 2026-05-23). A stateless, server-run
/// sell-floor — right-click to sell the held stack for its
/// `trade_value × count` in sats (the market-of-last-resort that
/// gives every item guaranteed liquidity). No owner, no block-entity.
/// Recipe: 8 OAK_PLANKS ring around 1 Diamond centre.
pub const BAZAAR_BLOCK: BlockId = 130;

/// Dye flowers (Spec 35, 2026-05-27). Wild, non-solid X-mesh plants
/// scattered on grass. Break to collect the flower (drops itself); craft
/// 1 flower → 1 primary dye. Cornflower=blue, Field Poppy=red,
/// Buttercup=yellow.
pub const CORNFLOWER: BlockId = 131;
pub const FIELD_POPPY: BlockId = 132;
pub const BUTTERCUP: BlockId = 133;

/// Fibre plants (Spec 36, 2026-05-27). Wild, non-solid plants. Cotton
/// (warm biomes) breaks to Cotton → String; Hemp (temperate) breaks to
/// Hemp Fibre → Rope. (Farmable planting is a fast-follow.)
pub const COTTON_PLANT: BlockId = 134;
pub const HEMP_PLANT: BlockId = 135;

/// Magnesium Ore (Spec 37, 2026-05-27). Mineable mineral (stone pickaxe+),
/// drops Magnesium → fertiliser / sparklers / flares / firestarter.
pub const MAGNESIUM_ORE: BlockId = 136;

/// Farmable fibre crops (Spec 36 Phase 2, 2026-05-27). Plant seeds on tilled
/// soil → 4 growth stages → mature drops fibre + seeds. The wild
/// COTTON_PLANT/HEMP_PLANT (134/135) remain the found-in-the-world version.
pub const COTTON_STAGE_0: BlockId = 137;
pub const COTTON_STAGE_1: BlockId = 138;
pub const COTTON_STAGE_2: BlockId = 139;
pub const COTTON_STAGE_3: BlockId = 140;
pub const HEMP_STAGE_0: BlockId = 141;
pub const HEMP_STAGE_1: BlockId = 142;
pub const HEMP_STAGE_2: BlockId = 143;
pub const HEMP_STAGE_3: BlockId = 144;

/// Coloured wallpaper (Dyed-paper décor, 2026-05-27). Placeable decorative
/// blocks crafted from Papyrus Sheet + a dye — the dyes' first in-world
/// consumer. One per dye colour (13). Solid, opaque, drops themselves.
pub const WALLPAPER_WHITE: BlockId = 145;
pub const WALLPAPER_BLACK: BlockId = 146;
pub const WALLPAPER_RED: BlockId = 147;
pub const WALLPAPER_BLUE: BlockId = 148;
pub const WALLPAPER_YELLOW: BlockId = 149;
pub const WALLPAPER_ORANGE: BlockId = 150;
pub const WALLPAPER_GREEN: BlockId = 151;
pub const WALLPAPER_PURPLE: BlockId = 152;
pub const WALLPAPER_PINK: BlockId = 153;
pub const WALLPAPER_LIME: BlockId = 154;
pub const WALLPAPER_LIGHT_BLUE: BlockId = 155;
pub const WALLPAPER_GREY: BlockId = 156;
pub const WALLPAPER_LIGHT_GREY: BlockId = 157;
// Spec 35 Phase 2 completion (2026-05-28) — wallpaper variants for the
// three 3-input mix dyes. LATENT_PRINT (Spec 38) sits between them and
// the wallpaper range at id 158; the new wallpapers continue at 159.
pub const WALLPAPER_BROWN: BlockId = 159;
pub const WALLPAPER_CYAN: BlockId = 160;
pub const WALLPAPER_MAGENTA: BlockId = 161;

// Spec 35 farmable-flower follow-on (2026-05-28). Each of the three
// primary dye flowers (Cornflower / Field Poppy / Buttercup at ids
// 131-133) becomes the **mature** form of its own 4-stage crop ladder;
// the three immature stages live here. Wild-scatter via
// `World::place_vegetation` still plants the mature form directly so
// players who haven't unlocked farming can still find their first
// seeds; mining ANY mature flower drops the flower + 1-2 seeds.
pub const CORNFLOWER_STAGE_0: BlockId = 162;
pub const CORNFLOWER_STAGE_1: BlockId = 163;
pub const CORNFLOWER_STAGE_2: BlockId = 164;
pub const FIELD_POPPY_STAGE_0: BlockId = 165;
pub const FIELD_POPPY_STAGE_1: BlockId = 166;
pub const FIELD_POPPY_STAGE_2: BlockId = 167;
pub const BUTTERCUP_STAGE_0: BlockId = 168;
pub const BUTTERCUP_STAGE_1: BlockId = 169;
pub const BUTTERCUP_STAGE_2: BlockId = 170;

// Spec 36 Fences mini-spec (2026-05-28). The simplest possible fence
// primitive: a single solid post block with no connectivity logic.
// Built so a Lead tether (Spec 36 Phase 2 follow-on) has something
// to anchor to; richer fences (rails, gates, species variants) are
// a v2 spec. Crafted via the Minecraft-style `Plank/Stick/Plank` 2×3
// recipe so the existing muscle memory works. Texture reuses
// `TEX_OAK_PLANKS` — fence posts ARE made of planks.
pub const FENCE_POST: BlockId = 171;

// Spec 38 cyanotype-art variant (v1, 2026-05-28). Hang-as-décor
// destination from a Developed Plan: right-clicking a wall while
// holding a Developed `Item::Plan(_)` places this block + consumes
// the plan. v1 renders as a generic blueprint-blue framed picture
// (single texture); the proper wall-orientation art-CAPTURE mechanic
// that samples a 2D wall slice into per-instance art content is
// deferred to v2 — it needs BLUEPRINT_PAPER orientation tracking +
// a new `PlanKind::Art` data shape.
pub const CYANOTYPE_PRINT: BlockId = 172;

// Spec 35 dyed-décor — Bunting (2026-05-28). One block per dye colour
// (16 ids, 173-188), matching the existing wallpaper palette. Crafted
// as `Dye + String + Dye` 1×3 horizontal → 4 bunting of that dye's
// colour. Non-solid + transparent — renders via the X-mesh small-cube
// path same as flowers + crops. "Bunting" is the row of triangular
// flags strung on a line; visually it's a single decorative block per
// cell so the player can string a row by placing several.
pub const BUNTING_WHITE: BlockId = 173;
pub const BUNTING_BLACK: BlockId = 174;
pub const BUNTING_RED: BlockId = 175;
pub const BUNTING_BLUE: BlockId = 176;
pub const BUNTING_YELLOW: BlockId = 177;
pub const BUNTING_ORANGE: BlockId = 178;
pub const BUNTING_GREEN: BlockId = 179;
pub const BUNTING_PURPLE: BlockId = 180;
pub const BUNTING_PINK: BlockId = 181;
pub const BUNTING_LIME: BlockId = 182;
pub const BUNTING_LIGHT_BLUE: BlockId = 183;
pub const BUNTING_GREY: BlockId = 184;
pub const BUNTING_LIGHT_GREY: BlockId = 185;
pub const BUNTING_BROWN: BlockId = 186;
pub const BUNTING_CYAN: BlockId = 187;
pub const BUNTING_MAGENTA: BlockId = 188;

// Spec 35 dyed-décor — Paper Lantern (2026-05-28). One block per
// dye colour (16 ids, 189-204). Solid + opaque + light-emitting
// (level 12 via the `light_emission` match below — slightly dimmer
// than a torch at 14). Crafted `Papyrus + Stick + Dye` 1×3
// horizontal → 1 paper lantern of that dye's colour. The dyes' first
// light-source consumer; brightens a base after dark + reads as a
// soft glowing cube of paper.
pub const PAPER_LANTERN_WHITE: BlockId = 189;
pub const PAPER_LANTERN_BLACK: BlockId = 190;
pub const PAPER_LANTERN_RED: BlockId = 191;
pub const PAPER_LANTERN_BLUE: BlockId = 192;
pub const PAPER_LANTERN_YELLOW: BlockId = 193;
pub const PAPER_LANTERN_ORANGE: BlockId = 194;
pub const PAPER_LANTERN_GREEN: BlockId = 195;
pub const PAPER_LANTERN_PURPLE: BlockId = 196;
pub const PAPER_LANTERN_PINK: BlockId = 197;
pub const PAPER_LANTERN_LIME: BlockId = 198;
pub const PAPER_LANTERN_LIGHT_BLUE: BlockId = 199;
pub const PAPER_LANTERN_GREY: BlockId = 200;
pub const PAPER_LANTERN_LIGHT_GREY: BlockId = 201;
pub const PAPER_LANTERN_BROWN: BlockId = 202;
pub const PAPER_LANTERN_CYAN: BlockId = 203;
pub const PAPER_LANTERN_MAGENTA: BlockId = 204;

// Spec 35 dyed-décor — Kite (2026-05-28). Closes the bunting / lantern
// / **kite** trio. One block per dye colour (16 ids, 205-220). Non-
// solid + transparent X-mesh decorative; mirrors the bunting render
// path. Recipe: `Cloth on top + String column + Cloth on bottom` 3-
// tall vertical → 1 kite of the cloth's colour (the cloth is itself
// crafted from 4 Cotton in a 2×2, so the kite carries forward the
// cotton-bolt's colour). For v1 every kite uses White Cloth + a dye
// in the dye slot, so the colour comes from the dye; v2 can let
// pre-dyed cloth carry its own colour and skip the dye slot.
pub const KITE_WHITE: BlockId = 205;
pub const KITE_BLACK: BlockId = 206;
pub const KITE_RED: BlockId = 207;
pub const KITE_BLUE: BlockId = 208;
pub const KITE_YELLOW: BlockId = 209;
pub const KITE_ORANGE: BlockId = 210;
pub const KITE_GREEN: BlockId = 211;
pub const KITE_PURPLE: BlockId = 212;
pub const KITE_PINK: BlockId = 213;
pub const KITE_LIME: BlockId = 214;
pub const KITE_LIGHT_BLUE: BlockId = 215;
pub const KITE_GREY: BlockId = 216;
pub const KITE_LIGHT_GREY: BlockId = 217;
pub const KITE_BROWN: BlockId = 218;
pub const KITE_CYAN: BlockId = 219;
pub const KITE_MAGENTA: BlockId = 220;

// Banner block (2026-05-28) — first **Cloth** consumer. Ground-
// mounted decorative pole with a dyed cloth flag at the top + a
// wooden shaft below. One block per dye colour (16 ids, 221-236).
// Solid + opaque; texture wraps a flag-on-pole pattern around the
// 16×16 face. Crafted `Dye + Cloth + Stick` 3-tall vertical column
// → 1 banner of that dye's colour (top → bottom: flag colour,
// fabric, pole).
pub const BANNER_WHITE: BlockId = 221;
pub const BANNER_BLACK: BlockId = 222;
pub const BANNER_RED: BlockId = 223;
pub const BANNER_BLUE: BlockId = 224;
pub const BANNER_YELLOW: BlockId = 225;
pub const BANNER_ORANGE: BlockId = 226;
pub const BANNER_GREEN: BlockId = 227;
pub const BANNER_PURPLE: BlockId = 228;
pub const BANNER_PINK: BlockId = 229;
pub const BANNER_LIME: BlockId = 230;
pub const BANNER_LIGHT_BLUE: BlockId = 231;
pub const BANNER_GREY: BlockId = 232;
pub const BANNER_LIGHT_GREY: BlockId = 233;
pub const BANNER_BROWN: BlockId = 234;
pub const BANNER_CYAN: BlockId = 235;
pub const BANNER_MAGENTA: BlockId = 236;

// Sail block (2026-05-28) — first **Canvas** consumer. Larger
// decorative dyed-canvas square for ship-flag / awning use. One
// block per dye colour (16 ids, 237-252). Solid + opaque; texture is
// mostly dye-coloured canvas with a cross-hatch weave so it reads as
// fabric, not wallpaper. Crafted `Dye / Canvas / Stick` 3-tall
// vertical column → 1 sail of that dye's colour — same shape as
// Banner with Canvas swapped for Cloth (the cotton-vs-hemp split:
// banner = fine textile, sail = coarse sailcloth).
pub const SAIL_WHITE: BlockId = 237;
pub const SAIL_BLACK: BlockId = 238;
pub const SAIL_RED: BlockId = 239;
pub const SAIL_BLUE: BlockId = 240;
pub const SAIL_YELLOW: BlockId = 241;
pub const SAIL_ORANGE: BlockId = 242;
pub const SAIL_GREEN: BlockId = 243;
pub const SAIL_PURPLE: BlockId = 244;
pub const SAIL_PINK: BlockId = 245;
pub const SAIL_LIME: BlockId = 246;
pub const SAIL_LIGHT_BLUE: BlockId = 247;
pub const SAIL_GREY: BlockId = 248;
pub const SAIL_LIGHT_GREY: BlockId = 249;
pub const SAIL_BROWN: BlockId = 250;
pub const SAIL_CYAN: BlockId = 251;
pub const SAIL_MAGENTA: BlockId = 252;

// Per-wood-species fence posts (Fences v2 — 2026-05-28). The existing
// FENCE_POST = 171 stays as the oak variant (it uses TEX_OAK_PLANKS);
// `OAK_FENCE_POST` is added below as a forward-compat alias. The six
// new species variants live at 253-258 and reuse the same plank-side
// texture with the species' tint. Connected geometry (the visible
// joining mesh between adjacent posts) + gates stay v3.
pub const BIRCH_FENCE_POST: BlockId = 253;
pub const SPRUCE_FENCE_POST: BlockId = 254;
pub const JUNGLE_FENCE_POST: BlockId = 255;
pub const ACACIA_FENCE_POST: BlockId = 256;
pub const DARK_OAK_FENCE_POST: BlockId = 257;
pub const RUBBER_FENCE_POST: BlockId = 258;

// Tent block (2026-05-28) — first multi-block-décor primitive +
// second Canvas consumer alongside Sail. Solid + opaque single-cube
// for v1 (a "small dome tent" footprint); multi-block variants like
// a 2×2 family tent + a sleep mechanic are future polish. Crafted
// `Canvas Canvas Canvas / Stick . Stick` (2×3 with empty bottom-
// centre) → 1 Tent — six-input recipe distinct from every other
// 2×3 pattern (Bed, Helmet, Boots, Fence Post).
pub const TENT: BlockId = 259;
// 260 = `rail::TRACK` (genesis:track) — the next sequential block id lives in
// `rail.rs` with the rest of the rail-freight logic; registered in the
// `BlockRegistry::new()` push list below right after TENT.

// #30 building-detail blocks (2026-06-16) — the foundation-free subset. Both are
// non-solid + transparent so they render through `mesh.rs::non_solid_shape_for`
// (no custom-mesh / per-block-state needed). Ladder is climbable (see
// `is_climbable`); Carpet is decorative-only. Doors/stairs/slabs/panes wait on the
// block-shape foundation — see `docs/foundations/2026-06-16-building-detail-blocks.md`.
pub const LADDER: BlockId = 261;
pub const CARPET: BlockId = 262;

// #47 — Grave (2026-06-16). Solid headstone block placed at death holding the
// player's snapshotted inventory (`BlockEntityData::Grave`); right-click to
// recover items to their original slots, break to spill. See
// `docs/foundations/2026-06-15-graves-and-keep-inventory.md`.
pub const GRAVE: BlockId = 263;

// #15 — tiered storage chests. The wood `CHEST` (112) is tier 0; these add
// capacity. See `docs/foundations/2026-06-16-tiered-storage.md`.
pub const COPPER_CHEST: BlockId = 264;
pub const IRON_CHEST: BlockId = 265;
pub const DIAMOND_CHEST: BlockId = 266;
pub const SATORI_CHEST: BlockId = 267;

// ─── Spec 48 (Electricity / Power & Logic) — the redstone replacement ───
// Conductor (insulated copper cable) + its energised visual twin.
pub const CABLE: BlockId = 268;
pub const CABLE_LIT: BlockId = 269;
// Consumer — electric lamp (off / lit visual twin; lit emits light).
pub const ELECTRIC_LAMP: BlockId = 270;
pub const ELECTRIC_LAMP_LIT: BlockId = 271;
// Inputs.
pub const LEVER: BlockId = 272;
pub const BUTTON: BlockId = 273;
pub const PRESSURE_PLATE: BlockId = 274;
// Logic.
pub const LOGIC_GATE: BlockId = 275;
// Generate-and-store.
pub const HAND_CRANK: BlockId = 276;
pub const STEAM_GENERATOR: BlockId = 277;
pub const STEAM_GENERATOR_LIT: BlockId = 278;
pub const BATTERY: BlockId = 279;
// Spec 48 Phase 2 — sensors. Armed/tripped state rides the meta byte (no lit
// twin); the beam itself is a render-only effect.
pub const BEAM_SENSOR: BlockId = 280;
pub const MIRROR: BlockId = 281;
pub const MOTION_SENSOR: BlockId = 282;

// F1 — block-shape foundation (Solo Buildout Wave 2). Shaped blocks store their
// orientation/half in the meta byte and resolve collision + render geometry via
// `block_shape::{collision_aabbs, render_cuboids}`. STONE_SLAB is the first
// proof-of-foundation block; the building-detail family (stairs, doors, panes,
// walls, …) is added on top once the foundation is live.
pub const STONE_SLAB: BlockId = 283;
pub const STONE_STAIRS: BlockId = 284;
// Wave 2 — first open/close building block (proves the toggle machinery the
// doors + double-doors reuse). Facing = blocked axis; meta state bit 0 = open.
pub const OAK_FENCE_GATE: BlockId = 285;
// Trapdoor — closed lid (floor/ceiling) ↔ open vertical flap. meta state bit 0
// = open, bit 1 = top-mounted; facing = the wall the flap swings to.
pub const OAK_TRAPDOOR: BlockId = 286;
// Panes — thin vertical sheets (Pane shape). facing = the axis the sheet spans.
pub const GLASS_PANE: BlockId = 287;
pub const IRON_BARS: BlockId = 288;
// Door — a thin panel, two cells tall. One id for both halves; meta state bit 1
// distinguishes top from bottom, bit 0 = open, aux bit 0 = hinge side.
pub const OAK_DOOR: BlockId = 289;
// Wall — a connecting block (Wall shape): central post + neighbour-derived arms.
// Geometry comes from a live connection mask, not stored meta (like Pane).
pub const COBBLESTONE_WALL: BlockId = 290;
// NOTE: the signal-input blocks LEVER (272) / BUTTON (273) / PRESSURE_PLATE
// (274) already exist as Electricity (Spec 48) power devices. Wave 2c gives
// those existing blocks proper flush F1 shapes (see block_shape::shape_of) —
// it does NOT mint new ids.
// Sign — a standing text board (Sign shape). Text lives in a
// `BlockEntityData::Sign`; facing = the readable side. Pass-through.
pub const OAK_SIGN: BlockId = 291;
// Item Frame — a thin wall plate (ItemFrame shape) displaying one item from a
// `BlockEntityData::ItemFrame`; facing = the wall it's mounted on. Pass-through.
pub const ITEM_FRAME: BlockId = 292;

// ── Spec 49 (Explosives) — supply chain + the keg (293..=297) ──
// Brimstone = "burning stone" = sulphur ore. Mines (stone pickaxe+) to Sulphur.
pub const BRIMSTONE: BlockId = 293;
// Nitre deposit — niter evaporite. Mines (stone pickaxe+) to Saltpetre.
pub const NITRE_ORE: BlockId = 294;
// Composter — farming nitre-bed. Ages plant/food waste → Compost → Saltpetre.
// Holds a `BlockEntityData::Composter`.
pub const COMPOSTER: BlockId = 295;
// Blasting Keg — a barrel charge (NEVER the red TNT cube). A power-network
// sink with a fuse; lit by the Magnesium Firestarter or a rising-edge pulse.
pub const BLASTING_KEG: BlockId = 296;
// Plunger Detonator — boxed T-handle switch. A Button-style momentary power
// source (directional via `block_meta`).
pub const PLUNGER_DETONATOR: BlockId = 297;
// P7 — Hopper. An item conduit: each interval it moves one item from the
// container directly above it into the container directly below it. v1 is a
// pure conduit (no internal buffer), so no block-entity / save change.
pub const HOPPER: BlockId = 298;
// P10 — Lava. Non-solid fluid hazard: emits full light (15), burns anything
// standing in it, and fills deep caves at world-gen. v1 is static (no flow);
// flow + obsidian + lava buckets are follow-ups.
pub const LAVA: BlockId = 299;
// P11 — Piston. A block that, when powered by the Electricity grid, extends a
// PISTON_HEAD arm and shoves the column of pushable blocks in front of it by one
// (Minecraft-style). Push direction is stored in the block meta (0..5 facing).
pub const PISTON: BlockId = 300;
// P11 — the extended piston arm. Placed in the cell directly ahead of a powered
// piston (its presence = "extended"); removed on retract. Same facing meta as
// its base. Solid (you can stand on it).
pub const PISTON_HEAD: BlockId = 301;
// Campaign B — Obsidian. The dark, near-indestructible stone formed where
// water quenches lava (see `lava.rs` + `water.rs`). Fully solid; emits no
// light. Drops itself when mined.
pub const OBSIDIAN: BlockId = 302;
// Campaign D — Sticky Piston. Pushes like a plain piston, but on RETRACT it
// also pulls the single block stuck to its head back into place. Crafted from a
// Piston + Rubber (the sticky binder). Same facing meta as PISTON.
pub const STICKY_PISTON: BlockId = 303;

// Fire (2026-07-04 gap-fill wave) — a spreading, burning-out flame cell.
// Non-solid, light 15, no drop; simulated by `fire::FireSystem` on BOTH the
// client tick and the authoritative server (lava idiom). Spread is gated by
// `WorldMeta.fire_spread_enabled`.
pub const FIRE: BlockId = 304;

// Dispenser + Dropper (2026-07-04 gap-fill wave) — 9-slot container blocks
// that eject their first item on a power rising edge, out of the meta-facing
// side. Dispenser SHOOTS arrows; dropper always tosses the item entity.
// State lives in `BlockEntityData::Dispenser` (dispenser.rs).
pub const DISPENSER: BlockId = 305;
pub const DROPPER: BlockId = 306;

// Saplings (2026-07-04 gap-fill wave) — one planted-sapling block per wood
// species. Placed from the sapling MATERIAL (material_as_placeable_block),
// gated to grass/dirt (sapling::can_plant_sapling_at), grown into the real
// worldgen tree by growth::advance_saplings (tree_shapes::place_tree).
pub const SAPLING_OAK: BlockId = 307;
pub const SAPLING_BIRCH: BlockId = 308;
pub const SAPLING_SPRUCE: BlockId = 309;
pub const SAPLING_JUNGLE: BlockId = 310;
pub const SAPLING_ACACIA: BlockId = 311;
pub const SAPLING_DARK_OAK: BlockId = 312;
pub const SAPLING_RUBBER: BlockId = 313;

// Snow layer (2026-07-04 gap-fill wave) — thin accumulating snow. Layer count
// (1..=8) in meta AUX; snowfall stacks it and converts to full SNOW at 8.
pub const SNOW_LAYER: BlockId = 314;

// Pet Bed (2026-07-06 pets wave) — a placeable anti-loss anchor. When a
// player's tamed pet would die, `pet_bed::find_pet_bed_near` looks for one
// of these in the loaded world and rescues the pet to it instead (teleport +
// full heal) rather than letting it die. Stateless full-cube; crafted from
// Wool + Oak Planks (see crafting.rs's `2x3` section — distinct grid shape
// from the player BED so the two recipes don't collide).
pub const PET_BED: BlockId = 315;

// Spec 48 Phase 4 (Electricity) — Water Wheel. A paddle wheel on an iron axle:
// a power SOURCE that turns whenever a *flowing* water cell touches it (one of
// the four horizontal neighbours, or the cell directly below). Still water — a
// pond, a lone source block — drives nothing, so a builder has to cut a channel
// and give the water somewhere to fall. `WATER_WHEEL_TURNING` is the spinning
// visual twin (the Steam Generator lit/unlit pattern); it emits no light and
// drops the idle block when mined.
pub const WATER_WHEEL: BlockId = 316;
pub const WATER_WHEEL_TURNING: BlockId = 317;

// Wind, Copper & Electricity wave §2.2 — Windmill. The Water Wheel's dry twin:
// a power SOURCE that turns on the derived breeze (`crate::wind`) rather than
// on a current, so a builder with no river still has a renewable source. It
// only turns out in the open — clear air above and elbow room on at least two
// sides — and the breeze is stronger the higher you build, which is the whole
// teaching point. `WINDMILL_TURNING` is the spinning visual twin (the Water
// Wheel / Steam Generator lit-twin pattern); it emits no light and drops the
// idle block when mined.
pub const WINDMILL: BlockId = 318;
pub const WINDMILL_TURNING: BlockId = 319;

/// The planted-sapling block for a species. Symmetric counterpart to
/// `species_for_sapling` (which IS live, via `is_sapling_block`); this forward
/// direction has no caller yet.
#[allow(dead_code)]
pub fn sapling_block_for(s: WoodSpecies) -> BlockId {
    match s {
        WoodSpecies::Oak => SAPLING_OAK,
        WoodSpecies::Birch => SAPLING_BIRCH,
        WoodSpecies::Spruce => SAPLING_SPRUCE,
        WoodSpecies::Jungle => SAPLING_JUNGLE,
        WoodSpecies::Acacia => SAPLING_ACACIA,
        WoodSpecies::DarkOak => SAPLING_DARK_OAK,
        WoodSpecies::Rubber => SAPLING_RUBBER,
    }
}

/// Reverse of [`sapling_block_for`].
pub fn species_for_sapling(id: BlockId) -> Option<WoodSpecies> {
    Some(match id {
        SAPLING_OAK => WoodSpecies::Oak,
        SAPLING_BIRCH => WoodSpecies::Birch,
        SAPLING_SPRUCE => WoodSpecies::Spruce,
        SAPLING_JUNGLE => WoodSpecies::Jungle,
        SAPLING_ACACIA => WoodSpecies::Acacia,
        SAPLING_DARK_OAK => WoodSpecies::DarkOak,
        SAPLING_RUBBER => WoodSpecies::Rubber,
        _ => return None,
    })
}

/// Is this a planted-sapling block?
pub fn is_sapling_block(id: BlockId) -> bool {
    species_for_sapling(id).is_some()
}

/// Which species' leaves are these? (Reverse of `leaves_block_for` — feeds
/// the leaf-decay sapling drop.)
pub fn species_for_leaves(id: BlockId) -> Option<WoodSpecies> {
    Some(match id {
        OAK_LEAVES => WoodSpecies::Oak,
        BIRCH_LEAVES => WoodSpecies::Birch,
        SPRUCE_LEAVES => WoodSpecies::Spruce,
        JUNGLE_LEAVES => WoodSpecies::Jungle,
        ACACIA_LEAVES => WoodSpecies::Acacia,
        DARK_OAK_LEAVES => WoodSpecies::DarkOak,
        RUBBER_LEAVES => WoodSpecies::Rubber,
        _ => return None,
    })
}

/// Per-wood-species fence-post alias for the canonical oak variant.
/// FENCE_POST = OAK_FENCE_POST = 171; the constant existed before
/// the v2 species split so we keep it as the alias. Lead-tether
/// anchoring + /give shorthand keep using the legacy name.
pub const OAK_FENCE_POST: BlockId = FENCE_POST;

// Spec 38 (Blueprint / Cyanotype, 2026-05-27). LATENT_PRINT is the
// in-world block-entity host for a captured-but-undeveloped Plan.
// Place a Latent `Item::Plan(_)` on the ground → places LATENT_PRINT
// + stamps `BlockEntityData::LatentPrint(LatentPrintData)` carrying the
// Plan. The per-tick `latent_print::tick_develop` driver advances the
// embedded `develop_state` whenever the block sees full sky-light
// during daytime; right-click retrieves the Plan back into inventory
// at whatever develop_state has been reached. Texture is shared with
// BLUEPRINT_PAPER (pale parchment / sensitised face) — the develop
// progress surfaces via the inventory icon colour, not the block face,
// for the v1 visual.
pub const LATENT_PRINT: BlockId = 158;

/// Spec 28b — wood species enumeration. Drives the per-species block
/// helpers (`log_block_for` etc.) + the sapling material lookup. All
/// recipes that consume "any planks" should accept any species; the
/// `is_planks_slot` predicate covers the union.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum WoodSpecies {
    Oak,
    Birch,
    Spruce,
    Jungle,
    Acacia,
    DarkOak,
    Rubber,
}

// The four items below (`ALL_WOOD_SPECIES`, `log_block_for`, `leaves_block_for`,
// `planks_block_for`) are exercised by cross-check tests here and in
// `tree_shapes.rs` (which hardcodes per-species blocks directly rather than
// calling these), not by any production call site yet — hence
// `cfg_attr(not(test), ...)` rather than a plain allow.
#[cfg_attr(not(test), allow(dead_code))]
pub const ALL_WOOD_SPECIES: &[WoodSpecies] = &[
    WoodSpecies::Oak,
    WoodSpecies::Birch,
    WoodSpecies::Spruce,
    WoodSpecies::Jungle,
    WoodSpecies::Acacia,
    WoodSpecies::DarkOak,
    WoodSpecies::Rubber,
];

#[cfg_attr(not(test), allow(dead_code))]
pub fn log_block_for(s: WoodSpecies) -> BlockId {
    match s {
        WoodSpecies::Oak => OAK_LOG,
        WoodSpecies::Birch => BIRCH_LOG,
        WoodSpecies::Spruce => SPRUCE_LOG,
        WoodSpecies::Jungle => JUNGLE_LOG,
        WoodSpecies::Acacia => ACACIA_LOG,
        WoodSpecies::DarkOak => DARK_OAK_LOG,
        WoodSpecies::Rubber => RUBBER_LOG,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn leaves_block_for(s: WoodSpecies) -> BlockId {
    match s {
        WoodSpecies::Oak => OAK_LEAVES,
        WoodSpecies::Birch => BIRCH_LEAVES,
        WoodSpecies::Spruce => SPRUCE_LEAVES,
        WoodSpecies::Jungle => JUNGLE_LEAVES,
        WoodSpecies::Acacia => ACACIA_LEAVES,
        WoodSpecies::DarkOak => DARK_OAK_LEAVES,
        WoodSpecies::Rubber => RUBBER_LEAVES,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn planks_block_for(s: WoodSpecies) -> BlockId {
    match s {
        WoodSpecies::Oak => OAK_PLANKS,
        WoodSpecies::Birch => BIRCH_PLANKS,
        WoodSpecies::Spruce => SPRUCE_PLANKS,
        WoodSpecies::Jungle => JUNGLE_PLANKS,
        WoodSpecies::Acacia => ACACIA_PLANKS,
        WoodSpecies::DarkOak => DARK_OAK_PLANKS,
        WoodSpecies::Rubber => RUBBER_PLANKS,
    }
}

/// Inverse of `planks_block_for` — returns the species of a planks
/// block, or `None` if the id isn't planks. Added 2026-05-28 for the
/// per-species fence-post recipes: the crafting handler needs to read
/// which species' planks the player used + return the matching post.
pub fn species_for_planks(id: BlockId) -> Option<WoodSpecies> {
    match id {
        OAK_PLANKS => Some(WoodSpecies::Oak),
        BIRCH_PLANKS => Some(WoodSpecies::Birch),
        SPRUCE_PLANKS => Some(WoodSpecies::Spruce),
        JUNGLE_PLANKS => Some(WoodSpecies::Jungle),
        ACACIA_PLANKS => Some(WoodSpecies::Acacia),
        DARK_OAK_PLANKS => Some(WoodSpecies::DarkOak),
        RUBBER_PLANKS => Some(WoodSpecies::Rubber),
        _ => None,
    }
}

/// The two fluids. Neither is `solid` in the registry (you swim through them),
/// so anything asking "is this cell in the way?" for a reason other than
/// collision — wind reaching a Windmill's sails, say — has to ask this too.
pub fn is_fluid(id: BlockId) -> bool {
    matches!(id, WATER | LAVA)
}

/// Spec 48 (Electricity) — conductors that carry power and show a lit twin.
#[inline]
pub fn is_cable(id: BlockId) -> bool {
    matches!(id, CABLE | CABLE_LIT)
}

/// Spec 48 (Electricity) — any block belonging to the power/logic tier.
pub fn is_power_block(id: BlockId) -> bool {
    matches!(
        id,
        CABLE | CABLE_LIT
            | ELECTRIC_LAMP
            | ELECTRIC_LAMP_LIT
            | LEVER
            | BUTTON
            | PRESSURE_PLATE
            | LOGIC_GATE
            | HAND_CRANK
            | STEAM_GENERATOR
            | STEAM_GENERATOR_LIT
            | BATTERY
            | BEAM_SENSOR
            | MIRROR
            | MOTION_SENSOR
            // Spec 49 (Explosives) — the keg is a power sink; the plunger a source.
            | BLASTING_KEG
            | PLUNGER_DETONATOR
            // Spec 48 Phase 4 — the Water Wheel is a stream-driven source.
            | WATER_WHEEL
            | WATER_WHEEL_TURNING
            // Wind/Copper/Electricity §2.2 — the Windmill is a wind-driven source.
            | WINDMILL
            | WINDMILL_TURNING
    )
}

/// True iff `id` is any per-species fence-post variant. Used by the
/// Lead-tether anchor check + future fence-aware systems so a single
/// call subsumes all 7 species without hardcoding the list at every
/// call site.
#[inline]
pub fn is_fence_post(id: BlockId) -> bool {
    matches!(
        id,
        OAK_FENCE_POST
            | BIRCH_FENCE_POST
            | SPRUCE_FENCE_POST
            | JUNGLE_FENCE_POST
            | ACACIA_FENCE_POST
            | DARK_OAK_FENCE_POST
            | RUBBER_FENCE_POST
    )
}

/// Per-wood-species fence-post BlockId for a species.
pub fn fence_post_for_species(s: WoodSpecies) -> BlockId {
    match s {
        WoodSpecies::Oak => OAK_FENCE_POST,
        WoodSpecies::Birch => BIRCH_FENCE_POST,
        WoodSpecies::Spruce => SPRUCE_FENCE_POST,
        WoodSpecies::Jungle => JUNGLE_FENCE_POST,
        WoodSpecies::Acacia => ACACIA_FENCE_POST,
        WoodSpecies::DarkOak => DARK_OAK_FENCE_POST,
        WoodSpecies::Rubber => RUBBER_FENCE_POST,
    }
}

/// Returns the sapling MaterialId for a given species. Saplings are
/// `MaterialId` rather than blocks so they sit alongside seeds in the
/// inventory; on placement they convert into a future SAPLING_STAGE_0
/// block (deferred to live tree-gen).
pub fn sapling_material_for(s: WoodSpecies) -> crate::item::MaterialId {
    use crate::item::MaterialId as M;
    match s {
        WoodSpecies::Oak => M::OakSapling,
        WoodSpecies::Birch => M::BirchSapling,
        WoodSpecies::Spruce => M::SpruceSapling,
        WoodSpecies::Jungle => M::JungleSapling,
        WoodSpecies::Acacia => M::AcaciaSapling,
        WoodSpecies::DarkOak => M::DarkOakSapling,
        WoodSpecies::Rubber => M::RubberSapling,
    }
}

/// Predicate: is this BlockId any species' planks? Used by recipes
/// that accept any plank variant (door, sign, future crafting recipes
/// that don't care about colour).
pub fn is_any_planks(id: BlockId) -> bool {
    matches!(
        id,
        OAK_PLANKS
            | BIRCH_PLANKS
            | SPRUCE_PLANKS
            | JUNGLE_PLANKS
            | ACACIA_PLANKS
            | DARK_OAK_PLANKS
            | RUBBER_PLANKS
    )
}

/// Can fire consume this block? Derived match over the wood families (the
/// `light_emission` idiom — never a hardcoded list inside `fire::FireSystem`,
/// so a future mod-API / new block family extends it in one place).
pub fn flammable(id: BlockId) -> bool {
    is_any_log_block(id) || is_any_leaves(id) || is_any_planks(id)
}

/// Predicate: is this BlockId any species' log?
pub fn is_any_log_block(id: BlockId) -> bool {
    matches!(
        id,
        OAK_LOG
            | BIRCH_LOG
            | SPRUCE_LOG
            | JUNGLE_LOG
            | ACACIA_LOG
            | DARK_OAK_LOG
            | RUBBER_LOG
            | RUBBER_LOG_TAPPED
    )
}

/// Predicate: is this BlockId any species' leaves?
pub fn is_any_leaves(id: BlockId) -> bool {
    matches!(
        id,
        OAK_LEAVES
            | BIRCH_LEAVES
            | SPRUCE_LEAVES
            | JUNGLE_LEAVES
            | ACACIA_LEAVES
            | DARK_OAK_LEAVES
            | RUBBER_LEAVES
    )
}
// Satori storage block (Spec 5 §3.8). 9 Satori ↔ 1 block, round-trip.
pub const SATORI_BLOCK: BlockId = 29;
// Farming Tier 1 (Wave 26, 2026-05-17). Hoe-tilled soil that crops plant into.
pub const TILLED_SOIL: BlockId = 30;
// Crop blocks — 4 growth stages each per crop. All transparent + non-solid
// (walk-through). One block ID per stage chosen over a state-bit metadata
// scheme because state-bit support in the chunk format isn't load-bearing-
// tested yet and the registry cost is tiny (12 ids of u16's 65535 space).
pub const WHEAT_STAGE_0: BlockId = 31;
pub const WHEAT_STAGE_1: BlockId = 32;
pub const WHEAT_STAGE_2: BlockId = 33;
pub const WHEAT_STAGE_3: BlockId = 34; // mature
pub const CARROT_STAGE_0: BlockId = 35;
pub const CARROT_STAGE_1: BlockId = 36;
pub const CARROT_STAGE_2: BlockId = 37;
pub const CARROT_STAGE_3: BlockId = 38; // mature
pub const POTATO_STAGE_0: BlockId = 39;
pub const POTATO_STAGE_1: BlockId = 40;
pub const POTATO_STAGE_2: BlockId = 41;
pub const POTATO_STAGE_3: BlockId = 42; // mature
// Campfire (Wave 27, 2026-05-18). Lit emits light + cooks meat; unlit is
// inert. See foundation `2026-05-18-campfire.md`.
pub const CAMPFIRE: BlockId = 43;
pub const CAMPFIRE_UNLIT: BlockId = 44;
// Campfire smoke pillar (Wave 28, 2026-05-18). Non-solid, transparent grey
// block placed in a column above a lit campfire when leaves are burning.
// Lifecycle managed entirely by the campfire tick. See foundation
// `2026-05-18-campfire-extensions.md`.
pub const CAMPFIRE_SMOKE: BlockId = 45;
// Corn (Wave 28, 2026-05-18). Single-block-tall stereotypical crop with
// 4 growth stages matching the Wheat/Carrot/Potato pattern. Harvested ear
// (Corn) bakes at the campfire into Corn on the Cob (BakedCorn).
pub const CORN_STAGE_0: BlockId = 46;
pub const CORN_STAGE_1: BlockId = 47;
pub const CORN_STAGE_2: BlockId = 48;
pub const CORN_STAGE_3: BlockId = 49; // mature

// Village Bell (Spec 19 phase 10). Marker block — placement registers a
// 32-block radius "potential-village" zone that Wandering Villagers
// pathfind toward. Crafted from iron + sticks + planks.
pub const VILLAGE_BELL: BlockId = 50;
// Wave 29 — log-seasoning workstation. The Drying Rack stacks up to 8
// green logs and matures them into seasoned logs over ~5 real minutes
// per slot, gated by an open-air block directly above (encourages outdoor
// or open-window placement). Per-rack state lives in `World::drying_racks`.
// Note: id 51 (not the seasoning spec's originally-stated 50) because the
// villages branch landed VILLAGE_BELL at 50 first. See
// `docs/foundations/2026-05-19-log-seasoning.md`.
pub const DRYING_RACK: BlockId = 51;
// Papyrus Reed (Spec 23 — Foundation A of Build Schematics, 2026-05-19).
// Four-stage water-adjacent plant that produces papyrus reeds; reeds craft
// into papyrus sheets (first paper material in the engine, unblocks Spec
// 24's Blueprint Paper recipe). Mining a mature stage drops 1-2 reeds + resets
// the block to STAGE_0 in place (sugarcane-style auto-regrow). All four
// are transparent + non-solid so the player can wade between stalks.
// See `docs/foundations/2026-05-19-papyrus-reed.md`.
pub const PAPYRUS_STAGE_0: BlockId = 52;
pub const PAPYRUS_STAGE_1: BlockId = 53;
pub const PAPYRUS_STAGE_2: BlockId = 54;
pub const PAPYRUS_STAGE_3: BlockId = 55;
// Build Schematics Core (Spec 24 — Foundation B of Build Schematics,
// 2026-05-19). Three new blocks: BLUEPRINT_PAPER is the parchment-drafting
// square players lay around a building before right-clicking to
// capture; CONSTRUCTION_ANCHOR is *registered but never actually placed* —
// in-progress builds are tracked in `World.construction_anchors` (an in-memory
// map), not by a world block. The id is kept reserved for a future visible
// marker (engine audit 2026-06-04, E6: the old "engine-placed marker" wording
// was wrong); ARCHITECT_PLAQUE
// auto-places last in every animated build carrying the derivation
// chain (admin-protected). See `docs/foundations/2026-05-19-build-schematics-core.md`.
pub const BLUEPRINT_PAPER: BlockId = 56;
pub const CONSTRUCTION_ANCHOR: BlockId = 57;
pub const ARCHITECT_PLAQUE: BlockId = 58;

// Spec 20 (Furnace foundation). FURNACE is the unlit / idle state;
// FURNACE_LIT shows the glow while a smelt is in progress. Both
// blocks share top + bottom textures and only swap the side
// (front-glow vs front-dark). State lives in `World::block_entities`
// as `BlockEntityData::Furnace(FurnaceData)` per Spec 20 Phase 2.
pub const FURNACE: BlockId = 59;
pub const FURNACE_LIT: BlockId = 60;

// Texture array layer indices (must match texture_gen::generate_textures order)
pub const TEX_STONE: u32 = 0;
pub const TEX_DIRT: u32 = 1;
pub const TEX_GRASS_TOP: u32 = 2;
pub const TEX_GRASS_SIDE: u32 = 3;
pub const TEX_BEDROCK: u32 = 4;
pub const TEX_SAND: u32 = 5;
pub const TEX_OAK_LOG_SIDE: u32 = 6;
pub const TEX_OAK_LOG_TOP: u32 = 7;
pub const TEX_OAK_LEAVES: u32 = 8;
pub const TEX_OAK_PLANKS: u32 = 9;
pub const TEX_COBBLESTONE: u32 = 10;
pub const TEX_WATER: u32 = 11;
pub const TEX_GRAVEL: u32 = 12;
pub const TEX_SANDSTONE: u32 = 13;
pub const TEX_SNOW: u32 = 14;
pub const TEX_CRAFTING_TABLE_TOP: u32 = 15;
pub const TEX_BED_TOP: u32 = 84;
pub const TEX_BED_SIDE: u32 = 85;
pub const TEX_COAL_ORE: u32 = 86;
pub const TEX_IRON_ORE: u32 = 87;
pub const TEX_DIAMOND_ORE: u32 = 88;
pub const TEX_GLASS: u32 = 93;
pub const TEX_COAL_BLOCK: u32 = 94;
pub const TEX_IRON_BLOCK: u32 = 95;
pub const TEX_DIAMOND_BLOCK: u32 = 96;
pub const TEX_TORCH: u32 = 97;
pub const TEX_TALL_GRASS: u32 = 103;
// Deepslate + Satori block textures (Wave 25 — layers 106..=110)
pub const TEX_PURE_DEEPSLATE: u32 = 106;
pub const TEX_DEEPSLATE_COAL_ORE: u32 = 107;
pub const TEX_DEEPSLATE_IRON_ORE: u32 = 108;
pub const TEX_DEEPSLATE_DIAMOND_ORE: u32 = 109;
pub const TEX_SATORI_BLOCK: u32 = 110;
// Satori item-drop texture (layer 111)
pub const TEX_ITEM_SATORI: u32 = 111;
// Farming Tier 1 — tilled soil top face (Wave 26, 2026-05-17).
pub const TEX_TILLED_SOIL: u32 = 112;
// Farming Tier 1 item drops (Wave 26 — layers 113-117).
pub const TEX_ITEM_WHEAT_SEEDS: u32 = 113;
pub const TEX_ITEM_WHEAT: u32 = 114;
pub const TEX_ITEM_BREAD: u32 = 115;
pub const TEX_ITEM_CARROT: u32 = 116;
pub const TEX_ITEM_POTATO: u32 = 117;
// Crop-stage textures (Wave 26 — layers 118-129).
pub const TEX_WHEAT_STAGE_0: u32 = 118;
pub const TEX_WHEAT_STAGE_1: u32 = 119;
pub const TEX_WHEAT_STAGE_2: u32 = 120;
pub const TEX_WHEAT_STAGE_3: u32 = 121;
pub const TEX_CARROT_STAGE_0: u32 = 122;
pub const TEX_CARROT_STAGE_1: u32 = 123;
pub const TEX_CARROT_STAGE_2: u32 = 124;
pub const TEX_CARROT_STAGE_3: u32 = 125;
pub const TEX_POTATO_STAGE_0: u32 = 126;
pub const TEX_POTATO_STAGE_1: u32 = 127;
pub const TEX_POTATO_STAGE_2: u32 = 128;
pub const TEX_POTATO_STAGE_3: u32 = 129;
// Campfire textures (Wave 27 — layers 130-133).
pub const TEX_CAMPFIRE_LIT_TOP: u32 = 130;
pub const TEX_CAMPFIRE_LIT_SIDE: u32 = 131;
pub const TEX_CAMPFIRE_UNLIT_TOP: u32 = 132;
pub const TEX_CAMPFIRE_UNLIT_SIDE: u32 = 133;
// Wave 27 — flint item drop + flint-and-steel tool.
pub const TEX_ITEM_FLINT: u32 = 134;
// Layer 135 (flint-and-steel tool icon) is generated in texture_gen.rs and
// registered by name in texture_registry.rs; the tool-icon lookup (hud_ui.rs /
// craft_ui.rs) currently renders a plain "F" glyph rather than this texture,
// so the named constant had no consumer — removed rather than kept as dead
// weight. Re-add if the icon lookup switches to it.
// Wave 28 — campfire smoke pillar (single layer, grey alpha wisp).
pub const TEX_CAMPFIRE_SMOKE: u32 = 136;
// Wave 28 — corn crop stages (layers 137-140) + corn item drops (141-143).
pub const TEX_CORN_STAGE_0: u32 = 137;
pub const TEX_CORN_STAGE_1: u32 = 138;
pub const TEX_CORN_STAGE_2: u32 = 139;
pub const TEX_CORN_STAGE_3: u32 = 140;
pub const TEX_ITEM_CORN_SEEDS: u32 = 141;
pub const TEX_ITEM_CORN: u32 = 142;
pub const TEX_ITEM_BAKED_CORN: u32 = 143;
// Wave 28 — baked potato + baked carrot item drops (layers 144-145).
pub const TEX_ITEM_BAKED_POTATO: u32 = 144;
pub const TEX_ITEM_BAKED_CARROT: u32 = 145;
// Wave 29 — Drying Rack textures (layers 158-159; villages took 146-157).
pub const TEX_DRYING_RACK_TOP: u32 = 158;
pub const TEX_DRYING_RACK_SIDE: u32 = 159;
// Papyrus Reed (Spec 23 — Foundation A, 2026-05-19). Four stage textures
// (160-163) + two item-drop textures (164-165: reed material + sheet
// material). Generated procedurally in `texture_gen.rs`.
pub const TEX_PAPYRUS_STAGE_0: u32 = 160;
pub const TEX_PAPYRUS_STAGE_1: u32 = 161;
pub const TEX_PAPYRUS_STAGE_2: u32 = 162;
pub const TEX_PAPYRUS_STAGE_3: u32 = 163;
pub const TEX_ITEM_PAPYRUS_REED: u32 = 164;
pub const TEX_ITEM_PAPYRUS_SHEET: u32 = 165;
// Build Schematics Core (Spec 24 — Foundation B, 2026-05-19). Three
// blocks × top+side (BLUEPRINT_PAPER / CONSTRUCTION_ANCHOR / ARCHITECT_PLAQUE)
// = 6 layers. Construction Anchor shares one texture across faces (it's
// a surveyor's flag). Blueprint Paper + Architect's Plaque get distinct top
// + side textures since both are visually meaningful as workstation
// blocks (the top is the parchment face).
pub const TEX_BLUEPRINT_PAPER_TOP: u32 = 166;
pub const TEX_BLUEPRINT_PAPER_SIDE: u32 = 167;
pub const TEX_CONSTRUCTION_ANCHOR: u32 = 168;
pub const TEX_ARCHITECT_PLAQUE_TOP: u32 = 169;
pub const TEX_ARCHITECT_PLAQUE_SIDE: u32 = 170;
// Spec 20 (Furnace) — top shared between lit and unlit (an iron-banded
// stone slab from above), side texture differs (front-dark hollow vs
// orange-glow rectangle). Layers 171-173.
pub const TEX_FURNACE_TOP: u32 = 171;
pub const TEX_FURNACE_SIDE_UNLIT: u32 = 172;
pub const TEX_FURNACE_SIDE_LIT: u32 = 173;
// Spec 16 Phase 3b — deepslate visual variants. Tier-ascending: THIN
// has a subtle blue speckle, HEALTHY shows visible blue-orange veins,
// FAT is dense and glowing. Layers 174-176.
pub const TEX_PURE_DEEPSLATE_THIN: u32 = 174;
pub const TEX_PURE_DEEPSLATE_HEALTHY: u32 = 175;
pub const TEX_PURE_DEEPSLATE_FAT: u32 = 176;
// Spec 21 Vendor Block — chest-style wood top with iron coin-slot
// motif on the front. Single texture used for all faces on alpha
// (front-vs-back face distinction is Phase 11 polish). Layer 177.
pub const TEX_VENDOR_BLOCK: u32 = 177;
// Spec 26 Drafting Table — parchment-on-wood top + plain-wood sides.
// Single texture for all faces on alpha. Layer 178.
pub const TEX_DRAFTING_TABLE: u32 = 178;
// Spec 22 Phase 7 — red-tinted variant of the campfire smoke pillar used
// while the source campfire's `raid_warning_active` flag is set. Mesh
// builder swaps to this layer per-cell at build time; no new BlockId
// (per open question 1: a flag on the existing CAMPFIRE_SMOKE block).
// Layer 179.
pub const TEX_CAMPFIRE_SMOKE_WARNING: u32 = 179;
// HP-2 (2026-05-22) — Chest. Wood-grain side with iron clasps + a darker
// lid-line on the top. Single texture per face axis: top has the lid
// line, side has the clasp + keyhole motif.
pub const TEX_CHEST_TOP: u32 = 180;
pub const TEX_CHEST_SIDE: u32 = 181;
// #15 — tiered-storage chest textures (377-384), tinted per tier.
pub const TEX_COPPER_CHEST_TOP: u32 = 377;
pub const TEX_COPPER_CHEST_SIDE: u32 = 378;
pub const TEX_IRON_CHEST_TOP: u32 = 379;
pub const TEX_IRON_CHEST_SIDE: u32 = 380;
pub const TEX_DIAMOND_CHEST_TOP: u32 = 381;
pub const TEX_DIAMOND_CHEST_SIDE: u32 = 382;
pub const TEX_SATORI_CHEST_TOP: u32 = 383;
pub const TEX_SATORI_CHEST_SIDE: u32 = 384;
// Spec 48 (Electricity) — power-block textures (385..=396).
pub const TEX_CABLE: u32 = 385;
pub const TEX_CABLE_LIT: u32 = 386;
pub const TEX_ELECTRIC_LAMP: u32 = 387;
pub const TEX_ELECTRIC_LAMP_LIT: u32 = 388;
pub const TEX_LEVER: u32 = 389;
pub const TEX_BUTTON: u32 = 390;
pub const TEX_PRESSURE_PLATE: u32 = 391;
pub const TEX_LOGIC_GATE: u32 = 392;
pub const TEX_HAND_CRANK: u32 = 393;
pub const TEX_STEAM_GENERATOR: u32 = 394;
pub const TEX_STEAM_GENERATOR_LIT: u32 = 395;
pub const TEX_BATTERY: u32 = 396;
// Spec 48 Phase 2 — sensor textures (397..=399).
pub const TEX_BEAM_SENSOR: u32 = 397;
pub const TEX_MIRROR: u32 = 398;
pub const TEX_MOTION_SENSOR: u32 = 399;
// Spec 49 (Explosives) — ore / workstation / keg / plunger textures (400..=404).
pub const TEX_BRIMSTONE: u32 = 400;
pub const TEX_NITRE_ORE: u32 = 401;
pub const TEX_COMPOSTER: u32 = 402;
pub const TEX_BLASTING_KEG: u32 = 403;
pub const TEX_PLUNGER_DETONATOR: u32 = 404;

/// HP-3 — Brigand Hideout Banner. Procedural purple-and-red banner motif
/// with a wooden post; rendered as a transparent decorative cube at the
/// hideout palisade gate.
pub const TEX_BRIGAND_HIDEOUT_BANNER: u32 = 182;

/// HP-3 v2 — Trophy Wall side. Wood-grain plaque with a stylised
/// purple-and-red Chieftain trophy mounted at the centre.
pub const TEX_TROPHY_WALL: u32 = 183;

/// Salt feature texture layers (2026-05-23).
pub const TEX_ROCK_SALT: u32 = 184;
pub const TEX_SALT_LICK: u32 = 185;
pub const TEX_SALT_LAMP: u32 = 186;
pub const TEX_SALT_BLOCK: u32 = 187;
pub const TEX_SALT_PATH_TOP: u32 = 188;
pub const TEX_SALT_PATH_SIDE: u32 = 189;

/// Rubber feature texture layers (2026-05-23).
pub const TEX_RUBBER_LOG: u32 = 190;
pub const TEX_RUBBER_PLANKS: u32 = 191;
pub const TEX_RUBBER_LEAVES: u32 = 192;
pub const TEX_RUBBER_LOG_TAPPED: u32 = 193;

/// Mob Bounty Board texture layer (Spec 33, 2026-05-23).
pub const TEX_BOUNTY_BOARD: u32 = 194;

/// Tip Jar texture layer (Spec 34, 2026-05-23).
pub const TEX_TIP_JAR: u32 = 195;

/// Repair Bench texture layer (Spec 35, 2026-05-23).
pub const TEX_REPAIR_BENCH: u32 = 196;

/// Plot Marker texture layer (Spec 36, 2026-05-23).
pub const TEX_PLOT_MARKER: u32 = 197;

/// Market Bell texture layer (Spec 37, 2026-05-23).
pub const TEX_MARKET_BELL: u32 = 198;

/// Auction Block texture layer (Spec 38, 2026-05-23).
pub const TEX_AUCTION_BLOCK: u32 = 199;

/// Bazaar Block texture layer (Spec 39, 2026-05-23).
pub const TEX_BAZAAR_BLOCK: u32 = 200;

/// Block-break crack overlay (Spec 05 §2.2). Ten stages occupy layers
/// 207-216; stage N (0 = hairline, 9 = heavy fracturing) is `TEX_CRACK_BASE + N`.
/// Drawn over the targeted block by the crack render pass, not a block's own
/// face texture.
pub const TEX_CRACK_BASE: u32 = 207;

// Spec 35/36 plant block textures (2026-05-27 — layers 220-224). Sit
// after the bee/squid coats (217-219, in entity_model.rs). Item textures
// for the dyes + fibre live in entity_model.rs (225-230).
pub const TEX_CORNFLOWER: u32 = 220;
pub const TEX_FIELD_POPPY: u32 = 221;
pub const TEX_BUTTERCUP: u32 = 222;
pub const TEX_COTTON_PLANT: u32 = 223;
pub const TEX_HEMP_PLANT: u32 = 224;

// Spec 37 Magnesium ore block texture (2026-05-27 — layer 231; item icons
// 232-236 live in entity_model.rs).
pub const TEX_MAGNESIUM_ORE: u32 = 231;

// Spec 36 Phase 2 fibre crop-stage textures (2026-05-27 — layers 247-254;
// dye Phase 2 items take 237-246). Seed item icons (255-256) are in
// entity_model.rs.
pub const TEX_COTTON_STAGE_0: u32 = 247;
pub const TEX_COTTON_STAGE_1: u32 = 248;
pub const TEX_COTTON_STAGE_2: u32 = 249;
pub const TEX_COTTON_STAGE_3: u32 = 250;
pub const TEX_HEMP_STAGE_0: u32 = 251;
pub const TEX_HEMP_STAGE_1: u32 = 252;
pub const TEX_HEMP_STAGE_2: u32 = 253;
pub const TEX_HEMP_STAGE_3: u32 = 254;

// Coloured wallpaper block textures (2026-05-27 — layers 257-269; seed item
// icons took 255-256). One per dye colour, block order white..light_grey.
pub const TEX_WALLPAPER_WHITE: u32 = 257;
pub const TEX_WALLPAPER_BLACK: u32 = 258;
pub const TEX_WALLPAPER_RED: u32 = 259;
pub const TEX_WALLPAPER_BLUE: u32 = 260;
pub const TEX_WALLPAPER_YELLOW: u32 = 261;
pub const TEX_WALLPAPER_ORANGE: u32 = 262;
pub const TEX_WALLPAPER_GREEN: u32 = 263;
pub const TEX_WALLPAPER_PURPLE: u32 = 264;
pub const TEX_WALLPAPER_PINK: u32 = 265;
pub const TEX_WALLPAPER_LIME: u32 = 266;
pub const TEX_WALLPAPER_LIGHT_BLUE: u32 = 267;
pub const TEX_WALLPAPER_GREY: u32 = 268;
pub const TEX_WALLPAPER_LIGHT_GREY: u32 = 269;
// Spec 35 Phase 2 completion (2026-05-28) — wallpaper textures for the
// three new dyes. Slots 273-275 sit just after the dye-item icons at
// 270-272 (see `entity_model.rs`).
pub const TEX_WALLPAPER_BROWN: u32 = 273;
pub const TEX_WALLPAPER_CYAN: u32 = 274;
pub const TEX_WALLPAPER_MAGENTA: u32 = 275;
// Spec 35 farmable-flower follow-on (2026-05-28). 9 stage textures
// (279-287) + 3 seed item icons (288-290) sit after Spec 36 Phase 2's
// rope/cloth/canvas at 276-278.
pub const TEX_CORNFLOWER_STAGE_0: u32 = 279;
pub const TEX_CORNFLOWER_STAGE_1: u32 = 280;
pub const TEX_CORNFLOWER_STAGE_2: u32 = 281;
pub const TEX_FIELD_POPPY_STAGE_0: u32 = 282;
pub const TEX_FIELD_POPPY_STAGE_1: u32 = 283;
pub const TEX_FIELD_POPPY_STAGE_2: u32 = 284;
pub const TEX_BUTTERCUP_STAGE_0: u32 = 285;
pub const TEX_BUTTERCUP_STAGE_1: u32 = 286;
pub const TEX_BUTTERCUP_STAGE_2: u32 = 287;
// Spec 38 cyanotype-art variant (2026-05-28). Single texture for v1
// — blueprint-blue background with a white frame border. Layer 291
// follows the farmable-flower seed icons at 288-290.
pub const TEX_CYANOTYPE_PRINT: u32 = 291;
// Spec 35 dyed-décor — Bunting textures (2026-05-28). One per colour
// (16 layers, 292-307). Hand-built via `gen_bunting` — coloured
// triangle pattern on a transparent background.
pub const TEX_BUNTING_WHITE: u32 = 292;
pub const TEX_BUNTING_BLACK: u32 = 293;
pub const TEX_BUNTING_RED: u32 = 294;
pub const TEX_BUNTING_BLUE: u32 = 295;
pub const TEX_BUNTING_YELLOW: u32 = 296;
pub const TEX_BUNTING_ORANGE: u32 = 297;
pub const TEX_BUNTING_GREEN: u32 = 298;
pub const TEX_BUNTING_PURPLE: u32 = 299;
pub const TEX_BUNTING_PINK: u32 = 300;
pub const TEX_BUNTING_LIME: u32 = 301;
pub const TEX_BUNTING_LIGHT_BLUE: u32 = 302;
pub const TEX_BUNTING_GREY: u32 = 303;
pub const TEX_BUNTING_LIGHT_GREY: u32 = 304;
pub const TEX_BUNTING_BROWN: u32 = 305;
pub const TEX_BUNTING_CYAN: u32 = 306;
pub const TEX_BUNTING_MAGENTA: u32 = 307;
// Spec 35 dyed-décor — Paper Lantern textures (2026-05-28). 16 layers
// (308-323). `gen_paper_lantern` paints a dye-tinted paper face with a
// soft glow gradient + faint vertical seams (the paper folds).
pub const TEX_PAPER_LANTERN_WHITE: u32 = 308;
pub const TEX_PAPER_LANTERN_BLACK: u32 = 309;
pub const TEX_PAPER_LANTERN_RED: u32 = 310;
pub const TEX_PAPER_LANTERN_BLUE: u32 = 311;
pub const TEX_PAPER_LANTERN_YELLOW: u32 = 312;
pub const TEX_PAPER_LANTERN_ORANGE: u32 = 313;
pub const TEX_PAPER_LANTERN_GREEN: u32 = 314;
pub const TEX_PAPER_LANTERN_PURPLE: u32 = 315;
pub const TEX_PAPER_LANTERN_PINK: u32 = 316;
pub const TEX_PAPER_LANTERN_LIME: u32 = 317;
pub const TEX_PAPER_LANTERN_LIGHT_BLUE: u32 = 318;
pub const TEX_PAPER_LANTERN_GREY: u32 = 319;
pub const TEX_PAPER_LANTERN_LIGHT_GREY: u32 = 320;
pub const TEX_PAPER_LANTERN_BROWN: u32 = 321;
pub const TEX_PAPER_LANTERN_CYAN: u32 = 322;
pub const TEX_PAPER_LANTERN_MAGENTA: u32 = 323;
// Spec 35 dyed-décor — Kite textures (2026-05-28). 16 layers (324-339).
// Hand-built `gen_kite` — diamond-shape kite body + cross-spars + tail.
pub const TEX_KITE_WHITE: u32 = 324;
pub const TEX_KITE_BLACK: u32 = 325;
pub const TEX_KITE_RED: u32 = 326;
pub const TEX_KITE_BLUE: u32 = 327;
pub const TEX_KITE_YELLOW: u32 = 328;
pub const TEX_KITE_ORANGE: u32 = 329;
pub const TEX_KITE_GREEN: u32 = 330;
pub const TEX_KITE_PURPLE: u32 = 331;
pub const TEX_KITE_PINK: u32 = 332;
pub const TEX_KITE_LIME: u32 = 333;
pub const TEX_KITE_LIGHT_BLUE: u32 = 334;
pub const TEX_KITE_GREY: u32 = 335;
pub const TEX_KITE_LIGHT_GREY: u32 = 336;
pub const TEX_KITE_BROWN: u32 = 337;
pub const TEX_KITE_CYAN: u32 = 338;
pub const TEX_KITE_MAGENTA: u32 = 339;
// Banner textures (2026-05-28). 16 layers (340-355). `gen_banner`
// paints a dyed flag occupying the top ~2/3 of the face + a wooden
// shaft below.
pub const TEX_BANNER_WHITE: u32 = 340;
pub const TEX_BANNER_BLACK: u32 = 341;
pub const TEX_BANNER_RED: u32 = 342;
pub const TEX_BANNER_BLUE: u32 = 343;
pub const TEX_BANNER_YELLOW: u32 = 344;
pub const TEX_BANNER_ORANGE: u32 = 345;
pub const TEX_BANNER_GREEN: u32 = 346;
pub const TEX_BANNER_PURPLE: u32 = 347;
pub const TEX_BANNER_PINK: u32 = 348;
pub const TEX_BANNER_LIME: u32 = 349;
pub const TEX_BANNER_LIGHT_BLUE: u32 = 350;
pub const TEX_BANNER_GREY: u32 = 351;
pub const TEX_BANNER_LIGHT_GREY: u32 = 352;
pub const TEX_BANNER_BROWN: u32 = 353;
pub const TEX_BANNER_CYAN: u32 = 354;
pub const TEX_BANNER_MAGENTA: u32 = 355;
// Sail textures (2026-05-28). 16 layers (356-371). `gen_sail` paints
// a dyed canvas face with cross-hatch weave for the sailcloth look.
pub const TEX_SAIL_WHITE: u32 = 356;
pub const TEX_SAIL_BLACK: u32 = 357;
pub const TEX_SAIL_RED: u32 = 358;
pub const TEX_SAIL_BLUE: u32 = 359;
pub const TEX_SAIL_YELLOW: u32 = 360;
pub const TEX_SAIL_ORANGE: u32 = 361;
pub const TEX_SAIL_GREEN: u32 = 362;
pub const TEX_SAIL_PURPLE: u32 = 363;
pub const TEX_SAIL_PINK: u32 = 364;
pub const TEX_SAIL_LIME: u32 = 365;
pub const TEX_SAIL_LIGHT_BLUE: u32 = 366;
pub const TEX_SAIL_GREY: u32 = 367;
pub const TEX_SAIL_LIGHT_GREY: u32 = 368;
pub const TEX_SAIL_BROWN: u32 = 369;
pub const TEX_SAIL_CYAN: u32 = 370;
pub const TEX_SAIL_MAGENTA: u32 = 371;
// Tent texture (2026-05-28). Single layer; canvas dome with
// a ridgepole + two guy-line stays.
pub const TEX_TENT: u32 = 372;
// Track texture (Rail freight Phase 1). Single layer; two parallel
// steel rails over a sleeper base — the flat ground-slab rail face.
pub const TEX_TRACK: u32 = 373;
// Rail auto-connect (2026-07-02) — east-west rail texture (gen_track rotated
// 90°), appended at index 444. Registered by name ("blocks/track_ew") in
// texture_registry.rs rather than through this constant, so the named
// constant had no consumer — removed rather than kept as dead weight.
/// Rail/cable connecting-geometry v2 — clean solid metal colours (appended layers).
pub const TEX_RAIL_STEEL: u32 = 445;
pub const TEX_CABLE_COPPER: u32 = 446;
pub const TEX_CABLE_COPPER_LIT: u32 = 447;
/// Sleeper/ballast bed drawn under the rails (appended layer).
pub const TEX_RAIL_BASE: u32 = 448;
/// Fire (2026-07-04) — procedural licking-flame noise, all faces.
pub const TEX_FIRE: u32 = 449;
/// Dispenser/Dropper (2026-07-04) — furnace-style metal face with a round
/// muzzle hole (dispenser dark bore, dropper shallow dish).
pub const TEX_DISPENSER_SIDE: u32 = 450;
pub const TEX_DROPPER_SIDE: u32 = 451;
/// Sapling (2026-07-04) — shared cross-quad-ish sprout texture; species read
/// comes from the per-block tint colour.
pub const TEX_SAPLING: u32 = 452;
/// Particle framework (2026-07-05) — billboard textures. SOFT = radial alpha
/// blob (smoke/splash/snow/puffs), SPARK = hot dot (embers), STREAK = thin
/// vertical rain line, CHIP = hard square (block-break debris, tinted by the
/// instance colour).
pub const TEX_PARTICLE_SOFT: u32 = 453;
pub const TEX_PARTICLE_SPARK: u32 = 454;
pub const TEX_PARTICLE_STREAK: u32 = 455;
pub const TEX_PARTICLE_CHIP: u32 = 456;
/// Pet Bed (2026-07-06 pets wave) — a wool cushion in a wood-crate frame,
/// distinct from the player BED's pillow-and-mattress read (smaller/rounder
/// cushion, no pillow). TOP = cushion + frame corners, SIDE = wood crate
/// with a red cushion band along the rim.
pub const TEX_PET_BED_TOP: u32 = 457;
pub const TEX_PET_BED_SIDE: u32 = 458;

/// Water Wheel layers (Spec 48 Phase 4, 2026-09-06). These append AFTER the
/// species-tint run, so they start at `texture_gen::POST_TINT_LAYER_START`
/// (506) rather than continuing the 400-block run — the tint bake owns
/// 459..=505 and must stay contiguous. Lock-step with `generate_textures`.
pub const TEX_WATER_WHEEL: u32 = 506;
pub const TEX_WATER_WHEEL_TURNING: u32 = 507;
/// Copper Ore (Wind, Copper & Electricity wave, 2026-09-07, Spec 02 §1) —
/// procedural stone base with copper-orange flecks, same family as
/// `gen_iron_ore`. Appended after the Water Wheel layers (same "post-tint
/// block layer" reason: minted after the species-tint bake, so it can't live
/// in the contiguous 400-block run). Lock-step with `generate_textures`.
pub const TEX_COPPER_ORE: u32 = 508;
/// Windmill layers (Wind, Copper & Electricity wave §2.2, 2026-09-07) — four
/// canvas sails on a plank tower face; the turning twin smears them round.
/// Appended after the Copper Ore layer for the same post-tint reason. Top and
/// bottom faces reuse `TEX_OAK_PLANKS` (the tower cap), so the pair costs two
/// layers, taking the atlas to 511 of the 512-layer device floor. Lock-step
/// with `generate_textures`.
pub const TEX_WINDMILL: u32 = 509;
pub const TEX_WINDMILL_TURNING: u32 = 510;
// #30 building-detail blocks (2026-06-16). Ladder = oak rails + rungs with
// transparent gaps; Carpet = near-white tintable wool weave.
pub const TEX_LADDER: u32 = 374;
pub const TEX_CARPET: u32 = 375;
// #47 — Grave headstone texture (weathered stone slab on an earth mound).
pub const TEX_GRAVE: u32 = 376;

/// Spec 16 Phase 3 — true for any block that mining-mechanically counts
/// as "pure deepslate": the canonical `PURE_DEEPSLATE` (id 25), and —
/// when Phase 3b lands — the three visual-variant tiers (THIN / HEALTHY
/// / FAT, ids 61-63).
///
/// All variants share identical gameplay identity: same break time, same
/// tool requirement (Stone+), same mine-drop (PURE_DEEPSLATE item), same
/// Satori-vein spawn eligibility, same Proof-of-Play behaviour (the
/// HMAC at a fixed position is variant-independent by construction —
/// see the `proof_hash_independent_of_deepslate_variant` regression in
/// `proof_of_play.rs::tests`). Only the texture coordinates differ.
///
/// Use this helper anywhere that special-cases PURE_DEEPSLATE for a
/// gameplay-level reason (mining time, tool gate, ore-spawn eligibility,
/// proof-of-play deepslate detection). Reserve direct `== PURE_DEEPSLATE`
/// equality for cases where you specifically mean "the canonical /
/// default variant" — e.g., the mine-drop arm normalises to that one
/// block-id regardless of which variant the player struck.
///
/// **Phase 3b delivery (2026-05-20)**: helper now matches all four
/// pure-deepslate variants. The mining-mechanic-parity invariant
/// (same HMAC roll regardless of variant) is locked by
/// `proof_hash_independent_of_deepslate_variant` in `proof_of_play.rs`.
#[inline]
pub fn is_pure_deepslate_family(id: BlockId) -> bool {
    matches!(
        id,
        PURE_DEEPSLATE | PURE_DEEPSLATE_THIN | PURE_DEEPSLATE_HEALTHY | PURE_DEEPSLATE_FAT,
    )
}

/// Owner-inbox #1/2/3 (2026-06-03) — is `id` one of the 16 solid wallpaper
/// décor blocks (the painter for a face-overlay)? The ids are 145..=157 and
/// 159..=161; `LATENT_PRINT = 158` splits the range and must be excluded.
#[inline]
pub fn is_wallpaper(id: BlockId) -> bool {
    (WALLPAPER_WHITE..=WALLPAPER_LIGHT_GREY).contains(&id)
        || (WALLPAPER_BROWN..=WALLPAPER_MAGENTA).contains(&id)
}

/// The 16 wallpaper "paint colours" in a FIXED order. Painting a dye in the
/// Workshop stores its wallpaper block; the micro-model bake maps that block to
/// a flat SOLID-COLOUR texture layer (so a painted pixel is one solid colour,
/// not the wallpaper's decorative pattern). Kept in lockstep with the solid
/// layers pushed in `texture_gen` (same order) + [`wallpaper_solid_layer`].
pub const PAINT_WALLPAPERS: [BlockId; 16] = [
    WALLPAPER_WHITE, WALLPAPER_BLACK, WALLPAPER_RED, WALLPAPER_BLUE,
    WALLPAPER_YELLOW, WALLPAPER_ORANGE, WALLPAPER_GREEN, WALLPAPER_PURPLE,
    WALLPAPER_PINK, WALLPAPER_LIME, WALLPAPER_LIGHT_BLUE, WALLPAPER_GREY,
    WALLPAPER_LIGHT_GREY, WALLPAPER_BROWN, WALLPAPER_CYAN, WALLPAPER_MAGENTA,
];

/// First texture layer of the 16 solid paint-colour fills, appended after the
/// last block texture. MUST equal the number of block/item textures pushed
/// before the fills (405). The per-species leaf layers below (421..=425) are
/// pushed AFTER the fills, so they don't disturb this base.
pub const TEX_PAINT_SOLID_BASE: u32 = 405;

// #130 — per-species leaf textures (layers 421..=425), appended after the 16
// Workshop paint-solid fills (405..=420). Retinted oak-leaf noise so birch /
// spruce / jungle / acacia / dark-oak foliage reads distinct instead of one
// flat oak green. Lock-step with `texture_gen::generate_textures` + `TEXTURE_KEYS`.
pub const TEX_BIRCH_LEAVES: u32 = 421;
pub const TEX_SPRUCE_LEAVES: u32 = 422;
pub const TEX_JUNGLE_LEAVES: u32 = 423;
pub const TEX_ACACIA_LEAVES: u32 = 424;
pub const TEX_DARK_OAK_LEAVES: u32 = 425;

// #132 — per-species WOOD textures (layers 426..=440), appended after the leaf
// layers. Finishes per-species trees: logs + planks no longer alias oak.
// Lock-step with `texture_gen::generate_textures` + `TEXTURE_KEYS`.
pub const TEX_BIRCH_LOG_SIDE: u32 = 426;
pub const TEX_BIRCH_LOG_TOP: u32 = 427;
pub const TEX_BIRCH_PLANKS: u32 = 428;
pub const TEX_SPRUCE_LOG_SIDE: u32 = 429;
pub const TEX_SPRUCE_LOG_TOP: u32 = 430;
pub const TEX_SPRUCE_PLANKS: u32 = 431;
pub const TEX_JUNGLE_LOG_SIDE: u32 = 432;
pub const TEX_JUNGLE_LOG_TOP: u32 = 433;
pub const TEX_JUNGLE_PLANKS: u32 = 434;
pub const TEX_ACACIA_LOG_SIDE: u32 = 435;
pub const TEX_ACACIA_LOG_TOP: u32 = 436;
pub const TEX_ACACIA_PLANKS: u32 = 437;
pub const TEX_DARK_OAK_LOG_SIDE: u32 = 438;
pub const TEX_DARK_OAK_LOG_TOP: u32 = 439;
pub const TEX_DARK_OAK_PLANKS: u32 = 440;

/// The flat solid-colour texture layer for a wallpaper paint block (Workshop
/// micro-model paint), or `None` if `block` isn't a paint wallpaper.
pub fn wallpaper_solid_layer(block: BlockId) -> Option<u32> {
    PAINT_WALLPAPERS
        .iter()
        .position(|&b| b == block)
        .map(|i| TEX_PAINT_SOLID_BASE + i as u32)
}

/// Static block properties.
#[derive(Clone, Debug)]
pub struct BlockDef {
    pub name: &'static str,
    pub solid: bool,
    pub transparent: bool,
    /// Falls when unsupported (e.g. sand, gravel).
    pub gravity: bool,
    /// RGB color for hotbar HUD display.
    pub color: [f32; 3],
    /// Texture layer indices: top face, bottom face, side faces.
    pub tex_top: u32,
    pub tex_bottom: u32,
    pub tex_side: u32,
}

/// Phase 4 (third-person camera) — how the camera treats a block when its
/// pull-back ray would pass through it. **Derived per block from
/// `solid`+`transparent`** (no per-block authoring): it keys off registry INTENT,
/// not the visual box, which is the day-one fix for Minecraft's Glass-vs-Barrier
/// inconsistency (MC-189617/175927). Engine-generic (cross-game). Render-only —
/// the interaction/aim ray is never affected.
/// Spec: `docs/foundations/2026-06-09-third-person-camera.md` (Phase 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraOcclusion {
    /// Opaque solid — squeeze the camera in before it (Phase-2 collision).
    Squeeze,
    /// See-through or non-solid — the camera passes straight through (you can see
    /// past it, so it must never clip the view). Glass, leaves, fences, air.
    PassThrough,
    /// Reserved — orbit the camera around the block instead of squeezing. Until
    /// the rotate path exists it falls back to collide-like (Squeeze) behaviour.
    #[allow(dead_code)] // reserved tag, not yet emitted by the derived default
    RotateAround,
    /// Reserved — don't move the camera; fade the avatar instead (Phase 3).
    #[allow(dead_code)] // reserved tag, not yet emitted by the derived default
    FadeOnly,
}

impl CameraOcclusion {
    /// The default occlusion for a block, derived from its `solid`+`transparent`
    /// flags — no per-block authoring. Opaque solids squeeze; anything you can see
    /// through (transparent) or walk through (non-solid) passes through.
    pub fn default_for(solid: bool, transparent: bool) -> Self {
        if solid && !transparent {
            CameraOcclusion::Squeeze
        } else {
            CameraOcclusion::PassThrough
        }
    }

    /// Whether the camera-collision ray stops at this block (pushing the camera
    /// in). Squeeze and the reserved RotateAround collide; PassThrough and
    /// FadeOnly let the camera through.
    pub fn collides(self) -> bool {
        matches!(self, CameraOcclusion::Squeeze | CameraOcclusion::RotateAround)
    }
}

/// Block registry — indexed by BlockId for O(1) lookup.
pub struct BlockRegistry {
    blocks: Vec<BlockDef>,
}

impl BlockRegistry {
    pub fn new() -> Self {
        let mut blocks = vec![

        // 0: Air
        BlockDef {
            name: "genesis:air",
            solid: false,
            transparent: true,
            gravity: false,
            color: [0.0, 0.0, 0.0],
            tex_top: 0, tex_bottom: 0, tex_side: 0,
        },

        // 1: Stone
        BlockDef {
            name: "genesis:stone",
            solid: true,
            transparent: false,
            gravity: false,
            color: [0.5, 0.5, 0.5],
            tex_top: TEX_STONE, tex_bottom: TEX_STONE, tex_side: TEX_STONE,
        },

        // 2: Dirt
        BlockDef {
            name: "genesis:dirt",
            solid: true,
            transparent: false,
            gravity: false,
            color: [0.53, 0.33, 0.18],
            tex_top: TEX_DIRT, tex_bottom: TEX_DIRT, tex_side: TEX_DIRT,
        },

        // 3: Grass — green top, dirt bottom, striped sides
        BlockDef {
            name: "genesis:grass",
            solid: true,
            transparent: false,
            gravity: false,
            color: [0.36, 0.6, 0.22],
            tex_top: TEX_GRASS_TOP, tex_bottom: TEX_DIRT, tex_side: TEX_GRASS_SIDE,
        },

        // 4: Bedrock
        BlockDef {
            name: "genesis:bedrock",
            solid: true,
            transparent: false,
            gravity: false,
            color: [0.2, 0.2, 0.2],
            tex_top: TEX_BEDROCK, tex_bottom: TEX_BEDROCK, tex_side: TEX_BEDROCK,
        },

        // 5: Sand
        BlockDef {
            name: "genesis:sand",
            solid: true,
            transparent: false,
            gravity: true,
            color: [0.86, 0.82, 0.62],
            tex_top: TEX_SAND, tex_bottom: TEX_SAND, tex_side: TEX_SAND,
        },

        // 6: Water
        BlockDef {
            name: "genesis:water",
            solid: false,
            transparent: true,
            gravity: false,
            color: [0.2, 0.4, 0.8],
            tex_top: TEX_WATER, tex_bottom: TEX_WATER, tex_side: TEX_WATER,
        },

        // 7: Oak Log — bark sides, ring top/bottom
        BlockDef {
            name: "genesis:oak_log",
            solid: true,
            transparent: false,
            gravity: false,
            color: [0.4, 0.3, 0.19],
            tex_top: TEX_OAK_LOG_TOP, tex_bottom: TEX_OAK_LOG_TOP, tex_side: TEX_OAK_LOG_SIDE,
        },

        // 8: Oak Leaves
        BlockDef {
            name: "genesis:oak_leaves",
            solid: true,
            transparent: true,
            gravity: false,
            color: [0.23, 0.47, 0.12],
            tex_top: TEX_OAK_LEAVES, tex_bottom: TEX_OAK_LEAVES, tex_side: TEX_OAK_LEAVES,
        },

        // 9: Oak Planks
        BlockDef {
            name: "genesis:oak_planks",
            solid: true,
            transparent: false,
            gravity: false,
            color: [0.73, 0.58, 0.38],
            tex_top: TEX_OAK_PLANKS, tex_bottom: TEX_OAK_PLANKS, tex_side: TEX_OAK_PLANKS,
        },

        // 10: Cobblestone
        BlockDef {
            name: "genesis:cobblestone",
            solid: true,
            transparent: false,
            gravity: false,
            color: [0.45, 0.45, 0.45],
            tex_top: TEX_COBBLESTONE, tex_bottom: TEX_COBBLESTONE, tex_side: TEX_COBBLESTONE,
        },

        // 11: Gravel
        BlockDef {
            name: "genesis:gravel",
            solid: true,
            transparent: false,
            gravity: true,
            color: [0.55, 0.53, 0.51],
            tex_top: TEX_GRAVEL, tex_bottom: TEX_GRAVEL, tex_side: TEX_GRAVEL,
        },

        // 12: Sandstone
        BlockDef {
            name: "genesis:sandstone",
            solid: true,
            transparent: false,
            gravity: false,
            color: [0.82, 0.76, 0.55],
            tex_top: TEX_SANDSTONE, tex_bottom: TEX_SANDSTONE, tex_side: TEX_SANDSTONE,
        },

        // 13: Snow
        BlockDef {
            name: "genesis:snow",
            solid: true,
            transparent: false,
            gravity: false,
            color: [0.95, 0.97, 1.0],
            tex_top: TEX_SNOW, tex_bottom: TEX_SNOW, tex_side: TEX_SNOW,
        },

        // 14: Crafting Table — grid pattern top, planks sides
        BlockDef {
            name: "genesis:crafting_table",
            solid: true,
            transparent: false,
            gravity: false,
            color: [0.6, 0.45, 0.25],
            tex_top: TEX_CRAFTING_TABLE_TOP, tex_bottom: TEX_OAK_PLANKS, tex_side: TEX_OAK_PLANKS,
        },

        // 15: Bed — red mattress + pillow on top, wood sides. Right-click at
        // night to sleep (skip to morning + restore health + set spawn).
        BlockDef {
            name: "genesis:bed",
            solid: true,
            transparent: false,
            gravity: false,
            color: [0.85, 0.20, 0.20],
            tex_top: TEX_BED_TOP, tex_bottom: TEX_OAK_PLANKS, tex_side: TEX_BED_SIDE,
        },

        // 16: Coal Ore — stone with black flecks. Drops Coal when mined.
        BlockDef {
            name: "genesis:coal_ore",
            solid: true, transparent: false, gravity: false,
            color: [0.30, 0.30, 0.30],
            tex_top: TEX_COAL_ORE, tex_bottom: TEX_COAL_ORE, tex_side: TEX_COAL_ORE,
        },

        // 17: Iron Ore — stone with tan/orange flecks. Drops Raw Iron.
        BlockDef {
            name: "genesis:iron_ore",
            solid: true, transparent: false, gravity: false,
            color: [0.55, 0.45, 0.35],
            tex_top: TEX_IRON_ORE, tex_bottom: TEX_IRON_ORE, tex_side: TEX_IRON_ORE,
        },

        // 18: Diamond Ore — stone with cyan flecks. Drops Diamond. Rare, deep.
        BlockDef {
            name: "genesis:diamond_ore",
            solid: true, transparent: false, gravity: false,
            color: [0.40, 0.65, 0.65],
            tex_top: TEX_DIAMOND_ORE, tex_bottom: TEX_DIAMOND_ORE, tex_side: TEX_DIAMOND_ORE,
        },

        // 19: Glass — solid (you can stand on it) but transparent.
        BlockDef {
            name: "genesis:glass",
            solid: true, transparent: true, gravity: false,
            color: [0.85, 0.92, 0.95],
            tex_top: TEX_GLASS, tex_bottom: TEX_GLASS, tex_side: TEX_GLASS,
        },

        // 20: Coal Block — compressed storage (9 coal → 1 block, reversible).
        BlockDef {
            name: "genesis:coal_block",
            solid: true, transparent: false, gravity: false,
            color: [0.10, 0.10, 0.10],
            tex_top: TEX_COAL_BLOCK, tex_bottom: TEX_COAL_BLOCK, tex_side: TEX_COAL_BLOCK,
        },

        // 21: Iron Block.
        BlockDef {
            name: "genesis:iron_block",
            solid: true, transparent: false, gravity: false,
            color: [0.85, 0.85, 0.85],
            tex_top: TEX_IRON_BLOCK, tex_bottom: TEX_IRON_BLOCK, tex_side: TEX_IRON_BLOCK,
        },

        // 22: Diamond Block.
        BlockDef {
            name: "genesis:diamond_block",
            solid: true, transparent: false, gravity: false,
            color: [0.40, 0.95, 0.92],
            tex_top: TEX_DIAMOND_BLOCK, tex_bottom: TEX_DIAMOND_BLOCK, tex_side: TEX_DIAMOND_BLOCK,
        },

        // 23: Torch — light source. Currently rendered as a small textured
        // cube (proper X-shape rendering lands when block-shape variants are
        // a thing). Marked transparent so neighbouring faces render against
        // it correctly. Solid=false so the player can walk through (placement
        // collision behaviour matches MC for now).
        BlockDef {
            name: "genesis:torch",
            solid: false, transparent: true, gravity: false,
            color: [0.95, 0.78, 0.30],
            tex_top: TEX_TORCH, tex_bottom: TEX_TORCH, tex_side: TEX_TORCH,
        },

        // 24: Tall Grass — placed above a Grass block when the player uses
        // bonemeal on it (Wave 22). Walkable, transparent. Future polish:
        // X-shape mesh, drops nothing on break.
        BlockDef {
            name: "genesis:tall_grass",
            solid: false, transparent: true, gravity: false,
            color: [0.45, 0.75, 0.30],
            tex_top: TEX_TALL_GRASS, tex_bottom: TEX_TALL_GRASS, tex_side: TEX_TALL_GRASS,
        },

        // 25: Pure Deepslate — darker, harder variant of stone that fills the
        // world below Y_dp (Spec 2 §5.3.1a). The substrate for Orange Bitcoin
        // Gem veins (Spec 6 §2.2c) — gems only drop from breaking *pure*
        // deepslate (not deepslate ore variants). Requires a stone+ pickaxe.
        BlockDef {
            name: "genesis:pure_deepslate",
            solid: true, transparent: false, gravity: false,
            color: [0.30, 0.30, 0.32],
            tex_top: TEX_PURE_DEEPSLATE, tex_bottom: TEX_PURE_DEEPSLATE, tex_side: TEX_PURE_DEEPSLATE,
        },

        // 26: Deepslate Coal Ore — coal-bearing deepslate. Drops Coal.
        BlockDef {
            name: "genesis:deepslate_coal_ore",
            solid: true, transparent: false, gravity: false,
            color: [0.22, 0.22, 0.22],
            tex_top: TEX_DEEPSLATE_COAL_ORE, tex_bottom: TEX_DEEPSLATE_COAL_ORE, tex_side: TEX_DEEPSLATE_COAL_ORE,
        },

        // 27: Deepslate Iron Ore — iron-bearing deepslate. Drops Raw Iron.
        BlockDef {
            name: "genesis:deepslate_iron_ore",
            solid: true, transparent: false, gravity: false,
            color: [0.42, 0.34, 0.28],
            tex_top: TEX_DEEPSLATE_IRON_ORE, tex_bottom: TEX_DEEPSLATE_IRON_ORE, tex_side: TEX_DEEPSLATE_IRON_ORE,
        },

        // 28: Deepslate Diamond Ore — the deeper, denser diamond source.
        // Drops Diamond. Same iron-pickaxe requirement as stone-tier.
        BlockDef {
            name: "genesis:deepslate_diamond_ore",
            solid: true, transparent: false, gravity: false,
            color: [0.32, 0.55, 0.55],
            tex_top: TEX_DEEPSLATE_DIAMOND_ORE, tex_bottom: TEX_DEEPSLATE_DIAMOND_ORE, tex_side: TEX_DEEPSLATE_DIAMOND_ORE,
        },

        // 29: Satori Block — 9 Satori compressed into a storage block
        // (round-trip via 1×1 reverse recipe, same shape as diamond/iron/coal
        // storage blocks).
        BlockDef {
            name: "genesis:satori_block",
            solid: true, transparent: false, gravity: false,
            color: [0.95, 0.55, 0.18],
            tex_top: TEX_SATORI_BLOCK, tex_bottom: TEX_SATORI_BLOCK, tex_side: TEX_SATORI_BLOCK,
        },

        // 30: Tilled Soil — created by right-clicking dirt or grass with a Hoe
        // (Spec 5 §3.x Farming, foundation `2026-05-14-farming-system.md`).
        // The block crops plant into. Visually a darker dirt with furrow lines.
        // Sides + bottom render as plain dirt so a partially-tilled patch
        // looks correct against neighbouring untilled dirt.
        BlockDef {
            name: "genesis:tilled_soil",
            solid: true, transparent: false, gravity: false,
            color: [0.45, 0.30, 0.18],
            tex_top: TEX_TILLED_SOIL, tex_bottom: TEX_DIRT, tex_side: TEX_DIRT,
        },
        ];

        // 31..=34 / 35..=38 / 39..=42: Crop stages — wheat / carrot / potato.
        // All transparent + non-solid (you walk through them) + gravity:false.
        // Visual progression encoded by the per-stage texture index (Phase 5).
        // Mature stage = `_STAGE_3` per crop family; harvest drops are wired
        // in Phase 7. tex_top/bottom/side use the same per-stage texture so
        // the crop is visible from every angle.
        let crop_stages = [
            ("genesis:wheat_stage_0",  TEX_WHEAT_STAGE_0,  [0.50, 0.65, 0.30]),
            ("genesis:wheat_stage_1",  TEX_WHEAT_STAGE_1,  [0.60, 0.70, 0.30]),
            ("genesis:wheat_stage_2",  TEX_WHEAT_STAGE_2,  [0.75, 0.72, 0.30]),
            ("genesis:wheat_stage_3",  TEX_WHEAT_STAGE_3,  [0.95, 0.82, 0.35]),
            ("genesis:carrot_stage_0", TEX_CARROT_STAGE_0, [0.40, 0.65, 0.30]),
            ("genesis:carrot_stage_1", TEX_CARROT_STAGE_1, [0.45, 0.68, 0.30]),
            ("genesis:carrot_stage_2", TEX_CARROT_STAGE_2, [0.55, 0.70, 0.30]),
            ("genesis:carrot_stage_3", TEX_CARROT_STAGE_3, [0.60, 0.72, 0.30]),
            ("genesis:potato_stage_0", TEX_POTATO_STAGE_0, [0.40, 0.60, 0.28]),
            ("genesis:potato_stage_1", TEX_POTATO_STAGE_1, [0.45, 0.62, 0.28]),
            ("genesis:potato_stage_2", TEX_POTATO_STAGE_2, [0.55, 0.64, 0.28]),
            ("genesis:potato_stage_3", TEX_POTATO_STAGE_3, [0.65, 0.66, 0.28]),
        ];
        for (name, tex, color) in crop_stages {
            blocks.push(BlockDef {
                name,
                solid: false,
                transparent: true,
                gravity: false,
                color,
                tex_top: tex, tex_bottom: tex, tex_side: tex,
            });
        }

        // 43: Campfire (lit) — emits light, cooks meat. Top texture animates
        // a flame; sides show glowing logs. State + cooking is tracked in
        // `World::block_entities` (Spec 17 foundation `2026-05-18-campfire.md`).
        blocks.push(BlockDef {
            name: "genesis:campfire",
            solid: true, transparent: false, gravity: false,
            color: [0.60, 0.40, 0.20],
            tex_top: TEX_CAMPFIRE_LIT_TOP, tex_bottom: TEX_OAK_LOG_SIDE,
            tex_side: TEX_CAMPFIRE_LIT_SIDE,
        });
        // 44: Campfire (unlit) — same shape, no flame, no light, no cooking.
        // Right-clicking with stick (friction) or flint-and-steel transitions
        // to CAMPFIRE provided there's fuel in the block-entity.
        blocks.push(BlockDef {
            name: "genesis:campfire_unlit",
            solid: true, transparent: false, gravity: false,
            color: [0.30, 0.25, 0.20],
            tex_top: TEX_CAMPFIRE_UNLIT_TOP, tex_bottom: TEX_OAK_LOG_SIDE,
            tex_side: TEX_CAMPFIRE_UNLIT_SIDE,
        });

        // 45: Campfire Smoke pillar — non-solid, transparent. Placed in a
        // column above a lit campfire when leaves are burning. Spec 18
        // foundation `2026-05-18-campfire-extensions.md`.
        blocks.push(BlockDef {
            name: "genesis:campfire_smoke",
            solid: false, transparent: true, gravity: false,
            color: [0.72, 0.72, 0.72],
            tex_top: TEX_CAMPFIRE_SMOKE, tex_bottom: TEX_CAMPFIRE_SMOKE,
            tex_side: TEX_CAMPFIRE_SMOKE,
        });

        // 46-49: Corn crop stages (Wave 28 — Spec 18). Non-solid + transparent
        // so the player can walk through tall crops the same way as wheat /
        // carrot / potato stages.
        let corn_stage_tex = [
            TEX_CORN_STAGE_0, TEX_CORN_STAGE_1, TEX_CORN_STAGE_2, TEX_CORN_STAGE_3,
        ];
        for (i, &tex) in corn_stage_tex.iter().enumerate() {
            blocks.push(BlockDef {
                name: match i {
                    0 => "genesis:corn_stage_0",
                    1 => "genesis:corn_stage_1",
                    2 => "genesis:corn_stage_2",
                    _ => "genesis:corn_stage_3",
                },
                solid: false, transparent: true, gravity: false,
                color: [0.55 + 0.10 * i as f32, 0.65, 0.30],
                tex_top: tex, tex_bottom: tex, tex_side: tex,
            });
        }

        // 50: Village Bell (Spec 19 phase 10). Reuses the iron-block texture
        // top/bottom + cobblestone sides for the post — purely visual; the
        // mechanic is the side-table entry placement creates in `World`.
        blocks.push(BlockDef {
            name: "genesis:village_bell",
            solid: true, transparent: false, gravity: false,
            color: [0.78, 0.74, 0.50],
            tex_top: TEX_IRON_BLOCK, tex_bottom: TEX_COBBLESTONE,
            tex_side: TEX_COBBLESTONE,
        });

        // 51: Drying Rack (Wave 29 — Spec 29). Workstation block that
        // matures green logs into seasoned logs over real-time ticks when
        // the block directly above is AIR (airflow / open-sky gate).
        // Stylised stack-of-logs top with a vertical-bar frame on the
        // sides keeps it readable as a workstation.
        blocks.push(BlockDef {
            name: "genesis:drying_rack",
            solid: true, transparent: false, gravity: false,
            color: [0.55, 0.40, 0.22],
            tex_top: TEX_DRYING_RACK_TOP,
            tex_bottom: TEX_OAK_LOG_SIDE,
            tex_side: TEX_DRYING_RACK_SIDE,
        });

        // 52-55: Papyrus Reed stages (Spec 23 — Foundation A of Build
        // Schematics, 2026-05-19). Four growth stages mirroring the
        // wheat/carrot/potato ladder. Non-solid + transparent so the
        // player can wade between reeds while harvesting. Mining a
        // mature stage drops 1-2 reeds + replaces with STAGE_0 (the
        // root keeps growing; see `growth::crop_break`).
        let papyrus_stages = [
            ("genesis:papyrus_stage_0", TEX_PAPYRUS_STAGE_0, [0.55, 0.65, 0.30]),
            ("genesis:papyrus_stage_1", TEX_PAPYRUS_STAGE_1, [0.60, 0.70, 0.32]),
            ("genesis:papyrus_stage_2", TEX_PAPYRUS_STAGE_2, [0.68, 0.75, 0.32]),
            ("genesis:papyrus_stage_3", TEX_PAPYRUS_STAGE_3, [0.85, 0.82, 0.45]),
        ];
        for (name, tex, color) in papyrus_stages {
            blocks.push(BlockDef {
                name,
                solid: false,
                transparent: true,
                gravity: false,
                color,
                tex_top: tex, tex_bottom: tex, tex_side: tex,
            });
        }

        // 56: Blueprint Paper (Spec 24 — Foundation B). Parchment-drafting
        // square laid around a building before capture. Solid + opaque
        // — the player walks on tiles like normal blocks. Top face is
        // parchment-cream; sides are dark walnut border so the tile
        // reads as a drafting board.
        blocks.push(BlockDef {
            name: "genesis:blueprint_paper",
            solid: true, transparent: false, gravity: false,
            color: [0.85, 0.78, 0.55],
            tex_top: TEX_BLUEPRINT_PAPER_TOP,
            tex_bottom: TEX_BLUEPRINT_PAPER_SIDE,
            tex_side: TEX_BLUEPRINT_PAPER_SIDE,
        });

        // 57: Construction Anchor (Spec 24). Engine-placed marker at
        // an in-progress build site. Non-solid + transparent — the
        // player walks through it. Single surveyor's-flag texture used
        // on every face. Admin-protected: only the world-owner can
        // mine it during normal play (game_loop guards the break).
        blocks.push(BlockDef {
            name: "genesis:construction_anchor",
            solid: false, transparent: true, gravity: false,
            color: [0.88, 0.30, 0.30],
            tex_top: TEX_CONSTRUCTION_ANCHOR,
            tex_bottom: TEX_CONSTRUCTION_ANCHOR,
            tex_side: TEX_CONSTRUCTION_ANCHOR,
        });

        // 58: Architect's Plaque (Spec 24). Auto-placed last in every
        // animated build. Solid + opaque — visible as a flat parchment
        // on a wood-frame block. Carries the derivation chain via
        // `World::architect_plaques`; right-click opens the attribution
        // dialog (tip-target placeholder until Spec 25 wires sats).
        // Admin-protected.
        blocks.push(BlockDef {
            name: "genesis:architect_plaque",
            solid: true, transparent: false, gravity: false,
            color: [0.92, 0.86, 0.62],
            tex_top: TEX_ARCHITECT_PLAQUE_TOP,
            tex_bottom: TEX_ARCHITECT_PLAQUE_SIDE,
            tex_side: TEX_ARCHITECT_PLAQUE_SIDE,
        });

        // 59: Furnace (Spec 20). Unlit / idle state. Mirrors the Campfire
        // pattern — two block-ids (FURNACE / FURNACE_LIT), one for each
        // visual state. Block-entity state (input/fuel/output slots,
        // smelt progress) lives in `World::block_entities`.
        blocks.push(BlockDef {
            name: "genesis:furnace",
            solid: true, transparent: false, gravity: false,
            color: [0.45, 0.42, 0.40],
            tex_top: TEX_FURNACE_TOP,
            tex_bottom: TEX_FURNACE_TOP,
            tex_side: TEX_FURNACE_SIDE_UNLIT,
        });

        // 60: Furnace lit. Visual swap — orange glow on side faces.
        // Active state mirror of FURNACE; both drop the unlit form
        // when mined (mine_drop normalisation, same as Campfire).
        blocks.push(BlockDef {
            name: "genesis:furnace_lit",
            solid: true, transparent: false, gravity: false,
            color: [0.62, 0.45, 0.30],
            tex_top: TEX_FURNACE_TOP,
            tex_bottom: TEX_FURNACE_TOP,
            tex_side: TEX_FURNACE_SIDE_LIT,
        });

        // 61-63: Spec 16 Phase 3b deepslate visual variants.
        // Gameplay-identical to PURE_DEEPSLATE (mining time, tool gate,
        // proof-of-play behaviour); only the texture differs. Chunk-gen
        // picks variant from `reserve.richness` via
        // `biome::pick_deepslate_variant`. Mining any variant drops the
        // canonical PURE_DEEPSLATE item via the mine_drop normalisation
        // arm (so inventory icons stay honest).
        blocks.push(BlockDef {
            name: "genesis:pure_deepslate_thin",
            solid: true, transparent: false, gravity: false,
            color: [0.30, 0.32, 0.36],
            tex_top: TEX_PURE_DEEPSLATE_THIN,
            tex_bottom: TEX_PURE_DEEPSLATE_THIN,
            tex_side: TEX_PURE_DEEPSLATE_THIN,
        });
        blocks.push(BlockDef {
            name: "genesis:pure_deepslate_healthy",
            solid: true, transparent: false, gravity: false,
            color: [0.32, 0.34, 0.40],
            tex_top: TEX_PURE_DEEPSLATE_HEALTHY,
            tex_bottom: TEX_PURE_DEEPSLATE_HEALTHY,
            tex_side: TEX_PURE_DEEPSLATE_HEALTHY,
        });
        blocks.push(BlockDef {
            name: "genesis:pure_deepslate_fat",
            solid: true, transparent: false, gravity: false,
            color: [0.36, 0.38, 0.46],
            tex_top: TEX_PURE_DEEPSLATE_FAT,
            tex_bottom: TEX_PURE_DEEPSLATE_FAT,
            tex_side: TEX_PURE_DEEPSLATE_FAT,
        });

        // 64: Spec 21 Vendor Block. Solid + opaque + non-gravity.
        // Block-entity state (owner, mode, slot, stock, escrow) lives
        // in `World::block_entities` as `BlockEntityData::Vendor`. The
        // place handler registers `VendorOwner::LocalPlayer(pidx)`.
        // Mining: owner-only; non-owners get a "Only the owner can
        // mine this" toast (Spec 21 Phase 7 anti-grief).
        blocks.push(BlockDef {
            name: "genesis:vendor_block",
            solid: true, transparent: false, gravity: false,
            color: [0.55, 0.40, 0.25],
            tex_top: TEX_VENDOR_BLOCK,
            tex_bottom: TEX_VENDOR_BLOCK,
            tex_side: TEX_VENDOR_BLOCK,
        });

        // 65: Spec 26 Drafting Table. Workstation for the Builder
        // profession (a villager who claims it reads as Builder).
        // v1 has no commission UI — Phases 5+ post-MVP. Solid + opaque
        // + non-gravity; mineable normally (drops itself).
        blocks.push(BlockDef {
            name: "genesis:drafting_table",
            solid: true, transparent: false, gravity: false,
            color: [0.80, 0.70, 0.45],
            tex_top: TEX_DRAFTING_TABLE,
            tex_bottom: TEX_DRAFTING_TABLE,
            tex_side: TEX_DRAFTING_TABLE,
        });

        // 66: Spec 28c — Limestone. Decorative stone variant, off-white
        // with pale yellow tint. Distribution from biome stone-replace
        // noise once 28a lands; until then placeable via creative.
        blocks.push(BlockDef {
            name: "genesis:limestone",
            solid: true, transparent: false, gravity: false,
            color: [0.92, 0.90, 0.78],
            tex_top: 0, tex_bottom: 0, tex_side: 0, // BRIDGE: shared with stone tex until 28c texture pack
        });

        // 67: Spec 28c — Marble. Veined white, slightly cooler than limestone.
        blocks.push(BlockDef {
            name: "genesis:marble",
            solid: true, transparent: false, gravity: false,
            color: [0.95, 0.95, 0.92],
            tex_top: 0, tex_bottom: 0, tex_side: 0,
        });

        // 68: Spec 28c — Granite. Pink-brown speckled hard stone.
        blocks.push(BlockDef {
            name: "genesis:granite",
            solid: true, transparent: false, gravity: false,
            color: [0.65, 0.45, 0.40],
            tex_top: 0, tex_bottom: 0, tex_side: 0,
        });

        // 69: Spec 28c — Slate (the decorative variant — does NOT collide
        // with the pure-deepslate-family ids 25-28+61-63 which are the
        // depth-vein blocks. This is a near-surface decorative stone.)
        blocks.push(BlockDef {
            name: "genesis:slate",
            solid: true, transparent: false, gravity: false,
            color: [0.32, 0.32, 0.38],
            tex_top: 0, tex_bottom: 0, tex_side: 0,
        });

        // 70: Spec 28c — Copper Ore. Mineable with Stone+ pickaxe;
        // drops Copper material. Distribution from biome ore noise: the
        // Wind, Copper & Electricity wave (2026-09-07, Spec 02 §1) added the
        // mid-depth stone band in `biome::ore_at` — Copper Ore now generates
        // naturally, with a real procedural texture (`TEX_COPPER_ORE`).
        blocks.push(BlockDef {
            name: "genesis:copper_ore",
            solid: true, transparent: false, gravity: false,
            color: [0.70, 0.45, 0.30],
            tex_top: TEX_COPPER_ORE, tex_bottom: TEX_COPPER_ORE, tex_side: TEX_COPPER_ORE,
        });

        // 71: Spec 28c — Bone Block (decorative placed-bone, like skeletons
        // or large fossils). Crafted from 9 bone material in 3×3.
        blocks.push(BlockDef {
            name: "genesis:bone_block",
            solid: true, transparent: false, gravity: false,
            color: [0.92, 0.90, 0.78],
            tex_top: 0, tex_bottom: 0, tex_side: 0,
        });

        // 72: Spec 28c — Hay Bale (decorative + future animal-food).
        // Crafted from 9 wheat in 3×3.
        blocks.push(BlockDef {
            name: "genesis:hay_bale",
            solid: true, transparent: false, gravity: false,
            color: [0.85, 0.72, 0.30],
            tex_top: 0, tex_bottom: 0, tex_side: 0,
        });

        // 73: Spec 28c — Amethyst Block (decorative gem block). Crafted
        // from 4 amethyst material in 2×2.
        blocks.push(BlockDef {
            name: "genesis:amethyst_block",
            solid: true, transparent: false, gravity: false,
            color: [0.55, 0.30, 0.75],
            tex_top: 0, tex_bottom: 0, tex_side: 0,
        });

        // 74-88: Spec 28b Wood Species. Five new species × {log, leaves,
        // planks}, each with its OWN textures now: leaves #130 (TEX_*_LEAVES,
        // 421..=425), logs + planks #132 (TEX_*_LOG_{SIDE,TOP}/TEX_*_PLANKS,
        // 426..=440). Logs + planks are solid + opaque. Leaves are solid +
        // TRANSPARENT (#131 fancy-leaf flip): they render see-through (cutout-
        // alpha holes) via the transparent-solid pass `greedy_transparent_face`.
        // Earlier (#130) they were opaque only because no such pass existed yet.
        blocks.push(BlockDef { // 74 BIRCH_LOG
            name: "genesis:birch_log",
            solid: true, transparent: false, gravity: false,
            color: [0.95, 0.92, 0.82],
            tex_top: TEX_BIRCH_LOG_TOP, tex_bottom: TEX_BIRCH_LOG_TOP, tex_side: TEX_BIRCH_LOG_SIDE,
        });
        blocks.push(BlockDef { // 75 BIRCH_LEAVES
            name: "genesis:birch_leaves",
            solid: true, transparent: true, gravity: false,
            color: [0.55, 0.78, 0.45],
            tex_top: TEX_BIRCH_LEAVES, tex_bottom: TEX_BIRCH_LEAVES, tex_side: TEX_BIRCH_LEAVES,
        });
        blocks.push(BlockDef { // 76 BIRCH_PLANKS
            name: "genesis:birch_planks",
            solid: true, transparent: false, gravity: false,
            color: [0.92, 0.88, 0.72],
            tex_top: TEX_BIRCH_PLANKS, tex_bottom: TEX_BIRCH_PLANKS, tex_side: TEX_BIRCH_PLANKS,
        });
        blocks.push(BlockDef { // 77 SPRUCE_LOG
            name: "genesis:spruce_log",
            solid: true, transparent: false, gravity: false,
            color: [0.42, 0.28, 0.18],
            tex_top: TEX_SPRUCE_LOG_TOP, tex_bottom: TEX_SPRUCE_LOG_TOP, tex_side: TEX_SPRUCE_LOG_SIDE,
        });
        blocks.push(BlockDef { // 78 SPRUCE_LEAVES
            name: "genesis:spruce_leaves",
            solid: true, transparent: true, gravity: false,
            color: [0.30, 0.50, 0.30],
            tex_top: TEX_SPRUCE_LEAVES, tex_bottom: TEX_SPRUCE_LEAVES, tex_side: TEX_SPRUCE_LEAVES,
        });
        blocks.push(BlockDef { // 79 SPRUCE_PLANKS
            name: "genesis:spruce_planks",
            solid: true, transparent: false, gravity: false,
            color: [0.55, 0.38, 0.22],
            tex_top: TEX_SPRUCE_PLANKS, tex_bottom: TEX_SPRUCE_PLANKS, tex_side: TEX_SPRUCE_PLANKS,
        });
        blocks.push(BlockDef { // 80 JUNGLE_LOG
            name: "genesis:jungle_log",
            solid: true, transparent: false, gravity: false,
            color: [0.55, 0.45, 0.25],
            tex_top: TEX_JUNGLE_LOG_TOP, tex_bottom: TEX_JUNGLE_LOG_TOP, tex_side: TEX_JUNGLE_LOG_SIDE,
        });
        blocks.push(BlockDef { // 81 JUNGLE_LEAVES
            name: "genesis:jungle_leaves",
            solid: true, transparent: true, gravity: false,
            color: [0.30, 0.85, 0.20],
            tex_top: TEX_JUNGLE_LEAVES, tex_bottom: TEX_JUNGLE_LEAVES, tex_side: TEX_JUNGLE_LEAVES,
        });
        blocks.push(BlockDef { // 82 JUNGLE_PLANKS
            name: "genesis:jungle_planks",
            solid: true, transparent: false, gravity: false,
            color: [0.72, 0.52, 0.30],
            tex_top: TEX_JUNGLE_PLANKS, tex_bottom: TEX_JUNGLE_PLANKS, tex_side: TEX_JUNGLE_PLANKS,
        });
        blocks.push(BlockDef { // 83 ACACIA_LOG
            name: "genesis:acacia_log",
            solid: true, transparent: false, gravity: false,
            color: [0.65, 0.42, 0.22],
            tex_top: TEX_ACACIA_LOG_TOP, tex_bottom: TEX_ACACIA_LOG_TOP, tex_side: TEX_ACACIA_LOG_SIDE,
        });
        blocks.push(BlockDef { // 84 ACACIA_LEAVES
            name: "genesis:acacia_leaves",
            solid: true, transparent: true, gravity: false,
            color: [0.40, 0.65, 0.25],
            tex_top: TEX_ACACIA_LEAVES, tex_bottom: TEX_ACACIA_LEAVES, tex_side: TEX_ACACIA_LEAVES,
        });
        blocks.push(BlockDef { // 85 ACACIA_PLANKS
            name: "genesis:acacia_planks",
            solid: true, transparent: false, gravity: false,
            color: [0.85, 0.55, 0.20],
            tex_top: TEX_ACACIA_PLANKS, tex_bottom: TEX_ACACIA_PLANKS, tex_side: TEX_ACACIA_PLANKS,
        });
        blocks.push(BlockDef { // 86 DARK_OAK_LOG
            name: "genesis:dark_oak_log",
            solid: true, transparent: false, gravity: false,
            color: [0.20, 0.12, 0.08],
            tex_top: TEX_DARK_OAK_LOG_TOP, tex_bottom: TEX_DARK_OAK_LOG_TOP, tex_side: TEX_DARK_OAK_LOG_SIDE,
        });
        blocks.push(BlockDef { // 87 DARK_OAK_LEAVES
            name: "genesis:dark_oak_leaves",
            solid: true, transparent: true, gravity: false,
            color: [0.20, 0.45, 0.20],
            tex_top: TEX_DARK_OAK_LEAVES, tex_bottom: TEX_DARK_OAK_LEAVES, tex_side: TEX_DARK_OAK_LEAVES,
        });
        blocks.push(BlockDef { // 88 DARK_OAK_PLANKS
            name: "genesis:dark_oak_planks",
            solid: true, transparent: false, gravity: false,
            color: [0.35, 0.22, 0.12],
            tex_top: TEX_DARK_OAK_PLANKS, tex_bottom: TEX_DARK_OAK_PLANKS, tex_side: TEX_DARK_OAK_PLANKS,
        });

        // 89-110: T1.5 crops + workstations. BRIDGE: textures reuse the
        // closest existing analogue (wheat-stage for crop stages, oak
        // log for workstations) — per-block textures land with the
        // T1.5 texture-pack work in Phase 9 of the spec.
        //
        // Crops are non-solid, transparent (walk-through) at all stages,
        // matching the wheat/carrot/potato/corn convention. Mature
        // stages drop their crops + seeds on break.
        let crop_def = |name: &'static str, color: [f32; 3]| BlockDef {
            name,
            solid: false, transparent: true, gravity: false,
            color,
            tex_top: TEX_WHEAT_STAGE_3, tex_bottom: TEX_WHEAT_STAGE_3, tex_side: TEX_WHEAT_STAGE_3,
        };

        blocks.push(crop_def("genesis:sugarcane", [0.75, 0.90, 0.55])); // 89
        blocks.push(crop_def("genesis:sugar_beet_stage_0", [0.35, 0.50, 0.20])); // 90
        blocks.push(crop_def("genesis:sugar_beet_stage_1", [0.55, 0.70, 0.30])); // 91
        blocks.push(crop_def("genesis:sugar_beet_stage_2", [0.75, 0.85, 0.45])); // 92
        blocks.push(crop_def("genesis:sugar_beet_stage_3", [0.92, 0.92, 0.88])); // 93
        blocks.push(crop_def("genesis:beetroot_stage_0", [0.25, 0.55, 0.25])); // 94
        blocks.push(crop_def("genesis:beetroot_stage_1", [0.40, 0.55, 0.30])); // 95
        blocks.push(crop_def("genesis:beetroot_stage_2", [0.55, 0.40, 0.30])); // 96
        blocks.push(crop_def("genesis:beetroot_stage_3", [0.55, 0.15, 0.20])); // 97
        // Pumpkin block is SOLID + opaque (unlike crop stages — it's
        // the harvested fruit, not a growing crop).
        blocks.push(BlockDef { // 98 PUMPKIN
            name: "genesis:pumpkin",
            solid: true, transparent: false, gravity: false,
            color: [0.95, 0.55, 0.18],
            tex_top: TEX_WHEAT_STAGE_3, tex_bottom: TEX_WHEAT_STAGE_3, tex_side: TEX_WHEAT_STAGE_3,
        });
        blocks.push(crop_def("genesis:pumpkin_stem_0", [0.35, 0.55, 0.25])); // 99
        blocks.push(crop_def("genesis:pumpkin_stem_1", [0.45, 0.65, 0.30])); // 100
        blocks.push(crop_def("genesis:pumpkin_stem_2", [0.55, 0.60, 0.30])); // 101
        blocks.push(crop_def("genesis:pumpkin_stem_3", [0.65, 0.55, 0.25])); // 102
        blocks.push(crop_def("genesis:pumpkin_stem_4", [0.75, 0.45, 0.20])); // 103 mature
        blocks.push(crop_def("genesis:berry_bush_0", [0.35, 0.50, 0.30])); // 104
        blocks.push(crop_def("genesis:berry_bush_1", [0.40, 0.55, 0.30])); // 105
        blocks.push(crop_def("genesis:berry_bush_2", [0.45, 0.45, 0.30])); // 106
        blocks.push(crop_def("genesis:berry_bush_3", [0.55, 0.20, 0.35])); // 107 mature

        // Workstation blocks — solid + opaque, like the Furnace.
        blocks.push(BlockDef { // 108 MILL
            name: "genesis:mill",
            solid: true, transparent: false, gravity: false,
            color: [0.55, 0.50, 0.40],
            tex_top: TEX_OAK_PLANKS, tex_bottom: TEX_OAK_PLANKS, tex_side: TEX_OAK_PLANKS,
        });
        blocks.push(BlockDef { // 109 OVEN
            name: "genesis:oven",
            solid: true, transparent: false, gravity: false,
            color: [0.45, 0.35, 0.30],
            tex_top: TEX_OAK_PLANKS, tex_bottom: TEX_OAK_PLANKS, tex_side: TEX_OAK_PLANKS,
        });
        blocks.push(BlockDef { // 110 AGING_RACK
            name: "genesis:aging_rack",
            solid: true, transparent: false, gravity: false,
            color: [0.60, 0.45, 0.25],
            tex_top: TEX_OAK_LOG_SIDE, tex_bottom: TEX_OAK_LOG_SIDE, tex_side: TEX_OAK_LOG_SIDE,
        });
        blocks.push(BlockDef { // 111 BEE_HIVE
            name: "genesis:bee_hive",
            solid: true, transparent: false, gravity: false,
            color: [0.78, 0.55, 0.28],
            tex_top: TEX_OAK_LOG_TOP, tex_bottom: TEX_OAK_LOG_TOP, tex_side: TEX_OAK_PLANKS,
        });
        blocks.push(BlockDef { // 112 CHEST (HP-2)
            name: "genesis:chest",
            solid: true, transparent: false, gravity: false,
            color: [0.62, 0.45, 0.20],
            tex_top: TEX_CHEST_TOP, tex_bottom: TEX_CHEST_SIDE, tex_side: TEX_CHEST_SIDE,
        });
        blocks.push(BlockDef { // 113 BRIGAND_HIDEOUT_BANNER (HP-3)
            name: "genesis:brigand_hideout_banner",
            solid: false, transparent: true, gravity: false,
            color: [0.55, 0.18, 0.42],
            tex_top: TEX_BRIGAND_HIDEOUT_BANNER,
            tex_bottom: TEX_BRIGAND_HIDEOUT_BANNER,
            tex_side: TEX_BRIGAND_HIDEOUT_BANNER,
        });
        blocks.push(BlockDef { // 114 TROPHY_WALL (HP-3 v2)
            name: "genesis:trophy_wall",
            solid: true, transparent: false, gravity: false,
            color: [0.62, 0.40, 0.20],
            tex_top: TEX_OAK_PLANKS,
            tex_bottom: TEX_OAK_PLANKS,
            tex_side: TEX_TROPHY_WALL,
        });
        blocks.push(BlockDef { // 115 ROCK_SALT (Salt)
            name: "genesis:rock_salt",
            solid: true, transparent: false, gravity: false,
            color: [0.85, 0.78, 0.78],
            tex_top: TEX_ROCK_SALT,
            tex_bottom: TEX_ROCK_SALT,
            tex_side: TEX_ROCK_SALT,
        });
        blocks.push(BlockDef { // 116 SALT_LICK (Salt)
            name: "genesis:salt_lick",
            solid: true, transparent: false, gravity: false,
            color: [0.92, 0.88, 0.80],
            tex_top: TEX_SALT_LICK,
            tex_bottom: TEX_SALT_LICK,
            tex_side: TEX_SALT_LICK,
        });
        blocks.push(BlockDef { // 117 SALT_LAMP (Salt)
            name: "genesis:salt_lamp",
            solid: true, transparent: false, gravity: false,
            color: [0.98, 0.72, 0.45],
            tex_top: TEX_SALT_LAMP,
            tex_bottom: TEX_SALT_LAMP,
            tex_side: TEX_SALT_LAMP,
        });
        blocks.push(BlockDef { // 118 SALT_BLOCK (Salt)
            name: "genesis:salt_block",
            solid: true, transparent: false, gravity: false,
            color: [0.95, 0.92, 0.92],
            tex_top: TEX_SALT_BLOCK,
            tex_bottom: TEX_SALT_BLOCK,
            tex_side: TEX_SALT_BLOCK,
        });
        blocks.push(BlockDef { // 119 SALT_PATH (Salt)
            name: "genesis:salt_path",
            solid: true, transparent: false, gravity: false,
            color: [0.62, 0.50, 0.36],
            tex_top: TEX_SALT_PATH_TOP,
            tex_bottom: TEX_DIRT,
            tex_side: TEX_SALT_PATH_SIDE,
        });
        blocks.push(BlockDef { // 120 RUBBER_LOG (Rubber)
            name: "genesis:rubber_log",
            solid: true, transparent: false, gravity: false,
            color: [0.86, 0.74, 0.58],
            tex_top: TEX_RUBBER_LOG,
            tex_bottom: TEX_RUBBER_LOG,
            tex_side: TEX_RUBBER_LOG,
        });
        blocks.push(BlockDef { // 121 RUBBER_PLANKS (Rubber)
            name: "genesis:rubber_planks",
            solid: true, transparent: false, gravity: false,
            color: [0.92, 0.82, 0.68],
            tex_top: TEX_RUBBER_PLANKS,
            tex_bottom: TEX_RUBBER_PLANKS,
            tex_side: TEX_RUBBER_PLANKS,
        });
        blocks.push(BlockDef { // 122 RUBBER_LEAVES (Rubber)
            name: "genesis:rubber_leaves",
            solid: false, transparent: true, gravity: false,
            color: [0.32, 0.55, 0.28],
            tex_top: TEX_RUBBER_LEAVES,
            tex_bottom: TEX_RUBBER_LEAVES,
            tex_side: TEX_RUBBER_LEAVES,
        });
        blocks.push(BlockDef { // 123 RUBBER_LOG_TAPPED (Rubber)
            name: "genesis:rubber_log_tapped",
            solid: true, transparent: false, gravity: false,
            color: [0.86, 0.74, 0.58],
            tex_top: TEX_RUBBER_LOG,
            tex_bottom: TEX_RUBBER_LOG,
            tex_side: TEX_RUBBER_LOG_TAPPED,
        });
        blocks.push(BlockDef { // 124 BOUNTY_BOARD (Spec 33)
            name: "genesis:bounty_board",
            solid: true, transparent: false, gravity: false,
            color: [0.55, 0.36, 0.20],
            tex_top: TEX_BOUNTY_BOARD,
            tex_bottom: TEX_BOUNTY_BOARD,
            tex_side: TEX_BOUNTY_BOARD,
        });
        blocks.push(BlockDef { // 125 TIP_JAR (Spec 34)
            name: "genesis:tip_jar",
            solid: true, transparent: false, gravity: false,
            color: [0.70, 0.55, 0.20], // warm gold-glazed jar tint
            tex_top: TEX_TIP_JAR,
            tex_bottom: TEX_TIP_JAR,
            tex_side: TEX_TIP_JAR,
        });
        blocks.push(BlockDef { // 126 REPAIR_BENCH (Spec 35)
            name: "genesis:repair_bench",
            solid: true, transparent: false, gravity: false,
            color: [0.32, 0.32, 0.36], // dark worn metal
            tex_top: TEX_REPAIR_BENCH,
            tex_bottom: TEX_REPAIR_BENCH,
            tex_side: TEX_REPAIR_BENCH,
        });
        blocks.push(BlockDef { // 127 PLOT_MARKER (Spec 36)
            name: "genesis:plot_marker",
            solid: true, transparent: false, gravity: false,
            color: [0.85, 0.70, 0.15], // hazard-yellow survey stake
            tex_top: TEX_PLOT_MARKER,
            tex_bottom: TEX_PLOT_MARKER,
            tex_side: TEX_PLOT_MARKER,
        });
        blocks.push(BlockDef { // 128 MARKET_BELL (Spec 37)
            name: "genesis:market_bell",
            solid: true, transparent: false, gravity: false,
            color: [0.80, 0.62, 0.25], // brass market bell
            tex_top: TEX_MARKET_BELL,
            tex_bottom: TEX_MARKET_BELL,
            tex_side: TEX_MARKET_BELL,
        });
        blocks.push(BlockDef { // 129 AUCTION_BLOCK (Spec 38)
            name: "genesis:auction_block",
            solid: true, transparent: false, gravity: false,
            color: [0.62, 0.45, 0.28], // polished auction podium
            tex_top: TEX_AUCTION_BLOCK,
            tex_bottom: TEX_AUCTION_BLOCK,
            tex_side: TEX_AUCTION_BLOCK,
        });
        blocks.push(BlockDef { // 130 BAZAAR_BLOCK (Spec 39)
            name: "genesis:bazaar_block",
            solid: true, transparent: false, gravity: false,
            color: [0.40, 0.55, 0.45], // teal-green trading-post canopy
            tex_top: TEX_BAZAAR_BLOCK,
            tex_bottom: TEX_BAZAAR_BLOCK,
            tex_side: TEX_BAZAAR_BLOCK,
        });

        // Dye flowers (Spec 35) + fibre plants (Spec 36) — all non-solid
        // X-mesh plants like tall grass. 2026-05-27.
        blocks.push(BlockDef { // 131 CORNFLOWER
            name: "genesis:cornflower",
            solid: false, transparent: true, gravity: false,
            color: [0.30, 0.40, 0.85],
            tex_top: TEX_CORNFLOWER, tex_bottom: TEX_CORNFLOWER, tex_side: TEX_CORNFLOWER,
        });
        blocks.push(BlockDef { // 132 FIELD_POPPY
            name: "genesis:field_poppy",
            solid: false, transparent: true, gravity: false,
            color: [0.82, 0.16, 0.14],
            tex_top: TEX_FIELD_POPPY, tex_bottom: TEX_FIELD_POPPY, tex_side: TEX_FIELD_POPPY,
        });
        blocks.push(BlockDef { // 133 BUTTERCUP
            name: "genesis:buttercup",
            solid: false, transparent: true, gravity: false,
            color: [0.96, 0.85, 0.20],
            tex_top: TEX_BUTTERCUP, tex_bottom: TEX_BUTTERCUP, tex_side: TEX_BUTTERCUP,
        });
        blocks.push(BlockDef { // 134 COTTON_PLANT
            name: "genesis:cotton_plant",
            solid: false, transparent: true, gravity: false,
            color: [0.90, 0.90, 0.84],
            tex_top: TEX_COTTON_PLANT, tex_bottom: TEX_COTTON_PLANT, tex_side: TEX_COTTON_PLANT,
        });
        blocks.push(BlockDef { // 135 HEMP_PLANT
            name: "genesis:hemp_plant",
            solid: false, transparent: true, gravity: false,
            color: [0.42, 0.58, 0.32],
            tex_top: TEX_HEMP_PLANT, tex_bottom: TEX_HEMP_PLANT, tex_side: TEX_HEMP_PLANT,
        });
        blocks.push(BlockDef { // 136 MAGNESIUM_ORE (Spec 37)
            name: "genesis:magnesium_ore",
            solid: true, transparent: false, gravity: false,
            color: [0.80, 0.80, 0.84], // pale silver-white speckle on stone
            tex_top: TEX_MAGNESIUM_ORE, tex_bottom: TEX_MAGNESIUM_ORE, tex_side: TEX_MAGNESIUM_ORE,
        });
        // Spec 36 Phase 2 — farmable cotton + hemp crop stages (137-144).
        // Transparent non-solid crops like wheat; drops handled by crop_break.
        let crop = |name: &'static str, tex: u32| BlockDef {
            name,
            solid: false, transparent: true, gravity: false,
            color: [0.55, 0.70, 0.40],
            tex_top: tex, tex_bottom: tex, tex_side: tex,
        };
        blocks.push(crop("genesis:cotton_stage_0", TEX_COTTON_STAGE_0)); // 137
        blocks.push(crop("genesis:cotton_stage_1", TEX_COTTON_STAGE_1)); // 138
        blocks.push(crop("genesis:cotton_stage_2", TEX_COTTON_STAGE_2)); // 139
        blocks.push(crop("genesis:cotton_stage_3", TEX_COTTON_STAGE_3)); // 140
        blocks.push(crop("genesis:hemp_stage_0", TEX_HEMP_STAGE_0)); // 141
        blocks.push(crop("genesis:hemp_stage_1", TEX_HEMP_STAGE_1)); // 142
        blocks.push(crop("genesis:hemp_stage_2", TEX_HEMP_STAGE_2)); // 143
        blocks.push(crop("genesis:hemp_stage_3", TEX_HEMP_STAGE_3)); // 144

        // Coloured wallpaper (dyed-paper décor) — solid opaque decorative
        // blocks, one per dye colour (145-157). Drop themselves on mine.
        let wp = |name: &'static str, color: [f32; 3], tex: u32| BlockDef {
            name,
            solid: true, transparent: false, gravity: false,
            color, tex_top: tex, tex_bottom: tex, tex_side: tex,
        };
        blocks.push(wp("genesis:wallpaper_white", [0.93, 0.93, 0.93], TEX_WALLPAPER_WHITE)); // 145
        blocks.push(wp("genesis:wallpaper_black", [0.13, 0.13, 0.15], TEX_WALLPAPER_BLACK)); // 146
        blocks.push(wp("genesis:wallpaper_red", [0.78, 0.18, 0.16], TEX_WALLPAPER_RED)); // 147
        blocks.push(wp("genesis:wallpaper_blue", [0.24, 0.36, 0.82], TEX_WALLPAPER_BLUE)); // 148
        blocks.push(wp("genesis:wallpaper_yellow", [0.93, 0.84, 0.22], TEX_WALLPAPER_YELLOW)); // 149
        blocks.push(wp("genesis:wallpaper_orange", [0.88, 0.50, 0.16], TEX_WALLPAPER_ORANGE)); // 150
        blocks.push(wp("genesis:wallpaper_green", [0.32, 0.62, 0.24], TEX_WALLPAPER_GREEN)); // 151
        blocks.push(wp("genesis:wallpaper_purple", [0.52, 0.26, 0.68], TEX_WALLPAPER_PURPLE)); // 152
        blocks.push(wp("genesis:wallpaper_pink", [0.92, 0.58, 0.72], TEX_WALLPAPER_PINK)); // 153
        blocks.push(wp("genesis:wallpaper_lime", [0.56, 0.84, 0.32], TEX_WALLPAPER_LIME)); // 154
        blocks.push(wp("genesis:wallpaper_light_blue", [0.48, 0.72, 0.92], TEX_WALLPAPER_LIGHT_BLUE)); // 155
        blocks.push(wp("genesis:wallpaper_grey", [0.45, 0.45, 0.48], TEX_WALLPAPER_GREY)); // 156
        blocks.push(wp("genesis:wallpaper_light_grey", [0.72, 0.72, 0.74], TEX_WALLPAPER_LIGHT_GREY)); // 157

        // 158: Latent Print (Spec 38). Visually identical to Blueprint
        // Paper (parchment-cream); the world-side block-entity carries
        // the develop state. Solid + opaque + walkable like a paper
        // tile — the player builds nothing on it, they just leave it
        // out to catch the sun.
        blocks.push(BlockDef {
            name: "genesis:latent_print",
            solid: true, transparent: false, gravity: false,
            color: [0.85, 0.78, 0.55],
            tex_top: TEX_BLUEPRINT_PAPER_TOP,
            tex_bottom: TEX_BLUEPRINT_PAPER_SIDE,
            tex_side: TEX_BLUEPRINT_PAPER_SIDE,
        });

        // 159-161 — Spec 35 Phase 2 completion (2026-05-28). Brown /
        // Cyan / Magenta wallpapers — the 3-input-mix companion to the
        // existing 13 dyed-paper variants.
        blocks.push(wp("genesis:wallpaper_brown", [0.45, 0.29, 0.18], TEX_WALLPAPER_BROWN)); // 159
        blocks.push(wp("genesis:wallpaper_cyan", [0.22, 0.72, 0.78], TEX_WALLPAPER_CYAN)); // 160
        blocks.push(wp("genesis:wallpaper_magenta", [0.82, 0.27, 0.65], TEX_WALLPAPER_MAGENTA)); // 161

        // 162-170 — Spec 35 farmable-flower follow-on (2026-05-28). All
        // crop-stage flowers are non-solid + transparent + tinted by
        // their flower's signature colour, matching the wild flower
        // blocks at 131-133. Pulled out of a small helper so the nine
        // entries don't repeat the pattern.
        let crop_flower = |name, color: [f32; 3], tex| BlockDef {
            name,
            solid: false, transparent: true, gravity: false,
            color,
            tex_top: tex, tex_bottom: tex, tex_side: tex,
        };
        blocks.push(crop_flower("genesis:cornflower_stage_0", [0.55, 0.65, 0.42], TEX_CORNFLOWER_STAGE_0));
        blocks.push(crop_flower("genesis:cornflower_stage_1", [0.45, 0.55, 0.55], TEX_CORNFLOWER_STAGE_1));
        blocks.push(crop_flower("genesis:cornflower_stage_2", [0.36, 0.46, 0.72], TEX_CORNFLOWER_STAGE_2));
        blocks.push(crop_flower("genesis:field_poppy_stage_0", [0.55, 0.65, 0.42], TEX_FIELD_POPPY_STAGE_0));
        blocks.push(crop_flower("genesis:field_poppy_stage_1", [0.65, 0.45, 0.36], TEX_FIELD_POPPY_STAGE_1));
        blocks.push(crop_flower("genesis:field_poppy_stage_2", [0.78, 0.22, 0.18], TEX_FIELD_POPPY_STAGE_2));
        blocks.push(crop_flower("genesis:buttercup_stage_0", [0.55, 0.65, 0.42], TEX_BUTTERCUP_STAGE_0));
        blocks.push(crop_flower("genesis:buttercup_stage_1", [0.78, 0.74, 0.36], TEX_BUTTERCUP_STAGE_1));
        blocks.push(crop_flower("genesis:buttercup_stage_2", [0.92, 0.84, 0.22], TEX_BUTTERCUP_STAGE_2));

        // 171 — Spec 36 Fences mini-spec. Solid + opaque cube using
        // TEX_OAK_PLANKS so it slots in next to plank buildings without
        // a custom texture. v2 specs can subclass / re-texture per
        // wood species; for the alpha there's one post.
        blocks.push(BlockDef {
            name: "genesis:fence_post",
            solid: true, transparent: false, gravity: false,
            color: [0.62, 0.50, 0.32],
            tex_top: TEX_OAK_PLANKS,
            tex_bottom: TEX_OAK_PLANKS,
            tex_side: TEX_OAK_PLANKS,
        });

        // 172 — Spec 38 cyanotype-art (2026-05-28). Wall-décor block;
        // solid + opaque so it occupies the placement face cleanly.
        // Single blueprint-blue + white-frame texture for v1.
        blocks.push(BlockDef {
            name: "genesis:cyanotype_print",
            solid: true, transparent: false, gravity: false,
            color: [0.16, 0.32, 0.62],
            tex_top: TEX_CYANOTYPE_PRINT,
            tex_bottom: TEX_CYANOTYPE_PRINT,
            tex_side: TEX_CYANOTYPE_PRINT,
        });

        // 173-188 — Spec 35 dyed-décor Bunting (2026-05-28). Non-solid
        // + transparent X-mesh decorative blocks. Same shape as the
        // tall-grass / flower render path. Tint mirrors the existing
        // wallpaper palette for the same dye.
        let bunting = |name: &'static str, color: [f32; 3], tex: u32| BlockDef {
            name,
            solid: false, transparent: true, gravity: false,
            color, tex_top: tex, tex_bottom: tex, tex_side: tex,
        };
        blocks.push(bunting("genesis:bunting_white", [0.93, 0.93, 0.93], TEX_BUNTING_WHITE)); // 173
        blocks.push(bunting("genesis:bunting_black", [0.13, 0.13, 0.15], TEX_BUNTING_BLACK)); // 174
        blocks.push(bunting("genesis:bunting_red", [0.82, 0.18, 0.16], TEX_BUNTING_RED)); // 175
        blocks.push(bunting("genesis:bunting_blue", [0.24, 0.36, 0.82], TEX_BUNTING_BLUE)); // 176
        blocks.push(bunting("genesis:bunting_yellow", [0.93, 0.84, 0.22], TEX_BUNTING_YELLOW)); // 177
        blocks.push(bunting("genesis:bunting_orange", [0.88, 0.50, 0.16], TEX_BUNTING_ORANGE)); // 178
        blocks.push(bunting("genesis:bunting_green", [0.32, 0.62, 0.24], TEX_BUNTING_GREEN)); // 179
        blocks.push(bunting("genesis:bunting_purple", [0.52, 0.26, 0.68], TEX_BUNTING_PURPLE)); // 180
        blocks.push(bunting("genesis:bunting_pink", [0.92, 0.58, 0.72], TEX_BUNTING_PINK)); // 181
        blocks.push(bunting("genesis:bunting_lime", [0.56, 0.84, 0.32], TEX_BUNTING_LIME)); // 182
        blocks.push(bunting("genesis:bunting_light_blue", [0.48, 0.72, 0.92], TEX_BUNTING_LIGHT_BLUE)); // 183
        blocks.push(bunting("genesis:bunting_grey", [0.45, 0.45, 0.48], TEX_BUNTING_GREY)); // 184
        blocks.push(bunting("genesis:bunting_light_grey", [0.72, 0.72, 0.74], TEX_BUNTING_LIGHT_GREY)); // 185
        blocks.push(bunting("genesis:bunting_brown", [0.45, 0.29, 0.18], TEX_BUNTING_BROWN)); // 186
        blocks.push(bunting("genesis:bunting_cyan", [0.22, 0.72, 0.78], TEX_BUNTING_CYAN)); // 187
        blocks.push(bunting("genesis:bunting_magenta", [0.82, 0.27, 0.65], TEX_BUNTING_MAGENTA)); // 188

        // 189-204 — Spec 35 dyed-décor Paper Lantern (2026-05-28).
        // Solid + opaque cubes; light emission set in
        // `light_emission` below (level 12). Tint matches the dye.
        let lantern = |name: &'static str, color: [f32; 3], tex: u32| BlockDef {
            name,
            solid: true, transparent: false, gravity: false,
            color, tex_top: tex, tex_bottom: tex, tex_side: tex,
        };
        blocks.push(lantern("genesis:paper_lantern_white", [0.97, 0.97, 0.92], TEX_PAPER_LANTERN_WHITE)); // 189
        blocks.push(lantern("genesis:paper_lantern_black", [0.28, 0.28, 0.32], TEX_PAPER_LANTERN_BLACK)); // 190
        blocks.push(lantern("genesis:paper_lantern_red", [0.92, 0.45, 0.40], TEX_PAPER_LANTERN_RED)); // 191
        blocks.push(lantern("genesis:paper_lantern_blue", [0.50, 0.65, 0.92], TEX_PAPER_LANTERN_BLUE)); // 192
        blocks.push(lantern("genesis:paper_lantern_yellow", [0.97, 0.92, 0.55], TEX_PAPER_LANTERN_YELLOW)); // 193
        blocks.push(lantern("genesis:paper_lantern_orange", [0.95, 0.70, 0.40], TEX_PAPER_LANTERN_ORANGE)); // 194
        blocks.push(lantern("genesis:paper_lantern_green", [0.58, 0.82, 0.48], TEX_PAPER_LANTERN_GREEN)); // 195
        blocks.push(lantern("genesis:paper_lantern_purple", [0.72, 0.55, 0.85], TEX_PAPER_LANTERN_PURPLE)); // 196
        blocks.push(lantern("genesis:paper_lantern_pink", [0.97, 0.78, 0.85], TEX_PAPER_LANTERN_PINK)); // 197
        blocks.push(lantern("genesis:paper_lantern_lime", [0.78, 0.92, 0.55], TEX_PAPER_LANTERN_LIME)); // 198
        blocks.push(lantern("genesis:paper_lantern_light_blue", [0.72, 0.88, 0.96], TEX_PAPER_LANTERN_LIGHT_BLUE)); // 199
        blocks.push(lantern("genesis:paper_lantern_grey", [0.65, 0.65, 0.68], TEX_PAPER_LANTERN_GREY)); // 200
        blocks.push(lantern("genesis:paper_lantern_light_grey", [0.85, 0.85, 0.88], TEX_PAPER_LANTERN_LIGHT_GREY)); // 201
        blocks.push(lantern("genesis:paper_lantern_brown", [0.72, 0.55, 0.42], TEX_PAPER_LANTERN_BROWN)); // 202
        blocks.push(lantern("genesis:paper_lantern_cyan", [0.55, 0.88, 0.92], TEX_PAPER_LANTERN_CYAN)); // 203
        blocks.push(lantern("genesis:paper_lantern_magenta", [0.92, 0.55, 0.82], TEX_PAPER_LANTERN_MAGENTA)); // 204

        // 205-220 — Spec 35 dyed-décor Kite (2026-05-28). Non-solid +
        // transparent X-mesh; same render path as bunting. Tint
        // mirrors the wallpaper palette.
        let kite = |name: &'static str, color: [f32; 3], tex: u32| BlockDef {
            name,
            solid: false, transparent: true, gravity: false,
            color, tex_top: tex, tex_bottom: tex, tex_side: tex,
        };
        blocks.push(kite("genesis:kite_white", [0.93, 0.93, 0.93], TEX_KITE_WHITE)); // 205
        blocks.push(kite("genesis:kite_black", [0.13, 0.13, 0.15], TEX_KITE_BLACK)); // 206
        blocks.push(kite("genesis:kite_red", [0.82, 0.18, 0.16], TEX_KITE_RED)); // 207
        blocks.push(kite("genesis:kite_blue", [0.24, 0.36, 0.82], TEX_KITE_BLUE)); // 208
        blocks.push(kite("genesis:kite_yellow", [0.93, 0.84, 0.22], TEX_KITE_YELLOW)); // 209
        blocks.push(kite("genesis:kite_orange", [0.88, 0.50, 0.16], TEX_KITE_ORANGE)); // 210
        blocks.push(kite("genesis:kite_green", [0.32, 0.62, 0.24], TEX_KITE_GREEN)); // 211
        blocks.push(kite("genesis:kite_purple", [0.52, 0.26, 0.68], TEX_KITE_PURPLE)); // 212
        blocks.push(kite("genesis:kite_pink", [0.92, 0.58, 0.72], TEX_KITE_PINK)); // 213
        blocks.push(kite("genesis:kite_lime", [0.56, 0.84, 0.32], TEX_KITE_LIME)); // 214
        blocks.push(kite("genesis:kite_light_blue", [0.48, 0.72, 0.92], TEX_KITE_LIGHT_BLUE)); // 215
        blocks.push(kite("genesis:kite_grey", [0.45, 0.45, 0.48], TEX_KITE_GREY)); // 216
        blocks.push(kite("genesis:kite_light_grey", [0.72, 0.72, 0.74], TEX_KITE_LIGHT_GREY)); // 217
        blocks.push(kite("genesis:kite_brown", [0.45, 0.29, 0.18], TEX_KITE_BROWN)); // 218
        blocks.push(kite("genesis:kite_cyan", [0.22, 0.72, 0.78], TEX_KITE_CYAN)); // 219
        blocks.push(kite("genesis:kite_magenta", [0.82, 0.27, 0.65], TEX_KITE_MAGENTA)); // 220

        // 221-236 — Banner block (2026-05-28). Solid + opaque cube;
        // tint from the dye, texture wraps a flag-on-pole motif.
        let banner = |name: &'static str, color: [f32; 3], tex: u32| BlockDef {
            name,
            solid: true, transparent: false, gravity: false,
            color, tex_top: tex, tex_bottom: tex, tex_side: tex,
        };
        blocks.push(banner("genesis:banner_white", [0.93, 0.93, 0.93], TEX_BANNER_WHITE)); // 221
        blocks.push(banner("genesis:banner_black", [0.13, 0.13, 0.15], TEX_BANNER_BLACK)); // 222
        blocks.push(banner("genesis:banner_red", [0.82, 0.18, 0.16], TEX_BANNER_RED)); // 223
        blocks.push(banner("genesis:banner_blue", [0.24, 0.36, 0.82], TEX_BANNER_BLUE)); // 224
        blocks.push(banner("genesis:banner_yellow", [0.93, 0.84, 0.22], TEX_BANNER_YELLOW)); // 225
        blocks.push(banner("genesis:banner_orange", [0.88, 0.50, 0.16], TEX_BANNER_ORANGE)); // 226
        blocks.push(banner("genesis:banner_green", [0.32, 0.62, 0.24], TEX_BANNER_GREEN)); // 227
        blocks.push(banner("genesis:banner_purple", [0.52, 0.26, 0.68], TEX_BANNER_PURPLE)); // 228
        blocks.push(banner("genesis:banner_pink", [0.92, 0.58, 0.72], TEX_BANNER_PINK)); // 229
        blocks.push(banner("genesis:banner_lime", [0.56, 0.84, 0.32], TEX_BANNER_LIME)); // 230
        blocks.push(banner("genesis:banner_light_blue", [0.48, 0.72, 0.92], TEX_BANNER_LIGHT_BLUE)); // 231
        blocks.push(banner("genesis:banner_grey", [0.45, 0.45, 0.48], TEX_BANNER_GREY)); // 232
        blocks.push(banner("genesis:banner_light_grey", [0.72, 0.72, 0.74], TEX_BANNER_LIGHT_GREY)); // 233
        blocks.push(banner("genesis:banner_brown", [0.45, 0.29, 0.18], TEX_BANNER_BROWN)); // 234
        blocks.push(banner("genesis:banner_cyan", [0.22, 0.72, 0.78], TEX_BANNER_CYAN)); // 235
        blocks.push(banner("genesis:banner_magenta", [0.82, 0.27, 0.65], TEX_BANNER_MAGENTA)); // 236

        // 237-252 — Sail block (2026-05-28). Solid + opaque cube;
        // same constructor as banner (the difference is the texture's
        // weave + the recipe's Canvas input).
        let sail = |name: &'static str, color: [f32; 3], tex: u32| BlockDef {
            name,
            solid: true, transparent: false, gravity: false,
            color, tex_top: tex, tex_bottom: tex, tex_side: tex,
        };
        blocks.push(sail("genesis:sail_white", [0.93, 0.93, 0.93], TEX_SAIL_WHITE)); // 237
        blocks.push(sail("genesis:sail_black", [0.13, 0.13, 0.15], TEX_SAIL_BLACK)); // 238
        blocks.push(sail("genesis:sail_red", [0.82, 0.18, 0.16], TEX_SAIL_RED)); // 239
        blocks.push(sail("genesis:sail_blue", [0.24, 0.36, 0.82], TEX_SAIL_BLUE)); // 240
        blocks.push(sail("genesis:sail_yellow", [0.93, 0.84, 0.22], TEX_SAIL_YELLOW)); // 241
        blocks.push(sail("genesis:sail_orange", [0.88, 0.50, 0.16], TEX_SAIL_ORANGE)); // 242
        blocks.push(sail("genesis:sail_green", [0.32, 0.62, 0.24], TEX_SAIL_GREEN)); // 243
        blocks.push(sail("genesis:sail_purple", [0.52, 0.26, 0.68], TEX_SAIL_PURPLE)); // 244
        blocks.push(sail("genesis:sail_pink", [0.92, 0.58, 0.72], TEX_SAIL_PINK)); // 245
        blocks.push(sail("genesis:sail_lime", [0.56, 0.84, 0.32], TEX_SAIL_LIME)); // 246
        blocks.push(sail("genesis:sail_light_blue", [0.48, 0.72, 0.92], TEX_SAIL_LIGHT_BLUE)); // 247
        blocks.push(sail("genesis:sail_grey", [0.45, 0.45, 0.48], TEX_SAIL_GREY)); // 248
        blocks.push(sail("genesis:sail_light_grey", [0.72, 0.72, 0.74], TEX_SAIL_LIGHT_GREY)); // 249
        blocks.push(sail("genesis:sail_brown", [0.45, 0.29, 0.18], TEX_SAIL_BROWN)); // 250
        blocks.push(sail("genesis:sail_cyan", [0.22, 0.72, 0.78], TEX_SAIL_CYAN)); // 251
        blocks.push(sail("genesis:sail_magenta", [0.82, 0.27, 0.65], TEX_SAIL_MAGENTA)); // 252

        // 253-258 — Per-wood-species fence posts (Fences v2 species
        // slice, 2026-05-28). Same constructor as the canonical
        // FENCE_POST = 171 (oak) but tinted with the species' colour.
        // All reuse TEX_OAK_PLANKS for the face — the tint reads as
        // the species without needing per-species plank textures.
        // Texture pack work that diversifies plank textures is its
        // own future polish; the recipe + species partition are the
        // gameplay-level slice today.
        let species_fence = |name: &'static str, color: [f32; 3]| BlockDef {
            name,
            solid: true, transparent: false, gravity: false,
            color, tex_top: TEX_OAK_PLANKS, tex_bottom: TEX_OAK_PLANKS, tex_side: TEX_OAK_PLANKS,
        };
        blocks.push(species_fence("genesis:birch_fence_post", [0.92, 0.88, 0.72])); // 253
        blocks.push(species_fence("genesis:spruce_fence_post", [0.55, 0.38, 0.22])); // 254
        blocks.push(species_fence("genesis:jungle_fence_post", [0.72, 0.52, 0.30])); // 255
        blocks.push(species_fence("genesis:acacia_fence_post", [0.85, 0.55, 0.20])); // 256
        blocks.push(species_fence("genesis:dark_oak_fence_post", [0.35, 0.22, 0.12])); // 257
        blocks.push(species_fence("genesis:rubber_fence_post", [0.46, 0.36, 0.28])); // 258

        // 259 — Tent (2026-05-28). Solid + opaque canvas-dome cube;
        // first multi-block-décor primitive (v1 is single-cube; the
        // multi-block family-tent variant is future polish). Canvas
        // beige tint with the dedicated `gen_tent` texture below.
        blocks.push(BlockDef {
            name: "genesis:tent",
            solid: true, transparent: false, gravity: false,
            color: [0.78, 0.70, 0.50],
            tex_top: TEX_TENT, tex_bottom: TEX_TENT, tex_side: TEX_TENT,
        });

        // 260 — Track (Rail freight Phase 1). Flat directional rail you
        // walk *over*, not into: non-solid + transparent so it renders as
        // a thin ground slab (see `mesh.rs::non_solid_shape_for`) and never
        // blocks movement. Steel-on-sleeper `gen_track` texture below.
        blocks.push(BlockDef {
            name: "genesis:track",
            solid: false, transparent: true, gravity: false,
            color: [0.40, 0.32, 0.22],
            tex_top: TEX_TRACK, tex_bottom: TEX_TRACK, tex_side: TEX_TRACK,
        });

        // 261 — Ladder (#30). Non-solid + transparent so you stand *inside* it
        // and climb (see `physics` climb branch + `is_climbable`); rendered as a
        // thin back-panel via `non_solid_shape_for`. Oak rail-and-rung texture.
        blocks.push(BlockDef {
            name: "genesis:ladder",
            solid: false, transparent: true, gravity: false,
            color: [0.47, 0.34, 0.19],
            tex_top: TEX_LADDER, tex_bottom: TEX_LADDER, tex_side: TEX_LADDER,
        });

        // 262 — Carpet (#30). Non-solid thin decorative top layer; you walk on
        // the block beneath it, so no collision change. Near-white tintable wool.
        blocks.push(BlockDef {
            name: "genesis:carpet",
            solid: false, transparent: true, gravity: false,
            color: [0.85, 0.40, 0.42],
            tex_top: TEX_CARPET, tex_bottom: TEX_CARPET, tex_side: TEX_CARPET,
        });

        // 263 — Grave (#47). Solid headstone placed at death holding the player's
        // inventory snapshot (`BlockEntityData::Grave`). Right-click recovers,
        // break spills. Opaque grey stone.
        blocks.push(BlockDef {
            name: "genesis:grave",
            solid: true, transparent: false, gravity: false,
            color: [0.58, 0.58, 0.60],
            tex_top: TEX_GRAVE, tex_bottom: TEX_GRAVE, tex_side: TEX_GRAVE,
        });

        // #15 — tiered-storage chests (264-267). Same chest body, tinted per
        // tier; bigger capacity + (top tiers) auto-collect handled in `chest.rs`.
        blocks.push(BlockDef { // 264 COPPER_CHEST
            name: "genesis:copper_chest",
            solid: true, transparent: false, gravity: false,
            color: [0.69, 0.42, 0.27],
            tex_top: TEX_COPPER_CHEST_TOP, tex_bottom: TEX_COPPER_CHEST_SIDE, tex_side: TEX_COPPER_CHEST_SIDE,
        });
        blocks.push(BlockDef { // 265 IRON_CHEST
            name: "genesis:iron_chest",
            solid: true, transparent: false, gravity: false,
            color: [0.60, 0.60, 0.63],
            tex_top: TEX_IRON_CHEST_TOP, tex_bottom: TEX_IRON_CHEST_SIDE, tex_side: TEX_IRON_CHEST_SIDE,
        });
        blocks.push(BlockDef { // 266 DIAMOND_CHEST
            name: "genesis:diamond_chest",
            solid: true, transparent: false, gravity: false,
            color: [0.44, 0.78, 0.80],
            tex_top: TEX_DIAMOND_CHEST_TOP, tex_bottom: TEX_DIAMOND_CHEST_SIDE, tex_side: TEX_DIAMOND_CHEST_SIDE,
        });
        blocks.push(BlockDef { // 267 SATORI_CHEST
            name: "genesis:satori_chest",
            solid: true, transparent: false, gravity: false,
            color: [0.86, 0.59, 0.22],
            tex_top: TEX_SATORI_CHEST_TOP, tex_bottom: TEX_SATORI_CHEST_SIDE, tex_side: TEX_SATORI_CHEST_SIDE,
        });

        // ─── Spec 48 (Electricity) ids 268..=279 ───
        // Procedural textures (Spec 48 #5A): one identifiable texture per block.
        // Per-face directional orientation (lever throw, generator front) is the
        // deferred #5B mesh pass; lit variants differ from unlit so the power
        // tick's block-id flip reads as on/off.
        blocks.push(BlockDef { // 268 CABLE
            name: "electricity:cable",
            solid: false, transparent: true, gravity: false,
            color: [0.80, 0.50, 0.20],
            tex_top: TEX_CABLE, tex_bottom: TEX_CABLE, tex_side: TEX_CABLE,
        });
        blocks.push(BlockDef { // 269 CABLE_LIT
            name: "electricity:cable_lit",
            solid: false, transparent: true, gravity: false,
            color: [1.0, 0.75, 0.30],
            tex_top: TEX_CABLE_LIT, tex_bottom: TEX_CABLE_LIT, tex_side: TEX_CABLE_LIT,
        });
        blocks.push(BlockDef { // 270 ELECTRIC_LAMP
            name: "electricity:lamp",
            solid: true, transparent: false, gravity: false,
            color: [0.85, 0.85, 0.60],
            tex_top: TEX_ELECTRIC_LAMP, tex_bottom: TEX_ELECTRIC_LAMP, tex_side: TEX_ELECTRIC_LAMP,
        });
        blocks.push(BlockDef { // 271 ELECTRIC_LAMP_LIT
            name: "electricity:lamp_lit",
            solid: true, transparent: false, gravity: false,
            color: [1.0, 0.98, 0.75],
            tex_top: TEX_ELECTRIC_LAMP_LIT, tex_bottom: TEX_ELECTRIC_LAMP_LIT, tex_side: TEX_ELECTRIC_LAMP_LIT,
        });
        blocks.push(BlockDef { // 272 LEVER
            name: "electricity:lever",
            solid: false, transparent: true, gravity: false,
            color: [0.55, 0.40, 0.25],
            tex_top: TEX_LEVER, tex_bottom: TEX_LEVER, tex_side: TEX_LEVER,
        });
        blocks.push(BlockDef { // 273 BUTTON
            name: "electricity:button",
            solid: false, transparent: true, gravity: false,
            color: [0.60, 0.45, 0.30],
            tex_top: TEX_BUTTON, tex_bottom: TEX_BUTTON, tex_side: TEX_BUTTON,
        });
        blocks.push(BlockDef { // 274 PRESSURE_PLATE
            name: "electricity:pressure_plate",
            solid: false, transparent: true, gravity: false,
            color: [0.62, 0.47, 0.32],
            tex_top: TEX_PRESSURE_PLATE, tex_bottom: TEX_PRESSURE_PLATE, tex_side: TEX_PRESSURE_PLATE,
        });
        blocks.push(BlockDef { // 275 LOGIC_GATE
            name: "electricity:logic_gate",
            solid: false, transparent: true, gravity: false,
            color: [0.70, 0.70, 0.72],
            tex_top: TEX_LOGIC_GATE, tex_bottom: TEX_LOGIC_GATE, tex_side: TEX_LOGIC_GATE,
        });
        blocks.push(BlockDef { // 276 HAND_CRANK
            name: "electricity:hand_crank",
            solid: true, transparent: false, gravity: false,
            color: [0.55, 0.40, 0.25],
            tex_top: TEX_HAND_CRANK, tex_bottom: TEX_HAND_CRANK, tex_side: TEX_HAND_CRANK,
        });
        blocks.push(BlockDef { // 277 STEAM_GENERATOR
            name: "electricity:steam_generator",
            solid: true, transparent: false, gravity: false,
            color: [0.45, 0.45, 0.48],
            tex_top: TEX_STEAM_GENERATOR, tex_bottom: TEX_STEAM_GENERATOR, tex_side: TEX_STEAM_GENERATOR,
        });
        blocks.push(BlockDef { // 278 STEAM_GENERATOR_LIT
            name: "electricity:steam_generator_lit",
            solid: true, transparent: false, gravity: false,
            color: [0.70, 0.55, 0.30],
            tex_top: TEX_STEAM_GENERATOR_LIT, tex_bottom: TEX_STEAM_GENERATOR, tex_side: TEX_STEAM_GENERATOR_LIT,
        });
        blocks.push(BlockDef { // 279 BATTERY
            name: "electricity:battery",
            solid: true, transparent: false, gravity: false,
            color: [0.30, 0.55, 0.35],
            tex_top: TEX_BATTERY, tex_bottom: TEX_BATTERY, tex_side: TEX_BATTERY,
        });
        // ─── Spec 48 Phase 2 — sensors (ids 280..=282) ───
        blocks.push(BlockDef { // 280 BEAM_SENSOR
            name: "electricity:beam_sensor",
            solid: true, transparent: false, gravity: false,
            color: [0.55, 0.20, 0.20],
            tex_top: TEX_BEAM_SENSOR, tex_bottom: TEX_BEAM_SENSOR, tex_side: TEX_BEAM_SENSOR,
        });
        blocks.push(BlockDef { // 281 MIRROR
            name: "electricity:mirror",
            solid: true, transparent: false, gravity: false,
            color: [0.80, 0.85, 0.90],
            tex_top: TEX_MIRROR, tex_bottom: TEX_MIRROR, tex_side: TEX_MIRROR,
        });
        blocks.push(BlockDef { // 282 MOTION_SENSOR
            name: "electricity:motion_sensor",
            solid: true, transparent: false, gravity: false,
            color: [0.30, 0.45, 0.55],
            tex_top: TEX_MOTION_SENSOR, tex_bottom: TEX_MOTION_SENSOR, tex_side: TEX_MOTION_SENSOR,
        });
        // F1 shaped blocks. `transparent: true` so neighbours render their faces
        // toward the open half (you see the floor under a slab); `solid: true`
        // keeps them obstacles for mob AI / lighting. They are excluded from the
        // greedy pass by `block_shape::is_shaped` and drawn by the shaped-emit
        // branch of `emit_non_solid_blocks` — exactly like micro-model blocks.
        blocks.push(BlockDef { // 283 STONE_SLAB
            name: "genesis:stone_slab",
            solid: true, transparent: true, gravity: false,
            color: [0.50, 0.50, 0.50],
            tex_top: TEX_STONE, tex_bottom: TEX_STONE, tex_side: TEX_STONE,
        });
        blocks.push(BlockDef { // 284 STONE_STAIRS
            name: "genesis:stone_stairs",
            solid: true, transparent: true, gravity: false,
            color: [0.50, 0.50, 0.50],
            tex_top: TEX_STONE, tex_bottom: TEX_STONE, tex_side: TEX_STONE,
        });
        blocks.push(BlockDef { // 285 OAK_FENCE_GATE
            name: "genesis:oak_fence_gate",
            solid: true, transparent: true, gravity: false,
            color: [0.62, 0.49, 0.30],
            tex_top: TEX_OAK_PLANKS, tex_bottom: TEX_OAK_PLANKS, tex_side: TEX_OAK_PLANKS,
        });
        blocks.push(BlockDef { // 286 OAK_TRAPDOOR
            name: "genesis:oak_trapdoor",
            solid: true, transparent: true, gravity: false,
            color: [0.62, 0.49, 0.30],
            tex_top: TEX_OAK_PLANKS, tex_bottom: TEX_OAK_PLANKS, tex_side: TEX_OAK_PLANKS,
        });
        blocks.push(BlockDef { // 287 GLASS_PANE
            name: "genesis:glass_pane",
            solid: true, transparent: true, gravity: false,
            color: [0.78, 0.90, 0.95],
            tex_top: TEX_GLASS, tex_bottom: TEX_GLASS, tex_side: TEX_GLASS,
        });
        blocks.push(BlockDef { // 288 IRON_BARS
            name: "genesis:iron_bars",
            solid: true, transparent: true, gravity: false,
            color: [0.80, 0.80, 0.82],
            tex_top: TEX_IRON_BLOCK, tex_bottom: TEX_IRON_BLOCK, tex_side: TEX_IRON_BLOCK,
        });
        blocks.push(BlockDef { // 289 OAK_DOOR
            name: "genesis:oak_door",
            solid: true, transparent: true, gravity: false,
            color: [0.62, 0.49, 0.30],
            tex_top: TEX_OAK_PLANKS, tex_bottom: TEX_OAK_PLANKS, tex_side: TEX_OAK_PLANKS,
        });
        blocks.push(BlockDef { // 290 COBBLESTONE_WALL
            name: "genesis:cobblestone_wall",
            solid: true, transparent: true, gravity: false,
            color: [0.42, 0.42, 0.42],
            tex_top: TEX_COBBLESTONE, tex_bottom: TEX_COBBLESTONE, tex_side: TEX_COBBLESTONE,
        });
        // Sign — pass-through (solid: false) thin board; text in a block-entity.
        blocks.push(BlockDef { // 291 OAK_SIGN
            name: "genesis:oak_sign",
            solid: false, transparent: true, gravity: false,
            color: [0.62, 0.49, 0.30],
            tex_top: TEX_OAK_PLANKS, tex_bottom: TEX_OAK_PLANKS, tex_side: TEX_OAK_PLANKS,
        });
        // Item Frame — pass-through thin plate; displayed item in a block-entity.
        blocks.push(BlockDef { // 292 ITEM_FRAME
            name: "genesis:item_frame",
            solid: false, transparent: true, gravity: false,
            color: [0.62, 0.49, 0.30],
            tex_top: TEX_OAK_PLANKS, tex_bottom: TEX_OAK_PLANKS, tex_side: TEX_OAK_PLANKS,
        });

        // ── Spec 49 (Explosives) — 293..=297. Order = id; never reorder. ──
        // Brimstone (sulphur ore) — yellow-green crystalline on dark rock.
        blocks.push(BlockDef { // 293 BRIMSTONE
            name: "genesis:brimstone",
            solid: true, transparent: false, gravity: false,
            color: [0.78, 0.74, 0.22],
            tex_top: TEX_BRIMSTONE, tex_bottom: TEX_BRIMSTONE, tex_side: TEX_BRIMSTONE,
        });
        // Nitre ore — pale white-grey crust.
        blocks.push(BlockDef { // 294 NITRE_ORE
            name: "genesis:nitre_ore",
            solid: true, transparent: false, gravity: false,
            color: [0.82, 0.82, 0.80],
            tex_top: TEX_NITRE_ORE, tex_bottom: TEX_NITRE_ORE, tex_side: TEX_NITRE_ORE,
        });
        // Composter — slatted wood bin (block-entity host).
        blocks.push(BlockDef { // 295 COMPOSTER
            name: "genesis:composter",
            solid: true, transparent: false, gravity: false,
            color: [0.46, 0.34, 0.20],
            tex_top: TEX_COMPOSTER, tex_bottom: TEX_COMPOSTER, tex_side: TEX_COMPOSTER,
        });
        // Blasting Keg — banded barrel with a visible fuse (NOT red, no lettering).
        blocks.push(BlockDef { // 296 BLASTING_KEG
            name: "genesis:blasting_keg",
            solid: true, transparent: false, gravity: false,
            color: [0.42, 0.28, 0.16],
            tex_top: TEX_BLASTING_KEG, tex_bottom: TEX_BLASTING_KEG, tex_side: TEX_BLASTING_KEG,
        });
        // Plunger Detonator — boxed T-handle switch (directional via block_meta).
        blocks.push(BlockDef { // 297 PLUNGER_DETONATOR
            name: "electricity:plunger_detonator",
            solid: true, transparent: false, gravity: false,
            color: [0.30, 0.22, 0.18],
            tex_top: TEX_PLUNGER_DETONATOR, tex_bottom: TEX_PLUNGER_DETONATOR, tex_side: TEX_PLUNGER_DETONATOR,
        });
        // P7 — Hopper. v1 reuses the furnace's dark, receptacle-like textures
        // (BRIDGE: a dedicated funnel sprite + shape land in a later texture wave).
        blocks.push(BlockDef { // 298 HOPPER
            name: "genesis:hopper",
            solid: true, transparent: false, gravity: false,
            color: [0.28, 0.28, 0.30],
            tex_top: TEX_FURNACE_TOP, tex_bottom: TEX_FURNACE_TOP, tex_side: TEX_FURNACE_SIDE_UNLIT,
        });
        // P10 — Lava. Non-solid (you fall in + burn) + opaque fiery glow.
        // v1 reuses the lit-furnace face texture (BRIDGE: a dedicated animated
        // lava texture lands in a later texture wave). Light emission = 15 is
        // set in `light_emission`.
        blocks.push(BlockDef { // 299 LAVA
            // transparent:true routes lava through the non-solid small-cube
            // emit path (`mesh::emit_non_solid_blocks` + `non_solid_shape_for`,
            // which has a dedicated full-block lava shape) so it renders as a
            // glowing fluid block rather than being culled like a solid.
            name: "genesis:lava",
            solid: false, transparent: true, gravity: false,
            color: [0.95, 0.42, 0.12],
            tex_top: TEX_FURNACE_SIDE_LIT, tex_bottom: TEX_FURNACE_SIDE_LIT, tex_side: TEX_FURNACE_SIDE_LIT,
        });
        // P11 — Piston body. Reuses iron-block + planks textures for v1 (BRIDGE:
        // a dedicated directional piston face lands in a later texture wave).
        blocks.push(BlockDef { // 300 PISTON
            name: "genesis:piston",
            solid: true, transparent: false, gravity: false,
            color: [0.55, 0.48, 0.40],
            tex_top: TEX_OAK_PLANKS, tex_bottom: TEX_IRON_BLOCK, tex_side: TEX_IRON_BLOCK,
        });
        // P11 — Piston head (the extended arm).
        blocks.push(BlockDef { // 301 PISTON_HEAD
            name: "genesis:piston_head",
            solid: true, transparent: false, gravity: false,
            color: [0.70, 0.58, 0.40],
            tex_top: TEX_OAK_PLANKS, tex_bottom: TEX_OAK_PLANKS, tex_side: TEX_OAK_PLANKS,
        });
        // Campaign B — Obsidian (water-quenched lava). Very dark, fully solid.
        // v1 reuses the coal-block texture (BRIDGE: a dedicated obsidian texture
        // lands in a later texture wave, like LAVA/PISTON before it).
        blocks.push(BlockDef { // 302 OBSIDIAN
            name: "genesis:obsidian",
            solid: true, transparent: false, gravity: false,
            color: [0.10, 0.06, 0.16],
            tex_top: TEX_COAL_BLOCK, tex_bottom: TEX_COAL_BLOCK, tex_side: TEX_COAL_BLOCK,
        });
        // Campaign D — Sticky Piston. Reuses the piston body textures with a
        // rubber-amber tint (BRIDGE: a sticky face lands in a later texture wave).
        blocks.push(BlockDef { // 303 STICKY_PISTON
            name: "genesis:sticky_piston",
            solid: true, transparent: false, gravity: false,
            color: [0.55, 0.42, 0.24],
            tex_top: TEX_OAK_PLANKS, tex_bottom: TEX_IRON_BLOCK, tex_side: TEX_IRON_BLOCK,
        });
        blocks.push(BlockDef { // 304 FIRE
            name: "genesis:fire",
            solid: false, transparent: true, gravity: false,
            color: [0.95, 0.55, 0.15],
            tex_top: TEX_FIRE, tex_bottom: TEX_FIRE, tex_side: TEX_FIRE,
        });
        blocks.push(BlockDef { // 305 DISPENSER
            name: "electricity:dispenser",
            solid: true, transparent: false, gravity: false,
            color: [0.45, 0.45, 0.47],
            tex_top: TEX_FURNACE_TOP, tex_bottom: TEX_FURNACE_TOP, tex_side: TEX_DISPENSER_SIDE,
        });
        blocks.push(BlockDef { // 306 DROPPER
            name: "electricity:dropper",
            solid: true, transparent: false, gravity: false,
            color: [0.50, 0.50, 0.52],
            tex_top: TEX_FURNACE_TOP, tex_bottom: TEX_FURNACE_TOP, tex_side: TEX_DROPPER_SIDE,
        });
        blocks.push(BlockDef { // 307 SAPLING_OAK
            name: "genesis:sapling_oak",
            solid: false, transparent: true, gravity: false,
            color: [0.35, 0.55, 0.22],
            tex_top: TEX_SAPLING, tex_bottom: TEX_SAPLING, tex_side: TEX_SAPLING,
        });
        blocks.push(BlockDef { // 308 SAPLING_BIRCH
            name: "genesis:sapling_birch",
            solid: false, transparent: true, gravity: false,
            color: [0.55, 0.65, 0.35],
            tex_top: TEX_SAPLING, tex_bottom: TEX_SAPLING, tex_side: TEX_SAPLING,
        });
        blocks.push(BlockDef { // 309 SAPLING_SPRUCE
            name: "genesis:sapling_spruce",
            solid: false, transparent: true, gravity: false,
            color: [0.20, 0.40, 0.25],
            tex_top: TEX_SAPLING, tex_bottom: TEX_SAPLING, tex_side: TEX_SAPLING,
        });
        blocks.push(BlockDef { // 310 SAPLING_JUNGLE
            name: "genesis:sapling_jungle",
            solid: false, transparent: true, gravity: false,
            color: [0.25, 0.60, 0.20],
            tex_top: TEX_SAPLING, tex_bottom: TEX_SAPLING, tex_side: TEX_SAPLING,
        });
        blocks.push(BlockDef { // 311 SAPLING_ACACIA
            name: "genesis:sapling_acacia",
            solid: false, transparent: true, gravity: false,
            color: [0.55, 0.50, 0.20],
            tex_top: TEX_SAPLING, tex_bottom: TEX_SAPLING, tex_side: TEX_SAPLING,
        });
        blocks.push(BlockDef { // 312 SAPLING_DARK_OAK
            name: "genesis:sapling_dark_oak",
            solid: false, transparent: true, gravity: false,
            color: [0.22, 0.35, 0.15],
            tex_top: TEX_SAPLING, tex_bottom: TEX_SAPLING, tex_side: TEX_SAPLING,
        });
        blocks.push(BlockDef { // 313 SAPLING_RUBBER
            name: "genesis:sapling_rubber",
            solid: false, transparent: true, gravity: false,
            color: [0.30, 0.55, 0.35],
            tex_top: TEX_SAPLING, tex_bottom: TEX_SAPLING, tex_side: TEX_SAPLING,
        });
        blocks.push(BlockDef { // 314 SNOW_LAYER
            name: "genesis:snow_layer",
            solid: true, transparent: false, gravity: false,
            color: [0.95, 0.96, 0.98],
            tex_top: TEX_SNOW, tex_bottom: TEX_SNOW, tex_side: TEX_SNOW,
        });
        // Pet Bed (2026-07-06 pets wave) — wool cushion in a wood-crate
        // frame; drops itself (default `other` arm in mine_drop below).
        blocks.push(BlockDef { // 315 PET_BED
            name: "genesis:pet_bed",
            solid: true, transparent: false, gravity: false,
            color: [0.75, 0.30, 0.24],
            tex_top: TEX_PET_BED_TOP, tex_bottom: TEX_OAK_PLANKS, tex_side: TEX_PET_BED_SIDE,
        });
        // Spec 48 Phase 4 (Electricity) — Water Wheel idle / turning. Wooden
        // paddles on an iron axle; the turning twin offsets + blurs the paddles
        // so it reads as spinning. Emits no light (see `light_emission`).
        blocks.push(BlockDef { // 316 WATER_WHEEL
            name: "electricity:water_wheel",
            solid: true, transparent: false, gravity: false,
            color: [0.55, 0.42, 0.28],
            tex_top: TEX_WATER_WHEEL, tex_bottom: TEX_WATER_WHEEL, tex_side: TEX_WATER_WHEEL,
        });
        blocks.push(BlockDef { // 317 WATER_WHEEL_TURNING
            name: "electricity:water_wheel_turning",
            solid: true, transparent: false, gravity: false,
            color: [0.58, 0.46, 0.32],
            tex_top: TEX_WATER_WHEEL_TURNING, tex_bottom: TEX_WATER_WHEEL,
            tex_side: TEX_WATER_WHEEL_TURNING,
        });
        // Wind, Copper & Electricity wave §2.2 — Windmill idle / turning.
        // Canvas sails on a plank tower; the turning twin smears the sails
        // round their hub. Top + bottom are the plank cap (a layer saved).
        // Emits no light (see `light_emission`).
        blocks.push(BlockDef { // 318 WINDMILL
            name: "electricity:windmill",
            solid: true, transparent: false, gravity: false,
            color: [0.72, 0.66, 0.48],
            tex_top: TEX_OAK_PLANKS, tex_bottom: TEX_OAK_PLANKS, tex_side: TEX_WINDMILL,
        });
        blocks.push(BlockDef { // 319 WINDMILL_TURNING
            name: "electricity:windmill_turning",
            solid: true, transparent: false, gravity: false,
            color: [0.76, 0.70, 0.52],
            tex_top: TEX_OAK_PLANKS, tex_bottom: TEX_OAK_PLANKS,
            tex_side: TEX_WINDMILL_TURNING,
        });

        Self { blocks }
    }

    /// What an item-stack is dropped when this block is mined. Most blocks
    /// drop themselves; ores drop the corresponding raw material.
    pub fn mine_drop(&self, id: BlockId) -> crate::item::ItemStack {
        use crate::item::{ItemStack, MaterialId};
        match id {
            // Mining stone yields cobblestone (Minecraft baseline) —
            // the unlocking item for the Stone-tier tool tree and the
            // Furnace recipe (8 cobble ring). The STONE block id stays
            // valid as a placement target (creative `/give`, future
            // smelt-cobble-back-to-stone recipe) — only the natural
            // mining drop changed.
            STONE => ItemStack::new_block(COBBLESTONE, 1),
            COAL_ORE | DEEPSLATE_COAL_ORE => ItemStack::new_material(MaterialId::Coal, 1),
            IRON_ORE | DEEPSLATE_IRON_ORE => ItemStack::new_material(MaterialId::RawIron, 1),
            DIAMOND_ORE | DEEPSLATE_DIAMOND_ORE => ItemStack::new_material(MaterialId::Diamond, 1),
            // Spec 28c — Copper Ore drops Copper raw; smelts to CopperIngot.
            COPPER_ORE => ItemStack::new_material(MaterialId::Copper, 1),
            // Salt — ROCK_SALT ore drops 1 Salt as the deterministic base
            // (mine_drop_with_seed bumps the count to 2 on roughly half
            // of strikes via a position-seeded roll).
            ROCK_SALT => ItemStack::new_material(MaterialId::Salt, 1),
            // Salt — mining SALT_PATH always yields 1 DIRT base; the
            // mine_drop_with_seed variant adds a chance of recovering
            // 1 Salt on top.
            SALT_PATH => ItemStack::new_block(DIRT, 1),
            // Rubber — tapped log drops 1 GreenLog (matches the species-
            // neutral timber convention). The cooldown state is lost
            // when the player fells the tree. The live RUBBER_LOG itself
            // mines to GreenLog via the multi-species log arm above.
            RUBBER_LOG_TAPPED => ItemStack::new_material(MaterialId::GreenLog, 1),
            // Mob Bounty Board — drops itself as a placeable material.
            BOUNTY_BOARD => ItemStack::new_material(MaterialId::BountyBoardItem, 1),
            // Tip Jar — drops itself. Anti-grief on non-owner break is
            // applied at the call site (mirrors Vendor Block); this arm
            // is the canonical owner-break drop.
            TIP_JAR => ItemStack::new_material(MaterialId::TipJarItem, 1),
            // Repair Bench — stateless; drops itself, breaks freely.
            REPAIR_BENCH => ItemStack::new_material(MaterialId::RepairBenchItem, 1),
            // Plot Marker — drops itself. Plot release on owner-break
            // is handled at the game_loop call site (the PlotData entry
            // is removed there); this arm is the canonical drop.
            PLOT_MARKER => ItemStack::new_material(MaterialId::PlotMarkerItem, 1),
            // Market Bell — drops itself. Hub release on owner-break
            // handled at the game_loop call site.
            MARKET_BELL => ItemStack::new_material(MaterialId::MarketBellItem, 1),
            // Auction Block — drops itself. Lot-return + block-entity
            // cleanup on owner-break handled at the game_loop call site.
            AUCTION_BLOCK => ItemStack::new_material(MaterialId::AuctionBlockItem, 1),
            // Bazaar Block — stateless; drops itself, breaks freely.
            BAZAAR_BLOCK => ItemStack::new_material(MaterialId::BazaarBlockItem, 1),
            // T1.5 — mature crops drop their crop + seeds. Immature
            // stages drop their seeds only (the player gets back what
            // they planted but no harvest). Pumpkin is the harvested
            // fruit itself.
            SUGARCANE => ItemStack::new_block(SUGARCANE, 1),
            SUGAR_BEET_STAGE_3 => ItemStack::new_material(MaterialId::SugarBeet, 1),
            SUGAR_BEET_STAGE_0 | SUGAR_BEET_STAGE_1 | SUGAR_BEET_STAGE_2 => {
                ItemStack::new_material(MaterialId::SugarBeetSeeds, 1)
            }
            BEETROOT_STAGE_3 => ItemStack::new_material(MaterialId::Beetroot, 1),
            BEETROOT_STAGE_0 | BEETROOT_STAGE_1 | BEETROOT_STAGE_2 => {
                ItemStack::new_material(MaterialId::BeetrootSeeds, 1)
            }
            PUMPKIN => ItemStack::new_material(MaterialId::PumpkinFood, 1),
            PUMPKIN_STEM_4 => ItemStack::new_material(MaterialId::PumpkinFood, 1),
            PUMPKIN_STEM_0 | PUMPKIN_STEM_1 | PUMPKIN_STEM_2 | PUMPKIN_STEM_3 => {
                ItemStack::empty()
            }
            // Berry Bush at mature stage drops berries; immature drops
            // nothing (the bush regrows naturally, no seed needed).
            BERRY_BUSH_3 => ItemStack::new_material(MaterialId::Berries, 1),
            BERRY_BUSH_0 | BERRY_BUSH_1 | BERRY_BUSH_2 => ItemStack::empty(),
            // Workstation blocks drop themselves. CHEST is included here
            // for the block-id drop; the contents-spill happens upstream
            // in `block_interact`-side break handling via `chest::cleanup_chest`.
            MILL | OVEN | AGING_RACK | BEE_HIVE | CHEST | COPPER_CHEST | IRON_CHEST
            | DIAMOND_CHEST | SATORI_CHEST => ItemStack::new_block(id, 1),
            // Both campfire variants drop the unlit form. The lit/unlit
            // distinction is block-state only — a "lit campfire item" in
            // the inventory would re-place as visually lit until the next
            // tick demotes it (no fuel). Normalising on the unlit drop
            // keeps the inventory icon honest.
            CAMPFIRE | CAMPFIRE_UNLIT => ItemStack::new_block(CAMPFIRE_UNLIT, 1),
            // Furnace variants (Spec 20) follow the same normalisation
            // pattern — drop the unlit form regardless of which variant
            // was mined. The block-entity at this position is removed
            // by the mining handler; on re-place the player starts
            // from a fresh empty furnace.
            FURNACE | FURNACE_LIT => ItemStack::new_block(FURNACE, 1),
            // Spec 48 Phase 4 — a spinning Water Wheel mines back to the idle
            // block (the FURNACE_LIT pattern), so the lit twin never enters an
            // inventory as a distinct item.
            WATER_WHEEL | WATER_WHEEL_TURNING => ItemStack::new_block(WATER_WHEEL, 1),
            // …and a turning Windmill mines back to its idle block, same rule.
            WINDMILL | WINDMILL_TURNING => ItemStack::new_block(WINDMILL, 1),
            // Spec 16 Phase 3b — pure-deepslate visual variants
            // normalise to the canonical id on drop so inventory icons
            // stay consistent (the variant tier was a server-side
            // signal at chunk-gen time; the player's pile of mined
            // deepslate doesn't carry the visual richness through).
            PURE_DEEPSLATE_THIN | PURE_DEEPSLATE_HEALTHY | PURE_DEEPSLATE_FAT => {
                ItemStack::new_block(PURE_DEEPSLATE, 1)
            }
            // Campfire smoke is a render-only pillar. Mining it (e.g. with
            // a creative-mode break) yields nothing. The campfire tick
            // manages its lifecycle.
            CAMPFIRE_SMOKE => ItemStack::empty(),
            // Fire is snuffed out by punching it, never harvested.
            FIRE => ItemStack::empty(),
            // Breaking a planted sapling returns the sapling item.
            id if is_sapling_block(id) => ItemStack::new_material(
                sapling_material_for(species_for_sapling(id).unwrap()),
                1,
            ),
            // HP-3 — banner is purely decorative. Mining it yields
            // nothing; the hideout's identity lives in the
            // `World::brigand_hideouts` side-table, not in this block.
            BRIGAND_HIDEOUT_BANNER => ItemStack::empty(),
            // Log Seasoning (Wave 29). Mining any oak-log block (tree
            // trunk OR player-placed) drops 1 GreenLog material. The
            // OAK_LOG block id stays valid as a placement target — only
            // the mine drop changed. Old saves' OAK_LOG block-items in
            // inventory are still placeable and still burn as 60 s fuel
            // (campfire fuel-value back-compat arm).
            //
            // Spec 28b — every species' log mines to the same GreenLog.
            // Species-neutral GreenLog keeps the Wave 29 drying-rack
            // pipeline simple (one fuel ladder, one workstation recipe);
            // visual species identity lives in the placed block, not the
            // log material. Per-species sawmill output is deferred to a
            // post-alpha "wood-grade economy" wave.
            OAK_LOG | BIRCH_LOG | SPRUCE_LOG | JUNGLE_LOG | ACACIA_LOG | DARK_OAK_LOG | RUBBER_LOG => {
                ItemStack::new_material(MaterialId::GreenLog, 1)
            }
            // Fibre plants (Spec 36). Phase 2: the WILD plant yields SEEDS —
            // you bring them home to farm. The fibre itself (Cotton / Hemp
            // Fibre) comes from harvesting the grown crop (see growth.rs
            // crop_break). This is the seed bootstrap + avoids a 1×1 recipe
            // clash with Cotton → String.
            COTTON_PLANT => ItemStack::new_material(MaterialId::CottonSeeds, 1),
            HEMP_PLANT => ItemStack::new_material(MaterialId::HempSeeds, 1),
            // Spec 37 — Magnesium Ore drops the raw mineral.
            MAGNESIUM_ORE => ItemStack::new_material(MaterialId::Magnesium, 1),
            // Spec 49 — Brimstone drops Sulphur, Nitre drops Saltpetre (1 base;
            // mine_drop_with_seed bumps to 2 on ~half of strikes). The Composter,
            // Blasting Keg, and Plunger Detonator self-drop via the `other` arm.
            BRIMSTONE => ItemStack::new_material(MaterialId::Sulphur, 1),
            NITRE_ORE => ItemStack::new_material(MaterialId::Saltpetre, 1),
            // P11 — the piston ARM is transient (placed by the piston when it
            // extends, not a real placeable). Breaking it yields the piston body
            // back, matching Minecraft; the body self-drops via `other`.
            PISTON_HEAD => ItemStack::new_block(PISTON, 1),
            // Dye flowers (Spec 35) drop themselves (placeable) — handled
            // by the `other` self-drop default below.
            other => ItemStack::new_block(other, 1),
        }
    }

    /// What an item-stack is dropped when this block is mined, with a
    /// per-roll random seed. Used for chance-based drops: currently
    /// just gravel → flint at 15%. Most callers stick with
    /// [`Self::mine_drop`] which is deterministic; the strike-time
    /// code path uses this when the position + tick are available to
    /// hash into a seed (matching the crop-drop determinism pattern
    /// in growth.rs).
    pub fn mine_drop_with_seed(&self, id: BlockId, rng_seed: u64) -> crate::item::ItemStack {
        use crate::item::{ItemStack, MaterialId};
        // Gravel → Flint at 15% (Wave 27 — campfire ignition material).
        // Falls through to mine_drop otherwise.
        if id == GRAVEL {
            // LCG-style mix matching crop_break's pattern.
            let r = rng_seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            if (r % 100) < 15 {
                return ItemStack::new_material(MaterialId::Flint, 1);
            }
        }
        // Salt — ROCK_SALT drops 2 Salt on the seeded hit (50 %), 1 Salt
        // base otherwise. Modelling "1-2 per strike" via the same
        // gravel→flint precedent.
        if id == ROCK_SALT {
            let r = rng_seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            if r.is_multiple_of(2) {
                return ItemStack::new_material(MaterialId::Salt, 2);
            }
        }
        // Spec 49 — Brimstone/Nitre drop 2 on the seeded hit (50%), 1 base
        // (same "1-2 per strike" precedent as Rock Salt).
        if id == BRIMSTONE {
            let r = rng_seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            if r.is_multiple_of(2) {
                return ItemStack::new_material(MaterialId::Sulphur, 2);
            }
        }
        if id == NITRE_ORE {
            let r = rng_seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            if r.is_multiple_of(2) {
                return ItemStack::new_material(MaterialId::Saltpetre, 2);
            }
        }
        self.mine_drop(id)
    }

    /// Optional bonus drop alongside the primary `mine_drop_with_seed`
    /// stack. Used when a block's spec says "always drop X + chance of
    /// Y" (e.g. SALT_PATH: 1 Dirt always + 0-1 Salt at 30 %). Callers
    /// add both stacks to the player's inventory; an empty `None` is
    /// the common case.
    pub fn bonus_mine_drop(&self, id: BlockId, rng_seed: u64) -> Option<crate::item::ItemStack> {
        if id == SALT_PATH {
            // Different multiplier than the primary mixer so the bonus
            // roll isn't perfectly correlated with the primary path's
            // roll. 30 % recovery.
            let r = rng_seed
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .wrapping_add(0xBF58_476D_1CE4_E5B9);
            if (r % 100) < 30 {
                return Some(crate::item::ItemStack::new_material(crate::item::MaterialId::Salt, 1));
            }
        }
        None
    }

    pub fn get(&self, id: BlockId) -> &BlockDef {
        self.blocks.get(id as usize).unwrap_or(&self.blocks[0])
    }

    /// Whether a block id is registered. `get()` silently falls back to AIR for
    /// unknown ids; callers that accept block ids from the network must reject
    /// unknown ids up-front rather than let the silent fallback corrupt state.
    pub fn is_known(&self, id: BlockId) -> bool {
        (id as usize) < self.blocks.len()
    }

    /// Number of registered block ids (including AIR at index 0). Used by
    /// the inventory explorer to enumerate all blocks.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_solid(&self, id: BlockId) -> bool {
        self.get(id).solid
    }

    pub fn is_transparent(&self, id: BlockId) -> bool {
        self.get(id).transparent
    }

    /// Phase 4 — the third-person camera-occlusion class for a block, derived
    /// from its `solid`+`transparent` flags (no per-block authoring). This is the
    /// seam for authored exceptions: a block needing non-default behaviour (an
    /// invisible barrier that still blocks the camera, fences that rotate-around)
    /// branches on `id` here before the derived fallthrough. Render-only.
    pub fn camera_occlusion(&self, id: BlockId) -> CameraOcclusion {
        CameraOcclusion::default_for(self.is_solid(id), self.is_transparent(id))
    }

    /// Phase 4 — whether the third-person camera-collision ray stops at this
    /// block. See-through blocks (glass, leaves) and non-solids let the camera
    /// pass so it never clips on something you can see past.
    pub fn camera_occludes(&self, id: BlockId) -> bool {
        self.camera_occlusion(id).collides()
    }

    pub fn color(&self, id: BlockId) -> [f32; 3] {
        self.get(id).color
    }

    /// Get texture layer for the top face of a block.
    pub fn tex_top(&self, id: BlockId) -> u32 {
        self.get(id).tex_top
    }

    /// Get texture layer for the bottom face of a block.
    pub fn tex_bottom(&self, id: BlockId) -> u32 {
        self.get(id).tex_bottom
    }

    /// Get texture layer for side faces of a block.
    pub fn tex_side(&self, id: BlockId) -> u32 {
        self.get(id).tex_side
    }

    /// Returns true if this block falls when unsupported (e.g. sand, gravel).
    pub fn has_gravity(&self, id: BlockId) -> bool {
        self.get(id).gravity
    }

    /// Spec 30 — light emission level for a block id (0..=15).
    /// 0 means no emission. Encoded as a match rather than a BlockDef
    /// field so adding a new emitter is a one-line addition without
    /// touching every existing block constructor.
    pub fn light_emission(&self, id: BlockId) -> u8 {
        match id {
            // P10 — Lava is the brightest natural light source (Minecraft 15).
            LAVA => 15,
            // Fire burns as bright as lava (Minecraft 15).
            FIRE => 15,
            // Torches — primary portable light source.
            TORCH => 14,
            // Salt feature — Salt Lamp matches torch.
            SALT_LAMP => 14,
            // Lit campfire — same level as a torch.
            CAMPFIRE => 14,
            // Lit furnace — slightly dimmer (Minecraft uses 13).
            FURNACE_LIT => 13,
            // Spec 35 dyed-décor Paper Lanterns (2026-05-28). Soft
            // diffuse glow — slightly dimmer than the bright fire-based
            // emitters so a lantern reads as accent-light, not main-
            // light, in a built space. Every colour variant emits at
            // the same level — the tint comes from the texture.
            PAPER_LANTERN_WHITE
            | PAPER_LANTERN_BLACK
            | PAPER_LANTERN_RED
            | PAPER_LANTERN_BLUE
            | PAPER_LANTERN_YELLOW
            | PAPER_LANTERN_ORANGE
            | PAPER_LANTERN_GREEN
            | PAPER_LANTERN_PURPLE
            | PAPER_LANTERN_PINK
            | PAPER_LANTERN_LIME
            | PAPER_LANTERN_LIGHT_BLUE
            | PAPER_LANTERN_GREY
            | PAPER_LANTERN_LIGHT_GREY
            | PAPER_LANTERN_BROWN
            | PAPER_LANTERN_CYAN
            | PAPER_LANTERN_MAGENTA => 12,
            // Spec 48 (Electricity) — a lit Electric Lamp is a primary,
            // switchable light source (torch-bright). A running Steam Generator
            // gives a warmer, dimmer combustion glow. The unlit ids emit nothing,
            // so the Lamp↔Lamp-lit / Gen↔Gen-lit block-id flip the power tick
            // broadcasts also drives the lighting BFS on / off.
            ELECTRIC_LAMP_LIT => 14,
            STEAM_GENERATOR_LIT => 11,
            // Lava (future) — would emit 15. Glowstone-equivalent
            // would emit 15. No emitters today besides the above.
            _ => 0,
        }
    }

    /// Spec 30 — light absorption (per-step decay) for a block id.
    /// 0 = transparent for light propagation; 15 = full opaque.
    /// Maps directly off the `transparent` flag with one carve-out
    /// for the water variant (transparent visually but absorbs
    /// block-light slightly when propagating through it).
    pub fn light_absorption(&self, id: BlockId) -> u8 {
        if id == AIR { return 0; }
        // Water absorbs a little (Minecraft uses 2/3 attenuation).
        if id == WATER { return 1; }
        if self.is_transparent(id) { return 0; }
        15
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_known_accepts_registered_ids_only() {
        let reg = BlockRegistry::new();
        assert!(reg.is_known(AIR));
        assert!(reg.is_known(CRAFTING_TABLE));
        assert!(reg.is_known(BED));
        assert!(reg.is_known(COAL_ORE));
        assert!(reg.is_known(IRON_ORE));
        assert!(reg.is_known(DIAMOND_ORE));
        assert!(reg.is_known(GLASS));
        assert!(reg.is_known(COAL_BLOCK));
        assert!(reg.is_known(IRON_BLOCK));
        assert!(reg.is_known(DIAMOND_BLOCK));
        assert!(reg.is_known(TORCH));
        assert!(reg.is_known(TALL_GRASS));
        assert!(reg.is_known(PURE_DEEPSLATE));
        assert!(reg.is_known(DEEPSLATE_COAL_ORE));
        assert!(reg.is_known(DEEPSLATE_IRON_ORE));
        assert!(reg.is_known(DEEPSLATE_DIAMOND_ORE));
        assert!(reg.is_known(SATORI_BLOCK));
        assert!(reg.is_known(TILLED_SOIL));
        // All 12 crop-stage blocks registered.
        for id in WHEAT_STAGE_0..=POTATO_STAGE_3 {
            assert!(reg.is_known(id), "crop stage id {id} should be registered");
        }
        // Campfire blocks (Wave 27).
        assert!(reg.is_known(CAMPFIRE));
        assert!(reg.is_known(CAMPFIRE_UNLIT));
        // Wave 28 — smoke pillar + four corn stages.
        assert!(reg.is_known(CAMPFIRE_SMOKE));
        for id in CORN_STAGE_0..=CORN_STAGE_3 {
            assert!(reg.is_known(id), "corn stage id {id} should be registered");
        }
        // Spec 19 — Village Bell.
        assert!(reg.is_known(VILLAGE_BELL), "village bell should be registered");
        // Wave 29 — Drying Rack workstation (id 51, after VILLAGE_BELL = 50).
        assert!(reg.is_known(DRYING_RACK));
        // Spec 23 — Papyrus stages 52-55.
        for id in PAPYRUS_STAGE_0..=PAPYRUS_STAGE_3 {
            assert!(reg.is_known(id), "papyrus stage id {id} should be registered");
        }
        // Spec 24 — Build Schematics Core blocks 56-58.
        assert!(reg.is_known(BLUEPRINT_PAPER));
        assert!(reg.is_known(CONSTRUCTION_ANCHOR));
        assert!(reg.is_known(ARCHITECT_PLAQUE));
        // Spec 20 — Furnace + Furnace lit blocks 59-60.
        assert!(reg.is_known(FURNACE));
        assert!(reg.is_known(FURNACE_LIT));
        // Spec 16 Phase 3b — pure-deepslate visual variants 61-63.
        assert!(reg.is_known(PURE_DEEPSLATE_THIN));
        assert!(reg.is_known(PURE_DEEPSLATE_HEALTHY));
        assert!(reg.is_known(PURE_DEEPSLATE_FAT));
        // Spec 21 Vendor Block — id 64.
        assert!(reg.is_known(VENDOR_BLOCK));
        // Spec 26 Drafting Table — id 65.
        assert!(reg.is_known(DRAFTING_TABLE));
        // Spec 28c Materials Expansion — ids 66-73.
        assert!(reg.is_known(LIMESTONE));
        assert!(reg.is_known(MARBLE));
        assert!(reg.is_known(GRANITE));
        assert!(reg.is_known(SLATE));
        assert!(reg.is_known(COPPER_ORE));
        assert!(reg.is_known(BONE_BLOCK));
        assert!(reg.is_known(HAY_BALE));
        assert!(reg.is_known(AMETHYST_BLOCK));
        // Spec 28b Wood Species — ids 74-88.
        for s in ALL_WOOD_SPECIES {
            assert!(reg.is_known(log_block_for(*s)));
            assert!(reg.is_known(leaves_block_for(*s)));
            assert!(reg.is_known(planks_block_for(*s)));
        }
        // T1.5 crops + workstations — ids 89-110.
        for id in 89u16..=110 {
            assert!(reg.is_known(id), "T1.5 block id {id} should be registered");
        }
        // Spec 28d chunk 8 — Bee Hive at id 111.
        assert!(reg.is_known(BEE_HIVE));
        // HP-2 — Chest at id 112.
        assert!(reg.is_known(CHEST));
        // HP-3 — banner id 113.
        assert!(reg.is_known(BRIGAND_HIDEOUT_BANNER));
        // HP-3 v2 — Trophy Wall id 114.
        assert!(reg.is_known(TROPHY_WALL));
        // Salt — 5 blocks at 115-119.
        assert!(reg.is_known(ROCK_SALT));
        assert!(reg.is_known(SALT_LICK));
        assert!(reg.is_known(SALT_LAMP));
        assert!(reg.is_known(SALT_BLOCK));
        assert!(reg.is_known(SALT_PATH));
        // Rubber — 4 blocks at 120-123.
        assert!(reg.is_known(RUBBER_LOG));
        assert!(reg.is_known(RUBBER_PLANKS));
        assert!(reg.is_known(RUBBER_LEAVES));
        assert!(reg.is_known(RUBBER_LOG_TAPPED));
        // Mob Bounty Board — id 124.
        assert!(reg.is_known(BOUNTY_BOARD));
        // Tip Jar — id 125.
        assert!(reg.is_known(TIP_JAR));
        // Repair Bench — id 126.
        assert!(reg.is_known(REPAIR_BENCH));
        // Plot Marker — id 127.
        assert!(reg.is_known(PLOT_MARKER));
        // Market Bell — id 128.
        assert!(reg.is_known(MARKET_BELL));
        // Auction Block — id 129.
        assert!(reg.is_known(AUCTION_BLOCK));
        // Bazaar Block — id 130.
        assert!(reg.is_known(BAZAAR_BLOCK));
        assert!(reg.is_known(MAGNESIUM_ORE));
        assert!(reg.is_known(HEMP_STAGE_3));
        assert!(reg.is_known(WALLPAPER_LIGHT_GREY));
        assert!(reg.is_known(LATENT_PRINT));
        assert!(reg.is_known(WALLPAPER_BROWN));
        assert!(reg.is_known(WALLPAPER_CYAN));
        assert!(reg.is_known(WALLPAPER_MAGENTA));
        assert!(reg.is_known(CORNFLOWER_STAGE_0));
        assert!(reg.is_known(BUTTERCUP_STAGE_2));
        assert!(reg.is_known(FENCE_POST));
        assert!(reg.is_known(CYANOTYPE_PRINT));
        assert!(reg.is_known(BUNTING_WHITE));
        assert!(reg.is_known(BUNTING_MAGENTA));
        assert!(reg.is_known(PAPER_LANTERN_WHITE));
        assert!(reg.is_known(PAPER_LANTERN_MAGENTA));
        assert!(reg.is_known(KITE_WHITE));
        assert!(reg.is_known(KITE_MAGENTA));
        assert!(reg.is_known(BANNER_WHITE));
        assert!(reg.is_known(BANNER_MAGENTA));
        assert!(reg.is_known(SAIL_WHITE));
        assert!(reg.is_known(SAIL_MAGENTA));
        assert!(reg.is_known(BIRCH_FENCE_POST));
        assert!(reg.is_known(RUBBER_FENCE_POST));
        assert!(reg.is_known(TENT));
        assert!(reg.is_known(crate::rail::TRACK));
        assert!(reg.is_known(LADDER));
        assert!(reg.is_known(CARPET));
        assert!(reg.is_known(GRAVE));
        // #15 — tiered-storage chests.
        assert!(reg.is_known(COPPER_CHEST));
        assert!(reg.is_known(IRON_CHEST));
        assert!(reg.is_known(DIAMOND_CHEST));
        assert!(reg.is_known(SATORI_CHEST));
        // Spec 48 (Electricity) appended ids 268..=279 (Phase 1) + 280..=282
        // (Phase 2 sensors).
        assert!(reg.is_known(CABLE));
        assert!(reg.is_known(BATTERY));
        assert!(reg.is_known(BEAM_SENSOR));
        assert!(reg.is_known(MIRROR));
        assert!(reg.is_known(MOTION_SENSOR));
        // F1 shaped blocks (283..=284) + Wave-2 fence gate (285).
        assert!(reg.is_known(STONE_SLAB));
        assert!(reg.is_known(STONE_STAIRS));
        assert!(reg.is_known(OAK_FENCE_GATE));
        assert!(reg.is_known(OAK_TRAPDOOR));
        assert!(reg.is_known(GLASS_PANE));
        assert!(reg.is_known(IRON_BARS));
        assert!(reg.is_known(OAK_DOOR));
        assert!(reg.is_known(COBBLESTONE_WALL));
        assert!(reg.is_known(OAK_SIGN));
        assert!(reg.is_known(ITEM_FRAME));
        assert!(reg.is_known(BRIMSTONE));
        assert!(reg.is_known(NITRE_ORE));
        assert!(reg.is_known(COMPOSTER));
        assert!(reg.is_known(BLASTING_KEG));
        assert!(reg.is_known(PLUNGER_DETONATOR));
        assert!(reg.is_known(HOPPER));
        assert!(reg.is_known(LAVA));
        assert!(reg.is_known(PISTON));
        assert!(reg.is_known(PISTON_HEAD));
        assert!(reg.is_known(OBSIDIAN));
        assert!(reg.is_known(STICKY_PISTON));
        assert!(reg.is_known(FIRE));
        assert!(reg.is_known(DISPENSER));
        assert!(reg.is_known(DROPPER));
        assert!(reg.is_known(SAPLING_OAK));
        assert!(reg.is_known(SAPLING_RUBBER));
        assert!(reg.is_known(SNOW_LAYER));
        assert!(reg.is_known(PET_BED));
        assert!(reg.is_known(WATER_WHEEL));
        assert!(reg.is_known(WATER_WHEEL_TURNING));
        // WINDMILL_TURNING is now the last registered block id. Bump this
        // when a new block is appended to the registry.
        assert!(reg.is_known(WINDMILL));
        assert!(reg.is_known(WINDMILL_TURNING));
        assert!(!reg.is_known(WINDMILL_TURNING + 1));
        assert!(!reg.is_known(u16::MAX));
    }

    // ── T1.5 crop + workstation drops ─────────────────────────────

    #[test]
    fn t1_5_mature_crops_drop_their_food() {
        use crate::item::{Item, MaterialId};
        let reg = BlockRegistry::new();
        assert!(matches!(reg.mine_drop(SUGAR_BEET_STAGE_3).item,
            Item::Material(MaterialId::SugarBeet)));
        assert!(matches!(reg.mine_drop(BEETROOT_STAGE_3).item,
            Item::Material(MaterialId::Beetroot)));
        assert!(matches!(reg.mine_drop(PUMPKIN).item,
            Item::Material(MaterialId::PumpkinFood)));
        assert!(matches!(reg.mine_drop(BERRY_BUSH_3).item,
            Item::Material(MaterialId::Berries)));
    }

    #[test]
    fn t1_5_immature_crops_drop_seeds_or_nothing() {
        use crate::item::{Item, MaterialId};
        let reg = BlockRegistry::new();
        // Sugar beet immature stages drop SugarBeetSeeds.
        for stage in [SUGAR_BEET_STAGE_0, SUGAR_BEET_STAGE_1, SUGAR_BEET_STAGE_2] {
            assert!(matches!(reg.mine_drop(stage).item,
                Item::Material(MaterialId::SugarBeetSeeds)));
        }
        // Beetroot immature stages drop BeetrootSeeds.
        for stage in [BEETROOT_STAGE_0, BEETROOT_STAGE_1, BEETROOT_STAGE_2] {
            assert!(matches!(reg.mine_drop(stage).item,
                Item::Material(MaterialId::BeetrootSeeds)));
        }
        // Pumpkin stem stages 0-3 drop nothing (immature stem is fragile).
        for stem in [PUMPKIN_STEM_0, PUMPKIN_STEM_1, PUMPKIN_STEM_2, PUMPKIN_STEM_3] {
            assert_eq!(reg.mine_drop(stem).count, 0,
                "immature pumpkin stem must drop nothing");
        }
        // Berry bush immature stages drop nothing (it regrows on its own).
        for stage in [BERRY_BUSH_0, BERRY_BUSH_1, BERRY_BUSH_2] {
            assert_eq!(reg.mine_drop(stage).count, 0);
        }
    }

    #[test]
    fn t1_5_workstation_blocks_drop_themselves() {
        use crate::item::Item;
        let reg = BlockRegistry::new();
        for block in [MILL, OVEN, AGING_RACK, BEE_HIVE] {
            let drop = reg.mine_drop(block);
            match drop.item {
                Item::Block(b) if b == block => {}
                _ => panic!("workstation block {block} should drop itself"),
            }
        }
    }

    // ── Spec 28b Wood Species ──────────────────────────────────────

    #[test]
    fn all_wood_species_listed() {
        // Spec 28b: 6 species; Rubber added 2026-05-23 brings it to 7.
        assert_eq!(ALL_WOOD_SPECIES.len(), 7);
    }

    #[test]
    fn species_helpers_round_trip_distinct_ids() {
        use std::collections::HashSet;
        let mut logs = HashSet::new();
        let mut leaves = HashSet::new();
        let mut planks = HashSet::new();
        for s in ALL_WOOD_SPECIES {
            logs.insert(log_block_for(*s));
            leaves.insert(leaves_block_for(*s));
            planks.insert(planks_block_for(*s));
        }
        assert_eq!(logs.len(), 7, "each species has a distinct log id");
        assert_eq!(leaves.len(), 7);
        assert_eq!(planks.len(), 7);
    }

    #[test]
    fn every_species_log_mines_to_green_log() {
        use crate::item::{Item, MaterialId};
        let reg = BlockRegistry::new();
        for s in ALL_WOOD_SPECIES {
            let drop = reg.mine_drop(log_block_for(*s));
            match drop.item {
                Item::Material(MaterialId::GreenLog) => {}
                _ => panic!("{:?} log should mine to GreenLog", s),
            }
        }
    }

    #[test]
    fn every_species_leaves_drop_themselves() {
        use crate::item::Item;
        let reg = BlockRegistry::new();
        for s in ALL_WOOD_SPECIES {
            let drop = reg.mine_drop(leaves_block_for(*s));
            match drop.item {
                Item::Block(b) if b == leaves_block_for(*s) => {}
                _ => panic!("{:?} leaves should drop themselves", s),
            }
        }
    }

    #[test]
    fn is_any_planks_recognises_every_species() {
        for s in ALL_WOOD_SPECIES {
            assert!(is_any_planks(planks_block_for(*s)),
                "is_any_planks must recognise {:?} planks", s);
        }
        assert!(!is_any_planks(STONE));
        assert!(!is_any_planks(OAK_LOG));
    }

    #[test]
    fn is_any_log_block_recognises_every_species() {
        for s in ALL_WOOD_SPECIES {
            assert!(is_any_log_block(log_block_for(*s)));
        }
        assert!(!is_any_log_block(OAK_PLANKS));
    }

    #[test]
    fn is_any_leaves_recognises_every_species() {
        for s in ALL_WOOD_SPECIES {
            assert!(is_any_leaves(leaves_block_for(*s)));
        }
        assert!(!is_any_leaves(OAK_PLANKS));
    }

    #[test]
    fn sapling_material_maps_per_species() {
        use crate::item::MaterialId;
        // Each species maps to a distinct sapling material.
        assert_eq!(sapling_material_for(WoodSpecies::Oak), MaterialId::OakSapling);
        assert_eq!(sapling_material_for(WoodSpecies::Birch), MaterialId::BirchSapling);
        assert_eq!(sapling_material_for(WoodSpecies::Spruce), MaterialId::SpruceSapling);
        assert_eq!(sapling_material_for(WoodSpecies::Jungle), MaterialId::JungleSapling);
        assert_eq!(sapling_material_for(WoodSpecies::Acacia), MaterialId::AcaciaSapling);
        assert_eq!(sapling_material_for(WoodSpecies::DarkOak), MaterialId::DarkOakSapling);
    }

    // ── Spec 16 Phase 3b — pure-deepslate family ─────────────────────

    #[test]
    fn is_wallpaper_covers_sixteen_ids_and_skips_latent_print() {
        // Owner-inbox #1/2/3 — 145..=157 and 159..=161 are wallpaper; 158 is
        // LATENT_PRINT and must NOT be treated as wallpaper.
        for id in 145u16..=157 {
            assert!(is_wallpaper(id), "id {id} should be wallpaper");
        }
        for id in 159u16..=161 {
            assert!(is_wallpaper(id), "id {id} should be wallpaper");
        }
        assert!(!is_wallpaper(LATENT_PRINT), "158 LATENT_PRINT is not wallpaper");
        assert!(!is_wallpaper(144));
        assert!(!is_wallpaper(162));
        assert!(!is_wallpaper(STONE));
        // The named boundary constants line up with the numeric range.
        assert!(is_wallpaper(WALLPAPER_WHITE));
        assert!(is_wallpaper(WALLPAPER_MAGENTA));
    }

    #[test]
    fn is_pure_deepslate_family_matches_all_four_variants() {
        // Phase 3b: helper now matches the canonical PURE_DEEPSLATE
        // plus the three visual-variant tiers. Mining-mechanic parity
        // is locked by `proof_hash_independent_of_deepslate_variant`
        // in `proof_of_play.rs`.
        assert!(is_pure_deepslate_family(PURE_DEEPSLATE));
        assert!(is_pure_deepslate_family(PURE_DEEPSLATE_THIN));
        assert!(is_pure_deepslate_family(PURE_DEEPSLATE_HEALTHY));
        assert!(is_pure_deepslate_family(PURE_DEEPSLATE_FAT));
        assert!(!is_pure_deepslate_family(STONE));
        assert!(!is_pure_deepslate_family(DEEPSLATE_COAL_ORE));
        assert!(!is_pure_deepslate_family(SATORI_BLOCK));
    }

    #[test]
    fn mine_drop_normalises_variants_to_canonical_pure_deepslate() {
        let reg = BlockRegistry::new();
        for variant in [PURE_DEEPSLATE_THIN, PURE_DEEPSLATE_HEALTHY, PURE_DEEPSLATE_FAT] {
            let drop = reg.mine_drop(variant);
            match drop.item {
                crate::item::Item::Block(b) => assert_eq!(
                    b, PURE_DEEPSLATE,
                    "mining a variant must drop the canonical PURE_DEEPSLATE",
                ),
                _ => panic!("expected Block(PURE_DEEPSLATE), got {:?}", drop.item),
            }
        }
    }

    #[test]
    fn pure_deepslate_variant_ids_are_reserved_for_phase_3b() {
        // Spec 16 Phase 3b will register BlockDefs at these positions —
        // the constants declared here pin the ids so 3b doesn't drift.
        assert_eq!(PURE_DEEPSLATE_THIN, 61);
        assert_eq!(PURE_DEEPSLATE_HEALTHY, 62);
        assert_eq!(PURE_DEEPSLATE_FAT, 63);
    }

    #[test]
    fn campfire_uses_lit_textures_when_lit() {
        // The block registry doesn't model the lit-emitting property
        // yet; that's a future renderer addition. For now: just verify
        // the lit campfire uses the lit texture and the unlit uses the
        // unlit one — so when the renderer learns about light emission
        // it can read the right tex constants.
        let reg = BlockRegistry::new();
        let lit = reg.get(CAMPFIRE);
        assert_eq!(lit.tex_top, TEX_CAMPFIRE_LIT_TOP);
        assert_eq!(lit.tex_side, TEX_CAMPFIRE_LIT_SIDE);
        let unlit = reg.get(CAMPFIRE_UNLIT);
        assert_eq!(unlit.tex_top, TEX_CAMPFIRE_UNLIT_TOP);
        assert_eq!(unlit.tex_side, TEX_CAMPFIRE_UNLIT_SIDE);
    }

    #[test]
    fn gravel_sometimes_drops_flint() {
        // Spec 17 Phase 6 — gravel drops flint at 15%. Test averages
        // across many breaks; expect 10-22% to allow for variance.
        let reg = BlockRegistry::new();
        let mut flint_drops = 0;
        let trials = 10_000;
        for seed in 0..trials as u64 {
            let drop = reg.mine_drop_with_seed(GRAVEL, seed);
            match drop.item {
                crate::item::Item::Material(crate::item::MaterialId::Flint) => {
                    flint_drops += 1;
                }
                crate::item::Item::Block(GRAVEL) => {}
                _ => panic!("unexpected gravel drop: {:?}", drop.item),
            }
        }
        let pct = (flint_drops * 100) / trials;
        assert!((10..=22).contains(&pct), "flint drop rate {pct}% out of [10, 22]");
    }

    #[test]
    fn gravel_drop_is_deterministic_per_seed() {
        // Same seed → same outcome (replay-safe).
        let reg = BlockRegistry::new();
        let a = reg.mine_drop_with_seed(GRAVEL, 42);
        let b = reg.mine_drop_with_seed(GRAVEL, 42);
        // Item is non-Eq but matches! lets us check structurally.
        let same = matches!(
            (&a.item, &b.item),
            (crate::item::Item::Material(crate::item::MaterialId::Flint),
             crate::item::Item::Material(crate::item::MaterialId::Flint))
        ) || matches!(
            (&a.item, &b.item),
            (crate::item::Item::Block(_), crate::item::Item::Block(_))
        );
        assert!(same, "deterministic gravel drop expected");
    }

    #[test]
    fn non_gravel_blocks_ignore_seed() {
        // Stone always drops cobblestone regardless of seed
        // (deterministic — the chance-drop seed only affects blocks
        // like gravel→flint).
        let reg = BlockRegistry::new();
        for seed in 0..100 {
            let drop = reg.mine_drop_with_seed(STONE, seed);
            match drop.item {
                crate::item::Item::Block(id) => assert_eq!(id, COBBLESTONE),
                _ => panic!("stone should always drop cobblestone"),
            }
        }
    }

    #[test]
    fn oak_leaves_drop_themselves_on_break() {
        // Pre-Wave-27 the default mine_drop already returned the block
        // itself for leaves — confirming the contract so a future
        // refactor doesn't accidentally regress it.
        let reg = BlockRegistry::new();
        let drop = reg.mine_drop(OAK_LEAVES);
        match drop.item {
            crate::item::Item::Block(id) => assert_eq!(id, OAK_LEAVES),
            _ => panic!("expected leaves block to drop itself"),
        }
        assert_eq!(drop.count, 1);
    }

    #[test]
    fn oak_log_mine_drops_green_log_material() {
        // Wave 29 — mining an OAK_LOG (tree trunk or placed block) now
        // drops a GreenLog material instead of the block itself. The
        // material can be re-placed as a block via the right-click
        // handler's `material_as_placeable_block` lookup.
        use crate::item::{Item, MaterialId};
        let reg = BlockRegistry::new();
        let drop = reg.mine_drop(OAK_LOG);
        match drop.item {
            Item::Material(MaterialId::GreenLog) => {}
            other => panic!("expected GreenLog material, got {:?}", other),
        }
        assert_eq!(drop.count, 1);
    }

    #[test]
    fn oak_log_block_is_still_registered_for_placement() {
        // Back-compat: OAK_LOG block id stays valid even though mining
        // returns a material instead of the block. Old saves with
        // ItemStack::new_block(OAK_LOG, ..) in inventory must still
        // place a real oak-log block.
        let reg = BlockRegistry::new();
        let def = reg.get(OAK_LOG);
        assert_eq!(def.name, "genesis:oak_log");
        assert!(def.solid);
        assert!(!def.transparent);
    }

    #[test]
    fn crop_blocks_are_transparent_and_walkable() {
        // Walk-through, see-through, no gravity. Invariant across all
        // 12 stage blocks so a player can run rows without colliding
        // with the crops.
        let reg = BlockRegistry::new();
        for id in WHEAT_STAGE_0..=POTATO_STAGE_3 {
            let def = reg.get(id);
            assert!(!def.solid, "{id} should not be solid");
            assert!(def.transparent, "{id} should be transparent");
            assert!(!def.gravity, "{id} should not have gravity");
        }
    }

    #[test]
    fn tilled_soil_renders_dirt_on_sides() {
        // The top face renders the hoe-furrow texture; sides + bottom
        // render as plain dirt so a partly-tilled row blends cleanly
        // into the surrounding terrain.
        let reg = BlockRegistry::new();
        let def = reg.get(TILLED_SOIL);
        assert_eq!(def.tex_top, TEX_TILLED_SOIL);
        assert_eq!(def.tex_bottom, TEX_DIRT);
        assert_eq!(def.tex_side, TEX_DIRT);
        assert!(def.solid);
        assert!(!def.transparent);
        assert!(!def.gravity);
    }

    #[test]
    fn tall_grass_is_walkable_and_transparent() {
        let reg = BlockRegistry::new();
        let g = reg.get(TALL_GRASS);
        assert!(!g.solid);
        assert!(g.transparent);
    }

    #[test]
    fn torch_is_walkable_and_transparent() {
        let reg = BlockRegistry::new();
        let t = reg.get(TORCH);
        assert!(!t.solid, "torch should be walkable (non-solid)");
        assert!(t.transparent, "torch should be transparent");
    }

    #[test]
    fn glass_block_is_transparent_and_solid() {
        let reg = BlockRegistry::new();
        let g = reg.get(GLASS);
        assert!(g.solid, "glass should be solid (stand-able)");
        assert!(g.transparent, "glass should be transparent");
    }

    #[test]
    fn each_species_leaf_uses_its_own_texture() {
        // #130 request — birch / spruce / jungle / acacia / dark-oak each get a
        // distinct leaf texture so foliage reads by species, instead of every
        // tree aliasing the one oak green.
        let reg = BlockRegistry::new();
        let pairs = [
            (BIRCH_LEAVES, TEX_BIRCH_LEAVES),
            (SPRUCE_LEAVES, TEX_SPRUCE_LEAVES),
            (JUNGLE_LEAVES, TEX_JUNGLE_LEAVES),
            (ACACIA_LEAVES, TEX_ACACIA_LEAVES),
            (DARK_OAK_LEAVES, TEX_DARK_OAK_LEAVES),
        ];
        for (block_id, tex) in pairs {
            let def = reg.get(block_id);
            assert_eq!(def.tex_top, tex, "block {block_id} top should use its own leaf texture");
            assert_eq!(def.tex_side, tex, "block {block_id} side should use its own leaf texture");
            assert_eq!(def.tex_bottom, tex, "block {block_id} bottom should use its own leaf texture");
            assert_ne!(def.tex_top, TEX_OAK_LEAVES, "block {block_id} must not alias the oak texture");
        }
    }

    #[test]
    fn each_species_log_and_plank_uses_its_own_texture() {
        // #132 — logs + planks no longer alias the oak texture (leaves: #130).
        // A birch tree should have a pale birch trunk, not an oak-grained one.
        let reg = BlockRegistry::new();
        let logs = [
            (BIRCH_LOG, TEX_BIRCH_LOG_SIDE, TEX_BIRCH_LOG_TOP),
            (SPRUCE_LOG, TEX_SPRUCE_LOG_SIDE, TEX_SPRUCE_LOG_TOP),
            (JUNGLE_LOG, TEX_JUNGLE_LOG_SIDE, TEX_JUNGLE_LOG_TOP),
            (ACACIA_LOG, TEX_ACACIA_LOG_SIDE, TEX_ACACIA_LOG_TOP),
            (DARK_OAK_LOG, TEX_DARK_OAK_LOG_SIDE, TEX_DARK_OAK_LOG_TOP),
        ];
        for (id, side, top) in logs {
            let def = reg.get(id);
            assert_eq!(def.tex_side, side, "log {id} side should use its own texture");
            assert_eq!(def.tex_top, top, "log {id} top should use its own texture");
            assert_ne!(def.tex_side, TEX_OAK_LOG_SIDE, "log {id} must not alias the oak log");
        }
        let planks = [
            (BIRCH_PLANKS, TEX_BIRCH_PLANKS),
            (SPRUCE_PLANKS, TEX_SPRUCE_PLANKS),
            (JUNGLE_PLANKS, TEX_JUNGLE_PLANKS),
            (ACACIA_PLANKS, TEX_ACACIA_PLANKS),
            (DARK_OAK_PLANKS, TEX_DARK_OAK_PLANKS),
        ];
        for (id, tex) in planks {
            let def = reg.get(id);
            assert_eq!(def.tex_top, tex, "planks {id} should use their own texture");
            assert_ne!(def.tex_top, TEX_OAK_PLANKS, "planks {id} must not alias the oak planks");
        }
    }

    #[test]
    fn all_leaves_are_transparent_solid_for_the_fancy_pass() {
        // #131 fancy-leaf flip — leaves are SOLID (walkable-on, MC-parity) +
        // TRANSPARENT (see-through), so they render via the transparent-solid
        // mesh pass (`greedy_transparent_face`) with cutout-alpha holes. They were
        // opaque under #130's stopgap only until that pass existed; #131 built it,
        // and the flip restores see-through canopies (camera passes through too,
        // since `camera_occlusion::default_for(solid, transparent)` → PassThrough).
        let reg = BlockRegistry::new();
        for id in [
            OAK_LEAVES,
            BIRCH_LEAVES,
            SPRUCE_LEAVES,
            JUNGLE_LEAVES,
            ACACIA_LEAVES,
            DARK_OAK_LEAVES,
        ] {
            let def = reg.get(id);
            assert!(def.solid, "block {id} (leaves) stays solid — MC-parity walkable-on");
            assert!(
                def.transparent,
                "block {id} (leaves) must be transparent to render see-through via #131"
            );
        }
    }

    #[test]
    fn ore_blocks_drop_raw_materials_not_themselves() {
        use crate::item::{Item, MaterialId};
        let reg = BlockRegistry::new();
        let coal = reg.mine_drop(COAL_ORE);
        match coal.item {
            Item::Material(MaterialId::Coal) => {}
            _ => panic!("coal_ore should drop coal material"),
        }
        let iron = reg.mine_drop(IRON_ORE);
        match iron.item {
            Item::Material(MaterialId::RawIron) => {}
            _ => panic!("iron_ore should drop raw_iron material"),
        }
        let diamond = reg.mine_drop(DIAMOND_ORE);
        match diamond.item {
            Item::Material(MaterialId::Diamond) => {}
            _ => panic!("diamond_ore should drop diamond material"),
        }
        // Spec 28c — Copper Ore drops raw Copper (smelts to CopperIngot).
        let copper = reg.mine_drop(COPPER_ORE);
        match copper.item {
            Item::Material(MaterialId::Copper) => {}
            _ => panic!("copper_ore should drop copper material"),
        }
    }

    #[test]
    fn spec_28c_stone_variants_drop_themselves() {
        use crate::item::Item;
        let reg = BlockRegistry::new();
        for block in [LIMESTONE, MARBLE, GRANITE, SLATE] {
            let drop = reg.mine_drop(block);
            match drop.item {
                Item::Block(b) if b == block => {}
                _ => panic!("block {block} should drop itself"),
            }
        }
    }

    #[test]
    fn spec_28c_decorative_blocks_drop_themselves() {
        use crate::item::Item;
        let reg = BlockRegistry::new();
        for block in [BONE_BLOCK, HAY_BALE, AMETHYST_BLOCK] {
            let drop = reg.mine_drop(block);
            match drop.item {
                Item::Block(b) if b == block => {}
                _ => panic!("decorative block {block} should drop itself"),
            }
        }
    }

    #[test]
    fn deepslate_ore_blocks_drop_same_materials_as_stone_tier() {
        use crate::item::{Item, MaterialId};
        let reg = BlockRegistry::new();
        let dcoal = reg.mine_drop(DEEPSLATE_COAL_ORE);
        match dcoal.item {
            Item::Material(MaterialId::Coal) => {}
            _ => panic!("deepslate_coal_ore should drop coal"),
        }
        let diron = reg.mine_drop(DEEPSLATE_IRON_ORE);
        match diron.item {
            Item::Material(MaterialId::RawIron) => {}
            _ => panic!("deepslate_iron_ore should drop raw iron"),
        }
        let ddiamond = reg.mine_drop(DEEPSLATE_DIAMOND_ORE);
        match ddiamond.item {
            Item::Material(MaterialId::Diamond) => {}
            _ => panic!("deepslate_diamond_ore should drop diamond"),
        }
    }

    #[test]
    fn pure_deepslate_drops_itself() {
        use crate::item::Item;
        let reg = BlockRegistry::new();
        let drop = reg.mine_drop(PURE_DEEPSLATE);
        match drop.item {
            Item::Block(id) => assert_eq!(id, PURE_DEEPSLATE),
            _ => panic!("pure deepslate should drop itself (cobbled deepslate is future polish)"),
        }
    }

    #[test]
    fn stone_mine_drops_cobblestone() {
        // Minecraft-baseline: mining stone yields cobblestone — the
        // unlocking item for Stone-tier tools + the Furnace recipe.
        // The STONE block id stays a valid placement target (creative
        // / future smelt-cobble-back-to-stone arm); only the natural
        // mining drop changed.
        use crate::item::Item;
        let reg = BlockRegistry::new();
        let drop = reg.mine_drop(STONE);
        match drop.item {
            Item::Block(id) => assert_eq!(id, COBBLESTONE),
            other => panic!("expected COBBLESTONE block, got {other:?}"),
        }
    }

    #[test]
    fn cobblestone_mine_drops_itself() {
        // Defence-in-depth: cobble doesn't recurse into itself or
        // into stone. Locks the contract so a future "double-cobble"
        // refactor accident is caught.
        use crate::item::Item;
        let reg = BlockRegistry::new();
        let drop = reg.mine_drop(COBBLESTONE);
        match drop.item {
            Item::Block(id) => assert_eq!(id, COBBLESTONE),
            other => panic!("expected COBBLESTONE block, got {other:?}"),
        }
    }

    #[test]
    fn bed_block_present_with_distinct_textures() {
        let reg = BlockRegistry::new();
        let bed = reg.get(BED);
        assert_eq!(bed.name, "genesis:bed");
        assert!(bed.solid);
        assert_ne!(bed.tex_top, bed.tex_side, "bed top + side must differ");
    }

    #[test]
    fn papyrus_stage_blocks_are_transparent_and_walkable() {
        // Spec 23: stalks let the player wade through them while
        // harvesting — non-solid + transparent + non-gravity invariants
        // across all four stages.
        let reg = BlockRegistry::new();
        for id in PAPYRUS_STAGE_0..=PAPYRUS_STAGE_3 {
            let def = reg.get(id);
            assert!(!def.solid, "papyrus stage {id} should not be solid");
            assert!(def.transparent, "papyrus stage {id} should be transparent");
            assert!(!def.gravity, "papyrus stage {id} should not have gravity");
        }
    }

    #[test]
    fn salt_path_primary_drop_is_always_dirt() {
        // Spec: "drops 1 Dirt + 0-1 Salt". The primary path is
        // always Dirt; Salt is a separate bonus_mine_drop roll.
        let reg = BlockRegistry::new();
        for seed in 0..200u64 {
            let drop = reg.mine_drop_with_seed(SALT_PATH, seed);
            assert_eq!(drop.count, 1, "seed {seed}: SALT_PATH drop count must be 1");
            match drop.item {
                crate::item::Item::Block(DIRT) => {},
                other => panic!("seed {seed}: primary SALT_PATH drop must be a Dirt block, got {:?}", other),
            }
        }
    }

    #[test]
    fn salt_path_bonus_drop_recovers_salt_roughly_30_percent() {
        // Spec: 30 % chance of bonus 1 Salt alongside the primary Dirt.
        // Sample a wide seed range; expect ≈ 30 % hit rate.
        let reg = BlockRegistry::new();
        let n = 2_000u64;
        let mut hits = 0u32;
        for seed in 0..n {
            if reg.bonus_mine_drop(SALT_PATH, seed).is_some() {
                hits += 1;
            }
        }
        let rate = hits as f32 / n as f32;
        // ±5 percentage points is a generous tolerance for the sample size.
        assert!(rate > 0.25 && rate < 0.35,
            "SALT_PATH bonus rate should be ~30 %, got {:.3}", rate);
    }

    #[test]
    fn bonus_drop_returns_none_for_non_salt_path_blocks() {
        let reg = BlockRegistry::new();
        for id in [STONE, DIRT, GRASS, ROCK_SALT, SALT_LICK, SALT_LAMP, SALT_BLOCK] {
            assert!(reg.bonus_mine_drop(id, 42).is_none(),
                "{:?} should not produce a bonus drop", id);
        }
    }

    #[test]
    fn papyrus_stage_textures_match_constants() {
        // Stage textures progress 160..=163 in the texture atlas; each
        // stage's BlockDef wires the same per-stage tex to top/side/bottom
        // so the reed is visible from every angle.
        let reg = BlockRegistry::new();
        let expected = [
            (PAPYRUS_STAGE_0, TEX_PAPYRUS_STAGE_0),
            (PAPYRUS_STAGE_1, TEX_PAPYRUS_STAGE_1),
            (PAPYRUS_STAGE_2, TEX_PAPYRUS_STAGE_2),
            (PAPYRUS_STAGE_3, TEX_PAPYRUS_STAGE_3),
        ];
        for (id, tex) in expected {
            let def = reg.get(id);
            assert_eq!(def.tex_top, tex, "stage {id} top tex");
            assert_eq!(def.tex_bottom, tex, "stage {id} bottom tex");
            assert_eq!(def.tex_side, tex, "stage {id} side tex");
        }
    }

    // ── Third-person camera — Phase 4 (per-block camera occlusion) ───────────
    // The camera treats each block by a registry-derived CameraOcclusion tag with
    // NO per-block authoring: opaque solids Squeeze the camera in; see-through
    // blocks (glass/leaves) and non-solids PassThrough so the camera never clips
    // on something you can see past. Keys off registry INTENT (solid+transparent),
    // not a visual box — the day-one fix for Minecraft's Glass-vs-Barrier
    // inconsistency (MC-189617/175927).

    #[test]
    fn camera_occlusion_default_for_opaque_solid_is_squeeze() {
        assert_eq!(CameraOcclusion::default_for(true, false), CameraOcclusion::Squeeze);
    }

    #[test]
    fn camera_occlusion_default_for_transparent_solid_passes_through() {
        // Glass/leaves: solid for movement but see-through → the camera passes.
        assert_eq!(CameraOcclusion::default_for(true, true), CameraOcclusion::PassThrough);
    }

    #[test]
    fn camera_occlusion_default_for_non_solid_passes_through() {
        assert_eq!(CameraOcclusion::default_for(false, true), CameraOcclusion::PassThrough);
        assert_eq!(CameraOcclusion::default_for(false, false), CameraOcclusion::PassThrough);
    }

    #[test]
    fn camera_occlusion_collides_only_for_squeeze_and_rotate() {
        // Squeeze (and RotateAround, until rotate is implemented) push the camera;
        // PassThrough and FadeOnly let it through (the avatar fades instead).
        assert!(CameraOcclusion::Squeeze.collides());
        assert!(CameraOcclusion::RotateAround.collides());
        assert!(!CameraOcclusion::PassThrough.collides());
        assert!(!CameraOcclusion::FadeOnly.collides());
    }

    #[test]
    fn registry_camera_occlusion_keys_off_intent_not_visual_box() {
        let reg = BlockRegistry::new();
        // Stone: opaque solid → Squeeze (camera collides).
        assert_eq!(reg.camera_occlusion(STONE), CameraOcclusion::Squeeze);
        assert!(reg.camera_occludes(STONE));
        // Glass: solid-but-transparent → PassThrough (see-through; the MC
        // Glass-vs-Barrier bug fixed by keying off registry intent).
        assert_eq!(reg.camera_occlusion(GLASS), CameraOcclusion::PassThrough);
        assert!(!reg.camera_occludes(GLASS));
        // Leaves: solid + transparent (#131 fancy-leaf flip) → the derived
        // default is PassThrough, so the third-person camera glides through a
        // canopy instead of squeezing in. No authored override needed — it falls
        // straight out of keying off registry intent (solid + transparent).
        assert_eq!(reg.camera_occlusion(OAK_LEAVES), CameraOcclusion::PassThrough);
        assert!(!reg.camera_occludes(OAK_LEAVES));
        // Air is non-solid → PassThrough.
        assert!(!reg.camera_occludes(AIR));
    }
}
