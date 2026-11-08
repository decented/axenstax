//! The postcard moment: when a trial is finished, a burst of colour and a
//! chime in the world, then (a moment later, so the burst is in the picture)
//! the world view is captured without the HUD, reduced to a postcard, kept
//! locally (`postcard.rs`) and shown on the completion card. A capture that
//! fails is silent: the trial completes exactly as before, just without a
//! picture. One postcard per finished run; a retry or a new world resets it.

use web_time::{Duration, Instant};

/// How long the burst plays before the picture is taken.
const MOMENT: Duration = Duration::from_millis(650);

/// How many frames the GPU read-back may take before the picture is given up
/// on (about three seconds at 60 frames a second). A read-back that never
/// completes (a lost device, a throttled tab) must not hold the card's
/// "developing" slot for the rest of the run.
const READBACK_FRAME_LIMIT: u32 = 180;

/// Where the postcard flow is for the current run.
#[derive(Default)]
pub(crate) enum PostcardFlow {
    /// No trial finished in this run yet.
    #[default]
    Idle,
    /// The burst is playing; the picture is taken at `at`.
    Moment { token: &'static str, at: Instant },
    /// The picture is being read back from the GPU; `frames` counts the
    /// frames spent waiting (see [`READBACK_FRAME_LIMIT`]).
    Capturing { token: &'static str, frames: u32 },
    /// Done for this run: the picture, if one was kept.
    Shown { tex: Option<egui::TextureHandle> },
}

/// What the completion card shows in its picture slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PostcardSlot {
    /// Nothing (no postcard this run, or it could not be taken).
    None,
    /// Being taken: the slot is held so the buttons don't jump.
    Developing,
    /// The postcard.
    Ready(egui::TextureId),
}

impl PostcardFlow {
    pub(crate) fn slot(&self) -> PostcardSlot {
        match self {
            PostcardFlow::Idle => PostcardSlot::None,
            PostcardFlow::Moment { .. } | PostcardFlow::Capturing { .. } => PostcardSlot::Developing,
            PostcardFlow::Shown { tex } => tex.as_ref().map_or(PostcardSlot::None, |t| PostcardSlot::Ready(t.id())),
        }
    }
}

/// Has the read-back waited long enough (this many frames) to be dropped?
fn readback_expired(frames_waited: u32) -> bool {
    frames_waited >= READBACK_FRAME_LIMIT
}

/// Turn a kept postcard into an egui texture.
pub(crate) fn texture_from_png(ctx: &egui::Context, name: &str, png: &[u8]) -> Option<egui::TextureHandle> {
    let (w, h, rgba) = crate::postcard::decode_png(png)?;
    let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
    Some(ctx.load_texture(name, img, egui::TextureOptions::LINEAR))
}

impl crate::GameState {
    /// A trial with this postcard token was just finished: play the moment and
    /// schedule the picture. Only the first call in a run does anything.
    pub(crate) fn begin_postcard(&mut self, token: &'static str) {
        if !matches!(self.postcard, PostcardFlow::Idle) || self.players.is_empty() {
            return;
        }
        let eye = self.players[0].player.eye_pos();
        let ahead = self.players[0].camera.forward();
        let at = eye + glam::Vec3::new(ahead.x, 0.0, ahead.z).normalize_or_zero() * 2.5
            - glam::Vec3::new(0.0, 0.6, 0.0);
        let seed = crate::postcard::now_ms();
        self.particles.burst_confetti(at, 70, seed);
        self.audio.play_gem_pickup();
        self.postcard = PostcardFlow::Moment { token, at: Instant::now() + MOMENT };
    }

    /// Advance the postcard flow by one frame. `aspect` is player 0's viewport
    /// width / height. Never blocks.
    pub(crate) fn step_postcard(&mut self, aspect: f32) {
        match &self.postcard {
            PostcardFlow::Moment { token, at } if Instant::now() >= *at => {
                let token = *token;
                self.postcard = if self.renderer.start_postcard_capture(0, aspect) {
                    PostcardFlow::Capturing { token, frames: 0 }
                } else {
                    PostcardFlow::Shown { tex: None }
                };
            }
            PostcardFlow::Capturing { token, frames } => {
                let (token, frames) = (*token, *frames);
                match self.renderer.poll_postcard_capture() {
                    crate::renderer::PostcardPoll::Pending => {
                        if readback_expired(frames + 1) {
                            // Give up quietly: the card keeps the art and no postcard.
                            self.renderer.cancel_postcard_capture();
                            self.postcard = PostcardFlow::Shown { tex: None };
                        } else {
                            self.postcard = PostcardFlow::Capturing { token, frames: frames + 1 };
                        }
                    }
                    crate::renderer::PostcardPoll::Ready { rgba, width, height } => {
                        let tex = crate::postcard::crop_and_reduce(
                            &rgba,
                            width,
                            height,
                            crate::postcard::POSTCARD_W,
                            crate::postcard::POSTCARD_H,
                        )
                        .and_then(|card| crate::postcard::save(token, &card, crate::postcard::now_ms()))
                        .and_then(|png| texture_from_png(&self.renderer.egui.ctx, "trial_postcard", &png));
                        self.postcard = PostcardFlow::Shown { tex };
                    }
                    crate::renderer::PostcardPoll::Failed | crate::renderer::PostcardPoll::Idle => {
                        self.postcard = PostcardFlow::Shown { tex: None };
                    }
                }
            }
            _ => {}
        }
    }

    /// A new run, a retry or a new world: the next finish takes a new postcard.
    pub(crate) fn reset_postcard(&mut self) {
        self.postcard = PostcardFlow::Idle;
        self.renderer.cancel_postcard_capture();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slot_is_held_while_developing_and_empty_when_nothing_was_kept() {
        assert_eq!(PostcardFlow::Idle.slot(), PostcardSlot::None);
        let moment = PostcardFlow::Moment { token: "sprint", at: Instant::now() };
        assert_eq!(moment.slot(), PostcardSlot::Developing);
        assert_eq!(PostcardFlow::Capturing { token: "sprint", frames: 0 }.slot(), PostcardSlot::Developing);
        assert_eq!(PostcardFlow::Shown { tex: None }.slot(), PostcardSlot::None);
    }

    #[test]
    fn a_read_back_that_never_completes_is_dropped_after_a_few_seconds() {
        assert!(!readback_expired(0));
        assert!(!readback_expired(READBACK_FRAME_LIMIT - 1));
        assert!(readback_expired(READBACK_FRAME_LIMIT));
        // Three seconds at 60 frames a second, give or take.
        assert!((150..=240).contains(&READBACK_FRAME_LIMIT));
    }
}
