//! Shared mob-death → dropped-item routing (wave-hardening backlog,
//! 2026-07-11). The ONE place both simulation sides — the client-side
//! `GameState` death sweep (game_loop.rs) and `GameServer::tick`
//! (server.rs) — turn `combat::despawn_dead`'s result into dropped-item
//! entities: the generic per-species loot table (`mob::drops_for`), the
//! wolf-specific tamed/untamed table (`wolf::drops_for_wolf`), a dead
//! steed's cargo-pack spill, and the salt-lick drop bonus.
//!
//! Extracted from game_loop.rs so the hosted server actually spawns
//! drops — `GameServer::tick` used to DISCARD the deaths vector, so a mob
//! killed in the server sim dropped nothing at all. Same shared-free-
//! function pattern as `spawning::tick_mob_spawning` and
//! `falling_blocks::tick_falling_blocks` (Tasks 1b/1c).
//!
//! On the wire since phase 2 (2026-07-11, v57/v58): the entity diff (`entity_broadcast`)
//! broadcasts these drops as `EntityKind::Item` spawns, the server runs
//! lifetimes + pickup for server-simulated players (`InventoryGrant`), and
//! late joiners get the pre-existing drops backfilled at join-accept
//! (2026-07-12). Still riding the dual-sim rework: full-fidelity wire
//! encoding for tool/plan/armour drops (those stay on the server floor).

use glam::Vec3;

/// Ownership/cargo state that must be captured BEFORE `combat::despawn_dead`
/// removes the entities. Keyed by position: `despawn_dead` returns positions,
/// not entity ids, and spawn-time (x, z) uniqueness at world-tick granularity
/// is good enough for the match (the exact scheme the game_loop sweep used
/// inline since Spec 28d.wolves R5).
pub struct PreDespawnSnapshots {
    wolves: Vec<(Vec3, crate::wolf::WolfData)>,
    packs: Vec<(Vec3, crate::chest::ChestData)>,
}

/// Position-key match: same (x, z) within a hundredth of a block.
fn same_spot(a: Vec3, b: Vec3) -> bool {
    (a.x - b.x).abs() < 0.01 && (a.z - b.z).abs() < 0.01
}

/// Capture every dying wolf's `WolfData` (tamed-vs-untamed drop table) and
/// every dying pack-carrier's cargo `ChestData` (spill instead of destroy).
/// Call immediately before `combat::despawn_dead`.
pub fn snapshot_before_despawn(ecs: &hecs::World) -> PreDespawnSnapshots {
    let mut wolves: Vec<(Vec3, crate::wolf::WolfData)> = Vec::new();
    for (e, (health, kind, wolf_data)) in ecs
        .query::<(
            &crate::combat::Health,
            &crate::entity::MobKind,
            &crate::wolf::WolfData,
        )>()
        .iter()
    {
        if health.is_dead()
            && kind.0 == crate::mob::MobType::Wolf
            && let Ok(pos) = ecs.get::<&crate::entity::Position>(e)
        {
            wolves.push((pos.0, wolf_data.clone()));
        }
    }
    let mut packs: Vec<(Vec3, crate::chest::ChestData)> = Vec::new();
    for (e, (health, kind, horse_data)) in ecs
        .query::<(
            &crate::combat::Health,
            &crate::entity::MobKind,
            &crate::horse_ai::HorseData,
        )>()
        .iter()
    {
        if health.is_dead()
            && crate::mob::can_carry_pack(kind.0)
            && let Some(pack) = &horse_data.pack
            && let Ok(pos) = ecs.get::<&crate::entity::Position>(e)
        {
            packs.push((pos.0, pack.clone()));
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
    snaps: &PreDespawnSnapshots,
    kind: crate::mob::MobType,
    pos: Vec3,
    world_time: u32,
) {
    let pos_seed = death_seed(pos, world_time);

    // Spec 28d.wolves R5 — wolf drops route through the wolf-specific table
    // when WolfData was snapshotted (always, since R5 wired live-tame spawn).
    // Tamed wolves drop nothing (emotional loss); untamed drop leather +
    // bones. A Wolf without WolfData shouldn't exist after R5 — defensive
    // fallback to the generic table if one slips through.
    let wolf_data = (kind == crate::mob::MobType::Wolf)
        .then(|| snaps.wolves.iter().find(|(p, _)| same_spot(*p, pos)))
        .flatten();
    if let Some((_p, data)) = wolf_data {
        for stack in crate::wolf::drops_for_wolf(data, pos_seed as u64) {
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
        && let Some((_p, pack)) = snaps.packs.iter().find(|(p, _)| same_spot(*p, pos))
    {
        for stack in spill_pack(pack) {
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
