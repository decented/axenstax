//! Conservation against frozen accepted terms after actual continuation.
use super::*;

pub(super) fn giver_entity(game: &HeadlessGame, npc: [u8; 16]) -> hecs::Entity {
    let entities: Vec<_> = game
        .state
        .ecs
        .query::<&crate::quest_npcs::QuestNpcId>()
        .iter()
        .filter(|(_, id)| id.0 == npc)
        .map(|(e, _)| e)
        .collect();
    assert_eq!(entities.len(), 1, "the original durable giver is unique");
    entities[0]
}
pub(in crate::test_integration) fn complete_native_once(game: &mut HeadlessGame) {
    assert!(game.state.hosted_server.is_none() && !game.state.joined());
    let quest = native(game).quests.accepted[0].clone();
    let treasury = game.state.world.village_treasuries.clone();
    match quest.flavour {
        crate::quest::QuestFlavour::Fetch { item, count } | crate::quest::QuestFlavour::Make { item, count } => {
            let pos = game.state.players[0].player.pos + Vec3::Y * 0.5;
            crate::entity::spawn_item(
                &mut game.state.ecs,
                pos,
                crate::item::ItemStack::new_material(item, count),
                game.state.tick_counter as u32,
            );
            let deadline = game.state.tick_counter + 100;
            while game.state.players[0].inventory.count_material(item) < u16::from(count) {
                game.ticks(1);
                assert!(game.state.tick_counter < deadline, "actual world pickup did not supply the natural quest");
            }
        }
        crate::quest::QuestFlavour::Kill { mob, count } => {
            let have = crate::quest_play::kill_progress(&native(game), &quest).unwrap();
            for _ in have..u32::from(count) {
                let pos = game.state.players[0].player.pos + Vec3::X;
                let victim = crate::entity::spawn_mob(&mut game.state.ecs, mob, pos);
                game.state.ecs.get::<&mut crate::combat::Health>(victim).unwrap().current = 1.0;
                let slot = &game.state.players[0];
                assert!(crate::combat::strike(
                    &mut game.state.ecs,
                    victim,
                    &crate::combat::Swing {
                        attacker: crate::combat::Attacker::Local(0),
                        owner_key: "local-player-0",
                        eye: slot.player.eye_pos(),
                        look_dir: (pos - slot.player.eye_pos()).normalize(),
                        crit: false,
                        sprinting: false,
                        sneaking: false,
                        base_damage: slot.inventory.hotbar_attack_damage(slot.hotbar_slot),
                    }
                ));
                game.ticks(1);
                assert!(!game.state.ecs.contains(victim));
            }
        }
    }
    native_setup::original_dialogue(game, quest.npc);
    let giver = giver_entity(game, quest.npc);
    game.state.sats_policy.bitcoin_enabled = true;
    game.state.players[0].charter_allows_sats = true;
    assert!(
        game.state.players[0].inventory.free_slot_count() > quest.rewards.len(),
        "this control proves all exact rewards fit"
    );
    let expected_bag = paid_bag(bag(&game.state.players[0]), &quest);
    let expected_reputation = reward_reputation(game.state.players[0].reputation.score(quest.village), &quest);
    let before_drops = drops(&game.state.ecs);
    let before_revision = game.state.players[0].gameplay_revision;
    game.state.turn_in_native_quest(0, giver, quest.npc, quest.id).unwrap();
    assert_eq!(bag(&game.state.players[0]), expected_bag, "exact frozen Fetch/Make cost and every reward once");
    assert_eq!(drops(&game.state.ecs), before_drops, "fitting rewards do not invent or lose an overflow drop");
    assert_eq!(game.state.players[0].reputation.score(quest.village), expected_reputation);
    assert_eq!(game.state.players[0].gameplay_revision, before_revision + 1);
    assert!(native(game).quests.accepted.is_empty());
    assert_eq!(game.state.world.village_treasuries, treasury, "portable completion has zero monetary treasury effect");
    let done = native(game);
    let done_bag =
        bincode::serialize(&game.state.players[0].inventory.slots_iter().map(|slot| slot.cloned()).collect::<Vec<_>>())
            .unwrap();
    assert!(game.state.turn_in_native_quest(0, giver, quest.npc, quest.id).is_err());
    assert_eq!(native(game), done);
    assert_eq!(
        bincode::serialize(&game.state.players[0].inventory.slots_iter().map(|slot| slot.cloned()).collect::<Vec<_>>())
            .unwrap(),
        done_bag
    );
    game.state.save_live_world().unwrap();
    let folder = game.state.world_name.clone();
    assert!(game.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby));
    open(game, &folder);
    let record = native(game);
    assert!(record.quests.accepted.is_empty());
    assert_eq!(record.kills, done.kills);
    assert_eq!(record.reputation, done.reputation);
    assert_eq!(bag(&game.state.players[0]), expected_bag, "checked reopen preserves paid holdings");
    let reopened_drops = drops(&game.state.ecs);
    let giver = giver_entity(game, quest.npc);
    assert!(
        game.state.turn_in_native_quest(0, giver, quest.npc, quest.id).is_err(),
        "reopen cannot pay the completed quest again"
    );
    assert_eq!(bag(&game.state.players[0]), expected_bag, "reopened duplicate cannot pay or consume again");
    assert_eq!(drops(&game.state.ecs), reopened_drops, "reopened refusal has no overflow effects");
    assert_eq!(game.state.world.village_treasuries, treasury);
}

pub(super) fn bag(slot: &crate::player_slot::PlayerSlot) -> BTreeMap<Vec<u8>, i64> {
    let mut values = BTreeMap::new();
    for stack in slot.inventory.slots_iter().flatten() {
        *values.entry(bincode::serialize(&stack.item).unwrap()).or_default() += i64::from(stack.count);
    }
    values
}
pub(super) fn drops(ecs: &hecs::World) -> BTreeMap<Vec<u8>, i64> {
    let mut values = BTreeMap::new();
    for (_, entity) in ecs.query::<&crate::entity::ItemEntity>().iter() {
        *values.entry(bincode::serialize(&entity.stack.item).unwrap()).or_default() += i64::from(entity.stack.count);
    }
    values
}
pub(super) fn paid_bag(
    mut before: BTreeMap<Vec<u8>, i64>,
    quest: &crate::progression::AcceptedQuest,
) -> BTreeMap<Vec<u8>, i64> {
    if let crate::quest::QuestFlavour::Fetch { item, count } | crate::quest::QuestFlavour::Make { item, count } =
        quest.flavour
    {
        *before.entry(bincode::serialize(&crate::item::Item::Material(item)).unwrap()).or_default() -= i64::from(count);
    }
    for saved in &quest.rewards {
        let stack = saved.restore().unwrap().expect("natural frozen reward is nonempty");
        *before.entry(bincode::serialize(&stack.item).unwrap()).or_default() += i64::from(stack.count);
    }
    assert!(before.values().all(|count| *count >= 0));
    before.retain(|_, count| *count != 0);
    before
}
pub(super) fn reward_reputation(before: i16, quest: &crate::progression::AcceptedQuest) -> i16 {
    before.saturating_add(quest.reputation).clamp(crate::reputation::REP_MIN, crate::reputation::REP_MAX)
}
pub(super) fn giver_entity_server(host: &HostedServer, npc: [u8; 16]) -> hecs::Entity {
    let values: Vec<_> = host
        .server
        .ecs
        .query::<&crate::quest_npcs::QuestNpcId>()
        .iter()
        .filter(|(_, id)| id.0 == npc)
        .map(|(e, _)| e)
        .collect();
    assert_eq!(values.len(), 1);
    values[0]
}
