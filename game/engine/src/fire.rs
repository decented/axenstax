//! Fire spread (2026-07-04 gap-fill wave).
//!
//! A `FIRE` cell is a scheduled event stream (the leaf-decay idiom, not the
//! water source/retract idiom): each active cell holds a next-event tick and
//! an age. On each event the fire may be doused by rain, lights any adjacent
//! Blasting Keg fuse, consumes one adjacent flammable block (replacing it with
//! more fire — spread and fuel burn are the same act, gated by the world's
//! `fire_spread_enabled` toggle), and eventually burns out: immediately-ish
//! with no fuel adjacent, at the age cap regardless.
//!
//! Runs on BOTH the client `GameState::tick` and the authoritative
//! `GameServer::tick` (the `fluids.rs` parity discipline). Deterministic —
//! every roll is a position+tick hash, no RNG, so client and server agree in
//! single-player and the server is authoritative in multiplayer.

use ahash::AHashMap;

use crate::block;
use crate::world::World;

/// Ticks between events for one fire cell (MC's 30-tick fire tick).
pub const FIRE_EVENT_INTERVAL: u64 = 30;
/// A fire with NO adjacent fuel goes out after this many events (~1.5–3 s).
pub const NO_FUEL_MAX_AGE: u8 = 1;
/// Absolute age cap — even a fuelled fire dies after this many events, so a
/// lone ignition can never burn forever in place.
pub const MAX_AGE: u8 = 12;
/// Safety valve: total simultaneously-active fire cells. When the cap is hit,
/// fire stops spreading (existing cells still burn out). A forest fire
/// self-limits instead of eating the loaded world.
pub const MAX_ACTIVE_FIRE: usize = 256;
/// Percent chance per event that rain douses the cell.
pub const RAIN_DOUSE_PCT: u64 = 40;
/// Events processed per tick call, so a big blaze can't stall a frame.
const EVENT_BUDGET: usize = 64;
/// One-in-N chance per settled lava cell per fluid tick to ignite a
/// neighbouring exposed flammable.
const LAVA_IGNITE_ONE_IN: u64 = 12;

const NEIGHBOURS: [(i32, i32, i32); 6] =
    [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)];

#[derive(Clone, Copy, Debug)]
struct FireCell {
    next_event: u64,
    age: u8,
}

/// Deterministic position+tick hash (the weather-roll idiom).
fn hash(x: i32, y: i32, z: i32, tick: u64) -> u64 {
    let mut h = (x as u64 & 0xFFFF) | ((y as u64 & 0xFFFF) << 16) | ((z as u64 & 0xFFFF) << 32);
    h ^= tick.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^ (h >> 33)
}

pub struct FireSystem {
    cells: AHashMap<(i32, i32, i32), FireCell>,
}

impl Default for FireSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl FireSystem {
    pub fn new() -> Self {
        Self { cells: AHashMap::new() }
    }

    /// Number of live fire cells (cap bookkeeping + tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn active(&self) -> usize {
        self.cells.len()
    }

    /// Positions of every burning cell (particle ambience — smoke/embers).
    pub fn positions(&self) -> impl Iterator<Item = (i32, i32, i32)> + '_ {
        self.cells.keys().copied()
    }

    /// Set `pos` alight: writes the FIRE block and schedules its first event.
    /// The target must currently be AIR (ignite an empty cell above/beside
    /// fuel) or a flammable block (consume it directly). Refuses — returning
    /// `false` — beyond [`MAX_ACTIVE_FIRE`] or on any other block.
    pub fn ignite(&mut self, world: &mut World, x: i32, y: i32, z: i32, now: u64) -> bool {
        if self.cells.len() >= MAX_ACTIVE_FIRE {
            return false;
        }
        let b = world.get_block(x, y, z);
        if b != block::AIR && !block::flammable(b) {
            return false;
        }
        world.set_block(x, y, z, block::FIRE);
        world.set_meta((x, y, z), 0);
        // Stagger the first event by a position hash so a row of ignitions
        // doesn't strobe in lockstep.
        let jitter = hash(x, y, z, now) % 10;
        self.cells.insert(
            (x, y, z),
            FireCell { next_event: now + FIRE_EVENT_INTERVAL + jitter, age: 0 },
        );
        true
    }

    /// Re-adopt persisted FIRE blocks after a load / chunk stream (the lava
    /// `register_column_sources` idiom).
    pub fn register_column_fires(&mut self, cx: i32, cz: i32, world: &World, now: u64) {
        let cs = crate::chunk::CHUNK_SIZE as i32;
        let (base_x, base_z) = (cx * cs, cz * cs);
        for lx in 0..cs {
            for lz in 0..cs {
                for y in 0..6 * cs {
                    let (wx, wz) = (base_x + lx, base_z + lz);
                    if world.get_block(wx, y, wz) == block::FIRE {
                        self.cells.entry((wx, y, wz)).or_insert(FireCell {
                            next_event: now + FIRE_EVENT_INTERVAL,
                            age: 0,
                        });
                    }
                }
            }
        }
    }

    /// Lava starts fires: for each settled lava cell, a small deterministic
    /// chance to ignite an adjacent AIR cell that itself touches a flammable
    /// block. Call with the dirty list the lava tick returned.
    pub fn ignite_from_lava(
        &mut self,
        world: &mut World,
        lava_dirty: &[(i32, i32, i32)],
        now: u64,
    ) -> Vec<(i32, i32, i32)> {
        let mut lit = Vec::new();
        for &(x, y, z) in lava_dirty {
            if world.get_block(x, y, z) != block::LAVA {
                continue;
            }
            if !hash(x, y, z, now).is_multiple_of(LAVA_IGNITE_ONE_IN) {
                continue;
            }
            'cell: for &(dx, dy, dz) in &NEIGHBOURS {
                let (ax, ay, az) = (x + dx, y + dy, z + dz);
                // Spec 02 §7.5 — never light a fire inside a column that is not
                // present (evicted, dropped or never loaded).
                if !world.is_column_present_at(ax, az) || world.get_block(ax, ay, az) != block::AIR {
                    continue;
                }
                for &(ex, ey, ez) in &NEIGHBOURS {
                    if block::flammable(world.get_block(ax + ex, ay + ey, az + ez))
                        && self.ignite(world, ax, ay, az, now)
                    {
                        lit.push((ax, ay, az));
                        break 'cell;
                    }
                }
            }
        }
        lit
    }

    /// Advance every due fire cell. Returns dirty positions (block changed)
    /// for re-meshing / lighting / broadcast. `raining` is the caller's
    /// weather flag; `spread_enabled` is the world's `fire_spread_enabled`.
    pub fn tick(
        &mut self,
        world: &mut World,
        now: u64,
        raining: bool,
        spread_enabled: bool,
    ) -> Vec<(i32, i32, i32)> {
        let mut dirty: Vec<(i32, i32, i32)> = Vec::new();
        let due: Vec<(i32, i32, i32)> = self
            .cells
            .iter()
            .filter(|(_, c)| c.next_event <= now)
            .map(|(p, _)| *p)
            .take(EVENT_BUDGET)
            .collect();

        for pos in due {
            let (x, y, z) = pos;
            // Cell went stale (dug out, overwritten, doused externally), or its
            // column was unloaded (Spec 02 §7.5 — reads go through to evicted
            // chunks, so without this it would keep burning there; the FIRE
            // block stays and `register_column_fires` re-adopts it on restore).
            if !world.is_column_present_at(x, z) || world.get_block(x, y, z) != block::FIRE {
                self.cells.remove(&pos);
                continue;
            }
            let age = self.cells[&pos].age;

            // Rain douses.
            if raining && hash(x, y, z, now) % 100 < RAIN_DOUSE_PCT {
                world.set_block(x, y, z, block::AIR);
                dirty.push(pos);
                self.cells.remove(&pos);
                continue;
            }

            // Light any adjacent Blasting Keg's fuse (the flint-and-steel /
            // electrical-trigger idiom; `detonation_permitted` still gates the
            // actual blast at detonate time).
            for &(dx, dy, dz) in &NEIGHBOURS {
                let npos = (x + dx, y + dy, z + dz);
                if world.get_block(npos.0, npos.1, npos.2) == block::BLASTING_KEG
                    && let Some(d) = world.power_device_at_mut(npos)
                        && d.kind == crate::power::PowerDeviceKind::BlastingKeg && d.charge == 0 {
                            d.charge = crate::power::KEG_FUSE_TICKS;
                        }
            }

            // Spread = consume one adjacent flammable (hash-picked), becoming
            // fire there. Gated by the world toggle and the active-cell cap.
            let mut fuel: Vec<(i32, i32, i32)> = Vec::new();
            for &(dx, dy, dz) in &NEIGHBOURS {
                let np = (x + dx, y + dy, z + dz);
                if world.is_column_present_at(np.0, np.2)
                    && block::flammable(world.get_block(np.0, np.1, np.2))
                {
                    fuel.push(np);
                }
            }
            if spread_enabled && !fuel.is_empty() && self.cells.len() < MAX_ACTIVE_FIRE {
                let pick = fuel[(hash(x, y, z, now ^ 0xF1FE) % fuel.len() as u64) as usize];
                world.set_block(pick.0, pick.1, pick.2, block::FIRE);
                world.set_meta(pick, 0);
                dirty.push(pick);
                let jitter = hash(pick.0, pick.1, pick.2, now) % 10;
                self.cells.insert(
                    pick,
                    FireCell { next_event: now + FIRE_EVENT_INTERVAL + jitter, age: 0 },
                );
            }

            // Burn out: quickly with no fuel, at the age cap regardless.
            let starving = fuel.is_empty();
            if (starving && age >= NO_FUEL_MAX_AGE) || age >= MAX_AGE {
                world.set_block(x, y, z, block::AIR);
                dirty.push(pos);
                self.cells.remove(&pos);
                continue;
            }

            if let Some(c) = self.cells.get_mut(&pos) {
                c.age = age.saturating_add(1);
                c.next_event = now + FIRE_EVENT_INTERVAL + hash(x, y, z, now ^ 0xA9) % 10;
            }
        }
        dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::world::World;

    fn stone_pad(w: &mut World) {
        for x in -4..=4 {
            for z in -4..=4 {
                w.set_block(x, 9, z, block::STONE);
            }
        }
    }

    /// Run enough ticks for several fire events.
    fn run(fs: &mut FireSystem, w: &mut World, from: u64, events: u64, raining: bool, spread: bool) {
        for t in from..from + events * (FIRE_EVENT_INTERVAL + 10) {
            fs.tick(w, t, raining, spread);
        }
    }

    #[test]
    fn fire_consumes_adjacent_flammable_and_spreads() {
        let mut w = World::new();
        stone_pad(&mut w);
        w.set_block(1, 10, 0, block::OAK_PLANKS);
        let mut fs = FireSystem::new();
        assert!(fs.ignite(&mut w, 0, 10, 0, 0), "ignite the air cell beside the planks");
        // Sample per tick: the planks must pass through FIRE (consumed by the
        // spread) before the whole blaze starves and burns out to AIR.
        let mut planks_caught_fire = false;
        for t in 1..5 * (FIRE_EVENT_INTERVAL + 10) {
            fs.tick(&mut w, t, false, true);
            if w.get_block(1, 10, 0) == block::FIRE {
                planks_caught_fire = true;
            }
        }
        assert!(planks_caught_fire, "the planks were consumed by fire (spread)");
        assert_ne!(
            w.get_block(1, 10, 0),
            block::OAK_PLANKS,
            "the fuel block did not survive"
        );
        assert_eq!(fs.active(), 0, "with all fuel spent, the blaze burnt itself out");
    }

    #[test]
    fn fire_burns_out_on_bare_stone() {
        let mut w = World::new();
        stone_pad(&mut w);
        let mut fs = FireSystem::new();
        assert!(fs.ignite(&mut w, 0, 10, 0, 0));
        run(&mut fs, &mut w, 1, 4, false, true);
        assert_eq!(w.get_block(0, 10, 0), block::AIR, "no fuel → burns out");
        assert_eq!(fs.active(), 0, "cell bookkeeping cleared");
    }

    #[test]
    fn fire_eventually_dies_even_with_endless_fuel_ring() {
        // Age cap: a fire cell surrounded by fuel still dies by MAX_AGE (its
        // spawned children keep the blaze going instead — bounded by the cap).
        let mut w = World::new();
        stone_pad(&mut w);
        // Refill fuel around the fire every tick so it never starves.
        let mut fs = FireSystem::new();
        assert!(fs.ignite(&mut w, 0, 10, 0, 0));
        let mut alive_events = 0u32;
        for t in 1..(MAX_AGE as u64 + 4) * (FIRE_EVENT_INTERVAL + 10) {
            // Keep one neighbour flammable but NOT convertible: replace any
            // spread immediately with fresh planks and douse child fires.
            fs.tick(&mut w, t, false, true);
            for &(dx, dz) in &[(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                if w.get_block(dx, 10, dz) != block::OAK_PLANKS {
                    w.set_block(dx, 10, dz, block::OAK_PLANKS);
                }
            }
            if w.get_block(0, 10, 0) == block::FIRE {
                alive_events += 1;
            } else {
                break;
            }
        }
        assert_ne!(alive_events, 0, "fire lived at least one event");
        assert_ne!(
            w.get_block(0, 10, 0),
            block::FIRE,
            "age cap put the original cell out"
        );
    }

    #[test]
    fn rain_douses_fire() {
        let mut w = World::new();
        stone_pad(&mut w);
        w.set_block(1, 10, 0, block::OAK_PLANKS); // fuelled, so only rain kills it early
        let mut fs = FireSystem::new();
        assert!(fs.ignite(&mut w, 0, 10, 0, 0));
        // The 40% hash roll varies with `now` — within a handful of events the
        // douse must land (deterministically, for this position/seed stream).
        run(&mut fs, &mut w, 1, 8, true, false);
        assert_eq!(w.get_block(0, 10, 0), block::AIR, "rain doused the fire");
    }

    #[test]
    fn spread_toggle_off_protects_blocks_but_still_burns_out() {
        let mut w = World::new();
        stone_pad(&mut w);
        w.set_block(1, 10, 0, block::OAK_PLANKS);
        let mut fs = FireSystem::new();
        assert!(fs.ignite(&mut w, 0, 10, 0, 0));
        run(&mut fs, &mut w, 1, MAX_AGE as u64 + 3, false, false);
        assert_eq!(w.get_block(1, 10, 0), block::OAK_PLANKS, "toggle off: fuel untouched");
        assert_eq!(w.get_block(0, 10, 0), block::AIR, "fire still burnt out");
    }

    #[test]
    fn fire_lights_adjacent_keg_fuse() {
        let mut w = World::new();
        stone_pad(&mut w);
        w.set_block(1, 10, 0, block::BLASTING_KEG);
        w.insert_power_device(
            (1, 10, 0),
            crate::power::PowerDeviceData::new(
                crate::power::PowerDeviceKind::BlastingKeg,
                crate::meta::Facing::North,
            ),
        );
        let mut fs = FireSystem::new();
        assert!(fs.ignite(&mut w, 0, 10, 0, 0));
        run(&mut fs, &mut w, 1, 2, false, true);
        let d = w.power_device_at((1, 10, 0)).expect("keg device present");
        assert_eq!(
            d.charge,
            crate::power::KEG_FUSE_TICKS,
            "fire lit the keg fuse (detonation gate still applies at blast time)"
        );
    }

    #[test]
    fn active_cap_refuses_new_ignitions() {
        let mut w = World::new();
        let mut fs = FireSystem::new();
        let mut placed = 0usize;
        // Try to light far more cells than the cap allows.
        'outer: for x in 0..40 {
            for z in 0..40 {
                if fs.ignite(&mut w, x, 30, z, 0) {
                    placed += 1;
                } else {
                    break 'outer;
                }
            }
        }
        assert_eq!(placed, MAX_ACTIVE_FIRE, "cap stops the {placed}th ignition");
        assert!(!fs.ignite(&mut w, 100, 30, 100, 0), "over cap refuses");
    }

    #[test]
    fn ignite_refuses_solid_non_flammable_targets() {
        let mut w = World::new();
        w.set_block(0, 10, 0, block::STONE);
        let mut fs = FireSystem::new();
        assert!(!fs.ignite(&mut w, 0, 10, 0, 0), "can't set stone itself alight");
        assert_eq!(w.get_block(0, 10, 0), block::STONE);
    }

    #[test]
    fn lava_ignites_neighbouring_exposed_wood() {
        let mut w = World::new();
        stone_pad(&mut w);
        w.set_block(0, 10, 0, block::LAVA);
        w.set_block(2, 10, 0, block::OAK_LOG);
        // (1,10,0) is the AIR cell between lava and log.
        let mut fs = FireSystem::new();
        let mut lit = Vec::new();
        // The one-in-N roll depends on the tick — sweep ticks until it lands.
        for t in 0..200u64 {
            lit = fs.ignite_from_lava(&mut w, &[(0, 10, 0)], t);
            if !lit.is_empty() {
                break;
            }
        }
        assert_eq!(lit, vec![(1, 10, 0)], "the air gap beside the log caught fire");
        assert_eq!(w.get_block(1, 10, 0), block::FIRE);
    }

    /// Spec 02 §7.5 — fire never spreads into an evicted column, and a fire
    /// cell inside one stops ticking (the column re-registers on restore).
    #[test]
    fn fire_does_not_spread_into_or_burn_in_an_evicted_column() {
        let mut w = World::new();
        stone_pad(&mut w);
        for z in -1..=1 {
            w.set_block(16, 9, z, block::STONE);
        }
        w.set_block(16, 10, 0, block::OAK_PLANKS);
        let mut fs = FireSystem::new();
        assert!(fs.ignite(&mut w, 15, 10, 0, 0));
        // A fire inside the column that is about to be evicted.
        assert!(fs.ignite(&mut w, 16, 10, 1, 0));
        assert!(w.evict_column(1, 0));
        run(&mut fs, &mut w, 1, 6, false, true);
        assert_eq!(w.get_block(16, 10, 0), block::OAK_PLANKS, "no spread into evicted planks");
        assert_eq!(w.get_block(16, 10, 1), block::FIRE, "evicted fire is frozen, not burnt out");
        assert_eq!(fs.active(), 0, "evicted fire cell dropped from the sim");
    }
}
