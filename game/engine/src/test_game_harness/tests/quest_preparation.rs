//! Real first-scope failure, current-live retry and selected-primary custody.
use super::*;
use crate::progression::{GameplayBinding, GameplayRecord, QuestProgress};
use crate::transport::ClientTransport as _;
use std::path::Path;

#[derive(Debug, PartialEq)]
pub(super) struct Live {
    pub(super) scope: Option<[u8; 16]>,
    pub(super) binding: Option<GameplayBinding>,
    local: Option<GameplayBinding>,
    progress: QuestProgress,
    clock: crate::quest_state::GameplayClock,
}

pub(super) fn live(game: &HeadlessGame) -> Live {
    let slot = &game.state.players[0];
    Live {
        scope: game.state.world.quest_scope,
        binding: slot.personal_binding.clone(),
        local: game.state.local_personal.clone(),
        progress: slot.quest_progress.clone(),
        clock: slot.gameplay_clock,
    }
}

pub(super) fn primary(directory: &Path) -> Vec<u8> {
    std::fs::read(directory.join("world.dat")).unwrap()
}

pub(super) fn join_accepts(
    client: &crate::transport::ChannelClientTransport,
) -> Vec<crate::protocol::JoinAcceptPacket> {
    let mut accepted = Vec::new();
    while let Some(bytes) = client.try_recv_from_server() {
        if let Some((crate::protocol::PacketType::JoinAccept, body)) =
            crate::protocol::deserialize_header(&bytes)
        {
            accepted.push(crate::protocol::safe_deserialize(body).unwrap());
        }
    }
    accepted
}

pub(super) fn selected_record(directory: &Path) -> (crate::save::WorldSave, GameplayRecord) {
    let selected = crate::save::read_world_save(&primary(directory)).unwrap();
    let saved =
        crate::joiner_save::JoinerSave::decode(&selected.local_personal.as_ref().unwrap().0)
            .unwrap();
    let record = saved.personal.as_ref().unwrap().record().unwrap();
    (selected, record)
}

pub(super) fn kill_villager(game: &mut HeadlessGame, village: (i32, i32)) -> u64 {
    let pos = game.state.players[0].player.pos + glam::Vec3::X;
    game.state
        .world
        .village_anchors
        .insert(village, pos.to_array().map(|v| v.floor() as i32));
    let victim = crate::entity::spawn_mob(&mut game.state.ecs, crate::mob::MobType::Villager, pos);
    game.state
        .ecs
        .get::<&mut crate::combat::Health>(victim)
        .unwrap()
        .current = 0.0;
    game.ticks(1);
    assert!(
        !game.state.ecs.contains(victim),
        "the actual native sweep consumed the corpse"
    );
    game.state.players[0]
        .last_villager_kill_rep_tick
        .expect("real legacy penalty tick")
}

fn custody(game: &HeadlessGame) -> (Vec<u8>, [usize; 3]) {
    let slot = &game.state.players[0];
    let bytes = bincode::serialize(&(
        slot.inventory
            .slots_iter()
            .map(|stack| stack.cloned())
            .collect::<Vec<_>>(),
        slot.armour_slots,
        &slot.crafting_ui.cursor_item,
        &slot.crafting_ui.grid,
    ))
    .unwrap();
    let ptr = |stack: &crate::item::ItemStack| match &stack.item {
        crate::item::Item::Plan(plan) => plan.cells.as_ptr() as usize,
        _ => panic!("actual private Plan body required"),
    };
    (
        bytes,
        [
            ptr(slot.inventory.slot(1).unwrap()),
            ptr(slot.crafting_ui.cursor_item.as_ref().unwrap()),
            ptr(slot.crafting_ui.grid[0][0].as_ref().unwrap()),
        ],
    )
}

fn private_bodies(game: &mut HeadlessGame) {
    let plan = |name: &str| crate::item::ItemStack {
        item: crate::item::Item::Plan(crate::plan::PlanData {
            name: name.into(),
            ..crate::plan::PlanData::debug_3x3_stone()
        }),
        count: 1,
    };
    let slot = &mut game.state.players[0];
    slot.inventory
        .set_slot(1, Some(plan("Pending bag private body")));
    slot.crafting_ui.cursor_item = Some(plan("Pending cursor private body"));
    slot.crafting_ui.grid[0][0] = Some(plan("Pending grid private body"));
    let mut armour = crate::armour::ArmourItem::new(
        crate::armour::ArmourSlot::Helmet,
        crate::armour::ArmourMaterial::Iron,
    );
    armour.durability = 7;
    slot.armour_slots[0] = Some(armour);
}

pub(super) fn handover_env(directory: &Path) -> crate::handover_game::SourceEnv {
    use nostr::{Keys, SecretKey, Timestamp, ToBech32};
    let persona = Keys::new(SecretKey::from_slice(&[83; 32]).unwrap());
    let runtime = Keys::new(SecretKey::from_slice(&[84; 32]).unwrap());
    let now = 1_800_000_000;
    let root = crate::handover::context::RootContext::new(
        runtime.public_key().to_bytes(),
        &persona.public_key().to_bech32().unwrap(),
        now + 3600,
        &["ws://127.0.0.1:9".into()],
        now,
    )
    .unwrap();
    let attestation = pollster::block_on(crate::runtime_identity::mint_player_attestation(
        &persona,
        &runtime.public_key(),
        Timestamp::from(now),
        1,
    ))
    .unwrap();
    crate::handover_game::SourceEnv {
        runtime,
        root,
        scratch: directory.join("scratch"),
        bearer: crate::handover::live::bearer::Bearer::new([85; 16]),
        attestation,
        continuation: None,
    }
}

pub(super) struct Retry {
    scope: [u8; 16],
    first_tick: u64,
    village: (i32, i32),
    custody: (Vec<u8>, [usize; 3]),
}

pub(super) fn native_scope_failure_tail(
    game: &mut HeadlessGame,
    directory: &Path,
    fault: &crate::fsync::tests::DirectoryFault,
) -> Retry {
    let (selected, initial) = selected_record(directory);
    let scope = selected.quest_world.as_ref().unwrap().scope;
    assert_eq!(
        game.state.gameplay_preparation.as_ref().unwrap().scope,
        scope
    );
    assert!(
        matches!(initial.context, crate::progression::GameplayContext::WorldLocal {
        scope: saved, player: crate::progression::WorldPlayer::LegacyLocal(0),
    } if saved == scope)
    );
    // These effects happen AFTER failure, on the still-unbound real owner.
    // A retry cannot install the stale, pre-effect selected progression.
    clear_pad(game, None);
    let village = (7, 8);
    let first_tick = kill_villager(game, village);
    assert_eq!(
        game.state.players[0].kill_counter[&crate::mob::MobType::Villager],
        1
    );
    assert_eq!(game.state.players[0].reputation.per_village[&village], -25);
    game.ticks(7);
    game.state.players[0].inventory.set_slot(
        4,
        Some(crate::item::ItemStack::new_material(
            crate::item::MaterialId::Stick,
            3,
        )),
    );
    private_bodies(game);
    let owned = custody(game);
    let before = live(game);
    let hits = fault.failures();
    assert!(
        game.state.prepare_local_gameplay().is_err(),
        "persistent fault still refuses retry"
    );
    assert!(
        fault.failures() > hits,
        "the retry itself reached its strict barrier"
    );
    assert_eq!(live(game), before);
    assert_eq!(
        custody(game),
        owned,
        "persistent refusal retains exact private bodies and live allocations"
    );
    let (retried, record) = selected_record(directory);
    assert_eq!(retried.quest_world.as_ref().unwrap().scope, scope);
    assert_eq!(record.quests.penalty_left, 593);
    assert_eq!(
        record
            .reputation
            .iter()
            .find(|(id, _)| *id == village)
            .map(|(_, n)| *n),
        Some(-25)
    );
    let retained = primary(directory);
    assert!(
        game.state.save_live_world().is_err(),
        "ordinary manual save cannot substitute unbound authority"
    );
    assert_eq!(primary(directory), retained);
    game.state.pause_save();
    assert!(game.state.toast.is_some());
    assert_eq!(
        primary(directory),
        retained,
        "the actual pause-save path cannot replace provisional authority"
    );
    assert_eq!(custody(game), owned);
    assert!(!game.state.leave_world(
        crate::world_exit::SaveChoice::Save,
        crate::world_exit::ExitTo::Lobby
    ));
    assert_eq!(
        primary(directory),
        retained,
        "close/save refuses before replacing the actual primary"
    );
    assert_eq!(live(game), before);
    assert_eq!(custody(game), owned);
    let autosave = directory.join("autosave");
    assert!(!autosave.exists());
    game.state.autosave_counter = 5999;
    game.ticks(1);
    assert_eq!(
        game.state.autosave_counter, 0,
        "an actual autosave boundary was reached"
    );
    assert!(
        !autosave.exists(),
        "no newer contradictory autosave was created"
    );
    assert_eq!(primary(directory), retained);
    assert_eq!(
        custody(game),
        owned,
        "actual refused autosave retains private body custody"
    );
    assert_eq!(
        game.state
            .begin_handover_with((1, 1, [86; 32]), handover_env(directory), 1_800_000_000),
        Err(crate::gameplay_preparation::PENDING.into())
    );
    assert!(game.state.handover.is_none());
    assert_eq!(primary(directory), retained);
    game.ticks(4);
    assert_eq!(custody(game), owned);
    Retry {
        scope,
        first_tick,
        village,
        custody: owned,
    }
}

pub(super) fn assert_native_retry(game: &HeadlessGame, directory: &Path, retry: Retry) {
    assert!(game.state.gameplay_preparation.is_none());
    assert_eq!(game.state.world.quest_scope, Some(retry.scope));
    let (selected, record) = selected_record(directory);
    assert_eq!(selected.quest_world.as_ref().unwrap().scope, retry.scope);
    let binding = game.state.players[0].personal_binding.as_ref().unwrap();
    assert_eq!(
        record,
        GameplayRecord::from_slot(binding.context(), &game.state.players[0]).unwrap()
    );
    assert_eq!(
        record.quests.penalty_left,
        (600 - (game.state.tick_counter - retry.first_tick)) as u32
    );
    assert_eq!(
        record
            .kills
            .iter()
            .find(|(kind, _)| *kind == crate::mob::MobType::Villager)
            .map(|(_, n)| *n),
        Some(1)
    );
    assert_eq!(
        record
            .reputation
            .iter()
            .find(|(id, _)| *id == retry.village)
            .map(|(_, n)| *n),
        Some(-25)
    );
    assert_eq!(game.state.players[0].inventory.slot(4).unwrap().count, 3);
    assert_eq!(
        custody(game),
        retry.custody,
        "successful current-live retry retains the same private body allocations"
    );
    assert!(!game.state.session_saves.save_failed);
}
