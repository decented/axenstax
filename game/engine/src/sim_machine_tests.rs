use super::*;
use crate::item::{ItemStack, MaterialId};

fn step_world(world: &mut World, batch: &mut MachineBatch) {
    let (mut ecs, mut water, mut lava, mut fire) = (
        hecs::World::new(),
        crate::water::WaterSystem::new(),
        crate::lava::LavaSystem::new(),
        crate::fire::FireSystem::new(),
    );
    let registry = crate::block::BlockRegistry::new();
    let mut ctx = MachineCtx {
        world,
        ecs: &mut ecs,
        water: &mut water,
        lava: &mut lava,
        fire: &mut fire,
        registry: &registry,
        tick: 4,
    };
    step(
        &mut ctx,
        MachineRules {
            raining: false,
            seed: 42,
            can_detonate: false,
            mode: crate::play_mode::PlayMode::Survival,
        },
        &LocalMachineState::default(),
        batch,
        &mut Vec::new(),
        |_, _| {},
    );
}

fn ready_furnace() -> crate::furnace::FurnaceData {
    crate::furnace::FurnaceData {
        input: Some(ItemStack::new_material(MaterialId::Copper, 1)),
        smelt_progress: 199,
        smelt_total: 200,
        fuel_ticks_remaining: 10,
        ..Default::default()
    }
}

#[test]
fn d4_machines_completion_capacity_defers_without_losing_input_or_facts() {
    let mut world = World::new();
    for x in 0..=MAX_MACHINE_FACTS as i32 {
        world.set_block(x, 64, 0, block::FURNACE);
        world.insert_furnace((x, 64, 0), ready_furnace());
    }
    let mut batch = MachineBatch::reserve(true).unwrap();
    step_world(&mut world, &mut batch);
    assert_eq!(batch.facts().len(), MAX_MACHINE_FACTS);
    assert!(batch.owned_bytes() <= MAX_MACHINE_BYTES);
    assert_eq!(
        world
            .iter_furnaces()
            .filter(|(_, f)| f.output.is_some())
            .count(),
        MAX_MACHINE_FACTS
    );
    let pending: Vec<_> = world
        .iter_furnaces()
        .filter(|(_, f)| f.input.is_some())
        .collect();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].1.smelt_progress, 199);
    assert_eq!(
        pending[0].1.fuel_ticks_remaining, 10,
        "deferred furnace has not mutated"
    );
    let mut next = MachineBatch::reserve(true).unwrap();
    step_world(&mut world, &mut next);
    assert_eq!(next.facts().len(), 1);
    assert_eq!(
        world
            .iter_furnaces()
            .filter(|(_, f)| f.output.is_some())
            .count(),
        MAX_MACHINE_FACTS + 1
    );
    assert!(world.iter_furnaces().all(|(_, f)| f.input.is_none()));
}

#[test]
fn d4_machines_full_batch_leaves_expiring_fuse_armed_then_delivers_once() {
    let mut world = World::new();
    let pos = (0, 64, 0);
    world.set_block(pos.0, pos.1, pos.2, block::BLASTING_KEG);
    let mut keg = crate::power::PowerDeviceData::new(
        crate::power::PowerDeviceKind::BlastingKeg,
        crate::meta::Facing::Up,
    );
    keg.charge = 1;
    world.insert_power_device(pos, keg);
    let mut full = MachineBatch::reserve(true).unwrap();
    full.facts
        .resize(MAX_MACHINE_FACTS, MachineFact::PistonUsed);
    step_world(&mut world, &mut full);
    assert_eq!(world.power_device_at(pos).unwrap().charge, 1);
    let mut next = MachineBatch::reserve(true).unwrap();
    step_world(&mut world, &mut next);
    assert_eq!(world.power_device_at(pos).unwrap().charge, 0);
    assert!(
        matches!(next.facts(), [MachineFact::Keg { pos: cell, detonated: false, .. }] if *cell == pos)
    );
    let mut last = MachineBatch::reserve(true).unwrap();
    step_world(&mut world, &mut last);
    assert!(last.facts().is_empty(), "spent fuse never delivers twice");
}

fn anchor(pos: Cell) -> crate::plan::ConstructionAnchorData {
    crate::plan::ConstructionAnchorData {
        plan: crate::plan::PlanData::debug_3x3_stone(),
        rotations: 0,
        anchor: pos,
        placed_index: 0,
        locked_materials: Vec::new(),
        is_creative_build: true,
        pace_divider: 1,
        pace_counter: 0,
        builder_credit: None,
    }
}

#[test]
fn d4_machines_generic_construction_leaves_commission_lifecycle_as_sole_owner() {
    let mut world = World::new();
    let pos = (0, 64, 0);
    world.set_block(0, 63, 0, block::STONE);
    let mut data = anchor(pos);
    data.builder_credit = Some(crate::builder::BuilderCredit {
        villager_name: "Builder".into(),
        village_name: None,
        commissioner_npub: "local-player-0".into(),
    });
    world.construction_anchors.insert(pos, data);
    let mut batch = MachineBatch::reserve(true).unwrap();
    step_world(&mut world, &mut batch);
    let saved = &world.construction_anchors[&pos];
    assert_eq!((saved.placed_index, saved.pace_counter), (0, 0));
    assert!(world.architect_plaques.is_empty());
}

#[test]
fn d4_machines_construction_waits_for_all_targets_before_pace_or_material_writes() {
    let mut world = World::new();
    let pos = (15, 64, 0);
    world.set_block(15, 63, 0, block::STONE);
    world.construction_anchors.insert(pos, anchor(pos));
    let mut batch = MachineBatch::reserve(true).unwrap();
    step_world(&mut world, &mut batch);
    assert_eq!(world.construction_anchors[&pos].pace_counter, 0);
    assert_eq!(world.construction_anchors[&pos].placed_index, 0);
    assert!(!world.is_column_present_at(16, 0));
    world.set_block(16, 63, 0, block::STONE);
    step_world(&mut world, &mut batch);
    assert!(
        world
            .construction_anchors
            .get(&pos)
            .is_none_or(|data| data.placed_index > 0)
    );
}

#[test]
fn d4_machines_malformed_restored_anchor_does_not_write_outside_declared_dimensions() {
    let mut world = World::new();
    let pos = (0, 64, 0);
    world.set_block(0, 63, 0, block::STONE);
    let mut data = anchor(pos);
    data.plan.width = 0;
    world.construction_anchors.insert(pos, data);
    let mut batch = MachineBatch::reserve(true).unwrap();
    step_world(&mut world, &mut batch);
    assert_eq!(world.construction_anchors[&pos].pace_counter, 0);
    assert!(world.architect_plaques.is_empty());
}

#[test]
fn d4_machines_cosmetic_build_name_and_owned_allocations_are_bounded() {
    let mut batch = MachineBatch::reserve(true).unwrap();
    batch.build_finished(&"🦎".repeat(1000));
    assert!(batch.building_completed);
    assert_eq!(batch.build_name.len(), BUILD_NAME_BYTES);
    assert!(batch.owned_bytes() <= MAX_MACHINE_BYTES);
    let large = Vec::with_capacity(MAX_MACHINE_BYTES / std::mem::size_of::<MachineFact>() + 1);
    batch.facts = large;
    assert!(
        batch.owned_bytes() > MAX_MACHINE_BYTES,
        "capacity is charged even with length zero"
    );
}
