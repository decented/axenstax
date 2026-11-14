//! What a held item (or an empty hand) does to the MOB under the crosshair: one
//! pure predicate per mob action, shared by the right-click chain's arms and by
//! the hand rule ([`crate::hands`]) the way [`crate::tool_uses`] does it for tools.
//!
//! The question the hand rule asks is "does a hand really do something to this
//! mob?". A mob wins the click only when the answer is yes: feeding it its food,
//! taming it, a Lead, a bucket on a ready cow, shears on a ready sheep, riding
//! or saddling a steed, trading with a villager, taking a Lead off a tethered
//! mob, a command to your own tamed pet. A hostile mob (the brigand, the
//! marauder, the berserker...), or a mob that nothing held does anything to,
//! never takes the click, so a shield can be raised at the enemy in front of you.
//!
//! The chain's arms ask these functions for the part of their test that is
//! about the species and the held item; what a predicate cannot see stays the
//! arm's own refusal (a baby's cooldown is [`MobFacts::can_breed`], plain values
//! the caller reads off its ECS or off the joiner's mirror). Callers:
//! [`crate::local_mob_click::plan_right_click`] (Lead, feed, milk, shear), the
//! villager arm and `mob_interact::tame_companion` in `game_loop`, and the
//! joiner's [`crate::remote_mobs::MirrorTarget::right_click_action`].
//!
//! Errs toward "the mob has a use": a steed another seat is riding still counts
//! as ridable, so the click is left to the chain, which refuses it.

use crate::item::{Item, MaterialId};
use crate::mob::{MobCategory, MobType};

/// What the click's caller knows about the mob under the crosshair, as plain
/// values (local play reads them off its ECS, a joiner off its mirror).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MobFacts {
    pub kind: MobType,
    /// An adult off its breeding cooldown and not already in love: the state
    /// `mob_interact::feed` accepts food in.
    pub can_breed: bool,
    /// `mob_interact::product_ready`: the cow's milk, the sheep's wool.
    pub product_ready: bool,
    /// On a Lead (anyone's).
    pub tethered: bool,
    /// Tamed by anyone (a wild mob can still be tamed).
    pub tamed: bool,
    /// Tamed by this player: its owner, so an empty hand commands it.
    pub own_pet: bool,
}

impl MobFacts {
    /// A mob of `kind` in no particular state: wild, adult, ready, untied.
    pub fn of(kind: MobType) -> Self {
        Self { kind, can_breed: true, product_ready: true, tethered: false, tamed: false, own_pet: false }
    }
}

/// A Lead fastens a passive mob (`mob_interact::lead_attach`).
pub fn lead_attaches(kind: MobType) -> bool {
    crate::mob::mob_def(kind).category == MobCategory::Passive
}

/// `m` is the breeding food of `kind` and the mob can take it now
/// (`mob_interact::feed`). The horse family feeds only while sneaking.
pub fn feeds(kind: MobType, can_breed: bool, sneak: bool, m: MaterialId) -> bool {
    can_breed && crate::breeding::breeding_food(kind) == Some(m) && crate::breeding::breeding_feed_allowed(kind, sneak)
}

/// A bucket on a cow whose milk is ready (`mob_interact::milk`).
pub fn milks(kind: MobType, product_ready: bool) -> bool {
    kind == MobType::Cow && product_ready
}

/// Shears on a sheep whose wool is ready (`mob_interact::shear`).
pub fn shears(kind: MobType, product_ready: bool) -> bool {
    crate::tool_uses::shears_cut(kind) && product_ready
}

/// The companion species' taming food: raw fish for a cat (or a Cat Treat),
/// seeds for a parrot, berries for a fox (`mob_interact::tame_companion`).
pub fn companion_food(kind: MobType, m: MaterialId) -> bool {
    (kind == MobType::Cat && m == MaterialId::CatTreat) || crate::companion::tame_food(kind) == Some(m)
}

/// `m` tames this untamed mob: a Bone on a wolf, Mixed Berries on a Nostrich,
/// a companion's food (`mob_interact::tame_*`). An already-tamed mob takes none.
pub fn tames_with(kind: MobType, tamed: bool, m: MaterialId) -> bool {
    !tamed
        && match kind {
            MobType::Wolf => m == MaterialId::Bone,
            MobType::Nostrich => m == MaterialId::Berries,
            k => companion_food(k, m),
        }
}

/// A villager or a peddler talks and trades (the villager arm).
pub fn trades(kind: MobType) -> bool {
    matches!(kind, MobType::Villager | MobType::Peddler)
}

/// An empty hand commands your own tamed pet: a wolf or a Nostrich sits or
/// follows, a companion steps through its commands (`mob_interact::sit_toggle`).
pub fn commands_pet(kind: MobType, own_pet: bool) -> bool {
    own_pet && (matches!(kind, MobType::Wolf | MobType::Nostrich) || crate::companion::is_companion_species(kind))
}

/// Does a click on this mob do something whatever the hands hold: a steed is
/// mounted (a pack equipped from a chest is the same click), a villager trades,
/// a tethered mob has its Lead taken off. These arms read no item (the detach
/// reads only that it is not a Lead; a Lead's own use is [`lead_attaches`]).
pub fn takes_the_click_whatever_is_held(f: &MobFacts) -> bool {
    crate::mob::is_rideable(f.kind) || trades(f.kind) || f.tethered
}

/// Does `item` (a hand's item) do something to this mob, in the chain's order:
/// a Lead, breeding food, a bucket, shears, taming food. The empty hand is
/// [`commands_pet`] and the unconditional arms [`takes_the_click_whatever_is_held`].
pub fn item_works_on(item: &Item, f: &MobFacts, sneak: bool) -> bool {
    match item {
        Item::Material(m) => {
            (*m == MaterialId::Lead && lead_attaches(f.kind))
                || feeds(f.kind, f.can_breed, sneak, *m)
                || (*m == MaterialId::Bucket && milks(f.kind, f.product_ready))
                || tames_with(f.kind, f.tamed, *m)
        }
        Item::Tool(t) => t.tool_type == crate::crafting::ToolType::Shears && shears(f.kind, f.product_ready),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crafting::{Tool, ToolMaterial, ToolType};

    fn mat(m: MaterialId) -> Item {
        Item::Material(m)
    }

    #[test]
    fn a_hostile_mob_takes_nothing_a_hand_holds() {
        for kind in [MobType::Brigand, MobType::Marauder, MobType::Berserker] {
            let f = MobFacts::of(kind);
            assert!(!takes_the_click_whatever_is_held(&f), "{kind:?}");
            for m in [MaterialId::Wheat, MaterialId::Lead, MaterialId::Bucket, MaterialId::Bone, MaterialId::Bread] {
                assert!(!item_works_on(&mat(m), &f, true), "{kind:?} {m:?}");
            }
            let shears = Item::Tool(Tool::new(ToolType::Shears, ToolMaterial::Iron));
            assert!(!item_works_on(&shears, &f, false), "{kind:?} shears");
            assert!(!commands_pet(kind, false));
        }
    }

    #[test]
    fn each_mob_action_answers_for_its_own_species_and_state() {
        let cow = MobFacts::of(MobType::Cow);
        assert!(item_works_on(&mat(MaterialId::Wheat), &cow, false), "breeding food");
        assert!(item_works_on(&mat(MaterialId::Lead), &cow, false), "a Lead on a passive mob");
        assert!(item_works_on(&mat(MaterialId::Bucket), &cow, false), "a ready cow's milk");
        assert!(!item_works_on(&mat(MaterialId::Carrot), &cow, false), "not a cow's food");
        assert!(!item_works_on(&mat(MaterialId::Wheat), &MobFacts { can_breed: false, ..cow }, false), "a calf or one in love");
        assert!(!item_works_on(&mat(MaterialId::Bucket), &MobFacts { product_ready: false, ..cow }, false), "milked a moment ago");
        let shears = Item::Tool(Tool::new(ToolType::Shears, ToolMaterial::Iron));
        assert!(!item_works_on(&shears, &cow, false), "shears do nothing to a cow");
        let sheep = MobFacts::of(MobType::Sheep);
        assert!(item_works_on(&shears, &sheep, false));
        assert!(!item_works_on(&shears, &MobFacts { product_ready: false, ..sheep }, false), "shorn");
        // Taming food, for an untamed mob only.
        let wolf = MobFacts::of(MobType::Wolf);
        assert!(item_works_on(&mat(MaterialId::Bone), &wolf, false));
        assert!(!item_works_on(&mat(MaterialId::Bone), &MobFacts { tamed: true, ..wolf }, false));
        assert!(item_works_on(&mat(MaterialId::RawFish), &MobFacts::of(MobType::Cat), false));
        assert!(item_works_on(&mat(MaterialId::CatTreat), &MobFacts::of(MobType::Cat), false));
    }

    #[test]
    fn a_steed_a_villager_and_a_tethered_mob_take_the_click_whatever_is_held() {
        assert!(takes_the_click_whatever_is_held(&MobFacts::of(MobType::Horse)));
        assert!(takes_the_click_whatever_is_held(&MobFacts::of(MobType::Donkey)));
        assert!(takes_the_click_whatever_is_held(&MobFacts::of(MobType::Villager)));
        assert!(takes_the_click_whatever_is_held(&MobFacts::of(MobType::Peddler)));
        assert!(takes_the_click_whatever_is_held(&MobFacts { tethered: true, ..MobFacts::of(MobType::Cow) }));
        assert!(!takes_the_click_whatever_is_held(&MobFacts::of(MobType::Cow)));
    }

    #[test]
    fn an_empty_hand_commands_only_your_own_tamed_pet() {
        assert!(commands_pet(MobType::Wolf, true));
        assert!(commands_pet(MobType::Cat, true));
        assert!(!commands_pet(MobType::Wolf, false), "wild or someone else's");
        assert!(!commands_pet(MobType::Cow, true), "a cow is no pet");
    }
}
