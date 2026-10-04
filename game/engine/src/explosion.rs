//! Explosion resolution (Spec 49 — Explosives). Pure geometry + damage helpers
//! plus the world-mutating [`resolve_blast`] for the Blasting Keg.
//!
//! THE ONE LOAD-BEARING RULE: blasting is **demolition, not mining**. Blasted
//! blocks drop **nothing** and run **no Proof-of-Play hash** — [`resolve_blast`]
//! only ever calls `set_block(AIR)`, never the mine-drop / `add_work` / HMAC
//! path. Ore never drops from a blast. This stops explosives shortcutting the
//! pickaxe-strike reward loop (Proof-of-Play integrity). See the spec's
//! §"Proof-of-Play integrity" and `reference_proof_of_play_is_proof_of_work`.
//!
//! Implements the Spec 05 §6.3/§11 `Explosion` damage hook (the particle + sound
//! hooks land at the call site: `audio::play_explosion`; a richer particle pass
//! is deferred until the engine grows a particle framework).

use crate::block::{self, BlockId};
use crate::world::World;

/// Blast radius in blocks (~4, tunable — Axolittle tunes the feel at playtest).
pub const BLAST_RADIUS: f32 = 4.0;
/// The keg's blast power: a block is destroyed iff its [`blast_resistance`] is
/// below this. Tuned so common terrain + ore go, but the tough/immune set holds.
pub const KEG_BLAST_POWER: f32 = 10.0;
/// Peak blast damage at the centre, in half-hearts (tunable).
pub const KEG_BLAST_DAMAGE: f32 = 12.0;
/// Sympathetic detonation: a keg caught in a blast lights a short fuse so the
/// chain ripples rather than vaporising instantly. ~0.5 s @ 20 TPS.
pub const CHAIN_FUSE_TICKS: u32 = 10;
/// Resistance sentinel: a block this strong is never breached by a keg.
pub const IMMUNE: f32 = f32::INFINITY;

/// Per-block blast resistance — a **derived** table (like `camera_occlusion`),
/// not a stored `BlockDef` field. An *intentional* table, NOT `block_hardness`
/// (which is "fist-seconds": stone is a slow 10 s fist-mine yet should blast
/// easily). Bedrock and Satori are immune; deepslate + metal/gem storage blocks
/// resist a single keg (deep mining isn't trivialised); stone-family + ores are
/// tough-but-blastable; everything soft else. Obsidian is immune (the spec's
/// "Bedrock, Obsidian, and Satori are immune" rule).
///
/// Economy blocks (Vendor, Tip Jar, Auction, Plot Marker, Market Bell) are
/// also immune (audit 2026-09-27): a keg lit next to someone else's shop must
/// not wipe their stock, escrow or plot claim. Their block-entity survives
/// with the block. Containers (chest, grave, furnace, …) stay blastable, but
/// [`resolve_blast`] spills their contents instead of deleting them.
pub fn blast_resistance(id: BlockId) -> f32 {
    match id {
        block::AIR => 0.0,
        // Immune — never breached by a keg.
        block::BEDROCK | block::SATORI_BLOCK | block::OBSIDIAN => IMMUNE,
        id if is_economy_block(id) => IMMUNE,
        // Deepslate stays hard — a single keg shouldn't trivialise deep mining.
        id if block::is_pure_deepslate_family(id) => 30.0,
        block::DEEPSLATE_COAL_ORE | block::DEEPSLATE_IRON_ORE | block::DEEPSLATE_DIAMOND_ORE => {
            30.0
        }
        // Compressed metal/gem storage blocks resist a single keg.
        block::IRON_BLOCK | block::DIAMOND_BLOCK | block::COAL_BLOCK => 18.0,
        // Stone-family + ores: tough, but a keg breaches them (no drop — see the
        // demolition-not-mining rule).
        block::STONE
        | block::COBBLESTONE
        | block::SANDSTONE
        | block::COAL_ORE
        | block::IRON_ORE
        | block::DIAMOND_ORE
        | block::COPPER_ORE
        | block::MAGNESIUM_ORE
        | block::BRIMSTONE
        | block::NITRE_ORE => 6.0,
        // Everything else (dirt, sand, wood, leaves, glass, crops, the keg
        // itself, …) is soft.
        _ => 1.0,
    }
}

/// Economy blocks whose block-entity holds another player's stock, escrow or
/// land claim. Blast-immune (see [`blast_resistance`]).
pub fn is_economy_block(id: BlockId) -> bool {
    matches!(
        id,
        block::VENDOR_BLOCK
            | block::TIP_JAR
            | block::AUCTION_BLOCK
            | block::PLOT_MARKER
            | block::MARKET_BELL
    )
}

/// Empty whatever container state sits at `p` and return it as item stacks,
/// so a blast spills a chest/grave/furnace/dispenser/campfire/drying rack/
/// composter/item frame/latent print (and any face attachments) instead of
/// deleting it. Removes the block-entity. Also returns any cells cleared as a
/// side effect (a campfire's smoke pillar) so the caller can relight/broadcast
/// them. Idempotent on cells that hold nothing.
pub fn spill_block_contents(
    world: &mut World,
    p: (i32, i32, i32),
) -> (Vec<crate::item::ItemStack>, Vec<(i32, i32, i32)>) {
    use crate::item::{Item, ItemStack};
    let (x, y, z) = p;
    let mut spill = Vec::new();
    // A Steam Generator's fuel box (a FurnaceData) spills too (review N1).
    // First: the cleanup_* helpers below remove the block-entity outright.
    if let Some(f) = world.power_device_at_mut(p).and_then(|d| d.fuel.as_mut()) {
        spill.extend([f.input.take(), f.fuel.take(), f.output.take()].into_iter().flatten());
    }
    spill.extend(crate::chest::cleanup_chest(world, x, y, z));
    spill.extend(crate::furnace::cleanup_furnace(world, x, y, z));
    spill.extend(crate::dispenser::cleanup_dispenser(world, x, y, z));
    spill.extend(crate::grave::cleanup_grave(world, x, y, z));
    spill.extend(crate::drying_rack::cleanup_drying_rack(world, x, y, z));
    let mut cleared = Vec::new();
    if world.campfire_at(p).is_some() {
        let (cf_spill, smoke) = crate::campfire::cleanup_campfire(world, x, y, z);
        spill.extend(cf_spill);
        cleared = smoke;
    }
    if let Some(frame) = world.item_frame_at_mut(p) {
        spill.extend(frame.take());
    }
    if let Some(lp) = world.latent_print_at(p).cloned() {
        spill.push(ItemStack { item: Item::Plan(lp.plan_data), count: 1 });
    }
    if let Some(c) = world.composter_at_mut(p) {
        spill.extend(c.input.take());
        spill.extend(c.fuel.take());
        spill.extend(c.output.take());
    }
    for att in world.remove_face_attachments_at(p).into_iter().flatten() {
        spill.push(ItemStack {
            item: crate::blueprint_attach::recovered_item_for(&att),
            count: 1,
        });
    }
    world.remove_block_entity(p);
    spill.retain(|s| s.count > 0);
    (spill, cleared)
}

/// Keep the fluid systems' source bookkeeping in step with a blast: every
/// destroyed cell goes through the same `fluids::notify_block_edit(old, AIR)`
/// path a pickaxe or bucket removal takes, so a blasted water/lava source is
/// dropped (no phantom source that makes a re-poured bucket inert) and every
/// neighbour of the crater is woken to flow in.
pub fn notify_fluids_of_blast(
    water: &mut crate::water::WaterSystem,
    lava: &mut crate::lava::LavaSystem,
    world: &World,
    destroyed: &[((i32, i32, i32), BlockId)],
) {
    for &((x, y, z), old) in destroyed {
        crate::fluids::notify_block_edit(water, lava, world, x, y, z, old, block::AIR);
    }
}

/// Spec 49 gating — may a keg detonate? Only when explosives are enabled for the
/// world AND the play mode permits world edits (Adventure/Spectator are
/// read-only). Electricity is a trigger, not a bypass: this holds however the
/// keg was lit. Pure — the caller passes `play_mode.can_edit_world()`.
pub fn detonation_permitted(explosives_enabled: bool, can_edit_world: bool) -> bool {
    explosives_enabled && can_edit_world
}

/// Positions within the spherical blast whose resistance is below `power` (the
/// blocks a keg destroys). Pure — `block_at` supplies block ids. AIR is skipped;
/// immune blocks (∞ resistance) are never returned.
pub fn blocks_in_blast<F: Fn(i32, i32, i32) -> BlockId>(
    center: (i32, i32, i32),
    radius: f32,
    power: f32,
    block_at: F,
) -> Vec<(i32, i32, i32)> {
    let r = radius.ceil() as i32;
    let r2 = radius * radius;
    let mut out = Vec::new();
    for dx in -r..=r {
        for dy in -r..=r {
            for dz in -r..=r {
                if (dx * dx + dy * dy + dz * dz) as f32 > r2 {
                    continue;
                }
                let p = (center.0 + dx, center.1 + dy, center.2 + dz);
                let id = block_at(p.0, p.1, p.2);
                if id != block::AIR && blast_resistance(id) < power {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// Blast damage to an entity at `target` from a blast at `center`: linear
/// distance falloff to 0 at the radius edge, scaled by `los_factor` (0..=1; 1.0
/// = clear line of sight, lower = shielded). Pure.
pub fn blast_damage(
    center: (f32, f32, f32),
    target: (f32, f32, f32),
    radius: f32,
    max_damage: f32,
    los_factor: f32,
) -> f32 {
    let (dx, dy, dz) = (target.0 - center.0, target.1 - center.1, target.2 - center.2);
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    if dist >= radius {
        return 0.0;
    }
    (max_damage * (1.0 - dist / radius) * los_factor).max(0.0)
}

/// A line-of-sight scale in `[0.3, 1.0]` from `from` to `to`: samples cells along
/// the ray and reduces toward a 0.3 floor as more solid blocks shield the
/// target. Pure — `is_solid` supplies block solidity.
pub fn line_of_sight_factor<F: Fn(i32, i32, i32) -> bool>(
    from: (f32, f32, f32),
    to: (f32, f32, f32),
    is_solid: F,
) -> f32 {
    const STEPS: i32 = 8;
    let mut blocked = 0;
    for i in 1..STEPS {
        let t = i as f32 / STEPS as f32;
        let x = (from.0 + (to.0 - from.0) * t).floor() as i32;
        let y = (from.1 + (to.1 - from.1) * t).floor() as i32;
        let z = (from.2 + (to.2 - from.2) * t).floor() as i32;
        if is_solid(x, y, z) {
            blocked += 1;
        }
    }
    (1.0 - (blocked as f32 / (STEPS - 1) as f32) * 0.7).clamp(0.3, 1.0)
}

/// Outcome of resolving a blast: the cleared cells (for relight + re-mesh +
/// broadcast) and the kegs chain-ignited (excluding the centre).
pub struct BlastOutcome {
    /// `(pos, old_block)` for every cell turned to AIR.
    pub destroyed: Vec<((i32, i32, i32), BlockId)>,
    /// Other kegs caught in the radius whose fuses were lit (sympathetic detonation).
    pub chained: Vec<(i32, i32, i32)>,
    /// Container contents spilled by the blast, `(pos, stack)`. The caller
    /// spawns them as item entities — a blast never deletes stored items.
    pub spilled: Vec<((i32, i32, i32), crate::item::ItemStack)>,
}

/// Resolve a Blasting Keg detonation at `center`: clear every destructible block
/// in the radius to AIR (**no drop, no Proof-of-Play work** — demolition, not
/// mining), and chain-ignite any OTHER keg caught in the radius (a short fuse, so
/// the chain ripples). Returns the changes for the caller to relight, re-mesh,
/// broadcast, and damage entities around. Mutates the world's **blocks only** —
/// never the player inventory, never `total_work`, never a hash.
pub fn resolve_blast(
    world: &mut World,
    center: (i32, i32, i32),
    radius: f32,
    power: f32,
) -> BlastOutcome {
    let positions = blocks_in_blast(center, radius, power, |x, y, z| world.get_block(x, y, z));
    let mut destroyed = Vec::new();
    let mut chained = Vec::new();
    let mut spilled = Vec::new();
    for p in positions {
        let id = world.get_block(p.0, p.1, p.2);
        if id == block::AIR {
            // Already cleared as a side effect this blast (a smoke pillar).
            continue;
        }
        if id == block::BLASTING_KEG && p != center {
            // Sympathetic detonation — light the neighbour's fuse short instead
            // of vaporising it. Lazily ensure it's a PowerDevice keg.
            let lit = world.power_device_at(p).is_some_and(|d| d.charge > 0);
            if !lit {
                if world.power_device_at(p).is_none() {
                    world.insert_power_device(
                        p,
                        crate::power::PowerDeviceData::new(
                            crate::power::PowerDeviceKind::BlastingKeg,
                            crate::meta::Facing::Up,
                        ),
                    );
                }
                if let Some(d) = world.power_device_at_mut(p) {
                    d.charge = CHAIN_FUSE_TICKS;
                }
            }
            chained.push(p);
            continue;
        }
        // Demolition: clear to AIR. The block itself drops nothing (no work,
        // no HMAC strike), but whatever it STORED spills.
        let (stacks, side_cleared) = spill_block_contents(world, p);
        spilled.extend(stacks.into_iter().map(|s| (p, s)));
        world.set_block(p.0, p.1, p.2, block::AIR);
        destroyed.push((p, id));
        for c in side_cleared {
            destroyed.push((c, block::CAMPFIRE_SMOKE));
        }
    }
    BlastOutcome { destroyed, chained, spilled }
}

/// Apply computed blast hits `(player_index, raw_damage)` to player slots.
/// Returns the indices whose hit actually LANDED (armour-gated, i-frame-gated)
/// so the caller can run the on-owner-damage follow-ups (parrot dismount).
pub fn apply_player_blast_damage(
    players: &mut [crate::player_slot::PlayerSlot],
    hits: &[(usize, f32)],
) -> Vec<usize> {
    let mut landed = Vec::new();
    for &(i, d) in hits {
        if let Some(slot) = players.get_mut(i)
            && slot.take_damage_with_armour_from(d, crate::survival::DamageCause::Explosion)
        {
            landed.push(i);
        }
    }
    landed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bedrock_and_satori_are_immune() {
        assert_eq!(blast_resistance(block::BEDROCK), IMMUNE);
        assert_eq!(blast_resistance(block::SATORI_BLOCK), IMMUNE);
        assert!(blast_resistance(block::STONE) < KEG_BLAST_POWER, "stone is destructible");
    }

    #[test]
    fn blocks_in_blast_is_a_sphere_that_skips_immune_and_air() {
        let center = (0, 0, 0);
        let at = |_x: i32, y: i32, _z: i32| {
            // A floor of bedrock at y == -2, stone elsewhere, air above y == 2.
            if y == -2 {
                block::BEDROCK
            } else if y > 2 {
                block::AIR
            } else {
                block::STONE
            }
        };
        let hit = blocks_in_blast(center, BLAST_RADIUS, KEG_BLAST_POWER, at);
        assert!(!hit.is_empty());
        // No bedrock cell, no air cell, and every cell within the radius.
        for &(x, y, z) in &hit {
            assert_ne!(y, -2, "bedrock floor is immune");
            assert!(y <= 2, "air above is skipped");
            let d2 = (x * x + y * y + z * z) as f32;
            assert!(d2 <= BLAST_RADIUS * BLAST_RADIUS);
        }
    }

    #[test]
    fn blast_damage_falls_off_with_distance_and_los() {
        let c = (0.0, 0.0, 0.0);
        let at_centre = blast_damage(c, (0.0, 0.0, 0.0), BLAST_RADIUS, KEG_BLAST_DAMAGE, 1.0);
        assert_eq!(at_centre, KEG_BLAST_DAMAGE, "max at the centre");
        let near = blast_damage(c, (1.0, 0.0, 0.0), BLAST_RADIUS, KEG_BLAST_DAMAGE, 1.0);
        let far = blast_damage(c, (3.5, 0.0, 0.0), BLAST_RADIUS, KEG_BLAST_DAMAGE, 1.0);
        assert!(near > far && far > 0.0, "linear falloff");
        let beyond = blast_damage(c, (5.0, 0.0, 0.0), BLAST_RADIUS, KEG_BLAST_DAMAGE, 1.0);
        assert_eq!(beyond, 0.0, "no damage beyond the radius");
        // Line-of-sight blocked → less damage.
        let shielded = blast_damage(c, (1.0, 0.0, 0.0), BLAST_RADIUS, KEG_BLAST_DAMAGE, 0.4);
        assert!(shielded < near, "blocked line of sight reduces damage");
    }

    #[test]
    fn line_of_sight_factor_clear_vs_blocked() {
        let clear = line_of_sight_factor((0.0, 0.0, 0.0), (4.0, 0.0, 0.0), |_, _, _| false);
        assert_eq!(clear, 1.0, "clear sight = full damage");
        let blocked = line_of_sight_factor((0.0, 0.0, 0.0), (4.0, 0.0, 0.0), |_, _, _| true);
        assert!(blocked < 1.0 && blocked >= 0.3, "solid wall shields, floored at 0.3");
    }

    #[test]
    fn blast_clears_blocks_with_no_drop_and_no_proof_of_play() {
        // THE GUARD: blasting an ore vein demolishes it but accrues NO
        // Proof-of-Play work and (being world-blocks-only) yields no drops.
        let mut w = World::new();
        for x in 0..6 {
            for y in 0..6 {
                for z in 0..6 {
                    w.set_block(x, y, z, block::STONE);
                }
            }
        }
        w.set_block(3, 3, 3, block::IRON_ORE);
        w.set_block(2, 3, 3, block::BLASTING_KEG); // a neighbour to chain-ignite
        assert_eq!(w.total_work, 0);

        let outcome = resolve_blast(&mut w, (3, 3, 3), BLAST_RADIUS, KEG_BLAST_POWER);

        assert_eq!(w.get_block(3, 3, 3), block::AIR, "the ore is demolished");
        assert_eq!(w.total_work, 0, "a blast accrues NO Proof-of-Play work — demolition, not mining");
        assert!(!outcome.destroyed.is_empty(), "blocks were cleared");
        // The neighbour keg chain-ignited (short fuse) rather than vanishing.
        assert!(outcome.chained.contains(&(2, 3, 3)), "neighbour keg chain-ignited");
        assert_eq!(w.get_block(2, 3, 3), block::BLASTING_KEG, "chained keg still stands (it detonates next)");
        assert_eq!(
            w.power_device_at((2, 3, 3)).unwrap().charge,
            CHAIN_FUSE_TICKS,
            "chained keg has a short fuse"
        );
    }

    #[test]
    fn detonation_gating_requires_explosives_enabled_and_world_edits() {
        // Survival/Creative with explosives on → boom. Disabled OR a read-only
        // play mode (Adventure/Spectator can_edit_world == false) → no-op.
        assert!(detonation_permitted(true, true), "enabled + editable → detonate");
        assert!(!detonation_permitted(false, true), "explosives disabled → no-op");
        assert!(!detonation_permitted(true, false), "Adventure/Spectator → no-op");
        assert!(!detonation_permitted(false, false));
    }

    #[test]
    fn a_bedrock_wall_survives_a_blast() {
        let mut w = World::new();
        for x in 0..6 {
            for y in 0..6 {
                for z in 0..6 {
                    w.set_block(x, y, z, block::BEDROCK);
                }
            }
        }
        let outcome = resolve_blast(&mut w, (3, 3, 3), BLAST_RADIUS, KEG_BLAST_POWER);
        assert!(outcome.destroyed.is_empty(), "bedrock is immune — nothing cleared");
        assert_eq!(w.get_block(3, 3, 3), block::BEDROCK, "bedrock holds at the centre");
    }

    // --- Audit 2026-09-27: blast vs block-entities, obsidian, fluids ---

    fn stone_cube(w: &mut World) {
        for x in 0..7 {
            for y in 0..7 {
                for z in 0..7 {
                    w.set_block(x, y, z, block::STONE);
                }
            }
        }
    }

    #[test]
    fn obsidian_is_blast_immune() {
        assert_eq!(blast_resistance(block::OBSIDIAN), IMMUNE);
        let mut w = World::new();
        stone_cube(&mut w);
        w.set_block(3, 4, 3, block::OBSIDIAN);
        resolve_blast(&mut w, (3, 3, 3), BLAST_RADIUS, KEG_BLAST_POWER);
        assert_eq!(w.get_block(3, 4, 3), block::OBSIDIAN, "an obsidian vault holds");
    }

    #[test]
    fn economy_blocks_and_their_entities_survive_a_blast() {
        for id in [
            block::VENDOR_BLOCK,
            block::TIP_JAR,
            block::AUCTION_BLOCK,
            block::PLOT_MARKER,
            block::MARKET_BELL,
        ] {
            assert_eq!(blast_resistance(id), IMMUNE, "economy block {id} is immune");
        }
        let mut w = World::new();
        stone_cube(&mut w);
        w.set_block(4, 3, 3, block::VENDOR_BLOCK);
        w.insert_vendor(
            (4, 3, 3),
            crate::vendor::VendorData { stock: 7, escrow_sats: 50, ..Default::default() },
        );
        let out = resolve_blast(&mut w, (3, 3, 3), BLAST_RADIUS, KEG_BLAST_POWER);
        assert_eq!(w.get_block(4, 3, 3), block::VENDOR_BLOCK, "the vendor stands");
        let v = w.vendor_at((4, 3, 3)).expect("the vendor's entity survives");
        assert_eq!((v.stock, v.escrow_sats), (7, 50), "stock + escrow untouched");
        assert!(!out.destroyed.iter().any(|&(p, _)| p == (4, 3, 3)));
    }

    #[test]
    fn a_blasted_steam_generator_spills_its_fuel() {
        use crate::item::{ItemStack, MaterialId};
        let mut w = World::new();
        let mut d = crate::power::PowerDeviceData::new(
            crate::power::PowerDeviceKind::SteamGenerator,
            crate::meta::Facing::North,
        );
        d.fuel = Some(crate::furnace::FurnaceData {
            fuel: Some(ItemStack::new_material(MaterialId::Coal, 7)),
            ..Default::default()
        });
        w.insert_power_device((2, 2, 2), d);
        let (spill, _) = spill_block_contents(&mut w, (2, 2, 2));
        assert_eq!(spill.iter().map(|s| s.count as u32).sum::<u32>(), 7, "the fuel spills");
        assert!(w.power_device_at((2, 2, 2)).is_none(), "the entity is removed");
    }

    #[test]
    fn a_blasted_chest_spills_its_contents_instead_of_deleting_them() {
        use crate::item::{ItemStack, MaterialId};
        let mut w = World::new();
        stone_cube(&mut w);
        w.set_block(4, 3, 3, block::CHEST);
        let mut c = crate::chest::ChestData::new();
        c.slots[0] = Some(ItemStack::new_material(MaterialId::IronIngot, 12));
        c.slots[9] = Some(ItemStack::new_material(MaterialId::Coal, 5));
        w.insert_chest((4, 3, 3), c);

        let out = resolve_blast(&mut w, (3, 3, 3), BLAST_RADIUS, KEG_BLAST_POWER);

        assert_eq!(w.get_block(4, 3, 3), block::AIR, "the chest itself is demolished");
        assert!(w.chest_at((4, 3, 3)).is_none(), "no orphaned ChestData");
        let spilled: u32 = out
            .spilled
            .iter()
            .filter(|(p, _)| *p == (4, 3, 3))
            .map(|(_, s)| s.count as u32)
            .sum();
        assert_eq!(spilled, 17, "every stored item spills");
        assert_eq!(w.total_work, 0, "spilling is not mining");
    }

    #[test]
    fn a_blasted_water_source_is_dropped_and_its_neighbours_wake() {
        let mut w = World::new();
        stone_cube(&mut w);
        let mut water = crate::water::WaterSystem::new();
        let mut lava = crate::lava::LavaSystem::new();
        // A water cell inside the blast, and one just outside it.
        w.set_block(3, 3, 3, block::WATER);
        water.add_source(3, 3, 3);
        let _ = water.tick_spread(&mut w);

        let out = resolve_blast(&mut w, (3, 3, 3), BLAST_RADIUS, KEG_BLAST_POWER);
        notify_fluids_of_blast(&mut water, &mut lava, &w, &out.destroyed);

        assert!(!water.is_source(3, 3, 3), "no phantom source left in the crater");
        // A bucket poured back into the crater registers and flows.
        w.set_block(3, 3, 3, block::WATER);
        crate::fluids::notify_block_edit(&mut water, &mut lava, &w, 3, 3, 3, block::AIR, block::WATER);
        assert!(water.is_source(3, 3, 3));
        for _ in 0..8 {
            let _ = water.tick_spread(&mut w);
        }
        assert_eq!(w.get_block(3, 2, 3), block::WATER, "re-poured water flows (not inert)");
    }

    // --- apply_player_blast_damage (wave-hardening backlog, 2026-07-11) ---

    #[test]
    fn blast_damage_routes_through_equipped_armour() {
        // Keg damage used to bypass `take_damage_with_armour_from` — full iron
        // must reduce a 10.0 blast hit to 4.0 landed (the Spec 28e 60%
        // reduction the melee path already applies), and wear each piece.
        use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot};
        let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
        for s in [
            ArmourSlot::Helmet, ArmourSlot::Chestplate,
            ArmourSlot::Leggings, ArmourSlot::Boots,
        ] {
            slot.armour_slots[s as usize] = Some(ArmourItem::new(s, ArmourMaterial::Iron));
        }
        let start = slot.combat.health;
        let mut players = vec![slot];

        let landed = apply_player_blast_damage(&mut players, &[(0, 10.0)]);

        assert_eq!(landed, vec![0], "the armoured hit still lands");
        assert!(
            (players[0].combat.health - (start - 4.0)).abs() < 1e-3,
            "full iron: 10.0 raw -> 4.0 landed, got {} off {}",
            players[0].combat.health, start
        );
        let helmet = players[0].armour_slots[ArmourSlot::Helmet as usize].as_ref().unwrap();
        assert!(
            helmet.durability < crate::armour::max_durability(ArmourSlot::Helmet, ArmourMaterial::Iron),
            "blast damage wears armour like any other hit"
        );
    }

    #[test]
    fn blast_damage_on_a_dead_player_does_not_land() {
        let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
        slot.combat.dead = true;
        let mut players = vec![slot];

        let landed = apply_player_blast_damage(&mut players, &[(0, 10.0)]);

        assert!(landed.is_empty(), "a dead player takes no blast follow-ups");
    }
}
