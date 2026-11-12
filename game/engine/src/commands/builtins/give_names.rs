//! The `/give` item vocabulary as data: one set of name tables that both the
//! resolver (`give::resolve_item`) and Tab completion (`item_names`) read, so
//! a name can never be accepted but not suggested, or suggested but not
//! accepted.
//!
//! Every table row is `(names, value)`. The FIRST name is the canonical one
//! (what completion offers); the rest are aliases that keep working when
//! typed. `resolve_item` tries blocks, then tools, then bow-type tools, then
//! materials, then the two plan keywords; within a table the first matching row
//! wins. The test module `give_golden_tests` pins every accepted name and what
//! it resolves to.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use crate::block::{self, BlockId};
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::{Item, MaterialId as M};

/// First row whose names include `key`.
fn lookup<T: Copy>(table: &[(&[&str], T)], key: &str) -> Option<T> {
    table.iter().find(|(names, _)| names.contains(&key)).map(|(_, v)| *v)
}

/// Every block `/give` names, in resolution order.
pub(super) static BLOCK_NAMES: &[(&[&str], BlockId)] = &[
    (&["stone"], block::STONE),
    (&["dirt"], block::DIRT),
    (&["grass"], block::GRASS),
    (&["bedrock"], block::BEDROCK),
    (&["sand"], block::SAND),
    (&["water"], block::WATER),
    // Lava as a placeable block — mirrors `water` (apply_arena_setup registers
    // it as a source). Enables `/give lava` for testing + the "Floor Is Lava"
    // arena. The survival way to move lava stays the lava bucket.
    (&["lava"], block::LAVA),
    (&["oak_log", "log"], block::OAK_LOG),
    (&["oak_leaves", "leaves"], block::OAK_LEAVES),
    (&["oak_planks", "planks"], block::OAK_PLANKS),
    (&["cobblestone", "cobble"], block::COBBLESTONE),
    (&["gravel"], block::GRAVEL),
    (&["sandstone"], block::SANDSTONE),
    (&["snow"], block::SNOW),
    (&["crafting_table", "workbench"], block::CRAFTING_TABLE),
    (&["bed"], block::BED),
    (&["coal_ore"], block::COAL_ORE),
    (&["iron_ore"], block::IRON_ORE),
    (&["diamond_ore"], block::DIAMOND_ORE),
    // Wind, Copper & Electricity wave (2026-09-07) — `copper`/`copper_ore`
    // resolve to the ore BLOCK (checked before material_by_name below, so
    // this wins); `raw_copper` resolves to the smeltable MATERIAL instead.
    (&["copper_ore", "copper"], block::COPPER_ORE),
    (&["glass"], block::GLASS),
    (&["coal_block", "coalblock"], block::COAL_BLOCK),
    (&["iron_block", "ironblock"], block::IRON_BLOCK),
    (&["diamond_block", "diamondblock"], block::DIAMOND_BLOCK),
    (&["torch"], block::TORCH),
    (&["chest"], block::CHEST),
    // Rail freight (Phase 1) — `/give track` lets a builder lay rail (no
    // survival recipe yet). Depots are just crafted chests (`/give chest`
    // above, or the 8-plank ring) placed beside a track terminus.
    (&["track", "rail"], crate::rail::TRACK),
    (&["tall_grass", "tallgrass"], block::TALL_GRASS),
    // Farming Tier 1 (Wave 26) — tilled soil + 12 crop-stage blocks.
    // Convenience aliases: `wheat`, `carrot`, `potato` (no stage
    // suffix) are MATERIAL names — handled by material_by_name —
    // but the crop *blocks* live here so `/give wheat_stage_3 1`
    // gives a placeable mature wheat for setup screenshots.
    (&["tilled_soil", "tilledsoil", "farmland"], block::TILLED_SOIL),
    (&["wheat_stage_0", "wheat_sprout"], block::WHEAT_STAGE_0),
    (&["wheat_stage_1"], block::WHEAT_STAGE_1),
    (&["wheat_stage_2"], block::WHEAT_STAGE_2),
    (&["wheat_stage_3", "wheat_mature"], block::WHEAT_STAGE_3),
    (&["carrot_stage_0", "carrot_sprout"], block::CARROT_STAGE_0),
    (&["carrot_stage_1"], block::CARROT_STAGE_1),
    (&["carrot_stage_2"], block::CARROT_STAGE_2),
    (&["carrot_stage_3", "carrot_mature"], block::CARROT_STAGE_3),
    (&["potato_stage_0", "potato_sprout"], block::POTATO_STAGE_0),
    (&["potato_stage_1"], block::POTATO_STAGE_1),
    (&["potato_stage_2"], block::POTATO_STAGE_2),
    (&["potato_stage_3", "potato_mature"], block::POTATO_STAGE_3),
    // Wave 27 — campfire (lit / unlit). Default `campfire` is the
    // unlit form so the player has to fuel + ignite it (matches
    // the gameplay flow). Aliases for both states for testing.
    (&["campfire", "camp_fire", "campfire_unlit"], block::CAMPFIRE_UNLIT),
    (&["campfire_lit", "lit_campfire"], block::CAMPFIRE),
    // Wave 28 — campfire smoke pillar block. Mostly a debug aid;
    // normal gameplay places + clears smoke via the campfire tick.
    (&["campfire_smoke", "smoke"], block::CAMPFIRE_SMOKE),
    // Wave 28 — corn crop stages. Single-block-tall, matches the
    // wheat/carrot/potato stage layout.
    (&["corn_stage_0", "corn_sprout"], block::CORN_STAGE_0),
    (&["corn_stage_1"], block::CORN_STAGE_1),
    (&["corn_stage_2"], block::CORN_STAGE_2),
    (&["corn_stage_3", "corn_mature"], block::CORN_STAGE_3),
    // Wave 29 — Drying Rack workstation. Built from 4 sticks 2×2 in
    // normal gameplay; `/give` for fast testing of the seasoning loop.
    (&["drying_rack", "dryingrack", "rack"], block::DRYING_RACK),
    // Spec 23 — Papyrus Reed stages. Debug-only `/give` for fast
    // visual checks of a specific stage; normal gameplay plants
    // stage 0 + grows via the tick.
    (&["papyrus_stage_0", "papyrus_sprout"], block::PAPYRUS_STAGE_0),
    (&["papyrus_stage_1"], block::PAPYRUS_STAGE_1),
    (&["papyrus_stage_2"], block::PAPYRUS_STAGE_2),
    (&["papyrus_stage_3", "papyrus_mature"], block::PAPYRUS_STAGE_3),
    // Spec 24 — Build Schematics Core. BLUEPRINT_PAPER is craftable; the
    // anchor + plaque are admin-debug aliases (engine places them
    // automatically during a build).
    (&["blueprint_paper", "tile", "plan_scroll", "scroll"], block::BLUEPRINT_PAPER),
    (&["construction_anchor", "anchor", "foundation_stone"], block::CONSTRUCTION_ANCHOR),
    (&["architect_plaque", "plaque", "masons_mark", "mark"], block::ARCHITECT_PLAQUE),
    (&["hay_bale", "hay_rick", "hay"], block::HAY_BALE),
    (&["bone_block", "bone_cairn"], block::BONE_BLOCK),
    (&["amethyst_block", "amethyst_cluster", "amethyst"], block::AMETHYST_BLOCK),
    (&["sugarcane", "sugar_cane", "cane"], block::SUGARCANE),
    (&["vendor_block", "market_stall", "stall"], block::VENDOR_BLOCK),
    (&["drafting_table", "drafting_bench", "drafting"], block::DRAFTING_TABLE),
    // Salt feature blocks.
    (&["rock_salt", "rocksalt", "salt_ore"], block::ROCK_SALT),
    (&["salt_lick", "saltlick"], block::SALT_LICK),
    (&["salt_lamp", "saltlamp"], block::SALT_LAMP),
    (&["salt_block", "saltblock"], block::SALT_BLOCK),
    (&["salt_path", "saltpath"], block::SALT_PATH),
    // Rubber feature blocks.
    (&["rubber_log", "rubberlog"], block::RUBBER_LOG),
    (&["rubber_planks", "rubberplanks"], block::RUBBER_PLANKS),
    (&["rubber_leaves", "rubberleaves"], block::RUBBER_LEAVES),
    (&["rubber_log_tapped", "rubberlogtapped", "tapped_log"], block::RUBBER_LOG_TAPPED),
    // Spec 36 Fences mini-spec (2026-05-28) — single fence post.
    (&["fence_post", "fencepost", "fence", "post"], block::FENCE_POST),
    // Per-wood-species fence posts (2026-05-28).
    (&["oak_fence_post", "oakfencepost"], block::OAK_FENCE_POST),
    (&["birch_fence_post", "birchfencepost"], block::BIRCH_FENCE_POST),
    (&["spruce_fence_post", "sprucefencepost"], block::SPRUCE_FENCE_POST),
    (&["jungle_fence_post", "junglefencepost"], block::JUNGLE_FENCE_POST),
    (&["acacia_fence_post", "acaciafencepost"], block::ACACIA_FENCE_POST),
    (&["dark_oak_fence_post", "darkoakfencepost"], block::DARK_OAK_FENCE_POST),
    (&["rubber_fence_post", "rubberfencepost"], block::RUBBER_FENCE_POST),
    // R5 — Tent (first multi-block-décor primitive + second
    // Canvas consumer).
    (&["tent"], block::TENT),
    // Spec 38 cyanotype-art v1 (2026-05-28) — direct /give for
    // testing; in normal play the print is acquired by right-
    // clicking a wall with a Developed Plan.
    (&["cyanotype_print", "cyanotypeprint", "cyanotype", "print"], block::CYANOTYPE_PRINT),
    // Spec 48 (Electricity / Power & Logic) — the redstone replacement.
    (&["cable", "wire", "copper_cable"], block::CABLE),
    (&["lamp", "electric_lamp", "electriclamp"], block::ELECTRIC_LAMP),
    (&["lever"], block::LEVER),
    (&["button"], block::BUTTON),
    (&["pressure_plate", "pressureplate", "plate"], block::PRESSURE_PLATE),
    (&["logic_gate", "logicgate", "gate"], block::LOGIC_GATE),
    (&["hand_crank", "handcrank", "crank"], block::HAND_CRANK),
    (&["steam_generator", "steamgenerator", "generator"], block::STEAM_GENERATOR),
    (&["battery", "accumulator"], block::BATTERY),
    // Spec 48 Phase 4 — the stream-driven source.
    (&["water_wheel", "waterwheel", "wheel"], block::WATER_WHEEL),
    // Wind, Copper & Electricity wave §2.2 — the wind-driven source.
    (&["windmill", "wind_mill", "mill"], block::WINDMILL),
    // Spec 48 Phase 2 — sensors.
    (&["beam_sensor", "beamsensor", "beam"], block::BEAM_SENSOR),
    (&["mirror"], block::MIRROR),
    (&["motion_sensor", "motionsensor", "motion"], block::MOTION_SENSOR),
    // Spec 49 (Explosives).
    (&["brimstone", "sulphur_ore", "sulfur_ore"], block::BRIMSTONE),
    (&["nitre_ore", "niter_ore", "nitre"], block::NITRE_ORE),
    (&["composter"], block::COMPOSTER),
    (&["blasting_keg", "keg", "explosive_keg"], block::BLASTING_KEG),
    (&["plunger_detonator", "plunger", "detonator"], block::PLUNGER_DETONATOR),
    // F1 block-shape foundation — shaped blocks (slab/stairs/fence gate).
    (&["stone_slab", "slab"], block::STONE_SLAB),
    (&["stone_stairs", "stairs"], block::STONE_STAIRS),
    // "gate" alone already resolves to the Electricity logic gate above
    // (Spec 48 predates this shaped-block foundation); "fence_gate" stays
    // as the short alias here to avoid the collision.
    (&["oak_fence_gate", "fence_gate"], block::OAK_FENCE_GATE),
    (&["oak_trapdoor", "trapdoor"], block::OAK_TRAPDOOR),
    (&["glass_pane", "pane"], block::GLASS_PANE),
    (&["iron_bars", "bars"], block::IRON_BARS),
    (&["oak_door", "door"], block::OAK_DOOR),
    (&["cobblestone_wall", "cobble_wall", "wall"], block::COBBLESTONE_WALL),
    (&["oak_sign", "sign"], block::OAK_SIGN),
    (&["item_frame", "frame"], block::ITEM_FRAME),
    // Living world S3 — the Feast and Commons blocks.
    (&["feast_table", "feasttable", "feast"], block::FEAST_TABLE),
    (&["commons_stone", "commonsstone", "commons"], block::COMMONS_STONE),
    // Living world Tier 2 S6 — the Commons Shelf.
    (&["commons_shelf", "commonsshelf", "shelf"], block::COMMONS_SHELF),
    (&["site_board", "siteboard"], block::SITE_BOARD),
    // Smelting + redstone-equivalent blocks — added so Trials challenge kits
    // (Smith / Engineer) can hand them out. The engine-managed PISTON_HEAD is
    // deliberately NOT giveable.
    (&["furnace"], block::FURNACE),
    (&["furnace_lit", "lit_furnace"], block::FURNACE_LIT),
    (&["piston"], block::PISTON),
    (&["sticky_piston", "stickypiston"], block::STICKY_PISTON),
    // Building variants: `spruce_stairs`, `sandstone_slab`, `dark_oak_door`,
    // `granite_wall`, … for every wood and stone (the table in
    // `building_variants`). The short `slab`/`stairs`/`door`/`trapdoor`/
    // `wall` aliases above stay the stone / oak / cobblestone originals.
];

pub(super) fn block_by_name(key: &str) -> Option<BlockId> {
    // The building-variant table mints the rest (`spruce_stairs`, ...).
    lookup(BLOCK_NAMES, key).or_else(|| crate::building_variants::id_by_name(key))
}

/// Every crafting material `/give` names. Tried AFTER blocks, tools and bows,
/// so a name a block also uses (`cable`, `nitre`) gives the block.
pub(super) static MATERIAL_NAMES: &[(&[&str], M)] = &[
    (&["bellows"], M::Bellows), // Spec 40 — the Workshop inflate tool
    // Spec 49 (Explosives).
    (&["sulphur", "sulfur"], M::Sulphur),
    (&["saltpetre", "saltpeter", "nitre", "niter"], M::Saltpetre),
    (&["black_powder", "blackpowder", "gunpowder"], M::BlackPowder),
    (&["compost"], M::Compost),
    // Spec 37 — Magnesium. Mineral + its products.
    (&["magnesium"], M::Magnesium),
    (&["fertiliser", "fertilizer"], M::Fertiliser),
    (&["sparkler"], M::Sparkler),
    (&["flare"], M::Flare),
    (&["magnesium_firestarter", "firestarter", "fire_starter"], M::MagnesiumFirestarter),
    // Buckets MC-parity — empty + filled. Fill an empty Bucket at a water/lava
    // source in-world, or /give the filled variants directly for testing.
    (&["bucket", "empty_bucket"], M::Bucket),
    (&["water_bucket", "waterbucket"], M::WaterBucket),
    (&["lava_bucket", "lavabucket"], M::LavaBucket),
    (&["stick"], M::Stick),
    (&["leather"], M::Leather),
    (&["feather"], M::Feather),
    (&["wool"], M::Wool),
    (&["bone"], M::Bone),
    (&["raw_beef", "rawbeef", "beef"], M::RawBeef),
    (&["raw_porkchop", "rawporkchop", "porkchop"], M::RawPorkchop),
    (&["raw_chicken", "rawchicken"], M::RawChicken),
    (&["raw_mutton", "rawmutton", "mutton"], M::RawMutton),
    // Companion tame foods (Task 19 — "A Friend in Need" trial): cats
    // fancy raw fish, foxes fancy berries. Parrot's tame food (wheat
    // seeds) already resolves via the `wheat_seeds` alias above.
    (&["raw_fish", "rawfish"], M::RawFish),
    (&["berries", "berry"], M::Berries),
    (&["string"], M::String),
    (&["bonemeal", "bone_meal"], M::Bonemeal),
    (&["honey_bottle", "honeybottle", "honey_jar", "honeyjar"], M::HoneyBottle),
    (&["coal"], M::Coal),
    (&["raw_iron", "rawiron", "iron"], M::RawIron),
    (&["diamond"], M::Diamond),
    (&["iron_ingot", "ironingot", "ingot"], M::IronIngot),
    // Wind, Copper & Electricity wave (2026-09-07) — `raw_copper` is the
    // smeltable material `/give copper`/`copper_ore` gives the ore BLOCK
    // (see block_by_name above, checked first). `copper_ingot` was
    // missing entirely; added alongside for parity with iron_ingot.
    (&["raw_copper", "rawcopper"], M::Copper),
    (&["copper_ingot", "copperingot"], M::CopperIngot),
    (&["cooked_beef", "cookedbeef", "steak"], M::CookedBeef),
    (&["cooked_porkchop", "cookedporkchop", "cooked_pork"], M::CookedPorkchop),
    (&["cooked_chicken", "cookedchicken"], M::CookedChicken),
    (&["cooked_mutton", "cookedmutton"], M::CookedMutton),
    (&["arrow", "arrows"], M::Arrow),
    // Satori (Wave 25) — `/give satori 5` works for testing the gem
    // economy before the proof-of-play strike path produces them
    // naturally. Legacy aliases (orange_gem / gem) retained so old
    // muscle memory still works.
    (&["satori", "orange_gem", "orangegem", "gem"], M::Satori),
    // Farming Tier 1 (Wave 26) — seed + food drops. `wheat` /
    // `carrot` / `potato` (no stage suffix) are the harvest
    // products; the matching `*_seeds` / dual-purpose seed items
    // sit alongside. Bread is the crafted staple food.
    (&["wheat"], M::Wheat),
    (&["wheat_seeds", "wheatseeds", "seeds"], M::WheatSeeds),
    (&["bread"], M::Bread),
    (&["carrot"], M::Carrot),
    (&["potato"], M::Potato),
    // Wave 27 — flint from gravel; used to craft flint-and-steel.
    (&["flint"], M::Flint),
    // Wave 28 — corn (separate-seed pattern, like wheat) + baked
    // variants from the campfire cooking loop.
    (&["corn"], M::Corn),
    (&["corn_seeds", "cornseeds"], M::CornSeeds),
    (&["baked_corn", "bakedcorn", "corn_on_the_cob", "corn_on_cob"], M::BakedCorn),
    (&["baked_potato", "bakedpotato"], M::BakedPotato),
    (&["baked_carrot", "bakedcarrot"], M::BakedCarrot),
    // Wave 29 — log seasoning materials. GreenLog is the tree-fell
    // drop in normal gameplay; SeasonedLog is the Drying Rack output;
    // KilnDriedLog is /give-only this wave (future Kiln workstation).
    (&["green_log", "greenlog"], M::GreenLog),
    (&["seasoned_log", "seasonedlog"], M::SeasonedLog),
    (&["kiln_dried_log", "kilndriedlog", "kiln_log", "kilnlog"], M::KilnDriedLog),
    // Spec 23 — Papyrus Reed + Papyrus Sheet (first paper material).
    // `papyrus` defaults to the reed; `paper` to the sheet. The
    // `papyrus_paper` alias matches the goal-doc spelling.
    (&["papyrus_reed", "papyrus", "reed"], M::PapyrusReed),
    (&["papyrus_sheet", "papyrussheet", "papyrus_paper", "paper"], M::PapyrusSheet),
    // Salt feature materials.
    (&["salt"], M::Salt),
    (&["salt_cured_beef", "saltcuredbeef", "cured_beef"], M::SaltCuredBeef),
    (&["salt_cured_porkchop", "saltcuredporkchop", "cured_pork"], M::SaltCuredPorkchop),
    (&["salt_cured_mutton", "saltcuredmutton", "cured_mutton"], M::SaltCuredMutton),
    (&["salt_cured_chicken", "saltcuredchicken", "cured_chicken"], M::SaltCuredChicken),
    (&["salt_cured_rabbit", "saltcuredrabbit", "cured_rabbit"], M::SaltCuredRabbit),
    (&["salt_cured_nostrich_meat", "saltcurednostrichmeat", "cured_nostrich"], M::SaltCuredNostrichMeat),
    (&["seasoned_bread", "seasonedbread"], M::SeasonedBread),
    (&["seasoned_baked_potato", "seasonedbakedpotato", "seasoned_potato"], M::SeasonedBakedPotato),
    (&["seasoned_baked_carrot", "seasonedbakedcarrot", "seasoned_carrot"], M::SeasonedBakedCarrot),
    (&["seasoned_baked_corn", "seasonedbakedcorn", "seasoned_corn"], M::SeasonedBakedCorn),
    (&["seasoned_cooked_beef", "seasonedcookedbeef", "seasoned_beef"], M::SeasonedCookedBeef),
    (&["seasoned_cooked_porkchop", "seasonedcookedporkchop", "seasoned_pork"], M::SeasonedCookedPorkchop),
    (&["seasoned_cooked_mutton", "seasonedcookedmutton"], M::SeasonedCookedMutton),
    (&["seasoned_cooked_chicken", "seasonedcookedchicken"], M::SeasonedCookedChicken),
    (&["seasoned_cooked_rabbit", "seasonedcookedrabbit"], M::SeasonedCookedRabbit),
    // Rubber feature materials.
    (&["rubber", "latex"], M::Rubber),
    (&["rubber_sapling", "rubbersapling"], M::RubberSapling),
    // Living world S4 — the Grey Quarry's kit hands out saplings of three
    // trees and Leads (animals are led into the quarry on one).
    (&["oak_sapling", "oaksapling"], M::OakSapling),
    (&["birch_sapling", "birchsapling"], M::BirchSapling),
    (&["spruce_sapling", "sprucesapling"], M::SpruceSapling),
    (&["jungle_sapling", "junglesapling"], M::JungleSapling),
    (&["acacia_sapling", "acaciasapling"], M::AcaciaSapling),
    (&["dark_oak_sapling", "darkoaksapling"], M::DarkOakSapling),
    (&["lead", "leash"], M::Lead),
    (&["rubber_ball", "rubberball", "ball"], M::RubberBall),
    (&["copper_cable", "coppercable", "cable"], M::CopperCable),
    (&["bounty_board", "bountyboard", "board"], M::BountyBoardItem),
    (&["tip_jar", "tipjar", "jar"], M::TipJarItem),
    (&["repair_bench", "repairbench", "anvil"], M::RepairBenchItem),
    (&["plot_marker", "plotmarker", "plot", "claim"], M::PlotMarkerItem),
    (&["market_bell", "marketbell", "market"], M::MarketBellItem),
    (&["auction_block", "auctionblock", "auction"], M::AuctionBlockItem),
    (&["bazaar", "bazaar_block", "merchant"], M::BazaarBlockItem),
    // Craftable Armoured Carts (CA2) — the three cart-item tiers, so
    // testers can `/give iron_cart` before the survival recipes are
    // unlocked. The bare `cart` alias gives the unarmoured wood tier.
    (&["wood_cart", "woodcart", "cart"], M::WoodCart),
    (&["iron_cart", "ironcart"], M::IronCart),
    (&["diamond_cart", "diamondcart"], M::DiamondCart),
    // Spec 35 farmable-flower follow-on (2026-05-28) — seed aliases
    // for the three primary dye flowers. The crop blocks themselves
    // are placed via worldgen + seed planting, not /give.
    (&["cornflower_seeds", "cornflowerseeds"], M::CornflowerSeeds),
    (&["field_poppy_seeds", "fieldpoppyseeds", "poppy_seeds", "poppyseeds"], M::FieldPoppySeeds),
    (&["buttercup_seeds", "buttercupseeds"], M::ButtercupSeeds),
    // Spec 35 dyes — the 16 colours, givable for the Colour Lab trial and
    // for testing the dye-mix / wallpaper-paint recipes. Names are the
    // snake_case of the MaterialId variant.
    (&["red_dye", "reddye"], M::RedDye),
    (&["yellow_dye", "yellowdye"], M::YellowDye),
    (&["blue_dye", "bluedye"], M::BlueDye),
    (&["white_dye", "whitedye"], M::WhiteDye),
    (&["black_dye", "blackdye"], M::BlackDye),
    (&["green_dye", "greendye"], M::GreenDye),
    (&["orange_dye", "orangedye"], M::OrangeDye),
    (&["purple_dye", "purpledye"], M::PurpleDye),
    (&["pink_dye", "pinkdye"], M::PinkDye),
    (&["lime_dye", "limedye"], M::LimeDye),
    (&["light_blue_dye", "lightbluedye"], M::LightBlueDye),
    (&["grey_dye", "greydye", "gray_dye", "graydye"], M::GreyDye),
    (&["light_grey_dye", "lightgreydye", "light_gray_dye"], M::LightGreyDye),
    (&["cyan_dye", "cyandye"], M::CyanDye),
    (&["magenta_dye", "magentadye"], M::MagentaDye),
    (&["brown_dye", "browndye"], M::BrownDye),
];

pub(super) fn material_by_name(key: &str) -> Option<crate::item::MaterialId> {
    lookup(MATERIAL_NAMES, key)
}

/// Tool materials. Canonical is `wooden`; `wood` and the legacy gem spellings
/// still parse. Canonical `satori_pickaxe` is single-word.
static TOOL_MATERIALS: &[(&[&str], ToolMaterial)] = &[
    (&["wooden", "wood"], ToolMaterial::Wood),
    (&["stone"], ToolMaterial::Stone),
    (&["iron"], ToolMaterial::Iron),
    (&["diamond"], ToolMaterial::Diamond),
    (&["satori", "orange", "orange_gem", "orangegem", "gem"], ToolMaterial::Satori),
];

/// Tool kinds. Farming Tier 1 (Wave 26): all 5 tiers parse a hoe.
static TOOL_KINDS: &[(&[&str], ToolType)] = &[
    (&["pickaxe"], ToolType::Pickaxe),
    (&["axe"], ToolType::Axe),
    (&["sword"], ToolType::Sword),
    (&["shovel"], ToolType::Shovel),
    (&["hoe"], ToolType::Hoe),
];

pub(super) fn tool_by_name(key: &str) -> Option<(ToolType, ToolMaterial)> {
    // Split from the right so multi-word materials like "orange_gem_pickaxe"
    // (legacy alias) still parse as ("orange_gem", "pickaxe") rather than
    // ("orange", "gem_pickaxe").
    let (mat, tool) = key.rsplit_once('_')?;
    let material = lookup(TOOL_MATERIALS, mat)?;
    let tool_type = lookup(TOOL_KINDS, tool)?;
    Some((tool_type, material))
}

/// Single-tier tools (`/give bow`, Wave 23, and the rest): not tier-based like
/// pickaxes, so they get their own table. The underscore-multi-word names
/// don't fit `tool_by_name`'s `<material>_<tool>` split.
pub(super) static BOW_NAMES: &[(&[&str], (ToolType, ToolMaterial))] = &[
    (&["bow"], (ToolType::Bow, ToolMaterial::Wood)),
    // Wave 27.
    (&["flint_and_steel", "flintandsteel", "lighter"], (ToolType::FlintAndSteel, ToolMaterial::Iron)),
    // Rubber feature — single-tier tools.
    (&["slingshot", "sling"], (ToolType::Slingshot, ToolMaterial::Wood)),
    (&["eraser", "rubber_eraser"], (ToolType::Eraser, ToolMaterial::Wood)),
    // Blueprint feature — Drafting Stamp.
    (&["drafting_stamp", "stamp"], (ToolType::DraftingStamp, ToolMaterial::Wood)),
    // Single-tier farm/utility tools (fixed material by recipe) — added so
    // Trials kits (Angler / Farmhand) can hand them out.
    (&["fishing_rod", "fishingrod", "rod"], (ToolType::FishingRod, ToolMaterial::Wood)),
    (&["shears"], (ToolType::Shears, ToolMaterial::Iron)),
];

pub(super) fn bow_by_name(key: &str) -> Option<(ToolType, ToolMaterial)> {
    lookup(BOW_NAMES, key)
}

// ---- The suggestion list ----------------------------------------------------------------

/// Names that work when typed but are deliberately not offered by Tab.
const NOT_SUGGESTED: &[&str] = &[
    // The campfire's smoke pillar is placed and cleared by the campfire tick;
    // `/give` for it is a debug aid, not something to offer a player.
    "campfire_smoke",
];

/// One suggested name per giveable item: `(name, the item it gives)`, sorted by
/// name. The name is the canonical (first) spelling, or the first later
/// spelling that really resolves to the item when a block shadows the
/// canonical one (`copper_cable` gives the cable BLOCK; the material is
/// `coppercable`). Each item appears once, under the name of the first table
/// row that gives it.
pub(super) static CATALOGUE: LazyLock<Vec<(String, Item)>> = LazyLock::new(build_catalogue);

fn build_catalogue() -> Vec<(String, Item)> {
    let mut out: BTreeMap<String, Item> = BTreeMap::new();
    let mut offer = |names: &[&str], item: Item| {
        if NOT_SUGGESTED.contains(&names[0]) {
            return;
        }
        // One name per item: when two rows give the same item (`oak_fence_post`
        // is the same block as `fence_post`), the first row keeps it.
        if out.values().any(|offered| *offered == item) {
            return;
        }
        let resolves = |n: &str| super::give::resolve_item(n, 1).is_ok_and(|stack| stack.item == item);
        if let Some(name) = names.iter().find(|n| resolves(n)) {
            out.entry((*name).to_string()).or_insert(item);
        }
    };
    for (names, id) in BLOCK_NAMES {
        offer(names, Item::Block(*id));
    }
    for (id, material, shape) in crate::building_variants::all() {
        let name = format!("{}_{}", material.key, shape.suffix());
        offer(&[name.as_str()], Item::Block(id));
    }
    for (material_names, material) in TOOL_MATERIALS {
        for (kind_names, kind) in TOOL_KINDS {
            let name = format!("{}_{}", material_names[0], kind_names[0]);
            offer(&[name.as_str()], Item::Tool(Tool::new(*kind, *material)));
        }
    }
    for (names, (kind, material)) in BOW_NAMES {
        offer(names, Item::Tool(Tool::new(*kind, *material)));
    }
    for (names, material) in MATERIAL_NAMES {
        offer(names, Item::Material(*material));
    }
    offer(&["debug_plan"], Item::Plan(crate::plan::PlanData::debug_3x3_stone()));
    offer(&["hut_plan"], Item::Plan(crate::plan::PlanData::small_hut()));
    out.into_iter().collect()
}

static ITEM_NAMES: LazyLock<Vec<String>> =
    LazyLock::new(|| CATALOGUE.iter().map(|(name, _)| name.clone()).collect());

static BLOCK_ONLY_NAMES: LazyLock<Vec<String>> = LazyLock::new(|| {
    CATALOGUE
        .iter()
        .filter(|(_, item)| matches!(item, Item::Block(_)))
        .map(|(name, _)| name.clone())
        .collect()
});

/// Every name Tab completes after `/give`, sorted. Aliases are not listed but
/// still work when typed.
pub fn item_names() -> &'static [String] {
    &ITEM_NAMES
}

/// The subset of `item_names` that gives a placeable block (what `/we set`,
/// `/we replace` and `/we walls` accept).
pub fn block_names() -> &'static [String] {
    &BLOCK_ONLY_NAMES
}
