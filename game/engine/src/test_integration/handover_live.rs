//! World handover, live source integration: the control link is bound to an
//! exact authenticated seat and the source freeze stops every tick-driven
//! path while control frames still flow; a pre-retirement release resumes the
//! same seat's waiting gameplay through the ordinary path.
use super::joiner_authority::{accepted, cell_beside, send_edits};
use super::lent_world::start_lent;
use crate::block;
use crate::hosted_server::HostedServer;
use crate::protocol;
use crate::sim_lend::OwnedSimParts;
use crate::transport::{ChannelClientTransport, ClientTransport};

pub(super) const BINDING: [u8; 32] = [17; 32];

fn now_secs() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32
}

/// A signed join over a bound channel through one lent tick; returns the
/// client, its seat and the seat's (generation, verified key).
pub(super) fn signed_join_lent(
    hs: &mut HostedServer,
    host: &mut OwnedSimParts,
    seckey: [u8; 32],
) -> (ChannelClientTransport, usize, u32, [u8; 32]) {
    let client = hs.attach_test_remote_with_binding(Some(BINDING));
    let chal = loop {
        let pkt = client.try_recv_from_server().expect("challenge issued on attach");
        let (ptype, payload) = protocol::deserialize_header(&pkt).unwrap();
        if ptype == protocol::PacketType::Challenge {
            break protocol::safe_deserialize::<protocol::ChallengePacket>(payload).unwrap();
        }
    };
    let origin = crate::signet::join_origin(Some(BINDING));
    let ev = crate::hosted_server::tests::signed_auth_event(seckey, &chal.nonce_hex, &origin, now_secs());
    let req = crate::hosted_server::tests::join_with_auth(ev, None);
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::JoinRequest, &req));
    host.lend_tick(hs);
    let slot = accepted(&client).expect("signed join accepted").player_index as usize;
    super::joiner_authority::settle_chunk_push(hs, &client, |hs| host.lend_tick(hs));
    let (generation, key) = hs.handover_seat(slot).expect("authenticated live seat");
    (client, slot, generation, key)
}

fn handover_frame(bytes: &[u8]) -> Vec<u8> {
    let mut p = vec![protocol::PacketType::Handover as u8];
    p.extend_from_slice(bytes);
    p
}

#[test]
fn handover_link_binds_only_the_exact_authenticated_seat() {
    let (mut hs, mut host) = start_lent("handover-bind");
    let (client, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x51; 32]);
    assert_eq!(hs.handover_bind(slot, generation.wrapping_add(1), key), Err("that player is no longer signed in to this world"));
    assert!(hs.handover_bind(slot, generation, [9; 32]).is_err(), "another key");
    assert!(hs.handover_bind(0, generation, key).is_err(), "the host's own seat");
    hs.handover_bind(slot, generation, key).unwrap();
    assert!(hs.handover_bind(slot, generation, key).is_err(), "one link at a time");
    // A frame from the bound seat reaches the link through ordinary processing.
    client.send_to_server(&handover_frame(b"hello"));
    host.lend_tick(&mut hs);
    assert_eq!(hs.handover_take_frames().unwrap(), vec![b"hello".to_vec()]);
    // The seat leaves: the link breaks and cannot freeze the source.
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Disconnect, &()));
    host.lend_tick(&mut hs);
    assert!(hs.handover_check().is_err());
    assert!(hs.handover_freeze().is_err());
    assert!(!hs.handover_frozen());
}

#[test]
fn handover_frozen_source_runs_no_tick_admits_nobody_and_resumes_after_release() {
    let (mut hs, mut host) = start_lent("handover-freeze");
    let (client, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x52; 32]);
    let (other, other_slot, _, _) = signed_join_lent(&mut hs, &mut host, [0x53; 32]);
    let accept = hs.test_accept_channel();
    hs.handover_bind(slot, generation, key).unwrap();
    hs.handover_freeze().unwrap();
    let cell = cell_beside(&hs, slot, 1, 0, 0);
    let other_cell = cell_beside(&hs, other_slot, 0, 0, 1);
    let before = host.world.get_block(cell.0, cell.1, cell.2);
    let other_before = host.world.get_block(other_cell.0, other_cell.1, other_cell.2);
    assert_ne!(before, block::GLASS);
    // Both seats act; a newcomer's connection arrives; control frames flow.
    send_edits(&hs, &client, slot, 1, &[(cell, block::GLASS)]);
    send_edits(&hs, &other, other_slot, 1, &[(other_cell, block::GLASS)]);
    client.send_to_server(&handover_frame(b"accept"));
    let (server_side, newcomer) = crate::transport::channel_pair();
    accept.send(Box::new(server_side)).unwrap();
    let tally = host.world.sim_tally;
    for _ in 0..5 {
        host.lend_tick(&mut hs); // what an ungated caller would do: still nothing
    }
    assert_eq!(hs.handover_take_frames().unwrap(), vec![b"accept".to_vec()]);
    assert!(host.world.sim_tally == tally, "a frozen source simulated");
    assert_eq!(host.world.get_block(cell.0, cell.1, cell.2), before, "frozen edit applied");
    assert_eq!(host.world.get_block(other_cell.0, other_cell.1, other_cell.2), other_before);
    assert!(newcomer.try_recv_from_server().is_none(), "admission ran while frozen");
    // Pre-retirement release: same generations, so the waiting edits are
    // judged by the ordinary path, once, and the newcomer is met.
    hs.handover_release();
    assert!(!hs.handover_frozen());
    host.lend_tick(&mut hs);
    assert!(host.world.sim_tally != tally, "a released source must simulate again");
    assert_eq!(host.world.get_block(cell.0, cell.1, cell.2), block::GLASS);
    assert_eq!(host.world.get_block(other_cell.0, other_cell.1, other_cell.2), block::GLASS);
    let mut challenged = false;
    while let Some(p) = newcomer.try_recv_from_server() {
        challenged |= matches!(protocol::deserialize_header(&p), Some((protocol::PacketType::Challenge, _)));
    }
    assert!(challenged, "the newcomer was not admitted after release");
}

#[test]
fn handover_eligibility_refuses_live_seat_state_it_cannot_carry() {
    use crate::handover::live::eligibility::{Refusal, Seat, seats};
    let (mut hs, mut host) = start_lent("handover-elig");
    let (_c, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x61; 32]);
    let seat = Seat { slot, generation, key };
    assert_eq!(seats(&hs.server, seat), Ok(()), "a fresh tracked seat with no credits");
    assert!(seats(&hs.server, Seat { generation: generation.wrapping_add(1), ..seat }).is_err());
    assert!(seats(&hs.server, Seat { key: [3; 32], ..seat }).is_err());
    // One server credit: in flight first, then delivered — both refuse.
    hs.server.queue_kill_event(slot, generation, crate::mob::MobType::Cow, glam::Vec3::ZERO, false, false);
    assert!(seats(&hs.server, seat).is_err(), "credit still in flight");
    host.lend_tick(&mut hs);
    assert_eq!(
        seats(&hs.server, seat),
        Err(Refusal("That player has village or hunting progress this session, which this version can't move yet."))
    );
    // An unreadable record of a connected seat refuses (kept untouched).
    let (mut hs, mut host) = start_lent("handover-elig-read");
    let (_c, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x62; 32]);
    hs.server.failed_joiner_reads.insert(key);
    assert!(seats(&hs.server, Seat { slot, generation, key }).is_err());
    // Another signed-in player present: their agreement can't be asked yet.
    let (mut hs, mut host) = start_lent("handover-elig-other");
    let (_a, slot, generation, key) = signed_join_lent(&mut hs, &mut host, [0x63; 32]);
    let (_b, _, _, _) = signed_join_lent(&mut hs, &mut host, [0x64; 32]);
    assert!(seats(&hs.server, Seat { slot, generation, key }).is_err());
}
