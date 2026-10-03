//! Mob Bounty Board — first combat-economy primitive (Spec 33).
//!
//! Server-issued daily bounties: "Kill 10 Brigands for 100 sats". A
//! placeable `BOUNTY_BOARD` block surfaces the day's bounties via a
//! right-click dialog (`bounty_ui.rs`). Each bounty has a target
//! mob kind + required kill count + payout. Claiming consumes the
//! kill_counter and fires `economy::apply_sats_payout(_,
//! PayoutKind::BountyClaim, _, _)`, with a reputation fallback when
//! sats are suppressed (Charter-off / Bitcoin-disabled).
//!
//! Bounties refresh every `BOUNTY_REFRESH_TICKS` (~1 in-game day),
//! deterministic per `(world_seed, day_index)`. Each ActiveBounty
//! has a monotonic id allocated from `World.bounty_next_id` so claims
//! are scoped to a specific rotation.
//!
//! Spec: `docs/foundations/2026-05-23-mob-bounty-board.md`.

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::mob::MobType;
use crate::world::World;

type HmacSha256 = Hmac<Sha256>;

/// One in-game day in ticks. Same cadence as the rubber tap cooldown
/// and the brigand hideout replenisher.
pub const BOUNTY_REFRESH_TICKS: u64 = 24_000;

/// Minimum number of active bounties per rotation. Always ≥ this many.
pub const MIN_ACTIVE_BOUNTIES: usize = 2;

/// Maximum number of active bounties per rotation. Caps the dialog
/// size + prevents the daily roll from cluttering.
pub const MAX_ACTIVE_BOUNTIES: usize = 3;

/// One bounty template — the design intent. Live `ActiveBounty`s are
/// instantiated from these.
#[derive(Clone, Copy, Debug)]
pub struct BountyTemplate {
    pub mob_kind: MobType,
    pub required_count: u32,
    pub payout_sats: u64,
    /// Reputation awarded to nearby villages when sats are suppressed
    /// (Charter-off / Bitcoin-disabled server). The bounty is still
    /// claimable — just pays in social currency.
    pub fallback_rep: i32,
    pub label: &'static str,
}

/// v1 template pool. Three fixed entries — small, legible, deliberately
/// covers the most-spawning mobs (Brigand / Marauder) so the
/// kid can complete bounties without bespoke hunting trips.
pub const BOUNTY_TEMPLATES: &[BountyTemplate] = &[
    BountyTemplate {
        mob_kind: MobType::Brigand,
        required_count: 10,
        payout_sats: 100,
        fallback_rep: 50,
        label: "Kill 10 Brigands",
    },
    BountyTemplate {
        mob_kind: MobType::Marauder,
        required_count: 5,
        payout_sats: 75,
        fallback_rep: 35,
        label: "Kill 5 Marauders",
    },
    BountyTemplate {
        mob_kind: MobType::Brigand,
        required_count: 3,
        payout_sats: 150,
        fallback_rep: 75,
        label: "Kill 3 Brigands",
    },
];

/// One bounty active in the current rotation. Many players may claim
/// it independently — per-player claim tracking lives on
/// `PlayerSlot.bounties_claimed`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ActiveBounty {
    pub id: u32,
    /// Index into `BOUNTY_TEMPLATES`. Stable across the rotation.
    pub template_idx: usize,
    /// Monotonic tick at issue time — for audit / staleness checks.
    pub issued_tick: u64,
}

impl ActiveBounty {
    /// Convenience — look up the template this bounty was rolled from.
    pub fn template(&self) -> Option<&'static BountyTemplate> {
        BOUNTY_TEMPLATES.get(self.template_idx)
    }
}

/// True iff the player has accumulated enough kills for the template's
/// requirement.
pub fn can_claim(template: &BountyTemplate, player_kills: u32) -> bool {
    player_kills >= template.required_count
}

/// Pseudo-random integer in `[0, n)` from a 3-input mix. Deterministic.
/// Used to pick which templates make the day's rotation.
fn roll(world_seed: u32, day_index: u64, salt: u64, n: u64) -> u64 {
    let mut h = (world_seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= day_index.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= salt.wrapping_mul(0x94D0_49BB_1331_11EB);
    h = (h ^ (h >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    h = (h ^ (h >> 31)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    (h ^ (h >> 30)) % n
}

/// Roll the bounty rotation for a given `(world_seed, day_index)`.
/// Picks 2-3 templates from `BOUNTY_TEMPLATES` deterministically.
/// Assigns each a fresh id from `next_id` (which is incremented).
pub fn roll_daily_bounties(
    world_seed: u32,
    day_index: u64,
    next_id: &mut u32,
) -> Vec<ActiveBounty> {
    // How many bounties this rotation? 2 or 3, biased toward 3.
    // (roll mod 4 → 0 picks 2, others pick 3 → 25% small, 75% big.)
    let count = if roll(world_seed, day_index, 0xA001, 4) == 0 {
        MIN_ACTIVE_BOUNTIES
    } else {
        MAX_ACTIVE_BOUNTIES
    };
    let template_count = BOUNTY_TEMPLATES.len() as u64;
    let count = count.min(BOUNTY_TEMPLATES.len());

    // Sample `count` distinct template indices.
    let mut picked: Vec<usize> = Vec::with_capacity(count);
    let mut attempts = 0u64;
    while picked.len() < count && attempts < 32 {
        let candidate = roll(world_seed, day_index, 0xB002 + attempts, template_count) as usize;
        if !picked.contains(&candidate) {
            picked.push(candidate);
        }
        attempts += 1;
    }
    // Defensive: if rejection sampling stalled, fall through to
    // sequential fill so we always meet the minimum.
    let mut idx = 0;
    while picked.len() < count && idx < BOUNTY_TEMPLATES.len() {
        if !picked.contains(&idx) {
            picked.push(idx);
        }
        idx += 1;
    }

    let issued_tick = day_index.saturating_mul(BOUNTY_REFRESH_TICKS);
    picked
        .into_iter()
        .map(|template_idx| {
            let id = *next_id;
            *next_id = next_id.wrapping_add(1);
            // Skip id 0 — used as "uninitialised" sentinel in legacy saves.
            if *next_id == 0 {
                *next_id = 1;
            }
            ActiveBounty {
                id,
                template_idx,
                issued_tick,
            }
        })
        .collect()
}

/// Tick driver — call from `game_loop::tick` + `server::tick` once per
/// tick. Self-throttled: rolls a new rotation only when the day_index
/// changes (or when bounties is empty, i.e. fresh world). Returns true
/// iff a fresh rotation was rolled this tick (for callers that want to
/// emit a toast or log).
///
/// `monotonic_tick` MUST be a monotonic counter (`tick_counter`), NOT
/// `world_time` (cyclic 0-23999) — `BOUNTY_REFRESH_TICKS = 24_000` =
/// the day length, so the same cyclic-clock bug pattern as rubber /
/// brigand / snowfall applies (see merge `1f4c3c0`).
pub fn tick_bounty_refresh(
    world: &mut World,
    monotonic_tick: u64,
    world_seed: u32,
) -> bool {
    let new_day = monotonic_tick / BOUNTY_REFRESH_TICKS;
    let prev_day = world.bounty_last_refresh_tick / BOUNTY_REFRESH_TICKS;
    // Skip when same day AND bounties already populated. The
    // "bounties empty" branch handles fresh worlds + legacy saves
    // whose bounty_last_refresh_tick is 0 but bounty_next_id is 0
    // → we want to seed the rotation immediately.
    if new_day == prev_day && !world.bounties.is_empty() {
        return false;
    }
    // Allocate a fresh next_id on first run.
    if world.bounty_next_id == 0 {
        world.bounty_next_id = 1;
    }
    world.bounties = roll_daily_bounties(world_seed, new_day, &mut world.bounty_next_id);
    world.bounty_last_refresh_tick = monotonic_tick;
    true
}

/// Outcome of a claim attempt. Mirrors `combat::ExplosionOutcome` —
/// the UI hands this off to the game-loop side which applies the
/// payout / toast / log effects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// Claim accepted. `payout_sats` is what the player should be
    /// credited (pre-Charter, pre-policy — caller still pipes through
    /// `economy::apply_sats_payout` to honour suppression). `kills_drained`
    /// is what was subtracted from `kill_counter`.
    Accepted { payout_sats: u64, fallback_rep: i32, kills_drained: u32, mob_kind: MobType },
    /// Bounty id not in the active rotation — likely a stale dialog
    /// after a daily refresh. Never actually produced by the claim function
    /// (an unrecognised id currently classifies as `InvalidTemplate`
    /// instead) — matched defensively in game_loop.rs's label lookup.
    #[allow(dead_code)]
    StaleBounty,
    /// Player hasn't accumulated enough kills yet. UI gates this but
    /// the helper is defensive.
    InsufficientKills,
    /// Player already claimed this bounty id in the current rotation.
    AlreadyClaimed,
    /// Template index out of range — should never happen; defensive
    /// against a corrupt save.
    InvalidTemplate,
}

/// Attempt to claim a bounty by id. Pure-state mutation on
/// `player_kill_counter` + `player_claimed`; returns the outcome for
/// the caller to apply side-effects (sats payout, rep payout, toast,
/// audit log).
///
/// Caller responsibilities:
/// - Look up `bounty` from `world.bounties` (by id, before calling).
/// - If `Accepted`: pipe `payout_sats` through `economy::apply_sats_payout`
///   with the right kind (`PayoutKind::BountyClaim`) + emit the audit
///   hash + apply the rep fallback when sats are suppressed.
pub fn try_claim(
    bounty: &ActiveBounty,
    player_kill_counter: &mut ahash::AHashMap<MobType, u32>,
    player_claimed: &mut ahash::AHashMap<u32, u32>,
) -> ClaimOutcome {
    let Some(template) = bounty.template() else {
        return ClaimOutcome::InvalidTemplate;
    };
    if player_claimed.contains_key(&bounty.id) {
        return ClaimOutcome::AlreadyClaimed;
    }
    let kills = *player_kill_counter.get(&template.mob_kind).unwrap_or(&0);
    if !can_claim(template, kills) {
        return ClaimOutcome::InsufficientKills;
    }
    // Drain + record.
    let new_kills = kills.saturating_sub(template.required_count);
    player_kill_counter.insert(template.mob_kind, new_kills);
    player_claimed.insert(bounty.id, template.required_count);
    ClaimOutcome::Accepted {
        payout_sats: template.payout_sats,
        fallback_rep: template.fallback_rep,
        kills_drained: template.required_count,
        mob_kind: template.mob_kind,
    }
}

/// HMAC-SHA256 audit hash over a bounty claim. Per the economies
/// vision §6.1: "each kill is recorded with a hash of (player_pubkey,
/// mob_id, world_position, server_secret)". The Mob Bounty Board's
/// equivalent is the claim hash — captures the kill_count and
/// mob_kind of a successful claim so the server-side replay check
/// can verify it post-hoc.
///
/// Returns the 32-byte HMAC output. Caller logs / persists as
/// appropriate. v1 = informational only; server-side verification
/// waits on Spec 1 Phase 4 (engine-side Signet signing bridge).
pub fn claim_audit_hash(
    player_pubkey: &[u8; 32],
    mob_kind: MobType,
    kill_count: u32,
    server_secret: &[u8],
) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(server_secret)
        .expect("HMAC accepts any key length");
    mac.update(b"bounty-claim-v1");
    mac.update(player_pubkey);
    mac.update(&(mob_kind as u32).to_le_bytes());
    mac.update(&kill_count.to_le_bytes());
    let out = mac.finalize().into_bytes();
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&out);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_claim_returns_true_at_exact_count() {
        let t = &BOUNTY_TEMPLATES[0]; // Brigand x10
        assert!(can_claim(t, 10));
        assert!(can_claim(t, 100));
    }

    #[test]
    fn can_claim_returns_false_below_count() {
        let t = &BOUNTY_TEMPLATES[0]; // Brigand x10
        assert!(!can_claim(t, 9));
        assert!(!can_claim(t, 0));
    }

    #[test]
    fn roll_daily_bounties_is_deterministic_per_seed_and_day() {
        // Same (seed, day) input → same template_idx output. The id
        // counter is the only side that varies, and only because
        // `next_id` is mutated by the call.
        let mut next_a = 1u32;
        let mut next_b = 1u32;
        let a = roll_daily_bounties(42, 7, &mut next_a);
        let b = roll_daily_bounties(42, 7, &mut next_b);
        assert_eq!(a.len(), b.len());
        for (ab, bb) in a.iter().zip(b.iter()) {
            assert_eq!(ab.template_idx, bb.template_idx);
            assert_eq!(ab.issued_tick, bb.issued_tick);
        }
    }

    #[test]
    fn roll_daily_bounties_returns_2_or_3_bounties() {
        let mut next = 1u32;
        for seed in 1..200u32 {
            for day in 0..5u64 {
                let bs = roll_daily_bounties(seed, day, &mut next);
                assert!(bs.len() >= MIN_ACTIVE_BOUNTIES,
                    "seed={seed}, day={day}: expected ≥{MIN_ACTIVE_BOUNTIES}, got {}",
                    bs.len());
                assert!(bs.len() <= MAX_ACTIVE_BOUNTIES,
                    "seed={seed}, day={day}: expected ≤{MAX_ACTIVE_BOUNTIES}, got {}",
                    bs.len());
            }
        }
    }

    #[test]
    fn roll_daily_bounties_picks_distinct_templates() {
        // Within one rotation, no template index appears twice.
        let mut next = 1u32;
        for seed in 1..50u32 {
            for day in 0..10u64 {
                let bs = roll_daily_bounties(seed, day, &mut next);
                let mut seen: Vec<usize> = bs.iter().map(|b| b.template_idx).collect();
                seen.sort();
                let n_before = seen.len();
                seen.dedup();
                assert_eq!(seen.len(), n_before,
                    "seed={seed}, day={day}: duplicate template index in rotation");
            }
        }
    }

    #[test]
    fn roll_daily_bounties_advances_id_counter() {
        let mut next = 1u32;
        let a = roll_daily_bounties(42, 1, &mut next);
        let b = roll_daily_bounties(42, 2, &mut next);
        // Every id in `a` is < every id in `b`.
        let max_a = a.iter().map(|b| b.id).max().unwrap();
        let min_b = b.iter().map(|b| b.id).min().unwrap();
        assert!(max_a < min_b,
            "id counter must advance across rotations: max_a={max_a}, min_b={min_b}");
    }

    #[test]
    fn claim_audit_hash_is_deterministic() {
        let pubkey = [0x11u8; 32];
        let secret = b"test-secret";
        let h1 = claim_audit_hash(&pubkey, MobType::Brigand, 10, secret);
        let h2 = claim_audit_hash(&pubkey, MobType::Brigand, 10, secret);
        assert_eq!(h1, h2);
    }

    #[test]
    fn claim_audit_hash_changes_with_player() {
        let pubkey_a = [0x11u8; 32];
        let pubkey_b = [0x22u8; 32];
        let secret = b"test-secret";
        let h1 = claim_audit_hash(&pubkey_a, MobType::Brigand, 10, secret);
        let h2 = claim_audit_hash(&pubkey_b, MobType::Brigand, 10, secret);
        assert_ne!(h1, h2);
    }

    #[test]
    fn claim_audit_hash_changes_with_mob_kind() {
        let pubkey = [0x11u8; 32];
        let secret = b"test-secret";
        let h1 = claim_audit_hash(&pubkey, MobType::Brigand, 10, secret);
        let h2 = claim_audit_hash(&pubkey, MobType::Marauder, 10, secret);
        assert_ne!(h1, h2);
    }

    #[test]
    fn tick_bounty_refresh_no_op_within_period() {
        let mut world = World::new();
        let _ = tick_bounty_refresh(&mut world, 100, 42);
        let bounties_after_first = world.bounties.clone();
        let last_after_first = world.bounty_last_refresh_tick;
        // Another tick within the same day window.
        let did_refresh = tick_bounty_refresh(&mut world, 200, 42);
        assert!(!did_refresh, "should not refresh within the same day");
        assert_eq!(world.bounties.len(), bounties_after_first.len());
        assert_eq!(world.bounty_last_refresh_tick, last_after_first);
    }

    #[test]
    fn try_claim_drains_counter_and_records_on_accept() {
        let bounty = ActiveBounty { id: 42, template_idx: 0, issued_tick: 0 };
        let mut kills: ahash::AHashMap<MobType, u32> = ahash::AHashMap::new();
        kills.insert(MobType::Brigand, 15);
        let mut claimed: ahash::AHashMap<u32, u32> = ahash::AHashMap::new();
        let out = try_claim(&bounty, &mut kills, &mut claimed);
        match out {
            ClaimOutcome::Accepted { payout_sats, kills_drained, mob_kind, .. } => {
                assert_eq!(payout_sats, 100);
                assert_eq!(kills_drained, 10);
                assert_eq!(mob_kind, MobType::Brigand);
            }
            other => panic!("expected Accepted, got {other:?}"),
        }
        assert_eq!(*kills.get(&MobType::Brigand).unwrap(), 5,
            "kill_counter should be drained by required_count");
        assert_eq!(*claimed.get(&42).unwrap(), 10,
            "claim should be recorded under the bounty id");
    }

    #[test]
    fn try_claim_blocks_double_claim() {
        let bounty = ActiveBounty { id: 42, template_idx: 0, issued_tick: 0 };
        let mut kills: ahash::AHashMap<MobType, u32> = ahash::AHashMap::new();
        kills.insert(MobType::Brigand, 30);
        let mut claimed: ahash::AHashMap<u32, u32> = ahash::AHashMap::new();
        let _ = try_claim(&bounty, &mut kills, &mut claimed);
        let second = try_claim(&bounty, &mut kills, &mut claimed);
        assert_eq!(second, ClaimOutcome::AlreadyClaimed);
        // kill_counter unchanged after the rejected second claim.
        assert_eq!(*kills.get(&MobType::Brigand).unwrap(), 20);
    }

    #[test]
    fn try_claim_rejects_insufficient_kills() {
        let bounty = ActiveBounty { id: 42, template_idx: 0, issued_tick: 0 };
        let mut kills: ahash::AHashMap<MobType, u32> = ahash::AHashMap::new();
        kills.insert(MobType::Brigand, 5);  // below required 10
        let mut claimed: ahash::AHashMap<u32, u32> = ahash::AHashMap::new();
        let out = try_claim(&bounty, &mut kills, &mut claimed);
        assert_eq!(out, ClaimOutcome::InsufficientKills);
        assert_eq!(*kills.get(&MobType::Brigand).unwrap(), 5, "kill_counter unchanged");
        assert!(claimed.is_empty(), "no claim recorded");
    }

    #[test]
    fn try_claim_rejects_invalid_template_idx() {
        let bounty = ActiveBounty { id: 42, template_idx: 999, issued_tick: 0 };
        let mut kills: ahash::AHashMap<MobType, u32> = ahash::AHashMap::new();
        let mut claimed: ahash::AHashMap<u32, u32> = ahash::AHashMap::new();
        let out = try_claim(&bounty, &mut kills, &mut claimed);
        assert_eq!(out, ClaimOutcome::InvalidTemplate);
    }

    #[test]
    fn tick_bounty_refresh_replaces_on_period_boundary() {
        let mut world = World::new();
        let _ = tick_bounty_refresh(&mut world, 100, 42);
        let first_ids: Vec<u32> = world.bounties.iter().map(|b| b.id).collect();
        // Advance past one full day cycle.
        let did_refresh = tick_bounty_refresh(&mut world, BOUNTY_REFRESH_TICKS + 50, 42);
        assert!(did_refresh, "should refresh after one day");
        let second_ids: Vec<u32> = world.bounties.iter().map(|b| b.id).collect();
        // No overlap — old bounty ids are gone, new ones replace them.
        for id in &first_ids {
            assert!(!second_ids.contains(id),
                "stale bounty id {id} survived the refresh");
        }
    }
}
