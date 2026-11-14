//! The Shield (off-hand release, slice 5): one pure rule for every body that
//! can hold one.
//!
//! A shield is a `ToolType::Shield` held in either hand. Holding the use
//! button (right-click, or the touch use button) raises it when the click has
//! no other use ([`crate::hands::item_has_use`]: a chest, a door or a mob under
//! the crosshair still wins). While raised the body walks at sneak speed and
//! does not sprint (`PlayerIntent::guarding`), and from the [`RAISE_TICKS`]-th
//! consecutive raised tick the shield is *effective*: an attack from the front
//! half is fully blocked.
//!
//! [`blocks`] is the one rule. The local player (`PlayerSlot::shield_guard`),
//! a joiner's server body (`ServerPlayer::shield_guard`) and a lent world's
//! host slot all build a [`Guard`] from their own position, look and raised
//! count and ask it. Lava, fire, falling, drowning and starvation carry no
//! attacker and are never asked. No mob disables a shield.
//!
//! **Which attacks.** Hostile melee, a species hit (bee sting, goat charge,
//! shark bite; `SimEvent::Hit` carries the attacker's position) and a blast
//! whose centre is the attacker. Projectiles cannot hit a player in this
//! engine (`entity::tick_projectiles` hits blocks and mobs only) and there is
//! no player-versus-player melee, so those two paths do not exist to hook;
//! [`blocks`] takes any attacker position, so they would use it unchanged.
//!
//! **Effective durability.** Whether a seat can block is read from the *effective*
//! window, never the raw one: the durability a block already owes (a queued
//! `WearShield`, or wear this very tick that is not queued yet) comes off first, so
//! a shield owed broken protects nothing, whether or not its client has said it
//! wore it ([`Guard::durability`], [`crate::combat::PlayerCombat::shield_blocks`]).
//!
//! **Wear and the block window.** A block costs the shield `1 + floor(damage)`
//! durability ([`wear_for`], the damage being the hit before armour and after
//! the difficulty scale). Hostile melee has no attack cooldown (the only rate
//! limit on a hit is the victim's i-frame window), so a block opens its own
//! window of [`BLOCK_WINDOW_TICKS`] (the i-frame length) on the body's
//! `PlayerCombat`: a block wears and clacks once per window, and a front hit
//! inside the window is still blocked, free.

use glam::Vec3;

use crate::hands::Hand;
use crate::item::Item;

/// Consecutive raised ticks before a shield is effective: 0.25 s at 20 TPS.
pub const RAISE_TICKS: u32 = 5;

/// How long a wearing block holds its window open (the i-frame length): a front
/// hit inside it is blocked without wear or sound.
pub const BLOCK_WINDOW_TICKS: u32 = crate::combat::INVINCIBILITY_TICKS;

/// A body that holds a raised shield, as the block rule sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guard {
    /// Where the body stands.
    pub victim: Vec3,
    /// Its look direction; only the horizontal part is read.
    pub look: Vec3,
    /// Consecutive ticks the shield has been raised.
    pub raised_for: u32,
    /// The hand of the shield that guards (the main hand's first,
    /// [`guard_hand`]) and the durability it has EFFECTIVELY: what it holds once
    /// every wear already owed is taken off. A shield with none left protects
    /// nothing, and wear owed from this guard names it by both
    /// ([`ShieldOwed`]).
    pub hand: Hand,
    pub durability: u16,
}

impl Guard {
    /// Does this body's shield stop an attack from `attacker`?
    pub fn blocks(&self, attacker: Vec3) -> bool {
        blocks(attacker, self.victim, self.look, self.raised_for)
    }
}

/// The attacker is within 90 degrees of the victim's horizontal look
/// direction (the front half, exactly 90 degrees excluded). Height is ignored:
/// an attacker above or below in the same column is not "in front", and a
/// victim looking straight up or down has no front.
pub fn in_front(attacker: Vec3, victim: Vec3, look: Vec3) -> bool {
    let to_attacker = Vec3::new(attacker.x - victim.x, 0.0, attacker.z - victim.z);
    let facing = Vec3::new(look.x, 0.0, look.z);
    if to_attacker.length_squared() < 1e-8 || facing.length_squared() < 1e-8 {
        return false;
    }
    to_attacker.normalize().dot(facing.normalize()) > 1e-4
}

/// The one block rule: is the shield effective ([`RAISE_TICKS`] raised ticks
/// or more) and the attacker in the front half?
pub fn blocks(attacker: Vec3, victim: Vec3, look: Vec3, raised_for_ticks: u32) -> bool {
    raised_for_ticks >= RAISE_TICKS && in_front(attacker, victim, look)
}

/// The horizontal look direction of a body that faces `yaw` (the camera's
/// convention: yaw 0 looks toward -Z).
pub fn look_from_yaw(yaw: f32) -> Vec3 {
    Vec3::new(-yaw.sin(), 0.0, -yaw.cos())
}

/// Durability one block costs: `1 + floor(damage)`.
pub fn wear_for(damage: f32) -> u16 {
    if !damage.is_finite() || damage < 0.0 {
        return 1;
    }
    1u16.saturating_add(damage.floor().min(f32::from(u16::MAX - 1)) as u16)
}

/// One more tick of a raise: the count continues while raised, and drops to
/// zero the tick it is released.
pub fn next_raised_ticks(previous: u32, raised: bool) -> u32 {
    if raised { previous.saturating_add(1) } else { 0 }
}

/// Is `item` a shield?
pub fn is_shield(item: Option<&Item>) -> bool {
    matches!(item, Some(Item::Tool(t)) if t.tool_type == crate::crafting::ToolType::Shield)
}

/// The hand whose shield guards, given both hands' items: the main hand's if it
/// holds one, else the off-hand's, else none. One rule for both copies (a
/// client and the server wear the same shield).
pub fn guard_hand(main: Option<&Item>, off: Option<&Item>) -> Option<Hand> {
    if is_shield(main) {
        Some(Hand::Main)
    } else if is_shield(off) {
        Some(Hand::Off)
    } else {
        None
    }
}

/// Durability one block of a raised shield owes, naming the shield by its hand
/// and the durability it had when the block fell (the identity a `WearShield`
/// carries; see [`wear_blocking`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShieldOwed {
    pub hand: Hand,
    pub durability: u16,
    pub amount: u16,
}

/// What a shield did while one body's hits were landed: the durability it lost
/// and the clacks that sounded (one per wearing block).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockTally {
    pub wear: u16,
    pub clacks: u8,
}

/// The shield's side of landing hits on one body: the guard (if a shield is
/// raised) and what it blocked.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShieldCtx {
    pub guard: Option<Guard>,
    pub tally: BlockTally,
}

impl ShieldCtx {
    pub fn with(guard: Option<Guard>) -> Self {
        Self { guard, tally: BlockTally::default() }
    }
}

/// What [`wear_blocking`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WearOutcome {
    /// The shield lost durability; `broke` when it reached 0 and was removed.
    Worn { broke: bool },
    /// That hand holds no shield (a copy that drifted): nothing worn.
    Missing,
}

/// The durability of the shield `item` is, if it is one.
fn shield_durability(item: Option<&crate::item::ItemStack>) -> Option<u16> {
    match item.map(|s| &s.item) {
        Some(Item::Tool(t)) if t.tool_type == crate::crafting::ToolType::Shield => Some(t.durability),
        _ => None,
    }
}

/// The hand that guards given both hands' stacks, with the durability its shield
/// has: [`guard_hand`] plus the number. `None` when neither hand holds a shield.
pub fn guarding(main: Option<&crate::item::ItemStack>, off: Option<&crate::item::ItemStack>) -> Option<(Hand, u16)> {
    let hand = guard_hand(main.map(|s| &s.item), off.map(|s| &s.item))?;
    let durability = shield_durability(match hand {
        Hand::Main => main,
        Hand::Off => off,
    })?;
    Some((hand, durability))
}

/// Where the shield a wear is owed against is NOW, for both copies of a joiner's
/// window: a block named the shield by its `hand` (the main hand names hotbar slot
/// `slot`) and the `durability` it had, and a swap or a window move may have moved
/// it since. The shield with exactly that durability wins, searched across EVERY
/// eligible place (the named hand's place, the other hand's, then the whole
/// inventory), so a shield moved to another slot is still the one that pays and a
/// spare that took its place does not; failing that, the shared hand-aware
/// resolution a swing's wear uses ([`crate::joiner_actions::where_now_hand`]) finds a
/// shield by kind (it is single-tier), the named hand's place first.
///
/// Shields of equal durability are interchangeable: when two hold the same value
/// the first found in that order pays, and both copies of the window find the same
/// one (the one rule), so which of two identical shields pays is not a defect and no
/// lineage is tracked for it.
pub fn locate(
    inventory: &crate::inventory::Inventory,
    offhand: &Option<crate::item::ItemStack>,
    hand: Hand,
    slot: usize,
    durability: u16,
) -> Option<crate::joiner_actions::HeldAt> {
    use crate::joiner_actions::HeldAt;
    let places = match hand {
        Hand::Main => [HeldAt::Slot(slot), HeldAt::Offhand],
        Hand::Off => [HeldAt::Offhand, HeldAt::Slot(slot)],
    };
    let stack = |at: HeldAt| match at {
        HeldAt::Slot(i) => inventory.slot(i),
        HeldAt::Offhand => offhand.as_ref(),
    };
    if let Some(at) = places
        .into_iter()
        .chain((0..36).map(HeldAt::Slot))
        .find(|&at| shield_durability(stack(at)) == Some(durability))
    {
        return Some(at);
    }
    let any = Item::Tool(crate::crafting::Tool::new(crate::crafting::ToolType::Shield, crate::crafting::ToolMaterial::Wood));
    crate::joiner_actions::where_now_hand(inventory, offhand, hand, slot, &any)
}

/// Wear the shield a block was owed against, wherever it is now ([`locate`]), by
/// `n` durability, breaking it at 0 like any tool (`Inventory::use_tool_at`, the
/// cell rule every tool wears by). THE wear rule: a `WearShield` window event
/// applies it on both copies of a joiner's window, the fold that reads a shield's
/// effective durability applies it, and a local body wears the shield it has just
/// blocked with by it, at once.
pub fn wear_blocking(
    inventory: &mut crate::inventory::Inventory,
    slot: usize,
    offhand: &mut Option<crate::item::ItemStack>,
    hand: Hand,
    durability: u16,
    n: u16,
) -> WearOutcome {
    use crate::joiner_actions::HeldAt;
    let Some(at) = locate(inventory, offhand, hand, slot, durability) else {
        return WearOutcome::Missing;
    };
    let mut broke = false;
    for _ in 0..n {
        let info = match at {
            HeldAt::Slot(i) => inventory.use_tool_at(i),
            HeldAt::Offhand => crate::inventory::use_tool_cell(offhand),
        };
        match info {
            Some(info) if info.just_broke => {
                broke = true;
                break;
            }
            Some(_) => {}
            None => break,
        }
    }
    WearOutcome::Worn { broke }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: Vec3 = Vec3::new(0.0, 64.0, 0.0);

    /// Looking toward -Z (yaw 0).
    fn look() -> Vec3 {
        look_from_yaw(0.0)
    }

    /// An attacker `deg` degrees off the look direction, 2 blocks away, at the
    /// victim's height.
    fn at(deg: f32) -> Vec3 {
        let r = deg.to_radians();
        // Rotate the forward (-Z) about Y.
        V + Vec3::new(-r.sin() * 2.0, 0.0, -r.cos() * 2.0)
    }

    #[test]
    fn the_front_arc_is_within_ninety_degrees_of_the_horizontal_look() {
        assert!(in_front(at(0.0), V, look()), "dead ahead");
        assert!(in_front(at(89.0), V, look()), "89 degrees to the side");
        assert!(in_front(at(-89.0), V, look()), "89 degrees to the other side");
        assert!(!in_front(at(91.0), V, look()), "91 degrees is behind the line");
        assert!(!in_front(at(-91.0), V, look()));
        assert!(!in_front(at(180.0), V, look()), "directly behind");
        assert!(!in_front(at(90.0), V, look()), "exactly 90 degrees is not within it");
    }

    #[test]
    fn above_and_below_are_judged_horizontally() {
        // Same column: there is no horizontal direction, so no front.
        assert!(!in_front(V + Vec3::new(0.0, 3.0, 0.0), V, look()), "overhead");
        assert!(!in_front(V - Vec3::new(0.0, 3.0, 0.0), V, look()), "below");
        // In front horizontally but far above or below: still in front.
        assert!(in_front(at(0.0) + Vec3::new(0.0, 5.0, 0.0), V, look()));
        assert!(in_front(at(0.0) - Vec3::new(0.0, 5.0, 0.0), V, look()));
        // Looking straight up or down: pitch is ignored (the rule reads the
        // horizontal part of the look), and a look with no horizontal part has
        // no front.
        assert!(in_front(at(0.0), V, Vec3::new(0.0, 0.3, -1.0)), "a pitched look keeps its heading");
        assert!(!in_front(at(0.0), V, Vec3::Y), "straight up: no front");
    }

    #[test]
    fn the_raise_takes_five_ticks() {
        for ticks in 0..RAISE_TICKS {
            assert!(!blocks(at(0.0), V, look(), ticks), "{ticks} raised ticks: not effective");
        }
        assert!(blocks(at(0.0), V, look(), 5), "5 ticks: effective");
        assert!(blocks(at(0.0), V, look(), 6));
        assert!(!blocks(at(180.0), V, look(), 50), "effective, but from behind");
        // Four and five, as the plan states it.
        assert!(!blocks(at(0.0), V, look(), 4));
    }

    #[test]
    fn a_yaw_turns_the_front() {
        // Yaw 90 degrees looks toward -X (the camera's convention): an attacker
        // at -X is in front, one at +Z (the old front's right side) is not.
        let l = look_from_yaw(std::f32::consts::FRAC_PI_2);
        assert!(in_front(V + Vec3::new(-2.0, 0.0, 0.0), V, l));
        assert!(!in_front(V + Vec3::new(2.0, 0.0, 0.0), V, l));
    }

    #[test]
    fn wear_is_one_plus_the_whole_points_of_damage() {
        assert_eq!(wear_for(0.0), 1);
        assert_eq!(wear_for(0.9), 1);
        assert_eq!(wear_for(1.0), 2);
        assert_eq!(wear_for(3.0), 4);
        assert_eq!(wear_for(3.9), 4);
        assert_eq!(wear_for(f32::NAN), 1, "a non-number costs the base one");
    }

    #[test]
    fn the_raised_count_continues_while_held_and_resets_on_release() {
        let mut n = 0;
        for expected in 1..=6 {
            n = next_raised_ticks(n, true);
            assert_eq!(n, expected);
        }
        assert_eq!(next_raised_ticks(n, false), 0);
    }

    #[test]
    fn the_guarding_hand_is_the_main_hand_first() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let shield = Item::Tool(Tool::new(ToolType::Shield, ToolMaterial::Wood));
        let sword = Item::Tool(Tool::new(ToolType::Sword, ToolMaterial::Iron));
        assert_eq!(guard_hand(Some(&shield), Some(&shield)), Some(Hand::Main));
        assert_eq!(guard_hand(Some(&sword), Some(&shield)), Some(Hand::Off));
        assert_eq!(guard_hand(Some(&shield), None), Some(Hand::Main));
        assert_eq!(guard_hand(Some(&sword), Some(&sword)), None);
        assert_eq!(guard_hand(None, None), None);
    }

    fn shield_stack(durability: u16) -> crate::item::ItemStack {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        crate::item::ItemStack::new_tool(Tool { durability, ..Tool::new(ToolType::Shield, ToolMaterial::Wood) })
    }

    fn durability(stack: Option<&crate::item::ItemStack>) -> Option<u16> {
        match stack.map(|s| &s.item) {
            Some(Item::Tool(t)) => Some(t.durability),
            _ => None,
        }
    }

    #[test]
    fn wear_takes_durability_from_the_named_hand_and_breaks_at_zero() {
        let mut inv = crate::inventory::Inventory::new();
        let mut off = Some(shield_stack(10));
        inv.set_slot(2, Some(shield_stack(100)));
        assert_eq!(wear_blocking(&mut inv, 2, &mut off, Hand::Main, 100, 4), WearOutcome::Worn { broke: false });
        assert_eq!(durability(inv.slot(2)), Some(96));
        assert_eq!(durability(off.as_ref()), Some(10), "the off-hand is untouched");
        assert_eq!(wear_blocking(&mut inv, 2, &mut off, Hand::Off, 10, 9), WearOutcome::Worn { broke: false });
        assert_eq!(durability(off.as_ref()), Some(1));
        assert_eq!(wear_blocking(&mut inv, 2, &mut off, Hand::Off, 1, 5), WearOutcome::Worn { broke: true });
        assert!(off.is_none(), "a shield worn to nothing is gone, as any tool is");
        assert_eq!(durability(inv.slot(2)), Some(96), "the other shield did not pay for it");
    }

    #[test]
    fn wear_finds_no_shield_in_a_hand_that_holds_something_else() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let mut inv = crate::inventory::Inventory::new();
        let mut off = None;
        inv.set_slot(0, Some(crate::item::ItemStack::new_tool(Tool::new(ToolType::Sword, ToolMaterial::Iron))));
        assert_eq!(wear_blocking(&mut inv, 0, &mut off, Hand::Main, 100, 3), WearOutcome::Missing, "a sword is not worn as a shield");
        assert_eq!(wear_blocking(&mut inv, 5, &mut off, Hand::Main, 100, 3), WearOutcome::Missing, "an empty slot");
        assert_eq!(durability(inv.slot(0)), Some(Tool::new(ToolType::Sword, ToolMaterial::Iron).durability), "unchanged");
    }

    /// The blocking shield pays wherever an F swap has put it: a sword now in
    /// the named slot is not worn, and with a shield in each hand the one that
    /// blocked (by its durability) pays, not the one now in the named hand.
    #[test]
    fn a_swap_in_flight_moves_the_wear_with_the_shield() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let sword = || crate::item::ItemStack::new_tool(Tool::new(ToolType::Sword, ToolMaterial::Iron));
        let sword_full = Tool::new(ToolType::Sword, ToolMaterial::Iron).durability;
        // The shield blocked from the main hand (slot 0, durability 100), then F put
        // it in the off-hand and the sword in the main hand.
        let mut inv = crate::inventory::Inventory::new();
        inv.set_slot(0, Some(sword()));
        let mut off = Some(shield_stack(100));
        assert_eq!(wear_blocking(&mut inv, 0, &mut off, Hand::Main, 100, 5), WearOutcome::Worn { broke: false });
        assert_eq!(durability(off.as_ref()), Some(95), "the shield, now in the off-hand, paid");
        assert_eq!(durability(inv.slot(0)), Some(sword_full), "the sword is untouched");
        // A shield in each hand, the main one the blocker (200); F swapped them.
        let mut inv = crate::inventory::Inventory::new();
        inv.set_slot(0, Some(shield_stack(50)));
        let mut off = Some(shield_stack(200));
        assert_eq!(wear_blocking(&mut inv, 0, &mut off, Hand::Main, 200, 5), WearOutcome::Worn { broke: false });
        assert_eq!(durability(off.as_ref()), Some(195), "the blocker (200) paid, though it is in the other hand now");
        assert_eq!(durability(inv.slot(0)), Some(50), "the other shield did not");
        // No swap, a shield in each hand: the named hand's.
        let mut inv = crate::inventory::Inventory::new();
        inv.set_slot(0, Some(shield_stack(200)));
        let mut off = Some(shield_stack(50));
        wear_blocking(&mut inv, 0, &mut off, Hand::Main, 200, 5);
        assert_eq!((durability(inv.slot(0)), durability(off.as_ref())), (Some(195), Some(50)));
        // Equal durabilities: the named hand's place.
        let mut inv = crate::inventory::Inventory::new();
        inv.set_slot(0, Some(shield_stack(80)));
        let mut off = Some(shield_stack(80));
        wear_blocking(&mut inv, 0, &mut off, Hand::Off, 80, 5);
        assert_eq!((durability(inv.slot(0)), durability(off.as_ref())), (Some(80), Some(75)));
    }

    #[test]
    fn guarding_names_the_hand_and_the_durability() {
        assert_eq!(guarding(Some(&shield_stack(7)), Some(&shield_stack(9))), Some((Hand::Main, 7)));
        assert_eq!(guarding(None, Some(&shield_stack(9))), Some((Hand::Off, 9)));
        assert_eq!(guarding(None, None), None);
    }

    /// The window event the server queues and the client applies is this one rule on
    /// both copies, so the two windows agree after it.
    #[test]
    fn the_wear_window_event_wears_both_copies_alike() {
        use crate::window_events::WindowEvent;
        let event = WindowEvent::WearShield { slot: 1, hand: Hand::Main, durability: 200, amount: 5 };
        let WindowEvent::WearShield { slot, hand, durability: worn_at, amount } = event else { unreachable!() };
        let (mut server, mut client) = (crate::inventory::Inventory::new(), crate::inventory::Inventory::new());
        let (mut server_off, mut client_off) = (None, None);
        server.set_slot(1, Some(shield_stack(200)));
        client.set_slot(1, Some(shield_stack(200)));
        wear_blocking(&mut server, slot, &mut server_off, hand, worn_at, amount);
        wear_blocking(&mut client, slot, &mut client_off, hand, worn_at, amount);
        assert_eq!(durability(server.slot(1)), Some(195));
        assert_eq!(durability(server.slot(1)), durability(client.slot(1)));
    }

    #[test]
    fn a_guard_asks_the_one_rule() {
        let g = Guard { victim: V, look: look(), raised_for: 5, hand: Hand::Main, durability: 10 };
        assert!(g.blocks(at(0.0)));
        assert!(!g.blocks(at(180.0)));
        assert!(!Guard { raised_for: 4, ..g }.blocks(at(0.0)));
    }
}
