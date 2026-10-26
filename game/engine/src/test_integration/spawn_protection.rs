//! Spawn protection on a joiner's server body (`crate::spawn_protection`):
//! for 15 s after it enters the world and after each respawn, hostile mobs
//! neither hurt nor target it; a hit on a mob ends it; other harm still lands.

use glam::Vec3;

use crate::block;
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::mob::MobType;
use crate::mob_ai::{AiState, MobAi};
use crate::spawn_protection::{shielded, PROTECTION_TICKS};
use crate::transport::{ChannelClientTransport, ClientTransport};

use super::joiner_authority::join_guest;

struct Rig {
    hs: HostedServer,
    client: ChannelClientTransport,
    slot: usize,
    at: Vec3,
}

impl Rig {
    /// A dedicated server with one guest joiner just in the world, standing
    /// on a stone floor at (40, 80, 40), with no mobs but the test's.
    fn new(tag: &str) -> Self {
        let mut hs = HostedServer::start(
            0,
            format!("spawn-protection-{tag}-{}", std::process::id()),
            42,
            0,
            RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        let (client, slot) = join_guest(&mut hs, "Newcomer");
        let (fx, fy, fz) = (40, 80, 40);
        for x in fx - 6..=fx + 6 {
            for z in fz - 6..=fz + 6 {
                hs.server.world.set_block(x, fy - 1, z, block::STONE);
                for y in fy..=fy + 4 {
                    hs.server.world.set_block(x, y, z, block::AIR);
                }
            }
        }
        let at = Vec3::new(fx as f32 + 0.5, fy as f32, fz as f32 + 0.5);
        hs.server.column_streamer = None;
        hs.server.column_refill_per_tick = 0;
        crate::remote_mobs::purge_private_mobs(&mut hs.server.ecs);
        let mut rig = Rig { hs, client, slot, at };
        rig.stand();
        rig
    }

    fn sp(&mut self) -> &mut crate::server::ServerPlayer {
        &mut self.hs.server.players[self.slot]
    }

    /// Put the body back on its spot, at rest.
    fn stand(&mut self) {
        let at = self.at;
        let p = &mut self.sp().player;
        p.pos = at;
        p.velocity = Vec3::ZERO;
        p.reset_fall();
    }

    fn tick(&mut self, n: u32) {
        for _ in 0..n {
            self.hs.tick();
            while self.client.try_recv_from_server().is_some() {}
        }
    }

    fn spawn(&mut self, kind: MobType, offset: Vec3) -> hecs::Entity {
        crate::entity::spawn_mob(&mut self.hs.server.ecs, kind, self.at + offset)
    }

    /// Hold `mob` beside the body, idle and about to look for a target.
    fn hold(&mut self, mob: hecs::Entity, offset: Vec3) {
        self.stand();
        let at = self.at;
        let ecs = &mut self.hs.server.ecs;
        ecs.get::<&mut crate::entity::Position>(mob).unwrap().0 = at + offset;
        ecs.get::<&mut crate::entity::Velocity>(mob).unwrap().0 = Vec3::ZERO;
        ecs.get::<&mut MobAi>(mob).unwrap().state = AiState::Idle { timer: 0 };
    }

    fn chasing(&self, mob: hecs::Entity) -> bool {
        matches!(self.hs.server.ecs.get::<&MobAi>(mob).unwrap().state, AiState::Chase)
    }

    fn health(&mut self) -> f32 {
        self.sp().combat.health
    }
}

/// A Berserker chases day and night (`brigand::Tier::always_aggressive`).
const BESIDE: Vec3 = Vec3::new(0.6, 0.0, 0.0);

#[test]
fn a_new_joiner_takes_no_hostile_melee_and_is_no_target_for_the_window_then_is() {
    let mut rig = Rig::new("window");
    assert!(shielded(rig.sp()), "entering the world arms it");
    let left = rig.sp().spawn_protection;
    assert!(left <= PROTECTION_TICKS && left > PROTECTION_TICKS - 60, "armed at the join ({left} left)");
    let mob = rig.spawn(MobType::Berserker, BESIDE);
    let mut ticks = 0;
    while shielded(rig.sp()) {
        rig.hold(mob, BESIDE);
        rig.tick(1);
        ticks += 1;
        if !shielded(rig.sp()) {
            break;
        }
        assert_eq!(rig.health(), 20.0, "no hostile hit lands {ticks} ticks in");
        assert!(!rig.chasing(mob), "no hostile targets the body {ticks} ticks in");
        assert!(ticks <= PROTECTION_TICKS, "the window ends");
    }
    assert_eq!(ticks, left, "it lasts exactly what was armed");
    rig.hold(mob, BESIDE);
    rig.tick(2);
    assert!(rig.health() < 20.0, "after the window the hit lands");
    assert_eq!(rig.sp().combat.last_damage, crate::survival::DamageCause::Mob(MobType::Berserker));
    rig.hold(mob, Vec3::new(5.0, 0.0, 0.0));
    rig.tick(1);
    assert!(rig.chasing(mob), "and the body is a target again");
}

#[test]
fn a_respawn_arms_it_again() {
    let mut rig = Rig::new("respawn");
    crate::spawn_protection::disarm(rig.sp());
    rig.sp().combat.die(crate::survival::DamageCause::Mob(MobType::Berserker));
    rig.sp().dead_ticks = crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN;
    let slot = rig.slot;
    let spawn = rig.hs.server.respawn_player(slot).expect("respawned");
    assert_eq!(rig.sp().spawn_protection, PROTECTION_TICKS, "a respawn arms the full window");
    rig.at = spawn;
    let mob = rig.spawn(MobType::Berserker, BESIDE);
    for _ in 0..20 {
        rig.hold(mob, BESIDE);
        rig.tick(1);
        assert_eq!(rig.health(), 20.0, "no hostile hit on the respawned body");
        assert!(!rig.chasing(mob));
    }
}

#[test]
fn hitting_a_mob_ends_it() {
    let mut rig = Rig::new("hit");
    assert!(shielded(rig.sp()));
    let mob = rig.spawn(MobType::Berserker, BESIDE);
    rig.hold(mob, BESIDE);
    let slot = rig.slot;
    rig.hs.strike_joiner_swing(slot, mob, false, false, 1.0);
    assert!(!shielded(rig.sp()), "the joiner's own hit ends the protection");
    rig.hold(mob, BESIDE);
    rig.sp().combat.invincible_timer = 0;
    rig.tick(2);
    assert!(rig.health() < 20.0, "and the mob's hits land from then on");
}

#[test]
fn harm_that_is_not_a_mob_still_lands_while_protected() {
    let mut rig = Rig::new("lava");
    assert!(shielded(rig.sp()));
    let (x, y, z) = (rig.at.x.floor() as i32, rig.at.y as i32, rig.at.z.floor() as i32);
    rig.hs.server.world.set_block(x, y, z, block::LAVA);
    rig.tick(crate::survival::CONTACT_HAZARD_PERIOD_TICKS as u32 + 1);
    assert!(shielded(rig.sp()), "still inside the window");
    assert!(rig.health() < 20.0, "lava burns a protected body");
    assert_eq!(rig.sp().combat.last_damage, crate::survival::DamageCause::Lava);
}
