//! The Trials island (Trials in Two Acts §C.3), laid out: pure. One function,
//! [`layout`], turns "which trials are open, which are hinted at" into every
//! block of the island's built area, the signs, and the doorways the flow
//! watches. No world, no GPU: `trials_island_build` writes the result into the
//! island world, `trials_island_flow` reads the doorways.
//!
//! The shape, on the flat floor (surface y 79), round spawn at the origin
//! (distance is the square distance, `max(|x|, |z|)`):
//!
//! - **Plaza** (0..=4): marble, a signpost to each district, four lanterns.
//! - **Inner lawn** (5..=8) with the clock corner's dioramas, the **running
//!   track** (9..=11) round it, with the clock corner's doorways straddling
//!   it, one on each side, and an outer lawn edge (12).
//! - **Four districts**, one per shelf, each a wedge on the compass (Make
//!   north, Tend east, Tech south, Brave west), with three rows of plots
//!   facing the plaza (rows at 15, 23 and 31, room for 3, 4 and 6 plots).
//! - **Shore**: sand (41..=42) then water (43..=46).
//!
//! Every trial has a FIXED plot, whatever is unlocked, so a doorway never
//! moves; a plot is a doorway (open), a roped-off plot with a hint sign (one of
//! the next unlocks) or plain grass (hidden: nothing says how many there are).
//! The layout covers the whole built area, air included, so writing it again
//! changes nothing, and a plot that has since unlocked is rebuilt as a doorway.

use crate::block::{self, BlockId};
use crate::meta::Facing;

/// The flat floor's surface (`World::generate_flat_column`).
pub const GROUND_Y: i32 = 79;
/// The land's edge; the sand and water lie beyond it.
pub const LAND_R: i32 = 40;
const SAND_R: i32 = 42;
/// The outer edge of the water: the island's whole built (and protected) area.
pub const ISLAND_R: i32 = 46;
/// Lowest and highest rows the layout writes.
pub const MIN_Y: i32 = GROUND_Y - 1;
pub const MAX_Y: i32 = GROUND_Y + 8;
/// A cell the layout leaves as it is (below the floor, away from the water).
pub const KEEP: BlockId = BlockId::MAX;

const PLAZA_R: i32 = 4;
const TRACK: (i32, i32) = (9, 11);
/// A district plot: 8 wide (arch 5, gap, diorama 2), 6 deep, 2 between plots.
const SLOT_W: i32 = 8;
const SLOT_D: i32 = 6;
const SLOT_GAP: i32 = 2;
/// Each row's distance from the centre at its front.
const ROWS: [i32; 3] = [15, 23, 31];
/// Biggest diorama footprint a plot holds: 5 deep, 2 wide.
const DIO_LONG: i32 = 5;
const DIO_SHORT: i32 = 2;

/// Where a trial stands on the island.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum District {
    Make,
    Tend,
    Tech,
    Brave,
    /// The clock corner: the doorways on the running track.
    Clock,
}

impl District {
    /// Quarter turns from north (Make north, Tend east, Tech south, Brave west).
    fn turns(self) -> u8 {
        match self {
            District::Make | District::Clock => 0,
            District::Tend => 1,
            District::Tech => 2,
            District::Brave => 3,
        }
    }

    /// The shelf's colour, as a block (plinths, edges).
    pub fn colour(self) -> BlockId {
        match self {
            District::Make => block::WALLPAPER_ORANGE,
            District::Tend => block::WALLPAPER_GREEN,
            District::Tech => block::WALLPAPER_YELLOW,
            District::Brave => block::WALLPAPER_PURPLE,
            District::Clock => block::WALLPAPER_RED,
        }
    }

    fn lantern(self) -> BlockId {
        match self {
            District::Make => block::PAPER_LANTERN_ORANGE,
            District::Tend => block::PAPER_LANTERN_GREEN,
            District::Tech => block::PAPER_LANTERN_YELLOW,
            District::Brave => block::PAPER_LANTERN_PURPLE,
            District::Clock => block::PAPER_LANTERN_RED,
        }
    }

    fn signpost(self) -> &'static str {
        match self {
            District::Make => "Make\nThis way, north",
            District::Tend => "Tend\nThis way, east",
            District::Tech => "Tech\nThis way, south",
            District::Brave => "Brave\nThis way, west",
            District::Clock => "Against the clock\nThe doorways on the track",
        }
    }
}

/// What a trial's plot shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlotState {
    /// Not unlocked and not next: plain grass.
    Hidden,
    /// One of the next unlocks: a roped-off plot with this hint on a sign.
    Hint(String),
    /// Unlocked: a doorway with its diorama and a sign with this title.
    Open { title: String },
}

/// One trial, as the layout needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotInput {
    pub token: &'static str,
    pub district: District,
    pub state: SlotState,
    /// The card diorama, `([x, y, z], block)`, any origin (normalised here).
    pub diorama: Vec<([i32; 3], BlockId)>,
}

/// A doorway the flow watches. Cells are feet cells (y `GROUND_Y + 1`).
#[derive(Clone, Debug, PartialEq)]
pub struct Doorway {
    pub token: &'static str,
    /// Stepping into one of these cells opens the hero card.
    pub open_min: [i32; 2],
    pub open_max: [i32; 2],
    /// The card stays open while the feet stay in these cells.
    pub zone_min: [i32; 2],
    pub zone_max: [i32; 2],
    /// Where a player coming back from this trial stands, facing the arch.
    pub return_pos: [f32; 3],
    pub return_yaw: f32,
    /// The postcard's wall exhibit: anchor cell and facing (exhibit yaw).
    pub exhibit_at: [i32; 3],
    pub exhibit_yaw: f32,
}

impl Doorway {
    fn contains(min: [i32; 2], max: [i32; 2], pos: [f32; 3]) -> bool {
        let (x, z) = (pos[0].floor() as i32, pos[2].floor() as i32);
        let y = pos[1].floor() as i32;
        (GROUND_Y + 1..=GROUND_Y + 2).contains(&y)
            && (min[0]..=max[0]).contains(&x)
            && (min[1]..=max[1]).contains(&z)
    }

    /// Are these feet in the arch's opening?
    pub fn opens_at(&self, pos: [f32; 3]) -> bool {
        Self::contains(self.open_min, self.open_max, pos)
    }

    /// Are these feet still on the doorway's plinth?
    pub fn holds(&self, pos: [f32; 3]) -> bool {
        Self::contains(self.zone_min, self.zone_max, pos)
    }
}

/// A sign to stand, with its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignSpot {
    pub pos: [i32; 3],
    pub text: String,
    pub facing: Facing,
}

/// The whole island.
#[derive(Clone, Debug, PartialEq)]
pub struct IslandLayout {
    cells: Vec<BlockId>,
    pub doorways: Vec<Doorway>,
    pub signs: Vec<SignSpot>,
    /// Each plot's and doorway's own cells (arch, plinth, diorama, sign), by token.
    pub footprints: Vec<(&'static str, Vec<[i32; 3]>)>,
}

const SIDE: i32 = 2 * ISLAND_R + 1;
const HEIGHT: i32 = MAX_Y - MIN_Y + 1;

fn index(x: i32, y: i32, z: i32) -> Option<usize> {
    let inside = x.abs() <= ISLAND_R && z.abs() <= ISLAND_R && (MIN_Y..=MAX_Y).contains(&y);
    inside.then(|| (((y - MIN_Y) * SIDE + (z + ISLAND_R)) * SIDE + (x + ISLAND_R)) as usize)
}

/// Square distance from spawn.
pub fn ring(x: i32, z: i32) -> i32 {
    x.abs().max(z.abs())
}

/// May a block aimed at in this column not be broken or built on? The built
/// area, and the ring just outside it (a block placed against a cell there
/// could land on the island).
pub fn is_protected(x: i32, z: i32) -> bool {
    ring(x, z) <= ISLAND_R + 1
}

/// Where a fresh visit starts: the middle of the plaza, facing north.
pub const SPAWN: [f32; 3] = [0.5, (GROUND_Y + 1) as f32, 0.5];

impl IslandLayout {
    /// What the layout puts at a cell ([`KEEP`] outside the built area).
    pub fn block_at(&self, x: i32, y: i32, z: i32) -> BlockId {
        index(x, y, z).map_or(KEEP, |i| self.cells[i])
    }

    /// Every cell the layout writes in one chunk column, `((x, y, z), block)`.
    pub fn column_cells(&self, cx: i32, cz: i32, chunk: i32) -> Vec<((i32, i32, i32), BlockId)> {
        let mut out = Vec::new();
        for z in cz * chunk..(cz + 1) * chunk {
            for x in cx * chunk..(cx + 1) * chunk {
                for y in MIN_Y..=MAX_Y {
                    let b = self.block_at(x, y, z);
                    if b != KEEP {
                        out.push(((x, y, z), b));
                    }
                }
            }
        }
        out
    }

    /// The chunk columns the island covers.
    pub fn columns(chunk: i32) -> Vec<(i32, i32)> {
        let (lo, hi) = ((-ISLAND_R).div_euclid(chunk), ISLAND_R.div_euclid(chunk));
        (lo..=hi).flat_map(|cz| (lo..=hi).map(move |cx| (cx, cz))).collect()
    }

    pub fn doorway(&self, token: &str) -> Option<&Doorway> {
        self.doorways.iter().find(|d| d.token == token)
    }

    /// The doorway whose opening these feet stand in.
    pub fn doorway_opening_at(&self, pos: [f32; 3]) -> Option<&Doorway> {
        self.doorways.iter().find(|d| d.opens_at(pos))
    }
}

// ---------------------------------------------------------------------------
// Building the grid
// ---------------------------------------------------------------------------

/// Rotate a north-frame cell (or direction) by quarter turns, clockwise seen
/// from above (north -> east -> south -> west).
fn rot(turns: u8, x: i32, z: i32) -> (i32, i32) {
    match turns % 4 {
        0 => (x, z),
        1 => (-z, x),
        2 => (-x, -z),
        _ => (z, -x),
    }
}

fn facing_of(dx: i32, dz: i32) -> Facing {
    match (dx.signum(), dz.signum()) {
        (0, 1) => Facing::South,
        (0, -1) => Facing::North,
        (1, 0) => Facing::East,
        _ => Facing::West,
    }
}

/// Exhibit yaw for an outward wall normal (`exhibit::yaw_to_normal`'s inverse).
fn normal_yaw(nx: i32, nz: i32) -> f32 {
    (nx as f32).atan2(nz as f32)
}

/// Camera yaw looking along `(fx, fz)` (`camera::forward_from`'s inverse).
fn look_yaw(fx: i32, fz: i32) -> f32 {
    (-fx as f32).atan2(-fz as f32)
}

struct Grid {
    cells: Vec<BlockId>,
    signs: Vec<SignSpot>,
    doorways: Vec<Doorway>,
    footprints: Vec<(&'static str, Vec<[i32; 3]>)>,
}

/// A north-frame painter for one district (or one clock doorway).
struct Frame<'g> {
    g: &'g mut Grid,
    turns: u8,
    /// Cells written through [`Frame::own`], for the footprint.
    owned: Vec<[i32; 3]>,
}

impl Grid {
    fn set(&mut self, x: i32, y: i32, z: i32, b: BlockId) {
        if let Some(i) = index(x, y, z) {
            self.cells[i] = b;
        }
    }

    fn get(&self, x: i32, y: i32, z: i32) -> BlockId {
        index(x, y, z).map_or(KEEP, |i| self.cells[i])
    }

    fn frame(&mut self, turns: u8) -> Frame<'_> {
        Frame { g: self, turns, owned: Vec::new() }
    }
}

impl Frame<'_> {
    fn world(&self, x: i32, z: i32) -> (i32, i32) {
        rot(self.turns, x, z)
    }

    fn set(&mut self, x: i32, y: i32, z: i32, b: BlockId) {
        let (wx, wz) = self.world(x, z);
        self.g.set(wx, y, wz, b);
    }

    /// Write a cell that belongs to this plot's own structure.
    fn own(&mut self, x: i32, y: i32, z: i32, b: BlockId) {
        let (wx, wz) = self.world(x, z);
        self.g.set(wx, y, wz, b);
        self.owned.push([wx, y, wz]);
    }

    fn get(&self, x: i32, y: i32, z: i32) -> BlockId {
        let (wx, wz) = self.world(x, z);
        self.g.get(wx, y, wz)
    }

    fn sign(&mut self, x: i32, z: i32, text: String, toward: (i32, i32)) {
        let (wx, wz) = self.world(x, z);
        let (fx, fz) = rot(self.turns, toward.0, toward.1);
        self.g.set(wx, GROUND_Y + 1, wz, block::OAK_SIGN);
        self.owned.push([wx, GROUND_Y + 1, wz]);
        self.g.signs.push(SignSpot { pos: [wx, GROUND_Y + 1, wz], text, facing: facing_of(fx, fz) });
    }

    /// The (min, max) world cells of a north-frame rectangle.
    fn rect(&self, a: (i32, i32), b: (i32, i32)) -> ([i32; 2], [i32; 2]) {
        let (ax, az) = self.world(a.0, a.1);
        let (bx, bz) = self.world(b.0, b.1);
        ([ax.min(bx), az.min(bz)], [ax.max(bx), az.max(bz)])
    }

    /// The centre of a north-frame cell, in the world.
    fn centre(&self, x: i32, z: i32) -> [f32; 3] {
        let (wx, wz) = self.world(x, z);
        [wx as f32 + 0.5, (GROUND_Y + 1) as f32, wz as f32 + 0.5]
    }

    /// Place a diorama so its cell `(0, 0, 0)` lands at north-frame `at`, its
    /// long axis along `long` and short axis along `short` (unit vectors), its
    /// bottom layer set into the floor. Liquids are cased in glass where they
    /// would touch air (and lava is lidded), so nothing flows or burns.
    fn diorama(&mut self, cells: &[([i32; 3], BlockId)], at: (i32, i32), long: (i32, i32), short: (i32, i32)) {
        let Some(norm) = normalise(cells) else { return };
        let mut placed = Vec::new();
        for ([dx, dy, dz], b) in norm {
            let x = at.0 + long.0 * dx + short.0 * dz;
            let z = at.1 + long.1 * dx + short.1 * dz;
            self.own(x, GROUND_Y + dy, z, b);
            placed.push((x, GROUND_Y + dy, z, b));
        }
        for (x, y, z, b) in placed {
            if b != block::WATER && b != block::LAVA {
                continue;
            }
            let mut around = vec![(x + 1, y, z), (x - 1, y, z), (x, y, z + 1), (x, y, z - 1), (x, y - 1, z)];
            if b == block::LAVA {
                around.push((x, y + 1, z));
            }
            for (nx, ny, nz) in around {
                let open = self.get(nx, ny, nz);
                if ny > GROUND_Y && (open == block::AIR || open == KEEP) {
                    self.own(nx, ny, nz, block::GLASS);
                } else if ny <= GROUND_Y && open == KEEP {
                    self.own(nx, ny, nz, block::DIRT);
                }
            }
        }
    }

    fn finish(self, token: &'static str) {
        let owned = self.owned;
        self.g.footprints.push((token, owned));
    }
}

/// Shift a diorama to start at the origin and drop what falls off a plot
/// (`DIO_LONG` by `DIO_SHORT`, at most 4 high).
fn normalise(cells: &[([i32; 3], BlockId)]) -> Option<Vec<([i32; 3], BlockId)>> {
    let min = |i: usize| cells.iter().map(|(p, _)| p[i]).min();
    let (mx, my, mz) = (min(0)?, min(1)?, min(2)?);
    Some(
        cells
            .iter()
            .map(|(p, b)| ([p[0] - mx, p[1] - my, p[2] - mz], *b))
            .filter(|(p, b)| p[0] < DIO_LONG && p[2] < DIO_SHORT && p[1] < 5 && *b != block::AIR)
            .collect(),
    )
}

/// The plots in one district, front row first, the middle of each row first:
/// `(row distance, left edge x)` in the north frame.
pub(crate) fn slot_positions() -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for d in ROWS {
        let width = 2 * (d - 1) + 1;
        let n = (width + SLOT_GAP) / (SLOT_W + SLOT_GAP);
        let span = n * SLOT_W + (n - 1) * SLOT_GAP;
        let left = -(span / 2);
        let mut cols: Vec<i32> = (0..n).collect();
        let centre = |c: i32| (2 * (left + c * (SLOT_W + SLOT_GAP)) + SLOT_W).abs();
        cols.sort_by_key(|c| (centre(*c), *c));
        out.extend(cols.into_iter().map(|c| (d, left + c * (SLOT_W + SLOT_GAP))));
    }
    out
}

/// Inside the district's wedge (north frame: strictly nearer north than east or west).
fn in_wedge(x: i32, z: i32) -> bool {
    x.abs() < -z
}

fn base(g: &mut Grid) {
    for z in -ISLAND_R..=ISLAND_R {
        for x in -ISLAND_R..=ISLAND_R {
            let r = ring(x, z);
            for y in GROUND_Y + 1..=MAX_Y {
                g.set(x, y, z, block::AIR);
            }
            if r > SAND_R {
                g.set(x, GROUND_Y, z, block::WATER);
                g.set(x, GROUND_Y - 1, z, block::WATER);
            } else if r > LAND_R {
                g.set(x, GROUND_Y, z, block::SAND);
            } else if r <= PLAZA_R {
                g.set(x, GROUND_Y, z, if r == PLAZA_R { block::SANDSTONE } else { block::MARBLE });
            } else if (TRACK.0..=TRACK.1).contains(&r) {
                g.set(x, GROUND_Y, z, District::Clock.colour());
            } else {
                g.set(x, GROUND_Y, z, block::GRASS);
            }
        }
    }
    // Lanterns on posts at the plaza's corners.
    for (x, z) in [(-PLAZA_R, -PLAZA_R), (PLAZA_R, -PLAZA_R), (-PLAZA_R, PLAZA_R), (PLAZA_R, PLAZA_R)] {
        g.set(x, GROUND_Y + 1, z, block::FENCE_POST);
        g.set(x, GROUND_Y + 2, z, block::FENCE_POST);
        g.set(x, GROUND_Y + 3, z, block::PAPER_LANTERN_WHITE);
    }
}

/// A district's avenue from the plaza to its front row, paved and edged in
/// its colour (the track stays the track).
fn avenue(f: &mut Frame<'_>, district: District) {
    for z in -(ROWS[0] - 1)..=-(PLAZA_R + 1) {
        if (TRACK.0..=TRACK.1).contains(&-z) {
            continue;
        }
        for x in -2..=2 {
            f.set(x, GROUND_Y, z, if x.abs() == 2 { district.colour() } else { block::SANDSTONE });
        }
    }
}

/// One district plot, its front row at distance `d`, left edge `x0`.
fn district_slot(f: &mut Frame<'_>, district: District, slot: &SlotInput, d: i32, x0: i32) {
    // north-frame cell of plot cell (u along the row, v deep).
    let at = |u: i32, v: i32| (x0 + u, -(d + v));
    let walkway = |f: &mut Frame<'_>| {
        for u in -1..=SLOT_W {
            for v in -2..=-1 {
                let (x, z) = at(u, v);
                if in_wedge(x, z) {
                    f.set(x, GROUND_Y, z, if v == -2 { district.colour() } else { block::SANDSTONE });
                }
            }
        }
    };
    match &slot.state {
        SlotState::Hidden => {}
        SlotState::Hint(text) => {
            walkway(f);
            for u in 0..SLOT_W {
                for v in 1..SLOT_D {
                    let edge = u == 0 || u == SLOT_W - 1 || v == 1 || v == SLOT_D - 1;
                    if edge && (u + v) % 2 == 1 {
                        let (x, z) = at(u, v);
                        f.own(x, GROUND_Y + 1, z, block::FENCE_POST);
                    }
                }
            }
            let (x, z) = at(3, 0);
            f.sign(x, z, text.clone(), (0, 1));
        }
        SlotState::Open { title } => {
            walkway(f);
            for u in 0..=4 {
                for v in 1..=3 {
                    let (x, z) = at(u, v);
                    f.own(x, GROUND_Y, z, district.colour());
                }
                let (x, z) = at(u, 2);
                if u == 0 || u == 4 {
                    for y in GROUND_Y + 1..=GROUND_Y + 4 {
                        f.own(x, y, z, block::MARBLE);
                    }
                }
                f.own(x, GROUND_Y + 5, z, block::MARBLE);
                f.own(x, GROUND_Y + 6, z, district.colour());
            }
            let (x, z) = at(2, 2);
            f.own(x, GROUND_Y + 7, z, district.lantern());
            // The diorama beside the arch: its long axis runs back (v), short along the row.
            let (ox, oz) = at(6, 1);
            f.diorama(&slot.diorama, (ox, oz), (0, -1), (1, 0));
            let (x, z) = at(5, 1);
            f.sign(x, z, title.clone(), (0, 1));
            let (open_min, open_max) = f.rect(at(1, 2), at(3, 2));
            let (zone_min, zone_max) = f.rect(at(0, 1), at(4, 3));
            let (rx, rz) = at(2, 0);
            let (ex, ez) = at(2, 1);
            let (ewx, ewz) = f.world(ex, ez);
            let (fx, fz) = rot(f.turns, 0, -1);
            let (nx, nz) = rot(f.turns, 0, 1);
            f.g.doorways.push(Doorway {
                token: slot.token,
                open_min,
                open_max,
                zone_min,
                zone_max,
                return_pos: f.centre(rx, rz),
                return_yaw: look_yaw(fx, fz),
                exhibit_at: [ewx, GROUND_Y + 5, ewz],
                exhibit_yaw: normal_yaw(nx, nz),
            });
        }
    }
}

/// One clock-corner doorway, straddling the north side of the track (rotated
/// for the others), its diorama on the inner lawn beside it.
fn clock_slot(f: &mut Frame<'_>, slot: &SlotInput) {
    let SlotState::Open { title } = &slot.state else { return };
    const X: i32 = 6;
    let (outer, inner) = (-(TRACK.1 + 1), -(TRACK.0 - 1));
    for z in outer..=inner {
        if z == outer || z == inner {
            for y in GROUND_Y + 1..=GROUND_Y + 4 {
                f.own(X, y, z, block::MARBLE);
            }
            f.own(X, GROUND_Y, z, District::Clock.colour());
        }
        f.own(X, GROUND_Y + 5, z, block::MARBLE);
        f.own(X, GROUND_Y + 6, z, District::Clock.colour());
    }
    f.own(X, GROUND_Y + 7, -(TRACK.0 + 1), District::Clock.lantern());
    f.diorama(&slot.diorama, (X - 3, inner + 1), (1, 0), (0, 1));
    f.sign(X + 2, inner + 1, title.clone(), (0, 1));
    let (open_min, open_max) = f.rect((X, -TRACK.1), (X, -TRACK.0));
    let (zone_min, zone_max) = f.rect((X - 1, -TRACK.1), (X + 1, -TRACK.0));
    let (ex, ez) = f.world(X - 1, -(TRACK.0 + 1));
    let (fx, fz) = rot(f.turns, 1, 0);
    let (nx, nz) = rot(f.turns, -1, 0);
    f.g.doorways.push(Doorway {
        token: slot.token,
        open_min,
        open_max,
        zone_min,
        zone_max,
        return_pos: f.centre(X - 2, -(TRACK.0 + 1)),
        return_yaw: look_yaw(fx, fz),
        exhibit_at: [ex, GROUND_Y + 5, ez],
        exhibit_yaw: normal_yaw(nx, nz),
    });
}

/// Lay out the island for these trials (in `TRIAL_ORDER` order: each
/// district's plots are taken in that order, so a trial's plot never moves).
pub fn layout(slots: &[SlotInput]) -> IslandLayout {
    let side = SIDE as usize;
    let mut g = Grid {
        cells: vec![KEEP; side * side * HEIGHT as usize],
        signs: Vec::new(),
        doorways: Vec::new(),
        footprints: Vec::new(),
    };
    base(&mut g);
    let positions = slot_positions();
    for district in [District::Make, District::Tend, District::Tech, District::Brave] {
        let mine: Vec<&SlotInput> = slots.iter().filter(|s| s.district == district).collect();
        let mut f = g.frame(district.turns());
        if mine.iter().any(|s| s.state != SlotState::Hidden) {
            avenue(&mut f, district);
        }
        f.sign(0, -(PLAZA_R - 1), district.signpost().to_string(), (0, 1));
        f.finish("plaza");
        for (slot, (d, x0)) in mine.into_iter().zip(positions.iter().copied()) {
            let mut f = g.frame(district.turns());
            district_slot(&mut f, district, slot, d, x0);
            f.finish(slot.token);
        }
    }
    for (i, slot) in slots.iter().filter(|s| s.district == District::Clock).take(4).enumerate() {
        let mut f = g.frame(i as u8);
        clock_slot(&mut f, slot);
        f.finish(slot.token);
    }
    let mut f = g.frame(0);
    f.sign(-(PLAZA_R - 1), -(PLAZA_R - 1), District::Clock.signpost().to_string(), (0, 1));
    f.finish("plaza");
    IslandLayout { cells: g.cells, doorways: g.doorways, signs: g.signs, footprints: g.footprints }
}

/// How many plots a district holds.
#[cfg_attr(not(test), allow(dead_code))]
pub fn district_capacity() -> usize {
    slot_positions().len()
}

#[cfg(test)]
#[path = "trials_island_layout_tests.rs"]
mod tests;
