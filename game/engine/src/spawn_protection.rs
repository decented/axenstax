//! Spawn protection: for [`PROTECTION_SECS`] after a player enters the world
//! and after each respawn, a join or a respawn at night is not a death at the
//! spawn point. On every kind of world the server runs (a dedicated server, a
//! Keep, an owning `--no-lend` host, and a lending host or `--solo-hosted`
//! game, whose mob AI runs in its server since D4 slice 3), a protected
//! joiner:
//!
//! - takes no hostile melee,
//! - is no target for hostile mobs: the mob AI and the brigand pre-pass see
//!   it at the "no body" position ([`targetable_pos`]), so none picks it and a
//!   chase already on it gives up,
//! - takes no blast damage (a keg set off near it, by anyone, harms it not:
//!   `explosion::apply_joiner_blast_damage`).
//!
//! It covers joiners' server bodies only: a host's own (local) players are
//! their client's, unprotected, as in a standalone game.
//!
//! Hitting a mob (or loosing a shot) ends the window early. Fall, drowning,
//! lava, fire, hunger and every other harm that is neither a mob's nor a
//! blast's are unchanged. Server-side only: no protocol field, no client
//! indicator.
//!
//! The state is one countdown on `ServerPlayer` (`spawn_protection`, in
//! ticks): [`arm`] sets it, [`tick`] counts it down once per server tick,
//! [`disarm`] clears it, [`shielded`] reads it.

use crate::server::ServerPlayer;

/// How long the protection lasts.
pub const PROTECTION_SECS: u64 = 15;

/// The server's tick length: the fixed step of the Keep world's and the
/// dedicated server's loops (`keep_world::run`, `server_main`: 50 ms, 20 TPS).
pub const TICK_MILLIS: u64 = 50;

/// [`PROTECTION_SECS`] in server ticks.
pub const PROTECTION_TICKS: u32 = (PROTECTION_SECS * 1000 / TICK_MILLIS) as u32;

/// Start (or restart) the window: on entering the world and on a respawn.
pub fn arm(sp: &mut ServerPlayer) {
    sp.spawn_protection = PROTECTION_TICKS;
}

/// End the window now: the player hit a mob.
pub fn disarm(sp: &mut ServerPlayer) {
    sp.spawn_protection = 0;
}

/// One server tick passes.
pub fn tick(sp: &mut ServerPlayer) {
    sp.spawn_protection = sp.spawn_protection.saturating_sub(1);
}

/// Is the player inside the window: no hostile melee or blast lands on it and
/// no mob picks it as a target (module doc).
pub fn shielded(sp: &ServerPlayer) -> bool {
    sp.spawn_protection > 0
}

/// Where mob AI sees this player: its body, or — while [`shielded`] — the
/// "no body" position (`species_ai::ABSENT_PLAYER`), far past every detect
/// and lose range. Hostiles neither pick it nor keep chasing it (a chase
/// gives up and idles), and with someone still in the list the rest of mob
/// AI (wandering wildlife) runs as before rather than pausing.
pub fn targetable_pos(sp: &ServerPlayer) -> glam::Vec3 {
    if shielded(sp) {
        crate::species_ai::ABSENT_PLAYER
    } else {
        sp.player.pos
    }
}

#[cfg(test)]
mod tests;
