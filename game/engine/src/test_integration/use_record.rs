//! Living world Tier 2 S3: how a place is used, nights slept, and Worn Paths.
//!
//! The save half runs real servers over a real folder. The joiner half runs a
//! real `HostedServer` with a joiner over the channel transport (walking, and
//! sleeping in a bed); the lending half lends a host's world and checks the
//! phase runs once per tick and that a host's own seat wears paths too.
use super::joiner_authority::{block_changes_seen, join_guest};
use crate::block;
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::item_actions;
use crate::protocol::{self, ItemAction};
use crate::save::{read_world_save, world_dir, WorldsRootGuard};
use crate::server::GameServer;
use crate::sim_lend::SimSystem;
use crate::transport::{ChannelClientTransport, ClientTransport};
use crate::use_record::{RegionUse, SavedRegionUse};
use crate::world::World;
use glam::Vec3;

fn saved_world_save(world: &str) -> crate::save::WorldSave {
    read_world_save(&std::fs::read(world_dir(world).join("world.dat")).unwrap()).unwrap()
}

// ── Save and load ───────────────────────────────────────────────────

#[test]
fn the_use_record_survives_a_save_and_load_and_the_wear_counts_do_not() {
    let _g = WorldsRootGuard::new("use-record-roundtrip");
    let mut s = GameServer::new(0, "w".into(), 42);
    s.world.use_record.note_present_at(5, 40, 40);
    s.world.use_record.note_present_at(6, 40, 40);
    s.world.use_record.note_present_at(6, -20, 5);
    s.world.use_record.note_slept_at(3, 41, 41);
    s.world.path_wear = Default::default();
    s.world.set_block(1, 40, 1, block::GRASS);
    for i in 0..5 {
        let x = if i % 2 == 0 { 1.5 } else { 2.5 };
        let foot = crate::path_wear::Foot { key: 0, pos: Vec3::new(x, 41.0, 1.5), on_ground: true };
        crate::path_wear::tick_wear(&mut s.world, &[foot], false);
    }
    assert!(!s.world.path_wear.is_empty(), "a partial count exists");
    s.try_save().unwrap();

    let saved = saved_world_save("w").region_use.expect("the field is written");
    assert_eq!(saved.lived_day, 2);
    assert_eq!(saved.regions.len(), 2, "regions (2,2) and (-2,0)");

    let mut back = GameServer::new(0, "w".into(), 42);
    back.initial_load().unwrap();
    assert_eq!(back.world.use_record, s.world.use_record);
    assert_eq!(back.world.use_record.get((2, 2)), Some(&RegionUse { days: 2, last_day: 2, nights: 1, last_night: Some(3) }));
    assert!(back.world.path_wear.is_empty(), "partial wear is runtime only");
    // It is live: the next day adds to it and a second save carries it.
    back.world.use_record.note_present_at(9, 40, 40);
    back.try_save().unwrap();
    assert_eq!(saved_world_save("w").region_use.unwrap().lived_day, 3);
}

#[test]
fn a_world_nobody_has_used_saves_the_field_absent() {
    let _g = WorldsRootGuard::new("use-record-absent");
    let mut s = GameServer::new(0, "w".into(), 42);
    s.world.place_player_block(3, 64, 5, block::STONE);
    s.try_save().unwrap();
    assert!(saved_world_save("w").region_use.is_none());
}

#[test]
fn a_footer_62_save_loads_with_an_empty_table() {
    // A footer-62 writer (Tier 2 S2) had 57 fields: the same bytes without the new last one.
    let mut save = crate::save::minimal_world_save_for_tests(7);
    save.region_use = None;
    let mut body = bincode::serialize(&save).unwrap();
    assert_eq!(body.pop(), Some(0), "the absent field is one None byte");
    body.extend_from_slice(&crate::save_format::footer_bytes(62));
    let (old, damaged) = crate::save::read_world_save_reporting(&body).expect("an older save loads");
    assert!(!damaged);
    assert!(old.region_use.is_none());
    let mut w = World::new();
    w.use_record.note_present_at(4, 3, 3);
    crate::save::apply_world_save_state(&mut w, &old);
    assert!(w.use_record.is_empty(), "restoring a save without the field leaves nothing used");
    assert_eq!(w.use_record.lived_day(), 0);
}

#[test]
fn the_encoding_is_the_same_whatever_order_regions_were_used_in() {
    let uses: Vec<(i32, i32)> = vec![(40, 40), (-20, 5), (300, -300), (7, 7)];
    let bytes_for = |name: &str, order: &[usize]| {
        let _g = WorldsRootGuard::new(name);
        let mut s = GameServer::new(0, "w".into(), 42);
        s.world.use_record.begin_day(4);
        for &i in order {
            let (x, z) = uses[i];
            s.world.use_record.note_present_at(4, x, z);
        }
        s.world.use_record.note_slept_at(2, 40, 40);
        s.try_save().unwrap();
        bincode::serialize(&saved_world_save("w").region_use.unwrap()).unwrap()
    };
    assert_eq!(bytes_for("use-record-order-a", &[0, 1, 2, 3]), bytes_for("use-record-order-b", &[3, 2, 1, 0]));
}

#[test]
fn an_invalid_row_is_dropped_with_a_warning_never_a_refusal_to_load() {
    let mut save = crate::save::minimal_world_save_for_tests(8);
    let used = RegionUse { days: 1, last_day: 1, nights: 0, last_night: None };
    save.region_use = Some(SavedRegionUse {
        lived_day: 1,
        last_world_day: Some(0),
        regions: vec![((0, 0), used), ((1, 0), RegionUse { last_day: 50, ..used })],
    });
    let bytes = crate::save_format::encode_world_save(&save).unwrap();
    let (back, damaged) = crate::save::read_world_save_reporting(&bytes).expect("the world still loads");
    assert!(!damaged);
    assert_eq!(back.region_use, save.region_use, "the reader keeps what it was given");
    let mut w = World::new();
    crate::save::apply_world_save_state(&mut w, &back);
    assert!(w.use_record.get((0, 0)).is_some());
    assert!(w.use_record.get((1, 0)).is_none(), "used in the future of its own record");
}

#[test]
fn the_server_digest_covers_the_use_record_in_a_canonical_order() {
    let digest = |name: &str, order: &[i32]| {
        let _g = WorldsRootGuard::new(name);
        let mut s = GameServer::new(0, "w".into(), 42);
        s.world.use_record.begin_day(2);
        for &x in order {
            s.world.use_record.note_present_at(2, x, 5);
        }
        s.sim_digest()
    };
    let one = digest("use-record-digest-1", &[10, 100]);
    assert_eq!(one, digest("use-record-digest-2", &[100, 10]));
    assert_ne!(one, digest("use-record-digest-3", &[10]), "where people have lived is part of the digest");
}

// ── A joiner on a dedicated server ──────────────────────────────────

/// A dedicated server with one guest standing on a stone floor at (40, 80, 40)
/// (`floor_and_stand`), survival, no mobs, no streaming.
struct Rig {
    hs: HostedServer,
    client: ChannelClientTransport,
    slot: usize,
    input: u64,
    seq: u32,
    changes: Vec<protocol::BlockChange>,
    outcomes: Vec<protocol::ItemActionOutcomePacket>,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let mut hs = HostedServer::start(
            0,
            format!("use-record-{tag}-{}", std::process::id()),
            42,
            0,
            RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        let (client, slot) = join_guest(&mut hs, "Walker");
        let mut world = std::mem::replace(&mut hs.server.world, World::new());
        super::joiners_act::floor_and_stand(&mut world, &mut hs, slot);
        hs.server.world = world;
        hs.server.column_streamer = None;
        hs.server.column_refill_per_tick = 0;
        hs.server.difficulty = crate::survival::Difficulty::Peaceful;
        crate::remote_mobs::purge_private_mobs(&mut hs.server.ecs);
        hs.server.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut rig = Rig { hs, client, slot, input: 0, seq: 0, changes: Vec::new(), outcomes: Vec::new() };
        rig.tick();
        rig
    }

    fn tick(&mut self) {
        self.hs.tick();
        while let Some(pkt) = self.client.try_recv_from_server() {
            let Some((ptype, payload)) = protocol::deserialize_header(&pkt) else { continue };
            match ptype {
                protocol::PacketType::StateUpdate => {
                    if let Ok(s) = protocol::safe_deserialize::<protocol::StateUpdatePacket>(payload) {
                        self.changes.extend(s.block_changes);
                    }
                }
                protocol::PacketType::ItemActionOutcome => {
                    self.outcomes.push(protocol::safe_deserialize(payload).unwrap());
                }
                _ => {}
            }
        }
    }

    fn pos(&self) -> Vec3 {
        self.hs.server.players[self.slot].player.pos
    }

    /// One tick of walking along x at `yaw`.
    fn walk(&mut self, yaw: f32) {
        self.input += 1;
        let at = self.pos();
        let input = protocol::InputPacket {
            tick: self.input,
            yaw,
            health: 20.0,
            move_forward: 1.0,
            x: at.x,
            y: at.y,
            z: at.z,
            ..Default::default()
        };
        self.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
        self.tick();
    }

    /// The yaw that walks towards +x (`east`) or -x.
    fn yaw_towards(east: bool) -> f32 {
        [0.0f32, std::f32::consts::FRAC_PI_2, std::f32::consts::PI, -std::f32::consts::FRAC_PI_2]
            .into_iter()
            .find(|&yaw| {
                let f = crate::camera::forward_from(yaw, 0.0);
                if east { f.x > 0.9 } else { f.x < -0.9 }
            })
            .expect("one of the four quarter turns faces along x")
    }

    /// A lane of grass at z = 40, x in 37..=43, laid over the stone floor.
    fn lay_lane(&mut self) {
        for x in 37..=43 {
            self.hs.server.world.set_block(x, 79, 40, block::GRASS);
        }
    }

    /// Pace the lane until `done` or `max` ticks; returns the ticks taken.
    fn pace_until(&mut self, max: u32, done: impl Fn(&Rig) -> bool) -> u32 {
        let (east, west) = (Self::yaw_towards(true), Self::yaw_towards(false));
        let mut going_east = true;
        for t in 0..max {
            if done(self) {
                return t;
            }
            let x = self.pos().x;
            if going_east && x >= 42.8 {
                going_east = false;
            } else if !going_east && x <= 37.7 {
                going_east = true;
            }
            self.walk(if going_east { east } else { west });
        }
        max
    }
}

#[test]
fn a_joiner_walking_back_and_forth_over_grass_wears_a_path_the_joiner_sees() {
    let mut rig = Rig::new("walk");
    rig.lay_lane();
    let lane = |rig: &Rig| (37..=43).filter(|&x| rig.hs.server.world.get_block(x, 79, 40) == block::WORN_PATH).count();
    // Early on nothing has worn: 24 footfalls on one cell take 24 passes over it.
    rig.pace_until(60, |_| false);
    assert_eq!(lane(&rig), 0, "a couple of passes make no path");
    let used = rig.pace_until(3000, |r| lane(r) > 0);
    assert!(used < 3000, "the lane wears through in the end");
    assert!(rig.hs.server.world.use_record.lived_day() >= 1, "and the same body has lived a day here");
    // The block is the authority's: not placed, nobody's, and the joiner was told.
    let worn: Vec<i32> = (37..=43).filter(|&x| rig.hs.server.world.get_block(x, 79, 40) == block::WORN_PATH).collect();
    for &x in &worn {
        assert!(!rig.hs.server.world.is_placed(x, 79, 40));
        assert!(
            rig.changes.iter().any(|c| (c.x, c.y, c.z) == (x, 79, 40) && c.new_block == block::WORN_PATH),
            "the joiner heard that ({x}, 79, 40) became a Worn Path"
        );
    }
    // Only the lane wore; the stone floor beside it is untouched, and a changed cell is told once.
    assert!(rig.changes.iter().filter(|c| c.new_block == block::WORN_PATH).all(|c| c.z == 40 && c.y == 79));
    assert_eq!(
        rig.changes.iter().filter(|c| c.new_block == block::WORN_PATH).count(),
        worn.len(),
        "each worn cell is sent once"
    );
}

#[test]
fn a_joiner_in_a_creative_world_wears_no_path() {
    let mut rig = Rig::new("walk-creative");
    rig.hs.server.set_play_mode(crate::play_mode::PlayMode::Creative);
    rig.lay_lane();
    rig.pace_until(1500, |_| false);
    assert!((37..=43).all(|x| rig.hs.server.world.get_block(x, 79, 40) == block::GRASS));
    assert!(rig.hs.server.world.path_wear.is_empty());
    assert!(rig.hs.server.world.use_record.lived_day() >= 1, "but the day is still lived there");
}

#[test]
fn a_dead_joiner_is_not_present_so_no_lived_day_passes() {
    let mut rig = Rig::new("presence-dead");
    rig.hs.server.players[rig.slot].combat.die(crate::survival::DamageCause::Generic);
    for _ in 0..100 {
        rig.tick();
    }
    assert!(rig.hs.server.world.use_record.is_empty(), "nobody alive is present");
}

#[test]
fn a_standing_joiner_counts_one_lived_day_in_its_region_however_long_it_stands() {
    let mut rig = Rig::new("presence");
    for _ in 0..200 {
        rig.tick();
    }
    let used = rig.hs.server.world.use_record.get((2, 2)).copied().expect("the joiner's region");
    assert_eq!((used.days, rig.hs.server.world.use_record.lived_day()), (1, 1), "ten seconds, one lived day");
}

fn bed_ahead(rig: &mut Rig) -> [i32; 3] {
    let at = rig.pos();
    let bed = [at.x.floor() as i32, at.y as i32, at.z.floor() as i32 + 2];
    rig.hs.server.world.set_block(bed[0], bed[1], bed[2], block::BED);
    bed
}

fn sleep(rig: &mut Rig, bed: [i32; 3]) -> protocol::ItemActionOutcomePacket {
    rig.seq += 1;
    let pkt = protocol::ItemActionPacket { seq: rig.seq, action: ItemAction::Sleep { bed }, events_applied: u32::MAX };
    rig.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
    rig.tick();
    let seq = rig.seq;
    rig.outcomes.iter().find(|o| o.seq == seq).expect("answered").clone()
}

#[test]
fn a_joiners_sleep_adds_a_night_to_the_beds_region_once_a_night() {
    let mut rig = Rig::new("sleep");
    let bed = bed_ahead(&mut rig);
    rig.hs.server.world_time = 0; // midnight
    let region = crate::use_record::region_of(bed[0], bed[2]);
    assert_eq!(rig.hs.server.world.use_record.get(region).map(|r| r.nights), None);
    assert!(sleep(&mut rig, bed).accepted);
    let r = *rig.hs.server.world.use_record.get(region).expect("the bed's region");
    assert_eq!(r.nights, 1);
    assert_eq!(r.last_night, Some(rig.hs.server.night_calendar.tonight()));
    // Refused (once a night): nothing more is counted.
    let again = sleep(&mut rig, bed);
    assert!(!again.accepted);
    assert_eq!(rig.hs.server.world.use_record.get(region).unwrap().nights, 1);
    // A refused sleep by day counts nothing either.
    rig.hs.server.world_time = 12_000;
    rig.tick();
    let by_day = sleep(&mut rig, bed);
    assert!(!by_day.accepted);
    assert_eq!(rig.hs.server.world.use_record.get(region).unwrap().nights, 1);
    // The next night counts again.
    rig.hs.server.world_time = 20_000;
    rig.tick();
    assert!(sleep(&mut rig, bed).accepted);
    assert_eq!(rig.hs.server.world.use_record.get(region).unwrap().nights, 2);
    let _ = item_actions::bed_spawn(bed);
}

// ── A lending host ──────────────────────────────────────────────────

#[test]
fn on_a_lending_host_the_phase_runs_once_per_tick_and_the_hosts_own_seat_wears_paths() {
    let (mut hs, mut host) = super::lent_world::start_lent("use-record");
    let (client, slot) = super::lent_world::join_guest_lent(&mut hs, &mut host, "Visitor");
    assert!(hs.server.players[0].is_present_and_alive(), "the host's own seat is a present body");
    // A grass lane under the host's seat, over a stone floor, in the host's world.
    for x in 37..=43 {
        host.world.set_block(x, 79, 40, block::GRASS);
        for y in 80..=83 {
            host.world.set_block(x, y, 40, block::AIR);
        }
    }
    let _ = block_changes_seen(&client);
    let tally = |host: &crate::sim_lend::OwnedSimParts| host.world.sim_tally.get(SimSystem::UseRecord);
    // A position-trusted seat reports no `on_ground`; it paces the lane by being put there.
    let mut seen = Vec::new();
    for i in 0..60u32 {
        let x = if i % 2 == 0 { 40.5 } else { 41.5 };
        hs.server.players[0].player.pos = Vec3::new(x, 80.0, 40.5);
        let before = tally(&host);
        host.lend_tick(&mut hs);
        assert_eq!(tally(&host) - before, 1, "tick {i}: once per lent tick, not twice (and not skipped)");
        seen.extend(block_changes_seen(&client));
    }
    // Lived days and the day's presence were counted once.
    assert_eq!(host.world.use_record.lived_day(), 1);
    assert_eq!(host.world.use_record.get((2, 2)).map(|r| r.days), Some(1));
    // 30 footfalls on each of the two cells: both are Worn Paths, in the host's own world,
    // and the joiner was told.
    for x in [40, 41] {
        assert_eq!(host.world.get_block(x, 79, 40), block::WORN_PATH, "({x}, 79, 40)");
        assert!(seen.iter().any(|c| (c.x, c.y, c.z) == (x, 79, 40) && c.new_block == block::WORN_PATH));
    }
    let _ = slot;
}
