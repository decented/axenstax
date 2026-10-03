//! Minecraft-style movement physics.
//!
//! All values from Spec 05 Section 1. Server runs at 20 TPS.
//! Physics uses momentum-based model: velocity_next = (velocity + accel) * drag

use glam::Vec3;
use crate::block::BlockRegistry;
use crate::camera::Camera;
use crate::player_intent::PlayerIntent;
use crate::world::World;

// Spec 05 Section 1.1 — speeds in blocks/second, converted to blocks/tick at 20 TPS
const WALK_SPEED: f32 = 4.317;
const SPRINT_SPEED: f32 = 5.612;
const SNEAK_SPEED: f32 = 1.295;
const FLY_SPEED: f32 = 10.89;
const FLY_SPRINT_SPEED: f32 = 21.78;

// Spec 05 Section 1.2
const JUMP_VELOCITY: f32 = 0.42; // blocks/tick (upward impulse)

// Spec 05 Section 1.3
const GROUND_DRAG: f32 = 0.91;
const GROUND_ACCEL: f32 = 0.1;
const AIR_DRAG: f32 = 0.91;
const AIR_ACCEL: f32 = 0.02;
// 2026-06-16 playtest (Axolittle "slide to much when you stop"): when grounded
// with NO movement input, decay horizontal velocity with a much stronger drag so
// the player stops promptly instead of skating. Kept SEPARATE from GROUND_DRAG so
// top speed is unchanged — lowering GROUND_DRAG itself would also cut the
// steady-state speed (v_max = a·drag/(1−drag)). Velocities below STOP_EPS snap to
// zero to kill residual creep. Feel-tunable.
const STOP_DRAG: f32 = 0.6;
const STOP_EPS: f32 = 0.005; // blocks/tick — below this (grounded, no input) → 0

// Spec 05 Section 1.1
const GRAVITY: f32 = 0.08; // blocks/tick² (applied each tick as downward accel)
// Terminal fall speed (blocks/tick). This USED to be load-bearing for collision
// safety: integration was un-swept, so a fall of ≥1 block/tick tunnelled through
// a 1-block floor and this cap was the only thing preventing it. Since 2026-09-06
// integration is SUB-STEPPED ([`substep_count`] / [`Player::integrate_substepped`]),
// which stops a fall of any speed, this is now purely a FEEL constant —
// Minecraft-like terminal velocity, and what the fall-damage curve is tuned
// against. Re-tune it freely; do not rely on it for collision safety.
const MAX_FALL_SPEED: f32 = 0.78;

// 2026-09-06 — sub-stepped collision. Per-sub-step displacement ceiling
// (blocks). `resolve_collisions` only inspects the single cell column the
// leading edge lands in, so any sub-move that crosses a whole cell can skip
// the block inside it. This MUST stay well under min(PLAYER_WIDTH, 1.0); 0.4
// leaves comfortable margin at every speed the game produces (creative sprint
// flight, the fastest, is 1.089 b/tick → 3 sub-steps).
const MAX_STEP: f32 = 0.4;
// Hard ceiling on sub-steps so a pathological velocity can never stall a tick.
// 64 × MAX_STEP = 25.6 b/tick, far above anything the game or the server-side
// speed cap (`server.rs` MAX_HORIZONTAL_PER_TICK) permits.
const MAX_SUBSTEPS: usize = 64;

// 2026-09-06 — swept-landing tolerance (blocks). The Y pass only accepts a box
// as ground/ceiling if the foot (resp. head) was already clear of it BEFORE the
// sub-move; this slack absorbs float error in the "rest exactly on the box top"
// case, where `prev_foot_y` should equal `b.max[1]` to the bit. Must stay far
// below any real block face gap (the vault bug it fixes had a 0.74-block gap).
const LAND_EPS: f32 = 1e-3;
// Horizontal inset (blocks) applied to the Y pass's footprint, so ground/ceiling
// contact needs a real millimetre of overlap rather than an exact hit on a block
// face. Mirrors the 0.01 vertical inset the X/Z passes already use.
const FOOT_INSET: f32 = 1e-3;

/// Number of collision sub-steps needed for a per-tick displacement of `speed`
/// blocks: `ceil(speed / MAX_STEP)`, at least 1. Pure, so the rule is
/// unit-testable without a world.
fn substep_count(speed: f32) -> usize {
    if speed.is_nan() || speed <= MAX_STEP {
        return 1;
    }
    ((speed / MAX_STEP).ceil() as usize).min(MAX_SUBSTEPS)
}

// Spec 05 §1.6 — ladder climb speed (blocks/second → /tick). Gentle, controllable.
const LADDER_CLIMB_SPEED: f32 = 2.5;

/// #30 — the per-tick vertical velocity while on a ladder. Jump climbs up, sneak
/// descends, neither clings in place (gravity is suppressed on a ladder). Pure,
/// so the climb rule is unit-tested independent of the world/collision.
pub fn ladder_climb_vy(jump_held: bool, sneak: bool) -> f32 {
    let step = LADDER_CLIMB_SPEED / 20.0;
    if jump_held {
        step
    } else if sneak {
        -step
    } else {
        0.0
    }
}

// COSMETIC INVARIANT (F2 — docs/foundations/2026-06-02-standard-avatar-and-byo-skins.md):
// These collision/eye-height constants are the player's ONLY hitbox. They are
// deliberately independent of the visual avatar model (entity_model.rs) and of
// CosmeticDescriptor. No skin, overlay, future 3D model, cape, or hat may change
// them — cosmetics are visual-only, so cosmetic geometry can never affect server
// fairness. Re-tune here ONLY as a deliberate, gameplay-reviewed change.
// Spec 05 Section 1.4
const PLAYER_WIDTH: f32 = 0.6;
const PLAYER_HEIGHT: f32 = 1.8;
const PLAYER_EYE_HEIGHT: f32 = 1.62; // Standard Minecraft eye height
// No consumer references this named constant — auto-step-up onto a half-
// block obstacle isn't implemented in the collision/movement code below
// (unlike PLAYER_WIDTH/HEIGHT/EYE_HEIGHT above, all live).
#[allow(dead_code)]
const STEP_HEIGHT: f32 = 0.5;

/// Rail-ramp direction gate: the off-axis fraction (0 = straight up/down the
/// slope, 1 = straight across it) above which a rail ramp behaves like a plain
/// block wall instead of a walkable slope. Below it you snap up onto the ramp;
/// at/along the axis you ride it 1:1. Tunable feel knob (see `apply_ramp_ride`).
const RAMP_SIDE_TOLERANCE: f32 = 0.20;

pub struct Player {
    /// Foot position (bottom-centre of hitbox).
    pub pos: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    pub flying: bool,
    pub in_water: bool,
    /// Task 15 — grounded sprint-speed multiplier from equipped Boots
    /// (`armour::sprint_multiplier`). 1.0 = no boots bonus (default).
    /// The caller (`PlayerSlot`, which owns `armour_slots`) refreshes this
    /// from the equipped Boots material each tick, before calling `tick()` —
    /// `Player` itself has no armour concept, only this scalar knob, so
    /// physics stays decoupled from the armour data layer. Grounded sprint
    /// only (Spec 05 §1.1); flying/noclip speed is untouched by boots.
    pub sprint_boots_mult: f32,
}

impl Player {
    pub fn new(spawn: Vec3) -> Self {
        Self {
            pos: spawn,
            velocity: Vec3::ZERO,
            on_ground: false,
            flying: false,
            in_water: false,
            sprint_boots_mult: 1.0,
        }
    }

    pub fn eye_pos(&self) -> Vec3 {
        self.pos + Vec3::new(0.0, PLAYER_EYE_HEIGHT, 0.0)
    }

    /// Run one physics tick (called at 20 TPS).
    ///
    /// `mode` gates flight + collision: Survival/Adventure are grounded and
    /// collide; Creative toggles flight (and collides); Spectator always flies
    /// AND passes through blocks (noclip). Flight is force-cleared in grounded
    /// modes so a Creative→Survival flip drops the player (Spec 05 §1.2 / §8).
    pub fn tick(
        &mut self,
        input: &PlayerIntent,
        camera: &Camera,
        world: &World,
        registry: &BlockRegistry,
        mode: crate::play_mode::PlayMode,
    ) {
        if !mode.flies() {
            // Grounded modes (Survival, Adventure): clear active flight + ignore toggle.
            self.flying = false;
        } else if mode.is_spectator() {
            // Spectator always flies (noclip cruise).
            self.flying = true;
        } else if input.toggle_flight {
            // Creative: toggle flight on double-tap space.
            self.flying = !self.flying;
            if self.flying {
                self.velocity.y = 0.0;
            }
        }

        // The movement paths below compute `self.velocity` ONLY — they never
        // write `self.pos`. Integration is done once, below, in collision-safe
        // sub-steps (see `integrate_substepped`).
        if self.flying {
            self.tick_flying(input, camera);
        } else {
            self.tick_survival(input, camera, world, registry);
        }

        // Spectator noclip: skip collision so the player passes through blocks
        // (and with nothing to collide against, one whole-displacement move is
        // exactly equivalent to sub-stepping).
        if mode.noclip() {
            self.pos += self.velocity;
        } else {
            self.integrate_substepped(world, registry);
        }
    }

    /// Apply this tick's `velocity` to `pos` in collision-safe sub-steps.
    ///
    /// 2026-09-06 fix — player report "you still go through blocks". Previously
    /// `tick` applied the whole per-tick displacement in one shot and called
    /// [`Player::resolve_collisions`] once, un-swept. Each axis pass only
    /// inspects the single cell column the leading edge occupies *after* the
    /// move, so a displacement approaching (or exceeding) one block could land
    /// the edge in the cell BEYOND a 1-block wall or floor — which was
    /// therefore never examined. Creative sprint flight is 21.78 b/s =
    /// **1.089 blocks/tick**, comfortably past that budget.
    ///
    /// The fix: split the displacement into `n = ceil(|v| / MAX_STEP)` equal
    /// sub-moves and resolve after each, so no sub-move can ever cross a cell
    /// unexamined. `n == 1` for ordinary walking/sprinting, so normal-speed
    /// behaviour (and every existing test) is bit-identical.
    ///
    /// Two details preserve the old semantics:
    /// * `resolve_collisions` zeroes the velocity component of any axis it
    ///   stopped. The remaining sub-steps drop that component from the delta
    ///   too, so a blocked axis stays blocked for the rest of the tick instead
    ///   of grinding into the wall.
    /// * `on_ground` is OR-ed across the sub-steps. Landing on sub-step 1 zeroes
    ///   `velocity.y`, which makes sub-step 2's Y pass a no-op that would
    ///   otherwise leave the freshly-cleared `on_ground` reading "airborne".
    fn integrate_substepped(&mut self, world: &World, registry: &BlockRegistry) {
        let n = substep_count(self.velocity.length());
        let mut delta = self.velocity / n as f32;
        let mut grounded = false;
        for _ in 0..n {
            let prev_pos = self.pos;
            self.pos += delta;
            self.resolve_collisions(prev_pos, world, registry);
            grounded |= self.on_ground;
            if self.velocity.x == 0.0 {
                delta.x = 0.0;
            }
            if self.velocity.y == 0.0 {
                delta.y = 0.0;
            }
            if self.velocity.z == 0.0 {
                delta.z = 0.0;
            }
        }
        self.on_ground = grounded;
    }

    fn tick_survival(&mut self, input: &PlayerIntent, camera: &Camera, world: &World, registry: &BlockRegistry) {
        // Water detection
        let feet_block_x = self.pos.x.floor() as i32;
        let feet_block_z = self.pos.z.floor() as i32;
        let feet_y = self.pos.y.floor() as i32;
        self.in_water = world.is_water(feet_block_x, feet_y, feet_block_z);

        // #30 — ladder climbing (Spec 05 §1.6). Out of water, while the feet
        // overlap a climbable block, gravity is cancelled and jump/sneak drive
        // a steady climb up/down (else cling in place). Horizontal walking is
        // unchanged so you can step off; collision still runs afterwards.
        if !self.in_water
            && world.is_climbable(feet_block_x, feet_y, feet_block_z)
        {
            let speed = WALK_SPEED;
            let mut move_dir = Vec3::ZERO;
            move_dir += camera.horizontal_forward() * input.move_forward;
            move_dir += camera.right() * input.move_right;
            if move_dir.length_squared() > 0.0 {
                move_dir = move_dir.normalize();
            }
            let speed_per_tick = speed / 20.0;
            self.velocity.x = move_dir.x * speed_per_tick;
            self.velocity.z = move_dir.z * speed_per_tick;
            self.velocity.y = ladder_climb_vy(input.jump_held, input.sneak);
            self.on_ground = false;
            // `tick` integrates (sub-stepped) — never write `pos` here.
            return;
        }

        if self.in_water {
            // Swimming: free-form movement like flying but slower, with water drag
            let speed = if input.sprint { 4.0 } else { 2.5 }; // swim sprint!
            let mut move_dir = Vec3::ZERO;
            let fwd = camera.horizontal_forward();
            let right = camera.right();

            move_dir += fwd * input.move_forward;
            move_dir += right * input.move_right;
            // Space = swim up, Shift = dive down
            if input.jump_held { move_dir.y += 1.0; }
            if input.sneak { move_dir.y -= 1.0; }

            if move_dir.length_squared() > 0.0 {
                move_dir = move_dir.normalize();
            }

            let speed_per_tick = speed / 20.0;

            // Direct velocity control (like flying but with water drag)
            self.velocity.x = move_dir.x * speed_per_tick * 0.8 + self.velocity.x * 0.3;
            self.velocity.z = move_dir.z * speed_per_tick * 0.8 + self.velocity.z * 0.3;

            if input.jump_held || input.sneak {
                // Active swimming: direct Y control
                self.velocity.y = move_dir.y * speed_per_tick * 0.8 + self.velocity.y * 0.3;
            } else {
                // Passive: slow sink with drag
                self.velocity.y = (self.velocity.y - 0.01) * 0.8;
            }

            // The current: flowing water nudges the swimmer downstream. You
            // can always swim against it (push is well under swim speed).
            if let Some(f) =
                crate::water::flow_vector(world, feet_block_x, feet_y, feet_block_z)
            {
                self.velocity.x += f[0] * crate::water::FLOW_PUSH;
                self.velocity.z += f[2] * crate::water::FLOW_PUSH;
            }

            // `tick` integrates (sub-stepped) — never write `pos` here.
            return;
        }

        // Determine target speed. Task 15 — Rubber Boots scale sprint only
        // (`sprint_boots_mult`, refreshed by the caller each tick from the
        // equipped Boots material); walking/sneaking are unaffected.
        let speed = if input.sprint {
            SPRINT_SPEED * self.sprint_boots_mult
        } else if input.sneak {
            SNEAK_SPEED
        } else {
            WALK_SPEED
        };

        // Build horizontal movement direction from input
        let mut move_dir = Vec3::ZERO;
        let fwd = camera.horizontal_forward();
        let right = camera.right();

        move_dir += fwd * input.move_forward;
        move_dir += right * input.move_right;

        let has_move_input = move_dir.length_squared() > 0.0;
        if has_move_input {
            move_dir = move_dir.normalize();
        }

        // Drag and acceleration factors depend on ground contact. When grounded
        // with no movement input, use STOP_DRAG so the player halts quickly
        // instead of skating (Axolittle "slide to much when you stop"); top speed
        // is unaffected because that's set by GROUND_DRAG while moving.
        let (drag, accel_factor) = if self.on_ground {
            if has_move_input {
                (GROUND_DRAG, GROUND_ACCEL)
            } else {
                (STOP_DRAG, GROUND_ACCEL)
            }
        } else {
            (AIR_DRAG, AIR_ACCEL)
        };

        // Apply acceleration: velocity = (velocity + accel) * drag
        // The target acceleration is scaled to match expected top speed:
        // At steady state: v = (v + a*dir) * drag → v = a*drag/(1-drag)
        // We want v_max = speed/20 (convert b/s to b/tick)
        let speed_per_tick = speed / 20.0;
        let accel_magnitude = speed_per_tick * accel_factor;

        self.velocity.x = (self.velocity.x + move_dir.x * accel_magnitude) * drag;
        self.velocity.z = (self.velocity.z + move_dir.z * accel_magnitude) * drag;
        // Snap residual creep to a dead stop (grounded, no input).
        if self.on_ground && !has_move_input {
            if self.velocity.x.abs() < STOP_EPS { self.velocity.x = 0.0; }
            if self.velocity.z.abs() < STOP_EPS { self.velocity.z = 0.0; }
        }

        // Gravity, capped at terminal fall speed. The cap is a feel constant,
        // not a tunnelling guard — sub-stepped integration handles any speed
        // (see MAX_FALL_SPEED / `integrate_substepped`).
        self.velocity.y -= GRAVITY;
        if self.velocity.y < -MAX_FALL_SPEED {
            self.velocity.y = -MAX_FALL_SPEED;
        }

        // Jump (only when on ground, Spec 05 Section 1.2)
        if input.jump_held && self.on_ground {
            self.velocity.y = JUMP_VELOCITY;
            self.on_ground = false;

            // Sprint-jump: boost forward velocity for ~4 block jump distance
            if input.sprint && move_dir.length_squared() > 0.0 {
                let boost_dir = move_dir.normalize();
                self.velocity.x += boost_dir.x * 0.2;
                self.velocity.z += boost_dir.z * 0.2;
            }
        }

        // #2 — sneak edge-protection (Minecraft "shift won't walk off ledges";
        // 2026-06-16 playtest: Axolittle wanted a crouch that stops you falling
        // off while building out over a drop). While sneaking AND grounded, zero
        // any horizontal velocity component that would carry the whole hitbox off
        // every supporting block. Checked per-axis so you can still slide ALONG
        // an edge. `on_ground` was cleared above if this tick jumped, so a
        // sneak-jump off a ledge is still allowed. support_y is the block directly
        // below the feet — reliable because Y-collision snaps pos.y to the block
        // top each tick.
        if input.sneak && self.on_ground {
            let half_w = PLAYER_WIDTH / 2.0;
            let support_y = self.pos.y.floor() as i32 - 1;
            let has_support = |x: f32, z: f32| {
                let min_x = (x - half_w).floor() as i32;
                let max_x = (x + half_w).floor() as i32;
                let min_z = (z - half_w).floor() as i32;
                let max_z = (z + half_w).floor() as i32;
                (min_x..=max_x)
                    .any(|bx| (min_z..=max_z).any(|bz| world.is_solid(bx, support_y, bz, registry)))
            };
            let new_x = self.pos.x + self.velocity.x;
            let new_z = self.pos.z + self.velocity.z;
            if !has_support(new_x, self.pos.z) {
                self.velocity.x = 0.0;
            }
            if !has_support(self.pos.x, new_z) {
                self.velocity.z = 0.0;
            }
        }

        // No `pos` write: `tick` integrates the velocity in collision-safe
        // sub-steps (`integrate_substepped`).
    }

    fn tick_flying(&mut self, input: &PlayerIntent, camera: &Camera) {
        let speed = if input.sprint {
            FLY_SPRINT_SPEED
        } else {
            FLY_SPEED
        };

        let mut move_dir = Vec3::ZERO;
        let fwd = camera.horizontal_forward();
        let right = camera.right();

        move_dir += fwd * input.move_forward;
        move_dir += right * input.move_right;
        if input.jump_held {
            move_dir += Vec3::Y;
        }
        if input.sneak {
            move_dir -= Vec3::Y;
        }

        if move_dir.length_squared() > 0.0 {
            move_dir = move_dir.normalize();
        }

        let speed_per_tick = speed / 20.0;
        self.velocity = move_dir * speed_per_tick;
        // No `pos` write — see `integrate_substepped`. Sprint flight is
        // 1.089 b/tick, the case that made the un-swept move unsafe.
    }

    /// AABB collision against voxel grid.
    /// Spec 05 Section 1.4: resolve Y first, then X, then Z.
    fn resolve_collisions(&mut self, prev_pos: Vec3, world: &World, registry: &BlockRegistry) {
        let half_w = PLAYER_WIDTH / 2.0;
        // Y-axis resolution first (gravity/jumping). F1 — resolve against
        // per-block collision boxes (full cube for ordinary solids, sub-boxes
        // for slabs/stairs) instead of a boolean. Scanning floor(pos.y) for the
        // foot still works: gravity nudges the foot into the supporting box's
        // cell each tick, so the box's true top is found.
        self.on_ground = false;
        // Rail ramps ride as a tilted slope (1:1) before the axis-collision
        // passes, so the leading edge is already lifted onto the ramp surface
        // and the step blocks never wall an on-axis climber. Side-on approaches
        // are rejected inside and fall through to normal wall collision below.
        self.apply_ramp_ride(world);
        {
            // The Y pass takes its horizontal footprint from the position
            // BEFORE this sub-move (`prev_pos`), not after — i.e. it behaves as
            // if Y were integrated first and X/Z afterwards, which is what
            // "resolve Y first" (Spec 05 §1.4) means. The sub-move applies all
            // three axes at once, so using the post-move footprint let the Y
            // pass see a horizontal penetration of a few millimetres that the
            // X/Z passes below were about to undo, and read the wall it was
            // penetrating as ground. The foot HEIGHT is still the post-move
            // `self.pos.y` — that is the swept vertical motion being resolved.
            // `FOOT_INSET` shrinks the footprint by a hair so a landing needs
            // real overlap, never a floating-point tie on a block face (the
            // X/Z passes shrink their vertical extent by 0.01 for the same
            // reason).
            let min_x = (prev_pos.x - half_w + FOOT_INSET).floor() as i32;
            let max_x = (prev_pos.x + half_w - FOOT_INSET).floor() as i32;
            let min_z = (prev_pos.z - half_w + FOOT_INSET).floor() as i32;
            let max_z = (prev_pos.z + half_w - FOOT_INSET).floor() as i32;
            let px0 = prev_pos.x - half_w + FOOT_INSET;
            let px1 = prev_pos.x + half_w - FOOT_INSET;
            let pz0 = prev_pos.z - half_w + FOOT_INSET;
            let pz1 = prev_pos.z + half_w - FOOT_INSET;

            if self.velocity.y < 0.0 {
                // Falling — land on the highest box top beneath the feet.
                let foot_y = self.pos.y.floor() as i32;
                let mut best_top = f32::NEG_INFINITY;
                for bx in min_x..=max_x {
                    for bz in min_z..=max_z {
                        for b in world.collision_boxes_at(bx, foot_y, bz, registry) {
                            // SWEPT LANDING (2026-09-06): a falling player may
                            // only land on a box whose top the foot was at or
                            // above BEFORE this sub-move. Without this, a
                            // horizontal penetration of a few millimetres —
                            // which the later X/Z passes are about to undo —
                            // let the Y pass treat a wall metres above the foot
                            // as ground and teleport the player onto its top
                            // (sprint-jumping vaulted any wall ≥2 tall).
                            // EXCEPTION: if the hitbox was ALREADY intersecting
                            // this box at the start of the sub-move the player
                            // is genuinely stuck inside terrain (spawned in it,
                            // a block placed on them, a teleport), so keep the
                            // old unconditional push-out to the box top.
                            if overlap_xz(px0, px1, pz0, pz1, &b)
                                && self.pos.y < b.max[1]
                                && (prev_pos.y >= b.max[1] - LAND_EPS
                                    || hitbox_intersects(prev_pos, &b))
                                && b.max[1] > best_top
                            {
                                best_top = b.max[1];
                            }
                        }
                    }
                }
                if best_top.is_finite() {
                    self.pos.y = best_top;
                    self.velocity.y = 0.0;
                    self.on_ground = true;
                }
            } else if self.velocity.y > 0.0 {
                // Rising — bonk on the lowest box bottom above the head
                // (head bonking, Spec 05 Section 1.4).
                let head_y = (self.pos.y + PLAYER_HEIGHT).floor() as i32;
                let mut best_bottom = f32::INFINITY;
                for bx in min_x..=max_x {
                    for bz in min_z..=max_z {
                        for b in world.collision_boxes_at(bx, head_y, bz, registry) {
                            // Symmetric swept rule: a rising player only bonks
                            // a box whose underside the head was at or below
                            // before this sub-move — same stuck-case exception.
                            if overlap_xz(px0, px1, pz0, pz1, &b)
                                && self.pos.y + PLAYER_HEIGHT > b.min[1]
                                && (prev_pos.y + PLAYER_HEIGHT <= b.min[1] + LAND_EPS
                                    || hitbox_intersects(prev_pos, &b))
                                && b.min[1] < best_bottom
                            {
                                best_bottom = b.min[1];
                            }
                        }
                    }
                }
                if best_bottom.is_finite() {
                    self.pos.y = best_bottom - PLAYER_HEIGHT;
                    self.velocity.y = 0.0;
                }
            }
        }

        // X-axis resolution (F1 — per-block boxes).
        {
            let min_y = self.pos.y.floor() as i32;
            let max_y = (self.pos.y + PLAYER_HEIGHT - 0.01).floor() as i32;
            let min_z = (self.pos.z - half_w).floor() as i32;
            let max_z = (self.pos.z + half_w).floor() as i32;
            let py0 = self.pos.y;
            let py1 = self.pos.y + PLAYER_HEIGHT - 0.01;
            let pz0 = self.pos.z - half_w;
            let pz1 = self.pos.z + half_w;

            if self.velocity.x > 0.0 {
                let edge_x = (self.pos.x + half_w).floor() as i32;
                let mut best_left = f32::INFINITY;
                for by in min_y..=max_y {
                    for bz in min_z..=max_z {
                        for b in world.collision_boxes_at(edge_x, by, bz, registry) {
                            if overlap_yz(py0, py1, pz0, pz1, &b)
                                && self.pos.x + half_w > b.min[0]
                                && b.min[0] < best_left
                            {
                                best_left = b.min[0];
                            }
                        }
                    }
                }
                if best_left.is_finite() {
                    self.pos.x = best_left - half_w;
                    self.velocity.x = 0.0;
                }
            } else if self.velocity.x < 0.0 {
                let edge_x = (self.pos.x - half_w).floor() as i32;
                let mut best_right = f32::NEG_INFINITY;
                for by in min_y..=max_y {
                    for bz in min_z..=max_z {
                        for b in world.collision_boxes_at(edge_x, by, bz, registry) {
                            if overlap_yz(py0, py1, pz0, pz1, &b)
                                && self.pos.x - half_w < b.max[0]
                                && b.max[0] > best_right
                            {
                                best_right = b.max[0];
                            }
                        }
                    }
                }
                if best_right.is_finite() {
                    self.pos.x = best_right + half_w;
                    self.velocity.x = 0.0;
                }
            }
        }

        // Z-axis resolution (F1 — per-block boxes).
        {
            let min_y = self.pos.y.floor() as i32;
            let max_y = (self.pos.y + PLAYER_HEIGHT - 0.01).floor() as i32;
            let min_x = (self.pos.x - half_w).floor() as i32;
            let max_x = (self.pos.x + half_w).floor() as i32;
            let px0 = self.pos.x - half_w;
            let px1 = self.pos.x + half_w;
            let py0 = self.pos.y;
            let py1 = self.pos.y + PLAYER_HEIGHT - 0.01;

            if self.velocity.z > 0.0 {
                let edge_z = (self.pos.z + half_w).floor() as i32;
                let mut best_front = f32::INFINITY;
                for by in min_y..=max_y {
                    for bx in min_x..=max_x {
                        for b in world.collision_boxes_at(bx, by, edge_z, registry) {
                            if overlap_xy(px0, px1, py0, py1, &b)
                                && self.pos.z + half_w > b.min[2]
                                && b.min[2] < best_front
                            {
                                best_front = b.min[2];
                            }
                        }
                    }
                }
                if best_front.is_finite() {
                    self.pos.z = best_front - half_w;
                    self.velocity.z = 0.0;
                }
            } else if self.velocity.z < 0.0 {
                let edge_z = (self.pos.z - half_w).floor() as i32;
                let mut best_back = f32::NEG_INFINITY;
                for by in min_y..=max_y {
                    for bx in min_x..=max_x {
                        for b in world.collision_boxes_at(bx, by, edge_z, registry) {
                            if overlap_xy(px0, px1, py0, py1, &b)
                                && self.pos.z - half_w < b.max[2]
                                && b.max[2] > best_back
                            {
                                best_back = b.max[2];
                            }
                        }
                    }
                }
                if best_back.is_finite() {
                    self.pos.z = best_back + half_w;
                    self.velocity.z = 0.0;
                }
            }
        }

        // Prevent falling through the void
        if self.pos.y < -64.0 {
            self.pos.y = 65.0;
            self.velocity = Vec3::ZERO;
        }
    }

    /// Ramp-aware vertical assist — makes a 45° rail ramp walk like a slope.
    ///
    /// A rail ramp is a diagonal staircase of solid blocks, each carrying a
    /// TRACK that links one-block-up-and-over into a continuous tilted surface
    /// (`rail::rail_ascend_dir` + `mesh::emit_ramp_rail`). Without help the
    /// player hits each step block as a 1-block wall and has to jump. This
    /// method, run at the top of collision resolution, instead sets the feet
    /// onto the tilted rail surface every tick so you rise **1:1** with your
    /// forward travel — smoothly, not in one-block snaps.
    ///
    /// Direction gate: the assist only engages when you are heading up/down the
    /// slope axis. `off_axis` (0 = straight along the slope, 1 = straight across
    /// it) beyond [`RAMP_SIDE_TOLERANCE`] is treated as approaching the ramp
    /// side-on, where each step stays a solid wall (normal collision blocks
    /// you). Within tolerance you snap up onto the surface.
    fn apply_ramp_ride(&mut self, world: &World) {
        // A real upward launch (a jump) opts out — let normal physics carry the
        // arc; falling (vy < 0) still rides so descents stay glued to the slope.
        if self.velocity.y > 0.12 {
            return;
        }
        let half_w = PLAYER_WIDTH / 2.0;
        let (vx, vz) = (self.velocity.x, self.velocity.z);
        let speed = (vx * vx + vz * vz).sqrt();
        let fy = self.pos.y.floor() as i32;
        let is_track =
            |c: crate::rail::Cell| world.get_block(c.0, c.1, c.2) == crate::rail::TRACK;

        // Unit movement direction (zero when standing still).
        let (ux, uz) = if speed > 1e-4 {
            (vx / speed, vz / speed)
        } else {
            (0.0, 0.0)
        };
        // Ramp cells to consider, best first: the step directly underfoot, then
        // the cell the leading edge is moving into (mounting a ramp, or crossing
        // from a flat top onto the next slope cell).
        let candidates = [
            (self.pos.x.floor() as i32, self.pos.z.floor() as i32),
            (
                (self.pos.x + ux * half_w).floor() as i32,
                (self.pos.z + uz * half_w).floor() as i32,
            ),
        ];

        for (cx, cz) in candidates {
            // The ramp rail sits at the same level as the feet (you stand on the
            // step whose top == the rail cell's floor); ±1 covers a foot that has
            // dipped/risen a hair between the step and the rail.
            for ry in [fy, fy + 1, fy - 1] {
                if world.get_block(cx, ry, cz) != crate::rail::TRACK {
                    continue;
                }
                let dir = match crate::rail::rail_ascend_dir(is_track, (cx, ry, cz)) {
                    Some(d) => d,
                    None => continue, // a flat rail is not a ramp
                };
                let (dxf, dzf) = (dir.0 as f32, dir.1 as f32);
                // Direction gate: reject side-on approaches (they stay walls).
                if speed > 1e-4 {
                    let along = (vx * dxf + vz * dzf) / speed; // -1..=1
                    let off_axis = (1.0 - along * along).max(0.0).sqrt(); // |sin θ|
                    if off_axis > RAMP_SIDE_TOLERANCE {
                        continue;
                    }
                }
                // Sample the surface at the uphill (+dir) edge of the hitbox —
                // the highest support under the feet. This is velocity-independent
                // so the resting height equals the climbing height (stopping
                // mid-ramp holds, doesn't sink), and it keeps the front of the
                // hitbox above the next step while climbing. Anchored to THIS cell.
                let sx = self.pos.x + dxf * half_w;
                let sz = self.pos.z + dzf * half_w;
                // Tilted surface height: `frac` climbs 0→1 from the low (−dir)
                // edge to the high (+dir) edge, so the rail top is `ry + frac`
                // (matching `emit_ramp_rail`'s `h_at`).
                let frac = match dir {
                    (1, 0) => sx - cx as f32,
                    (-1, 0) => (cx as f32 + 1.0) - sx,
                    (0, 1) => sz - cz as f32,
                    _ => (cz as f32 + 1.0) - sz,
                };
                let surface = ry as f32 + frac.clamp(0.0, 1.0);
                // Only ride a ramp whose surface is within a step of the feet —
                // never one we merely pass under or over.
                if (self.pos.y - surface).abs() > 1.1 {
                    continue;
                }
                self.pos.y = surface;
                self.velocity.y = 0.0;
                self.on_ground = true;
                return;
            }
        }
    }
}

/// F1 collision overlap helpers — whether the player AABB overlaps a block box
/// on the two axes *perpendicular* to the one being resolved. Strict `>`/`<`
/// (touching faces don't count) matches the old full-cube behaviour.
fn overlap_xz(px0: f32, px1: f32, pz0: f32, pz1: f32, b: &crate::block_shape::Aabb) -> bool {
    px1 > b.min[0] && px0 < b.max[0] && pz1 > b.min[2] && pz0 < b.max[2]
}
fn overlap_yz(py0: f32, py1: f32, pz0: f32, pz1: f32, b: &crate::block_shape::Aabb) -> bool {
    py1 > b.min[1] && py0 < b.max[1] && pz1 > b.min[2] && pz0 < b.max[2]
}
fn overlap_xy(px0: f32, px1: f32, py0: f32, py1: f32, b: &crate::block_shape::Aabb) -> bool {
    px1 > b.min[0] && px0 < b.max[0] && py1 > b.min[1] && py0 < b.max[1]
}

/// Full 3-axis test: does the player hitbox with its foot at `foot` already
/// intersect `b`? Used by the swept Y pass for the STUCK case — a player whose
/// hitbox was inside a solid before the sub-move is pushed out unconditionally,
/// exactly as the pre-2026-09-06 code did for every case.
fn hitbox_intersects(foot: Vec3, b: &crate::block_shape::Aabb) -> bool {
    let half_w = PLAYER_WIDTH / 2.0;
    foot.x + half_w > b.min[0]
        && foot.x - half_w < b.max[0]
        && foot.y + PLAYER_HEIGHT > b.min[1]
        && foot.y < b.max[1]
        && foot.z + half_w > b.min[2]
        && foot.z - half_w < b.max[2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{self, BlockRegistry};
    use crate::play_mode::PlayMode;
    use crate::world::World;

    fn empty_setup() -> (World, BlockRegistry, Camera) {
        let world = World::new();
        let registry = BlockRegistry::new();
        let cam = Camera::new(Vec3::new(0.0, 100.0, 0.0), 1.0);
        (world, registry, cam)
    }

    fn solid_floor_setup() -> (World, BlockRegistry, Camera) {
        let mut world = World::new();
        let registry = BlockRegistry::new();
        for x in -5..=5 {
            for z in -5..=5 {
                world.set_block(x, 63, z, block::STONE);
            }
        }
        let cam = Camera::new(Vec3::new(0.0, 64.0, 0.0), 1.0);
        (world, registry, cam)
    }

    /// F1 — an 11×11 platform of slabs at y=63 with the given facing.
    fn slab_floor_setup(facing: crate::meta::Facing) -> (World, BlockRegistry, Camera) {
        let mut world = World::new();
        let registry = BlockRegistry::new();
        for x in -5..=5 {
            for z in -5..=5 {
                world.set_block(x, 63, z, block::STONE_SLAB);
                world.set_meta((x, 63, z), crate::meta::with_facing(0, facing));
            }
        }
        let cam = Camera::new(Vec3::new(0.0, 64.0, 0.0), 1.0);
        (world, registry, cam)
    }

    #[test]
    fn full_cube_collision_boxes_match_legacy_is_solid() {
        // Regression guard: a full-cube solid returns exactly the unit box the
        // old boolean path assumed; air returns nothing.
        let (world, registry, _) = solid_floor_setup();
        let boxes = world.collision_boxes_at(0, 63, 0, &registry);
        assert_eq!(boxes.len(), 1);
        assert_eq!(
            boxes[0],
            crate::block_shape::Aabb::new([0.0, 63.0, 0.0], [1.0, 64.0, 1.0])
        );
        assert!(world.collision_boxes_at(0, 80, 0, &registry).is_empty());
    }

    #[test]
    fn player_lands_on_bottom_slab_at_half_height() {
        use crate::meta::Facing;
        let (world, registry, cam) = slab_floor_setup(Facing::Down);
        let mut p = Player::new(Vec3::new(0.0, 65.0, 0.0));
        let idle = PlayerIntent::default();
        for _ in 0..80 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(p.on_ground, "should rest on the slab");
        // Slab cell y=63, bottom slab top = 63.5.
        assert!(
            (p.pos.y - 63.5).abs() < 0.01,
            "foot should rest at 63.5, got {}",
            p.pos.y
        );
    }

    #[test]
    fn player_lands_on_top_slab_at_full_height() {
        use crate::meta::Facing;
        let (world, registry, cam) = slab_floor_setup(Facing::Up);
        let mut p = Player::new(Vec3::new(0.0, 65.0, 0.0));
        let idle = PlayerIntent::default();
        for _ in 0..80 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(p.on_ground);
        // Top slab in cell 63 occupies [63.5, 64.0]; foot rests at 64.0.
        assert!(
            (p.pos.y - 64.0).abs() < 0.01,
            "foot should rest at 64.0, got {}",
            p.pos.y
        );
    }

    #[test]
    fn tick_is_deterministic_same_inputs_same_outputs() {
        // Run the same intent sequence twice and assert byte-identical state.
        // Critical precondition for Task 1d's client prediction + server
        // reconciliation: if Player::tick isn't deterministic, clients and
        // server never agree.
        let (world, registry, cam) = solid_floor_setup();

        let intent = PlayerIntent {
            move_forward: 1.0,
            move_right: 0.0,
            sprint: false, sneak: false,
            jump_held: false, jump_pressed: false,
            ..Default::default()
        };

        let mut p1 = Player::new(Vec3::new(0.0, 64.0, 0.0));
        let mut p2 = Player::new(Vec3::new(0.0, 64.0, 0.0));

        for _ in 0..100 {
            p1.tick(&intent, &cam, &world, &registry, PlayMode::Survival);
            p2.tick(&intent, &cam, &world, &registry, PlayMode::Survival);
        }

        assert_eq!(p1.pos.to_array(), p2.pos.to_array(), "position drift");
        assert_eq!(p1.velocity.to_array(), p2.velocity.to_array(), "velocity drift");
        assert_eq!(p1.on_ground, p2.on_ground);
        assert_eq!(p1.flying, p2.flying);
    }

    #[test]
    fn gravity_pulls_player_down_when_airborne() {
        let (world, registry, cam) = empty_setup();
        let mut p = Player::new(Vec3::new(0.0, 100.0, 0.0));
        let idle = PlayerIntent::default();

        let start_y = p.pos.y;
        for _ in 0..5 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(
            p.pos.y < start_y,
            "gravity should have pulled player down: start={start_y}, end={}",
            p.pos.y,
        );
        assert!(p.velocity.y < 0.0, "y velocity should be negative (falling)");
    }

    #[test]
    fn player_lands_on_solid_floor() {
        // Dropping from slightly above the floor converges to rest.
        let (world, registry, cam) = solid_floor_setup();
        let mut p = Player::new(Vec3::new(0.0, 65.0, 0.0));
        let idle = PlayerIntent::default();

        for _ in 0..60 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        // Top of the floor is y=64, player foot should be on it.
        assert!(p.on_ground, "player should be on ground after landing");
        assert!(
            (p.pos.y - 64.0).abs() < 0.01,
            "player foot should rest at y=64, got {}",
            p.pos.y
        );
    }

    #[test]
    fn stop_drag_decays_horizontal_speed_quickly_when_input_stops() {
        // 2026-06-16 playtest ("slide too much when you stop") — pin the tuned
        // feel value so a future edit can't silently drift it, and pin the
        // decay behaviour it produces: grounded + no input must shed speed
        // fast rather than let the player skate.
        assert_eq!(STOP_DRAG, 0.6, "tuned feel value — must not drift silently");

        let (world, registry, cam) = solid_floor_setup();
        let mut p = Player::new(Vec3::new(0.0, 64.0, 0.0));
        let moving = PlayerIntent { move_forward: 1.0, ..Default::default() };
        let idle = PlayerIntent::default();

        // Build up horizontal speed while grounded and moving. Kept short
        // enough that the player stays on the 11×11 platform (top speed
        // converges well before this many ticks).
        for _ in 0..15 {
            p.tick(&moving, &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(p.on_ground, "should be grounded before the stop test");
        let moving_speed = (p.velocity.x * p.velocity.x + p.velocity.z * p.velocity.z).sqrt();
        assert!(moving_speed > 0.01, "should have built up horizontal speed");

        // Drop input; speed must decay below 0.01 within a small bounded
        // number of ticks (each tick multiplies horizontal velocity by
        // STOP_DRAG while grounded with no input).
        let mut speed = moving_speed;
        let mut ticks_taken = 0;
        for i in 1..=10 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
            speed = (p.velocity.x * p.velocity.x + p.velocity.z * p.velocity.z).sqrt();
            ticks_taken = i;
            if speed < 0.01 {
                break;
            }
        }
        assert!(
            speed < 0.01,
            "horizontal speed should decay below 0.01 within 10 ticks of no \
             input, got {speed} after {ticks_taken} ticks"
        );
    }

    #[test]
    fn cannot_walk_through_wall() {
        // Build a floor plus a wall at x=2.
        let mut world = World::new();
        let registry = BlockRegistry::new();
        for x in -5..=5 {
            for z in -5..=5 {
                world.set_block(x, 63, z, block::STONE);
            }
        }
        for y in 64..=66 {
            for z in -5..=5 {
                world.set_block(2, y, z, block::STONE);
            }
        }
        let cam = Camera::new(Vec3::new(0.0, 64.0, 0.0), 1.0);
        let mut p = Player::new(Vec3::new(0.0, 64.0, 0.0));
        let forward_x = PlayerIntent {
            move_forward: 0.0,
            move_right: 1.0, // move right (cam forward is -Z, right is +X)
            sprint: false,
            ..Default::default()
        };
        // Use a camera yaw that makes +X the forward/right direction.
        let mut cam = cam;
        cam.yaw = 0.0;

        for _ in 0..200 {
            p.tick(&forward_x, &cam, &world, &registry, PlayMode::Survival);
        }

        // Wall face is at x=2.0 from the -x side. Player width 0.6 → half 0.3.
        // So foot centre should be <= 2.0 - 0.3 = 1.7.
        assert!(
            p.pos.x <= 1.75,
            "player walked through wall: x={}",
            p.pos.x
        );
    }

    #[test]
    fn jump_does_not_exceed_bounded_height() {
        let (world, registry, cam) = solid_floor_setup();
        let mut p = Player::new(Vec3::new(0.0, 64.0, 0.0));
        // Give player one tick on the ground to establish on_ground=true,
        // then apply jump.
        let idle = PlayerIntent::default();
        let jump = PlayerIntent {
            jump_pressed: true,
            jump_held: true,
            ..Default::default()
        };
        for _ in 0..5 { p.tick(&idle, &cam, &world, &registry, PlayMode::Survival); } // settle
        assert!(p.on_ground, "player should be on ground before jump");

        // One jump tick, then idle to allow arc to complete.
        p.tick(&jump, &cam, &world, &registry, PlayMode::Survival);
        let mut max_height = p.pos.y;
        for _ in 0..100 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
            if p.pos.y > max_height { max_height = p.pos.y; }
        }
        // Spec 05: jump height ~1.25 blocks above origin. Allow 1.5 ceiling.
        assert!(
            max_height <= 64.0 + 1.5,
            "jump height {} exceeds 1.5 blocks above floor y=64",
            max_height
        );
        assert!(
            max_height >= 64.0 + 1.0,
            "jump height {} below 1 block — jump didn't fire",
            max_height
        );
    }

    /// A full-width 45° rail ramp ascending +X: base ground at feet-65 to the
    /// west, four diagonal steps (each a solid block + a linking TRACK) climbing
    /// to feet-68, then a flat top platform. Spans z∈[-3,3] so the 0.6-wide
    /// player stays on it.
    fn ramp_world_east() -> (World, BlockRegistry, Camera) {
        let mut world = World::new();
        let registry = BlockRegistry::new();
        // Base ground the player walks in from (top = 65).
        for x in -10..=-1 {
            for z in -3..=3 {
                world.set_block(x, 64, z, block::STONE);
            }
        }
        // Diagonal staircase: step i at x=i has solid top (65+i) and a rail above
        // it; each rail links one-up-and-over into the next, forming the slope.
        for i in 0..=3 {
            for z in -3..=3 {
                world.set_block(i, 64 + i, z, block::STONE); // solid step, top 65+i
                world.set_block(i, 65 + i, z, crate::rail::TRACK); // linking rail
            }
        }
        // Flat top platform at feet 68 beyond the ramp.
        for x in 4..=10 {
            for z in -3..=3 {
                world.set_block(x, 67, z, block::STONE);
            }
        }
        let mut cam = Camera::new(Vec3::new(0.0, 66.0, 0.0), 1.0);
        cam.yaw = 0.0; // +X is forward/right
        (world, registry, cam)
    }

    #[test]
    fn walks_up_a_rail_ramp_smoothly() {
        // Walking straight up the ramp climbs 1:1 with forward travel and never
        // snaps a whole block at once.
        let (world, registry, cam) = ramp_world_east();
        let mut p = Player::new(Vec3::new(-2.0, 65.0, 0.0));
        let idle = PlayerIntent::default();
        for _ in 0..5 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        let walk = PlayerIntent { move_right: 1.0, ..Default::default() };
        let mut prev_y = p.pos.y;
        let mut max_step = 0.0f32;
        let mut mid: Option<(f32, f32)> = None;
        for _ in 0..70 {
            p.tick(&walk, &cam, &world, &registry, PlayMode::Survival);
            max_step = max_step.max((p.pos.y - prev_y).abs());
            prev_y = p.pos.y;
            if mid.is_none() && (1.3..=1.7).contains(&p.pos.x) {
                mid = Some((p.pos.x, p.pos.y));
            }
        }
        assert!(p.pos.x > 3.5, "should have climbed the ramp, x={}", p.pos.x);
        assert!(
            (p.pos.y - 68.0).abs() < 0.25,
            "should stand on the 68 top, y={}",
            p.pos.y
        );
        // Smooth: never a full-block snap (a 1:1 slope at walking speed rises a
        // fraction of a block per tick).
        assert!(
            max_step < 0.45,
            "climb must be smooth, biggest single-tick rise was {}",
            max_step
        );
        // 1:1: the slope is anchored at (x=0, feet=65), so mid-ramp feet ≈ 65+x.
        let (mx, my) = mid.expect("player should pass through mid-ramp");
        assert!(
            (my - (65.0 + mx)).abs() < 0.5,
            "mid-ramp feet {my} should track the 1:1 slope (65+x={})",
            65.0 + mx
        );
    }

    #[test]
    fn stopping_mid_ramp_holds_height() {
        // Regression: stopping half-way up must not drop the player — the feet
        // stay on the slope at the height they climbed to.
        let (world, registry, cam) = ramp_world_east();
        let mut p = Player::new(Vec3::new(-2.0, 65.0, 0.0));
        let idle = PlayerIntent::default();
        for _ in 0..5 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        let walk = PlayerIntent { move_right: 1.0, ..Default::default() };
        // Climb until part-way up the ramp.
        for _ in 0..28 {
            p.tick(&walk, &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(
            p.pos.x > 0.8 && p.pos.y > 65.8,
            "precondition: should be part-way up the ramp (x={}, y={})",
            p.pos.x,
            p.pos.y
        );
        // Let residual momentum settle (releasing the key coasts a fraction),
        // then capture the resting height.
        for _ in 0..10 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        let held = p.pos.y;
        assert!(held > 65.8, "precondition: still part-way up (y={held})");
        // Standing still from here must hold that height exactly (no sinking).
        for _ in 0..20 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(
            (p.pos.y - held).abs() < 0.02,
            "standing still on the ramp must hold height: {held} -> {}",
            p.pos.y
        );
    }

    #[test]
    fn walks_down_a_rail_ramp_smoothly() {
        // Walking down the ramp descends smoothly, gluing the feet to the slope.
        let (world, registry, cam) = ramp_world_east();
        let mut p = Player::new(Vec3::new(5.0, 68.0, 0.0)); // on the top platform
        let idle = PlayerIntent::default();
        for _ in 0..5 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        let walk = PlayerIntent { move_right: -1.0, ..Default::default() }; // west, down
        let mut prev_y = p.pos.y;
        let mut max_step = 0.0f32;
        for _ in 0..55 {
            p.tick(&walk, &cam, &world, &registry, PlayMode::Survival);
            max_step = max_step.max((p.pos.y - prev_y).abs());
            prev_y = p.pos.y;
        }
        assert!(p.pos.x < 0.5, "should have descended past the base, x={}", p.pos.x);
        assert!(
            (p.pos.y - 65.0).abs() < 0.3,
            "should end on the base at 65, y={}",
            p.pos.y
        );
        assert!(
            max_step < 0.5,
            "descent must be smooth, biggest single-tick drop was {}",
            max_step
        );
    }

    #[test]
    fn ramp_side_on_approach_does_not_climb() {
        // Direction gate: heading up the slope axis climbs, but moving across it
        // (off-axis beyond tolerance) does not ride up — it stays a wall.
        let (world, registry, cam) = ramp_world_east();
        let idle = PlayerIntent::default();

        // Control — walking +X (on-axis) climbs.
        let mut pc = Player::new(Vec3::new(-1.0, 65.0, 0.0));
        for _ in 0..5 {
            pc.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        let fwd = PlayerIntent { move_right: 1.0, ..Default::default() };
        for _ in 0..40 {
            pc.tick(&fwd, &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(pc.pos.y > 66.5, "on-axis walk should climb, y={}", pc.pos.y);

        // Gate — from the base, walking across the slope (−Z) must not climb.
        let mut pz = Player::new(Vec3::new(0.0, 65.0, 0.0));
        for _ in 0..5 {
            pz.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        let across = PlayerIntent { move_forward: 1.0, ..Default::default() }; // yaw 0 → −Z
        for _ in 0..15 {
            pz.tick(&across, &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(pz.pos.z.abs() > 0.5, "should have moved across, z={}", pz.pos.z);
        assert!(
            pz.pos.y < 65.6,
            "moving across the slope must not climb it, y={}",
            pz.pos.y
        );
    }

    #[test]
    fn bare_step_without_rail_still_blocks() {
        // A 1-block step WITHOUT a rail must still block — the auto step-up is
        // gated to rail-topped blocks, never a general 1-block wall-climb.
        let mut world = World::new();
        let registry = BlockRegistry::new();
        for x in -5..=5 {
            for z in -5..=5 {
                world.set_block(x, 63, z, block::STONE);
            }
        }
        for z in -5..=5 {
            world.set_block(2, 64, z, block::STONE); // bare step, nothing on top
        }
        let mut cam = Camera::new(Vec3::new(0.0, 64.0, 0.0), 1.0);
        cam.yaw = 0.0;
        let mut p = Player::new(Vec3::new(0.0, 64.0, 0.0));
        let idle = PlayerIntent::default();
        for _ in 0..5 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        let walk = PlayerIntent { move_right: 1.0, ..Default::default() };
        for _ in 0..200 {
            p.tick(&walk, &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(
            p.pos.x <= 1.75,
            "player should be blocked by the bare step, x={}",
            p.pos.x
        );
        assert!(
            p.pos.y < 64.5,
            "player should not have climbed the bare step, y={}",
            p.pos.y
        );
    }

    /// A long, narrow runway (x=-100..=100, z=-2..=2) at y=63 — long enough
    /// that a full-tilt sprint never runs off the edge within the ~60-tick
    /// window the sprint-boots tests below settle over.
    fn runway_setup() -> (World, BlockRegistry, Camera) {
        let mut world = World::new();
        let registry = BlockRegistry::new();
        for x in -100..=100 {
            for z in -2..=2 {
                world.set_block(x, 63, z, block::STONE);
            }
        }
        let cam = Camera::new(Vec3::new(0.0, 64.0, 0.0), 1.0);
        (world, registry, cam)
    }

    /// Task 15 — Rubber Boots sprint bonus. `Player::sprint_boots_mult`
    /// (set by the caller from `armour::sprint_multiplier`) must scale the
    /// grounded sprint top speed by exactly that factor.
    #[test]
    fn rubber_boots_sprint_multiplier_scales_top_speed() {
        let (world, registry, cam) = runway_setup();
        let sprint = PlayerIntent { move_right: 1.0, sprint: true, ..Default::default() };

        let mut bare = Player::new(Vec3::new(0.0, 64.0, 0.0));
        let mut booted = Player::new(Vec3::new(0.0, 64.0, 0.0));
        let mult = crate::armour::sprint_multiplier(Some(crate::armour::ArmourMaterial::Rubber));
        booted.sprint_boots_mult = mult;

        // Run to steady-state top speed (drag/accel converge within ~40 ticks).
        for _ in 0..60 {
            bare.tick(&sprint, &cam, &world, &registry, PlayMode::Survival);
            booted.tick(&sprint, &cam, &world, &registry, PlayMode::Survival);
        }

        // Both stay grounded on the runway — pure-horizontal velocity, so
        // `.length()` is the sprint speed with no vertical contamination.
        assert!(bare.on_ground && booted.on_ground, "both should still be on the runway");
        let bare_speed = bare.velocity.length();
        let booted_speed = booted.velocity.length();
        assert!(bare_speed > 0.0, "bare sprint should be moving");
        let ratio = booted_speed / bare_speed;
        assert!(
            (ratio - mult).abs() < 0.01,
            "booted speed should be exactly the multiplier ({mult}) times bare speed, got ratio {ratio}"
        );
        // Belt-and-braces per the task brief: the multiplier stays capped at
        // 1.4. The live server gate (server.rs MAX_HORIZONTAL_PER_TICK =
        // 1.089 × 1.5 ≈ 1.6335 b/tick, FLY_SPRINT_SPEED-derived) has ample
        // room, but 1.4 keeps booted sprint-jump (≈0.597 b/tick peak) well
        // clear; full rationale on `armour::sprint_multiplier`.
        assert!(mult <= 1.4, "boots multiplier must stay capped at 1.4, got {mult}");
    }

    #[test]
    fn no_rubber_boots_leaves_sprint_speed_unchanged() {
        let (world, registry, cam) = runway_setup();
        let sprint = PlayerIntent { move_right: 1.0, sprint: true, ..Default::default() };

        let mut default_mult = Player::new(Vec3::new(0.0, 64.0, 0.0));
        let mut explicit_no_boots = Player::new(Vec3::new(0.0, 64.0, 0.0));
        explicit_no_boots.sprint_boots_mult = crate::armour::sprint_multiplier(None);

        for _ in 0..60 {
            default_mult.tick(&sprint, &cam, &world, &registry, PlayMode::Survival);
            explicit_no_boots.tick(&sprint, &cam, &world, &registry, PlayMode::Survival);
        }

        assert!(
            (default_mult.velocity.length() - explicit_no_boots.velocity.length()).abs() < 1e-4,
            "no boots (default) should match explicit sprint_multiplier(None) == 1.0"
        );
    }
}

/// TDD tests for PlayMode-specific physics behaviours.
///
/// Camera at yaw=0.0 → horizontal_forward = (0, 0, -1) (Minecraft convention,
/// looking along -Z). So `move_forward = 1.0` moves the player in the -Z
/// direction. Wall blocks are placed in the -Z direction from the player start.
#[cfg(test)]
mod mode_tests {
    use super::*;
    use crate::block::{self, BlockRegistry};
    use crate::play_mode::PlayMode;
    use crate::world::World;

    /// A wall of stone blocks at z=-2, spanning y=64..=65, in a floor at y=63.
    /// Player foot starts at (0.5, 64.0, 0.0) — in the -Z lane with the wall.
    /// Camera yaw=0 → forward is -Z, so `move_forward=1.0` drives the player
    /// into the wall.
    fn wall_setup() -> (World, BlockRegistry, Camera) {
        let mut world = World::new();
        let registry = BlockRegistry::new();
        // Floor at y=63 so player lands at foot y=64.
        for x in -5..=5 {
            for z in -5..=5 {
                world.set_block(x, 63, z, block::STONE);
            }
        }
        // Wall at z=-2 (player moves in -Z direction toward it).
        for y in 64..=65 {
            for x in -5..=5 {
                world.set_block(x, y, -2, block::STONE);
            }
        }
        // Camera yaw=0 → horizontal_forward = (0, 0, -1).
        let cam = Camera::new(Vec3::new(0.5, 64.0, 0.0), 1.0);
        (world, registry, cam)
    }

    fn forward_input() -> PlayerIntent {
        PlayerIntent {
            move_forward: 1.0,
            ..Default::default()
        }
    }

    // ── #30 — ladder climbing ────────────────────────────────────────

    #[test]
    fn ladder_climb_vy_directions() {
        let up = ladder_climb_vy(true, false);
        let down = ladder_climb_vy(false, true);
        assert!(up > 0.0, "jump climbs up");
        assert!(down < 0.0, "sneak climbs down");
        assert_eq!(ladder_climb_vy(false, false), 0.0, "no input clings in place");
        assert_eq!(up, -down, "symmetric up/down speed");
    }

    fn jump_input() -> PlayerIntent {
        PlayerIntent { jump_held: true, ..Default::default() }
    }

    #[test]
    fn standing_in_a_ladder_climbs_up_on_jump_not_falls() {
        // A ladder column at the player's cell; holding jump rises steadily,
        // and crucially does NOT fall under gravity while clinging.
        let mut world = World::new();
        let registry = BlockRegistry::new();
        for x in -2..=2 {
            for z in -2..=2 {
                world.set_block(x, 63, z, block::STONE);
            }
        }
        // Ladder column spanning y=64..=70 at the player's cell (0,_,0).
        for y in 64..=70 {
            world.set_block(0, y, 0, block::LADDER);
        }
        let cam = Camera::new(Vec3::new(0.5, 64.0, 0.5), 1.0);
        let mut p = Player::new(Vec3::new(0.5, 64.0, 0.5));
        let start_y = p.pos.y;
        for _ in 0..10 {
            p.tick(&jump_input(), &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(p.pos.y > start_y + 0.5, "held jump climbs the ladder, y={}", p.pos.y);

        // Now cling (no input): should hold height, not plummet.
        let held_y = p.pos.y;
        p.tick(&PlayerIntent::default(), &cam, &world, &registry, PlayMode::Survival);
        assert!((p.pos.y - held_y).abs() < 0.05, "clings in place, y={}", p.pos.y);
    }

    // ── #2 — sneak edge-protection ──────────────────────────────────

    /// A 3×3 plateau at y=63 (blocks x∈0..=2, z∈0..=2 → world x,z ∈ [0,3]),
    /// surrounded by void. Camera yaw=0 → `move_right=1.0` drives +X toward the
    /// far edge at x=3.0.
    fn plateau_setup() -> (World, BlockRegistry, Camera) {
        let mut world = World::new();
        let registry = BlockRegistry::new();
        for x in 0..=2 {
            for z in 0..=2 {
                world.set_block(x, 63, z, block::STONE);
            }
        }
        let mut cam = Camera::new(Vec3::new(0.5, 64.0, 0.5), 1.0);
        cam.yaw = 0.0;
        (world, registry, cam)
    }

    fn sneak_right_input() -> PlayerIntent {
        PlayerIntent { move_right: 1.0, sneak: true, ..Default::default() }
    }

    fn walk_right_input() -> PlayerIntent {
        PlayerIntent { move_right: 1.0, ..Default::default() }
    }

    #[test]
    fn sneaking_does_not_walk_off_ledge() {
        // Sneaking toward the +X void edge must keep the player on the plateau —
        // never falling off — even after sustained input.
        let (world, registry, cam) = plateau_setup();
        let mut p = Player::new(Vec3::new(0.5, 64.0, 0.5));
        // Settle onto the plateau first.
        for _ in 0..5 {
            p.tick(&PlayerIntent::default(), &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(p.on_ground, "player should be grounded before sneaking");

        for _ in 0..120 {
            p.tick(&sneak_right_input(), &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(
            p.on_ground && p.pos.y > 63.5,
            "sneaking must keep the player on the plateau, got pos.y={} on_ground={}",
            p.pos.y,
            p.on_ground,
        );
    }

    #[test]
    fn walking_off_ledge_falls_without_sneak() {
        // Contrast: the SAME approach without sneak must walk off and fall, so
        // the test above proves sneak — not some unrelated clamp — does the work.
        let (world, registry, cam) = plateau_setup();
        let mut p = Player::new(Vec3::new(0.5, 64.0, 0.5));
        for _ in 0..5 {
            p.tick(&PlayerIntent::default(), &cam, &world, &registry, PlayMode::Survival);
        }
        for _ in 0..120 {
            p.tick(&walk_right_input(), &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(
            p.pos.y < 62.0,
            "walking without sneak should fall off the plateau, got pos.y={}",
            p.pos.y,
        );
    }

    #[test]
    fn spectator_passes_through_solid_block() {
        // Spectator noclip: player should fly through the wall at z=-2 and
        // end up on the far side (z < -2.3, i.e. past the back face + half-width).
        let (world, registry, cam) = wall_setup();
        let mut p = Player::new(Vec3::new(0.5, 65.0, 0.0));
        p.flying = true;
        for _ in 0..40 {
            p.tick(&forward_input(), &cam, &world, &registry, PlayMode::Spectator);
        }
        assert!(
            p.pos.z < -2.3,
            "spectator should pass through wall at z=-2, pos.z={}",
            p.pos.z
        );
    }

    #[test]
    fn creative_still_collides_with_solid_block() {
        // Creative collides: player flying toward wall at z=-2 must stop before
        // passing through. The near face of the block is at z=-2.0; with
        // player half-width=0.3 the foot centre stops at z >= -2.0 + 0.3 = -1.7.
        let (world, registry, cam) = wall_setup();
        let mut p = Player::new(Vec3::new(0.5, 65.0, 0.0));
        p.flying = true;
        for _ in 0..40 {
            p.tick(&forward_input(), &cam, &world, &registry, PlayMode::Creative);
        }
        assert!(
            p.pos.z >= -1.85,
            "creative should collide with wall at z=-2, pos.z={}",
            p.pos.z
        );
    }

    // ── 2026-09-06 — swept/sub-stepped collision ────────────────────────
    //
    // Player report: "you still go through blocks". `Player::tick` applied the
    // WHOLE per-tick displacement in one shot and resolved once, un-swept, and
    // each axis pass only inspects the single cell column the leading edge
    // lands in. Creative sprint flight is 1.089 b/tick, so the leading edge
    // could land in the cell BEYOND a 1-block wall — the wall was never
    // examined. These tests pin the fix (sub-stepped integration).

    /// Whether the player's hitbox currently overlaps any solid block cell.
    /// Shrunk by an epsilon so resting flush against a face doesn't count.
    fn hitbox_in_solid(p: &Player, world: &World, registry: &BlockRegistry) -> bool {
        const EPS: f32 = 1e-3;
        const HALF_W: f32 = 0.3;
        const HEIGHT: f32 = 1.8;
        let x0 = (p.pos.x - HALF_W + EPS).floor() as i32;
        let x1 = (p.pos.x + HALF_W - EPS).floor() as i32;
        let y0 = (p.pos.y + EPS).floor() as i32;
        let y1 = (p.pos.y + HEIGHT - EPS).floor() as i32;
        let z0 = (p.pos.z - HALF_W + EPS).floor() as i32;
        let z1 = (p.pos.z + HALF_W - EPS).floor() as i32;
        (x0..=x1).any(|bx| {
            (y0..=y1).any(|by| (z0..=z1).any(|bz| world.is_solid(bx, by, bz, registry)))
        })
    }

    /// A 1-block-thick stone floor at y=63 with nothing beneath it, so a
    /// tunnelling fall keeps going instead of being caught by a second layer.
    fn thin_floor_world() -> (World, BlockRegistry) {
        let mut world = World::new();
        let registry = BlockRegistry::new();
        for x in -3..=3 {
            for z in -3..=3 {
                world.set_block(x, 63, z, block::STONE);
            }
        }
        (world, registry)
    }

    #[test]
    fn creative_sprint_flight_cannot_pass_a_one_block_wall() {
        // The headline regression. Sprint flight (1.089 b/tick) at the 1-thick
        // wall at z=-2, from several sub-cell start offsets along the travel
        // axis — whether the old code happened to sample the wall cell at all
        // depended on the fractional part of the position.
        //
        // The wall cell z=-2 spans world z ∈ [-2.0, -1.0]; approached from +Z
        // its near face is z=-1.0, so a collided player rests at z = -0.7.
        for start_z in [0.0f32, 0.25, 0.5, 0.75] {
            let (world, registry, cam) = wall_setup();
            let mut p = Player::new(Vec3::new(0.5, 65.0, start_z));
            p.flying = true;
            let sprint_forward = PlayerIntent {
                move_forward: 1.0,
                sprint: true,
                ..Default::default()
            };
            for tick in 0..40 {
                p.tick(&sprint_forward, &cam, &world, &registry, PlayMode::Creative);
                assert!(
                    !hitbox_in_solid(&p, &world, &registry),
                    "start_z={start_z} tick={tick}: hitbox inside a solid block at {:?}",
                    p.pos
                );
                assert!(
                    p.pos.z >= -0.85,
                    "start_z={start_z} tick={tick}: sprint flight passed the wall, pos.z={}",
                    p.pos.z
                );
            }
        }
    }

    #[test]
    fn sprint_jump_does_not_tunnel_through_a_wall() {
        // Survival sprint-jump peaks around 0.6 b/tick — inside the old
        // single-cell budget, but only just and with nothing guarding it.
        // Sub-stepping makes it structurally safe: the horizontal barrier is
        // never skipped, the player is stopped flush at z = -0.7 (the wall
        // cell z=-2 spans [-2,-1], so its near face is -1.0) and the Z
        // velocity is zeroed rather than carried through the block.
        //
        // NOTE: this deliberately stops at the point the SEPARATE Y-resolution
        // vault bug used to take over — see
        // `sprint_jump_cannot_vault_a_two_block_wall` below, which now carries
        // the full arc.
        let (world, registry, cam) = wall_setup();
        let mut p = Player::new(Vec3::new(0.5, 64.0, 0.0));
        let idle = PlayerIntent::default();
        for _ in 0..5 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        let sprint_jump = PlayerIntent {
            move_forward: 1.0,
            sprint: true,
            jump_held: true,
            jump_pressed: true,
            ..Default::default()
        };
        // Ticks 0..=5 cover the approach, the impact and the flush rest — the
        // whole window in which tunnelling could occur.
        for tick in 0..=5 {
            p.tick(&sprint_jump, &cam, &world, &registry, PlayMode::Survival);
            assert!(
                !hitbox_in_solid(&p, &world, &registry),
                "tick={tick}: hitbox inside a solid block at {:?}",
                p.pos
            );
            assert!(
                p.pos.z >= -0.85,
                "tick={tick}: sprint-jump tunnelled past the wall face, pos.z={}",
                p.pos.z
            );
        }
        assert!(
            (p.pos.z - -0.7).abs() < 1e-3,
            "should come to rest flush against the wall face, pos.z={}",
            p.pos.z
        );
        assert_eq!(p.velocity.z, 0.0, "the wall must zero the Z velocity");
    }

    /// REGRESSION (fixed 2026-09-06, swept landing). The defect this pins:
    ///
    /// Survival, hold sprint+forward+jump into the 2-block wall at z=-2. The
    /// player rests flush at z=-0.7 with `velocity.z == 0`; the next tick
    /// `tick_survival` accelerates from zero to `velocity.z ≈ -0.0051`, so the
    /// hitbox overlaps the wall column by ~5 mm. Meanwhile the jump arc has the
    /// feet at y≈65.26 — inside the wall's UPPER cell (y=65) — and falling.
    /// `resolve_collisions` runs Y before X/Z (Spec 05 §1.4) and the falling
    /// branch used to accept ANY box in the foot's cell with `pos.y < b.max[1]`,
    /// however far above the foot that top was. The 5 mm horizontal overlap —
    /// which the X/Z passes were about to undo — therefore read as ground and
    /// the player was snapped from y=65.26 to y=66.0, onto the wall top, after
    /// which nothing blocked them (traced: z ran to -2.03 by tick 13).
    ///
    /// The fix is the swept landing rule: land only if the foot was at or above
    /// the box top BEFORE the sub-move (or was already inside it — see
    /// `a_player_already_inside_a_block_is_still_pushed_out`).
    #[test]
    fn sprint_jump_cannot_vault_a_two_block_wall() {
        let (world, registry, cam) = wall_setup();
        let mut p = Player::new(Vec3::new(0.5, 64.0, 0.0));
        let idle = PlayerIntent::default();
        for _ in 0..5 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        let sprint_jump = PlayerIntent {
            move_forward: 1.0,
            sprint: true,
            jump_held: true,
            jump_pressed: true,
            ..Default::default()
        };
        // A jump from y=64 peaks at y=65.32 (0.42 impulse, 0.08 gravity), so
        // 65.4 is a hair above the apex and well below the wall top at y=66.
        const APEX_CEILING: f32 = 65.4;
        for tick in 0..120 {
            p.tick(&sprint_jump, &cam, &world, &registry, PlayMode::Survival);
            assert!(
                !hitbox_in_solid(&p, &world, &registry),
                "tick={tick}: hitbox inside a solid block at {:?}",
                p.pos
            );
            assert!(
                p.pos.z >= -0.85,
                "tick={tick}: sprint-jump clipped through the wall, pos.z={}",
                p.pos.z
            );
            assert!(
                p.pos.y <= APEX_CEILING,
                "tick={tick}: sprint-jump rose above the jump apex — vaulted onto \
                 the wall — pos.y={}",
                p.pos.y
            );
        }
        // ...and, once the input is released, is still on the near side,
        // resting on the FLOOR (jump_held bounces them continuously, so this
        // has to settle first or it samples a random point of the arc).
        for _ in 0..20 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(p.on_ground, "should end grounded, y={}", p.pos.y);
        assert!(
            (p.pos.y - 64.0).abs() < 1e-3,
            "should end on the floor top y=64, got {}",
            p.pos.y
        );
    }

    #[test]
    fn a_player_already_inside_a_block_is_still_pushed_out() {
        // The swept-landing rule's deliberate exception. A player whose hitbox
        // is ALREADY intersecting a solid at the start of a sub-move (spawned
        // inside terrain, a block placed on them, a teleport) keeps the old
        // unconditional push-out to the box top — the swept rule would
        // otherwise leave them stuck sinking through the world.
        let mut world = World::new();
        let registry = BlockRegistry::new();
        for x in -3..=3 {
            for z in -3..=3 {
                world.set_block(x, 63, z, block::STONE);
                world.set_block(x, 64, z, block::STONE); // a second layer to be buried in
            }
        }
        let cam = Camera::new(Vec3::new(0.5, 65.0, 0.5), 1.0);
        // Foot at 64.3 — a third of the way INTO the upper layer's cell.
        let mut p = Player::new(Vec3::new(0.5, 64.3, 0.5));
        assert!(
            hitbox_in_solid(&p, &world, &registry),
            "precondition: the player must start inside a solid"
        );
        p.tick(&PlayerIntent::default(), &cam, &world, &registry, PlayMode::Survival);
        assert!(
            (p.pos.y - 65.0).abs() < 1e-3,
            "a buried player must be pushed out onto the block top y=65, got {}",
            p.pos.y
        );
        assert!(p.on_ground, "and be grounded there");
        assert!(
            !hitbox_in_solid(&p, &world, &registry),
            "and no longer be inside a solid, pos={:?}",
            p.pos
        );
    }

    #[test]
    fn landing_on_a_step_from_above_still_works() {
        // Two halves, both of which the swept rule must leave alone.
        //
        // (a) Walk off a 1-block ledge: the drop to the floor below must land
        //     normally — a plain falling landing, foot above the box top before
        //     the sub-move.
        // (b) Sprint-JUMP up onto that same 1-block step from the low side: the
        //     player clears y=65 on the way up (the X pass stops scanning the
        //     step's cell once the foot is above it), drifts over it, and lands
        //     on top at 65.0 on the way down.
        //
        // NOTE there is no auto step-up in this engine — `STEP_HEIGHT` is dead
        // code and `bare_step_without_rail_still_blocks` pins that a 1-block
        // step WALKED into stays a wall. Only rail ramps assist (see
        // `walks_up_a_rail_ramp_smoothly`). So (b) has to be a real jump.
        let mut world = World::new();
        let registry = BlockRegistry::new();
        for x in -8..=60 {
            for z in -5..=5 {
                world.set_block(x, 63, z, block::STONE);
            }
        }
        // A 1-block step starting at x=2 (its -X face is x=2.0, top y=65.0),
        // running well past anything the test walks/jumps to.
        for x in 2..=60 {
            for z in -5..=5 {
                world.set_block(x, 64, z, block::STONE);
            }
        }
        let mut cam = Camera::new(Vec3::new(0.0, 64.0, 0.0), 1.0);
        cam.yaw = 0.0; // move_right = +X

        // (a) start on TOP of the step and walk -X off its edge at x=2.0.
        let mut p = Player::new(Vec3::new(5.5, 65.0, 0.5));
        let walk_left = PlayerIntent { move_right: -1.0, ..Default::default() };
        for _ in 0..40 {
            p.tick(&walk_left, &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(
            p.pos.x < 1.7,
            "should have walked off the ledge, x={}",
            p.pos.x
        );
        assert!(p.on_ground, "should have landed after the 1-block drop");
        assert!(
            (p.pos.y - 64.0).abs() < 1e-3,
            "should land on the lower floor top y=64, got {}",
            p.pos.y
        );

        // (b) sprint-jump back up onto the step from the low side.
        let mut p = Player::new(Vec3::new(0.0, 64.0, 0.5));
        let idle = PlayerIntent::default();
        for _ in 0..5 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        let sprint_jump_right = PlayerIntent {
            move_right: 1.0,
            sprint: true,
            jump_held: true,
            jump_pressed: true,
            ..Default::default()
        };
        for tick in 0..60 {
            p.tick(&sprint_jump_right, &cam, &world, &registry, PlayMode::Survival);
            assert!(
                !hitbox_in_solid(&p, &world, &registry),
                "tick={tick}: hitbox inside a solid block at {:?}",
                p.pos
            );
        }
        // Let the arc settle so the final assertion isn't taken mid-jump.
        for _ in 0..20 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(p.on_ground, "should have settled, y={}", p.pos.y);
        assert!(
            p.pos.x > 2.0,
            "a 1-block step must still be jumpable, x={} y={}",
            p.pos.x,
            p.pos.y
        );
        assert!(
            (p.pos.y - 65.0).abs() < 1e-3,
            "should be standing on the step top y=65, got {}",
            p.pos.y
        );
    }

    #[test]
    fn jumping_beside_a_wall_lands_back_on_the_floor() {
        // Jump straight up while stood flush against the 2-block wall: the wall
        // is beside the hitbox for the whole arc, so a lenient Y pass could
        // grab it as ground. The player must come straight back down to the
        // floor, never onto the wall.
        let (world, registry, cam) = wall_setup();
        // Flush against the wall face at z=-1.0 (hitbox half-width 0.3).
        let mut p = Player::new(Vec3::new(0.5, 64.0, -0.7));
        let idle = PlayerIntent::default();
        for _ in 0..5 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
        }
        assert!(p.on_ground, "precondition: grounded beside the wall");
        // Keep LEANING on the wall for the whole arc — that re-acceleration
        // from a dead stop is what puts the ~5 mm of hitbox inside the wall
        // column each tick, which is the thing the Y pass must not read as
        // ground. ONE jump though: holding jump bounces forever and the arc
        // would never settle for the final assertion.
        let lean = PlayerIntent { move_forward: 1.0, ..Default::default() };
        let jump = PlayerIntent { jump_held: true, jump_pressed: true, ..lean.clone() };
        p.tick(&jump, &cam, &world, &registry, PlayMode::Survival);
        let mut highest = p.pos.y;
        for tick in 0..40 {
            p.tick(&lean, &cam, &world, &registry, PlayMode::Survival);
            highest = highest.max(p.pos.y);
            assert!(
                !hitbox_in_solid(&p, &world, &registry),
                "tick={tick}: hitbox inside a solid block at {:?}",
                p.pos
            );
            assert!(
                p.pos.z >= -0.85,
                "tick={tick}: pushed into the wall, pos.z={}",
                p.pos.z
            );
        }
        assert!(
            highest <= 65.4,
            "must never rise above the jump apex (wall top is 66), highest={highest}"
        );
        assert!(
            (p.pos.y - 64.0).abs() < 1e-3,
            "must land back on the floor top y=64, got {}",
            p.pos.y
        );
    }

    #[test]
    fn long_fall_lands_on_a_one_block_floor() {
        // ~200-block drop onto a 1-thick floor: must land ON it, never below.
        let (world, registry) = thin_floor_world();
        let cam = Camera::new(Vec3::new(0.5, 264.0, 0.5), 1.0);
        let mut p = Player::new(Vec3::new(0.5, 264.0, 0.5));
        let idle = PlayerIntent::default();
        let mut lowest = p.pos.y;
        for _ in 0..600 {
            p.tick(&idle, &cam, &world, &registry, PlayMode::Survival);
            lowest = lowest.min(p.pos.y);
        }
        assert!(p.on_ground, "should have landed, y={}", p.pos.y);
        assert!(
            (p.pos.y - 64.0).abs() < 1e-3,
            "should rest on the floor top y=64, got {}",
            p.pos.y
        );
        assert!(
            lowest >= 64.0 - 1e-3,
            "must never dip below the floor top, lowest={lowest}"
        );
    }

    #[test]
    fn a_fast_fall_is_stopped_by_substepping_not_the_speed_cap() {
        // Drive the integrator directly with a fall velocity far above
        // MAX_FALL_SPEED (which `tick_survival` would otherwise clamp), to
        // prove it is the SUB-STEPPING, not the cap, that keeps a fast fall
        // out of the floor. Un-substepped this is 65.5 − 3.0 = 62.5, whose
        // foot cell (62) is air — straight through the 1-block floor at y=63.
        let (world, registry) = thin_floor_world();
        let mut p = Player::new(Vec3::new(0.5, 65.5, 0.5));
        p.velocity = Vec3::new(0.0, -3.0, 0.0);
        assert!(
            p.velocity.y.abs() > MAX_FALL_SPEED,
            "precondition: the test velocity must exceed the feel cap"
        );

        p.integrate_substepped(&world, &registry);

        assert!(p.on_ground, "should have landed, y={}", p.pos.y);
        assert!(
            (p.pos.y - 64.0).abs() < 1e-3,
            "should rest on the floor top y=64, got {}",
            p.pos.y
        );
        assert_eq!(p.velocity.y, 0.0, "landing must zero the fall velocity");
    }

    #[test]
    fn substep_count_matches_speed() {
        assert_eq!(substep_count(0.0), 1, "standing still takes one pass");
        assert_eq!(substep_count(0.3), 1);
        assert_eq!(substep_count(0.4), 1, "exactly MAX_STEP still fits in one");
        assert_eq!(substep_count(0.41), 2);
        assert_eq!(substep_count(1.089), 3, "creative sprint flight");
        // Degenerate inputs must not produce 0 sub-steps (no move at all) or
        // an unbounded loop.
        assert_eq!(substep_count(f32::NAN), 1);
        assert_eq!(substep_count(f32::INFINITY), MAX_SUBSTEPS);
        assert_eq!(substep_count(1.0e9), MAX_SUBSTEPS);
    }

    #[test]
    fn survival_force_clears_flight() {
        // Survival must force-clear flying even if it was pre-set.
        let (world, registry, cam) = wall_setup();
        let mut p = Player::new(Vec3::new(0.5, 70.0, 0.0));
        p.flying = true;
        p.tick(&PlayerIntent::default(), &cam, &world, &registry, PlayMode::Survival);
        assert!(!p.flying, "survival must force-clear flight");
    }

    #[test]
    fn adventure_force_clears_flight() {
        // Adventure (grounded mode) must also force-clear flying.
        let (world, registry, cam) = wall_setup();
        let mut p = Player::new(Vec3::new(0.5, 70.0, 0.0));
        p.flying = true;
        p.tick(&PlayerIntent::default(), &cam, &world, &registry, PlayMode::Adventure);
        assert!(!p.flying, "adventure must force-clear flight");
    }

    #[test]
    fn spectator_always_flies() {
        // Spectator must have flying=true even if it started false.
        let (world, registry, cam) = wall_setup();
        let mut p = Player::new(Vec3::new(0.5, 70.0, 0.0));
        p.flying = false;
        p.tick(&PlayerIntent::default(), &cam, &world, &registry, PlayMode::Spectator);
        assert!(p.flying, "spectator must always fly");
    }
}

