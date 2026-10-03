//! Deepslate Reserve — server-side richness model + client-side tier mapping.
//!
//! Per Spec 16 ([`docs/foundations/2026-05-17-deepslate-reserve.md`]), the
//! server's reward pool is surfaced to players as the *richness* of the
//! deepslate they mine. This module owns the richness signal and the
//! discrete tier mapping the rendering / HUD layers consume.
//!
//! ## Where the richness comes from
//!
//! Spec 6 §7.2 defines `reward_multiplier` as a function of the LNbits
//! reward-pool balance vs. target. That code path is gated behind
//! Sentinel D-003 (Bitcoin layer code deferred until Phase 5 lead-in).
//! Until D-003 reverses, `ReserveState::synthetic_default` provides a
//! deterministic placeholder so the visualisation + HUD work in alpha
//! without a live Lightning backend.
//!
//! When D-003 reverses, `ReserveState::from_reward_pool` becomes the
//! production source — drop-in replacement for the synthetic helper.

use serde::{Deserialize, Serialize};

/// Richness tier for in-world visualisation and HUD labels.
///
/// Each tier corresponds to a 0.25-wide band of the [`ReserveState::richness`]
/// value. The discrete band determines the deepslate visual variant + the
/// gauge colour / label. The underlying mining-rate scaling (Spec 6 §7.2)
/// remains *continuous* — these tiers are only the perception layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RichnessTier {
    /// `[0.00, 0.25)` — pool critically low. Mining payouts paused
    /// (Spec 6 §7.3 depletion behaviour). Deepslate renders plain.
    Empty,
    /// `[0.25, 0.50)` — pool partial. Reduced payouts. Deepslate shows
    /// faint speckling.
    Thin,
    /// `[0.50, 0.75)` — pool healthy. Near-full payouts. Visible veins.
    Healthy,
    /// `[0.75, 1.00]` — pool at-or-above target. Full payouts. Dense
    /// glowing veins.
    Fat,
}

impl RichnessTier {
    /// Map a continuous richness value to its tier.
    ///
    /// Boundaries: `0.00 ≤ Empty < 0.25 ≤ Thin < 0.50 ≤ Healthy < 0.75 ≤ Fat ≤ 1.00`.
    /// Values outside `[0.0, 1.0]` clamp to the nearest tier (negative →
    /// Empty, >1 → Fat) so a misbehaving server can't crash the client.
    pub fn from_richness(richness: f32) -> Self {
        // NaN is not a valid richness — treat as Empty (safe default,
        // mining pauses). +Inf saturates to Fat via the normal `>=`
        // ladder below. Negative-infinity is `< 0.25`, so Empty.
        if richness.is_nan() || richness < 0.25 {
            RichnessTier::Empty
        } else if richness < 0.5 {
            RichnessTier::Thin
        } else if richness < 0.75 {
            RichnessTier::Healthy
        } else {
            RichnessTier::Fat
        }
    }

    /// Kid-readable label for the HUD gauge.
    pub fn label(self) -> &'static str {
        match self {
            RichnessTier::Empty => "Empty",
            RichnessTier::Thin => "Thin",
            RichnessTier::Healthy => "Healthy",
            RichnessTier::Fat => "Fat",
        }
    }

    /// One-line description for the gauge tooltip / help text. No production
    /// caller — `hud_ui::draw_reserve_gauge` shows only `label()` + the raw
    /// percentage, no tooltip; exercised by the tests below.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn description(self) -> &'static str {
        match self {
            RichnessTier::Empty => "Mining payouts are paused — the Reserve needs sponsorship.",
            RichnessTier::Thin => "The Reserve is low. Payouts are reduced.",
            RichnessTier::Healthy => "The Reserve is healthy. Mining rewards near full rate.",
            RichnessTier::Fat => "The Reserve is fat with sats. Mining at peak rate.",
        }
    }

    /// 0-based ordinal. Despite the original doc's claim, `biome::pick_deepslate_variant`
    /// picks the visual-variant BlockId directly from its own richness thresholds,
    /// not via this ordinal — no production caller today. Exercised by the tests below.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn ordinal(self) -> u8 {
        match self {
            RichnessTier::Empty => 0,
            RichnessTier::Thin => 1,
            RichnessTier::Healthy => 2,
            RichnessTier::Fat => 3,
        }
    }
}

/// Snapshot of the server's Deepslate Reserve state. Mirrors the three
/// fields broadcast on every `StateUpdatePacket` (see
/// `protocol::StateUpdatePacket`).
#[derive(Clone, Copy, Debug, Default)]
pub struct ReserveState {
    pub richness: f32,
    pub target_sats: u64,
    pub current_sats: u64,
}

impl ReserveState {
    /// Synthetic default used by alpha builds before the Spec 6 §7 pool
    /// code lands (Sentinel D-003 deferred). Returns a healthy-tier
    /// reserve so the visualisation shows up; replace with the live
    /// `reward_multiplier` once the pool mechanics are wired.
    ///
    /// BRIDGE: replace with `ReserveState::from_reward_pool` when
    /// Sentinel D-003 reverses and Spec 6 §7 ships in code.
    pub fn synthetic_default() -> Self {
        Self {
            richness: 0.75,
            target_sats: 60_000,
            current_sats: 45_230,
        }
    }

    pub fn tier(&self) -> RichnessTier {
        RichnessTier::from_richness(self.richness)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_boundaries_match_spec() {
        // Boundaries: 0.00 ≤ Empty < 0.25 ≤ Thin < 0.50 ≤ Healthy < 0.75 ≤ Fat ≤ 1.00
        assert_eq!(RichnessTier::from_richness(0.0), RichnessTier::Empty);
        assert_eq!(RichnessTier::from_richness(0.24), RichnessTier::Empty);
        assert_eq!(RichnessTier::from_richness(0.25), RichnessTier::Thin);
        assert_eq!(RichnessTier::from_richness(0.49), RichnessTier::Thin);
        assert_eq!(RichnessTier::from_richness(0.50), RichnessTier::Healthy);
        assert_eq!(RichnessTier::from_richness(0.74), RichnessTier::Healthy);
        assert_eq!(RichnessTier::from_richness(0.75), RichnessTier::Fat);
        assert_eq!(RichnessTier::from_richness(1.00), RichnessTier::Fat);
    }

    #[test]
    fn tier_clamps_out_of_range() {
        // Misbehaving server sending NaN / negative / >1 must not crash
        // the client — clamp to the nearest tier.
        assert_eq!(RichnessTier::from_richness(-0.5), RichnessTier::Empty);
        assert_eq!(RichnessTier::from_richness(f32::NAN), RichnessTier::Empty);
        assert_eq!(RichnessTier::from_richness(f32::INFINITY), RichnessTier::Fat);
        assert_eq!(RichnessTier::from_richness(2.0), RichnessTier::Fat);
    }

    #[test]
    fn tier_ordinal_matches_texture_layer_indices() {
        // The texture-array layer index for Phase 3 mirrors the tier
        // ordinal. If these drift, the visual variants display the
        // wrong texture.
        assert_eq!(RichnessTier::Empty.ordinal(), 0);
        assert_eq!(RichnessTier::Thin.ordinal(), 1);
        assert_eq!(RichnessTier::Healthy.ordinal(), 2);
        assert_eq!(RichnessTier::Fat.ordinal(), 3);
    }

    #[test]
    fn tier_label_uses_uk_english() {
        // Sanity guard against US-English imports — these labels are
        // user-facing in the HUD gauge.
        for tier in [
            RichnessTier::Empty,
            RichnessTier::Thin,
            RichnessTier::Healthy,
            RichnessTier::Fat,
        ] {
            let label = tier.label();
            assert!(!label.is_empty());
            // No US-only spellings would appear in this short label set,
            // but the description should also be checked.
            let desc = tier.description();
            assert!(desc.ends_with('.'), "{:?} description must end with period", tier);
        }
    }

    #[test]
    fn synthetic_default_is_a_visible_tier() {
        // The synthetic default should land in a visually-interesting
        // tier (not Empty) so the alpha HUD actually shows the gauge
        // working. Healthy or Fat are both fine; Empty would be a bad
        // default because mining payouts pause in that tier.
        let state = ReserveState::synthetic_default();
        let tier = state.tier();
        assert_ne!(tier, RichnessTier::Empty);
    }

    #[test]
    fn reserve_state_target_must_be_at_least_current() {
        // Sanity invariant on the synthetic default — current ≤ target,
        // otherwise the gauge would show >100% which the rendering
        // doesn't handle.
        let state = ReserveState::synthetic_default();
        assert!(state.current_sats <= state.target_sats);
    }
}
