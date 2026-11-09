//! Living world Tier 2 S10 — a joiner is told how many built where and how
//! lived-in each region is (`region_people`; Spec 04 v100, Tier 2 spec §10).
//!
//! These drive REAL joins through `HostedServer::tick` (the push-joiner
//! harness: a `RemoteClient` over the channel transport) and read what the
//! client ended up holding. The pure parts (wire form, quantisation, the
//! joiner's map) are tested in `region_people`; the joiner's bloom against the
//! host's in `bloom/tests.rs`.

use glam::Vec3;

use super::push_joiner::{start_host, Joiner};
use crate::authorship::AuthorKey;
use crate::block;
use crate::bloom::{BloomField, TickInput};
use crate::hosted_server::HostedServer;
use crate::region_people::{quantise_u, PEOPLE_EVERY_TICKS};

fn author(name: &str) -> AuthorKey {
    AuthorKey::Npub(name.to_string())
}

/// The column the host's body stands in.
fn host_column(hs: &HostedServer) -> (i32, i32) {
    crate::chunk_stream::column_of(hs.server.players[0].player.pos)
}

/// `n` blocks in a row in column `col`, each by `who` (cycled), up in the sky.
fn build(world: &mut crate::world::World, col: (i32, i32), start: i32, who: &[&str]) {
    for (i, name) in who.iter().enumerate() {
        world.place_player_block_as(col.0 * 16 + 2 + (start + i as i32) % 12, 100 + (start + i as i32) / 12, col.1 * 16 + 2, block::STONE, Some(&author(name)));
    }
}

/// Play `ticks` frames: input, tick, take in.
fn play(j: &mut Joiner, hs: &mut HostedServer, ticks: usize) {
    for _ in 0..ticks {
        j.ack();
        hs.tick();
        j.take_in();
    }
}

fn told(j: &Joiner, col: (i32, i32)) -> (u8, u8) {
    j.rc.region_people().get(col.0, col.1)
}

/// A joiner's own bloom field over the world it was pushed, reading what it was told.
fn joiner_p(j: &Joiner, col: (i32, i32), use_told: bool) -> f32 {
    let mut field = BloomField::new();
    let here = [Vec3::new(col.0 as f32 * 16.0 + 8.0, 90.0, col.1 as f32 * 16.0 + 8.0)];
    let ecs = hecs::World::new();
    let biome = j.biome.as_ref().expect("the join gave the seed");
    for _ in 0..600 {
        field.tick(&TickInput {
            world: &j.world,
            ecs: &ecs,
            biome_gen: biome,
            registry: &j.registry,
            local_players: &here,
            remote_players: &[],
            creative: false,
            dt: 0.05,
            allow_grey_event: false,
            told_people: use_told.then(|| j.rc.region_people()),
        });
        if let Some((_, p, _, _)) = field.region_terms(col) {
            return p;
        }
    }
    panic!("region {col:?} was never scanned");
}

#[test]
fn two_builders_in_a_region_reach_a_joiner_as_a_count_and_lift_its_bloom() {
    let mut hs = start_host("people-b");
    let col = host_column(&hs);
    build(&mut hs.server.world, col, 0, &["alice", "bob", "alice"]);
    // A neighbouring column with a single builder: one is not "together".
    build(&mut hs.server.world, (col.0 + 1, col.1), 0, &["alice", "alice"]);
    let mut j = Joiner::join(&mut hs, 3, false);
    play(&mut j, &mut hs, PEOPLE_EVERY_TICKS as usize + 60);

    assert_eq!(told(&j, col).0, 2, "two distinct builders");
    assert_eq!(told(&j, (col.0 + 1, col.1)), (1, 0), "one builder is told as one; its b is still 0");
    assert_eq!(told(&j, (col.0 - 1, col.1)), (0, 0));

    let with = joiner_p(&j, col, true);
    let without = joiner_p(&j, col, false);
    assert_eq!(without, 0.0, "a joiner told nothing reads 0");
    assert!(
        (with - crate::authorship::built_together(2)).abs() < 1e-4,
        "its b is the host's formula: {with}"
    );
}

#[test]
fn values_are_sent_only_when_they_change_and_never_twice_in_ten_seconds() {
    let mut hs = start_host("people-cadence");
    let col = host_column(&hs);
    build(&mut hs.server.world, col, 0, &["alice", "bob"]);
    let mut j = Joiner::join(&mut hs, 2, false);

    // The first packet.
    let mut first = None;
    for t in 0..(PEOPLE_EVERY_TICKS as usize + 40) {
        play(&mut j, &mut hs, 1);
        if j.rc.region_people().packets_taken() == 1 {
            first = Some(t);
            break;
        }
    }
    let first = first.expect("a packet came");
    assert_eq!(told(&j, col).0, 2);

    // A change right after it waits for the beat.
    build(&mut hs.server.world, col, 2, &["carol"]);
    play(&mut j, &mut hs, PEOPLE_EVERY_TICKS as usize - 10);
    assert_eq!(j.rc.region_people().packets_taken(), 1, "ten seconds have not passed");
    assert_eq!(told(&j, col).0, 2);
    play(&mut j, &mut hs, 30);
    assert_eq!(j.rc.region_people().packets_taken(), 2, "the next beat carries it");
    assert_eq!(told(&j, col).0, 3);
    let _ = first;

    // Nothing changes: nothing is resent, over several beats.
    play(&mut j, &mut hs, PEOPLE_EVERY_TICKS as usize * 3);
    assert_eq!(j.rc.region_people().packets_taken(), 2, "unchanged values are not resent");

    // A region back at zero is told once.
    for i in 0..3 {
        hs.server.world.set_block(col.0 * 16 + 2 + i, 100, col.1 * 16 + 2, block::AIR);
        hs.server.world.set_placed(col.0 * 16 + 2 + i, 100, col.1 * 16 + 2, false);
    }
    play(&mut j, &mut hs, PEOPLE_EVERY_TICKS as usize + 5);
    assert_eq!(j.rc.region_people().packets_taken(), 3);
    assert_eq!(told(&j, col), (0, 0));
    assert_eq!(hs.people_sent_for_test(j.slot).told_len(), 0, "and then forgotten");
}

#[test]
fn a_region_outside_the_sent_set_is_never_mentioned() {
    let mut hs = start_host("people-sentset");
    let col = host_column(&hs);
    let far = (col.0 + 25, col.1);
    build(&mut hs.server.world, far, 0, &["alice", "bob", "carol"]);
    build(&mut hs.server.world, col, 0, &["alice", "bob"]);
    let mut j = Joiner::join(&mut hs, 2, false);
    play(&mut j, &mut hs, PEOPLE_EVERY_TICKS as usize * 2 + 20);
    assert_eq!(told(&j, col).0, 2, "a region it holds is told");
    assert_eq!(told(&j, far), (0, 0), "a region it was never pushed is not");
    assert!(!hs.chunk_push_for_test(j.slot).has_sent_column(far.0, far.1));
    assert_eq!(hs.people_sent_for_test(j.slot).told_len(), 1);
}

#[test]
fn the_lived_in_term_arrives_once_there_is_a_use_record_and_sixteen_placed_blocks() {
    let mut hs = start_host("people-u");
    let col = host_column(&hs);
    for day in 0..10 {
        hs.server.world.use_record.note_present_at(100 + day, col.0 * 16 + 8, col.1 * 16 + 8);
    }
    // Fifteen blocks by two builders: below the gate, u is 0.
    let names: Vec<&str> = (0..15).map(|i| if i % 2 == 0 { "alice" } else { "bob" }).collect();
    build(&mut hs.server.world, col, 0, &names);
    assert_eq!(hs.server.world.lived_in_term(col.0, col.1), 0.0);
    let mut j = Joiner::join(&mut hs, 2, false);
    play(&mut j, &mut hs, PEOPLE_EVERY_TICKS as usize + 40);
    assert_eq!(told(&j, col), (2, 0), "builders yes, u not yet");

    // The sixteenth block: u follows on the next beat, with the host's own number.
    build(&mut hs.server.world, col, 15, &["bob"]);
    let u = hs.server.world.lived_in_term(col.0, col.1);
    assert!(u > 0.2, "{u}");
    play(&mut j, &mut hs, PEOPLE_EVERY_TICKS as usize + 5);
    assert_eq!(told(&j, col), (2, quantise_u(u)));
    assert!(joiner_p(&j, col, true) > crate::authorship::built_together(2), "the lived-in term lifts it too");
}

#[test]
fn a_released_slot_forgets_what_it_was_told_and_a_new_joiner_is_told_afresh() {
    let mut hs = start_host("people-reuse");
    let col = host_column(&hs);
    build(&mut hs.server.world, col, 0, &["alice", "bob"]);
    let mut j = Joiner::join(&mut hs, 2, false);
    play(&mut j, &mut hs, PEOPLE_EVERY_TICKS as usize + 40);
    let slot = j.slot;
    assert_eq!(hs.people_sent_for_test(slot).told_len(), 1);
    j.rc.disconnect();
    hs.tick();
    assert!(hs.slot_is_free(slot));
    assert_eq!(hs.people_sent_for_test(slot).told_len(), 0, "released with the slot");

    let mut next = Joiner::join(&mut hs, 2, false);
    assert_eq!(next.slot, slot);
    play(&mut next, &mut hs, PEOPLE_EVERY_TICKS as usize + 40);
    assert_eq!(told(&next, col).0, 2, "told again from the start");
}

#[test]
fn a_lending_host_tells_its_joiner_from_the_hosts_own_tables() {
    let (mut hs, mut host) = super::lent_world::start_lent("people");
    let col = crate::chunk_stream::column_of(hs.server.players[0].player.pos);
    // The host's world is where who built what is kept.
    build(&mut host.world, col, 0, &["alice", "bob", "carol"]);
    let mut j = Joiner::join_with(&mut hs, 2, false, |hs| host.lend_tick(hs));
    for _ in 0..(PEOPLE_EVERY_TICKS as usize + 60) {
        j.ack();
        host.lend_tick(&mut hs);
        j.take_in();
    }
    assert_eq!(told(&j, col).0, 3);
    assert_eq!(j.rc.region_people().packets_taken(), 1, "once, not once per system that touches the joiner");
}

#[test]
fn nothing_that_names_anyone_is_in_the_packet_a_joiner_receives() {
    // The packet's bytes for a region with two named builders hold the place and
    // two counts, and no trace of either name.
    let mut hs = start_host("people-anon");
    let col = host_column(&hs);
    build(&mut hs.server.world, col, 0, &["npub1alicealicealice", "npub1bobbobbobbob"]);
    let mut sent = crate::region_people::PeopleSent::default();
    let packet = sent.plan(0, &hs.server.world, [(col.0, col.1, 1)]).expect("something to say");
    let bytes = crate::protocol::serialize_packet(crate::protocol::PacketType::RegionPeople, &packet);
    for needle in [&b"alice"[..], b"bob", b"npub"] {
        assert!(!bytes.windows(needle.len()).any(|w| w == needle));
    }
    assert!(packet.regions.iter().all(|r| r.builders <= 8));
    assert_eq!(bytes.len(), 1 + 8 + 10 * packet.regions.len());
}

#[test]
fn a_column_dropped_and_pushed_again_within_one_beat_is_told_its_numbers_again() {
    // The ride case: the client unloads a column round the server's body and,
    // before the next ten-second beat, the server pushes it again. The joiner
    // forgot the column's numbers with it; the authority must not take them to
    // still stand just because the pair is unchanged.
    use super::push_joiner::move_body;
    let mut hs = start_host("people-repush");
    let mut j = Joiner::join(&mut hs, 2, false);
    j.settle(&mut hs);
    let me = j.column(&hs);
    let col = (me.0 + 1, me.1);
    build(&mut hs.server.world, col, 0, &["alice", "bob"]);
    // Play to the beat that tells it (the server's beat counter is its own).
    for _ in 0..(PEOPLE_EVERY_TICKS as usize + 40) {
        play(&mut j, &mut hs, 1);
        if told(&j, col).0 == 2 {
            break;
        }
    }
    assert_eq!(told(&j, col).0, 2);
    let packets = j.rc.region_people().packets_taken();

    // Let the column go (as `drop_column_meshes` does: the world and the numbers),
    // then walk the body off and back so the server pushes it again, all inside
    // one beat.
    j.let_go(col);
    j.rc.region_people_mut().forget(col.0, col.1);
    assert_eq!(told(&j, col), (0, 0));
    let mut gap = 0;
    move_body(&mut hs, j.slot, -5);
    gap += j.settle(&mut hs);
    move_body(&mut hs, j.slot, 5);
    gap += j.settle(&mut hs);
    assert!(j.intake.column_complete(col), "pushed again");
    assert!(gap < PEOPLE_EVERY_TICKS as usize - 20, "the gap {gap} fits inside one beat, which is the case under test");

    // The values are unchanged, yet the joiner is told them again.
    play(&mut j, &mut hs, PEOPLE_EVERY_TICKS as usize + 20);
    assert_eq!(told(&j, col).0, 2, "told again after the re-push");
    assert!(j.rc.region_people().packets_taken() > packets);
}
