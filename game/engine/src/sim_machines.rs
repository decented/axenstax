//! Shared four-tick machine rules. Outcomes are already applied to the world;
//! the bounded local batch carries policy and presentation facts, never items.
use crate::{block, block_machines::MachineCtx, protocol::BlockChange, world::World};
use glam::Vec3;

pub type Cell = (i32, i32, i32);
pub const MAX_MACHINE_FACTS: usize = 128;
pub const MAX_MACHINE_BYTES: usize = 16 * 1024;
const MAX_VISUALS: usize = 8;
const BUILD_NAME_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Default)]
pub struct LocalMachineState {
    pub positions: [Option<Vec3>; 4],
    pub chests: [Option<Cell>; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct MachineRules {
    pub raining: bool,
    pub seed: u32,
    pub can_detonate: bool,
    pub mode: crate::play_mode::PlayMode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MachineFact {
    FurnaceCompleted(Cell),
    PistonUsed,
    /// Damage was calculated against each blast's settled world. The client
    /// applies these hits in order through its existing armour/i-frame rules.
    Keg {
        pos: Cell,
        hits: [f32; 4],
        close_chests: u8,
        detonated: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ejection {
    pub pos: Cell,
    pub spawn: Vec3,
    pub dir: Vec3,
    pub effect: crate::dispenser::EjectEffect,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MachineBatch {
    deliver_local: bool,
    facts: Vec<MachineFact>,
    pub ejections: Vec<Ejection>,
    pub saplings: Vec<Cell>,
    pub rack_ready: bool,
    pub building_completed: bool,
    pub build_name: String,
}

impl MachineBatch {
    pub fn reserve(deliver_local: bool) -> Option<Self> {
        let mut batch = Self {
            deliver_local,
            ..Self::default()
        };
        if deliver_local {
            batch.facts.try_reserve_exact(MAX_MACHINE_FACTS).ok()?;
            batch.ejections.try_reserve_exact(MAX_VISUALS).ok()?;
            batch.saplings.try_reserve_exact(MAX_VISUALS).ok()?;
            batch.build_name.try_reserve_exact(BUILD_NAME_BYTES).ok()?;
        }
        (batch.owned_bytes() <= MAX_MACHINE_BYTES).then_some(batch)
    }

    pub fn owned_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.facts.capacity() * std::mem::size_of::<MachineFact>()
            + self.ejections.capacity() * std::mem::size_of::<Ejection>()
            + self.saplings.capacity() * std::mem::size_of::<Cell>()
            + self.build_name.capacity()
    }

    pub fn facts(&self) -> &[MachineFact] {
        &self.facts
    }
    fn room(&self) -> bool {
        !self.deliver_local || self.facts.len() < MAX_MACHINE_FACTS
    }
    fn remaining(&self) -> usize {
        if self.deliver_local {
            MAX_MACHINE_FACTS - self.facts.len()
        } else {
            usize::MAX
        }
    }
    fn record(&mut self, fact: MachineFact) {
        if self.deliver_local {
            assert!(self.room(), "reserve machine delivery before mutation");
            self.facts.push(fact);
        }
    }
    fn build_finished(&mut self, name: &str) {
        if !self.deliver_local {
            return;
        }
        self.building_completed = true;
        self.build_name.clear();
        for ch in name.chars() {
            if self.build_name.len() + ch.len_utf8() > BUILD_NAME_BYTES {
                break;
            }
            self.build_name.push(ch);
        }
    }
}

/// One machine phase, in the established growth→eject→piston→workstation→
/// campfire→furnace→composter→keg→rack→manual-construction order. The caller
/// owns the cadence. `on_blast` lands remote damage at each blast boundary;
/// local effects retain that order in the reserved batch.
pub fn step(
    ctx: &mut MachineCtx<'_>,
    rules: MachineRules,
    locals: &LocalMachineState,
    batch: &mut MachineBatch,
    changes: &mut Vec<BlockChange>,
    mut on_blast: impl FnMut(&World, Cell),
) {
    crate::bee_hive::accumulate_honey(ctx.world, ctx.ecs, ctx.tick);
    let growth = crate::growth::tick_growth(ctx.world, ctx.tick, rules.raining, rules.seed);
    changes.extend(growth.crop_changes);
    for &cell in &growth.grown_cells {
        if batch.deliver_local
            && batch.saplings.len() < MAX_VISUALS
            && block::is_any_log_block(ctx.world.get_block(cell.0, cell.1, cell.2))
        {
            batch.saplings.push(cell);
        }
    }
    changes.extend(crate::block_machines::cell_changes(
        ctx.world,
        &growth.grown_cells,
    ));

    for order in crate::dispenser::tick_dispensers(ctx.world) {
        let pos = order.pos;
        let out = crate::dispenser::realise_order(ctx, order);
        if batch.deliver_local && batch.ejections.len() < MAX_VISUALS {
            batch.ejections.push(Ejection {
                pos,
                spawn: out.spawn,
                dir: out.dir,
                effect: out.effect,
            });
        }
        changes.extend(out.changes);
    }
    if batch.room() {
        let cells = crate::piston::tick_pistons(ctx.world);
        if !cells.is_empty() {
            batch.record(MachineFact::PistonUsed);
        }
        changes.extend(crate::block_machines::cell_changes(ctx.world, &cells));
    }

    if ctx
        .tick
        .is_multiple_of(crate::villager::CLAIM_SCAN_INTERVAL as u64)
    {
        crate::villager::release_orphan_claims(ctx.ecs, ctx.world);
        crate::villager::tick_workstation_claims(ctx.ecs, ctx.world);
        crate::villager::tick_gossip_refresh(ctx.ecs, ctx.tick);
        crate::villager::tick_raid_warning_gossip(ctx.ecs, ctx.world);
    }
    tick_campfires(ctx, changes);

    let furnaces: Vec<_> = ctx.world.iter_furnaces().map(|(p, _)| p).collect();
    for pos in furnaces {
        // If this furnace could finish, its policy fact must fit before even
        // its fuel/input changes. A full batch defers the remaining furnaces.
        if !batch.room() {
            break;
        }
        if crate::furnace::tick_at(ctx.world, pos, changes)
            .is_some_and(|out| out.recipe_completed.is_some())
        {
            batch.record(MachineFact::FurnaceCompleted(pos));
        }
    }
    crate::composter::tick_all(ctx.world);

    for pos in crate::power::tick_keg_fuses_bounded(ctx.world, batch.remaining()) {
        let mut hits = [0.0; 4];
        let mut close_chests = 0;
        if rules.can_detonate {
            let blast = crate::explosion::detonate_keg_core(
                ctx.world,
                ctx.ecs,
                ctx.water,
                ctx.lava,
                ctx.registry,
                pos,
            );
            for (slot, body) in locals.positions.iter().enumerate() {
                if let Some(body) = body {
                    let centre = (pos.0 as f32 + 0.5, pos.1 as f32 + 0.5, pos.2 as f32 + 0.5);
                    let at = (body.x, body.y, body.z);
                    let los = crate::explosion::line_of_sight_factor(centre, at, |x, y, z| {
                        ctx.registry.is_solid(ctx.world.get_block(x, y, z))
                    });
                    hits[slot] = crate::explosion::blast_damage(
                        centre,
                        at,
                        crate::explosion::BLAST_RADIUS,
                        crate::explosion::KEG_BLAST_DAMAGE,
                        los,
                    );
                }
                if locals.chests[slot]
                    .is_some_and(|chest| blast.destroyed.iter().any(|&(cell, _)| cell == chest))
                {
                    close_chests |= 1 << slot;
                }
            }
            changes.extend(blast.changes);
            on_blast(ctx.world, pos);
        }
        batch.record(MachineFact::Keg {
            pos,
            hits,
            close_chests,
            detonated: rules.can_detonate,
        });
    }

    let racks: Vec<_> = ctx.world.drying_racks.keys().copied().collect();
    for pos in racks {
        if !ctx.world.is_column_present_at(pos.0, pos.2) {
            continue;
        }
        let operating = crate::drying_rack::is_operating(ctx.world, pos.0, pos.1, pos.2);
        if let Some(data) = ctx.world.drying_racks.get_mut(&pos) {
            let out = crate::drying_rack::tick_one(data, operating);
            batch.rack_ready |= batch.deliver_local && !out.newly_mature_slots.is_empty();
        }
    }
    tick_construction(ctx.world, batch, changes);
}

fn tick_campfires(ctx: &mut MachineCtx<'_>, changes: &mut Vec<BlockChange>) {
    let positions: Vec<_> = ctx.world.block_entities.keys().copied().collect();
    for pos in positions {
        if !ctx.world.is_column_present_at(pos.0, pos.2) {
            continue;
        }
        let current = ctx.world.get_block(pos.0, pos.1, pos.2);
        let Some(data) = ctx.world.campfire_at_mut(pos) else {
            continue;
        };
        let out = crate::campfire::tick_one(data, current);
        if let Some(block) = out.block_change {
            ctx.world.set_block(pos.0, pos.1, pos.2, block);
            crate::lighting::update_for_block_change(ctx.world, pos, current, block, ctx.registry);
            changes.push(crate::game_loop::broadcast_change(
                ctx.world, pos.0, pos.1, pos.2, block,
            ));
        }
        let smoke = match out.smoke_state {
            crate::campfire::SmokeState::On => {
                crate::campfire::place_smoke_pillar(ctx.world, pos.0, pos.1, pos.2)
            }
            crate::campfire::SmokeState::Off => {
                crate::campfire::clear_smoke_pillar(ctx.world, pos.0, pos.1, pos.2)
            }
            crate::campfire::SmokeState::NoChange => Vec::new(),
        };
        changes.extend(crate::block_machines::cell_changes(ctx.world, &smoke));
    }
}

fn tick_construction(world: &mut World, batch: &mut MachineBatch, changes: &mut Vec<BlockChange>) {
    let anchors: Vec<_> = world
        .construction_anchors
        .iter()
        // A commission's every-tick lifecycle is its only owner. Its refund,
        // online-player and status policy stays outside the generic sweep.
        .filter(|(_, data)| data.builder_credit.is_none() && construction_loaded(world, data))
        .map(|(&pos, _)| pos)
        .collect();
    for anchor in anchors {
        for (x, y, z, block) in crate::plan::tick_build(world, anchor) {
            changes.push(crate::game_loop::broadcast_change(world, x, y, z, block));
        }
        if crate::plan::build_is_complete(world, anchor) {
            if let Some(data) = world.construction_anchors.get(&anchor) {
                batch.build_finished(&data.plan.name);
            }
            if let Some(pos) = crate::plan::complete_build(world, anchor) {
                changes.push(crate::game_loop::broadcast_change(
                    world,
                    pos.0,
                    pos.1,
                    pos.2,
                    block::ARCHITECT_PLAQUE,
                ));
            }
        }
    }
}

fn construction_loaded(world: &World, data: &crate::plan::ConstructionAnchorData) -> bool {
    let (x, y, z) = data.anchor;
    if data.plan.width == 0
        || data.plan.depth == 0
        || data.plan.height == 0
        || data.plan.cells.iter().any(|cell| {
            cell.rx >= data.plan.width || cell.ry >= data.plan.height || cell.rz >= data.plan.depth
        })
    {
        return false;
    }
    let (width, depth) = if data.rotations.is_multiple_of(2) {
        (data.plan.width, data.plan.depth)
    } else {
        (data.plan.depth, data.plan.width)
    };
    let present = |dx: u8, dy: u8, dz: u8| {
        let Some(tx) = x.checked_add(i32::from(dx)) else {
            return false;
        };
        let Some(tz) = z.checked_add(i32::from(dz)) else {
            return false;
        };
        let Some(ty) = y
            .checked_add(1)
            .and_then(|base| base.checked_add(i32::from(dy)))
        else {
            return false;
        };
        (0..=(crate::world::MAX_CHUNK_Y + 1) * crate::chunk::CHUNK_SIZE as i32 - 1).contains(&ty)
            && world.is_column_present_at(tx, tz)
    };
    // Include actual targets and the final plaque before pace/material writes.
    // Side tables survive eviction; get_block alone is not a presence test.
    world.is_column_present_at(x, z)
        && present(width / 2, 0, depth / 2)
        && crate::plan::rotate_cells(
            &data.plan.cells,
            data.rotations,
            data.plan.width,
            data.plan.depth,
        )
        .iter()
        .all(|cell| present(cell.rx, cell.ry, cell.rz))
}

#[cfg(test)]
#[path = "sim_machine_tests.rs"]
mod tests;
