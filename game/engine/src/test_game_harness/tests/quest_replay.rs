//! Actual GameState intake of repeated retained quest batches. The injected
//! transport delays only genuine result/receipt carriers; it invents no state,
//! window marker, personal holder or acknowledgement.
use super::*;
use crate::{
    action_identity::InventoryMode,
    quest_wire::QuestOperation,
    transport::{ClientTransport, Packet},
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Delivery {
    hold_results: bool,
    hold_receipts: bool,
    result: Option<Packet>,
    receipts: VecDeque<Packet>,
    actions: Vec<Packet>,
    unfit: Vec<crate::protocol::ItemActionPacket>,
    // Only non-secret result metadata is retained for the actual routing trace.
    results: Vec<(crate::action_identity::ActionFate, crate::quest_wire::QuestNote, u32, u32, u32)>,
    hold_rewards: bool,
    hold_last: bool,
    rewards: VecDeque<Packet>,
    personal: VecDeque<Packet>,
    views: Vec<u32>,
    personal_acks: Vec<(u32, u32)>,
}
struct DelayedTransport {
    inner: crate::transport::ChannelClientTransport,
    delivery: Arc<Mutex<Delivery>>,
}
impl ClientTransport for DelayedTransport {
    fn send_to_server(&self, bytes: &[u8]) {
        let mut delivery = self.delivery.lock().unwrap();
        match crate::protocol::deserialize_header(bytes) {
            Some((crate::protocol::PacketType::Action, _)) => delivery.actions.push(bytes.to_vec()),
            Some((crate::protocol::PacketType::ItemAction, body)) => {
                let item: crate::protocol::ItemActionPacket =
                    crate::protocol::safe_deserialize(body).unwrap();
                if matches!(item.action, crate::protocol::ItemAction::GrantUnfit { .. }) {
                    delivery.unfit.push(item);
                }
            }
            Some((crate::protocol::PacketType::PersonalSync, body)) => {
                if let crate::protocol::PersonalSyncPacket::Applied(ack) = crate::protocol::safe_deserialize(body).unwrap() {
                    delivery.personal_acks.push((ack.window_event, ack.life));
                }
            }
            _ => {}
        }
        self.inner.send_to_server(bytes);
    }
    fn try_recv_from_server(&self) -> Option<Packet> {
        let mut delivery = self.delivery.lock().unwrap();
        if !delivery.hold_results
            && let Some(result) = delivery.result.take()
        {
            return Some(result);
        }
        if !delivery.hold_receipts
            && let Some(receipt) = delivery.receipts.pop_front()
        {
            return Some(receipt);
        }
        if !delivery.hold_rewards && let Some(reward) = delivery.rewards.pop_front() {
            return Some(reward);
        }
        if !delivery.hold_last && let Some(personal) = delivery.personal.pop_front() {
            return Some(personal);
        }
        while let Some(packet) = self.inner.try_recv_from_server() {
            if let Some((crate::protocol::PacketType::QuestResult, body)) = crate::protocol::deserialize_header(&packet) {
                let result = crate::quest_wire::QuestResult::decode(body).unwrap();
                delivery.results.push((result.fate, result.note, result.window_event, result.included_events, result.action.events_applied));
            }
            if let Some((crate::protocol::PacketType::QuestView, body)) = crate::protocol::deserialize_header(&packet) {
                let view: crate::quest_wire::QuestViewPush = crate::protocol::safe_deserialize(body).unwrap();
                delivery.views.push(view.window_event);
            }
            if let Some((crate::protocol::PacketType::InventoryGrant, body)) = crate::protocol::deserialize_header(&packet)
                && delivery.hold_rewards
            {
                let grant: crate::protocol::InventoryGrantPacket = crate::protocol::safe_deserialize(body).unwrap();
                if delivery.results.last().is_some_and(|(_, _, cost, last, _)| grant.window_event > *cost && grant.window_event < *last) {
                    delivery.rewards.push_back(packet);
                    continue;
                }
            }
            if let Some((crate::protocol::PacketType::PersonalSync, body)) = crate::protocol::deserialize_header(&packet)
                && delivery.hold_last
            {
                let personal: crate::protocol::PersonalSyncPacket = crate::protocol::safe_deserialize(body).unwrap();
                if matches!(personal, crate::protocol::PersonalSyncPacket::Snapshot(snapshot) if delivery.results.last().is_some_and(|(_, _, _, last, _)| snapshot.window_event == *last)) {
                    delivery.personal.push_back(packet);
                    continue;
                }
            }
            match crate::protocol::deserialize_header(&packet) {
                Some((crate::protocol::PacketType::QuestResult, _)) if delivery.hold_results => {
                    if let Some(result) = &delivery.result {
                        assert_eq!(
                            result, &packet,
                            "repeat retained the exact cost/result bytes"
                        );
                    } else {
                        delivery.result = Some(packet);
                    }
                    continue;
                }
                Some((crate::protocol::PacketType::ActionReceipt, _)) if delivery.hold_receipts => {
                    delivery.receipts.push_back(packet);
                    continue;
                }
                _ => return Some(packet),
            }
        }
        None
    }
    fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
}

fn bread(inventory: &crate::inventory::Inventory) -> u32 {
    inventory
        .slots_iter()
        .flatten()
        .filter(|stack| stack.item == crate::item::Item::Material(crate::item::MaterialId::Bread))
        .map(|stack| u32::from(stack.count))
        .sum()
}

fn trace(stage: &str, host: &crate::hosted_server::HostedServer, hg: &HeadlessGame, i: usize, target: u32, delivery: &Arc<Mutex<Delivery>>) {
    let client = hg.state.remote_client.as_ref().unwrap();
    let slot = &hg.state.players[0];
    let sp = &host.server.players[i];
    let raw = crate::staged_window::MarkerWindow::try_capture_parts(&slot.inventory, &slot.armour_slots, &slot.crafting_ui.cursor_item, &slot.crafting_ui.grid, &hg.state.registry).unwrap();
    let authoritative = crate::window_events::pack_raw_marker(sp, &host.server.registry).unwrap();
    let effective = crate::window_events::effective_window(sp);
    let effective = crate::staged_window::MarkerWindow::try_capture_parts(&effective.inv, &effective.armour, &effective.cursor, &effective.grid, &host.server.registry).unwrap();
    let giver = host.server.ecs.query::<(&crate::entity::ProtocolId, &crate::entity::Position)>().iter().find_map(|(_, (id, at))| (id.0 == target).then_some(at.0));
    eprintln!("Quest replay {stage}: mode={:?} connected={} applied={} last_sent={} held={} physical_accepted={} server_accepted={} bag_places={} bread={} client_digest={} raw_digest={} effective_digest={} actor={:?} giver={giver:?} distance={:?} yaw={} result_metadata={:?} inbox_queues={:?} fifo_entries={}", sp.inventory_mode, client.is_connected(), client.applied_window_event(), sp.window_events.last_number(), client.pack_window_inbox_held(), slot.quest_progress.accepted.len(), sp.personal.as_ref().unwrap().record().unwrap().quests.accepted.len(), slot.inventory.slots_iter().flatten().count(), bread(&slot.inventory), raw.digest_with(slot.crafting_ui.station(), None), authoritative.digest_with(sp.station, None), effective.digest_with(sp.station, None), sp.player.pos, giver.map(|at| at.distance(sp.player.pos)), sp.yaw, delivery.lock().unwrap().results, hg.state.window_inbox.diagnostic_event_queues(), client.chunk_queue.len());
}

/// Wait for the genuine physical frontier, while leaving held receipts held.
/// No server tick is needed merely to drain already received world carriers.
fn wait_applied(hg: &mut HeadlessGame, event: u32) {
    for _ in 0..200 {
        if hg.state.remote_client.as_ref().is_some_and(|client| client.applied_window_event() >= event && !client.pack_window_inbox_held()) { return; }
        hg.frames(1);
        assert!(hg.state.remote_client.as_ref().is_some_and(|client| client.is_connected()), "actual replay application lost its connection");
    }
    assert!(hg.state.remote_client.as_ref().is_some_and(|client| client.applied_window_event() >= event && !client.pack_window_inbox_held()), "actual numbered quest result never reached the physical frontier");
}

fn setup(
    mode: InventoryMode,
    tag: &str,
    full: bool,
) -> (
    HeadlessGame,
    crate::hosted_server::HostedServer,
    usize,
    Arc<Mutex<Delivery>>,
    u32,
    u64,
    [u8; 16],
) {
    let mut hg = HeadlessGame::boot_into_world(&format!("harness-quest-{tag}"));
    hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
    let mut host = crate::hosted_server::HostedServer::start(
        0,
        format!("harness-quest-{tag}-{mode:?}-{}", std::process::id()),
        42,
        0,
        crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
    )
    .unwrap();
    host.select_inventory_mode(mode).unwrap();
    host.server.world.world_type = "normal".into();
    host.server.world.is_workshop = false;
    host.server.difficulty = crate::survival::Difficulty::Peaceful;
    let delivery = Arc::new(Mutex::new(Delivery::default()));
    let transport = host.attach_test_remote();
    hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
        Box::new(DelayedTransport {
            inner: transport,
            delivery: delivery.clone(),
        }),
        crate::remote_client::build_join_request_guest("Quest", 0),
        None,
    ));
    pack_wait(&mut host, &mut hg, |server, game| {
        game.state
            .remote_client
            .as_ref()
            .is_some_and(|remote| remote.personal_ready())
            && server.server.players.last().is_some_and(|sp| {
                sp.personal
                    .as_ref()
                    .is_some_and(|personal| sp.personal_delivery.quiescent(personal, sp.respawns))
            })
    });
    let i = host.server.players.len() - 1;
    clear_pad(&mut hg, Some(&mut host));
    let at = host.server.players[i].player.pos + glam::Vec3::new(0.0, 0.0, -2.0);
    host.server
        .world
        .village_anchors
        .insert((2, 3), at.to_array().map(|n| n.floor() as i32));
    let npc = crate::entity::spawn_mob(&mut host.server.ecs, crate::mob::MobType::Villager, at);
    host.server
        .ecs
        .get::<&mut crate::villager::VillagerComponent>(npc)
        .unwrap()
        .profession = crate::villager::Profession::Blacksmith;
    pack_wait(&mut host, &mut hg, |server, game| {
        server
            .server
            .ecs
            .get::<&crate::entity::ProtocolId>(npc)
            .is_ok_and(|id| game.state.remote_mobs.position_of(id.0).is_some())
    });
    let target = host
        .server
        .ecs
        .get::<&crate::entity::ProtocolId>(npc)
        .unwrap()
        .0;
    hg.state.request_quest(target, 0, QuestOperation::Talk);
    pack_wait(&mut host, &mut hg, |_, game| {
        !game.state.pack_window_frozen(0) && game.state.joined_quest_open()
    });
    // This explicit server-authored frozen item fixture covers a supported
    // non-Plan reward. Natural pools remain unchanged (currently no items).
    host.server.players[i]
        .quest_session
        .as_mut()
        .unwrap()
        .offer
        .as_mut()
        .unwrap()
        .quest
        .rewards = vec![
        crate::saved_item_slot::SavedItemSlot::capture_exact(Some(
            &crate::item::ItemStack::new_material(crate::item::MaterialId::Bread, 2),
        ))
        .unwrap(),
    ];
    hg.state.request_quest(target, 0, QuestOperation::Talk);
    pack_wait(&mut host, &mut hg, |_, game| {
        !game.state.pack_window_frozen(0)
    });
    let view = hg
        .state
        .remote_client
        .as_ref()
        .unwrap()
        .quest_view
        .clone()
        .unwrap();
    let terms = view.terms.as_ref().unwrap();
    assert_eq!(terms.rewards.as_slice()[0].count, 2);
    hg.state.request_quest(
        target,
        view.incarnation,
        QuestOperation::Accept { id: terms.id },
    );
    pack_wait(&mut host, &mut hg, |server, game| {
        !game.state.pack_window_frozen(0)
            && server.server.players[i]
                .personal
                .as_ref()
                .unwrap()
                .record()
                .unwrap()
                .quests
                .accepted
                .len()
                == 1
    });
    let crate::quest::QuestFlavour::Fetch { item, count } = terms.flavour else {
        panic!("Blacksmith uses the existing Fetch pool");
    };
    let mut kit = vec![crate::item::ItemStack::new_material(
        item,
        count + u8::from(full),
    )];
    if full {
        kit.extend((1..36).map(|_| crate::item::ItemStack::new_block(crate::block::STONE, 64)));
    }
    // Genuine server grants align actual holdings, rather than copying a raw bag.
    host.grant_to_joiner(i, kit);
    let kit_event = host.server.players[i].window_events.last_number();
    pack_wait(&mut host, &mut hg, |_, game| {
        game.state.remote_client.as_ref().is_some_and(|client| client.applied_window_event() >= kit_event)
            && game.state.players[0]
            .inventory
            .slots_iter()
            .flatten()
            .any(|stack| stack.item == crate::item::Item::Material(item) && stack.count >= count)
    });
    trace("kit physically applied", &host, &hg, i, target, &delivery);
    assert_eq!(bread(&hg.state.players[0].inventory), 0);
    (hg, host, i, delivery, target, view.incarnation, terms.id)
}

fn replay(after_application: bool, full: bool) {
    isolate_saves();
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let tag = format!("repeat-{after_application}-{full}");
        let (mut hg, mut host, i, delivery, target, incarnation, id) = setup(mode, &tag, full);
        {
            let mut d = delivery.lock().unwrap();
            d.hold_receipts = true;
            d.hold_results = !after_application;
            d.actions.clear();
            d.unfit.clear();
            d.results.clear();
        }
        trace("before TurnIn", &host, &hg, i, target, &delivery);
        hg.state
            .request_quest(target, incarnation, QuestOperation::TurnIn { id });
        hg.state.network_send_input();
        host.tick();
        hg.frames(1);
        pack_wait(&mut host, &mut hg, |_, _| !delivery.lock().unwrap().results.is_empty());
        trace("first batch observed", &host, &hg, i, target, &delivery);
        let original = delivery
            .lock()
            .unwrap()
            .actions
            .last()
            .cloned()
            .expect("real typed TurnIn was sent");
        assert!(
            host.server.players[i]
                .personal
                .as_ref()
                .unwrap()
                .record()
                .unwrap()
                .quests
                .accepted
                .is_empty()
        );
        assert!(
            hg.state.pack_window_frozen(0),
            "held real receipt retains the shared mutation guard"
        );
        if after_application {
            let last = delivery.lock().unwrap().results.last().expect("actual result metadata").3;
            wait_applied(&mut hg, last);
            trace("first batch physically applied", &host, &hg, i, target, &delivery);
            assert!(
                hg.state.players[0].quest_progress.accepted.is_empty(),
                "first actual PersonalSync installed before the replay"
            );
            assert_eq!(
                bread(&hg.state.players[0].inventory),
                if full { 0 } else { 2 }
            );
        } else {
            assert!(
                hg.state
                    .remote_client
                    .as_ref()
                    .unwrap()
                    .pack_window_inbox_held(),
                "actual result remains delayed; grants stay owned across frames"
            );
            assert_eq!(bread(&hg.state.players[0].inventory), 0);
        }
        // Re-send the exact retained request through the same real transport.
        hg.state.remote_client.as_ref().unwrap().send_raw(&original);
        host.tick();
        hg.frames(1);
        trace("repeat while receipt held", &host, &hg, i, target, &delivery);
        if !after_application {
            delivery.lock().unwrap().hold_results = false;
            hg.frames(1);
        }
        let last = delivery.lock().unwrap().results.last().expect("actual result metadata").3;
        wait_applied(&mut hg, last);
        trace("before conservation assertion", &host, &hg, i, target, &delivery);
        assert_eq!(
            bread(&hg.state.players[0].inventory),
            if full { 0 } else { 2 },
            "repeated numbered reward applied twice at actual GameState intake"
        );
        if full {
            let returned: u32 = delivery.lock().unwrap()
                .unfit
                .iter()
                .filter_map(|packet| match packet.action {
                    crate::protocol::ItemAction::GrantUnfit { count, .. } => Some(u32::from(count)),
                    _ => None,
                })
                .sum();
            assert_eq!(
                returned, 2,
                "only one genuine overflow report may arise from a repeated grant carrier"
            );
        }
        delivery.lock().unwrap().hold_receipts = false;
        pack_wait(&mut host, &mut hg, |server, game| {
            !game.state.pack_window_frozen(0) && server.server.players[i].pack_flight.is_none()
        });
        assert_eq!(
            bread(&crate::window_events::effective_window(&host.server.players[i]).inv),
            if full { 0 } else { 2 }
        );
        assert!(host.server.players[i].is_in_world());
    }
}

struct PartialRig {
    hg: HeadlessGame,
    host: crate::hosted_server::HostedServer,
    i: usize,
    delivery: Arc<Mutex<Delivery>>,
    target: u32,
    before_event: u32,
    before_window: crate::staged_window::MarkerSlots,
    last: u32,
}

fn physical_window(hg: &HeadlessGame) -> crate::staged_window::MarkerSlots {
    let slot = &hg.state.players[0];
    crate::staged_window::MarkerWindow::try_capture_parts(&slot.inventory, &slot.armour_slots, &slot.crafting_ui.cursor_item, &slot.crafting_ui.grid, &hg.state.registry).unwrap().slots()
}

fn partial_setup(mode: InventoryMode, tag: &str, hold_rewards: bool, hold_last: bool) -> PartialRig {
    let (mut hg, mut host, i, delivery, target, incarnation, id) = setup(mode, tag, false);
    let before_event = hg.state.remote_client.as_ref().unwrap().applied_window_event();
    let before_window = physical_window(&hg);
    {
        let mut d = delivery.lock().unwrap();
        d.hold_rewards = hold_rewards;
        d.hold_last = hold_last;
        d.hold_receipts = true;
        d.results.clear();
        d.views.clear();
        d.personal_acks.clear();
        d.unfit.clear();
    }
    hg.state.request_quest(target, incarnation, QuestOperation::TurnIn { id });
    // Actual world-ready intake is the prerequisite. The offending baseline
    // has already drained the cost, so pending-cost presence is not a wait.
    pack_wait(&mut host, &mut hg, |_, game| {
        let d = delivery.lock().unwrap();
        game.state.remote_client.as_ref().is_some_and(|client| client.quest_world_intake_observed())
            && d.results.last().is_some_and(|(_, _, _, last, _)| d.views.iter().any(|view| view > last))
            && (!hold_rewards || !d.rewards.is_empty())
            && (!hold_last || !d.personal.is_empty())
    });
    trace("partial batch at actual world intake", &host, &hg, i, target, &delivery);
    let (fate, note, cost, last, base) = *delivery.lock().unwrap().results.last().unwrap();
    assert_eq!(fate, crate::action_identity::ActionFate::Accepted);
    assert_eq!(note, crate::quest_wire::QuestNote::Completed);
    assert_eq!(cost, before_event + 1);
    assert_eq!(last, cost + 2, "the real fixture has one frozen reward and one personal fence");
    assert_eq!(base, before_event);
    assert!(host.server.players[i].personal.as_ref().unwrap().record().unwrap().quests.accepted.is_empty());
    assert!(hg.state.pack_window_frozen(0), "held receipt still owns the existing shared flight");
    {
        let d = delivery.lock().unwrap();
        if hold_rewards {
            let (_, body) = crate::protocol::deserialize_header(d.rewards.front().unwrap()).unwrap();
            let grant: crate::protocol::InventoryGrantPacket = crate::protocol::safe_deserialize(body).unwrap();
            assert_eq!((grant.window_event, grant.count), (cost + 1, 2));
        }
        if hold_last {
            let (_, body) = crate::protocol::deserialize_header(d.personal.front().unwrap()).unwrap();
            let crate::protocol::PersonalSyncPacket::Snapshot(snapshot) = crate::protocol::safe_deserialize(body).unwrap() else { panic!("held carrier is the actual LAST snapshot") };
            assert_eq!((snapshot.window_event, snapshot.life), (last, host.server.players[i].respawns));
        }
    }
    PartialRig { hg, host, i, delivery, target, before_event, before_window, last }
}

fn partial(hold_rewards: bool, hold_last: bool) {
    isolate_saves();
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = partial_setup(mode, &format!("partial-{hold_rewards}-{hold_last}"), hold_rewards, hold_last);
        // Own runtime assertion: decode and world intake cannot promote a
        // partial cost/view beyond its genuinely missing grant or LAST body.
        assert_eq!(rig.hg.state.remote_client.as_ref().unwrap().applied_window_event(), rig.before_event, "a higher view advanced the physical prefix while the quest batch was incomplete");
        assert_eq!(physical_window(&rig.hg), rig.before_window, "partial quest carriers spent or rewarded the actual player");
        assert_eq!(rig.hg.state.players[0].quest_progress.accepted.len(), 1);
        assert!(rig.delivery.lock().unwrap().personal_acks.is_empty(), "an incomplete quest batch acknowledged its progression");
        assert!(rig.delivery.lock().unwrap().unfit.is_empty());
        {
            let mut d = rig.delivery.lock().unwrap();
            d.hold_rewards = false;
            d.hold_last = false;
        }
        wait_applied(&mut rig.hg, rig.last);
        trace("partial batch completed", &rig.host, &rig.hg, rig.i, rig.target, &rig.delivery);
        assert_eq!(bread(&rig.hg.state.players[0].inventory), 2);
        assert!(rig.hg.state.players[0].quest_progress.accepted.is_empty());
        assert!(rig.delivery.lock().unwrap().personal_acks.iter().any(|&(event, life)| event == rig.last && life == 0));
        rig.delivery.lock().unwrap().hold_receipts = false;
        pack_wait(&mut rig.host, &mut rig.hg, |server, game| !game.state.pack_window_frozen(0) && server.server.players[rig.i].pack_flight.is_none());
        assert_eq!(bread(&crate::window_events::effective_window(&rig.host.server.players[rig.i]).inv), 2);
        assert!(rig.host.server.players[rig.i].is_in_world());
    }
}

#[test]
#[ignore = "needs actual GPU window intake with partial reliable followups"]
fn game_harness_d2c3_partial_cost_view_waits_for_reward_and_last() { partial(true, true); }

#[test]
#[ignore = "needs actual GPU window intake with partial reliable followups"]
fn game_harness_d2c3_partial_last_waits_for_missing_reward() { partial(true, false); }

#[test]
#[ignore = "needs actual GPU window intake with partial reliable followups"]
fn game_harness_d2c3_partial_reward_waits_for_missing_last() { partial(false, true); }

#[test]
#[ignore = "needs actual GPU death, minimum delay, respawn and current personal proof"]
fn game_harness_d2c3_partial_old_quest_does_not_block_respawn_or_resurrect_reward() {
    isolate_saves();
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = partial_setup(mode, "partial-death", true, true);
        let i = rig.i;
        let last = rig.last;
        let old_life = rig.host.server.players[rig.i].respawns;
        // A genuine authoritative death runs through the normal host tick.
        // Put the respawn away from the old grave, so a real pickup cannot
        // be mistaken for an old reward carrier reinstalling stock.
        let sp = &mut rig.host.server.players[rig.i];
        sp.spawn_pos = sp.player.pos + glam::Vec3::new(24.0, 0.0, 0.0);
        sp.combat.health = 0.0;
        sp.combat.dead = true;
        sp.death_handled = false;
        partial_death_wait(&mut rig.host, &mut rig.hg, i, last, &rig.delivery, "minimum-dead-delay", |server, _| {
            let sp = &server.server.players[i];
            sp.death_handled && sp.combat.dead && sp.dead_ticks >= crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN
        });
        assert!(rig.hg.state.players[0].combat.dead, "the real server death must reach the client's death-screen state");
        assert!(rig.hg.state.players[0].combat.request_respawn(), "choose Respawn through the normal client combat/UI path");
        let new_life = old_life.checked_add(1).unwrap();
        partial_death_wait(&mut rig.host, &mut rig.hg, i, last, &rig.delivery, "current-life-personal-proof", |server, game| {
            let sp = &server.server.players[i];
            sp.respawns == new_life && sp.is_present_and_alive()
                && game.state.remote_client.as_ref().is_some_and(|client| client.applied_actor_respawns() == new_life && client.personal_inbox.as_ref().is_some_and(|inbox| inbox.applied_through(last, new_life)))
                && sp.personal.as_ref().is_some_and(|personal| sp.personal_delivery.quiescent(personal, new_life))
        });
        assert_eq!(bread(&rig.hg.state.players[0].inventory), 0);
        assert!(rig.hg.state.players[0].quest_progress.accepted.is_empty());
        assert!(rig.delivery.lock().unwrap().personal_acks.iter().any(|&(event, life)| event >= rig.last && life == new_life), "only real current-life progression settles the old flight");
        assert!(!rig.delivery.lock().unwrap().personal.is_empty(), "the old LAST is still genuinely withheld");
        {
            let mut d = rig.delivery.lock().unwrap();
            d.hold_rewards = false;
            d.hold_last = false;
            d.hold_receipts = false;
        }
        partial_death_wait(&mut rig.host, &mut rig.hg, i, last, &rig.delivery, "old-carriers-and-receipt-retired", |server, game| !game.state.pack_window_frozen(0) && server.server.players[i].pack_flight.is_none());
        assert_eq!(bread(&rig.hg.state.players[0].inventory), 0, "late old-body rewards cannot reappear after actual respawn");
        assert!(rig.hg.state.remote_client.as_ref().unwrap().is_connected());
        let graves: u32 = rig.host.server.world.iter_graves().flat_map(|(_, grave)| grave.slots.iter().flatten()).filter(|stack| stack.item == crate::item::Item::Material(crate::item::MaterialId::Bread)).map(|stack| u32::from(stack.count)).sum();
        let floor: u32 = rig.host.server.ecs.query::<&crate::entity::ItemEntity>().iter().filter(|(_, item)| item.stack.item == crate::item::Item::Material(crate::item::MaterialId::Bread)).map(|(_, item)| u32::from(item.stack.count)).sum();
        assert_eq!(graves + floor + bread(&crate::window_events::effective_window(&rig.host.server.players[rig.i]).inv), 2, "death retains the original reward once in authoritative custody");
    }
}

/// Stage-tagged observation of the real lifecycle schedule. Only non-secret
/// scalar state is printed, and no marker, acknowledgement or body is set.
fn partial_death_wait(server: &mut crate::hosted_server::HostedServer, hg: &mut HeadlessGame, i: usize, last: u32, delivery: &Arc<Mutex<Delivery>>, stage: &str, mut ready: impl FnMut(&crate::hosted_server::HostedServer, &HeadlessGame) -> bool) {
    for _ in 0..200 {
        if ready(server, hg) {
            partial_death_trace(server, hg, i, last, delivery, stage);
            return;
        }
        harness_step(server, hg);
        if !hg.state.remote_client.as_ref().is_some_and(|client| client.is_connected()) {
            partial_death_trace(server, hg, i, last, delivery, stage);
            panic!("partial quest lifecycle stage {stage} lost its connection");
        }
    }
    partial_death_trace(server, hg, i, last, delivery, stage);
    assert!(ready(server, hg), "partial quest lifecycle stage {stage} did not converge");
}

fn partial_death_trace(server: &crate::hosted_server::HostedServer, hg: &HeadlessGame, i: usize, last: u32, delivery: &Arc<Mutex<Delivery>>, stage: &str) {
    let sp = &server.server.players[i];
    let client = hg.state.remote_client.as_ref().unwrap();
    let life = client.applied_actor_respawns();
    let d = delivery.lock().unwrap();
    eprintln!("Partial quest lifecycle stage={stage} server_dead={} dead_ticks={} handled={} server_life={} alive={} client_dead={} client_health={} applied_life={life} prefix={} personal_pending={:?} personal_applied={} delivery_pending={:?} quiescent={} server_flight={} held_rewards={} held_personal={} held_receipts={}",
        sp.combat.dead, sp.dead_ticks, sp.death_handled, sp.respawns, sp.is_present_and_alive(), hg.state.players[0].combat.dead, hg.state.players[0].combat.health, client.applied_window_event(),
        client.personal_inbox.as_ref().map(|inbox| (inbox.life, inbox.pending_count())), client.personal_inbox.as_ref().is_some_and(|inbox| inbox.applied_through(last, life)),
        sp.personal_delivery.pending.as_ref().map(|snapshot| (snapshot.life, snapshot.window_event, snapshot.revision)), sp.personal.as_ref().is_some_and(|personal| sp.personal_delivery.quiescent(personal, sp.respawns)),
        sp.pack_flight.is_some(), d.rewards.len(), d.personal.len(), d.receipts.len());
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_d2c3_repeat_batch_before_actual_application_conserves_reward() {
    replay(false, false);
}
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_d2c3_repeat_batch_after_application_before_receipt_conserves_reward() {
    replay(true, false);
}
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_d2c3_repeat_batch_full_bag_reports_genuine_overflow_once() {
    replay(true, true);
}
