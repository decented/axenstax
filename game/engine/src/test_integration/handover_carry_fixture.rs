//! Genuine common quest setup before the first active-carry boundary.
use crate::{
    hosted_server::{HostWorld, HostedServer, RemoteTransport},
    progression::{GameplayBinding, GameplayRecord},
    protocol::{self, PacketType},
    quest_wire::{QuestOperation, QuestView},
    test_game_harness::HeadlessGame,
    transport::{ChannelClientTransport, ClientTransport},
};
use glam::Vec3;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(super) type Seat = (usize, u32, [u8; 32]);
pub(super) fn key(secret: [u8; 32]) -> [u8; 32] {
    nostr::Keys::new(nostr::SecretKey::from_slice(&secret).unwrap()).public_key().to_bytes()
}
pub(super) fn native(game: &HeadlessGame) -> GameplayRecord {
    let slot = &game.state.players[0];
    GameplayRecord::from_slot(slot.personal_binding.as_ref().unwrap().context(), slot).unwrap()
}
#[path = "handover_carry_fixture/native.rs"]
mod native_setup;
#[path = "handover_carry_fixture/observe.rs"]
mod observe;
use observe::Observation;
#[path = "handover_carry_fixture/continuation.rs"]
pub(in crate::test_integration) mod continuation;
#[path = "handover_carry_fixture/custody.rs"]
pub(in crate::test_integration) mod custody;
pub(super) fn active_source(name: &str, secret: [u8; 32]) -> HeadlessGame {
    native_setup::active_source(name, secret)
}

pub(super) fn step(source: &mut HeadlessGame, peer: &mut HeadlessGame) {
    peer.ticks(1);
    peer.state.network_send_input();
    source.hosted_ticks(1);
    source.frames(1);
    peer.frames(1);
}
pub(super) fn settle(source: &mut HeadlessGame, peer: &mut HeadlessGame) -> Seat {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        step(source, peer);
        let hs = source.state.hosted_server.as_ref().unwrap();
        if let Some((i, sp)) = hs.server.players.iter().enumerate().find(|(_, p)| p.server_simulated && p.is_in_world())
            && let Some((generation, key)) = hs.handover_seat(i)
            && peer
                .state
                .remote_client
                .as_ref()
                .is_some_and(|r| r.is_connected() && r.personal_ready() && r.handover_authority_settled())
            && sp.window_events.is_quiescent()
            && sp.personal_delivery.quiescent(sp.personal.as_ref().unwrap(), sp.respawns)
            && sp.action_results.as_ref().is_none_or(|r| r.is_quiescent())
            && peer.state.window_inbox.is_empty()
        {
            assert_eq!(source.state.tick_counter, hs.server.tick_counter);
            assert!(source.state.gameplay_preparation.is_none());
            return (i, generation, key);
        }
        assert!(Instant::now() < deadline, "the real joined current action/window/Personal boundary did not settle");
    }
}
pub(super) fn attach(
    source: &mut HeadlessGame,
    peer: &mut HeadlessGame,
    client: ChannelClientTransport,
    secret: [u8; 32],
    source_persona: [u8; 32],
    source_runtime: [u8; 32],
) -> (Seat, Observation) {
    let (client, observation) = observe::wrap(client);
    let bytes = client.try_recv_from_server().expect("actual bound attach challenge");
    let (kind, body) = protocol::deserialize_header(&bytes).unwrap();
    assert_eq!(kind, PacketType::Challenge);
    let challenge: protocol::ChallengePacket = protocol::safe_deserialize(body).unwrap();
    let mut request = crate::remote_client::build_join_request_guest("Carry participant", 0);
    request.auth_event = Some(
        pollster::block_on(crate::signet::native_signer::build_join_auth_event(
            &nostr::Keys::new(nostr::SecretKey::from_slice(&secret).unwrap()),
            &challenge.nonce_hex,
            &crate::signet::join_origin(Some(super::handover_live::BINDING)),
        ))
        .unwrap(),
    );
    peer.state.remote_client =
        Some(crate::remote_client::RemoteClient::from_transport(Box::new(client), request, None));
    peer.state.set_joined_host(source_persona, source_runtime);
    assert!(matches!(peer.state.mode, crate::GameMode::Playing));
    let seat = settle(source, peer);
    assert!(seat.2 == key(secret), "actual signed participant has the wrong npub");
    (seat, observation)
}
pub(super) fn start_lent(source: &mut HeadlessGame) {
    let hs = HostedServer::start_host(
        1,
        source.state.world_name.clone(),
        source.state.biome_gen.seed,
        0,
        RemoteTransport::WebSocket { port: 0 },
        HostWorld::Lent,
    )
    .unwrap();
    source.state.hosted_server = Some(hs);
}

pub(super) fn quest(
    source: &mut HeadlessGame,
    peer: &mut HeadlessGame,
    observation: &Observation,
    target: u32,
    incarnation: u64,
    op: QuestOperation,
) -> Option<QuestView> {
    observe::quest(source, peer, observation, target, incarnation, op)
}
pub(super) fn giver(
    source: &mut HeadlessGame,
    peer: &mut HeadlessGame,
    profession: crate::villager::Profession,
) -> u32 {
    let seat = settle(source, peer);
    let sp = &source.state.hosted_server.as_ref().unwrap().server.players[seat.0];
    let at = sp.player.pos + crate::camera::forward_from(sp.yaw, 0.0) * 1.5;
    source.state.world.village_anchors.insert((2, 3), at.to_array().map(|n| n.floor() as i32));
    let e = crate::entity::spawn_mob(&mut source.state.ecs, crate::mob::MobType::Villager, at);
    source.state.ecs.get::<&mut crate::villager::VillagerComponent>(e).unwrap().profession = profession;
    step(source, peer);
    source.state.ecs.get::<&crate::entity::ProtocolId>(e).unwrap().0
}

#[derive(Debug, PartialEq)]
pub(super) struct Boundary {
    files: BTreeMap<PathBuf, Vec<u8>>,
    source_record: Vec<u8>,
    source_binding: GameplayBinding,
    source_clock: crate::quest_state::GameplayClock,
    remote_record: Vec<u8>,
    remote_clock: crate::quest_state::GameplayClock,
    remote_binding: GameplayBinding,
    remote_allocation: (usize, usize, usize),
    remote_delivery: Vec<u8>,
    peer_progression: Vec<u8>,
    peer_binding: Option<GameplayBinding>,
    peer_clock: crate::quest_state::GameplayClock,
    peer_frontier: (Option<crate::action_identity::ActionHeader>, u32, u32, bool, bool, usize, usize, bool),
    body_places: ([usize; 4], [usize; 4]),
    source_body: Vec<u8>,
    remote_body: Vec<u8>,
    source_ptrs: Vec<usize>,
    ticks: (u64, u64, u64, u32),
    peer_ticks: (u64, u64, u32),
    scope: Option<[u8; 16]>,
    givers: Vec<[u8; 16]>,
    held: bool,
    fence: bool,
}
fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in std::fs::read_dir(path).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(p.strip_prefix(root).unwrap().into(), std::fs::read(p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}
pub(super) fn boundary(source: &HeadlessGame, peer: &HeadlessGame, seat: Seat) -> Boundary {
    let sp = &source.state.hosted_server.as_ref().unwrap().server.players[seat.0];
    let slot = &source.state.players[0];
    let mut givers: Vec<_> =
        source.state.ecs.query::<&crate::quest_npcs::QuestNpcId>().iter().map(|(_, id)| id.0).collect();
    givers.sort();
    let window = |slot: &crate::player_slot::PlayerSlot| {
        bincode::serialize(&(
            slot.inventory.slots_iter().map(|s| s.cloned()).collect::<Vec<_>>(),
            slot.armour_slots,
            &slot.crafting_ui.cursor_item,
            &slot.crafting_ui.grid,
        ))
        .unwrap()
    };
    let personal = sp.personal.as_ref().unwrap();
    let remote = peer.state.remote_client.as_ref().unwrap();
    let inbox = remote.personal_inbox.as_ref().unwrap();
    let physical = |s: &crate::player_slot::PlayerSlot| {
        [
            &s.inventory as *const _ as usize,
            &s.armour_slots as *const _ as usize,
            &s.crafting_ui.cursor_item as *const _ as usize,
            &s.crafting_ui.grid as *const _ as usize,
        ]
    };
    // This natural-activity baseline contains no private Plan fixture. These
    // are actual physical place addresses, not claimed private-body pointers.
    for s in [slot, &peer.state.players[0]] {
        assert!(
            !s.inventory
                .slots_iter()
                .flatten()
                .chain(s.crafting_ui.cursor_item.iter())
                .chain(s.crafting_ui.grid.iter().flatten().flatten())
                .any(|stack| matches!(&stack.item, crate::item::Item::Plan(_)))
        );
    }
    Boundary {
        files: files(&crate::save::world_dir(&source.state.world_name)),
        source_record: native(source).encode().unwrap(),
        source_binding: slot.personal_binding.clone().unwrap(),
        source_clock: slot.gameplay_clock,
        remote_record: sp.personal.as_ref().unwrap().bytes.clone(),
        remote_clock: sp.gameplay_clock,
        remote_binding: personal.binding.clone(),
        remote_allocation: (personal.bytes.as_ptr() as usize, personal.bytes.len(), personal.bytes.capacity()),
        remote_delivery: bincode::serialize(&(
            sp.personal_delivery.pending.clone(),
            sp.personal_delivery.last,
            sp.personal_delivery.acknowledged_window,
            sp.personal_delivery.revision,
            sp.personal_delivery.last_bytes,
            sp.personal_delivery.initialised,
            sp.personal_delivery.started_tick,
            sp.personal_delivery.sent_tick,
            sp.window_events.last_number(),
            sp.window_events.is_quiescent(),
            sp.progression_credits,
            sp.personal_read_failed,
        ))
        .unwrap(),
        // The context is the actual server binding; this is a read of the
        // peer's live maps, not a claim about private Inbox context fields.
        peer_progression: GameplayRecord::from_slot(personal.binding.context(), &peer.state.players[0])
            .unwrap()
            .encode()
            .unwrap(),
        peer_binding: peer.state.players[0].personal_binding.clone(),
        peer_clock: peer.state.players[0].gameplay_clock,
        peer_frontier: (
            remote.action_journal.as_ref().map(|j| j.next_header(remote.applied_window_event()).unwrap()),
            remote.applied_window_event(),
            remote.applied_actor_respawns(),
            remote.personal_ready(),
            remote.handover_authority_settled(),
            inbox.pending_count(),
            inbox.owned_bytes(),
            inbox.initialised(),
        ),
        body_places: (physical(slot), physical(&peer.state.players[0])),
        source_body: window(slot),
        remote_body: window(&peer.state.players[0]),
        source_ptrs: slot
            .inventory
            .slots_iter()
            .flatten()
            .map(|s| s as *const crate::item::ItemStack as usize)
            .collect(),
        ticks: (
            source.state.tick_counter,
            source.state.hosted_server.as_ref().unwrap().server.tick_counter,
            source.state.world.total_ticks,
            source.state.world_time,
        ),
        peer_ticks: (peer.state.tick_counter, peer.state.world.total_ticks, peer.state.world_time),
        scope: source.state.world.quest_scope,
        givers,
        held: source.state.handover_holds_play(),
        fence: crate::save_fence::is_up(&crate::save::world_dir(&source.state.world_name)),
    }
}

fn remote_record(source: &HeadlessGame, seat: Seat) -> GameplayRecord {
    let host = &source.state.hosted_server.as_ref().unwrap().server;
    let sp = &host.players[seat.0];
    sp.gameplay_clock.project(&sp.personal.as_ref().unwrap().record().unwrap(), host.tick_counter).0
}
fn remote_kill(source: &mut HeadlessGame, peer: &mut HeadlessGame, kind: crate::mob::MobType) {
    let seat = settle(source, peer);
    while source.state.tick_counter
        < source.state.hosted_server.as_ref().unwrap().server.players[seat.0].next_swing_tick
    {
        step(source, peer);
    }
    let sp = &source.state.hosted_server.as_ref().unwrap().server.players[seat.0];
    let at = sp.player.pos + crate::camera::forward_from(sp.yaw, 0.0) * 2.0;
    let entity = crate::entity::spawn_mob(&mut source.state.ecs, kind, at);
    step(source, peer);
    let target = source.state.ecs.get::<&crate::entity::ProtocolId>(entity).unwrap().0;
    source.state.ecs.get::<&mut crate::entity::Position>(entity).unwrap().0 = at;
    source.state.ecs.get::<&mut crate::combat::Health>(entity).unwrap().current = 1.0;
    peer.state.send_entity_attack(
        0,
        crate::remote_mobs::MirrorTarget {
            id: target,
            kind,
            tamed: false,
            pack: false,
            baby: false,
            tethered: false,
            product_not_ready: false,
        },
        false,
        false,
    );
    peer.ticks(1);
    peer.state.network_send_input();
    source.hosted_ticks(1);
    // Inspect the lethal result before any wall-clock frame can run the next
    // native sweep. The following complete tick still owns real attribution.
    assert!(source.state.ecs.get::<&crate::combat::Health>(entity).unwrap().is_dead());
    assert!(matches!(source.state.ecs.get::<&crate::combat::LastAttacker>(entity).unwrap().0,
        crate::combat::Attacker::Remote {slot,generation} if slot==seat.0 && generation==seat.1));
    source.frames(1);
    peer.frames(1);
    step(source, peer); // The real next native half owns this Lent death sweep.
    assert!(!source.state.ecs.contains(entity));
    settle(source, peer);
}
pub(super) fn active_recipient(source: &mut HeadlessGame, peer: &mut HeadlessGame, observation: &Observation) -> Seat {
    let target = giver(source, peer, crate::villager::Profession::Carpenter);
    let mut offered = quest(source, peer, observation, target, 0, QuestOperation::Talk).unwrap();
    for attempt in 0..12 {
        if matches!(offered.terms.as_ref().unwrap().flavour, crate::quest::QuestFlavour::Kill { .. }) {
            break;
        }
        quest(source, peer, observation, target, offered.incarnation, QuestOperation::Close);
        for _ in 0..=attempt % 3 {
            step(source, peer);
        }
        offered = quest(source, peer, observation, target, 0, QuestOperation::Talk).unwrap();
    }
    let terms = offered.terms.as_ref().unwrap().clone();
    let crate::quest::QuestFlavour::Kill { mob, .. } = terms.flavour else {
        panic!("actual natural Carpenter pool did not offer Kill")
    };
    assert!(!matches!(mob, crate::mob::MobType::Nostrich), "unsupported Vow is not a clean carried credit fixture");
    let seat = settle(source, peer);
    remote_kill(source, peer, mob);
    let before_accept = remote_record(source, seat).kills.iter().find(|(kind, _)| *kind == mob).unwrap().1;
    assert_eq!(before_accept, 1);
    quest(source, peer, observation, target, offered.incarnation, QuestOperation::Accept { id: terms.id });
    let accepted = remote_record(source, seat).quests.accepted.into_iter().find(|q| q.id == terms.id).unwrap();
    assert_eq!(accepted.baseline, before_accept);
    assert_eq!(accepted.npc, offered.npc);
    remote_kill(source, peer, mob);
    let record = remote_record(source, seat);
    assert_eq!(record.kills.iter().find(|(kind, _)| *kind == mob).unwrap().1, 2);
    assert_eq!(crate::quest_play::kill_progress(&record, &accepted).unwrap(), 1);
    quest(source, peer, observation, target, offered.incarnation, QuestOperation::Close);
    let second = giver(source, peer, crate::villager::Profession::Blacksmith);
    let declined = quest(source, peer, observation, second, 0, QuestOperation::Talk).unwrap();
    assert_ne!(declined.npc, offered.npc);
    quest(
        source,
        peer,
        observation,
        second,
        declined.incarnation,
        QuestOperation::Decline { id: declined.terms.unwrap().id },
    );
    quest(source, peer, observation, second, declined.incarnation, QuestOperation::Close);
    assert!(!peer.state.joined_quest_open());
    remote_kill(source, peer, crate::mob::MobType::Villager);
    let seat = settle(source, peer);
    let record = remote_record(source, seat);
    assert!(record.quests.penalty_left > 0 && record.quests.penalty_left < 600);
    assert!(record.quests.declines.iter().any(|(id, left)| *id == declined.npc && *left > 0 && *left < 6000));
    assert_eq!(peer.state.players[0].reputation.score((2, 3)), -25);
    assert_eq!(record.quests.accepted, [accepted]);
    assert_ne!(native(source).quests.accepted[0].id, record.quests.accepted[0].id);
    assert!(source.state.players[0].nostrich_vow.is_none());
    assert!(peer.state.players[0].nostrich_vow.is_none());
    seat
}
pub(super) fn assert_boundary_ready(source: &HeadlessGame, peer: &HeadlessGame, seat: Seat) {
    let record = native(source);
    assert_eq!(record.quests.accepted.len(), 1);
    assert!(record.quests.penalty_left > 0 && record.quests.penalty_left < 600);
    assert!(record.quests.declines.iter().any(|(_, left)| *left > 0 && *left < 6000));
    assert!(
        source.state.players[0].active_quests.is_empty() && source.state.players[0].villager_decline_until.is_empty()
    );
    let remote = remote_record(source, seat);
    assert_eq!(remote.quests.accepted.len(), 1);
    assert_eq!(peer.state.players[0].quest_progress.accepted, remote.quests.accepted);
    assert!(source.state.gameplay_preparation.is_none());
    assert!(source.state.players[0].dialogue_villager.is_none());
    assert!(!peer.state.joined_quest_open());
    assert!(!source.state.handover_holds_play());
    assert!(!crate::save_fence::is_up(&crate::save::world_dir(&source.state.world_name)));
    assert_eq!(source.state.tick_counter, source.state.hosted_server.as_ref().unwrap().server.tick_counter);
    eprintln!(
        "carry-boundary tick={} source_revision={} source_penalty={} source_declines={:?} recipient_revision={} recipient_penalty={} recipient_declines={:?} source_quest={:?} recipient_quest={:?}",
        source.state.tick_counter,
        record.revision,
        record.quests.penalty_left,
        record.quests.declines,
        remote.revision,
        remote.quests.penalty_left,
        remote.quests.declines,
        record.quests.accepted,
        remote.quests.accepted
    );
}
