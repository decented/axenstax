//! Anti-X-ray chunk-stream obfuscation (Spec 8 §5.2.2).
//!
//! Buried ore — an ore block with no air-facing side, invisible without
//! tunnelling — is replaced with its host rock (stone / pure deepslate) in the
//! block data sent to a client. An X-ray cheat can't reveal what it never
//! received. Exposed ore (any non-solid neighbour, e.g. a cave wall) is sent
//! truthfully, matching Minecraft's caving loop.
//!
//! Pure (no winit/wgpu/network) so it unit-tests in isolation and runs
//! server-side, in the shared chunk send-path, **before any bytes leave the
//! server** (the §5.2.2 guarantee). It is the foundation the networked spectator
//! (design Phase 3) and real remote-multiplayer chunk streaming both build on.
//!
//! Built + tested, not yet wired into the live chunk-stream send path (that
//! integration point lands with real remote-multiplayer streaming).

// Module-scoped (not crate-wide) — this file's own doc comment above already
// establishes the whole module as tested-but-unwired, so a per-item
// cfg_attr(not(test)) repeated six times would just restate it.
#![allow(dead_code)]

use crate::block::{self, BlockId};
use crate::world::World;

/// The six face-neighbour offsets.
const FACE_NEIGHBOURS: [(i32, i32, i32); 6] =
    [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)];

/// The host rock a buried ore is disguised as — stone for overworld ores, pure
/// deepslate for deepslate ores (so the disguise blends with its rock layer).
/// A non-ore block returns itself.
fn host_rock(ore: BlockId) -> BlockId {
    match ore {
        // Wind, Copper & Electricity wave (2026-09-07) — buried Copper Ore
        // disguises as stone, same as Coal/Iron/Diamond (Spec 02 §1). Magnesium/
        // Brimstone/Nitre are left as-is — pre-existing decision, not this wave's.
        block::COAL_ORE | block::IRON_ORE | block::DIAMOND_ORE | block::COPPER_ORE => block::STONE,
        block::DEEPSLATE_COAL_ORE | block::DEEPSLATE_IRON_ORE | block::DEEPSLATE_DIAMOND_ORE => {
            block::PURE_DEEPSLATE
        }
        other => other,
    }
}

/// Whether `b` is an ore whose buried instances are hidden by the filter.
pub fn is_obfuscatable_ore(b: BlockId) -> bool {
    host_rock(b) != b
}

/// A neighbour fully occludes an adjacent ore face when it is a solid, opaque
/// block. Air / glass / water / leaves do NOT occlude — they expose the face.
fn occludes(registry: &block::BlockRegistry, b: BlockId) -> bool {
    registry.is_solid(b) && !registry.is_transparent(b)
}

/// Spec 8 §5.2.2 — a block is "buried" when **none** of its 6 face-neighbours is
/// non-solid, i.e. every neighbour fully occludes its face so the block has no
/// air-facing side. OOB / unloaded neighbours read as AIR (non-occluding →
/// exposed), so an edge block is never mistaken for buried.
pub fn is_buried(world: &World, registry: &block::BlockRegistry, x: i32, y: i32, z: i32) -> bool {
    FACE_NEIGHBOURS
        .iter()
        .all(|(dx, dy, dz)| occludes(registry, world.get_block(x + dx, y + dy, z + dz)))
}

/// The block type to SEND for `(x, y, z)`: the host rock if it's a buried ore,
/// else the real block. Exposed ore is always sent truthfully.
pub fn obfuscated_block(
    world: &World,
    registry: &block::BlockRegistry,
    x: i32,
    y: i32,
    z: i32,
) -> BlockId {
    let b = world.get_block(x, y, z);
    if is_obfuscatable_ore(b) && is_buried(world, registry, x, y, z) {
        host_rock(b)
    } else {
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{
        BlockRegistry, AIR, COAL_ORE, COPPER_ORE, DEEPSLATE_DIAMOND_ORE, PURE_DEEPSLATE, STONE,
    };

    /// A safe in-range underground position (all 6 neighbours are valid blocks,
    /// not a world-height edge).
    const P: (i32, i32, i32) = (8, 32, 8);

    /// Place an ore at `P` fully encased in `shell` on all 6 faces.
    fn encased(ore: BlockId, shell: BlockId) -> World {
        let mut w = World::new();
        w.set_block(P.0, P.1, P.2, ore);
        for (dx, dy, dz) in FACE_NEIGHBOURS {
            w.set_block(P.0 + dx, P.1 + dy, P.2 + dz, shell);
        }
        w
    }

    #[test]
    fn buried_overworld_ore_is_disguised_as_stone() {
        let reg = BlockRegistry::new();
        let w = encased(COAL_ORE, STONE);
        assert!(is_buried(&w, &reg, P.0, P.1, P.2), "fully-encased ore is buried");
        assert_eq!(obfuscated_block(&w, &reg, P.0, P.1, P.2), STONE, "buried coal → stone");
    }

    #[test]
    fn buried_deepslate_ore_is_disguised_as_deepslate() {
        let reg = BlockRegistry::new();
        let w = encased(DEEPSLATE_DIAMOND_ORE, PURE_DEEPSLATE);
        assert_eq!(
            obfuscated_block(&w, &reg, P.0, P.1, P.2),
            PURE_DEEPSLATE,
            "buried deepslate diamond → pure deepslate"
        );
    }

    #[test]
    fn exposed_ore_is_sent_truthfully() {
        let reg = BlockRegistry::new();
        let mut w = encased(COAL_ORE, STONE);
        // Open the +Y face to air → the ore now has an air-facing side.
        w.set_block(P.0, P.1 + 1, P.2, AIR);
        assert!(!is_buried(&w, &reg, P.0, P.1, P.2), "an air-facing side exposes it");
        assert_eq!(obfuscated_block(&w, &reg, P.0, P.1, P.2), COAL_ORE, "exposed ore unchanged");
    }

    #[test]
    fn ore_at_a_data_edge_reads_as_exposed() {
        // A lone ore with all-AIR (unset) neighbours must NOT be treated as buried.
        let reg = BlockRegistry::new();
        let mut w = World::new();
        w.set_block(P.0, P.1, P.2, COAL_ORE);
        assert!(!is_buried(&w, &reg, P.0, P.1, P.2));
        assert_eq!(obfuscated_block(&w, &reg, P.0, P.1, P.2), COAL_ORE);
    }

    #[test]
    fn non_ore_blocks_are_never_obfuscated() {
        let reg = BlockRegistry::new();
        let w = encased(STONE, STONE); // buried stone
        assert!(!is_obfuscatable_ore(STONE));
        assert_eq!(obfuscated_block(&w, &reg, P.0, P.1, P.2), STONE, "stone stays stone");
    }

    #[test]
    fn buried_copper_is_disguised_as_stone() {
        // Wind, Copper & Electricity wave (2026-09-07) — Copper Ore is
        // stone-only (no deepslate variant), so it always disguises as STONE.
        let reg = BlockRegistry::new();
        let w = encased(COPPER_ORE, STONE);
        assert!(is_buried(&w, &reg, P.0, P.1, P.2), "fully-encased copper is buried");
        assert_eq!(obfuscated_block(&w, &reg, P.0, P.1, P.2), STONE, "buried copper → stone");
    }

    #[test]
    fn exposed_copper_is_sent_truthfully() {
        let reg = BlockRegistry::new();
        let mut w = encased(COPPER_ORE, STONE);
        w.set_block(P.0, P.1 + 1, P.2, AIR);
        assert!(!is_buried(&w, &reg, P.0, P.1, P.2), "an air-facing side exposes it");
        assert_eq!(
            obfuscated_block(&w, &reg, P.0, P.1, P.2),
            COPPER_ORE,
            "exposed copper unchanged"
        );
    }

    #[test]
    fn transparent_neighbour_exposes_buried_ore() {
        // A glass / water side is non-occluding → the ore is exposed, not buried.
        let reg = BlockRegistry::new();
        let mut w = encased(COAL_ORE, STONE);
        // Find a transparent block id to use as a neighbour (glass-like).
        let transparent = (0..reg.len() as u16).find(|&id| {
            reg.is_transparent(id) && !reg.is_solid(id) || (reg.is_transparent(id) && id != AIR)
        });
        if let Some(t) = transparent {
            w.set_block(P.0, P.1 + 1, P.2, t);
            assert!(!is_buried(&w, &reg, P.0, P.1, P.2), "transparent neighbour (id {t}) exposes the ore");
        }
    }
}
