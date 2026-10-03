//! Per-world play mode (Spec 05 §8). The single source of truth for how a
//! player interacts with the world. Named `PlayMode` — NOT `GameMode`, which
//! is already the top-level UI state machine in `main.rs` (Splash/Menu/Playing).
//!
//! Current granularity is world-level (one mode per world/session), matching
//! the legacy `is_creative` flag it replaces. Spec 05 §8.4's per-player target
//! is a documented follow-up (see the BRIDGE note in Spec 05 §8).
//!
//! `is_creative: bool` survives elsewhere as a SINGLE-WRITER CACHED PROJECTION
//! of `self == Creative`, written only via `GameState::set_play_mode` /
//! `GameServer::set_play_mode`. Read sites are unchanged; this enum owns the
//! truth.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlayMode {
    /// Full loop: health/hunger, mining, crafting, combat, proof-of-play.
    #[default]
    Survival,
    /// Fly, infinite blocks, instant break, no damage. Powers Workshop
    /// authoring + creators building maps.
    Creative,
    /// Permission-locked: the world is read-only to the player (v1 denies ALL
    /// break/place; a per-block allowlist is the reserved follow-up). Other
    /// interactions (doors, mobs, trade) still work. Powers scenarios,
    /// challenges, tutorials, and "play this build, don't wreck it".
    Adventure,
    /// Local spectator: noclip fly, no break/place/interact. The full 3-tier
    /// networked spectator (Spec 04) — watching others, invisibility,
    /// entity-POV — is out of scope here.
    Spectator,
}

impl PlayMode {
    /// True only for Creative. This is the predicate the legacy `is_creative`
    /// cache mirrors — creative privileges (infinite blocks, instant break,
    /// plot bypass, command availability, UI affordances).
    pub fn is_creative(self) -> bool {
        matches!(self, PlayMode::Creative)
    }

    // No caller reads these two (not even a test) — `is_creative`/`is_spectator`
    // below are the ones actually consulted; kept for the symmetric predicate
    // family a future per-mode gate (e.g. an Adventure allowlist check) would want.
    #[allow(dead_code)]
    pub fn is_survival(self) -> bool {
        matches!(self, PlayMode::Survival)
    }

    #[allow(dead_code)]
    pub fn is_adventure(self) -> bool {
        matches!(self, PlayMode::Adventure)
    }

    pub fn is_spectator(self) -> bool {
        matches!(self, PlayMode::Spectator)
    }

    /// Whether the player may break or place blocks at all. Survival + Creative
    /// can edit; Adventure + Spectator cannot (v1 — Adventure's per-block
    /// allowlist is the reserved follow-up). This is the predicate the two
    /// break/place gates in `game_loop.rs` consult.
    pub fn can_edit_world(self) -> bool {
        matches!(self, PlayMode::Survival | PlayMode::Creative)
    }

    /// Whether the player can fly. Creative toggles flight; Spectator always
    /// flies (noclip). Survival + Adventure are grounded.
    pub fn flies(self) -> bool {
        matches!(self, PlayMode::Creative | PlayMode::Spectator)
    }

    /// Whether the player passes through blocks (collision disabled).
    /// Spectator only.
    pub fn noclip(self) -> bool {
        matches!(self, PlayMode::Spectator)
    }

    /// Persisted/JSON string form, stored as `WorldMeta.game_mode`. Stable —
    /// changing these breaks save round-tripping.
    pub fn as_meta_str(self) -> &'static str {
        match self {
            PlayMode::Survival => "survival",
            PlayMode::Creative => "creative",
            PlayMode::Adventure => "adventure",
            PlayMode::Spectator => "spectator",
        }
    }

    /// Parse from the persisted `WorldMeta.game_mode` string. Unknown / legacy
    /// values fall back to Survival (forward-compatible: an old binary reading
    /// a newer mode it doesn't know lands on the safest default).
    pub fn from_meta_str(s: &str) -> PlayMode {
        match s {
            "creative" => PlayMode::Creative,
            "adventure" => PlayMode::Adventure,
            "spectator" => PlayMode::Spectator,
            _ => PlayMode::Survival,
        }
    }

    /// Short human label for chat/HUD.
    pub fn label(self) -> &'static str {
        match self {
            PlayMode::Survival => "Survival",
            PlayMode::Creative => "Creative",
            PlayMode::Adventure => "Adventure",
            PlayMode::Spectator => "Spectator",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [PlayMode; 4] = [
        PlayMode::Survival,
        PlayMode::Creative,
        PlayMode::Adventure,
        PlayMode::Spectator,
    ];

    #[test]
    fn default_is_survival() {
        assert_eq!(PlayMode::default(), PlayMode::Survival);
    }

    #[test]
    fn can_edit_world_truth_table() {
        assert!(PlayMode::Survival.can_edit_world());
        assert!(PlayMode::Creative.can_edit_world());
        assert!(!PlayMode::Adventure.can_edit_world());
        assert!(!PlayMode::Spectator.can_edit_world());
    }

    #[test]
    fn flies_truth_table() {
        assert!(!PlayMode::Survival.flies());
        assert!(PlayMode::Creative.flies());
        assert!(!PlayMode::Adventure.flies());
        assert!(PlayMode::Spectator.flies());
    }

    #[test]
    fn noclip_is_spectator_only() {
        assert!(PlayMode::Spectator.noclip());
        for m in ALL {
            if m != PlayMode::Spectator {
                assert!(!m.noclip(), "{m:?} must not noclip");
            }
        }
    }

    #[test]
    fn is_creative_matches_cache_predicate() {
        for m in ALL {
            assert_eq!(m.is_creative(), m == PlayMode::Creative);
        }
    }

    #[test]
    fn meta_str_round_trips_all_modes() {
        for m in ALL {
            assert_eq!(PlayMode::from_meta_str(m.as_meta_str()), m);
        }
    }

    #[test]
    fn legacy_and_unknown_meta_decode_to_survival() {
        assert_eq!(PlayMode::from_meta_str("survival"), PlayMode::Survival);
        assert_eq!(PlayMode::from_meta_str("creative"), PlayMode::Creative);
        assert_eq!(PlayMode::from_meta_str(""), PlayMode::Survival);
        assert_eq!(PlayMode::from_meta_str("garbage"), PlayMode::Survival);
    }
}
