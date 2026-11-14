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

/// Durability the server owes the joiner's shield so far this tick (blocks not yet queued).
fn owed(rig: &Rig, slot: usize) -> u16 {
    rig.hs.server.players[slot].shield_owed.iter().map(|o| o.amount).sum()
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
    assert_eq!(owed(&rig, slot), 5, "wear 1 + floor(4), queued");
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
    assert!(owed(&rig, slot) > 0, "and it wears");
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

/// A rig whose joiner holds a shield with `durability` in its main hand, in `mode`
/// (the seat kinds: Mirror, or Strict).
fn rig_with_weak_shield(tag: &str, durability: u16, mode: crate::action_identity::InventoryMode) -> (Rig, hecs::Entity) {
    let (mut rig, brigand) = rig_with_shield(tag, true);
    let slot = rig.joiners[0].slot;
    let sp = &mut rig.hs.server.players[slot];
    sp.inventory.set_slot(
        0,
        Some(crate::item::ItemStack::new_tool(Tool { durability, ..Tool::new(ToolType::Shield, ToolMaterial::Wood) })),
    );
    sp.inventory_mode = mode;
    (rig, brigand)
}

/// A hit from the front with `brigand`, the block window and i-frames as they are
/// (a hit inside an open window is blocked free).
fn bite_keeping_window(rig: &mut Rig, brigand: hecs::Entity) {
    rig.place(brigand, Vec3::new(0.0, 0.0, 0.6));
    rig.hs.server.difficulty = NORMAL;
    rig.tick(1);
    rig.hs.server.difficulty = crate::survival::Difficulty::Peaceful;
}

/// Review 2 finding 1. A shield with 1 durability blocks a front hit, whose wear (5)
/// is queued; the joiner's client WITHHOLDS the acknowledgement. The raw copy still
/// holds the shield, but it is owed broken: it protects nothing, whether the next
/// front hit falls inside the old block window or after it.
fn a_shield_owed_broken_protects_nothing_while_its_wear_is_withheld(mode: crate::action_identity::InventoryMode, tag: &str) {
    let (mut rig, brigand) = rig_with_weak_shield(tag, 1, mode);
    let slot = rig.joiners[0].slot;
    raise(&mut rig);
    rig.tick(RAISE_TICKS + 1);
    let hp = health(&rig);
    bite(&mut rig, brigand, true);
    assert_eq!(health(&rig), hp, "the first front hit is blocked");
    assert_eq!(shield_worn(&rig).len(), 1, "and its wear is queued");
    assert_eq!(rig.hs.server.players[slot].window_events.waiting(), 1, "the wear waits for the client's acknowledgement");
    assert_eq!(shadow_shield_durability(&rig), Some(1), "the raw copy still holds the shield");
    assert_eq!(rig.hs.server.players[slot].effective_shield(), None, "but it is owed broken");
    assert!(rig.hs.server.players[slot].shield_guard().is_none(), "so it guards nothing");
    // The next hit falls inside the old block window: it lands.
    bite_keeping_window(&mut rig, brigand);
    assert!(health(&rig) < hp, "a hit inside the old window lands: nothing is left to block it");
    // And once the window has run out, with the acknowledgement still withheld (the
    // landed hit knocked the body away: stand it back first).
    rig.hs.server.players[slot].player.pos = rig.at;
    rig.hs.server.players[slot].player.velocity = Vec3::ZERO;
    let hp = health(&rig);
    bite(&mut rig, brigand, true);
    assert!(health(&rig) < hp, "after the window, still unacknowledged: the hit lands");
    assert!(rig.hs.server.players[slot].window_events.has_waiting_shield_wear(), "never acknowledged: the wear still waits");
    assert_eq!(rig.hs.server.players[slot].shield_ticks, 0, "no raise is counted for a shield owed broken");
}

#[test]
fn a_mirror_seat_owed_a_broken_shield_protects_nothing_while_the_wear_is_withheld() {
    a_shield_owed_broken_protects_nothing_while_its_wear_is_withheld(crate::action_identity::InventoryMode::Mirror, "owed-mirror");
}

#[test]
fn a_strict_seat_owed_a_broken_shield_protects_nothing_while_the_wear_is_withheld() {
    a_shield_owed_broken_protects_nothing_while_its_wear_is_withheld(crate::action_identity::InventoryMode::Strict, "owed-strict");
}

/// Review 2 finding 1, within one tick: the wear a block of THIS tick owes (not yet
/// queued) counts too, so a second front hit of the same tick (a species hit after
/// the melee) meets a shield that is already broken.
#[test]
fn the_wear_owed_earlier_in_the_same_tick_counts_against_the_shield() {
    let (mut rig, _brigand) = rig_with_weak_shield("owed-same-tick", 3, crate::action_identity::InventoryMode::Mirror);
    let slot = rig.joiners[0].slot;
    raise(&mut rig);
    rig.tick(RAISE_TICKS + 1);
    let cause = crate::survival::DamageCause::Mob(MobType::Goat);
    let front = rig.at + Vec3::new(0.0, 0.0, 2.0);
    let hp = health(&rig);
    assert!(!rig.hs.server.land_hit_on_joiner(slot, 4.0, cause, Vec3::ZERO, Some(front)), "3 durability, a hit costing 5: blocked");
    assert_eq!(owed(&rig, slot), 5);
    assert_eq!(rig.hs.server.players[slot].effective_shield(), None, "owed broken, before anything is queued");
    assert!(rig.hs.server.land_hit_on_joiner(slot, 4.0, cause, Vec3::ZERO, Some(front)), "the next front hit of the same tick lands");
    assert!(health(&rig) < hp);
}

/// Review 2 finding 3. Species hits ran before the raise was counted for the tick, and
/// blasts the same: so on the fifth raised tick a sting read 4 and landed while the
/// melee after it read 5 and was blocked, and on the tick after a release a sting still
/// read the old count. One raise state per tick, ahead of every phase.
fn a_bee() -> impl Fn(&mut Rig, usize) -> hecs::Entity {
    |rig, slot| {
        let (bee, _) = rig.spawn(MobType::Bee, Vec3::new(0.0, 0.0, 1.5));
        rig.ecs_mut()
            .get::<&mut crate::bee_ai::BeeData>(bee)
            .expect("a bee carries its data")
            .state = crate::bee_ai::BeeAiState::Sting { target_id: slot as u64, until_tick: 0 };
        bee
    }
}

#[test]
fn a_sting_and_a_melee_hit_agree_on_the_fifth_raised_tick() {
    let (mut rig, brigand) = rig_with_shield("agree-fifth", true);
    let slot = rig.joiners[0].slot;
    raise(&mut rig);
    rig.tick(RAISE_TICKS - 2);
    let bee = a_bee()(&mut rig, slot);
    rig.tick(0);
    assert_eq!(rig.hs.server.players[slot].shield_ticks, RAISE_TICKS - 1, "four raised ticks so far");
    // The tick the shield becomes effective: the sting (a species hit) and the bite
    // (hostile melee) both fall in it.
    let hp = health(&rig);
    rig.hs.server.players[slot].combat.invincible_timer = 0;
    rig.hs.server.players[slot].combat.shield_window = 0;
    rig.place(brigand, Vec3::new(0.0, 0.0, 0.6));
    rig.place(bee, Vec3::new(0.0, 0.0, 1.5));
    rig.hs.server.difficulty = NORMAL;
    rig.tick(1);
    rig.hs.server.difficulty = crate::survival::Difficulty::Peaceful;
    assert_eq!(rig.hs.server.players[slot].shield_ticks, RAISE_TICKS);
    assert!(rig.ecs().get::<&crate::bee_ai::BeeData>(bee).is_err(), "the bee stung, and is gone");
    assert_eq!(health(&rig), hp, "the fifth raised tick: the sting and the bite are both blocked");
    assert_eq!(shield_worn(&rig).len(), 1, "one wearing block, the other inside its window");
}

#[test]
fn a_sting_and_a_melee_hit_agree_on_the_tick_after_a_release() {
    let (mut rig, brigand) = rig_with_shield("agree-release", true);
    let slot = rig.joiners[0].slot;
    raise(&mut rig);
    rig.tick(RAISE_TICKS + 3);
    assert!(rig.hs.server.players[slot].shield_ticks > RAISE_TICKS, "effective");
    let bee = a_bee()(&mut rig, slot);
    // The release arrives; the sting and the bite fall in the next tick.
    rig.hs.server.players[slot].shield_latch = false;
    let hp = health(&rig);
    rig.hs.server.players[slot].combat.invincible_timer = 0;
    rig.hs.server.players[slot].combat.shield_window = 0;
    rig.place(brigand, Vec3::new(0.0, 0.0, 0.6));
    rig.place(bee, Vec3::new(0.0, 0.0, 1.5));
    rig.hs.server.difficulty = NORMAL;
    rig.tick(1);
    rig.hs.server.difficulty = crate::survival::Difficulty::Peaceful;
    assert_eq!(rig.hs.server.players[slot].shield_ticks, 0, "released");
    assert!(health(&rig) < hp, "lowered: the hits land");
    assert!(shield_worn(&rig).is_empty(), "and neither was blocked (no wear): the release was in force for the sting too");
}

/// Review 2 finding 3, blasts: a lent world's early machine blasts run before the
/// server's own tick, so the clock phase (which runs ahead of them) counts the raise.
#[test]
fn a_lent_worlds_early_blast_reads_the_raise_counted_for_its_tick() {
    let mut rig = Rig::new_lent("blast-order", 1);
    let slot = rig.joiners[0].slot;
    crate::spawn_protection::disarm(&mut rig.hs.server.players[slot]);
    rig.hs.server.players[slot].inventory.set_slot(0, Some(crate::item::ItemStack::new_tool(Tool::new(ToolType::Shield, ToolMaterial::Wood))));
    rig.hs.server.players[slot].hotbar_slot = 0;
    raise(&mut rig);
    rig.tick(RAISE_TICKS - 1);
    assert_eq!(rig.hs.server.players[slot].shield_ticks, RAISE_TICKS - 1, "four raised ticks");
    let registry = crate::block::BlockRegistry::new();
    let at = rig.at;
    let cell = |dz: f32| ((at.x - 0.5).round() as i32, at.y as i32, (at.z + dz - 0.5).round() as i32);
    let mode = crate::play_mode::PlayMode::Survival;
    let blast = |rig: &mut Rig, front: bool| {
        let host = rig.host.as_mut().expect("lent");
        let seed = crate::sim_clock::ClockSeed {
            world_time: host.clock.world_time,
            world_time_step: 1,
            tick_counter: host.clock.tick_counter,
            weather: host.clock.weather,
            weather_lock_applied: None,
            night_calendar: host.clock.night_calendar,
            calendar_id: host.clock.calendar_id,
        };
        // What `lend_tick` does first: the clock phase, then the early machine beat.
        rig.hs.server.run_clock_phase(&seed, true, None, &mut host.world.sim_tally).expect("one clock phase");
        rig.hs.server.players[slot].combat.invincible_timer = 0;
        rig.hs.server.players[slot].combat.shield_window = 0;
        let landed = crate::explosion::apply_joiner_blast_damage(
            &mut rig.hs.server.players,
            &host.world,
            &registry,
            cell(if front { 2.0 } else { -2.0 }),
            mode,
        );
        landed.contains(&slot)
    };
    let hp = health(&rig);
    assert!(!blast(&mut rig, true), "the fifth raised tick: the early blast from the front is blocked");
    assert_eq!(rig.hs.server.players[slot].shield_ticks, RAISE_TICKS);
    assert_eq!(health(&rig), hp);
}

/// Review 2 finding 4. A joiner's main-hand shield has 3 durability. A front hit
/// costing 5 is blocked, then a rear hit kills the joiner in the same tick. The wear
/// is queued before the deaths settle, so the grave's window has the shield worn out:
/// no intact shield ends up in the grave.
#[test]
fn a_joiner_dying_in_the_blocking_tick_leaves_no_intact_shield_in_the_grave() {
    let (mut rig, _brigand) = rig_with_weak_shield("grave", 3, crate::action_identity::InventoryMode::Mirror);
    let slot = rig.joiners[0].slot;
    rig.hs.server.play_mode = crate::play_mode::PlayMode::Survival;
    raise(&mut rig);
    rig.tick(RAISE_TICKS + 1);
    let cause = crate::survival::DamageCause::Mob(MobType::Goat);
    let front = rig.at + Vec3::new(0.0, 0.0, 2.0);
    let behind = rig.at + Vec3::new(0.0, 0.0, -2.0);
    // Both hits land in one server tick's mob phase.
    assert!(!rig.hs.server.land_hit_on_joiner(slot, 4.0, cause, Vec3::ZERO, Some(front)), "blocked");
    rig.hs.server.players[slot].combat.invincible_timer = 0;
    assert!(rig.hs.server.land_hit_on_joiner(slot, 1000.0, cause, Vec3::ZERO, Some(behind)), "the rear hit lands and kills");
    assert!(rig.hs.server.players[slot].combat.dead);
    // The server's tick settles the death (wear queued first, then the grave).
    rig.hs.server.players[slot].shield_latch = false;
    rig.hs.tick();
    let is_shield = |s: &crate::item::ItemStack| matches!(&s.item, Item::Tool(t) if t.tool_type == ToolType::Shield);
    let in_grave = rig.world_mut().iter_graves().any(|(_, g)| g.slots.iter().flatten().any(is_shield));
    let on_floor = rig.ecs().query::<&crate::entity::ItemEntity>().iter().any(|(_, i)| is_shield(&i.stack));
    assert!(!in_grave && !on_floor, "the shield that was owed broken is not in the grave or on the floor");
}

// --- Review 3 ---

/// Review 3 #1. A joiner puts its only shield into an item frame: the server inserts
/// it and queues a `Take`, leaving the raw copy unchanged until the acknowledgement.
/// The shield is spent: it protects nothing, and the next front hit lands, on either
/// seat kind.
fn a_shield_a_pending_take_has_spent_protects_nothing(mode: crate::action_identity::InventoryMode, tag: &str) {
    let (mut rig, brigand) = rig_with_weak_shield(tag, crate::crafting::SHIELD_DURABILITY, mode);
    let slot = rig.joiners[0].slot;
    raise(&mut rig);
    rig.tick(RAISE_TICKS + 1);
    assert!(rig.hs.server.players[slot].shield_guard().is_some(), "guarding before the frame takes it");
    let now = rig.hs.server.tick_counter;
    let gone = Item::Tool(Tool::new(ToolType::Shield, ToolMaterial::Wood));
    crate::window_events::queue(
        &mut rig.hs.server.players[slot],
        crate::window_events::WindowEvent::Take { slot: 0, hand: crate::hands::Hand::Main, item: gone, n: 1, how: "item frame" },
        now,
    );
    assert_eq!(shadow_shield_durability(&rig), Some(crate::crafting::SHIELD_DURABILITY), "the raw copy still holds it");
    assert_eq!(rig.hs.server.players[slot].effective_shield(), None, "but a pending take has spent it");
    assert!(rig.hs.server.players[slot].shield_guard().is_none());
    let hp = health(&rig);
    bite(&mut rig, brigand, true);
    assert!(health(&rig) < hp, "the front hit lands: the shield is in the frame");
    assert!(shield_worn(&rig).is_empty(), "and nothing wears");
}

#[test]
fn a_mirror_seats_shield_taken_by_a_pending_event_protects_nothing() {
    a_shield_a_pending_take_has_spent_protects_nothing(crate::action_identity::InventoryMode::Mirror, "take-mirror");
}

#[test]
fn a_strict_seats_shield_taken_by_a_pending_event_protects_nothing() {
    a_shield_a_pending_take_has_spent_protects_nothing(crate::action_identity::InventoryMode::Strict, "take-strict");
}

/// Review 3 #2. The blocking shield (200) moves to another slot and a spare (50) takes
/// its place before the wear lands. The shield that blocked pays, wherever it went.
#[test]
fn a_wear_finds_the_shield_that_blocked_wherever_a_window_move_put_it() {
    let (mut rig, brigand) = rig_with_weak_shield("moved", 200, crate::action_identity::InventoryMode::Mirror);
    let slot = rig.joiners[0].slot;
    raise(&mut rig);
    rig.tick(RAISE_TICKS + 1);
    bite(&mut rig, brigand, true);
    let wear = shield_worn(&rig)[0].1;
    // The window move, before the client's acknowledgement: the 200 shield to slot 5,
    // a spare 50 shield in its place.
    let sp = &mut rig.hs.server.players[slot];
    let shield = |d| Some(crate::item::ItemStack::new_tool(Tool { durability: d, ..Tool::new(ToolType::Shield, ToolMaterial::Wood) }));
    sp.inventory.set_slot(5, shield(200));
    sp.inventory.set_slot(0, shield(50));
    rig.report();
    let sp = &rig.hs.server.players[slot];
    let at = |i| match sp.inventory.slot(i).map(|s| &s.item) {
        Some(Item::Tool(t)) => Some(t.durability),
        _ => None,
    };
    assert_eq!(at(5), Some(200 - wear), "the shield that blocked pays");
    assert_eq!(at(0), Some(50), "the spare that took its place does not");
}

/// Review 3 #2, both copies: the one rule finds the same shield for a client's window
/// and the server's, so the two agree. Equal shields are interchangeable: whichever of
/// two identical shields pays, the two copies pay the same one, and no lineage is kept.
#[test]
fn both_copies_of_a_window_wear_the_same_shield() {
    use crate::hands::Hand;
    let shield = |d| Some(crate::item::ItemStack::new_tool(Tool { durability: d, ..Tool::new(ToolType::Shield, ToolMaterial::Wood) }));
    let build = || {
        let mut inv = crate::inventory::Inventory::new();
        inv.set_slot(0, shield(50));
        inv.set_slot(5, shield(200));
        (inv, None)
    };
    let (mut a, mut a_off) = build();
    let (mut b, mut b_off) = build();
    for (inv, off) in [(&mut a, &mut a_off), (&mut b, &mut b_off)] {
        let out = crate::shield::wear_blocking(inv, 0, off, Hand::Main, 200, 7);
        assert_eq!(out, crate::shield::WearOutcome::Worn { broke: false });
    }
    let dur = |inv: &crate::inventory::Inventory, i| match inv.slot(i).map(|s| &s.item) {
        Some(Item::Tool(t)) => Some(t.durability),
        _ => None,
    };
    assert_eq!((dur(&a, 0), dur(&a, 5)), (Some(50), Some(193)));
    assert_eq!((dur(&a, 0), dur(&a, 5)), (dur(&b, 0), dur(&b, 5)), "both copies agree");
    // Identical shields: the named hand's place pays first, on both copies alike.
    let (mut c, mut c_off) = (crate::inventory::Inventory::new(), shield(80));
    c.set_slot(0, shield(80));
    let mut d = c.clone();
    let mut d_off = c_off.clone();
    crate::shield::wear_blocking(&mut c, 0, &mut c_off, Hand::Main, 80, 3);
    crate::shield::wear_blocking(&mut d, 0, &mut d_off, Hand::Main, 80, 3);
    assert_eq!((dur(&c, 0), c_off.as_ref().map(|s| &s.item)), (dur(&d, 0), d_off.as_ref().map(|s| &s.item)));
    assert_eq!(dur(&c, 0), Some(77), "the named hand's shield");
}

/// Review 3 #3. A lent host counts its raise in the clock phase, ahead of its inputs.
/// A release processed in the tick takes effect for every damage phase after it: a
/// species hit and a hostile melee hit land, though the tick's count still says
/// "raised". (A raise may count one tick late, and an early blast may still see the
/// previous tick's raise: Spec 05.)
#[test]
fn a_lent_hosts_processed_release_lowers_the_shield_for_the_hits_after_it() {
    let mut rig = Rig::new_lent("lent-release", 1);
    let slot = rig.joiners[0].slot;
    crate::spawn_protection::disarm(&mut rig.hs.server.players[slot]);
    rig.hs.server.players[slot].inventory.set_slot(0, Some(crate::item::ItemStack::new_tool(Tool::new(ToolType::Shield, ToolMaterial::Wood))));
    rig.hs.server.players[slot].hotbar_slot = 0;
    raise(&mut rig);
    rig.tick(RAISE_TICKS + 3);
    assert!(rig.hs.server.players[slot].shield_ticks > RAISE_TICKS, "raised and effective");
    let (brigand, _) = rig.spawn(MobType::Brigand, Vec3::new(0.0, 0.0, 6.0));
    let bee = a_bee()(&mut rig, slot);
    // The release arrives as an input: the lent tick's clock phase has already counted
    // the old latch; the inbound pass then lowers it, before the species and melee phases.
    let input = protocol::InputPacket { tick: 99, yaw: std::f32::consts::PI, health: 20.0, shield_raised: false, ..Default::default() };
    rig.joiners[0].client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
    rig.hs.server.players[slot].combat.invincible_timer = 0;
    rig.hs.server.players[slot].combat.shield_window = 0;
    rig.place(brigand, Vec3::new(0.0, 0.0, 0.6));
    rig.place(bee, Vec3::new(0.0, 0.0, 1.5));
    let hp = health(&rig);
    rig.hs.server.difficulty = NORMAL;
    rig.tick(1);
    rig.hs.server.difficulty = crate::survival::Difficulty::Peaceful;
    assert!(!rig.hs.server.players[slot].shield_latch, "the release was processed");
    assert!(rig.ecs().get::<&crate::bee_ai::BeeData>(bee).is_err(), "the bee stung");
    assert!(health(&rig) < hp, "the hits land: the release is in force for every phase after it");
    assert!(shield_worn(&rig).is_empty(), "and neither was blocked");
}

/// A lent rig whose joiner holds a shield of `durability` in its main hand, raised
/// and effective, in Survival.
fn lent_rig_with_shield(tag: &str, durability: u16) -> Rig {
    let mut rig = Rig::new_lent(tag, 1);
    let slot = rig.joiners[0].slot;
    crate::spawn_protection::disarm(&mut rig.hs.server.players[slot]);
    let sp = &mut rig.hs.server.players[slot];
    sp.inventory.set_slot(
        0,
        Some(crate::item::ItemStack::new_tool(Tool { durability, ..Tool::new(ToolType::Shield, ToolMaterial::Wood) })),
    );
    sp.hotbar_slot = 0;
    rig.hs.server.play_mode = crate::play_mode::PlayMode::Survival;
    raise(&mut rig);
    rig.tick(RAISE_TICKS + 1);
    rig
}

/// What a lent tick does before its server tick, as far as this joiner is concerned:
/// the clock phase (the raise counted), then the host's early blasts: one from the
/// front that wears the shield (it breaks a weak one), and when `rear_kills` one from
/// behind that kills. Then the connection closes: the Disconnect is read by the
/// server tick's inbound pass, before the end-of-tick wear queue has run.
fn early_blasts_then_the_seat_leaves(rig: &mut Rig, rear_kills: bool) {
    let slot = rig.joiners[0].slot;
    let registry = crate::block::BlockRegistry::new();
    let at = rig.at;
    let cell = |dz: f32| ((at.x - 0.5).round() as i32, at.y as i32, (at.z + dz - 0.5).round() as i32);
    let mode = crate::play_mode::PlayMode::Survival;
    {
        let host = rig.host.as_mut().expect("lent");
        let seed = crate::sim_clock::ClockSeed {
            world_time: host.clock.world_time,
            world_time_step: 1,
            tick_counter: host.clock.tick_counter,
            weather: host.clock.weather,
            weather_lock_applied: None,
            night_calendar: host.clock.night_calendar,
            calendar_id: host.clock.calendar_id,
        };
        rig.hs.server.run_clock_phase(&seed, true, None, &mut host.world.sim_tally).expect("one clock phase");
        rig.hs.server.players[slot].combat.invincible_timer = 0;
        rig.hs.server.players[slot].combat.shield_window = 0;
        let landed = crate::explosion::apply_joiner_blast_damage(&mut rig.hs.server.players, &host.world, &registry, cell(2.0), mode);
        assert!(!landed.contains(&slot), "the front blast is blocked");
        assert!(rig.hs.server.players[slot].effective_shield().is_none(), "and breaks the weak shield, effectively");
        if rear_kills {
            let sp = &mut rig.hs.server.players[slot];
            sp.combat.invincible_timer = 0;
            sp.combat.shield_window = 0;
            sp.combat.health = 0.5;
            let landed = crate::explosion::apply_joiner_blast_damage(&mut rig.hs.server.players, &host.world, &registry, cell(-2.0), mode);
            assert!(landed.contains(&slot), "the rear blast lands");
            assert!(rig.hs.server.players[slot].combat.dead, "and kills");
        }
    }
    rig.joiners[0].leave();
    let host = rig.host.as_mut().expect("lent");
    let clock = host.clock;
    crate::sim_lend::LentSim::lend(&mut rig.hs, host.parts(), clock).tick();
}

/// Review 3 #4. An early front blast breaks a weak shield (effectively), a rear blast
/// kills the joiner, and the seat's disconnect is read in the server tick's inbound
/// pass, before the end-of-tick wear queue. The death is settled from a window that
/// already has the wear: no intact shield in the grave.
#[test]
fn a_seat_that_leaves_after_dying_in_the_blocking_tick_leaves_no_intact_shield_in_the_grave() {
    let mut rig = lent_rig_with_shield("leave-dead", 1);
    early_blasts_then_the_seat_leaves(&mut rig, true);
    let is_shield = |s: &crate::item::ItemStack| matches!(&s.item, Item::Tool(t) if t.tool_type == ToolType::Shield);
    let in_grave = rig.world_mut().iter_graves().any(|(_, g)| g.slots.iter().flatten().any(is_shield));
    let on_floor = rig.ecs().query::<&crate::entity::ItemEntity>().iter().any(|(_, i)| is_shield(&i.stack));
    assert!(!in_grave && !on_floor, "the shield the early blast broke is not in the grave or on the floor");
}

/// Review 3 #4, a survivor: it leaves in the same boundary, and the record saved for
/// its npub has the shield worn out (queued before the leave save), not intact.
#[test]
fn a_signed_survivor_that_leaves_in_the_blocking_tick_saves_the_worn_shield() {
    let _root = crate::save::WorldsRootGuard::new("shield-leave-alive");
    let mut rig = lent_rig_with_shield("leave-alive", 1);
    let slot = rig.joiners[0].slot;
    rig.sign_in(0);
    // Bound to this world, as a signed joiner's progression is (a guest's is never saved).
    let scope = [23; 16];
    rig.host.as_mut().expect("lent").world.quest_scope = Some(scope);
    rig.hs.server.players[slot].personal = Some(
        crate::joiner_save::BoundPersonal::fresh(crate::progression::GameplayBinding::WorldLocal {
            scope,
            player: crate::progression::WorldPlayer::Npub(NPUB_KEY),
        })
        .expect("a binding"),
    );
    // The rig can save this seat at all (a precondition, so a failure here says why);
    // then clear the record so only the leave save can have written it.
    let path = rig.hs.server.joiner_path(NPUB_KEY);
    rig.hs.server.save_joiner(slot).expect("a signed seat in the world saves");
    assert!(path.exists(), "the precondition save wrote {}", path.display());
    std::fs::remove_file(&path).expect("clear the record");
    early_blasts_then_the_seat_leaves(&mut rig, false);
    assert!(!rig.hs.server.players[slot].combat.dead, "it survives");
    let bytes = std::fs::read(&path).expect("the leave save wrote the joiner's record");
    let window = crate::joiner_save::JoinerSave::decode(&bytes).expect("decodes").window().expect("a window");
    let shield_left = window.inv.slots_iter().flatten().any(|s| matches!(&s.item, Item::Tool(t) if t.tool_type == ToolType::Shield))
        || window.offhand.as_ref().is_some_and(|s| matches!(&s.item, Item::Tool(t) if t.tool_type == ToolType::Shield));
    assert!(!shield_left, "the saved window holds no intact shield: the one the blast broke is gone");
    let _ = std::fs::remove_file(&path);
}
