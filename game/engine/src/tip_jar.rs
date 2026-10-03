//! Tip Jar — first spectator-economy primitive (Spec 34).
//!
//! A placeable block tied to a player. Other players right-click to
//! send sats; the owner accrues `escrow_sats` and withdraws via the
//! same dialog. Honours Charter flag + Nostrich Vow gating at the
//! call site (in `game_loop.rs`); this module is pure data + pure
//! helpers.
//!
//! Owner is `TipJarOwner::LocalPlayer(pidx)` on alpha. The
//! `TipJarOwner::Npub(String)` variant is declared so the
//! post-Spec-1-Phase-4 migration is a plain enum-arm extension; the
//! same migration should cover `VendorOwner::LocalPlayer` per the
//! Round 2 pre-build audit (both share the latent split-screen →
//! solo-reload edge case).
//!
//! Spec: `docs/foundations/2026-05-23-tip-jar.md`.

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Owner of a Tip Jar. Two variants — the second is the live shape
/// once the Signet pubkey wiring lands.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TipJarOwner {
    /// Alpha shape — owner is a local-player slot index. Stable
    /// across save+reload as long as the player count is preserved.
    /// BRIDGE: split-screen → solo reload locks the owner out.
    /// Replace with `Npub(String)` when Spec 1 Phase 4 signing
    /// bridge lands — same migration should cover
    /// `VendorOwner::LocalPlayer`.
    LocalPlayer(usize),
    /// Post-Phase-4 shape — owner is identified by their NIP-19
    /// npub. Survives any save+reload and any player-slot shuffle.
    Npub(String),
}

/// Per-jar state. Lives in `World.block_entities` via
/// `BlockEntityData::TipJar(TipJarData)`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TipJarData {
    pub owner: Option<TipJarOwner>,
    pub escrow_sats: u64,
    /// Monotonic tick the last accepted tip arrived. Drives a brief
    /// sparkle animation on the block (BRIDGE: animation hook not
    /// yet wired — replace when entity-model gets per-block-entity
    /// animation slots).
    pub last_tip_tick: u64,
    /// Lifetime total tips received (informational; surfaces in the
    /// owner's withdraw dialog).
    pub lifetime_tips_received: u64,
}

/// Outcome of `try_tip`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipOutcome {
    /// Tip accepted. `credited_to_escrow` is the amount added to
    /// `data.escrow_sats` (always equals the request — the suppression
    /// + Charter gate lives at the caller, not here).
    Accepted { credited_to_escrow: u64 },
    /// Player tried to tip their own jar. UI hides the tip buttons
    /// for owners; this is the defensive fallback.
    SelfTipBlocked,
    /// Amount was 0 — defensive (UI doesn't fire 0-sat buttons).
    ZeroAmount,
    /// Jar has no owner set (shouldn't happen — place-time stamps
    /// it). Defensive against a corrupt save.
    NoOwner,
}

/// True iff `tipper_pidx` is the local-player owner of this jar.
/// For `Npub` owners this returns false (the owner-vs-tipper check
/// at that point uses an npub-equality predicate from the caller).
pub fn is_local_owner(owner: &Option<TipJarOwner>, tipper_pidx: usize) -> bool {
    matches!(owner, Some(TipJarOwner::LocalPlayer(p)) if *p == tipper_pidx)
}

/// Apply a tip. Pure state mutation on `data`. Returns the outcome;
/// the caller routes the side-effects (drain tipper sats balance,
/// fire `economy::apply_sats_payout` for accounting, emit audit
/// hash, toast).
pub fn try_tip(
    data: &mut TipJarData,
    tipper_pidx: usize,
    amount_sats: u64,
    current_tick: u64,
) -> TipOutcome {
    if data.owner.is_none() {
        return TipOutcome::NoOwner;
    }
    if amount_sats == 0 {
        return TipOutcome::ZeroAmount;
    }
    if is_local_owner(&data.owner, tipper_pidx) {
        return TipOutcome::SelfTipBlocked;
    }
    data.escrow_sats = data.escrow_sats.saturating_add(amount_sats);
    data.lifetime_tips_received = data.lifetime_tips_received.saturating_add(amount_sats);
    data.last_tip_tick = current_tick;
    TipOutcome::Accepted { credited_to_escrow: amount_sats }
}

/// Owner withdraws all escrowed sats. Returns the amount the caller
/// should credit to the requesting player's sats_balance.
/// Returns 0 (no-op) when the requesting player isn't the owner.
pub fn try_withdraw(data: &mut TipJarData, requesting_pidx: usize) -> u64 {
    if !is_local_owner(&data.owner, requesting_pidx) {
        return 0;
    }
    let amount = data.escrow_sats;
    data.escrow_sats = 0;
    amount
}

/// HMAC-SHA256 audit hash for a tip event. Mirrors
/// `bounty::claim_audit_hash`. Tipper pubkey + recipient pubkey +
/// amount + server_secret → 32 bytes.
///
/// v1: pubkeys are zeroed-out at the call site (BRIDGE — same
/// pattern as the bounty claim handler; resolves when Spec 1 Phase 4
/// signing bridge lands).
pub fn tip_audit_hash(
    tipper_pubkey: &[u8; 32],
    recipient_pubkey: &[u8; 32],
    amount_sats: u64,
    server_secret: &[u8],
) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(server_secret)
        .expect("HMAC accepts any key length");
    mac.update(b"tip-jar-v1");
    mac.update(tipper_pubkey);
    mac.update(recipient_pubkey);
    mac.update(&amount_sats.to_le_bytes());
    let out = mac.finalize().into_bytes();
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&out);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jar_owned_by(pidx: usize) -> TipJarData {
        TipJarData {
            owner: Some(TipJarOwner::LocalPlayer(pidx)),
            ..Default::default()
        }
    }

    #[test]
    fn try_tip_credits_escrow_for_non_owner() {
        let mut data = jar_owned_by(0);
        let out = try_tip(&mut data, 1, 5, 1000);
        assert_eq!(out, TipOutcome::Accepted { credited_to_escrow: 5 });
        assert_eq!(data.escrow_sats, 5);
        assert_eq!(data.lifetime_tips_received, 5);
        assert_eq!(data.last_tip_tick, 1000);
    }

    #[test]
    fn try_tip_blocks_self_tip() {
        let mut data = jar_owned_by(3);
        let out = try_tip(&mut data, 3, 5, 1000);
        assert_eq!(out, TipOutcome::SelfTipBlocked);
        assert_eq!(data.escrow_sats, 0,
            "self-tip must not accrue escrow");
    }

    #[test]
    fn try_tip_rejects_zero_amount() {
        let mut data = jar_owned_by(0);
        let out = try_tip(&mut data, 1, 0, 1000);
        assert_eq!(out, TipOutcome::ZeroAmount);
        assert_eq!(data.escrow_sats, 0);
    }

    #[test]
    fn try_tip_rejects_no_owner() {
        let mut data = TipJarData::default();
        let out = try_tip(&mut data, 1, 5, 1000);
        assert_eq!(out, TipOutcome::NoOwner);
        assert_eq!(data.escrow_sats, 0);
    }

    #[test]
    fn try_tip_accumulates_across_multiple_tips() {
        let mut data = jar_owned_by(0);
        let _ = try_tip(&mut data, 1, 5, 100);
        let _ = try_tip(&mut data, 1, 25, 200);
        let _ = try_tip(&mut data, 2, 100, 300);
        assert_eq!(data.escrow_sats, 130);
        assert_eq!(data.lifetime_tips_received, 130);
        assert_eq!(data.last_tip_tick, 300);
    }

    #[test]
    fn try_withdraw_drains_escrow_for_owner_only() {
        let mut data = jar_owned_by(0);
        data.escrow_sats = 250;
        data.lifetime_tips_received = 250;
        let amount = try_withdraw(&mut data, 0);
        assert_eq!(amount, 250);
        assert_eq!(data.escrow_sats, 0,
            "escrow must be drained");
        assert_eq!(data.lifetime_tips_received, 250,
            "lifetime counter must NOT reset on withdraw");
    }

    #[test]
    fn try_withdraw_returns_zero_for_non_owner() {
        let mut data = jar_owned_by(0);
        data.escrow_sats = 250;
        let amount = try_withdraw(&mut data, 1);
        assert_eq!(amount, 0);
        assert_eq!(data.escrow_sats, 250,
            "non-owner withdraw must not drain");
    }

    #[test]
    fn try_withdraw_zero_when_no_escrow() {
        let mut data = jar_owned_by(0);
        let amount = try_withdraw(&mut data, 0);
        assert_eq!(amount, 0);
    }

    #[test]
    fn is_local_owner_handles_npub_variant() {
        let owner = Some(TipJarOwner::Npub("npub1test".to_string()));
        // LocalPlayer-pidx check should be false for an Npub owner;
        // the caller uses a separate npub-equality predicate.
        assert!(!is_local_owner(&owner, 0));
        assert!(!is_local_owner(&owner, 7));
    }

    #[test]
    fn tip_audit_hash_is_deterministic() {
        let tipper = [0x11u8; 32];
        let recipient = [0x22u8; 32];
        let secret = b"test-secret";
        let h1 = tip_audit_hash(&tipper, &recipient, 25, secret);
        let h2 = tip_audit_hash(&tipper, &recipient, 25, secret);
        assert_eq!(h1, h2);
    }

    #[test]
    fn tip_audit_hash_changes_with_amount() {
        let tipper = [0x11u8; 32];
        let recipient = [0x22u8; 32];
        let secret = b"test-secret";
        let h1 = tip_audit_hash(&tipper, &recipient, 25, secret);
        let h2 = tip_audit_hash(&tipper, &recipient, 26, secret);
        assert_ne!(h1, h2);
    }

    #[test]
    fn tip_audit_hash_distinct_from_bounty_audit_hash() {
        // The two helpers use distinct domain separation strings
        // ("bounty-claim-v1" vs "tip-jar-v1") so the hashes can't
        // collide across payout kinds.
        let pubkey = [0xAAu8; 32];
        let secret = b"shared-secret";
        let tip_h = tip_audit_hash(&pubkey, &pubkey, 100, secret);
        let bounty_h = crate::bounty::claim_audit_hash(
            &pubkey, crate::mob::MobType::Brigand, 100, secret,
        );
        assert_ne!(tip_h, bounty_h);
    }
}
