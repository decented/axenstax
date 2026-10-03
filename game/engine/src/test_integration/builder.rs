//! Spec 26 — NPC Builder commission flow integration tests.
//!
//! Drives `builder::process_builder_commissions` end-to-end against a
//! synthetic World + ECS: spawn a Builder villager, attach a
//! commission, tick the lifecycle PathingToSite → Building → Returning
//! → Done, and verify the plaque + village treasury + status reach the
//! expected end-state.

use ahash::AHashMap;

use crate::block;
use crate::builder::{
    self, BuilderCommission, CommissionStatus, FailureReason,
};
use crate::economy::ServerSatsPolicy;
use crate::entity::{Position, spawn_mob};
use crate::mob::MobType;
use crate::plan::{CapturedCell, PlanData, PlanLicense};
use crate::villager::{Profession, VillagerComponent};
use crate::world::World;

fn fixture_plan(cells: Vec<CapturedCell>) -> PlanData {
    let (max_x, max_y, max_z) = cells.iter().fold((0u8, 0u8, 0u8), |acc, c| {
        (acc.0.max(c.rx), acc.1.max(c.ry), acc.2.max(c.rz))
    });
    PlanData {
        version: 1,
        name: "test cottage".into(),
        author_npub: String::new(),
        license: PlanLicense::Ccbysa,
        derivation_chain: Vec::new(),
        is_master: true,
        width: max_x + 1,
        depth: max_z + 1,
        height: max_y + 1,
        cells,
        authored_in: "survival".into(),
        develop_state: crate::plan::DevelopState::Developed,
            kind: crate::plan::PlanKind::Building,
    }
}

fn cell(rx: u8, ry: u8, rz: u8, b: u16) -> CapturedCell {
    CapturedCell { rx, ry, rz, block_id: b }
}

fn spawn_builder_at(
    ecs: &mut hecs::World,
    pos: glam::Vec3,
    workstation: [i32; 3],
) -> hecs::Entity {
    spawn_mob(ecs, MobType::Villager, pos);
    // The most-recently-spawned villager is the highest id.
    let id = ecs
        .query::<&crate::entity::MobKind>()
        .iter()
        .map(|(id, _)| id)
        .last()
        .expect("villager spawned");
    let vc = VillagerComponent {
        profession: Profession::Builder,
        claimed_workstation: Some(workstation),
        ..Default::default()
    };
    let _ = ecs.insert_one(id, vc);
    id
}

fn make_commission(
    plan: PlanData,
    anchor: [i32; 3],
    workstation: [i32; 3],
) -> BuilderCommission {
    BuilderCommission {
        plan,
        anchor,
        rotations: 0,
        locked_materials: vec![(block::STONE, 2)],
        fee_sats: 160,
        commissioner_npub: "local-player-0".into(),
        village_id: Some((0, 0)),
        workstation,
        status: CommissionStatus::PathingToSite,
        created_tick: 0,
        status_entered_tick: 0,
    }
}

#[test]
fn full_commission_cycle_walks_through_to_done() {
    // End-to-end: NPC at workstation, commission posted, tick repeatedly
    // until the villager reaches the site, builds, returns, and the
    // commission clears.
    let mut world = World::new();
    let mut ecs = hecs::World::new();
    // Solid floor under the anchor + worktation so block placement works.
    for x in -2..=12 {
        for z in -2..=12 {
            world.set_block(x, 63, z, block::STONE);
        }
    }

    // Spawn the Builder at the workstation.
    let workstation = [0, 64, 0];
    let anchor = [10, 64, 10];
    let builder = spawn_builder_at(
        &mut ecs,
        glam::Vec3::new(0.5, 64.0, 0.5),
        workstation,
    );

    // 2-cell plan.
    let plan = fixture_plan(vec![
        cell(0, 0, 0, block::STONE),
        cell(1, 0, 0, block::STONE),
    ]);
    let commission = make_commission(plan, anchor, workstation);

    // Attach commission.
    {
        let mut vc = ecs.get::<&mut VillagerComponent>(builder).unwrap();
        vc.commission = Some(commission);
    }

    // Tick until the lifecycle reaches Done.
    let mut current_tick: u64 = 1;
    let mut saw_plaque = false;
    let online = |_npub: &str| true;
    for _ in 0..4000 {
        let outcomes = builder::process_builder_commissions(
            &mut ecs, &mut world, current_tick, online,
        );
        for o in outcomes {
            if o.plaque.is_some() {
                saw_plaque = true;
            }
        }
        let cleared = ecs
            .get::<&VillagerComponent>(builder)
            .map(|vc| vc.commission.is_none())
            .unwrap_or(true);
        if cleared {
            break;
        }
        current_tick += 1;
    }

    let vc = ecs.get::<&VillagerComponent>(builder).unwrap();
    assert!(vc.commission.is_none(), "commission should be cleared at Done");
    assert!(saw_plaque, "plaque should have been placed during the build");

    // Plaque is in the world's architect_plaques map.
    assert!(
        !world.architect_plaques.is_empty(),
        "world.architect_plaques should hold the dropped plaque",
    );
    // The plaque should carry the BuilderCredit.
    let (_, plaque_data) = world.architect_plaques.iter().next().unwrap();
    assert!(plaque_data.builder_credit.is_some(),
        "plaque should carry BuilderCredit for the NPC commission");
}

#[test]
fn pathfind_timeout_emits_refund_and_clears_commission() {
    // Spawn the Builder *far* from the anchor and don't let them move
    // (zero step toward — but our step_toward still moves them). To
    // guarantee timeout, set the anchor at a position the villager
    // can't reach in the timeout budget. Simpler: skip movement by
    // pinning the position each tick.
    let mut world = World::new();
    let mut ecs = hecs::World::new();
    let workstation = [0, 64, 0];
    let anchor = [10_000, 64, 10_000]; // very far
    let builder = spawn_builder_at(
        &mut ecs,
        glam::Vec3::new(0.5, 64.0, 0.5),
        workstation,
    );
    let plan = fixture_plan(vec![cell(0, 0, 0, block::STONE)]);
    {
        let mut vc = ecs.get::<&mut VillagerComponent>(builder).unwrap();
        vc.commission = Some(make_commission(plan, anchor, workstation));
    }
    // Tick past the timeout. Even with step_toward nudging the
    // villager 0.15 m/tick over PATH_TIMEOUT_TICKS = 1200 ticks they
    // cover at most 180 m, far short of 10 km.
    let mut current_tick: u64 = 1;
    let mut saw_refund = false;
    let online = |_: &str| true;
    for _ in 0..2000 {
        let outcomes = builder::process_builder_commissions(
            &mut ecs, &mut world, current_tick, online,
        );
        for o in outcomes {
            if matches!(o.action, builder::CommissionTickAction::Refund(_)) {
                saw_refund = true;
            }
        }
        if ecs
            .get::<&VillagerComponent>(builder)
            .map(|vc| vc.commission.is_none())
            .unwrap_or(true)
        {
            break;
        }
        current_tick += 1;
    }
    assert!(saw_refund, "PathTimeoutToSite should emit a Refund outcome");
}

#[test]
fn settle_commission_credits_treasury_to_correct_village() {
    // The settle helper is unit-tested; this is the integration check
    // that calling it against a freshly-created World mutates the
    // village_treasuries map in place.
    let mut treasuries: AHashMap<(i32, i32), u64> = AHashMap::new();
    treasuries.insert((0, 0), 50);
    let policy = ServerSatsPolicy::bitcoin_enabled_policy();
    let r = builder::settle_commission_to_treasury(
        &mut treasuries,
        Some((0, 0)),
        100,
        &policy,
        true,
    );
    assert_eq!(r.credited, 100);
    assert_eq!(treasuries.get(&(0, 0)).copied(), Some(150));
}

#[test]
fn offline_commissioner_pauses_build_progress() {
    // While the commissioner is offline, tick_build is skipped — the
    // anchor's placed_index doesn't advance.
    let mut world = World::new();
    let mut ecs = hecs::World::new();
    for x in -2..=12 {
        for z in -2..=12 {
            world.set_block(x, 63, z, block::STONE);
        }
    }
    let workstation = [0, 64, 0];
    let anchor = [1, 64, 0];
    let builder = spawn_builder_at(
        &mut ecs,
        glam::Vec3::new(1.0, 64.0, 0.5), // already at the anchor
        workstation,
    );
    let plan = fixture_plan(vec![
        cell(0, 0, 0, block::STONE),
        cell(1, 0, 0, block::STONE),
    ]);
    let mut c = make_commission(plan, anchor, workstation);
    c.status = CommissionStatus::Building { progress: 0 };
    // Seed the construction anchor too — the builder.rs StartBuilding
    // path normally does this, but we're skipping straight to Building.
    world.construction_anchors.insert(
        (anchor[0], anchor[1], anchor[2]),
        crate::plan::ConstructionAnchorData {
            plan: c.plan.clone(),
            rotations: 0,
            anchor: (anchor[0], anchor[1], anchor[2]),
            placed_index: 0,
            locked_materials: Vec::new(),
            is_creative_build: true,
            pace_divider: 2,
            pace_counter: 0,
            builder_credit: None,
        },
    );
    {
        let mut vc = ecs.get::<&mut VillagerComponent>(builder).unwrap();
        vc.commission = Some(c);
    }
    // Tick 10 times while offline — no progress.
    let online_false = |_: &str| false;
    for t in 1..=10u64 {
        builder::process_builder_commissions(&mut ecs, &mut world, t, online_false);
    }
    let anchor_data = world.construction_anchors.get(&(1, 64, 0)).unwrap();
    assert_eq!(anchor_data.placed_index, 0, "offline build should not advance");

    // Now flip online → build advances.
    let online_true = |_: &str| true;
    for t in 11..=60u64 {
        builder::process_builder_commissions(&mut ecs, &mut world, t, online_true);
    }
    // After online ticks, the build should complete (2 cells × pace 2
    // × 2 cells per build tick = within budget).
    let still = world.construction_anchors.get(&(1, 64, 0));
    // build_complete removes the anchor when done.
    assert!(still.is_none() || still.unwrap().placed_index > 0,
        "online build should advance");
}

#[test]
fn cancel_refund_returns_half_fee_and_remaining_materials() {
    let plan = fixture_plan(vec![
        cell(0, 0, 0, block::STONE),
        cell(1, 0, 0, block::STONE),
    ]);
    let c = BuilderCommission {
        plan,
        anchor: [0, 64, 0],
        rotations: 0,
        locked_materials: vec![(block::STONE, 2)],
        fee_sats: 200,
        commissioner_npub: "local-player-0".into(),
        village_id: Some((0, 0)),
        workstation: [0, 64, 0],
        status: CommissionStatus::Building { progress: 1 },
        created_tick: 0,
        status_entered_tick: 0,
    };
    let intent = builder::cancel_commission_refund(&c, 1);
    assert_eq!(intent.reason, FailureReason::Cancelled);
    assert_eq!(intent.fee_sats, 100); // 50% of 200
    // Materials returned: 1 cell remaining (2 cells - 1 placed) → 1 stone.
    let stone = intent
        .materials
        .iter()
        .find(|(id, _)| *id == block::STONE)
        .map(|(_, c)| *c)
        .unwrap_or(0);
    assert_eq!(stone, 1);
}

#[test]
fn villager_position_advances_toward_anchor_each_tick() {
    let mut world = World::new();
    let mut ecs = hecs::World::new();
    let workstation = [0, 64, 0];
    let anchor = [5, 64, 5];
    let builder = spawn_builder_at(
        &mut ecs,
        glam::Vec3::new(0.5, 64.0, 0.5),
        workstation,
    );
    let plan = fixture_plan(vec![cell(0, 0, 0, block::STONE)]);
    {
        let mut vc = ecs.get::<&mut VillagerComponent>(builder).unwrap();
        vc.commission = Some(make_commission(plan, anchor, workstation));
    }
    let start_pos = ecs.get::<&Position>(builder).unwrap().0;
    let online = |_: &str| true;
    for t in 1..=10u64 {
        builder::process_builder_commissions(&mut ecs, &mut world, t, online);
    }
    let end_pos = ecs.get::<&Position>(builder).unwrap().0;
    let target = glam::Vec3::new(5.5, 64.0, 5.5);
    let start_dist = (target - start_pos).length();
    let end_dist = (target - end_pos).length();
    assert!(end_dist < start_dist,
        "villager should move toward the anchor (start_dist={start_dist}, end_dist={end_dist})");
}
