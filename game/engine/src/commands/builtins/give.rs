use crate::block::{self, BlockId};
use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::Command;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::ItemStack;

pub struct GiveCommand;

/// Resolve a player-typed item name to a constructed `ItemStack`.
/// Returns `Err(message)` for unknown items.
///
/// `pub(crate)` so the scenario runner (`scenario.rs` + the `/scenario`
/// handler) can provision a `ScenarioDef.kit` through the same vocabulary the
/// `/give` command uses — one resolver, no duplicate item tables.
pub(crate) fn resolve_item(name: &str, count: u8) -> Result<ItemStack, String> {
    let key = name.to_lowercase().replace('-', "_");
    if let Some(block_id) = block_by_name(&key) {
        return Ok(ItemStack::new_block(block_id, count.max(1)));
    }
    if let Some((tt, tm)) = tool_by_name(&key) {
        // Tools don't stack; ignore count.
        return Ok(ItemStack::new_tool(Tool::new(tt, tm)));
    }
    if let Some((tt, tm)) = bow_by_name(&key) {
        return Ok(ItemStack::new_tool(Tool::new(tt, tm)));
    }
    if let Some(mat) = material_by_name(&key) {
        return Ok(ItemStack::new_material(mat, count.max(1)));
    }
    // Spec 24 — debug Plan item. 3×3 stone footprint, default name +
    // CC-BY-SA licence. Useful for testing Inspect / place / animated
    // build paths once those phases ship. Plans don't stack, so count
    // is ignored.
    if matches!(key.as_str(), "debug_plan" | "plan" | "test_plan") {
        return Ok(crate::item::ItemStack {
            item: crate::item::Item::Plan(crate::plan::PlanData::debug_3x3_stone()),
            count: 1,
        });
    }
    // Wind, Copper & Electricity wave (2026-09-07) — the little-hut plan the
    // "Follow the Plan" build-along Trial provisions. Plans don't stack, so
    // count is ignored here too.
    if matches!(key.as_str(), "hut_plan" | "house_plan" | "small_house") {
        return Ok(crate::item::ItemStack {
            item: crate::item::Item::Plan(crate::plan::PlanData::small_hut()),
            count: 1,
        });
    }
    Err(format!(
        "unknown item: '{name}' (try /give stone, /give wooden_pickaxe, /give bow, /give arrow, /give coal, etc.)"
    ))
}

fn block_by_name(key: &str) -> Option<BlockId> {
    match key {
        "stone" => Some(block::STONE),
        "dirt" => Some(block::DIRT),
        "grass" => Some(block::GRASS),
        "bedrock" => Some(block::BEDROCK),
        "sand" => Some(block::SAND),
        "water" => Some(block::WATER),
        // Lava as a placeable block — mirrors `water` (apply_arena_setup registers
        // it as a source). Enables `/give lava` for testing + the "Floor Is Lava"
        // arena. The survival way to move lava stays the lava bucket.
        "lava" => Some(block::LAVA),
        "oak_log" | "log" => Some(block::OAK_LOG),
        "oak_leaves" | "leaves" => Some(block::OAK_LEAVES),
        "oak_planks" | "planks" => Some(block::OAK_PLANKS),
        "cobblestone" | "cobble" => Some(block::COBBLESTONE),
        "gravel" => Some(block::GRAVEL),
        "sandstone" => Some(block::SANDSTONE),
        "snow" => Some(block::SNOW),
        "crafting_table" | "workbench" => Some(block::CRAFTING_TABLE),
        "bed" => Some(block::BED),
        "coal_ore" => Some(block::COAL_ORE),
        "iron_ore" => Some(block::IRON_ORE),
        "diamond_ore" => Some(block::DIAMOND_ORE),
        // Wind, Copper & Electricity wave (2026-09-07) — `copper`/`copper_ore`
        // resolve to the ore BLOCK (checked before material_by_name below, so
        // this wins); `raw_copper` resolves to the smeltable MATERIAL instead.
        "copper_ore" | "copper" => Some(block::COPPER_ORE),
        "glass" => Some(block::GLASS),
        "coal_block" | "coalblock" => Some(block::COAL_BLOCK),
        "iron_block" | "ironblock" => Some(block::IRON_BLOCK),
        "diamond_block" | "diamondblock" => Some(block::DIAMOND_BLOCK),
        "torch" => Some(block::TORCH),
        "chest" => Some(block::CHEST),
        // Rail freight (Phase 1) — `/give track` lets a builder lay rail (no
        // survival recipe yet). Depots are just crafted chests (`/give chest`
        // above, or the 8-plank ring) placed beside a track terminus.
        "track" | "rail" => Some(crate::rail::TRACK),
        "tall_grass" | "tallgrass" => Some(block::TALL_GRASS),
        // Farming Tier 1 (Wave 26) — tilled soil + 12 crop-stage blocks.
        // Convenience aliases: `wheat`, `carrot`, `potato` (no stage
        // suffix) are MATERIAL names — handled by material_by_name —
        // but the crop *blocks* live here so `/give wheat_stage_3 1`
        // gives a placeable mature wheat for setup screenshots.
        "tilled_soil" | "tilledsoil" | "farmland" => Some(block::TILLED_SOIL),
        "wheat_stage_0" | "wheat_sprout" => Some(block::WHEAT_STAGE_0),
        "wheat_stage_1" => Some(block::WHEAT_STAGE_1),
        "wheat_stage_2" => Some(block::WHEAT_STAGE_2),
        "wheat_stage_3" | "wheat_mature" => Some(block::WHEAT_STAGE_3),
        "carrot_stage_0" | "carrot_sprout" => Some(block::CARROT_STAGE_0),
        "carrot_stage_1" => Some(block::CARROT_STAGE_1),
        "carrot_stage_2" => Some(block::CARROT_STAGE_2),
        "carrot_stage_3" | "carrot_mature" => Some(block::CARROT_STAGE_3),
        "potato_stage_0" | "potato_sprout" => Some(block::POTATO_STAGE_0),
        "potato_stage_1" => Some(block::POTATO_STAGE_1),
        "potato_stage_2" => Some(block::POTATO_STAGE_2),
        "potato_stage_3" | "potato_mature" => Some(block::POTATO_STAGE_3),
        // Wave 27 — campfire (lit / unlit). Default `campfire` is the
        // unlit form so the player has to fuel + ignite it (matches
        // the gameplay flow). Aliases for both states for testing.
        "campfire" | "camp_fire" | "campfire_unlit" => Some(block::CAMPFIRE_UNLIT),
        "campfire_lit" | "lit_campfire" => Some(block::CAMPFIRE),
        // Wave 28 — campfire smoke pillar block. Mostly a debug aid;
        // normal gameplay places + clears smoke via the campfire tick.
        "campfire_smoke" | "smoke" => Some(block::CAMPFIRE_SMOKE),
        // Wave 28 — corn crop stages. Single-block-tall, matches the
        // wheat/carrot/potato stage layout.
        "corn_stage_0" | "corn_sprout" => Some(block::CORN_STAGE_0),
        "corn_stage_1" => Some(block::CORN_STAGE_1),
        "corn_stage_2" => Some(block::CORN_STAGE_2),
        "corn_stage_3" | "corn_mature" => Some(block::CORN_STAGE_3),
        // Wave 29 — Drying Rack workstation. Built from 4 sticks 2×2 in
        // normal gameplay; `/give` for fast testing of the seasoning loop.
        "drying_rack" | "dryingrack" | "rack" => Some(block::DRYING_RACK),
        // Spec 23 — Papyrus Reed stages. Debug-only `/give` for fast
        // visual checks of a specific stage; normal gameplay plants
        // stage 0 + grows via the tick.
        "papyrus_stage_0" | "papyrus_sprout" => Some(block::PAPYRUS_STAGE_0),
        "papyrus_stage_1" => Some(block::PAPYRUS_STAGE_1),
        "papyrus_stage_2" => Some(block::PAPYRUS_STAGE_2),
        "papyrus_stage_3" | "papyrus_mature" => Some(block::PAPYRUS_STAGE_3),
        // Spec 24 — Build Schematics Core. BLUEPRINT_PAPER is craftable; the
        // anchor + plaque are admin-debug aliases (engine places them
        // automatically during a build).
        "blueprint_paper" | "tile" | "plan_scroll" | "scroll" => Some(block::BLUEPRINT_PAPER),
        "construction_anchor" | "anchor" | "foundation_stone" => Some(block::CONSTRUCTION_ANCHOR),
        "architect_plaque" | "plaque" | "masons_mark" | "mark" => Some(block::ARCHITECT_PLAQUE),
        "hay_bale" | "hay_rick" | "hay" => Some(block::HAY_BALE),
        "bone_block" | "bone_cairn" => Some(block::BONE_BLOCK),
        "amethyst_block" | "amethyst_cluster" | "amethyst" => Some(block::AMETHYST_BLOCK),
        "sugarcane" | "sugar_cane" | "cane" => Some(block::SUGARCANE),
        "vendor_block" | "market_stall" | "stall" => Some(block::VENDOR_BLOCK),
        "drafting_table" | "drafting_bench" | "drafting" => Some(block::DRAFTING_TABLE),
        // Salt feature blocks.
        "rock_salt" | "rocksalt" | "salt_ore" => Some(block::ROCK_SALT),
        "salt_lick" | "saltlick" => Some(block::SALT_LICK),
        "salt_lamp" | "saltlamp" => Some(block::SALT_LAMP),
        "salt_block" | "saltblock" => Some(block::SALT_BLOCK),
        "salt_path" | "saltpath" => Some(block::SALT_PATH),
        // Rubber feature blocks.
        "rubber_log" | "rubberlog" => Some(block::RUBBER_LOG),
        "rubber_planks" | "rubberplanks" => Some(block::RUBBER_PLANKS),
        "rubber_leaves" | "rubberleaves" => Some(block::RUBBER_LEAVES),
        "rubber_log_tapped" | "rubberlogtapped" | "tapped_log" => Some(block::RUBBER_LOG_TAPPED),
        // Spec 36 Fences mini-spec (2026-05-28) — single fence post.
        "fence_post" | "fencepost" | "fence" | "post" => Some(block::FENCE_POST),
        // Per-wood-species fence posts (2026-05-28).
        "oak_fence_post" | "oakfencepost" => Some(block::OAK_FENCE_POST),
        "birch_fence_post" | "birchfencepost" => Some(block::BIRCH_FENCE_POST),
        "spruce_fence_post" | "sprucefencepost" => Some(block::SPRUCE_FENCE_POST),
        "jungle_fence_post" | "junglefencepost" => Some(block::JUNGLE_FENCE_POST),
        "acacia_fence_post" | "acaciafencepost" => Some(block::ACACIA_FENCE_POST),
        "dark_oak_fence_post" | "darkoakfencepost" => Some(block::DARK_OAK_FENCE_POST),
        "rubber_fence_post" | "rubberfencepost" => Some(block::RUBBER_FENCE_POST),
        // R5 — Tent (first multi-block-décor primitive + second
        // Canvas consumer).
        "tent" => Some(block::TENT),
        // Spec 38 cyanotype-art v1 (2026-05-28) — direct /give for
        // testing; in normal play the print is acquired by right-
        // clicking a wall with a Developed Plan.
        "cyanotype_print" | "cyanotypeprint" | "cyanotype" | "print" => Some(block::CYANOTYPE_PRINT),
        // Spec 48 (Electricity / Power & Logic) — the redstone replacement.
        "cable" | "wire" | "copper_cable" => Some(block::CABLE),
        "lamp" | "electric_lamp" | "electriclamp" => Some(block::ELECTRIC_LAMP),
        "lever" => Some(block::LEVER),
        "button" => Some(block::BUTTON),
        "pressure_plate" | "pressureplate" | "plate" => Some(block::PRESSURE_PLATE),
        "logic_gate" | "logicgate" | "gate" => Some(block::LOGIC_GATE),
        "hand_crank" | "handcrank" | "crank" => Some(block::HAND_CRANK),
        "steam_generator" | "steamgenerator" | "generator" => Some(block::STEAM_GENERATOR),
        "battery" | "accumulator" => Some(block::BATTERY),
        // Spec 48 Phase 4 — the stream-driven source.
        "water_wheel" | "waterwheel" | "wheel" => Some(block::WATER_WHEEL),
        // Wind, Copper & Electricity wave §2.2 — the wind-driven source.
        "windmill" | "wind_mill" | "mill" => Some(block::WINDMILL),
        // Spec 48 Phase 2 — sensors.
        "beam_sensor" | "beamsensor" | "beam" => Some(block::BEAM_SENSOR),
        "mirror" => Some(block::MIRROR),
        "motion_sensor" | "motionsensor" | "motion" => Some(block::MOTION_SENSOR),
        // Spec 49 (Explosives).
        "brimstone" | "sulphur_ore" | "sulfur_ore" => Some(block::BRIMSTONE),
        "nitre_ore" | "niter_ore" | "nitre" => Some(block::NITRE_ORE),
        "composter" => Some(block::COMPOSTER),
        "blasting_keg" | "keg" | "explosive_keg" => Some(block::BLASTING_KEG),
        "plunger_detonator" | "plunger" | "detonator" => Some(block::PLUNGER_DETONATOR),
        // F1 block-shape foundation — shaped blocks (slab/stairs/fence gate).
        "stone_slab" | "slab" => Some(block::STONE_SLAB),
        "stone_stairs" | "stairs" => Some(block::STONE_STAIRS),
        // "gate" alone already resolves to the Electricity logic gate above
        // (Spec 48 predates this shaped-block foundation); "fence_gate" stays
        // as the short alias here to avoid the collision.
        "oak_fence_gate" | "fence_gate" => Some(block::OAK_FENCE_GATE),
        "oak_trapdoor" | "trapdoor" => Some(block::OAK_TRAPDOOR),
        "glass_pane" | "pane" => Some(block::GLASS_PANE),
        "iron_bars" | "bars" => Some(block::IRON_BARS),
        "oak_door" | "door" => Some(block::OAK_DOOR),
        "cobblestone_wall" | "cobble_wall" | "wall" => Some(block::COBBLESTONE_WALL),
        "oak_sign" | "sign" => Some(block::OAK_SIGN),
        "item_frame" | "frame" => Some(block::ITEM_FRAME),
        // Smelting + redstone-equivalent blocks — added so Trials challenge kits
        // (Smith / Engineer) can hand them out. The engine-managed PISTON_HEAD is
        // deliberately NOT giveable.
        "furnace" => Some(block::FURNACE),
        "furnace_lit" | "lit_furnace" => Some(block::FURNACE_LIT),
        "piston" => Some(block::PISTON),
        "sticky_piston" | "stickypiston" => Some(block::STICKY_PISTON),
        _ => None,
    }
}

fn material_by_name(key: &str) -> Option<crate::item::MaterialId> {
    use crate::item::MaterialId as M;
    match key {
        "bellows" => Some(M::Bellows), // Spec 40 — the Workshop inflate tool
        // Spec 49 (Explosives).
        "sulphur" | "sulfur" => Some(M::Sulphur),
        "saltpetre" | "saltpeter" | "nitre" | "niter" => Some(M::Saltpetre),
        "black_powder" | "blackpowder" | "gunpowder" => Some(M::BlackPowder),
        "compost" => Some(M::Compost),
        // Spec 37 — Magnesium. Mineral + its products.
        "magnesium" => Some(M::Magnesium),
        "fertiliser" | "fertilizer" => Some(M::Fertiliser),
        "sparkler" => Some(M::Sparkler),
        "flare" => Some(M::Flare),
        "magnesium_firestarter" | "firestarter" | "fire_starter" => Some(M::MagnesiumFirestarter),
        // Buckets MC-parity — empty + filled. Fill an empty Bucket at a water/lava
        // source in-world, or /give the filled variants directly for testing.
        "bucket" | "empty_bucket" => Some(M::Bucket),
        "water_bucket" | "waterbucket" => Some(M::WaterBucket),
        "lava_bucket" | "lavabucket" => Some(M::LavaBucket),
        "stick" => Some(M::Stick),
        "leather" => Some(M::Leather),
        "feather" => Some(M::Feather),
        "wool" => Some(M::Wool),
        "bone" => Some(M::Bone),
        "raw_beef" | "rawbeef" | "beef" => Some(M::RawBeef),
        "raw_porkchop" | "rawporkchop" | "porkchop" => Some(M::RawPorkchop),
        "raw_chicken" | "rawchicken" => Some(M::RawChicken),
        "raw_mutton" | "rawmutton" | "mutton" => Some(M::RawMutton),
        // Companion tame foods (Task 19 — "A Friend in Need" trial): cats
        // fancy raw fish, foxes fancy berries. Parrot's tame food (wheat
        // seeds) already resolves via the `wheat_seeds` alias above.
        "raw_fish" | "rawfish" => Some(M::RawFish),
        "berries" | "berry" => Some(M::Berries),
        "string" => Some(M::String),
        "bonemeal" | "bone_meal" => Some(M::Bonemeal),
        "honey_bottle" | "honeybottle" | "honey_jar" | "honeyjar" => Some(M::HoneyBottle),
        "coal" => Some(M::Coal),
        "raw_iron" | "rawiron" | "iron" => Some(M::RawIron),
        "diamond" => Some(M::Diamond),
        "iron_ingot" | "ironingot" | "ingot" => Some(M::IronIngot),
        // Wind, Copper & Electricity wave (2026-09-07) — `raw_copper` is the
        // smeltable material `/give copper`/`copper_ore` gives the ore BLOCK
        // (see block_by_name above, checked first). `copper_ingot` was
        // missing entirely; added alongside for parity with iron_ingot.
        "raw_copper" | "rawcopper" => Some(M::Copper),
        "copper_ingot" | "copperingot" => Some(M::CopperIngot),
        "cooked_beef" | "cookedbeef" | "steak" => Some(M::CookedBeef),
        "cooked_porkchop" | "cookedporkchop" | "cooked_pork" => Some(M::CookedPorkchop),
        "cooked_chicken" | "cookedchicken" => Some(M::CookedChicken),
        "cooked_mutton" | "cookedmutton" => Some(M::CookedMutton),
        "arrow" | "arrows" => Some(M::Arrow),
        // Satori (Wave 25) — `/give satori 5` works for testing the gem
        // economy before the proof-of-play strike path produces them
        // naturally. Legacy aliases (orange_gem / gem) retained so old
        // muscle memory still works.
        "satori" | "orange_gem" | "orangegem" | "gem" => Some(M::Satori),
        // Farming Tier 1 (Wave 26) — seed + food drops. `wheat` /
        // `carrot` / `potato` (no stage suffix) are the harvest
        // products; the matching `*_seeds` / dual-purpose seed items
        // sit alongside. Bread is the crafted staple food.
        "wheat" => Some(M::Wheat),
        "wheat_seeds" | "wheatseeds" | "seeds" => Some(M::WheatSeeds),
        "bread" => Some(M::Bread),
        "carrot" => Some(M::Carrot),
        "potato" => Some(M::Potato),
        // Wave 27 — flint from gravel; used to craft flint-and-steel.
        "flint" => Some(M::Flint),
        // Wave 28 — corn (separate-seed pattern, like wheat) + baked
        // variants from the campfire cooking loop.
        "corn" => Some(M::Corn),
        "corn_seeds" | "cornseeds" => Some(M::CornSeeds),
        "baked_corn" | "bakedcorn" | "corn_on_the_cob" | "corn_on_cob" => Some(M::BakedCorn),
        "baked_potato" | "bakedpotato" => Some(M::BakedPotato),
        "baked_carrot" | "bakedcarrot" => Some(M::BakedCarrot),
        // Wave 29 — log seasoning materials. GreenLog is the tree-fell
        // drop in normal gameplay; SeasonedLog is the Drying Rack output;
        // KilnDriedLog is /give-only this wave (future Kiln workstation).
        "green_log" | "greenlog" => Some(M::GreenLog),
        "seasoned_log" | "seasonedlog" => Some(M::SeasonedLog),
        "kiln_dried_log" | "kilndriedlog" | "kiln_log" | "kilnlog" => Some(M::KilnDriedLog),
        // Spec 23 — Papyrus Reed + Papyrus Sheet (first paper material).
        // `papyrus` defaults to the reed; `paper` to the sheet. The
        // `papyrus_paper` alias matches the goal-doc spelling.
        "papyrus_reed" | "papyrus" | "reed" => Some(M::PapyrusReed),
        "papyrus_sheet" | "papyrussheet" | "papyrus_paper" | "paper" => Some(M::PapyrusSheet),
        // Salt feature materials.
        "salt" => Some(M::Salt),
        "salt_cured_beef" | "saltcuredbeef" | "cured_beef" => Some(M::SaltCuredBeef),
        "salt_cured_porkchop" | "saltcuredporkchop" | "cured_pork" => Some(M::SaltCuredPorkchop),
        "salt_cured_mutton" | "saltcuredmutton" | "cured_mutton" => Some(M::SaltCuredMutton),
        "salt_cured_chicken" | "saltcuredchicken" | "cured_chicken" => Some(M::SaltCuredChicken),
        "salt_cured_rabbit" | "saltcuredrabbit" | "cured_rabbit" => Some(M::SaltCuredRabbit),
        "salt_cured_nostrich_meat" | "saltcurednostrichmeat" | "cured_nostrich" => Some(M::SaltCuredNostrichMeat),
        "seasoned_bread" | "seasonedbread" => Some(M::SeasonedBread),
        "seasoned_baked_potato" | "seasonedbakedpotato" | "seasoned_potato" => Some(M::SeasonedBakedPotato),
        "seasoned_baked_carrot" | "seasonedbakedcarrot" | "seasoned_carrot" => Some(M::SeasonedBakedCarrot),
        "seasoned_baked_corn" | "seasonedbakedcorn" | "seasoned_corn" => Some(M::SeasonedBakedCorn),
        "seasoned_cooked_beef" | "seasonedcookedbeef" | "seasoned_beef" => Some(M::SeasonedCookedBeef),
        "seasoned_cooked_porkchop" | "seasonedcookedporkchop" | "seasoned_pork" => Some(M::SeasonedCookedPorkchop),
        "seasoned_cooked_mutton" | "seasonedcookedmutton" => Some(M::SeasonedCookedMutton),
        "seasoned_cooked_chicken" | "seasonedcookedchicken" => Some(M::SeasonedCookedChicken),
        "seasoned_cooked_rabbit" | "seasonedcookedrabbit" => Some(M::SeasonedCookedRabbit),
        // Rubber feature materials.
        "rubber" | "latex" => Some(M::Rubber),
        "rubber_sapling" | "rubbersapling" => Some(M::RubberSapling),
        "rubber_ball" | "rubberball" | "ball" => Some(M::RubberBall),
        "copper_cable" | "coppercable" | "cable" => Some(M::CopperCable),
        "bounty_board" | "bountyboard" | "board" => Some(M::BountyBoardItem),
        "tip_jar" | "tipjar" | "jar" => Some(M::TipJarItem),
        "repair_bench" | "repairbench" | "anvil" => Some(M::RepairBenchItem),
        "plot_marker" | "plotmarker" | "plot" | "claim" => Some(M::PlotMarkerItem),
        "market_bell" | "marketbell" | "market" => Some(M::MarketBellItem),
        "auction_block" | "auctionblock" | "auction" => Some(M::AuctionBlockItem),
        "bazaar" | "bazaar_block" | "merchant" => Some(M::BazaarBlockItem),
        // Craftable Armoured Carts (CA2) — the three cart-item tiers, so
        // testers can `/give iron_cart` before the survival recipes are
        // unlocked. The bare `cart` alias gives the unarmoured wood tier.
        "wood_cart" | "woodcart" | "cart" => Some(M::WoodCart),
        "iron_cart" | "ironcart" => Some(M::IronCart),
        "diamond_cart" | "diamondcart" => Some(M::DiamondCart),
        // Spec 35 farmable-flower follow-on (2026-05-28) — seed aliases
        // for the three primary dye flowers. The crop blocks themselves
        // are placed via worldgen + seed planting, not /give.
        "cornflower_seeds" | "cornflowerseeds" => Some(M::CornflowerSeeds),
        "field_poppy_seeds" | "fieldpoppyseeds" | "poppy_seeds" | "poppyseeds" => Some(M::FieldPoppySeeds),
        "buttercup_seeds" | "buttercupseeds" => Some(M::ButtercupSeeds),
        // Spec 35 dyes — the 16 colours, givable for the Colour Lab trial and
        // for testing the dye-mix / wallpaper-paint recipes. Names are the
        // snake_case of the MaterialId variant.
        "red_dye" | "reddye" => Some(M::RedDye),
        "yellow_dye" | "yellowdye" => Some(M::YellowDye),
        "blue_dye" | "bluedye" => Some(M::BlueDye),
        "white_dye" | "whitedye" => Some(M::WhiteDye),
        "black_dye" | "blackdye" => Some(M::BlackDye),
        "green_dye" | "greendye" => Some(M::GreenDye),
        "orange_dye" | "orangedye" => Some(M::OrangeDye),
        "purple_dye" | "purpledye" => Some(M::PurpleDye),
        "pink_dye" | "pinkdye" => Some(M::PinkDye),
        "lime_dye" | "limedye" => Some(M::LimeDye),
        "light_blue_dye" | "lightbluedye" => Some(M::LightBlueDye),
        "grey_dye" | "greydye" | "gray_dye" | "graydye" => Some(M::GreyDye),
        "light_grey_dye" | "lightgreydye" | "light_gray_dye" => Some(M::LightGreyDye),
        "cyan_dye" | "cyandye" => Some(M::CyanDye),
        "magenta_dye" | "magentadye" => Some(M::MagentaDye),
        "brown_dye" | "browndye" => Some(M::BrownDye),
        _ => None,
    }
}

fn tool_by_name(key: &str) -> Option<(ToolType, ToolMaterial)> {
    // Split from the right so multi-word materials like "orange_gem_pickaxe"
    // (legacy alias) still parse as ("orange_gem", "pickaxe") rather than
    // ("orange", "gem_pickaxe"). Canonical "satori_pickaxe" is single-word.
    let (mat, tool) = key.rsplit_once('_')?;
    let material = match mat {
        "wood" | "wooden" => ToolMaterial::Wood,
        "stone" => ToolMaterial::Stone,
        "iron" => ToolMaterial::Iron,
        "diamond" => ToolMaterial::Diamond,
        "satori" | "orange" | "orange_gem" | "orangegem" | "gem" => ToolMaterial::Satori,
        _ => return None,
    };
    let tool_type = match tool {
        "pickaxe" => ToolType::Pickaxe,
        "axe" => ToolType::Axe,
        "sword" => ToolType::Sword,
        "shovel" => ToolType::Shovel,
        // Farming Tier 1 (Wave 26). All 5 tiers parse.
        "hoe" => ToolType::Hoe,
        _ => return None,
    };
    Some((tool_type, material))
}

/// `/give bow` — Wave 23. Bows are a single wood-tier item, not tier-based
/// like pickaxes/swords/etc, so they get their own resolver. Wave 27
/// adds `flint_and_steel` to the same single-tier-item parser; the
/// underscore-multi-word name doesn't fit `tool_by_name`'s
/// `<material>_<tool>` split.
fn bow_by_name(key: &str) -> Option<(ToolType, ToolMaterial)> {
    match key {
        "bow" => Some((ToolType::Bow, ToolMaterial::Wood)),
        "flint_and_steel" | "flintandsteel" | "lighter" => {
            Some((ToolType::FlintAndSteel, ToolMaterial::Iron))
        }
        // Rubber feature — single-tier tools.
        "slingshot" | "sling" => Some((ToolType::Slingshot, ToolMaterial::Wood)),
        "eraser" | "rubber_eraser" => Some((ToolType::Eraser, ToolMaterial::Wood)),
        // Blueprint feature — Drafting Stamp.
        "drafting_stamp" | "stamp" => Some((ToolType::DraftingStamp, ToolMaterial::Wood)),
        // Single-tier farm/utility tools (fixed material by recipe) — added so
        // Trials kits (Angler / Farmhand) can hand them out.
        "fishing_rod" | "fishingrod" | "rod" => Some((ToolType::FishingRod, ToolMaterial::Wood)),
        "shears" => Some((ToolType::Shears, ToolMaterial::Iron)),
        _ => None,
    }
}

impl Command for GiveCommand {
    fn name(&self) -> &'static str {
        "give"
    }
    fn help(&self) -> &'static str {
        "Add an item to your inventory"
    }
    fn usage(&self) -> &'static str {
        "/give <item> [count]   e.g. /give stone 64, /give wooden_pickaxe"
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        let item_name = match args.first() {
            Some(s) => s,
            None => {
                let msg = "usage: /give <item> [count]".to_string();
                ctx.error(msg.clone());
                return CommandResult::Error(msg);
            }
        };
        let (count, capped_from): (u8, Option<u32>) = match args.get(1) {
            Some(s) => match s.parse::<u32>() {
                Ok(n) if n >= 1 => {
                    if n > 64 {
                        (64, Some(n))
                    } else {
                        (n as u8, None)
                    }
                }
                Ok(_) => {
                    let msg = "count must be ≥ 1".to_string();
                    ctx.error(msg.clone());
                    return CommandResult::Error(msg);
                }
                Err(_) => {
                    let msg = format!("can't parse '{s}' as a count");
                    ctx.error(msg.clone());
                    return CommandResult::Error(msg);
                }
            },
            None => (1, None),
        };
        let stack = match resolve_item(item_name, count) {
            Ok(s) => s,
            Err(e) => {
                ctx.error(e.clone());
                return CommandResult::Error(e);
            }
        };
        let Some(slot) = ctx.players.get_mut(ctx.player_idx) else {
            let msg = "no player slot".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        };
        if slot.inventory.add_item(stack).is_none() {
            // /give breaks pure-survival, consistent with /clear and /gamemode
            // (engine audit 2026-06-04, F: /give marked cheats_used but not the
            // pure-survival ledger, so `/give diamond_block 64` still read as a
            // pure survival run). Owner decision P0.3.
            *ctx.cheats_used_marker = true;
            *ctx.pure_survival_broken_marker = true;
            let msg = match capped_from {
                Some(n) => format!("Gave {count} × {item_name} (capped from {n})"),
                None => format!("Gave {count} × {item_name}"),
            };
            ctx.success(msg);
            CommandResult::Success
        } else {
            let msg = "inventory full".to_string();
            ctx.error(msg.clone());
            CommandResult::Error(msg)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::player_slot::PlayerSlot;

    fn run(args: &[&str]) -> (CommandResult, Vec<PlayerSlot>, Vec<String>) {
        let cmd = GiveCommand;
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        let mut log = Vec::new();
        let mut ch = false;
        let mut ev = false;
        let mut ps = false;
        let reg = CommandRegistry::new();
        let mut ctx = CommandContext {
            world: &mut world,
            world_time: &mut t,
            world_time_step: &mut s,
            is_creative: &mut creative, play_mode: &mut { use crate::play_mode::PlayMode; PlayMode::Survival },
            seed: 42,
            world_name: "test",
            players: &mut players,
            player_idx: 0,
            op_level: OpLevel::Op,
            current_tick: 0,
            log: &mut log,
            registry: &reg,
            cheats_used_marker: &mut ch,
            ever_creative_marker: &mut ev,
            pure_survival_broken_marker: &mut ps,
        };
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let r = cmd.execute(&mut ctx, &owned);
        let lines: Vec<String> = log.iter().map(|l| l.text.clone()).collect();
        (r, players, lines)
    }

    /// Like `run`, but surfaces the (cheats_used, pure_survival_broken) markers.
    fn run_markers(args: &[&str]) -> (CommandResult, bool, bool) {
        let cmd = GiveCommand;
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        let mut log = Vec::new();
        let mut ch = false;
        let mut ev = false;
        let mut ps = false;
        let reg = CommandRegistry::new();
        let mut ctx = CommandContext {
            world: &mut world,
            world_time: &mut t,
            world_time_step: &mut s,
            is_creative: &mut creative, play_mode: &mut { use crate::play_mode::PlayMode; PlayMode::Survival },
            seed: 42,
            world_name: "test",
            players: &mut players,
            player_idx: 0,
            op_level: OpLevel::Op,
            current_tick: 0,
            log: &mut log,
            registry: &reg,
            cheats_used_marker: &mut ch,
            ever_creative_marker: &mut ev,
            pure_survival_broken_marker: &mut ps,
        };
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let r = cmd.execute(&mut ctx, &owned);
        (r, ch, ps)
    }

    #[test]
    fn gives_stone() {
        let (r, players, _) = run(&["stone", "5"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        assert_eq!(slot.count, 5);
    }

    #[test]
    fn give_breaks_pure_survival_like_clear() {
        // Engine audit F / owner decision P0.3 — /give must flag the integrity
        // ledger (cheats_used + pure_survival_broken), not just cheats_used.
        let (r, cheats_used, pure_survival_broken) = run_markers(&["diamond_block", "64"]);
        assert_eq!(r, CommandResult::Success);
        assert!(cheats_used, "/give marks cheats_used");
        assert!(pure_survival_broken, "/give breaks pure-survival, like /clear");
    }

    #[test]
    fn give_failure_does_not_flag_the_ledger() {
        // An invalid item must not mark the ledger (no items actually granted).
        let (r, cheats_used, pure_survival_broken) = run_markers(&["banana"]);
        assert!(matches!(r, CommandResult::Error(_)));
        assert!(!cheats_used && !pure_survival_broken, "failed /give leaves the ledger clean");
    }

    #[test]
    fn defaults_count_to_one() {
        let (r, players, _) = run(&["dirt"]);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(players[0].inventory.slot(0).unwrap().count, 1);
    }

    #[test]
    fn caps_count_at_stack_size() {
        let (r, players, lines) = run(&["stone", "999"]);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(players[0].inventory.slot(0).unwrap().count, 64);
        // The success message must tell the user we capped — silent capping
        // would leave them confused about why /give stone 999 only gave 64.
        assert!(
            lines.iter().any(|l| l.contains("capped from 999")),
            "expected capped message, got: {lines:?}"
        );
    }

    #[test]
    fn rejects_unknown_item() {
        let (r, _, _) = run(&["banana"]);
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn gives_tool() {
        let (r, players, _) = run(&["wooden_pickaxe"]);
        assert_eq!(r, CommandResult::Success);
        // Tools land somewhere in inventory; verify at least one slot has a tool.
        let has_tool = (0..36).any(|i| {
            players[0].inventory.slot(i)
                .map(|s| matches!(s.item, crate::item::Item::Tool(_)))
                .unwrap_or(false)
        });
        assert!(has_tool, "tool not added to inventory");
    }

    #[test]
    fn rejects_zero_count() {
        let (r, _, _) = run(&["stone", "0"]);
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn rejects_garbage_count() {
        let (r, _, _) = run(&["stone", "many"]);
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn handles_aliases() {
        let (r, players, _) = run(&["log", "1"]);
        assert_eq!(r, CommandResult::Success);
        // Should give oak_log
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Block(id) => assert_eq!(*id, block::OAK_LOG),
            _ => panic!("expected block"),
        }
    }

    #[test]
    fn no_args_errors() {
        let (r, _, _) = run(&[]);
        assert!(matches!(r, CommandResult::Error(_)));
    }

    // --- Farming Tier 1 (Wave 26) /give parsing ---

    #[test]
    fn give_tilled_soil_works() {
        let (r, players, _) = run(&["tilled_soil", "4"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Block(id) => assert_eq!(*id, block::TILLED_SOIL),
            _ => panic!("expected tilled-soil block"),
        }
        assert_eq!(slot.count, 4);
    }

    #[test]
    fn give_seeds_works() {
        let (r, players, _) = run(&["wheat_seeds", "16"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Material(m) => {
                assert!(matches!(m, crate::item::MaterialId::WheatSeeds));
            }
            _ => panic!("expected wheat seeds"),
        }
        assert_eq!(slot.count, 16);
    }

    #[test]
    fn give_raw_fish_and_berries_work() {
        // Task 19 — "A Friend in Need" trial's kit needs these to resolve
        // (cat's tame food is raw fish, fox's is berries).
        let (r, players, _) = run(&["raw_fish", "6"]);
        assert_eq!(r, CommandResult::Success);
        match &players[0].inventory.slot(0).unwrap().item {
            crate::item::Item::Material(m) => assert!(matches!(m, crate::item::MaterialId::RawFish)),
            _ => panic!("expected raw fish"),
        }
        let (r, players, _) = run(&["berries", "8"]);
        assert_eq!(r, CommandResult::Success);
        match &players[0].inventory.slot(0).unwrap().item {
            crate::item::Item::Material(m) => assert!(matches!(m, crate::item::MaterialId::Berries)),
            _ => panic!("expected berries"),
        }
    }

    #[test]
    fn give_bread_works() {
        let (r, players, _) = run(&["bread", "3"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Material(m) => {
                assert!(matches!(m, crate::item::MaterialId::Bread));
            }
            _ => panic!("expected bread"),
        }
    }

    #[test]
    fn give_satori_hoe_works() {
        let (r, players, _) = run(&["satori_hoe"]);
        assert_eq!(r, CommandResult::Success);
        let has_hoe = (0..36).any(|i| {
            players[0].inventory.slot(i).map(|s| match &s.item {
                crate::item::Item::Tool(t) => {
                    t.tool_type == crate::crafting::ToolType::Hoe
                        && t.material == crate::crafting::ToolMaterial::Satori
                }
                _ => false,
            }).unwrap_or(false)
        });
        assert!(has_hoe, "Satori Hoe not in inventory");
    }

    #[test]
    fn give_all_five_hoe_tiers_works() {
        for tier_name in ["wooden_hoe", "stone_hoe", "iron_hoe", "diamond_hoe", "satori_hoe"] {
            let (r, _, _) = run(&[tier_name]);
            assert_eq!(r, CommandResult::Success, "failed to give {tier_name}");
        }
    }

    #[test]
    fn give_mature_crop_block_works() {
        let (r, players, _) = run(&["wheat_stage_3", "1"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Block(id) => assert_eq!(*id, block::WHEAT_STAGE_3),
            _ => panic!("expected wheat_stage_3 block"),
        }
    }

    // --- Wave 27 campfire / flint / flint-and-steel ---

    #[test]
    fn give_campfire_works_and_is_unlit_by_default() {
        // `/give campfire` must produce the UNLIT variant (player must
        // fuel + ignite). `/give campfire_lit` is the test-only variant.
        let (r, players, _) = run(&["campfire", "2"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Block(id) => assert_eq!(*id, block::CAMPFIRE_UNLIT),
            _ => panic!("expected unlit campfire"),
        }
        assert_eq!(slot.count, 2);
    }

    #[test]
    fn give_campfire_lit_alias_returns_lit_block() {
        let (r, players, _) = run(&["campfire_lit"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Block(id) => assert_eq!(*id, block::CAMPFIRE),
            _ => panic!("expected lit campfire"),
        }
    }

    #[test]
    fn give_flint_works() {
        let (r, players, _) = run(&["flint", "5"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Material(m) => {
                assert!(matches!(m, crate::item::MaterialId::Flint));
            }
            _ => panic!("expected flint material"),
        }
        assert_eq!(slot.count, 5);
    }

    #[test]
    fn give_flint_and_steel_works() {
        // Multi-word tool name; tested via the bow_by_name parser path.
        let (r, players, _) = run(&["flint_and_steel"]);
        assert_eq!(r, CommandResult::Success);
        let has_fas = (0..36).any(|i| {
            players[0].inventory.slot(i).map(|s| match &s.item {
                crate::item::Item::Tool(t) => {
                    t.tool_type == crate::crafting::ToolType::FlintAndSteel
                        && t.durability == crate::crafting::FLINT_AND_STEEL_DURABILITY
                }
                _ => false,
            }).unwrap_or(false)
        });
        assert!(has_fas, "Flint and Steel not in inventory at full durability");
    }

    #[test]
    fn give_leaves_works() {
        let (r, players, _) = run(&["leaves", "10"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Block(id) => assert_eq!(*id, block::OAK_LEAVES),
            _ => panic!("expected oak leaves block"),
        }
        assert_eq!(slot.count, 10);
    }

    // --- Spec 23 — Papyrus Reed / Sheet aliases ---

    #[test]
    fn give_papyrus_reed_works() {
        for alias in &["papyrus_reed", "papyrus", "reed"] {
            let (r, players, _) = run(&[alias, "3"]);
            assert_eq!(r, CommandResult::Success, "alias '{alias}' failed");
            let slot = players[0].inventory.slot(0).unwrap();
            match &slot.item {
                crate::item::Item::Material(crate::item::MaterialId::PapyrusReed) => {}
                _ => panic!("alias '{alias}' did not resolve to PapyrusReed"),
            }
            assert_eq!(slot.count, 3);
        }
    }

    #[test]
    fn give_papyrus_sheet_works() {
        // Canonical + every alias including `paper` (since PapyrusSheet
        // is the first paper material in the engine).
        for alias in &["papyrus_sheet", "papyrussheet", "papyrus_paper", "paper"] {
            let (r, players, _) = run(&[alias, "5"]);
            assert_eq!(r, CommandResult::Success, "alias '{alias}' failed");
            let slot = players[0].inventory.slot(0).unwrap();
            match &slot.item {
                crate::item::Item::Material(crate::item::MaterialId::PapyrusSheet) => {}
                _ => panic!("alias '{alias}' did not resolve to PapyrusSheet"),
            }
            assert_eq!(slot.count, 5);
        }
    }

    #[test]
    fn give_papyrus_stage_blocks_work() {
        for (alias, expected) in [
            ("papyrus_stage_0", block::PAPYRUS_STAGE_0),
            ("papyrus_sprout", block::PAPYRUS_STAGE_0),
            ("papyrus_stage_1", block::PAPYRUS_STAGE_1),
            ("papyrus_stage_2", block::PAPYRUS_STAGE_2),
            ("papyrus_stage_3", block::PAPYRUS_STAGE_3),
            ("papyrus_mature", block::PAPYRUS_STAGE_3),
        ] {
            let (r, players, _) = run(&[alias]);
            assert_eq!(r, CommandResult::Success, "alias '{alias}' failed");
            let slot = players[0].inventory.slot(0).unwrap();
            match &slot.item {
                crate::item::Item::Block(id) => assert_eq!(*id, expected, "alias '{alias}'"),
                _ => panic!("alias '{alias}' did not resolve to a block"),
            }
        }
    }

    #[test]
    fn gives_coal_material() {
        let (r, players, _) = run(&["coal", "10"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        assert_eq!(slot.count, 10);
        match &slot.item {
            crate::item::Item::Material(crate::item::MaterialId::Coal) => {}
            _ => panic!("expected Material(Coal)"),
        }
    }

    #[test]
    fn gives_diamond_material_default_count() {
        let (r, players, _) = run(&["diamond"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        assert_eq!(slot.count, 1);
        match &slot.item {
            crate::item::Item::Material(crate::item::MaterialId::Diamond) => {}
            _ => panic!("expected Material(Diamond)"),
        }
    }

    #[test]
    fn historical_naming_pass_give_aliases() {
        // Old aliases still work.
        assert_eq!(block_by_name("blueprint_paper"), Some(block::BLUEPRINT_PAPER));
        assert_eq!(block_by_name("vendor_block"), Some(block::VENDOR_BLOCK));
        assert_eq!(block_by_name("hay_bale"), Some(block::HAY_BALE));
        // New historical aliases also work.
        assert_eq!(block_by_name("plan_scroll"), Some(block::BLUEPRINT_PAPER));
        assert_eq!(block_by_name("market_stall"), Some(block::VENDOR_BLOCK));
        assert_eq!(block_by_name("hay_rick"), Some(block::HAY_BALE));
        assert_eq!(block_by_name("bone_cairn"), Some(block::BONE_BLOCK));
        assert_eq!(block_by_name("amethyst_cluster"), Some(block::AMETHYST_BLOCK));
        assert_eq!(block_by_name("sugar_cane"), Some(block::SUGARCANE));
        assert_eq!(block_by_name("foundation_stone"), Some(block::CONSTRUCTION_ANCHOR));
        assert_eq!(block_by_name("masons_mark"), Some(block::ARCHITECT_PLAQUE));
        assert_eq!(block_by_name("drafting_bench"), Some(block::DRAFTING_TABLE));
        // Material aliases (old + new).
        assert_eq!(material_by_name("bonemeal"), Some(crate::item::MaterialId::Bonemeal));
        assert_eq!(material_by_name("bone_meal"), Some(crate::item::MaterialId::Bonemeal));
        assert_eq!(material_by_name("honey_bottle"), Some(crate::item::MaterialId::HoneyBottle));
        assert_eq!(material_by_name("honey_jar"), Some(crate::item::MaterialId::HoneyBottle));
    }

    #[test]
    fn trials_kit_items_added_to_give_resolve() {
        // Items added so Trials challenge kits (Smith / Engineer / Angler /
        // Farmhand) can hand them out. A regression here ships a broken kit.
        assert_eq!(block_by_name("furnace"), Some(block::FURNACE));
        assert_eq!(block_by_name("furnace_lit"), Some(block::FURNACE_LIT));
        assert_eq!(block_by_name("piston"), Some(block::PISTON));
        assert_eq!(block_by_name("sticky_piston"), Some(block::STICKY_PISTON));
        assert_eq!(bow_by_name("fishing_rod"), Some((ToolType::FishingRod, ToolMaterial::Wood)));
        assert_eq!(bow_by_name("shears"), Some((ToolType::Shears, ToolMaterial::Iron)));
        // End-to-end through the public resolver too.
        assert!(resolve_item("furnace", 1).is_ok());
        assert!(resolve_item("fishing_rod", 1).is_ok());
        assert!(resolve_item("shears", 1).is_ok());
    }

    #[test]
    fn material_aliases_work() {
        // raw_iron / iron / rawiron should all resolve to RawIron.
        for alias in &["iron", "raw_iron", "rawiron"] {
            let (r, players, _) = run(&[alias, "1"]);
            assert_eq!(r, CommandResult::Success, "alias '{alias}' failed");
            let slot = players[0].inventory.slot(0).unwrap();
            match &slot.item {
                crate::item::Item::Material(crate::item::MaterialId::RawIron) => {}
                _ => panic!("alias '{alias}' did not resolve to RawIron"),
            }
        }
    }

    #[test]
    fn copper_aliases_work() {
        // Wind, Copper & Electricity wave (2026-09-07) — `copper`/`copper_ore`
        // give the ore BLOCK (block_by_name wins over material_by_name),
        // `raw_copper`/`rawcopper` give the smeltable material.
        for alias in &["copper", "copper_ore"] {
            assert_eq!(block_by_name(alias), Some(block::COPPER_ORE), "alias '{alias}'");
        }
        for alias in &["raw_copper", "rawcopper"] {
            assert_eq!(
                material_by_name(alias),
                Some(crate::item::MaterialId::Copper),
                "alias '{alias}'"
            );
        }
        assert_eq!(
            material_by_name("copper_ingot"),
            Some(crate::item::MaterialId::CopperIngot)
        );
        assert_eq!(
            material_by_name("copperingot"),
            Some(crate::item::MaterialId::CopperIngot)
        );
        // End-to-end through the public resolver too.
        for alias in &["copper", "copper_ore", "raw_copper", "copper_ingot"] {
            assert!(resolve_item(alias, 1).is_ok(), "resolve_item('{alias}') failed");
        }
    }

    #[test]
    fn cart_item_aliases_resolve() {
        // Craftable Armoured Carts (CA2) — `/give iron_cart` etc. so testers
        // can spawn cart items before the survival recipes are unlocked.
        use crate::item::MaterialId as M;
        assert_eq!(material_by_name("wood_cart"), Some(M::WoodCart));
        assert_eq!(material_by_name("woodcart"), Some(M::WoodCart));
        assert_eq!(material_by_name("cart"), Some(M::WoodCart));
        assert_eq!(material_by_name("iron_cart"), Some(M::IronCart));
        assert_eq!(material_by_name("ironcart"), Some(M::IronCart));
        assert_eq!(material_by_name("diamond_cart"), Some(M::DiamondCart));
        assert_eq!(material_by_name("diamondcart"), Some(M::DiamondCart));
    }

    #[test]
    fn magnesium_family_aliases_resolve() {
        // Spec 37 — Magnesium mineral + its products had zero /give aliases,
        // making them impossible to test-spawn (docs/superpowers/specs/2026-07-09).
        use crate::item::MaterialId as M;
        assert_eq!(material_by_name("magnesium"), Some(M::Magnesium));
        assert_eq!(material_by_name("fertiliser"), Some(M::Fertiliser));
        assert_eq!(material_by_name("fertilizer"), Some(M::Fertiliser));
        assert_eq!(material_by_name("sparkler"), Some(M::Sparkler));
        assert_eq!(material_by_name("flare"), Some(M::Flare));
        assert_eq!(material_by_name("magnesium_firestarter"), Some(M::MagnesiumFirestarter));
        assert_eq!(material_by_name("firestarter"), Some(M::MagnesiumFirestarter));
        assert_eq!(material_by_name("fire_starter"), Some(M::MagnesiumFirestarter));
    }
}
