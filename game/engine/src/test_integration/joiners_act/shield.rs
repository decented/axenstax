//! Off-hand release (slice 5) — the Shield for a joiner: the server decides.
//!
//! A joiner's `InputPacket.shield_raised` is latched; the server counts the raise
//! itself, protects only while ITS copy of the window holds a shield, lands the
//! hits through the one block rule (`shield::blocks`), wears the shield through
//! its copy (`WindowEvent::WearShield`) and tells the joiner (`ShieldWorn`).

use super::*;
use crate::shield::RAISE_TICKS;

const NORMAL: crate::survival::Difficulty = crate::survival::Difficulty::Normal;

/// A rig (Peaceful until a [`bite`]) whose joiner (facing +Z) holds a shield in its main hand
/// in the server's copy (`with_shield`), past its spawn protection, and a brigand
/// standing well off. The raise is not latched yet ([`raise`]).
fn rig_with_shield(tag: &str, with_shield: bool) -> (Rig, hecs::Entity) {
    let mut rig = Rig::new(tag, 1);
    let slot = rig.joiners[0].slot;
    let sp = &mut rig.hs.server.players[slot];
    crate::spawn_protection::disarm(sp);
    if with_shield {
        sp.inventory.set_slot(0, Some(crate::item::ItemStack::new_tool(Tool::new(ToolType::Shield, ToolMaterial::Wood))));
    }
    sp.hotbar_slot = 0;
    let (brigand, _) = rig.spawn(MobType::Brigand, Vec3::new(0.0, 0.0, 6.0));
    (rig, brigand)
}

/// The joiner's client reports the shield raised from now on.
fn raise(rig: &mut Rig) {
    let slot = rig.joiners[0].slot;
    rig.hs.server.players[slot].shield_latch = true;
}

fn shadow_shield_durability(rig: &Rig) -> Option<u16> {
    match rig.hs.server.players[rig.joiners[0].slot].inventory.slot(0).map(|s| &s.item) {
        Some(Item::Tool(t)) => Some(t.durability),
        _ => None,
    }
}

fn shield_worn(rig: &Rig) -> Vec<(crate::hands::Hand, u16)> {
    rig.joiners[0]
        .inbox
        .events
        .iter()
        .filter_map(|e| match e {
            PlayerEventType::ShieldWorn { hand, amount, .. } => Some((*hand, *amount)),
            _ => None,
        })
        .collect()
}

fn health(rig: &Rig) -> f32 {
    rig.hs.server.players[rig.joiners[0].slot].combat.health
}

/// The body's i-frames and block window clear, `brigand` set beside it, and one
/// tick for it to bite (the joiner faces +Z: `front` is +Z).
fn bite(rig: &mut Rig, brigand: hecs::Entity, front: bool) {
    let slot = rig.joiners[0].slot;
    let z = if front { 0.6 } else { -0.6 };
    rig.hs.server.players[slot].combat.invincible_timer = 0;
    rig.hs.server.players[slot].combat.shield_window = 0;
    rig.place(brigand, Vec3::new(0.0, 0.0, z));
    rig.hs.server.difficulty = NORMAL;
    rig.tick(1);
    rig.hs.server.difficulty = crate::survival::Difficulty::Peaceful;
}

#[test]
fn a_raised_joiner_shield_blocks_a_hostile_in_front_and_wears_through_the_servers_copy() {
    let (mut rig, brigand) = rig_with_shield("front", true);
    let slot = rig.joiners[0].slot;
    raise(&mut rig);
    rig.tick(RAISE_TICKS + 1);
    assert!(rig.hs.server.players[slot].shield_ticks >= RAISE_TICKS, "the server counted the raise itself");
    let hp = health(&rig);
    bite(&mut rig, brigand, true);
    assert_eq!(health(&rig), hp, "no damage from the front");
    assert_eq!(rig.hs.server.players[slot].combat.invincible_timer, 0, "and no i-frame window");
    let raw = crate::survival::scale_mob_damage(crate::combat::HOSTILE_MELEE_DAMAGE, NORMAL);
    let wear = crate::shield::wear_for(raw);
    assert_eq!(shield_worn(&rig), vec![(crate::hands::Hand::Main, wear)], "one wearing block: 1 + floor(damage)");
    // The joiner's client says it wore its own; the server wears its copy.
    assert_eq!(shadow_shield_durability(&rig), Some(crate::crafting::SHIELD_DURABILITY), "applied in the client's order");
    rig.report();
    assert_eq!(
        shadow_shield_durability(&rig),
        Some(crate::crafting::SHIELD_DURABILITY - wear),
        "the server's copy wore by the same rule"
    );
}

#[test]
fn an_attack_from_behind_a_raised_joiner_shield_damages_as_before() {
    let (mut rig, brigand) = rig_with_shield("behind", true);
    raise(&mut rig);
    rig.tick(RAISE_TICKS + 1);
    let hp = health(&rig);
    bite(&mut rig, brigand, false);
    assert!(health(&rig) < hp, "from behind: the hit lands");
    assert!(shield_worn(&rig).is_empty(), "and the shield wears nothing");
}

#[test]
fn a_joiner_whose_server_copy_holds_no_shield_is_not_protected_though_the_flag_is_set() {
    let (mut rig, brigand) = rig_with_shield("noshield", false);
    let slot = rig.joiners[0].slot;
    raise(&mut rig);
    rig.tick(RAISE_TICKS + 1);
    assert_eq!(rig.hs.server.players[slot].shield_ticks, 0, "no shield in its copy: no raise counted");
    assert!(rig.hs.server.players[slot].shield_flag_logged, "said once in the log");
    let hp = health(&rig);
    bite(&mut rig, brigand, true);
    assert!(health(&rig) < hp, "the hit lands");
    assert!(shield_worn(&rig).is_empty());
}

#[test]
fn the_server_counts_the_raise_itself_four_ticks_do_not_protect_five_do() {
    // The count the hazard pass reads is the number of ticks the flag has been up,
    // that tick included. Four: the hit lands.
    let (mut early, brigand) = rig_with_shield("four", true);
    let slot = early.joiners[0].slot;
    raise(&mut early);
    early.tick(RAISE_TICKS - 2);
    let hp = health(&early);
    bite(&mut early, brigand, true);
    assert_eq!(early.hs.server.players[slot].shield_ticks, RAISE_TICKS - 1);
    assert!(health(&early) < hp, "4 ticks: not effective");
    // Five: blocked.
    let (mut ready, brigand) = rig_with_shield("five", true);
    let slot = ready.joiners[0].slot;
    raise(&mut ready);
    ready.tick(RAISE_TICKS - 1);
    let hp = health(&ready);
    bite(&mut ready, brigand, true);
    assert_eq!(ready.hs.server.players[slot].shield_ticks, RAISE_TICKS);
    assert_eq!(health(&ready), hp, "5 ticks: blocked");
}

#[test]
fn a_species_hit_on_a_joiner_follows_the_same_rule_and_an_unattributed_hit_is_never_blocked() {
    let (mut rig, _brigand) = rig_with_shield("species", true);
    raise(&mut rig);
    let slot = rig.joiners[0].slot;
    rig.tick(RAISE_TICKS + 1);
    let cause = crate::survival::DamageCause::Mob(MobType::Goat);
    let front = rig.at + Vec3::new(0.0, 0.0, 2.0);
    let behind = rig.at + Vec3::new(0.0, 0.0, -2.0);
    let knock = Vec3::new(0.0, 0.0, -0.4);
    let hp = health(&rig);
    assert!(!rig.hs.server.land_hit_on_joiner(slot, 4.0, cause, knock, Some(front)), "front: blocked");
    assert_eq!(health(&rig), hp);
    assert_eq!(rig.hs.server.players[slot].player.velocity, Vec3::ZERO, "no knockback");
    assert_eq!(rig.hs.server.players[slot].shield_wear, 5, "wear 1 + floor(4), queued");
    rig.hs.server.players[slot].combat.shield_window = 0;
    assert!(rig.hs.server.land_hit_on_joiner(slot, 4.0, cause, knock, Some(behind)), "behind: lands");
    assert!(health(&rig) < hp);
    rig.hs.server.players[slot].combat.invincible_timer = 0;
    let hp = health(&rig);
    assert!(rig.hs.server.land_hit_on_joiner(slot, 4.0, cause, Vec3::ZERO, None), "no attacker position (a Nostrich's kick-back): never blocked");
    assert!(health(&rig) < hp);
}

#[test]
fn a_blast_whose_centre_is_in_front_of_a_raised_joiner_shield_is_blocked() {
    let (mut rig, _brigand) = rig_with_shield("blast", true);
    raise(&mut rig);
    let slot = rig.joiners[0].slot;
    rig.tick(RAISE_TICKS + 1);
    let registry = crate::block::BlockRegistry::new();
    let at = rig.at;
    let cell = |dz: f32| ((at.x - 0.5).round() as i32, at.y as i32, (at.z + dz - 0.5).round() as i32);
    let mode = crate::play_mode::PlayMode::Survival;
    let hp = health(&rig);
    let world = std::mem::replace(rig.world_mut(), crate::world::World::new());
    let landed = crate::explosion::apply_joiner_blast_damage(&mut rig.hs.server.players, &world, &registry, cell(2.0), mode);
    assert!(!landed.contains(&slot), "front: blocked");
    assert_eq!(health(&rig), hp);
    assert!(rig.hs.server.players[slot].shield_wear > 0, "and it wears");
    rig.hs.server.players[slot].combat.shield_window = 0;
    let landed = crate::explosion::apply_joiner_blast_damage(&mut rig.hs.server.players, &world, &registry, cell(-2.0), mode);
    assert!(landed.contains(&slot), "behind: lands");
    assert!(health(&rig) < hp);
    *rig.world_mut() = world;
}

/// The flag rides the input: the server latches it, relays it as the avatar's GUARDING
/// bit, and simulates the joiner at sneak speed (no sprint) from the same flag.
#[test]
fn the_raised_flag_slows_the_joiners_body_and_is_relayed_as_the_guarding_pose() {
    let mut rig = Rig::new("flag", 2);
    let go = |rig: &mut Rig, j: usize, raised: bool, n: u32| {
        for _ in 0..n {
            rig.joiners[j].input_seq += 1;
            let seq = rig.joiners[j].input_seq;
            let input = protocol::InputPacket {
                tick: seq,
                yaw: std::f32::consts::PI,
                health: 20.0,
                move_forward: 1.0,
                sprint: true,
                shield_raised: raised,
                ..Default::default()
            };
            rig.joiners[j].client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
        }
    };
    let z0: Vec<f32> = rig.joiners.iter().map(|j| rig.hs.server.players[j.slot].player.pos.z).collect();
    for _ in 0..12 {
        go(&mut rig, 0, true, 1);
        go(&mut rig, 1, false, 1);
        rig.tick(1);
    }
    let moved = |rig: &Rig, j: usize| rig.hs.server.players[rig.joiners[j].slot].player.pos.z - z0[j];
    let (guarded, free) = (moved(&rig, 0), moved(&rig, 1));
    assert!(guarded > 0.2, "the guard still walks ({guarded})");
    assert!(guarded < free * 0.6, "but slower than a sprinting body ({guarded} vs {free})");
    let g = &rig.hs.server.players[rig.joiners[0].slot];
    assert!(g.shield_latch, "latched");
    let state = crate::server::collect_player_state(g, 0);
    assert_ne!(state.flags & protocol::player_flags::GUARDING, 0, "relayed as the guarding pose");
    let f = &rig.hs.server.players[rig.joiners[1].slot];
    assert_eq!(crate::server::collect_player_state(f, 1).flags & protocol::player_flags::GUARDING, 0);
}
