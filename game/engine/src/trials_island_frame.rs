//! The Trials island's drawing board (part of `trials_island_layout`): a
//! grid over the built area and a north-frame painter that rotates each
//! district (or clock doorway) into place, records every plot's own cells,
//! stands signs and cases diorama liquids. Pure.

use super::{index, Doorway, SignSpot, DIO_LONG, DIO_SHORT, GROUND_Y, KEEP};
use crate::block::{self, BlockId};
use crate::meta::Facing;

/// Rotate a north-frame cell (or direction) by quarter turns, clockwise seen
/// from above (north -> east -> south -> west).
pub(super) fn rot(turns: u8, x: i32, z: i32) -> (i32, i32) {
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
pub(super) fn normal_yaw(nx: i32, nz: i32) -> f32 {
    (nx as f32).atan2(nz as f32)
}

/// Camera yaw looking along `(fx, fz)` (`camera::forward_from`'s inverse).
pub(super) fn look_yaw(fx: i32, fz: i32) -> f32 {
    (-fx as f32).atan2(-fz as f32)
}

pub(super) struct Grid {
    pub(super) cells: Vec<BlockId>,
    pub(super) signs: Vec<SignSpot>,
    pub(super) doorways: Vec<Doorway>,
    pub(super) footprints: Vec<(&'static str, Vec<[i32; 3]>)>,
}

/// A north-frame painter for one district (or one clock doorway).
pub(super) struct Frame<'g> {
    pub(super) g: &'g mut Grid,
    pub(super) turns: u8,
    /// Cells written through [`Frame::own`], for the footprint.
    owned: Vec<[i32; 3]>,
}

impl Grid {
    pub(super) fn set(&mut self, x: i32, y: i32, z: i32, b: BlockId) {
        if let Some(i) = index(x, y, z) {
            self.cells[i] = b;
        }
    }

    pub(super) fn get(&self, x: i32, y: i32, z: i32) -> BlockId {
        index(x, y, z).map_or(KEEP, |i| self.cells[i])
    }

    pub(super) fn frame(&mut self, turns: u8) -> Frame<'_> {
        Frame { g: self, turns, owned: Vec::new() }
    }
}

impl Frame<'_> {
    pub(super) fn world(&self, x: i32, z: i32) -> (i32, i32) {
        rot(self.turns, x, z)
    }

    pub(super) fn set(&mut self, x: i32, y: i32, z: i32, b: BlockId) {
        let (wx, wz) = self.world(x, z);
        self.g.set(wx, y, wz, b);
    }

    /// Write a cell that belongs to this plot's own structure.
    pub(super) fn own(&mut self, x: i32, y: i32, z: i32, b: BlockId) {
        let (wx, wz) = self.world(x, z);
        self.g.set(wx, y, wz, b);
        self.owned.push([wx, y, wz]);
    }

    pub(super) fn get(&self, x: i32, y: i32, z: i32) -> BlockId {
        let (wx, wz) = self.world(x, z);
        self.g.get(wx, y, wz)
    }

    pub(super) fn sign(&mut self, x: i32, z: i32, text: String, toward: (i32, i32)) {
        let (wx, wz) = self.world(x, z);
        let (fx, fz) = rot(self.turns, toward.0, toward.1);
        self.g.set(wx, GROUND_Y + 1, wz, block::OAK_SIGN);
        self.owned.push([wx, GROUND_Y + 1, wz]);
        self.g.signs.push(SignSpot { pos: [wx, GROUND_Y + 1, wz], text, facing: facing_of(fx, fz) });
    }

    /// The (min, max) world cells of a north-frame rectangle.
    pub(super) fn rect(&self, a: (i32, i32), b: (i32, i32)) -> ([i32; 2], [i32; 2]) {
        let (ax, az) = self.world(a.0, a.1);
        let (bx, bz) = self.world(b.0, b.1);
        ([ax.min(bx), az.min(bz)], [ax.max(bx), az.max(bz)])
    }

    /// The centre of a north-frame cell, in the world.
    pub(super) fn centre(&self, x: i32, z: i32) -> [f32; 3] {
        let (wx, wz) = self.world(x, z);
        [wx as f32 + 0.5, (GROUND_Y + 1) as f32, wz as f32 + 0.5]
    }

    /// Place a diorama so its cell `(0, 0, 0)` lands at north-frame `at`, its
    /// long axis along `long` and short axis along `short` (unit vectors), its
    /// bottom layer set into the floor. Liquids are cased in glass where they
    /// would touch air (and lava is lidded), so nothing flows or burns.
    pub(super) fn diorama(&mut self, cells: &[([i32; 3], BlockId)], at: (i32, i32), long: (i32, i32), short: (i32, i32)) {
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

    pub(super) fn finish(self, token: &'static str) {
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
