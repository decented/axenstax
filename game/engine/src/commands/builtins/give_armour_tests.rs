//! `/give` and armour: every piece of every tier, the spellings a player types for them
//! (`satori helmet`, `Satoshi armour`, `iron-boots`), whole sets, multi-word item names with a
//! trailing count, the helpful wrong-name message, and Tab completion of all of it.

use super::give::{resolve_item, GiveCommand};
use crate::armour::{all_armour_combos, ArmourMaterial, ArmourSlot};
use crate::commands::complete::CompletionContext;
use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, CommandRegistry, OpLevel};
use crate::item::Item;
use crate::player_slot::PlayerSlot;
use crate::world::World;

fn run(args: &[&str]) -> (CommandResult, Vec<PlayerSlot>, Vec<String>) {
    let cmd = GiveCommand;
    let mut world = World::new();
    let mut t = 0u32;
    let mut s = 4u32;
    let mut creative = false;
    let mut mode = crate::play_mode::PlayMode::Survival;
    let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
    let mut log = Vec::new();
    let (mut ch, mut ev, mut ps) = (false, false, false);
    let reg = CommandRegistry::new();
    let mut ctx = CommandContext {
        world: &mut world,
        world_time: &mut t,
        world_time_step: &mut s,
        is_creative: &mut creative,
        play_mode: &mut mode,
        seed: 42,
        world_name: "test",
        players: &mut players,
        player_idx: 0,
        op_level: OpLevel::Op,
        current_tick: 0,
        log: &mut log,
        registry: &reg,
        cheats_used_marker: &mut ch,
        ever_creative_marker: &mut ev,
        pure_survival_broken_marker: &mut ps,
    };
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let r = cmd.execute(&mut ctx, &owned);
    let lines: Vec<String> = log.iter().map(|l| l.text.clone()).collect();
    (r, players, lines)
}

/// Every armour piece the player now holds, as (slot, tier).
fn armour_held(players: &[PlayerSlot]) -> Vec<(ArmourSlot, ArmourMaterial)> {
    (0..36)
        .filter_map(|i| players[0].inventory.slot(i))
        .filter_map(|s| match &s.item {
            Item::Armour(a) => Some((a.slot, a.material)),
            _ => None,
        })
        .collect()
}

fn piece(name: &str) -> Option<(ArmourSlot, ArmourMaterial)> {
    match resolve_item(name, 1).ok()?.item {
        Item::Armour(a) => Some((a.slot, a.material)),
        _ => None,
    }
}

fn complete(before: &[&str], partial: &str) -> Vec<String> {
    let reg = CommandRegistry::new();
    let ctx = CompletionContext { op_level: OpLevel::Op, registry: &reg, roster_names: &[] };
    GiveCommand.complete(before, partial, &ctx)
}

#[test]
fn every_spelling_of_a_satori_piece_resolves_to_it() {
    use ArmourSlot::*;
    let want = ArmourMaterial::Satori;
    for (spelling, slot) in [
        ("satori_helmet", Helmet),
        ("satori helmet", Helmet),
        ("Satori-Helmet", Helmet),
        ("SATORI HELMET", Helmet),
        ("satoshi helmet", Helmet),
        ("satoshi_helmet", Helmet),
        ("satori helm", Helmet),
        ("helmet_satori", Helmet),
        ("satori_chestplate", Chestplate),
        ("satori chest", Chestplate),
        ("satoshi chestplate", Chestplate),
        ("satori_leggings", Leggings),
        ("satori legs", Leggings),
        ("satori_boots", Boots),
        ("satoshi boots", Boots),
    ] {
        assert_eq!(piece(spelling), Some((slot, want)), "'{spelling}'");
    }
}

#[test]
fn every_tier_and_slot_that_exists_is_a_suggested_name_and_resolves() {
    let names = super::give::item_names();
    for (slot, material) in all_armour_combos() {
        let name = format!("{}_{}", format!("{material:?}").to_lowercase(), format!("{slot:?}").to_lowercase());
        assert!(names.contains(&name), "Tab does not offer '{name}'");
        assert_eq!(piece(&name), Some((slot, material)), "'{name}'");
    }
}

#[test]
fn satori_armour_gives_the_full_set_of_four_under_every_spelling() {
    for spelling in [&["satori", "armour"][..], &["satori", "armor"], &["satoshi", "armour"], &["satoshi", "armor"], &["satori_armour"], &["Satori-Armor"]] {
        let (r, players, lines) = run(spelling);
        assert_eq!(r, CommandResult::Success, "{spelling:?}: {lines:?}");
        let mut got = armour_held(&players);
        got.sort_by_key(|(s, _)| *s as u8);
        let want: Vec<_> = [ArmourSlot::Helmet, ArmourSlot::Chestplate, ArmourSlot::Leggings, ArmourSlot::Boots]
            .into_iter()
            .map(|s| (s, ArmourMaterial::Satori))
            .collect();
        assert_eq!(got, want, "{spelling:?}");
    }
}

#[test]
fn every_tier_has_a_set_and_the_message_names_the_real_armour() {
    let (r, players, lines) = run(&["iron", "armour"]);
    assert_eq!(r, CommandResult::Success);
    assert_eq!(armour_held(&players).len(), 4);
    assert!(armour_held(&players).iter().all(|(_, m)| *m == ArmourMaterial::Iron));
    let (_, _, lines_satori) = run(&["satoshi", "armour"]);
    assert!(lines_satori.iter().any(|l| l.contains("Satori")), "{lines_satori:?}");
    assert!(!lines_satori.iter().any(|l| l.to_lowercase().contains("satoshi")), "{lines_satori:?}");
    let _ = lines;
}

#[test]
fn a_count_after_a_multi_word_name_is_the_count() {
    let (r, players, _) = run(&["satori", "helmet", "2"]);
    assert_eq!(r, CommandResult::Success);
    assert_eq!(armour_held(&players), vec![(ArmourSlot::Helmet, ArmourMaterial::Satori); 2]);

    let (r, players, _) = run(&["satori", "helmet"]);
    assert_eq!(r, CommandResult::Success);
    assert_eq!(armour_held(&players).len(), 1);

    let (r, players, _) = run(&["oak", "planks", "64"]);
    assert_eq!(r, CommandResult::Success);
    let stack = players[0].inventory.slot(0).unwrap();
    assert_eq!((stack.item.clone(), stack.count), (Item::Block(crate::block::OAK_PLANKS), 64));
}

#[test]
fn the_single_word_forms_still_work() {
    let (r, players, _) = run(&["stone", "5"]);
    assert_eq!(r, CommandResult::Success);
    assert_eq!(players[0].inventory.slot(0).unwrap().count, 5);
    let (r, players, _) = run(&["wooden_pickaxe"]);
    assert_eq!(r, CommandResult::Success);
    assert!(matches!(players[0].inventory.slot(0).unwrap().item, Item::Tool(_)));
}

#[test]
fn a_wrong_name_names_the_nearest_match_not_a_count_error() {
    let (r, _, lines) = run(&["satori", "helmt"]);
    assert!(matches!(r, CommandResult::Error(_)));
    let said = lines.join(" ");
    assert!(said.contains("Did you mean 'satori_helmet'"), "{said}");
    assert!(!said.contains("count"), "{said}");

    // A real item with a word that is not a number is still the count that is wrong.
    let (r, _, lines) = run(&["stone", "many"]);
    assert!(matches!(r, CommandResult::Error(_)));
    assert!(lines.join(" ").contains("can't parse 'many' as a count"), "{lines:?}");

    // Nothing near: the old hint, naming the new form too.
    let (_, _, lines) = run(&["zzzzzzzz"]);
    assert!(lines.join(" ").contains("unknown item"), "{lines:?}");
}

#[test]
fn tab_offers_the_armour_and_the_next_word_of_a_multi_word_name() {
    let names = complete(&[], "satori_h");
    assert!(names.contains(&"satori_helmet".to_string()), "{names:?}");
    let sets = complete(&[], "satori_a");
    assert!(sets.contains(&"satori_armour".to_string()), "{sets:?}");
    assert_eq!(complete(&["satori"], "hel"), vec!["helmet".to_string()]);
    assert!(complete(&["oak"], "pla").contains(&"planks".to_string()));
    assert!(complete(&["stone", "5"], "").is_empty(), "nothing is offered after the count");
}
