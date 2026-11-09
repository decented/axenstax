//! Living world Tier 2 S4: how often each Plan has been built in this world.
//!
//! The completion half drives the real authority: a dedicated `TestHost`
//! through `sim_machines::tick_construction`, a `HostedServer` with a joiner
//! connected, and the NPC commission lifecycle through
//! `builder::process_builder_commissions`. The save half runs real servers over
//! a real folder. The plaque wording is tested where it lives
//! (`plan_ui::tests`).
use super::joiner_authority::{cell_beside, join_guest};
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::block;
use crate::builder::{self, BuilderCommission, CommissionStatus};
use crate::plan::{content_hash, ConstructionAnchorData, DerivationLink, PlanData, PlanLicense};
use crate::plan_builds::{PlanCounts, PlanHash, SavedPlanBuilds};
use crate::save::{read_world_save, world_dir, WorldsRootGuard};
use crate::server::GameServer;
use crate::test_harness::{TestConfig, TestHost};
use crate::villager::{Profession, VillagerComponent};
use crate::world::World;

const Y: i32 = 64;

fn saved_world_save(world: &str) -> crate::save::WorldSave {
    read_world_save(&std::fs::read(world_dir(world).join("world.dat")).unwrap()).unwrap()
}

fn link(name: &str, hash: PlanHash) -> DerivationLink {
    DerivationLink {
        author_npub: String::new(),
        plan_name: name.to_string(),
        license: PlanLicense::default(),
        captured_at: 0,
        plan_hash: hash,
    }
}

/// A plan derived (twice over, with a repeat) from two ancestors; returns it and
/// its own hash.
fn derived_plan() -> (PlanData, PlanHash) {
    let mut plan = PlanData::debug_3x3_stone();
    let own = content_hash(&plan);
    plan.derivation_chain = vec![link("old", [1; 32]), link("older", [2; 32]), link("old again", [1; 32]), link("me", own)];
    (plan, own)
}

fn anchor_for(plan: PlanData, at: (i32, i32, i32)) -> ConstructionAnchorData {
    ConstructionAnchorData {
        plan,
        rotations: 0,
        anchor: at,
        placed_index: 0,
        locked_materials: Vec::new(),
        is_creative_build: true,
        pace_divider: 1,
        pace_counter: 0,
        builder_credit: None,
    }
}

// ── Completion ──────────────────────────────────────────────────────

#[test]
fn a_player_build_completing_on_the_authority_counts_its_plan_and_each_ancestor_once() {
    let mut h = TestHost::start_with(TestConfig::default());
    h.server.simulates_block_machines = true;
    let anchor = (6, Y, 6);
    for x in anchor.0..anchor.0 + 3 {
        for z in anchor.2..anchor.2 + 3 {
            h.set_block(x, Y, z, block::STONE);
        }
    }
    let (plan, own) = derived_plan();
    h.server.world.construction_anchors.insert(anchor, anchor_for(plan.clone(), anchor));
    assert!(h.server.world.plan_builds.is_empty());
    h.tick(24);
    assert!(!h.server.world.construction_anchors.contains_key(&anchor), "the authority completes the build");
    let t = &h.server.world.plan_builds;
    assert_eq!(t.counts(&own), PlanCounts { built: 1, built_from: 0 });
    assert_eq!(t.counts(&[1; 32]), PlanCounts { built: 0, built_from: 1 }, "a repeat in the chain counts once");
    assert_eq!(t.counts(&[2; 32]), PlanCounts { built: 0, built_from: 1 });
    assert_eq!(t.ideas_total(), (1, 1));
    // The same design again, elsewhere: the counts add.
    let again = (20, Y, 6);
    for x in again.0..again.0 + 3 {
        for z in again.2..again.2 + 3 {
            h.set_block(x, Y, z, block::STONE);
        }
    }
    h.server.world.construction_anchors.insert(again, anchor_for(plan, again));
    h.tick(24);
    assert_eq!(h.server.world.plan_builds.counts(&own).built, 2);
    assert_eq!(h.server.world.plan_builds.counts(&[1; 32]).built_from, 2);
}

/// Run a creative build of `plan` to completion on a fresh dedicated host and
/// return the host.
fn complete_on_host(plan: PlanData) -> TestHost {
    let mut h = TestHost::start_with(TestConfig::default());
    h.server.simulates_block_machines = true;
    let anchor = (6, Y, 6);
    for x in anchor.0..anchor.0 + 3 {
        for z in anchor.2..anchor.2 + 3 {
            h.set_block(x, Y, z, block::STONE);
        }
    }
    h.server.world.construction_anchors.insert(anchor, anchor_for(plan, anchor));
    h.tick(24);
    assert!(!h.server.world.construction_anchors.contains_key(&anchor), "the build completed");
    h
}

#[test]
fn a_licence_copy_counts_as_a_build_of_its_design_under_the_key_its_plaque_reads() {
    // A master, and the copy a licence sale hands over: same cells and chain, but
    // `is_master` cleared, which the recomputed content hash covers.
    let mut master = PlanData::debug_3x3_stone();
    master.is_master = true;
    let m = content_hash(&master);
    master.derivation_chain = vec![link("Tower", m)];
    let mut copy = master.clone();
    copy.is_master = false;
    assert_ne!(content_hash(&copy), m, "the premise: a copy's recomputed hash is not the master's");

    let h = complete_on_host(copy);
    let t = &h.server.world.plan_builds;
    let plaque = h.server.world.architect_plaques.values().next().expect("a plaque was placed");
    let key = plaque.chain.last().expect("a chain").plan_hash;
    assert_eq!(key, m);
    // What the plaque reads: built once, and nothing was built from it.
    let counts = t.counts(&key);
    assert_eq!(counts, PlanCounts { built: 1, built_from: 0 }, "the master is not credited as an ancestor of its own copy");
    let lines = crate::plan_ui::plaque_lines(&plaque.chain, Some(counts));
    assert!(lines.contains(&"Built 1 time in this world.".to_string()), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("built from")), "{lines:?}");
    // No stray row under the recomputed hash.
    assert_eq!(t.ideas_total(), (1, 1));
}

#[test]
fn a_derivative_of_a_licence_copy_still_credits_the_master_once() {
    let mut master = PlanData::debug_3x3_stone();
    let m = content_hash(&master);
    master.derivation_chain = vec![link("Tower", m)];
    // A derivative: its own captured hash D after the master's link.
    let mut derived = master.clone();
    derived.is_master = false;
    derived.derivation_chain.push(link("Tower II", [9; 32]));
    let h = complete_on_host(derived);
    let t = &h.server.world.plan_builds;
    assert_eq!(t.counts(&[9; 32]), PlanCounts { built: 1, built_from: 0 });
    assert_eq!(t.counts(&m), PlanCounts { built: 0, built_from: 1 });
}

#[test]
fn a_plan_with_no_chain_is_counted_under_its_recomputed_hash() {
    let mut plan = PlanData::debug_3x3_stone();
    plan.derivation_chain.clear();
    let own = content_hash(&plan);
    let h = complete_on_host(plan);
    assert_eq!(h.server.world.plan_builds.counts(&own), PlanCounts { built: 1, built_from: 0 });
    assert_eq!(h.server.world.plan_builds.ideas_total(), (1, 1));
}

#[test]
fn a_build_still_going_counts_nothing_until_it_completes() {
    let mut h = TestHost::start_with(TestConfig::default());
    h.server.simulates_block_machines = true;
    let anchor = (6, Y, 6);
    for x in anchor.0..anchor.0 + 3 {
        for z in anchor.2..anchor.2 + 3 {
            h.set_block(x, Y, z, block::STONE);
        }
    }
    h.server.world.construction_anchors.insert(anchor, anchor_for(derived_plan().0, anchor));
    h.tick(5);
    assert!(h.server.world.construction_anchors.contains_key(&anchor), "mid-build");
    assert!(h.server.world.plan_builds.is_empty(), "nothing counted yet");
    // Taking the anchor away (a refund, a cancel) is not a completion.
    h.server.world.construction_anchors.remove(&anchor);
    h.tick(24);
    assert!(h.server.world.plan_builds.is_empty());
}

#[test]
fn a_build_completing_while_a_joiner_is_connected_counts_and_the_joiner_is_told_only_of_the_blocks() {
    // A dedicated server (no local seat): its server runs the block machines.
    let mut hs = HostedServer::start(0, format!("plan-builds-{}", std::process::id()), 42, 0, RemoteTransport::WebSocket { port: 0 })
        .expect("the dedicated server starts");
    hs.server.difficulty = crate::survival::Difficulty::Peaceful;
    assert!(hs.server.simulates_block_machines);
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let at = cell_beside(&hs, slot, 4, -1, 4);
    let (plan, own) = derived_plan();
    hs.server.world.construction_anchors.insert(at, anchor_for(plan, at));
    let _ = super::joiner_authority::block_changes_seen(&client);
    for _ in 0..80 {
        hs.tick();
        if !hs.server.world.construction_anchors.contains_key(&at) {
            break;
        }
    }
    assert!(!hs.server.world.construction_anchors.contains_key(&at), "the build completed on the server ({:?}, machines {})", hs.server.world.construction_anchors.get(&at).map(|d| (d.placed_index, d.pace_counter)), hs.server.simulates_block_machines);
    assert_eq!(hs.server.world.plan_builds.counts(&own).built, 1);
    assert_eq!(hs.server.world.plan_builds.counts(&[1; 32]).built_from, 1);
    // The joiner hears the plaque like any block; the table itself is not on the wire.
    let plaque = (at.0 + 1, at.1 + 1, at.2 + 1);
    let seen = super::joiner_authority::block_changes_seen(&client);
    assert!(
        seen.iter().any(|bc| (bc.x, bc.y, bc.z) == plaque && bc.new_block == block::ARCHITECT_PLAQUE),
        "the joiner is sent the plaque block: {seen:?}"
    );
}

fn commission(plan: PlanData, anchor: [i32; 3], workstation: [i32; 3]) -> BuilderCommission {
    BuilderCommission {
        plan,
        anchor,
        rotations: 0,
        locked_materials: Vec::new(),
        fee_sats: 0,
        commissioner_npub: "local-player-0".to_string(),
        village_id: None,
        workstation,
        status: CommissionStatus::PathingToSite,
        created_tick: 0,
        status_entered_tick: 0,
    }
}

fn spawn_commissioned(ecs: &mut hecs::World, plan: PlanData, anchor: [i32; 3], workstation: [i32; 3], at: glam::Vec3) -> hecs::Entity {
    crate::entity::spawn_mob(ecs, crate::mob::MobType::Villager, at);
    let id = ecs.query::<&crate::entity::MobKind>().iter().map(|(id, _)| id).last().expect("villager spawned");
    let vc = VillagerComponent {
        profession: Profession::Builder,
        claimed_workstation: Some(workstation),
        commission: Some(commission(plan, anchor, workstation)),
        ..Default::default()
    };
    let _ = ecs.insert_one(id, vc);
    id
}

#[test]
fn an_npc_commission_completing_counts_its_plan_and_a_refunded_one_counts_nothing() {
    let (mut plan, _) = derived_plan();
    // Three cells in a row keeps the commission short; the hash is of what it builds.
    plan.cells = (0..3).map(|rx| crate::plan::CapturedCell { rx, ry: 0, rz: 0, block_id: block::STONE }).collect();
    plan.width = 3;
    plan.depth = 1;
    plan.height = 1;
    let own = content_hash(&plan);
    plan.derivation_chain = vec![link("old", [1; 32]), link("me", own)];

    // Completed.
    let mut world = World::new();
    let mut ecs = hecs::World::new();
    spawn_commissioned(&mut ecs, plan.clone(), [1, 64, 0], [0, 64, 0], glam::Vec3::new(1.0, 64.0, 0.5));
    for t in 1..=600u64 {
        builder::process_builder_commissions(&mut ecs, &mut world, t, |_: &str| true);
        if !world.architect_plaques.is_empty() {
            break;
        }
    }
    assert!(!world.architect_plaques.is_empty(), "the NPC finished the build");
    assert_eq!(world.plan_builds.counts(&own), PlanCounts { built: 1, built_from: 0 });
    assert_eq!(world.plan_builds.counts(&[1; 32]), PlanCounts { built: 0, built_from: 1 });

    // Refunded: a site no one can reach in time.
    let mut world = World::new();
    let mut ecs = hecs::World::new();
    let id = spawn_commissioned(&mut ecs, plan, [10_000, 64, 10_000], [0, 64, 0], glam::Vec3::new(0.5, 64.0, 0.5));
    let mut refunded = false;
    for t in 1..=2000u64 {
        for o in builder::process_builder_commissions(&mut ecs, &mut world, t, |_: &str| true) {
            refunded |= matches!(o.action, builder::CommissionTickAction::Refund(_));
        }
        if ecs.get::<&VillagerComponent>(id).map(|vc| vc.commission.is_none()).unwrap_or(true) {
            break;
        }
    }
    assert!(refunded, "the commission was refunded");
    assert!(world.plan_builds.is_empty(), "a refunded build counts nothing");
}

// ── Save and load ───────────────────────────────────────────────────

#[test]
fn the_build_counts_survive_a_save_and_load() {
    let _g = WorldsRootGuard::new("plan-builds-roundtrip");
    let mut s = GameServer::new(0, "w".into(), 42);
    s.world.plan_builds.note_completed([7; 32], &[[1; 32], [2; 32]]);
    s.world.plan_builds.note_completed([7; 32], &[[1; 32]]);
    s.try_save().unwrap();
    let saved = saved_world_save("w").plan_builds.expect("the field is written");
    assert_eq!(saved.rows, vec![([1; 32], 0, 2), ([2; 32], 0, 1), ([7; 32], 2, 0)]);
    let mut back = GameServer::new(0, "w".into(), 42);
    back.initial_load().unwrap();
    assert_eq!(back.world.plan_builds, s.world.plan_builds);
    // It is live: another build adds to it and a second save carries it.
    back.world.plan_builds.note_completed([7; 32], &[]);
    back.try_save().unwrap();
    assert_eq!(saved_world_save("w").plan_builds.unwrap().rows[2], ([7; 32], 3, 0));
}

#[test]
fn a_world_nothing_was_built_in_saves_the_field_absent() {
    let _g = WorldsRootGuard::new("plan-builds-absent");
    let s = GameServer::new(0, "w".into(), 42);
    s.try_save().unwrap();
    assert!(saved_world_save("w").plan_builds.is_none());
}

#[test]
fn the_saved_bytes_do_not_depend_on_the_order_builds_completed_in() {
    let bytes_for = |name: &str, order: &[u8]| {
        let _g = WorldsRootGuard::new(name);
        let mut s = GameServer::new(0, "w".into(), 42);
        for &i in order {
            s.world.plan_builds.note_completed([i; 32], &[[i.wrapping_add(50); 32]]);
        }
        s.try_save().unwrap();
        bincode::serialize(&saved_world_save("w").plan_builds.unwrap()).unwrap()
    };
    assert_eq!(bytes_for("plan-builds-order-a", &[9, 3, 200, 1]), bytes_for("plan-builds-order-b", &[1, 200, 3, 9]));
}

#[test]
fn a_footer_64_save_loads_with_an_empty_table() {
    // A footer-64 writer (Tier 2 S3) had 58 fields: the same bytes without the new last one.
    let mut save = crate::save::minimal_world_save_for_tests(7);
    save.plan_builds = None;
    save.plot_succession = None;
    save.site_boards = None;
    let mut body = bincode::serialize(&save).unwrap();
    assert_eq!(body.pop(), Some(0), "the absent site boards are one None byte");
    assert_eq!(body.pop(), Some(0), "the absent succession table is one None byte");
    assert_eq!(body.pop(), Some(0), "the absent field is one None byte");
    body.extend_from_slice(&crate::save_format::footer_bytes(64));
    let (old, damaged) = crate::save::read_world_save_reporting(&body).expect("an older save loads");
    assert!(!damaged);
    assert!(old.plan_builds.is_none());
    let mut w = World::new();
    w.plan_builds.note_completed([1; 32], &[]);
    crate::save::apply_world_save_state(&mut w, &old);
    assert!(w.plan_builds.is_empty(), "restoring a save without the field leaves nothing counted");
}

#[test]
fn an_invalid_row_is_dropped_with_a_warning_never_a_refusal_to_load() {
    let mut save = crate::save::minimal_world_save_for_tests(8);
    save.plan_builds = Some(SavedPlanBuilds { rows: vec![([1; 32], 2, 0), ([1; 32], 9, 9), ([2; 32], 0, 0)] });
    let bytes = crate::save_format::encode_world_save(&save).unwrap();
    let (back, damaged) = crate::save::read_world_save_reporting(&bytes).expect("the world still loads");
    assert!(!damaged);
    let mut w = World::new();
    crate::save::apply_world_save_state(&mut w, &back);
    assert_eq!(w.plan_builds.counts(&[1; 32]), PlanCounts { built: 2, built_from: 0 });
    assert_eq!(w.plan_builds.counts(&[2; 32]), PlanCounts::default());
}

#[test]
fn the_server_digest_covers_the_build_counts_in_a_canonical_order() {
    let digest = |name: &str, order: &[u8]| {
        let _g = WorldsRootGuard::new(name);
        let mut s = GameServer::new(0, "w".into(), 42);
        for &i in order {
            s.world.plan_builds.note_completed([i; 32], &[]);
        }
        s.sim_digest()
    };
    let one = digest("plan-builds-digest-1", &[10, 100]);
    assert_eq!(one, digest("plan-builds-digest-2", &[100, 10]));
    assert_ne!(one, digest("plan-builds-digest-3", &[10]), "what has been built is part of the digest");
}
