//! Particle framework (2026-07-05) — the "grows a particle framework" that
//! three systems were explicitly waiting on (explosion.rs, block_interact.rs,
//! hud_ui.rs rain).
//!
//! CPU-simulated pool → GPU-instanced camera-facing billboards. STRICTLY
//! client-side visual: no protocol, no save, no server involvement — the sim
//! runs per-FRAME with real dt (not the 20 TPS tick), capacity is gated by
//! `GraphicsSettings.particles` (Off / Reduced / Full), and everything is
//! deterministic (position+seed hashes, no RNG — house discipline).
//!
//! Feel knobs live in the constants below — Axo playtest tunes them.

use glam::Vec3;

use crate::block;

/// Pool capacity per graphics level. Off disables the system entirely (the
/// legacy 2D rain streak overlay stays in that mode).
pub const CAP_REDUCED: usize = 512;
pub const CAP_FULL: usize = 2048;

/// Gravity for heavy particles (chips, embers, splash), blocks/s².
const GRAVITY: f32 = 18.0;
/// Air drag per second (velocity multiplier ≈ drag^dt).
const DRAG_PER_SEC: f32 = 0.55;
/// Rain fall speed, blocks/s.
const RAIN_FALL: f32 = 22.0;
/// Snow fall speed, blocks/s.
const SNOW_FALL: f32 = 1.6;

/// Murmur-style avalanche (the growth.rs lesson: plain xors keep parity).
fn hash(seed: u64) -> u64 {
    let mut h = seed;
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^ (h >> 33)
}

/// Deterministic float in [-1, 1) from (seed, lane).
fn jitter(seed: u64, lane: u64) -> f32 {
    let h = hash(seed.wrapping_add(lane.wrapping_mul(0x9E37_79B9_7F4A_7C15)));
    ((h & 0xFFFF) as f32 / 32768.0) - 1.0
}

/// Deterministic float in [0, 1).
fn unit(seed: u64, lane: u64) -> f32 {
    (jitter(seed, lane) + 1.0) * 0.5
}

#[derive(Clone, Copy, Debug)]
pub struct Particle {
    pub pos: Vec3,
    pub vel: Vec3,
    /// Seconds remaining.
    pub life: f32,
    /// Starting life (drives the fade curve).
    pub max_life: f32,
    /// World-space half-extent of the billboard.
    pub size: f32,
    pub color: [f32; 4],
    pub tex_layer: u32,
    /// Fraction of [`GRAVITY`] applied (0 = floats, 1 = falls hard).
    pub gravity: f32,
    /// 0 = no drag, 1 = full [`DRAG_PER_SEC`].
    pub drag: f32,
    /// Dies on crossing below this Y (rain hits a roof / the ground).
    pub kill_y: f32,
    /// Smoke-style growth: size multiplier per second (0 = constant size).
    pub grow: f32,
}

pub struct ParticleSystem {
    pool: Vec<Particle>,
    cap: usize,
}

impl Default for ParticleSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl ParticleSystem {
    pub fn new() -> Self {
        Self { pool: Vec::new(), cap: CAP_FULL }
    }

    /// Apply the graphics setting. Off clears the pool immediately.
    pub fn set_cap(&mut self, level: crate::graphics_settings::ParticleLevel) {
        use crate::graphics_settings::ParticleLevel as L;
        self.cap = match level {
            L::Off => 0,
            L::Reduced => CAP_REDUCED,
            L::Full => CAP_FULL,
        };
        if self.pool.len() > self.cap {
            self.pool.truncate(self.cap);
        }
    }

    /// No production caller — tested directly.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn live(&self) -> usize {
        self.pool.len()
    }

    pub fn is_enabled(&self) -> bool {
        self.cap > 0
    }

    /// True when ambient emitters should halve their rates (Reduced tier).
    pub fn is_reduced(&self) -> bool {
        self.cap == CAP_REDUCED
    }

    pub fn clear(&mut self) {
        self.pool.clear();
    }

    fn spawn(&mut self, p: Particle) {
        if self.pool.len() < self.cap {
            self.pool.push(p);
        }
    }

    // ── Burst emitters (deterministic per seed) ────────────────────────

    /// Block-break chips: block-coloured squares thrown out and down.
    pub fn burst_chips(&mut self, pos: Vec3, color: [f32; 3], n: usize, seed: u64) {
        for i in 0..n as u64 {
            self.spawn(Particle {
                pos: pos + Vec3::new(jitter(seed, i * 3) * 0.3, unit(seed, i * 3 + 1) * 0.4, jitter(seed, i * 3 + 2) * 0.3),
                vel: Vec3::new(jitter(seed, i * 5) * 2.5, 2.0 + unit(seed, i * 5 + 1) * 2.5, jitter(seed, i * 5 + 2) * 2.5),
                life: 0.4 + unit(seed, i * 7) * 0.4,
                max_life: 0.8,
                size: 0.05 + unit(seed, i * 11) * 0.04,
                color: [color[0], color[1], color[2], 1.0],
                tex_layer: block::TEX_PARTICLE_CHIP,
                gravity: 1.0,
                drag: 0.4,
                kill_y: f32::MIN,
                grow: 0.0,
            });
        }
    }

    /// Block-break chips sampling the block's OWN texture (mini block faces,
    /// reads better than a flat tint at 16 px). Explosion debris keeps the
    /// flat-colour variant.
    pub fn burst_chips_textured(&mut self, pos: Vec3, tex_layer: u32, n: usize, seed: u64) {
        for i in 0..n as u64 {
            self.spawn(Particle {
                pos: pos + Vec3::new(jitter(seed, i * 3) * 0.3, unit(seed, i * 3 + 1) * 0.4, jitter(seed, i * 3 + 2) * 0.3),
                vel: Vec3::new(jitter(seed, i * 5) * 2.5, 2.0 + unit(seed, i * 5 + 1) * 2.5, jitter(seed, i * 5 + 2) * 2.5),
                life: 0.4 + unit(seed, i * 7) * 0.4,
                max_life: 0.8,
                size: 0.05 + unit(seed, i * 11) * 0.04,
                color: [1.0, 1.0, 1.0, 1.0],
                tex_layer,
                gravity: 1.0,
                drag: 0.4,
                kill_y: f32::MIN,
                grow: 0.0,
            });
        }
    }

    /// Lightning bolt: a bright white-blue column of sparks from the strike
    /// point up into the sky, gone in a blink (the flash carries the drama).
    pub fn bolt(&mut self, ground: Vec3, height: f32, seed: u64) {
        let steps = height as u64;
        for i in 0..steps {
            let sway = jitter(seed, i) * 0.35;
            self.spawn(Particle {
                pos: ground + Vec3::new(sway, i as f32, jitter(seed, i + 97) * 0.35),
                vel: Vec3::ZERO,
                life: 0.18 + unit(seed, i + 41) * 0.08,
                max_life: 0.26,
                size: 0.16,
                color: [0.92, 0.95, 1.0, 1.0],
                tex_layer: block::TEX_PARTICLE_SPARK,
                gravity: 0.0,
                drag: 0.0,
                kill_y: f32::MIN,
                grow: 0.0,
            });
        }
    }

    /// Rising, growing, fading smoke.
    pub fn burst_smoke(&mut self, pos: Vec3, n: usize, seed: u64) {
        for i in 0..n as u64 {
            self.spawn(Particle {
                pos: pos + Vec3::new(jitter(seed, i * 3) * 0.25, unit(seed, i * 3 + 1) * 0.2, jitter(seed, i * 3 + 2) * 0.25),
                vel: Vec3::new(jitter(seed, i * 5) * 0.3, 0.8 + unit(seed, i * 5 + 1) * 0.6, jitter(seed, i * 5 + 2) * 0.3),
                life: 1.5 + unit(seed, i * 7),
                max_life: 2.5,
                size: 0.10 + unit(seed, i * 11) * 0.06,
                color: [0.25, 0.24, 0.23, 0.55],
                tex_layer: block::TEX_PARTICLE_SOFT,
                gravity: 0.0,
                drag: 0.15,
                kill_y: f32::MIN,
                grow: 0.35,
            });
        }
    }

    /// Fire embers — bright sparks that pop up and rain back down.
    pub fn burst_embers(&mut self, pos: Vec3, n: usize, seed: u64) {
        for i in 0..n as u64 {
            self.spawn(Particle {
                pos: pos + Vec3::new(jitter(seed, i * 3) * 0.3, 0.2, jitter(seed, i * 3 + 2) * 0.3),
                vel: Vec3::new(jitter(seed, i * 5) * 1.2, 2.5 + unit(seed, i * 5 + 1) * 2.0, jitter(seed, i * 5 + 2) * 1.2),
                life: 0.5 + unit(seed, i * 7) * 0.5,
                max_life: 1.0,
                size: 0.03,
                color: [1.0, 0.62, 0.18, 1.0],
                tex_layer: block::TEX_PARTICLE_SPARK,
                gravity: 0.55,
                drag: 0.2,
                kill_y: f32::MIN,
                grow: 0.0,
            });
        }
    }

    /// Water splash — white-blue droplets out and up.
    pub fn burst_splash(&mut self, pos: Vec3, n: usize, seed: u64) {
        for i in 0..n as u64 {
            self.spawn(Particle {
                pos: pos + Vec3::new(jitter(seed, i * 3) * 0.35, 0.05, jitter(seed, i * 3 + 2) * 0.35),
                vel: Vec3::new(jitter(seed, i * 5) * 2.0, 2.2 + unit(seed, i * 5 + 1) * 1.8, jitter(seed, i * 5 + 2) * 2.0),
                life: 0.35 + unit(seed, i * 7) * 0.25,
                max_life: 0.6,
                size: 0.045,
                color: [0.75, 0.85, 1.0, 0.9],
                tex_layer: block::TEX_PARTICLE_SOFT,
                gravity: 1.0,
                drag: 0.25,
                kill_y: pos.y - 0.3,
                grow: 0.0,
            });
        }
    }

    /// Small directional puff (dispenser muzzle, arrow loose).
    pub fn puff(&mut self, pos: Vec3, dir: Vec3, n: usize, seed: u64) {
        for i in 0..n as u64 {
            self.spawn(Particle {
                pos,
                vel: dir * (1.5 + unit(seed, i * 5) * 1.0)
                    + Vec3::new(jitter(seed, i * 3) * 0.5, jitter(seed, i * 3 + 1) * 0.5, jitter(seed, i * 3 + 2) * 0.5),
                life: 0.25 + unit(seed, i * 7) * 0.15,
                max_life: 0.4,
                size: 0.06,
                color: [0.8, 0.8, 0.8, 0.6],
                tex_layer: block::TEX_PARTICLE_SOFT,
                gravity: 0.0,
                drag: 0.6,
                kill_y: f32::MIN,
                grow: 0.4,
            });
        }
    }

    /// Sapling-growth poof — fresh green motes.
    pub fn poof_green(&mut self, pos: Vec3, n: usize, seed: u64) {
        for i in 0..n as u64 {
            self.spawn(Particle {
                pos: pos + Vec3::new(jitter(seed, i * 3) * 0.4, unit(seed, i * 3 + 1) * 0.8, jitter(seed, i * 3 + 2) * 0.4),
                vel: Vec3::new(jitter(seed, i * 5) * 0.6, 0.5 + unit(seed, i * 5 + 1) * 0.8, jitter(seed, i * 5 + 2) * 0.6),
                life: 0.6 + unit(seed, i * 7) * 0.5,
                max_life: 1.1,
                size: 0.05,
                color: [0.45, 0.85, 0.35, 0.9],
                tex_layer: block::TEX_PARTICLE_SOFT,
                gravity: 0.05,
                drag: 0.3,
                kill_y: f32::MIN,
                grow: 0.1,
            });
        }
    }

    /// Bee trail mote: a tiny golden speck that hangs briefly where the bee
    /// just was.
    pub fn bee_mote(&mut self, pos: Vec3, seed: u64) {
        self.spawn(Particle {
            pos: pos + Vec3::new(jitter(seed, 1) * 0.1, jitter(seed, 2) * 0.1, jitter(seed, 3) * 0.1),
            vel: Vec3::new(0.0, -0.15, 0.0),
            life: 0.5 + unit(seed, 4) * 0.3,
            max_life: 0.8,
            size: 0.025,
            color: [1.0, 0.85, 0.35, 0.8],
            tex_layer: block::TEX_PARTICLE_SOFT,
            gravity: 0.0,
            drag: 0.0,
            kill_y: f32::MIN,
            grow: 0.0,
        });
    }

    /// One rain streak: spawned high, falls fast, dies at `kill_y` (the first
    /// solid surface below the spawn point — roofs shelter).
    pub fn rain_streak(&mut self, spawn: Vec3, kill_y: f32) {
        self.spawn(Particle {
            pos: spawn,
            vel: Vec3::new(0.0, -RAIN_FALL, 0.0),
            life: 3.0,
            max_life: 3.0,
            size: 0.09,
            color: [0.62, 0.72, 0.92, 0.55],
            tex_layer: block::TEX_PARTICLE_STREAK,
            gravity: 0.0,
            drag: 0.0,
            kill_y,
            grow: 0.0,
        });
    }

    /// One drifting snowflake (ambient tundra/mountain weather).
    pub fn snow_flake(&mut self, spawn: Vec3, kill_y: f32, seed: u64) {
        self.spawn(Particle {
            pos: spawn,
            vel: Vec3::new(jitter(seed, 1) * 0.4, -SNOW_FALL - unit(seed, 2) * 0.6, jitter(seed, 3) * 0.4),
            life: 14.0,
            max_life: 14.0,
            size: 0.035,
            color: [0.95, 0.96, 1.0, 0.85],
            tex_layer: block::TEX_PARTICLE_SOFT,
            gravity: 0.0,
            drag: 0.0,
            kill_y,
            grow: 0.0,
        });
    }

    // ── Simulation + instance build ────────────────────────────────────

    /// Advance the pool by `dt` seconds (call per frame; clamp dt upstream).
    /// `solid_at` kills particles that fly into solid geometry — chips stop at
    /// walls, smoke dies at ceilings (previously only the `kill_y` floor plane
    /// existed). Pass `|_, _, _| false` for a free-space sim (tests).
    pub fn tick(&mut self, dt: f32, solid_at: impl Fn(i32, i32, i32) -> bool) {
        let mut i = 0;
        while i < self.pool.len() {
            let p = &mut self.pool[i];
            p.life -= dt;
            p.vel.y -= GRAVITY * p.gravity * dt;
            let drag = 1.0 - (1.0 - DRAG_PER_SEC) * p.drag * dt;
            p.vel *= drag.clamp(0.0, 1.0);
            p.pos += p.vel * dt;
            if p.grow > 0.0 {
                p.size += p.size * p.grow * dt;
            }
            let dead = p.life <= 0.0
                || p.pos.y < p.kill_y
                || solid_at(
                    p.pos.x.floor() as i32,
                    p.pos.y.floor() as i32,
                    p.pos.z.floor() as i32,
                );
            if dead {
                self.pool.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Build the GPU instances for every live particle. Alpha fades out over
    /// the final 40% of life so nothing pops. `light_at` returns the local
    /// effective light 0..=15 — particles in a dark cave dim instead of
    /// glowing (sparks/embers are exempt: they ARE light sources visually).
    /// Pass `|_, _, _| 15` for full-bright (tests).
    pub fn instances(
        &self,
        light_at: impl Fn(i32, i32, i32) -> u8,
        out: &mut Vec<crate::mesh::ParticleInstance>,
    ) {
        out.clear();
        for p in &self.pool {
            let frac = (p.life / p.max_life).clamp(0.0, 1.0);
            let alpha = p.color[3] * (frac / 0.4).min(1.0);
            let lit = if p.tex_layer == block::TEX_PARTICLE_SPARK {
                1.0
            } else {
                let l = light_at(
                    p.pos.x.floor() as i32,
                    p.pos.y.floor() as i32,
                    p.pos.z.floor() as i32,
                ) as f32
                    / 15.0;
                l.max(0.25)
            };
            out.push(crate::mesh::ParticleInstance {
                pos: [p.pos.x, p.pos.y, p.pos.z],
                size: p.size,
                color: [p.color[0] * lit, p.color[1] * lit, p.color[2] * lit, alpha],
                tex_layer: p.tex_layer,
                _pad: [0; 3],
            });
        }
    }
}

/// Public deterministic jitter in [-1, 1) for ambient spawn scatter (the
/// game-loop weather emitters).
pub fn ambient_jitter(seed: u64, lane: u64) -> f32 {
    jitter(seed, lane)
}

/// Where a falling weather particle spawned at `(x, from_y, z)` should die:
/// the top surface of the first non-air block below (roofs shelter interiors),
/// or far below if the column is open all the way down.
pub fn surface_kill_y(world: &crate::world::World, x: f32, from_y: f32, z: f32) -> f32 {
    let (bx, bz) = (x.floor() as i32, z.floor() as i32);
    let start = from_y.floor() as i32;
    for y in (start - 64..=start).rev() {
        if world.get_block(bx, y, bz) != block::AIR {
            return y as f32 + 1.05;
        }
    }
    from_y - 80.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics_settings::ParticleLevel;

    #[test]
    fn cap_is_respected_and_off_disables() {
        let mut ps = ParticleSystem::new();
        ps.set_cap(ParticleLevel::Reduced);
        for k in 0..CAP_REDUCED + 200 {
            ps.burst_chips(Vec3::ZERO, [1.0, 0.0, 0.0], 1, k as u64);
        }
        assert_eq!(ps.live(), CAP_REDUCED, "reduced cap holds");
        ps.set_cap(ParticleLevel::Off);
        assert_eq!(ps.live(), 0, "off clears the pool");
        ps.burst_smoke(Vec3::ZERO, 8, 1);
        assert_eq!(ps.live(), 0, "off spawns nothing");
        assert!(!ps.is_enabled());
    }

    #[test]
    fn particles_expire_by_life_and_by_kill_y() {
        let mut ps = ParticleSystem::new();
        ps.rain_streak(Vec3::new(0.0, 20.0, 0.0), 10.0);
        ps.burst_chips(Vec3::ZERO, [0.5; 3], 1, 7);
        assert_eq!(ps.live(), 2);
        // 1 s: the streak (fall 22 b/s) crosses kill_y=10; the chip (max
        // life 0.8s) times out.
        for _ in 0..60 {
            ps.tick(1.0 / 60.0, |_, _, _| false);
        }
        assert_eq!(ps.live(), 0, "both expiry paths fire");
    }

    #[test]
    fn bursts_are_deterministic_per_seed() {
        let mut a = ParticleSystem::new();
        let mut b = ParticleSystem::new();
        a.burst_embers(Vec3::new(1.0, 2.0, 3.0), 6, 42);
        b.burst_embers(Vec3::new(1.0, 2.0, 3.0), 6, 42);
        let (mut ia, mut ib) = (Vec::new(), Vec::new());
        a.instances(|_, _, _| 15, &mut ia);
        b.instances(|_, _, _| 15, &mut ib);
        assert_eq!(ia.len(), 6);
        for (x, y) in ia.iter().zip(ib.iter()) {
            assert_eq!(x.pos, y.pos);
            assert_eq!(x.color, y.color);
        }
        let mut c = ParticleSystem::new();
        c.burst_embers(Vec3::new(1.0, 2.0, 3.0), 6, 43);
        let mut ic = Vec::new();
        c.instances(|_, _, _| 15, &mut ic);
        assert_ne!(ia[0].pos, ic[0].pos, "different seed differs");
    }

    #[test]
    fn bolt_and_textured_chips_spawn_expected_counts() {
        let mut ps = ParticleSystem::new();
        ps.bolt(Vec3::new(0.5, 64.0, 0.5), 40.0, 7);
        assert_eq!(ps.live(), 40, "one spark per block of bolt height");
        let mut ic = Vec::new();
        ps.instances(|_, _, _| 15, &mut ic);
        assert!(ic.iter().all(|i| i.tex_layer == crate::block::TEX_PARTICLE_SPARK));
        let mut ps2 = ParticleSystem::new();
        ps2.burst_chips_textured(Vec3::ZERO, 123, 10, 9);
        let mut ic2 = Vec::new();
        ps2.instances(|_, _, _| 15, &mut ic2);
        assert_eq!(ic2.len(), 10);
        assert!(
            ic2.iter().all(|i| i.tex_layer == 123 && i.color[0] == 1.0),
            "textured chips sample the block's layer untinted"
        );
        // Bolt sparks die fast (the flash carries the drama, not lingering dots).
        for _ in 0..30 {
            ps.tick(1.0 / 60.0, |_, _, _| false);
        }
        assert_eq!(ps.live(), 0, "bolt gone within half a second");
    }

    #[test]
    fn wall_collision_kills_particles() {
        let mut ps = ParticleSystem::new();
        ps.burst_chips(Vec3::new(0.5, 10.5, 0.5), [1.0; 3], 4, 3);
        // Everything above y=10 counts as solid → chips die on first tick.
        ps.tick(0.05, |_, y, _| y >= 10);
        assert_eq!(ps.live(), 0, "chips fly into the wall and die");
    }

    #[test]
    fn dark_caves_dim_particles_but_sparks_stay_bright() {
        let mut ps = ParticleSystem::new();
        ps.burst_smoke(Vec3::ZERO, 1, 5);
        ps.burst_embers(Vec3::ZERO, 1, 5);
        let mut dark = Vec::new();
        ps.instances(|_, _, _| 0, &mut dark);
        let mut lit = Vec::new();
        ps.instances(|_, _, _| 15, &mut lit);
        let (smoke_i, spark_i) = if dark[0].tex_layer == crate::block::TEX_PARTICLE_SPARK {
            (1, 0)
        } else {
            (0, 1)
        };
        assert!(
            dark[smoke_i].color[0] < lit[smoke_i].color[0],
            "smoke dims in the dark"
        );
        assert_eq!(
            dark[spark_i].color[0], lit[spark_i].color[0],
            "embers/sparks are their own light source"
        );
    }

    #[test]
    fn alpha_fades_toward_end_of_life() {
        let mut ps = ParticleSystem::new();
        ps.burst_smoke(Vec3::ZERO, 1, 9);
        let mut early = Vec::new();
        ps.instances(|_, _, _| 15, &mut early);
        // Age the particle to its final 10% of life.
        let p_life = {
            let mut l = 0.0;
            for _ in 0..2000 {
                ps.tick(0.001, |_, _, _| false);
                let mut now = Vec::new();
                ps.instances(|_, _, _| 15, &mut now);
                if now.is_empty() {
                    break;
                }
                l = now[0].color[3];
            }
            l
        };
        assert!(
            p_life < early[0].color[3],
            "alpha near death ({p_life}) is below spawn alpha ({})",
            early[0].color[3]
        );
    }

    #[test]
    fn smoke_grows_and_instance_count_tracks_pool() {
        let mut ps = ParticleSystem::new();
        ps.burst_smoke(Vec3::ZERO, 4, 5);
        let mut before = Vec::new();
        ps.instances(|_, _, _| 15, &mut before);
        ps.tick(0.5, |_, _, _| false);
        let mut after = Vec::new();
        ps.instances(|_, _, _| 15, &mut after);
        assert_eq!(before.len(), 4);
        assert_eq!(after.len(), ps.live());
        assert!(after[0].size > before[0].size, "smoke grows");
    }
}


#[cfg(test)]
mod shader_guard {
    /// WGSL errors otherwise surface only at RUNTIME pipeline creation (native
    /// window / browser) — parse the shader at test time so a bad edit fails
    /// `cargo test`, not the first frame. Uses wgpu's re-exported naga.
    #[test]
    fn shader_wgsl_parses() {
        let src = include_str!("shader.wgsl");
        wgpu::naga::front::wgsl::parse_str(src).expect("shader.wgsl must parse");
    }

    #[test]
    fn overlay_wgsl_parses() {
        let src = include_str!("overlay.wgsl");
        wgpu::naga::front::wgsl::parse_str(src).expect("overlay.wgsl must parse");
    }
}
