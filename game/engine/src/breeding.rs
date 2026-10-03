//! Animal breeding (P5 gap-closure).
//!
//! Feed two adult animals their breeding food → both enter *love mode* → if
//! two in-love, pairable animals are near each other on the breeding tick, a
//! baby spawns and both parents go on a breeding cooldown. Babies grow into
//! adults after a delay and can't breed until grown. Pairing is normally
//! same-species, but the Horse family has one cross: Horse x Donkey -> Mule
//! ([`pair_allowed`] / [`offspring_kind`], Task 12). Mules are sterile — they
//! have no breeding food ([`breeding_food`]), so they can never enter love
//! mode in the first place.
//!
//! Love/cooldown/baby state here is **ephemeral ECS components** — runtime
//! only, exactly like the mobs themselves (tamed mobs ARE persisted across
//! save/load via `SavedTamedPetData`, see `save.rs`, but this breeding-cycle
//! state is not part of that). The pure pairing/grow logic lives in
//! [`tick_breeding`] so it's unit-testable; the `game_loop` owns the feed
//! right-click and the baby spawn.

use crate::entity::{MobKind, Position};
use crate::item::MaterialId;
use crate::mob::MobType;
use glam::Vec3;

/// How long an animal stays receptive after being fed (30 s @ 20 TPS). Long
/// enough to herd a mate over, short enough that a fed animal doesn't breed
/// hours later.
pub const LOVE_DURATION_TICKS: u64 = 600;

/// Cooldown before an adult can breed again (5 min @ 20 TPS), so a wheat stack
/// can't be spammed into an instant population explosion.
pub const BREED_COOLDOWN_TICKS: u64 = 6000;

/// How long a baby takes to grow into an adult (10 min @ 20 TPS).
pub const BABY_GROW_TICKS: u64 = 12000;

/// How close two in-love animals must be (blocks) to pair into a baby.
pub const BREED_RADIUS: f32 = 8.0;

/// Render scale for a baby relative to the adult model.
pub const BABY_RENDER_SCALE: f32 = 0.6;

/// In love and looking for a mate until `until_tick`.
#[derive(Clone, Copy, Debug)]
pub struct InLove {
    pub until_tick: u64,
}

/// Recently bred (or just born) — can't breed until `until_tick`.
#[derive(Clone, Copy, Debug)]
pub struct BreedCooldown {
    pub until_tick: u64,
}

/// A juvenile that becomes an adult (loses this component) at `adult_at_tick`.
#[derive(Clone, Copy, Debug)]
pub struct Baby {
    pub adult_at_tick: u64,
}

/// Which item breeds this species (Minecraft-style). `None` = not breedable
/// here. Deliberately limited to the core farm animals + the Horse family:
/// the Wolf breeds via taming and the Nostrich reproduces via egg-laying + the
/// Vow, so their feed items (bone/berries) don't double as breeding triggers.
/// The Mule has **no** arm here on purpose — it's the sterile cross-breed
/// offspring (Task 12) and must never itself be fed into love mode.
pub fn breeding_food(kind: MobType) -> Option<MaterialId> {
    Some(match kind {
        MobType::Cow | MobType::Sheep | MobType::Goat => MaterialId::Wheat,
        MobType::Pig | MobType::Rabbit => MaterialId::Carrot,
        MobType::Chicken => MaterialId::WheatSeeds,
        MobType::Horse | MobType::Donkey => MaterialId::Wheat,
        _ => return None,
    })
}

/// Whether a breeding-feed right-click on `kind` should feed it (vs. falling
/// through to whatever else a plain right-click does, e.g. mounting). The
/// Horse family (Horse/Donkey/Mule) is sneak-gated: it's also rideable, so a
/// plain right-click must mount it (pre-wave behaviour) rather than feed it
/// into love mode — sneak + right-click is the deliberate "feed" gesture
/// (Task 5, 2026-07-07). Every other feedable animal is ungated, matching
/// pre-Task-5 behaviour: a plain right-click with the right food feeds it.
pub fn breeding_feed_allowed(kind: MobType, sneak: bool) -> bool {
    !crate::mob::is_horse_family(kind) || sneak
}

/// Whether two in-love animals may pair into a baby: same species, or the one
/// deliberate cross — Horse x Donkey (either order) -> Mule. Everything else
/// (including Horse x Mule or Donkey x Mule, since Mules are sterile and can
/// never even enter love mode) stays same-species-only.
pub fn pair_allowed(a: MobType, b: MobType) -> bool {
    a == b || matches!((a, b), (MobType::Horse, MobType::Donkey) | (MobType::Donkey, MobType::Horse))
}

/// What a pairing produces: the cross pair makes a Mule, everything else
/// (same-species pairs) makes more of that species.
pub fn offspring_kind(a: MobType, b: MobType) -> MobType {
    if a != b {
        MobType::Mule
    } else {
        a
    }
}

/// One breeding tick. Pure w.r.t. its inputs: grows babies, expires love +
/// cooldown windows, and pairs in-love same-species adults within
/// [`BREED_RADIUS`] into babies. Returns `(species, position)` for each baby to
/// spawn — the caller does the ECS spawn so entity creation stays out of the
/// query borrow.
/// A baby produced this tick: where to spawn it + the genetics it inherited
/// from its two parents (Wave 1A). The caller spawns the mob and attaches both
/// the `Baby`/`BreedCooldown` markers and this `Genetics`.
pub struct NewBaby {
    pub kind: MobType,
    pub pos: Vec3,
    pub genetics: crate::genetics::Genetics,
}

pub fn tick_breeding(ecs: &mut hecs::World, tick: u64) -> Vec<NewBaby> {
    // 1. Grow babies that have reached adulthood.
    let grown: Vec<hecs::Entity> = ecs
        .query::<&Baby>()
        .iter()
        .filter(|(_, b)| tick >= b.adult_at_tick)
        .map(|(id, _)| id)
        .collect();
    for id in grown {
        let _ = ecs.remove_one::<Baby>(id);
    }

    // 2. Expire love windows + finished cooldowns (cleanup so stale components
    //    don't accumulate).
    let fell_out: Vec<hecs::Entity> = ecs
        .query::<&InLove>()
        .iter()
        .filter(|(_, l)| tick >= l.until_tick)
        .map(|(id, _)| id)
        .collect();
    for id in fell_out {
        let _ = ecs.remove_one::<InLove>(id);
    }
    let cooled: Vec<hecs::Entity> = ecs
        .query::<&BreedCooldown>()
        .iter()
        .filter(|(_, c)| tick >= c.until_tick)
        .map(|(id, _)| id)
        .collect();
    for id in cooled {
        let _ = ecs.remove_one::<BreedCooldown>(id);
    }

    // 3. Snapshot the in-love adults (babies never carry InLove, so no need to
    //    re-check). Greedily pair same-species lovers within radius.
    // Snapshot lovers WITH their genetics (Wave 1A) — an animal with no
    // Genetics component breeds as the baseline, so legacy/wild stock still works.
    let lovers: Vec<(hecs::Entity, MobType, Vec3, crate::genetics::Genetics)> = ecs
        .query::<(&Position, &MobKind, &InLove, Option<&crate::genetics::Genetics>)>()
        .iter()
        .map(|(id, (p, k, _, g))| (id, k.0, p.0, g.copied().unwrap_or_default()))
        .collect();

    let mut used: Vec<hecs::Entity> = Vec::new();
    let mut babies: Vec<NewBaby> = Vec::new();
    let mut parents: Vec<hecs::Entity> = Vec::new();
    for i in 0..lovers.len() {
        let (id_a, kind_a, pos_a, gen_a) = lovers[i];
        if used.contains(&id_a) {
            continue;
        }
        for &(id_b, kind_b, pos_b, gen_b) in lovers.iter().skip(i + 1) {
            if used.contains(&id_b) || !pair_allowed(kind_a, kind_b) {
                continue;
            }
            if (pos_a - pos_b).length() > BREED_RADIUS {
                continue;
            }
            used.push(id_a);
            used.push(id_b);
            // Deterministic breed seed from tick + the pair's rounded midpoint.
            let mid = (pos_a + pos_b) * 0.5;
            let seed = (tick as u32)
                .wrapping_mul(2_654_435_761)
                ^ (mid.x as i32 as u32).wrapping_mul(40_503)
                ^ (mid.z as i32 as u32).wrapping_mul(73_856_093);
            let genetics = crate::genetics::breed(&gen_a, &gen_b, seed);
            babies.push(NewBaby { kind: offspring_kind(kind_a, kind_b), pos: mid, genetics });
            parents.push(id_a);
            parents.push(id_b);
            break;
        }
    }

    // 4. Parents: drop love, start cooldown.
    for id in parents {
        let _ = ecs.remove_one::<InLove>(id);
        let _ = ecs.insert_one(
            id,
            BreedCooldown {
                until_tick: tick + BREED_COOLDOWN_TICKS,
            },
        );
    }

    babies
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity;

    #[test]
    fn breeding_food_covers_core_farm_animals_only() {
        assert_eq!(breeding_food(MobType::Cow), Some(MaterialId::Wheat));
        assert_eq!(breeding_food(MobType::Pig), Some(MaterialId::Carrot));
        assert_eq!(breeding_food(MobType::Chicken), Some(MaterialId::WheatSeeds));
        // Excluded — own mechanics.
        assert_eq!(breeding_food(MobType::Wolf), None);
        assert_eq!(breeding_food(MobType::Nostrich), None);
        assert_eq!(breeding_food(MobType::Brigand), None);
    }

    #[test]
    fn horse_family_breeding_foods() {
        assert_eq!(breeding_food(MobType::Horse), Some(MaterialId::Wheat));
        assert_eq!(breeding_food(MobType::Donkey), Some(MaterialId::Wheat));
        assert_eq!(breeding_food(MobType::Mule), None, "mules are sterile");
    }

    // Task 5 (2026-07-07) — the pets wave's cross-breeding fix removed the old
    // `breeding_food(Horse) == None` exclusion, but left the game_loop
    // breeding-feed right-click running (and `continue`-ing) BEFORE the mount
    // trigger. Net effect: a plain wheat right-click on your own horse fed it
    // into love mode instead of mounting it, and horses in existing pens could
    // suddenly wheat-breed. `breeding_feed_allowed` is the sneak gate that
    // restores pre-wave mount-first behaviour for the horse family while
    // leaving every other feedable animal untouched.
    #[test]
    fn breeding_feed_allowed_gates_horse_family_by_sneak() {
        // Plain right-click must NOT feed the horse family — game_loop's
        // breeding-feed branch must fall through so the mount trigger below it
        // fires instead (pre-wave behaviour).
        assert!(!breeding_feed_allowed(MobType::Horse, false));
        assert!(!breeding_feed_allowed(MobType::Donkey, false));
        assert!(!breeding_feed_allowed(MobType::Mule, false));
        // Sneak + right-click is the deliberate "feed for love mode" gesture.
        assert!(breeding_feed_allowed(MobType::Horse, true));
        assert!(breeding_feed_allowed(MobType::Donkey, true));
        assert!(breeding_feed_allowed(MobType::Mule, true));
    }

    #[test]
    fn breeding_feed_allowed_is_ungated_for_non_horse_family() {
        // Cows and the rest of the core farm animals feed on a plain
        // right-click exactly as before this task — sneak makes no difference.
        assert!(breeding_feed_allowed(MobType::Cow, false));
        assert!(breeding_feed_allowed(MobType::Cow, true));
        assert!(breeding_feed_allowed(MobType::Pig, false));
        assert!(breeding_feed_allowed(MobType::Chicken, false));
    }

    #[test]
    fn horse_x_donkey_makes_a_mule() {
        let mut ecs = hecs::World::new();
        let a = entity::spawn_mob(&mut ecs, MobType::Horse, Vec3::new(0.0, 64.0, 0.0));
        let b = entity::spawn_mob(&mut ecs, MobType::Donkey, Vec3::new(2.0, 64.0, 0.0));
        let _ = ecs.insert_one(a, InLove { until_tick: 1000 });
        let _ = ecs.insert_one(b, InLove { until_tick: 1000 });
        let babies = tick_breeding(&mut ecs, 10);
        assert_eq!(babies.len(), 1);
        assert_eq!(babies[0].kind, MobType::Mule);
    }

    #[test]
    fn cross_pairing_is_horse_donkey_only() {
        assert!(pair_allowed(MobType::Cow, MobType::Cow));
        assert!(pair_allowed(MobType::Horse, MobType::Donkey));
        assert!(pair_allowed(MobType::Donkey, MobType::Horse));
        assert!(!pair_allowed(MobType::Horse, MobType::Mule));
        assert!(!pair_allowed(MobType::Cow, MobType::Pig));
    }

    #[test]
    fn two_in_love_cows_nearby_make_one_baby() {
        let mut ecs = hecs::World::new();
        let a = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, 0.0));
        let b = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(2.0, 64.0, 0.0));
        let _ = ecs.insert_one(a, InLove { until_tick: 1000 });
        let _ = ecs.insert_one(b, InLove { until_tick: 1000 });
        let babies = tick_breeding(&mut ecs, 10);
        assert_eq!(babies.len(), 1, "one baby from the pair");
        assert_eq!(babies[0].kind, MobType::Cow);
        // Both parents now on cooldown, love cleared.
        assert!(ecs.get::<&BreedCooldown>(a).is_ok());
        assert!(ecs.get::<&BreedCooldown>(b).is_ok());
        assert!(ecs.get::<&InLove>(a).is_err());
        assert!(ecs.get::<&InLove>(b).is_err());
    }

    #[test]
    fn different_species_do_not_pair() {
        let mut ecs = hecs::World::new();
        let a = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, 0.0));
        let b = entity::spawn_mob(&mut ecs, MobType::Pig, Vec3::new(1.0, 64.0, 0.0));
        let _ = ecs.insert_one(a, InLove { until_tick: 1000 });
        let _ = ecs.insert_one(b, InLove { until_tick: 1000 });
        assert!(tick_breeding(&mut ecs, 10).is_empty());
    }

    #[test]
    fn far_apart_lovers_do_not_pair() {
        let mut ecs = hecs::World::new();
        let a = entity::spawn_mob(&mut ecs, MobType::Sheep, Vec3::new(0.0, 64.0, 0.0));
        let b = entity::spawn_mob(&mut ecs, MobType::Sheep, Vec3::new(50.0, 64.0, 0.0));
        let _ = ecs.insert_one(a, InLove { until_tick: 1000 });
        let _ = ecs.insert_one(b, InLove { until_tick: 1000 });
        assert!(tick_breeding(&mut ecs, 10).is_empty());
    }

    #[test]
    fn expired_love_is_cleared_and_does_not_breed() {
        let mut ecs = hecs::World::new();
        let a = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, 0.0));
        let b = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(1.0, 64.0, 0.0));
        let _ = ecs.insert_one(a, InLove { until_tick: 5 });
        let _ = ecs.insert_one(b, InLove { until_tick: 5 });
        // tick 10 is past the until_tick → love expired before pairing.
        assert!(tick_breeding(&mut ecs, 10).is_empty());
        assert!(ecs.get::<&InLove>(a).is_err());
    }

    #[test]
    fn baby_grows_up_after_delay() {
        let mut ecs = hecs::World::new();
        let baby = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, 0.0));
        let _ = ecs.insert_one(baby, Baby { adult_at_tick: 100 });
        // Before the deadline → still a baby.
        let _ = tick_breeding(&mut ecs, 50);
        assert!(ecs.get::<&Baby>(baby).is_ok());
        // At/after the deadline → grows up.
        let _ = tick_breeding(&mut ecs, 100);
        assert!(ecs.get::<&Baby>(baby).is_err(), "baby should have grown up");
    }

    #[test]
    fn cooldown_blocks_immediate_rebreed_via_snapshot() {
        // A parent that just bred has BreedCooldown; even if re-fed it should
        // not be picked (the game_loop gates feeding on cooldown, but confirm
        // the component survives the tick that created it).
        let mut ecs = hecs::World::new();
        let a = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(0.0, 64.0, 0.0));
        let b = entity::spawn_mob(&mut ecs, MobType::Cow, Vec3::new(1.0, 64.0, 0.0));
        let _ = ecs.insert_one(a, InLove { until_tick: 9999 });
        let _ = ecs.insert_one(b, InLove { until_tick: 9999 });
        let _ = tick_breeding(&mut ecs, 10);
        let cd = ecs.get::<&BreedCooldown>(a).expect("cooldown set");
        assert_eq!(cd.until_tick, 10 + BREED_COOLDOWN_TICKS);
    }
}
