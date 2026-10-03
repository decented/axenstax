//! gamestr — opt-in publish of Hash Dash scores to the gamestr leaderboard
//! (NIP-133, kind 33334) from the PWA/WASM build.
//!
//! Design: `docs/superpowers/specs/2026-06-11-gamestr-wasm-scoring-design.md`.
//!
//! All wire concerns — the signer, the relay, the minimal kind-0 handle, and the
//! opt-in (default-OFF) overlay — live in `tools/sites/game/static/gamestr.js`.
//! This module is only the TRIGGER and the SCOPE GATE: at a scenario's
//! not-ended → ended edge the game loop calls [`offer_score`], which (on WASM,
//! for an in-scope scenario with a signer present) hands the score to the JS
//! overlay. Nothing is signed or published unless the player ticks the box.
//!
//! Score integrity is HONOUR-SYSTEM for v1: a client-signed kind-33334 event is
//! forgeable. Acceptable for a staffed booth; server-validated proof-of-play is
//! the long-run answer (out of scope). See the spec §1.

use crate::scenario::{ScenarioDef, ScenarioKind};

/// The gamestr `d`-tag game id for a scenario, or `None` when it must NOT be
/// boarded. v1 scope is **Hash Dash only** — this match is the single point that
/// enforces it, so adding a new [`ScenarioKind`] forces an explicit boarding
/// decision rather than silently publishing.
pub fn gamestr_game_id(def: &ScenarioDef) -> Option<String> {
    match def.kind {
        ScenarioKind::HashDash => Some("axenstax-hash-dash".to_string()),
        // Satori Rush is time-to-genesis (lower = better), which clashes with
        // NIP-133's higher-is-better content; deferred until we pick a derived
        // score. Test scenarios never board.
        // Coverage challenges have no comparable score → never board.
        ScenarioKind::SatoriRush
        | ScenarioKind::Challenge
        | ScenarioKind::Test => None,
    }
}

/// Whether to emit the leaderboard offer on this simulation tick: true *exactly*
/// on the not-ended → ended transition, so the offer fires once per scenario.
/// (A Timed scenario stays ended once the timer expires, so `was_ended` is true
/// on every subsequent tick and this never re-fires.) Pure — unit-tested on all
/// targets.
pub fn should_offer_on_tick(was_ended: bool, now_ended: bool) -> bool {
    !was_ended && now_ended
}

// --- WASM bridge to tools/sites/game/static/gamestr.js ----------------------
// Mirrors the open_stash.rs / beacon.js extern pattern. The engine treats the
// offer as best-effort and fire-and-forget.

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    /// Show the opt-in leaderboard overlay for `game_id` with `work` points.
    /// gamestr.js owns the checkbox (default OFF), the handle field, the signer
    /// acquisition, and the kind-0 + kind-33334 publish; nothing is signed unless
    /// the player opts in and clicks Post.
    #[wasm_bindgen(js_name = axenstax_gamestr_offer, catch)]
    async fn js_gamestr_offer(game_id: String, work: f64) -> Result<JsValue, JsValue>;
}

/// Offer to publish a just-ended scenario's score to the leaderboard. No-op
/// unless the scenario is in scope ([`gamestr_game_id`] is `Some`). On WASM the
/// overlay ALWAYS appears for an in-scope scenario; it does NOT gate on a live
/// signer, because a phone bunker may not be connected at the round-end instant
/// (the background reconnect can take >15s). gamestr.js connects the bunker
/// lazily when the player clicks Post — the same pattern cloud save uses.
///
/// The scope gate runs on every target (so it's exercised by the unit tests and
/// a mis-scoped scenario is caught everywhere); native is a no-op past the gate
/// (no DOM / JS signer).
pub fn offer_score(def: &ScenarioDef, score: u64) {
    let Some(game_id) = gamestr_game_id(def) else {
        return;
    };
    #[cfg(target_arch = "wasm32")]
    {
        let work = score as f64;
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = js_gamestr_offer(game_id, work).await {
                log::warn!("gamestr offer failed: {e:?}");
            }
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Native has no JS signer/relay — nothing to publish past the gate.
        let _ = (game_id, score);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::{builtin_test_def, hash_dash_def, satori_rush_def};

    #[test]
    fn hash_dash_boards_with_stable_id() {
        assert_eq!(
            gamestr_game_id(&hash_dash_def()).as_deref(),
            Some("axenstax-hash-dash")
        );
    }

    #[test]
    fn satori_rush_does_not_board() {
        assert_eq!(gamestr_game_id(&satori_rush_def()), None);
    }

    #[test]
    fn test_scenario_does_not_board() {
        assert_eq!(gamestr_game_id(&builtin_test_def()), None);
    }

    #[test]
    fn offer_fires_once_on_end_transition() {
        assert!(should_offer_on_tick(false, true), "fires on not-ended -> ended");
        assert!(!should_offer_on_tick(false, false), "no end, no offer");
        assert!(!should_offer_on_tick(true, true), "already ended, no re-fire");
        assert!(!should_offer_on_tick(true, false), "impossible un-end, no offer");
    }
}
