//! Sign block-entity (Solo Buildout Wave 2c).
//!
//! A standing sign is a thin board on a post (an F1 `Sign` shape) that carries
//! a few lines of player-authored text. Right-click opens the `sign_ui` editor;
//! looking at a placed sign surfaces its text in the HUD. The text is held in a
//! `BlockEntityData::Sign(SignData)` on the world and persisted via
//! `save::SavedSign` — the same pattern as chests / furnaces.
//!
//! The geometry + facing live in `block_shape` (pure); only the *text* state
//! lives here. Signs are pass-through (no collision), matching Minecraft.

use serde::{Deserialize, Serialize};

/// Max characters a sign holds. Generous enough for four short lines; the
/// editor clamps to this so a pasted essay can't bloat a save.
pub const SIGN_MAX_CHARS: usize = 120;

/// Editable text on one sign. One free-form string (newlines allowed) rather
/// than four fixed slots — the renderer/HUD wraps it; the editor is a plain
/// multiline box. Empty is valid (a blank sign).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignData {
    pub text: String,
}

impl SignData {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clamp authored text to `SIGN_MAX_CHARS` (by char count, not bytes, so a
    /// multi-byte glyph is never split). Returns the stored string.
    pub fn set_text(&mut self, text: &str) -> &str {
        if text.chars().count() > SIGN_MAX_CHARS {
            self.text = text.chars().take(SIGN_MAX_CHARS).collect();
        } else {
            self.text = text.to_string();
        }
        &self.text
    }

    /// The lines to show, split on newlines, blank lines preserved. No
    /// production caller yet (the sign-rendering HUD reads `.text` directly);
    /// exercised by the tests below.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.text.split('\n')
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_text_clamps_to_max_chars() {
        let mut s = SignData::new();
        let long: String = "x".repeat(SIGN_MAX_CHARS + 50);
        s.set_text(&long);
        assert_eq!(s.text.chars().count(), SIGN_MAX_CHARS);
    }

    #[test]
    fn set_text_never_splits_a_multibyte_glyph() {
        let mut s = SignData::new();
        // Each emoji is multiple bytes but one char; clamping by char keeps
        // them whole.
        let emoji: String = "🪓".repeat(SIGN_MAX_CHARS + 10);
        s.set_text(&emoji);
        assert_eq!(s.text.chars().count(), SIGN_MAX_CHARS);
        // Round-trips through serde unchanged (no broken UTF-8).
        let json = serde_json::to_string(&s).unwrap();
        let back: SignData = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn lines_split_on_newlines_and_blank_detected() {
        let mut s = SignData::new();
        assert!(s.is_blank());
        s.set_text("Welcome\nto\nAxe'n'Stax");
        assert_eq!(s.lines().count(), 3);
        assert!(!s.is_blank());
        s.set_text("   ");
        assert!(s.is_blank(), "whitespace-only reads as blank");
    }
}
