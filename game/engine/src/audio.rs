//! Audio engine — procedural sound effects for block interaction and movement.
//!
//! Seven synthesised sounds (no audio files exist, anywhere): block break,
//! block place, footstep, explosion, thunder, gem pickup, Genesis Block
//! fanfare. Each is a short recipe of tone / noise layers ([`layers`]) rendered
//! to mono PCM by the pure [`render_mix`] — the SAME samples on every target.
//!
//! - **Native** (incl. Android): rodio `play_raw` of the rendered buffer.
//! - **WASM**: Web Audio. The rendered PCM is copied into an `AudioBuffer`
//!   (cached per sound) and played through a master `GainNode`. Nothing is
//!   fetched and nothing is `include_bytes!`'d, so the bundle-size gate is
//!   untouched (code only). The `AudioContext` starts suspended under every
//!   browser's autoplay policy; [`AudioEngine::new`] registers a capture-phase
//!   `pointerdown` / `keydown` / `touchend` / `click` listener on `window` that
//!   calls `resume()`, and sounds requested before the context is running are
//!   dropped (never queued — a queue would burst out on unlock).
//!
//! Master volume + mute live in `GraphicsSettings` (per-device, persisted on
//! both targets) and reach the engine through [`AudioEngine::set_master`]
//! (world entry, the settings panel, the lobby settings panel).
//!
//! Spec 05 §11. Footsteps every 0.45 s walking, 0.35 s sprinting.

/// Sample rate every sound is rendered at (mono).
pub(crate) const SAMPLE_RATE: u32 = 44_100;

/// The seven sounds the game can request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sound {
    Break,
    Place,
    Footstep,
    Explosion,
    Thunder,
    GemPickup,
    GenesisBlock,
}

impl Sound {
    /// Every sound, in discriminant order.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) const ALL: [Sound; 7] = [
        Sound::Break,
        Sound::Place,
        Sound::Footstep,
        Sound::Explosion,
        Sound::Thunder,
        Sound::GemPickup,
        Sound::GenesisBlock,
    ];

    /// Dense index (cache slot on the web backend).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn index(self) -> usize {
        self as usize
    }
}

/// One synthesis layer of a sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Layer {
    /// Sine tone with a linear decay envelope, starting `delay` seconds in.
    Tone { secs: f32, freq: f32, volume: f32, delay: f32 },
    /// White noise (fixed-seed LCG) with a linear decay envelope.
    Noise { secs: f32, volume: f32 },
}

/// The recipe for each sound. Tunable at the Axolittle playtest.
pub(crate) fn layers(sound: Sound) -> &'static [Layer] {
    match sound {
        Sound::Break => &[Layer::Noise { secs: 0.08, volume: 0.3 }],
        Sound::Place => &[Layer::Tone { secs: 0.06, freq: 600.0, volume: 0.25, delay: 0.0 }],
        Sound::Footstep => &[Layer::Noise { secs: 0.04, volume: 0.15 }],
        // Spec 49 (Explosives) — the Blasting Keg "boom": a low-frequency body
        // tone under a longer, louder debris/blast noise burst.
        Sound::Explosion => &[
            Layer::Tone { secs: 0.45, freq: 70.0, volume: 0.45, delay: 0.0 },
            Layer::Noise { secs: 0.50, volume: 0.50 },
        ],
        // Thunder — a longer, deeper rumble than the keg boom (2026-07-05).
        Sound::Thunder => &[
            Layer::Tone { secs: 1.1, freq: 52.0, volume: 0.40, delay: 0.0 },
            Layer::Noise { secs: 1.4, volume: 0.30 },
        ],
        // Routine Satori pickup — small bright chime (A5, ~880 Hz).
        Sound::GemPickup => &[Layer::Tone { secs: 0.12, freq: 880.0, volume: 0.28, delay: 0.0 }],
        // Genesis Block fanfare — the first Satori found in a world. Ascending
        // major arpeggio C5 → E5 → G5 → C6, ~0.1 s apart.
        Sound::GenesisBlock => &[
            Layer::Tone { secs: 0.20, freq: 523.25, volume: 0.32, delay: 0.0 },
            Layer::Tone { secs: 0.20, freq: 659.25, volume: 0.32, delay: 0.10 },
            Layer::Tone { secs: 0.20, freq: 783.99, volume: 0.32, delay: 0.20 },
            Layer::Tone { secs: 0.20, freq: 1046.50, volume: 0.32, delay: 0.30 },
        ],
    }
}

/// Render one layer to mono PCM, every sample scaled by `gain`.
pub(crate) fn render_layer(layer: Layer, gain: f32) -> Vec<f32> {
    let rate = SAMPLE_RATE as f32;
    match layer {
        Layer::Tone { secs, freq, volume, delay } => {
            let delay_samples = (rate * delay) as usize;
            let num_samples = (rate * secs) as usize;
            let mut out = Vec::with_capacity(delay_samples + num_samples);
            out.resize(delay_samples, 0.0);
            for i in 0..num_samples {
                let t = i as f32 / rate;
                let envelope = 1.0 - (t / secs);
                out.push((t * freq * 2.0 * std::f32::consts::PI).sin() * volume * envelope * gain);
            }
            out
        }
        Layer::Noise { secs, volume } => {
            let num_samples = (rate * secs) as usize;
            let mut out = Vec::with_capacity(num_samples);
            let mut rng_state: u32 = 12345;
            for i in 0..num_samples {
                let t = i as f32 / rate;
                let envelope = 1.0 - (t / secs);
                rng_state = rng_state.wrapping_mul(1103515245).wrapping_add(12345);
                let noise = (rng_state as f32 / u32::MAX as f32) * 2.0 - 1.0;
                out.push(noise * volume * envelope * gain);
            }
            out
        }
    }
}

/// Render a whole sound: every layer summed, hard-limited to `[-1, 1]`.
pub(crate) fn render_mix(sound: Sound, gain: f32) -> Vec<f32> {
    let mut out: Vec<f32> = Vec::new();
    for layer in layers(sound) {
        let part = render_layer(*layer, gain);
        if part.len() > out.len() {
            out.resize(part.len(), 0.0);
        }
        for (o, v) in out.iter_mut().zip(part) {
            *o += v;
        }
    }
    for v in &mut out {
        *v = v.clamp(-1.0, 1.0);
    }
    out
}

/// Slider position (`0..=1`) + mute → linear amplitude gain. The slider is
/// squared so the lower half of the travel is usable (loudness is not linear
/// in amplitude); `1.0` leaves the sounds exactly as authored.
pub(crate) fn effective_gain(volume: f32, muted: bool) -> f32 {
    if muted || !volume.is_finite() {
        return 0.0;
    }
    let v = volume.clamp(0.0, 1.0);
    v * v
}

// ─── Native audio (rodio) ────────────────────────────────────────────────────
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{effective_gain, render_mix, Sound, SAMPLE_RATE};
    use rodio::Source;
    use std::time::{Duration, Instant};

    pub struct AudioEngine {
        _stream: Option<rodio::OutputStream>,
        stream_handle: Option<rodio::OutputStreamHandle>,
        last_footstep: Instant,
        /// Linear gain from the master volume / mute setting.
        gain: f32,
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
            Self { _stream: stream, stream_handle, last_footstep: Instant::now(), gain: 1.0 }
        }

        /// No caller — `new()` already falls back to silent internally on
        /// init failure, so an explicit silent constructor is redundant today.
        #[allow(dead_code)]
        pub fn new_silent() -> Self {
            Self { _stream: None, stream_handle: None, last_footstep: Instant::now(), gain: 1.0 }
        }

        /// Apply the persisted master volume (`0..=1`) and mute switch.
        pub fn set_master(&mut self, volume: f32, muted: bool) {
            self.gain = effective_gain(volume, muted);
        }

        pub fn play_break(&self) { self.play(Sound::Break); }
        pub fn play_place(&self) { self.play(Sound::Place); }
        /// Spec 49 (Explosives) — the Blasting Keg "boom".
        pub fn play_explosion(&self) { self.play(Sound::Explosion); }
        /// Thunder — fired by a lightning strike during a thunderstorm.
        pub fn play_thunder(&self) { self.play(Sound::Thunder); }
        /// Routine Satori pickup — small bright chime.
        pub fn play_gem_pickup(&self) { self.play(Sound::GemPickup); }
        /// Genesis Block fanfare — the *first* Satori found in this world.
        pub fn play_genesis_block(&self) { self.play(Sound::GenesisBlock); }

        pub fn play_footstep(&mut self, sprinting: bool) {
            let interval = if sprinting { Duration::from_millis(350) } else { Duration::from_millis(450) };
            if self.last_footstep.elapsed() >= interval {
                self.play(Sound::Footstep);
                self.last_footstep = Instant::now();
            }
        }

        fn play(&self, sound: Sound) {
            let Some(handle) = &self.stream_handle else { return };
            if self.gain <= 0.0 {
                return;
            }
            let samples = render_mix(sound, self.gain);
            let source = rodio::buffer::SamplesBuffer::new(1, SAMPLE_RATE, samples);
            let _ = handle.play_raw(source.convert_samples());
        }
    }
}

// ─── Web audio (Web Audio API) ───────────────────────────────────────────────
#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::{effective_gain, render_mix, Sound, SAMPLE_RATE};
    use std::cell::RefCell;
    use std::time::Duration;
    use wasm_bindgen::JsCast;
    use web_time::Instant;

    pub struct AudioEngine {
        ctx: Option<web_sys::AudioContext>,
        /// Master gain node every sound is routed through (volume + mute).
        master: Option<web_sys::GainNode>,
        /// Rendered `AudioBuffer`s, one slot per [`Sound`] (filled lazily).
        buffers: RefCell<Vec<Option<web_sys::AudioBuffer>>>,
        last_footstep: Instant,
        /// Linear gain from the master volume / mute setting.
        gain: f32,
    }

    /// Resume the context on the first user gesture (browser autoplay policy).
    /// Capture phase, on `window`, so nothing downstream can swallow the event.
    /// Calling `resume()` on an already-running context is a no-op, so the
    /// listeners are simply left in place.
    fn install_unlock_listeners(ctx: &web_sys::AudioContext) {
        let Some(window) = web_sys::window() else { return };
        let ctx = ctx.clone();
        let closure = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
            let _ = ctx.resume();
        });
        for event in ["pointerdown", "keydown", "touchend", "click"] {
            let _ = window.add_event_listener_with_callback_and_bool(
                event,
                closure.as_ref().unchecked_ref(),
                true,
            );
        }
        closure.forget();
    }

    impl AudioEngine {
        pub fn new() -> Self {
            let ctx = match web_sys::AudioContext::new() {
                Ok(c) => Some(c),
                Err(e) => {
                    log::warn!("Web audio init failed (will run silent): {e:?}");
                    None
                }
            };
            let master = ctx.as_ref().and_then(|c| {
                let g = c.create_gain().ok()?;
                g.connect_with_audio_node(&c.destination()).ok()?;
                Some(g)
            });
            if let Some(c) = &ctx {
                install_unlock_listeners(c);
            }
            Self {
                ctx,
                master,
                buffers: RefCell::new(vec![None; Sound::ALL.len()]),
                last_footstep: Instant::now(),
                gain: 1.0,
            }
        }

        #[allow(dead_code)]
        pub fn new_silent() -> Self {
            Self {
                ctx: None,
                master: None,
                buffers: RefCell::new(vec![None; Sound::ALL.len()]),
                last_footstep: Instant::now(),
                gain: 1.0,
            }
        }

        /// Apply the persisted master volume (`0..=1`) and mute switch.
        pub fn set_master(&mut self, volume: f32, muted: bool) {
            self.gain = effective_gain(volume, muted);
            if let Some(master) = &self.master {
                master.gain().set_value(self.gain);
            }
        }

        pub fn play_break(&self) { self.play(Sound::Break); }
        pub fn play_place(&self) { self.play(Sound::Place); }
        pub fn play_explosion(&self) { self.play(Sound::Explosion); }
        pub fn play_thunder(&self) { self.play(Sound::Thunder); }
        pub fn play_gem_pickup(&self) { self.play(Sound::GemPickup); }
        pub fn play_genesis_block(&self) { self.play(Sound::GenesisBlock); }

        pub fn play_footstep(&mut self, sprinting: bool) {
            let interval = if sprinting { Duration::from_millis(350) } else { Duration::from_millis(450) };
            if self.last_footstep.elapsed() >= interval {
                self.play(Sound::Footstep);
                self.last_footstep = Instant::now();
            }
        }

        /// The cached `AudioBuffer` for `sound`, rendering it on first use.
        fn buffer_for(&self, ctx: &web_sys::AudioContext, sound: Sound) -> Option<web_sys::AudioBuffer> {
            if let Some(Some(buf)) = self.buffers.borrow().get(sound.index()) {
                return Some(buf.clone());
            }
            // Rendered at unit gain: volume + mute are the master node's job.
            let samples = render_mix(sound, 1.0);
            if samples.is_empty() {
                return None;
            }
            let buf = ctx
                .create_buffer(1, samples.len() as u32, SAMPLE_RATE as f32)
                .ok()?;
            buf.copy_to_channel(&samples, 0).ok()?;
            self.buffers.borrow_mut()[sound.index()] = Some(buf.clone());
            Some(buf)
        }

        fn play(&self, sound: Sound) {
            let (Some(ctx), Some(master)) = (&self.ctx, &self.master) else { return };
            if self.gain <= 0.0 {
                return;
            }
            if ctx.state() != web_sys::AudioContextState::Running {
                // Still locked by the autoplay policy (no gesture yet) or
                // suspended: try to wake it, but drop THIS sound rather than
                // queue it, so nothing bursts out late.
                let _ = ctx.resume();
                return;
            }
            let Some(buffer) = self.buffer_for(ctx, sound) else { return };
            let Ok(source) = ctx.create_buffer_source() else { return };
            source.set_buffer(Some(&buffer));
            if source.connect_with_audio_node(master).is_err() {
                return;
            }
            let _ = source.start();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::AudioEngine;

#[cfg(target_arch = "wasm32")]
pub use wasm::AudioEngine;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_has_a_recipe_and_renders_audible_samples() {
        for s in Sound::ALL {
            assert!(!layers(s).is_empty(), "{s:?} has no layers");
            let pcm = render_mix(s, 1.0);
            assert!(!pcm.is_empty(), "{s:?} rendered nothing");
            assert!(pcm.iter().any(|v| v.abs() > 0.01), "{s:?} is silent");
            assert!(pcm.iter().all(|v| (-1.0..=1.0).contains(v)), "{s:?} clips");
        }
    }

    #[test]
    fn sound_indices_are_dense_and_unique() {
        for (i, s) in Sound::ALL.iter().enumerate() {
            assert_eq!(s.index(), i);
        }
    }

    #[test]
    fn gain_scales_the_samples_and_zero_silences() {
        let full = render_mix(Sound::Place, 1.0);
        let half = render_mix(Sound::Place, 0.5);
        let silent = render_mix(Sound::Place, 0.0);
        assert_eq!(full.len(), half.len());
        for (f, h) in full.iter().zip(&half) {
            assert!((f * 0.5 - h).abs() < 1e-6);
        }
        assert!(silent.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn genesis_fanfare_is_longer_than_its_note_because_of_the_delays() {
        let one = render_layer(Layer::Tone { secs: 0.20, freq: 523.25, volume: 0.32, delay: 0.0 }, 1.0);
        let all = render_mix(Sound::GenesisBlock, 1.0);
        assert!(all.len() > one.len());
        // The last note starts 0.30 s in and runs 0.20 s.
        assert_eq!(all.len(), (SAMPLE_RATE as f32 * 0.30) as usize + (SAMPLE_RATE as f32 * 0.20) as usize);
    }

    #[test]
    fn effective_gain_is_unity_at_full_zero_when_muted_and_squared_between() {
        assert_eq!(effective_gain(1.0, false), 1.0);
        assert_eq!(effective_gain(0.0, false), 0.0);
        assert_eq!(effective_gain(1.0, true), 0.0);
        assert!((effective_gain(0.5, false) - 0.25).abs() < 1e-6);
        // Out-of-range / junk input never amplifies or panics.
        assert_eq!(effective_gain(7.0, false), 1.0);
        assert_eq!(effective_gain(-3.0, false), 0.0);
        assert_eq!(effective_gain(f32::NAN, false), 0.0);
    }
}
