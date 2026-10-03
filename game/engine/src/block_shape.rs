//! F1 — block-shape foundation.
//!
//! The engine stores one `u16` id per block plus a sparse `u8` meta byte
//! (`meta.rs`). Full-cube blocks need neither shape nor collision data — they
//! are the default. *Shaped* blocks (slabs, stairs, …) need three things the
//! greedy full-cube path can't express:
//!
//!   1. **per-block collision AABB(s)** — a slab is half-height, a stair is two
//!      boxes; physics must resolve against the real boxes, not a unit cube;
//!   2. **sub-cuboid render geometry** — the mesher emits one textured box per
//!      cuboid instead of a merged greedy quad;
//!   3. **orientation/variant state** — taken from the `meta` byte
//!      (`facing` = 3 bits, `state` = 2 bits), written at place-time.
//!
//! This module is the single source of truth for (1) and (2), as **pure**
//! functions of `(BlockShape, meta)`. `physics` consumes `collision_aabbs`;
//! `mesh` consumes `render_cuboids`; `block` maps id → `BlockShape` via
//! `shape_of`. Following the engine idiom (`light_emission`, `camera_occlusion`)
//! shape is a match on the registry, **not** a new `BlockDef` field — so the
//! ~280 existing block literals stay untouched and full-cube by default.
//!
//! Coordinates are block-local, in `[0,1]³` (x east, y up, z south).

use crate::block::{self, BlockId};
use crate::meta::{self, Facing};

/// An axis-aligned box in block-local `[0,1]³` space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    /// The full unit cube — the FullCube collision/render box.
    pub const FULL: Aabb = Aabb {
        min: [0.0, 0.0, 0.0],
        max: [1.0, 1.0, 1.0],
    };

    pub const fn new(min: [f32; 3], max: [f32; 3]) -> Aabb {
        Aabb { min, max }
    }

    /// Mirror this box across the horizontal mid-plane (y → 1-y). Used to turn a
    /// bottom-oriented shape into its upside-down ("top half") variant.
    pub fn flip_y(self) -> Aabb {
        Aabb {
            min: [self.min[0], 1.0 - self.max[1], self.min[2]],
            max: [self.max[0], 1.0 - self.min[1], self.max[2]],
        }
    }
}

/// The geometric family a block belongs to. Default is `FullCube` (handled
/// entirely by the greedy mesher + boolean collision — never reaches this
/// module). Shaped variants resolve their concrete boxes from the meta byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockShape {
    /// Ordinary solid cube. Listed for completeness; callers short-circuit it.
    FullCube,
    /// Snow layer (2026-07-04): a thin bottom slice, 1..=8 layers tall — the
    /// layer count lives in the meta AUX field (0 = 1 layer of 1/8 block).
    /// Snowfall stacks layers; at 8 the block converts to full SNOW.
    SnowLayer,
    /// Half-block. The occupied half is chosen by `meta.facing`:
    /// Down→bottom, Up→top (the two horizontal slabs), and N/S/W/E→the four
    /// **vertical** slabs (backlog #27). One field covers all six placements.
    Slab,
    /// Stair: a bottom (or top, if `meta.state != 0`) half-slab plus an
    /// upper (or lower) quarter on the `meta.facing` side. `facing` is the
    /// horizontal direction the tall back sits toward (the ascent direction).
    Stairs,
    /// Fence gate: a thin full-height panel across the passage axis when
    /// closed (blocks movement), swung clear when open (passable, end posts
    /// only). `meta.facing` = the blocked axis (N/S vs E/W); `meta.state` bit 0
    /// = open. Right-click toggles open.
    FenceGate,
    /// Trapdoor: a thin panel. Closed → a flat lid at the bottom (or top, if
    /// `meta.state` bit 1 set) of the cell; open → a thin vertical flap against
    /// the `meta.facing` wall. `meta.state` bit 0 = open. Right-click toggles.
    Trapdoor,
    /// Pane: thin full-height vertical sheets (glass panes, iron bars). A
    /// **connecting** shape: its geometry is a central post plus thin arms toward
    /// connected neighbours, derived from a live 4-bit connection mask (see
    /// `CONN_*`), NOT from stored meta. A lone pane is just the central post.
    Pane,
    /// Door: a thin full-height panel, **two cells tall**. Closed → flush on the
    /// `meta.facing` wall (blocks the doorway); open → swung 90° to a
    /// perpendicular wall (clear). `meta.state` bit 0 = open, bit 1 = top half;
    /// `meta.aux` bit 0 = hinge side. Both halves share facing/open/hinge.
    Door,
    /// Wall: a 0.5-wide central post plus up to four 0.25-wide arms toward
    /// connected neighbours (other walls + full-cube solids). Like `Pane` this is
    /// a **connecting** shape — the byte passed to `collision_aabbs`/
    /// `render_cuboids` is a computed connection mask, not the persisted meta.
    Wall,
    /// Button: a small flush nub on the face given by `meta.facing` (the
    /// protrusion direction = the clicked face's outward normal). **Shape only**
    /// — pass-through (no collision); the power/press behaviour lands with the
    /// Electricity arc. 6/16 wide × 4/16 tall × 2/16 deep.
    Button,
    /// Lever: a small base plate + handle on the `meta.facing` face. Right-click
    /// flips `meta.state` bit 0 (the handle visual) — **visual only**, no power.
    /// Pass-through (no collision).
    Lever,
    /// Pressure plate: a thin flat plate covering most of the cell floor.
    /// **Shape only** — pass-through; the step-on/power behaviour is deferred.
    PressurePlate,
    /// Standing sign: a thin text board on a short post. `meta.facing` is the
    /// direction the board faces (the readable side). Pass-through; the text
    /// lives in a `BlockEntityData::Sign`.
    Sign,
    /// Item frame: a thin plate flush on the `meta.facing` wall (facing = the
    /// clicked face's outward normal). Pass-through; the displayed item lives in
    /// a `BlockEntityData::ItemFrame` and renders as a small cube on the plate.
    ItemFrame,
}

/// Connection-mask bits for the *connecting* shapes (`Wall`, `Pane`). The
/// world/mesh layer derives this nibble from a block's live neighbours and
/// passes it where ordinary shapes pass their stored meta byte — so connecting
/// blocks restyle automatically when a neighbour is placed or removed, with no
/// persisted side-state to migrate. Bit ↔ neighbour direction matches
/// `Facing::offset`: N = −Z, S = +Z, W = −X, E = +X.
pub const CONN_N: u8 = 0b0001;
pub const CONN_S: u8 = 0b0010;
pub const CONN_W: u8 = 0b0100;
pub const CONN_E: u8 = 0b1000;

impl BlockShape {
    /// Does this shape need the meta byte for orientation? (Used at place-time
    /// to decide whether to stamp a facing.) No caller found — block_interact.rs's
    /// facing-stamp decision doesn't call this, not even in a test.
    #[allow(dead_code)]
    pub fn is_oriented(self) -> bool {
        !matches!(self, BlockShape::FullCube)
    }
}

/// Map a block id to its geometric shape. Match-based per the engine idiom
/// (`light_emission`, `camera_occlusion`) — the ~280 ordinary blocks fall
/// through to `FullCube` with no per-literal edit. This is the single place the
/// id→shape binding lives; physics, meshing and placement all consult it.
pub fn shape_of(id: BlockId) -> BlockShape {
    match id {
        block::SNOW_LAYER => BlockShape::SnowLayer,
        block::STONE_SLAB => BlockShape::Slab,
        block::STONE_STAIRS => BlockShape::Stairs,
        block::OAK_FENCE_GATE => BlockShape::FenceGate,
        block::OAK_TRAPDOOR => BlockShape::Trapdoor,
        block::GLASS_PANE | block::IRON_BARS => BlockShape::Pane,
        block::OAK_DOOR => BlockShape::Door,
        block::COBBLESTONE_WALL => BlockShape::Wall,
        // Wave 2c — flush shapes for the existing Electricity (Spec 48) input
        // devices. The power behaviour is unchanged; this only restyles them
        // from full cubes to proper button/lever/plate geometry.
        block::BUTTON => BlockShape::Button,
        block::LEVER => BlockShape::Lever,
        block::PRESSURE_PLATE => BlockShape::PressurePlate,
        block::OAK_SIGN => BlockShape::Sign,
        block::ITEM_FRAME => BlockShape::ItemFrame,
        _ => BlockShape::FullCube,
    }
}

/// True for the signal-input shapes (button, lever, pressure plate). The power
/// placement path uses this to leave their meta facing as the F1 shaped
/// placement set it (from the clicked face) rather than overwriting it.
pub fn is_signal_input(id: BlockId) -> bool {
    matches!(
        shape_of(id),
        BlockShape::Button | BlockShape::Lever | BlockShape::PressurePlate
    )
}

/// True for shapes whose geometry is derived from live neighbours (the byte
/// handed to `collision_aabbs`/`render_cuboids` is a `CONN_*` mask, not stored
/// meta). The world + mesh layers branch on this to compute the mask.
pub fn is_connecting(shape: BlockShape) -> bool {
    matches!(shape, BlockShape::Wall | BlockShape::Pane)
}

/// Does a connecting block of `shape` join to the neighbour block `neighbour`?
/// Walls join other walls; panes/bars join other panes/bars; both join any
/// full-cube solid block (a flat face to butt an arm against). Air, plants and
/// other shaped blocks don't form a connection.
pub fn connects(shape: BlockShape, neighbour: BlockId, registry: &block::BlockRegistry) -> bool {
    if neighbour == block::AIR {
        return false;
    }
    let nshape = shape_of(neighbour);
    let same_family = match shape {
        BlockShape::Wall => nshape == BlockShape::Wall,
        BlockShape::Pane => nshape == BlockShape::Pane,
        _ => false,
    };
    let full_solid = nshape == BlockShape::FullCube && registry.is_solid(neighbour);
    same_family || full_solid
}

/// True if a block is opened/closed by right-clicking (fence gate, trapdoor,
/// door). The dispatch arm in `game_loop` keys off this so each new openable
/// block is just a `shape_of` + this predicate.
pub fn is_toggleable(id: BlockId) -> bool {
    // NB: Lever is deliberately NOT here — the Electricity (Spec 48) right-click
    // handler owns lever toggling (game_loop), and that branch runs after this
    // one. Listing Lever would hijack the power lever's interaction.
    matches!(
        shape_of(id),
        BlockShape::FenceGate | BlockShape::Trapdoor | BlockShape::Door
    )
}

/// Whether a door cell is the upper half (state bit 1). The lower half is below.
pub fn door_is_top(m: u8) -> bool {
    meta::state(m) & 0b10 != 0
}
/// Whether a door is right-hinged (aux bit 0). Decides which perpendicular wall
/// it swings to when open.
pub fn door_hinge_right(m: u8) -> bool {
    meta::aux(m) & 0b001 != 0
}

/// Open/close state for toggleable shaped blocks (fence gate, and later
/// trapdoors + doors). Stored in `meta.state` bit 0 so it coexists with the
/// other state bit and the facing field.
pub fn is_open(m: u8) -> bool {
    meta::state(m) & 0b01 != 0
}
/// Flip the open bit, leaving facing + the other state bit untouched.
pub fn toggled_open(m: u8) -> u8 {
    let s = meta::state(m);
    meta::with_state(m, s ^ 0b01)
}

/// True if a block needs the shaped path (custom collision + sub-cuboid mesh +
/// place-time orientation). Cheap predicate used by the mesher's greedy-skip and
/// the placement code.
pub fn is_shaped(id: BlockId) -> bool {
    shape_of(id) != BlockShape::FullCube
}

/// The box a `Slab` occupies for the given facing.
fn slab_aabb(facing: Facing) -> Aabb {
    match facing {
        Facing::Down => Aabb::new([0.0, 0.0, 0.0], [1.0, 0.5, 1.0]),
        Facing::Up => Aabb::new([0.0, 0.5, 0.0], [1.0, 1.0, 1.0]),
        Facing::North => Aabb::new([0.0, 0.0, 0.0], [1.0, 1.0, 0.5]),
        Facing::South => Aabb::new([0.0, 0.0, 0.5], [1.0, 1.0, 1.0]),
        Facing::West => Aabb::new([0.0, 0.0, 0.0], [0.5, 1.0, 1.0]),
        Facing::East => Aabb::new([0.5, 0.0, 0.0], [1.0, 1.0, 1.0]),
    }
}

/// The two boxes a `Stairs` occupies: a half-slab + the tall quarter on the
/// `facing` side. `top_half` flips the whole shape upside-down.
fn stairs_aabbs(facing: Facing, top_half: bool) -> [Aabb; 2] {
    // Bottom-oriented: lower slab fills y∈[0,0.5]; the upper quarter (y∈[0.5,1])
    // sits on the half-cell toward `facing`.
    let slab = Aabb::new([0.0, 0.0, 0.0], [1.0, 0.5, 1.0]);
    let step = match facing {
        Facing::North => Aabb::new([0.0, 0.5, 0.0], [1.0, 1.0, 0.5]),
        Facing::South => Aabb::new([0.0, 0.5, 0.5], [1.0, 1.0, 1.0]),
        Facing::West => Aabb::new([0.0, 0.5, 0.0], [0.5, 1.0, 1.0]),
        Facing::East => Aabb::new([0.5, 0.5, 0.0], [1.0, 1.0, 1.0]),
        // Vertical facings are nonsensical for stairs; treat as North.
        _ => Aabb::new([0.0, 0.5, 0.0], [1.0, 1.0, 0.5]),
    };
    if top_half {
        [slab.flip_y(), step.flip_y()]
    } else {
        [slab, step]
    }
}

/// Whether a stair's meta marks it upside-down (top half). Stairs are never
/// toggle blocks, so reusing state bit 0 here doesn't clash with `is_open`
/// (which only applies to fence gates / trapdoors / doors).
pub fn stairs_is_top(m: u8) -> bool {
    meta::state(m) != 0
}

/// Fence-gate panel (closed): a thin full-height wall across the passage axis.
fn fence_gate_panel(facing: Facing) -> Aabb {
    match facing {
        Facing::North | Facing::South => Aabb::new([0.0, 0.0, 0.4375], [1.0, 1.0, 0.5625]),
        _ => Aabb::new([0.4375, 0.0, 0.0], [0.5625, 1.0, 1.0]),
    }
}

/// Fence-gate end posts (open): two short stubs at the cell edges; the centre
/// is clear so the player walks through.
fn fence_gate_open_posts(facing: Facing) -> [Aabb; 2] {
    match facing {
        Facing::North | Facing::South => [
            Aabb::new([0.0, 0.0, 0.4375], [0.125, 1.0, 0.5625]),
            Aabb::new([0.875, 0.0, 0.4375], [1.0, 1.0, 0.5625]),
        ],
        _ => [
            Aabb::new([0.4375, 0.0, 0.0], [0.5625, 1.0, 0.125]),
            Aabb::new([0.4375, 0.0, 0.875], [0.5625, 1.0, 1.0]),
        ],
    }
}

/// Trapdoor / flap thickness (3/16, the Minecraft convention).
const TRAPDOOR_THICK: f32 = 0.1875;

/// Whether a trapdoor is mounted at the top of its cell (vs the floor). Reads
/// state bit 1 — bit 0 is the open flag.
pub fn trapdoor_is_top(m: u8) -> bool {
    meta::state(m) & 0b10 != 0
}

/// A door panel (one cell of the 2-tall door). Closed → flush on the `facing`
/// wall, blocking the doorway; open → swung 90° to a perpendicular wall, chosen
/// by the hinge side. Thickness 3/16, full height (per cell).
fn door_box(facing: Facing, open: bool, hinge_right: bool) -> Aabb {
    let t = 0.1875;
    if !open {
        match facing {
            Facing::North => Aabb::new([0.0, 0.0, 0.0], [1.0, 1.0, t]),
            Facing::South => Aabb::new([0.0, 0.0, 1.0 - t], [1.0, 1.0, 1.0]),
            Facing::West => Aabb::new([0.0, 0.0, 0.0], [t, 1.0, 1.0]),
            _ => Aabb::new([1.0 - t, 0.0, 0.0], [1.0, 1.0, 1.0]), // East + fallback
        }
    } else {
        // Open → a perpendicular wall; hinge picks which one.
        match facing {
            Facing::North | Facing::South => {
                if hinge_right {
                    Aabb::new([1.0 - t, 0.0, 0.0], [1.0, 1.0, 1.0])
                } else {
                    Aabb::new([0.0, 0.0, 0.0], [t, 1.0, 1.0])
                }
            }
            _ => {
                if hinge_right {
                    Aabb::new([0.0, 0.0, 1.0 - t], [1.0, 1.0, 1.0])
                } else {
                    Aabb::new([0.0, 0.0, 0.0], [1.0, 1.0, t])
                }
            }
        }
    }
}

/// Glass-pane / iron-bars geometry for a connection mask: a 2/16-thin central
/// post (full height) plus a thin arm reaching to the cell edge for every
/// connected neighbour. A lone pane (`conn == 0`) is just the post.
fn pane_cuboids(conn: u8) -> Vec<Aabb> {
    let lo = 0.4375;
    let hi = 0.5625; // 2/16 thick, centred
    let mut v = Vec::with_capacity(5);
    v.push(Aabb::new([lo, 0.0, lo], [hi, 1.0, hi]));
    if conn & CONN_N != 0 {
        v.push(Aabb::new([lo, 0.0, 0.0], [hi, 1.0, 0.5]));
    }
    if conn & CONN_S != 0 {
        v.push(Aabb::new([lo, 0.0, 0.5], [hi, 1.0, 1.0]));
    }
    if conn & CONN_W != 0 {
        v.push(Aabb::new([0.0, 0.0, lo], [0.5, 1.0, hi]));
    }
    if conn & CONN_E != 0 {
        v.push(Aabb::new([0.5, 0.0, lo], [1.0, 1.0, hi]));
    }
    v
}

/// Wall geometry for a connection mask: a 0.5-wide central post (full height)
/// plus a 0.25-wide arm (13/16 tall, the Minecraft convention) reaching to the
/// cell edge for every connected neighbour. A lone wall is just the post.
fn wall_cuboids(conn: u8) -> Vec<Aabb> {
    let arm_h = 0.8125; // 13/16
    let alo = 0.3125; // 5/16 — arm half-width inset
    let ahi = 0.6875; // 11/16
    let mut v = Vec::with_capacity(5);
    v.push(Aabb::new([0.25, 0.0, 0.25], [0.75, 1.0, 0.75]));
    if conn & CONN_N != 0 {
        v.push(Aabb::new([alo, 0.0, 0.0], [ahi, arm_h, 0.5]));
    }
    if conn & CONN_S != 0 {
        v.push(Aabb::new([alo, 0.0, 0.5], [ahi, arm_h, 1.0]));
    }
    if conn & CONN_W != 0 {
        v.push(Aabb::new([0.0, 0.0, alo], [0.5, arm_h, ahi]));
    }
    if conn & CONN_E != 0 {
        v.push(Aabb::new([0.5, 0.0, alo], [1.0, arm_h, ahi]));
    }
    v
}

/// Button nub for a mount face. `facing` is the protrusion direction (= the
/// clicked face's outward normal); the nub is a thin slab against the OPPOSITE
/// (support) wall. 6/16 wide, 4/16 tall (centred) on wall mounts, 2/16 deep.
fn button_box(facing: Facing) -> Aabb {
    let t = 0.125; // 2/16 protrusion
    let (a, b) = (0.3125, 0.6875); // 5/16..11/16 (6/16 wide)
    let (c, d) = (0.375, 0.625); // 6/16..10/16 (handle band for wall mounts)
    match facing {
        Facing::Up => Aabb::new([a, 0.0, a], [b, t, b]),
        Facing::Down => Aabb::new([a, 1.0 - t, a], [b, 1.0, b]),
        // Protrude North (−Z) ⇒ support is the +Z wall.
        Facing::North => Aabb::new([a, c, 1.0 - t], [b, d, 1.0]),
        Facing::South => Aabb::new([a, c, 0.0], [b, d, t]),
        // Protrude West (−X) ⇒ support is the +X wall.
        Facing::West => Aabb::new([1.0 - t, c, a], [1.0, d, b]),
        Facing::East => Aabb::new([0.0, c, a], [t, d, b]),
    }
}

/// Lever geometry: a small base plate flush on the support wall plus a stubby
/// handle leaning off it. `on` tilts the handle to the opposite edge (visual
/// only). Two boxes (base + handle).
fn lever_boxes(facing: Facing, on: bool) -> [Aabb; 2] {
    let t = 0.1875; // base plate depth (3/16)
    let (a, b) = (0.3125, 0.6875); // base plate span (6/16)
    // The handle is a thin stub; `on` shifts it from the low edge to the high.
    let (hl, hh) = if on { (0.5, 0.8125) } else { (0.1875, 0.5) };
    match facing {
        Facing::Up => [
            Aabb::new([a, 0.0, a], [b, t, b]),
            Aabb::new([0.4375, t, hl], [0.5625, t + 0.125, hh]),
        ],
        Facing::Down => [
            Aabb::new([a, 1.0 - t, a], [b, 1.0, b]),
            Aabb::new([0.4375, 1.0 - t - 0.125, hl], [0.5625, 1.0 - t, hh]),
        ],
        Facing::North => [
            Aabb::new([a, a, 1.0 - t], [b, b, 1.0]),
            Aabb::new([0.4375, hl, 1.0 - t - 0.125], [0.5625, hh, 1.0 - t]),
        ],
        Facing::South => [
            Aabb::new([a, a, 0.0], [b, b, t]),
            Aabb::new([0.4375, hl, t], [0.5625, hh, t + 0.125]),
        ],
        Facing::West => [
            Aabb::new([1.0 - t, a, a], [1.0, b, b]),
            Aabb::new([1.0 - t - 0.125, hl, 0.4375], [1.0 - t, hh, 0.5625]),
        ],
        Facing::East => [
            Aabb::new([0.0, a, a], [t, b, b]),
            Aabb::new([t, hl, 0.4375], [t + 0.125, hh, 0.5625]),
        ],
    }
}

/// Pressure-plate geometry: a thin flat plate covering most of the cell floor.
fn pressure_plate_box() -> Aabb {
    Aabb::new([0.0625, 0.0, 0.0625], [0.9375, 0.0625, 0.9375])
}

/// Standing-sign geometry: a thin board on a short centre post. `facing` is the
/// direction the board's face points (N/S → board spans X, thin in Z; E/W →
/// board spans Z, thin in X). Two boxes (post + board).
fn sign_boxes(facing: Facing) -> [Aabb; 2] {
    // Centre post: a thin stick from the floor to mid-height.
    let post = Aabb::new([0.45, 0.0, 0.45], [0.55, 0.5, 0.55]);
    let board = match facing {
        Facing::West | Facing::East => Aabb::new([0.45, 0.5, 0.1], [0.55, 1.0, 0.9]),
        // N/S and the vertical fallbacks present a Z-thin board.
        _ => Aabb::new([0.1, 0.5, 0.45], [0.9, 1.0, 0.55]),
    };
    [post, board]
}

/// Item-frame plate: a 7/8 × 7/8 thin (1/16) panel flush against the support
/// wall opposite `facing` (the clicked face's outward normal).
fn item_frame_plate(facing: Facing) -> Aabb {
    let t = 0.0625; // 1/16 thick
    let (a, b) = (0.0625, 0.9375); // 7/8 square inset
    match facing {
        Facing::Up => Aabb::new([a, 0.0, a], [b, t, b]),
        Facing::Down => Aabb::new([a, 1.0 - t, a], [b, 1.0, b]),
        // Protrude North (−Z) ⇒ support is the +Z wall.
        Facing::North => Aabb::new([a, a, 1.0 - t], [b, b, 1.0]),
        Facing::South => Aabb::new([a, a, 0.0], [b, b, t]),
        Facing::West => Aabb::new([1.0 - t, a, a], [1.0, b, b]),
        Facing::East => Aabb::new([0.0, a, a], [t, b, b]),
    }
}

/// The small cube a *framed item* renders as, sitting just off the plate toward
/// `facing`. Public so the mesher can place the framed-item texture there.
pub fn item_frame_item_cube(facing: Facing) -> Aabb {
    let near = 0.0625; // plate thickness
    let far = 0.25; // item depth off the plate
    let (a, b) = (0.25, 0.75); // 1/2-cell item square
    match facing {
        Facing::Up => Aabb::new([a, near, a], [b, far, b]),
        Facing::Down => Aabb::new([a, 1.0 - far, a], [b, 1.0 - near, b]),
        Facing::North => Aabb::new([a, a, 1.0 - far], [b, b, 1.0 - near]),
        Facing::South => Aabb::new([a, a, near], [b, b, far]),
        Facing::West => Aabb::new([1.0 - far, a, a], [1.0 - near, b, b]),
        Facing::East => Aabb::new([near, a, a], [far, b, b]),
    }
}

/// Map a clicked face's outward normal to the `Facing` a wall-mounted shape
/// (button, lever) should protrude toward. `[0,1,0]` (top) → `Up`, etc.
pub fn facing_from_face_normal(face_normal: [i32; 3]) -> Facing {
    match face_normal {
        [0, 1, 0] => Facing::Up,
        [0, -1, 0] => Facing::Down,
        [0, 0, -1] => Facing::North,
        [0, 0, 1] => Facing::South,
        [-1, 0, 0] => Facing::West,
        [1, 0, 0] => Facing::East,
        _ => Facing::Up,
    }
}

/// The single box a trapdoor occupies for (facing, open, top-mounted).
fn trapdoor_box(facing: Facing, open: bool, top: bool) -> Aabb {
    let t = TRAPDOOR_THICK;
    if open {
        // A thin vertical flap against the `facing` wall.
        match facing {
            Facing::North => Aabb::new([0.0, 0.0, 0.0], [1.0, 1.0, t]),
            Facing::South => Aabb::new([0.0, 0.0, 1.0 - t], [1.0, 1.0, 1.0]),
            Facing::West => Aabb::new([0.0, 0.0, 0.0], [t, 1.0, 1.0]),
            _ => Aabb::new([1.0 - t, 0.0, 0.0], [1.0, 1.0, 1.0]), // East + vertical fallback
        }
    } else if top {
        Aabb::new([0.0, 1.0 - t, 0.0], [1.0, 1.0, 1.0])
    } else {
        Aabb::new([0.0, 0.0, 0.0], [1.0, t, 1.0])
    }
}

/// The solid boxes a shaped block occupies, for collision. `FullCube` returns a
/// single unit cube (callers usually special-case it to avoid the allocation).
pub fn collision_aabbs(shape: BlockShape, m: u8) -> Vec<Aabb> {
    match shape {
        BlockShape::FullCube => vec![Aabb::FULL],
        BlockShape::SnowLayer => vec![snow_layer_box(meta::aux(m))],
        BlockShape::Slab => vec![slab_aabb(meta::facing(m))],
        BlockShape::Stairs => stairs_aabbs(meta::facing(m), stairs_is_top(m)).to_vec(),
        BlockShape::FenceGate => {
            // Open → passable (no collision); closed → the blocking panel.
            if is_open(m) {
                Vec::new()
            } else {
                vec![fence_gate_panel(meta::facing(m))]
            }
        }
        BlockShape::Trapdoor => {
            vec![trapdoor_box(meta::facing(m), is_open(m), trapdoor_is_top(m))]
        }
        // Connecting shapes: `m` is a CONN_* mask, not stored meta.
        BlockShape::Pane => pane_cuboids(m),
        BlockShape::Wall => wall_cuboids(m),
        BlockShape::Door => {
            vec![door_box(meta::facing(m), is_open(m), door_hinge_right(m))]
        }
        // Signal-input shapes are pass-through (no collision) — they only
        // render until the Electricity arc gives them power behaviour.
        BlockShape::Button | BlockShape::Lever | BlockShape::PressurePlate => Vec::new(),
        // Signs + item frames are pass-through; you read/look, not bump.
        BlockShape::Sign | BlockShape::ItemFrame => Vec::new(),
    }
}

/// The boxes a shaped block draws. Identical to the collision boxes for the
/// solid shapes here (slabs, stairs); kept separate because later shapes
/// (panes, fences) render thinner than they collide, or vice-versa. The mesher
/// textures each box by face direction (+Y→tex_top, −Y→tex_bottom, sides→tex_side).
pub fn render_cuboids(shape: BlockShape, m: u8) -> Vec<Aabb> {
    match shape {
        BlockShape::FullCube => vec![Aabb::FULL],
        BlockShape::SnowLayer => vec![snow_layer_box(meta::aux(m))],
        BlockShape::Slab => vec![slab_aabb(meta::facing(m))],
        BlockShape::Stairs => stairs_aabbs(meta::facing(m), stairs_is_top(m)).to_vec(),
        BlockShape::FenceGate => {
            // Open → two end posts (passage clear); closed → the panel.
            if is_open(m) {
                fence_gate_open_posts(meta::facing(m)).to_vec()
            } else {
                vec![fence_gate_panel(meta::facing(m))]
            }
        }
        BlockShape::Trapdoor => {
            vec![trapdoor_box(meta::facing(m), is_open(m), trapdoor_is_top(m))]
        }
        // Connecting shapes: `m` is a CONN_* mask, not stored meta.
        BlockShape::Pane => pane_cuboids(m),
        BlockShape::Wall => wall_cuboids(m),
        BlockShape::Door => {
            vec![door_box(meta::facing(m), is_open(m), door_hinge_right(m))]
        }
        BlockShape::Button => vec![button_box(meta::facing(m))],
        BlockShape::Lever => lever_boxes(meta::facing(m), is_open(m)).to_vec(),
        BlockShape::PressurePlate => vec![pressure_plate_box()],
        BlockShape::Sign => sign_boxes(meta::facing(m)).to_vec(),
        BlockShape::ItemFrame => vec![item_frame_plate(meta::facing(m))],
    }
}

/// The thin bottom slice of a snow layer: `(aux + 1) / 8` blocks tall.
fn snow_layer_box(aux: u8) -> Aabb {
    Aabb {
        min: [0.0, 0.0, 0.0],
        max: [1.0, (aux.min(7) as f32 + 1.0) * 0.125, 1.0],
    }
}

/// Place-time orientation for a **slab** from the face the player clicked and
/// where on that face (vertical fraction) they aimed.
///
/// - Click the **top** of a block (normal +Y) → a bottom slab (rests on it).
/// - Click the **bottom** (normal −Y) → a top slab.
/// - Click a **side** → a vertical slab against that side, UNLESS the aim is in
///   the top/bottom third of the face, which gives a top/bottom horizontal slab
///   (matches Minecraft's "click high/low on a side" affordance).
pub fn slab_placement_facing(face_normal: [i32; 3], hit_y_frac: f32) -> Facing {
    match face_normal {
        [0, 1, 0] => Facing::Down,
        [0, -1, 0] => Facing::Up,
        // Side click: vertical slab toward the clicked face's own cell, i.e.
        // the slab fills the half nearest the neighbour you placed against —
        // that is the *opposite* of the outward normal.
        [0, 0, 1] => horizontal_or(hit_y_frac, Facing::North), // placed on a south face → fill north half
        [0, 0, -1] => horizontal_or(hit_y_frac, Facing::South),
        [1, 0, 0] => horizontal_or(hit_y_frac, Facing::West),
        [-1, 0, 0] => horizontal_or(hit_y_frac, Facing::East),
        _ => Facing::Down,
    }
}

/// On a side click, the top/bottom third of the face yields a horizontal slab;
/// the middle yields the given vertical facing.
fn horizontal_or(hit_y_frac: f32, vertical: Facing) -> Facing {
    if hit_y_frac >= 0.667 {
        Facing::Up
    } else if hit_y_frac <= 0.333 {
        Facing::Down
    } else {
        vertical
    }
}

/// Place-time orientation for **stairs**: face the way the player is looking so
/// the tall back is behind the step they walk up. `(fwd_x, fwd_z)` is the
/// camera's horizontal forward vector (`Camera::forward()`); the clicked face
/// picks the half (bottom face → upside-down stair).
pub fn stairs_placement(fwd_x: f32, fwd_z: f32, face_normal: [i32; 3]) -> (Facing, bool) {
    let facing = facing_from_forward(fwd_x, fwd_z);
    let top_half = matches!(face_normal, [0, -1, 0]);
    (facing, top_half)
}

/// Map a horizontal forward vector to the nearest cardinal facing. Mirrors the
/// engine's existing place-time facing derivation (the power-block handler in
/// `game_loop`), so shaped blocks orient like every other directional block.
/// Engine convention: `Camera::forward()` at yaw 0 points toward −Z (North).
pub fn facing_from_forward(fwd_x: f32, fwd_z: f32) -> Facing {
    if fwd_x.abs() > fwd_z.abs() {
        if fwd_x > 0.0 {
            Facing::East
        } else {
            Facing::West
        }
    } else if fwd_z > 0.0 {
        Facing::South
    } else {
        Facing::North
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_facing(f: Facing) -> u8 {
        meta::with_facing(0, f)
    }

    #[test]
    fn snow_layer_boxes_scale_with_aux_layers() {
        // 1 layer = 1/8 block; 8 layers = a full cube's height.
        let one = collision_aabbs(BlockShape::SnowLayer, crate::meta::with_aux(0, 0));
        assert_eq!(one.len(), 1);
        assert!((one[0].max[1] - 0.125).abs() < 1e-6, "single layer is 1/8 tall");
        let eight = collision_aabbs(BlockShape::SnowLayer, crate::meta::with_aux(0, 7));
        assert!((eight[0].max[1] - 1.0).abs() < 1e-6, "8 layers reach the cell top");
        assert_eq!(
            render_cuboids(BlockShape::SnowLayer, crate::meta::with_aux(0, 3)),
            collision_aabbs(BlockShape::SnowLayer, crate::meta::with_aux(0, 3)),
            "render matches collision"
        );
        assert_eq!(shape_of(block::SNOW_LAYER), BlockShape::SnowLayer);
    }

    #[test]
    fn shape_of_maps_known_blocks_else_full_cube() {
        assert_eq!(shape_of(block::STONE_SLAB), BlockShape::Slab);
        assert_eq!(shape_of(block::STONE_STAIRS), BlockShape::Stairs);
        assert_eq!(shape_of(block::STONE), BlockShape::FullCube);
        assert_eq!(shape_of(block::AIR), BlockShape::FullCube);
        assert!(is_shaped(block::STONE_SLAB));
        assert!(!is_shaped(block::STONE));
    }

    #[test]
    fn full_cube_is_one_unit_box() {
        assert_eq!(collision_aabbs(BlockShape::FullCube, 0), vec![Aabb::FULL]);
        assert_eq!(render_cuboids(BlockShape::FullCube, 0), vec![Aabb::FULL]);
    }

    #[test]
    fn bottom_slab_fills_lower_half() {
        let a = collision_aabbs(BlockShape::Slab, meta_facing(Facing::Down));
        assert_eq!(a, vec![Aabb::new([0.0, 0.0, 0.0], [1.0, 0.5, 1.0])]);
    }

    #[test]
    fn top_slab_fills_upper_half() {
        let a = collision_aabbs(BlockShape::Slab, meta_facing(Facing::Up));
        assert_eq!(a, vec![Aabb::new([0.0, 0.5, 0.0], [1.0, 1.0, 1.0])]);
    }

    #[test]
    fn vertical_slabs_cover_all_four_sides() {
        // #27 — each side-facing fills the matching half, full height.
        assert_eq!(
            collision_aabbs(BlockShape::Slab, meta_facing(Facing::North))[0],
            Aabb::new([0.0, 0.0, 0.0], [1.0, 1.0, 0.5])
        );
        assert_eq!(
            collision_aabbs(BlockShape::Slab, meta_facing(Facing::East))[0],
            Aabb::new([0.5, 0.0, 0.0], [1.0, 1.0, 1.0])
        );
        // Each vertical slab is full-height (distinguishes it from horizontal).
        for f in [Facing::North, Facing::South, Facing::West, Facing::East] {
            let b = collision_aabbs(BlockShape::Slab, meta_facing(f))[0];
            assert_eq!(b.min[1], 0.0);
            assert_eq!(b.max[1], 1.0);
        }
    }

    #[test]
    fn render_matches_collision_for_solid_shapes() {
        for f in Facing::ALL {
            let m = meta_facing(f);
            assert_eq!(
                collision_aabbs(BlockShape::Slab, m),
                render_cuboids(BlockShape::Slab, m)
            );
        }
    }

    #[test]
    fn stairs_are_two_boxes_slab_plus_step() {
        let a = collision_aabbs(BlockShape::Stairs, meta_facing(Facing::North));
        assert_eq!(a.len(), 2);
        // Lower slab.
        assert_eq!(a[0], Aabb::new([0.0, 0.0, 0.0], [1.0, 0.5, 1.0]));
        // Upper quarter on the north (−Z) half.
        assert_eq!(a[1], Aabb::new([0.0, 0.5, 0.0], [1.0, 1.0, 0.5]));
    }

    #[test]
    fn upside_down_stairs_flip_to_top() {
        let m = meta::with_state(meta_facing(Facing::North), 1);
        assert!(stairs_is_top(m));
        let a = collision_aabbs(BlockShape::Stairs, m);
        // Top slab now.
        assert_eq!(a[0], Aabb::new([0.0, 0.5, 0.0], [1.0, 1.0, 1.0]));
        // Lower quarter on the north half.
        assert_eq!(a[1], Aabb::new([0.0, 0.0, 0.0], [1.0, 0.5, 0.5]));
    }

    #[test]
    fn fence_gate_closed_blocks_open_passes() {
        let closed = meta_facing(Facing::North); // open bit 0 = closed
        let c = collision_aabbs(BlockShape::FenceGate, closed);
        assert_eq!(c.len(), 1, "closed gate has a blocking panel");
        // The panel spans the full cell on X and is thin on Z (blocks N/S).
        assert_eq!(c[0].min[0], 0.0);
        assert_eq!(c[0].max[0], 1.0);
        assert!(c[0].max[2] - c[0].min[2] < 0.2);

        let open = toggled_open(closed);
        assert!(is_open(open));
        assert!(
            collision_aabbs(BlockShape::FenceGate, open).is_empty(),
            "open gate is passable"
        );
        // Open still renders something (two end posts) so it doesn't vanish.
        assert_eq!(render_cuboids(BlockShape::FenceGate, open).len(), 2);
    }

    #[test]
    fn trapdoor_closed_is_a_thin_lid_open_is_a_wall() {
        // Closed, bottom-mounted → a flat lid at the floor (thin in Y).
        let closed = meta_facing(Facing::North);
        let c = collision_aabbs(BlockShape::Trapdoor, closed);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].min[1], 0.0);
        assert!(c[0].max[1] < 0.2, "lid is thin in Y");
        assert_eq!(c[0].max[0], 1.0);

        // Top-mounted (state bit 1) → lid at the ceiling.
        let top = meta::with_state(closed, 0b10);
        assert!(trapdoor_is_top(top));
        let ct = collision_aabbs(BlockShape::Trapdoor, top)[0];
        assert_eq!(ct.max[1], 1.0);
        assert!(ct.min[1] > 0.8);

        // Open → a thin vertical flap against the north wall (thin in Z).
        let open = toggled_open(closed);
        let o = collision_aabbs(BlockShape::Trapdoor, open)[0];
        assert_eq!(o.max[1], 1.0, "flap is full height");
        assert!(o.max[2] < 0.2, "flap is thin in Z (north wall)");
    }

    #[test]
    fn door_closed_blocks_doorway_open_swings_aside() {
        // Closed, facing North → thin panel flush on the north wall (z≈0).
        let closed = meta_facing(Facing::North);
        let c = collision_aabbs(BlockShape::Door, closed)[0];
        assert_eq!(c.min[2], 0.0);
        assert!(c.max[2] < 0.2, "closed door is flush on the facing wall");
        assert_eq!(c.max[0], 1.0, "spans the doorway");
        // Open (left hinge) → swung to a perpendicular (west) wall (x≈0, thin X).
        let open = toggled_open(closed);
        assert!(is_open(open));
        let o = collision_aabbs(BlockShape::Door, open)[0];
        assert!(o.max[0] < 0.2, "open door is against a perpendicular wall");
        assert_eq!(o.max[2], 1.0);
        // Right hinge swings to the opposite (east) wall.
        let open_r = meta::with_aux(open, 0b001);
        assert!(door_hinge_right(open_r));
        let or = collision_aabbs(BlockShape::Door, open_r)[0];
        assert!(or.min[0] > 0.8, "right-hinge open door swings to the east wall");
    }

    #[test]
    fn door_top_bit_and_toggleable() {
        let bottom = meta_facing(Facing::East); // is_top bit clear
        assert!(!door_is_top(bottom));
        let top = meta::with_state(bottom, 0b10);
        assert!(door_is_top(top));
        assert!(is_toggleable(block::OAK_DOOR));
        assert_eq!(shape_of(block::OAK_DOOR), BlockShape::Door);
    }

    #[test]
    fn lone_pane_is_just_a_thin_centre_post() {
        // No connections → a single thin centred post, full height.
        let p = collision_aabbs(BlockShape::Pane, 0);
        assert_eq!(p.len(), 1);
        assert!(p[0].min[0] > 0.4 && p[0].max[0] < 0.6);
        assert!(p[0].min[2] > 0.4 && p[0].max[2] < 0.6);
        assert_eq!(p[0].min[1], 0.0);
        assert_eq!(p[0].max[1], 1.0);
        assert_eq!(shape_of(block::GLASS_PANE), BlockShape::Pane);
        assert_eq!(shape_of(block::IRON_BARS), BlockShape::Pane);
        assert!(!is_toggleable(block::GLASS_PANE));
        assert!(is_connecting(BlockShape::Pane));
    }

    #[test]
    fn pane_connected_east_west_spans_full_x() {
        // Connected W and E → post + two arms reach both edges (a flat sheet).
        let ew = render_cuboids(BlockShape::Pane, CONN_W | CONN_E);
        let min_x = ew.iter().map(|b| b.min[0]).fold(1.0f32, f32::min);
        let max_x = ew.iter().map(|b| b.max[0]).fold(0.0f32, f32::max);
        assert_eq!(min_x, 0.0);
        assert_eq!(max_x, 1.0);
        // Still thin in Z everywhere.
        assert!(ew.iter().all(|b| b.max[2] - b.min[2] < 0.2));
    }

    #[test]
    fn lone_wall_is_a_central_post() {
        let w = collision_aabbs(BlockShape::Wall, 0);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0], Aabb::new([0.25, 0.0, 0.25], [0.75, 1.0, 0.75]));
        assert_eq!(shape_of(block::COBBLESTONE_WALL), BlockShape::Wall);
        assert!(is_connecting(BlockShape::Wall));
    }

    #[test]
    fn wall_grows_an_arm_per_connection() {
        // North + East connections → post + 2 arms reaching those edges.
        let w = render_cuboids(BlockShape::Wall, CONN_N | CONN_E);
        assert_eq!(w.len(), 3);
        // An arm reaches the north (-Z) edge…
        assert!(w.iter().any(|b| b.min[2] == 0.0 && b.max[2] <= 0.5));
        // …and the east (+X) edge.
        assert!(w.iter().any(|b| b.max[0] == 1.0 && b.min[0] >= 0.5));
        // Arms are short (13/16), the post is full height.
        assert!(w.iter().filter(|b| b.max[1] < 1.0).all(|b| (b.max[1] - 0.8125).abs() < 1e-6));
    }

    #[test]
    fn signal_shapes_are_flush_and_pass_through() {
        // Button: a small nub against the support wall, no collision.
        let bn = render_cuboids(BlockShape::Button, meta_facing(Facing::South));
        assert_eq!(bn.len(), 1);
        // South-protruding ⇒ slab on the −Z (north) wall, thin in Z.
        assert!(bn[0].max[2] <= 0.125);
        assert!(collision_aabbs(BlockShape::Button, meta_facing(Facing::South)).is_empty());

        // Floor button sits flush on the cell floor.
        let up = render_cuboids(BlockShape::Button, meta_facing(Facing::Up))[0];
        assert_eq!(up.min[1], 0.0);
        assert!(up.max[1] <= 0.125);

        // Pressure plate is a thin floor plate, pass-through.
        let pp = render_cuboids(BlockShape::PressurePlate, 0);
        assert_eq!(pp.len(), 1);
        assert!(pp[0].max[1] <= 0.0625);
        assert!(collision_aabbs(BlockShape::PressurePlate, 0).is_empty());

        // Lever renders a base + handle, no collision.
        let lv = render_cuboids(BlockShape::Lever, meta_facing(Facing::Up));
        assert_eq!(lv.len(), 2);
        assert!(collision_aabbs(BlockShape::Lever, meta_facing(Facing::Up)).is_empty());
        // The Electricity handler owns the lever toggle, NOT the generic path.
        assert!(!is_toggleable(block::LEVER));
        assert!(!is_toggleable(block::BUTTON));
        // The existing power-input blocks now resolve to flush shapes.
        assert_eq!(shape_of(block::BUTTON), BlockShape::Button);
        assert_eq!(shape_of(block::LEVER), BlockShape::Lever);
        assert_eq!(shape_of(block::PRESSURE_PLATE), BlockShape::PressurePlate);
        assert!(is_signal_input(block::LEVER));
        assert!(!is_signal_input(block::COBBLESTONE_WALL));
        // The handle box shifts when the on-bit (meta state bit 0) flips.
        let off = render_cuboids(BlockShape::Lever, meta_facing(Facing::Up));
        let on = render_cuboids(BlockShape::Lever, meta::with_state(meta_facing(Facing::Up), 1));
        assert_ne!(off[1], on[1], "handle box shifts when toggled");
    }

    #[test]
    fn sign_is_post_plus_facing_board_and_pass_through() {
        // N-facing → board spans X, thin in Z; post underneath.
        let ns = render_cuboids(BlockShape::Sign, meta_facing(Facing::North));
        assert_eq!(ns.len(), 2);
        let board = ns[1];
        assert!(board.min[0] < 0.2 && board.max[0] > 0.8, "board spans X");
        assert!(board.max[2] - board.min[2] < 0.2, "board thin in Z");
        assert!(board.min[1] >= 0.5, "board sits on the upper half");
        // E-facing rotates the board to span Z instead.
        let ew = render_cuboids(BlockShape::Sign, meta_facing(Facing::East))[1];
        assert!(ew.min[2] < 0.2 && ew.max[2] > 0.8, "board spans Z when facing E/W");
        // Signs never collide.
        assert!(collision_aabbs(BlockShape::Sign, meta_facing(Facing::North)).is_empty());
        assert_eq!(shape_of(block::OAK_SIGN), BlockShape::Sign);
    }

    #[test]
    fn item_frame_is_a_thin_flush_plate_pass_through() {
        // South-facing (clicked a block's south face) ⇒ plate on the −Z wall.
        let p = render_cuboids(BlockShape::ItemFrame, meta_facing(Facing::South));
        assert_eq!(p.len(), 1);
        assert!(p[0].max[2] <= 0.0625, "plate is thin + flush on the wall");
        assert!(p[0].min[0] > 0.0 && p[0].max[0] < 1.0, "plate is inset (a frame border)");
        // Pass-through.
        assert!(collision_aabbs(BlockShape::ItemFrame, meta_facing(Facing::South)).is_empty());
        // The framed-item cube sits off the plate toward the viewer.
        let cube = item_frame_item_cube(Facing::South);
        assert!(cube.min[2] >= 0.0625 && cube.max[2] <= 0.25);
        assert_eq!(shape_of(block::ITEM_FRAME), BlockShape::ItemFrame);
    }

    #[test]
    fn facing_from_face_normal_maps_cardinals() {
        assert_eq!(facing_from_face_normal([0, 1, 0]), Facing::Up);
        assert_eq!(facing_from_face_normal([0, -1, 0]), Facing::Down);
        assert_eq!(facing_from_face_normal([0, 0, -1]), Facing::North);
        assert_eq!(facing_from_face_normal([0, 0, 1]), Facing::South);
        assert_eq!(facing_from_face_normal([-1, 0, 0]), Facing::West);
        assert_eq!(facing_from_face_normal([1, 0, 0]), Facing::East);
    }

    #[test]
    fn connects_rules_by_family_and_full_solids() {
        let reg = block::BlockRegistry::new();
        // Wall ↔ wall, pane ↔ pane.
        assert!(connects(BlockShape::Wall, block::COBBLESTONE_WALL, &reg));
        assert!(connects(BlockShape::Pane, block::GLASS_PANE, &reg));
        assert!(connects(BlockShape::Pane, block::IRON_BARS, &reg));
        // Cross-family does NOT connect.
        assert!(!connects(BlockShape::Wall, block::GLASS_PANE, &reg));
        assert!(!connects(BlockShape::Pane, block::COBBLESTONE_WALL, &reg));
        // Both connect to a full-cube solid; never to air.
        assert!(connects(BlockShape::Wall, block::STONE, &reg));
        assert!(connects(BlockShape::Pane, block::STONE, &reg));
        assert!(!connects(BlockShape::Wall, block::AIR, &reg));
        // A shaped neighbour that isn't same-family doesn't connect (no flat face).
        assert!(!connects(BlockShape::Wall, block::STONE_SLAB, &reg));
    }

    #[test]
    fn toggleable_blocks_are_flagged() {
        assert!(is_toggleable(block::OAK_FENCE_GATE));
        assert!(is_toggleable(block::OAK_TRAPDOOR));
        assert!(!is_toggleable(block::STONE_SLAB));
        assert!(!is_toggleable(block::STONE));
    }

    #[test]
    fn open_toggle_preserves_facing() {
        let m = meta_facing(Facing::East);
        let m2 = toggled_open(m);
        assert!(is_open(m2));
        assert_eq!(meta::facing(m2), Facing::East, "facing survives the toggle");
        // Toggling back closes it.
        assert!(!is_open(toggled_open(m2)));
    }

    #[test]
    fn slab_placement_top_face_gives_bottom_slab() {
        assert_eq!(slab_placement_facing([0, 1, 0], 0.5), Facing::Down);
    }

    #[test]
    fn slab_placement_bottom_face_gives_top_slab() {
        assert_eq!(slab_placement_facing([0, -1, 0], 0.5), Facing::Up);
    }

    #[test]
    fn slab_placement_side_middle_gives_vertical() {
        // South-face click in the middle → vertical slab filling the north half.
        assert_eq!(slab_placement_facing([0, 0, 1], 0.5), Facing::North);
        assert_eq!(slab_placement_facing([1, 0, 0], 0.5), Facing::West);
    }

    #[test]
    fn slab_placement_side_high_low_gives_horizontal() {
        assert_eq!(slab_placement_facing([0, 0, 1], 0.9), Facing::Up);
        assert_eq!(slab_placement_facing([0, 0, 1], 0.1), Facing::Down);
    }

    #[test]
    fn facing_from_forward_cardinals() {
        // Camera::forward() at yaw 0 points toward -Z (North).
        assert_eq!(facing_from_forward(0.0, -1.0), Facing::North);
        assert_eq!(facing_from_forward(0.0, 1.0), Facing::South);
        assert_eq!(facing_from_forward(1.0, 0.0), Facing::East);
        assert_eq!(facing_from_forward(-1.0, 0.0), Facing::West);
        // Dominant axis wins on a diagonal.
        assert_eq!(facing_from_forward(0.9, -0.2), Facing::East);
        assert_eq!(facing_from_forward(0.2, -0.9), Facing::North);
    }

    #[test]
    fn stairs_placement_faces_camera_direction() {
        // Looking North, clicking a top face → north-facing bottom stair.
        let (f, top) = stairs_placement(0.0, -1.0, [0, 1, 0]);
        assert_eq!(f, Facing::North);
        assert!(!top);
        // Clicking a bottom face → upside-down stair.
        let (_, top2) = stairs_placement(0.0, 1.0, [0, -1, 0]);
        assert!(top2);
    }
}
