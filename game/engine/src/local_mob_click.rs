//! A local player's right-click on a mob in its own sim (single-player, a
//! host's own seats): milking, shearing and taming a companion — the
//! inventory side of `mob_interact`'s rules (review D2b B1).
//!
//! These run in the per-player FRAME loop (`game_loop`, ~60 Hz), so each one
//! acts only on a frame that is a right-click the hand may use
//! ([`right_click_ready`]: the place gesture pressed, the cursor captured, off
//! the 8-tick place cooldown) — exactly the gate the feed and Lead branches
//! beside them have always had. Before the fix they had none: a bucket held
//! on a cow in the crosshair milked it every frame without a click, shears
//! sheared, and companion food rolled a tame (and was eaten) every frame.
//!
//! A refused milk or shear does not eat the click ([`MobClick::eats_click`]):
//! the caller falls through to the next interaction, so filling a bucket at
//! water beside a recently milked cow works (review D2b B1 residual).
//!
//! Toasts, audio, particles, challenge events and the cooldown stay with the
//! caller, which owns them.

use crate::item::{Item, MaterialId};
use crate::mob::MobType;
use crate::mob_interact::Interaction;
use crate::player_intent::PlayerIntent;
use crate::player_slot::PlayerSlot;

/// Is this frame a right-click the player's hand may act on: the place
/// gesture pressed, the cursor captured (no menu open), and off the place
/// cooldown a previous right-click set.
pub fn right_click_ready(intent: &PlayerIntent, slot: &PlayerSlot) -> bool {
    intent.place_block && intent.cursor_captured && slot.place_cooldown == 0
}

/// A right-click that reached a mob: which, what species, and what happened.
pub struct MobClick {
    pub target: hecs::Entity,
    pub kind: MobType,
    pub interaction: Interaction,
}

impl MobClick {
    /// Did this click do something that uses it up? A refusal (the cow isn't
    /// ready, the wool is growing back) does not: the click falls through to
    /// the next interaction exactly as if the mob were not there (review D2b
    /// B1 residual) — a bucket aimed at water beside a cow just milked still
    /// fills. The caller shows the refusal's toast but sets no cooldown and
    /// does not skip the rest of the right-click chain.
    pub fn eats_click(&self) -> bool {
        self.interaction.done
    }
}

/// Animals Wave 2 — a bucket on a cow, or shears on a sheep, in the
/// crosshair (`mob_interact::milk` / `shear`). On a `clicked` frame only.
/// When it happened, what it used comes out of the hand and its products go
/// into the inventory (any overflow drops at the player's feet). A refusal
/// (the cow isn't ready, the wool is growing back) is returned too, for its
/// toast, and takes nothing; it does not eat the click
/// ([`MobClick::eats_click`]).
pub fn harvest(
    ecs: &mut hecs::World,
    slot: &mut PlayerSlot,
    clicked: bool,
    tick: u64,
) -> Option<MobClick> {
    if !clicked {
        return None;
    }
    let hot = slot.hotbar_slot;
    let held = slot.inventory.hotbar_slot(hot).map(|s| s.item.clone());
    let eye = slot.player.eye_pos();
    let look_dir = slot.camera.forward();
    let (target, Some(kind)) = crate::combat::find_attack_target(ecs, eye, look_dir)? else {
        return None;
    };
    let interaction = crate::mob_interact::milk(ecs, target, kind, held.as_ref(), tick)
        .or_else(|| crate::mob_interact::shear(ecs, target, kind, held.as_ref(), tick))?;
    if interaction.done {
        if interaction.consume > 0
            && let Some(Item::Material(m)) = held
        {
            slot.inventory.consume_one_material(hot, m);
        }
        for stack in interaction.give.iter().cloned() {
            if let Some(leftover) = slot.inventory.add_item(stack) {
                crate::entity::spawn_item(ecs, slot.player.pos, leftover, tick as u32);
            }
        }
    }
    Some(MobClick { target, kind, interaction })
}

/// Companions wave — a Cat / Parrot / Fox (…) in the crosshair offered its
/// food (`mob_interact::tame_companion`; a Cat Treat always tames a Cat).
/// On a `clicked` frame only. The food goes on a roll either way, as in
/// single-player; an already-tamed companion doesn't take it (`None`).
pub fn tame_companion(
    ecs: &mut hecs::World,
    slot: &mut PlayerSlot,
    pidx: usize,
    clicked: bool,
    tick: u64,
) -> Option<MobClick> {
    if !clicked {
        return None;
    }
    let hot = slot.hotbar_slot;
    let mat: MaterialId = match slot.inventory.hotbar_slot(hot).map(|s| &s.item) {
        Some(Item::Material(m)) => *m,
        _ => return None,
    };
    let eye = slot.player.eye_pos();
    let look_dir = slot.camera.forward();
    let (target, Some(kind)) = crate::combat::find_attack_target(ecs, eye, look_dir)? else {
        return None;
    };
    let key = crate::tameable::local_owner_key(pidx);
    let actor = crate::mob_interact::Actor {
        owner_key: Some(&key),
        tether: crate::tether::TetherTarget::Player(pidx),
        who: crate::combat::Attacker::Local(pidx),
    };
    let held = Item::Material(mat);
    let interaction = crate::mob_interact::tame_companion(ecs, target, kind, Some(&held), &actor, tick)?;
    slot.inventory.consume_one_material(hot, mat);
    Some(MobClick { target, kind, interaction })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::ItemStack;
    use glam::Vec3;

    /// Player 0 at the origin looking straight down -z (yaw 0), holding
    /// `stack` in hotbar slot 0, and a `kind` 1.5 blocks in front of its eye.
    fn facing(kind: MobType, stack: ItemStack) -> (hecs::World, PlayerSlot, hecs::Entity) {
        let mut slot = PlayerSlot::new(0, Vec3::new(0.5, 64.0, 0.5), 1.0);
        slot.camera.yaw = 0.0;
        slot.camera.pitch = 0.0;
        slot.hotbar_slot = 0;
        slot.inventory.set_slot(0, Some(stack));
        let mut ecs = hecs::World::new();
        let eye = slot.player.eye_pos();
        let mob = crate::entity::spawn_mob(&mut ecs, kind, Vec3::new(eye.x, 64.0, eye.z - 1.5));
        (ecs, slot, mob)
    }

    fn count(slot: &PlayerSlot) -> u8 {
        slot.inventory.hotbar_slot(0).map_or(0, |s| s.count)
    }

    /// Review D2b B1 — holding the crosshair on a ready cow with a bucket for
    /// many frames without a click milks nothing and keeps the bucket; the
    /// click does it once.
    #[test]
    fn a_bucket_held_on_a_cow_without_a_click_milks_nothing() {
        let (mut ecs, mut slot, _cow) = facing(MobType::Cow, ItemStack::new_material(MaterialId::Bucket, 1));
        for frame in 0..300 {
            assert!(harvest(&mut ecs, &mut slot, false, frame).is_none());
        }
        assert_eq!(count(&slot), 1, "no click: the bucket stays");
        assert_eq!(slot.inventory.count_material(MaterialId::MilkBucket), 0);
        let click = harvest(&mut ecs, &mut slot, true, 300).expect("the click reaches the cow");
        assert!(click.interaction.done);
        assert_eq!(slot.inventory.count_material(MaterialId::Bucket), 0);
        assert_eq!(slot.inventory.count_material(MaterialId::MilkBucket), 1);
    }

    /// Review D2b B1 — companion food held on a wild cat for many frames
    /// without a click rolls no tame and eats nothing.
    #[test]
    fn companion_food_held_without_a_click_is_never_eaten() {
        let (mut ecs, mut slot, cat) = facing(MobType::Cat, ItemStack::new_material(MaterialId::RawFish, 16));
        for frame in 0..300 {
            assert!(tame_companion(&mut ecs, &mut slot, 0, false, frame).is_none());
        }
        assert_eq!(count(&slot), 16, "no click: no food eaten");
        assert!(crate::tameable::pet_owner_of(&ecs, cat).is_none(), "and no tame");
        let click = tame_companion(&mut ecs, &mut slot, 0, true, 300).expect("the click reaches the cat");
        assert_eq!(click.interaction.consume, 1);
        assert_eq!(count(&slot), 15, "the click's roll eats one");
    }

    /// The gate is the other branches' gate: pressed, captured, off cooldown.
    #[test]
    fn a_right_click_is_ready_only_pressed_captured_and_off_cooldown() {
        let mut slot = PlayerSlot::new(0, Vec3::ZERO, 1.0);
        let intent = PlayerIntent { place_block: true, cursor_captured: true, ..Default::default() };
        assert!(right_click_ready(&intent, &slot));
        assert!(!right_click_ready(&PlayerIntent { place_block: false, ..intent.clone() }, &slot));
        assert!(!right_click_ready(&PlayerIntent { cursor_captured: false, ..intent.clone() }, &slot));
        slot.place_cooldown = 3;
        assert!(!right_click_ready(&intent, &slot));
    }

    /// Review D2b B1 residual — a cow just milked refuses a second bucket, and
    /// the refusal does not eat the click: nothing is taken, the caller sets
    /// no cooldown, and the click falls through (to the bucket fill at water
    /// beside the cow). The click that milks it does eat the click.
    #[test]
    fn a_refused_milk_or_shear_does_not_eat_the_click() {
        let (mut ecs, mut slot, _cow) = facing(MobType::Cow, ItemStack::new_material(MaterialId::Bucket, 2));
        let first = harvest(&mut ecs, &mut slot, true, 100).expect("the cow is ready");
        assert!(first.interaction.done && first.eats_click(), "milking uses the click up");
        let again = harvest(&mut ecs, &mut slot, true, 101).expect("the cow is in the cone: refused");
        assert!(!again.interaction.done && !again.eats_click(), "not ready: the click falls through");
        assert_eq!(slot.inventory.count_material(MaterialId::Bucket), 1, "the refusal took nothing");
        assert_eq!(slot.inventory.count_material(MaterialId::MilkBucket), 1, "and gave nothing");
        assert_eq!(slot.place_cooldown, 0, "harvest sets no cooldown: that is the caller's, on a done click");

        let (mut ecs, mut slot, _sheep) = facing(MobType::Sheep, ItemStack::new_tool(crate::crafting::Tool::new(
                crate::crafting::ToolType::Shears,
                crate::crafting::ToolMaterial::Iron,
            )));
        let shorn = harvest(&mut ecs, &mut slot, true, 100).expect("the sheep is ready");
        assert!(shorn.eats_click());
        let regrowing = harvest(&mut ecs, &mut slot, true, 101).expect("refused: wool growing back");
        assert!(!regrowing.eats_click());
    }
}
