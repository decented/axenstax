//! Living world Tier 2 S8: blight on big single-crop fields.
//!
//! The save half runs real servers over a real folder. The joiner half runs a
//! real dedicated `HostedServer` (the same shared growth pass a lending host and
//! the standalone path reach through `sim_machines::step`) with a joiner on the
//! channel transport, and checks the joiner is sent the blight with its kind; the
//! same rig shows creative and a running trial blight nothing.
use super::joiner_authority::join_guest;
use crate::blight::CropKind;
use crate::block;
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::protocol;
use crate::save::{read_world_save, world_dir, WorldsRootGuard};
use crate::server::GameServer;
use crate::transport::{ChannelClientTransport, ClientTransport};

/// A dedicated server with a guest on a stone floor at (40, 80, 40), and a full
/// 16 x 16 field of `crop` laid at y = 80 over tilled soil in the guest's chunk
/// column (the column the guest hears about), sunlit.
struct Rig {
    hs: HostedServer,
    client: ChannelClientTransport,
    changes: Vec<protocol::BlockChange>,
}

const FIELD_X: i32 = 32;
const FIELD_Z: i32 = 32;

impl Rig {
    fn new(tag: &str, crop: block::BlockId) -> Self {
        let mut hs = HostedServer::start(
            0,
            format!("blight-{tag}-{}", std::process::id()),
            42,
            0,
            RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        let (client, slot) = join_guest(&mut hs, "Farmer");
        let mut world = std::mem::replace(&mut hs.server.world, crate::world::World::new());
        super::joiners_act::floor_and_stand(&mut world, &mut hs, slot);
        for x in FIELD_X..FIELD_X + 16 {
            for z in FIELD_Z..FIELD_Z + 16 {
                world.set_block(x, 79, z, block::TILLED_SOIL);
                world.set_block(x, 80, z, crop);
                world.set_sky_light_at(x, 80, z, 15);
            }
        }
        hs.server.world = world;
        hs.server.column_streamer = None;
        hs.server.column_refill_per_tick = 0;
        hs.server.difficulty = crate::survival::Difficulty::Peaceful;
        crate::remote_mobs::purge_private_mobs(&mut hs.server.ecs);
        hs.server.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut rig = Rig { hs, client, changes: Vec::new() };
        rig.tick();
        rig
    }

    fn tick(&mut self) {
        self.hs.tick();
        while let Some(pkt) = self.client.try_recv_from_server() {
            let Some((ptype, payload)) = protocol::deserialize_header(&pkt) else { continue };
            if ptype == protocol::PacketType::StateUpdate
                && let Ok(s) = protocol::safe_deserialize::<protocol::StateUpdatePacket>(payload)
            {
                self.changes.extend(s.block_changes);
            }
        }
    }

    /// One growth pass: the clock set one tick short of the next multiple of 200
    /// (a clock restored mid-count, with the beat counter following it as a
    /// restore does), so this tick steps the clock onto the cadence.
    fn pass(&mut self) {
        self.hs.server.tick_counter = (self.hs.server.tick_counter / 200 + 1) * 200 - 1;
        self.hs.server.falling_tick_counter = crate::sim_clock::beat_counter_for(self.hs.server.tick_counter);
        self.tick();
    }

    fn blighted_in_world(&self) -> usize {
        (FIELD_X..FIELD_X + 16)
            .flat_map(|x| (FIELD_Z..FIELD_Z + 16).map(move |z| (x, z)))
            .filter(|&(x, z)| self.hs.server.world.get_block(x, 80, z) == block::BLIGHTED_CROP)
            .count()
    }

    fn blights_seen(&self) -> Vec<protocol::BlockChange> {
        self.changes.iter().filter(|c| c.new_block == block::BLIGHTED_CROP).cloned().collect()
    }
}

#[test]
fn a_joiner_is_sent_the_blight_of_a_big_wheat_field_with_its_kind() {
    let mut rig = Rig::new("joiner", block::WHEAT_STAGE_0);
    // The crops keep growing between passes; put the field back to stage 0 so
    // every live crop "would advance" and none matures out of the roll.
    for _ in 0..30 {
        for x in FIELD_X..FIELD_X + 16 {
            for z in FIELD_Z..FIELD_Z + 16 {
                if crate::growth::is_crop(rig.hs.server.world.get_block(x, 80, z)) {
                    rig.hs.server.world.set_block(x, 80, z, block::WHEAT_STAGE_0);
                }
            }
        }
        rig.pass();
        if !rig.blights_seen().is_empty() {
            break;
        }
    }
    let seen = rig.blights_seen();
    assert!(!seen.is_empty(), "256 wheat in one column blight within 30 passes");
    assert!(rig.blighted_in_world() >= seen.len().min(1));
    for c in &seen {
        assert_eq!(c.meta, CropKind::Wheat.index(), "the joiner is told the kind");
        assert_eq!(rig.hs.server.world.meta_at(c.x, c.y, c.z), CropKind::Wheat.index());
        assert!(!rig.hs.server.world.is_placed(c.x, c.y, c.z));
    }
}

#[test]
fn a_mixed_field_blights_nothing_on_the_server() {
    let mut rig = Rig::new("mixed", block::WHEAT_STAGE_0);
    for x in FIELD_X..FIELD_X + 16 {
        for z in FIELD_Z..FIELD_Z + 16 {
            if (x + z) % 2 == 0 {
                rig.hs.server.world.set_block(x, 80, z, block::CARROT_STAGE_0);
            }
        }
    }
    for _ in 0..12 {
        rig.pass();
    }
    assert_eq!(rig.blighted_in_world(), 0);
    assert!(rig.blights_seen().is_empty());
}

#[test]
fn a_creative_world_and_a_running_trial_blight_nothing() {
    // Control first: the same field does blight in survival with nothing running.
    let grow_and_count = |rig: &mut Rig, passes: u32| {
        for _ in 0..passes {
            for x in FIELD_X..FIELD_X + 16 {
                for z in FIELD_Z..FIELD_Z + 16 {
                    if crate::growth::is_crop(rig.hs.server.world.get_block(x, 80, z)) {
                        rig.hs.server.world.set_block(x, 80, z, block::WHEAT_STAGE_0);
                    }
                }
            }
            rig.pass();
        }
        rig.blighted_in_world()
    };
    let mut survival = Rig::new("control", block::WHEAT_STAGE_0);
    assert!(grow_and_count(&mut survival, 20) > 0, "the control blights");

    let mut creative = Rig::new("creative", block::WHEAT_STAGE_0);
    creative.hs.server.set_play_mode(crate::play_mode::PlayMode::Creative);
    assert_eq!(grow_and_count(&mut creative, 20), 0, "creative");

    let mut trial = Rig::new("trial", block::WHEAT_STAGE_0);
    trial.hs.server.world.trial_running = true;
    assert_eq!(grow_and_count(&mut trial, 20), 0, "a trial is running");
}

#[test]
fn a_blight_keeps_its_kind_across_a_save_and_load() {
    let _g = WorldsRootGuard::new("blight-roundtrip");
    let mut s = GameServer::new(0, "w".into(), 42);
    s.world.set_block(3, 70, 3, block::HEMP_STAGE_2);
    s.world.set_block(4, 70, 3, block::BUTTERCUP_STAGE_1);
    crate::blight::blight_cell(&mut s.world, 3, 70, 3, CropKind::Hemp);
    crate::blight::blight_cell(&mut s.world, 4, 70, 3, CropKind::Buttercup);
    s.try_save().unwrap();
    let on_disk = read_world_save(&std::fs::read(world_dir("w").join("world.dat")).unwrap()).unwrap();
    assert!(on_disk.block_meta.contains(&(3, 70, 3, CropKind::Hemp.index())));

    let mut back = GameServer::new(0, "w".into(), 42);
    back.initial_load().unwrap();
    assert_eq!(back.world.get_block(3, 70, 3), block::BLIGHTED_CROP);
    assert_eq!(crate::blight::kind_remembered(&back.world, 3, 70, 3), Some(CropKind::Hemp));
    assert_eq!(crate::blight::kind_remembered(&back.world, 4, 70, 3), Some(CropKind::Buttercup));
    // And it still spreads after the reload: only to its own kind.
    back.world.set_block(3, 70, 4, block::HEMP_STAGE_3);
    back.world.set_block(3, 70, 2, block::WHEAT_STAGE_3);
    for p in 1..=400u64 {
        crate::blight::spread(&mut back.world, 200 * p, vec![(3, 70, 3)]);
    }
    assert_eq!(back.world.get_block(3, 70, 4), block::BLIGHTED_CROP);
    assert_eq!(back.world.get_block(3, 70, 2), block::WHEAT_STAGE_3);
}

#[test]
fn a_blighted_crop_breaks_to_nothing_for_every_breaker() {
    let reg = block::BlockRegistry::new();
    assert!(!crate::break_drops::yields_drops(block::BLIGHTED_CROP));
    let mut w = crate::world::World::new();
    crate::blight::blight_cell(&mut w, 1, 70, 1, CropKind::Carrot);
    let y = crate::break_drops::break_yield(
        &w,
        &reg,
        block::BLIGHTED_CROP,
        (1, 70, 1),
        None,
        100,
        &crate::break_drops::PopKeys { secret: &[7u8; 32], world_seed: 42, epoch: 0 },
    );
    assert!(y.drops.is_empty(), "no drops: {:?}", y.drops);
    assert_eq!(y.replacement, block::AIR);
    assert!(y.gem.is_none());
    assert!(!y.crop, "it is not harvested like a crop");
    assert_eq!(reg.mine_drop(block::BLIGHTED_CROP).count, 0);
}

#[test]
fn whatever_takes_a_blighted_crop_away_takes_its_kind_byte_with_it() {
    // `World::set_block` is the funnel every break reaches (a local seat, a
    // lending host's, a joiner's edit on the server, a blast, a flood, a piston).
    for replacement in [block::AIR, block::WHEAT_STAGE_0, block::STONE, block::WATER] {
        let mut w = crate::world::World::new();
        w.set_block(5, 70, 5, block::WHEAT_STAGE_1);
        crate::blight::blight_cell(&mut w, 5, 70, 5, CropKind::Cotton);
        assert_eq!(w.meta_at(5, 70, 5), CropKind::Cotton.index());
        w.set_block(5, 70, 5, replacement);
        assert_eq!(w.meta_at(5, 70, 5), 0, "the kind byte left with the crop (-> {replacement})");
        // And what is put there next starts clean, and is sent clean.
        w.set_block(5, 70, 5, block::WHEAT_STAGE_0);
        assert_eq!(w.meta_at(5, 70, 5), 0);
        assert_eq!(crate::protocol::BlockChange::with_meta(5, 70, 5, block::WHEAT_STAGE_0, w.meta_at(5, 70, 5)).meta, 0);
        assert!(!w.block_meta.contains_key(&(5, 70, 5)), "nothing left to save for the cell");
    }
    // Setting the same blighted block again keeps its kind.
    let mut w = crate::world::World::new();
    crate::blight::blight_cell(&mut w, 1, 70, 1, CropKind::Carrot);
    w.set_block(1, 70, 1, block::BLIGHTED_CROP);
    assert_eq!(w.meta_at(1, 70, 1), CropKind::Carrot.index());
}

#[test]
fn a_broadcast_replacing_a_blighted_crop_leaves_the_receiver_with_the_sent_meta() {
    // A joiner (or a host's loopback) holds the kind byte too. A replacement
    // carrying the very same byte value still lands as sent.
    let mut w = crate::world::World::new();
    crate::blight::blight_cell(&mut w, 2, 70, 2, CropKind::Wheat);
    let sent = protocol::BlockChange::with_meta(2, 70, 2, block::WHEAT_STAGE_0, CropKind::Wheat.index());
    assert!(w.apply_remote_block_change(&sent));
    assert_eq!(w.meta_at(2, 70, 2), CropKind::Wheat.index(), "as sent");
    let mut w = crate::world::World::new();
    crate::blight::blight_cell(&mut w, 2, 70, 2, CropKind::Wheat);
    let broke = protocol::BlockChange::with_meta(2, 70, 2, block::AIR, 0);
    assert!(w.apply_remote_block_change(&broke));
    assert_eq!(w.meta_at(2, 70, 2), 0);
}
