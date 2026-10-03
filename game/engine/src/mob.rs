//! Mob type definitions — kind, size, colour, health, category.
//!
//! Definitions live in `data/mobs/*.toml` and are baked into the binary via
//! `include_str!` at compile time. This matches spec 05 §12.2 intent
//! (data-driven mob definitions) while keeping the WASM build self-contained
//! (no runtime file IO on a browser target).
//!
//! Adding a mob:
//! 1. Write `data/mobs/<id>.toml` with the fields below.
//! 2. Add the variant to `MobType` and the `DEFS` include-list.
//! 3. Add the `id → MobType` match in `MobType::from_toml_id`.

use std::sync::LazyLock;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum MobType {
    Cow,
    Chicken,
    Pig,
    Sheep,
    /// Spec 19 — passive NPC. Lives in villages. Right-click opens quest UI.
    Villager,
    /// Spec 19 — rare-spawn drifter that pathfinds to unclaimed Village Bells
    /// and claims a valid dwelling, converting to a normal Villager.
    /// Renamed from WanderingVillager in Historical Pivot HP-0 (2026-05-22).
    /// On-wire EntityKind discriminant unchanged (positional protocol).
    Peddler,
    /// Spec 28d.wolves — tameable companion mob. Five AI states + bone taming
    /// + owner pubkey on `WolfData` (block-entity-style component fetched from
    ///   `World::wolf_data`). See `wolf.rs` for behaviour. Appended bincode-
    ///   positionally so existing saves keep their indices.
    Wolf,
    /// Spec 28d (chunk 3 of the 2026-05-21 rolling plan). Plains/Savanna
    /// large passive. Eventual rideable (saddle), bred from horse+horse,
    /// donkey+horse → mule. Spawns via biome scatter (2026-05-23); the
    /// signature ride/breed AI is written but not yet dispatched (audit E2).
    /// See `horse_ai.rs`.
    Horse,
    /// Spec 28d. Small passive (Plains/Forest/SnowyTundra). Fast-twitch
    /// hop AI. Drops RawRabbit + RabbitHide (15% chance). Spawns via biome
    /// scatter; the hop AI is written but not yet dispatched (audit E2).
    /// See `rabbit_ai.rs`.
    Rabbit,
    /// Spec 28d (chunk 4). Mountains passive-with-charge. Wanders idly
    /// most of the time but occasionally rams a nearby player or mob
    /// (1 in 600 ticks ≈ once per 30 s) for 1 HP + knockback. Drops 0-2
    /// wool. Spawns via biome scatter; the charge AI is written but not yet
    /// dispatched (audit E2). See `goat_ai.rs`.
    Goat,
    /// Spec 28d (chunk 5). Flying neutral insect. Wanders around its
    /// home hive (chunk 8) pollinating crops. Stung-by-player triggers
    /// an aggressive Sting window; impact drops a single Stinger and
    /// despawns the bee. See `bee_ai.rs`.
    Bee,
    /// Spec 28d (chunk 6). Aquatic passive. Lives in WATER blocks;
    /// drifts on the in-water wander cycle; suffocates when pushed out
    /// of water (3 s grace then 1 HP/tick). Drops InkSac on death.
    /// See `squid_ai.rs`.
    Squid,
    /// Spec 28d.nostrich (2026-05-21). Purple ostrich — the Nostr mascot.
    /// Savanna-only spawn, family groups of 2-3. Passive-flee + retaliate-
    /// kick AI; sprint-faster speed (8.0 b/s). Tameable via berries.
    /// Tamed: lays a NostrichEgg every 24,000 ticks (1 in-game day) +
    /// sheds a NostrichFeather every 48,000 ticks. KILLING ONE TRIGGERS
    /// "The Nostrich's Vow" — a 24,000-tick curse blocking vendor trade,
    /// village reputation, and sats payouts on the killer. Purify via
    /// a Nostrich Memorial (3×3 purple wool + 3 feathers + 1 egg).
    /// 1 NostrichEgg ≈ 21 chicken eggs (the Bitcoin nod).
    Nostrich,
    /// Historical Pivot Sub-Foundation 2 (HP-2, 2026-05-22). Forest /
    /// Taiga territorial omnivore. Smells food (mature crops + food-
    /// stocked Chests) within 12 blocks and walks to take it.
    /// Charges at 6 b/s when hit. Drops 1-3 Bone + 0-1 Leather. See
    /// `bear_ai.rs`.
    Bear,
    /// HP-2. Savanna pack hunter. Lazy by day; hunts at night. Pack
    /// of 2-4 hyenas spawn together; ≥3 within 6 blocks triggers
    /// PackBoost (+1 damage each). Drops 1-2 Bone. See `hyena_ai.rs`.
    Hyena,
    /// HP-3 (2026-05-23). Human bandit, T1 of the brigand ladder. HP 16,
    /// patrols 16-block radius around home Brigand Hideout by day,
    /// chases nearest player at night, flees back to hideout when
    /// HP < 25 %. Drops 0-1 wool/bread/iron. See `brigand.rs` for the
    /// tier-tunable AI hooks (`BrigandTier` + `HomeHideout` components).
    Brigand,
    /// HP-3. Mid-tier human raider. HP 28, 24-block detect range,
    /// never flees, otherwise mirrors Brigand AI. Drops 1-2 iron + 0-1
    /// bread + 0-1 leather. Marks the captain of each fresh hideout
    /// in the spawn order.
    Marauder,
    /// HP-3. Top-tier human raider — boss of a "rare" hideout. HP 45,
    /// 32-block detect range, **always aggressive** (day and night),
    /// never flees. Drops 2-3 iron + 1 BrigandChieftainTrophy + 0-1
    /// leather. Trophy is reserved by HP-1 — this is the kill source.
    Berserker,
    /// HP-4 (2026-05-23). Human village defender — replaces the Iron
    /// Golem in the historical pivot. HP 60, melee damage 8, walks at
    /// 4 b/s. Auto-spawns alongside the Iron Golem when a village hits
    /// the Spec 19 threshold (≥3 villagers + ≥5 houses); HP-6 retires
    /// the golem leaving the Knight as the sole defender. Reuses the
    /// existing `AiState::GolemGuard` patrol + `tick_golem_combat`
    /// engagement loop via the generalised `is_village_defender`
    /// helper. Drops 2-3 IronIngot + 0-1 Leather.
    Knight,
    /// Six-wave Wave 2 (aquatic) — schooling prey fish. Flees predators +
    /// players, panic propagates to the school. Hand-catchable; the prey layer
    /// the Shark hunts. Drops RawFish.
    Fish,
    /// Aquatic apex predator (deep ocean only — telegraphed + avoidable). Hunts
    /// Fish + wounded prey; drips a Shark Tooth when it ATTACKS prey (non-lethal,
    /// husbandry-over-slaughter). Makes the Ocean biome carry real stakes.
    Shark,
    /// Bioluminescent deep-water/cave squid. Drips Glow Ink when calm → glow
    /// décor + bio-lamp light recipes.
    GlowSquid,
    /// Wild fauna wave — skittish forest/taiga animal. Flees; drops a pelt
    /// (Leather). Tameable + rabbit/chicken-hunting are follow-up polish.
    Fox,
    /// Tundra apex threat (extends the predator theme). Hostile chase AI; drops
    /// Bone + Leather.
    PolarBear,
    /// Tundra herd animal. Passive wander + flee; drops Leather + venison
    /// (RawBeef). Cold-biome draft/sled use is a follow-up.
    Reindeer,
    /// Companions wave — tameable cat (raw fish). Follows its owner; wards off
    /// threats. Generic CompanionData taming.
    Cat,
    /// Companions wave — tameable parrot (seeds). Follows its owner; (flight +
    /// threat-alarm are follow-up polish).
    Parrot,
    /// Logistics wave — rideable draft animal (half a horse's flair). Drops
    /// Leather. Chest-carry is a follow-up.
    Donkey,
    /// Logistics wave — rideable Donkey×Horse cross (the breeding-genetics
    /// showcase). Drops Leather.
    Mule,
    /// Pets wave Task 13 — coastal/shoreline passive. Spawns on SAND near
    /// sea level (see [`spawn_surface_ok`]). Flees when attacked. Drops a
    /// Crab Claw, the input to the Reach Claw build tool. APPENDED LAST —
    /// new variants must stay at the end (discriminant order is the wire/
    /// save encoding).
    Crab,
}

impl MobType {
    /// Safe name → mob lookup (snake_case ids). Single source of truth for the
    /// id↔MobType map; returns `None` for an unknown id. Used by authoring
    /// commands (`/place donkey`, #129) and the TOML loader (which wraps it with
    /// a panic). A couple of friendly no-underscore aliases are accepted too.
    pub fn from_name(id: &str) -> Option<MobType> {
        Some(match id {
            "cow" => MobType::Cow,
            "chicken" => MobType::Chicken,
            "pig" => MobType::Pig,
            "sheep" => MobType::Sheep,
            "villager" => MobType::Villager,
            "peddler" => MobType::Peddler,
            "wolf" => MobType::Wolf,
            "horse" => MobType::Horse,
            "rabbit" => MobType::Rabbit,
            "goat" => MobType::Goat,
            "bee" => MobType::Bee,
            "squid" => MobType::Squid,
            "nostrich" => MobType::Nostrich,
            "bear" => MobType::Bear,
            "hyena" => MobType::Hyena,
            "brigand" => MobType::Brigand,
            "marauder" => MobType::Marauder,
            "berserker" => MobType::Berserker,
            "knight" => MobType::Knight,
            "fish" => MobType::Fish,
            "shark" => MobType::Shark,
            "glow_squid" | "glowsquid" => MobType::GlowSquid,
            "fox" => MobType::Fox,
            "polar_bear" | "polarbear" => MobType::PolarBear,
            "reindeer" => MobType::Reindeer,
            "cat" => MobType::Cat,
            "parrot" => MobType::Parrot,
            "donkey" => MobType::Donkey,
            "mule" => MobType::Mule,
            "crab" => MobType::Crab,
            _ => return None,
        })
    }

    fn from_toml_id(id: &str) -> Self {
        Self::from_name(id)
            .unwrap_or_else(|| panic!("unknown mob id in data/mobs/*.toml: {id}"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MobCategory {
    Passive,
    Hostile,
}

impl MobCategory {
    fn from_str(s: &str) -> Self {
        match s {
            "passive" => MobCategory::Passive,
            "hostile" => MobCategory::Hostile,
            other => panic!("unknown category in data/mobs/*.toml: {other}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct MobDef {
    pub mob_type: MobType,
    pub category: MobCategory,
    pub width: f32,
    pub height: f32,
    /// Species coat tint, from the per-mob TOML. LIVE since 2026-07-11
    /// (mob-species-tint foundation): `texture_gen::generate_textures` bakes
    /// per-species tinted copies of the donor coat layers for every
    /// mesh-reuse species in `texture_gen::SPECIES_TINTS` (Fox, Cat, Crab,
    /// Polar Bear, Reindeer, Donkey/Mule, Parrot, Glow Squid), and
    /// `entity_model::retint` points their models at the tinted run. The
    /// villager-tier humans (Brigand/Marauder/Berserker/Knight) still ignore
    /// it — they get a bespoke armoured mesh post-playtest (layer budget:
    /// the tint run must stay under the 512-layer device floor).
    pub color: [f32; 3],
    pub health: u16,
    /// Movement speed in blocks/second.
    pub speed: f32,
    pub name: String,
}

/// Prey animals that bolt when a player strikes them (P2 gap-closure — the
/// "animals run when you hit them" reflex the live AI was missing). Deliberately
/// an allow-list rather than `category == Passive`: it excludes the tamed Wolf
/// companion, the Villager/Peddler NPCs (own grace window), the Bee (stings),
/// and the Bear/Hyena (which aggress instead of flee).
pub fn flees_when_attacked(kind: MobType) -> bool {
    matches!(
        kind,
        MobType::Cow
            | MobType::Chicken
            | MobType::Pig
            | MobType::Sheep
            | MobType::Horse
            | MobType::Rabbit
            | MobType::Goat
            | MobType::Squid
            | MobType::Nostrich
            | MobType::Fish
            | MobType::GlowSquid
            | MobType::Fox
            | MobType::Reindeer
            | MobType::Donkey
            | MobType::Mule
            | MobType::Crab
    )
}

/// Mounts the player can ride + steer (Logistics wave adds Donkey + Mule to the
/// Horse). Used by the riding mount-trigger + the steering gate.
pub fn is_rideable(kind: MobType) -> bool {
    // Horse family = steady gallop; the Nostrich is the wacky speed-building,
    // drifting, water-skimming mount (see `nostrich_ride`).
    matches!(
        kind,
        MobType::Horse | MobType::Donkey | MobType::Mule | MobType::Nostrich
    )
}

/// The true horse family — Horse + the draft animals Donkey + Mule. They share
/// the herd-wander idle AI (`HorseData`) and the gallop ride. The Nostrich is
/// rideable but is NOT horse-family — it has its own arcade physics + AI.
pub fn is_horse_family(kind: MobType) -> bool {
    matches!(kind, MobType::Horse | MobType::Donkey | MobType::Mule)
}

/// Whether `kind` rides with the Nostrich's arcade physics (vs the horse
/// gallop). Live mount-dispatch code (game_loop.rs) checks
/// `MobType::Nostrich` directly at each call site instead of through this
/// helper.
#[allow(dead_code)]
pub fn is_nostrich_mount(kind: MobType) -> bool {
    matches!(kind, MobType::Nostrich)
}

/// Task 11 (2026-07-06) — the draft animals that can be equipped with a
/// cargo pack (a Chest strapped to a KEPT steed, opened with sneak +
/// empty-hand right-click). The Horse is excluded — it's the pure-mount of
/// the family; only the beasts of burden carry freight.
pub fn can_carry_pack(kind: MobType) -> bool {
    matches!(kind, MobType::Donkey | MobType::Mule)
}

/// Per-species flee-speed multiplier over base walk speed. Rabbits are extra
/// skittish (bolt fast); horse/nostrich leg it at a quick trot; the rest at a
/// steady boosted pace.
pub fn flee_speed_mult(kind: MobType) -> f32 {
    match kind {
        MobType::Rabbit => 1.8,
        MobType::Horse | MobType::Nostrich => 1.5,
        _ => 1.3,
    }
}

/// Wire shape for `data/mobs/*.toml`. Deserialised + converted to MobDef at
/// startup.
#[derive(Deserialize)]
struct RawMobDef {
    id: String,
    category: String,
    width: f32,
    height: f32,
    color: [f32; 3],
    health: u16,
    speed: f32,
    name: String,
}

impl RawMobDef {
    fn resolve(self) -> MobDef {
        MobDef {
            mob_type: MobType::from_toml_id(&self.id),
            category: MobCategory::from_str(&self.category),
            width: self.width,
            height: self.height,
            color: self.color,
            health: self.health,
            speed: self.speed,
            name: self.name,
        }
    }
}

/// Compiled-in mob definition files. Order defines `MOBS` iteration order.
const DEFS: &[(&str, &str)] = &[
    ("cow", include_str!("../data/mobs/cow.toml")),
    ("chicken", include_str!("../data/mobs/chicken.toml")),
    ("pig", include_str!("../data/mobs/pig.toml")),
    ("sheep", include_str!("../data/mobs/sheep.toml")),
    ("villager", include_str!("../data/mobs/villager.toml")),
    ("peddler", include_str!("../data/mobs/peddler.toml")),
    ("wolf", include_str!("../data/mobs/wolf.toml")),
    ("horse", include_str!("../data/mobs/horse.toml")),
    ("rabbit", include_str!("../data/mobs/rabbit.toml")),
    ("goat", include_str!("../data/mobs/goat.toml")),
    ("bee", include_str!("../data/mobs/bee.toml")),
    ("squid", include_str!("../data/mobs/squid.toml")),
    ("nostrich", include_str!("../data/mobs/nostrich.toml")),
    ("bear", include_str!("../data/mobs/bear.toml")),
    ("hyena", include_str!("../data/mobs/hyena.toml")),
    ("brigand", include_str!("../data/mobs/brigand.toml")),
    ("marauder", include_str!("../data/mobs/marauder.toml")),
    ("berserker", include_str!("../data/mobs/berserker.toml")),
    ("knight", include_str!("../data/mobs/knight.toml")),
    ("fish", include_str!("../data/mobs/fish.toml")),
    ("shark", include_str!("../data/mobs/shark.toml")),
    ("glow_squid", include_str!("../data/mobs/glow_squid.toml")),
    ("fox", include_str!("../data/mobs/fox.toml")),
    ("polar_bear", include_str!("../data/mobs/polar_bear.toml")),
    ("reindeer", include_str!("../data/mobs/reindeer.toml")),
    ("cat", include_str!("../data/mobs/cat.toml")),
    ("parrot", include_str!("../data/mobs/parrot.toml")),
    ("donkey", include_str!("../data/mobs/donkey.toml")),
    ("mule", include_str!("../data/mobs/mule.toml")),
    ("crab", include_str!("../data/mobs/crab.toml")),
];

/// All mob definitions, loaded once at first access from the baked-in TOML.
pub static MOBS: LazyLock<Vec<MobDef>> = LazyLock::new(|| {
    DEFS.iter()
        .map(|(file, contents)| {
            toml::from_str::<RawMobDef>(contents)
                .unwrap_or_else(|e| panic!("parse data/mobs/{file}.toml: {e}"))
                .resolve()
        })
        .collect()
});

/// O(1) lookup table indexed by `MobType as usize`. Replaces a linear
/// `MOBS.iter().find()` that ran per-mob-per-tick across mob AI / entity diff /
/// combat (engine audit 2026-06-04, B). Built once from `MOBS`; a discriminant
/// with no `MobDef` stays `None` so `mob_def` keeps its panic-on-missing.
static MOB_BY_TYPE: LazyLock<Vec<Option<&'static MobDef>>> = LazyLock::new(|| {
    let max = MOBS.iter().map(|m| m.mob_type as usize).max().unwrap_or(0);
    let mut table: Vec<Option<&'static MobDef>> = vec![None; max + 1];
    for m in MOBS.iter() {
        table[m.mob_type as usize] = Some(m);
    }
    table
});

/// Look up a mob definition by type. O(1) via [`MOB_BY_TYPE`].
pub fn mob_def(kind: MobType) -> &'static MobDef {
    MOB_BY_TYPE
        .get(kind as usize)
        .and_then(|slot| *slot)
        .unwrap_or_else(|| panic!("mob_def: no definition for {kind:?}"))
}

/// Per-biome spawn-weight table. Returns the **passive-mob** roster for a
/// biome (hostiles spawn at night via the existing `tick_mob_spawning`
/// table, not via this biome-keyed pass). Each entry's `weight` is on a
/// 0..=100 scale — caller normalises before sampling.
///
/// Pure: no world reads. The live consumer is the future biome-aware
/// passive spawn pass; today only `/spawn` triggers passive mob entries.
pub fn biome_passive_spawn_weights(biome: crate::biome::Biome) -> Vec<(MobType, u8)> {
    use crate::biome::Biome;
    match biome {
        Biome::Plains => vec![
            (MobType::Cow, 20),
            (MobType::Sheep, 16),
            (MobType::Pig, 15),
            (MobType::Chicken, 15),
            (MobType::Horse, 10),
            (MobType::Bee, 8),
            (MobType::Donkey, 8),
            (MobType::Rabbit, 5),
            // Logistics — wild mules are rare (mostly a bred Donkey×Horse cross).
            (MobType::Mule, 3),
        ],
        Biome::Forest => vec![
            (MobType::Cow, 15),
            (MobType::Sheep, 15),
            (MobType::Pig, 15),
            (MobType::Chicken, 10),
            (MobType::Wolf, 10),
            (MobType::Rabbit, 15),
            (MobType::Bee, 15),
            (MobType::Horse, 5),
            // Wild fauna wave — foxes in the forest (they'll hunt the rabbits
            // + chickens once predator-prey AI lands).
            (MobType::Fox, 8),
            // HP-2 — Bears spawn in Forest at weight 3 (rare ambient
            // threat). Per the spec these aren't passive but the biome
            // pool is the only registration surface today; the actual
            // hostile spawner reads from this list to seed Forest mobs.
            (MobType::Bear, 3),
        ],
        Biome::BirchForest => vec![
            (MobType::Cow, 20),
            (MobType::Sheep, 15),
            (MobType::Wolf, 15),
            (MobType::Rabbit, 20),
            (MobType::Chicken, 10),
        ],
        Biome::Taiga => vec![
            (MobType::Wolf, 30),
            (MobType::Sheep, 15),
            (MobType::Rabbit, 25),
            (MobType::Chicken, 5),
            // HP-2 — Bears in Taiga at weight 3 (snowy bears too).
            (MobType::Bear, 3),
            // Wild fauna wave — foxes + reindeer roam the taiga.
            (MobType::Fox, 12),
            (MobType::Reindeer, 8),
        ],
        Biome::SnowyTundra => vec![
            (MobType::Rabbit, 35),
            (MobType::Wolf, 25),
            (MobType::Reindeer, 22),
            (MobType::Sheep, 10),
            // Wild fauna wave — the tundra apex threat (sparse).
            (MobType::PolarBear, 6),
        ],
        Biome::Savanna => vec![
            (MobType::Horse, 35),
            (MobType::Cow, 20),
            (MobType::Sheep, 10),
            (MobType::Chicken, 10),
            // Spec 28d.nostrich — purple ostrich, family groups of 2-3.
            // Rare-but-present on the Savanna; horses still dominate.
            (MobType::Nostrich, 5),
            // HP-2 — Savanna pack hunter. Weight 5 → roughly hyenas
            // appear once for every two cohorts of cows / nostriches.
            (MobType::Hyena, 5),
            // Logistics — donkeys roam the savanna alongside horses.
            (MobType::Donkey, 10),
        ],
        Biome::Jungle => vec![
            (MobType::Chicken, 25),
            (MobType::Pig, 20),
            (MobType::Parrot, 15),
            (MobType::Cat, 10),
            (MobType::Rabbit, 5),
        ],
        Biome::Desert => vec![
            (MobType::Rabbit, 30),
        ],
        // Mountains + Ocean — sparse passive mob density.
        Biome::Mountains => vec![
            (MobType::Goat, 30),
            (MobType::Sheep, 20),
            (MobType::Rabbit, 10),
        ],
        Biome::Ocean => vec![
            // Aquatic wave — schooling Fish dominate; Squid + Glow Squid drift;
            // the Shark is the rare apex predator (deep water → real stakes).
            (MobType::Fish, 50),
            (MobType::Squid, 30),
            (MobType::GlowSquid, 12),
            (MobType::Shark, 8),
            // Pets wave Task 13 — Crab is a coastal/shoreline spawn, gated to
            // SAND near sea level by `spawn_surface_ok` at the accept point
            // (`entity::scatter_mobs_in_column`), not the seabed the rest of
            // this Ocean roster spawns on.
            (MobType::Crab, 10),
        ],
    }
}

/// Drop table: what items a mob produces when killed. Counts are seeded by
/// `seed` (typically derived from world time + entity position) so the same
/// kill in the same tick is reproducible across save/replay paths.
///
/// Numbers chosen to mirror Minecraft baseline drops (no Looting / no Fire
/// — those layer on later when enchantments + cooked variants ship).
pub fn drops_for(kind: MobType, seed: u32) -> Vec<crate::item::ItemStack> {
    use crate::item::{ItemStack, MaterialId};

    fn roll(seed: u32, salt: u32, max_inclusive: u32) -> u32 {
        let mut h = seed.wrapping_mul(374761393) ^ salt.wrapping_mul(668265263);
        h = (h ^ (h >> 13)).wrapping_mul(1274126177);
        (h ^ (h >> 16)) % (max_inclusive + 1)
    }

    fn material(id: MaterialId, count: u32) -> ItemStack {
        ItemStack::new_material(id, count.max(1) as u8)
    }

    match kind {
        MobType::Cow => {
            let mut drops = Vec::new();
            let leather = roll(seed, 1, 2);             // 0-2
            if leather > 0 { drops.push(material(MaterialId::Leather, leather)); }
            let beef = 1 + roll(seed, 2, 2);            // 1-3
            drops.push(material(MaterialId::RawBeef, beef));
            // Phase D: skeletons are gone, so livestock re-source Bone for
            // the Bone -> Bonemeal farming path. ~12.5% chance (1 of 8).
            if roll(seed, 30, 7) == 0 { drops.push(material(MaterialId::Bone, 1)); }
            drops
        }
        MobType::Pig => {
            let mut drops = Vec::new();
            let porkchop = 1 + roll(seed, 3, 2);        // 1-3
            drops.push(material(MaterialId::RawPorkchop, porkchop));
            // Phase D — see Cow. ~12.5% chance (1 of 8).
            if roll(seed, 31, 7) == 0 { drops.push(material(MaterialId::Bone, 1)); }
            drops
        }
        MobType::Sheep => {
            let mut drops = vec![material(MaterialId::Wool, 1)];
            let mutton = 1 + roll(seed, 4, 1);          // 1-2
            drops.push(material(MaterialId::RawMutton, mutton));
            // Phase D — see Cow. ~12.5% chance (1 of 8).
            if roll(seed, 32, 7) == 0 { drops.push(material(MaterialId::Bone, 1)); }
            drops
        }
        MobType::Chicken => {
            let mut drops = Vec::new();
            let feathers = roll(seed, 5, 2);            // 0-2
            if feathers > 0 { drops.push(material(MaterialId::Feather, feathers)); }
            drops.push(material(MaterialId::RawChicken, 1));
            drops
        }
        // Villagers drop nothing on death — Spec 19 anti-farming choice. The
        // reputation hit + 30s damage cooldown discourage killing for loot.
        MobType::Villager => vec![],
        // Wandering Villagers likewise drop nothing.
        MobType::Peddler => vec![],
        // Spec 28d.wolves — drops depend on tame status. The seed-based
        // table here covers the untamed case (leather + 1-2 bones); the
        // tamed-wolf emotional-loss rule (no drops) is enforced upstream
        // via `wolf::drops_for_wolf` once `WolfData` is queryable from the
        // mob ECS. Until that wiring lands, falling through to this arm
        // produces the untamed table — safe default since the only wolves
        // in the world right now are spawn-time wolves (no tame state).
        MobType::Wolf => {
            let bones = 1 + roll(seed, 13, 1); // 1-2
            vec![
                material(MaterialId::Leather, 1),
                material(MaterialId::Bone, bones),
            ]
        }
        // Spec 28d — Horse passive. Drops 0-2 leather. No meat (Minecraft
        // parity — horses are mounts, not livestock).
        MobType::Horse => {
            let leather = roll(seed, 14, 2);
            if leather > 0 { vec![material(MaterialId::Leather, leather)] } else { vec![] }
        }
        // Spec 28d — Rabbit. Always 1 raw rabbit. 15% chance of hide
        // (deterministic per-seed roll: 0 of 6 outcomes triggers hide).
        MobType::Rabbit => {
            let mut drops = vec![material(MaterialId::RawRabbit, 1)];
            let hide_roll = roll(seed, 15, 6);
            if hide_roll == 0 {
                drops.push(material(MaterialId::RabbitHide, 1));
            }
            drops
        }
        // Spec 28d — Goat. 0-2 wool (Minecraft parity — goats yield
        // wool-equivalent fibre but no goat-specific raw meat at v1).
        MobType::Goat => {
            let wool = roll(seed, 16, 2);
            if wool > 0 { vec![material(MaterialId::Wool, wool)] } else { vec![] }
        }
        // Spec 28d — Bee. On death drops nothing by default (Minecraft
        // parity — bees only "die" after stinging, which is what
        // produces the Stinger drop via the bee_ai impact path, not
        // this generic drops_for table). Killed-by-player bees produce
        // a single Stinger here for completeness.
        MobType::Bee => {
            vec![material(MaterialId::BeeStinger, 1)]
        }
        // Spec 28d — Squid. 1-3 InkSacs (Minecraft parity).
        MobType::Squid => {
            let ink = 1 + roll(seed, 17, 2);
            vec![material(MaterialId::InkSac, ink)]
        }
        // Aquatic wave — Fish drop their catch; Sharks a tooth (the behaviour-
        // gated drip happens in the predator AI; this is the on-kill fallback);
        // Glow Squid their glow ink.
        MobType::Fish => vec![material(MaterialId::RawFish, 1)],
        MobType::Shark => vec![material(MaterialId::SharkTooth, 1 + roll(seed, 30, 1))],
        MobType::GlowSquid => {
            let ink = 1 + roll(seed, 31, 2);
            vec![material(MaterialId::GlowInk, ink)]
        }
        // Wild fauna wave — pelts + venison, reusing existing materials.
        MobType::Fox => {
            let pelt = roll(seed, 32, 1); // 0-1 Leather (pelt)
            if pelt > 0 { vec![material(MaterialId::Leather, pelt)] } else { vec![] }
        }
        MobType::PolarBear => {
            let mut drops = vec![material(MaterialId::Bone, 1 + roll(seed, 33, 2))]; // 1-3
            let leather = roll(seed, 34, 1);
            if leather > 0 { drops.push(material(MaterialId::Leather, leather)); }
            drops
        }
        MobType::Reindeer => {
            let mut drops = Vec::new();
            let leather = roll(seed, 35, 2); // 0-2
            if leather > 0 { drops.push(material(MaterialId::Leather, leather)); }
            drops.push(material(MaterialId::RawBeef, 1 + roll(seed, 36, 1))); // 1-2 venison
            drops
        }
        // Companions — pets give nothing on death (you tame them, you don't
        // harvest them); a parrot sheds the odd feather.
        MobType::Cat => vec![],
        MobType::Parrot => {
            let f = roll(seed, 37, 2);
            if f > 0 { vec![material(MaterialId::Feather, f)] } else { vec![] }
        }
        // Logistics — draft animals drop hide (like the Horse).
        MobType::Donkey | MobType::Mule => {
            let l = roll(seed, 38, 2);
            if l > 0 { vec![material(MaterialId::Leather, l)] } else { vec![] }
        }
        // Spec 28d.nostrich — drops on kill. 1-3 feathers (3 max
        // because killed Nostriches give a "tribute" of feathers; less
        // than a kind hand-shed). 0-1 raw nostrich meat. The Vow
        // mechanic that punishes the killer is applied separately
        // by the combat path; this table only enumerates items.
        MobType::Nostrich => {
            let mut drops = Vec::new();
            let feathers = 1 + roll(seed, 21, 2); // 1-3
            drops.push(material(MaterialId::NostrichFeather, feathers));
            let meat = roll(seed, 22, 1); // 0-1
            if meat > 0 {
                drops.push(material(MaterialId::RawNostrichMeat, meat));
            }
            drops
        }
        // HP-2 Bear — 1-3 Bone + 0-1 Leather (puff, no gore).
        MobType::Bear => {
            let mut drops = Vec::new();
            let bones = 1 + roll(seed, 23, 2); // 1-3
            drops.push(material(MaterialId::Bone, bones));
            let leather = roll(seed, 24, 1); // 0-1
            if leather > 0 {
                drops.push(material(MaterialId::Leather, leather));
            }
            drops
        }
        // HP-2 Hyena — 1-2 Bone.
        MobType::Hyena => {
            let bones = 1 + roll(seed, 25, 1); // 1-2
            vec![material(MaterialId::Bone, bones)]
        }
        // HP-3 Brigand — 0-1 each of wool / bread / iron. Scavenger feel.
        MobType::Brigand => {
            let mut drops = Vec::new();
            let wool = roll(seed, 26, 1); // 0-1
            if wool > 0 { drops.push(material(MaterialId::Wool, wool)); }
            let bread = roll(seed, 27, 1); // 0-1
            if bread > 0 { drops.push(material(MaterialId::Bread, bread)); }
            let iron = roll(seed, 28, 1); // 0-1
            if iron > 0 { drops.push(material(MaterialId::IronIngot, iron)); }
            drops
        }
        // HP-3 Marauder — 1-2 iron + 0-1 bread + 0-1 leather + 10 %
        // chance of a random Chainmail armour piece (slot rolled per
        // kill). Chainmail dormancy fix (2026-05-28, Fantasy-Excision
        // follow-up): the tier had no live source post-Spec-24-fantasy-
        // excision and was effectively dead. Re-sourcing it here keeps
        // the in-game ladder honest (Leather → Iron / Chainmail →
        // Diamond → Satori) without needing a new mob — a Marauder is
        // narratively the right "mailed brigand-captain" drop.
        // Calibration: ~10 % per kill × random-slot means roughly 40
        // kills for a full set vs ~16 for an Iron set crafted from the
        // Marauder's own iron drops; Chainmail's rarer-than-Iron
        // pedigree comes out right.
        MobType::Marauder => {
            let mut drops = Vec::new();
            let iron = 1 + roll(seed, 29, 1); // 1-2
            drops.push(material(MaterialId::IronIngot, iron));
            let bread = roll(seed, 30, 1); // 0-1
            if bread > 0 { drops.push(material(MaterialId::Bread, bread)); }
            let leather = roll(seed, 31, 1); // 0-1
            if leather > 0 { drops.push(material(MaterialId::Leather, leather)); }
            if roll(seed, 40, 9) == 0 {
                // 1-in-10 — random slot per kill.
                let slot = match roll(seed, 41, 3) {
                    0 => crate::armour::ArmourSlot::Helmet,
                    1 => crate::armour::ArmourSlot::Chestplate,
                    2 => crate::armour::ArmourSlot::Leggings,
                    _ => crate::armour::ArmourSlot::Boots,
                };
                drops.push(ItemStack::new_armour(
                    slot,
                    crate::armour::ArmourMaterial::Chainmail,
                ));
            }
            drops
        }
        // HP-3 Berserker — 2-3 iron + 1 BrigandChieftainTrophy + 0-1 leather.
        // Trophy is the deterministic "kill the boss" reward; 100 % drop.
        MobType::Berserker => {
            let mut drops = Vec::new();
            let iron = 2 + roll(seed, 32, 1); // 2-3
            drops.push(material(MaterialId::IronIngot, iron));
            drops.push(material(MaterialId::BrigandChieftainTrophy, 1));
            let leather = roll(seed, 33, 1); // 0-1
            if leather > 0 { drops.push(material(MaterialId::Leather, leather)); }
            drops
        }
        // HP-4 Knight — village defender; 2-3 iron + 0-1 leather.
        MobType::Knight => {
            let mut drops = Vec::new();
            let iron = 2 + roll(seed, 34, 1); // 2-3
            drops.push(material(MaterialId::IronIngot, iron));
            let leather = roll(seed, 35, 1); // 0-1
            if leather > 0 { drops.push(material(MaterialId::Leather, leather)); }
            drops
        }
        // Pets wave Task 13 — Crab. 1-2 Crab Claws (mirrors the Shark Tooth
        // roll idiom: always at least one, occasionally two).
        MobType::Crab => vec![material(MaterialId::CrabClaw, 1 + roll(seed, 50, 1))],
    }
}

/// Pets wave Task 13 — per-species spawn-surface gate, checked at the
/// scatter accept point (`entity::scatter_mobs_in_column`) after a species
/// has been picked from the biome's weight table. Defaults `true` for every
/// existing mob (their placement is already governed by the caller's
/// surface/biome logic); Crab additionally requires a SAND surface within
/// 3 blocks of sea level (a shoreline, not the open seabed the rest of the
/// Ocean roster spawns on).
pub fn spawn_surface_ok(kind: MobType, surface_block: crate::block::BlockId, y: i32) -> bool {
    match kind {
        MobType::Crab => {
            surface_block == crate::block::SAND
                && (crate::biome::SEA_LEVEL - 3..=crate::biome::SEA_LEVEL + 3).contains(&y)
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_mob_tomls_load() {
        assert_eq!(MOBS.len(), 30, "expected 30 mobs: 19 post-excision + aquatic (Fish/Shark/Glow Squid) + wild fauna (Fox/Polar Bear/Reindeer) + companions (Cat/Parrot) + logistics (Donkey/Mule) + Crab");
        for kind in [
            MobType::Cow, MobType::Chicken,
            MobType::Pig, MobType::Sheep,
            MobType::Villager, MobType::Peddler,
            MobType::Wolf, MobType::Horse, MobType::Rabbit, MobType::Goat,
            MobType::Bee, MobType::Squid,
            MobType::Nostrich, MobType::Bear, MobType::Hyena,
            MobType::Brigand, MobType::Marauder, MobType::Berserker,
            MobType::Knight,
        ] {
            let def = mob_def(kind);
            assert_eq!(def.mob_type, kind);
            assert!(def.width > 0.0 && def.height > 0.0);
            assert!(def.health > 0);
            assert!(!def.name.is_empty());
        }
    }

    #[test]
    fn mob_def_o1_lookup_matches_linear_scan_for_every_mob() {
        // The O(1) MOB_BY_TYPE index must return the exact same MobDef the old
        // linear scan did, for every defined mob (engine audit 2026-06-04, B).
        for m in MOBS.iter() {
            let via_index = mob_def(m.mob_type);
            let via_scan = MOBS.iter().find(|d| d.mob_type == m.mob_type).unwrap();
            assert!(std::ptr::eq(via_index, via_scan),
                "O(1) lookup != linear scan for {:?}", m.mob_type);
        }
    }

    #[test]
    fn wolf_baseline_stats() {
        let w = mob_def(MobType::Wolf);
        assert_eq!(w.category, MobCategory::Passive,
            "untamed wolves are categorised passive at the data level; combat behaviour gates on tame state, not category");
        assert_eq!(w.health, 20, "wolf has player-equivalent HP");
        assert!(w.height < 1.0, "wolf is shorter than a player");
        assert_eq!(w.name, "Wolf");
    }

    #[test]
    fn wolf_drops_leather_and_one_or_two_bones() {
        // Untamed-wolf seed table — the tamed-empty case is enforced in
        // `wolf::drops_for_wolf` once tame state is wired through.
        for seed in 0..200 {
            let drops = drops_for(MobType::Wolf, seed);
            assert_eq!(drops.len(), 2);
            assert!(matches!(drops[0].item, crate::item::Item::Material(crate::item::MaterialId::Leather)));
            let bone_count = drops[1].count;
            assert!(bone_count == 1 || bone_count == 2);
        }
    }

    #[test]
    fn villager_is_passive_with_human_proportions() {
        let v = mob_def(MobType::Villager);
        assert_eq!(v.category, MobCategory::Passive);
        assert_eq!(v.health, 20);
        assert!(v.height > 1.5, "villager is bipedal ({})", v.height);
        assert_eq!(v.name, "Villager");
    }

    #[test]
    fn peddler_is_passive_villager_baseline() {
        let w = mob_def(MobType::Peddler);
        assert_eq!(w.category, MobCategory::Passive);
        assert_eq!(w.health, 20);
        assert!((w.speed - mob_def(MobType::Villager).speed).abs() < 0.5,
            "peddler speed should be near the standard villager");
    }

    #[test]
    fn peddler_mob_loads_from_renamed_toml() {
        let def = mob_def(MobType::Peddler);
        assert!(!def.name.is_empty(), "Peddler def should have loaded from data/mobs/peddler.toml");
        assert_eq!(def.name, "Peddler");
    }

    #[test]
    fn villager_and_peddler_drop_nothing() {
        for seed in 0u32..50 {
            assert!(drops_for(MobType::Villager, seed).is_empty(),
                "villager seed {seed} dropped items");
            assert!(drops_for(MobType::Peddler, seed).is_empty(),
                "peddler seed {seed} dropped items");
        }
    }

    #[test]
    fn cow_is_passive() {
        let c = mob_def(MobType::Cow);
        assert_eq!(c.category, MobCategory::Passive);
    }

    #[test]
    fn pig_and_sheep_are_passive() {
        assert_eq!(mob_def(MobType::Pig).category, MobCategory::Passive);
        assert_eq!(mob_def(MobType::Sheep).category, MobCategory::Passive);
    }

    // --- drop tables ---

    #[test]
    fn cow_drops_at_least_one_beef() {
        // Cow always drops 1-3 beef, regardless of leather roll.
        for seed in 0u32..50 {
            let drops = drops_for(MobType::Cow, seed);
            let beef_count: u8 = drops.iter().map(|s| match s.item {
                crate::item::Item::Material(crate::item::MaterialId::RawBeef) => s.count,
                _ => 0,
            }).sum();
            assert!(beef_count >= 1 && beef_count <= 3,
                "cow seed {seed}: beef={beef_count}");
        }
    }

    #[test]
    fn sheep_always_drops_one_wool() {
        for seed in 0u32..50 {
            let drops = drops_for(MobType::Sheep, seed);
            assert!(drops.iter().any(|s| matches!(s.item,
                crate::item::Item::Material(crate::item::MaterialId::Wool))),
                "sheep seed {seed}: missing wool");
        }
    }

    #[test]
    fn drops_are_deterministic() {
        // Same kind + seed must give the same drops.
        let a = drops_for(MobType::Pig, 12345);
        let b = drops_for(MobType::Pig, 12345);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.count, y.count);
        }
    }

    #[test]
    fn livestock_can_drop_bone() {
        // Phase D (fantasy-roster excision): skeletons are gone, but farming
        // still needs Bone -> Bonemeal. Cow / Sheep / Pig now each have a
        // modest chance to drop a bone. Sweep enough deterministic seeds to
        // confirm Bone appears in the possible drop set for each.
        for kind in [MobType::Cow, MobType::Sheep, MobType::Pig] {
            let mut saw_bone = false;
            for seed in 0u32..500 {
                let drops = drops_for(kind, seed);
                if drops.iter().any(|s| matches!(s.item,
                    crate::item::Item::Material(crate::item::MaterialId::Bone)))
                {
                    saw_bone = true;
                    break;
                }
            }
            assert!(saw_bone, "{kind:?} should be able to drop Bone across the seed sweep");
        }
    }

    #[test]
    fn livestock_bone_is_zero_or_one() {
        // Bone is a 0-1 drop on livestock — never a stack.
        for kind in [MobType::Cow, MobType::Sheep, MobType::Pig] {
            for seed in 0u32..500 {
                let drops = drops_for(kind, seed);
                let bones: u8 = drops.iter().map(|s| match s.item {
                    crate::item::Item::Material(crate::item::MaterialId::Bone) => s.count,
                    _ => 0,
                }).sum();
                assert!(bones <= 1, "{kind:?} seed {seed}: bone count {bones} > 1");
            }
        }
    }

    // Chunk 3 — Horse + Rabbit ------------------------------------------------

    #[test]
    fn horse_baseline_stats() {
        let h = mob_def(MobType::Horse);
        assert_eq!(h.category, MobCategory::Passive);
        assert_eq!(h.health, 30);
        assert!(h.height > 1.0, "horse is taller than 1 block");
        assert!(h.speed >= 3.0, "horse is faster than most mobs (it's a mount)");
        assert_eq!(h.name, "Horse");
    }

    #[test]
    fn rabbit_baseline_stats() {
        let r = mob_def(MobType::Rabbit);
        assert_eq!(r.category, MobCategory::Passive);
        assert_eq!(r.health, 3, "rabbit is fragile");
        assert!(r.height < 1.0);
        assert!(r.width < 1.0);
        assert_eq!(r.name, "Rabbit");
    }

    #[test]
    fn horse_drops_zero_to_two_leather() {
        for seed in 0u32..50 {
            let drops = drops_for(MobType::Horse, seed);
            // No more than 2 leather total; never any other material.
            let leather: u8 = drops.iter().map(|s| match s.item {
                crate::item::Item::Material(crate::item::MaterialId::Leather) => s.count,
                _ => 0,
            }).sum();
            assert!(leather <= 2, "horse seed {seed}: too many leather ({leather})");
            for s in &drops {
                let ok = matches!(s.item, crate::item::Item::Material(crate::item::MaterialId::Leather));
                assert!(ok, "horse drops only leather; got {:?}", s.item);
            }
        }
    }

    #[test]
    fn rabbit_always_drops_one_raw_rabbit() {
        for seed in 0u32..50 {
            let drops = drops_for(MobType::Rabbit, seed);
            let raw_rabbit_count: u8 = drops.iter().map(|s| match s.item {
                crate::item::Item::Material(crate::item::MaterialId::RawRabbit) => s.count,
                _ => 0,
            }).sum();
            assert_eq!(raw_rabbit_count, 1, "rabbit seed {seed}: expected exactly 1 raw rabbit");
        }
    }

    #[test]
    fn biome_spawn_weights_are_sensible() {
        // Every biome's weights must sum to <=100 (since they're on a
        // 0..=100 scale + non-overlapping). Plains must include Horse.
        // Taiga must include Wolf. SnowyTundra must include Rabbit.
        // Mountains must include Goat.
        use crate::biome::Biome;

        let plains = biome_passive_spawn_weights(Biome::Plains);
        let plains_total: u32 = plains.iter().map(|(_, w)| *w as u32).sum();
        assert!(plains_total <= 100, "Plains weights sum to {plains_total}");
        assert!(plains.iter().any(|(m, _)| matches!(m, MobType::Horse)),
            "Plains must include Horse");

        let taiga = biome_passive_spawn_weights(Biome::Taiga);
        assert!(taiga.iter().any(|(m, _)| matches!(m, MobType::Wolf)),
            "Taiga must include Wolf");

        let tundra = biome_passive_spawn_weights(Biome::SnowyTundra);
        assert!(tundra.iter().any(|(m, _)| matches!(m, MobType::Rabbit)),
            "SnowyTundra must include Rabbit");

        let mountains = biome_passive_spawn_weights(Biome::Mountains);
        assert!(mountains.iter().any(|(m, _)| matches!(m, MobType::Goat)),
            "Mountains must include Goat");

        let ocean = biome_passive_spawn_weights(Biome::Ocean);
        assert!(ocean.iter().any(|(m, _)| matches!(m, MobType::Squid)),
            "Ocean roster should include Squid (chunk 6)");
    }

    #[test]
    fn goat_baseline_stats() {
        let g = mob_def(MobType::Goat);
        assert_eq!(g.category, MobCategory::Passive);
        assert_eq!(g.health, 10);
        assert!(g.height > 1.0 && g.height < 2.0);
        assert_eq!(g.name, "Goat");
    }

    #[test]
    fn bee_baseline_stats() {
        let b = mob_def(MobType::Bee);
        assert_eq!(b.category, MobCategory::Passive);
        assert!(b.height < 0.5);
        assert!(b.width < 0.5);
        assert_eq!(b.name, "Bee");
    }

    #[test]
    fn squid_baseline_stats() {
        let s = mob_def(MobType::Squid);
        assert_eq!(s.category, MobCategory::Passive);
        assert_eq!(s.name, "Squid");
    }

    #[test]
    fn squid_drops_one_to_three_ink_sacs() {
        for seed in 0u32..50 {
            let drops = drops_for(MobType::Squid, seed);
            assert_eq!(drops.len(), 1, "squid drops are one InkSac stack");
            let s = &drops[0];
            assert!(matches!(s.item,
                crate::item::Item::Material(crate::item::MaterialId::InkSac)));
            assert!(s.count >= 1 && s.count <= 3, "ink count must be 1-3, got {}", s.count);
        }
    }

    #[test]
    fn bee_drops_a_stinger() {
        let drops = drops_for(MobType::Bee, 0);
        assert_eq!(drops.len(), 1);
        assert!(matches!(drops[0].item,
            crate::item::Item::Material(crate::item::MaterialId::BeeStinger)));
    }

    #[test]
    fn plains_and_forest_include_bees() {
        use crate::biome::Biome;
        let plains = biome_passive_spawn_weights(Biome::Plains);
        let forest = biome_passive_spawn_weights(Biome::Forest);
        assert!(plains.iter().any(|(m, _)| matches!(m, MobType::Bee)),
            "Plains should include Bees");
        assert!(forest.iter().any(|(m, _)| matches!(m, MobType::Bee)),
            "Forests should include Bees");
    }

    #[test]
    fn goat_drops_zero_to_two_wool() {
        for seed in 0u32..50 {
            let drops = drops_for(MobType::Goat, seed);
            let wool: u8 = drops.iter().map(|s| match s.item {
                crate::item::Item::Material(crate::item::MaterialId::Wool) => s.count,
                _ => 0,
            }).sum();
            assert!(wool <= 2, "goat seed {seed}: too many wool ({wool})");
            for s in &drops {
                assert!(matches!(s.item, crate::item::Item::Material(crate::item::MaterialId::Wool)),
                    "goat drops only wool");
            }
        }
    }

    // ── HP-2 Bear + Hyena drop tables ────────────────────────────

    #[test]
    fn bear_baseline_stats() {
        let b = mob_def(MobType::Bear);
        assert_eq!(b.category, MobCategory::Hostile);
        assert_eq!(b.health, 30, "bear is tanky");
        assert!(b.width >= 1.0 && b.height >= 1.0, "bear is large");
        assert_eq!(b.name, "Bear");
    }

    #[test]
    fn hyena_baseline_stats() {
        let h = mob_def(MobType::Hyena);
        assert_eq!(h.category, MobCategory::Hostile);
        assert_eq!(h.health, 12, "hyena is fragile");
        assert!(h.speed >= 5.0, "hyena is fast");
        assert_eq!(h.name, "Hyena");
    }

    #[test]
    fn bear_drops_one_to_three_bone_and_zero_to_one_leather() {
        let mut min_bone = u8::MAX;
        let mut max_bone = 0u8;
        let mut leather_seen = false;
        let mut no_leather_seen = false;
        for seed in 0u32..400 {
            let drops = drops_for(MobType::Bear, seed);
            let bones: u8 = drops.iter().map(|s| match s.item {
                crate::item::Item::Material(crate::item::MaterialId::Bone) => s.count,
                _ => 0,
            }).sum();
            let leather: u8 = drops.iter().map(|s| match s.item {
                crate::item::Item::Material(crate::item::MaterialId::Leather) => s.count,
                _ => 0,
            }).sum();
            assert!(bones >= 1 && bones <= 3, "bear seed {seed}: bones={bones}");
            assert!(leather <= 1, "bear seed {seed}: leather={leather}");
            if bones < min_bone { min_bone = bones; }
            if bones > max_bone { max_bone = bones; }
            if leather > 0 { leather_seen = true; } else { no_leather_seen = true; }
        }
        assert_eq!(min_bone, 1, "expected some seeds to drop 1 bone");
        assert_eq!(max_bone, 3, "expected some seeds to drop 3 bones");
        assert!(leather_seen, "leather should fire sometimes");
        assert!(no_leather_seen, "leather is 0-1, not always present");
    }

    #[test]
    fn hyena_drops_one_to_two_bone_only() {
        let mut min_bone = u8::MAX;
        let mut max_bone = 0u8;
        for seed in 0u32..200 {
            let drops = drops_for(MobType::Hyena, seed);
            assert_eq!(drops.len(), 1, "hyena drops just bone");
            let bones: u8 = drops.iter().map(|s| match s.item {
                crate::item::Item::Material(crate::item::MaterialId::Bone) => s.count,
                _ => 0,
            }).sum();
            assert!(bones >= 1 && bones <= 2, "hyena seed {seed}: bones={bones}");
            if bones < min_bone { min_bone = bones; }
            if bones > max_bone { max_bone = bones; }
        }
        assert_eq!(min_bone, 1);
        assert_eq!(max_bone, 2);
    }

    #[test]
    fn forest_and_taiga_include_bear_savanna_includes_hyena() {
        use crate::biome::Biome;
        let forest = biome_passive_spawn_weights(Biome::Forest);
        assert!(forest.iter().any(|(m, _)| matches!(m, MobType::Bear)),
            "Forest must include Bear (HP-2)");
        let taiga = biome_passive_spawn_weights(Biome::Taiga);
        assert!(taiga.iter().any(|(m, _)| matches!(m, MobType::Bear)),
            "Taiga must include Bear (HP-2)");
        let savanna = biome_passive_spawn_weights(Biome::Savanna);
        assert!(savanna.iter().any(|(m, _)| matches!(m, MobType::Hyena)),
            "Savanna must include Hyena (HP-2)");
    }

    #[test]
    fn wolf_drop_includes_bone_every_kill() {
        // HP-2 spec — verify the wolf-bone-drop chain is alive (the
        // 2026-05-20 wolf table already promised 1-2 bones; this test
        // is the HP-2 promise that the chain doesn't regress when
        // Sub 6 removes Skeleton spawning).
        for seed in 0u32..200 {
            let drops = drops_for(MobType::Wolf, seed);
            let bones: u8 = drops.iter().map(|s| match s.item {
                crate::item::Item::Material(crate::item::MaterialId::Bone) => s.count,
                _ => 0,
            }).sum();
            assert!(bones >= 1, "wolf seed {seed}: must drop at least one bone");
        }
    }

    // ── HP-3 Brigand / Marauder / Berserker ───────────────────────

    #[test]
    fn brigand_baseline_stats() {
        let b = mob_def(MobType::Brigand);
        assert_eq!(b.category, MobCategory::Hostile);
        assert_eq!(b.health, 16);
        assert!(b.height > 1.5, "brigand is human-bipedal");
        assert_eq!(b.name, "Brigand");
    }

    #[test]
    fn marauder_baseline_stats() {
        let m = mob_def(MobType::Marauder);
        assert_eq!(m.category, MobCategory::Hostile);
        assert_eq!(m.health, 28);
        assert_eq!(m.name, "Marauder");
    }

    #[test]
    fn berserker_baseline_stats() {
        let b = mob_def(MobType::Berserker);
        assert_eq!(b.category, MobCategory::Hostile);
        assert_eq!(b.health, 45, "berserker is the boss tier");
        assert!(b.speed >= 5.0, "berserker is fast");
        assert_eq!(b.name, "Berserker");
    }

    #[test]
    fn brigand_drops_match_table() {
        let mut saw_wool = false;
        let mut saw_bread = false;
        let mut saw_iron = false;
        for seed in 0u32..400 {
            let drops = drops_for(MobType::Brigand, seed);
            for s in &drops {
                assert!(s.count <= 1, "brigand drops are 0-1 each, got {}", s.count);
                use crate::item::{Item, MaterialId};
                match &s.item {
                    Item::Material(MaterialId::Wool) => saw_wool = true,
                    Item::Material(MaterialId::Bread) => saw_bread = true,
                    Item::Material(MaterialId::IronIngot) => saw_iron = true,
                    other => panic!("unexpected brigand drop: {other:?}"),
                }
            }
        }
        assert!(saw_wool, "expected at least one wool drop across 400 seeds");
        assert!(saw_bread, "expected at least one bread drop across 400 seeds");
        assert!(saw_iron, "expected at least one iron drop across 400 seeds");
    }

    #[test]
    fn marauder_always_drops_iron_in_range() {
        for seed in 0u32..200 {
            let drops = drops_for(MobType::Marauder, seed);
            let iron: u8 = drops.iter().map(|s| match s.item {
                crate::item::Item::Material(crate::item::MaterialId::IronIngot) => s.count,
                _ => 0,
            }).sum();
            assert!(iron >= 1 && iron <= 2, "marauder seed {seed}: iron={iron}");
        }
    }

    // ── Chainmail dormancy fix (2026-05-28, fantasy-excision follow-up) ──

    #[test]
    fn marauder_occasionally_drops_chainmail() {
        // Calibrated at 10 % per kill. Over 1000 deterministic seeds
        // the count should sit comfortably around 100 (not exact —
        // hash-driven, so we test a sensible band).
        let mut chainmail_count = 0;
        for seed in 0u32..1000 {
            let drops = drops_for(MobType::Marauder, seed);
            let dropped_chainmail = drops.iter().any(|s| matches!(
                &s.item,
                crate::item::Item::Armour(a)
                    if a.material == crate::armour::ArmourMaterial::Chainmail
            ));
            if dropped_chainmail { chainmail_count += 1; }
        }
        // 70..150 = a generous ±50 % band around the 10 % target so
        // tweaks to the seed/salt math don't false-positive the gate
        // while still locking the "~10 %, not 0 % or 50 %" intent.
        assert!(
            (70..=150).contains(&chainmail_count),
            "expected ~10 % chainmail drop rate over 1000 marauder kills, \
             got {chainmail_count}"
        );
    }

    #[test]
    fn marauder_chainmail_covers_all_four_slots() {
        // Over enough samples every armour slot should appear at
        // least once. Locks the "random slot, not just one" property.
        use crate::armour::ArmourSlot;
        let mut seen = [false; 4];
        for seed in 0u32..2000 {
            let drops = drops_for(MobType::Marauder, seed);
            for s in &drops {
                if let crate::item::Item::Armour(a) = &s.item {
                    if a.material == crate::armour::ArmourMaterial::Chainmail {
                        let i = match a.slot {
                            ArmourSlot::Helmet => 0,
                            ArmourSlot::Chestplate => 1,
                            ArmourSlot::Leggings => 2,
                            ArmourSlot::Boots => 3,
                        };
                        seen[i] = true;
                    }
                }
            }
        }
        assert!(seen.iter().all(|&v| v),
            "expected every slot to appear; got {seen:?}");
    }

    #[test]
    fn brigand_never_drops_chainmail() {
        // Only Marauder is the Chainmail source; Brigand stays a
        // scavenger-only drop table.
        for seed in 0u32..500 {
            let drops = drops_for(MobType::Brigand, seed);
            for s in &drops {
                if let crate::item::Item::Armour(a) = &s.item {
                    assert_ne!(a.material, crate::armour::ArmourMaterial::Chainmail,
                        "brigand seed {seed} dropped Chainmail (should not)");
                }
            }
        }
    }

    #[test]
    fn berserker_always_drops_trophy_and_two_to_three_iron() {
        for seed in 0u32..200 {
            let drops = drops_for(MobType::Berserker, seed);
            let trophy_count: u8 = drops.iter().map(|s| match s.item {
                crate::item::Item::Material(crate::item::MaterialId::BrigandChieftainTrophy) => s.count,
                _ => 0,
            }).sum();
            assert_eq!(trophy_count, 1,
                "berserker seed {seed}: trophy count {trophy_count} (must be exactly 1)");
            let iron: u8 = drops.iter().map(|s| match s.item {
                crate::item::Item::Material(crate::item::MaterialId::IronIngot) => s.count,
                _ => 0,
            }).sum();
            assert!(iron >= 2 && iron <= 3, "berserker seed {seed}: iron={iron}");
        }
    }

    // ── HP-4 Knight ───────────────────────────────────────────────

    #[test]
    fn knight_baseline_stats() {
        let k = mob_def(MobType::Knight);
        assert_eq!(k.category, MobCategory::Passive,
            "Knight is passive-to-players (village defender)");
        assert_eq!(k.health, 60, "Knight HP is 60 (tanky defender, above the 20-HP humans)");
        assert!(k.height > 1.5, "Knight is human-bipedal");
        assert!(k.speed >= 3.0, "Knight walks at human speed, not Golem's plod");
        assert_eq!(k.name, "Knight");
    }

    #[test]
    fn knight_drops_two_to_three_iron_and_zero_to_one_leather() {
        let mut min_iron = u8::MAX;
        let mut max_iron = 0u8;
        let mut leather_seen = false;
        let mut no_leather_seen = false;
        for seed in 0u32..200 {
            let drops = drops_for(MobType::Knight, seed);
            let iron: u8 = drops.iter().map(|s| match s.item {
                crate::item::Item::Material(crate::item::MaterialId::IronIngot) => s.count,
                _ => 0,
            }).sum();
            let leather: u8 = drops.iter().map(|s| match s.item {
                crate::item::Item::Material(crate::item::MaterialId::Leather) => s.count,
                _ => 0,
            }).sum();
            assert!(iron >= 2 && iron <= 3, "knight seed {seed}: iron={iron}");
            assert!(leather <= 1, "knight seed {seed}: leather={leather}");
            if iron < min_iron { min_iron = iron; }
            if iron > max_iron { max_iron = iron; }
            if leather > 0 { leather_seen = true; } else { no_leather_seen = true; }
        }
        assert_eq!(min_iron, 2);
        assert_eq!(max_iron, 3);
        assert!(leather_seen, "leather should fire sometimes");
        assert!(no_leather_seen, "leather is 0-1, not always present");
    }

    #[test]
    fn rabbit_hide_is_a_minority_drop() {
        // 1/6 of seeds should drop hide; over 600 trials we expect ~100
        // hides. Test that we see some but not all.
        let mut hide_count = 0;
        for seed in 0u32..600 {
            let drops = drops_for(MobType::Rabbit, seed);
            if drops.iter().any(|s| matches!(s.item,
                crate::item::Item::Material(crate::item::MaterialId::RabbitHide))) {
                hide_count += 1;
            }
        }
        assert!(hide_count > 50, "expected hide drops across 600 seeds, got {hide_count}");
        assert!(hide_count < 300, "hide drop rate too high ({hide_count}/600)");
    }

    #[test]
    fn only_draft_animals_carry_packs() {
        assert!(can_carry_pack(MobType::Donkey));
        assert!(can_carry_pack(MobType::Mule));
        assert!(!can_carry_pack(MobType::Horse));
    }

    // ── Pets wave Task 13 — Crab ─────────────────────────────────

    #[test]
    fn crab_baseline_stats() {
        let c = mob_def(MobType::Crab);
        assert_eq!(c.category, MobCategory::Passive);
        assert_eq!(c.health, 6);
        assert!(c.width < 1.0 && c.height < 1.0, "crab is small and low");
        assert_eq!(c.name, "Crab");
    }

    #[test]
    fn crab_flees_when_attacked() {
        assert!(flees_when_attacked(MobType::Crab));
    }

    #[test]
    fn crab_drops_one_or_two_crab_claws() {
        for seed in 0u32..200 {
            let drops = drops_for(MobType::Crab, seed);
            assert_eq!(drops.len(), 1, "crab drops just claws");
            let claws: u8 = drops.iter().map(|s| match s.item {
                crate::item::Item::Material(crate::item::MaterialId::CrabClaw) => s.count,
                _ => 0,
            }).sum();
            assert!(claws == 1 || claws == 2, "crab seed {seed}: claws={claws}");
        }
    }

    #[test]
    fn ocean_biome_includes_crab() {
        use crate::biome::Biome;
        let ocean = biome_passive_spawn_weights(Biome::Ocean);
        assert!(ocean.iter().any(|(m, _)| matches!(m, MobType::Crab)),
            "Ocean must include Crab (Pets wave Task 13)");
    }

    #[test]
    fn spawn_surface_ok_gates_crab_to_sand_near_sea_level() {
        use crate::biome::SEA_LEVEL;
        // Sand within the +-3 band → ok.
        assert!(spawn_surface_ok(MobType::Crab, crate::block::SAND, SEA_LEVEL));
        assert!(spawn_surface_ok(MobType::Crab, crate::block::SAND, SEA_LEVEL - 3));
        assert!(spawn_surface_ok(MobType::Crab, crate::block::SAND, SEA_LEVEL + 3));
        // Just outside the band → rejected.
        assert!(!spawn_surface_ok(MobType::Crab, crate::block::SAND, SEA_LEVEL - 4));
        assert!(!spawn_surface_ok(MobType::Crab, crate::block::SAND, SEA_LEVEL + 4));
        // Right Y, wrong surface block → rejected.
        assert!(!spawn_surface_ok(MobType::Crab, crate::block::GRASS, SEA_LEVEL));
        assert!(!spawn_surface_ok(MobType::Crab, crate::block::STONE, SEA_LEVEL));
    }

    #[test]
    fn spawn_surface_ok_defaults_true_for_every_other_mob() {
        use crate::biome::SEA_LEVEL;
        // Wildly wrong surface/Y for a land animal — the default arm never
        // gates non-Crab species (their placement is governed upstream).
        for kind in [
            MobType::Cow, MobType::Wolf, MobType::Fish, MobType::Shark,
            MobType::Squid, MobType::Nostrich, MobType::Donkey,
        ] {
            assert!(spawn_surface_ok(kind, crate::block::STONE, SEA_LEVEL - 40));
        }
    }
}
