//! C2a (2026-10-07, protocol v73) — a joiner's hunger, eating and sleep are
//! the server's.
//!
//! Every test drives a REAL `HostedServer` over the in-process transport — a
//! dedicated server (0 local slots), or a `--no-lend` host for the local-slot
//! case — and reads what the joiner is actually sent: its `own_hunger` and
//! health on each `StateUpdate`, its `ItemActionOutcome`s and its own
//! `PlayerEvent`s.

use glam::Vec3;

use crate::block;
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::item::{Item, ItemStack, MaterialId};
use crate::item_actions::{self, ItemNote};
use crate::protocol::{self, ItemAction, PlayerEventType};
use crate::survival::Difficulty;
use crate::transport::{ChannelClientTransport, ClientTransport};

use super::joiner_authority::join_guest;

/// What the joiner was sent that these tests read.
#[derive(Default)]
struct Inbox {
    /// `(own_hunger, own health)` per StateUpdate, in order.
    own: Vec<(u8, f32)>,
    outcomes: Vec<protocol::ItemActionOutcomePacket>,
    events: Vec<PlayerEventType>,
}

impl Inbox {
    fn drain(&mut self, client: &ChannelClientTransport, slot: usize) {
        while let Some(pkt) = client.try_recv_from_server() {
            let Some((ptype, payload)) = protocol::deserialize_header(&pkt) else { continue };
            match ptype {
                protocol::PacketType::StateUpdate => {
                    let s: protocol::StateUpdatePacket = protocol::safe_deserialize(payload).unwrap();
                    if let Some(me) = s.players.iter().find(|p| p.player_index as usize == slot) {
                        self.own.push((s.own_hunger, me.health));
                    }
                }
                protocol::PacketType::ItemActionOutcome => {
                    self.outcomes.push(protocol::safe_deserialize(payload).unwrap());
                }
                protocol::PacketType::PlayerEvent => {
                    let e: protocol::PlayerEventPacket = protocol::safe_deserialize(payload).unwrap();
                    if e.player_index as usize == slot {
                        self.events.push(e.event);
                    }
                }
                _ => {}
            }
        }
    }

    fn outcome(&self, seq: u32) -> &protocol::ItemActionOutcomePacket {
        self.outcomes.iter().find(|o| o.seq == seq).expect("the request was answered")
    }

    fn last_hunger(&self) -> u8 {
        self.own.last().expect("a StateUpdate").0
    }
}

struct Rig {
    hs: HostedServer,
    client: ChannelClientTransport,
    slot: usize,
    inbox: Inbox,
    seq: u32,
    at: Vec3,
}

impl Rig {
    /// A dedicated server with one guest joiner standing on a stone floor at
    /// (40, 80, 40), looking along +z, no mobs, no streaming.
    fn new(tag: &str) -> Self {
        let mut hs = HostedServer::start(
            0,
            format!("joiner-hunger-{tag}-{}", std::process::id()),
            42,
            0,
            RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        let (client, slot) = join_guest(&mut hs, "Hungry");
        let mut world = std::mem::replace(&mut hs.server.world, crate::world::World::new());
        let at = super::joiners_act::floor_and_stand(&mut world, &mut hs, slot);
        hs.server.world = world;
        hs.server.column_streamer = None;
        hs.server.column_refill_per_tick = 0;
        hs.server.difficulty = Difficulty::Peaceful;
        crate::remote_mobs::purge_private_mobs(&mut hs.server.ecs);
        let mut rig = Rig { hs, client, slot, inbox: Inbox::default(), seq: 0, at };
        rig.tick(1);
        rig
    }

    fn tick(&mut self, n: u32) {
        for _ in 0..n {
            self.hs.tick();
            self.inbox.drain(&self.client, self.slot);
        }
    }

    fn sp(&mut self) -> &mut crate::server::ServerPlayer {
        &mut self.hs.server.players[self.slot]
    }

    fn send(&mut self, action: ItemAction) -> u32 {
        self.seq += 1;
        let pkt = protocol::ItemActionPacket { seq: self.seq, action };
        self.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
        self.seq
    }

    fn eat(&mut self, item: &Item) -> u32 {
        let (held_kind, held_id) = crate::inventory::item_to_ref(item).to_wire();
        self.send(ItemAction::Eat {
            hotbar_slot: 0,
            held_kind,
            held_id,
            held_full: crate::inventory::item_to_wire_full(item),
        })
    }

    fn sleep(&mut self, bed: [i32; 3]) -> u32 {
        self.send(ItemAction::Sleep { bed })
    }

    /// A bed two blocks ahead (+z) of the joiner, on the floor.
    fn bed_ahead(&mut self) -> [i32; 3] {
        let bed = [self.at.x.floor() as i32, self.at.y as i32, self.at.z.floor() as i32 + 2];
        self.hs.server.world.set_block(bed[0], bed[1], bed[2], block::BED);
        bed
    }

    fn shadow_count(&self, m: MaterialId) -> u32 {
        self.hs.server.players[self.slot].inventory.count_material(m) as u32
    }
}

fn bread() -> Item {
    Item::Material(MaterialId::Bread)
}

fn bread_value() -> f32 {
    bread().food_value().expect("bread is food")
}

// ─── Metabolism (decision 1, 2) ────────────────────────────────────────────

#[test]
fn a_joiners_hunger_drains_on_the_server_and_reaches_its_client() {
    let mut rig = Rig::new("drain");
    assert_eq!(rig.inbox.last_hunger(), 20, "it starts full, and is told so");
    rig.tick(crate::combat::HUNGER_DRAIN_INTERVAL_TICKS);
    assert_eq!(rig.sp().combat.hunger, 19, "one drain interval: one point");
    assert_eq!(rig.inbox.last_hunger(), 19, "its own_hunger follows the server's");
}

#[test]
fn regen_needs_hunger_eighteen_or_more() {
    let mut rig = Rig::new("regen");
    rig.sp().combat.health = 10.0;
    rig.sp().combat.hunger = 17;
    rig.tick(200);
    assert_eq!(rig.sp().combat.health, 10.0, "hungry: no regen");
    rig.sp().combat.hunger = 18;
    rig.tick(81);
    assert_eq!(rig.sp().combat.health, 11.0, "well fed: a pulse");
    assert_eq!(rig.sp().combat.hunger, 17, "which cost a hunger point");
    assert_eq!(rig.inbox.own.last().unwrap().1, 11.0, "and the joiner sees it");
}

#[test]
fn starvation_on_hard_kills_a_joiner_with_a_starvation_death() {
    let mut rig = Rig::new("starve-hard");
    rig.hs.server.difficulty = Difficulty::Hard;
    rig.sp().combat.health = 2.0;
    rig.sp().combat.hunger = 0;
    rig.tick(3 * crate::combat::STARVATION_INTERVAL_TICKS);
    assert!(rig.sp().combat.dead, "Hard's floor is 0: starvation kills");
    assert_eq!(
        rig.inbox.events,
        vec![PlayerEventType::DiedOf { cause: protocol::WireDamageCause::Starvation }],
        "the server's death, announced like its other hazard deaths"
    );
}

#[test]
fn the_easy_starvation_floor_holds() {
    let mut rig = Rig::new("starve-easy");
    rig.hs.server.difficulty = Difficulty::Easy;
    let floor = Difficulty::Easy.rules().starvation_floor;
    rig.sp().combat.health = floor + 3.0;
    rig.sp().combat.hunger = 0;
    rig.tick(10 * crate::combat::STARVATION_INTERVAL_TICKS);
    assert_eq!(rig.sp().combat.health, floor, "starves down to Easy's floor, no further");
    assert!(!rig.sp().combat.dead);
}

#[test]
fn a_creative_joiner_is_exempt() {
    let mut rig = Rig::new("creative");
    rig.hs.server.play_mode = crate::play_mode::PlayMode::Creative;
    rig.hs.server.difficulty = Difficulty::Hard;
    rig.sp().combat.health = 1.0;
    rig.sp().combat.hunger = 0;
    rig.tick(5 * crate::combat::STARVATION_INTERVAL_TICKS);
    assert_eq!(rig.sp().combat.health, 20.0, "a creative body is kept whole (the client's rule)");
    assert!(!rig.sp().combat.dead);
    assert!(rig.inbox.events.is_empty(), "and never dies");
}

/// A host's local slot keeps `tick_timers()` only: its client runs its
/// metabolism and writes its health; the server's copy neither drains nor
/// regenerates (review D2a MEDIUM-1).
#[test]
fn a_host_local_slot_runs_no_server_metabolism() {
    let mut hs = HostedServer::start(
        1,
        format!("joiner-hunger-local-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("--no-lend host starts");
    hs.server.difficulty = Difficulty::Hard;
    assert!(!hs.server.players[0].server_simulated, "slot 0 is the host's own");
    hs.server.players[0].combat.health = 12.0;
    hs.server.players[0].combat.hunger = 19;
    for _ in 0..(crate::combat::HUNGER_DRAIN_INTERVAL_TICKS + 100) {
        hs.tick();
    }
    assert_eq!(hs.server.players[0].combat.hunger, 19, "no drain");
    assert_eq!(hs.server.players[0].combat.health, 12.0, "no regen");
}

/// The same metabolism runs for a lending host's joiners (inside the lend
/// window, on the host's own world); the host's local slot is untouched.
#[test]
fn a_lending_hosts_joiner_hunger_drains_on_the_server() {
    let (mut hs, mut host) = super::lent_world::start_lent("c2a-hunger");
    let (client, slot) = super::lent_world::join_guest_lent(&mut hs, &mut host, "Lent");
    let mut world = std::mem::replace(&mut host.world, crate::world::World::new());
    super::joiners_act::floor_and_stand(&mut world, &mut hs, slot);
    host.world = world;
    hs.server.difficulty = Difficulty::Peaceful;
    crate::remote_mobs::purge_private_mobs(&mut host.ecs);
    hs.server.players[slot].combat.hunger_drain_ticks = 0;
    let local_hunger = hs.server.players[0].combat.hunger;
    for _ in 0..crate::combat::HUNGER_DRAIN_INTERVAL_TICKS {
        host.lend_tick(&mut hs);
        while client.try_recv_from_server().is_some() {}
    }
    assert!(hs.server.players[slot].is_present_and_alive());
    assert_eq!(hs.server.players[slot].combat.hunger, 19, "the joiner's hunger drained on the server");
    assert_eq!(hs.server.players[0].combat.hunger, local_hunger, "the host's own slot did not");
}

// ─── Eating (decision 4) ───────────────────────────────────────────────────

#[test]
fn an_accepted_eat_heals_feeds_and_takes_the_food_from_the_shadow() {
    let mut rig = Rig::new("eat");
    rig.sp().inventory.set_slot(0, Some(ItemStack::new_material(MaterialId::Bread, 3)));
    rig.sp().combat.health = 10.0;
    rig.sp().combat.hunger = 10;
    let seq = rig.eat(&bread());
    rig.tick(1);
    let out = rig.inbox.outcome(seq);
    assert!(out.accepted);
    assert_eq!(out.consume_held, 1);
    assert_eq!(out.note, ItemNote::None.to_wire());
    let v = bread_value();
    assert_eq!(rig.sp().combat.health, 10.0 + v, "healed by the food value");
    assert_eq!(rig.sp().combat.hunger, 10 + v as u8, "and fed by it");
    assert_eq!(rig.shadow_count(MaterialId::Bread), 2, "the shadow lost one bread");
    assert_eq!(rig.inbox.last_hunger(), 10 + v as u8, "the joiner hears its new hunger");
}

#[test]
fn a_full_joiner_is_refused_and_nothing_is_taken() {
    let mut rig = Rig::new("eat-full");
    rig.sp().inventory.set_slot(0, Some(ItemStack::new_material(MaterialId::Bread, 3)));
    let seq = rig.eat(&bread());
    rig.tick(1);
    let out = rig.inbox.outcome(seq);
    assert!(!out.accepted);
    assert_eq!(out.consume_held, 0);
    assert_eq!(ItemNote::from_wire(out.note), ItemNote::NotHungry);
    assert_eq!(ItemNote::from_wire(out.note).toast(), Some("You're not hungry."));
    assert_eq!(rig.shadow_count(MaterialId::Bread), 3, "nothing taken");
}

#[test]
fn a_second_eat_inside_the_cooldown_is_refused() {
    let mut rig = Rig::new("eat-cooldown");
    rig.sp().inventory.set_slot(0, Some(ItemStack::new_material(MaterialId::Bread, 5)));
    rig.sp().combat.hunger = 2;
    let first = rig.eat(&bread());
    rig.tick(1);
    assert!(rig.inbox.outcome(first).accepted);
    let hunger = rig.sp().combat.hunger;
    // Read at the server 11 ticks after the first: 5 ticks of the cooldown
    // left, more than the jitter slack forgives (C2a verify M1).
    rig.tick(item_actions::EAT_COOLDOWN_TICKS - item_actions::EAT_JITTER_SLACK_TICKS - 2);
    let second = rig.eat(&bread());
    rig.tick(1);
    let out = rig.inbox.outcome(second);
    assert!(!out.accepted, "11 ticks after the first");
    assert_eq!(ItemNote::from_wire(out.note), ItemNote::TooSoon);
    assert_eq!(rig.sp().combat.hunger, hunger, "no second meal");
    assert_eq!(rig.shadow_count(MaterialId::Bread), 4);
    // One tick on, the cooldown is down to the slack: the next is taken.
    let third = rig.eat(&bread());
    rig.tick(1);
    assert!(rig.inbox.outcome(third).accepted, "12 ticks after the first");
    assert_eq!(rig.shadow_count(MaterialId::Bread), 3);
}

/// Two eats read `apart` server ticks apart: were both accepted?
fn two_eats_apart(tag: &str, apart: u32) -> (bool, bool) {
    let mut rig = Rig::new(tag);
    rig.sp().inventory.set_slot(0, Some(ItemStack::new_material(MaterialId::Bread, 5)));
    rig.sp().combat.hunger = 2;
    let first = rig.eat(&bread());
    rig.tick(1);
    rig.tick(apart - 1);
    let second = rig.eat(&bread());
    rig.tick(1);
    (rig.inbox.outcome(first).accepted, rig.inbox.outcome(second).accepted)
}

#[test]
fn two_eats_13_ticks_apart_are_both_accepted_and_11_apart_are_not() {
    // The client spaces its eats 16 ticks apart; arrival skew can bring two
    // of them 13 apart at the server. Under 12 is a real rate-limit breach.
    assert_eq!(two_eats_apart("eat-13", 13), (true, true), "13 apart: honest, skewed");
    assert_eq!(two_eats_apart("eat-12", 12), (true, true), "12 apart: the slack's edge");
    assert_eq!(two_eats_apart("eat-11", 11), (true, false), "11 apart: too soon");
}

#[test]
fn a_too_soon_refusal_takes_nothing_and_says_nothing() {
    let mut rig = Rig::new("eat-quiet");
    rig.sp().inventory.set_slot(0, Some(ItemStack::new_material(MaterialId::Bread, 5)));
    rig.sp().combat.hunger = 2;
    rig.eat(&bread());
    rig.tick(1);
    let again = rig.eat(&bread());
    rig.tick(1);
    let out = rig.inbox.outcome(again);
    assert_eq!(ItemNote::from_wire(out.note), ItemNote::TooSoon);
    assert_eq!(ItemNote::from_wire(out.note).toast(), None, "no toast for a pacing refusal");
    assert_eq!(rig.shadow_count(MaterialId::Bread), 4);
}

#[test]
fn a_non_food_claim_is_refused() {
    let mut rig = Rig::new("eat-stick");
    rig.sp().combat.hunger = 5;
    let seq = rig.eat(&Item::Material(MaterialId::Stick));
    rig.tick(1);
    let out = rig.inbox.outcome(seq);
    assert!(!out.accepted);
    assert_eq!(ItemNote::from_wire(out.note), ItemNote::NotFood);
    assert_eq!(rig.sp().combat.hunger, 5);
}

/// A shadow short of the food (it never saw the joiner's own bread) is a
/// log-only mismatch, as for D2b's interactions: the eat is still accepted.
#[test]
fn an_eat_the_shadow_cannot_pay_is_still_accepted() {
    let mut rig = Rig::new("eat-short");
    rig.sp().combat.hunger = 5;
    let seq = rig.eat(&bread());
    rig.tick(1);
    assert!(rig.inbox.outcome(seq).accepted);
    assert_eq!(rig.sp().combat.hunger, 5 + bread_value() as u8);
}

#[test]
fn a_dead_joiner_cannot_eat() {
    let mut rig = Rig::new("eat-dead");
    rig.sp().combat.hunger = 5;
    rig.sp().combat.die(crate::survival::DamageCause::Generic);
    let seq = rig.eat(&bread());
    rig.tick(1);
    assert_eq!(ItemNote::from_wire(rig.inbox.outcome(seq).note), ItemNote::NotNow);
}

/// Decision 7 — item actions have a per-tick budget of 4; one past it waits
/// in the inbound queue for the next tick (never dropped, never refused for
/// budget), and every one is answered, in order.
#[test]
fn item_actions_past_the_budget_wait_for_the_next_tick() {
    let mut rig = Rig::new("budget");
    rig.hs.server.world_time = 12000; // noon: every sleep is refused, cheaply
    let bed = rig.bed_ahead();
    let seqs: Vec<u32> = (0..6).map(|_| rig.sleep(bed)).collect();
    rig.tick(1);
    assert_eq!(rig.inbox.outcomes.len(), 4, "four read this tick");
    rig.tick(1);
    let answered: Vec<u32> = rig.inbox.outcomes.iter().map(|o| o.seq).collect();
    assert_eq!(answered, seqs, "the rest the next tick, in order");
    assert!(rig.inbox.outcomes.iter().all(|o| ItemNote::from_wire(o.note) == ItemNote::NotNight));
}

// ─── Sleeping (decision 5) ─────────────────────────────────────────────────

#[test]
fn sleeping_at_night_sets_the_spawn_heals_and_a_respawn_lands_at_the_bed() {
    let mut rig = Rig::new("sleep");
    rig.hs.server.world_time = 0; // midnight
    let bed = rig.bed_ahead();
    rig.sp().combat.health = 8.0;
    rig.sp().combat.hunger = 12;
    let seq = rig.sleep(bed);
    rig.tick(1);
    let out = rig.inbox.outcome(seq);
    assert!(out.accepted, "night, a bed in reach");
    assert_eq!(out.consume_held, 0);
    assert_eq!(rig.sp().spawn_pos, item_actions::bed_spawn(bed));
    assert_eq!(rig.sp().combat.health, 20.0, "healed to full");
    assert_eq!(rig.sp().combat.hunger, 12, "hunger left alone");
    assert!(rig.hs.server.world_time < 100, "the night is not skipped");

    // Die anywhere; the Respawn stands the body on the bed.
    rig.sp().player.pos = rig.at + Vec3::new(-5.0, 0.0, -5.0);
    rig.sp().combat.die(crate::survival::DamageCause::Generic);
    rig.tick(crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN + 1);
    rig.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Respawn, &()));
    rig.tick(1);
    let at = item_actions::bed_spawn(bed);
    assert!(
        rig.inbox.events.contains(&PlayerEventType::Respawned { x: at.x, y: at.y, z: at.z }),
        "respawned standing on the bed: {:?}",
        rig.inbox.events
    );
    assert_eq!(rig.sp().player.pos, at);
}

/// The far-respawn column wait keeps working with a bed spawn on a server
/// that owns its world: the bed's column was streamed out while the joiner
/// was away; the Respawn loads it and lands there.
#[test]
fn a_respawn_at_a_bed_whose_column_was_unloaded_still_lands_there() {
    let mut rig = Rig::new("sleep-far");
    rig.hs.server.world_time = 0;
    let bed = rig.bed_ahead();
    let seq = rig.sleep(bed);
    rig.tick(1);
    assert!(rig.inbox.outcome(seq).accepted);
    let col = crate::chunk_stream::column_of(item_actions::bed_spawn(bed));
    rig.hs.server.world.evict_column(col.0, col.1);
    rig.hs.server.loaded_columns.remove(&col);
    rig.sp().combat.die(crate::survival::DamageCause::Generic);
    rig.tick(crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN + 1);
    rig.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Respawn, &()));
    rig.tick(1);
    assert!(rig.hs.server.loaded_columns.contains(&col), "the bed's column was loaded for it");
    let respawned = rig
        .inbox
        .events
        .iter()
        .find_map(|e| match *e {
            PlayerEventType::Respawned { x, z, .. } => Some((x, z)),
            _ => None,
        })
        .expect("the Respawn was answered");
    let at = item_actions::bed_spawn(bed);
    assert_eq!(respawned, (at.x, at.z), "in the bed's column");
}

#[test]
fn a_second_sleep_the_same_night_is_refused() {
    let mut rig = Rig::new("sleep-twice");
    rig.hs.server.world_time = 0;
    let bed = rig.bed_ahead();
    let first = rig.sleep(bed);
    rig.tick(1);
    assert!(rig.inbox.outcome(first).accepted);
    rig.sp().combat.health = 5.0;
    let second = rig.sleep(bed);
    rig.tick(1);
    let out = rig.inbox.outcome(second);
    assert!(!out.accepted);
    assert_eq!(ItemNote::from_wire(out.note), ItemNote::SleptTonight);
    assert_eq!(rig.sp().combat.health, 5.0, "no second heal");
    // The next night it may.
    rig.hs.server.world_time = 12000;
    rig.tick(1);
    rig.hs.server.world_time = 20000;
    rig.tick(1);
    let next_night = rig.sleep(bed);
    rig.tick(1);
    assert!(rig.inbox.outcome(next_night).accepted, "a new night");
}

#[test]
fn sleeping_by_day_or_at_a_far_bed_or_no_bed_is_refused() {
    let mut rig = Rig::new("sleep-refused");
    let bed = rig.bed_ahead();
    rig.hs.server.world_time = 12000;
    let day = rig.sleep(bed);
    rig.tick(1);
    assert_eq!(ItemNote::from_wire(rig.inbox.outcome(day).note), ItemNote::NotNight);

    rig.hs.server.world_time = 0;
    let far = [bed[0] + 30, bed[1], bed[2]];
    rig.hs.server.world.set_block(far[0], far[1], far[2], block::BED);
    let too_far = rig.sleep(far);
    rig.tick(1);
    assert_eq!(ItemNote::from_wire(rig.inbox.outcome(too_far).note), ItemNote::BedTooFar);

    let no_bed = rig.sleep([bed[0] + 1, bed[1], bed[2]]);
    rig.tick(1);
    assert_eq!(ItemNote::from_wire(rig.inbox.outcome(no_bed).note), ItemNote::NotABed);
    let spawn = rig.sp().spawn_pos;
    assert_ne!(spawn, item_actions::bed_spawn(bed), "no refused sleep set the spawn");
}

// ─── C2a-fix: tick-paced eating, the night index, the ordering N4 relies on ─

/// Bites a held-down right-click gets in 10 s of play at `fps`, through the
/// slot's own timer: the fixed 20 Hz tick decrements it, every frame asks.
fn bites_in_ten_seconds(fps: u32) -> u32 {
    let mut slot = crate::player_slot::PlayerSlot::new(0, Vec3::new(0.0, 64.0, 0.0), 1.0);
    let (mut t, mut next_tick, mut bites) = (0.0_f64, 0.0_f64, 0);
    while t < 10.0 {
        while next_tick <= t {
            slot.tick_eat_cooldown();
            next_tick += 0.05;
        }
        if crate::health_sync::may_eat_now(slot.eat_cooldown, false, false) {
            slot.start_eat_cooldown();
            bites += 1;
        }
        t += 1.0 / f64::from(fps);
    }
    bites
}

#[test]
fn eating_is_paced_in_ticks_at_any_frame_rate() {
    // One bite per 0.8 s (16 ticks): 13 in 10 s counting the first. The old
    // frame-counted cooldown gave 10 s * fps / 16 bites — 90 at 144 fps.
    for fps in [20, 30, 60, 75, 144, 240] {
        let bites = bites_in_ten_seconds(fps);
        assert!((12..=13).contains(&bites), "{fps} fps: {bites} bites in 10 s");
    }
}

#[test]
fn a_sleep_in_the_first_night_tick_after_a_slept_night_is_a_new_night() {
    // Night k: the joiner sleeps. The clock goes to day, then the host sets
    // dusk — before inbound processing, so the calendar has not seen it. The
    // Sleep arm shows it the clock first: a NEW night, so it is accepted
    // (pre-fix it was refused as the night just slept) and marked as it.
    let mut rig = Rig::new("night-after-slept");
    rig.hs.server.world_time = 0;
    rig.tick(1);
    let bed = rig.bed_ahead();
    let first = rig.sleep(bed);
    rig.tick(1);
    assert!(rig.inbox.outcome(first).accepted);
    let slept = rig.hs.server.night_calendar.tonight();
    assert_eq!(rig.sp().slept_night, Some(slept));
    rig.hs.server.world_time = 12000;
    rig.tick(2);
    rig.sp().combat.health = 5.0;
    rig.hs.server.world_time = 20000; // dusk, not yet observed
    let next = rig.sleep(bed);
    rig.tick(1);
    assert!(rig.inbox.outcome(next).accepted, "the first tick of the next night");
    assert_eq!(rig.sp().slept_night, Some(slept + 1));
    assert_eq!(rig.hs.server.night_calendar.tonight(), slept + 1);
    assert_eq!(rig.sp().combat.health, 20.0);
}

#[test]
fn a_sleep_in_the_first_night_tick_after_an_unslept_night_is_counted_as_tonights() {
    // The joiner slept no night. The first tick of dusk it sleeps: accepted,
    // and marked as THIS night — so a second sleep a tick later is refused
    // and heals nothing (pre-fix the mark was last night's, and the
    // calendar's catch-up let it sleep and heal again the same night).
    let mut rig = Rig::new("night-first-tick");
    rig.hs.server.world_time = 12000;
    rig.tick(2);
    let bed = rig.bed_ahead();
    rig.hs.server.world_time = 20000; // dusk, not yet observed
    let first = rig.sleep(bed);
    rig.tick(1);
    assert!(rig.inbox.outcome(first).accepted);
    let tonight = rig.hs.server.night_calendar.tonight();
    assert_eq!(rig.sp().slept_night, Some(tonight), "marked with tonight's number");
    rig.sp().combat.health = 5.0;
    let second = rig.sleep(bed);
    rig.tick(1);
    let out = rig.inbox.outcome(second);
    assert!(!out.accepted);
    assert_eq!(ItemNote::from_wire(out.note), ItemNote::SleptTonight);
    assert_eq!(rig.sp().combat.health, 5.0, "no second heal");
}

/// What the joiner's stream held, in the order it arrived.
#[derive(Debug, PartialEq)]
enum Arrived {
    Outcome { seq: u32 },
    State { acked: u64 },
}

#[test]
fn an_outcome_arrives_before_the_state_update_that_acknowledges_the_input_after_it() {
    // C2a verify L4 — the ordering N4's claim-ending relies on, pinned over
    // the channel transport with a real `JoinerActions`: the request goes
    // out ahead of input `next`; the server answers it inline while reading
    // that input's packets, and the StateUpdate acknowledging `next` is
    // built after. If the ack could overtake the outcome the claim would
    // end early (can_afford passes, a second request spends the same item).
    use crate::joiner_actions::{apply_item_outcome, Asked, JoinerActions, Pending};
    let mut rig = Rig::new("ordering");
    let two_bread = ItemStack::new_material(MaterialId::Bread, 2);
    rig.sp().inventory.set_slot(0, Some(two_bread.clone()));
    rig.sp().combat.hunger = 2;
    let mut inv = crate::inventory::Inventory::new();
    inv.set_slot(0, Some(two_bread));

    let mut ja = JoinerActions::default();
    let next_input = 1_u64; // the first input of the session
    let ask = Pending { kind: Asked::Eat, mob: None, hotbar_slot: 0, held: Some(bread()) };
    assert!(ja.can_afford(&inv, Asked::Eat, Some(&bread())));
    let seq = ja.record(ask, next_input);
    assert_eq!(rig.eat(&bread()), seq, "the request goes out under the seq it was recorded with");
    assert!(ja.eat_in_flight(), "the request is in flight");
    let input = protocol::InputPacket {
        tick: next_input,
        x: rig.at.x,
        y: rig.at.y,
        z: rig.at.z,
        health: 20.0,
        ..Default::default()
    };
    rig.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
    rig.hs.tick();

    let mut stream = Vec::new();
    while let Some(pkt) = rig.client.try_recv_from_server() {
        let Some((ptype, payload)) = protocol::deserialize_header(&pkt) else { continue };
        match ptype {
            protocol::PacketType::ItemActionOutcome => {
                let o: protocol::ItemActionOutcomePacket = protocol::safe_deserialize(payload).unwrap();
                stream.push((Arrived::Outcome { seq: o.seq }, Some(o)));
            }
            protocol::PacketType::StateUpdate => {
                let st: protocol::StateUpdatePacket = protocol::safe_deserialize(payload).unwrap();
                stream.push((Arrived::State { acked: st.last_acked_input }, None));
            }
            _ => {}
        }
    }
    let at = |wanted: &Arrived| stream.iter().position(|(a, _)| a == wanted);
    let outcome_at = at(&Arrived::Outcome { seq }).expect("the request was answered");
    let ack_at = stream
        .iter()
        .position(|(a, _)| matches!(a, Arrived::State { acked } if *acked >= next_input))
        .expect("a StateUpdate acknowledged the input");
    assert!(outcome_at < ack_at, "outcome before its acknowledgement: {:?}", stream.iter().map(|s| &s.0).collect::<Vec<_>>());

    // Replay it the way the client does, in arrival order: the claim is
    // PAID (one bread taken), not released unpaid.
    for (arrived, outcome) in &stream {
        match (arrived, outcome) {
            (Arrived::Outcome { seq }, Some(o)) => {
                let pending = ja.take(*seq).expect("still waiting for it");
                assert_eq!(apply_item_outcome(&mut inv, &pending, o), 1);
            }
            (Arrived::State { acked }, _) => ja.acknowledged(*acked),
            _ => unreachable!(),
        }
    }
    assert_eq!(count(&inv), 1, "paid exactly once");
    assert_eq!(rig.shadow_count(MaterialId::Bread), 1, "the server took the same one");
    assert!(ja.can_afford(&inv, Asked::Eat, Some(&bread())), "the second bread is free to eat");
}

fn count(inv: &crate::inventory::Inventory) -> u32 {
    inv.count_material(MaterialId::Bread) as u32
}
