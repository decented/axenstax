//! The loot router used by the shared `sim_death` authority pass: generic
//! and wolf-specific tables, exact-entity live cargo spill and salt bonus.
//! World items keep their existing entity-diff and numbered pickup paths;
//! local death notices never grant a second copy of an item.

use glam::Vec3;

/// Ownership/cargo state that must be captured BEFORE `combat::despawn_dead`
/// removes the entities. Exact generational entity ids distinguish even
/// two steeds dying at the same position in the same tick.
pub struct PreDespawnSnapshots {
    wolves: std::collections::HashMap<hecs::Entity, crate::wolf::WolfData>,
    packs: std::collections::HashMap<hecs::Entity, crate::chest::ChestData>,
}

/// Capture every dying wolf's drop table and pack-carrier's live cargo.
/// The bounded shared pass uses `snapshot_entities` for its selected batch.
#[cfg(test)]
pub fn snapshot_before_despawn(ecs: &hecs::World) -> PreDespawnSnapshots {
    snapshot_entities(ecs, ecs.query::<&crate::combat::Health>().iter()
        .filter(|(_, h)| h.is_dead()).map(|(entity, _)| entity))
}

pub(crate) fn snapshot_entities(ecs: &hecs::World, entities: impl Iterator<Item = hecs::Entity>) -> PreDespawnSnapshots {
    let mut wolves = std::collections::HashMap::new();
    let mut packs = std::collections::HashMap::new();
    for entity in entities {
        if !ecs.get::<&crate::combat::Health>(entity).is_ok_and(|h| h.is_dead()) { continue; }
        let Ok(kind) = ecs.get::<&crate::entity::MobKind>(entity) else { continue };
        if kind.0 == crate::mob::MobType::Wolf
            && let Ok(data) = ecs.get::<&crate::wolf::WolfData>(entity) {
            wolves.insert(entity, (*data).clone());
        }
        if crate::mob::can_carry_pack(kind.0)
            && let Ok(data) = ecs.get::<&crate::horse_ai::HorseData>(entity)
            && let Some(pack) = &data.pack {
            packs.insert(entity, pack.clone());
        }
    }
    PreDespawnSnapshots { wolves, packs }
}

/// Task 2 (bug-hardening review, 2026-07-07) — a dead Donkey/Mule's equipped
/// cargo pack used to vanish with the mob: `HorseData.pack` (up to 27 slots)
/// was never consulted by the death sweep, so the chest + everything inside
/// it was silently destroyed. Returns every contained stack plus one Chest
/// block item (the pack itself was a consumed Chest — see the equip arm in
/// game_loop.rs). Pure: the caller spawns each returned stack as an item
/// entity at the mob's death position.
pub fn spill_pack(pack: &crate::chest::ChestData) -> Vec<crate::item::ItemStack> {
    let mut out: Vec<crate::item::ItemStack> = pack.slots.iter().flatten().cloned().collect();
    out.push(crate::item::ItemStack::new_block(crate::block::CHEST, 1));
    out
}

/// Deterministic per-death seed: the death position's (x, z) bits +
/// `world_time`, so the same kill in the same tick replays identically on
/// both simulation sides. Public so callers can seed death-adjacent effects
/// (the attributed-kill smoke puff) consistently with the drops.
pub fn death_seed(pos: Vec3, world_time: u32) -> u32 {
    (pos.x as i32 as u32).wrapping_mul(374761393)
        ^ (pos.z as i32 as u32).wrapping_mul(668265263)
        ^ world_time.wrapping_mul(2246822519)
}

/// Spawn every drop owed by one `despawn_dead` death: species loot table
/// (wolves route through `wolf::drops_for_wolf` — tamed wolves drop nothing),
/// cargo-pack spill, and the salt-lick drop bonus.
pub fn spawn_drops_for_death(
    ecs: &mut hecs::World,
    world: &crate::world::World,
    snaps: &mut PreDespawnSnapshots,
    death: crate::combat::MobDeath,
    world_time: u32,
) {
    let crate::combat::MobDeath { entity, kind, pos, .. } = death;
    let pos_seed = death_seed(pos, world_time);

    // Spec 28d.wolves R5 — wolf drops route through the wolf-specific table
    // when WolfData was snapshotted (always, since R5 wired live-tame spawn).
    // Tamed wolves drop nothing (emotional loss); untamed drop leather +
    // bones. A Wolf without WolfData shouldn't exist after R5 — defensive
    // fallback to the generic table if one slips through.
    let wolf_data = (kind == crate::mob::MobType::Wolf)
        .then(|| snaps.wolves.remove(&entity))
        .flatten();
    if let Some(data) = wolf_data {
        for stack in crate::wolf::drops_for_wolf(&data, pos_seed as u64) {
            crate::entity::spawn_item(ecs, pos, stack, pos_seed);
        }
    } else {
        for stack in crate::mob::drops_for(kind, pos_seed) {
            crate::entity::spawn_item(ecs, pos, stack, pos_seed);
        }
    }

    // A dead Donkey/Mule wearing a cargo pack spills the chest + contents on
    // top of the generic drop table. A Pet-Bed-rescued steed never appears in
    // `despawn_dead`'s result, so this can't double-drop.
    if crate::mob::can_carry_pack(kind)
        && let Some(pack) = snaps.packs.remove(&entity)
    {
        for stack in spill_pack(&pack) {
            crate::entity::spawn_item(ecs, pos, stack, pos_seed.wrapping_add(0x9A11));
        }
    }

    // Salt feature — drop-bonus arm. Affected species dying inside any
    // SALT_LICK aura drop one extra of their primary product.
    if let Some(primary) = crate::salt_lick::salt_lick_primary_drop(kind)
        && crate::salt_lick::within_any_salt_lick_aura(world, pos)
    {
        let bonus = crate::item::ItemStack::new_material(primary, 1);
        crate::entity::spawn_item(ecs, pos, bonus, pos_seed.wrapping_add(0x5A17));
    }
}

#[cfg(test)]
mod tests {
    use super::spill_pack;
    use crate::chest::ChestData;
    use crate::item::{ItemStack, MaterialId};

    #[test]
    fn d2c2_colocated_pack_deaths_spill_each_live_cargo_once() {
        let mut ecs = hecs::World::new();
        let world = crate::world::World::new();
        let pos = glam::Vec3::new(4.0, 80.0, 4.0);
        for (kind, item, count) in [
            (crate::mob::MobType::Donkey, crate::item::Item::Material(MaterialId::Bread), 3),
            (crate::mob::MobType::Mule, crate::item::Item::Block(crate::block::STONE), 5),
        ] {
            let id = crate::entity::spawn_mob(&mut ecs, kind, pos);
            let mut pack = ChestData::new();
            pack.slots[0] = Some(ItemStack { item, count });
            ecs.get::<&mut crate::horse_ai::HorseData>(id).unwrap().pack = Some(pack);
            ecs.get::<&mut crate::combat::Health>(id).unwrap().current = 0.0;
        }
        let mut snapshots = super::snapshot_before_despawn(&ecs);
        let deaths = crate::combat::despawn_dead(&mut ecs);
        for death in deaths {
            super::spawn_drops_for_death(&mut ecs, &world, &mut snapshots, death, 17);
            // Re-delivering this dead entity cannot drain its cargo again.
            super::spawn_drops_for_death(&mut ecs, &world, &mut snapshots, death, 17);
        }
        assert!(snapshots.packs.is_empty(), "each cargo snapshot is consumed");
        let units = |item: crate::item::Item| ecs.query::<&crate::entity::ItemEntity>().iter()
            .filter(|(_, drop)| drop.stack.item == item).map(|(_, drop)| u32::from(drop.stack.count)).sum::<u32>();
        assert_eq!(units(crate::item::Item::Material(MaterialId::Bread)), 3, "each steed's own bread, never the first colocated pack twice");
        assert_eq!(units(crate::item::Item::Block(crate::block::STONE)), 5, "the second steed's cargo is never lost");
        assert_eq!(units(crate::item::Item::Block(crate::block::CHEST)), 2, "one real pack Chest per steed");
    }

    #[test]
    fn empty_pack_still_spills_the_chest_itself() {
        let pack = ChestData::new();
        let spilled = spill_pack(&pack);
        assert_eq!(spilled, vec![ItemStack::new_block(crate::block::CHEST, 1)]);
    }

    #[test]
    fn filled_slots_spill_alongside_one_chest_item() {
        let mut pack = ChestData::new();
        pack.slots[0] = Some(ItemStack::new_material(MaterialId::Stick, 5));
        pack.slots[3] = Some(ItemStack::new_block(crate::block::STONE, 12));
        let spilled = spill_pack(&pack);
        assert_eq!(spilled.len(), 3, "2 filled slots + 1 chest");
        assert!(spilled.contains(&ItemStack::new_material(MaterialId::Stick, 5)));
        assert!(spilled.contains(&ItemStack::new_block(crate::block::STONE, 12)));
        assert!(spilled.contains(&ItemStack::new_block(crate::block::CHEST, 1)));
    }
}
