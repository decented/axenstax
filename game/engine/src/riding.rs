//! Riding — one rule, per tick, for every seat (MP-D2c-1; Spec 05 §5.10,
//! Spec 04 §4.2h).
//!
//! A seat is a player on a mount: a rideable mob (Horse, Donkey, Mule,
//! Nostrich — `mob::is_rideable`) or a rail cart. Whoever simulates the
//! seat runs the same three steps each tick:
//!
//! 1. [`steer`] — the rider's input drives the mount, right before entity
//!    physics integrates its velocity (after every system that writes a
//!    mob's velocity, so nothing overrides the rider): the horse family
//!    gallops along the look's forward/right axes and jumps off the ground;
//!    the Nostrich runs its arcade wind-up (`nostrich_ride::step`, one step
//!    per tick); a cart rolls itself (`cart::tick_carts`). The dismount
//!    gestures are here too: sneak off a horse, sneak off a Nostrich below
//!    [`nostrich_ride::DISMOUNT_SPEED`] (above it sneak drifts), sink off a
//!    Nostrich too slow over water, jump off a cart.
//! 2. Entity physics and the cart roll move the mount.
//! 3. [`follow`] — the rider's body is pinned to the seat, no fall
//!    accruing; a parked cart or a mount that is gone or dead ends the ride.
//!
//! Who runs it: single-player and a host's own seats in `GameState::tick`;
//! a joiner's seat on the server (`GameServer`), steered from its latest
//! input (Spec 04 §4.2h). A ridden mount carries [`Ridden`], which the mob AI
//! and the horse herd AI skip and which keeps a second rider off.

use glam::{Vec2, Vec3};

use crate::block::BlockRegistry;
use crate::entity::{MobKind, OnGround, Position, Ridden, Velocity};
use crate::mob::MobType;
use crate::physics::Player;
use crate::player_intent::PlayerIntent;
use crate::world::World;

/// Horse-family gallop: 6 b/s, per tick.
pub const HORSE_SPEED_PER_TICK: f32 = 6.0 / 20.0;

/// A mount's jump impulse (b/tick) — a player's.
pub const MOUNT_JUMP_VELOCITY: f32 = 0.42;

/// Seconds per tick: the Nostrich's ride physics step.
const TICK_SECS: f32 = 1.0 / 20.0;

/// How far below a dismounted rider [`settle`] looks for footing.
const SETTLE_MAX_DROP: i32 = 8;

/// The ride's own state, carried tick to tick (the Nostrich's wind-up and
/// drifted heading; a horse or cart keeps none). Fresh at each mount.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RideState {
    /// Forward speed, b/s.
    pub speed: f32,
    /// Drifted heading `(x, z)`.
    pub heading: Vec2,
}

/// What a rider asks of its mount this tick: the movement axes, the look's
/// yaw (the camera's, not the body's), sneak and held jump.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeatInput {
    pub forward: f32,
    pub right: f32,
    pub yaw: f32,
    pub sneak: bool,
    pub jump: bool,
}

impl SeatInput {
    /// From a tick's intent and the look it was made with. A jump is held
    /// jump (or this tick's press): the wire carries held jump only.
    pub fn from_intent(intent: &PlayerIntent, yaw: f32) -> Self {
        Self {
            forward: intent.move_forward,
            right: intent.move_right,
            yaw,
            sneak: intent.sneak,
            jump: intent.jump_held || intent.jump_pressed,
        }
    }

    /// The desired horizontal direction (`camera::Camera::horizontal_forward`
    /// × forward + `Camera::right` × right, from `yaw`), not normalised.
    fn direction(&self) -> Vec3 {
        let forward = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        forward * self.forward + right * self.right
    }
}

/// Why a rider gets off at [`steer`] (the ends [`follow`] finds — a parked
/// cart, a mount gone or dead — are its [`Follow`]; a rider's own death ends
/// its ride wherever its seat is held).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dismount {
    /// Sneak (a horse; a Nostrich slowed below its dismount speed).
    Sneak,
    /// A Nostrich too slow over water sank.
    Sank,
    /// A jump off a cart.
    HopOff,
}

/// Why a mount can't take this rider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountRefusal {
    /// Not a rideable mob or a cart (or not there at all, or dead).
    NotRideable,
    /// Someone is already riding it.
    Taken,
}

/// May `mount` take a rider: a living rideable mob, or a cart, that nobody
/// rides.
pub fn may_mount(ecs: &hecs::World, mount: hecs::Entity) -> Result<(), MountRefusal> {
    let rideable = match ecs.get::<&MobKind>(mount) {
        Ok(kind) => {
            crate::mob::is_rideable(kind.0)
                && ecs.get::<&crate::combat::Health>(mount).is_ok_and(|h| !h.is_dead())
        }
        Err(_) => ecs.get::<&crate::cart::CartData>(mount).is_ok(),
    };
    if !rideable {
        return Err(MountRefusal::NotRideable);
    }
    if ecs.get::<&Ridden>(mount).is_ok() {
        return Err(MountRefusal::Taken);
    }
    Ok(())
}

/// Seat a rider on `mount` (checked with [`may_mount`]): it carries
/// [`Ridden`], and the ride starts from a standstill.
pub fn mount(ecs: &mut hecs::World, mount: hecs::Entity, ride: &mut RideState) {
    let _ = ecs.insert_one(mount, Ridden);
    *ride = RideState::default();
}

/// The ride on `mount` is over: it carries no [`Ridden`] (whether or not it
/// is still there).
pub fn unmount(ecs: &mut hecs::World, mount: hecs::Entity, ride: &mut RideState) {
    let _ = ecs.remove_one::<Ridden>(mount);
    *ride = RideState::default();
}

/// Steer `mount` one tick from `input` (before entity physics). `Some`: the
/// rider gets off (the caller [`unmount`]s and [`settle`]s). A mount that is
/// neither a rideable mob nor a cart (gone) is left to [`follow`].
pub fn steer(
    ecs: &mut hecs::World,
    world: &World,
    mount: hecs::Entity,
    input: &SeatInput,
    ride: &mut RideState,
) -> Option<Dismount> {
    let kind = ecs.get::<&MobKind>(mount).map(|k| k.0).ok();
    match kind {
        Some(MobType::Nostrich) => steer_nostrich(ecs, world, mount, input, ride),
        Some(k) if crate::mob::is_rideable(k) => steer_horse(ecs, mount, input),
        // A cart rolls itself; the rider only hops off.
        _ => (input.jump && ecs.get::<&crate::cart::CartData>(mount).is_ok()).then_some(Dismount::HopOff),
    }
}

/// Horse / Donkey / Mule: a steady ~6 b/s gallop where the look points.
fn steer_horse(ecs: &mut hecs::World, mount: hecs::Entity, input: &SeatInput) -> Option<Dismount> {
    if input.sneak {
        return Some(Dismount::Sneak);
    }
    let dir = input.direction().normalize_or_zero();
    let on_ground = ecs.get::<&OnGround>(mount).is_ok_and(|g| g.0);
    if let Ok(mut vel) = ecs.get::<&mut Velocity>(mount) {
        vel.0.x = dir.x * HORSE_SPEED_PER_TICK;
        vel.0.z = dir.z * HORSE_SPEED_PER_TICK;
        if input.jump && on_ground {
            vel.0.y = MOUNT_JUMP_VELOCITY;
        }
    }
    if dir.length_squared() > 1e-6
        && let Ok(mut ai) = ecs.get::<&mut crate::mob_ai::MobAi>(mount)
    {
        ai.facing = dir.z.atan2(dir.x);
    }
    None
}

/// Nostrich: the wind-up, drift and water-skim of `nostrich_ride::step`,
/// one step a tick. Sneak drifts; below the dismount speed it hops you off.
fn steer_nostrich(
    ecs: &mut hecs::World,
    world: &World,
    mount: hecs::Entity,
    input: &SeatInput,
    ride: &mut RideState,
) -> Option<Dismount> {
    if input.sneak && ride.speed < crate::nostrich_ride::DISMOUNT_SPEED {
        return Some(Dismount::Sneak);
    }
    let dir = input.direction();
    let pos = ecs.get::<&Position>(mount).map_or(Vec3::ZERO, |p| p.0);
    let (fx, fy, fz) = (pos.x.floor() as i32, pos.y.floor() as i32, pos.z.floor() as i32);
    // At AND just below the feet, so a bird skimming the surface (its feet on
    // it, the water the cell below) still reads as over water.
    let over_water = world.is_water(fx, fy, fz) || world.is_water(fx, fy - 1, fz);
    let st = crate::nostrich_ride::step(
        ride.speed,
        ride.heading,
        Vec2::new(dir.x, dir.z),
        input.sneak,
        over_water,
        TICK_SECS,
    );
    ride.speed = st.speed;
    ride.heading = st.heading;
    if st.dismount {
        return Some(Dismount::Sank);
    }
    let on_ground = ecs.get::<&OnGround>(mount).is_ok_and(|g| g.0);
    if let Ok(mut vel) = ecs.get::<&mut Velocity>(mount) {
        vel.0.x = st.vel_xz.x * TICK_SECS; // b/s → b/tick
        vel.0.z = st.vel_xz.y * TICK_SECS;
        if st.water_run {
            vel.0.y = 0.0; // skim flat across the surface
        } else if input.jump && on_ground {
            vel.0.y = MOUNT_JUMP_VELOCITY;
        }
    }
    if st.heading.length_squared() > 1e-6
        && let Ok(mut ai) = ecs.get::<&mut crate::mob_ai::MobAi>(mount)
    {
        ai.facing = st.heading.y.atan2(st.heading.x);
    }
    // Skimming: on top of the water surface, so the bird runs across it
    // rather than bobbing into it.
    if st.water_run {
        let mut top = fy;
        while world.is_water(fx, top + 1, fz) {
            top += 1;
        }
        let surface = (top + 1) as f32;
        if let Ok(mut p) = ecs.get::<&mut Position>(mount)
            && p.0.y < surface
        {
            p.0.y = surface;
        }
    }
    None
}

/// The seat after physics moved the mount (see [`follow`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Follow {
    /// Pinned: the rider's eye is here.
    Seated { eye: Vec3 },
    /// Pinned, and the cart has parked: the ride ends here.
    Arrived { eye: Vec3 },
    /// The mount is gone or dead: the ride ends where the body is.
    Gone,
}

/// Pin `body` to the seat of `mount` (after entity physics and the cart
/// roll): its eye at `cart::rider_eye_position`, at rest, off the ground,
/// with no fall accruing.
pub fn follow(ecs: &hecs::World, mount: hecs::Entity, body: &mut Player) -> Follow {
    let dead = ecs.get::<&crate::combat::Health>(mount).is_ok_and(|h| h.is_dead());
    let Some(at) = ecs.get::<&Position>(mount).ok().map(|p| p.0).filter(|_| !dead) else {
        return Follow::Gone;
    };
    let eye = pin(at, body);
    let parked = ecs.get::<&crate::cart::CartData>(mount).is_ok_and(|c| c.speed <= 0.0);
    if parked { Follow::Arrived { eye } } else { Follow::Seated { eye } }
}

/// Pin `body` to the seat over a mount at `mount_pos`; returns the eye. The
/// eye height is the body's own (`eye_pos().y - pos.y`); the foot position
/// is back-solved from the seat eye, so the rest of the engine sees a
/// normal foot-anchored player.
pub fn pin(mount_pos: Vec3, body: &mut Player) -> Vec3 {
    let eye_height = body.eye_pos().y - body.pos.y;
    let eye = crate::cart::rider_eye_position(mount_pos, eye_height);
    body.pos = eye - Vec3::new(0.0, eye_height, 0.0);
    body.velocity = Vec3::ZERO;
    body.on_ground = false;
    // A ride is not a fall: getting off must not land one begun before.
    body.reset_fall();
    eye
}

/// Stand a rider who just got off on solid footing: a seated rider sits
/// over its mount's feet (a cart's on the non-solid track), so it would
/// drop through. The first solid block within [`SETTLE_MAX_DROP`] below;
/// over a true void it is left to fall, as off any ledge.
pub fn settle(body: &mut Player, world: &World, registry: &BlockRegistry) {
    let (bx, top, bz) = (body.pos.x.floor() as i32, body.pos.y.floor() as i32, body.pos.z.floor() as i32);
    for dy in 0..=SETTLE_MAX_DROP {
        let by = top - dy;
        if world.is_solid(bx, by, bz, registry) {
            body.pos.y = (by + 1) as f32;
            body.velocity = Vec3::ZERO;
            body.reset_fall();
            body.on_ground = true;
            return;
        }
    }
}

/// The toast a rider sees on getting on (single-player's wording).
pub fn mount_toast(kind: MobType) -> &'static str {
    if kind == MobType::Nostrich {
        "Yeehaw! Hold forward to wind up, hold Crouch to DRIFT, go FAST to skim water. Crouch when stopped to hop off."
    } else {
        "Mounted! WASD to ride, Space to jump, Sneak to dismount."
    }
}

/// The toast a Nostrich rider sees when it sinks them off.
pub const SANK_TOAST: &str = "Too slow — SPLASH! The Nostrich sank and you fell off.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;

    /// A stone floor at y = 63 over `-r..=r`.
    fn floor(r: i32) -> World {
        let mut world = World::new();
        for x in -r..=r {
            for z in -r..=r {
                world.set_block(x, 63, z, block::STONE);
            }
        }
        world
    }

    /// Facing +z: `Camera::horizontal_forward` at yaw π.
    const SOUTH: f32 = std::f32::consts::PI;

    fn ahead(forward: f32) -> SeatInput {
        SeatInput { forward, yaw: SOUTH, ..Default::default() }
    }

    fn vel(ecs: &hecs::World, e: hecs::Entity) -> Vec3 {
        ecs.get::<&Velocity>(e).unwrap().0
    }

    /// The axes are the camera's: forward is `horizontal_forward`, right is
    /// `right`, so a mount goes where a walker would.
    #[test]
    fn the_seat_axes_are_the_cameras() {
        for yaw in [0.0f32, 0.7, SOUTH, -2.1] {
            let mut cam = crate::camera::Camera::new(Vec3::ZERO, 1.0);
            cam.yaw = yaw;
            let f = SeatInput { forward: 1.0, yaw, ..Default::default() }.direction();
            let r = SeatInput { right: 1.0, yaw, ..Default::default() }.direction();
            assert!((f - cam.horizontal_forward()).length() < 1e-6);
            assert!((r - cam.right()).length() < 1e-6);
        }
    }

    /// One steer, one physics step, one follow: the horse gallops where the
    /// look points at 0.3 b/tick and the rider sits on it; sneak gets off and
    /// lands on the floor.
    #[test]
    fn a_horse_gallops_per_tick_carries_its_rider_and_sneak_gets_off() {
        let world = floor(16);
        let reg = BlockRegistry::new();
        let mut ecs = hecs::World::new();
        let horse = crate::entity::spawn_mob(&mut ecs, MobType::Horse, Vec3::new(0.5, 64.0, 0.5));
        let mut ride = RideState::default();
        assert_eq!(may_mount(&ecs, horse), Ok(()));
        mount(&mut ecs, horse, &mut ride);
        assert_eq!(may_mount(&ecs, horse), Err(MountRefusal::Taken), "one rider");
        let mut body = Player::new(Vec3::new(0.5, 64.0, 0.5));
        // Settle the horse on the floor first.
        for _ in 0..5 {
            crate::entity::tick_entities(&mut ecs, &world, &reg);
        }
        for _ in 0..10 {
            let before = ecs.get::<&Position>(horse).unwrap().0;
            assert_eq!(steer(&mut ecs, &world, horse, &ahead(1.0), &mut ride), None);
            assert!((vel(&ecs, horse) - Vec3::new(0.0, vel(&ecs, horse).y, HORSE_SPEED_PER_TICK)).length() < 1e-5);
            crate::entity::tick_entities(&mut ecs, &world, &reg);
            let after = ecs.get::<&Position>(horse).unwrap().0;
            // Physics drags the velocity once before it moves the mount.
            let step = HORSE_SPEED_PER_TICK * crate::entity::DRAG;
            assert!((after.z - before.z - step).abs() < 1e-3, "{before} -> {after}");
            let Follow::Seated { eye } = follow(&ecs, horse, &mut body) else { panic!("seated") };
            assert!((body.pos - (after + Vec3::new(0.0, crate::cart::SEAT_LIFT, 0.0))).length() < 1e-4);
            assert!((eye - body.eye_pos()).length() < 1e-4);
            assert_eq!(body.velocity, Vec3::ZERO);
        }
        let sneak = SeatInput { sneak: true, ..ahead(1.0) };
        assert_eq!(steer(&mut ecs, &world, horse, &sneak, &mut ride), Some(Dismount::Sneak));
        unmount(&mut ecs, horse, &mut ride);
        settle(&mut body, &world, &reg);
        assert!(ecs.get::<&Ridden>(horse).is_err());
        assert_eq!(body.pos.y, 64.0, "on the floor");
        assert!(body.on_ground);
    }

    /// A ridden horse jumps only off the ground.
    #[test]
    fn a_horse_jumps_off_the_ground_only() {
        let world = floor(4);
        let reg = BlockRegistry::new();
        let mut ecs = hecs::World::new();
        let horse = crate::entity::spawn_mob(&mut ecs, MobType::Donkey, Vec3::new(0.5, 64.0, 0.5));
        let mut ride = RideState::default();
        mount(&mut ecs, horse, &mut ride);
        for _ in 0..5 {
            crate::entity::tick_entities(&mut ecs, &world, &reg);
        }
        let jump = SeatInput { jump: true, ..Default::default() };
        steer(&mut ecs, &world, horse, &jump, &mut ride);
        assert_eq!(vel(&ecs, horse).y, MOUNT_JUMP_VELOCITY);
        crate::entity::tick_entities(&mut ecs, &world, &reg);
        steer(&mut ecs, &world, horse, &jump, &mut ride);
        assert!(vel(&ecs, horse).y < MOUNT_JUMP_VELOCITY, "airborne: no second impulse");
    }

    /// D2c-1 — the Nostrich takes ONE ride step a tick, so its wind-up
    /// follows its constants as written: `BASE_SPEED` off the line, then
    /// `ACCEL` b/s² up to `MAX_SPEED` (2 s). (Steered per rendered frame it
    /// wound up about three times faster at 60 fps.)
    #[test]
    fn a_nostrich_winds_up_per_tick_by_its_constants() {
        use crate::nostrich_ride::{ACCEL, BASE_SPEED, MAX_SPEED};
        let world = floor(64);
        let mut ecs = hecs::World::new();
        let bird = crate::entity::spawn_mob(&mut ecs, MobType::Nostrich, Vec3::new(0.5, 64.0, 0.5));
        let mut ride = RideState::default();
        mount(&mut ecs, bird, &mut ride);
        for tick in 1..=60u32 {
            assert_eq!(steer(&mut ecs, &world, bird, &ahead(1.0), &mut ride), None);
            let want = (BASE_SPEED + ACCEL * tick as f32 / 20.0).min(MAX_SPEED);
            assert!((ride.speed - want).abs() < 1e-3, "tick {tick}: {} vs {want}", ride.speed);
            let v = vel(&ecs, bird);
            assert!((Vec2::new(v.x, v.z).length() - want / 20.0).abs() < 1e-4, "b/s → b/tick");
        }
        assert_eq!(ride.speed, MAX_SPEED, "flat out after 2 s");
        // Sneak at speed drifts; it gets you off only once stopped.
        let sneak = SeatInput { sneak: true, ..ahead(1.0) };
        assert_eq!(steer(&mut ecs, &world, bird, &sneak, &mut ride), None, "drift");
        let stopped = SeatInput { sneak: true, ..Default::default() };
        let mut off = None;
        for _ in 0..40 {
            off = steer(&mut ecs, &world, bird, &stopped, &mut ride);
            if off.is_some() {
                break;
            }
        }
        assert_eq!(off, Some(Dismount::Sneak));
    }

    /// Too slow over water: the Nostrich sinks its rider off.
    #[test]
    fn a_slow_nostrich_over_water_sinks_its_rider_off() {
        let mut world = floor(4);
        world.set_block(0, 63, 0, block::WATER);
        let mut ecs = hecs::World::new();
        let bird = crate::entity::spawn_mob(&mut ecs, MobType::Nostrich, Vec3::new(0.5, 64.0, 0.5));
        let mut ride = RideState::default();
        mount(&mut ecs, bird, &mut ride);
        assert_eq!(steer(&mut ecs, &world, bird, &ahead(1.0), &mut ride), Some(Dismount::Sank));
    }

    /// A cart rolls itself: the rider's keys do nothing but a jump, which
    /// hops it off; a parked cart ends the ride where it stands.
    #[test]
    fn a_cart_takes_only_a_jump_and_parking_ends_the_ride() {
        let world = World::new();
        let mut ecs = hecs::World::new();
        let cart = crate::cart::spawn_cart(&mut ecs, (0, 64, 0));
        let mut ride = RideState::default();
        assert_eq!(may_mount(&ecs, cart), Ok(()));
        mount(&mut ecs, cart, &mut ride);
        assert_eq!(steer(&mut ecs, &world, cart, &ahead(1.0), &mut ride), None);
        let jump = SeatInput { jump: true, ..Default::default() };
        assert_eq!(steer(&mut ecs, &world, cart, &jump, &mut ride), Some(Dismount::HopOff));
        let mut body = Player::new(Vec3::new(0.5, 64.0, 0.5));
        assert!(matches!(follow(&ecs, cart, &mut body), Follow::Arrived { .. }), "speed 0 = parked");
        ecs.get::<&mut crate::cart::CartData>(cart).unwrap().speed = crate::cart::CART_SPEED;
        assert!(matches!(follow(&ecs, cart, &mut body), Follow::Seated { .. }));
    }

    /// A mount that died or is gone ends the ride; a dead or unrideable mob
    /// takes no rider.
    #[test]
    fn a_dead_or_vanished_mount_ends_the_ride_and_takes_no_rider() {
        let mut ecs = hecs::World::new();
        let horse = crate::entity::spawn_mob(&mut ecs, MobType::Mule, Vec3::new(0.5, 64.0, 0.5));
        let cow = crate::entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(2.5, 64.0, 0.5));
        assert_eq!(may_mount(&ecs, cow), Err(MountRefusal::NotRideable));
        let mut ride = RideState::default();
        mount(&mut ecs, horse, &mut ride);
        let mut body = Player::new(Vec3::new(0.5, 64.0, 0.5));
        assert!(matches!(follow(&ecs, horse, &mut body), Follow::Seated { .. }));
        ecs.get::<&mut crate::combat::Health>(horse).unwrap().current = 0.0;
        assert_eq!(follow(&ecs, horse, &mut body), Follow::Gone, "dead");
        let _ = ecs.remove_one::<Ridden>(horse);
        assert_eq!(may_mount(&ecs, horse), Err(MountRefusal::NotRideable));
        ecs.despawn(horse).unwrap();
        assert_eq!(follow(&ecs, horse, &mut body), Follow::Gone, "gone");
    }
}
