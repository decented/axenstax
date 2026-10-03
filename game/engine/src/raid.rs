//! Spec 22 — Raid Defence MVP (Phases 2-13).
//!
//! A village with population + treasury can be **raided** by a hostile
//! wave. Defenders (any player inside the village radius) earn a share
//! of the treasury proportional to their kill count, plus reputation;
//! a failed defence drops the village's reputation tier for every
//! player.
//!
//! Engine-generic primitives lifted from the spec's "cross-game lift"
//! section: daily-roll scheduler + composition table + attribution +
//! treasury-share payout. AxeNStax-specific data: wave composition mob
//! kinds + treasury thresholds.
//!
//! **Out of scope (deferred per the user's "single-round MVP only"
//! call):** multi-wave (Phase 14), pre-raid villager rumours (15), rare-
//! loot tiered drops (16), Vendor Block raid-supplies highlight (17),
//! raid leaderboard (18). The scheduler shape leaves room for each
//! (e.g. `WaveKind` is an enum so adding `Multi(Vec<WaveKind>)` is
//! mechanical) but no v1 wiring exists.

use ahash::AHashMap;
use serde::{Deserialize, Serialize};

use crate::mob::MobType;
use crate::reputation::VillageId;

/// Stable per-raid identifier. Monotonic across the world's lifetime;
/// persisted with the scheduler. Wrapping at u32::MAX is acceptable —
/// even a heavy-raid playthrough wouldn't exhaust this in 100 years.
pub type RaidId = u32;

/// Wave size. Treasury tier picks one of these three on alpha; multi-
/// wave + harder tiers extend the enum post-MVP.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WaveKind {
    Small,
    Medium,
    Large,
}

impl WaveKind {
    pub fn total_mobs(self) -> u32 {
        match self {
            WaveKind::Small => 5,
            WaveKind::Medium => 10,
            WaveKind::Large => 20,
        }
    }

    /// Mob mix per wave — count of each MobType the wave spawns with.
    /// Returns a Vec of `(MobType, count)` totalling [`Self::total_mobs`].
    ///
    /// HP-5 (2026-05-23) — historical-pivot refresh: fantasy mobs swapped
    /// for brigand tiers. Lower tiers lean Brigand (low-HP grunts);
    /// higher tiers add Marauder + Berserker pressure. Same totals
    /// (5 / 10 / 20) as the pre-pivot table so bounty maths + scheduler
    /// thresholds stay valid.
    pub fn composition(self) -> &'static [(MobType, u32)] {
        match self {
            // Small: 4 Brigand + 1 Marauder. Beginner-friendly; many
            // bodies, one elite.
            WaveKind::Small => &[
                (MobType::Brigand, 4),
                (MobType::Marauder, 1),
            ],
            // Medium: 6 Brigand + 3 Marauder + 1 Berserker. First
            // Berserker shows up; meaningful step up.
            WaveKind::Medium => &[
                (MobType::Brigand, 6),
                (MobType::Marauder, 3),
                (MobType::Berserker, 1),
            ],
            // Large: 10 Brigand + 7 Marauder + 3 Berserker. Multi-
            // Berserker push; serious threat to a defended village.
            WaveKind::Large => &[
                (MobType::Brigand, 10),
                (MobType::Marauder, 7),
                (MobType::Berserker, 3),
            ],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            WaveKind::Small => "Small",
            WaveKind::Medium => "Medium",
            WaveKind::Large => "Large",
        }
    }
}

/// Treasury tiers. Pure function — same treasury always picks the same
/// kind. Thresholds match Spec §7 "Raid frequency on alpha".
pub fn wave_kind_for_treasury(treasury_sats: u64) -> Option<WaveKind> {
    if treasury_sats == 0 {
        None
    } else if treasury_sats < 500 {
        Some(WaveKind::Small)
    } else if treasury_sats < 5_000 {
        Some(WaveKind::Medium)
    } else {
        Some(WaveKind::Large)
    }
}

/// Sats drained from the treasury when a raid is scheduled. Drain is the
/// bounty pool: distributed proportional to defender kills on success;
/// returned to the treasury on failure (no defenders means no payout).
pub fn treasury_drain_for(treasury_sats: u64, kind: WaveKind) -> u64 {
    let cap = match kind {
        WaveKind::Small => 100,
        WaveKind::Medium => 500,
        WaveKind::Large => 2_000,
    };
    treasury_sats.min(cap)
}

/// Probability of a raid firing on a given day, given the village's
/// treasury size + wave kind. 20 % daily roll for Small, scaling up.
/// Returns 0..=10_000 (basis-point bucket) so the seeded roll can use
/// integer arithmetic.
fn roll_threshold_bps(kind: WaveKind) -> u32 {
    match kind {
        WaveKind::Small => 2_000,  // 20 %
        WaveKind::Medium => 4_000, // 40 %
        WaveKind::Large => 6_000,  // 60 %
    }
}

/// Pure: would the (world_seed, village_id, day) tuple roll a raid
/// against this village given its current treasury? **Deterministic** —
/// same inputs always produce the same outcome. Returns the picked
/// wave kind on a hit, or None.
pub fn roll_for_day(
    world_seed: u32,
    village_id: VillageId,
    day: u64,
    treasury_sats: u64,
) -> Option<WaveKind> {
    let kind = wave_kind_for_treasury(treasury_sats)?;
    // Mix the seed inputs into a single u32 so the % falls in [0, 9999].
    let mut h = world_seed.wrapping_mul(0x9E3779B1);
    h ^= (village_id.0 as u32).wrapping_mul(0x85EBCA77);
    h = h.rotate_left(11).wrapping_mul(0xC2B2AE3D);
    h ^= (village_id.1 as u32).wrapping_mul(0x27D4EB2F);
    h = h.rotate_left(7).wrapping_mul(0x165667B1);
    h ^= (day as u32).wrapping_mul(0xD3A2646C);
    h = h.rotate_left(13).wrapping_mul(0xCA9B53FB);
    let bucket = (h ^ (h >> 16)) % 10_000;
    if bucket < roll_threshold_bps(kind) {
        Some(kind)
    } else {
        None
    }
}

/// Where defenders earn bounty credit. Matches Spec 19 villager-claim
/// radius so the geometry stays consistent across the village stack.
pub const DEFENDER_RADIUS_BLOCKS: f32 = 24.0;

/// Mobs spawn ~16-20 blocks from the anchor at the village perimeter.
pub const WAVE_SPAWN_RADIUS_MIN: f32 = 16.0;
pub const WAVE_SPAWN_RADIUS_MAX: f32 = 20.0;

/// How long the smoke-pillar red-shift "raid coming" warning lasts
/// (5 in-game minutes at the standard 20-min day = 5/20 of 24000 ticks
/// = 6000 ticks). The warning fires first; mobs spawn at the end.
pub const RAID_WARNING_TICKS: u64 = 6_000;

/// Raid deadline after spawn — if mobs are still alive past this, the
/// defence is considered failed (Spec §16: "if the deadline expires
/// with mobs still alive, the village reputation tier drops by 1").
/// 10 minutes at 20 TPS.
pub const RAID_DEADLINE_TICKS: u64 = 12_000;

/// Eligibility threshold — see Spec §8 "Raid integration with Iron
/// Golem auto-spawn": 5 villagers AND 5 houses AND treasury > 0.
pub const MIN_ELIGIBLE_VILLAGERS: u32 = 5;
pub const MIN_ELIGIBLE_HOUSES: u32 = 5;

/// Treasury-skim percentages (Phase 13 — THE critical fix). 20 % of
/// every quest payout + 5 % of every Vendor Block sale within the
/// village radius go to the village treasury.
pub const QUEST_TREASURY_SKIM_PERCENT: u64 = 20;
pub const VENDOR_TREASURY_SKIM_PERCENT: u64 = 5;
/// A vendor block within this many blocks of a village anchor counts
/// as "inside the village" for the 5 % tax.
pub const VENDOR_VILLAGE_RADIUS: f32 = 64.0;

/// Reputation deltas applied on raid result. Defenders gain +10 per
/// defence; the killing-blow defender gains +20; failed defence drops
/// the village by one tier (-10 lifts most ranges down a tier).
pub const DEFENDER_REP_GAIN: i16 = 10;
pub const KILLING_BLOW_REP_BONUS: i16 = 10;
pub const FAILED_RAID_REP_PENALTY: i16 = -10;

/// Where the raid currently sits in its lifecycle. Pure-data; the
/// scheduler advances `status` based on tick comparisons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RaidStatus {
    /// Smoke-pillar red-shift is up; mobs haven't spawned yet.
    Warning,
    /// Wave has spawned. Defenders accrue kills.
    Active,
    /// All raid mobs are dead. Bounty has been distributed (or will be
    /// on the next scheduler tick).
    Cleared,
    /// Deadline expired with mobs still alive. Reputation penalty
    /// applied; no bounty.
    Failed,
}

/// One in-flight raid. Lives in `World::active_raids`. The
/// `contribution_table` is a `Vec<(pidx, kills)>` rather than an
/// AHashMap because ahash has no serde feature and the table stays
/// tiny (<= 4 entries even with split-screen).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Raid {
    pub id: RaidId,
    pub village_id: VillageId,
    pub village_anchor: [i32; 3],
    pub wave_kind: WaveKind,
    /// Sats taken from the treasury at raid-schedule time. Returned on
    /// Failed; split among defenders on Cleared.
    pub treasury_drain: u64,
    /// Per-player kill counts. Vec of `(local PlayerSlot index, kills)`
    /// on alpha; multiplayer-pubkey keying lands with Spec 1 Phase 4.
    pub contribution_table: Vec<(usize, u32)>,
    /// Player who landed the killing blow on the LAST raid mob (used
    /// for the +20 bonus). None until the raid clears.
    pub killing_blow: Option<usize>,
    /// Tick at which the warning was raised (used to compute
    /// `spawn_at_tick = warning_at_tick + RAID_WARNING_TICKS`).
    pub warning_at_tick: u64,
    /// Tick at which the wave spawned (set when status transitions
    /// `Warning -> Active`). Used to compute deadline.
    pub spawn_at_tick: Option<u64>,
    pub status: RaidStatus,
    /// Tracks how many raid-tagged mobs are still alive in the ECS.
    /// Decremented by the kill-attribution path; we don't store the
    /// entity ids because hecs::Entity isn't Serialize.
    pub mobs_alive: u32,
    /// Total mobs spawned at start (snapshot — not decremented). Used
    /// to detect raid-cleared without scanning.
    pub mobs_total: u32,
}

impl Raid {
    /// Create a new raid in the Warning state. Caller wires the id,
    /// village_id + anchor, wave kind, treasury drain, and current
    /// tick (used as `warning_at_tick`).
    pub fn new_warning(
        id: RaidId,
        village_id: VillageId,
        village_anchor: [i32; 3],
        wave_kind: WaveKind,
        treasury_drain: u64,
        current_tick: u64,
    ) -> Self {
        Self {
            id,
            village_id,
            village_anchor,
            wave_kind,
            treasury_drain,
            contribution_table: Vec::new(),
            killing_blow: None,
            warning_at_tick: current_tick,
            spawn_at_tick: None,
            status: RaidStatus::Warning,
            mobs_alive: 0,
            mobs_total: wave_kind.total_mobs(),
        }
    }

    /// Compute the spawn tick (the moment the warning ends + mobs come).
    pub fn spawn_tick(&self) -> u64 {
        self.warning_at_tick.saturating_add(RAID_WARNING_TICKS)
    }

    /// Compute the deadline tick (post-spawn). Returns None while still
    /// in the warning window.
    pub fn deadline_tick(&self) -> Option<u64> {
        self.spawn_at_tick
            .map(|t| t.saturating_add(RAID_DEADLINE_TICKS))
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self.status, RaidStatus::Cleared | RaidStatus::Failed)
    }
}

/// Scheduler resource. Lives in `World::raid_scheduler`. Holds the
/// monotonic raid-id counter + the last in-game day on which the daily
/// roll fired (so reloading the world doesn't double-roll the current
/// day).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RaidScheduler {
    pub next_raid_id: RaidId,
    pub last_rolled_day: u64,
}

impl RaidScheduler {
    pub fn new() -> Self {
        Self {
            next_raid_id: 1,
            last_rolled_day: 0,
        }
    }

    /// Allocate the next raid id (monotonic).
    pub fn allocate_id(&mut self) -> RaidId {
        let id = self.next_raid_id;
        self.next_raid_id = self.next_raid_id.wrapping_add(1).max(1);
        id
    }
}

/// Pure: compute the spawn position for the `i`-th mob of a wave around
/// the village anchor. Deterministic on (raid_id, i). Radius randomised
/// inside [WAVE_SPAWN_RADIUS_MIN, WAVE_SPAWN_RADIUS_MAX].
pub fn spawn_position_for(
    anchor: [i32; 3],
    raid_id: RaidId,
    i: u32,
) -> glam::Vec3 {
    let n_total = 32u32; // ring resolution for deterministic angles
    let h = raid_id.wrapping_mul(2654435761).wrapping_add(i.wrapping_mul(0x9E3779B9));
    let angle_step = std::f32::consts::TAU / n_total as f32;
    let angle = (i % n_total) as f32 * angle_step + ((h % 100) as f32 / 1000.0);
    let r_span = WAVE_SPAWN_RADIUS_MAX - WAVE_SPAWN_RADIUS_MIN;
    let radius = WAVE_SPAWN_RADIUS_MIN + ((h.rotate_left(7) % 1000) as f32 / 1000.0) * r_span;
    let dx = radius * angle.cos();
    let dz = radius * angle.sin();
    glam::Vec3::new(
        anchor[0] as f32 + 0.5 + dx,
        anchor[1] as f32 + 1.0,
        anchor[2] as f32 + 0.5 + dz,
    )
}

/// Pure: compute each defender's share of the bounty pool. Returns the
/// per-player credited sats, by PlayerSlot index, in iteration order.
/// Players with zero kills are omitted. The remainder (rounding loss)
/// is returned as the second tuple element and stays in the treasury.
pub fn bounty_shares(
    contribution_table: &[(usize, u32)],
    treasury_drain: u64,
) -> (Vec<(usize, u64)>, u64) {
    let total_kills: u32 = contribution_table.iter().map(|(_, k)| k).sum();
    if total_kills == 0 || treasury_drain == 0 {
        return (Vec::new(), treasury_drain);
    }
    // Deterministic iteration order — sort by pidx so split-screen
    // tests don't flake on table iteration order.
    let mut entries: Vec<(usize, u32)> = contribution_table
        .iter()
        .filter(|(_, k)| *k > 0)
        .copied()
        .collect();
    entries.sort_by_key(|(p, _)| *p);
    let mut out: Vec<(usize, u64)> = Vec::with_capacity(entries.len());
    let mut paid: u64 = 0;
    for (pidx, kills) in entries {
        let share = (treasury_drain as u128 * kills as u128 / total_kills as u128) as u64;
        if share > 0 {
            out.push((pidx, share));
            paid += share;
        }
    }
    let remainder = treasury_drain.saturating_sub(paid);
    (out, remainder)
}

/// Helper: increment the contribution-table entry for `pidx` by 1,
/// inserting a new entry if absent.
pub fn record_kill(table: &mut Vec<(usize, u32)>, pidx: usize) {
    for entry in table.iter_mut() {
        if entry.0 == pidx {
            entry.1 = entry.1.saturating_add(1);
            return;
        }
    }
    table.push((pidx, 1));
}

/// Pure: compute the 20 % treasury skim on a quest payout. Returns
/// `(player_keeps, village_skim)`. Sum is always the input.
pub fn split_quest_payout(gross_sats: u64) -> (u64, u64) {
    let skim = gross_sats * QUEST_TREASURY_SKIM_PERCENT / 100;
    (gross_sats.saturating_sub(skim), skim)
}

/// Pure: compute the 5 % treasury skim on a vendor sale. Returns
/// `(seller_keeps, village_skim)`.
pub fn split_vendor_payment(price_sats: u64) -> (u64, u64) {
    let skim = price_sats * VENDOR_TREASURY_SKIM_PERCENT / 100;
    (price_sats.saturating_sub(skim), skim)
}

/// Pure: find the nearest village anchor to `pos` within
/// `VENDOR_VILLAGE_RADIUS`. Returns the village id when there's a hit
/// — meaning a vendor at `pos` is "inside" that village for the tax
/// rule. Iterates `village_anchors`; cheap with handfuls of villages.
pub fn village_for_vendor_position(
    anchors: &AHashMap<VillageId, [i32; 3]>,
    pos: (i32, i32, i32),
) -> Option<VillageId> {
    let p = glam::Vec3::new(pos.0 as f32, pos.1 as f32, pos.2 as f32);
    let mut best: Option<(VillageId, f32)> = None;
    for (&vid, &a) in anchors {
        let av = glam::Vec3::new(a[0] as f32, a[1] as f32, a[2] as f32);
        let d = (av - p).length();
        if d <= VENDOR_VILLAGE_RADIUS
            && best.map(|(_, bd)| d < bd).unwrap_or(true)
        {
            best = Some((vid, d));
        }
    }
    best.map(|(vid, _)| vid)
}

/// Phase 13 — optional server-top-up hook. Alpha is a no-op (no
/// admin UI surface yet); kept here so when Bitcoin-enabled servers
/// configure per-day donations the existing code path is the
/// integration point. No caller yet by design (there's no admin UI to
/// drive it), unlike `credit_treasury` below which is live.
#[allow(dead_code)]
pub fn apply_server_topup(
    _treasuries: &mut AHashMap<VillageId, u64>,
    _per_day_sats: u64,
) {
    // No-op on alpha. Servers wanting top-up implement here.
}

/// Phase 13 — credit `skim` sats into the village treasury at
/// `village_id`. Returns the new treasury balance.
pub fn credit_treasury(
    treasuries: &mut AHashMap<VillageId, u64>,
    village_id: VillageId,
    skim: u64,
) -> u64 {
    let entry = treasuries.entry(village_id).or_insert(0);
    *entry = entry.saturating_add(skim);
    *entry
}

/// Find the village id within `DEFENDER_RADIUS_BLOCKS` of the given
/// player position. None if no village is in range. Iterates the
/// anchors map; cheap with handfuls of villages. No production caller
/// yet — tested directly.
#[cfg_attr(not(test), allow(dead_code))]
pub fn village_within_defender_radius(
    anchors: &AHashMap<VillageId, [i32; 3]>,
    player_pos: glam::Vec3,
) -> Option<VillageId> {
    let mut best: Option<(VillageId, f32)> = None;
    for (&vid, &a) in anchors {
        let av = glam::Vec3::new(a[0] as f32 + 0.5, a[1] as f32 + 0.5, a[2] as f32 + 0.5);
        let d = (av - player_pos).length();
        if d <= DEFENDER_RADIUS_BLOCKS
            && best.map(|(_, bd)| d < bd).unwrap_or(true)
        {
            best = Some((vid, d));
        }
    }
    best.map(|(vid, _)| vid)
}

/// Marker component attached to every mob spawned as part of a raid.
/// Lets the kill-attribution path identify which raid a dead mob
/// belonged to, so the contribution table can be incremented and the
/// "wave cleared" check can fire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RaidMember {
    pub raid_id: RaidId,
}

/// Per-village population snapshot used by the eligibility check.
/// Counts villagers (claimed or not) within the defender radius of
/// `anchor`. The "5 houses" gate is checked separately because the
/// procgen layout already exposes its house count via
/// `village_gen::layout_for_cell`.
pub fn count_villagers_near_anchor(
    ecs: &hecs::World,
    anchor: [i32; 3],
) -> u32 {
    use crate::entity::{MobKind, Position};
    let anchor_pos = glam::Vec3::new(
        anchor[0] as f32 + 0.5,
        anchor[1] as f32 + 0.5,
        anchor[2] as f32 + 0.5,
    );
    let mut count = 0u32;
    for (_id, (kind, pos)) in ecs.query::<(&MobKind, &Position)>().iter() {
        if !crate::villager::is_villager_kind(kind.0) {
            continue;
        }
        if (pos.0 - anchor_pos).length() <= DEFENDER_RADIUS_BLOCKS {
            count += 1;
        }
    }
    count
}

/// Per-village house count snapshot. Uses the deterministic
/// `village_gen::layout_for_cell` rather than scanning the world.
pub fn house_count_for_village(
    biome_gen: &crate::biome::BiomeGenerator,
    village_id: VillageId,
) -> u32 {
    let world_seed = biome_gen.seed;
    crate::village_gen::layout_for_cell(world_seed, village_id.0, village_id.1, biome_gen)
        .map(|layout| layout.houses.len() as u32)
        .unwrap_or(0)
}

/// Spec 22 Phase 2 — daily-tick scheduler. Walks every eligible
/// village (≥ MIN villagers + ≥ MIN houses + treasury > 0) and rolls
/// the deterministic per-(world_seed, village_id, day) probability.
/// On a hit: drains the treasury by the wave-tier cap, allocates a
/// raid id, appends a Warning-state `Raid` to `world.active_raids`.
/// Idempotent within a single day — the scheduler stamps
/// `last_rolled_day` so a re-call same day no-ops.
///
/// Returns the list of villages that just got a raid scheduled
/// (informational; caller surfaces toasts).
pub fn tick_scheduler_daily(
    world: &mut crate::world::World,
    ecs: &hecs::World,
    biome_gen: &crate::biome::BiomeGenerator,
    current_day: u64,
    current_tick: u64,
) -> Vec<RaidId> {
    if current_day == world.raid_scheduler.last_rolled_day && current_day != 0 {
        return Vec::new();
    }
    world.raid_scheduler.last_rolled_day = current_day;

    let world_seed = biome_gen.seed;
    let anchors: Vec<(VillageId, [i32; 3])> = world
        .village_anchors
        .iter()
        .map(|(&k, &v)| (k, v))
        .collect();
    let mut scheduled: Vec<RaidId> = Vec::new();

    for (vid, anchor) in anchors {
        // Skip villages that already have an active (non-terminal) raid.
        let already = world
            .active_raids
            .iter()
            .any(|r| r.village_id == vid && !r.is_terminal());
        if already {
            continue;
        }
        let villagers = count_villagers_near_anchor(ecs, anchor);
        if villagers < MIN_ELIGIBLE_VILLAGERS {
            continue;
        }
        let houses = house_count_for_village(biome_gen, vid);
        if houses < MIN_ELIGIBLE_HOUSES {
            continue;
        }
        let treasury = world.village_treasuries.get(&vid).copied().unwrap_or(0);
        let Some(wave_kind) = roll_for_day(world_seed, vid, current_day, treasury) else {
            continue;
        };
        let drain = treasury_drain_for(treasury, wave_kind);
        if drain == 0 {
            continue;
        }
        // Drain the treasury upfront — refunded on Failed (the raid
        // tick processor will write it back); split among defenders
        // on Cleared.
        if let Some(t) = world.village_treasuries.get_mut(&vid) {
            *t = t.saturating_sub(drain);
        }
        let id = world.raid_scheduler.allocate_id();
        world.active_raids.push(Raid::new_warning(
            id, vid, anchor, wave_kind, drain, current_tick,
        ));
        scheduled.push(id);
    }
    scheduled
}

/// Spec 22 Phase 7 — set / clear the `raid_warning_active` flag on the
/// village's campfire(s). The flag drives the smoke-pillar red-shift
/// (mesh builder reads it to pick the tinted texture variant).
/// Idempotent. Returns the set of chunks whose meshes need rebuilding
/// so the caller can mark them dirty — covers the campfire's own
/// chunk plus every chunk the smoke pillar passes through.
pub fn set_warning_on_village_campfires(
    world: &mut crate::world::World,
    village_id: VillageId,
    active: bool,
) -> ahash::AHashSet<(i32, i32, i32)> {
    use crate::block;
    let mut dirty: ahash::AHashSet<(i32, i32, i32)> = ahash::AHashSet::new();
    // Walk every campfire-block-entity position; flip the flag on
    // those within the defender radius of the village's anchor.
    let Some(anchor) = world.village_anchors.get(&village_id).copied() else {
        return dirty;
    };
    let anchor_pos = glam::Vec3::new(
        anchor[0] as f32 + 0.5,
        anchor[1] as f32 + 0.5,
        anchor[2] as f32 + 0.5,
    );
    let positions: Vec<(i32, i32, i32)> = world
        .iter_campfires()
        .map(|(p, _)| p)
        .filter(|(x, y, z)| {
            let p = glam::Vec3::new(*x as f32 + 0.5, *y as f32 + 0.5, *z as f32 + 0.5);
            (p - anchor_pos).length() <= DEFENDER_RADIUS_BLOCKS
        })
        .filter(|(x, y, z)| {
            let b = world.get_block(*x, *y, *z);
            b == block::CAMPFIRE || b == block::CAMPFIRE_UNLIT
        })
        .collect();
    for pos in positions {
        if let Some(cf) = world.campfire_at_mut(pos) {
            cf.raid_warning_active = active;
        }
        // Mark the campfire's chunk + every chunk the smoke pillar
        // could span (column of SMOKE_PILLAR_HEIGHT cells above).
        // Doing this regardless of pre/post-flag value keeps the
        // function idempotent and rebuilds harmlessly if the flag
        // didn't actually change.
        dirty.insert(crate::world::World::block_to_chunk(pos.0, pos.1, pos.2));
        for dy in 1..=crate::campfire::SMOKE_PILLAR_HEIGHT {
            dirty.insert(crate::world::World::block_to_chunk(pos.0, pos.1 + dy, pos.2));
        }
    }
    dirty
}

/// Per-raid tick effect — captured for the caller (game_loop) to
/// translate into reputation / bounty mutations on PlayerSlots, since
/// those state-mutating pieces aren't on the World.
#[derive(Clone, Debug)]
pub struct RaidResolution {
    pub raid_id: RaidId,
    pub village_id: VillageId,
    pub status: RaidStatus,
    pub treasury_drain: u64,
    pub contribution_table: Vec<(usize, u32)>,
    pub killing_blow: Option<usize>,
}

/// Spec 22 Phase 4 + Phase 6 — per-tick raid lifecycle update.
///
/// For each active raid:
/// - `Warning` + tick ≥ spawn_tick → spawn the wave's mobs at the
///   perimeter, flip to `Active`, tag each spawned mob with
///   `RaidMember(raid_id)` (entity attachment happens in game_loop —
///   this fn just produces the spawn list).
/// - `Active` + mobs_alive == 0 → flip to `Cleared`.
/// - `Active` + tick ≥ deadline → flip to `Failed`.
///
/// Returns the spawn list (for `Warning -> Active` transitions) plus
/// the resolutions for `Cleared` / `Failed` raids that the caller
/// settles. Terminal raids are NOT removed from `world.active_raids`
/// here — the caller drains them after applying the resolution so the
/// PlayerSlot reputation + sats updates have the contribution table.
pub fn tick_raids(
    world: &mut crate::world::World,
    current_tick: u64,
) -> (Vec<(RaidId, Vec<(MobType, glam::Vec3)>)>, Vec<RaidResolution>) {
    let mut to_spawn: Vec<(RaidId, Vec<(MobType, glam::Vec3)>)> = Vec::new();
    let mut resolutions: Vec<RaidResolution> = Vec::new();

    for raid in world.active_raids.iter_mut() {
        match raid.status {
            RaidStatus::Warning => {
                if current_tick >= raid.spawn_tick() {
                    // Build the spawn list deterministically.
                    let mut spawns: Vec<(MobType, glam::Vec3)> = Vec::new();
                    let mut i = 0u32;
                    for &(kind, count) in raid.wave_kind.composition() {
                        for _ in 0..count {
                            let pos = spawn_position_for(raid.village_anchor, raid.id, i);
                            spawns.push((kind, pos));
                            i += 1;
                        }
                    }
                    raid.mobs_alive = raid.mobs_total;
                    raid.spawn_at_tick = Some(current_tick);
                    raid.status = RaidStatus::Active;
                    to_spawn.push((raid.id, spawns));
                }
            }
            RaidStatus::Active => {
                if raid.mobs_alive == 0 {
                    raid.status = RaidStatus::Cleared;
                    resolutions.push(RaidResolution {
                        raid_id: raid.id,
                        village_id: raid.village_id,
                        status: RaidStatus::Cleared,
                        treasury_drain: raid.treasury_drain,
                        contribution_table: raid.contribution_table.clone(),
                        killing_blow: raid.killing_blow,
                    });
                } else if let Some(deadline) = raid.deadline_tick()
                    && current_tick >= deadline {
                        raid.status = RaidStatus::Failed;
                        resolutions.push(RaidResolution {
                            raid_id: raid.id,
                            village_id: raid.village_id,
                            status: RaidStatus::Failed,
                            treasury_drain: raid.treasury_drain,
                            contribution_table: raid.contribution_table.clone(),
                            killing_blow: raid.killing_blow,
                        });
                    }
            }
            // Terminal — should have been drained last tick. Skip.
            RaidStatus::Cleared | RaidStatus::Failed => {}
        }
    }
    (to_spawn, resolutions)
}

// NOTE (2026-07-12): a `raid_at_position(world, pos) -> Option<RaidId>`
// helper lived here, "meant to gate kill-attribution + surface the dialog
// raid-bounty line". Both consumers shipped 2026-05-22 with different
// primitives — attribution gates on player-to-anchor distance inline in
// the game_loop death sweep (Phase 5), the dialog banner goes
// `village_at_position` → `active_raid_for_village` (Phase 9) — so the
// helper stayed permanently dead and its comment kept re-generating a
// false "kill attribution is missing" backlog item. Removed; the live
// chain is pinned end-to-end by
// `test_game_harness::game_harness_raid_kill_attribution_settles_through_the_live_game_loop`.

/// Lookup: village_id of an active raid, if there is one at the given
/// village.
pub fn active_raid_for_village(
    world: &crate::world::World,
    village_id: VillageId,
) -> Option<&Raid> {
    world
        .active_raids
        .iter()
        .find(|r| r.village_id == village_id && !r.is_terminal())
}

/// Find the mutable Raid entry for `raid_id`, if any.
pub fn find_raid_mut(
    world: &mut crate::world::World,
    raid_id: RaidId,
) -> Option<&mut Raid> {
    world.active_raids.iter_mut().find(|r| r.id == raid_id)
}

/// Spawn a raid-tagged mob into the ECS. Mirrors `entity::spawn_mob`
/// then attaches a `RaidMember(raid_id)` component to the newly-spawned
/// entity so the kill-attribution path can identify it as part of a
/// raid. Returns the spawned entity id.
pub fn spawn_raid_mob(
    ecs: &mut hecs::World,
    kind: MobType,
    position: glam::Vec3,
    raid_id: RaidId,
) -> hecs::Entity {
    use crate::combat::Health;
    use crate::entity::{Hitbox, MobKind, OnGround, Position, Velocity};
    let def = crate::mob::mob_def(kind);
    
    ecs.spawn((
        Position(position),
        Velocity(glam::Vec3::ZERO),
        MobKind(kind),
        Hitbox { width: def.width, height: def.height },
        OnGround(false),
        crate::mob_ai::MobAi::new(),
        Health::new(def.health as f32),
        RaidMember { raid_id },
    ))
}

// ---------------------------------------------------------------------------
// Spec 22 Phase 16 — rare-loot tiered drops per wave kind.
//
// Per `docs/vision/sat-flow-and-economy-loops.md` §3.3 Bitcoin/Barter
// parity: raid bounties pay items + reputation ALWAYS; sats are an
// EXTRA in Bitcoin mode. Each wave kind drops a tier-keyed rare-item
// bonus on top of the standard mob drops. Bitcoin-disabled servers
// still get the rare items; sats payout simply suppresses upstream
// (handled by `economy::apply_sats_payout`).
//
// v1: flat-per-contributor model — each defender with at least one
// kill gets the wave kind's bonus stack. Simpler than proportional
// (clearer fairness, less maths, no rounding-loss complexity), and
// reads cleanly to the kid: "you defended → here's your reward".
// Proportional split is a polish pass.
//
// Bitcoin-disabled servers swap the Large-wave Diamond shard for a
// quest-only Bone bundle (drop-table consolation per spec §D). The
// sats line still suppresses upstream; this swap is the visible
// items-only equivalent of the lost sats bonus.
// ---------------------------------------------------------------------------

/// Whether the server is in Bitcoin-enabled mode. Flat boolean; the
/// per-player Charter flag is separate (Charter gates payouts at the
/// `apply_sats_payout` layer). `bonus_drops_for` reads this to pick
/// the right tier-3 table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerEconomyMode {
    BitcoinEnabled,
    BitcoinDisabled,
}

/// Pure: the per-contributor bonus stack for a defender of a wave of
/// the given kind. Returns an empty Vec for `Small` (standard mob
/// drops only); a tier-2 item for `Medium`; a tier-3 item for `Large`.
/// `economy` swaps the Large tier-3 line on Bitcoin-disabled servers
/// to a quest-only consolation per spec §D.
pub fn bonus_drops_for(
    wave_kind: WaveKind,
    economy: ServerEconomyMode,
) -> Vec<crate::item::ItemStack> {
    use crate::crafting::{Tool, ToolMaterial, ToolType};
    use crate::item::{ItemStack, MaterialId};
    match wave_kind {
        // Small: no extra. Standard mob drops only.
        WaveKind::Small => Vec::new(),
        // Medium: 1 iron ingot — solid mid-tier crafting input that
        // pairs with mob loot. Picked over "iron tool" because tools
        // don't stack and a single ingot reads as a clean bonus line.
        WaveKind::Medium => vec![ItemStack::new_material(MaterialId::IronIngot, 1)],
        // Large: 1 diamond on Bitcoin-enabled; 1 iron sword on
        // Bitcoin-disabled (a tier-3 quest-only equivalent — the sats
        // line is the lost extra, so the items-only consolation is
        // bumped up in kind to a usable weapon).
        WaveKind::Large => match economy {
            ServerEconomyMode::BitcoinEnabled => {
                vec![ItemStack::new_material(MaterialId::Diamond, 1)]
            }
            ServerEconomyMode::BitcoinDisabled => {
                vec![ItemStack::new_tool(Tool::new(ToolType::Sword, ToolMaterial::Iron))]
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Spec 22 Phase 17 — Vendor Block raid-supplies highlight.
//
// When a raid is **warned** (Warning state), every Vendor Block within
// 32 blocks of the village's anchor whose listed item passes the
// raid-supplies whitelist gets a 🛡️ tag in its buyer UI. Creates
// demand pressure for raid-prep items (food, arrows, swords, healing).
//
// Distance check uses 32 blocks (not the 64-block vendor-tax radius)
// because the highlight is about proximity to the threat — vendors at
// the far edge of the tax radius aren't visibly "close to the village
// being attacked" to the player.
// ---------------------------------------------------------------------------

/// Spec 22 Phase 17 — Vendor Block highlight proximity radius. Tighter
/// than the 64-block tax radius so highlights cluster around the
/// actual village being threatened.
pub const RAID_SUPPLIES_HIGHLIGHT_RADIUS: f32 = 32.0;

/// Pure: whether `item` counts as a raid-supplies item. Food, arrows,
/// swords (any tier), and healing items. Cobblestone / raw wood /
/// unrelated materials return false. Keeps the predicate simple — the
/// goal is "this would be useful in a raid", not exhaustive coverage.
pub fn is_raid_supplies(item: &crate::item::Item) -> bool {
    use crate::crafting::ToolType;
    use crate::item::{Item, MaterialId};
    // Food: any item with a food_value (covers cooked meats, bread,
    // baked veg, cake, honey bottle, etc).
    if item.is_food() {
        return true;
    }
    match item {
        // Arrows — direct raid combat consumable.
        Item::Material(MaterialId::Arrow) => true,
        // Healing — honey bottle counts as food and was caught above,
        // but cake also counts; bonemeal isn't healing. The
        // food-value path covers all current healing-classified items.
        // Swords — any tier.
        Item::Tool(t) if matches!(t.tool_type, ToolType::Sword) => true,
        _ => false,
    }
}

/// Pure: find the nearest village id whose anchor is within `radius`
/// blocks of `pos`. Returns None if no village is in range. Used by
/// the Vendor Block raid-supplies highlight to gate "is this vendor
/// inside a raid-warned village's neighbourhood?".
pub fn nearest_village_within(
    anchors: &AHashMap<VillageId, [i32; 3]>,
    pos: glam::Vec3,
    radius: f32,
) -> Option<VillageId> {
    let mut best: Option<(VillageId, f32)> = None;
    for (&vid, &a) in anchors {
        let av = glam::Vec3::new(a[0] as f32 + 0.5, a[1] as f32 + 0.5, a[2] as f32 + 0.5);
        let d = (av - pos).length();
        if d <= radius && best.map(|(_, bd)| d < bd).unwrap_or(true) {
            best = Some((vid, d));
        }
    }
    best.map(|(vid, _)| vid)
}

/// Pure: whether a Vendor Block at `vendor_pos` listing `item` should
/// surface the 🛡️ Raid-Supplies highlight. True iff (a) the item
/// passes `is_raid_supplies`, (b) there's a village within
/// `RAID_SUPPLIES_HIGHLIGHT_RADIUS` of the vendor, and (c) that
/// village has a raid in Warning state.
pub fn should_highlight_as_raid_supplies(
    world: &crate::world::World,
    vendor_pos: glam::Vec3,
    item: &crate::item::Item,
) -> bool {
    if !is_raid_supplies(item) {
        return false;
    }
    let Some(vid) = nearest_village_within(
        &world.village_anchors,
        vendor_pos,
        RAID_SUPPLIES_HIGHLIGHT_RADIUS,
    ) else {
        return false;
    };
    world
        .active_raids
        .iter()
        .any(|r| r.village_id == vid && r.status == RaidStatus::Warning)
}

// ---------------------------------------------------------------------------
// Spec 22 Phase 18 — raid leaderboard at the village.
//
// Per-village per-player kill totals across all raids defended there.
// Updated on raid-Cleared resolution; villager dialogue exposes the
// top-3 defenders. Pure social signal — no reputation effect, no
// mechanical gating.
//
// `PlayerKey` is the local PlayerSlot index on alpha. When multiplayer
// pubkey-keying lands with Spec 1 Phase 4 this lifts to the pubkey
// string (the leaderboard becomes globally meaningful then); for solo +
// split-screen alpha the slot index is good-enough fairness.
// ---------------------------------------------------------------------------

/// Per-player identifier on the leaderboard. Local PlayerSlot index on
/// alpha; pubkey string when Spec 1 Phase 4 lands.
pub type PlayerKey = usize;

/// Pure: increment per-village per-player kill totals from a raid's
/// `contribution_table`. The table is keyed by PlayerSlot index already
/// so this is a straight aggregation. Empty contribution_table is a
/// no-op (a raid with zero defenders adds nothing).
pub fn tally_raid_kills_into_leaderboard(
    leaderboard: &mut AHashMap<(VillageId, PlayerKey), u32>,
    village_id: VillageId,
    contribution_table: &[(usize, u32)],
) {
    for &(pidx, kills) in contribution_table {
        if kills == 0 {
            continue;
        }
        let entry = leaderboard.entry((village_id, pidx)).or_insert(0);
        *entry = entry.saturating_add(kills);
    }
}

/// Pure: return the top-K defenders of `village_id` by kill total,
/// sorted descending. Ties broken by lower PlayerKey first
/// (deterministic). Skips zero-kill entries (they shouldn't be in the
/// map anyway, but guard).
pub fn top_defenders(
    leaderboard: &AHashMap<(VillageId, PlayerKey), u32>,
    village_id: VillageId,
    k: usize,
) -> Vec<(PlayerKey, u32)> {
    let mut entries: Vec<(PlayerKey, u32)> = leaderboard
        .iter()
        .filter_map(|(&(vid, pk), &count)| {
            if vid == village_id && count > 0 {
                Some((pk, count))
            } else {
                None
            }
        })
        .collect();
    // Sort descending by kills, ties by PlayerKey ascending.
    entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    entries.truncate(k);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wave_kind_thresholds_are_monotonic_with_treasury() {
        // Spec §7: 0 → no raid, small/medium/large at increasing tiers.
        assert_eq!(wave_kind_for_treasury(0), None);
        assert_eq!(wave_kind_for_treasury(1), Some(WaveKind::Small));
        assert_eq!(wave_kind_for_treasury(100), Some(WaveKind::Small));
        assert_eq!(wave_kind_for_treasury(499), Some(WaveKind::Small));
        assert_eq!(wave_kind_for_treasury(500), Some(WaveKind::Medium));
        assert_eq!(wave_kind_for_treasury(4_999), Some(WaveKind::Medium));
        assert_eq!(wave_kind_for_treasury(5_000), Some(WaveKind::Large));
        assert_eq!(wave_kind_for_treasury(1_000_000), Some(WaveKind::Large));
    }

    #[test]
    fn wave_compositions_total_their_advertised_size() {
        for kind in [WaveKind::Small, WaveKind::Medium, WaveKind::Large] {
            let total: u32 = kind.composition().iter().map(|(_, c)| c).sum();
            assert_eq!(total, kind.total_mobs(), "{:?} composition total mismatch", kind);
        }
    }

    // HP-5 — wave-composition refresh tests.

    #[test]
    fn small_wave_is_four_brigands_plus_one_marauder() {
        let comp = WaveKind::Small.composition();
        assert!(
            comp.iter().any(|(m, n)| *m == MobType::Brigand && *n == 4),
            "Small wave should have 4 Brigands, got {:?}", comp);
        assert!(
            comp.iter().any(|(m, n)| *m == MobType::Marauder && *n == 1),
            "Small wave should have 1 Marauder, got {:?}", comp);
    }

    #[test]
    fn medium_wave_includes_first_berserker() {
        let comp = WaveKind::Medium.composition();
        assert!(
            comp.iter().any(|(m, n)| *m == MobType::Berserker && *n == 1),
            "Medium wave should have 1 Berserker, got {:?}", comp);
    }

    #[test]
    fn large_wave_has_multiple_berserkers() {
        let comp = WaveKind::Large.composition();
        let berserker_count: u32 = comp.iter()
            .filter(|(m, _)| *m == MobType::Berserker)
            .map(|(_, n)| n)
            .sum();
        assert!(
            berserker_count >= 2,
            "Large wave should have ≥2 Berserkers, got {berserker_count}");
    }

    #[test]
    fn no_fantasy_mobs_in_any_wave_after_hp5() {
        // The historical pivot's whole point — raid waves only ever spawn
        // the brigand-family roster (the fantasy mobs were excised entirely).
        for kind in [WaveKind::Small, WaveKind::Medium, WaveKind::Large] {
            for (m, _) in kind.composition() {
                assert!(
                    matches!(
                        m,
                        MobType::Brigand | MobType::Marauder | MobType::Berserker
                    ),
                    "wave {:?} contains non-brigand mob {:?}", kind, m);
            }
        }
    }

    #[test]
    fn scheduler_roll_is_deterministic_for_same_inputs() {
        // Same (seed, village, day, treasury) → same outcome every time.
        let seed = 12345u32;
        let vid = (3, 7);
        for day in 0..10 {
            for treasury in [100u64, 800, 6000] {
                let a = roll_for_day(seed, vid, day, treasury);
                let b = roll_for_day(seed, vid, day, treasury);
                assert_eq!(a, b, "non-deterministic roll at day {} treasury {}", day, treasury);
            }
        }
    }

    #[test]
    fn empty_treasury_never_rolls_a_raid() {
        // Every (seed, village, day) returns None when treasury == 0.
        for seed in [1u32, 42, 99_991] {
            for day in 0..30u64 {
                for vid in [(0, 0), (5, -3), (-12, 8)] {
                    assert_eq!(roll_for_day(seed, vid, day, 0), None);
                }
            }
        }
    }

    #[test]
    fn roll_thresholds_scale_with_wave_kind() {
        // Run many days for many villages at each treasury tier and
        // check the empirical hit rate is in the right ballpark.
        let seed = 0xC001u32;
        let n_days = 1000u64;
        let n_villages = 20i32;

        fn empirical_hit_rate(seed: u32, treasury: u64, n_days: u64, n_villages: i32) -> f64 {
            let mut hits = 0u32;
            let mut total = 0u32;
            for vgx in 0..n_villages {
                for day in 0..n_days {
                    total += 1;
                    if roll_for_day(seed, (vgx, 0), day, treasury).is_some() {
                        hits += 1;
                    }
                }
            }
            hits as f64 / total as f64
        }
        // Small: 20 % expected (allow ±5 % wiggle for hash variance).
        let r_small = empirical_hit_rate(seed, 100, n_days, n_villages);
        assert!(r_small > 0.15 && r_small < 0.25, "small rate off: {r_small}");
        // Medium: 40 % expected.
        let r_med = empirical_hit_rate(seed, 1000, n_days, n_villages);
        assert!(r_med > 0.35 && r_med < 0.45, "medium rate off: {r_med}");
        // Large: 60 % expected.
        let r_lg = empirical_hit_rate(seed, 50000, n_days, n_villages);
        assert!(r_lg > 0.55 && r_lg < 0.65, "large rate off: {r_lg}");
    }

    #[test]
    fn treasury_drain_caps_at_per_wave_ceiling() {
        // Empty pool → empty drain.
        assert_eq!(treasury_drain_for(0, WaveKind::Small), 0);
        // Small drain capped at 100.
        assert_eq!(treasury_drain_for(50, WaveKind::Small), 50);
        assert_eq!(treasury_drain_for(500, WaveKind::Small), 100);
        // Medium capped at 500.
        assert_eq!(treasury_drain_for(10_000, WaveKind::Medium), 500);
        // Large capped at 2000.
        assert_eq!(treasury_drain_for(10_000_000, WaveKind::Large), 2_000);
    }

    #[test]
    fn spawn_position_is_inside_perimeter_band() {
        let anchor = [100, 64, -50];
        for i in 0..40 {
            let p = spawn_position_for(anchor, 1, i);
            let dx = p.x - (anchor[0] as f32 + 0.5);
            let dz = p.z - (anchor[2] as f32 + 0.5);
            let r = (dx * dx + dz * dz).sqrt();
            // Allow a small epsilon for fp; min/max already inside band.
            assert!(
                r >= WAVE_SPAWN_RADIUS_MIN - 0.01
                    && r <= WAVE_SPAWN_RADIUS_MAX + 0.01,
                "spawn radius {r} outside [{}, {}]",
                WAVE_SPAWN_RADIUS_MIN,
                WAVE_SPAWN_RADIUS_MAX
            );
        }
    }

    #[test]
    fn bounty_shares_split_proportional_to_kills() {
        // 100 drain, p0=3 kills, p1=1 kill → p0 gets 75, p1 gets 25.
        let ct = vec![(0, 3), (1, 1)];
        let (shares, rem) = bounty_shares(&ct, 100);
        let map: AHashMap<usize, u64> = shares.into_iter().collect();
        assert_eq!(map.get(&0).copied(), Some(75));
        assert_eq!(map.get(&1).copied(), Some(25));
        assert_eq!(rem, 0);
    }

    #[test]
    fn bounty_shares_with_zero_total_kills_returns_empty() {
        let ct: Vec<(usize, u32)> = Vec::new();
        let (shares, rem) = bounty_shares(&ct, 100);
        assert!(shares.is_empty());
        assert_eq!(rem, 100, "no kills → drain stays in treasury (unrefunded yet)");
    }

    #[test]
    fn bounty_shares_remainder_rolls_back_into_treasury() {
        // 10 drain, three players with 1 kill each → 3,3,3 = 9; 1 left.
        let ct = vec![(0, 1), (1, 1), (2, 1)];
        let (shares, rem) = bounty_shares(&ct, 10);
        let total: u64 = shares.iter().map(|(_, s)| s).sum();
        assert_eq!(total + rem, 10, "shares + remainder must equal drain");
        assert_eq!(rem, 1);
    }

    #[test]
    fn record_kill_inserts_then_increments() {
        let mut table: Vec<(usize, u32)> = Vec::new();
        record_kill(&mut table, 0);
        assert_eq!(table, vec![(0, 1)]);
        record_kill(&mut table, 0);
        assert_eq!(table, vec![(0, 2)]);
        record_kill(&mut table, 1);
        assert_eq!(table, vec![(0, 2), (1, 1)]);
    }

    #[test]
    fn split_quest_payout_takes_twenty_percent_for_treasury() {
        let (kid, village) = split_quest_payout(100);
        assert_eq!(kid, 80);
        assert_eq!(village, 20);
        // Edge: zero gross → zero each.
        let (k0, v0) = split_quest_payout(0);
        assert_eq!(k0, 0);
        assert_eq!(v0, 0);
        // Edge: small value loses precision in the player's favour.
        let (k_small, v_small) = split_quest_payout(1);
        assert_eq!(k_small + v_small, 1);
        assert_eq!(v_small, 0, "1 sat → 0 to village (rounding favours player)");
    }

    #[test]
    fn split_vendor_payment_takes_five_percent_for_treasury() {
        let (seller, village) = split_vendor_payment(100);
        assert_eq!(seller, 95);
        assert_eq!(village, 5);
        let (s, v) = split_vendor_payment(40);
        assert_eq!(s + v, 40);
        assert_eq!(v, 2, "40 × 5 % = 2 (integer)");
    }

    #[test]
    fn village_for_vendor_position_respects_radius() {
        let mut anchors = AHashMap::new();
        anchors.insert((0, 0), [100, 64, 100]);
        anchors.insert((1, 0), [500, 64, 100]);
        // Inside 64 of (0,0).
        assert_eq!(
            village_for_vendor_position(&anchors, (140, 64, 100)),
            Some((0, 0))
        );
        // Closer to (1,0) but outside 64-block radius of either.
        assert_eq!(
            village_for_vendor_position(&anchors, (300, 64, 100)),
            None
        );
        // Inside 64 of (1,0).
        assert_eq!(
            village_for_vendor_position(&anchors, (540, 64, 100)),
            Some((1, 0))
        );
    }

    #[test]
    fn credit_treasury_accumulates_idempotently() {
        let mut t = AHashMap::new();
        let after_first = credit_treasury(&mut t, (0, 0), 10);
        assert_eq!(after_first, 10);
        let after_second = credit_treasury(&mut t, (0, 0), 15);
        assert_eq!(after_second, 25);
        // Different village → separate balance.
        let other = credit_treasury(&mut t, (1, 1), 7);
        assert_eq!(other, 7);
        assert_eq!(t.get(&(0, 0)).copied(), Some(25));
    }

    #[test]
    fn scheduler_allocates_monotonic_ids_starting_at_one() {
        let mut s = RaidScheduler::new();
        assert_eq!(s.allocate_id(), 1);
        assert_eq!(s.allocate_id(), 2);
        assert_eq!(s.allocate_id(), 3);
    }

    // --- Scheduler + lifecycle ---

    fn make_world_with_one_village(
        anchor: [i32; 3],
        vid: VillageId,
        treasury: u64,
    ) -> crate::world::World {
        let mut world = crate::world::World::new();
        world.village_anchors.insert(vid, anchor);
        if treasury > 0 {
            world.village_treasuries.insert(vid, treasury);
        }
        world
    }

    fn populate_villagers(
        ecs: &mut hecs::World,
        anchor: [i32; 3],
        count: u32,
    ) {
        let anchor_pos = glam::Vec3::new(
            anchor[0] as f32 + 0.5,
            anchor[1] as f32 + 0.5,
            anchor[2] as f32 + 0.5,
        );
        for i in 0..count {
            let pos = anchor_pos + glam::Vec3::new(i as f32 * 0.5, 0.0, 0.5);
            crate::entity::spawn_mob(ecs, MobType::Villager, pos);
        }
    }

    #[test]
    fn tick_raids_warning_to_active_emits_spawn_list_of_correct_size() {
        // A Warning raid past spawn_tick should flip to Active and the
        // returned spawn list should match the wave kind's count.
        let mut world = make_world_with_one_village([0, 64, 0], (0, 0), 5_000);
        world.active_raids.push(Raid::new_warning(
            1,
            (0, 0),
            [0, 64, 0],
            WaveKind::Medium,
            500,
            0,
        ));
        let (to_spawn, resolutions) = tick_raids(&mut world, RAID_WARNING_TICKS + 1);
        assert!(resolutions.is_empty());
        assert_eq!(to_spawn.len(), 1);
        let (raid_id, spawns) = &to_spawn[0];
        assert_eq!(*raid_id, 1);
        assert_eq!(spawns.len() as u32, WaveKind::Medium.total_mobs());
        // All spawns are inside the perimeter band.
        for (_, pos) in spawns {
            let dx = pos.x;
            let dz = pos.z;
            let r = (dx * dx + dz * dz).sqrt();
            assert!(r >= WAVE_SPAWN_RADIUS_MIN - 0.01 && r <= WAVE_SPAWN_RADIUS_MAX + 0.01);
        }
        assert_eq!(world.active_raids[0].status, RaidStatus::Active);
    }

    #[test]
    fn tick_raids_active_to_cleared_when_mobs_alive_zero() {
        let mut world = make_world_with_one_village([0, 64, 0], (0, 0), 5_000);
        let mut raid = Raid::new_warning(1, (0, 0), [0, 64, 0], WaveKind::Small, 100, 0);
        raid.status = RaidStatus::Active;
        raid.spawn_at_tick = Some(0);
        raid.mobs_alive = 0;
        raid.contribution_table = vec![(0, 5)];
        world.active_raids.push(raid);
        let (_to_spawn, resolutions) = tick_raids(&mut world, 100);
        assert_eq!(resolutions.len(), 1);
        assert_eq!(resolutions[0].status, RaidStatus::Cleared);
        assert_eq!(world.active_raids[0].status, RaidStatus::Cleared);
    }

    #[test]
    fn tick_raids_active_to_failed_when_deadline_expires() {
        let mut world = make_world_with_one_village([0, 64, 0], (0, 0), 5_000);
        let mut raid = Raid::new_warning(1, (0, 0), [0, 64, 0], WaveKind::Small, 100, 0);
        raid.status = RaidStatus::Active;
        raid.spawn_at_tick = Some(0);
        raid.mobs_alive = 3;
        world.active_raids.push(raid);
        let (_to_spawn, resolutions) = tick_raids(&mut world, RAID_DEADLINE_TICKS + 1);
        assert_eq!(resolutions.len(), 1);
        assert_eq!(resolutions[0].status, RaidStatus::Failed);
    }

    #[test]
    fn tick_scheduler_daily_is_idempotent_within_a_day() {
        // Same day, two calls — second is a no-op.
        let mut world = make_world_with_one_village([0, 64, 0], (0, 0), 5_000);
        // No villagers → no eligibility. But the test is whether the
        // scheduler stamps last_rolled_day, so call once + once and
        // check the field updates only on day-2.
        let bg = crate::biome::BiomeGenerator::new(42);
        let mut ecs = hecs::World::new();
        let _ = tick_scheduler_daily(&mut world, &ecs, &bg, 5, 100);
        assert_eq!(world.raid_scheduler.last_rolled_day, 5);
        let _ = tick_scheduler_daily(&mut world, &ecs, &bg, 5, 200);
        // No raids scheduled (no villagers), but the stamp shouldn't
        // double-roll.
        assert_eq!(world.raid_scheduler.last_rolled_day, 5);
        // Advance day → stamp updates.
        let _ = tick_scheduler_daily(&mut world, &mut ecs, &bg, 6, 300);
        assert_eq!(world.raid_scheduler.last_rolled_day, 6);
    }

    #[test]
    fn tick_scheduler_ignores_villages_below_eligibility_threshold() {
        // 5 houses, but only 2 villagers (under the 5-villager gate).
        let mut world = make_world_with_one_village([10000, 64, 10000], (-100, -100), 5_000);
        let bg = crate::biome::BiomeGenerator::new(42);
        let mut ecs = hecs::World::new();
        populate_villagers(&mut ecs, [10000, 64, 10000], 2);
        let scheduled = tick_scheduler_daily(&mut world, &ecs, &bg, 1, 100);
        assert!(scheduled.is_empty(), "should not schedule with <5 villagers");
    }

    #[test]
    fn tick_scheduler_ignores_villages_with_zero_treasury() {
        // Even with enough villagers, zero treasury means no raid.
        let mut world = make_world_with_one_village([10000, 64, 10000], (-100, -100), 0);
        let bg = crate::biome::BiomeGenerator::new(42);
        let mut ecs = hecs::World::new();
        populate_villagers(&mut ecs, [10000, 64, 10000], 10);
        let scheduled = tick_scheduler_daily(&mut world, &ecs, &bg, 1, 100);
        assert!(scheduled.is_empty(), "zero treasury → no raid");
    }

    #[test]
    fn scheduler_picks_same_wave_kind_across_repeated_calls() {
        // Determinism check across days that DO roll. Same seed +
        // same vid + same day + same treasury yields the same wave
        // kind (or None) every time.
        let seed = 42u32;
        let vid = (3, 7);
        for day in 0..50u64 {
            for treasury in [100u64, 1500, 50_000] {
                let a = roll_for_day(seed, vid, day, treasury);
                let b = roll_for_day(seed, vid, day, treasury);
                assert_eq!(a, b);
            }
        }
    }

    #[test]
    fn wave_kind_matches_treasury_tier_on_hit() {
        // For each treasury tier, find a day that rolls a hit + check
        // the kind matches the tier. Same composition expected.
        let seed = 1234u32;
        let vid = (0, 0);
        // Small tier (<500): hit must be Small.
        for day in 0..200u64 {
            if let Some(k) = roll_for_day(seed, vid, day, 100) {
                assert_eq!(k, WaveKind::Small);
                break;
            }
        }
        // Medium tier (500-4999): hit must be Medium.
        for day in 0..200u64 {
            if let Some(k) = roll_for_day(seed, vid, day, 1_000) {
                assert_eq!(k, WaveKind::Medium);
                break;
            }
        }
        // Large tier (>=5000): hit must be Large.
        for day in 0..200u64 {
            if let Some(k) = roll_for_day(seed, vid, day, 10_000) {
                assert_eq!(k, WaveKind::Large);
                break;
            }
        }
    }

    #[test]
    fn raid_bincode_round_trip_preserves_state() {
        // Spec 22 Phase 11 — save/load round-trip on an active raid.
        let raid = Raid {
            id: 42,
            village_id: (5, -3),
            village_anchor: [100, 64, -200],
            wave_kind: WaveKind::Medium,
            treasury_drain: 500,
            contribution_table: vec![(0, 4), (1, 3)],
            killing_blow: Some(1),
            warning_at_tick: 1000,
            spawn_at_tick: Some(7000),
            status: RaidStatus::Active,
            mobs_alive: 3,
            mobs_total: 10,
        };
        let bytes = bincode::serialize(&raid).unwrap();
        let back: Raid = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.id, 42);
        assert_eq!(back.village_id, (5, -3));
        assert_eq!(back.wave_kind, WaveKind::Medium);
        assert_eq!(back.treasury_drain, 500);
        assert_eq!(back.contribution_table, vec![(0, 4), (1, 3)]);
        assert_eq!(back.killing_blow, Some(1));
        assert_eq!(back.status, RaidStatus::Active);
        assert_eq!(back.mobs_alive, 3);
        assert_eq!(back.mobs_total, 10);
    }

    #[test]
    fn scheduler_bincode_round_trip_preserves_counter() {
        let mut s = RaidScheduler::new();
        let _ = s.allocate_id();
        let _ = s.allocate_id();
        s.last_rolled_day = 17;
        let bytes = bincode::serialize(&s).unwrap();
        let back: RaidScheduler = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.next_raid_id, 3);
        assert_eq!(back.last_rolled_day, 17);
    }

    #[test]
    fn tick_raids_does_not_advance_warning_before_spawn_tick() {
        // A Warning raid in the middle of its warning window stays
        // Warning. No spawns produced.
        let mut world = make_world_with_one_village([0, 64, 0], (0, 0), 5_000);
        world.active_raids.push(Raid::new_warning(
            1, (0, 0), [0, 64, 0], WaveKind::Small, 100, 1000,
        ));
        let mid_warning = 1000 + RAID_WARNING_TICKS / 2;
        let (to_spawn, resolutions) = tick_raids(&mut world, mid_warning);
        assert!(to_spawn.is_empty());
        assert!(resolutions.is_empty());
        assert_eq!(world.active_raids[0].status, RaidStatus::Warning);
    }

    #[test]
    fn village_within_defender_radius_picks_nearest() {
        let mut anchors = AHashMap::new();
        anchors.insert((0, 0), [0, 64, 0]);
        anchors.insert((1, 0), [100, 64, 0]);
        // Within range of (0,0).
        let vid = village_within_defender_radius(
            &anchors,
            glam::Vec3::new(5.0, 64.0, 0.0),
        );
        assert_eq!(vid, Some((0, 0)));
        // Outside the 24-block radius of either anchor.
        let vid = village_within_defender_radius(
            &anchors,
            glam::Vec3::new(50.0, 64.0, 0.0),
        );
        assert_eq!(vid, None);
    }

    #[test]
    fn set_warning_returns_smoke_pillar_chunks_for_rebuild() {
        // Spec 22 Phase 7 — the toggle returns the set of chunks whose
        // meshes need rebuilding so the smoke pillar's colour swaps on
        // the next frame. Must cover the campfire's own chunk plus
        // every chunk a SMOKE_PILLAR_HEIGHT column might pass through.
        let anchor = [0, 64, 0];
        let vid: VillageId = (0, 0);
        let mut world = make_world_with_one_village(anchor, vid, 5_000);
        let cf_pos = (0, 64, 0);
        world.set_block(cf_pos.0, cf_pos.1, cf_pos.2, crate::block::CAMPFIRE);
        world.insert_campfire(cf_pos, crate::campfire::CampfireData::default());

        let dirty = set_warning_on_village_campfires(&mut world, vid, true);

        assert!(world.campfire_at(cf_pos).unwrap().raid_warning_active);
        let cf_chunk = crate::world::World::block_to_chunk(cf_pos.0, cf_pos.1, cf_pos.2);
        assert!(
            dirty.contains(&cf_chunk),
            "campfire's own chunk {:?} missing from dirty set {:?}",
            cf_chunk, dirty,
        );
        for dy in 1..=crate::campfire::SMOKE_PILLAR_HEIGHT {
            let ch = crate::world::World::block_to_chunk(cf_pos.0, cf_pos.1 + dy, cf_pos.2);
            assert!(
                dirty.contains(&ch),
                "smoke-column chunk for dy={dy} ({:?}) missing from dirty set {:?}",
                ch, dirty,
            );
        }

        let dirty_clear = set_warning_on_village_campfires(&mut world, vid, false);
        assert!(!world.campfire_at(cf_pos).unwrap().raid_warning_active);
        assert!(
            dirty_clear.contains(&cf_chunk),
            "clear path must also return the campfire's chunk for re-render",
        );
    }

    #[test]
    fn set_warning_on_missing_village_returns_empty_dirty_set() {
        // Defensive: if the village id has no anchor (e.g. removed
        // between scheduling and toggle), the helper returns an empty
        // dirty set rather than panicking.
        let mut world = crate::world::World::new();
        let dirty = set_warning_on_village_campfires(&mut world, (99, 99), true);
        assert!(dirty.is_empty());
    }

    // --- Spec 22 Phase 16: rare-loot tiered drops per wave kind ---

    #[test]
    fn bonus_drops_small_wave_is_empty() {
        assert!(bonus_drops_for(WaveKind::Small, ServerEconomyMode::BitcoinEnabled).is_empty());
        assert!(bonus_drops_for(WaveKind::Small, ServerEconomyMode::BitcoinDisabled).is_empty());
    }

    #[test]
    fn bonus_drops_medium_wave_returns_one_tier_two_item() {
        for mode in [ServerEconomyMode::BitcoinEnabled, ServerEconomyMode::BitcoinDisabled] {
            let drops = bonus_drops_for(WaveKind::Medium, mode);
            assert_eq!(drops.len(), 1, "Medium must drop exactly 1 bonus item under {:?}", mode);
            let stack = &drops[0];
            assert_eq!(stack.count, 1);
            assert!(
                matches!(&stack.item, crate::item::Item::Material(crate::item::MaterialId::IronIngot)),
                "Medium bonus must be an iron ingot, got {:?}",
                stack.item
            );
        }
    }

    #[test]
    fn bonus_drops_large_wave_bitcoin_enabled_drops_diamond() {
        let drops = bonus_drops_for(WaveKind::Large, ServerEconomyMode::BitcoinEnabled);
        assert_eq!(drops.len(), 1);
        let stack = &drops[0];
        assert!(
            matches!(&stack.item, crate::item::Item::Material(crate::item::MaterialId::Diamond)),
            "Large/Bitcoin bonus must be a diamond, got {:?}",
            stack.item
        );
    }

    #[test]
    fn bonus_drops_large_wave_bitcoin_disabled_drops_iron_sword() {
        let drops = bonus_drops_for(WaveKind::Large, ServerEconomyMode::BitcoinDisabled);
        assert_eq!(drops.len(), 1);
        let stack = &drops[0];
        match &stack.item {
            crate::item::Item::Tool(t) => {
                assert_eq!(t.tool_type, crate::crafting::ToolType::Sword);
                assert_eq!(t.material, crate::crafting::ToolMaterial::Iron);
            }
            other => panic!("Large/no-Bitcoin bonus must be an iron sword, got {:?}", other),
        }
    }

    // --- Spec 22 Phase 17: Vendor Block raid-supplies highlight ---

    #[test]
    fn is_raid_supplies_classifies_food_arrows_swords() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        use crate::item::{Item, ItemStack, MaterialId};

        let bread = ItemStack::new_material(MaterialId::Bread, 1);
        assert!(is_raid_supplies(&bread.item));
        let cooked = ItemStack::new_material(MaterialId::CookedBeef, 1);
        assert!(is_raid_supplies(&cooked.item));
        let cake = Item::Material(MaterialId::Cake);
        assert!(is_raid_supplies(&cake));
        let honey = Item::Material(MaterialId::HoneyBottle);
        assert!(is_raid_supplies(&honey));

        let arrow = Item::Material(MaterialId::Arrow);
        assert!(is_raid_supplies(&arrow));

        for mat in [
            ToolMaterial::Wood,
            ToolMaterial::Stone,
            ToolMaterial::Iron,
            ToolMaterial::Diamond,
            ToolMaterial::Satori,
        ] {
            let sword = Item::Tool(Tool::new(ToolType::Sword, mat));
            assert!(is_raid_supplies(&sword), "{:?} sword should be raid supplies", mat);
        }

        let cobble = Item::Block(crate::block::COBBLESTONE);
        assert!(!is_raid_supplies(&cobble));
        let log = Item::Block(crate::block::OAK_LOG);
        assert!(!is_raid_supplies(&log));
        let bonemeal = Item::Material(MaterialId::Bonemeal);
        assert!(!is_raid_supplies(&bonemeal));

        let pickaxe = Item::Tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron));
        assert!(!is_raid_supplies(&pickaxe));
    }

    #[test]
    fn should_highlight_fires_when_vendor_near_warned_village_with_supplies() {
        use crate::item::{Item, MaterialId};

        let mut world = crate::world::World::new();
        let vid: VillageId = (0, 0);
        world.village_anchors.insert(vid, [0, 64, 0]);
        world.active_raids.push(Raid::new_warning(
            1, vid, [0, 64, 0], WaveKind::Small, 100, 0,
        ));

        let vendor_pos = glam::Vec3::new(10.0, 64.0, 0.0);
        let arrow = Item::Material(MaterialId::Arrow);
        assert!(should_highlight_as_raid_supplies(&world, vendor_pos, &arrow));

        let cobble = Item::Block(crate::block::COBBLESTONE);
        assert!(!should_highlight_as_raid_supplies(&world, vendor_pos, &cobble));

        let far_pos = glam::Vec3::new(100.0, 64.0, 0.0);
        assert!(!should_highlight_as_raid_supplies(&world, far_pos, &arrow));
    }

    #[test]
    fn should_highlight_does_not_fire_when_raid_is_active_not_warning() {
        use crate::item::{Item, MaterialId};
        let mut world = crate::world::World::new();
        let vid: VillageId = (0, 0);
        world.village_anchors.insert(vid, [0, 64, 0]);
        let mut raid = Raid::new_warning(1, vid, [0, 64, 0], WaveKind::Small, 100, 0);
        raid.status = RaidStatus::Active;
        world.active_raids.push(raid);
        let vendor_pos = glam::Vec3::new(10.0, 64.0, 0.0);
        let arrow = Item::Material(MaterialId::Arrow);
        assert!(!should_highlight_as_raid_supplies(&world, vendor_pos, &arrow));
    }

    #[test]
    fn nearest_village_within_returns_none_when_outside_radius() {
        let mut anchors = AHashMap::new();
        anchors.insert((0, 0), [0, 64, 0]);
        assert_eq!(
            nearest_village_within(&anchors, glam::Vec3::new(5.0, 64.0, 0.0), 32.0),
            Some((0, 0)),
        );
        assert_eq!(
            nearest_village_within(&anchors, glam::Vec3::new(100.0, 64.0, 0.0), 32.0),
            None,
        );
    }

    // --- Spec 22 Phase 18: raid leaderboard at the village ---

    #[test]
    fn tally_raid_kills_aggregates_per_player_per_village() {
        let mut board: AHashMap<(VillageId, PlayerKey), u32> = AHashMap::new();
        let vid: VillageId = (0, 0);

        tally_raid_kills_into_leaderboard(&mut board, vid, &[(0, 5), (1, 2)]);
        assert_eq!(board.get(&(vid, 0)).copied(), Some(5));
        assert_eq!(board.get(&(vid, 1)).copied(), Some(2));

        tally_raid_kills_into_leaderboard(&mut board, vid, &[(0, 3)]);
        assert_eq!(board.get(&(vid, 0)).copied(), Some(8), "p0 must accumulate across raids");
        assert_eq!(board.get(&(vid, 1)).copied(), Some(2));

        let vid2: VillageId = (1, 1);
        tally_raid_kills_into_leaderboard(&mut board, vid2, &[(0, 4)]);
        assert_eq!(board.get(&(vid2, 0)).copied(), Some(4));
        assert_eq!(board.get(&(vid, 0)).copied(), Some(8), "v0 unaffected by v1 tally");
    }

    #[test]
    fn tally_raid_kills_with_empty_contribution_is_noop() {
        let mut board: AHashMap<(VillageId, PlayerKey), u32> = AHashMap::new();
        tally_raid_kills_into_leaderboard(&mut board, (0, 0), &[]);
        assert!(board.is_empty());
    }

    #[test]
    fn tally_raid_kills_skips_zero_kill_entries() {
        let mut board: AHashMap<(VillageId, PlayerKey), u32> = AHashMap::new();
        let vid: VillageId = (0, 0);
        tally_raid_kills_into_leaderboard(&mut board, vid, &[(0, 5), (1, 0)]);
        assert_eq!(board.get(&(vid, 0)).copied(), Some(5));
        assert_eq!(board.get(&(vid, 1)), None, "0-kill entry must not be inserted");
    }

    #[test]
    fn top_defenders_returns_sorted_descending_top_k() {
        let mut board: AHashMap<(VillageId, PlayerKey), u32> = AHashMap::new();
        let vid: VillageId = (0, 0);
        board.insert((vid, 0), 5);
        board.insert((vid, 1), 12);
        board.insert((vid, 2), 8);
        board.insert((vid, 3), 1);
        let top = top_defenders(&board, vid, 3);
        assert_eq!(top, vec![(1, 12), (2, 8), (0, 5)]);
        let top1 = top_defenders(&board, vid, 1);
        assert_eq!(top1, vec![(1, 12)]);
        let top10 = top_defenders(&board, vid, 10);
        assert_eq!(top10.len(), 4);
    }

    #[test]
    fn top_defenders_breaks_ties_by_lower_player_key() {
        let mut board: AHashMap<(VillageId, PlayerKey), u32> = AHashMap::new();
        let vid: VillageId = (0, 0);
        board.insert((vid, 0), 5);
        board.insert((vid, 1), 5);
        let top = top_defenders(&board, vid, 2);
        assert_eq!(top, vec![(0, 5), (1, 5)]);
    }

    #[test]
    fn top_defenders_filters_by_village_id() {
        let mut board: AHashMap<(VillageId, PlayerKey), u32> = AHashMap::new();
        board.insert(((0, 0), 0), 5);
        board.insert(((1, 1), 0), 100);
        let top = top_defenders(&board, (0, 0), 3);
        assert_eq!(top, vec![(0, 5)]);
    }

    #[test]
    fn raid_kills_world_clear_wipes_leaderboard() {
        // Regression guard: World::clear must wipe raid_kills.
        let mut world = crate::world::World::new();
        world.raid_kills.insert(((0, 0), 0), 5);
        world.raid_kills.insert(((1, 1), 2), 10);
        assert_eq!(world.raid_kills.len(), 2);
        world.clear();
        assert!(world.raid_kills.is_empty(), "World::clear must wipe leaderboard");
    }
}
