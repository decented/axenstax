//! Spec 19 — villager profession + workstation-claim system.
//!
//! A villager spawns as `Profession::None` and walks the world; when they
//! stand within `CLAIM_RANGE` of a recognised workstation block that no other
//! villager has claimed, they bind to that profession. The claim is sticky:
//! the villager stays that profession until the workstation block is
//! destroyed. (Phase 6 will hand the profession off to the quest pool.)

use serde::{Deserialize, Serialize};

use crate::block::{self, BlockId};
use crate::entity::{MobKind, Position};
use crate::mob::MobType;
use crate::world::World;

/// Profession a villager can claim. Carpenter/Cook/Farmer are live in alpha;
/// Blacksmith + Scribe land when Furnace + Bookshelf blocks ship.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Profession {
    #[default]
    None,
    Farmer,
    Blacksmith,
    Cook,
    /// Renamed from Librarian in Historical Pivot HP-0 (2026-05-22).
    /// "Scribe" is the medieval English term for a manuscript-copying
    /// scholar; dedicated "librarian" as a role is post-medieval.
    Scribe,
    Carpenter,
    /// Spec 26 — commissioned NPC builder. Claims a DRAFTING_TABLE
    /// workstation. Phases 5-14 (commission dialog, NPC pathfind,
    /// animated half-rate build, fee formula, refund flow) are
    /// DEFERRED post-MVP. v1 just exposes the profession + the
    /// workstation-claim hook so a villager bound to a Drafting Table
    /// reads as Builder in the dialogue UI.
    Builder,
    // Historical Pivot Sub 7 (2026-05-23) — workstation-bound trades
    // for the T1.5 processed economy. Each claims one existing
    // workstation block: Mill → Miller, Oven → Baker, Aging Rack →
    // Brewer. Appended bincode-positionally to keep save compat.
    Miller,
    Baker,
    Brewer,
}

impl Profession {
    /// Human-readable label for the dialogue UI (phase 5 +).
    pub fn name(self) -> &'static str {
        match self {
            Profession::None => "Unemployed",
            Profession::Farmer => "Farmer",
            Profession::Blacksmith => "Blacksmith",
            Profession::Cook => "Cook",
            Profession::Scribe => "Scribe",
            Profession::Carpenter => "Carpenter",
            Profession::Builder => "Builder",
            Profession::Miller => "Miller",
            Profession::Baker => "Baker",
            Profession::Brewer => "Brewer",
        }
    }

    /// Lookup: which profession does this block bind a villager to? Returns
    /// `None` if the block isn't a workstation. The mapping is the source of
    /// truth for the workstation table called out in the spec.
    pub fn from_workstation_block(b: BlockId) -> Option<Profession> {
        match b {
            block::TILLED_SOIL => Some(Profession::Farmer),
            block::CAMPFIRE => Some(Profession::Cook),
            block::CRAFTING_TABLE => Some(Profession::Carpenter),
            // Spec 20 Phase 7 — Furnace (and its lit variant) claim
            // Blacksmiths. The existing BLACKSMITH_POOL in `quest.rs`
            // becomes reachable as soon as a player places a furnace
            // adjacent to a villager.
            block::FURNACE | block::FURNACE_LIT => Some(Profession::Blacksmith),
            // Spec 26 — Drafting Table claims a Builder.
            block::DRAFTING_TABLE => Some(Profession::Builder),
            // Historical Pivot Sub 7 — T1.5 processed-economy
            // workstations claim Miller / Baker / Brewer respectively.
            block::MILL => Some(Profession::Miller),
            block::OVEN => Some(Profession::Baker),
            block::AGING_RACK => Some(Profession::Brewer),
            // Bookshelf ships later — a future Scribe foundation.
            _ => None,
        }
    }
}

/// Per-villager state — profession + the workstation they've claimed.
///
/// Attached as an ECS component to every spawned `MobType::Villager` and
/// `MobType::Peddler` (the latter — historically WanderingVillager — to ease
/// the Phase 10 migration; once they convert, their profession is already
/// there to use).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VillagerComponent {
    pub profession: Profession,
    /// Workstation the villager is bound to. Cleared when the block is
    /// destroyed (see `release_orphan_claims`).
    pub claimed_workstation: Option<[i32; 3]>,
    /// Spec 19 gossip-extension — a single one-line rumour the villager
    /// says when the dialogue opens. Refreshed once per in-game day.
    /// `None` until the first refresh runs (so a freshly-spawned villager
    /// stays quiet for ~5 min of game time, which reads as "they haven't
    /// noticed you yet"). The pool is generic on alpha; future Vendor
    /// Block + Raid Defence specs extend it with cross-economy hooks
    /// per `docs/vision/sat-flow-and-economy-loops.md` §3.1.
    pub gossip_line: Option<String>,
    /// Tick at which `gossip_line` was last refreshed. Daily-tick check
    /// reads this to decide whether to re-roll.
    pub gossip_last_refresh_tick: u64,
    /// Spec 26 — active Builder commission, if any. Only Builders ever
    /// carry one; the field stays None on every other profession + on
    /// Builders between commissions. Persists across save/load so a
    /// quit-mid-commission resumes cleanly.
    #[serde(default)]
    pub commission: Option<crate::builder::BuilderCommission>,
}

/// How close (Chebyshev distance, blocks) a villager must be to a workstation
/// to claim it. Generous so a villager wandering near their kitchen still
/// counts.
pub const CLAIM_RANGE: i32 = 3;

/// Tick interval for the workstation-claim scan. 40 ticks ≈ 2 s @ 20 TPS —
/// fast enough to feel responsive when the kid plants tilled soil next to a
/// villager, cheap enough that the scan doesn't dominate the frame.
pub const CLAIM_SCAN_INTERVAL: u32 = 40;

/// Scan every Villager / Peddler and assign a profession if they
/// are standing next to an unclaimed workstation. Pure ECS — no rendering,
/// no audio. Called from the game loop at 1/CLAIM_SCAN_INTERVAL.
///
/// Returns the number of professions assigned this scan (informational; the
/// engine commands path uses it to surface a confirmation toast in dev mode).
pub fn tick_workstation_claims(ecs: &mut hecs::World, world: &World) -> u32 {
    // First pass — collect (villager_id, pos) + the set of already-claimed
    // workstation positions so the second pass doesn't double-claim.
    let mut candidates: Vec<(hecs::Entity, glam::Vec3)> = Vec::new();
    let mut already_claimed: ahash::AHashSet<[i32; 3]> = ahash::AHashSet::new();
    for (id, (kind, pos, vc)) in ecs.query::<(&MobKind, &Position, &VillagerComponent)>().iter() {
        if !is_villager_kind(kind.0) {
            continue;
        }
        if let Some(claim) = vc.claimed_workstation {
            already_claimed.insert(claim);
            // Already employed — no rescan unless block was destroyed (handled
            // by `release_orphan_claims`).
            continue;
        }
        candidates.push((id, pos.0));
    }

    // Second pass — for each unemployed villager, scan a Chebyshev cube around
    // their feet looking for a workstation block not already claimed.
    let mut assigned = 0u32;
    for (id, pos) in candidates {
        let cx = pos.x.floor() as i32;
        let cy = pos.y.floor() as i32;
        let cz = pos.z.floor() as i32;
        let found = find_claim_in_range(world, &already_claimed, cx, cy, cz);
        if let Some((wx, wy, wz, prof)) = found {
            if let Ok(mut vc) = ecs.get::<&mut VillagerComponent>(id) {
                vc.profession = prof;
                vc.claimed_workstation = Some([wx, wy, wz]);
            }
            already_claimed.insert([wx, wy, wz]);
            assigned += 1;
        }
    }
    assigned
}

/// After block changes, walk every claim and clear it if the underlying
/// block is no longer the profession's workstation. Cheap — only runs on
/// the player's dirty-chunk tick path.
pub fn release_orphan_claims(ecs: &mut hecs::World, world: &World) {
    // Two-pass to avoid borrow conflict on the ecs query while mutating.
    let mut orphans: Vec<hecs::Entity> = Vec::new();
    for (id, (kind, vc)) in ecs.query::<(&MobKind, &VillagerComponent)>().iter() {
        if !is_villager_kind(kind.0) {
            continue;
        }
        if let Some([x, y, z]) = vc.claimed_workstation {
            let blk = world.get_block(x, y, z);
            let still_matches = Profession::from_workstation_block(blk) == Some(vc.profession);
            if !still_matches {
                orphans.push(id);
            }
        }
    }
    for id in orphans {
        if let Ok(mut vc) = ecs.get::<&mut VillagerComponent>(id) {
            vc.profession = Profession::None;
            vc.claimed_workstation = None;
        }
    }
}

/// Whether this mob type carries a `VillagerComponent` and participates in
/// the profession system. Iron Golem is intentionally excluded — golems
/// don't work, they guard.
pub fn is_villager_kind(kind: MobType) -> bool {
    matches!(kind, MobType::Villager | MobType::Peddler)
}

fn find_claim_in_range(
    world: &World,
    already_claimed: &ahash::AHashSet<[i32; 3]>,
    cx: i32,
    cy: i32,
    cz: i32,
) -> Option<(i32, i32, i32, Profession)> {
    // Search a 7×7×7 cube centred on the villager's foot block. Earliest
    // hit wins — order is intentionally deterministic so save/load + replay
    // give the same claim assignments.
    for dy in -CLAIM_RANGE..=CLAIM_RANGE {
        for dz in -CLAIM_RANGE..=CLAIM_RANGE {
            for dx in -CLAIM_RANGE..=CLAIM_RANGE {
                let x = cx + dx;
                let y = cy + dy;
                let z = cz + dz;
                if already_claimed.contains(&[x, y, z]) {
                    continue;
                }
                let blk = world.get_block(x, y, z);
                if let Some(prof) = Profession::from_workstation_block(blk) {
                    return Some((x, y, z, prof));
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Gossip extension — Spec 19 follow-on, load-bearing for the three economy
// foundation specs (Furnace / Vendor Block / Raid Defence) that all depend
// on `VillagerComponent.gossip_line` as their discovery layer. Each villager
// holds one rumour line, refreshed daily. Generic pool on alpha; the Vendor
// Block and Raid Defence specs grow profession-specific + context-aware pools
// when they build.
// ---------------------------------------------------------------------------

/// One in-game day at the engine's tick rate. Matches
/// `reputation::REPUTATION_DECAY_INTERVAL_TICKS` so gossip + decay fire on
/// the same cadence.
pub const GOSSIP_REFRESH_INTERVAL_TICKS: u64 = 6000;

/// Generic flavour-text pool every villager can draw from on alpha.
/// Profession-keyed expansion lands when the dialogue path needs it;
/// per-village context (nearby vendors, raid warnings) layers on top of
/// this baseline.
const GENERIC_GOSSIP: &[&str] = &[
    "Strange tracks in the woods last night.",
    "The well water is sweeter on the east side of the village.",
    "I heard the cook's daughter married a wandering villager.",
    "If you've got bones, the Carpenter sometimes wants them.",
    "Some folks say the deeper you dig, the richer the rock gets.",
    "There's a campfire somewhere north of here — I can smell the smoke.",
    "Best harvest the wheat at dawn, before the dew burns off.",
    "If you see a Knight, don't run — they only chase the bad ones.",
    "The Blacksmith over the hill is always short of iron.",
    "Watch out for brigands after dark — they prowl where the torches run out.",
    "Old Margery says her crops grow faster when the moon is full.",
    "A wandering villager passed through last week. Quiet folk.",
    // Moonshot Phase A — Diamond-Age rumours. Pure flavour in the same
    // register as the lines above; they hint at the setting (diamonds are
    // the old money; a founding awaits) and change NO mechanic.
    "Diamonds buy less bread than they did last season.",
    "They say every world has a founding. Ours hasn't happened yet.",
    "My grandmother kept her diamonds under the floor. Wouldn't do it myself these days.",
];

/// Profession-flavoured lines that override the generic pool when set.
/// Sparse; most villagers fall back to the generic pool.
fn profession_gossip(prof: Profession) -> &'static [&'static str] {
    match prof {
        Profession::Farmer => &[
            "Carrots are easier than potatoes if you're just starting out.",
            "The plough leaves a tilled furrow for two days before the rain washes it.",
            "Bread keeps you full longer than raw wheat. Ask the Cook.",
        ],
        Profession::Cook => &[
            "A campfire cooks two raw meats at once if you stack them right.",
            "Bread + cooked meat is the meal of champions.",
            "Don't eat raw meat if there's a campfire nearby. Trust me.",
        ],
        Profession::Carpenter => &[
            "Four planks from one log. Anyone tells you otherwise is selling you short.",
            "Sticks make tools. Tools make a living.",
            "If you find a Workbench in the wild, treat it kind.",
        ],
        Profession::Blacksmith => &[
            "Iron from the deep stone is the best kind.",
            "A Furnace pays back the cobblestone in a day.",
            "Coal is cheaper than logs, but logs burn longer per piece.",
        ],
        Profession::Scribe => &[
            "Old maps mark old villages. Some of them are still there.",
            "The wind tells you where the next storm is coming from.",
            "Words travel further than people. Listen.",
        ],
        Profession::Builder => &[
            "A good plan is half the work; the other half's just stacking.",
            "Bring me a Plan and the materials. I'll lay the bricks.",
            "A Drafting Table is where buildings start their lives.",
        ],
        Profession::Miller => &[
            "Two stones grind better than one.",
            "Wheat in, flour out — same as it's always been.",
            "The wind never delivers on time.",
            "A heavy stone is a slow stone, but a fair one.",
        ],
        Profession::Baker => &[
            "Dough's done when it springs back.",
            "Cold oven, cold customers.",
            "Yeast asks for patience.",
            "A burnt loaf is a lesson, not a loss.",
        ],
        Profession::Brewer => &[
            "Time makes the drink, not the recipe.",
            "Honey first, water second, patience third.",
            "An empty cask is a sad cask.",
            "Don't drink the first batch — that's for the cooper.",
        ],
        Profession::None => &[],
    }
}

/// Deterministic per-villager seed for the gossip roll. Mixes the entity id
/// (so different villagers in the same village have different rumours) with
/// the world-day counter (so the rumour changes daily).
fn gossip_seed(entity: hecs::Entity, day: u64) -> u32 {
    // hecs::Entity Debug prints "id, gen" — pull the integer id by formatting.
    let s = format!("{:?}", entity);
    let id_hash: u32 = s.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    id_hash ^ (day as u32).wrapping_mul(0x9E3779B1)
}

/// Pick a gossip line for the given villager + day. Profession-flavoured
/// lines weight 30 % of the roll; the remaining 70 % is generic.
pub fn roll_gossip_line(entity: hecs::Entity, prof: Profession, day: u64) -> String {
    let seed = gossip_seed(entity, day);
    let prof_pool = profession_gossip(prof);
    let prof_roll = (seed % 100) < 30 && !prof_pool.is_empty();
    if prof_roll {
        let idx = ((seed >> 8) as usize) % prof_pool.len();
        prof_pool[idx].to_string()
    } else {
        let idx = ((seed >> 8) as usize) % GENERIC_GOSSIP.len();
        GENERIC_GOSSIP[idx].to_string()
    }
}

/// Daily-tick refresh — walks every villager whose `gossip_last_refresh_tick`
/// is older than `GOSSIP_REFRESH_INTERVAL_TICKS` and re-rolls their line.
/// Idempotent within a day; cheap to call on every minute-cadence tick.
pub fn tick_gossip_refresh(ecs: &mut hecs::World, current_tick: u64) -> u32 {
    let day = current_tick / GOSSIP_REFRESH_INTERVAL_TICKS;
    let mut refreshed = 0u32;
    // Two-pass to avoid mutating during the borrow.
    let mut to_refresh: Vec<(hecs::Entity, Profession)> = Vec::new();
    for (id, (kind, vc)) in ecs.query::<(&MobKind, &VillagerComponent)>().iter() {
        if !is_villager_kind(kind.0) {
            continue;
        }
        let need_refresh = vc.gossip_line.is_none()
            || current_tick.saturating_sub(vc.gossip_last_refresh_tick)
                >= GOSSIP_REFRESH_INTERVAL_TICKS;
        if need_refresh {
            to_refresh.push((id, vc.profession));
        }
    }
    for (id, prof) in to_refresh {
        let line = roll_gossip_line(id, prof, day);
        if let Ok(mut vc) = ecs.get::<&mut VillagerComponent>(id) {
            vc.gossip_line = Some(line);
            vc.gossip_last_refresh_tick = current_tick;
        }
        refreshed += 1;
    }
    refreshed
}

// ---------------------------------------------------------------------------
// Spec 22 Phase 15 — pre-raid villager gossip override.
//
// When a raid is scheduled (Warning state), every villager whose village is
// under threat surfaces a "raid coming" rumour instead of the daily-default
// gossip line. The player who talks to a villager BEFORE the smoke-pillar
// red-shift gets a 1-2-day warning. This is the discovery layer; the smoke
// pillar is the urgent 5-minute alert that fires later.
//
// Implementation: a separate tick function that runs AFTER
// `tick_gossip_refresh` and overrides `gossip_line` on villagers in raid-
// warned villages. Override is non-destructive — once the raid clears or
// transitions out of Warning, the next daily refresh restores the regular
// pool. Idempotent within a tick.
// ---------------------------------------------------------------------------

/// Flavour pool for raid-warning gossip lines. Small + concrete; the
/// player gets variety across villagers in the same village but the
/// signal is unambiguous ("brigands coming"). UK English.
///
/// HP-5 refresh (2026-05-23) — wording now names the brigands directly
/// instead of "bandits" / "strange folk", matching the historical-pivot
/// roster swap.
const RAID_WARNING_GOSSIP: &[&str] = &[
    "The shepherds say brigands are coming for the village. Soon.",
    "Marauders sighted at the treeline — an incursion's brewing, mark my words.",
    "I've packed the children inside. The brigands are on their way.",
    "The dogs won't settle. Something's moving in the dark towards us.",
];

/// Pick a raid-warning gossip line for the given villager + raid. Same
/// deterministic-per-entity-per-raid pattern as `roll_gossip_line` so a
/// villager's warning line is stable across re-reads within a raid.
pub fn roll_raid_warning_gossip(entity: hecs::Entity, raid_id: u32) -> String {
    let s = format!("{:?}", entity);
    let id_hash: u32 = s.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let seed = id_hash ^ raid_id.wrapping_mul(0x9E3779B1);
    let idx = (seed as usize) % RAID_WARNING_GOSSIP.len();
    RAID_WARNING_GOSSIP[idx].to_string()
}

/// Spec 22 Phase 15 — walk every villager and override their
/// `gossip_line` with a raid-warning rumour when their village has a
/// raid in Warning state. Called from the game loop right after
/// `tick_gossip_refresh`. Idempotent: re-running same-tick produces
/// the same lines. Returns the number of villagers whose line was
/// overridden (informational).
///
/// Villager-to-village assignment uses the same "nearest village
/// anchor" rule as `reputation::village_at_position` for consistency
/// with the raid-bounty banner in `villager_ui`.
pub fn tick_raid_warning_gossip(ecs: &mut hecs::World, world: &World) -> u32 {
    if world.active_raids.is_empty() {
        return 0;
    }
    let mut warned: ahash::AHashMap<crate::reputation::VillageId, crate::raid::RaidId> =
        ahash::AHashMap::new();
    for r in &world.active_raids {
        if r.status == crate::raid::RaidStatus::Warning {
            warned.insert(r.village_id, r.id);
        }
    }
    if warned.is_empty() {
        return 0;
    }
    let mut to_override: Vec<(hecs::Entity, crate::raid::RaidId)> = Vec::new();
    for (id, (kind, _vc, pos)) in ecs.query::<(&MobKind, &VillagerComponent, &Position)>().iter() {
        if !is_villager_kind(kind.0) {
            continue;
        }
        let Some(vid) = crate::reputation::village_at_position(world, pos.0) else {
            continue;
        };
        if let Some(&raid_id) = warned.get(&vid) {
            to_override.push((id, raid_id));
        }
    }
    let mut overridden = 0u32;
    for (id, raid_id) in to_override {
        let line = roll_raid_warning_gossip(id, raid_id);
        if let Ok(mut vc) = ecs.get::<&mut VillagerComponent>(id) {
            vc.gossip_line = Some(line);
        }
        overridden += 1;
    }
    overridden
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity;
    use crate::mob::MobType;

    fn spawn_villager(ecs: &mut hecs::World, pos: glam::Vec3) -> hecs::Entity {
        entity::spawn_mob(ecs, MobType::Villager, pos);
        let mut id = None;
        for (e, _) in ecs.query::<&MobKind>().iter() {
            id = Some(e);
        }
        let id = id.unwrap();
        let _ = ecs.insert_one(id, VillagerComponent::default());
        id
    }

    #[test]
    fn profession_from_block_table_covers_alpha_workstations() {
        assert_eq!(Profession::from_workstation_block(block::TILLED_SOIL), Some(Profession::Farmer));
        assert_eq!(Profession::from_workstation_block(block::CAMPFIRE), Some(Profession::Cook));
        assert_eq!(Profession::from_workstation_block(block::CRAFTING_TABLE), Some(Profession::Carpenter));
        // Spec 20 Phase 7 — Furnace + lit variant claim Blacksmiths.
        assert_eq!(Profession::from_workstation_block(block::FURNACE), Some(Profession::Blacksmith));
        assert_eq!(Profession::from_workstation_block(block::FURNACE_LIT), Some(Profession::Blacksmith));
        // Spec 26 — Drafting Table claims a Builder.
        assert_eq!(Profession::from_workstation_block(block::DRAFTING_TABLE), Some(Profession::Builder));
        // Block ids without workstation meaning return None.
        assert_eq!(Profession::from_workstation_block(block::STONE), None);
        assert_eq!(Profession::from_workstation_block(block::AIR), None);
    }

    #[test]
    fn builder_profession_label_is_human_readable() {
        // Spec 26 — guard the label so the dialogue UI shows "Builder"
        // (not "Unemployed") for villagers bound to a Drafting Table.
        assert_eq!(Profession::Builder.name(), "Builder");
    }

    #[test]
    fn hp7_trades_claim_t1_5_workstation_blocks() {
        // Historical Pivot Sub 7 — Mill → Miller, Oven → Baker,
        // Aging Rack → Brewer.
        assert_eq!(
            Profession::from_workstation_block(block::MILL),
            Some(Profession::Miller),
        );
        assert_eq!(
            Profession::from_workstation_block(block::OVEN),
            Some(Profession::Baker),
        );
        assert_eq!(
            Profession::from_workstation_block(block::AGING_RACK),
            Some(Profession::Brewer),
        );
    }

    #[test]
    fn hp7_professions_have_human_readable_labels() {
        assert_eq!(Profession::Miller.name(), "Miller");
        assert_eq!(Profession::Baker.name(), "Baker");
        assert_eq!(Profession::Brewer.name(), "Brewer");
    }

    #[test]
    fn hp7_professions_have_non_empty_gossip_pools() {
        assert!(!profession_gossip(Profession::Miller).is_empty());
        assert!(!profession_gossip(Profession::Baker).is_empty());
        assert!(!profession_gossip(Profession::Brewer).is_empty());
    }

    #[test]
    fn villager_next_to_mill_claims_miller() {
        let mut ecs = hecs::World::new();
        let mut world = World::new();
        world.set_block(1, 64, 0, block::MILL);
        let id = spawn_villager(&mut ecs, glam::Vec3::new(0.0, 64.0, 0.0));
        let n = tick_workstation_claims(&mut ecs, &world);
        assert_eq!(n, 1);
        let vc = ecs.get::<&VillagerComponent>(id).unwrap();
        assert_eq!(vc.profession, Profession::Miller);
        assert_eq!(vc.claimed_workstation, Some([1, 64, 0]));
    }

    #[test]
    fn unemployed_villager_claims_adjacent_tilled_soil_as_farmer() {
        let mut ecs = hecs::World::new();
        let mut world = World::new();
        // Floor under the villager + a tilled-soil block one block south.
        for x in -2..=2 { for z in -2..=2 { world.set_block(x, 63, z, block::DIRT); } }
        world.set_block(1, 64, 0, block::TILLED_SOIL);
        let id = spawn_villager(&mut ecs, glam::Vec3::new(0.0, 64.0, 0.0));

        let n = tick_workstation_claims(&mut ecs, &world);
        assert_eq!(n, 1, "expected one claim this scan");

        let vc = ecs.get::<&VillagerComponent>(id).unwrap();
        assert_eq!(vc.profession, Profession::Farmer);
        assert_eq!(vc.claimed_workstation, Some([1, 64, 0]));
    }

    #[test]
    fn second_villager_cannot_double_claim_same_workstation() {
        let mut ecs = hecs::World::new();
        let mut world = World::new();
        world.set_block(0, 64, 0, block::CAMPFIRE);
        let a = spawn_villager(&mut ecs, glam::Vec3::new(0.5, 64.0, 0.0));
        let b = spawn_villager(&mut ecs, glam::Vec3::new(1.5, 64.0, 0.0));

        let _ = tick_workstation_claims(&mut ecs, &world);

        let va = ecs.get::<&VillagerComponent>(a).unwrap();
        let vb = ecs.get::<&VillagerComponent>(b).unwrap();
        // Exactly one villager got the campfire, the other stayed None.
        let employed_count = [va.profession, vb.profession]
            .iter()
            .filter(|p| **p != Profession::None)
            .count();
        assert_eq!(employed_count, 1, "exactly one villager should hold the campfire claim");
    }

    #[test]
    fn destroyed_workstation_releases_claim() {
        let mut ecs = hecs::World::new();
        let mut world = World::new();
        world.set_block(0, 64, 0, block::CRAFTING_TABLE);
        let id = spawn_villager(&mut ecs, glam::Vec3::new(0.5, 64.0, 0.0));
        let _ = tick_workstation_claims(&mut ecs, &world);
        assert_eq!(ecs.get::<&VillagerComponent>(id).unwrap().profession, Profession::Carpenter);

        // Player breaks the crafting table.
        world.set_block(0, 64, 0, block::AIR);
        release_orphan_claims(&mut ecs, &world);

        let vc = ecs.get::<&VillagerComponent>(id).unwrap();
        assert_eq!(vc.profession, Profession::None);
        assert!(vc.claimed_workstation.is_none());
    }

    #[test]
    fn villager_keeps_claim_until_block_actually_disappears() {
        let mut ecs = hecs::World::new();
        let mut world = World::new();
        world.set_block(0, 64, 0, block::TILLED_SOIL);
        let id = spawn_villager(&mut ecs, glam::Vec3::new(0.5, 64.0, 0.0));
        let _ = tick_workstation_claims(&mut ecs, &world);

        // Rescan while block still there — claim unchanged.
        release_orphan_claims(&mut ecs, &world);
        assert_eq!(ecs.get::<&VillagerComponent>(id).unwrap().profession, Profession::Farmer);
    }

    #[test]
    fn knight_is_not_a_villager_kind() {
        assert!(!is_villager_kind(MobType::Knight));
        assert!(is_villager_kind(MobType::Villager));
        assert!(is_villager_kind(MobType::Peddler));
    }

    // --- Gossip extension ---

    #[test]
    fn gossip_refresh_populates_an_unset_line() {
        let mut ecs = hecs::World::new();
        let id = spawn_villager(&mut ecs, glam::Vec3::new(0.0, 64.0, 0.0));
        // Newly-spawned villager has no gossip line.
        assert!(ecs.get::<&VillagerComponent>(id).unwrap().gossip_line.is_none());
        let refreshed = tick_gossip_refresh(&mut ecs, 0);
        assert_eq!(refreshed, 1, "expected one villager refreshed");
        let line = ecs.get::<&VillagerComponent>(id).unwrap().gossip_line.clone();
        assert!(line.is_some(), "gossip line should be set after refresh");
    }

    #[test]
    fn gossip_does_not_re_roll_within_a_day() {
        let mut ecs = hecs::World::new();
        let id = spawn_villager(&mut ecs, glam::Vec3::new(0.0, 64.0, 0.0));
        let _ = tick_gossip_refresh(&mut ecs, 100);
        let first = ecs.get::<&VillagerComponent>(id).unwrap().gossip_line.clone();
        // Same tick within the same day → no re-roll.
        let refreshed = tick_gossip_refresh(&mut ecs, 100);
        assert_eq!(refreshed, 0, "second refresh on same day must not re-roll");
        let second = ecs.get::<&VillagerComponent>(id).unwrap().gossip_line.clone();
        assert_eq!(first, second);
    }

    #[test]
    fn gossip_re_rolls_after_a_day() {
        let mut ecs = hecs::World::new();
        let id = spawn_villager(&mut ecs, glam::Vec3::new(0.0, 64.0, 0.0));
        let _ = tick_gossip_refresh(&mut ecs, 0);
        let day_two_tick = GOSSIP_REFRESH_INTERVAL_TICKS + 10;
        let refreshed = tick_gossip_refresh(&mut ecs, day_two_tick);
        assert_eq!(refreshed, 1, "day-2 refresh must roll");
        let vc = ecs.get::<&VillagerComponent>(id).unwrap();
        assert_eq!(vc.gossip_last_refresh_tick, day_two_tick);
    }

    #[test]
    fn diamond_age_gossip_seeds_present_in_generic_pool() {
        // Moonshot Phase A — a couple of Diamond-Age rumour lines sit in the
        // generic pool (pure data; same register as "Strange tracks in the
        // woods"). Substring key so the exact wording stays owner-tunable.
        for key in ["Diamonds buy less bread", "every world has a founding"] {
            assert!(
                GENERIC_GOSSIP.iter().any(|l| l.contains(key)),
                "missing Diamond-Age gossip line containing '{key}'"
            );
        }
    }

    #[test]
    fn gossip_pools_never_carry_money_or_bitcoin_words() {
        // Moonshot guardrail (north-star §1/§9): villager speech never says
        // "Bitcoin"/"crypto"/"sats"/"earn". Whole-word match ("learn" is fine).
        let mut all: Vec<&str> = GENERIC_GOSSIP.to_vec();
        all.extend_from_slice(RAID_WARNING_GOSSIP);
        for prof in [
            Profession::Farmer, Profession::Blacksmith, Profession::Cook,
            Profession::Scribe, Profession::Carpenter, Profession::Builder,
            Profession::Miller, Profession::Baker, Profession::Brewer,
        ] {
            all.extend_from_slice(profession_gossip(prof));
        }
        for line in all {
            let lower = line.to_lowercase();
            for banned in ["bitcoin", "crypto", "sats", "earn", "earning", "earned"] {
                let hit = lower
                    .split(|ch: char| !ch.is_alphanumeric())
                    .any(|word| word == banned);
                assert!(!hit, "gossip line '{line}' contains banned word '{banned}'");
            }
        }
    }

    #[test]
    fn roll_gossip_line_is_deterministic_per_entity_and_day() {
        let mut ecs = hecs::World::new();
        let id = ecs.spawn((MobKind(MobType::Villager),));
        let a = roll_gossip_line(id, Profession::Farmer, 0);
        let b = roll_gossip_line(id, Profession::Farmer, 0);
        assert_eq!(a, b);
    }

    #[test]
    fn knight_skipped_by_gossip_tick() {
        let mut ecs = hecs::World::new();
        // Spawn a Knight; spawn_mob doesn't attach VillagerComponent for it.
        crate::entity::spawn_mob(&mut ecs, MobType::Knight, glam::Vec3::new(0.0, 64.0, 0.0));
        let refreshed = tick_gossip_refresh(&mut ecs, 0);
        assert_eq!(refreshed, 0, "knight should not be gossiped");
    }

    // --- Spec 22 Phase 15: raid-warning gossip override ---

    #[test]
    fn raid_warning_gossip_overrides_default_for_warned_village() {
        let mut ecs = hecs::World::new();
        let mut world = World::new();
        world.village_anchors.insert((0, 0), [0, 64, 0]);
        let id = spawn_villager(&mut ecs, glam::Vec3::new(2.0, 64.0, 0.0));
        let _ = tick_gossip_refresh(&mut ecs, 0);
        let default_line = ecs.get::<&VillagerComponent>(id).unwrap().gossip_line.clone().unwrap();
        world.active_raids.push(crate::raid::Raid::new_warning(
            7,
            (0, 0),
            [0, 64, 0],
            crate::raid::WaveKind::Medium,
            500,
            0,
        ));
        let n = tick_raid_warning_gossip(&mut ecs, &world);
        assert_eq!(n, 1, "expected one villager line overridden");
        let new_line = ecs.get::<&VillagerComponent>(id).unwrap().gossip_line.clone().unwrap();
        assert_ne!(new_line, default_line, "raid-warning line must override default");
        assert!(
            RAID_WARNING_GOSSIP.contains(&new_line.as_str()),
            "override line must come from the raid-warning pool, got: {new_line}"
        );
    }

    #[test]
    fn peaceful_village_villager_keeps_default_gossip_line() {
        let mut ecs = hecs::World::new();
        let mut world = World::new();
        world.village_anchors.insert((0, 0), [0, 64, 0]);
        let id = spawn_villager(&mut ecs, glam::Vec3::new(2.0, 64.0, 0.0));
        let _ = tick_gossip_refresh(&mut ecs, 0);
        let before = ecs.get::<&VillagerComponent>(id).unwrap().gossip_line.clone().unwrap();
        let n = tick_raid_warning_gossip(&mut ecs, &world);
        assert_eq!(n, 0);
        let after = ecs.get::<&VillagerComponent>(id).unwrap().gossip_line.clone().unwrap();
        assert_eq!(before, after, "peaceful-village line must not change");
    }

    #[test]
    fn raid_warning_gossip_skips_non_warning_raids() {
        // An Active (not Warning) raid is past the rumour-warning window —
        // the smoke pillar is already shifted, so the gossip override stops.
        let mut ecs = hecs::World::new();
        let mut world = World::new();
        world.village_anchors.insert((0, 0), [0, 64, 0]);
        let id = spawn_villager(&mut ecs, glam::Vec3::new(2.0, 64.0, 0.0));
        let _ = tick_gossip_refresh(&mut ecs, 0);
        let default_line = ecs.get::<&VillagerComponent>(id).unwrap().gossip_line.clone().unwrap();
        let mut raid = crate::raid::Raid::new_warning(1, (0, 0), [0, 64, 0], crate::raid::WaveKind::Small, 100, 0);
        raid.status = crate::raid::RaidStatus::Active;
        world.active_raids.push(raid);
        let n = tick_raid_warning_gossip(&mut ecs, &world);
        assert_eq!(n, 0, "Active raids don't override gossip");
        let after = ecs.get::<&VillagerComponent>(id).unwrap().gossip_line.clone().unwrap();
        assert_eq!(after, default_line);
    }

    #[test]
    fn raid_warning_gossip_is_deterministic_for_same_entity_and_raid() {
        let mut ecs = hecs::World::new();
        let id = ecs.spawn((MobKind(MobType::Villager),));
        let a = roll_raid_warning_gossip(id, 42);
        let b = roll_raid_warning_gossip(id, 42);
        assert_eq!(a, b);
    }
}
