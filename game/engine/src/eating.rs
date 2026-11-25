//! Eating you can see (Spec 05 §6.4.2, Spec 03 §5.10 "Eating").
//!
//! Presentation only. The rules are untouched: a right-click with food eats ONE
//! item at once (`PlayerSlot::try_eat_for`, or `send_eat_request` for a joiner)
//! and arms the 0.8 s eat cooldown. This module is the picture of that bite:
//! for the length of the cooldown the hand brings the food up to the mouth,
//! chews, and the food loses three bites, each a bite-shaped notch cut out of
//! its icon (the sprite is rebuilt from the icon's texels with the bitten ones
//! left out), with a few crumbs in the food's own colours falling from the mouth
//! and a soft munch per bite. When the 0.8 s are up the next item of the stack
//! (if any) is shown full size, and a held right-click starts the next bite.
//!
//! Nothing here touches the wire, the save or the timing: the state is a
//! per-local-player snapshot of the item being eaten (the slot may already be
//! empty or show the next item), the hand, the hotbar slot it was taken from and
//! how long the bite has run. It ends early when the hand switches, the slot
//! changes, the player dies or a screen opens.
//!
//! Other players do not see it: `PlayerState` carries no eating flag, and adding
//! one is a wire change. BRIDGE: an eating bit in the avatar flags, so a peer's
//! avatar raises its arm and loses bites too — replace when the next protocol
//! bump is owner-approved (Spec 05 §6.4.2 "known gap").

use glam::Vec3;

use crate::block::BlockRegistry;
use crate::hands::Hand;
use crate::item_model::{self, HeldItem, Mask};
use crate::protocol::ItemRef;
use crate::mesh::Vertex;
use crate::viewmodel::HandSide;

/// Bites a piece of food loses over one eat.
pub const BITES: usize = 3;
/// How long one eat runs: the eat cooldown, in seconds (16 ticks of 20 Hz).
pub const DURATION_SECS: f32 = crate::item_actions::EAT_COOLDOWN_TICKS as f32 / 20.0;
/// Where in the eat (0..1 of [`DURATION_SECS`]) each bite lands: after the hand
/// has come up, spaced by a chew, leaving the last stretch to lower the hand.
pub const BITE_AT: [f32; BITES] = [0.26, 0.50, 0.74];
/// The hand is fully raised by this progress...
const LIFT_IN: f32 = 0.20;
/// ...and starts coming down at this one.
const LIFT_OUT: f32 = 0.86;
/// A bite's kick (the head of the food jerks into the mouth) lasts this long.
const KICK_SECS: f32 = 0.12;
/// The chew wobble in between, in cycles per second.
const CHEW_HZ: f32 = 6.0;

/// What a bite takes off the ORIGINAL icon, as a share of its opaque texels:
/// the three together leave about a fifth of it.
const BITE_SHARE: [f32; BITES] = [0.28, 0.27, 0.25];
/// Where each bite is taken from, as a fraction of the icon's own bounding box
/// (x from the left, y from the TOP: the held food stands upright with its top
/// toward the mouth). The nearest still-opaque texel to this point is the
/// bite's centre, and the texels nearest to that centre go.
const BITE_AIM: [(f32, f32); BITES] = [(0.30, 0.0), (0.75, 0.0), (0.50, 0.0)];
/// A held thing that is not drawn from an icon (a food block) shrinks by this
/// much of its size per bite instead.
const SHRINK_PER_BITE: f32 = 0.22;
/// Colour of a crumb when the food's icon colours are not known (a pack's art).
const CRUMB_FALLBACK: [f32; 3] = [0.72, 0.52, 0.32];

/// How a cube-look food (an icon that fills its whole tile) is held while it is
/// eaten: a square of the icon, this wide, as thick as it is wide, so it reads
/// as the same small cube the whole item is, but made of texel columns a bite
/// can cut into. The same width as the cube it replaces (`item_model`).
const CUBE_FOOD_WIDTH: f32 = 0.3;

/// The first-person hand moves to the mouth: how far (fractions of the
/// viewmodel's half height) the fist rises and (of its half width) comes in.
const RISE_HALF_HEIGHT: f32 = 0.38;
const COME_IN_HALF_WIDTH: f32 = 0.40;
/// The extra rise a bite's kick and the chew wobble add (half heights).
const KICK_HALF_HEIGHT: f32 = 0.05;
/// The third-person arm's raise at the mouth (radians about the shoulder, the
/// forward-positive angle `shield_pose::GUARD_ARM_ANGLE` uses) and the wobble.
pub const AVATAR_ARM_ANGLE: f32 = 1.6;
const AVATAR_CHEW_ANGLE: f32 = 0.10;
/// How far the raised arm swings in toward the middle of the body, about the
/// vertical axis through its shoulder (radians, at full lift): with the raise it
/// takes the fist from beside the shoulder (0.4 out) to in front of the face, where
/// a straight 0.7-block arm can reach.
pub const AVATAR_ARM_YAW: f32 = 0.55;
/// Where a bite is in the third-person body's model space (feet at the origin,
/// facing -z): in front of the face, at the food's bitten top. The crumbs start
/// here ([`mouth`]) and the fist is placed so the food stands in front of it
/// ([`AVATAR_ARM_ANGLE`], [`AVATAR_ARM_YAW`]).
pub const AVATAR_BITE_HEIGHT: f32 = 1.55;
pub const AVATAR_BITE_FORWARD: f32 = 0.55;

/// One eat in progress for one local player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Eating {
    /// The food as it was when the bite was taken.
    pub held: HeldItem,
    /// The hand that holds it.
    pub hand: Hand,
    /// The hotbar slot it came from (the main hand's: a change of slot ends it).
    pub hotbar_slot: usize,
    /// A joiner's eat: the sequence number of the request it asked the server
    /// for, so the server's refusal of THAT request ends the picture
    /// ([`cancel_refused`]). `None` for an eat that was taken here and then.
    pub request: Option<u32>,
    age: f32,
    landed: usize,
    /// Test hook: a paused eat does not age (the GPU harness holds a frame).
    pub paused: bool,
}

/// The picture of an eat at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EatLook {
    /// Bites the food has lost so far (0..=[`BITES`]).
    pub bites: usize,
    /// How far the hand has come up to the mouth (0 down, 1 at the mouth).
    pub lift: f32,
    /// The chew: a small signed wobble, with the bite's kick in its positive half.
    pub chew: f32,
}

/// What the third-person avatar of the local player needs to draw an eat.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvatarEating {
    pub side: HandSide,
    pub held: HeldItem,
    pub look: EatLook,
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

impl Eating {
    /// A bite has just been taken from `held` in `hand` (hotbar slot
    /// `hotbar_slot` for the main hand).
    pub fn start(held: HeldItem, hand: Hand, hotbar_slot: usize) -> Self {
        Eating { held, hand, hotbar_slot, request: None, age: 0.0, landed: 0, paused: false }
    }

    /// This eat is a joiner's, asked for as request `seq`.
    pub fn asked_as(self, seq: u32) -> Self {
        Eating { request: Some(seq), ..self }
    }

    /// An eat `progress` (0..1) of the way through, with the bites that have
    /// landed by then already taken (the GPU harness, tests).
    #[cfg(test)]
    pub fn at_progress(held: HeldItem, hand: Hand, hotbar_slot: usize, progress: f32) -> Self {
        let age = progress.clamp(0.0, 1.0) * DURATION_SECS;
        Eating { held, hand, hotbar_slot, request: None, age, landed: bites_landed(progress), paused: true }
    }

    /// How far through the 0.8 s the eat is, 0..=1.
    pub fn progress(&self) -> f32 {
        (self.age / DURATION_SECS).clamp(0.0, 1.0)
    }

    /// Whether the eat has run its course.
    pub fn finished(&self) -> bool {
        self.age >= DURATION_SECS
    }

    /// Let `dt` seconds pass. Returns the indices of the bites that landed in
    /// this step (each exactly once, in order), the moment for crumbs and the
    /// munch.
    pub fn advance(&mut self, dt: f32) -> std::ops::Range<usize> {
        if self.paused || dt <= 0.0 {
            return self.landed..self.landed;
        }
        self.age = (self.age + dt).min(DURATION_SECS);
        let before = self.landed;
        self.landed = bites_landed(self.progress());
        before..self.landed
    }

    /// The picture now.
    pub fn look(&self) -> EatLook {
        let p = self.progress();
        let lift = smoothstep(p / LIFT_IN) * (1.0 - smoothstep((p - LIFT_OUT) / (1.0 - LIFT_OUT)));
        // The newest bite's kick: a quick jerk that dies away.
        let kick = (0..self.landed)
            .map(|b| 1.0 - (self.age - BITE_AT[b] * DURATION_SECS) / KICK_SECS)
            .fold(0.0_f32, f32::max)
            .clamp(0.0, 1.0);
        let wobble = (self.age * CHEW_HZ * std::f32::consts::TAU).sin() * 0.35;
        EatLook { bites: self.landed, lift, chew: lift * (kick + wobble * (1.0 - kick)) }
    }

    /// The avatar's view of the eat, for the hand it is in.
    pub fn avatar(&self) -> AvatarEating {
        let side = match self.hand {
            Hand::Main => HandSide::Right,
            Hand::Off => HandSide::Left,
        };
        AvatarEating { side, held: self.held, look: self.look() }
    }
}

/// How many of the [`BITE_AT`] moments `progress` has passed.
fn bites_landed(progress: f32) -> usize {
    BITE_AT.iter().filter(|at| progress >= **at).count()
}

/// The server refused (or passed by) request `seq`, a joiner's eat: end player
/// `pidx`'s picture of it, if that is the eat it asked for. An eat asked for since
/// (a later request) is left alone.
pub fn cancel_refused(eats: &mut std::collections::HashMap<usize, Eating>, pidx: usize, seq: u32) {
    if eats.get(&pidx).is_some_and(|e| e.request == Some(seq)) {
        eats.remove(&pidx);
    }
}

/// Seat `left` has gone and the seats after it shifted down by one: its eat goes
/// with it and the others follow their seats (the eats are keyed by seat index, so
/// an eat must never pass to whoever slides into the index).
pub fn reindex_after_seat_left(eats: &mut std::collections::HashMap<usize, Eating>, left: usize) {
    let kept: Vec<(usize, Eating)> = eats.drain().filter(|(i, _)| *i != left).map(|(i, e)| (if i > left { i - 1 } else { i }, e)).collect();
    eats.extend(kept);
}

/// The two time steps of a rendered frame, from the real time since the last one:
/// `(particles, eating)`. The particle physics step is clamped (a hitch must not
/// teleport crumbs); the eat is not, so it takes the time it takes, not twice as
/// long at 5 frames a second ([`Eating::advance`] caps itself at the end of the eat).
pub fn frame_steps(elapsed: f32) -> (f32, f32) {
    (elapsed.clamp(0.0, PARTICLE_STEP_MAX), elapsed.max(0.0))
}

/// The most one frame ages the particles, in seconds.
pub const PARTICLE_STEP_MAX: f32 = 0.1;

/// Whether an eat ends now, before its time: the hand was switched (the hotbar
/// slot of a main-hand eat changed), the player died, or a screen opened.
pub fn ends_early(eating: &Eating, hotbar_slot: usize, dead: bool, screen_open: bool) -> bool {
    dead || screen_open || (eating.hand == Hand::Main && eating.hotbar_slot != hotbar_slot)
}

// ── What a bite takes off ────────────────────────────────────────────────────

fn texels(mask: &Mask) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (y, row) in mask.iter().enumerate() {
        for x in 0..16 {
            if (row >> x) & 1 == 1 {
                out.push((x, y));
            }
        }
    }
    out
}

fn dist2(a: (usize, usize), b: (i32, i32)) -> i32 {
    let (dx, dy) = (a.0 as i32 - b.0, a.1 as i32 - b.1);
    dx * dx + dy * dy
}

/// The texels bite `bite` removes from `mask`, given `full`, the icon before
/// any bite: those nearest the opaque texel that is nearest the bite's aim
/// point, as many as its share of the ORIGINAL icon (at least one, never more
/// than are left).
fn bite_texels(full: &Mask, mask: &Mask, bite: usize) -> Vec<(usize, usize)> {
    let all = texels(full);
    let left = texels(mask);
    if left.is_empty() {
        return Vec::new();
    }
    let (x0, x1) = (all.iter().map(|t| t.0).min().unwrap_or(0), all.iter().map(|t| t.0).max().unwrap_or(15));
    let (y0, y1) = (all.iter().map(|t| t.1).min().unwrap_or(0), all.iter().map(|t| t.1).max().unwrap_or(15));
    let (ax, ay) = BITE_AIM[bite];
    let aim = ((x0 as f32 + ax * (x1 - x0) as f32).round() as i32, (y0 as f32 + ay * (y1 - y0) as f32).round() as i32);
    let centre = left.iter().copied().min_by_key(|t| (dist2(*t, aim), t.1, t.0)).unwrap_or(left[0]);
    let n = ((all.len() as f32 * BITE_SHARE[bite]).round() as usize).clamp(1, left.len());
    let mut sorted = left;
    let c = (centre.0 as i32, centre.1 as i32);
    sorted.sort_by_key(|t| (dist2(*t, c), t.1, t.0));
    sorted.truncate(n);
    sorted
}

/// `full` with its first `bites` bites taken (a bite is a bite-shaped notch, see
/// [`bite_texels`]); `bites` past [`BITES`] is [`BITES`].
pub fn bitten_mask(full: Mask, bites: usize) -> Mask {
    let mut mask = full;
    for bite in 0..bites.min(BITES) {
        for (x, y) in bite_texels(&full, &mask, bite) {
            mask[y] &= !(1u16 << x);
        }
    }
    mask
}

/// The number of opaque texels of a mask.
#[cfg(test)]
pub fn texel_count(mask: &Mask) -> usize {
    mask.iter().map(|r| r.count_ones() as usize).sum()
}

/// The icon layer an eaten item is drawn from, if it is drawn from one: a
/// material (every food is one, at the moment).
fn icon_layer(item: ItemRef) -> Option<u32> {
    match item {
        ItemRef::Material(id) => crate::item::MaterialId::try_from(id).ok().map(crate::entity_model::material_texture),
        _ => None,
    }
}

thread_local! {
    /// The bitten meshes already built, by icon layer and bites taken. A bite stage
    /// lasts a quarter of a second or more and every frame of it used to enumerate
    /// and sort the icon's texels, cut the mask and triangulate it again.
    static BITTEN_MESHES: std::cell::RefCell<std::collections::HashMap<(u32, usize), Vec<Vertex>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    /// The whole food's vertical extent in the fist (the grip is measured on the
    /// whole item), by icon layer and hand.
    static GRIP_BOUNDS: std::cell::RefCell<std::collections::HashMap<(u32, i8), (f32, f32)>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    /// Test hook: how many bitten meshes were actually built on this thread.
    #[cfg(test)]
    static MESH_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The bitten mesh of icon `layer` after `bites` (1..=[`BITES`]), built once.
fn bitten_icon_mesh(layer: u32, bites: usize) -> Vec<Vertex> {
    BITTEN_MESHES.with(|cache| {
        cache
            .borrow_mut()
            .entry((layer, bites))
            .or_insert_with(|| {
                #[cfg(test)]
                MESH_BUILDS.with(|n| n.set(n.get() + 1));
                build_bitten_icon_mesh(layer, bites)
            })
            .clone()
    })
}

/// The vertical extent `(lowest, highest)` of the WHOLE `held` in the hand `side`
/// (what the sprite grip is measured on, so a bitten food stays where it was in the
/// fist as its bites come off): cached for a food drawn from an icon.
pub fn grip_bounds(held: HeldItem, registry: &BlockRegistry, side: HandSide) -> (f32, f32) {
    let measure = || {
        item_model::held_item_mesh(held, registry, side)
            .iter()
            .fold((f32::MAX, f32::MIN), |a, v| (a.0.min(v.position[1]), a.1.max(v.position[1])))
    };
    match icon_layer(held.item) {
        Some(layer) => GRIP_BOUNDS.with(|cache| *cache.borrow_mut().entry((layer, side.sign() as i8)).or_insert_with(measure)),
        None => measure(),
    }
}

/// The local-space mesh of `held` in the hand `side` after `bites` bites: the
/// same frame and placement as [`item_model::held_item_mesh`], which it is
/// exactly when `bites` is 0.
///
/// A food drawn from an icon (a material) is rebuilt from the icon's texels
/// minus the bitten ones: a flat cut-out sprite centred on the hand when the icon
/// has a clear background, like the held sprite it replaces; a small cube of
/// texel columns, centred on the hand, when the icon fills its tile. Anything
/// else (a food block) is its own mesh shrunk about the hand.
pub fn eaten_item_mesh(held: HeldItem, registry: &BlockRegistry, side: HandSide, bites: usize) -> Vec<Vertex> {
    let bites = bites.min(BITES);
    if bites == 0 {
        return item_model::held_item_mesh(held, registry, side);
    }
    let Some(layer) = icon_layer(held.item) else {
        let scale = (1.0 - SHRINK_PER_BITE * bites as f32).max(0.1);
        let mut verts = item_model::held_item_mesh(held, registry, side);
        for v in &mut verts {
            v.position = (Vec3::from(v.position) * scale).to_array();
        }
        return verts;
    };
    bitten_icon_mesh(layer, bites)
}

/// [`eaten_item_mesh`] of an icon food, built from the texels.
fn build_bitten_icon_mesh(layer: u32, bites: usize) -> Vec<Vertex> {
    let mask = bitten_mask(item_model::layer_mask(layer), bites);
    if item_model::icon_has_clear_background(layer) {
        let quads = item_model::sprite_quads_of(mask, item_model::HELD_MATERIAL_WIDTH, item_model::SPRITE_DEPTH, false);
        // Centred on the hand, like the held sprite it replaces (`hold_pose`
        // places it in the fist).
        item_model::triangles_of(layer, &quads)
    } else {
        item_model::triangles_of(layer, &item_model::sprite_quads_of(mask, CUBE_FOOD_WIDTH, 1.0, false))
    }
}

/// Colours for the crumbs of bite `bite` of `held`: the colours of the texels
/// that bite took off. A short list, never empty.
pub fn crumb_colours(held: ItemRef, bite: usize) -> Vec<[f32; 3]> {
    let Some(layer) = icon_layer(held) else { return vec![CRUMB_FALLBACK] };
    let full = item_model::layer_mask(layer);
    let before = bitten_mask(full, bite);
    let after = bitten_mask(full, bite + 1);
    let mut colours: Vec<[f32; 3]> = texels(&before)
        .into_iter()
        .filter(|(x, y)| (after[*y] >> x) & 1 == 0)
        .filter_map(|(x, y)| item_model::texel_colour(layer, x, y))
        .collect();
    if colours.is_empty() {
        colours.push(CRUMB_FALLBACK);
    }
    colours
}

/// How many crumbs a bite throws.
pub const CRUMBS_PER_BITE: usize = 7;

// ── Where it is drawn ────────────────────────────────────────────────────────

/// The first-person fist's move to the mouth, in the viewmodel's view space
/// (+x right, +y up): rises and comes in toward the middle, with the bite's
/// kick. `half_height` is the viewmodel's half height at the slot depth.
pub fn view_offset(look: EatLook, side: HandSide, half_height: f32, aspect: f32) -> Vec3 {
    let rise = (RISE_HALF_HEIGHT * look.lift + KICK_HALF_HEIGHT * look.chew) * half_height;
    let come_in = COME_IN_HALF_WIDTH * look.lift * half_height * aspect;
    Vec3::new(-side.sign() * come_in, rise, 0.0)
}

/// The third-person arm's angle about the shoulder while eating, blended from
/// the angle it would have anyway (`rest`).
pub fn avatar_arm_angle(look: EatLook, rest: f32) -> f32 {
    rest + (AVATAR_ARM_ANGLE - rest) * look.lift + AVATAR_CHEW_ANGLE * look.chew
}

/// The third-person arm's swing in toward the middle about the vertical axis
/// through its shoulder, for the hand `side` (positive turns the right arm in,
/// negative the left): zero at rest, [`AVATAR_ARM_YAW`] at the mouth.
pub fn avatar_arm_yaw(look: EatLook, side: HandSide) -> f32 {
    side.sign() * AVATAR_ARM_YAW * look.lift
}

/// Where a bite's crumbs start, for a body standing at `feet` looking along
/// `forward` with its eye at `eye`: in first person just in front of and below
/// the eye (the mouth is below the view), in third person where the avatar holds
/// the food up ([`AVATAR_BITE_HEIGHT`], [`AVATAR_BITE_FORWARD`]).
pub fn mouth(first_person: bool, feet: Vec3, eye: Vec3, forward: Vec3) -> Vec3 {
    if first_person {
        eye + forward * 0.45 + Vec3::new(0.0, -0.32, 0.0)
    } else {
        let flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
        feet + Vec3::new(0.0, AVATAR_BITE_HEIGHT, 0.0) + flat * AVATAR_BITE_FORWARD
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::MaterialId;

    fn food(m: MaterialId) -> HeldItem {
        HeldItem::from(ItemRef::Material(m as u16))
    }

    fn beef() -> HeldItem {
        food(MaterialId::CookedBeef)
    }

    fn run(e: &mut Eating, secs: f32) -> Vec<usize> {
        let mut landed = Vec::new();
        let steps = (secs / 0.01).round() as usize;
        for _ in 0..steps {
            landed.extend(e.advance(0.01));
        }
        landed
    }

    #[test]
    fn an_eat_takes_three_bites_in_order_each_once() {
        let mut e = Eating::start(beef(), Hand::Main, 0);
        let landed = run(&mut e, DURATION_SECS + 0.1);
        assert_eq!(landed, vec![0, 1, 2]);
        assert!(e.finished());
        assert_eq!(e.look().bites, 3);
    }

    #[test]
    fn a_big_frame_step_still_lands_every_bite_once() {
        let mut e = Eating::start(beef(), Hand::Main, 0);
        assert_eq!(e.advance(10.0), 0..3);
        assert_eq!(e.advance(10.0), 3..3);
        assert!(e.finished());
    }

    #[test]
    fn bites_are_monotonic_over_time() {
        let mut e = Eating::start(beef(), Hand::Main, 0);
        let mut last = 0;
        for _ in 0..200 {
            e.advance(0.005);
            let b = e.look().bites;
            assert!(b >= last, "bites went {last} -> {b}");
            last = b;
        }
        assert_eq!(last, BITES);
    }

    #[test]
    fn the_hand_comes_up_stays_and_goes_down() {
        let mut e = Eating::start(beef(), Hand::Main, 0);
        assert_eq!(e.look().lift, 0.0);
        e.advance(DURATION_SECS * 0.5);
        assert!(e.look().lift > 0.99, "at the mouth mid-eat: {}", e.look().lift);
        e.advance(DURATION_SECS);
        assert!(e.look().lift < 0.01, "down at the end: {}", e.look().lift);
        for step in 0..=100 {
            let l = Eating::at_progress(beef(), Hand::Main, 0, step as f32 / 100.0).look().lift;
            assert!((0.0..=1.0).contains(&l));
        }
    }

    #[test]
    fn a_paused_eat_does_not_age_and_lands_nothing() {
        let mut e = Eating::at_progress(beef(), Hand::Main, 0, 0.5);
        let before = e;
        assert_eq!(e.advance(1.0), 2..2);
        assert_eq!(e, before);
    }

    #[test]
    fn the_progress_constructor_has_the_bites_that_landed() {
        for (p, bites) in [(0.0, 0), (0.25, 0), (0.30, 1), (0.60, 2), (0.80, 3), (1.0, 3)] {
            assert_eq!(Eating::at_progress(beef(), Hand::Main, 0, p).look().bites, bites, "at {p}");
        }
    }

    #[test]
    fn every_bite_takes_texels_and_the_mask_only_shrinks() {
        for id in 0..=u16::MAX {
            let Ok(m) = MaterialId::try_from(id) else { continue };
            if !crate::item::Item::Material(m).is_food() {
                continue;
            }
            let layer = crate::entity_model::material_texture(m);
            let full = item_model::layer_mask(layer);
            let total = texel_count(&full);
            assert!(total > 0, "{m:?} has no opaque texels");
            let mut prev = full;
            let mut prev_n = total;
            for bites in 1..=BITES {
                let now = bitten_mask(full, bites);
                let n = texel_count(&now);
                assert!(n < prev_n || prev_n == 0, "{m:?}: bite {bites} took nothing ({prev_n} -> {n})");
                for y in 0..16 {
                    assert_eq!(now[y] & !prev[y], 0, "{m:?}: bite {bites} grew the food back");
                }
                prev = now;
                prev_n = n;
            }
            assert!(prev_n * 100 <= total * 40 || total < 6, "{m:?}: {prev_n} of {total} left after three bites");
        }
    }

    #[test]
    fn a_bite_is_a_notch_at_the_top_not_a_hole() {
        // A solid square icon: the first bite removes a connected blob that
        // touches the top edge, and the bottom row is untouched.
        let full: Mask = [0xFFFF; 16];
        let one = bitten_mask(full, 1);
        assert_eq!(one[15], 0xFFFF, "the bottom row is whole");
        assert_ne!(one[0], 0xFFFF, "the top row has the notch");
        let removed = 256 - texel_count(&one);
        assert!((60..=80).contains(&removed), "a bite is about 28%: {removed}");
        // Every removed texel lies within the notch's radius of its centre
        // (no scattered holes): the removed set is contiguous.
        let gone: Vec<(usize, usize)> = (0..16).flat_map(|y| (0..16).map(move |x| (x, y))).filter(|(x, y)| (one[*y] >> x) & 1 == 0).collect();
        let reach = |a: (usize, usize), b: (usize, usize)| a.0.abs_diff(b.0) <= 1 && a.1.abs_diff(b.1) <= 1;
        let mut seen = vec![gone[0]];
        let mut frontier = vec![gone[0]];
        while let Some(t) = frontier.pop() {
            for g in &gone {
                if !seen.contains(g) && reach(t, *g) {
                    seen.push(*g);
                    frontier.push(*g);
                }
            }
        }
        assert_eq!(seen.len(), gone.len(), "the notch is one piece");
    }

    #[test]
    fn a_tiny_icon_still_loses_a_texel_per_bite() {
        let mut full: Mask = [0; 16];
        full[5] = 0b111;
        let mut left = texel_count(&full);
        for bites in 1..=BITES {
            let n = texel_count(&bitten_mask(full, bites));
            assert!(n < left || left == 0);
            left = n;
        }
    }

    #[test]
    fn zero_bites_is_the_whole_item_exactly() {
        let reg = BlockRegistry::new();
        for held in [beef(), food(MaterialId::Bread), food(MaterialId::Berries), HeldItem::from(ItemRef::Block(1))] {
            assert_eq!(
                eaten_item_mesh(held, &reg, HandSide::Right, 0).len(),
                item_model::held_item_mesh(held, &reg, HandSide::Right).len()
            );
        }
    }

    #[test]
    fn the_mesh_loses_geometry_each_bite_and_never_draws_a_bitten_texel() {
        let reg = BlockRegistry::new();
        for m in [MaterialId::CookedBeef, MaterialId::Bread, MaterialId::Berries, MaterialId::Cake, MaterialId::Stew] {
            let layer = crate::entity_model::material_texture(m);
            let held = food(m);
            let side_walls = |bites: usize| -> usize {
                eaten_item_mesh(held, &reg, HandSide::Right, bites).chunks_exact(3).filter(|t| Vec3::from(t[0].normal).z.abs() > 0.5).count()
            };
            // Faces front and back: one quad per row-run of the mask, so a bitten
            // mesh has fewer face triangles than rows x runs of the whole one
            // would only if texels went; at least it is never empty before bite 3
            // and every vertex is on the icon's layer with the cutout bit.
            for bites in 1..=BITES {
                let mesh = eaten_item_mesh(held, &reg, HandSide::Right, bites);
                let mask = bitten_mask(item_model::layer_mask(layer), bites);
                assert_eq!(mesh.is_empty(), texel_count(&mask) == 0, "{m:?} at {bites}");
                assert!(mesh.iter().all(|v| v.tex_layer == layer | crate::mesh::CUTOUT_FLAG), "{m:?}: wrong layer");
                // A front face centre must land on a masked texel.
                for tri in mesh.chunks_exact(3) {
                    if Vec3::from(tri[0].normal).z < 0.5 {
                        continue;
                    }
                    let uv = [0, 1, 2].map(|i| tri[i].uv).iter().fold([0.0f32; 2], |a, u| [a[0] + u[0] / 3.0, a[1] + u[1] / 3.0]);
                    let (x, y) = (((uv[0] * 16.0) as usize).min(15), ((uv[1] * 16.0) as usize).min(15));
                    assert!((mask[y] >> x) & 1 == 1, "{m:?} at {bites}: a front face covers the bitten texel ({x}, {y})");
                }
            }
            assert!(side_walls(1) > 0, "{m:?} draws nothing at one bite");
        }
    }

    #[test]
    fn a_food_block_shrinks_per_bite() {
        let reg = BlockRegistry::new();
        let held = HeldItem::from(ItemRef::Block(1));
        let extent = |bites| {
            eaten_item_mesh(held, &reg, HandSide::Right, bites).iter().map(|v| v.position[1].abs()).fold(0.0_f32, f32::max)
        };
        assert!(extent(1) < extent(0) && extent(2) < extent(1) && extent(3) < extent(2));
        assert!(extent(3) > 0.0);
    }

    #[test]
    fn crumbs_are_the_colours_of_the_bitten_texels() {
        for bite in 0..BITES {
            let colours = crumb_colours(beef().item, bite);
            assert!(!colours.is_empty());
            for c in &colours {
                assert!(c.iter().all(|v| (0.0..=1.0).contains(v)));
            }
        }
        // Not food from an icon: the fallback crumb.
        assert_eq!(crumb_colours(ItemRef::Block(1), 0), vec![CRUMB_FALLBACK]);
    }

    #[test]
    fn a_bite_throws_crumbs() {
        assert!(CRUMBS_PER_BITE >= 5);
        let mut system = crate::particles::ParticleSystem::new();
        system.burst_crumbs(Vec3::ZERO, &crumb_colours(beef().item, 0), CRUMBS_PER_BITE, 7);
        assert_eq!(system.live(), CRUMBS_PER_BITE);
    }

    #[test]
    fn the_eat_ends_early_on_slot_change_death_or_a_screen_but_not_otherwise() {
        let main = Eating::start(beef(), Hand::Main, 3);
        assert!(!ends_early(&main, 3, false, false));
        assert!(ends_early(&main, 4, false, false), "slot changed");
        assert!(ends_early(&main, 3, true, false), "died");
        assert!(ends_early(&main, 3, false, true), "a screen opened");
        // The off-hand is not tied to a hotbar slot.
        let off = Eating::start(beef(), Hand::Off, 3);
        assert!(!ends_early(&off, 5, false, false));
        assert!(ends_early(&off, 5, true, false));
    }

    #[test]
    fn the_fist_rises_and_comes_in_and_the_hands_mirror() {
        let look = Eating::at_progress(beef(), Hand::Main, 0, 0.5).look();
        let r = view_offset(look, HandSide::Right, 1.0, 16.0 / 9.0);
        let l = view_offset(look, HandSide::Left, 1.0, 16.0 / 9.0);
        assert!(r.y > 0.3, "rises: {r}");
        assert!(r.x < 0.0 && l.x > 0.0, "both come in toward the middle: {r} {l}");
        assert!((r.x + l.x).abs() < 1e-6 && (r.y - l.y).abs() < 1e-6);
        assert_eq!(view_offset(Eating::start(beef(), Hand::Main, 0).look(), HandSide::Right, 1.0, 1.0), Vec3::ZERO);
    }

    #[test]
    fn the_avatar_arm_goes_to_the_mouth_and_back() {
        let down = Eating::start(beef(), Hand::Main, 0).look();
        let up = Eating::at_progress(beef(), Hand::Main, 0, 0.5).look();
        assert_eq!(avatar_arm_angle(down, 0.0), 0.0);
        assert!((avatar_arm_angle(up, 0.0) - AVATAR_ARM_ANGLE).abs() < 0.15);
    }

    #[test]
    fn the_mouth_is_in_front_of_the_face() {
        let (feet, eye, fwd) = (Vec3::new(10.0, 64.0, 10.0), Vec3::new(10.0, 65.6, 10.0), Vec3::new(0.0, 0.0, -1.0));
        let fp = mouth(true, feet, eye, fwd);
        assert!(fp.z < eye.z && fp.y < eye.y);
        let tp = mouth(false, feet, eye, fwd);
        assert!((tp.y - (64.0 + AVATAR_BITE_HEIGHT)).abs() < 0.01 && (feet.z - tp.z - AVATAR_BITE_FORWARD).abs() < 0.01);
    }

    #[test]
    fn a_refused_eat_ends_only_the_eat_that_asked() {
        let mut eats = std::collections::HashMap::new();
        eats.insert(0, Eating::start(beef(), Hand::Main, 0).asked_as(7));
        eats.insert(1, Eating::start(beef(), Hand::Main, 0));
        cancel_refused(&mut eats, 0, 6);
        assert!(eats.contains_key(&0), "another request's refusal leaves it");
        cancel_refused(&mut eats, 1, 7);
        assert!(eats.contains_key(&1), "an eat taken here has no request to refuse");
        cancel_refused(&mut eats, 0, 7);
        assert!(!eats.contains_key(&0), "its own request's refusal ends it");
        assert!(eats.contains_key(&1));
    }

    #[test]
    fn a_seat_leaving_takes_its_eat_and_the_others_follow_their_seats() {
        let eat = |slot| Eating::start(beef(), Hand::Main, slot);
        let mut eats = std::collections::HashMap::new();
        eats.insert(0, eat(3));
        eats.insert(1, eat(3));
        eats.insert(3, eat(5));
        reindex_after_seat_left(&mut eats, 0);
        // Player 1's slot is player 0 now and keeps ITS food; player 0's is gone.
        assert_eq!(eats.len(), 2);
        assert_eq!(eats[&0].hotbar_slot, 3);
        assert_eq!(eats[&2].hotbar_slot, 5);
        assert!(!eats.contains_key(&1) && !eats.contains_key(&3));
        // The last seat leaving disturbs no one.
        let mut eats = std::collections::HashMap::from([(0, eat(1)), (1, eat(2))]);
        reindex_after_seat_left(&mut eats, 1);
        assert_eq!(eats.len(), 1);
        assert_eq!(eats[&0].hotbar_slot, 1);
    }

    #[test]
    fn a_slow_frame_does_not_stretch_the_eat() {
        // At 5 frames a second the particle step is clamped to 0.1 s and the eat's is not.
        let (particles, eating) = frame_steps(0.2);
        assert_eq!((particles, eating), (PARTICLE_STEP_MAX, 0.2));
        assert_eq!(frame_steps(0.016), (0.016, 0.016));
        assert_eq!(frame_steps(-1.0), (0.0, 0.0));
        let mut e = Eating::start(beef(), Hand::Main, 0);
        let mut secs = 0.0;
        while !e.finished() {
            e.advance(frame_steps(0.2).1);
            secs += 0.2;
        }
        assert!(secs <= DURATION_SECS + 0.2 + 1e-4, "an 0.8 s eat took {secs} s at 5 fps");
    }

    #[test]
    fn the_arm_swings_in_to_the_mouth_and_the_hands_mirror() {
        let up = Eating::at_progress(beef(), Hand::Main, 0, 0.5).look();
        let down = Eating::start(beef(), Hand::Main, 0).look();
        let (r, l) = (avatar_arm_yaw(up, HandSide::Right), avatar_arm_yaw(up, HandSide::Left));
        assert!((r - AVATAR_ARM_YAW).abs() < 0.01 && (r + l).abs() < 1e-6, "{r} {l}");
        assert_eq!(avatar_arm_yaw(down, HandSide::Right), 0.0);
    }

    fn same_mesh(a: &[Vertex], b: &[Vertex]) -> bool {
        bytemuck::cast_slice::<Vertex, u8>(a) == bytemuck::cast_slice::<Vertex, u8>(b)
    }

    #[test]
    fn a_bitten_mesh_is_built_once_per_food_and_bite_stage() {
        let reg = BlockRegistry::new();
        let builds = || MESH_BUILDS.with(std::cell::Cell::get);
        let layer = crate::entity_model::material_texture(MaterialId::CookedBeef);
        let first = eaten_item_mesh(beef(), &reg, HandSide::Right, 2);
        assert_eq!(builds(), 1);
        for _ in 0..50 {
            assert!(same_mesh(&eaten_item_mesh(beef(), &reg, HandSide::Right, 2), &first), "the same mesh, every frame");
        }
        assert_eq!(builds(), 1, "fifty more frames at the same bite built nothing");
        assert!(same_mesh(&first, &build_bitten_icon_mesh(layer, 2)), "and it is the mesh the texels give");
        eaten_item_mesh(beef(), &reg, HandSide::Right, 3);
        assert_eq!(builds(), 2, "the next bite is a new mesh");
        // The left hand's food is the same mesh, not another build.
        eaten_item_mesh(beef(), &reg, HandSide::Left, 3);
        assert_eq!(builds(), 2);
        // The grip is measured once per food and hand and is the whole item's.
        let whole = item_model::held_item_mesh(beef(), &reg, HandSide::Right);
        let want = whole.iter().fold((f32::MAX, f32::MIN), |a, v| (a.0.min(v.position[1]), a.1.max(v.position[1])));
        assert_eq!(grip_bounds(beef(), &reg, HandSide::Right), want);
        assert_eq!(grip_bounds(beef(), &reg, HandSide::Right), want);
    }

    #[test]
    fn the_eat_lasts_exactly_the_cooldown() {
        assert!((DURATION_SECS - 0.8).abs() < 1e-6);
        assert!(BITE_AT.windows(2).all(|w| w[0] < w[1]) && BITE_AT[BITES - 1] < LIFT_OUT);
    }
}
