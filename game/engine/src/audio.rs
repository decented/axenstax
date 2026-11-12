//! Audio engine — procedural sound effects for block interaction and movement.
//!
//! Eleven synthesised sounds (no audio files exist, anywhere): block break,
//! block place, footstep, explosion, thunder, gem pickup, Genesis Block
//! fanfare, and the living world's ambience (three birdsong chirps, a cricket
//! trill). Each is a short recipe of tone / noise layers ([`layers`]) rendered
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

/// The sounds the game can request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sound {
    Break,
    Place,
    Footstep,
    Explosion,
    Thunder,
    GemPickup,
    GenesisBlock,
    /// Living-world ambience (spec 2026-11-08 §2.2): three short chirps...
    Birdsong,
    BirdsongTrill,
    BirdsongWhistle,
    /// ...and a soft cricket trill. Quiet by design.
    Insects,
}

impl Sound {
    /// Every sound, in discriminant order.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) const ALL: [Sound; 11] = [
        Sound::Break,
        Sound::Place,
        Sound::Footstep,
        Sound::Explosion,
        Sound::Thunder,
        Sound::GemPickup,
        Sound::GenesisBlock,
        Sound::Birdsong,
        Sound::BirdsongTrill,
        Sound::BirdsongWhistle,
        Sound::Insects,
    ];

    /// One of the three chirps, chosen by `seed` (so two birds sound different).
    pub(crate) fn birdsong_variant(seed: u64) -> Sound {
        // Mix the bits first: callers pass counters and positions.
        let h = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 33;
        match h % 3 {
            0 => Sound::Birdsong,
            1 => Sound::BirdsongTrill,
            _ => Sound::BirdsongWhistle,
        }
    }

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
    /// Sine tone that swells in from silence over `attack` seconds, then rolls
    /// off to nothing at `secs` (see [`swell_envelope`]). The thunder's body.
    SwellTone { secs: f32, freq: f32, volume: f32, attack: f32 },
    /// Low-passed ("brown-ish") noise with the same swell envelope: white
    /// noise through two one-pole low-pass filters at `cutoff` Hz, scaled back
    /// to the loudness of the white noise it came from so `volume` means the
    /// same thing as for [`Layer::Noise`]. Almost all its energy is rumble, so
    /// it has no hiss.
    RumbleNoise { secs: f32, volume: f32, attack: f32, cutoff: f32 },
}

/// Amplitude envelope of the thunder layers, `0..=1`: a smoothstep rise from
/// silence to the peak at `t = attack`, then a roll-off (`(1 - u)^1.5`) to
/// nothing at `t = secs`. Starts at exactly 0 so there is no click or jump.
pub(crate) fn swell_envelope(t: f32, attack: f32, secs: f32) -> f32 {
    if t <= 0.0 || t >= secs {
        return 0.0;
    }
    if t < attack {
        let x = t / attack;
        return x * x * (3.0 - 2.0 * x);
    }
    let u = ((t - attack) / (secs - attack)).clamp(0.0, 1.0);
    (1.0 - u).powf(1.5)
}

/// The thunder recipe (see [`layers`]).
pub(crate) const THUNDER_LAYERS: [Layer; 2] = [
    Layer::SwellTone { secs: 2.0, freq: 100.0, volume: 0.09, attack: 0.35 },
    Layer::RumbleNoise { secs: 2.0, volume: 0.15, attack: 0.30, cutoff: 400.0 },
];

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
        // Thunder — a rumble that swells in and rolls off, not a bang
        // (softened 2026-11-12 after the "jump scare" playtest report; Spec 05
        // §"Thunder sound"). The old recipe was a 52 Hz tone (0.40) plus WHITE
        // noise (0.30), both at full amplitude at t = 0, so it peaked at ~0.70
        // instantly and, on laptop speakers that cannot play 52 Hz, sounded
        // like a sudden hiss. Now: a 100 Hz tone (laptops can play it) under
        // low-passed noise (400 Hz), each rising smoothly from silence to a
        // peak ~0.3 s in and rolling off over ~2 s; peak <= 0.35. Distance
        // gain and the flash-first delay live in `thunder.rs`.
        Sound::Thunder => &THUNDER_LAYERS,
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
        // Living-world ambience. Tiny, high, quiet: a bird is a few short
        // notes, a cricket is a run of ticks. Linear decay envelopes make each
        // note a soft chirp rather than a beep.
        // Softer, lower and rarer since the v0.2.31 playtest ("a high-pitched
        // twinkly sound every few seconds"): 1.7-2.8 kHz at about half the old
        // volume, not 3-4.6 kHz. The cadence is `bloom::present::sound_interval`.
        Sound::Birdsong => &[
            Layer::Tone { secs: 0.06, freq: 2000.0, volume: 0.035, delay: 0.0 },
            Layer::Tone { secs: 0.07, freq: 2500.0, volume: 0.035, delay: 0.08 },
            Layer::Tone { secs: 0.09, freq: 2200.0, volume: 0.03, delay: 0.17 },
        ],
        Sound::BirdsongTrill => &[
            Layer::Tone { secs: 0.035, freq: 2800.0, volume: 0.03, delay: 0.00 },
            Layer::Tone { secs: 0.035, freq: 2400.0, volume: 0.03, delay: 0.05 },
            Layer::Tone { secs: 0.035, freq: 2800.0, volume: 0.03, delay: 0.10 },
            Layer::Tone { secs: 0.035, freq: 2400.0, volume: 0.03, delay: 0.15 },
            Layer::Tone { secs: 0.035, freq: 2800.0, volume: 0.03, delay: 0.20 },
        ],
        Sound::BirdsongWhistle => &[
            Layer::Tone { secs: 0.10, freq: 2200.0, volume: 0.03, delay: 0.00 },
            Layer::Tone { secs: 0.10, freq: 1900.0, volume: 0.03, delay: 0.09 },
            Layer::Tone { secs: 0.14, freq: 1700.0, volume: 0.025, delay: 0.18 },
        ],
        // Cricket trill: three short runs of four ticks.
        Sound::Insects => &[
            Layer::Tone { secs: 0.025, freq: 3600.0, volume: 0.025, delay: 0.0 },
            Layer::Tone { secs: 0.025, freq: 3600.0, volume: 0.025, delay: 0.035 },
            Layer::Tone { secs: 0.025, freq: 3600.0, volume: 0.025, delay: 0.07 },
            Layer::Tone { secs: 0.025, freq: 3600.0, volume: 0.025, delay: 0.105 },
            Layer::Tone { secs: 0.025, freq: 3600.0, volume: 0.025, delay: 0.2 },
            Layer::Tone { secs: 0.025, freq: 3600.0, volume: 0.025, delay: 0.235 },
            Layer::Tone { secs: 0.025, freq: 3600.0, volume: 0.025, delay: 0.27 },
            Layer::Tone { secs: 0.025, freq: 3600.0, volume: 0.025, delay: 0.305 },
            Layer::Tone { secs: 0.025, freq: 3600.0, volume: 0.025, delay: 0.4 },
            Layer::Tone { secs: 0.025, freq: 3600.0, volume: 0.025, delay: 0.435 },
            Layer::Tone { secs: 0.025, freq: 3600.0, volume: 0.025, delay: 0.47 },
            Layer::Tone { secs: 0.025, freq: 3600.0, volume: 0.025, delay: 0.505 },
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
        Layer::SwellTone { secs, freq, volume, attack } => {
            let num_samples = (rate * secs) as usize;
            (0..num_samples)
                .map(|i| {
                    let t = i as f32 / rate;
                    (t * freq * 2.0 * std::f32::consts::PI).sin()
                        * volume
                        * swell_envelope(t, attack, secs)
                        * gain
                })
                .collect()
        }
        Layer::RumbleNoise { secs, volume, attack, cutoff } => {
            let num_samples = (rate * secs) as usize;
            // Same fixed-seed white noise as `Layer::Noise`, through two
            // one-pole low-pass filters.
            let a = 1.0 - (-2.0 * std::f32::consts::PI * cutoff / rate).exp();
            let (mut p1, mut p2) = (0.0_f32, 0.0_f32);
            let mut rng_state: u32 = 12345;
            let mut low: Vec<f32> = Vec::with_capacity(num_samples);
            for _ in 0..num_samples {
                rng_state = rng_state.wrapping_mul(1103515245).wrapping_add(12345);
                let white = (rng_state as f32 / u32::MAX as f32) * 2.0 - 1.0;
                p1 += a * (white - p1);
                p2 += a * (p1 - p2);
                low.push(p2);
            }
            // Filtering drains the level; put it back to white noise's RMS
            // (1/sqrt(3) for a uniform +-1 source) so `volume` is comparable.
            let rms = (low.iter().map(|v| v * v).sum::<f32>() / low.len().max(1) as f32).sqrt();
            let norm = if rms > 0.0 { 0.577_350_3 / rms } else { 0.0 };
            low.iter()
                .enumerate()
                .map(|(i, v)| v * norm * volume * swell_envelope(i as f32 / rate, attack, secs) * gain)
                .collect()
        }
    }
}

/// Render a whole sound: every layer summed, hard-limited to `[-1, 1]`.
pub(crate) fn render_mix(sound: Sound, gain: f32) -> Vec<f32> {
    render_layers(layers(sound), gain)
}

/// Render any stack of layers, summed and hard-limited (also used by the
/// thunder tests to render the retired recipe for comparison).
pub(crate) fn render_layers(stack: &[Layer], gain: f32) -> Vec<f32> {
    let mut out: Vec<f32> = Vec::new();
    for layer in stack {
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
        /// `gain` is the per-strike factor (distance fall-off times the
        /// Thunder slider, `thunder.rs`); it multiplies the master volume.
        pub fn play_thunder(&self, gain: f32) { self.play_scaled(Sound::Thunder, gain); }
        /// Routine Satori pickup — small bright chime.
        pub fn play_gem_pickup(&self) { self.play(Sound::GemPickup); }
        /// Genesis Block fanfare — the *first* Satori found in this world.
        pub fn play_genesis_block(&self) { self.play(Sound::GenesisBlock); }
        /// Living-world ambience: a bird (one of three chirps, by `seed`).
        pub fn play_birdsong(&self, seed: u64) { self.play(Sound::birdsong_variant(seed)); }
        /// Living-world ambience: a soft cricket trill.
        pub fn play_insects(&self) { self.play(Sound::Insects); }

        pub fn play_footstep(&mut self, sprinting: bool) {
            let interval = if sprinting { Duration::from_millis(350) } else { Duration::from_millis(450) };
            if self.last_footstep.elapsed() >= interval {
                self.play(Sound::Footstep);
                self.last_footstep = Instant::now();
            }
        }

        fn play(&self, sound: Sound) {
            self.play_scaled(sound, 1.0);
        }

        /// Play `sound` at the master gain times `scale` (`0..=1`).
        fn play_scaled(&self, sound: Sound, scale: f32) {
            let Some(handle) = &self.stream_handle else { return };
            let gain = self.gain * scale.clamp(0.0, 1.0);
            if gain <= 0.0 || !gain.is_finite() {
                return;
            }
            let samples = render_mix(sound, gain);
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
        /// Thunder at `gain` (distance fall-off times the Thunder slider,
        /// `thunder.rs`), on top of the master node. The buffer is the same
        /// synthesised, cached one as on native; the per-play gain is a
        /// throwaway `GainNode` between the source and the master.
        pub fn play_thunder(&self, gain: f32) { self.play_scaled(Sound::Thunder, gain); }
        pub fn play_gem_pickup(&self) { self.play(Sound::GemPickup); }
        pub fn play_genesis_block(&self) { self.play(Sound::GenesisBlock); }
        /// Living-world ambience: a bird (one of three chirps, by `seed`).
        pub fn play_birdsong(&self, seed: u64) { self.play(Sound::birdsong_variant(seed)); }
        /// Living-world ambience: a soft cricket trill.
        pub fn play_insects(&self) { self.play(Sound::Insects); }

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
            self.play_scaled(sound, 1.0);
        }

        /// Play `sound` through a per-play `GainNode` of `scale` (`0..=1`)
        /// feeding the master node (so volume and mute still apply).
        fn play_scaled(&self, sound: Sound, scale: f32) {
            let (Some(ctx), Some(master)) = (&self.ctx, &self.master) else { return };
            let scale = scale.clamp(0.0, 1.0);
            if self.gain <= 0.0 || scale <= 0.0 || !scale.is_finite() {
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
            if scale >= 1.0 {
                if source.connect_with_audio_node(master).is_err() {
                    return;
                }
            } else {
                let Ok(per_play) = ctx.create_gain() else { return };
                per_play.gain().set_value(scale);
                if source.connect_with_audio_node(&per_play).is_err()
                    || per_play.connect_with_audio_node(master).is_err()
                {
                    return;
                }
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

    #[test]
    fn ambience_is_quiet_and_birdsong_has_three_distinct_variants() {
        for s in [Sound::Birdsong, Sound::BirdsongTrill, Sound::BirdsongWhistle, Sound::Insects] {
            let peak = render_mix(s, 1.0).iter().fold(0.0_f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.02 && peak <= 0.06, "{s:?} peak {peak}: ambience stays soft");
        }
        let seen: std::collections::BTreeSet<usize> =
            (0..64u64).map(|seed| Sound::birdsong_variant(seed).index()).collect();
        assert_eq!(seen.len(), 3, "all three chirps get chosen");
        assert_eq!(Sound::birdsong_variant(9), Sound::birdsong_variant(9), "by seed");
    }

    // ── Thunder: the softened recipe against the retired one ────────────────

    /// The recipe before 2026-11-12, kept here only so the tests can prove the
    /// new one is softer: a 52 Hz tone and WHITE noise, both at full amplitude
    /// at t = 0 (peak ~0.70, so "a sudden hiss" on laptop speakers).
    const OLD_THUNDER: [Layer; 2] = [
        Layer::Tone { secs: 1.1, freq: 52.0, volume: 0.40, delay: 0.0 },
        Layer::Noise { secs: 1.4, volume: 0.30 },
    ];

    /// One RBJ biquad section (Butterworth Q) — the tests' band filters. Used
    /// instead of an FFT because the crate has no FFT dependency and these run
    /// in O(n) even in a debug build.
    fn biquad(x: &[f32], f0: f32, high_pass: bool) -> Vec<f32> {
        let w0 = 2.0 * std::f32::consts::PI * f0 / SAMPLE_RATE as f32;
        let alpha = w0.sin() / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let cw = w0.cos();
        let (b0, b1, b2) = if high_pass {
            ((1.0 + cw) / 2.0, -(1.0 + cw), (1.0 + cw) / 2.0)
        } else {
            ((1.0 - cw) / 2.0, 1.0 - cw, (1.0 - cw) / 2.0)
        };
        let (a0, a1, a2) = (1.0 + alpha, -2.0 * cw, 1.0 - alpha);
        let (b0, b1, b2, a1, a2) = (b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0);
        let (mut x1, mut x2, mut y1, mut y2) = (0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32);
        x.iter()
            .map(|&xn| {
                let y = b0 * xn + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
                x2 = x1;
                x1 = xn;
                y2 = y1;
                y1 = y;
                y
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    /// RMS of `x` between `lo` and `hi` Hz (two cascaded sections each side).
    fn band_rms(x: &[f32], lo: f32, hi: f32) -> f32 {
        let hp = biquad(&biquad(x, lo, true), lo, true);
        rms(&biquad(&biquad(&hp, hi, false), hi, false))
    }

    /// Energy above 2 kHz: the RMS of a 4th-order high-pass at 2 kHz, summed
    /// over the whole buffer (so a longer or louder hiss counts for more).
    fn hf_energy(x: &[f32]) -> f32 {
        let hp = biquad(&biquad(x, 2000.0, true), 2000.0, true);
        hp.iter().map(|v| v * v).sum::<f32>().sqrt()
    }

    /// `(time of 50% of peak, time of peak)` in seconds, from the RMS
    /// envelope over 20 ms windows (the raw samples oscillate, so they cannot
    /// say when the sound "gets loud").
    fn attack_times(x: &[f32]) -> (f32, f32) {
        let w = (SAMPLE_RATE as f32 * 0.02) as usize;
        let env: Vec<f32> = (0..x.len().saturating_sub(w)).step_by(w / 4).map(|i| rms(&x[i..i + w])).collect();
        let step = (w / 4) as f32 / SAMPLE_RATE as f32;
        let win = w as f32 / SAMPLE_RATE as f32;
        let peak = env.iter().cloned().fold(0.0_f32, f32::max);
        let half = env.iter().position(|e| *e >= 0.5 * peak).unwrap();
        let top = env.iter().position(|e| *e >= peak).unwrap();
        // Window centre.
        (half as f32 * step + win / 2.0, top as f32 * step + win / 2.0)
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0_f32, |m, v| m.max(v.abs()))
    }

    /// Everything the thunder tests and the evidence file measure.
    struct ThunderNumbers {
        peak: f32,
        half_at: f32,
        peak_at: f32,
        hf: f32,
        hf_share: f32,
        mid_rms: f32,
        secs: f32,
    }

    fn numbers(x: &[f32]) -> ThunderNumbers {
        let (half_at, peak_at) = attack_times(x);
        let total = x.iter().map(|v| v * v).sum::<f32>().sqrt();
        let hf = hf_energy(x);
        ThunderNumbers {
            peak: peak(x),
            half_at,
            peak_at,
            hf,
            hf_share: hf / total,
            mid_rms: band_rms(x, 150.0, 1000.0),
            secs: x.len() as f32 / SAMPLE_RATE as f32,
        }
    }

    /// Thunder as heard from `d` blocks with the Thunder slider at 100% and the
    /// master volume at 100%: the recipe rendered at the distance gain.
    fn thunder_at(d: f32) -> Vec<f32> {
        render_mix(Sound::Thunder, crate::thunder::gain(d) * effective_gain(1.0, false))
    }

    #[test]
    fn thunder_is_a_swell_that_starts_from_silence_and_rolls_off_in_about_two_seconds() {
        let new = render_mix(Sound::Thunder, 1.0);
        assert!(new[0].abs() < 1e-4, "starts at zero: {}", new[0]);
        // The first 5 ms (a hard start would already be at full level).
        assert!(peak(&new[..220]) < 0.01, "no jump in the first 5 ms: {}", peak(&new[..220]));
        let n = numbers(&new);
        assert!((1.9..=2.1).contains(&n.secs), "about 2 s long, was {} s", n.secs);
        assert!(n.half_at >= 0.150, "50% of peak no earlier than 150 ms, was {} s", n.half_at);
        assert!((0.25..=0.40).contains(&n.peak_at), "peak 250-400 ms in, was {} s", n.peak_at);
        // The retired recipe was at full level from the first window.
        let old = numbers(&render_layers(&OLD_THUNDER, 1.0));
        assert!(old.half_at < 0.05, "old recipe: 50% at {} s", old.half_at);
        assert!(old.half_at < 0.150, "this test fails on the old recipe");
    }

    #[test]
    fn thunder_peaks_at_most_035_where_the_old_recipe_peaked_near_07() {
        let new = peak(&render_mix(Sound::Thunder, 1.0));
        let old = peak(&render_layers(&OLD_THUNDER, 1.0));
        assert!(new <= 0.35, "new peak {new}");
        assert!(new > 0.20, "but not inaudibly quiet: {new}");
        assert!(old > 0.35, "the old recipe breaks the ceiling (peak {old}), so this test would have failed on it");
        // The slider and the distance only ever lower it.
        assert!(peak(&thunder_at(3.0)) <= 0.35);
    }

    #[test]
    fn thunder_has_less_than_half_the_old_high_frequency_content() {
        let new = numbers(&render_mix(Sound::Thunder, 1.0));
        let old = numbers(&render_layers(&OLD_THUNDER, 1.0));
        // Measured two ways: the energy above 2 kHz (high-pass), and its share
        // of the whole sound (so "quieter overall" cannot pass on its own).
        assert!(new.hf <= 0.5 * old.hf, "HF energy above 2 kHz: new {} vs old {}", new.hf, old.hf);
        assert!(
            new.hf_share <= 0.5 * old.hf_share,
            "HF share of the sound: new {} vs old {}",
            new.hf_share,
            old.hf_share
        );
    }

    #[test]
    fn thunder_stays_audible_on_laptop_speakers_at_30_blocks() {
        // A floor to go with the ceilings: 150 Hz to 1 kHz is what laptop
        // speakers actually reproduce. The new sound has MORE energy there at 3
        // blocks than the old one, and at 30 blocks it is still well above
        // inaudible (-40 dBFS RMS is 0.01).
        let near = band_rms(&thunder_at(3.0), 150.0, 1000.0);
        let far = band_rms(&thunder_at(30.0), 150.0, 1000.0);
        let old = band_rms(&render_layers(&OLD_THUNDER, 1.0), 150.0, 1000.0);
        assert!(far >= 0.010, "mid-band RMS at 30 blocks {far}");
        assert!(near > old, "mid-band RMS at 3 blocks {near} vs old {old}");
        assert!(far < near, "farther is quieter");
        assert!(far / near <= 0.45 + 1e-3, "{}", far / near);
    }

    /// Not a check: writes the before/after WAVs and numbers.md that go with
    /// the change. Run with
    /// `THUNDER_EVIDENCE_DIR=<dir> cargo test --lib write_thunder_evidence -- --ignored`.
    #[test]
    #[ignore]
    fn write_thunder_evidence() {
        let Ok(dir) = std::env::var("THUNDER_EVIDENCE_DIR") else { return };
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir).unwrap();
        fn wav(path: &std::path::Path, x: &[f32]) {
            let data: Vec<u8> = x.iter().flat_map(|v| ((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
            let mut out = Vec::new();
            out.extend(b"RIFF");
            out.extend((36 + data.len() as u32).to_le_bytes());
            out.extend(b"WAVEfmt ");
            out.extend(16u32.to_le_bytes());
            out.extend(1u16.to_le_bytes());
            out.extend(1u16.to_le_bytes());
            out.extend(SAMPLE_RATE.to_le_bytes());
            out.extend((SAMPLE_RATE * 2).to_le_bytes());
            out.extend(2u16.to_le_bytes());
            out.extend(16u16.to_le_bytes());
            out.extend(b"data");
            out.extend((data.len() as u32).to_le_bytes());
            out.extend(data);
            std::fs::write(path, out).unwrap();
        }
        let old = render_layers(&OLD_THUNDER, 1.0);
        let cases: [(&str, &str, Vec<f32>); 3] = [
            ("thunder-old.wav", "old recipe (any distance)", old.clone()),
            ("thunder-new-d3.wav", "new recipe, d = 3 blocks (gain 1.00)", thunder_at(3.0)),
            ("thunder-new-d30.wav", "new recipe, d = 30 blocks (gain 0.40)", thunder_at(30.0)),
        ];
        let o = numbers(&old);
        let mut md = String::from(
            "# Thunder: before and after (2026-11-12)\n\n\
             All sounds rendered at master volume 100%, Thunder slider 100%. Measured on the rendered samples, 44.1 kHz mono.\n\n\
             | sound | peak | 50% of peak at (s) | peak at (s) | HF energy >2 kHz | HF share | mid-band RMS 150 Hz-1 kHz | length (s) |\n\
             |---|---|---|---|---|---|---|---|\n",
        );
        for (file, label, x) in &cases {
            wav(&dir.join(file), x);
            let n = numbers(x);
            md.push_str(&format!(
                "| {label} (`{file}`) | {:.3} | {:.3} | {:.3} | {:.3} | {:.4} | {:.4} | {:.2} |\n",
                n.peak, n.half_at, n.peak_at, n.hf, n.hf_share, n.mid_rms, n.secs
            ));
        }
        let n3 = numbers(&thunder_at(3.0));
        md.push_str(&format!(
            "\nHF energy new(d=3) / old: {:.3}. HF share new / old: {:.3}.\n\n\
             Method: HF energy is the L2 norm of the signal after a 4th-order Butterworth high-pass at 2 kHz, over the whole buffer; HF share is that divided by the L2 norm of the unfiltered signal. \
             Mid-band RMS is the RMS after a 4th-order 150 Hz high-pass and 4th-order 1 kHz low-pass. Attack times come from a 20 ms-window RMS envelope. \
             Biquads rather than an FFT because the crate has no FFT dependency.\n\n\
             Delay before the rumble (not physical, so the flash warns first): d = 3 blocks {:.2} s ({} ticks), d = 30 blocks {:.2} s ({} ticks), d = 34 blocks {:.2} s ({} ticks).\n",
            n3.hf / o.hf,
            n3.hf_share / o.hf_share,
            crate::thunder::delay_secs(3.0),
            crate::thunder::delay_ticks(3.0),
            crate::thunder::delay_secs(30.0),
            crate::thunder::delay_ticks(30.0),
            crate::thunder::delay_secs(34.0),
            crate::thunder::delay_ticks(34.0),
        ));
        std::fs::write(dir.join("numbers.md"), md).unwrap();
    }
}
