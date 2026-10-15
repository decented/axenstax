//! Real signed role cut from a played retained folder and canonical return.
use super::*;
use crate::{hosted_server::quest_lifecycle_actor::Actor, quest_wire::QuestOperation};

pub(in crate::test_integration) fn complete_dedicated_and_return(game: &mut HeadlessGame, secret: [u8; 32]) {
    let before = native(game);
    let quest = before.quests.accepted[0].clone();
    let scope = game.state.world.quest_scope.unwrap();
    let folder = game.state.world_name.clone();
    let seed = game.state.biome_gen.seed;
    game.state.save_live_world().unwrap();
    assert!(game.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby));
    let mut host = HostedServer::start(0, folder.clone(), seed, 0, RemoteTransport::WebSocket { port: 0 }).unwrap();
    host.server.difficulty = crate::survival::Difficulty::Peaceful;
    let client = host.attach_test_remote_with_binding(Some([111; 32]));
    crate::hosted_server::tests::send_signed_join(&client, secret, Some([111; 32]));
    host.tick();
    let accept = client.try_recv_from_server().expect("real signed JoinAccept");
    let (tag, body) = protocol::deserialize_header(&accept).unwrap();
    assert_eq!(tag, PacketType::JoinAccept);
    let accepted: protocol::JoinAcceptPacket = protocol::safe_deserialize(body).unwrap();
    let mut actor = Actor::new(
        client,
        accepted.clone(),
        crate::player_slot::PlayerSlot::new(0, Vec3::ZERO, 1.0),
        crate::personal_sync::Inbox::new(accepted.action_session, accepted.action_life),
        0,
    );
    actor.drain(&host.server.registry);
    assert_eq!(actor.initial_carriers, [(1, "sync"), (2, "personal")]);
    native_setup::settle_actor(&mut host, &mut actor);
    let i = actor.i();
    assert!(host.server.players[i].verified_pubkey == Some(key(secret)));
    assert!(host.server.selected_local_personal.is_none(), "actual own-key claim completed its role cut");
    assert_eq!(host.server.world.quest_scope, Some(scope));
    assert_eq!(host.server.players[i].personal.as_ref().unwrap().record().unwrap().quests.accepted, [quest.clone()]);
    let giver = completion::giver_entity_server(&host, quest.npc);
    let target = host.server.ecs.get::<&crate::entity::ProtocolId>(giver).unwrap().0;
    native_setup::walk_to_giver(&mut host, &mut actor, target);
    let view = native_setup::actor_quest(&mut host, &mut actor, target, 0, QuestOperation::Talk).unwrap();
    assert_eq!(view.npc, quest.npc);
    assert_eq!(view.terms.as_ref().unwrap().id, quest.id);
    let (item, count) = match quest.flavour {
        crate::quest::QuestFlavour::Fetch { item, count } | crate::quest::QuestFlavour::Make { item, count } => {
            (item, count)
        }
        _ => panic!("retained source fixture owns a genuine natural Fetch/Make"),
    };
    host.grant_to_joiner(i, [crate::item::ItemStack::new_material(item, count)]);
    native_setup::settle_actor(&mut host, &mut actor);
    assert!(actor.slot.inventory.free_slot_count() > quest.rewards.len());
    let bag = completion::paid_bag(completion::bag(&actor.slot), &quest);
    let rep = completion::reward_reputation(actor.slot.reputation.score(quest.village), &quest);
    let treasury = host.server.world.village_treasuries.clone();
    let drops = completion::drops(&host.server.ecs);
    let revision = host.server.players[i].personal.as_ref().unwrap().record().unwrap().revision;
    native_setup::actor_quest(&mut host, &mut actor, target, view.incarnation, QuestOperation::TurnIn { id: quest.id });
    assert_eq!(completion::bag(&actor.slot), bag);
    assert_eq!(actor.slot.reputation.score(quest.village), rep);
    assert_eq!(completion::drops(&host.server.ecs), drops);
    assert_eq!(host.server.world.village_treasuries, treasury);
    let done = host.server.players[i].personal.as_ref().unwrap().record().unwrap();
    assert!(done.quests.accepted.is_empty());
    assert_eq!(done.revision, revision + 1);
    native_setup::repeat_completed(&mut host, &mut actor, target, view.incarnation, quest.id);
    assert_eq!(completion::bag(&actor.slot), bag);
    assert_eq!(host.server.players[i].personal.as_ref().unwrap().record().unwrap().revision, done.revision);
    actor.client.send_to_server(&protocol::serialize_packet(PacketType::Disconnect, &()));
    host.tick();
    assert!(host.slot_is_free(i));
    host.server.try_save().unwrap();
    let selected =
        crate::save::read_world_save(&std::fs::read(crate::save::world_dir(&folder).join("world.dat")).unwrap())
            .unwrap();
    assert!(selected.local_personal.is_none());
    let canonical = crate::handover::live::personal_adapter::path(&crate::save::world_dir(&folder), key(secret));
    let saved = crate::joiner_save::JoinerSave::decode(&std::fs::read(&canonical).unwrap()).unwrap();
    assert!(saved.personal.unwrap().record().unwrap().quests.accepted.is_empty());
    drop(actor);
    drop(host);
    open(game, &folder);
    assert_eq!(game.state.world.quest_scope, Some(scope));
    assert!(native(game).quests.accepted.is_empty());
    assert_eq!(native(game).kills, done.kills);
    assert_eq!(native(game).reputation, done.reputation);
    assert_eq!(completion::bag(&game.state.players[0]), bag);
    let reopened_drops = completion::drops(&game.state.ecs);
    let giver = completion::giver_entity(game, quest.npc);
    assert!(game.state.turn_in_native_quest(0, giver, quest.npc, quest.id).is_err());
    assert_eq!(completion::bag(&game.state.players[0]), bag, "returned duplicate cannot pay or consume again");
    assert_eq!(completion::drops(&game.state.ecs), reopened_drops);
    assert_eq!(game.state.world.village_treasuries, treasury);
}
