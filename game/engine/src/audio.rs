//! Audio engine — procedural sound effects for block interaction and movement.
//!
//! Uses rodio for playback. Sounds are generated as short PCM buffers.
//! Spec 05 Section 11: footsteps every 0.45s walking, 0.35s sprinting.
//!
//! On WASM: audio is disabled (silent stubs). rodio is native-only.

// ─── Native audio (rodio) ────────────────────────────────────────────────────
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::time::{Duration, Instant};
    use rodio::Source;

    pub struct AudioEngine {
        _stream: Option<rodio::OutputStream>,
        stream_handle: Option<rodio::OutputStreamHandle>,
        last_footstep: Instant,
    }

    impl AudioEngine {
        pub fn new() -> Self {
            let (stream, stream_handle) = match rodio::OutputStream::try_default() {
                Ok((s, h)) => (Some(s), Some(h)),
                Err(e) => {
                    log::warn!("Audio init failed (will run silent): {e}");
                    (None, None)
                }
            };
            Self { _stream: stream, stream_handle, last_footstep: Instant::now() }
        }

        /// No caller — `new()` already falls back to silent internally on
        /// init failure, so an explicit silent constructor is redundant today.
        #[allow(dead_code)]
        pub fn new_silent() -> Self {
            Self { _stream: None, stream_handle: None, last_footstep: Instant::now() }
        }

        pub fn play_break(&self) { self.play_noise(0.08, 0.3); }
        pub fn play_place(&self) { self.play_tone(0.06, 600.0, 0.25); }

        /// Spec 49 (Explosives) — the Blasting Keg "boom": a low-frequency body
        /// tone under a longer, louder debris/blast noise burst. Implements the
        /// Spec 05 §11 Explosion sound hook. Tunable at the Axolittle playtest.
        pub fn play_explosion(&self) {
            self.play_tone(0.45, 70.0, 0.45); // low-frequency body
            self.play_noise(0.50, 0.50);      // debris / blast noise
        }

        /// Thunder — a longer, deeper rumble than the keg boom. Fired by a
        /// lightning strike during a thunderstorm (2026-07-05).
        pub fn play_thunder(&self) {
            self.play_tone(1.1, 52.0, 0.40);
            self.play_noise(1.4, 0.30);
        }

        /// Routine Satori pickup — small bright chime (single high note).
        /// Fires on every Satori drop *except* the first Satori found in
        /// this world; that one gets `play_genesis_block` instead.
        pub fn play_gem_pickup(&self) {
            // Bright A5 ~880 Hz, short attack.
            self.play_tone(0.12, 880.0, 0.28);
        }

        /// Genesis Block fanfare — the *first* Satori found in this world.
        /// Ascending major-triad arpeggio
        /// (C5 → E5 → G5 → C6) over ~0.5s. Distinct enough that the player
        /// can't miss the milestone.
        pub fn play_genesis_block(&self) {
            // Arpeggio: C5, E5, G5, C6 each ~0.12s, slight overlap.
            let notes: &[(f32, f32)] = &[
                (523.25, 0.0),  // C5
                (659.25, 0.10), // E5
                (783.99, 0.20), // G5
                (1046.50, 0.30), // C6
            ];
            for &(freq, delay) in notes {
                self.play_tone_delayed(0.20, freq, 0.32, delay);
            }
        }

        /// Play a tone after `delay_secs` of silence. Used for the
        /// Genesis-Block arpeggio. Implementation: prepend zero-samples.
        fn play_tone_delayed(&self, duration_secs: f32, freq: f32, volume: f32, delay_secs: f32) {
            let Some(handle) = &self.stream_handle else { return };
            let sample_rate = 44100u32;
            let delay_samples = (sample_rate as f32 * delay_secs) as usize;
            let num_samples = (sample_rate as f32 * duration_secs) as usize;
            let total = delay_samples + num_samples;
            let mut samples = Vec::with_capacity(total);
            samples.resize(delay_samples, 0.0);
            for i in 0..num_samples {
                let t = i as f32 / sample_rate as f32;
                let envelope = 1.0 - (t / duration_secs);
                let sample = (t * freq * 2.0 * std::f32::consts::PI).sin() * volume * envelope;
                samples.push(sample);
            }
            let source = rodio::buffer::SamplesBuffer::new(1, sample_rate, samples);
            let _ = handle.play_raw(source.convert_samples());
        }

        pub fn play_footstep(&mut self, sprinting: bool) {
            let interval = if sprinting { Duration::from_millis(350) } else { Duration::from_millis(450) };
            if self.last_footstep.elapsed() >= interval {
                self.play_noise(0.04, 0.15);
                self.last_footstep = Instant::now();
            }
        }

        fn play_tone(&self, duration_secs: f32, freq: f32, volume: f32) {
            let Some(handle) = &self.stream_handle else { return };
            let sample_rate = 44100u32;
            let num_samples = (sample_rate as f32 * duration_secs) as usize;
            let mut samples = Vec::with_capacity(num_samples);
            for i in 0..num_samples {
                let t = i as f32 / sample_rate as f32;
                let envelope = 1.0 - (t / duration_secs);
                let sample = (t * freq * 2.0 * std::f32::consts::PI).sin() * volume * envelope;
                samples.push(sample);
            }
            let source = rodio::buffer::SamplesBuffer::new(1, sample_rate, samples);
            let _ = handle.play_raw(source.convert_samples());
        }

        fn play_noise(&self, duration_secs: f32, volume: f32) {
            let Some(handle) = &self.stream_handle else { return };
            let sample_rate = 44100u32;
            let num_samples = (sample_rate as f32 * duration_secs) as usize;
            let mut samples = Vec::with_capacity(num_samples);
            let mut rng_state: u32 = 12345;
            for i in 0..num_samples {
                let t = i as f32 / sample_rate as f32;
                let envelope = 1.0 - (t / duration_secs);
                rng_state = rng_state.wrapping_mul(1103515245).wrapping_add(12345);
                let noise = (rng_state as f32 / u32::MAX as f32) * 2.0 - 1.0;
                samples.push(noise * volume * envelope);
            }
            let source = rodio::buffer::SamplesBuffer::new(1, sample_rate, samples);
            let _ = handle.play_raw(source.convert_samples());
        }
    }
}

// ─── WASM audio (silent stubs) ───────────────────────────────────────────────
#[cfg(target_arch = "wasm32")]
mod wasm {
    pub struct AudioEngine;

    impl AudioEngine {
        pub fn new() -> Self { Self }
        pub fn new_silent() -> Self { Self }
        pub fn play_break(&self) {}
        pub fn play_place(&self) {}
        pub fn play_explosion(&self) {}
        pub fn play_thunder(&self) {}
        pub fn play_gem_pickup(&self) {}
        pub fn play_genesis_block(&self) {}
        pub fn play_footstep(&mut self, _sprinting: bool) {}
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::AudioEngine;

#[cfg(target_arch = "wasm32")]
pub use wasm::AudioEngine;
