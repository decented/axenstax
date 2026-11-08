//! The volatile sections of the checkpoint: what a save doesn't hold, or holds
//! only as of its last write, and that the required digest sections depend on.
//!
//! - **Clock:** the 2a authority clock (with `calendar_id`), plus what the
//!   `clocks` digest reads and no save carries: the day-clock step, the falling
//!   cadence, the rolled strike and the world's active ticks and work.
//! - **Modes:** what a dedicated server sets when it is built, so a restored one
//!   runs the same passes.
//! - **Streamer:** its settled anchors, one a player.
//! - **Sims:** the phase of lava, the clock of leaf decay, and each lava queue's cursor.
//!
//! So the image is bounded by constants and the player count, whatever the world holds.
//! The rest is the same kind of state and grows with the world, so it is stored records
//! bound through the manifest (`records.rs`), read by these same readers:
//!
//! - **Layout:** which chunks are live, which are evicted and which columns count
//!   as loaded. A load holds every saved chunk live, so this is what puts the
//!   stream-out state back (handoff §6, choice 11). Bytes stay in the records.
//! - **Water:** sources as the difference from a rescan (an ocean's whole set is far past
//!   the cap), and its queues in order.
//! - **Lava** (`lava_records.rs`): the same, in forms a drain doesn't rewrite.
//! - **Exposure clocks:** each cell's (in no save; 20 B a cell, and nothing prunes it).
//! - **Cell sims:** fire with its timers (`MAX_ACTIVE_FIRE` holds a run's ignitions, but a
//!   load re-adopts every fire block a column kept when it went out of memory), the
//!   leaf-decay queues, the update scheduler, the power flood and the rubber-tap cooldowns
//!   (a load rebuilds those at its own tick).
//! - **Player slots** (`players.rs`): a slot's size follows what it carries, and a plan in
//!   an inventory holds a captured building's cells.
//! - **Corpses** waiting to be swept (`deaths.rs`).
//!
//! Everything else in handoff §5 is the restoration ledger (Spec 01 §4.1.5).

use ahash::AHashSet;

use super::codec::{Malformed, Reader, Writer};
use super::records::StateRecords;
use crate::block_update::{ScheduleKind, ScheduledUpdate};
use crate::fluids::{FluidCheckpoint, LavaCheckpoint, PositionedQueue};
use super::lava_records;
use crate::server::{GameServer, ServerPlayer};
use crate::server_stream::ColumnStreamer;
use crate::sim_clock::SavedClock;
use crate::world::{ChunkColumnLayout, LAYOUT_COLUMN_SECTIONS};

type Pos = (i32, i32, i32);

pub(super) struct Volatile {
    clock: SavedClock,
    world_time_step: u32,
    falling_tick_counter: u32,
    pending_strike: Option<u64>,
    total_ticks: u64,
    total_work: u64,
    modes: Modes,
    streamer: Option<ColumnStreamer>,
    loaded: AHashSet<(i32, i32)>,
    layout: Vec<ChunkColumnLayout>,
    fire: Vec<(Pos, u64, u8)>,
    water: FluidCheckpoint,
    lava: LavaCheckpoint,
    lava_slow: u8,
    lava_spread_head: u64,
    lava_retract_head: u64,
    leaf: crate::leaf_decay::LeafDigestView,
    scheduler_pending: Vec<Pos>,
    scheduler_due: Vec<(u64, ScheduledUpdate)>,
    energised: Vec<(Pos, u8)>,
    mains: Vec<Pos>,
    tapped: Vec<(Pos, u64)>,
    pop_exposure: Vec<(Pos, u64)>,
    pub(super) players: Vec<ServerPlayer>,
    deaths: Vec<super::deaths::Corpse>,
}

struct Modes {
    simulates_block_machines: bool,
    animal_life_simulated: bool,
    rolls_lightning: bool,
    column_refill_per_tick: u32,
    fire_spread_enabled: bool,
    explosives_enabled: bool,
    render_distance: i32,
    play_mode: String,
    spawn_column: (i32, i32),
    pop_epoch: u32,
    next_attach_gen: u32,
}

fn sorted<T: Ord>(mut v: Vec<T>) -> Vec<T> {
    v.sort_unstable();
    v
}

pub(super) fn write_list(w: &mut Writer, cells: &[Pos]) {
    w.count(cells.len());
    for &p in cells {
        w.pos(p);
    }
}

fn read_list(r: &mut Reader<'_>) -> Result<Vec<Pos>, Malformed> {
    let n = r.count(12)?;
    (0..n).map(|_| r.pos()).collect()
}

/// A list that is written sorted, refused otherwise: each set has one form.
pub(super) fn read_sorted_list(r: &mut Reader<'_>) -> Result<Vec<Pos>, Malformed> {
    let cells = read_list(r)?;
    strictly_increasing(&cells, |c| *c)?;
    Ok(cells)
}

fn strictly_increasing<T, K: Ord>(items: &[T], key: impl Fn(&T) -> K) -> Result<(), Malformed> {
    if items.windows(2).all(|p| key(&p[0]) < key(&p[1])) {
        Ok(())
    } else {
        Err(Malformed("a set that isn't in its one sorted order"))
    }
}

pub(super) fn write(w: &mut Writer, s: &GameServer) -> Result<(), String> {
    write_clock(w, s);
    write_modes(w, s);
    write_streamer(w, s);
    write_sims(w, s);
    Ok(())
}

/// The state that is a stored record: what `persist_authority` writes, and what a
/// checkpoint binds the digests of.
pub(super) fn state_records(s: &GameServer) -> Result<StateRecords, String> {
    let mut layout = Writer::uncapped();
    write_layout(&mut layout, s)?;
    let (lava_extra, lava_missing) = s.lava.sources_from_rescan(&s.world, &s.loaded_columns);
    let mut water = Writer::uncapped();
    write_fluid(&mut water, &s.water.checkpoint_view(&s.world, &s.loaded_columns));
    let mut cells = Writer::uncapped();
    write_cells(&mut cells, s);
    let mut players = Writer::uncapped();
    super::players::write_all(&mut players, &s.players)?;
    let mut corpses = Writer::uncapped();
    super::deaths::write(&mut corpses, s)?;
    let mut exposure = Writer::uncapped();
    let exposed = sorted(s.world.pop_exposure.iter().map(|(p, t)| (*p, *t)).collect());
    exposure.count(exposed.len());
    for (pos, tick) in exposed {
        exposure.pos(pos);
        exposure.u64(tick);
    }
    Ok(StateRecords {
        layout: layout.into_bytes(),
        lava_spread: lava_records::spread_pieces(s.lava.spread_queue())?,
        lava_retract: lava_records::retract_pieces(s.lava.retract_queue())?,
        lava_sources: lava_records::source_sections(&lava_extra, &lava_missing),
        exposure: exposure.into_bytes(),
        water: water.into_bytes(),
        cells: cells.into_bytes(),
        players: players.into_bytes(),
        corpses: corpses.into_bytes(),
    })
}

/// Each volatile section's encoded size, for the size bench. The writer counts
/// past the cap, so this reports a world that doesn't fit too.
#[cfg(test)]
pub(super) fn section_sizes(s: &GameServer) -> Result<Vec<(&'static str, usize)>, String> {
    fn size(write: impl FnOnce(&mut Writer) -> Result<(), String>) -> Result<usize, String> {
        let mut w = Writer::new();
        write(&mut w)?;
        Ok(w.len())
    }
    let water = s.water.checkpoint_view(&s.world, &s.loaded_columns);
    let (lava_extra, lava_missing) = s.lava.sources_from_rescan(&s.world, &s.loaded_columns);
    let state = state_records(s)?;
    let bytes_of = |pieces: &[(u32, Vec<u8>)]| pieces.iter().map(|(_, piece)| piece.len()).sum::<usize>();
    Ok(vec![
        ("clock", size(|w| { write_clock(w, s); Ok(()) })?),
        ("modes", size(|w| { write_modes(w, s); Ok(()) })?),
        ("sims", size(|w| { write_sims(w, s); Ok(()) })?),
        ("record: layout", state.layout.len()),
        ("record: water", state.water.len()),
        ("record:   water sources", 12 * (water.extra.len() + water.missing.len())),
        ("record: cell sims", state.cells.len()),
        ("record: players", state.players.len()),
        ("record: corpses", state.corpses.len()),
        ("record: lava spread pieces", bytes_of(&state.lava_spread)),
        ("record: lava spread pieces, how many", state.lava_spread.len()),
        ("record: lava retract pieces", bytes_of(&state.lava_retract)),
        ("record: lava source sections", state.lava_sources.iter().map(|(_, section)| section.len()).sum()),
        ("record: lava source sections, how many", state.lava_sources.len()),
        ("record:   lava sources", 12 * (lava_extra.len() + lava_missing.len())),
        ("record: exposure", state.exposure.len()),
        ("record:   exposed cells", s.world.pop_exposure.len()),
    ])
}

/// Where each section starts in a checkpoint's bytes, for tests that edit one
/// and reseal.
#[cfg(test)]
pub(crate) struct SectionOffsets {
    pub modes: usize,
    pub streamer: usize,
    pub sims: usize,
}

#[cfg(test)]
pub(super) fn section_offsets(s: &GameServer, start: usize) -> Result<SectionOffsets, String> {
    let mut w = Writer::new();
    write_clock(&mut w, s);
    let modes = start + w.len();
    write_modes(&mut w, s);
    let streamer = start + w.len();
    write_streamer(&mut w, s);
    Ok(SectionOffsets { modes, streamer, sims: start + w.len() })
}

pub(super) fn read(r: &mut Reader<'_>) -> Result<Volatile, Malformed> {
    let clock = read_clock(r)?;
    let world_time_step = r.u32()?;
    let falling_tick_counter = r.u32()?;
    let pending_strike = r.opt(|r| r.u64())?;
    let total_ticks = r.u64()?;
    let total_work = r.u64()?;
    let modes = read_modes(r)?;
    let streamer = read_streamer(r)?;
    let lava_slow = r.u8()?;
    let leaf_tick = r.u64()?;
    let lava_spread_head = r.u64()?;
    let lava_retract_head = r.u64()?;
    let no_fluid = || FluidCheckpoint { extra: Vec::new(), missing: Vec::new(), spread: Vec::new(), retract: Vec::new() };
    Ok(Volatile {
        clock,
        world_time_step,
        falling_tick_counter,
        pending_strike,
        total_ticks,
        total_work,
        modes,
        streamer,
        loaded: AHashSet::new(),
        layout: Vec::new(),
        fire: Vec::new(),
        water: no_fluid(),
        lava: LavaCheckpoint::default(),
        lava_slow,
        lava_spread_head,
        lava_retract_head,
        leaf: (Vec::new(), Vec::new(), leaf_tick),
        scheduler_pending: Vec::new(),
        scheduler_due: Vec::new(),
        energised: Vec::new(),
        mains: Vec::new(),
        tapped: Vec::new(),
        pop_exposure: Vec::new(),
        players: Vec::new(),
        deaths: Vec::new(),
    })
}

fn write_clock(w: &mut Writer, s: &GameServer) {
    let c = s.saved_clock();
    w.u64(c.tick_counter);
    w.u32(c.world_time);
    w.u64(c.rain_until);
    w.u64(c.storm_until);
    w.opt(c.weather_lock_applied.as_deref(), |w, lock| w.str(lock));
    w.u32(c.nights);
    w.opt(c.was_night, |w, night| w.bool(night));
    w.raw(&c.calendar_id);
    w.u32(c.day_remainder);
    w.opt(c.day_last_tick, |w, tick| w.u64(tick));
    w.u32(s.world_time_step);
    w.u32(s.falling_tick_counter);
    w.opt(s.pending_strike, |w, strike| w.u64(strike));
    w.u64(s.world.total_ticks);
    w.u64(s.world.total_work);
}

fn read_clock(r: &mut Reader<'_>) -> Result<SavedClock, Malformed> {
    let clock = SavedClock {
        tick_counter: r.u64()?,
        world_time: r.u32()?,
        rain_until: r.u64()?,
        storm_until: r.u64()?,
        weather_lock_applied: r.opt(|r| r.string())?,
        nights: r.u32()?,
        was_night: r.opt(|r| r.bool())?,
        calendar_id: r.bytes16()?,
        day_remainder: r.u32()?,
        day_last_tick: r.opt(|r| r.u64())?,
    };
    if !clock.is_valid() {
        return Err(Malformed("a clock no build could have written"));
    }
    Ok(clock)
}

fn write_modes(w: &mut Writer, s: &GameServer) {
    w.bool(s.simulates_block_machines);
    w.bool(s.animal_life_simulated);
    w.bool(s.rolls_lightning);
    w.u32(s.column_refill_per_tick as u32);
    w.bool(s.fire_spread_enabled);
    w.bool(s.explosives_enabled);
    w.i32(s.render_distance);
    w.str(s.play_mode.as_meta_str());
    w.i32(s.spawn_column.0);
    w.i32(s.spawn_column.1);
    w.u32(s.pop_epoch);
    w.u32(s.next_attach_gen);
}

fn read_modes(r: &mut Reader<'_>) -> Result<Modes, Malformed> {
    Ok(Modes {
        simulates_block_machines: r.bool()?,
        animal_life_simulated: r.bool()?,
        rolls_lightning: r.bool()?,
        column_refill_per_tick: r.u32()?,
        fire_spread_enabled: r.bool()?,
        explosives_enabled: r.bool()?,
        render_distance: r.i32()?,
        play_mode: r.string()?,
        spawn_column: (r.i32()?, r.i32()?),
        pop_epoch: r.u32()?,
        next_attach_gen: r.u32()?,
    })
}

fn write_streamer(w: &mut Writer, s: &GameServer) {
    w.opt(s.column_streamer.as_ref(), |w, streamer| {
        let (sim_distance, budget, settled_for) = streamer.to_parts();
        w.i32(sim_distance);
        w.u32(budget as u32);
        w.opt(settled_for, |w, anchors| {
            w.count(anchors.len());
            for &(cx, cz) in anchors {
                w.i32(cx);
                w.i32(cz);
            }
        });
    });
}

fn read_streamer(r: &mut Reader<'_>) -> Result<Option<ColumnStreamer>, Malformed> {
    r.opt(|r| {
        let sim_distance = r.i32()?;
        let budget = r.u32()? as usize;
        let settled_for = r.opt(|r| {
            let n = r.count(8)?;
            (0..n).map(|_| Ok((r.i32()?, r.i32()?))).collect::<Result<Vec<_>, _>>()
        })?;
        Ok(ColumnStreamer::from_parts(sim_distance, budget, settled_for))
    })
}

const LOADED: u8 = 1;
const EVICTED: u8 = 2;
const PERSIST: u8 = 4;

/// The most sections a layout may name. `apply_chunk_layout` builds a chunk (about
/// 12.8 KB) for each one it finds no record of, so this is what bounds a restore's
/// allocation: about 3.4 GiB at worst, inside tier L's 4 GiB. It is also the runner's
/// ceiling of 262,144 keys, and a section that holds anything is a key, so no world a
/// runner can store passes it by more than its empty sections. The 8-joiner bench
/// world is about 9,200.
const MAX_LAYOUT_SECTIONS: u32 = 1 << 18;

/// The furthest column a run can hold: a body at the playable limit, plus what
/// streams in round it. Past this a column's first cell nears `i32`'s edge.
const MAX_COLUMN: i32 = crate::commands::builtins::tp::MAX_COORD as i32 / crate::chunk::CHUNK_SIZE as i32 + 64;

fn column_in_range((cx, cz): (i32, i32)) -> bool {
    (-MAX_COLUMN..=MAX_COLUMN).contains(&cx) && (-MAX_COLUMN..=MAX_COLUMN).contains(&cz)
}

/// One entry per column that holds a chunk or counts as loaded, sorted.
fn write_layout(w: &mut Writer, s: &GameServer) -> Result<(), String> {
    let mut columns: std::collections::BTreeMap<(i32, i32), ChunkColumnLayout> =
        s.world.chunk_layout()?.into_iter().map(|c| (c.column, c)).collect();
    for &column in &s.loaded_columns {
        columns.entry(column).or_insert_with(|| ChunkColumnLayout { column, ..Default::default() });
    }
    w.count(columns.len());
    for c in columns.values() {
        w.i32(c.column.0);
        w.i32(c.column.1);
        let mut flags = 0;
        if s.loaded_columns.contains(&c.column) {
            flags |= LOADED;
        }
        if c.evicted_column {
            flags |= EVICTED;
        }
        if c.persist {
            flags |= PERSIST;
        }
        w.u8(flags);
        w.u32(c.live_sections);
        w.u32(c.evicted_sections);
    }
    Ok(())
}

fn read_layout(r: &mut Reader<'_>) -> Result<(AHashSet<(i32, i32)>, Vec<ChunkColumnLayout>), Malformed> {
    let n = r.count(17)?;
    let mut loaded = AHashSet::new();
    let mut layout = Vec::with_capacity(n);
    let mut sections = 0u32;
    for _ in 0..n {
        let column = (r.i32()?, r.i32()?);
        let flags = r.u8()?;
        if flags > (LOADED | EVICTED | PERSIST) {
            return Err(Malformed("an unknown column flag"));
        }
        if !column_in_range(column) {
            return Err(Malformed("a layout column outside the playable range"));
        }
        if flags & LOADED != 0 {
            loaded.insert(column);
        }
        let (live_sections, evicted_sections) = (r.u32()?, r.u32()?);
        if (live_sections | evicted_sections) & !LAYOUT_COLUMN_SECTIONS != 0 {
            return Err(Malformed("a layout section outside a column"));
        }
        // A save keeps one copy of a section (`persistable_chunks`), so no record
        // could bring the second back.
        if live_sections & evicted_sections != 0 {
            return Err(Malformed("a layout section both live and evicted"));
        }
        // No run stores a section for a column it holds live, and a block write
        // there would make a second copy.
        if evicted_sections != 0 && flags & EVICTED == 0 {
            return Err(Malformed("evicted sections in a column not flagged evicted"));
        }
        sections = sections.saturating_add(live_sections.count_ones() + evicted_sections.count_ones());
        if sections > MAX_LAYOUT_SECTIONS {
            return Err(Malformed("a layout naming more sections than a checkpoint may"));
        }
        layout.push(ChunkColumnLayout {
            column,
            live_sections,
            evicted_sections,
            evicted_column: flags & EVICTED != 0,
            persist: flags & PERSIST != 0,
        });
    }
    strictly_increasing(&layout, |c| c.column)?;
    Ok((loaded, layout))
}

/// How many sections a layout's bytes name, through the shared reader.
#[cfg(test)]
pub(super) fn layout_sections_read(bytes: &[u8]) -> Result<u32, String> {
    let mut r = Reader::new(bytes);
    let (_, layout) = read_layout(&mut r).map_err(|e| e.0.to_string())?;
    Ok(layout.iter().map(|c| c.live_sections.count_ones() + c.evicted_sections.count_ones()).sum())
}

fn write_fluid(w: &mut Writer, f: &FluidCheckpoint) {
    write_list(w, &f.extra);
    write_list(w, &f.missing);
    w.count(f.spread.len());
    for &(x, y, z, distance) in &f.spread {
        w.pos((x, y, z));
        w.u8(distance);
    }
    write_list(w, &f.retract);
}

fn read_fluid(r: &mut Reader<'_>) -> Result<FluidCheckpoint, Malformed> {
    let extra = read_sorted_list(r)?;
    let missing = read_sorted_list(r)?;
    let n = r.count(13)?;
    let spread = (0..n)
        .map(|_| {
            let (x, y, z) = r.pos()?;
            Ok((x, y, z, r.u8()?))
        })
        .collect::<Result<_, _>>()?;
    Ok(FluidCheckpoint { extra, missing, spread, retract: read_list(r)? })
}

fn write_sims(w: &mut Writer, s: &GameServer) {
    w.u8(s.lava.slow_phase());
    w.u64(s.leaf_decay.digest_view().2);
    w.u64(s.lava.spread_queue().head());
    w.u64(s.lava.retract_queue().head());
}

/// The record of the sims that follow what the world holds.
fn write_cells(w: &mut Writer, s: &GameServer) {
    let fire = s.fire.digest_view();
    w.count(fire.len());
    for (pos, next_event, age) in fire {
        w.pos(pos);
        w.u64(next_event);
        w.u8(age);
    }

    let (check, decay, _) = s.leaf_decay.digest_view();
    write_list(w, &check);
    w.count(decay.len());
    for (x, y, z, at) in decay {
        w.pos((x, y, z));
        w.u64(at);
    }

    let (pending, due) = s.world.scheduler.digest_view();
    write_list(w, &pending);
    w.count(due.len());
    for (tick, update) in due {
        w.u64(tick);
        w.pos(update.pos);
        w.u8(schedule_kind(update.kind));
    }

    let energised = sorted(s.world.power.energised.iter().map(|(p, l)| (*p, *l)).collect());
    w.count(energised.len());
    for (pos, level) in energised {
        w.pos(pos);
        w.u8(level);
    }
    write_list(w, &sorted(s.world.power.mains.iter().copied().collect()));
    let tapped = sorted(s.world.tapped_rubber_logs.iter().map(|(p, t)| (*p, *t)).collect());
    w.count(tapped.len());
    for (pos, tick) in tapped {
        w.pos(pos);
        w.u64(tick);
    }
}

fn read_cells(r: &mut Reader<'_>, v: &mut Volatile) -> Result<(), Malformed> {
    let n = r.count(21)?;
    v.fire = (0..n).map(|_| Ok((r.pos()?, r.u64()?, r.u8()?))).collect::<Result<_, _>>()?;
    strictly_increasing(&v.fire, |c| c.0)?;
    v.leaf.0 = read_list(r)?;
    let n = r.count(20)?;
    v.leaf.1 = (0..n).map(|_| { let (x, y, z) = r.pos()?; Ok((x, y, z, r.u64()?)) }).collect::<Result<_, _>>()?;
    v.scheduler_pending = read_list(r)?;
    let n = r.count(21)?;
    v.scheduler_due = (0..n).map(|_| read_scheduled(r)).collect::<Result<_, _>>()?;
    let n = r.count(13)?;
    v.energised = (0..n).map(|_| Ok((r.pos()?, r.u8()?))).collect::<Result<_, _>>()?;
    strictly_increasing(&v.energised, |c| c.0)?;
    v.mains = read_sorted_list(r)?;
    let n = r.count(20)?;
    v.tapped = (0..n).map(|_| Ok((r.pos()?, r.u64()?))).collect::<Result<_, _>>()?;
    strictly_increasing(&v.tapped, |c| c.0)?;
    Ok(())
}

fn schedule_kind(kind: ScheduleKind) -> u8 {
    match kind {
        ScheduleKind::PowerReeval => 0,
        ScheduleKind::GateSettle => 1,
        ScheduleKind::ButtonRelease => 2,
        ScheduleKind::PlateRelease => 3,
    }
}

fn read_scheduled(r: &mut Reader<'_>) -> Result<(u64, ScheduledUpdate), Malformed> {
    let tick = r.u64()?;
    let pos = r.pos()?;
    let kind = match r.u8()? {
        0 => ScheduleKind::PowerReeval,
        1 => ScheduleKind::GateSettle,
        2 => ScheduleKind::ButtonRelease,
        3 => ScheduleKind::PlateRelease,
        _ => return Err(Malformed("an unknown scheduled update")),
    };
    Ok((tick, ScheduledUpdate { pos, kind }))
}

/// A cell a tick can add a neighbour's offset to without leaving `i32`.
pub(super) fn cell_in_range(&(x, y, z): &Pos) -> bool {
    const LIMIT: i32 = MAX_COLUMN * crate::chunk::CHUNK_SIZE as i32;
    [x, y, z].iter().all(|v| (-LIMIT..=LIMIT).contains(v))
}

/// The entries behind the cursor too: a restore writes them back, so they are state.
fn lava_in_range(l: &LavaCheckpoint) -> bool {
    l.extra.iter().chain(&l.missing).all(cell_in_range)
        && l.retract.drained().iter().chain(l.retract.iter()).all(cell_in_range)
        && l.spread.drained().iter().chain(l.spread.iter()).all(|&(x, y, z, _)| cell_in_range(&(x, y, z)))
}

fn fluid_in_range(f: &FluidCheckpoint) -> bool {
    f.extra.iter().chain(&f.missing).chain(&f.retract).all(cell_in_range)
        && f.spread.iter().all(|&(x, y, z, _)| cell_in_range(&(x, y, z)))
}

/// Finite, and where the game can put a body: the limit `/tp` clamps to.
fn body_in_range(sp: &ServerPlayer) -> bool {
    let limit = crate::commands::builtins::tp::MAX_COORD;
    let placed = |v: glam::Vec3| v.to_array().iter().all(|c| c.abs() <= limit);
    let p = &sp.player;
    placed(p.pos)
        && placed(sp.spawn_pos)
        && p.velocity.is_finite()
        && [p.fall_distance, sp.yaw, sp.pitch, sp.combat.health, sp.combat.max_health].iter().all(|v| v.is_finite())
}

impl Volatile {
    pub(super) fn tick_counter(&self) -> u64 {
        self.clock.tick_counter
    }

    /// The state that lies in stored records, read and range-checked by the same
    /// readers whichever end it is: the bytes a checkpoint is about to bind, or the
    /// ones a restore has just checked against their digests.
    pub(super) fn take_state(&mut self, state: &StateRecords) -> Result<(), Malformed> {
        fn whole<T>(bytes: &[u8], read: impl FnOnce(&mut Reader<'_>) -> Result<T, Malformed>) -> Result<T, Malformed> {
            let mut r = Reader::new(bytes);
            let value = read(&mut r)?;
            if !r.is_empty() {
                return Err(Malformed("bytes left over in a stored state record"));
            }
            Ok(value)
        }
        (self.loaded, self.layout) = whole(&state.layout, read_layout)?;
        let (drained, live) = lava_records::read_spread(self.lava_spread_head, &state.lava_spread)?;
        self.lava.spread = PositionedQueue::restored(self.lava_spread_head, drained, live);
        let (drained, live) = lava_records::read_retract(self.lava_retract_head, &state.lava_retract)?;
        self.lava.retract = PositionedQueue::restored(self.lava_retract_head, drained, live);
        (self.lava.extra, self.lava.missing) = lava_records::read_source_sections(&state.lava_sources)?;
        self.water = whole(&state.water, read_fluid)?;
        whole(&state.cells, |r| read_cells(r, self))?;
        self.players = whole(&state.players, super::players::read_all)?;
        self.deaths = whole(&state.corpses, super::deaths::read)?;
        self.pop_exposure = whole(&state.exposure, |r| {
            let n = r.count(20)?;
            let cells: Vec<_> = (0..n).map(|_| Ok((r.pos()?, r.u64()?))).collect::<Result<_, _>>()?;
            strictly_increasing(&cells, |c| c.0)?;
            Ok(cells)
        })?;
        self.validate()
    }

    /// The seal is a checksum, so a value can be sealed and absurd, and these are
    /// the ones a first tick would hang or overflow on: a sim distance of
    /// `i32::MAX` restores cleanly and the next `stream_columns` never ends. The
    /// bounds are what a real server can hold, not a client's settings: a server's
    /// render distance is only its load radius, and can be under the menu's floor.
    /// (The layout and the player count are refused as they are read, since those
    /// allocate.) What lies in records is empty here until [`Self::take_state`] has run,
    /// which validates again.
    pub(super) fn validate(&self) -> Result<(), Malformed> {
        if let Some(streamer) = &self.streamer {
            let (sim_distance, budget, settled_for) = streamer.to_parts();
            if sim_distance != crate::server_stream::clamp_sim_distance(sim_distance) {
                return Err(Malformed("a sim distance past its limits"));
            }
            if budget != crate::server_stream::SERVER_STREAM_BUDGET {
                return Err(Malformed("a streaming budget no server runs"));
            }
            if !settled_for.unwrap_or_default().iter().copied().all(column_in_range) {
                return Err(Malformed("a settled anchor outside the playable range"));
            }
        }
        if self.modes.column_refill_per_tick as usize > crate::server::HOST_COLUMN_REFILL_PER_TICK {
            return Err(Malformed("a column refill past its limit"));
        }
        if !(0..=crate::graphics_settings::RENDER_DISTANCE_MAX).contains(&self.modes.render_distance) {
            return Err(Malformed("a render distance past its limits"));
        }
        if !column_in_range(self.modes.spawn_column) {
            return Err(Malformed("a spawn column outside the playable range"));
        }
        if !self.players.iter().all(body_in_range) {
            return Err(Malformed("a body that isn't finite or is out of range"));
        }
        let (check, decay, _) = &self.leaf;
        let cells_in_range = self.fire.iter().all(|c| cell_in_range(&c.0))
            && fluid_in_range(&self.water)
            && lava_in_range(&self.lava)
            && check.iter().all(cell_in_range)
            && decay.iter().all(|&(x, y, z, _)| cell_in_range(&(x, y, z)))
            && self.scheduler_pending.iter().all(cell_in_range)
            && self.scheduler_due.iter().all(|(_, update)| cell_in_range(&update.pos))
            && self.energised.iter().all(|c| cell_in_range(&c.0))
            && self.mains.iter().all(cell_in_range)
            && self.tapped.iter().all(|c| cell_in_range(&c.0))
            && self.pop_exposure.iter().all(|c| cell_in_range(&c.0));
        if !cells_in_range {
            return Err(Malformed("a queued cell outside the playable range"));
        }
        if self.lava_slow >= crate::lava::SLOW_FACTOR {
            return Err(Malformed("a lava phase past its cycle"));
        }
        if !self.deaths.iter().all(super::deaths::Corpse::is_placed) {
            return Err(Malformed("a corpse that isn't anywhere"));
        }
        Ok(())
    }

    /// The pending deaths, which `deaths::apply` puts back once the rest is in.
    pub(super) fn take_deaths(&mut self) -> Vec<super::deaths::Corpse> {
        std::mem::take(&mut self.deaths)
    }

    /// The world's chunks and loaded columns as the checkpoint names them, ready
    /// for the manifest to be checked. Scattered wildlife standing in a column
    /// the run didn't hold goes with the column, as on a stream-out.
    pub(super) fn apply_layout(&mut self, s: &mut GameServer) {
        s.world.apply_chunk_layout(&self.layout);
        let mut gone: Vec<(i32, i32)> = s.loaded_columns.difference(&self.loaded).copied().collect();
        gone.sort_unstable();
        for (cx, cz) in gone {
            crate::entity::despawn_mobs_in_column(&mut s.ecs, cx, cz);
        }
        s.loaded_columns = std::mem::take(&mut self.loaded);
    }

    /// Everything else, once the manifest has checked out.
    pub(super) fn apply(self, s: &mut GameServer) {
        s.restore_saved_clock(Some(&self.clock));
        s.world_time_step = self.world_time_step;
        s.falling_tick_counter = self.falling_tick_counter;
        s.pending_strike = self.pending_strike;
        s.world.total_ticks = self.total_ticks;
        s.world.total_work = self.total_work;

        let m = self.modes;
        s.simulates_block_machines = m.simulates_block_machines;
        s.animal_life_simulated = m.animal_life_simulated;
        s.rolls_lightning = m.rolls_lightning;
        s.column_refill_per_tick = m.column_refill_per_tick as usize;
        s.fire_spread_enabled = m.fire_spread_enabled;
        s.explosives_enabled = m.explosives_enabled;
        s.render_distance = m.render_distance;
        s.set_play_mode(crate::play_mode::PlayMode::from_meta_str(&m.play_mode));
        s.spawn_column = m.spawn_column;
        s.pop_epoch = m.pop_epoch;
        s.next_attach_gen = m.next_attach_gen;
        s.column_streamer = self.streamer;

        s.fire.restore_cells(&self.fire);
        s.water.restore_checkpoint(&s.world, &s.loaded_columns, &self.water);
        s.lava.restore_checkpoint(&s.world, &s.loaded_columns, &self.lava, self.lava_slow);
        s.leaf_decay.restore_view(self.leaf);
        s.world.scheduler.restore_view(self.scheduler_pending, self.scheduler_due);
        s.world.power.energised = self.energised.into_iter().collect();
        s.world.power.mains = self.mains.into_iter().collect();
        s.world.tapped_rubber_logs = self.tapped.into_iter().collect();
        s.world.pop_exposure = self.pop_exposure.into_iter().collect();
        s.players = self.players;
    }
}
