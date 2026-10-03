//! Accessibility narration (#24) — Web Speech (TTS) for the HUD/status, so a
//! player who can't read the screen can still navigate (e.g. hear which hotbar
//! slot + item is selected). Deciding WHAT to narrate is pure + testable; the
//! actual `speak()` is web-only (a no-op on native for now — OS TTS is a
//! follow-up). Off by default — opt-in via Settings.
//!
//! Spec: `docs/foundations/2026-06-16-accessibility-narration.md`.

/// Throttles a narration stream so the same phrase isn't re-spoken every frame.
#[derive(Default)]
pub struct Narrator {
    last: String,
}

impl Narrator {
    /// Returns the phrase to speak only when it differs from the last one;
    /// otherwise `None` (already spoken — stay quiet).
    pub fn next(&mut self, phrase: String) -> Option<String> {
        if phrase == self.last {
            None
        } else {
            self.last = phrase.clone();
            Some(phrase)
        }
    }

    /// Forget the last phrase (e.g. on world entry) so it re-narrates. No
    /// production caller yet — tested directly.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn reset(&mut self) {
        self.last.clear();
    }
}

/// Narration phrase for the selected hotbar slot (1-indexed for speech).
pub fn hotbar_phrase(slot: usize, item_name: Option<&str>) -> String {
    match item_name {
        Some(n) => format!("Slot {}: {}", slot + 1, n),
        None => format!("Slot {}: empty", slot + 1),
    }
}

/// Narration phrase for a health value (clamped at 0). Unlike `hotbar_phrase`
/// above (live in game_loop.rs), no HUD narration hook reads this yet.
#[cfg_attr(not(test), allow(dead_code))]
pub fn health_phrase(hp: i32, max: i32) -> String {
    format!("Health {} of {}", hp.max(0), max)
}

/// Speak `text` via the browser's Web Speech API, cancelling any in-flight
/// utterance so navigation stays responsive. No-op on native (OS TTS = follow-up).
#[cfg(target_arch = "wasm32")]
pub fn speak(text: &str) {
    if let Some(win) = web_sys::window() {
        if let Ok(synth) = win.speech_synthesis() {
            synth.cancel();
            if let Ok(u) = web_sys::SpeechSynthesisUtterance::new_with_text(text) {
                synth.speak(&u);
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn speak(_text: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrator_speaks_only_on_change() {
        let mut n = Narrator::default();
        assert_eq!(n.next("a".into()), Some("a".to_string()));
        assert_eq!(n.next("a".into()), None, "same phrase stays quiet");
        assert_eq!(n.next("b".into()), Some("b".to_string()));
        n.reset();
        assert_eq!(n.next("b".into()), Some("b".to_string()), "reset re-narrates");
    }

    #[test]
    fn hotbar_phrase_is_one_indexed_and_handles_empty() {
        assert_eq!(hotbar_phrase(0, Some("Stone")), "Slot 1: Stone");
        assert_eq!(hotbar_phrase(8, None), "Slot 9: empty");
    }

    #[test]
    fn health_phrase_clamps_at_zero() {
        assert_eq!(health_phrase(6, 8), "Health 6 of 8");
        assert_eq!(health_phrase(-3, 8), "Health 0 of 8");
    }
}
