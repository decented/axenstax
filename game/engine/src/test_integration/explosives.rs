//! Spec 49 (Explosives) — end-to-end integration of the supply chain and the
//! blast. The per-module unit tests already lock the individual pieces
//! (`biome` ore placement, `composter` ageing, `power` fuse arming/countdown,
//! `explosion` blast + guards, `save_load` round-trip). This suite ties them
//! together: ore → material → recipe → keg, and a multi-keg chain reaction.

use crate::block::{self, BlockRegistry};
use crate::crafting::{match_recipe, CraftSlot};
use crate::explosion;
use crate::item::{Item, MaterialId};
use crate::power;
use crate::world::World;

// ── Supply chain: ore → material → recipe → keg ────────────────────────────────

#[test]
fn brimstone_mines_to_sulphur_and_nitre_to_saltpetre() {
    let reg = BlockRegistry::new();

    let sulphur = reg.mine_drop(block::BRIMSTONE);
    assert!(matches!(sulphur.item, Item::Material(MaterialId::Sulphur)));
    assert_eq!(sulphur.count, 1);

    let saltpetre = reg.mine_drop(block::NITRE_ORE);
    assert!(matches!(saltpetre.item, Item::Material(MaterialId::Saltpetre)));
    assert_eq!(saltpetre.count, 1);
}

#[test]
fn black_powder_recipe_matches_in_any_order() {
    let sulphur = CraftSlot::Material(MaterialId::Sulphur);
    let coal = CraftSlot::Material(MaterialId::Coal);
    let saltpetre = CraftSlot::Material(MaterialId::Saltpetre);
    let empty = [CraftSlot::Empty; 3];

    // The 1×3 shapeless recipe matches regardless of arrangement.
    for row in [
        [sulphur, coal, saltpetre],
        [saltpetre, sulphur, coal],
        [coal, saltpetre, sulphur],
    ] {
        let grid = [row, empty, empty];
        let out = match_recipe(&grid).expect("Black Powder recipe must match");
        assert!(matches!(out.item, Item::Material(MaterialId::BlackPowder)));
        assert_eq!(out.count, 3, "one craft seeds three charges");
    }
}

#[test]
fn blasting_keg_recipe_is_a_planks_ring_around_black_powder() {
    let p = CraftSlot::Block(block::OAK_PLANKS);
    let bp = CraftSlot::Material(MaterialId::BlackPowder);
    let grid = [[p, p, p], [p, bp, p], [p, p, p]];
    let out = match_recipe(&grid).expect("Blasting Keg recipe must match");
    assert!(matches!(out.item, Item::Block(b) if b == block::BLASTING_KEG));
    assert_eq!(out.count, 1);

    // A planks ring with an EMPTY centre is the Chest, not a keg — the centre
    // material is what distinguishes them.
    let chest_grid = [[p, p, p], [p, CraftSlot::Empty, p], [p, p, p]];
    let chest = match_recipe(&chest_grid).expect("empty-centre ring is a Chest");
    assert!(matches!(chest.item, Item::Block(b) if b == block::CHEST));
}

#[test]
fn full_supply_chain_ore_to_keg() {
    // Mine the two ores, craft Black Powder from the drops (+ coal), then ring
    // it with planks into a Blasting Keg — the whole ladder in one test.
    let reg = BlockRegistry::new();
    let sulphur = reg.mine_drop(block::BRIMSTONE).item;
    let saltpetre = reg.mine_drop(block::NITRE_ORE).item;
    assert!(matches!(sulphur, Item::Material(MaterialId::Sulphur)));
    assert!(matches!(saltpetre, Item::Material(MaterialId::Saltpetre)));

    let powder_grid = [
        [
            CraftSlot::from_item(&sulphur),
            CraftSlot::Material(MaterialId::Coal),
            CraftSlot::from_item(&saltpetre),
        ],
        [CraftSlot::Empty; 3],
        [CraftSlot::Empty; 3],
    ];
    let powder = match_recipe(&powder_grid).expect("black powder");
    assert!(matches!(powder.item, Item::Material(MaterialId::BlackPowder)));

    let p = CraftSlot::Block(block::OAK_PLANKS);
    let bp = CraftSlot::from_item(&powder.item);
    let keg_grid = [[p, p, p], [p, bp, p], [p, p, p]];
    let keg = match_recipe(&keg_grid).expect("blasting keg");
    assert!(matches!(keg.item, Item::Block(b) if b == block::BLASTING_KEG));
}

// ── Blast integration: world + explosion + power ───────────────────────────────

#[test]
fn a_keg_blast_clears_terrain_but_leaves_no_proof_of_play_work() {
    // Integration of resolve_blast over a real stone field: the crater forms,
    // but the world's lifetime Proof-of-Play work is untouched (demolition, not
    // mining). The drop-nothing rule is structural — resolve_blast only ever
    // sets AIR, never the mine-drop / add_work / HMAC path.
    let mut w = World::new();
    for x in -2..=4 {
        for y in -2..=4 {
            for z in -2..=4 {
                w.set_block(x, y, z, block::STONE);
            }
        }
    }
    w.set_block(1, 1, 1, block::IRON_ORE);
    w.set_block(1, 1, 1, block::BLASTING_KEG); // the keg sits on the vein
    let work_before = w.total_work;

    let outcome = explosion::resolve_blast(
        &mut w,
        (1, 1, 1),
        explosion::BLAST_RADIUS,
        explosion::KEG_BLAST_POWER,
    );

    assert!(!outcome.destroyed.is_empty(), "the blast cleared a crater");
    assert_eq!(w.get_block(1, 1, 1), block::AIR, "centre demolished");
    assert_eq!(w.total_work, work_before, "no Proof-of-Play work from a blast");
    assert_eq!(w.total_work, 0);
}

#[test]
fn a_blast_chain_ignites_a_neighbouring_keg_which_then_detonates() {
    // Sympathetic detonation: detonating one keg lights a short fuse on another
    // caught in the radius; that fuse then burns down to its own blast.
    let mut w = World::new();
    w.set_block(0, 0, 0, block::BLASTING_KEG);
    w.set_block(3, 0, 0, block::BLASTING_KEG); // within the ~4-block radius

    let outcome = explosion::resolve_blast(
        &mut w,
        (0, 0, 0),
        explosion::BLAST_RADIUS,
        explosion::KEG_BLAST_POWER,
    );

    // The detonated keg is gone; the neighbour chain-ignited but still stands.
    assert_eq!(w.get_block(0, 0, 0), block::AIR);
    assert!(outcome.chained.contains(&(3, 0, 0)), "neighbour chain-ignited");
    assert_eq!(
        w.get_block(3, 0, 0),
        block::BLASTING_KEG,
        "the chained keg stands until its short fuse burns down"
    );
    assert_eq!(
        w.power_device_at((3, 0, 0)).unwrap().charge,
        explosion::CHAIN_FUSE_TICKS,
        "the neighbour got a short sympathetic fuse"
    );

    // Burn the short fuse down → the neighbour detonates exactly once.
    let mut detonated = false;
    for _ in 0..explosion::CHAIN_FUSE_TICKS {
        if power::tick_keg_fuses(&mut w).contains(&(3, 0, 0)) {
            detonated = true;
            break;
        }
    }
    assert!(detonated, "the chained keg's fuse burns down to a detonation");
}

#[test]
fn bedrock_immovably_contains_a_blast() {
    // A bedrock shell around a keg: the blast vents nothing — bedrock is immune,
    // so a demolition can't breach the world floor / claim boundary.
    let mut w = World::new();
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                w.set_block(x, y, z, block::BEDROCK);
            }
        }
    }
    w.set_block(0, 0, 0, block::BLASTING_KEG); // overwrite the centre with a keg

    let outcome = explosion::resolve_blast(
        &mut w,
        (0, 0, 0),
        explosion::BLAST_RADIUS,
        explosion::KEG_BLAST_POWER,
    );

    // Only the keg itself (soft) is cleared; every bedrock cell holds.
    for &(p, _) in &outcome.destroyed {
        assert_ne!(
            w.get_block(p.0, p.1, p.2),
            block::BEDROCK,
            "no bedrock cell was destroyed"
        );
    }
    assert_eq!(w.get_block(1, 0, 0), block::BEDROCK, "the shell holds");
    assert_eq!(w.get_block(0, 1, 0), block::BEDROCK);
}
