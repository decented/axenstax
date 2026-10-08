//! C3c-2 (protocol v81) — shooting a bow or a slingshot: ONE rule for every
//! seat (Spec 04 §4.2f "Use requests", Spec 05 "Ranged weapons").
//!
//! - **Single-player and a host's own seats** (`GameState`'s right-click
//!   arm): the ammo is found ([`find_ammo`]) and taken from that slot, the
//!   weapon wears in the hand, and the projectile is spawned from the
//!   player's eye along the camera ([`launch`], [`spawn`]).
//! - **The server, for a joiner** (`HostedServer`, `ItemAction::Shoot`): the
//!   same rules, from the SERVER's own position for that player at eye
//!   height (never a client-sent origin) along the request's yaw and pitch;
//!   the ammo is an owed take from the server's copy of the window and the
//!   weapon's wear a window event, both applied by the client when the
//!   outcome comes back. The joined client spawns nothing: it sees the
//!   server's projectile (`remote_entities::RemoteProjectiles`).
//!
//! A joiner's shot is the server's projectile, owned by the joiner
//! (`entity::Shooter` with `Attacker::Remote`), so it does exactly what a
//! host's does: the arrow's damage on the first mob it reaches (no
//! knockback), the slingshot's stun, kill credit to the shooter, the 1C
//! no-friendly-fire shield, Bear/Hyena provocation. It stops at the first
//! solid block and is never picked up. It never reaches a player: a
//! projectile hits mobs only (`entity::tick_projectiles`).

use glam::Vec3;

use crate::crafting::ToolType;
use crate::inventory::Inventory;
use crate::item::{Item, MaterialId};
use crate::protocol::ShotWeapon;

/// The server's cooldown between a joiner's shots: single-player's
/// right-click cooldown after a shot (`place_cooldown = 8`, about 0.4 s). A
/// `Shoot` arriving sooner on the server's schedule (less
/// [`SHOT_JITTER_TICKS`]) is refused (`ItemNote::TooSoon`); each accepted
/// shot moves the schedule a full cooldown on.
pub const SHOT_COOLDOWN_TICKS: u64 = 8;

/// How early a joiner's shot may arrive on the server's schedule: two shots
/// the client spaced a full [`SHOT_COOLDOWN_TICKS`] apart are never refused
/// for arriving a tick or two closer (network jitter), but the long-run rate
/// is the client's (the shape of `hosted_server::ATTACK_COOLDOWN_JITTER_TICKS`).
pub const SHOT_JITTER_TICKS: u64 = 3;

/// The weapon `held` is, if it shoots.
pub fn weapon_of(held: Option<&Item>) -> Option<ShotWeapon> {
    match held {
        Some(Item::Tool(t)) if t.tool_type == ToolType::Bow => Some(ShotWeapon::Bow),
        Some(Item::Tool(t)) if t.tool_type == ToolType::Slingshot => Some(ShotWeapon::Slingshot),
        _ => None,
    }
}

/// What `weapon` fires: an arrow, or a rubber ball.
pub fn ammo_for(weapon: ShotWeapon) -> MaterialId {
    match weapon {
        ShotWeapon::Bow => MaterialId::Arrow,
        ShotWeapon::Slingshot => MaterialId::RubberBall,
    }
}

/// The ammo search: the first of the 36 slots holding `weapon`'s ammo (a
/// stack anywhere in the bag will do). The server pays the same unit by the
/// shared owed take (`joiner_actions::take_owed_window`), which finds this
/// slot first in a window that agrees.
pub fn find_ammo(inv: &Inventory, weapon: ShotWeapon) -> Option<usize> {
    let ammo = Item::Material(ammo_for(weapon));
    (0..36).find(|&i| inv.slot(i).is_some_and(|s| s.item == ammo))
}

/// The draw a click fires at: the bow has no draw (it always fires at full
/// speed); the slingshot fires at its full charge (the click is instant).
pub fn max_charge(weapon: ShotWeapon) -> u16 {
    match weapon {
        ShotWeapon::Bow => 0,
        ShotWeapon::Slingshot => crate::slingshot::SLINGSHOT_MAX_CHARGE_TICKS as u16,
    }
}

/// One shot leaving the weapon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Launch {
    /// Where it starts: a little in front of the eye, clear of the body.
    pub pos: Vec3,
    /// Blocks per tick.
    pub vel: Vec3,
    pub damage: f32,
    /// A rubber ball (stuns), not an arrow.
    pub blunt: bool,
}

/// `weapon` fired from `eye` along `dir` (a unit vector), drawn for `charge`
/// ticks, clamped to the weapon's maximum ([`max_charge`]).
pub fn launch(weapon: ShotWeapon, eye: Vec3, dir: Vec3, charge: u16) -> Launch {
    match weapon {
        ShotWeapon::Bow => Launch {
            pos: eye + dir * 0.5,
            vel: dir * crate::entity::ARROW_INITIAL_SPEED,
            damage: crate::entity::ARROW_DAMAGE,
            blunt: false,
        },
        ShotWeapon::Slingshot => {
            let charge = u32::from(charge.min(max_charge(weapon)));
            Launch {
                pos: eye + dir * 0.4,
                vel: dir * crate::slingshot::slingshot_velocity(charge),
                damage: crate::slingshot::slingshot_damage(charge),
                blunt: true,
            }
        }
    }
}

/// Spawn `shot` into `ecs`, fired by `owner`.
pub fn spawn(ecs: &mut hecs::World, shot: &Launch, owner: Option<crate::entity::Shooter>) {
    crate::entity::spawn_projectile(ecs, shot.pos, shot.vel, shot.damage, shot.blunt, owner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crafting::{Tool, ToolMaterial};
    use crate::item::ItemStack;

    #[test]
    fn a_bow_fires_arrows_and_a_slingshot_rubber_balls() {
        let bow = Item::Tool(Tool::new(ToolType::Bow, ToolMaterial::Wood));
        let sling = Item::Tool(Tool::new(ToolType::Slingshot, ToolMaterial::Wood));
        assert_eq!(weapon_of(Some(&bow)), Some(ShotWeapon::Bow));
        assert_eq!(weapon_of(Some(&sling)), Some(ShotWeapon::Slingshot));
        assert_eq!(weapon_of(Some(&Item::Material(MaterialId::Arrow))), None);
        assert_eq!(weapon_of(None), None);
        assert_eq!(ammo_for(ShotWeapon::Bow), MaterialId::Arrow);
        assert_eq!(ammo_for(ShotWeapon::Slingshot), MaterialId::RubberBall);
    }

    /// The single-player search: the first stack anywhere in the 36 slots.
    #[test]
    fn the_ammo_search_takes_the_first_stack_in_any_slot() {
        let mut inv = Inventory::new();
        assert_eq!(find_ammo(&inv, ShotWeapon::Bow), None);
        inv.set_slot(30, Some(ItemStack::new_material(MaterialId::Arrow, 4)));
        inv.set_slot(12, Some(ItemStack::new_material(MaterialId::Arrow, 1)));
        inv.set_slot(2, Some(ItemStack::new_material(MaterialId::RubberBall, 1)));
        assert_eq!(find_ammo(&inv, ShotWeapon::Bow), Some(12));
        assert_eq!(find_ammo(&inv, ShotWeapon::Slingshot), Some(2));
    }

    /// The launch single-player always used: the bow at full speed from half
    /// a block out; the slingshot at its charge, clamped to the maximum.
    #[test]
    fn a_launch_is_the_single_player_shot_and_the_charge_is_clamped() {
        let eye = Vec3::new(1.0, 70.0, 2.0);
        let dir = Vec3::new(0.0, 0.0, -1.0);
        let arrow = launch(ShotWeapon::Bow, eye, dir, 999);
        assert_eq!(arrow.pos, eye + dir * 0.5);
        assert_eq!(arrow.vel, dir * crate::entity::ARROW_INITIAL_SPEED);
        assert_eq!(arrow.damage, crate::entity::ARROW_DAMAGE);
        assert!(!arrow.blunt);
        let full = launch(ShotWeapon::Slingshot, eye, dir, max_charge(ShotWeapon::Slingshot));
        let over = launch(ShotWeapon::Slingshot, eye, dir, u16::MAX);
        assert_eq!(full, over, "clamped to the weapon's maximum");
        assert_eq!(full.damage, crate::slingshot::SLINGSHOT_MAX_DAMAGE);
        assert!(full.blunt);
        let weak = launch(ShotWeapon::Slingshot, eye, dir, 0);
        assert!(weak.vel.length() < full.vel.length());
    }
}
