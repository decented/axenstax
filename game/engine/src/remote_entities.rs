//! Client-side store for server-broadcast item entities (death-drops
//! phase 2b).
//!
//! A remote client renders the server's dropped items from the entity
//! spawn/update/despawn diff in each `StateUpdatePacket` — these are
//! render-only ghosts, never inserted into the client ECS (the client sim's
//! own `ItemEntity`s would otherwise re-run pickup/lifetime/physics on them
//! and double-grant). Pickup is server-authoritative: the server despawns
//! the entity (removing it here via the diff) and delivers the stack with an
//! `InventoryGrantPacket`, applied by [`apply_inventory_grant`].
//!
//! Projectiles in flight (MP-A3, protocol v67) get the same treatment in
//! [`RemoteProjectiles`]: a dedicated server's dispenser arrows fly, hit and
//! despawn in the SERVER's sim, and a joiner only draws them.
//!
//! Mob and cart spawns in the same diff go to the joiner's mob mirror,
//! `remote_mobs::RemoteMobs` (MP-D2a); these tables pass them through.

use glam::Vec3;
use std::collections::HashMap;

/// One `StateUpdate`'s entity deltas, kept together: the server orders a
/// packet's deltas (spawns, then updates, then despawns) and an entity can be
/// withdrawn in one packet and re-spawned in the next (`entity_broadcast`'s
/// interest radius). Folding several packets into one flat list would apply
/// the later spawn BEFORE the earlier despawn and delete the new copy
/// (review D2a MEDIUM-2), so a client keeps one of these per packet and
/// applies them in arrival order ([`apply_entity_batches`]).
#[derive(Clone, Debug, Default)]
pub struct EntityDeltas {
    pub spawns: Vec<crate::protocol::EntitySpawn>,
    pub updates: Vec<crate::protocol::EntityUpdate>,
    pub despawns: Vec<u32>,
}

impl EntityDeltas {
    /// Take a decoded `StateUpdate`'s entity deltas (leaving them empty).
    pub fn take_from(state: &mut crate::protocol::StateUpdatePacket) -> Self {
        Self {
            spawns: std::mem::take(&mut state.entity_spawns),
            updates: std::mem::take(&mut state.entity_updates),
            despawns: std::mem::take(&mut state.entity_despawns),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.spawns.is_empty() && self.updates.is_empty() && self.despawns.is_empty()
    }
}

/// Fold a frame's entity deltas into a joined client's render tables, one
/// `StateUpdate` at a time in arrival order: dropped items, projectiles,
/// and — when `mobs` is given (joined) — the mob and cart mirror.
pub fn apply_entity_batches(
    batches: &[EntityDeltas],
    items: &mut RemoteItems,
    projectiles: &mut RemoteProjectiles,
    mut mobs: Option<&mut crate::remote_mobs::RemoteMobs>,
) {
    for b in batches {
        items.apply(&b.spawns, &b.updates, &b.despawns);
        projectiles.apply(&b.spawns, &b.updates, &b.despawns);
        if let Some(mobs) = mobs.as_deref_mut() {
            mobs.apply(&b.spawns, &b.updates, &b.despawns);
        }
    }
}

/// One server-side dropped item, keyed by its wire `ProtocolId`.
pub struct RemoteItem {
    pub pos: Vec3,
    /// The stack's legacy wire ref. `Empty` ONLY when `full` carries the item
    /// instead (armour); a spawn with neither is skipped at insert — a plan
    /// drop encodes as nothing on both, and there is nothing to render for it.
    pub item: crate::protocol::ItemRef,
    /// Death-drops phase 3 (v61) — the full-fidelity payload when the pair
    /// above can't express the stack (tools, armour). `WireItem::None` for
    /// block/material drops, where the pair is already lossless.
    pub full: crate::protocol::WireItem,
    #[allow(dead_code)] // count will label the stack when drop-count HUD lands
    pub count: u8,
}

/// The remote-item table for the current server session.
#[derive(Default)]
pub struct RemoteItems {
    map: HashMap<u32, RemoteItem>,
}

impl RemoteItems {
    /// Fold one tick's entity diff into the table. Non-`Item` spawns and
    /// updates for unknown ids (mobs, carts — `remote_mobs` takes those)
    /// pass through untouched.
    pub fn apply(
        &mut self,
        spawns: &[crate::protocol::EntitySpawn],
        updates: &[crate::protocol::EntityUpdate],
        despawns: &[u32],
    ) {
        use crate::protocol::{EntityKind, ItemRef, WireItem};
        for s in spawns {
            if s.kind != EntityKind::Item {
                continue;
            }
            let item = ItemRef::from_wire(s.item_kind, s.item_id);
            if item == ItemRef::Empty && s.full_item == WireItem::None {
                // Nothing on either channel — a plan drop (deliberately
                // floor-bound) or a malformed spawn. Nothing renderable.
                continue;
            }
            self.map.insert(
                s.id,
                RemoteItem {
                    pos: Vec3::new(s.x, s.y, s.z),
                    item,
                    full: s.full_item,
                    count: s.item_count,
                },
            );
        }
        // Updates arrive for every shown entity that changed (mobs + carts
        // too) — only ids already in the table are items we track.
        for u in updates {
            if let Some(it) = self.map.get_mut(&u.id) {
                it.pos = Vec3::new(u.x, u.y, u.z);
            }
        }
        for id in despawns {
            self.map.remove(id);
        }
    }

    /// Drop the whole table — call when the server session ends.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &RemoteItem> {
        self.map.values()
    }

    #[cfg(test)]
    fn get(&self, id: u32) -> Option<&RemoteItem> {
        self.map.get(&id)
    }
}

/// One server-side projectile in flight, keyed by its wire `ProtocolId`.
pub struct RemoteProjectile {
    pub pos: Vec3,
    /// Unit heading the arrow model points along: from the spawn's yaw until
    /// the first update arrives, then from the motion between updates (which
    /// also carries the arc gravity puts on it — the yaw alone is flat).
    pub dir: Vec3,
}

/// The remote-projectile table for the current server session (MP-A3).
#[derive(Default)]
pub struct RemoteProjectiles {
    map: HashMap<u32, RemoteProjectile>,
}

impl RemoteProjectiles {
    /// Fold one tick's entity diff in. Non-`Projectile` spawns and updates for
    /// ids it doesn't hold (mobs, carts, items) pass through untouched.
    pub fn apply(
        &mut self,
        spawns: &[crate::protocol::EntitySpawn],
        updates: &[crate::protocol::EntityUpdate],
        despawns: &[u32],
    ) {
        for s in spawns {
            if s.kind != crate::protocol::EntityKind::Projectile {
                continue;
            }
            // Inverse of the server's `(-vx).atan2(-vz)` heading yaw.
            let dir = Vec3::new(-s.yaw.sin(), 0.0, -s.yaw.cos());
            self.map.insert(s.id, RemoteProjectile { pos: Vec3::new(s.x, s.y, s.z), dir });
        }
        for u in updates {
            if let Some(p) = self.map.get_mut(&u.id) {
                let next = Vec3::new(u.x, u.y, u.z);
                if let Some(dir) = (next - p.pos).try_normalize() {
                    p.dir = dir;
                }
                p.pos = next;
            }
        }
        for id in despawns {
            self.map.remove(id);
        }
    }

    /// Drop the whole table — call when the server session ends.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &RemoteProjectile> {
        self.map.values()
    }

    #[cfg(test)]
    fn get(&self, id: u32) -> Option<&RemoteProjectile> {
        self.map.get(&id)
    }
}

/// Apply a server `InventoryGrantPacket` to the local player: decode the
/// wire stack and add it to `inv`. If the inventory can't hold all of it,
/// the remainder is spawned as a LOCAL ground item at the player's feet so
/// nothing is silently lost (it re-enters via the normal client pickup pass
/// when space frees up). Returns `false` when the stack doesn't decode
/// (tampered or newer-version server) — nothing is changed in that case.
///
/// Decode precedence (death-drops phase 3, v61): a `full` payload other than
/// `WireItem::None` WINS over the legacy `(kind, id)` pair — it carries the
/// tool/armour fidelity the pair throws away. A `full` that fails validation
/// (unknown byte, zero durability) refuses the grant outright rather than
/// silently falling back to the lossy pair, which for a tool would mint a
/// fabricated full-durability item.
pub fn apply_inventory_grant(
    inv: &mut crate::inventory::Inventory,
    ecs: &mut hecs::World,
    player_pos: Vec3,
    grant: &crate::protocol::InventoryGrantPacket,
    registry: &crate::block::BlockRegistry,
) -> bool {
    let crate::protocol::InventoryGrantPacket { item_kind: kind, item_id: id, count, full_item } =
        *grant;
    let decoded = if full_item == crate::protocol::WireItem::None {
        crate::inventory::item_from_ref(kind, id, registry)
    } else {
        crate::inventory::item_from_wire_full(&full_item)
    };
    let Some(item) = decoded else {
        log::warn!(
            "InventoryGrant with undecodable stack (kind {kind}, id {id}, full {full_item:?}) — dropped"
        );
        return false;
    };
    if count == 0 {
        return false;
    }
    let stack = crate::item::ItemStack { item, count };
    if let Some(remainder) = inv.add_item(stack) {
        // Inventory full (or partially) — spill what didn't fit at the
        // player's feet as a normal local ground item; the client pickup
        // pass re-grants it when space frees up.
        if remainder.count > 0 {
            crate::entity::spawn_item(&mut *ecs, player_pos, remainder, id as u32);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{item_kind, EntityKind, EntitySpawn, EntityUpdate, ItemRef, WireItem};

    fn item_spawn(id: u32, kind: u8, item_id: u16, count: u8) -> EntitySpawn {
        EntitySpawn {
            id,
            kind: EntityKind::Item,
            x: 1.0,
            y: 65.0,
            z: 2.0,
            yaw: 0.0,
            health: 0,
            item_kind: kind,
            item_id,
            item_count: count,
            full_item: crate::protocol::WireItem::None,
        }
    }

    /// A spawn whose fidelity rides `full_item` (tools, armour).
    fn full_spawn(id: u32, item: &crate::item::Item) -> EntitySpawn {
        let (item_kind, item_id) = crate::inventory::item_to_ref(item).to_wire();
        EntitySpawn {
            item_kind,
            item_id,
            full_item: crate::inventory::item_to_wire_full(item),
            ..item_spawn(id, item_kind::EMPTY, 0, 1)
        }
    }

    fn mob_spawn(id: u32) -> EntitySpawn {
        EntitySpawn {
            id,
            kind: EntityKind::Cow,
            x: 0.0,
            y: 64.0,
            z: 0.0,
            yaw: 0.0,
            health: 10,
            item_kind: 0,
            item_id: 0,
            item_count: 0,
            full_item: crate::protocol::WireItem::None,
        }
    }

    #[test]
    fn apply_inserts_item_spawns_and_ignores_mobs() {
        let mut items = RemoteItems::default();
        items.apply(
            &[
                item_spawn(7, item_kind::MATERIAL, crate::item::MaterialId::Bone as u16, 2),
                mob_spawn(8),
            ],
            &[],
            &[],
        );
        let it = items.get(7).expect("item spawn stored");
        assert_eq!(it.item, ItemRef::Material(crate::item::MaterialId::Bone as u16));
        assert_eq!(it.count, 2);
        assert_eq!(it.pos, Vec3::new(1.0, 65.0, 2.0));
        assert!(items.get(8).is_none(), "mob spawn must not be stored");
    }

    #[test]
    fn apply_skips_spawns_with_nothing_on_either_channel() {
        // A plan drop encodes ItemRef::Empty AND WireItem::None — floor-bound
        // by design, nothing renderable.
        let mut items = RemoteItems::default();
        items.apply(&[item_spawn(3, item_kind::EMPTY, 0, 1)], &[], &[]);
        assert!(items.is_empty());
    }

    #[test]
    fn apply_stores_tool_and_armour_spawns_from_the_full_payload() {
        // Death-drops phase 3 — armour's legacy pair is `Empty`, so before
        // v61 the spawn was skipped outright and the drop was invisible.
        use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot};
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let mut pick = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        pick.durability = 37;
        let mut boots = ArmourItem::new(ArmourSlot::Boots, ArmourMaterial::Leather);
        boots.durability = 9;
        let tool_item = crate::item::Item::Tool(pick);
        let armour_item = crate::item::Item::Armour(boots);

        let mut items = RemoteItems::default();
        items.apply(
            &[full_spawn(11, &tool_item), full_spawn(12, &armour_item)],
            &[],
            &[],
        );
        assert_eq!(
            crate::inventory::item_from_wire_full(&items.get(11).unwrap().full),
            Some(tool_item)
        );
        assert_eq!(
            crate::inventory::item_from_wire_full(&items.get(12).unwrap().full),
            Some(armour_item),
            "armour renders from the full payload despite an Empty legacy pair"
        );
    }

    #[test]
    fn apply_moves_known_items_and_ignores_unknown_update_ids() {
        let mut items = RemoteItems::default();
        items.apply(
            &[item_spawn(7, item_kind::BLOCK, 3, 1)],
            &[],
            &[],
        );
        items.apply(
            &[],
            &[
                EntityUpdate { id: 7, x: 4.0, y: 66.0, z: -1.0, yaw: 0.0, state: 0, ..Default::default() },
                // A mob's update — unknown id here, must be a no-op.
                EntityUpdate { id: 99, x: 0.0, y: 0.0, z: 0.0, yaw: 0.0, state: 2, ..Default::default() },
            ],
            &[],
        );
        assert_eq!(items.get(7).unwrap().pos, Vec3::new(4.0, 66.0, -1.0));
        assert!(items.get(99).is_none());
    }

    #[test]
    fn apply_removes_on_despawn() {
        let mut items = RemoteItems::default();
        items.apply(&[item_spawn(7, item_kind::BLOCK, 3, 1)], &[], &[]);
        items.apply(&[], &[], &[7, 42]);
        assert!(items.is_empty(), "picked-up/decayed item leaves the table");
    }

    // ── MP-A3: server-side projectiles ─────────────────────────────────────

    fn projectile_spawn(id: u32, x: f32, yaw: f32) -> EntitySpawn {
        EntitySpawn {
            id,
            kind: EntityKind::Projectile,
            x,
            y: 80.0,
            z: 0.0,
            yaw,
            health: 0,
            item_kind: 0,
            item_id: 0,
            item_count: 0,
            full_item: WireItem::None,
        }
    }

    #[test]
    fn projectiles_are_tracked_apart_from_items_and_mobs() {
        let mut items = RemoteItems::default();
        let mut arrows = RemoteProjectiles::default();
        let spawns = [projectile_spawn(9, 1.0, 0.0), mob_spawn(8), item_spawn(7, item_kind::BLOCK, 3, 1)];
        items.apply(&spawns, &[], &[]);
        arrows.apply(&spawns, &[], &[]);
        assert!(items.get(9).is_none(), "an arrow is not a pickup-able item");
        assert!(items.get(7).is_some());
        assert_eq!(arrows.iter().count(), 1, "only the projectile spawn is an arrow");
        assert!(arrows.get(9).is_some());
    }

    #[test]
    fn a_projectile_points_along_its_spawn_yaw_then_along_its_flight() {
        let mut arrows = RemoteProjectiles::default();
        // Spawn yaw for "flying +x": the renderer's `(-vx).atan2(-vz)` rule.
        let east = (-1.0f32).atan2(-0.0);
        arrows.apply(&[projectile_spawn(9, 1.0, east)], &[], &[]);
        let dir = arrows.get(9).unwrap().dir;
        assert!(dir.x > 0.99 && dir.y.abs() < 1e-5, "from the yaw alone: {dir:?}");

        // Then the server's updates: the motion between them is the heading,
        // dipping as gravity takes it.
        arrows.apply(
            &[],
            &[EntityUpdate { id: 9, x: 2.0, y: 79.5, z: 0.0, yaw: east, state: 0, ..Default::default() }],
            &[],
        );
        let a = arrows.get(9).unwrap();
        assert_eq!(a.pos, Vec3::new(2.0, 79.5, 0.0));
        assert!(a.dir.x > 0.0 && a.dir.y < 0.0, "east and falling: {:?}", a.dir);
        assert!((a.dir.length() - 1.0).abs() < 1e-4);

        arrows.apply(&[], &[], &[9]);
        assert_eq!(arrows.iter().count(), 0, "a hit removes it");
    }

    #[test]
    fn clear_empties_the_projectile_table() {
        let mut arrows = RemoteProjectiles::default();
        arrows.apply(&[projectile_spawn(1, 0.0, 0.0)], &[], &[]);
        arrows.clear();
        assert_eq!(arrows.iter().count(), 0);
    }

    fn grant(kind: u8, id: u16, count: u8, full_item: WireItem) -> crate::protocol::InventoryGrantPacket {
        crate::protocol::InventoryGrantPacket { item_kind: kind, item_id: id, count, full_item }
    }

    #[test]
    fn grant_adds_stack_to_inventory() {
        let mut inv = crate::inventory::Inventory::new();
        let mut ecs = hecs::World::new();
        let reg = crate::block::BlockRegistry::new();
        let ok = apply_inventory_grant(
            &mut inv,
            &mut ecs,
            Vec3::new(0.0, 64.0, 0.0),
            &grant(item_kind::MATERIAL, crate::item::MaterialId::Bone as u16, 2, WireItem::None),
            &reg,
        );
        assert!(ok);
        let bones: u32 = inv
            .slots_iter()
            .flatten()
            .filter(|s| matches!(s.item, crate::item::Item::Material(crate::item::MaterialId::Bone)))
            .map(|s| s.count as u32)
            .sum();
        assert_eq!(bones, 2);
        assert_eq!(ecs.query::<&crate::entity::ItemEntity>().iter().count(), 0);
    }

    #[test]
    fn grant_overflow_spills_to_local_ground_item() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let mut inv = crate::inventory::Inventory::new();
        // Fill all 36 slots with non-stacking tools — zero headroom.
        for i in 0..36 {
            inv.set_slot(
                i,
                Some(crate::item::ItemStack::new_tool(Tool::new(
                    ToolType::Pickaxe,
                    ToolMaterial::Iron,
                ))),
            );
        }
        let mut ecs = hecs::World::new();
        let reg = crate::block::BlockRegistry::new();
        let ok = apply_inventory_grant(
            &mut inv,
            &mut ecs,
            Vec3::new(0.0, 64.0, 0.0),
            &grant(item_kind::MATERIAL, crate::item::MaterialId::Bone as u16, 2, WireItem::None),
            &reg,
        );
        assert!(ok);
        let ground: Vec<u8> = ecs
            .query::<&crate::entity::ItemEntity>()
            .iter()
            .map(|(_, it)| it.stack.count)
            .collect();
        assert_eq!(ground, vec![2], "whole stack spilled at the player's feet");
    }

    #[test]
    fn grant_rejects_undecodable_wire_pairs() {
        let mut inv = crate::inventory::Inventory::new();
        let mut ecs = hecs::World::new();
        let reg = crate::block::BlockRegistry::new();
        // A bare TOOL ref with no full payload is lossy — must never mint a
        // fabricated full-durability tool.
        assert!(!apply_inventory_grant(
            &mut inv,
            &mut ecs,
            Vec3::ZERO,
            &grant(item_kind::TOOL, 2, 1, WireItem::None),
            &reg
        ));
        // Hostile block id.
        assert!(!apply_inventory_grant(
            &mut inv,
            &mut ecs,
            Vec3::ZERO,
            &grant(item_kind::BLOCK, u16::MAX, 1, WireItem::None),
            &reg
        ));
        assert!(inv.slots_iter().flatten().count() == 0);
        assert_eq!(ecs.query::<&crate::entity::ItemEntity>().iter().count(), 0);
    }

    #[test]
    fn grant_decodes_the_full_payload_in_preference_to_the_pair() {
        // Death-drops phase 3 — the legacy pair says "iron tier"; the full
        // payload says "iron pickaxe, 37 durability". The payload wins.
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let mut pick = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        pick.durability = 37;
        let mut inv = crate::inventory::Inventory::new();
        let mut ecs = hecs::World::new();
        let reg = crate::block::BlockRegistry::new();
        let ok = apply_inventory_grant(
            &mut inv,
            &mut ecs,
            Vec3::ZERO,
            &grant(
                item_kind::TOOL,
                2,
                1,
                crate::inventory::item_to_wire_full(&crate::item::Item::Tool(pick)),
            ),
            &reg,
        );
        assert!(ok);
        let got: Vec<crate::item::Item> =
            inv.slots_iter().flatten().map(|s| s.item.clone()).collect();
        assert_eq!(got, vec![crate::item::Item::Tool(pick)]);
    }

    #[test]
    fn grant_refuses_a_tampered_full_payload_without_falling_back() {
        // Over-max clamps (see inventory.rs), but a zero durability or an
        // unknown byte must refuse outright — falling back to the lossy pair
        // would mint a fabricated full-durability tool, exactly what the
        // refusal exists to prevent.
        let mut inv = crate::inventory::Inventory::new();
        let mut ecs = hecs::World::new();
        let reg = crate::block::BlockRegistry::new();
        for bad in [
            WireItem::Tool { tool_type: 0, material: 2, durability: 0 },
            WireItem::Tool { tool_type: 200, material: 2, durability: 10 },
            WireItem::Armour { slot: 0, material: 200, durability: 10 },
        ] {
            assert!(
                !apply_inventory_grant(
                    &mut inv,
                    &mut ecs,
                    Vec3::ZERO,
                    &grant(item_kind::TOOL, 2, 1, bad),
                    &reg
                ),
                "{bad:?} must be refused"
            );
        }
        assert_eq!(inv.slots_iter().flatten().count(), 0);
        assert_eq!(ecs.query::<&crate::entity::ItemEntity>().iter().count(), 0);
    }
}
