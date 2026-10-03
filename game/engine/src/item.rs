//! Unified item system — blocks, tools, and future item types share one type.
//!
//! Every slot in the inventory holds an `ItemStack` which contains an `Item`.
//! Items can be blocks (placeable), tools (have durability), or other types
//! (food, armour, etc. — added later).

use serde::{Deserialize, Serialize};

use crate::block::BlockId;
use crate::crafting::Tool;

/// Simple material item IDs (non-block, non-tool items like sticks).
///
/// Adding a material:
/// 1. Add the variant here.
/// 2. Add display name in `Item::name`.
/// 3. Add hotbar colour in `Item::color`.
/// 4. (Optional) Add to a mob's drop table in `mob::drops_for`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MaterialId {
    Stick,
    // Mob drops — tangible items (Wave 2)
    Leather,
    Feather,
    Wool,
    Bone,
    // Mob drops — food (Wave 2 drop, Wave 4 makes them edible)
    RawBeef,
    RawPorkchop,
    RawChicken,
    RawMutton,
    // Cord material — sourced via the Wool → String recipe.
    String,
    // Crafted from bone (Wave 14). No mechanical use yet — placeholder for
    // future growth / dye systems. Crafting recipe: 1 Bone → 3 Bonemeal.
    Bonemeal,
    // Mined from ore blocks (Wave 13). Currently inventory-only; smelting +
    // tool-crafting integration land in later waves.
    Coal,
    RawIron,
    Diamond,
    // Smelting outputs (Wave 6).
    IronIngot,
    CookedBeef,
    CookedPorkchop,
    CookedChicken,
    CookedMutton,
    // Ranged combat (Wave 23). Crafted from sticks + feathers.
    Arrow,
    // Satori (Spec 5 §3.8 / Spec 6 §2.2c). The orange Bitcoin gem —
    // drops from pure deepslate at depth via deterministic vein algorithm;
    // in-world manifestation of Bitcoin sats on Bitcoin-enabled servers;
    // craftable into top-tier tools above the diamond tier. Mass-noun
    // singular/plural.
    Satori,
    // Farming Tier 1 (Wave 26, 2026-05-17). Appended to preserve bincode
    // variant indices for the items above. WheatSeeds is the canonical
    // separate-seed pattern (Minecraft wheat works this way); Carrot and
    // Potato double as both seed-and-food (plant the carrot directly;
    // eating one consumes it). Bread is the crafted food output of
    // 3 wheat → 1 bread.
    WheatSeeds,
    Wheat,
    Bread,
    Carrot,
    Potato,
    // Campfire (Wave 27, 2026-05-18). Dropped from gravel at 15%; the
    // head ingredient for flint-and-steel. Appended after the Wave 26
    // farming materials to preserve bincode indices.
    Flint,
    // Campfire extensions (Wave 28, 2026-05-18). Corn is a new crop that
    // matches the Carrot/Potato seed-and-food pattern but with a
    // separate-seed variant (CornSeeds drops alongside the Corn ear on
    // harvest, like Wheat). Baked variants are the campfire-cooked forms;
    // BakedCorn displays as "Corn on the Cob". Appended after Flint to
    // preserve bincode indices.
    CornSeeds,
    Corn,
    BakedCorn,
    BakedPotato,
    BakedCarrot,
    // Log Seasoning (Wave 29, 2026-05-19). Three-tier wood economy. Tree
    // mines drop GreenLog; the Drying Rack workstation seasons them into
    // SeasonedLog over ~5 real minutes; KilnDriedLog is the future Kiln
    // output (defined now for the fuel-value table; produced post-Spec 20).
    // Appended after Wave 28 baked-veg materials to preserve bincode
    // variant indices.
    GreenLog,
    SeasonedLog,
    KilnDriedLog,
    // Papyrus Reed (Spec 23 — Foundation A of Build Schematics, 2026-05-19).
    // PapyrusReed is the harvested + plantable material — placed on a
    // water-adjacent sand/dirt/grass tile sprouts a PAPYRUS_STAGE_0 block.
    // PapyrusSheet is the crafted paper (3 reeds horizontal → 3 sheets),
    // first paper material in the engine. Blueprint Paper (Spec 24) consumes
    // it via the new `is_paperish_slot` predicate. Appended after the
    // Wave 29 log materials to preserve bincode variant indices.
    PapyrusReed,
    PapyrusSheet,
    // Spec 28c Materials Expansion (2026-05-20). Appended bincode-positionally
    // so existing saves keep their indices. Sources: stone-variant mining
    // (no material — stone variants drop themselves as blocks), Copper Ore
    // (Copper raw → CopperIngot via furnace), and crafted alloys.
    Copper,
    Tin,
    Sulphur,
    Amethyst,
    CopperIngot,
    TinIngot,
    /// Bronze alloy (Copper + Tin smelt). Decorative on alpha — does NOT
    /// participate in the tool tier ladder (Wood/Stone/Iron/Diamond/Satori).
    BronzeIngot,
    Sugar,
    // Spec T1.5 Processed Economy Base (2026-05-14 spec, 2026-05-20 data
    // layer landed). Appended bincode-positionally. All inert at the
    // material-data layer; live crop growth + workstation interactions
    // are deferred to the playtest-gated phases. Live consumers will
    // arrive in follow-up PRs that bring up the Mill / Oven / Aging Rack.
    Bucket,
    MilkBucket,
    Egg,
    Flour,
    Dough,
    Cream,
    Butter,
    Cheese,
    SweetBread,
    Cake,
    PumpkinPie,
    BerryPie,
    Cookie,
    Pancakes,
    LoadedBakedPotato,
    Stew,
    BeetrootSoup,
    Bowl,
    /// Food variant of pumpkin (distinct from the Pumpkin block).
    /// Eaten raw at low value, baked into Pumpkin Pie at the Oven.
    PumpkinFood,
    SugarBeet,
    SugarBeetSeeds,
    Beetroot,
    BeetrootSeeds,
    Berries,
    // Spec 28b Wood Species (2026-05-20). One sapling per species — placed
    // on grass/dirt sprouts a future SAPLING_STAGE_0 block; on alpha,
    // saplings exist as inventory-only items pending live tree-gen.
    // Appended bincode-positionally.
    OakSapling,
    BirchSapling,
    SpruceSapling,
    JungleSapling,
    AcaciaSapling,
    DarkOakSapling,
    // Spec 28c Phase 5 (2026-05-20). Mob-drop materials planted in
    // advance of 28d Mobs — variants exist so future drops_for wiring
    // is one-line per mob. None of these are produced by anything in
    // the engine today; reachable only via /give in creative.
    Honeycomb,
    Honey,
    InkSac,
    // DEPRECATED 2026-05-22: historical pivot — no medieval analog
    // (bioluminescent fruit isn't real). Variant stays declared per
    // positional bincode rule; retained in inventory_explorer::ALL_MATERIAL_IDS
    // for save-compat visibility per HP-0 spec resolution of Open Question 1.
    GlowBerry,
    // Spec 28d.rabbit (chunk 3 of the 2026-05-21 rolling plan). Rabbit
    // drops. Appended bincode-positionally. Wired from Rabbit mob's
    // drop table in `mob::drops_for`; cooked variant produced by the
    // Furnace 10s recipe (RawRabbit → CookedRabbit). RabbitHide is a
    // decorative drop today; future Leather-equivalent recipes layer
    // on later.
    RawRabbit,
    CookedRabbit,
    RabbitHide,
    // Spec 28d chunk 5 — Bee + Hive primitive. HoneyBottle is the
    // drinkable food (Hive right-click with a Bucket-equivalent).
    // Stinger is dropped on a bee's sting impact; mostly decorative
    // for the alpha but reserved so the bee's despawn path has a
    // material identity. Appended bincode-positionally.
    HoneyBottle,
    BeeStinger,
    // Spec 28d.nostrich — Nostrich drops + recipe ingredients.
    // Appended bincode-positionally for save-compat.
    /// Purple ostrich feather. Sheds passively from tamed Nostriches
    /// every ~48,000 ticks; also drops 1-3 on kill (with curse).
    /// Premium arrow + banner ingredient; high trade-value commodity.
    NostrichFeather,
    /// One Nostrich Egg ≈ 21 chicken eggs by recipe yield (the
    /// Bitcoin nod). Stack-size 1. Tamed Nostriches lay one every
    /// 24,000 ticks (1 in-game day). Unlocks T5-tier premium recipes.
    NostrichEgg,
    /// Raw nostrich meat. Drops 0-1 on kill. Eating it triggers the
    /// Nostrich's Vow (curse) and only heals 1 hunger — it's tough
    /// and sour. Cooking it doesn't fix the vow trigger.
    RawNostrichMeat,
    /// Royal Pavlova — a T5-tier dessert recipe. Trade-value 120.
    RoyalPavlova,
    /// Nostrich Omelette — a T4-tier hearty breakfast. Trade-value 80.
    NostrichOmelette,
    /// Nostrich Custard — a T4-tier dessert. Trade-value 70.
    NostrichCustard,
    /// Nostrich-fletched arrow — +20% range, +20% damage vs regular
    /// arrow. Crafted from stick + flint + NostrichFeather.
    NostrichArrow,
    /// Purple banner — decorative wall-hanging crafted from cloth +
    /// NostrichFeather. Trade-value 25.
    PurpleBanner,
    // Spec HP-1 (2026-05-22). One drop per cleared Brigand Hideout —
    // generation lives in Sub 3. Trophy slot for the bandit-chieftain.
    BrigandChieftainTrophy,
    // Salt feature (2026-05-23). Mined from ROCK_SALT (1-2 per strike).
    // Crafting ingredient — not directly edible. See spec
    // docs/foundations/2026-05-23-salt.md.
    Salt,
    // Salt-Cured raw meats — end products (NOT further cookable).
    // +1 food_value and +1 trade_value over the raw input.
    SaltCuredBeef,
    SaltCuredPorkchop,
    SaltCuredMutton,
    SaltCuredChicken,
    SaltCuredRabbit,
    SaltCuredNostrichMeat,
    // Seasoned cooked staples — +1 food_value + +1 trade_value over the
    // cooked input. Bread/BakedPotato/BakedCarrot/BakedCorn + 5 cooked
    // meats. (Nostrich has NostrichOmelette, not a Cooked variant.)
    SeasonedBread,
    SeasonedBakedPotato,
    SeasonedBakedCarrot,
    SeasonedBakedCorn,
    SeasonedCookedBeef,
    SeasonedCookedPorkchop,
    SeasonedCookedMutton,
    SeasonedCookedChicken,
    SeasonedCookedRabbit,
    // Rubber feature (2026-05-23). See docs/foundations/2026-05-23-rubber.md.
    // Rubber = raw drop from tapping; RubberSapling plants the tree;
    // RubberBall is slingshot ammo; CopperCable is the data-laydown
    // material for the future electricity foundation spec.
    Rubber,
    RubberSapling,
    RubberBall,
    CopperCable,
    /// Mob Bounty Board (Spec 33, 2026-05-23) — the placeable form.
    /// Crafted via 8 planks ring + 1 IronIngot; placed by right-click
    /// (consumed); mining the placed board returns this material.
    BountyBoardItem,
    /// Tip Jar (Spec 34, 2026-05-23) — the placeable form. Crafted
    /// via 8 planks ring + 2 IronIngot stacked at centre; placed by
    /// right-click (consumed). Mining the placed jar by its owner
    /// returns this material; non-owner break is refused (anti-grief).
    TipJarItem,
    /// Repair Bench (Spec 35, 2026-05-23) — the placeable form.
    /// Crafted via 3 IronIngot top + 1 Stone centre + 3 Stone bottom.
    /// Stateless station; mining it returns this material.
    RepairBenchItem,
    /// Plot Marker (Spec 36, 2026-05-23) — the placeable form.
    /// Crafted via 4 IronIngot corners + 1 OAK_PLANKS centre.
    /// Placing claims a 32×32 plot; mining (as owner) releases it.
    PlotMarkerItem,
    /// Market Bell (Spec 37, 2026-05-23) — the placeable form.
    /// Crafted via iron / iron / plank column. Placing designates a
    /// Market Hub; mining (as owner) releases it.
    MarketBellItem,
    /// Auction Block (Spec 38, 2026-05-23) — the placeable form.
    /// Crafted via 4 IronIngot corners + 5 OAK_PLANKS. Placing
    /// creates an un-configured auction owned by the placer.
    AuctionBlockItem,
    /// Bazaar Block (Spec 39, 2026-05-23) — the placeable form.
    /// Crafted via 8 OAK_PLANKS ring + 1 Diamond centre. Stateless
    /// server-run sell-floor; mining returns this material.
    BazaarBlockItem,
    // Dyes (Spec 35, 2026-05-27). Primary dyes; 1 flower → 1 dye.
    // Black/White + mixing land in Phase 2.
    BlueDye,
    RedDye,
    YellowDye,
    // Fibre & cordage (Spec 36, 2026-05-27). Cotton → String (the honest
    // String source, replacing the Wool→String patch); Hemp Fibre → Rope.
    Cotton,
    HempFibre,
    Rope,
    // Magnesium (Spec 37, 2026-05-27). Mineral + its products.
    Magnesium,
    Fertiliser,           // Magnesium + Sulphur (Epsom salt) — crop-growth boost
    Sparkler,             // Magnesium + Stick — handheld bright sparkle
    Flare,                // Magnesium + Papyrus Sheet — bright signal light
    MagnesiumFirestarter, // Magnesium + Iron — lights campfires
    // Dye Phase 2 (Spec 35, 2026-05-27). Black/White from Ink Sac / Bone Meal;
    // secondaries + tints/shades via mixing. (Brown/Cyan/Magenta — needing
    // 3-input mixes — are a follow-on.)
    BlackDye,
    WhiteDye,
    OrangeDye,
    GreenDye,
    PurpleDye,
    PinkDye,
    LimeDye,
    LightBlueDye,
    GreyDye,
    LightGreyDye,
    // Fibre Phase 2 (Spec 36, 2026-05-27). Seeds for the farmable crops.
    CottonSeeds,
    HempSeeds,
    // Dye Phase 2 completion (Spec 35, 2026-05-28) — the three colours
    // that need 3-input mixes. Brown is the R+Y+B trio; Cyan is the
    // G+B pair (separate from Y+B → Green); Magenta is Purple+Pink
    // (with R+B+W as the secondary path per the spec's flower-mix
    // section).
    BrownDye,
    CyanDye,
    MagentaDye,
    // Spec 36 Phase 2 (2026-05-28) — Rope's first consumer + the fine
    // textile + sailcloth pair that close out the fibre triangle's
    // downstream slots. Lead = Rope + String (tether mechanic deferred
    // to v2; v1 is inventory-only); Cloth = 4 Cotton compact (fine
    // textile, bags/banners downstream); Canvas = 4 Hemp Fibre compact
    // (coarse sailcloth, future tents/sails/banners).
    Lead,
    Cloth,
    Canvas,
    // Spec 35 farmable-flower follow-on (2026-05-28). Seeds for the
    // three primary dye flowers — wheat-style self-sustaining: a mature
    // flower drops 1 flower-block + 1-2 seeds, the seeds plant the
    // matching crop-stage block on tilled soil. Wild flowers (still
    // scattered by `place_vegetation`) bootstrap the player into
    // farming the first time they break one.
    CornflowerSeeds,
    FieldPoppySeeds,
    ButtercupSeeds,
    /// Spec 40 (The Workshop) — the bellows tool. Held in the Workshop to inflate
    /// (right-click) / deflate (sneak + right-click) the nearest redesign project's
    /// working copy. Render-/authoring-only; no survival recipe.
    Bellows,
    /// Craftable Armoured Carts (CA2) — the three cart-vehicle item tiers. Each
    /// is the inventory form of a cart with the matching [`crate::cart::Hull`]:
    /// `WoodCart` → `Hull::Wood`, `IronCart` → `Hull::Iron`, `DiamondCart` →
    /// `Hull::Diamond`. CA3 right-click-places one of these on a TRACK cell to
    /// spawn the cart entity; CA4 drops the matching item when a cart is broken.
    /// `crate::cart::cart_hull_for_item` / `Hull::item_id` own the mapping.
    WoodCart,
    IronCart,
    DiamondCart,
    /// Spec 49 (Explosives) — refined saltpetre (potassium nitrate). Mined from
    /// `NITRE_ORE` or aged in a Composter; the oxidiser leg of Black Powder.
    Saltpetre,
    /// Spec 49 (Explosives) — Black Powder: `Sulphur + Charcoal/Coal + Saltpetre`
    /// (the real ~75/15/10 gunpowder formula). Crafts the Blasting Keg, and is
    /// the propellant the Magnesium spec flagged as missing (unblocks fireworks).
    BlackPowder,
    /// Spec 49 (Explosives) — Compost: aged plant/food waste from the Composter.
    /// The farming output, and the matter that ages further into Saltpetre (the
    /// nitre-bed chemistry). Pulls the long-deferred farming Composter forward.
    Compost,
    /// P6 — Fishing. The raw catch; cook on a campfire → [`MaterialId::CookedFish`].
    RawFish,
    /// P6 — Fishing. The cooked catch (better food value than raw).
    CookedFish,
    /// Aquatic wave — a Shark's tooth. Dripped when the shark attacks prey
    /// (non-lethal) or dropped on kill. High-value trade good + crafting input
    /// (serrated weapons / tipped arrows — recipes are a follow-up).
    SharkTooth,
    /// Aquatic wave — bioluminescent ink from a Glow Squid. Crafting input for
    /// glow décor + bio-lamp light (recipes are a follow-up).
    GlowInk,
    /// Water bucket — an empty Bucket filled at a WATER source. Right-click an
    /// empty cell to place a water source; empties back to a plain Bucket. The
    /// other half of the Buckets MC-parity feature (with `LavaBucket`).
    WaterBucket,
    /// Lava bucket — an empty Bucket filled at a LAVA source. Right-click to
    /// place a lava source; empties back to a plain Bucket. Pour water beside it
    /// to make obsidian. Discriminant order is the save/wire encoding
    /// (`material as u16`), so new variants must stay at the end.
    LavaBucket,
    /// Pets wave Task 7 — Recall Whistle. Right-click teleports every pet/
    /// steed you own into a ring around you (perched parrots skipped — see
    /// `tameable::owned_pet_entities`). Not consumed on use. APPENDED LAST —
    /// new variants must stay at the end (discriminant order is the wire
    /// encoding).
    RecallWhistle,
    /// Pets wave Task 9 — Cat Treat. A fish biscuit (Raw Fish + Wheat)
    /// that guarantees taming a Cat on the first right-click, skipping the
    /// generic `tame_food` 1-in-3 roll (see the companion-tame block in
    /// `game_loop.rs`). APPENDED LAST — new variants must stay at the end
    /// (discriminant order is the wire encoding).
    CatTreat,
    /// Pets wave Task 13 — Crab Claw. Dropped by a killed Crab
    /// (`mob::drops_for`). Raw crafting input for the Reach Claw. APPENDED
    /// LAST — new variants must stay at the end (discriminant order is the
    /// wire encoding).
    CrabClaw,
    /// Pets wave Task 13 — Reach Claw. Crafted from a Crab Claw + 2 Sticks
    /// (1×3 vertical column); held in the hotbar it extends block/entity
    /// interaction reach by [`crate::REACH_CLAW_BONUS`] (see
    /// `GameState::effective_reach`). APPENDED LAST — new variants must stay
    /// at the end (discriminant order is the wire encoding).
    ReachClaw,
}

/// Recover a `MaterialId` from its positional bincode discriminant index.
///
/// This is the exact inverse of the `material as u16` cast used on the wire
/// (`inventory::item_to_ref` -> `ItemRef::Material(id as u16)`): the enum has
/// no explicit discriminants, so a plain `as u16` yields the declaration-order
/// index, and this match maps that index back to the variant. The arms are
/// written out explicitly (rather than indexing a table) so adding a variant
/// forces a corresponding arm here — the same save-compat discipline the
/// `// Bincode-positional` comments enforce elsewhere. Out-of-range indices
/// (e.g. a newer client sending a material this build doesn't know) return
/// `Err(value)` so callers can fall back gracefully.
impl TryFrom<u16> for MaterialId {
    type Error = u16;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => MaterialId::Stick,
            1 => MaterialId::Leather,
            2 => MaterialId::Feather,
            3 => MaterialId::Wool,
            4 => MaterialId::Bone,
            5 => MaterialId::RawBeef,
            6 => MaterialId::RawPorkchop,
            7 => MaterialId::RawChicken,
            8 => MaterialId::RawMutton,
            9 => MaterialId::String,
            10 => MaterialId::Bonemeal,
            11 => MaterialId::Coal,
            12 => MaterialId::RawIron,
            13 => MaterialId::Diamond,
            14 => MaterialId::IronIngot,
            15 => MaterialId::CookedBeef,
            16 => MaterialId::CookedPorkchop,
            17 => MaterialId::CookedChicken,
            18 => MaterialId::CookedMutton,
            19 => MaterialId::Arrow,
            20 => MaterialId::Satori,
            21 => MaterialId::WheatSeeds,
            22 => MaterialId::Wheat,
            23 => MaterialId::Bread,
            24 => MaterialId::Carrot,
            25 => MaterialId::Potato,
            26 => MaterialId::Flint,
            27 => MaterialId::CornSeeds,
            28 => MaterialId::Corn,
            29 => MaterialId::BakedCorn,
            30 => MaterialId::BakedPotato,
            31 => MaterialId::BakedCarrot,
            32 => MaterialId::GreenLog,
            33 => MaterialId::SeasonedLog,
            34 => MaterialId::KilnDriedLog,
            35 => MaterialId::PapyrusReed,
            36 => MaterialId::PapyrusSheet,
            37 => MaterialId::Copper,
            38 => MaterialId::Tin,
            39 => MaterialId::Sulphur,
            40 => MaterialId::Amethyst,
            41 => MaterialId::CopperIngot,
            42 => MaterialId::TinIngot,
            43 => MaterialId::BronzeIngot,
            44 => MaterialId::Sugar,
            45 => MaterialId::Bucket,
            46 => MaterialId::MilkBucket,
            47 => MaterialId::Egg,
            48 => MaterialId::Flour,
            49 => MaterialId::Dough,
            50 => MaterialId::Cream,
            51 => MaterialId::Butter,
            52 => MaterialId::Cheese,
            53 => MaterialId::SweetBread,
            54 => MaterialId::Cake,
            55 => MaterialId::PumpkinPie,
            56 => MaterialId::BerryPie,
            57 => MaterialId::Cookie,
            58 => MaterialId::Pancakes,
            59 => MaterialId::LoadedBakedPotato,
            60 => MaterialId::Stew,
            61 => MaterialId::BeetrootSoup,
            62 => MaterialId::Bowl,
            63 => MaterialId::PumpkinFood,
            64 => MaterialId::SugarBeet,
            65 => MaterialId::SugarBeetSeeds,
            66 => MaterialId::Beetroot,
            67 => MaterialId::BeetrootSeeds,
            68 => MaterialId::Berries,
            69 => MaterialId::OakSapling,
            70 => MaterialId::BirchSapling,
            71 => MaterialId::SpruceSapling,
            72 => MaterialId::JungleSapling,
            73 => MaterialId::AcaciaSapling,
            74 => MaterialId::DarkOakSapling,
            75 => MaterialId::Honeycomb,
            76 => MaterialId::Honey,
            77 => MaterialId::InkSac,
            78 => MaterialId::GlowBerry,
            79 => MaterialId::RawRabbit,
            80 => MaterialId::CookedRabbit,
            81 => MaterialId::RabbitHide,
            82 => MaterialId::HoneyBottle,
            83 => MaterialId::BeeStinger,
            84 => MaterialId::NostrichFeather,
            85 => MaterialId::NostrichEgg,
            86 => MaterialId::RawNostrichMeat,
            87 => MaterialId::RoyalPavlova,
            88 => MaterialId::NostrichOmelette,
            89 => MaterialId::NostrichCustard,
            90 => MaterialId::NostrichArrow,
            91 => MaterialId::PurpleBanner,
            92 => MaterialId::BrigandChieftainTrophy,
            93 => MaterialId::Salt,
            94 => MaterialId::SaltCuredBeef,
            95 => MaterialId::SaltCuredPorkchop,
            96 => MaterialId::SaltCuredMutton,
            97 => MaterialId::SaltCuredChicken,
            98 => MaterialId::SaltCuredRabbit,
            99 => MaterialId::SaltCuredNostrichMeat,
            100 => MaterialId::SeasonedBread,
            101 => MaterialId::SeasonedBakedPotato,
            102 => MaterialId::SeasonedBakedCarrot,
            103 => MaterialId::SeasonedBakedCorn,
            104 => MaterialId::SeasonedCookedBeef,
            105 => MaterialId::SeasonedCookedPorkchop,
            106 => MaterialId::SeasonedCookedMutton,
            107 => MaterialId::SeasonedCookedChicken,
            108 => MaterialId::SeasonedCookedRabbit,
            109 => MaterialId::Rubber,
            110 => MaterialId::RubberSapling,
            111 => MaterialId::RubberBall,
            112 => MaterialId::CopperCable,
            113 => MaterialId::BountyBoardItem,
            114 => MaterialId::TipJarItem,
            115 => MaterialId::RepairBenchItem,
            116 => MaterialId::PlotMarkerItem,
            117 => MaterialId::MarketBellItem,
            118 => MaterialId::AuctionBlockItem,
            119 => MaterialId::BazaarBlockItem,
            120 => MaterialId::BlueDye,
            121 => MaterialId::RedDye,
            122 => MaterialId::YellowDye,
            123 => MaterialId::Cotton,
            124 => MaterialId::HempFibre,
            125 => MaterialId::Rope,
            126 => MaterialId::Magnesium,
            127 => MaterialId::Fertiliser,
            128 => MaterialId::Sparkler,
            129 => MaterialId::Flare,
            130 => MaterialId::MagnesiumFirestarter,
            131 => MaterialId::BlackDye,
            132 => MaterialId::WhiteDye,
            133 => MaterialId::OrangeDye,
            134 => MaterialId::GreenDye,
            135 => MaterialId::PurpleDye,
            136 => MaterialId::PinkDye,
            137 => MaterialId::LimeDye,
            138 => MaterialId::LightBlueDye,
            139 => MaterialId::GreyDye,
            140 => MaterialId::LightGreyDye,
            141 => MaterialId::CottonSeeds,
            142 => MaterialId::HempSeeds,
            143 => MaterialId::BrownDye,
            144 => MaterialId::CyanDye,
            145 => MaterialId::MagentaDye,
            146 => MaterialId::Lead,
            147 => MaterialId::Cloth,
            148 => MaterialId::Canvas,
            149 => MaterialId::CornflowerSeeds,
            150 => MaterialId::FieldPoppySeeds,
            151 => MaterialId::ButtercupSeeds,
            // Spec 40 — Bellows. It sits at this discriminant in declaration
            // order; it was previously absent from this table (authoring-only,
            // never serialised) which left a hole that the identity round-trip
            // didn't catch until a later append exposed it. Now mapped so the
            // table stays a true inverse of the `as u16` discriminant.
            152 => MaterialId::Bellows,
            // Craftable Armoured Carts (CA2).
            153 => MaterialId::WoodCart,
            154 => MaterialId::IronCart,
            155 => MaterialId::DiamondCart,
            // Spec 49 (Explosives).
            156 => MaterialId::Saltpetre,
            157 => MaterialId::BlackPowder,
            158 => MaterialId::Compost,
            159 => MaterialId::RawFish,
            160 => MaterialId::CookedFish,
            161 => MaterialId::SharkTooth,
            162 => MaterialId::GlowInk,
            163 => MaterialId::WaterBucket,
            164 => MaterialId::LavaBucket,
            165 => MaterialId::RecallWhistle,
            166 => MaterialId::CatTreat,
            167 => MaterialId::CrabClaw,
            168 => MaterialId::ReachClaw,
            other => return Err(other),
        })
    }
}

/// A single item. This is the core type that everything in the inventory uses.
///
/// `PartialEq` (2026-07-06, Task 11) — needed so `ItemStack`/`ChestData` can
/// derive it in turn (`HorseData.pack: Option<ChestData>` embeds a chest, and
/// `SavedTamedPetData` derives `PartialEq` for its wire round-trip tests).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Item {
    /// A placeable block.
    Block(BlockId),
    /// A tool with durability.
    Tool(Tool),
    /// A crafting material (not placeable, not a tool).
    Material(MaterialId),
    /// A captured-building blueprint (Spec 24 — Foundation B of Build
    /// Schematics). Per-instance content; never stacks. Holds the
    /// plan's name, author, licence, derivation chain, and captured
    /// cells. See `plan::PlanData`.
    Plan(crate::plan::PlanData),
    /// Spec 28e — armour piece. Per-instance durability so each
    /// helmet/chestplate/etc. wears independently. Never stacks. See
    /// `armour::ArmourItem` for the slot + material + durability.
    Armour(crate::armour::ArmourItem),
}

impl Item {
    /// Display name for title bar and UI.
    pub fn name(&self, registry: &crate::block::BlockRegistry) -> String {
        match self {
            Item::Block(id) => {
                match *id {
                    crate::block::HAY_BALE => return "Hay Rick".to_string(),
                    crate::block::BONE_BLOCK => return "Bone Cairn".to_string(),
                    crate::block::AMETHYST_BLOCK => return "Amethyst Cluster".to_string(),
                    crate::block::SUGARCANE => return "Sugar Cane".to_string(),
                    crate::block::VENDOR_BLOCK => return "Market Stall".to_string(),
                    crate::block::BLUEPRINT_PAPER => return "Blueprint Paper".to_string(),
                    crate::block::CONSTRUCTION_ANCHOR => return "Foundation Stone".to_string(),
                    crate::block::ARCHITECT_PLAQUE => return "Mason's Mark".to_string(),
                    crate::block::DRAFTING_TABLE => return "Drafting Bench".to_string(),
                    crate::block::CRAFTING_TABLE => return "Workbench".to_string(),
                    _ => {}
                }
                let raw = registry.get(*id).name;
                // Strip the registry namespace, whatever it is. Block names are
                // `<ns>:<snake_case>` — `genesis:` for the core set,
                // `electricity:` for Spec 48's — and only the part after the
                // colon is player-facing. Stripping just `genesis:` left every
                // power block reading "Electricity:water Wheel" in the
                // inventory; matching on the colon covers the next namespace too.
                raw.split_once(':')
                    .map_or(raw, |(_ns, rest)| rest)
                    .split('_')
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            None => String::new(),
                            Some(f) => f.to_uppercase().to_string() + c.as_str(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            Item::Tool(t) => t.name().to_string(),
            Item::Plan(p) => p.name.clone(),
            Item::Armour(a) => crate::armour::armour_label(a.slot, a.material).to_string(),
            Item::Material(m) => match m {
                MaterialId::Bellows => "Bellows".to_string(),
                MaterialId::Stick => "Stick".to_string(),
                MaterialId::Leather => "Leather".to_string(),
                MaterialId::Feather => "Feather".to_string(),
                MaterialId::Wool => "Wool".to_string(),
                MaterialId::Bone => "Bone".to_string(),
                MaterialId::RawBeef => "Raw Beef".to_string(),
                MaterialId::RawPorkchop => "Raw Porkchop".to_string(),
                MaterialId::RawChicken => "Raw Chicken".to_string(),
                MaterialId::RawMutton => "Raw Mutton".to_string(),
                MaterialId::String => "String".to_string(),
                MaterialId::Bonemeal => "Bone Meal".to_string(),
                MaterialId::Coal => "Coal".to_string(),
                MaterialId::RawIron => "Raw Iron".to_string(),
                MaterialId::Diamond => "Diamond".to_string(),
                MaterialId::IronIngot => "Iron Ingot".to_string(),
                MaterialId::CookedBeef => "Cooked Beef".to_string(),
                MaterialId::CookedPorkchop => "Cooked Porkchop".to_string(),
                MaterialId::CookedChicken => "Cooked Chicken".to_string(),
                MaterialId::CookedMutton => "Cooked Mutton".to_string(),
                MaterialId::Arrow => "Arrow".to_string(),
                MaterialId::Satori => "Satori".to_string(),
                MaterialId::WheatSeeds => "Wheat Seeds".to_string(),
                MaterialId::Wheat => "Wheat".to_string(),
                MaterialId::Bread => "Bread".to_string(),
                MaterialId::Carrot => "Carrot".to_string(),
                MaterialId::Potato => "Potato".to_string(),
                MaterialId::Flint => "Flint".to_string(),
                MaterialId::CornSeeds => "Corn Seeds".to_string(),
                MaterialId::Corn => "Corn".to_string(),
                MaterialId::BakedCorn => "Corn on the Cob".to_string(),
                MaterialId::BakedPotato => "Baked Potato".to_string(),
                MaterialId::BakedCarrot => "Baked Carrot".to_string(),
                MaterialId::GreenLog => "Green Log".to_string(),
                MaterialId::SeasonedLog => "Seasoned Log".to_string(),
                MaterialId::KilnDriedLog => "Kiln-Dried Log".to_string(),
                MaterialId::PapyrusReed => "Papyrus Reed".to_string(),
                MaterialId::PapyrusSheet => "Papyrus Sheet".to_string(),
                MaterialId::Copper => "Copper".to_string(),
                MaterialId::Tin => "Tin".to_string(),
                MaterialId::Sulphur => "Sulphur".to_string(),
                MaterialId::Amethyst => "Amethyst".to_string(),
                MaterialId::CopperIngot => "Copper Ingot".to_string(),
                MaterialId::TinIngot => "Tin Ingot".to_string(),
                MaterialId::BronzeIngot => "Bronze Ingot".to_string(),
                MaterialId::Sugar => "Sugar".to_string(),
                MaterialId::Bucket => "Bucket".to_string(),
                MaterialId::MilkBucket => "Milk Bucket".to_string(),
                MaterialId::WaterBucket => "Water Bucket".to_string(),
                MaterialId::LavaBucket => "Lava Bucket".to_string(),
                MaterialId::RecallWhistle => "Recall Whistle".to_string(),
                MaterialId::CatTreat => "Cat Treat".to_string(),
                MaterialId::Egg => "Egg".to_string(),
                MaterialId::Flour => "Flour".to_string(),
                MaterialId::Dough => "Dough".to_string(),
                MaterialId::Cream => "Cream".to_string(),
                MaterialId::Butter => "Butter".to_string(),
                MaterialId::Cheese => "Cheese".to_string(),
                MaterialId::SweetBread => "Sweet Bread".to_string(),
                MaterialId::Cake => "Cake".to_string(),
                MaterialId::PumpkinPie => "Pumpkin Pie".to_string(),
                MaterialId::BerryPie => "Berry Pie".to_string(),
                MaterialId::Cookie => "Cookie".to_string(),
                MaterialId::Pancakes => "Pancakes".to_string(),
                MaterialId::LoadedBakedPotato => "Loaded Baked Potato".to_string(),
                MaterialId::Stew => "Stew".to_string(),
                MaterialId::BeetrootSoup => "Beetroot Soup".to_string(),
                MaterialId::Bowl => "Bowl".to_string(),
                MaterialId::PumpkinFood => "Pumpkin".to_string(),
                MaterialId::SugarBeet => "Sugar Beet".to_string(),
                MaterialId::SugarBeetSeeds => "Sugar Beet Seeds".to_string(),
                MaterialId::Beetroot => "Beetroot".to_string(),
                MaterialId::BeetrootSeeds => "Beetroot Seeds".to_string(),
                MaterialId::Berries => "Berries".to_string(),
                MaterialId::OakSapling => "Oak Sapling".to_string(),
                MaterialId::BirchSapling => "Birch Sapling".to_string(),
                MaterialId::SpruceSapling => "Spruce Sapling".to_string(),
                MaterialId::JungleSapling => "Jungle Sapling".to_string(),
                MaterialId::AcaciaSapling => "Acacia Sapling".to_string(),
                MaterialId::DarkOakSapling => "Dark Oak Sapling".to_string(),
                MaterialId::Honeycomb => "Honeycomb".to_string(),
                MaterialId::Honey => "Honey".to_string(),
                MaterialId::InkSac => "Ink Sac".to_string(),
                MaterialId::GlowBerry => "Glow Berry".to_string(),
                MaterialId::RawRabbit => "Raw Rabbit".to_string(),
                MaterialId::CookedRabbit => "Cooked Rabbit".to_string(),
                MaterialId::RabbitHide => "Rabbit Hide".to_string(),
                MaterialId::HoneyBottle => "Honey Jar".to_string(),
                MaterialId::BeeStinger => "Bee Stinger".to_string(),
                MaterialId::NostrichFeather => "Nostrich Feather".to_string(),
                MaterialId::NostrichEgg => "Nostrich Egg".to_string(),
                MaterialId::RawNostrichMeat => "Raw Nostrich Meat".to_string(),
                MaterialId::RoyalPavlova => "Royal Pavlova".to_string(),
                MaterialId::NostrichOmelette => "Nostrich Omelette".to_string(),
                MaterialId::NostrichCustard => "Nostrich Custard".to_string(),
                MaterialId::NostrichArrow => "Nostrich Arrow".to_string(),
                MaterialId::PurpleBanner => "Purple Banner".to_string(),
                MaterialId::BrigandChieftainTrophy => "Brigand Chieftain Trophy".to_string(),
                MaterialId::Salt => "Salt".to_string(),
                MaterialId::SaltCuredBeef => "Salt-Cured Beef".to_string(),
                MaterialId::SaltCuredPorkchop => "Salt-Cured Porkchop".to_string(),
                MaterialId::SaltCuredMutton => "Salt-Cured Mutton".to_string(),
                MaterialId::SaltCuredChicken => "Salt-Cured Chicken".to_string(),
                MaterialId::SaltCuredRabbit => "Salt-Cured Rabbit".to_string(),
                MaterialId::SaltCuredNostrichMeat => "Salt-Cured Nostrich Meat".to_string(),
                MaterialId::SeasonedBread => "Seasoned Bread".to_string(),
                MaterialId::SeasonedBakedPotato => "Seasoned Baked Potato".to_string(),
                MaterialId::SeasonedBakedCarrot => "Seasoned Baked Carrot".to_string(),
                MaterialId::SeasonedBakedCorn => "Seasoned Corn on the Cob".to_string(),
                MaterialId::SeasonedCookedBeef => "Seasoned Cooked Beef".to_string(),
                MaterialId::SeasonedCookedPorkchop => "Seasoned Cooked Porkchop".to_string(),
                MaterialId::SeasonedCookedMutton => "Seasoned Cooked Mutton".to_string(),
                MaterialId::SeasonedCookedChicken => "Seasoned Cooked Chicken".to_string(),
                MaterialId::SeasonedCookedRabbit => "Seasoned Cooked Rabbit".to_string(),
                MaterialId::Rubber => "Rubber".to_string(),
                MaterialId::RubberSapling => "Rubber Sapling".to_string(),
                MaterialId::RubberBall => "Rubber Ball".to_string(),
                MaterialId::CopperCable => "Copper Cable".to_string(),
                MaterialId::BountyBoardItem => "Bounty Board".to_string(),
                MaterialId::TipJarItem => "Tip Jar".to_string(),
                MaterialId::RepairBenchItem => "Repair Bench".to_string(),
                MaterialId::PlotMarkerItem => "Plot Marker".to_string(),
                MaterialId::MarketBellItem => "Market Bell".to_string(),
                MaterialId::AuctionBlockItem => "Auction Block".to_string(),
                MaterialId::BazaarBlockItem => "Bazaar".to_string(),
                MaterialId::BlueDye => "Blue Dye".to_string(),
                MaterialId::RedDye => "Red Dye".to_string(),
                MaterialId::YellowDye => "Yellow Dye".to_string(),
                MaterialId::Cotton => "Cotton".to_string(),
                MaterialId::HempFibre => "Hemp Fibre".to_string(),
                MaterialId::Rope => "Rope".to_string(),
                MaterialId::Magnesium => "Magnesium".to_string(),
                MaterialId::Fertiliser => "Fertiliser".to_string(),
                MaterialId::Sparkler => "Sparkler".to_string(),
                MaterialId::Flare => "Flare".to_string(),
                MaterialId::MagnesiumFirestarter => "Magnesium Firestarter".to_string(),
                MaterialId::BlackDye => "Black Dye".to_string(),
                MaterialId::WhiteDye => "White Dye".to_string(),
                MaterialId::OrangeDye => "Orange Dye".to_string(),
                MaterialId::GreenDye => "Green Dye".to_string(),
                MaterialId::PurpleDye => "Purple Dye".to_string(),
                MaterialId::PinkDye => "Pink Dye".to_string(),
                MaterialId::LimeDye => "Lime Dye".to_string(),
                MaterialId::LightBlueDye => "Light Blue Dye".to_string(),
                MaterialId::GreyDye => "Grey Dye".to_string(),
                MaterialId::LightGreyDye => "Light Grey Dye".to_string(),
                MaterialId::CottonSeeds => "Cotton Seeds".to_string(),
                MaterialId::HempSeeds => "Hemp Seeds".to_string(),
                MaterialId::BrownDye => "Brown Dye".to_string(),
                MaterialId::CyanDye => "Cyan Dye".to_string(),
                MaterialId::MagentaDye => "Magenta Dye".to_string(),
                MaterialId::Lead => "Lead".to_string(),
                MaterialId::Cloth => "Cloth".to_string(),
                MaterialId::Canvas => "Canvas".to_string(),
                MaterialId::CornflowerSeeds => "Cornflower Seeds".to_string(),
                MaterialId::FieldPoppySeeds => "Field Poppy Seeds".to_string(),
                MaterialId::ButtercupSeeds => "Buttercup Seeds".to_string(),
                // Craftable Armoured Carts (CA2).
                MaterialId::WoodCart => "Wood Cart".to_string(),
                MaterialId::IronCart => "Iron Cart".to_string(),
                MaterialId::DiamondCart => "Diamond Cart".to_string(),
                // Spec 49 (Explosives).
                MaterialId::Saltpetre => "Saltpetre".to_string(),
                MaterialId::BlackPowder => "Black Powder".to_string(),
                MaterialId::Compost => "Compost".to_string(),
                MaterialId::RawFish => "Raw Fish".to_string(),
                MaterialId::CookedFish => "Cooked Fish".to_string(),
                MaterialId::SharkTooth => "Shark Tooth".to_string(),
                MaterialId::GlowInk => "Glow Ink".to_string(),
                MaterialId::CrabClaw => "Crab Claw".to_string(),
                MaterialId::ReachClaw => "Reach Claw".to_string(),
            },
        }
    }

    /// Hotbar display colour.
    pub fn color(&self, registry: &crate::block::BlockRegistry) -> [f32; 3] {
        match self {
            Item::Block(id) => registry.color(*id),
            Item::Tool(t) => t.color(),
            // Spec 38 (Blueprint / Cyanotype) — Latent prints step
            // through 4 visible develop stages (pale-yellow → faint
            // blue-green → blue → deep blue) as `exposure_ticks`
            // climbs toward `DEVELOP_THRESHOLD_TICKS`; Developed prints
            // render full Prussian blueprint-blue. The 4-stage step is
            // the spec's "visibly deepening pale → faint blue → blue →
            // deep blueprint-blue" progression, so the player can read
            // develop progress from the inventory icon at a glance.
            Item::Plan(p) => crate::plan::develop_state_color(p.develop_state),
            // Armour pieces tinted by material — broadly matches the
            // tool tier colours but a touch darker.
            Item::Armour(a) => match a.material {
                crate::armour::ArmourMaterial::Leather => [0.55, 0.35, 0.18],
                crate::armour::ArmourMaterial::Iron => [0.75, 0.75, 0.78],
                crate::armour::ArmourMaterial::Diamond => [0.40, 0.92, 0.90],
                crate::armour::ArmourMaterial::Satori => [0.95, 0.55, 0.18],
                crate::armour::ArmourMaterial::Chainmail => [0.55, 0.55, 0.60],
                crate::armour::ArmourMaterial::Rubber => [0.42, 0.30, 0.22],
            },
            Item::Material(m) => match m {
                MaterialId::Bellows => [0.5, 0.42, 0.32],
                MaterialId::Stick => [0.6, 0.45, 0.2],
                MaterialId::Leather => [0.55, 0.35, 0.18],
                MaterialId::Feather => [0.95, 0.95, 0.92],
                MaterialId::Wool => [0.95, 0.95, 0.92],
                MaterialId::Bone => [0.92, 0.90, 0.78],
                MaterialId::RawBeef => [0.85, 0.32, 0.30],
                MaterialId::RawPorkchop => [0.95, 0.55, 0.55],
                MaterialId::RawChicken => [0.95, 0.78, 0.65],
                MaterialId::RawMutton => [0.85, 0.40, 0.35],
                MaterialId::String => [0.92, 0.92, 0.88],
                MaterialId::Bonemeal => [0.95, 0.95, 0.92],
                MaterialId::Coal => [0.10, 0.10, 0.10],
                MaterialId::RawIron => [0.75, 0.62, 0.50],
                MaterialId::Diamond => [0.40, 0.95, 0.92],
                MaterialId::IronIngot => [0.78, 0.78, 0.80],
                MaterialId::CookedBeef => [0.55, 0.30, 0.20],
                MaterialId::CookedPorkchop => [0.78, 0.55, 0.42],
                MaterialId::CookedChicken => [0.80, 0.65, 0.42],
                MaterialId::CookedMutton => [0.60, 0.30, 0.25],
                MaterialId::Arrow => [0.80, 0.78, 0.72],
                MaterialId::Satori => [0.95, 0.55, 0.18],
                // Farming Tier 1 — earthy / golden / baked palette.
                MaterialId::WheatSeeds => [0.85, 0.75, 0.45],
                MaterialId::Wheat => [0.95, 0.82, 0.35],
                MaterialId::Bread => [0.78, 0.55, 0.30],
                MaterialId::Carrot => [0.95, 0.55, 0.18],
                MaterialId::Potato => [0.85, 0.75, 0.55],
                // Cool grey shard.
                MaterialId::Flint => [0.45, 0.42, 0.40],
                // Corn — yellow kernels through the lifecycle, getting
                // golden-brown when baked.
                MaterialId::CornSeeds => [0.85, 0.75, 0.30],
                MaterialId::Corn => [0.95, 0.78, 0.25],
                MaterialId::BakedCorn => [0.95, 0.70, 0.20],
                MaterialId::BakedPotato => [0.68, 0.52, 0.30],
                MaterialId::BakedCarrot => [0.85, 0.45, 0.15],
                // Log Seasoning palette: green-cast → warm amber → charred-dark.
                MaterialId::GreenLog => [0.45, 0.52, 0.22],
                MaterialId::SeasonedLog => [0.55, 0.38, 0.18],
                MaterialId::KilnDriedLog => [0.38, 0.26, 0.14],
                // Papyrus Reed (Spec 23) — riverbank green-tan stalk for the
                // raw reed; warm parchment cream for the crafted sheet.
                MaterialId::PapyrusReed => [0.55, 0.68, 0.32],
                MaterialId::PapyrusSheet => [0.92, 0.86, 0.62],
                // Spec 28c. Raw ores warm-toned; smelted ingots lighter.
                MaterialId::Copper => [0.78, 0.45, 0.30],
                MaterialId::Tin => [0.85, 0.85, 0.92],
                MaterialId::Sulphur => [0.95, 0.92, 0.30],
                MaterialId::Amethyst => [0.55, 0.30, 0.75],
                MaterialId::CopperIngot => [0.85, 0.55, 0.40],
                MaterialId::TinIngot => [0.92, 0.92, 0.95],
                MaterialId::BronzeIngot => [0.72, 0.55, 0.30],
                MaterialId::Sugar => [0.95, 0.95, 0.95],
                // T1.5 palette — earthy/cream tones for processed foods.
                MaterialId::Bucket => [0.65, 0.65, 0.72],
                MaterialId::MilkBucket => [0.95, 0.95, 0.95],
                MaterialId::WaterBucket => [0.25, 0.45, 0.85],
                MaterialId::LavaBucket => [0.9, 0.4, 0.12],
                MaterialId::RecallWhistle => [0.90, 0.88, 0.76], // bone-ivory, like Bone
                MaterialId::CatTreat => [0.80, 0.60, 0.35], // warm biscuit-brown
                MaterialId::Egg => [0.92, 0.88, 0.78],
                MaterialId::Flour => [0.92, 0.88, 0.75],
                MaterialId::Dough => [0.85, 0.78, 0.62],
                MaterialId::Cream => [0.95, 0.92, 0.85],
                MaterialId::Butter => [0.95, 0.85, 0.40],
                MaterialId::Cheese => [0.95, 0.80, 0.30],
                MaterialId::SweetBread => [0.85, 0.65, 0.35],
                MaterialId::Cake => [0.95, 0.92, 0.85],
                MaterialId::PumpkinPie => [0.85, 0.45, 0.15],
                MaterialId::BerryPie => [0.65, 0.25, 0.35],
                MaterialId::Cookie => [0.78, 0.55, 0.30],
                MaterialId::Pancakes => [0.92, 0.75, 0.45],
                MaterialId::LoadedBakedPotato => [0.85, 0.75, 0.45],
                MaterialId::Stew => [0.55, 0.35, 0.20],
                MaterialId::BeetrootSoup => [0.65, 0.20, 0.30],
                MaterialId::Bowl => [0.60, 0.45, 0.25],
                MaterialId::PumpkinFood => [0.95, 0.55, 0.18],
                MaterialId::SugarBeet => [0.92, 0.92, 0.88],
                MaterialId::SugarBeetSeeds => [0.85, 0.85, 0.75],
                MaterialId::Beetroot => [0.55, 0.15, 0.20],
                MaterialId::BeetrootSeeds => [0.65, 0.55, 0.40],
                MaterialId::Berries => [0.55, 0.20, 0.35],
                // 28b — saplings tinted to match their species leaves.
                MaterialId::OakSapling => [0.40, 0.65, 0.20],
                MaterialId::BirchSapling => [0.55, 0.78, 0.45],
                MaterialId::SpruceSapling => [0.30, 0.50, 0.30],
                MaterialId::JungleSapling => [0.30, 0.85, 0.20],
                MaterialId::AcaciaSapling => [0.40, 0.65, 0.25],
                MaterialId::DarkOakSapling => [0.20, 0.45, 0.20],
                // 28c Phase 5 — mob drops; warm amber for the bee
                // products, deep blue for ink, soft orange for glow.
                MaterialId::Honeycomb => [0.85, 0.55, 0.20],
                MaterialId::Honey => [0.95, 0.65, 0.10],
                MaterialId::InkSac => [0.10, 0.10, 0.18],
                MaterialId::GlowBerry => [0.95, 0.75, 0.35],
                MaterialId::RawRabbit => [0.78, 0.55, 0.45],
                MaterialId::CookedRabbit => [0.62, 0.38, 0.22],
                MaterialId::RabbitHide => [0.70, 0.62, 0.52],
                MaterialId::HoneyBottle => [0.95, 0.65, 0.10],
                MaterialId::BeeStinger => [0.95, 0.85, 0.30],
                MaterialId::NostrichFeather => [0.55, 0.35, 0.75],
                MaterialId::NostrichEgg => [0.85, 0.78, 0.65],
                MaterialId::RawNostrichMeat => [0.55, 0.32, 0.55],
                MaterialId::RoyalPavlova => [0.96, 0.92, 0.85],
                MaterialId::NostrichOmelette => [0.92, 0.82, 0.45],
                MaterialId::NostrichCustard => [0.95, 0.88, 0.55],
                MaterialId::NostrichArrow => [0.65, 0.45, 0.80],
                MaterialId::PurpleBanner => [0.50, 0.30, 0.70],
                MaterialId::BrigandChieftainTrophy => [0.45, 0.10, 0.12],
                MaterialId::Salt => [0.96, 0.94, 0.92],
                // Cured raws — same hue as raw, slightly muted with
                // faint salt-crystal flecks via the sprite.
                MaterialId::SaltCuredBeef       => [0.78, 0.32, 0.30],
                MaterialId::SaltCuredPorkchop   => [0.85, 0.50, 0.50],
                MaterialId::SaltCuredMutton     => [0.72, 0.35, 0.35],
                MaterialId::SaltCuredChicken    => [0.92, 0.78, 0.65],
                MaterialId::SaltCuredRabbit     => [0.82, 0.55, 0.45],
                MaterialId::SaltCuredNostrichMeat => [0.55, 0.18, 0.35],
                // Seasoned cooked — slightly warmer than the cooked base.
                MaterialId::SeasonedBread          => [0.85, 0.65, 0.40],
                MaterialId::SeasonedBakedPotato    => [0.78, 0.62, 0.40],
                MaterialId::SeasonedBakedCarrot    => [0.92, 0.62, 0.20],
                MaterialId::SeasonedBakedCorn      => [0.95, 0.82, 0.30],
                MaterialId::SeasonedCookedBeef     => [0.68, 0.38, 0.28],
                MaterialId::SeasonedCookedPorkchop => [0.85, 0.58, 0.45],
                MaterialId::SeasonedCookedMutton   => [0.70, 0.42, 0.32],
                MaterialId::SeasonedCookedChicken  => [0.92, 0.72, 0.50],
                MaterialId::SeasonedCookedRabbit   => [0.85, 0.62, 0.50],
                // Rubber feature.
                MaterialId::Rubber        => [0.42, 0.30, 0.22], // dark latex-amber brown
                MaterialId::RubberSapling => [0.32, 0.55, 0.28], // matches RUBBER_LEAVES
                MaterialId::RubberBall    => [0.42, 0.30, 0.22], // same as Rubber
                MaterialId::CopperCable   => [0.82, 0.48, 0.28], // copper + brown band
                MaterialId::BountyBoardItem => [0.55, 0.36, 0.20], // matches BOUNTY_BOARD block colour
                MaterialId::TipJarItem => [0.70, 0.55, 0.20], // matches TIP_JAR block colour
                MaterialId::RepairBenchItem => [0.32, 0.32, 0.36], // matches REPAIR_BENCH block colour
                MaterialId::PlotMarkerItem => [0.85, 0.70, 0.15], // matches PLOT_MARKER block colour
                MaterialId::MarketBellItem => [0.80, 0.62, 0.25], // matches MARKET_BELL block colour
                MaterialId::AuctionBlockItem => [0.62, 0.45, 0.28], // matches AUCTION_BLOCK block colour
                MaterialId::BazaarBlockItem => [0.40, 0.55, 0.45], // matches BAZAAR_BLOCK block colour
                // Dyes (Spec 35) — the dye's own colour.
                MaterialId::BlueDye => [0.22, 0.34, 0.85],
                MaterialId::RedDye => [0.82, 0.16, 0.14],
                MaterialId::YellowDye => [0.96, 0.82, 0.18],
                // Fibre & cordage (Spec 36).
                MaterialId::Cotton => [0.94, 0.94, 0.90], // soft off-white boll
                MaterialId::HempFibre => [0.62, 0.66, 0.42], // pale green-tan fibre
                MaterialId::Rope => [0.72, 0.58, 0.34], // twisted tan cordage
                // Magnesium (Spec 37).
                MaterialId::Magnesium => [0.82, 0.82, 0.86], // silver-white mineral
                MaterialId::Fertiliser => [0.78, 0.80, 0.55], // pale Epsom-salt green
                MaterialId::Sparkler => [0.95, 0.92, 0.70], // bright spark gold
                MaterialId::Flare => [1.0, 0.95, 0.85], // brilliant white
                MaterialId::MagnesiumFirestarter => [0.55, 0.55, 0.58], // grey striker
                // Dye Phase 2 (Spec 35) — each dye's own colour.
                MaterialId::BlackDye => [0.12, 0.12, 0.14],
                MaterialId::WhiteDye => [0.95, 0.95, 0.95],
                MaterialId::OrangeDye => [0.90, 0.50, 0.15],
                MaterialId::GreenDye => [0.30, 0.62, 0.22],
                MaterialId::PurpleDye => [0.55, 0.25, 0.70],
                MaterialId::PinkDye => [0.93, 0.55, 0.70],
                MaterialId::LimeDye => [0.55, 0.85, 0.30],
                MaterialId::LightBlueDye => [0.45, 0.70, 0.92],
                MaterialId::GreyDye => [0.45, 0.45, 0.48],
                MaterialId::LightGreyDye => [0.70, 0.70, 0.72],
                // Fibre seeds (Spec 36 Phase 2).
                MaterialId::CottonSeeds => [0.80, 0.82, 0.62],
                MaterialId::HempSeeds => [0.62, 0.68, 0.42],
                // Spec 35 Phase 2 completion (3-input mix dyes).
                MaterialId::BrownDye => [0.42, 0.27, 0.16], // earthen umber
                MaterialId::CyanDye => [0.20, 0.72, 0.78], // teal cyan
                MaterialId::MagentaDye => [0.82, 0.25, 0.62], // hot magenta
                // Spec 36 Phase 2 — rope-derived consumer + textiles.
                MaterialId::Lead => [0.72, 0.58, 0.36], // braided rope + ivory loop
                MaterialId::Cloth => [0.96, 0.94, 0.86], // soft off-white cotton textile
                MaterialId::Canvas => [0.80, 0.74, 0.50], // coarse beige sailcloth
                // Spec 35 farmable-flower follow-on — seed icons tinted
                // toward the parent flower so they're visually grouped
                // with the mature flower block in the inventory.
                MaterialId::CornflowerSeeds => [0.55, 0.65, 0.78], // pale blue-grey
                MaterialId::FieldPoppySeeds => [0.78, 0.55, 0.42], // ruddy pink-tan
                MaterialId::ButtercupSeeds => [0.86, 0.78, 0.42], // pale yellow-tan
                // Craftable Armoured Carts (CA2) — tinted by hull material so
                // the three tiers read at a glance in the hotbar.
                MaterialId::WoodCart => [0.52, 0.36, 0.20], // cart-body brown
                MaterialId::IronCart => [0.66, 0.67, 0.70], // iron-plate grey
                MaterialId::DiamondCart => [0.40, 0.92, 0.90], // diamond cyan
                // Spec 49 (Explosives) — saltpetre pale off-white; powder near-black.
                MaterialId::Saltpetre => [0.90, 0.90, 0.84],
                MaterialId::BlackPowder => [0.14, 0.13, 0.13],
                MaterialId::Compost => [0.36, 0.26, 0.16], // earthy brown
                MaterialId::RawFish => [0.62, 0.70, 0.74], // silvery blue-grey
                MaterialId::CookedFish => [0.80, 0.62, 0.40], // golden cooked
                MaterialId::SharkTooth => [0.93, 0.92, 0.86], // ivory
                MaterialId::GlowInk => [0.30, 0.85, 0.80], // cyan glow
                MaterialId::CrabClaw => [0.90, 0.88, 0.80], // pale claw ivory
                MaterialId::ReachClaw => [0.80, 0.35, 0.20], // rust-red, matches the Crab
            },
        }
    }

    /// Attack damage when used as a weapon.
    pub fn attack_damage(&self) -> f32 {
        match self {
            Item::Block(_) | Item::Material(_) | Item::Plan(_) | Item::Armour(_) => 1.0,
            Item::Tool(t) => t.attack_damage(),
        }
    }

    /// If this item is a block, return the block ID for placement.
    pub fn as_block(&self) -> Option<BlockId> {
        match self {
            Item::Block(id) => Some(*id),
            _ => None,
        }
    }

    /// If this item is a tool, return a mutable reference.
    pub fn as_tool_mut(&mut self) -> Option<&mut Tool> {
        match self {
            Item::Tool(t) => Some(t),
            _ => None,
        }
    }

    /// Max stack size. Tools and Plans stack to 1; blocks/materials to 64.
    pub fn max_stack(&self) -> u8 {
        match self {
            // Craftable Armoured Carts (CA2) — carts are vehicles, not bulk
            // materials, so they stack small (you carry a few, not a tower).
            Item::Material(
                MaterialId::WoodCart | MaterialId::IronCart | MaterialId::DiamondCart,
            ) => 16,
            Item::Block(_) | Item::Material(_) => 64,
            Item::Tool(_) | Item::Plan(_) | Item::Armour(_) => 1,
        }
    }

    /// Health restored by eating one of this item, in HP. Returns None for
    /// non-food items. Numbers are Minecraft-ish but not a one-to-one port —
    /// rotten flesh heals more here as a "desperation food" tradeoff.
    pub fn food_value(&self) -> Option<f32> {
        match self {
            // Raw meats (Wave 2 drops). Edible but inferior to cooked.
            Item::Material(MaterialId::RawBeef) => Some(3.0),
            Item::Material(MaterialId::RawPorkchop) => Some(3.0),
            Item::Material(MaterialId::RawChicken) => Some(2.0),
            Item::Material(MaterialId::RawMutton) => Some(2.0),
            // Cooked variants (Wave 6) — meaningfully better than raw,
            // matching Minecraft's "cooked = ~+66%" intuition.
            Item::Material(MaterialId::CookedBeef) => Some(5.0),
            // P6 — fishing: raw catch is light, cooking it pays off.
            Item::Material(MaterialId::RawFish) => Some(2.0),
            Item::Material(MaterialId::CookedFish) => Some(6.0),
            Item::Material(MaterialId::CookedPorkchop) => Some(5.0),
            Item::Material(MaterialId::CookedChicken) => Some(4.0),
            Item::Material(MaterialId::CookedMutton) => Some(4.0),
            // Farming Tier 1 (Wave 26). Raw wheat is not edible per
            // Minecraft (you bake bread); raw potato is poor; carrot
            // is fine raw; bread is the crafted staple.
            Item::Material(MaterialId::Bread) => Some(5.0),
            Item::Material(MaterialId::Carrot) => Some(3.0),
            Item::Material(MaterialId::Potato) => Some(1.0),
            // Corn (Wave 28). Raw is moderate; baked is a proper meal.
            // "Corn on the Cob" matches Baked Potato on hunger value —
            // both are kid-favourite campfire foods.
            Item::Material(MaterialId::Corn) => Some(2.0),
            Item::Material(MaterialId::BakedPotato) => Some(5.0),
            Item::Material(MaterialId::BakedCarrot) => Some(4.0),
            Item::Material(MaterialId::BakedCorn) => Some(5.0),
            // Farming Tier 1.5 — Processed Economy Base. Value ladder per
            // spec: raw mid-tier (3-5), simple processed (5-7), complex
            // baked (8-12), masterclass (15-20).
            Item::Material(MaterialId::MilkBucket) => Some(6.0),
            Item::Material(MaterialId::Egg) => Some(1.0),
            Item::Material(MaterialId::Cream) => Some(3.0),
            Item::Material(MaterialId::Butter) => Some(4.0),
            Item::Material(MaterialId::Cheese) => Some(6.0),
            Item::Material(MaterialId::SweetBread) => Some(7.0),
            Item::Material(MaterialId::Cake) => Some(12.0),
            Item::Material(MaterialId::PumpkinPie) => Some(8.0),
            Item::Material(MaterialId::BerryPie) => Some(8.0),
            Item::Material(MaterialId::Cookie) => Some(2.0),
            Item::Material(MaterialId::Pancakes) => Some(7.0),
            Item::Material(MaterialId::LoadedBakedPotato) => Some(10.0),
            Item::Material(MaterialId::Stew) => Some(10.0),
            Item::Material(MaterialId::BeetrootSoup) => Some(8.0),
            Item::Material(MaterialId::PumpkinFood) => Some(2.0),
            Item::Material(MaterialId::SugarBeet) => Some(1.0),
            Item::Material(MaterialId::Beetroot) => Some(2.0),
            Item::Material(MaterialId::Berries) => Some(2.0),
            // Rabbit (28d) — raw mid-low, cooked solid (matches the
            // RawChicken / CookedChicken values; rabbit sits between
            // chicken and porkchop in the Minecraft baseline).
            Item::Material(MaterialId::RawRabbit) => Some(3.0),
            Item::Material(MaterialId::CookedRabbit) => Some(5.0),
            // Bee (28d) — Honey Bottle is the drinkable food. 6 HP
            // matches the Minecraft baseline and sits alongside the
            // MilkBucket-equivalent at the T1.5 value ladder.
            Item::Material(MaterialId::HoneyBottle) => Some(6.0),
            // Spec 28d.nostrich — Nostrich food values.
            // RawNostrichMeat is deliberately weak (1 HP) — eating
            // the mascot is supposed to be a bad deal; the vow does
            // the real punishment elsewhere in the game-loop.
            Item::Material(MaterialId::RawNostrichMeat) => Some(1.0),
            // NostrichEgg is a recipe ingredient (Minecraft eggs aren't
            // raw-edible). The "21× chicken-egg" yield is delivered
            // recipe-side — one NostrichEgg occupies the same slot as
            // 21 chicken eggs would in cake/pancake/omelette outputs.
            // Cooked recipes are higher-still as the design intends.
            Item::Material(MaterialId::NostrichOmelette) => Some(12.0),
            Item::Material(MaterialId::RoyalPavlova) => Some(14.0),
            Item::Material(MaterialId::NostrichCustard) => Some(10.0),
            // Salt feature — Cured raws (+1 food_value over the raw
            // input) and Seasoned cooked staples (+1 over the cooked
            // input). Cured X is the end product (NOT further cookable).
            Item::Material(MaterialId::SaltCuredBeef)         => Some(4.0),
            Item::Material(MaterialId::SaltCuredPorkchop)     => Some(4.0),
            Item::Material(MaterialId::SaltCuredMutton)       => Some(3.0),
            Item::Material(MaterialId::SaltCuredChicken)      => Some(3.0),
            Item::Material(MaterialId::SaltCuredRabbit)       => Some(3.0),
            Item::Material(MaterialId::SaltCuredNostrichMeat) => Some(2.0),
            Item::Material(MaterialId::SeasonedBread)          => Some(6.0),
            Item::Material(MaterialId::SeasonedBakedPotato)    => Some(6.0),
            Item::Material(MaterialId::SeasonedBakedCarrot)    => Some(5.0),
            Item::Material(MaterialId::SeasonedBakedCorn)      => Some(6.0),
            Item::Material(MaterialId::SeasonedCookedBeef)     => Some(6.0),
            Item::Material(MaterialId::SeasonedCookedPorkchop) => Some(6.0),
            Item::Material(MaterialId::SeasonedCookedMutton)   => Some(5.0),
            Item::Material(MaterialId::SeasonedCookedChicken)  => Some(5.0),
            Item::Material(MaterialId::SeasonedCookedRabbit)   => Some(6.0),
            _ => None,
        }
    }

    /// Spec T1.5 Phase 11 — value ladder. Higher tier = more
    /// processing steps from raw inputs. Drives `trade_value` defaults
    /// + recipe-pricing heuristics. Pure data, no engine state.
    ///
    /// Tiers (0..=5):
    /// - 0: raw resources (Wheat, RawBeef, GreenLog, ore drops)
    /// - 1: single-step crafted (Bread, IronIngot, CookedBeef, Cream)
    /// - 2: two-step processed (Cheese, Butter, Sugar, Dough, Bucket)
    /// - 3: three-step baked goods (SweetBread, Cookie, Pancakes)
    /// - 4: complex multi-ingredient meals (Stew, BerryPie, PumpkinPie)
    /// - 5: masterclass (Cake, LoadedBakedPotato)
    pub fn complexity_tier(&self) -> u8 {
        match self {
            Item::Material(m) => match m {
                // Tier 0 — raw / tools (Spec 40 bellows is a creative authoring tool)
                MaterialId::Bellows
                | MaterialId::Wheat
                | MaterialId::WheatSeeds
                | MaterialId::Carrot
                | MaterialId::Potato
                | MaterialId::Corn
                | MaterialId::CornSeeds
                | MaterialId::RawBeef
                | MaterialId::RawPorkchop
                | MaterialId::RawChicken
                | MaterialId::RawMutton
                // P6 — the raw fishing catch is tier 0 (raw food).
                | MaterialId::RawFish
                // Aquatic wave — raw mob drops (tooth, glow ink) are tier 0.
                | MaterialId::SharkTooth
                | MaterialId::GlowInk
                // Pets wave Task 13 — raw mob drop (claw), tier 0.
                | MaterialId::CrabClaw
                | MaterialId::Egg
                | MaterialId::Berries
                | MaterialId::SugarBeet
                | MaterialId::SugarBeetSeeds
                | MaterialId::Beetroot
                | MaterialId::BeetrootSeeds
                | MaterialId::PumpkinFood
                | MaterialId::RawIron
                | MaterialId::Copper
                | MaterialId::Tin
                | MaterialId::Coal
                | MaterialId::Diamond
                | MaterialId::Amethyst
                | MaterialId::Sulphur
                | MaterialId::GreenLog
                | MaterialId::PapyrusReed
                | MaterialId::Stick
                | MaterialId::Leather
                | MaterialId::Feather
                | MaterialId::Wool
                | MaterialId::Bone
                | MaterialId::String
                | MaterialId::Flint
                // Spec 28b — saplings sit at tier 0 (raw biological).
                | MaterialId::OakSapling
                | MaterialId::BirchSapling
                | MaterialId::SpruceSapling
                | MaterialId::JungleSapling
                | MaterialId::AcaciaSapling
                | MaterialId::DarkOakSapling
                // 28c Phase 5 — mob drops are raw, tier 0.
                | MaterialId::Honeycomb
                | MaterialId::Honey
                | MaterialId::InkSac
                | MaterialId::GlowBerry
                // 28d.rabbit — raw drops are tier 0.
                | MaterialId::RawRabbit
                | MaterialId::RabbitHide
                // 28d.bee — Honey Bottle is technically a Hive
                // right-click output (tier 1 — single workstation
                // step), but listed here so it joins the food economy
                // ladder above; see the Tier 1 arm below.
                | MaterialId::BeeStinger
                // HP-1 — Brigand Chieftain Trophy: rarity drop from
                // Sub 3 Hideouts; tier 0 (raw rarity).
                | MaterialId::BrigandChieftainTrophy
                // Spec 28d.nostrich — raw drops sit at tier 0. The
                // Nostrich-egg is "raw" in the same sense as Egg —
                // its premium value comes from the recipe ladder.
                | MaterialId::NostrichFeather
                | MaterialId::NostrichEgg
                | MaterialId::RawNostrichMeat
                // Salt — mineable raw material; tier 0.
                | MaterialId::Salt
                // Rubber — raw drop from tapping a living tree; sapling
                // is a tier-0 biological. Both tier 0.
                | MaterialId::Rubber
                | MaterialId::RubberSapling
                // Spec 36 — raw plant fibre (like String/Wool), tier 0.
                | MaterialId::Cotton
                | MaterialId::HempFibre
                // Spec 37 — raw mined mineral, tier 0.
                | MaterialId::Magnesium
                // Spec 36 Phase 2 — seeds, tier 0 (raw biological).
                | MaterialId::CottonSeeds
                | MaterialId::HempSeeds
                // Spec 35 farmable-flower follow-on — flower seeds, tier 0.
                | MaterialId::CornflowerSeeds
                | MaterialId::FieldPoppySeeds
                | MaterialId::ButtercupSeeds
                // Spec 49 (Explosives) — raw mined/aged saltpetre.
                | MaterialId::Saltpetre => 0,
                // Tier 1 — single-step crafted
                MaterialId::Bread
                // Spec 35 dyes + Spec 36 Rope — single-step crafted, tier 1.
                | MaterialId::BlueDye
                | MaterialId::RedDye
                | MaterialId::YellowDye
                | MaterialId::Rope
                // Spec 37 magnesium products — single-step crafted, tier 1.
                | MaterialId::Fertiliser
                | MaterialId::Sparkler
                | MaterialId::Flare
                | MaterialId::MagnesiumFirestarter
                // Dye Phase 2 (Spec 35) — single-step crafted/mixed, tier 1.
                | MaterialId::BlackDye
                | MaterialId::WhiteDye
                | MaterialId::OrangeDye
                | MaterialId::GreenDye
                | MaterialId::PurpleDye
                | MaterialId::PinkDye
                | MaterialId::LimeDye
                | MaterialId::LightBlueDye
                | MaterialId::GreyDye
                | MaterialId::LightGreyDye
                // Spec 35 Phase 2 completion — 3-input mix dyes, still tier 1
                // (a single craft, just with three inputs instead of two).
                | MaterialId::BrownDye
                | MaterialId::CyanDye
                | MaterialId::MagentaDye
                // Spec 36 Phase 2 — Rope's consumer + textiles, single-step
                // crafted from already-crafted fibre, tier 1.
                | MaterialId::Lead
                | MaterialId::Cloth
                | MaterialId::Canvas
                | MaterialId::Bonemeal
                | MaterialId::Arrow
                | MaterialId::CookedBeef
                | MaterialId::CookedPorkchop
                | MaterialId::CookedChicken
                | MaterialId::CookedMutton
                // P6 — the cooked fishing catch (single campfire step).
                | MaterialId::CookedFish
                | MaterialId::CookedRabbit
                | MaterialId::HoneyBottle
                | MaterialId::BakedPotato
                | MaterialId::BakedCarrot
                | MaterialId::BakedCorn
                | MaterialId::IronIngot
                | MaterialId::CopperIngot
                | MaterialId::TinIngot
                | MaterialId::SeasonedLog
                | MaterialId::KilnDriedLog
                | MaterialId::PapyrusSheet
                | MaterialId::MilkBucket
                | MaterialId::Cream
                | MaterialId::Flour
                | MaterialId::Bowl
                | MaterialId::Bucket
                // Filled buckets = a Bucket + a free liquid; same trade tier.
                | MaterialId::WaterBucket
                | MaterialId::LavaBucket
                // Spec 28d.nostrich — single-craft arrow.
                | MaterialId::NostrichArrow
                // Salt — Cured raw meats are a single craft step from
                // a raw input (Salt + RawX, where Salt itself is raw).
                | MaterialId::SaltCuredBeef
                | MaterialId::SaltCuredPorkchop
                | MaterialId::SaltCuredMutton
                | MaterialId::SaltCuredChicken
                | MaterialId::SaltCuredRabbit
                | MaterialId::SaltCuredNostrichMeat
                // Rubber — 1 Rubber -> 4 RubberBalls is a single craft.
                | MaterialId::RubberBall
                // Craftable Armoured Carts (CA2) — Wood Cart is a single craft
                // from planks (tier 1); the armoured tiers escalate below.
                | MaterialId::WoodCart
                // Spec 49 (Explosives) — Black Powder is a single shapeless craft;
                // Compost is a single passive process in the Composter.
                | MaterialId::BlackPowder
                | MaterialId::Compost
                // Pets wave Task 7 — Bone + String, single craft, tier 1.
                | MaterialId::RecallWhistle
                // Pets wave Task 9 — Raw Fish + Wheat, single craft, tier 1.
                | MaterialId::CatTreat
                // Pets wave Task 13 — Crab Claw + 2 Sticks, single craft, tier 1.
                | MaterialId::ReachClaw => 1,
                // Tier 2 — two-step processed
                MaterialId::Sugar
                | MaterialId::Butter
                | MaterialId::Dough
                | MaterialId::BronzeIngot
                | MaterialId::Cheese
                // Spec 28d.nostrich — banner = cloth + feather (two
                // steps from raw).
                | MaterialId::PurpleBanner
                // Salt — Seasoned cooked staples = cooked then seasoned
                // (2 steps from raw inputs).
                | MaterialId::SeasonedBread
                | MaterialId::SeasonedBakedPotato
                | MaterialId::SeasonedBakedCarrot
                | MaterialId::SeasonedBakedCorn
                | MaterialId::SeasonedCookedBeef
                | MaterialId::SeasonedCookedPorkchop
                | MaterialId::SeasonedCookedMutton
                | MaterialId::SeasonedCookedChicken
                | MaterialId::SeasonedCookedRabbit
                // Rubber — CopperCable = Copper Ingot (tier 1) + Rubber
                // processing; tier 2.
                | MaterialId::CopperCable
                // Mob Bounty Board — 8 planks ring + 1 IronIngot
                // (tier 1 metal); tier 2 same as Vendor Block.
                | MaterialId::BountyBoardItem
                // Tip Jar — 8 planks ring + 2 IronIngot (tier 2 same
                // as Bounty Board / Vendor Block).
                | MaterialId::TipJarItem
                // Repair Bench — iron + stone station; tier 2.
                | MaterialId::RepairBenchItem
                // Plot Marker — iron + plank boundary post; tier 2.
                | MaterialId::PlotMarkerItem
                // Market Bell — iron + plank discovery beacon; tier 2.
                | MaterialId::MarketBellItem
                // Auction Block — iron + plank podium; tier 2.
                | MaterialId::AuctionBlockItem
                // Bazaar Block — plank + diamond trading post; tier 2.
                | MaterialId::BazaarBlockItem
                // Craftable Armoured Carts (CA2) — Iron Cart is the Wood Cart
                // (tier 1) clad in an iron ring; one step up, tier 2.
                | MaterialId::IronCart => 2,
                // Tier 3 — three-step baked goods
                MaterialId::SweetBread
                | MaterialId::Cookie
                | MaterialId::Pancakes
                | MaterialId::BeetrootSoup
                // Craftable Armoured Carts (CA2) — Diamond Cart is the Iron
                // Cart (tier 2) clad in a diamond ring; tier 3.
                | MaterialId::DiamondCart => 3,
                // Tier 4 — complex multi-ingredient
                MaterialId::Stew
                | MaterialId::PumpkinPie
                | MaterialId::BerryPie
                // Spec 28d.nostrich — Omelette + Custard sit at T4
                // (multi-ingredient hearty meal / dessert).
                | MaterialId::NostrichOmelette
                | MaterialId::NostrichCustard => 4,
                // Tier 5 — masterclass
                MaterialId::Cake
                | MaterialId::LoadedBakedPotato
                // Spec 28d.nostrich — Royal Pavlova is the showpiece.
                | MaterialId::RoyalPavlova => 5,
                // Bitcoin-tier material — sits separately from the food
                // economy ladder.
                MaterialId::Satori => 5,
            },
            // Blocks default to 0 (raw / placed) unless overridden later.
            Item::Block(_) => 0,
            // Tools have their own tier (ToolMaterial), conceptually mid-ladder.
            Item::Tool(_) => 2,
            // Plans are per-instance; complexity reflects effort to capture,
            // not a fixed tier. Default to 2 for trade pricing.
            Item::Plan(_) => 2,
            // Armour pieces scale by material tier — Leather T2, Iron T3,
            // Chainmail T3 (mob-drop rarity matches the craft effort),
            // Diamond T4, Satori T5.
            Item::Armour(a) => match a.material {
                crate::armour::ArmourMaterial::Leather => 2,
                crate::armour::ArmourMaterial::Iron => 3,
                crate::armour::ArmourMaterial::Chainmail => 3,
                crate::armour::ArmourMaterial::Diamond => 4,
                crate::armour::ArmourMaterial::Satori => 5,
                // Rubber Boots — utility piece between Leather + Iron;
                // T2 trade tier matches Leather (the base armour material).
                crate::armour::ArmourMaterial::Rubber => 2,
            },
        }
    }

    /// Spec T1.5 Phase 11 — default in-world trade value, in
    /// "trade-units". `None` = not tradeable (e.g., bedrock, air).
    /// Server operators override the per-item map; Bitcoin-enabled
    /// servers may convert to sats via a per-server rate (parent-
    /// controlled per Spec 6 §10.3).
    ///
    /// Defaults derive from complexity_tier: tier 0 = 1, tier 1 = 3,
    /// tier 2 = 8, tier 3 = 18, tier 4 = 40, tier 5 = 90. Roughly
    /// 2-3× per tier — rewards processing without making cake worth
    /// more than a Satori.
    pub fn trade_value(&self) -> Option<u64> {
        match self {
            // Untradeable blocks (placeholder — air/water specifically).
            Item::Block(b) if *b == crate::block::AIR || *b == crate::block::WATER => None,
            _ => Some(match self.complexity_tier() {
                0 => 1,
                1 => 3,
                2 => 8,
                3 => 18,
                4 => 40,
                _ => 90,
            }),
        }
    }

    /// Convenience predicate: does this item count as food the player can eat?
    pub fn is_food(&self) -> bool {
        self.food_value().is_some()
    }

    /// Poison ticks applied as an eat side-effect, in 20-TPS ticks.
    /// No item currently triggers poison (the poison mechanic is retained
    /// for future foods/effects).
    pub fn eat_poison_ticks(&self) -> u32 {
        0
    }

    /// Whether two items can stack together.
    pub fn can_stack_with(&self, other: &Item) -> bool {
        match (self, other) {
            (Item::Block(a), Item::Block(b)) => a == b,
            (Item::Material(a), Item::Material(b)) => a == b,
            // Tools and Plans are per-instance — never stack, even with
            // a structurally-identical sibling.
            _ => false,
        }
    }

    /// #45 — a deterministic, total sort key over every `Item` variant, used by
    /// `inventory::sort_slots`. Ordered by category first (blocks, then
    /// materials, then tools, then armour, then plans) so a sort groups like
    /// with like, then by a stable within-category id. All component enums are
    /// fieldless `Copy`, so the casts are stable discriminants.
    pub fn sort_key(&self) -> (u8, u32) {
        match self {
            Item::Block(id) => (0, *id as u32),
            Item::Material(mid) => (1, *mid as u32),
            Item::Tool(t) => (2, ((t.material as u32) << 16) | (t.tool_type as u32)),
            Item::Armour(a) => (3, ((a.material as u32) << 16) | (a.slot as u32)),
            // Plans are per-instance with no meaningful order; bucket them last
            // together so a sort is still deterministic.
            Item::Plan(_) => (4, 0),
        }
    }
}

/// If a material can be placed as a world block (right-click-to-place),
/// return the block id it represents. Returns `None` for non-placeable
/// materials. Cross-game-generic primitive — game-specific data lives in
/// the match arm below.
///
/// Wave 29 (log seasoning): all three log materials place as OAK_LOG so
/// players can build with green logs immediately after chopping a tree
/// without going through a 1:1 crafting conversion. Seasoning matters
/// for fuel quality + smoke economy, not for structural use.
pub fn material_as_placeable_block(m: MaterialId) -> Option<BlockId> {
    match m {
        MaterialId::GreenLog | MaterialId::SeasonedLog | MaterialId::KilnDriedLog => {
            Some(crate::block::OAK_LOG)
        }
        // Economy-block items (Specs 33-36) are stored as MaterialIds
        // (their recipes + mine_drops return materials), so they must
        // resolve to their placed block here or they're uncraftable-
        // into-placement. (Vendor Block predates this + uses
        // `Item::Block` directly, so it isn't listed.)
        MaterialId::BountyBoardItem => Some(crate::block::BOUNTY_BOARD),
        MaterialId::TipJarItem => Some(crate::block::TIP_JAR),
        MaterialId::RepairBenchItem => Some(crate::block::REPAIR_BENCH),
        MaterialId::PlotMarkerItem => Some(crate::block::PLOT_MARKER),
        MaterialId::MarketBellItem => Some(crate::block::MARKET_BELL),
        MaterialId::AuctionBlockItem => Some(crate::block::AUCTION_BLOCK),
        MaterialId::BazaarBlockItem => Some(crate::block::BAZAAR_BLOCK),
        // Saplings (2026-07-04) — plant as the species' sapling block. The
        // place path additionally gates on sapling::can_plant_sapling_at
        // (grass/dirt only).
        MaterialId::OakSapling => Some(crate::block::SAPLING_OAK),
        MaterialId::BirchSapling => Some(crate::block::SAPLING_BIRCH),
        MaterialId::SpruceSapling => Some(crate::block::SAPLING_SPRUCE),
        MaterialId::JungleSapling => Some(crate::block::SAPLING_JUNGLE),
        MaterialId::AcaciaSapling => Some(crate::block::SAPLING_ACACIA),
        MaterialId::DarkOakSapling => Some(crate::block::SAPLING_DARK_OAK),
        MaterialId::RubberSapling => Some(crate::block::SAPLING_RUBBER),
        _ => None,
    }
}

impl MaterialId {
    /// The WALLPAPER block whose colour this dye paints with (the "paint-with-
    /// blocks" colour source — `docs/foundations/2026-05-27-flowers-dyes-colour-mixing.md`).
    /// `None` for non-dye materials (e.g. the Bellows). One source of truth for
    /// both crafting (dye → wallpaper recipe) and the Workshop painter.
    pub fn paint_block(self) -> Option<crate::block::BlockId> {
        use crate::block;
        Some(match self {
            MaterialId::WhiteDye => block::WALLPAPER_WHITE,
            MaterialId::BlackDye => block::WALLPAPER_BLACK,
            MaterialId::RedDye => block::WALLPAPER_RED,
            MaterialId::BlueDye => block::WALLPAPER_BLUE,
            MaterialId::YellowDye => block::WALLPAPER_YELLOW,
            MaterialId::OrangeDye => block::WALLPAPER_ORANGE,
            MaterialId::GreenDye => block::WALLPAPER_GREEN,
            MaterialId::PurpleDye => block::WALLPAPER_PURPLE,
            MaterialId::PinkDye => block::WALLPAPER_PINK,
            MaterialId::LimeDye => block::WALLPAPER_LIME,
            MaterialId::LightBlueDye => block::WALLPAPER_LIGHT_BLUE,
            MaterialId::GreyDye => block::WALLPAPER_GREY,
            MaterialId::LightGreyDye => block::WALLPAPER_LIGHT_GREY,
            MaterialId::BrownDye => block::WALLPAPER_BROWN,
            MaterialId::CyanDye => block::WALLPAPER_CYAN,
            MaterialId::MagentaDye => block::WALLPAPER_MAGENTA,
            _ => return None,
        })
    }
}

/// Every dye, in palette display order (light → dark, roughly Minecraft's own
/// ordering). ONE source of truth for the skin painter's swatch row, so a
/// palette swatch can never drift from what the matching held dye paints.
pub const DYES: [MaterialId; 16] = [
    MaterialId::WhiteDye,
    MaterialId::LightGreyDye,
    MaterialId::GreyDye,
    MaterialId::BlackDye,
    MaterialId::RedDye,
    MaterialId::OrangeDye,
    MaterialId::YellowDye,
    MaterialId::LimeDye,
    MaterialId::GreenDye,
    MaterialId::CyanDye,
    MaterialId::LightBlueDye,
    MaterialId::BlueDye,
    MaterialId::PurpleDye,
    MaterialId::MagentaDye,
    MaterialId::PinkDye,
    MaterialId::BrownDye,
];

/// A dye's RGBA paint colour for the skin painter: the dye → its WALLPAPER block
/// → that block's registry colour → opaque `[u8;4]`. Keeps ONE source of truth
/// with the block painter (`paint_block` + `registry.color`). `None` for any
/// non-dye material (so a held block/tool/nothing can't paint a pixel).
pub fn dye_skin_color(m: MaterialId, registry: &crate::block::BlockRegistry) -> Option<[u8; 4]> {
    let block = m.paint_block()?;
    let c = registry.color(block);
    Some([
        (c[0] * 255.0).round().clamp(0.0, 255.0) as u8,
        (c[1] * 255.0).round().clamp(0.0, 255.0) as u8,
        (c[2] * 255.0).round().clamp(0.0, 255.0) as u8,
        255,
    ])
}

/// A stack of identical items in an inventory slot.
///
/// `PartialEq` (2026-07-06, Task 11) — needed so `ChestData` can derive it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemStack {
    pub item: Item,
    pub count: u8,
}

impl ItemStack {
    /// A 0-count placeholder. Used as a "no drop" return from `mine_drop`
    /// for blocks like campfire smoke that have no inventory representation.
    /// Callers should noop on count == 0.
    pub fn empty() -> Self {
        Self {
            item: Item::Block(crate::block::AIR),
            count: 0,
        }
    }

    pub fn new_block(block_id: BlockId, count: u8) -> Self {
        Self {
            item: Item::Block(block_id),
            count,
        }
    }

    pub fn new_tool(tool: Tool) -> Self {
        Self {
            item: Item::Tool(tool),
            count: 1,
        }
    }

    pub fn new_material(id: MaterialId, count: u8) -> Self {
        Self {
            item: Item::Material(id),
            count,
        }
    }

    /// Construct a fresh armour piece at full durability. Spec 28e —
    /// armour pieces never stack (each carries its own durability),
    /// so the count is always 1.
    pub fn new_armour(slot: crate::armour::ArmourSlot, material: crate::armour::ArmourMaterial) -> Self {
        Self {
            item: Item::Armour(crate::armour::ArmourItem::new(slot, material)),
            count: 1,
        }
    }
}

#[cfg(test)]
mod dye_palette_tests {
    use super::*;

    #[test]
    fn dyes_covers_every_dye_material_exactly_once() {
        // The palette row and the held-dye path must never drift apart: every
        // material with a paint_block must appear in DYES, and nothing else.
        let reg = crate::block::BlockRegistry::new();
        let mut seen = std::collections::HashSet::new();
        for d in DYES {
            assert!(seen.insert(d), "duplicate dye in the palette: {d:?}");
            assert!(
                dye_skin_color(d, &reg).is_some(),
                "{d:?} is in the palette but paints nothing"
            );
        }
        assert_eq!(DYES.len(), 16, "16 dye colours");
    }

    #[test]
    fn every_palette_colour_is_opaque_and_distinct() {
        let reg = crate::block::BlockRegistry::new();
        let mut seen = std::collections::HashSet::new();
        for d in DYES {
            let c = dye_skin_color(d, &reg).expect("dye paints");
            assert_eq!(c[3], 255, "{d:?} must paint an opaque pixel");
            assert!(seen.insert(c), "two dyes share a colour: {d:?} -> {c:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_id_try_from_round_trips_with_as_u16() {
        // The wire encodes `material as u16`; TryFrom must be its exact
        // inverse for every variant that ALL_MATERIAL_IDS knows about.
        // (ALL_MATERIAL_IDS is the discriminant-ordered, test-locked list
        // used elsewhere for the inventory explorer.)
        for &m in crate::inventory_explorer::ALL_MATERIAL_IDS {
            let idx = m as u16;
            assert_eq!(
                MaterialId::try_from(idx),
                Ok(m),
                "round-trip failed for {:?} at index {}",
                m,
                idx
            );
        }
        // First variant round-trips from 0.
        assert_eq!(MaterialId::try_from(0), Ok(MaterialId::Stick));
        // Out-of-range index fails gracefully (returns the offending value).
        assert_eq!(MaterialId::try_from(60000), Err(60000));
    }

    #[test]
    fn block_names_never_show_their_registry_namespace() {
        // Wind/Copper/Electricity wave — `Item::name` used to strip only
        // `genesis:`, so every Spec 48 block reached the inventory, the hotbar
        // and the crafting book reading "Electricity:water Wheel". A namespace
        // is an engine detail; the player sees the words after the colon.
        let reg = crate::block::BlockRegistry::new();
        for (id, want) in [
            (crate::block::WATER_WHEEL, "Water Wheel"),
            (crate::block::WINDMILL, "Windmill"),
            (crate::block::CABLE, "Cable"),
            (crate::block::STEAM_GENERATOR, "Steam Generator"),
            (crate::block::OAK_PLANKS, "Oak Planks"),
        ] {
            assert_eq!(Item::Block(id).name(&reg), want);
        }
        // Belt and braces across the whole registry: no label may carry a colon.
        for id in 0..reg.len() as crate::block::BlockId {
            let label = Item::Block(id).name(&reg);
            assert!(
                !label.contains(':'),
                "block {id} renders as {label:?} — a namespace leaked into the UI"
            );
        }
    }

    #[test]
    fn farming_food_values_match_spec() {
        // Spec 16 / farming-system Phase 4. Raw wheat isn't edible (you bake
        // bread); bread is the staple food at 5 HP. Carrot is mid-tier at
        // 3 HP, raw potato is low at 1 HP. Numbers from the spec.
        assert_eq!(Item::Material(MaterialId::Wheat).food_value(), None);
        assert_eq!(Item::Material(MaterialId::WheatSeeds).food_value(), None);
        assert_eq!(Item::Material(MaterialId::Bread).food_value(), Some(5.0));
        assert_eq!(Item::Material(MaterialId::Carrot).food_value(), Some(3.0));
        assert_eq!(Item::Material(MaterialId::Potato).food_value(), Some(1.0));
    }

    #[test]
    fn farming_items_have_unique_colours() {
        let registry = crate::block::BlockRegistry::new();
        let mut seen = std::collections::HashSet::new();
        for m in [
            MaterialId::WheatSeeds,
            MaterialId::Wheat,
            MaterialId::Bread,
            MaterialId::Carrot,
            MaterialId::Potato,
        ] {
            let c = Item::Material(m).color(&registry);
            let key = ((c[0] * 100.0) as i32, (c[1] * 100.0) as i32, (c[2] * 100.0) as i32);
            assert!(seen.insert(key), "duplicate hotbar colour for {:?}", m);
        }
    }

    #[test]
    fn farming_items_stack_to_64() {
        for m in [
            MaterialId::WheatSeeds,
            MaterialId::Wheat,
            MaterialId::Bread,
            MaterialId::Carrot,
            MaterialId::Potato,
        ] {
            assert_eq!(Item::Material(m).max_stack(), 64, "{:?} should stack to 64", m);
        }
    }

    #[test]
    fn three_new_log_materials_have_names_and_colours() {
        let registry = crate::block::BlockRegistry::new();
        let mut seen = std::collections::HashSet::new();
        for m in [MaterialId::GreenLog, MaterialId::SeasonedLog, MaterialId::KilnDriedLog] {
            let item = Item::Material(m);
            let name = item.name(&registry);
            assert!(!name.is_empty(), "{:?} must have a display name", m);
            let c = item.color(&registry);
            let key = ((c[0] * 100.0) as i32, (c[1] * 100.0) as i32, (c[2] * 100.0) as i32);
            assert!(seen.insert(key), "{:?} must have a unique hotbar colour", m);
            assert_eq!(item.max_stack(), 64, "{:?} stacks to 64", m);
        }
        assert_eq!(Item::Material(MaterialId::GreenLog).name(&registry), "Green Log");
        assert_eq!(Item::Material(MaterialId::SeasonedLog).name(&registry), "Seasoned Log");
        assert_eq!(Item::Material(MaterialId::KilnDriedLog).name(&registry), "Kiln-Dried Log");
    }

    #[test]
    fn material_as_placeable_block_covers_three_logs() {
        assert_eq!(material_as_placeable_block(MaterialId::GreenLog), Some(crate::block::OAK_LOG));
        assert_eq!(material_as_placeable_block(MaterialId::SeasonedLog), Some(crate::block::OAK_LOG));
        assert_eq!(material_as_placeable_block(MaterialId::KilnDriedLog), Some(crate::block::OAK_LOG));
    }

    #[test]
    fn material_as_placeable_block_returns_none_for_non_log() {
        assert_eq!(material_as_placeable_block(MaterialId::Stick), None);
        assert_eq!(material_as_placeable_block(MaterialId::Coal), None);
        assert_eq!(material_as_placeable_block(MaterialId::Wheat), None);
        assert_eq!(material_as_placeable_block(MaterialId::Flint), None);
    }

    #[test]
    fn economy_block_items_are_placeable() {
        // Regression: Specs 33-35 shipped these as MaterialIds whose
        // recipes return materials, but they were missing from this
        // mapping — meaning they crafted but couldn't be placed.
        assert_eq!(material_as_placeable_block(MaterialId::BountyBoardItem), Some(crate::block::BOUNTY_BOARD));
        assert_eq!(material_as_placeable_block(MaterialId::TipJarItem), Some(crate::block::TIP_JAR));
        assert_eq!(material_as_placeable_block(MaterialId::RepairBenchItem), Some(crate::block::REPAIR_BENCH));
        assert_eq!(material_as_placeable_block(MaterialId::PlotMarkerItem), Some(crate::block::PLOT_MARKER));
        assert_eq!(material_as_placeable_block(MaterialId::MarketBellItem), Some(crate::block::MARKET_BELL));
        assert_eq!(material_as_placeable_block(MaterialId::AuctionBlockItem), Some(crate::block::AUCTION_BLOCK));
        assert_eq!(material_as_placeable_block(MaterialId::BazaarBlockItem), Some(crate::block::BAZAAR_BLOCK));
    }

    #[test]
    fn new_log_materials_stack_with_self_only() {
        let g1 = Item::Material(MaterialId::GreenLog);
        let g2 = Item::Material(MaterialId::GreenLog);
        let s = Item::Material(MaterialId::SeasonedLog);
        assert!(g1.can_stack_with(&g2));
        assert!(!g1.can_stack_with(&s));
    }

    #[test]
    fn papyrus_materials_have_names_and_unique_colours() {
        let registry = crate::block::BlockRegistry::new();
        let reed = Item::Material(MaterialId::PapyrusReed);
        let sheet = Item::Material(MaterialId::PapyrusSheet);
        assert_eq!(reed.name(&registry), "Papyrus Reed");
        assert_eq!(sheet.name(&registry), "Papyrus Sheet");
        // Reed (green-tan) must differ from Sheet (parchment) at hotbar
        // resolution — otherwise they're confusable in inventory.
        assert_ne!(reed.color(&registry), sheet.color(&registry));
    }

    #[test]
    fn papyrus_materials_stack_to_64() {
        assert_eq!(Item::Material(MaterialId::PapyrusReed).max_stack(), 64);
        assert_eq!(Item::Material(MaterialId::PapyrusSheet).max_stack(), 64);
    }

    #[test]
    fn papyrus_materials_stack_with_self_only() {
        let r1 = Item::Material(MaterialId::PapyrusReed);
        let r2 = Item::Material(MaterialId::PapyrusReed);
        let s = Item::Material(MaterialId::PapyrusSheet);
        assert!(r1.can_stack_with(&r2));
        assert!(!r1.can_stack_with(&s));
    }

    /// Spec 28e — comprehensive is_food audit. Every MaterialId is
    /// classified as either food (returns Some food_value) or non-food
    /// (returns None). This single test guards against:
    /// - A new edible material missing its food_value entry.
    /// - A non-food material accidentally returning Some.
    ///
    /// The CANONICAL_FOODS list below is the spec contract — adding a
    /// new edible material requires updating BOTH the food_value match
    /// AND this list. Keeping them in sync is the test's job.
    #[test]
    fn is_food_classification_is_canonical() {
        use crate::inventory_explorer::ALL_MATERIAL_IDS;

        // The full set of MaterialIds that ARE food. Update when adding
        // a new edible material.
        let canonical_foods: &[MaterialId] = &[
            // Raw meats
            MaterialId::RawBeef, MaterialId::RawPorkchop,
            MaterialId::RawChicken, MaterialId::RawMutton,
            // Cooked meats
            MaterialId::CookedBeef, MaterialId::CookedPorkchop,
            MaterialId::CookedChicken, MaterialId::CookedMutton,
            // T1 farming foods
            MaterialId::Bread, MaterialId::Carrot, MaterialId::Potato,
            // Corn family
            MaterialId::Corn, MaterialId::BakedCorn,
            MaterialId::BakedPotato, MaterialId::BakedCarrot,
            // T1.5 processed economy
            MaterialId::MilkBucket, MaterialId::Egg,
            MaterialId::Cream, MaterialId::Butter, MaterialId::Cheese,
            MaterialId::SweetBread, MaterialId::Cake,
            MaterialId::PumpkinPie, MaterialId::BerryPie,
            MaterialId::Cookie, MaterialId::Pancakes,
            MaterialId::LoadedBakedPotato, MaterialId::Stew,
            MaterialId::BeetrootSoup,
            MaterialId::PumpkinFood, MaterialId::SugarBeet,
            MaterialId::Beetroot, MaterialId::Berries,
            // Spec 28d.rabbit + Bee — meat + drink food paths.
            MaterialId::RawRabbit, MaterialId::CookedRabbit,
            MaterialId::HoneyBottle,
            // Spec 28d.nostrich — mascot meat (intentionally weak) +
            // cooked recipe outputs. NostrichEgg is NOT food (recipe
            // ingredient only, per the Minecraft egg pattern).
            MaterialId::RawNostrichMeat,
            MaterialId::NostrichOmelette,
            MaterialId::RoyalPavlova,
            MaterialId::NostrichCustard,
            // Salt feature — Cured raw meats + Seasoned cooked staples
            // (each +1 food_value over its input). Now enumerated in
            // ALL_MATERIAL_IDS (Phase D resynced the explorer table), so
            // they must be listed here as canonical foods too.
            MaterialId::SaltCuredBeef, MaterialId::SaltCuredPorkchop,
            MaterialId::SaltCuredMutton, MaterialId::SaltCuredChicken,
            MaterialId::SaltCuredRabbit, MaterialId::SaltCuredNostrichMeat,
            MaterialId::SeasonedBread, MaterialId::SeasonedBakedPotato,
            MaterialId::SeasonedBakedCarrot, MaterialId::SeasonedBakedCorn,
            MaterialId::SeasonedCookedBeef, MaterialId::SeasonedCookedPorkchop,
            MaterialId::SeasonedCookedMutton, MaterialId::SeasonedCookedChicken,
            MaterialId::SeasonedCookedRabbit,
        ];

        // Every MaterialId must be exhaustively classified.
        for m in ALL_MATERIAL_IDS {
            let item = Item::Material(*m);
            let expected_food = canonical_foods.contains(m);
            let actual_food = item.is_food();
            assert_eq!(
                actual_food, expected_food,
                "MaterialId::{m:?}: expected is_food={expected_food}, got {actual_food}"
            );
        }

        // Every food in canonical_foods must have a positive food_value.
        for m in canonical_foods {
            let value = Item::Material(*m).food_value();
            assert!(
                matches!(value, Some(v) if v > 0.0),
                "MaterialId::{m:?} is canonical food but food_value = {value:?}"
            );
        }

        // No canonical_foods entry appears twice (catches copy-paste).
        let mut sorted: Vec<&MaterialId> = canonical_foods.iter().collect();
        sorted.sort_by_key(|m| format!("{m:?}"));
        for w in sorted.windows(2) {
            assert!(
                format!("{:?}", w[0]) != format!("{:?}", w[1]),
                "canonical_foods has duplicate: {:?}",
                w[0]
            );
        }
    }

    #[test]
    fn no_food_value_exceeds_player_max_hp() {
        // Sanity: no single food should heal more than the player's
        // max HP (20). If a future tweak pushes Cake or similar over
        // that limit, this fails so it can be reconsidered.
        use crate::inventory_explorer::ALL_MATERIAL_IDS;
        for m in ALL_MATERIAL_IDS {
            if let Some(v) = Item::Material(*m).food_value() {
                assert!(
                    v <= 20.0,
                    "MaterialId::{m:?} food_value {v} exceeds player max HP"
                );
            }
        }
    }

    #[test]
    fn material_id_try_from_is_count_locked_and_identity() {
        // `TryFrom<u16> for MaterialId` is a hand-maintained index table; a
        // mid-enum insert would shift every later discriminant and silently
        // desync the positional bincode wire decode. This round-trip + count
        // lock makes that fail loudly (engine audit 2026-06-04, E). When a new
        // MaterialId is legitimately appended, bump MATERIAL_ID_COUNT here.
        const MATERIAL_ID_COUNT: u16 = 169; // ids 0..=168 (167: CrabClaw, 168: ReachClaw)
        for n in 0..MATERIAL_ID_COUNT {
            let m = MaterialId::try_from(n)
                .unwrap_or_else(|_| panic!("id {n} must decode (< MATERIAL_ID_COUNT)"));
            assert_eq!(m as u16, n, "TryFrom must invert the discriminant exactly");
        }
        assert!(
            MaterialId::try_from(MATERIAL_ID_COUNT).is_err(),
            "id one past the last must be rejected — bump MATERIAL_ID_COUNT when adding a MaterialId",
        );
    }

    #[test]
    fn all_raw_meats_have_lower_value_than_cooked() {
        // Cooking should always increase food value — the value
        // ladder is the spec's central incentive structure.
        let pairs = [
            (MaterialId::RawBeef, MaterialId::CookedBeef),
            (MaterialId::RawPorkchop, MaterialId::CookedPorkchop),
            (MaterialId::RawChicken, MaterialId::CookedChicken),
            (MaterialId::RawMutton, MaterialId::CookedMutton),
            (MaterialId::Potato, MaterialId::BakedPotato),
            (MaterialId::Carrot, MaterialId::BakedCarrot),
            (MaterialId::Corn, MaterialId::BakedCorn),
        ];
        for (raw, cooked) in pairs {
            let raw_v = Item::Material(raw).food_value().unwrap();
            let cooked_v = Item::Material(cooked).food_value().unwrap();
            assert!(cooked_v > raw_v,
                "{cooked:?} ({cooked_v}) should be > {raw:?} ({raw_v})");
        }
    }

    #[test]
    fn historical_naming_pass_material_renames() {
        let registry = crate::block::BlockRegistry::new();
        assert_eq!(Item::Material(MaterialId::Bonemeal).name(&registry), "Bone Meal");
        assert_eq!(Item::Material(MaterialId::HoneyBottle).name(&registry), "Honey Jar");
    }

    #[test]
    fn historical_naming_pass_block_display_overrides() {
        use crate::block::{self, BlockRegistry};
        let registry = BlockRegistry::new();
        assert_eq!(Item::Block(block::HAY_BALE).name(&registry), "Hay Rick");
        assert_eq!(Item::Block(block::BONE_BLOCK).name(&registry), "Bone Cairn");
        assert_eq!(Item::Block(block::AMETHYST_BLOCK).name(&registry), "Amethyst Cluster");
        assert_eq!(Item::Block(block::SUGARCANE).name(&registry), "Sugar Cane");
        assert_eq!(Item::Block(block::VENDOR_BLOCK).name(&registry), "Market Stall");
        assert_eq!(Item::Block(block::BLUEPRINT_PAPER).name(&registry), "Blueprint Paper");
        assert_eq!(Item::Block(block::CONSTRUCTION_ANCHOR).name(&registry), "Foundation Stone");
        assert_eq!(Item::Block(block::ARCHITECT_PLAQUE).name(&registry), "Mason's Mark");
        assert_eq!(Item::Block(block::DRAFTING_TABLE).name(&registry), "Drafting Bench");
    }

    #[test]
    fn historical_naming_pass_unchanged_blocks_still_derive() {
        use crate::block::{self, BlockRegistry};
        let registry = BlockRegistry::new();
        assert_eq!(Item::Block(block::CRAFTING_TABLE).name(&registry), "Workbench");
        assert_eq!(Item::Block(block::FURNACE).name(&registry), "Furnace");
        assert_eq!(Item::Block(block::OAK_PLANKS).name(&registry), "Oak Planks");
    }

    // Salt feature (2026-05-23) — Cured + Seasoned variant ladder.

    #[test]
    fn salt_cured_lifts_food_value_by_one() {
        let raw = Item::Material(MaterialId::RawBeef);
        let cured = Item::Material(MaterialId::SaltCuredBeef);
        assert_eq!(cured.food_value(), Some(raw.food_value().unwrap() + 1.0));
    }

    #[test]
    fn seasoned_lifts_food_value_by_one() {
        let cooked = Item::Material(MaterialId::CookedBeef);
        let seasoned = Item::Material(MaterialId::SeasonedCookedBeef);
        assert_eq!(seasoned.food_value(), Some(cooked.food_value().unwrap() + 1.0));
    }

    #[test]
    fn salt_itself_is_not_food() {
        let salt = Item::Material(MaterialId::Salt);
        assert!(!salt.is_food());
        assert!(salt.food_value().is_none());
    }

    #[test]
    fn all_cured_and_seasoned_are_food() {
        let variants = [
            MaterialId::SaltCuredBeef, MaterialId::SaltCuredPorkchop,
            MaterialId::SaltCuredMutton, MaterialId::SaltCuredChicken,
            MaterialId::SaltCuredRabbit, MaterialId::SaltCuredNostrichMeat,
            MaterialId::SeasonedBread, MaterialId::SeasonedBakedPotato,
            MaterialId::SeasonedBakedCarrot, MaterialId::SeasonedBakedCorn,
            MaterialId::SeasonedCookedBeef, MaterialId::SeasonedCookedPorkchop,
            MaterialId::SeasonedCookedMutton, MaterialId::SeasonedCookedChicken,
            MaterialId::SeasonedCookedRabbit,
        ];
        for v in variants {
            let item = Item::Material(v);
            assert!(item.is_food(), "{v:?} should be food");
            assert!(item.food_value().is_some(), "{v:?} should have food_value");
        }
    }

    #[test]
    fn dye_maps_to_its_wallpaper_paint_block() {
        use crate::block;
        assert_eq!(MaterialId::WhiteDye.paint_block(), Some(block::WALLPAPER_WHITE));
        assert_eq!(MaterialId::BlackDye.paint_block(), Some(block::WALLPAPER_BLACK));
        assert_eq!(MaterialId::RedDye.paint_block(), Some(block::WALLPAPER_RED));
        assert_eq!(MaterialId::BlueDye.paint_block(), Some(block::WALLPAPER_BLUE));
        assert_eq!(MaterialId::YellowDye.paint_block(), Some(block::WALLPAPER_YELLOW));
        assert_eq!(MaterialId::OrangeDye.paint_block(), Some(block::WALLPAPER_ORANGE));
        assert_eq!(MaterialId::GreenDye.paint_block(), Some(block::WALLPAPER_GREEN));
        assert_eq!(MaterialId::PurpleDye.paint_block(), Some(block::WALLPAPER_PURPLE));
        assert_eq!(MaterialId::PinkDye.paint_block(), Some(block::WALLPAPER_PINK));
        assert_eq!(MaterialId::LimeDye.paint_block(), Some(block::WALLPAPER_LIME));
        assert_eq!(MaterialId::LightBlueDye.paint_block(), Some(block::WALLPAPER_LIGHT_BLUE));
        assert_eq!(MaterialId::GreyDye.paint_block(), Some(block::WALLPAPER_GREY));
        assert_eq!(MaterialId::LightGreyDye.paint_block(), Some(block::WALLPAPER_LIGHT_GREY));
        assert_eq!(MaterialId::BrownDye.paint_block(), Some(block::WALLPAPER_BROWN));
        assert_eq!(MaterialId::CyanDye.paint_block(), Some(block::WALLPAPER_CYAN));
        assert_eq!(MaterialId::MagentaDye.paint_block(), Some(block::WALLPAPER_MAGENTA));
        // A non-dye material has no paint colour.
        assert_eq!(MaterialId::Bellows.paint_block(), None);
    }

    #[test]
    fn every_dye_maps_to_a_skin_colour() {
        let reg = crate::block::BlockRegistry::new();
        // The 16 dyes are exactly the ones `paint_block()` maps (item.rs).
        const ALL_DYES: [MaterialId; 16] = [
            MaterialId::WhiteDye,
            MaterialId::BlackDye,
            MaterialId::RedDye,
            MaterialId::BlueDye,
            MaterialId::YellowDye,
            MaterialId::OrangeDye,
            MaterialId::GreenDye,
            MaterialId::PurpleDye,
            MaterialId::PinkDye,
            MaterialId::LimeDye,
            MaterialId::LightBlueDye,
            MaterialId::GreyDye,
            MaterialId::LightGreyDye,
            MaterialId::BrownDye,
            MaterialId::CyanDye,
            MaterialId::MagentaDye,
        ];
        for m in ALL_DYES {
            let c = crate::item::dye_skin_color(m, &reg);
            assert!(c.is_some(), "dye {m:?} must yield a paint colour");
            assert_eq!(c.unwrap()[3], 255, "opaque");
        }
        // A non-dye material yields nothing.
        assert!(crate::item::dye_skin_color(MaterialId::Bellows, &reg).is_none());
    }
}
