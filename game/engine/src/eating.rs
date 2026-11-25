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
pub const AVATAR_ARM_ANGLE: f32 = 1.75;
const AVATAR_CHEW_ANGLE: f32 = 0.10;

/// One eat in progress for one local player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Eating {
    /// The food as it was when the bite was taken.
    pub held: HeldItem,
    /// The hand that holds it.
    pub hand: Hand,
    /// The hotbar slot it came from (the main hand's: a change of slot ends it).
    pub hotbar_slot: usize,
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
        Eating { held, hand, hotbar_slot, age: 0.0, landed: 0, paused: false }
    }

    /// An eat `progress` (0..1) of the way through, with the bites that have
    /// landed by then already taken (the GPU harness, tests).
    #[cfg(test)]
    pub fn at_progress(held: HeldItem, hand: Hand, hotbar_slot: usize, progress: f32) -> Self {
        let age = progress.clamp(0.0, 1.0) * DURATION_SECS;
        Eating { held, hand, hotbar_slot, age, landed: bites_landed(progress), paused: true }
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

/// The local-space mesh of `held` in the hand `side` after `bites` bites: the
/// same frame and placement as [`item_model::held_item_mesh`], which it is
/// exactly when `bites` is 0.
///
/// A food drawn from an icon (a material) is rebuilt from the icon's texels
/// minus the bitten ones: a flat cut-out sprite beside the fist when the icon
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
    let mask = bitten_mask(item_model::layer_mask(layer), bites);
    if item_model::icon_has_clear_background(layer) {
        let quads = item_model::sprite_quads_of(mask, item_model::HELD_MATERIAL_WIDTH, item_model::SPRITE_DEPTH, false);
        let mut verts = item_model::triangles_of(layer, &quads);
        let offset = item_model::sprite_offset(side);
        for v in &mut verts {
            v.position = (Vec3::from(v.position) + offset).to_array();
        }
        verts
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

/// Where a bite's crumbs start, for a body standing at `feet` looking along
/// `forward` with its eye at `eye`: in first person just in front of and below
/// the eye (the mouth is below the view), in third person at the avatar's own
/// mouth.
pub fn mouth(first_person: bool, feet: Vec3, eye: Vec3, forward: Vec3) -> Vec3 {
    if first_person {
        eye + forward * 0.45 + Vec3::new(0.0, -0.32, 0.0)
    } else {
        let flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
        feet + Vec3::new(0.0, 1.45, 0.0) + flat * 0.28
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
        assert!((tp.y - 65.45).abs() < 0.01 && tp.z < feet.z);
    }

    #[test]
    fn the_eat_lasts_exactly_the_cooldown() {
        assert!((DURATION_SECS - 0.8).abs() < 1e-6);
        assert!(BITE_AT.windows(2).all(|w| w[0] < w[1]) && BITE_AT[BITES - 1] < LIFT_OUT);
    }
}
