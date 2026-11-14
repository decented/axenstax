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

/// What [`wear`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WearOutcome {
    /// The shield lost durability; `broke` when it reached 0 and was removed.
    Worn { broke: bool },
    /// That hand holds no shield (a copy that drifted): nothing worn.
    Missing,
}

/// Wear the shield in `hand` by `n` durability, breaking it at 0 like any tool
/// (`Inventory::use_tool_at`, the cell rule every tool wears by). `slot` is the
/// hotbar slot the main hand names. The ONE wear rule: a local body wears its own
/// shield by it, and the two copies of a joiner's window (the server's and its
/// client's) wear the same shield by it when a block is reported
/// (`WindowEvent::WearShield`).
pub fn wear(
    inventory: &mut crate::inventory::Inventory,
    slot: usize,
    offhand: &mut Option<crate::item::ItemStack>,
    hand: Hand,
    n: u16,
) -> WearOutcome {
    let holds = |item: Option<&crate::item::ItemStack>| is_shield(item.map(|s| &s.item));
    let present = match hand {
        Hand::Main => holds(inventory.slot(slot)),
        Hand::Off => holds(offhand.as_ref()),
    };
    if !present {
        return WearOutcome::Missing;
    }
    let mut broke = false;
    for _ in 0..n {
        let info = match hand {
            Hand::Main => inventory.use_tool_at(slot),
            Hand::Off => crate::inventory::use_tool_cell(offhand),
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
        assert_eq!(wear(&mut inv, 2, &mut off, Hand::Main, 4), WearOutcome::Worn { broke: false });
        assert_eq!(durability(inv.slot(2)), Some(96));
        assert_eq!(durability(off.as_ref()), Some(10), "the off-hand is untouched");
        assert_eq!(wear(&mut inv, 2, &mut off, Hand::Off, 9), WearOutcome::Worn { broke: false });
        assert_eq!(durability(off.as_ref()), Some(1));
        assert_eq!(wear(&mut inv, 2, &mut off, Hand::Off, 5), WearOutcome::Worn { broke: true });
        assert!(off.is_none(), "a shield worn to nothing is gone, as any tool is");
        assert_eq!(wear(&mut inv, 2, &mut off, Hand::Off, 1), WearOutcome::Missing, "nothing left to wear");
    }

    #[test]
    fn wear_finds_no_shield_in_a_hand_that_holds_something_else() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let mut inv = crate::inventory::Inventory::new();
        let mut off = None;
        inv.set_slot(0, Some(crate::item::ItemStack::new_tool(Tool::new(ToolType::Sword, ToolMaterial::Iron))));
        assert_eq!(wear(&mut inv, 0, &mut off, Hand::Main, 3), WearOutcome::Missing, "a sword is not worn as a shield");
        assert_eq!(wear(&mut inv, 5, &mut off, Hand::Main, 3), WearOutcome::Missing, "an empty slot");
        assert_eq!(durability(inv.slot(0)), Some(Tool::new(ToolType::Sword, ToolMaterial::Iron).durability), "unchanged");
    }

    /// The window event the server queues and the client applies is this one rule on
    /// both copies, so the two windows agree after it.
    #[test]
    fn the_wear_window_event_wears_both_copies_alike() {
        use crate::window_events::WindowEvent;
        let event = WindowEvent::WearShield { slot: 1, hand: Hand::Main, amount: 5 };
        let WindowEvent::WearShield { slot, hand, amount } = event else { unreachable!() };
        let (mut server, mut client) = (crate::inventory::Inventory::new(), crate::inventory::Inventory::new());
        let (mut server_off, mut client_off) = (None, None);
        server.set_slot(1, Some(shield_stack(200)));
        client.set_slot(1, Some(shield_stack(200)));
        wear(&mut server, slot, &mut server_off, hand, amount);
        wear(&mut client, slot, &mut client_off, hand, amount);
        assert_eq!(durability(server.slot(1)), Some(195));
        assert_eq!(durability(server.slot(1)), durability(client.slot(1)));
    }

    #[test]
    fn a_guard_asks_the_one_rule() {
        let g = Guard { victim: V, look: look(), raised_for: 5 };
        assert!(g.blocks(at(0.0)));
        assert!(!g.blocks(at(180.0)));
        assert!(!Guard { raised_for: 4, ..g }.blocks(at(0.0)));
    }
}
