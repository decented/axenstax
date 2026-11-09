//! Living world Tier 2 S2: who built a block, kept with the world.
//!
//! The save half runs real servers over a real folder (a restart is the old server
//! dropped and a new one loaded from the same folder). The protection half runs a
//! real `HostedServer` with a joiner over the channel transport, and the pure rule
//! for a local seat; the local break arm itself is driven in
//! `test_game_harness/tests/authorship.rs` (needs a GPU adapter).
use super::joiner_authority::{block_changes_seen, cell_beside, join_guest, send_edits, start_open_server};
use crate::authorship::{AuthorKey, SavedAuthorship};
use crate::block;
use crate::hosted_server::pubkey_to_npub;
use crate::plan::{CapturedCell, ConstructionAnchorData, PlanData};
use crate::save::{read_world_save, world_dir, WorldsRootGuard};
use crate::server::GameServer;
use crate::world::World;

fn npub_key(pk: &[u8; 32]) -> AuthorKey {
    AuthorKey::Npub(pubkey_to_npub(pk))
}

fn saved_world_save(world: &str) -> crate::save::WorldSave {
    read_world_save(&std::fs::read(world_dir(world).join("world.dat")).unwrap()).unwrap()
}

// ── Save and load ───────────────────────────────────────────────────

#[test]
fn authorship_survives_a_save_and_load_and_breaking_after_a_load_clears_it() {
    let _g = WorldsRootGuard::new("authorship-roundtrip");
    let mut s = GameServer::new(0, "w".into(), 42);
    let bob = AuthorKey::Npub("npub1bob".into());
    s.world.place_player_block_as(3, 64, 5, block::STONE, Some(&AuthorKey::Local(0)));
    s.world.place_player_block_as(4, 64, 5, block::STONE, Some(&bob));
    s.world.place_player_block_as(-40, 70, 99, block::GLASS, Some(&bob));
    s.world.place_player_block_as(5, 64, 5, block::STONE, None); // a guest's
    s.world.set_block(6, 64, 5, block::STONE); // natural
    s.try_save().unwrap();

    let saved = saved_world_save("w").block_authors.expect("the field is written");
    assert_eq!(saved.keys, vec![AuthorKey::Local(0), bob.clone()], "sorted, only the used keys");
    assert_eq!(saved.chunks.iter().map(|c| c.3.len()).sum::<usize>(), 3, "a guest's block has no row");

    let mut back = GameServer::new(0, "w".into(), 42);
    back.initial_load().unwrap();
    assert_eq!(back.world.author_at(3, 64, 5), Some(&AuthorKey::Local(0)));
    assert_eq!(back.world.author_at(4, 64, 5), Some(&bob));
    assert_eq!(back.world.author_at(-40, 70, 99), Some(&bob));
    assert_eq!(back.world.author_at(5, 64, 5), None, "a guest's block stays anonymous");
    assert_eq!(back.world.author_at(6, 64, 5), None);

    // The loaded table is live: breaking goes through the same funnel.
    back.world.set_placed(4, 64, 5, false);
    assert_eq!(back.world.author_at(4, 64, 5), None);
    back.try_save().unwrap();
    assert_eq!(saved_world_save("w").block_authors.unwrap().chunks.iter().map(|c| c.3.len()).sum::<usize>(), 2);
}

#[test]
fn a_world_with_nothing_recorded_saves_the_field_absent() {
    let _g = WorldsRootGuard::new("authorship-absent");
    let mut s = GameServer::new(0, "w".into(), 42);
    s.world.place_player_block(3, 64, 5, block::STONE);
    s.try_save().unwrap();
    assert!(saved_world_save("w").block_authors.is_none());
}

#[test]
fn a_save_from_before_the_field_loads_with_everything_anonymous() {
    // A footer-61 writer had 56 fields: the same bytes without the three newest (authorship,
    // the use record and the Plan build counts, one None byte each).
    let mut save = crate::save::minimal_world_save_for_tests(7);
    save.block_authors = None;
    save.region_use = None;
    save.plan_builds = None;
    let mut body = bincode::serialize(&save).unwrap();
    assert_eq!(body.pop(), Some(0), "the absent Plan build counts are one None byte");
    assert_eq!(body.pop(), Some(0), "the absent use record is one None byte");
    assert_eq!(body.pop(), Some(0), "the absent field is one None byte");
    body.extend_from_slice(&crate::save_format::footer_bytes(61));
    let (old, damaged) = crate::save::read_world_save_reporting(&body).expect("an older save loads");
    assert!(!damaged);
    assert!(old.block_authors.is_none());
    let mut w = World::new();
    w.place_player_block(3, 64, 5, block::STONE);
    crate::save::apply_world_save_state(&mut w, &old);
    assert!(w.authors.is_empty());
    assert!(w.is_placed(3, 64, 5), "the block itself is untouched");
    assert_eq!(w.author_at(3, 64, 5), None);
}

#[test]
fn an_invalid_record_is_dropped_with_a_warning_never_a_refusal_to_load() {
    let mut save = crate::save::minimal_world_save_for_tests(8);
    save.block_authors = Some(SavedAuthorship {
        keys: vec![AuthorKey::Local(0)],
        chunks: vec![(0, 4, 0, vec![(7, 0), (8, 9), (5000, 0)])],
        starters: vec![(1, 2, 3, 44)],
    });
    let bytes = crate::save_format::encode_world_save(&save).unwrap();
    let (back, damaged) = crate::save::read_world_save_reporting(&bytes).expect("the world still loads");
    assert!(!damaged);
    assert_eq!(back.block_authors, save.block_authors, "the reader keeps what it was given");
    let mut w = World::new();
    w.place_player_block(0, 64, 0, block::STONE); // makes chunk (0,4,0) exist
    w.set_placed(7, 64, 0, true);
    w.set_block(7, 64, 0, block::STONE);
    crate::save::apply_world_save_state(&mut w, &back);
    assert_eq!(w.authors.total(), 1, "only the one valid row, which is placed, is kept");
    assert_eq!(w.author_at(7, 64, 0), Some(&AuthorKey::Local(0)));
}

#[test]
fn the_encoding_is_the_same_whatever_order_the_blocks_were_placed_in() {
    let cells: Vec<(i32, i32, i32, AuthorKey)> = vec![
        (3, 64, 5, AuthorKey::Npub("npub1zed".into())),
        (40, 70, -3, AuthorKey::Local(1)),
        (4, 64, 5, AuthorKey::Npub("npub1amy".into())),
        (-20, 90, 7, AuthorKey::Local(0)),
        (3, 65, 5, AuthorKey::Npub("npub1amy".into())),
    ];
    let bytes_for = |name: &str, order: &[usize]| {
        let _g = WorldsRootGuard::new(name);
        let mut s = GameServer::new(0, "w".into(), 42);
        for &i in order {
            let (x, y, z, k) = &cells[i];
            s.world.place_player_block_as(*x, *y, *z, block::STONE, Some(k));
        }
        s.world.authors.set_starter((9, 9, 9), Some(&AuthorKey::Npub("npub1zed".into())));
        bincode::serialize(&s.world.authors.to_saved().unwrap()).unwrap()
    };
    let a = bytes_for("authorship-order-a", &[0, 1, 2, 3, 4]);
    let b = bytes_for("authorship-order-b", &[4, 3, 2, 1, 0]);
    assert_eq!(a, b, "same blocks, same record");
}

#[test]
fn the_server_digest_covers_authorship_in_a_canonical_order() {
    let digest = |name: &str, order: &[(i32, AuthorKey)]| {
        let _g = WorldsRootGuard::new(name);
        let mut s = GameServer::new(0, "w".into(), 42);
        for (x, k) in order {
            s.world.place_player_block_as(*x, 64, 5, block::STONE, Some(k));
        }
        s.sim_digest()
    };
    let (a, b) = (AuthorKey::Npub("npub1a".into()), AuthorKey::Npub("npub1b".into()));
    let one = digest("authorship-digest-1", &[(3, a.clone()), (4, b.clone())]);
    let two = digest("authorship-digest-2", &[(4, b.clone()), (3, a.clone())]);
    assert_eq!(one, two);
    let other = digest("authorship-digest-3", &[(3, b), (4, a)]);
    assert_ne!(one, other, "who built what is part of the digest");
}

#[test]
fn a_crash_between_the_chunk_files_and_the_record_leaves_no_author_on_an_unplaced_cell() {
    let _g = WorldsRootGuard::new("authorship-crash");
    let mut s = GameServer::new(0, "w".into(), 42);
    s.world.place_player_block_as(3, 64, 5, block::STONE, Some(&AuthorKey::Local(0)));
    // The record is newer than the chunk file: it names a cell that is not placed,
    // and a chunk that was never written.
    s.world.authors.set(crate::authorship::chunk_of(4, 64, 5), crate::authorship::cell_of(4, 64, 5), Some(&AuthorKey::Local(0)));
    s.world.authors.set((300, 4, 300), 5, Some(&AuthorKey::Local(0)));
    s.try_save().unwrap();
    let mut back = GameServer::new(0, "w".into(), 42);
    back.initial_load().unwrap();
    assert_eq!(back.world.authors.total(), 1);
    assert_eq!(back.world.author_at(3, 64, 5), Some(&AuthorKey::Local(0)));
}

// ── Plan builds ─────────────────────────────────────────────────────

fn anchor_for(plan: PlanData, at: (i32, i32, i32), creative: bool, locked: Vec<crate::item::ItemStack>) -> ConstructionAnchorData {
    ConstructionAnchorData {
        plan,
        rotations: 0,
        anchor: at,
        placed_index: 0,
        locked_materials: locked,
        is_creative_build: creative,
        pace_divider: 1,
        pace_counter: 0,
        builder_credit: None,
    }
}

fn run_build(world: &mut World, at: (i32, i32, i32)) -> usize {
    let mut stamped = 0;
    for _ in 0..40 {
        stamped += crate::plan::tick_build(world, at).len();
        if crate::plan::build_is_complete(world, at) {
            break;
        }
    }
    stamped
}

#[test]
fn a_plan_starter_is_credited_after_a_save_and_reload_mid_build() {
    let _g = WorldsRootGuard::new("authorship-plan-reload");
    let at = (10, 64, 10);
    let mut s = GameServer::new(0, "w".into(), 42);
    let starter = AuthorKey::Npub("npub1starter".into());
    s.world.construction_anchors.insert(at, anchor_for(PlanData::debug_3x3_stone(), at, true, Vec::new()));
    s.world.authors.set_starter(at, Some(&starter));
    let first = crate::plan::tick_build(&mut s.world, at);
    assert_eq!(first.len(), crate::plan::BLOCKS_PER_BUILD_TICK, "part of the build stands");
    for &(x, y, z, _) in &first {
        assert_eq!(s.world.author_at(x, y, z), Some(&starter));
    }
    s.try_save().unwrap();
    assert!(saved_world_save("w").block_authors.unwrap().starters.len() == 1, "the starter is in the record");

    let mut back = GameServer::new(0, "w".into(), 42);
    back.initial_load().unwrap();
    assert_eq!(back.world.authors.starter(at), Some(&starter), "the build resumes with its starter");
    let rest = run_build(&mut back.world, at);
    assert_eq!(rest, 9 - first.len());
    let done = crate::plan::complete_build(&mut back.world, at).expect("the build completes");
    for dx in 0..3 {
        for dz in 0..3 {
            let (x, y, z) = (10 + dx, 65, 10 + dz);
            assert_eq!(back.world.author_at(x, y, z), Some(&starter), "({x},{y},{z})");
        }
    }
    let plaque = done.plaque.expect("a plaque");
    assert_eq!(back.world.author_at(plaque.0, plaque.1, plaque.2), Some(&starter), "the plaque too");
    assert!(back.world.authors.starter(at).is_none(), "a finished build keeps no starter");
}

#[test]
fn a_starter_with_no_build_behind_it_is_dropped_on_load() {
    let _g = WorldsRootGuard::new("authorship-orphan-starter");
    let mut s = GameServer::new(0, "w".into(), 42);
    s.world.authors.set_starter((1, 64, 1), Some(&AuthorKey::Local(0)));
    s.try_save().unwrap();
    let mut back = GameServer::new(0, "w".into(), 42);
    back.initial_load().unwrap();
    assert!(back.world.authors.is_empty());
}

fn commons_world() -> World {
    let mut w = World::new();
    for x in 0..20 {
        for z in 0..20 {
            w.set_block(x, 63, z, block::STONE);
        }
    }
    w.set_block(8, 63, 8, block::COMMONS_STONE);
    w
}

#[test]
fn a_plan_stamps_over_its_own_and_natural_blocks_in_a_commons_but_skips_someone_elses() {
    let at = (4, 64, 4);
    let mut w = commons_world();
    let me = AuthorKey::Local(0);
    let other = AuthorKey::Npub("npub1other".into());
    // The plan's first layer is y 65. Natural at (4,65,4); mine at (5,65,4); theirs at (6,65,4).
    w.set_block(4, 65, 4, block::DIRT);
    w.place_player_block_as(5, 65, 4, block::DIRT, Some(&me));
    w.place_player_block_as(6, 65, 4, block::DIRT, Some(&other));
    let stone = crate::item::ItemStack::new_block(block::STONE, 9);
    w.construction_anchors.insert(at, anchor_for(PlanData::debug_3x3_stone(), at, false, vec![stone]));
    w.authors.set_starter(at, Some(&me));
    let stamped = run_build(&mut w, at);
    assert_eq!(stamped, 8, "eight of nine: theirs is skipped");
    assert_eq!(w.get_block(6, 65, 4), block::DIRT, "their block stands");
    assert_eq!(w.author_at(6, 65, 4), Some(&other), "and is still theirs");
    assert_eq!(w.get_block(5, 65, 4), block::STONE, "my own was built over");
    assert_eq!(w.author_at(5, 65, 4), Some(&me));
    assert_eq!(w.get_block(4, 65, 4), block::STONE, "natural ground was built over");
    let done = crate::plan::complete_build(&mut w, at).expect("completes");
    assert_eq!(done.leftovers.iter().map(|s| u32::from(s.count)).sum::<u32>(), 1, "the skipped cell's stone comes back as a leftover");
}

#[test]
fn the_same_plan_outside_a_commons_stamps_over_anything() {
    let at = (4, 64, 4);
    let mut w = commons_world();
    w.set_block(8, 63, 8, block::STONE); // no Commons any more
    let other = AuthorKey::Npub("npub1other".into());
    w.place_player_block_as(6, 65, 4, block::DIRT, Some(&other));
    w.construction_anchors.insert(at, anchor_for(PlanData::debug_3x3_stone(), at, true, Vec::new()));
    w.authors.set_starter(at, Some(&AuthorKey::Local(0)));
    assert_eq!(run_build(&mut w, at), 9);
    assert_eq!(w.author_at(6, 65, 4), Some(&AuthorKey::Local(0)), "the stamp is the new author's");
}

#[test]
fn a_plaque_never_replaces_someone_elses_block_in_a_commons() {
    let at = (4, 64, 4);
    let mut w = commons_world();
    let other = AuthorKey::Npub("npub1other".into());
    // The plaque goes at the plan's central floor cell (5,65,5).
    w.place_player_block_as(5, 65, 5, block::DIRT, Some(&other));
    w.construction_anchors.insert(at, anchor_for(PlanData::debug_3x3_stone(), at, true, Vec::new()));
    w.authors.set_starter(at, Some(&AuthorKey::Local(0)));
    run_build(&mut w, at);
    assert_eq!(w.get_block(5, 65, 5), block::DIRT, "their block was skipped by the stamp");
    let done = crate::plan::complete_build(&mut w, at).unwrap();
    assert_eq!(w.get_block(5, 65, 5), block::DIRT, "and not replaced by the plaque");
    assert_eq!(done.plaque, Some((5, 66, 5)), "the plaque went above it");
    assert_eq!(w.get_block(5, 66, 5), block::ARCHITECT_PLAQUE);
}

#[test]
fn an_npc_commission_credits_its_commissioner() {
    use crate::builder::{BuilderCommission, CommissionStatus};
    let plan = {
        let mut p = PlanData::debug_3x3_stone();
        // Three cells in a row: the plaque takes the middle one, the ends stay stone.
        p.cells = (0..3).map(|rx| CapturedCell { rx, ry: 0, rz: 0, block_id: block::STONE }).collect();
        p.width = 3;
        p.depth = 1;
        p
    };
    for (commissioner, want) in [("local-player-0", Some(AuthorKey::Local(0))), ("local-player-2", Some(AuthorKey::Local(2))), ("npub1someone", Some(AuthorKey::Npub("npub1someone".into()))), ("", None)] {
        let mut world = World::new();
        let mut ecs = hecs::World::new();
        let workstation = [0, 64, 0];
        let anchor = [1, 64, 0];
        crate::entity::spawn_mob(&mut ecs, crate::mob::MobType::Villager, glam::Vec3::new(1.0, 64.0, 0.5));
        let id = ecs.query::<&crate::entity::MobKind>().iter().map(|(id, _)| id).last().expect("villager spawned");
        let commission = BuilderCommission {
            plan: plan.clone(),
            anchor,
            rotations: 0,
            locked_materials: Vec::new(),
            fee_sats: 0,
            commissioner_npub: commissioner.to_string(),
            village_id: None,
            workstation,
            status: CommissionStatus::PathingToSite,
            created_tick: 0,
            status_entered_tick: 0,
        };
        let vc = crate::villager::VillagerComponent {
            profession: crate::villager::Profession::Builder,
            claimed_workstation: Some(workstation),
            commission: Some(commission),
            ..Default::default()
        };
        let _ = ecs.insert_one(id, vc);
        for t in 1..=600u64 {
            crate::builder::process_builder_commissions(&mut ecs, &mut world, t, |_: &str| true);
            if !world.architect_plaques.is_empty() {
                break;
            }
        }
        assert!(!world.architect_plaques.is_empty(), "{commissioner:?}: the NPC finished the build");
        for cell in [(1, 65, 0), (3, 65, 0), (2, 65, 0)] {
            assert_eq!(world.author_at(cell.0, cell.1, cell.2).cloned(), want, "{commissioner:?} {cell:?} (the last is the plaque)");
        }
        assert_eq!(world.get_block(1, 65, 0), block::STONE);
        assert_eq!(world.get_block(2, 65, 0), block::ARCHITECT_PLAQUE);
    }
}

// ── Commons protection ──────────────────────────────────────────────

#[test]
fn a_local_seat_may_break_its_own_and_natural_blocks_in_a_commons_but_not_another_seats() {
    let mut w = commons_world();
    let (me, mate) = (AuthorKey::Local(0), AuthorKey::Local(1));
    w.place_player_block_as(5, 64, 5, block::STONE, Some(&me));
    w.place_player_block_as(6, 64, 5, block::STONE, Some(&mate));
    w.place_player_block_as(7, 64, 5, block::STONE, Some(&AuthorKey::Npub("npub1far".into())));
    w.place_player_block(9, 64, 5, block::STONE); // placed, author unknown
    w.set_block(10, 64, 5, block::STONE); // natural
    let protects = |w: &World, x: i32, who: &AuthorKey| w.commons_protects(x, 64, 5, Some(who));
    assert!(!protects(&w, 5, &me), "my own");
    assert!(protects(&w, 6, &me), "another seat's");
    assert!(protects(&w, 7, &me), "a joiner's");
    assert!(!protects(&w, 6, &mate), "and it is theirs to break");
    assert!(!protects(&w, 9, &me), "an unknown author's block is free");
    assert!(!protects(&w, 10, &me), "so is a natural one");
    assert!(w.commons_protects(6, 64, 5, None), "a guest has no claim on an authored block");
    // Not in a Commons: nobody is protected.
    let mut w = World::new();
    w.place_player_block_as(6, 64, 5, block::STONE, Some(&mate));
    assert!(!w.commons_protects(6, 64, 5, Some(&me)));
    // Outside the footprint (marker +-16) of a distant Commons, too.
    let mut w = commons_world();
    w.place_player_block_as(8 + 17, 64, 5, block::STONE, Some(&mate));
    assert!(!w.commons_protects(8 + 17, 64, 5, Some(&me)));
    w.place_player_block_as(8 + 16, 64, 5, block::STONE, Some(&mate));
    assert!(w.commons_protects(8 + 16, 64, 5, Some(&me)), "the edge of the footprint is in");
}

#[test]
fn a_joiner_cannot_break_another_players_block_in_a_commons_and_is_sent_the_block_back() {
    let mut hs = start_open_server("authorship-commons");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let pk = [0x3c; 32];
    hs.server.players[slot].verified_pubkey = Some(pk);
    let mine = npub_key(&pk);
    let theirs = AuthorKey::Npub("npub1someoneelse".into());
    let stone = cell_beside(&hs, slot, 3, 0, 3);
    hs.server.world.set_block(stone.0, stone.1, stone.2, block::COMMONS_STONE);
    let host_block = cell_beside(&hs, slot, 1, 0, 0);
    let joiner_block = cell_beside(&hs, slot, 1, 0, 1);
    let guest_block = cell_beside(&hs, slot, 1, 0, 2);
    let natural = cell_beside(&hs, slot, 2, 0, 0);
    hs.server.world.place_player_block_as(host_block.0, host_block.1, host_block.2, block::STONE, Some(&AuthorKey::Local(0)));
    hs.server.world.place_player_block_as(joiner_block.0, joiner_block.1, joiner_block.2, block::STONE, Some(&mine));
    hs.server.world.place_player_block_as(guest_block.0, guest_block.1, guest_block.2, block::STONE, None);
    hs.server.world.set_block(natural.0, natural.1, natural.2, block::STONE);
    let _ = block_changes_seen(&client);

    // The host's block: refused, and the ghost is undone.
    send_edits(&hs, &client, slot, 1, &[(host_block, block::AIR)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(host_block.0, host_block.1, host_block.2), block::STONE, "the host's block stands");
    assert_eq!(hs.server.world.author_at(host_block.0, host_block.1, host_block.2), Some(&AuthorKey::Local(0)));
    let back = block_changes_seen(&client);
    assert!(
        back.iter().any(|bc| (bc.x, bc.y, bc.z) == host_block && bc.new_block == block::STONE),
        "the refused edit is un-ghosted with the authoritative block: {back:?}"
    );
    // Building over it is a break too.
    send_edits(&hs, &client, slot, 2, &[(host_block, block::GLASS)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(host_block.0, host_block.1, host_block.2), block::STONE);

    // Their own block, a guest's (author unknown) and a natural one: allowed.
    send_edits(&hs, &client, slot, 3, &[(joiner_block, block::AIR), (guest_block, block::AIR), (natural, block::AIR)]);
    hs.tick();
    for cell in [joiner_block, guest_block, natural] {
        assert_eq!(hs.server.world.get_block(cell.0, cell.1, cell.2), block::AIR, "{cell:?}");
    }
    assert_eq!(hs.server.world.author_at(joiner_block.0, joiner_block.1, joiner_block.2), None, "a break clears the author");

    // A different joiner is refused the first joiner's block.
    let again = cell_beside(&hs, slot, 1, 0, 1);
    send_edits(&hs, &client, slot, 4, &[(again, block::STONE)]);
    hs.tick();
    assert_eq!(hs.server.world.author_at(again.0, again.1, again.2), Some(&mine), "a placement is recorded under the joiner's npub");
    hs.server.players[slot].verified_pubkey = Some([0x4d; 32]);
    send_edits(&hs, &client, slot, 5, &[(again, block::AIR)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(again.0, again.1, again.2), block::STONE, "another joiner cannot break it");
    // A guest cannot either (it has no key to match).
    hs.server.players[slot].verified_pubkey = None;
    send_edits(&hs, &client, slot, 6, &[(again, block::AIR)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(again.0, again.1, again.2), block::STONE, "nor can a guest");
}

#[test]
fn creative_bypasses_commons_protection_for_a_joiner() {
    let mut hs = start_open_server("authorship-creative");
    hs.server.set_play_mode(crate::play_mode::PlayMode::Creative);
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let stone = cell_beside(&hs, slot, 3, 0, 3);
    hs.server.world.set_block(stone.0, stone.1, stone.2, block::COMMONS_STONE);
    let theirs = cell_beside(&hs, slot, 1, 0, 0);
    hs.server.world.place_player_block_as(theirs.0, theirs.1, theirs.2, block::STONE, Some(&AuthorKey::Local(0)));
    send_edits(&hs, &client, slot, 1, &[(theirs, block::AIR)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(theirs.0, theirs.1, theirs.2), block::AIR);
}

#[test]
fn outside_a_commons_a_joiner_may_break_anyones_block() {
    let mut hs = start_open_server("authorship-no-commons");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let theirs = cell_beside(&hs, slot, 1, 0, 0);
    hs.server.world.place_player_block_as(theirs.0, theirs.1, theirs.2, block::STONE, Some(&AuthorKey::Local(0)));
    send_edits(&hs, &client, slot, 1, &[(theirs, block::AIR)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(theirs.0, theirs.1, theirs.2), block::AIR, "only a Commons protects");
}

// ── Review fixes (S1 + S2) ──────────────────────────────────────────

fn plan_of_stone_row() -> PlanData {
    let mut p = PlanData::debug_3x3_stone();
    p.cells = (0..3).map(|rx| CapturedCell { rx, ry: 0, rz: 0, block_id: block::STONE }).collect();
    p.width = 3;
    p.depth = 1;
    p
}

/// SF1 — an author entry on a cell that holds no block (a blast or a fire left
/// the placed bit set) protects nothing and counts for nothing.
#[test]
fn an_author_entry_on_air_protects_nothing_and_counts_for_nothing() {
    let mut w = commons_world();
    let (me, other) = (AuthorKey::Local(0), AuthorKey::Npub("npub1other".into()));
    // A blast: the block goes, and the entry goes with it.
    w.place_player_block_as(5, 64, 5, block::STONE, Some(&other));
    assert!(w.commons_protects(5, 64, 5, Some(&me)));
    w.set_block(5, 64, 5, block::AIR);
    assert!(w.authors.is_empty(), "a cell that becomes AIR has no builder any more");
    // A stale entry made by hand (a crash, an older save) on an empty cell whose bit is set.
    w.set_placed(6, 64, 5, true);
    w.authors.set(crate::authorship::chunk_of(6, 64, 5), crate::authorship::cell_of(6, 64, 5), Some(&other));
    w.place_player_block_as(7, 64, 5, block::STONE, Some(&AuthorKey::Npub("npub1third".into())));
    assert_eq!(w.author_at(6, 64, 5), None);
    assert!(!w.commons_protects(6, 64, 5, Some(&me)), "empty space is nobody's");
    assert_eq!(w.authors_in_region(0, 0), 1, "only the live block counts toward b");
}

/// SF1 — and a placement into such a cell overwrites the stale entry with its
/// own author (the authority path, through a real joiner).
#[test]
fn a_joiner_placing_into_a_cell_with_a_stale_entry_owns_the_new_block() {
    let mut hs = start_open_server("authorship-stale");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let pk = [0x3c; 32];
    hs.server.players[slot].verified_pubkey = Some(pk);
    let mine = npub_key(&pk);
    let stone = cell_beside(&hs, slot, 3, 0, 3);
    hs.server.world.set_block(stone.0, stone.1, stone.2, block::COMMONS_STONE);
    let cell = cell_beside(&hs, slot, 1, 0, 0);
    hs.server.world.set_placed(cell.0, cell.1, cell.2, true);
    hs.server.world.authors.set(crate::authorship::chunk_of(cell.0, cell.1, cell.2), crate::authorship::cell_of(cell.0, cell.1, cell.2), Some(&AuthorKey::Local(0)));
    let _ = block_changes_seen(&client);
    send_edits(&hs, &client, slot, 1, &[(cell, block::STONE)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(cell.0, cell.1, cell.2), block::STONE, "the empty cell was the joiner's to build in");
    assert_eq!(hs.server.world.author_at(cell.0, cell.1, cell.2), Some(&mine), "and the block is theirs, not the stale entry's");
    // A guest's placement into a stale cell clears the entry: the block is anonymous.
    let cell2 = cell_beside(&hs, slot, 1, 0, 1);
    hs.server.world.set_placed(cell2.0, cell2.1, cell2.2, true);
    hs.server.world.authors.set(crate::authorship::chunk_of(cell2.0, cell2.1, cell2.2), crate::authorship::cell_of(cell2.0, cell2.1, cell2.2), Some(&AuthorKey::Local(0)));
    hs.server.players[slot].verified_pubkey = None;
    send_edits(&hs, &client, slot, 2, &[(cell2, block::STONE)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(cell2.0, cell2.1, cell2.2), block::STONE);
    assert_eq!(hs.server.world.author_at(cell2.0, cell2.1, cell2.2), None, "a guest's block is anonymous");
}

/// SF2 — a block that falls lands anonymous. B's sand falling onto a cell
/// where A once had an entry stays B's to break (and anyone's).
#[test]
fn a_falling_block_lands_anonymous_and_its_builder_can_still_break_it() {
    let (a, b) = (AuthorKey::Npub("npub1aaa".into()), AuthorKey::Npub("npub1bbb".into()));
    let mut w = commons_world();
    // A's entry is left on the empty cell (5,64,5) with its bit set.
    w.set_placed(5, 64, 5, true);
    w.authors.set(crate::authorship::chunk_of(5, 64, 5), crate::authorship::cell_of(5, 64, 5), Some(&a));
    // B places sand above it; it falls onto the cell.
    w.place_player_block_as(5, 65, 5, block::SAND, Some(&b));
    let mut water = crate::water::WaterSystem::new();
    let reg = crate::block::BlockRegistry::new();
    let changes = crate::falling_blocks::tick_falling_blocks(&mut w, &reg, &mut water, &[glam::Vec3::new(5.0, 64.0, 5.0)], 15);
    assert!(!changes.is_empty());
    assert_eq!(w.get_block(5, 64, 5), block::SAND, "the sand landed");
    assert_eq!(w.get_block(5, 65, 5), block::AIR);
    assert!(w.is_placed(5, 64, 5), "it is still player-placed (no Satori laundering)");
    assert_eq!(w.author_at(5, 64, 5), None, "but it lands anonymous, not A's");
    assert!(!w.commons_protects(5, 64, 5, Some(&b)), "B can break it");
    assert!(!w.commons_protects(5, 64, 5, Some(&a)), "so can A: nobody's");
}

/// SF3 — the Commons Stone is anyone's to take down, as at Tier 1, even
/// though it is placed (and so authored) like any other block.
#[test]
fn the_commons_stone_itself_can_be_broken_by_anyone() {
    let (me, other) = (AuthorKey::Local(0), AuthorKey::Npub("npub1other".into()));
    let mut w = World::new();
    for x in 0..20 {
        for z in 0..20 {
            w.set_block(x, 63, z, block::STONE);
        }
    }
    w.place_player_block_as(8, 64, 8, block::COMMONS_STONE, Some(&other));
    w.place_player_block_as(9, 64, 8, block::STONE, Some(&other));
    assert_eq!(w.commons_areas().len(), 1);
    assert!(!w.commons_protects(8, 64, 8, Some(&me)), "the marker is anyone's to remove");
    assert!(!w.commons_protects(8, 64, 8, None), "a guest's too");
    assert!(w.commons_protects(9, 64, 8, Some(&me)), "while a block beside it is still protected");

    // And through the authority: a joiner takes down the host's stone.
    let mut hs = start_open_server("authorship-stone");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    hs.server.players[slot].verified_pubkey = Some([0x3c; 32]);
    let stone = cell_beside(&hs, slot, 3, 0, 3);
    hs.server.world.place_player_block_as(stone.0, stone.1, stone.2, block::COMMONS_STONE, Some(&AuthorKey::Local(0)));
    let _ = block_changes_seen(&client);
    send_edits(&hs, &client, slot, 1, &[(stone, block::AIR)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(stone.0, stone.1, stone.2), block::AIR, "a joiner removed the host's Commons");
}

/// SF7 — building over an authored non-air block is breaking it, whatever
/// replaces it: a placed water source, a snow layer. The authority refuses a
/// joiner; the pure rule the local place arm asks says the same.
#[test]
fn building_over_someone_elses_replaceable_block_is_refused_like_a_break() {
    let (me, other) = (AuthorKey::Local(1), AuthorKey::Local(0));
    let mut w = commons_world();
    w.place_player_block_as(5, 64, 5, block::WATER, Some(&other));
    w.place_player_block_as(6, 64, 5, block::SNOW_LAYER, Some(&other));
    for x in [5, 6] {
        assert!(w.commons_protects(x, 64, 5, Some(&me)), "{x}: replacing it is breaking it");
        assert!(!w.commons_protects(x, 64, 5, Some(&other)), "{x}: the builder may replace their own");
    }
    assert!(!w.commons_protects(7, 64, 5, Some(&me)), "an empty cell is free to build in");

    let mut hs = start_open_server("authorship-replace");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    hs.server.players[slot].verified_pubkey = Some([0x3c; 32]);
    let stone = cell_beside(&hs, slot, 3, 0, 3);
    hs.server.world.set_block(stone.0, stone.1, stone.2, block::COMMONS_STONE);
    let water = cell_beside(&hs, slot, 1, 0, 0);
    let snow = cell_beside(&hs, slot, 1, 0, 1);
    hs.server.world.place_player_block_as(water.0, water.1, water.2, block::WATER, Some(&AuthorKey::Local(0)));
    hs.server.world.place_player_block_as(snow.0, snow.1, snow.2, block::SNOW_LAYER, Some(&AuthorKey::Local(0)));
    let _ = block_changes_seen(&client);
    send_edits(&hs, &client, slot, 1, &[(water, block::STONE), (snow, block::STONE)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(water.0, water.1, water.2), block::WATER, "the host's source stands");
    assert_eq!(hs.server.world.get_block(snow.0, snow.1, snow.2), block::SNOW_LAYER, "and the host's snow");
    assert_eq!(hs.server.world.author_at(water.0, water.1, water.2), Some(&AuthorKey::Local(0)));
}

/// SF6 — an NPC commission that skips a cell in a Commons hands that cell's
/// material back to the commissioner: stamped + returned = what the commission held.
#[test]
fn an_npc_commission_returns_the_materials_of_cells_it_skipped() {
    use crate::builder::{BuilderCommission, CommissionStatus};
    let other = AuthorKey::Npub("npub1other".into());
    let mut world = World::new();
    world.set_block(8, 63, 8, block::COMMONS_STONE);
    // The row runs (1..=3, 65, 0); someone else built at (3,65,0).
    world.place_player_block_as(3, 65, 0, block::STONE, Some(&other));
    let mut ecs = hecs::World::new();
    let workstation = [0, 64, 0];
    let anchor = [1, 64, 0];
    crate::entity::spawn_mob(&mut ecs, crate::mob::MobType::Villager, glam::Vec3::new(1.0, 64.0, 0.5));
    let id = ecs.query::<&crate::entity::MobKind>().iter().map(|(id, _)| id).last().expect("villager spawned");
    let commission = BuilderCommission {
        plan: plan_of_stone_row(),
        anchor,
        rotations: 0,
        locked_materials: vec![(block::STONE, 3)],
        fee_sats: 0,
        commissioner_npub: "local-player-0".to_string(),
        village_id: None,
        workstation,
        status: CommissionStatus::PathingToSite,
        created_tick: 0,
        status_entered_tick: 0,
    };
    let vc = crate::villager::VillagerComponent {
        profession: crate::villager::Profession::Builder,
        claimed_workstation: Some(workstation),
        commission: Some(commission),
        ..Default::default()
    };
    let _ = ecs.insert_one(id, vc);
    let (mut stamped, mut returned) = (0u32, 0u32);
    let mut who = None;
    for t in 1..=600u64 {
        for o in crate::builder::process_builder_commissions(&mut ecs, &mut world, t, |_: &str| true) {
            stamped += o.placed.len() as u32;
            for (b, n) in &o.skipped_materials {
                assert_eq!(*b, block::STONE);
                returned += n;
                who = Some(o.commissioner_npub.clone());
            }
        }
        if !world.architect_plaques.is_empty() {
            break;
        }
    }
    assert!(!world.architect_plaques.is_empty(), "the build finished");
    assert_eq!(world.author_at(3, 65, 0), Some(&other), "their block stands");
    assert_eq!(returned, 1, "the skipped cell's stone is handed back");
    assert_eq!(who.as_deref(), Some("local-player-0"), "to the commissioner");
    assert_eq!(stamped + returned, 3, "nothing the commission held is lost or made twice");
}

/// NIT1 — when every cell the plaque could take holds someone else's block,
/// there is no plaque rather than one over their work.
#[test]
fn no_plaque_is_better_than_one_over_someone_elses_work() {
    let at = (4, 64, 4);
    let mut w = commons_world();
    let other = AuthorKey::Npub("npub1other".into());
    // The plaque goes at (5, y, 5), y from 65 up through 72.
    for y in 65..73 {
        w.place_player_block_as(5, y, 5, block::DIRT, Some(&other));
    }
    w.construction_anchors.insert(at, anchor_for(PlanData::debug_3x3_stone(), at, true, Vec::new()));
    w.authors.set_starter(at, Some(&AuthorKey::Local(0)));
    run_build(&mut w, at);
    let done = crate::plan::complete_build(&mut w, at).expect("the build completes");
    assert_eq!(done.plaque, None, "no plaque");
    assert!(w.architect_plaques.is_empty());
    for y in 65..73 {
        assert_eq!(w.get_block(5, y, 5), block::DIRT, "{y}: their column stands");
        assert_eq!(w.author_at(5, y, 5), Some(&other));
    }
}
