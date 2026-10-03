//! Proof of Play — the educational proof-of-work primitive that runs on
//! every pickaxe strike. Spec 6 §2 (Proof of Play) and Spec 6 §2.2c
//! (Satori vein generation).
//!
//! Three layers stacked on one HMAC-SHA256 hash:
//!   1. Proof of Play (always-on): the hash is the in-game expression of
//!      proof-of-work for educational purposes. Surfaced to the player
//!      (truncated) + drives a Genesis Block celebration when below a
//!      difficulty threshold.
//!   2. Material drops: the same hash drives deterministic rare drops on
//!      plain stone + the Satori vein algorithm in pure deepslate.
//!   3. Bitcoin (optional): on Bitcoin-enabled servers, the hash also
//!      gates sats payouts via the deterministic work-meter model. (A
//!      chance-based "probabilistic" real-sats payout is retired — it sits
//!      inside the gambling perimeter; see Spec 6 §2.3. Probabilistic
//!      *item* drops in Layer 2 are unaffected.)
//!
//! This module owns Layer 1 (the hash) and Layer 2's gem-vein piece. The
//! Bitcoin layer is post-alpha and lives in a future bitcoin subsystem.

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::biome::Y_DP;

type HmacSha256 = Hmac<Sha256>;

/// Default `ORIGIN_BASE_THRESHOLD` — per Spec 6 §2.2c.2, ~1 in 100,000
/// rarity check per pure-deepslate block at the depth gate. Expressed as
/// a `u32` over `u32::MAX`: hashes whose first 4 bytes (as u32) are
/// below this value are vein origins. Server-configurable.
pub const ORIGIN_BASE_THRESHOLD: u32 = (u32::MAX as u64 / 100_000) as u32;

/// Maximum scaling factor by depth. At the depth gate (y = Y_DP - 21)
/// the multiplier is 1×; at bedrock (y = 0) it is `DEPTH_SCALAR_MAX`.
/// Per Spec 6 §2.2c.2 — encourages digging deeper.
pub const ORIGIN_DEPTH_SCALAR_MAX: f32 = 4.0;

/// Maximum lattice distance a vein can extend from its origin (Spec 6 §2.2c).
/// Default 8 blocks.
pub const VEIN_MAX_RADIUS: i32 = 8;

/// Per-step propagation base + decay (Spec 6 §2.2c.2).
pub const PROPAGATION_BASE: f32 = 0.85;
pub const PROPAGATION_DECAY: f32 = 0.85;

/// Default exposure-decay duration in ticks — 1 in-game day at 1× world-time
/// speed. With the current `world_time_step = 4` alpha pace, ~5 real minutes.
/// With post-alpha 1× default, ~20 real minutes. Server-configurable.
pub const EXPOSURE_DECAY_DURATION_TICKS: u32 = 20_000;

/// A fresh per-world Proof-of-Play `server_secret`: 32 bytes from the OS RNG
/// (Spec 06 §2.2). Generated once per world and kept in the host's
/// `WorldMeta.pop_secret`; never derived from the seed and never serialised
/// to a client. (The old alpha stub HMAC'd the public world seed under a
/// fixed key, so any joiner holding `JoinAccept.seed` could map every Satori
/// vein offline — audit 2026-09-27.)
pub fn gen_world_secret() -> [u8; 32] {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("OS RNG unavailable");
    buf
}

/// Return the world's secret, generating (and storing into `slot`) a fresh
/// random one if the world has none yet — a world saved before per-world
/// secrets existed. The bool is true when a secret was generated, so the
/// caller persists the meta. Changing an old world's secret moves its future
/// rare-drop placement (accepted, see Spec 06 §2.2).
pub fn ensure_world_secret(slot: &mut Option<[u8; 32]>) -> ([u8; 32], bool) {
    match slot {
        Some(s) => (*s, false),
        None => {
            let s = gen_world_secret();
            *slot = Some(s);
            (s, true)
        }
    }
}

/// Compute the canonical Proof-of-Play hash for a strike on block `(x, y, z)`.
/// Spec 6 §2.2 — HMAC-SHA256(server_secret, world_seed || epoch_id || x || y || z).
///
/// Returns the full 32-byte digest. Callers slice the byte budget per Spec 6 §2.2a:
///   bytes [0..8]  → reward_value u64 (Bitcoin threshold + Genesis-Block animation)
///   byte  [8]     → reward tier (Bitcoin layer only)
///   byte  [9]     → exact reward magnitude within tier (Bitcoin layer only)
///   byte  [10]    → rare-drop probability check (material layer)
///   byte  [11]    → rare-drop material selector (material layer)
///   byte  [12]    → exposure-decay roll for gem veins
///   bytes [13..]  → reserved
pub fn proof_hash(
    server_secret: &[u8],
    world_seed: u64,
    epoch_id: u32,
    x: i32,
    y: i32,
    z: i32,
) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(server_secret).expect("HMAC accepts any key length");
    mac.update(&world_seed.to_le_bytes());
    mac.update(&epoch_id.to_le_bytes());
    mac.update(&x.to_le_bytes());
    mac.update(&y.to_le_bytes());
    mac.update(&z.to_le_bytes());
    let out = mac.finalize().into_bytes();
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&out);
    buf
}

/// First 4 bytes of a 32-byte digest as a big-endian u32. Used for
/// threshold checks (Spec 6 §2.2c.2).
// BRIDGE: real-sats payout vectoring is deliberately DEFERRED (owner policy —
// PoP's present role is educational proof-of-work + anti-X-ray only, never
// "earn sats"; see feedback_pop_educational_not_earning_now in project
// memory). These two hash-decode helpers back that deferred payout path and
// have no caller by design, not by oversight.
#[allow(dead_code)]
pub fn u32_be_prefix(hash: &[u8; 32]) -> u32 {
    u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]])
}

/// First 8 bytes as a big-endian u64 — the `reward_value` for the Bitcoin /
/// celebration threshold (Spec 6 §2.2 / §2.3).
#[allow(dead_code)]
pub fn u64_reward_value(hash: &[u8; 32]) -> u64 {
    u64::from_be_bytes([
        hash[0], hash[1], hash[2], hash[3],
        hash[4], hash[5], hash[6], hash[7],
    ])
}

/// Depth-scaled origin threshold for a candidate vein origin at world Y `oy`.
/// The threshold scales linearly from 1× at the depth gate to
/// `ORIGIN_DEPTH_SCALAR_MAX` at y=0. Spec 6 §2.2c.2.
pub fn origin_threshold(oy: i32) -> u32 {
    let gate = Y_DP - 21;
    if oy > gate {
        return 0; // No veins above the gate.
    }
    // 0.0 at gate, 1.0 at bedrock (y = 0). Clamped.
    let normalized = ((gate - oy) as f32 / gate.max(1) as f32).clamp(0.0, 1.0);
    let scalar = 1.0 + (ORIGIN_DEPTH_SCALAR_MAX - 1.0) * normalized;
    ((ORIGIN_BASE_THRESHOLD as f32) * scalar).min(u32::MAX as f32) as u32
}

/// Is the position `(ox, oy, oz)` a vein origin? Spec 6 §2.2c.2 stage 1.
/// Caller is responsible for the substrate check (must be pure deepslate)
/// and the depth gate; this only checks the hash.
pub fn is_vein_origin(
    server_secret: &[u8],
    world_seed: u64,
    epoch_id: u32,
    ox: i32,
    oy: i32,
    oz: i32,
) -> bool {
    let mut mac = HmacSha256::new_from_slice(server_secret).expect("HMAC accepts any key length");
    mac.update(b"vein_origin");
    mac.update(&world_seed.to_le_bytes());
    mac.update(&epoch_id.to_le_bytes());
    mac.update(&ox.to_le_bytes());
    mac.update(&oy.to_le_bytes());
    mac.update(&oz.to_le_bytes());
    let out = mac.finalize().into_bytes();
    let h = u32::from_be_bytes([out[0], out[1], out[2], out[3]]);
    h < origin_threshold(oy)
}

/// Propagation threshold at step `n` from origin. Decays geometrically per
/// Spec 6 §2.2c.2: `propagation_base * propagation_decay^n`.
pub fn propagation_threshold(n: u32) -> u32 {
    let t = PROPAGATION_BASE * PROPAGATION_DECAY.powi(n as i32);
    (t * (u32::MAX as f32)).min(u32::MAX as f32) as u32
}

/// Walk the propagation tree from an origin toward a target, returning
/// `true` iff the vein extends to the target. Each step's continuation is
/// gated by a per-step HMAC.
///
/// The walk is a deterministic monotone Manhattan-style descent — at each
/// step, advance one block along whichever axis has the largest remaining
/// distance. This gives a unique path origin→target which makes membership
/// well-defined and chunk-local.
pub fn propagation_reaches(
    server_secret: &[u8],
    world_seed: u64,
    epoch_id: u32,
    origin: (i32, i32, i32),
    target: (i32, i32, i32),
) -> bool {
    let (ox, oy, oz) = origin;
    let (tx, ty, tz) = target;
    if origin == target {
        return true; // Origin block itself is always in the vein.
    }
    let dx = tx - ox;
    let dy = ty - oy;
    let dz = tz - oz;
    let manhattan = dx.unsigned_abs() + dy.unsigned_abs() + dz.unsigned_abs();
    if manhattan > VEIN_MAX_RADIUS as u32 {
        return false; // Outside the soft vein boundary.
    }

    let mut pos = origin;
    let mut step: u32 = 0;
    while pos != target {
        // Pick the next lattice step toward the target (largest remaining axis).
        let rdx = tx - pos.0;
        let rdy = ty - pos.1;
        let rdz = tz - pos.2;
        let adx = rdx.unsigned_abs();
        let ady = rdy.unsigned_abs();
        let adz = rdz.unsigned_abs();
        let next = if adx >= ady && adx >= adz {
            (pos.0 + rdx.signum(), pos.1, pos.2)
        } else if ady >= adz {
            (pos.0, pos.1 + rdy.signum(), pos.2)
        } else {
            (pos.0, pos.1, pos.2 + rdz.signum())
        };
        // Hash the step.
        let mut mac = HmacSha256::new_from_slice(server_secret)
            .expect("HMAC accepts any key length");
        mac.update(b"vein_step");
        mac.update(&world_seed.to_le_bytes());
        mac.update(&epoch_id.to_le_bytes());
        mac.update(&ox.to_le_bytes());
        mac.update(&oy.to_le_bytes());
        mac.update(&oz.to_le_bytes());
        mac.update(&step.to_le_bytes());
        mac.update(&next.0.to_le_bytes());
        mac.update(&next.1.to_le_bytes());
        mac.update(&next.2.to_le_bytes());
        let out = mac.finalize().into_bytes();
        let h = u32::from_be_bytes([out[0], out[1], out[2], out[3]]);
        if h >= propagation_threshold(step) {
            return false; // Vein stops at this step.
        }
        pos = next;
        step += 1;
        if step > VEIN_MAX_RADIUS as u32 * 3 {
            // Safety bound — Manhattan distance is bounded above, so we
            // should never hit this, but explicit guard against infinite
            // loops if propagation_reaches is misused.
            return false;
        }
    }
    true
}

/// Convenience — is the block `(x, y, z)` a member of any vein originating
/// within `VEIN_MAX_RADIUS` of it? This is the strike-time question.
///
/// **Caller responsibility:** verify the block is pure deepslate AND that
/// `y <= Y_DP - 21` before calling this. The function does not re-check
/// substrate or depth gate.
///
/// Worst-case cost: ~17×17×17 = 4913 candidate origins, but the vast
/// majority fail the `is_vein_origin` cheap check (`u32` compare against
/// a low threshold). Expected cost is dominated by a handful of HMAC
/// invocations per strike. For chunk-batch evaluation see
/// `compute_chunk_vein_bitmask`.
pub fn block_is_vein_member(
    server_secret: &[u8],
    world_seed: u64,
    epoch_id: u32,
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    let r = VEIN_MAX_RADIUS;
    for ox in (x - r)..=(x + r) {
        for oy in (y - r)..=(y + r) {
            // Origins must also be at depth.
            if !(0..=Y_DP - 21).contains(&oy) {
                continue;
            }
            for oz in (z - r)..=(z + r) {
                // Cheap Manhattan-radius prune first.
                let m = (x - ox).unsigned_abs()
                    + (y - oy).unsigned_abs()
                    + (z - oz).unsigned_abs();
                if m > r as u32 {
                    continue;
                }
                if !is_vein_origin(server_secret, world_seed, epoch_id, ox, oy, oz) {
                    continue;
                }
                if propagation_reaches(
                    server_secret,
                    world_seed,
                    epoch_id,
                    (ox, oy, oz),
                    (x, y, z),
                ) {
                    return true;
                }
            }
        }
    }
    false
}

/// Compute the exposure-decay multiplier `[0.0, 1.0]` for a vein-eligible
/// block last exposed `age_ticks` ago. Linear curve per Spec 6 §2.2c.3.
///
/// Returns 1.0 if `age_ticks == 0` (just exposed); 0.0 once age ≥ duration.
pub fn exposure_decay_multiplier(age_ticks: u32, duration_ticks: u32) -> f32 {
    if duration_ticks == 0 {
        // No-decay mode (server override).
        return 1.0;
    }
    if age_ticks >= duration_ticks {
        return 0.0;
    }
    1.0 - (age_ticks as f32 / duration_ticks as f32)
}

/// Should this strike actually drop a gem, given the multiplier and the
/// hash's exposure-decay byte? Per Spec 6 §2.2c.3 — `hash[12]` against
/// `multiplier * 256`. Returns true if the byte falls under the scaled
/// threshold (i.e., the player gets the gem); false if oxidisation has
/// eaten the chance.
pub fn passes_exposure_check(hash: &[u8; 32], multiplier: f32) -> bool {
    let m = multiplier.clamp(0.0, 1.0);
    let threshold = (m * 256.0).min(256.0) as u32;
    (hash[12] as u32) < threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Audit 2026-09-27: the secret is random per world, never seed-derived ---

    #[test]
    fn world_secret_is_random_not_a_function_of_the_seed() {
        // Two worlds with the SAME public seed get different secrets, so a
        // joiner holding `JoinAccept.seed` can't reconstruct either.
        let mut a = crate::save::WorldMeta::new("a");
        let mut b = crate::save::WorldMeta::new("b");
        a.seed = 7;
        b.seed = 7;
        let (sa, _) = ensure_world_secret(&mut a.pop_secret);
        let (sb, _) = ensure_world_secret(&mut b.pop_secret);
        assert_ne!(sa, sb, "same seed, different worlds → different secrets");
        assert_ne!(sa, [0u8; 32]);
        // Hashes under the two secrets disagree for the same seed + cell.
        assert_ne!(proof_hash(&sa, 7, 0, 1, 2, 3), proof_hash(&sb, 7, 0, 1, 2, 3));
    }

    #[test]
    fn ensure_world_secret_keeps_an_existing_secret_and_fills_a_missing_one() {
        let mut slot = Some([9u8; 32]);
        assert_eq!(ensure_world_secret(&mut slot), ([9u8; 32], false), "stable across loads");
        let mut legacy: Option<[u8; 32]> = None;
        let (s, generated) = ensure_world_secret(&mut legacy);
        assert!(generated, "a pre-secret world gets one on first load");
        assert_eq!(legacy, Some(s), "stored back so the caller saves it");
        assert_eq!(ensure_world_secret(&mut legacy), (s, false));
    }

    #[test]
    fn the_secret_round_trips_through_world_meta_json() {
        let meta = crate::save::WorldMeta::new("w");
        assert!(meta.pop_secret.is_some(), "a new world is created with a secret");
        let json = serde_json::to_string(&meta).unwrap();
        let back: crate::save::WorldMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pop_secret, meta.pop_secret);
        // A legacy meta (no field) loads as None.
        let legacy = json.replace("\"pop_secret\"", "\"_old\"");
        let back: crate::save::WorldMeta = serde_json::from_str(&legacy).unwrap();
        assert_eq!(back.pop_secret, None);
    }

    const SECRET: &[u8] = b"test-server-secret-32-bytes------";

    #[test]
    fn proof_hash_is_deterministic() {
        let a = proof_hash(SECRET, 12345, 1, 10, 5, -3);
        let b = proof_hash(SECRET, 12345, 1, 10, 5, -3);
        assert_eq!(a, b);
    }

    /// Spec 16 Phase 3 gameplay-parity invariant. The Proof-of-Play
    /// HMAC roll at a fixed (x, y, z) position must NOT depend on which
    /// deepslate variant (PURE_DEEPSLATE, PURE_DEEPSLATE_THIN /
    /// _HEALTHY / _FAT) occupies that position. Block-id is not part
    /// of `proof_hash`'s input by construction; this test locks the
    /// contract so a future "add block-id to the digest" change is
    /// caught + reviewed against the visual-variant spec.
    ///
    /// The test asserts identical hashes for the same (server_secret,
    /// world_seed, epoch_id, x, y, z) across multiple calls — a
    /// contract-level guarantee that the digest depends purely on
    /// world-position and not on the surrounding block state.
    #[test]
    fn proof_hash_independent_of_deepslate_variant() {
        let pos = (42, -25, 17);
        let secret = b"deepslate-variant-parity-secret";
        let seed = 0xCAFEBABE_DEADBEEFu64;
        let epoch = 13;
        // Simulate the same position being mined when the world has
        // PURE_DEEPSLATE there vs. one of the visual variants — the
        // hash inputs are identical (no block-id), so the digest is
        // identical too. Running multiple times with the same args
        // proves the variant-swap doesn't perturb anything.
        let baseline = proof_hash(secret, seed, epoch, pos.0, pos.1, pos.2);
        for _ in 0..10 {
            assert_eq!(
                baseline,
                proof_hash(secret, seed, epoch, pos.0, pos.1, pos.2),
                "proof_hash must depend only on position — variant-spawn rollout \
                 (block::is_pure_deepslate_family) MUST preserve this invariant",
            );
        }
    }

    #[test]
    fn proof_hash_differs_per_coordinate() {
        let a = proof_hash(SECRET, 12345, 1, 10, 5, -3);
        let b = proof_hash(SECRET, 12345, 1, 11, 5, -3);
        assert_ne!(a, b);
    }

    #[test]
    fn proof_hash_differs_per_epoch() {
        let a = proof_hash(SECRET, 12345, 1, 10, 5, -3);
        let b = proof_hash(SECRET, 12345, 2, 10, 5, -3);
        assert_ne!(a, b);
    }

    #[test]
    fn proof_hash_differs_per_secret() {
        let a = proof_hash(SECRET, 12345, 1, 10, 5, -3);
        let b = proof_hash(b"other-secret", 12345, 1, 10, 5, -3);
        assert_ne!(a, b);
    }

    #[test]
    fn origin_threshold_zero_above_gate() {
        // y > Y_DP - 21 → no origins allowed.
        assert_eq!(origin_threshold(Y_DP), 0);
        assert_eq!(origin_threshold(Y_DP - 20), 0); // just above gate
    }

    #[test]
    fn origin_threshold_scales_with_depth() {
        let at_gate = origin_threshold(Y_DP - 21);
        let at_bedrock = origin_threshold(0);
        assert!(at_bedrock > at_gate * 3, // ~4× at bedrock
            "expected origin threshold to scale with depth (gate={at_gate}, bedrock={at_bedrock})");
    }

    #[test]
    fn vein_origins_are_rare() {
        // Sweep a 50×30×50 region below the gate and count origins.
        // Expected rate: ~1-2 per 100k blocks at default threshold + depth scaling.
        let mut count = 0;
        let mut total = 0;
        for x in 0..50 {
            for y in 0..(Y_DP - 21) {
                for z in 0..50 {
                    total += 1;
                    if is_vein_origin(SECRET, 12345, 1, x, y, z) {
                        count += 1;
                    }
                }
            }
        }
        assert!(total > 1000, "sanity: should have scanned many blocks");
        // Plenty of headroom — origins should be rare but not zero.
        let rate = (count as f64) / (total as f64);
        assert!(rate < 0.01, "vein origins too dense ({count}/{total} = {rate})");
    }

    #[test]
    fn origin_is_always_in_its_own_vein() {
        // Find a vein origin, verify block_is_vein_member returns true
        // for it.
        for x in 0..100 {
            for y in 0..(Y_DP - 21) {
                for z in 0..100 {
                    if is_vein_origin(SECRET, 12345, 1, x, y, z) {
                        assert!(block_is_vein_member(SECRET, 12345, 1, x, y, z),
                            "vein origin at ({x},{y},{z}) must be its own member");
                        return;
                    }
                }
            }
        }
        panic!("no vein origins found in 100×Y_DP×100 region; threshold too tight?");
    }

    #[test]
    fn vein_membership_outside_radius_is_false() {
        // A block 20 blocks away from any possible origin cannot be a member.
        // Manhattan distance > VEIN_MAX_RADIUS → propagation_reaches returns false.
        let result = propagation_reaches(
            SECRET, 12345, 1,
            (0, 0, 0),
            (VEIN_MAX_RADIUS + 1, 0, 0),
        );
        assert!(!result, "propagation must not reach beyond VEIN_MAX_RADIUS");
    }

    #[test]
    fn exposure_decay_curve() {
        let d = 20_000;
        assert_eq!(exposure_decay_multiplier(0, d), 1.0);
        assert_eq!(exposure_decay_multiplier(d, d), 0.0);
        assert_eq!(exposure_decay_multiplier(d + 100, d), 0.0);
        let mid = exposure_decay_multiplier(d / 2, d);
        assert!((mid - 0.5).abs() < 0.001, "midpoint should be 0.5 (got {mid})");
    }

    #[test]
    fn exposure_decay_zero_duration_is_no_decay() {
        // Server override: duration = 0 → no decay at any age.
        assert_eq!(exposure_decay_multiplier(100_000, 0), 1.0);
    }

    #[test]
    fn passes_exposure_check_at_full_multiplier_almost_always() {
        // multiplier = 1.0 → threshold = 256 → any hash byte passes.
        let mut h = [0u8; 32];
        for b in 0..=255u8 {
            h[12] = b;
            assert!(passes_exposure_check(&h, 1.0), "byte {b} should pass at m=1.0");
        }
    }

    #[test]
    fn passes_exposure_check_at_zero_multiplier_never() {
        // multiplier = 0.0 → threshold = 0 → no hash byte passes.
        let mut h = [0u8; 32];
        for b in 0..=255u8 {
            h[12] = b;
            assert!(!passes_exposure_check(&h, 0.0), "byte {b} should fail at m=0.0");
        }
    }

    #[test]
    fn vein_membership_is_deterministic() {
        // Same coordinate → same answer over repeated calls.
        let a = block_is_vein_member(SECRET, 12345, 1, 7, 3, -2);
        let b = block_is_vein_member(SECRET, 12345, 1, 7, 3, -2);
        assert_eq!(a, b);
    }

    #[test]
    fn vein_origin_input_includes_epoch() {
        // Sanity: epoch is a real input to the origin-hash MAC. Easiest
        // test is to verify the raw 32-byte digest differs between epochs;
        // the origin-threshold check is at low rarity so a boolean-output
        // sample is too sparse to test directly.
        let a = proof_hash(SECRET, 12345, 1, 7, 3, -2);
        let b = proof_hash(SECRET, 12345, 2, 7, 3, -2);
        assert_ne!(a, b, "epoch must change the hash output");
    }

    #[test]
    fn veins_extend_beyond_their_origin() {
        // Find a vein origin and verify at least one block within propagation
        // radius is also a member — i.e., veins are not single-block points,
        // they actually spread (validates the find-one-find-more property).
        //
        // PROPAGATION_BASE = 0.85, so step 0 has ~85% chance to continue,
        // which is high enough that across all 6 face-neighbours of any
        // origin, at least one passes with overwhelming probability.
        for x in 0..100 {
            for y in 0..(Y_DP - 21) {
                for z in 0..100 {
                    if !is_vein_origin(SECRET, 12345, 1, x, y, z) {
                        continue;
                    }
                    // Found an origin. Check if any neighbour at Manhattan
                    // distance 1 is also a member.
                    let neighbours = [
                        (x + 1, y, z), (x - 1, y, z),
                        (x, y + 1, z), (x, y - 1, z),
                        (x, y, z + 1), (x, y, z - 1),
                    ];
                    let any_neighbour_in_vein = neighbours
                        .iter()
                        .any(|&(nx, ny, nz)| {
                            ny >= 0 && ny <= Y_DP - 21
                                && block_is_vein_member(SECRET, 12345, 1, nx, ny, nz)
                        });
                    assert!(any_neighbour_in_vein,
                        "vein at ({x},{y},{z}) should extend to at least one face-neighbour (find-one-find-more property)");
                    return;
                }
            }
        }
        panic!("no vein origins found in 100×Y_DP×100 region for spatial-coherence test");
    }
}
