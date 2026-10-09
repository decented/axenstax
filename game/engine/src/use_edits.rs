//! C3c-1 (2026-10-08, protocol v80) — a player's block-edit USES: the
//! right-clicks that change one cell and the hand together (a bucket filled
//! or emptied, a seed or a papyrus reed planted, bone meal on grass or a
//! crop, fertiliser, salt, an Eraser on paper, a rubber tap, a hoe), a
//! door's top half, and (C3c-3a, v83) a developed Plan hung as a cyanotype
//! print, its `used` the Plan by marker (`protocol::WireItem::Plan`).
//!
//! Three things live here, one rule each, run by every side that needs it:
//!
//! - **The rules.** What a use turns a cell into, lifted out of the
//!   right-click arms (`game_loop.rs`) where they were inline
//!   ([`sown_crop`], [`grows_tall_grass`], [`tills`], [`erases`],
//!   [`door_top_meta`]), beside the pure ones that already existed
//!   (`bucket::fill_result`/`empty_result`, `growth::bonemeal_advance`/
//!   `next_stage`, `papyrus::is_valid_planting_base`,
//!   `snowfall::target_block_accepts_salt_path`, `rubber::is_tappable`).
//!   Single-player and a joined client run them in the arms; the server runs
//!   them again to judge a joiner's claimed outcome ([`judge`]).
//! - **The tag.** A joined client's use edit carries a [`UseTag`]
//!   (`InputPacket::use_tags`): the use's [`UseKind`] and the hand BEFORE the
//!   use — its hotbar slot, the item it consumed, the tool it wore — stamped
//!   where the use is made ([`tag`]), because the edit's own `EditHand` is
//!   stamped after the use spent its item.
//! - **The mirror.** The server applies a joiner's accepted use to its copy
//!   of the joiner's inventory by the client's own steps ([`settle`]): take
//!   what it used (the shared owed-take search, `joiner_actions::take_owed`,
//!   from the tag's slot first), wear its tool (`joiner_inventory::wear_tool`,
//!   the client's `use_tool_at`), add what it made with the client's own
//!   `Inventory::add_item` (first matching stack, else first empty slot), so
//!   in lockstep both sides land it in the same slot. C3c-1-fix (M-2) — a
//!   use's overflow is what the CLIENT says didn't fit its bag
//!   (`UseTag::unfit`): the server spawns exactly that as a real ground item
//!   at the joiner (a joined client spills nothing of its own) and the copy
//!   adds the rest; what of that doesn't fit the copy is only counted.
//! - **The undo** (C3c-1-fix, M-4). A joined client keeps its own record of
//!   each use ([`SentUses`]); when the server refuses the use's edit (reach,
//!   a plot, the play mode…) it says so (`StateUpdatePacket::refused_uses`),
//!   and the client takes back what landed and gives back what it spent
//!   ([`undo`]), from its record, never from the notice.
//!
//! **Log-only, like all of C3a–C3c.** Nothing the edit check accepts is
//! refused for its use (the one exception, C3c-1-fix: a door's top half with
//! no door below, which costs nothing): the edit is applied whatever the
//! verdict ([`Verdict`]). C3c-1-fix (L-1) — the copy TRACKS the client: an
//! outcome the server's world had moved on from but the rule makes (honest
//! drift) mirrors the cost and the product; only a combination the rule
//! never makes (water poured from an empty bucket) mirrors the cost alone;
//! a tag that can't explain its edit at all is not a use (the edit is
//! classified as an ordinary one). Each miss is a `use_mismatch`
//! (`PossessionTally`). The mirror never makes an item from nothing: a use
//! whose cost (or, for a no-cost use, its required item or tool) the copy
//! can't pay adds no product.

use crate::block::{self, BlockId};
use crate::crafting::{Tool, ToolType};
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack, MaterialId};
use crate::joiner_inventory::WearCheck;
use crate::protocol::{BlockChange, UseTag, WireItem};
use crate::world::World;

/// One kind of block-edit use, one per RULE (not per item): bone meal and
/// fertiliser on a crop are one rule, every seed is one rule.
///
/// **On the wire as a `u8` ([`UseTag::kind`]), APPEND-ONLY**: never renumber
/// or reuse a value ([`Self::to_wire`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UseKind {
    /// An empty Bucket filled from a water or lava source: the source cell
    /// becomes AIR, the bucket the filled one (`bucket::fill_result`).
    BucketFill,
    /// A filled bucket emptied into an AIR cell: the cell becomes its fluid,
    /// the bucket the empty one (`bucket::empty_result`).
    BucketEmpty,
    /// A seed (wheat, carrot, potato, corn, cotton, hemp, cornflower, field
    /// poppy, buttercup) sown into the AIR above tilled soil ([`sown_crop`]).
    Sow,
    /// A papyrus reed planted above dirt, grass or sand by water
    /// (`papyrus::is_valid_planting_base`).
    PlantPapyrus,
    /// Bone meal on grass: tall grass in the AIR above ([`grows_tall_grass`]).
    GrowGrass,
    /// Bone meal (one or two stages, `growth::bonemeal_advance`) or
    /// fertiliser (one, `growth::next_stage`) on a growing crop.
    GrowCrop,
    /// Salt on grass, dirt or snow: a salt path
    /// (`snowfall::target_block_accepts_salt_path`).
    Salt,
    /// An Eraser on blueprint paper: the paper's cell becomes AIR, one
    /// Papyrus Sheet comes back, the Eraser wears ([`erases`]).
    Erase,
    /// An empty Bucket on a live rubber log: the log is tapped, one Rubber
    /// comes back, the bucket is kept (`rubber::is_tappable`).
    TapRubber,
    /// A hoe on the top of dirt or grass: tilled soil, the hoe wears ([`tills`]).
    Till,
    /// A door's top half, placed with its bottom half (the bottom's generic
    /// placement paid for the door): no cost, no gain.
    DoorUpper,
    /// C3c-3a (v83) — a developed Plan hung on a wall: the AIR (or water)
    /// cell before a wall face becomes a CYANOTYPE_PRINT and the Plan is
    /// spent (its body is gone: the print keeps no plan data). The tag's
    /// `used` is the Plan by marker; the server takes its marker placeholder.
    HangPrint,
}

impl UseKind {
    /// Every kind, in wire order: `ALL[k].to_wire() == k`.
    pub const ALL: [UseKind; 12] = [
        UseKind::BucketFill,
        UseKind::BucketEmpty,
        UseKind::Sow,
        UseKind::PlantPapyrus,
        UseKind::GrowGrass,
        UseKind::GrowCrop,
        UseKind::Salt,
        UseKind::Erase,
        UseKind::TapRubber,
        UseKind::Till,
        UseKind::DoorUpper,
        UseKind::HangPrint,
    ];

    /// The wire byte. APPEND-ONLY: pinned by
    /// `tests::use_kind_wire_bytes_are_pinned`.
    pub const fn to_wire(self) -> u8 {
        match self {
            UseKind::BucketFill => 0,
            UseKind::BucketEmpty => 1,
            UseKind::Sow => 2,
            UseKind::PlantPapyrus => 3,
            UseKind::GrowGrass => 4,
            UseKind::GrowCrop => 5,
            UseKind::Salt => 6,
            UseKind::Erase => 7,
            UseKind::TapRubber => 8,
            UseKind::Till => 9,
            UseKind::DoorUpper => 10,
            UseKind::HangPrint => 11,
        }
    }

    /// The kind a wire byte names, if any (an unknown byte is a newer or
    /// modified peer's).
    pub fn from_wire(b: u8) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.to_wire() == b)
    }

    /// Does the use consume the item in hand (one of the tag's `used`)?
    pub const fn consumes(self) -> bool {
        matches!(
            self,
            UseKind::BucketFill
                | UseKind::BucketEmpty
                | UseKind::Sow
                | UseKind::PlantPapyrus
                | UseKind::GrowGrass
                | UseKind::GrowCrop
                | UseKind::Salt
                | UseKind::HangPrint
        )
    }

    /// Does the use wear the tool in hand (the tag's `tool`)?
    pub const fn wears(self) -> bool {
        matches!(self, UseKind::Erase | UseKind::Till)
    }

    /// For the log.
    pub const fn label(self) -> &'static str {
        match self {
            UseKind::BucketFill => "bucket fill",
            UseKind::BucketEmpty => "bucket empty",
            UseKind::Sow => "sowing",
            UseKind::PlantPapyrus => "papyrus planting",
            UseKind::GrowGrass => "bone meal on grass",
            UseKind::GrowCrop => "crop accelerator",
            UseKind::Salt => "salt",
            UseKind::Erase => "eraser",
            UseKind::TapRubber => "rubber tap",
            UseKind::Till => "hoe",
            UseKind::DoorUpper => "door top half",
            UseKind::HangPrint => "cyanotype hang",
        }
    }

    /// A bit of its own, for a set of kinds (`PossessionTally`'s logged set).
    pub const fn bit(self) -> u16 {
        1 << self.to_wire()
    }
}

// ─── The rules (shared by single-player, a joined client and the server) ──

/// The stage-0 crop `seed` sows into the AIR above tilled soil, if it is a
/// seed. Was the inline table of the tilled-soil right-click arm.
pub fn sown_crop(seed: MaterialId) -> Option<BlockId> {
    Some(match seed {
        MaterialId::WheatSeeds => block::WHEAT_STAGE_0,
        MaterialId::Carrot => block::CARROT_STAGE_0,
        MaterialId::Potato => block::POTATO_STAGE_0,
        // Wave 28 — corn plants from CornSeeds (separate-seed pattern, like wheat).
        MaterialId::CornSeeds => block::CORN_STAGE_0,
        // Spec 36 Phase 2 — fibre crops.
        MaterialId::CottonSeeds => block::COTTON_STAGE_0,
        MaterialId::HempSeeds => block::HEMP_STAGE_0,
        // Spec 35 farmable-flower follow-on.
        MaterialId::CornflowerSeeds => block::CORNFLOWER_STAGE_0,
        MaterialId::FieldPoppySeeds => block::FIELD_POPPY_STAGE_0,
        MaterialId::ButtercupSeeds => block::BUTTERCUP_STAGE_0,
        _ => return None,
    })
}

/// Wave 22 — bone meal on `target` grows tall grass in the cell above, which
/// holds `above`: only on grass, only into AIR (into anything else the bone
/// meal is kept, so a stack isn't burnt against a ceiling).
pub fn grows_tall_grass(target: BlockId, above: BlockId) -> bool {
    target == block::GRASS && above == block::AIR
}

/// Farming Tier 1 — a hoe tills the top of `target` into tilled soil.
pub fn tills(target: BlockId) -> bool {
    matches!(target, block::DIRT | block::GRASS)
}

/// Rubber feature — an Eraser lifts `target` back to a Papyrus Sheet.
pub fn erases(target: BlockId) -> bool {
    target == block::BLUEPRINT_PAPER
}

/// C3c-3a — a cyanotype print can hang in the cell `(x, y, z)`: one of its
/// four sides is a wall (a block that isn't AIR or a fluid). The client hangs
/// it on the wall face it aimed at; the server, which sees only the cell,
/// asks for a wall on some side (the nearest faithful form of "on a wall
/// face").
pub fn hangs_on_wall(world: &World, x: i32, y: i32, z: i32) -> bool {
    [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .iter()
        .any(|&(dx, dz)| !matches!(world.get_block(x + dx, y, z + dz), block::AIR | block::WATER | block::LAVA))
}

/// F1 Wave 2 — a door's top half's meta, from its bottom half's: the same
/// facing and hinge, with the top bit set.
pub fn door_top_meta(bottom_meta: u8) -> u8 {
    crate::meta::with_state(bottom_meta, crate::meta::state(bottom_meta) | 0b10)
}

/// The stages `accelerator` (Bone meal or Fertiliser) can take `crop` to:
/// bone meal one or two (`growth::bonemeal_advance` picks by its seed),
/// fertiliser one. Empty for anything else or a mature crop.
fn accelerations(accelerator: &Item, crop: BlockId) -> [Option<BlockId>; 2] {
    let one = crate::growth::next_stage(crop);
    match accelerator {
        Item::Material(MaterialId::Bonemeal) => [one, one.and_then(crate::growth::next_stage)],
        Item::Material(MaterialId::Fertiliser) => [one, None],
        _ => [None, None],
    }
}

/// What a use gives back, by the rule ([`settle`] adds it): a fill's filled
/// bucket (of the fluid the cell held), an empty's empty bucket, an Eraser's
/// sheet, a tap's rubber.
fn product(kind: UseKind, old: BlockId) -> Option<ItemStack> {
    let material = match kind {
        UseKind::BucketFill => crate::bucket::fill_result(&Item::Material(MaterialId::Bucket), old, true)?,
        UseKind::BucketEmpty => MaterialId::Bucket,
        UseKind::Erase => MaterialId::PapyrusSheet,
        UseKind::TapRubber => MaterialId::Rubber,
        _ => return None,
    };
    Some(ItemStack::new_material(material, 1))
}

// ─── The tag (client) ────────────────────────────────────────────────────

/// The tag for a use of `kind` at `cell`, made from hotbar slot `slot`
/// holding `held` — read BEFORE the use spends or wears it: `used` is one of
/// `held` when the kind consumes it, `tool` is `held`'s state when the kind
/// wears it.
pub fn tag(kind: UseKind, cell: [i32; 3], slot: usize, held: Option<&Item>) -> UseTag {
    let used = kind
        .consumes()
        .then(|| held.map(|item| crate::inventory::stack_to_wire(&ItemStack { item: item.clone(), count: 1 })))
        .flatten();
    let tool = match held {
        Some(item) if kind.wears() => crate::inventory::item_to_wire_full(item),
        _ => WireItem::None,
    };
    UseTag {
        x: cell[0],
        y: cell[1],
        z: cell[2],
        kind: kind.to_wire(),
        slot: slot.min(usize::from(u8::MAX)) as u8,
        used,
        tool,
        // Set by the arm once its `add_item` has said what didn't fit.
        unfit: 0,
    }
}

// ─── The mirror (server) ─────────────────────────────────────────────────

/// What the server reads off its world for a use, BEFORE the edit lands
/// ([`judge`]): the cell's block then and, for a fill, whether it was a
/// source.
#[derive(Clone, Copy, Debug)]
pub struct Before {
    /// The cell's block before the edit.
    pub old: BlockId,
    /// Whether the cell's fluid (if any) was a source.
    pub source: bool,
}

/// C3c-1-fix (L-1) — the server's verdict on a use's claimed outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// An outcome the use's rule produces on the server's world, for what
    /// the tag says it used.
    Legal,
    /// An outcome the rule produces for what was used, but not from the
    /// server's world as it stands: honest drift (a source that flowed, a
    /// crop that grew, a cell another player changed, a log on the server's
    /// cooldown). Log-only tracks the client: the copy mirrors the cost AND
    /// the product (tallied as a drift mismatch).
    Drift,
    /// An outcome the rule never makes for what was used, whatever the world
    /// (water poured from an empty bucket, salt sown as a seed): the cost the
    /// client says it spent is mirrored, no product.
    Impossible,
    /// The kind can't explain the edit at all: its new block is never that
    /// kind's output (a `DoorUpper` or `Till` tag on an ordinary placement),
    /// or the kind byte is unknown. Not a use: the edit falls back to its
    /// ordinary classification (`check_placement` sees it).
    Unexplained,
}

/// The server's verdict on a use's outcome, read before the edit lands.
#[derive(Clone, Debug, PartialEq)]
pub struct Judged {
    pub verdict: Verdict,
    /// What the rule gives back for this outcome (added only for a legal or
    /// drifted one whose cost the copy pays). `None` for a drifted fill whose
    /// server cell holds no fluid any more: which bucket the client filled
    /// (water or lava) isn't on the tag.
    pub product: Option<ItemStack>,
    /// L-1 — a crop accelerator or bone meal on grass whose server cell is
    /// already at or past the stage the client sent (the crop grew on the
    /// server first): the server's cell is left as it is (setting it would set
    /// the crop BACK); the cost still settles. Not a refusal.
    pub keeps_server_cell: bool,
}

impl Judged {
    /// A use of a kind this build doesn't know (a modified peer's).
    pub fn unknown_kind() -> Self {
        Judged { verdict: Verdict::Unexplained, product: None, keeps_server_cell: false }
    }

    #[cfg(test)]
    pub fn legal(&self) -> bool {
        self.verdict == Verdict::Legal
    }
}

/// Every seed `sown_crop` sows (for [`explains`]).
const SEEDS: [MaterialId; 9] = [
    MaterialId::WheatSeeds,
    MaterialId::Carrot,
    MaterialId::Potato,
    MaterialId::CornSeeds,
    MaterialId::CottonSeeds,
    MaterialId::HempSeeds,
    MaterialId::CornflowerSeeds,
    MaterialId::FieldPoppySeeds,
    MaterialId::ButtercupSeeds,
];

/// A crop stage past its first: one a crop accelerator can make (from the
/// stage before it).
fn is_grown_stage(b: BlockId) -> bool {
    crate::growth::is_crop(b) && b != block::PAPYRUS_STAGE_0 && !SEEDS.iter().any(|&m| sown_crop(m) == Some(b))
}

/// L-1 — can a use of `kind` explain an edit leaving `new` (with `meta`) at
/// all, whatever was used and whatever the world held: is `new` one of the
/// kind's outputs?
pub fn explains(kind: UseKind, new: BlockId, meta: u8) -> bool {
    match kind {
        UseKind::BucketFill | UseKind::Erase => new == block::AIR,
        UseKind::BucketEmpty => matches!(new, block::WATER | block::LAVA),
        UseKind::Sow => SEEDS.iter().any(|&m| sown_crop(m) == Some(new)),
        UseKind::PlantPapyrus => new == block::PAPYRUS_STAGE_0,
        UseKind::GrowGrass => new == block::TALL_GRASS,
        UseKind::GrowCrop => is_grown_stage(new),
        UseKind::Salt => new == block::SALT_PATH,
        UseKind::TapRubber => new == block::RUBBER_LOG_TAPPED,
        UseKind::Till => new == block::TILLED_SOIL,
        UseKind::DoorUpper => new == block::OAK_DOOR && crate::block_shape::door_is_top(meta),
        UseKind::HangPrint => new == block::CYANOTYPE_PRINT,
    }
}

/// L-1 — could the client's rule make `new` with `used`, from SOME world
/// (the one the client saw)? False only for a combination the rule never
/// makes. C3c-2-fix (F-L3) — a no-cost kind (an Eraser, a tap, a hoe, a
/// door's top half) checks nothing it used, so it is possible only where it
/// [`explains`] the edit AND the server's `old` is the rule's input or its
/// output (another player got there first: [`no_cost_drift`]); on any other
/// block it is impossible, and mirrors no product.
fn possible(kind: UseKind, used: Option<&Item>, old: BlockId, new: BlockId, meta: u8) -> bool {
    let material = |m: MaterialId| used == Some(&Item::Material(m));
    match kind {
        UseKind::BucketFill => material(MaterialId::Bucket) && new == block::AIR,
        UseKind::BucketEmpty => {
            used.and_then(|u| crate::bucket::empty_result(u, block::AIR)).is_some_and(|(fluid, _)| fluid == new)
        }
        UseKind::Sow => matches!(used, Some(Item::Material(m)) if sown_crop(*m) == Some(new)),
        UseKind::PlantPapyrus => material(MaterialId::PapyrusReed) && new == block::PAPYRUS_STAGE_0,
        UseKind::GrowGrass => material(MaterialId::Bonemeal) && new == block::TALL_GRASS,
        UseKind::GrowCrop => {
            (material(MaterialId::Bonemeal) || material(MaterialId::Fertiliser)) && is_grown_stage(new)
        }
        UseKind::Salt => material(MaterialId::Salt) && new == block::SALT_PATH,
        UseKind::Erase | UseKind::TapRubber | UseKind::Till | UseKind::DoorUpper => {
            explains(kind, new, meta) && no_cost_drift(kind, old)
        }
        // C3c-3a — only a developed Plan hangs (a latent one lays flat).
        UseKind::HangPrint => {
            matches!(used, Some(Item::Plan(p)) if p.develop_state == crate::plan::DevelopState::Developed)
                && new == block::CYANOTYPE_PRINT
        }
    }
}

/// C3c-2-fix (F-L3) — is the server's `old` the input or the output of a
/// no-cost use of `kind`: the block the client's rule works on (blueprint
/// paper, a tappable log, dirt or grass, AIR for a door's top half), or the
/// one it leaves (another player erased, tapped, tilled or hung a door there
/// first)? Anything else can't be drift. (AIR where another player broke the
/// block first is not counted for a tap or a hoe: the server keeps no record
/// of who broke a cell. Such a race is impossible here — log-only, its edit
/// still applies — and mirrors no rubber.)
fn no_cost_drift(kind: UseKind, old: BlockId) -> bool {
    match kind {
        UseKind::Erase => erases(old) || old == block::AIR,
        UseKind::TapRubber => crate::rubber::is_tappable(old) || old == block::RUBBER_LOG_TAPPED,
        UseKind::Till => tills(old) || old == block::TILLED_SOIL,
        UseKind::DoorUpper => old == block::AIR || old == block::OAK_DOOR,
        _ => true,
    }
}

/// Is the server's `old` already at or past `sent` on the same crop's ladder
/// (`growth::next_stage`, from `sent` on)? For bone meal on grass: tall grass
/// already there.
fn at_or_past(kind: UseKind, old: BlockId, sent: BlockId) -> bool {
    match kind {
        UseKind::GrowGrass => old == block::TALL_GRASS && sent == block::TALL_GRASS,
        UseKind::GrowCrop => {
            let mut stage = Some(sent);
            while let Some(s) = stage {
                if s == old {
                    return true;
                }
                stage = crate::growth::next_stage(s);
            }
            false
        }
        _ => false,
    }
}

/// Judge a joiner's use of `kind`, which claims the edit `bc` (`before` the
/// edit) having used `used` (the tag's, decoded). A legal-outcome check: any
/// member of the rule's outcome set is accepted (the server does not re-roll
/// the client's bone-meal stage). `world` is the server's, before the edit:
/// the cells around it (the soil under a seed, the water by a reed, a door's
/// bottom half, a tapped log's cooldown) are read there. C3c-1-fix (L-1) —
/// what isn't legal is drift, impossible or unexplained ([`Verdict`]).
pub fn judge(kind: UseKind, used: Option<&Item>, bc: &BlockChange, before: Before, world: &World) -> Judged {
    let (old, new) = (before.old, bc.new_block);
    let below = || world.get_block(bc.x, bc.y - 1, bc.z);
    let material = |m: MaterialId| used == Some(&Item::Material(m));
    let legal = match kind {
        UseKind::BucketFill => {
            used.and_then(|u| crate::bucket::fill_result(u, old, before.source)).is_some() && new == block::AIR
        }
        UseKind::BucketEmpty => used.and_then(|u| crate::bucket::empty_result(u, old)).is_some_and(|(fluid, _)| fluid == new),
        UseKind::Sow => {
            let crop = match used {
                Some(Item::Material(m)) => sown_crop(*m),
                _ => None,
            };
            crop == Some(new) && old == block::AIR && below() == block::TILLED_SOIL
        }
        UseKind::PlantPapyrus => {
            material(MaterialId::PapyrusReed)
                && new == block::PAPYRUS_STAGE_0
                && old == block::AIR
                && crate::papyrus::is_valid_planting_base(world, bc.x, bc.y - 1, bc.z)
        }
        UseKind::GrowGrass => material(MaterialId::Bonemeal) && new == block::TALL_GRASS && grows_tall_grass(below(), old),
        UseKind::GrowCrop => used.is_some_and(|u| accelerations(u, old).contains(&Some(new))),
        UseKind::Salt => {
            material(MaterialId::Salt) && crate::snowfall::target_block_accepts_salt_path(old) && new == block::SALT_PATH
        }
        UseKind::Erase => erases(old) && new == block::AIR,
        UseKind::TapRubber => {
            crate::rubber::is_tappable(old)
                && new == block::RUBBER_LOG_TAPPED
                && !world.tapped_rubber_logs.contains_key(&(bc.x, bc.y, bc.z))
        }
        UseKind::Till => tills(old) && new == block::TILLED_SOIL,
        UseKind::DoorUpper => {
            new == block::OAK_DOOR
                && old == block::AIR
                && crate::block_shape::door_is_top(bc.meta)
                && below_holds_a_door_bottom(world, bc)
        }
        UseKind::HangPrint => {
            matches!(used, Some(Item::Plan(p)) if p.develop_state == crate::plan::DevelopState::Developed)
                && new == block::CYANOTYPE_PRINT
                && matches!(old, block::AIR | block::WATER)
                && hangs_on_wall(world, bc.x, bc.y, bc.z)
        }
    };
    let verdict = if legal {
        Verdict::Legal
    } else if !explains(kind, new, bc.meta) {
        Verdict::Unexplained
    } else if possible(kind, used, old, new, bc.meta) {
        Verdict::Drift
    } else {
        Verdict::Impossible
    };
    let product = match verdict {
        Verdict::Legal | Verdict::Drift => product(kind, old),
        Verdict::Impossible | Verdict::Unexplained => None,
    };
    let keeps_server_cell = matches!(verdict, Verdict::Drift | Verdict::Impossible) && at_or_past(kind, old, new);
    Judged { verdict, product, keeps_server_cell }
}

/// Why a use didn't mirror cleanly (log-only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseMiss {
    /// The outcome isn't one the rule produces for what was used.
    Outcome,
    /// C3c-1-fix (L-1) — an outcome the rule makes, but not from the
    /// server's world as it stands (honest drift): mirrored all the same.
    Drift,
    /// C3c-1-fix (L-1) — the kind can't explain the edit: classified as an
    /// ordinary edit instead.
    Unexplained,
    /// The copy held none of what the use consumed (or, for a tap, no
    /// bucket in the slot).
    NothingToTake,
    /// The copy's slot held no such tool to wear.
    NoTool,
}

impl UseMiss {
    pub const fn label(self) -> &'static str {
        match self {
            UseMiss::Outcome => "an outcome its rule can't produce",
            UseMiss::Drift => "an outcome the server's world had moved on from (drift; mirrored)",
            UseMiss::Unexplained => "an edit its kind can't explain (checked as an ordinary edit)",
            UseMiss::NothingToTake => "an item the server's copy didn't hold",
            UseMiss::NoTool => "a tool the server's copy didn't hold in that slot",
        }
    }
}

/// M-2 — the part of a use's product the client said didn't fit its bag
/// (`UseTag::unfit`), for the caller to spawn as a real ground item at the
/// joiner.
#[derive(Clone, Debug, PartialEq)]
pub struct Unfit {
    pub stack: ItemStack,
    /// The copy agrees: it paid the use's cost and, having added the rest of
    /// the product, had no room for this either. Spawned free. Otherwise the
    /// spawn is believed: charged to the joiner's believed bound
    /// (`window_ops::BelievedBucket`) and not spawned past it.
    pub corroborated: bool,
}

/// What [`settle`] did to the copy.
#[derive(Clone, Debug, PartialEq)]
pub struct Settled {
    /// The first thing that didn't match, if any (`None` = mirrored).
    pub miss: Option<UseMiss>,
    /// M-2 — units of `product − unfit` the copy had no room for: tallied
    /// only (`use_copy_overflow`), never spawned — the client holds them.
    pub copy_overflow: u8,
    /// M-2 — what the client said didn't fit, if anything (and the use makes
    /// a product).
    pub unfit: Option<Unfit>,
}

/// Mirror a joiner's use of `kind` on `inv`, the server's copy of its
/// inventory, by the client's steps in the client's order: take one of
/// `used` (from the tag's `slot` first: `joiner_actions::take_owed`), wear
/// `tool` in `slot`, then add the product (`judged.product`, for a legal or
/// drifted outcome whose cost or requirement the copy met) with `add_item`.
/// A tap requires a Bucket in `slot` (it keeps it).
///
/// C3c-1-fix (M-2) — the use's overflow is what the CLIENT says didn't fit
/// (`tag.unfit`): the copy adds `product − unfit` (what doesn't fit the copy
/// is only counted, `copy_overflow`), and the `unfit` part is handed back
/// ([`Settled::unfit`]) for the caller to spawn. Never called for an
/// [`Verdict::Unexplained`] use.
pub fn settle(inv: &mut Inventory, kind: UseKind, tag: &UseTag, used: Option<&Item>, judged: &Judged) -> Settled {
    let slot = usize::from(tag.slot);
    let mut miss = match judged.verdict {
        Verdict::Legal => None,
        Verdict::Drift => Some(UseMiss::Drift),
        Verdict::Impossible => Some(UseMiss::Outcome),
        Verdict::Unexplained => Some(UseMiss::Unexplained),
    };
    let mut paid = true;
    if kind.consumes() {
        paid = used.is_some_and(|item| crate::joiner_actions::take_owed(inv, slot, item, 1) == 1);
    } else if kind == UseKind::TapRubber {
        paid = matches!(inv.slot(slot).map(|s| &s.item), Some(Item::Material(MaterialId::Bucket)));
    }
    if !paid {
        miss = miss.or(Some(UseMiss::NothingToTake));
    }
    if kind.wears() {
        let tool = match crate::inventory::item_from_wire_full(&tag.tool) {
            Some(Item::Tool(t)) if wears_with(kind, &t) => Some(t),
            _ => None,
        };
        let worn = tool.is_some_and(|t| crate::joiner_inventory::wear_tool(inv, slot, &t) != WearCheck::Mismatched);
        if !worn {
            paid = false;
            miss = miss.or(Some(UseMiss::NoTool));
        }
    }
    let product = judged.product.clone().filter(|_| matches!(judged.verdict, Verdict::Legal | Verdict::Drift));
    let Some(product) = product else {
        return Settled { miss, copy_overflow: 0, unfit: None };
    };
    let unfit_n = tag.unfit.min(product.count);
    let landed = product.count - unfit_n;
    let mut copy_overflow = 0;
    if paid && landed > 0 {
        copy_overflow = inv.add_item(ItemStack { item: product.item.clone(), count: landed }).map_or(0, |s| s.count);
    }
    let unfit = (unfit_n > 0).then(|| {
        let stack = ItemStack { item: product.item.clone(), count: unfit_n };
        // Would the copy, having paid, have taken it? Then it can't say the
        // client's bag was full.
        let corroborated = paid && inv.clone().add_item(stack.clone()).is_some();
        Unfit { stack, corroborated }
    });
    Settled { miss, copy_overflow, unfit }
}

/// The tool a wearing use takes: an Eraser for an erase, a hoe for tilling.
fn wears_with(kind: UseKind, tool: &Tool) -> bool {
    match kind {
        UseKind::Erase => tool.tool_type == ToolType::Eraser,
        UseKind::Till => tool.tool_type == ToolType::Hoe,
        _ => false,
    }
}

// ─── The undo (client) ───────────────────────────────────────────────────

/// C3c-1-fix (M-4) — how many inputs past the one a use's edit could first
/// ride (`RemoteClient::next_input_seq` when it was made) a joined client
/// keeps its record of the use. The server acknowledges an input when it has
/// simulated its movement, but the input's edits can wait in its edit queue
/// past the per-tick edit budget (and a refusal can wait behind chunk pushes
/// in the client's outbox), so a refusal can arrive after the acknowledgement
/// of the input that carried the use: the record is held this long past it
/// (5 s) so the undo always has it.
pub const USE_RECORD_HOLD_INPUTS: u64 = 100;

// C3c-1-fix verify F-L6 — the hold must outlast the worst wait a notice can
// have in the client's outbox: a full block-delta queue and a full chunk
// window drained at the per-tick budget (about 53 ticks today), with half
// again for the budget's overheads (about 39 KiB of block deltas a tick in
// practice: about 66 ticks). A later bound change that would let a record
// expire before its notice fails to build here.
const _: () = assert!(
    ((crate::state_outbox::CLIENT_QUEUE_MAX_BYTES + crate::chunk_push::CHUNK_WINDOW_BYTES)
        / crate::state_outbox::CLIENT_TICK_BUDGET_BYTES) as u64
        * 3
        / 2
        < USE_RECORD_HOLD_INPUTS
);

/// Most use records a joined client keeps (the oldest goes first). An honest
/// client makes a use at most every 8 ticks: about 13 within the hold.
pub const MAX_USE_RECORDS: usize = 64;

/// C3c-1-fix (M-4) — a joined client's own record of a use it made: what the
/// use spent and what of its product landed in the bag, read where the use
/// was made. A refusal is undone from this, never from the notice.
#[derive(Clone, Debug, PartialEq)]
pub struct UseRecord {
    pub cell: [i32; 3],
    /// The use's kind, as its tag carried it.
    pub kind: u8,
    /// The hotbar slot it was made from.
    pub slot: usize,
    /// What it consumed (one), if anything.
    pub cost: Option<Item>,
    /// What of its product the bag took (`product − unfit`), if anything.
    pub landed: Option<ItemStack>,
    /// `RemoteClient::next_input_seq` when it was made.
    pub made_at: u64,
}

/// C3c-1-fix (M-4) — a joined client's records of its recent uses
/// ([`UseRecord`]), for undoing the ones the server refuses.
#[derive(Clone, Debug, Default)]
pub struct SentUses {
    records: std::collections::VecDeque<UseRecord>,
}

impl SentUses {
    /// Keep `record` (dropping the oldest past [`MAX_USE_RECORDS`]).
    pub fn record(&mut self, record: UseRecord) {
        self.records.push_back(record);
        while self.records.len() > MAX_USE_RECORDS {
            self.records.pop_front();
        }
    }

    /// The server has applied our inputs up to `acked`: drop the records held
    /// [`USE_RECORD_HOLD_INPUTS`] past that. Call AFTER applying the
    /// refusals that came with it.
    pub fn acknowledged(&mut self, acked: u64) {
        self.records.retain(|r| acked < r.made_at.saturating_add(USE_RECORD_HOLD_INPUTS));
    }

    /// The record a refusal at `cell` of `kind` undoes: the NEWEST of that
    /// cell and kind, taken out. An older one there was more likely accepted
    /// (it is only held for a while); two refused in flight are both undone,
    /// whichever order.
    pub fn take(&mut self, cell: [i32; 3], kind: u8) -> Option<UseRecord> {
        let at = self.records.iter().rposition(|r| r.cell == cell && r.kind == kind)?;
        self.records.remove(at)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Forget everything (the session ended).
    pub fn clear(&mut self) {
        self.records.clear();
    }
}

/// What undoing one refused use did ([`undo`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Undone {
    /// Units of the landed product the window no longer held (spent, moved
    /// away, dropped): a shortfall, logged. C3c-2-fix (F-M1) — with any, the
    /// cost is not given back.
    pub short: u8,
    /// Units of the cost that didn't fit back: lost on the client (the
    /// server's copy, which never applied the use, still holds them).
    pub lost: u8,
}

/// C3c-1-fix (M-4) — undo the refused use `record` on the client's window:
/// take back the product that landed (the shared owed-take search, exact
/// first, from the use's slot first: `joiner_actions::take_owed_held`, which
/// also searches the grid and cursor), then give back the cost: into its own
/// slot when that is empty or holds a stack it joins, else wherever
/// `add_item` puts it. Tool wear is not undone.
///
/// C3c-2-fix (F-M1) — the cost comes back only when the product came back
/// in full (`short == 0`). A product already gone (emptied, dropped,
/// deposited) was spent by a later act the server may have accepted — an
/// empty of the filled bucket gives its bucket back on its own — so giving
/// the cost back too would mint one (a bucket a cycle, on a slow link). The
/// shortfall is logged and nothing is given: the client may end BELOW the
/// server's copy (a believed deposit or a Q-drop already made the product
/// real), never above it.
pub fn undo(inv: &mut Inventory, ui: &mut crate::craft_ui::CraftingUi, record: &UseRecord) -> Undone {
    let mut undone = Undone::default();
    if let Some(landed) = &record.landed {
        let taken = crate::joiner_actions::take_owed_held(inv, ui, record.slot, &landed.item, landed.count);
        undone.short = landed.count.saturating_sub(taken);
    }
    if let Some(cost) = &record.cost
        && undone.short == 0
    {
        let one = ItemStack { item: cost.clone(), count: 1 };
        undone.lost = give_back(inv, record.slot, one);
    }
    undone
}

/// C3c-2-fix (F-M1) — undo a frame's refusal notices `refused` (in the
/// server's order: oldest first) from the client's own records `uses`,
/// NEWEST first: chained uses whose notices land together (a fill, then an
/// empty of the bucket it filled, both refused) are undone exactly in that
/// order, the empty's product taken back and its cost returned before the
/// fill's product is looked for. Each notice with what undoing it did, in
/// the order undone (`None`: no record of it — nothing to undo).
pub fn undo_refused(
    uses: &mut SentUses,
    inv: &mut Inventory,
    ui: &mut crate::craft_ui::CraftingUi,
    refused: &[crate::protocol::RefusedUse],
) -> Vec<(crate::protocol::RefusedUse, Option<Undone>)> {
    refused
        .iter()
        .rev()
        .map(|r| (*r, uses.take([r.x, r.y, r.z], r.kind).map(|rec| undo(inv, ui, &rec))))
        .collect()
}

/// C3c-2-fix (F-L2) — does the cell below `bc` hold a door's BOTTOM half?
/// A door's top half stands on a bottom half only, never on another top half
/// (a column of free door tops).
pub fn below_holds_a_door_bottom(world: &World, bc: &BlockChange) -> bool {
    world.get_block(bc.x, bc.y - 1, bc.z) == block::OAK_DOOR
        && !crate::block_shape::door_is_top(world.meta_at(bc.x, bc.y - 1, bc.z))
}

/// Put `stack` back in `slot` if it is empty or holds a stack it joins with
/// room, else `add_item`. Returns how many didn't fit.
fn give_back(inv: &mut Inventory, slot: usize, stack: ItemStack) -> u8 {
    match inv.slot(slot) {
        None if slot < 36 => {
            inv.set_slot(slot, Some(stack));
            return 0;
        }
        Some(s) if s.item.can_stack_with(&stack.item) && s.count.saturating_add(stack.count) <= s.item.max_stack() => {
            let joined = ItemStack { item: s.item.clone(), count: s.count + stack.count };
            inv.set_slot(slot, Some(joined));
            return 0;
        }
        _ => {}
    }
    inv.add_item(stack).map_or(0, |s| s.count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crafting::ToolMaterial;

    fn bc(x: i32, y: i32, z: i32, new_block: BlockId) -> BlockChange {
        BlockChange { x, y, z, new_block, meta: 0 }
    }

    fn mat(m: MaterialId) -> Item {
        Item::Material(m)
    }

    fn before(old: BlockId) -> Before {
        Before { old, source: true }
    }

    /// The wire bytes are APPEND-ONLY: a renumbering would make a peer read
    /// one use as another.
    #[test]
    fn use_kind_wire_bytes_are_pinned() {
        let bytes: Vec<u8> = UseKind::ALL.iter().map(|k| k.to_wire()).collect();
        // C3c-3a (v83) appends HangPrint = 11.
        assert_eq!(bytes, (0..12).collect::<Vec<u8>>());
        for k in UseKind::ALL {
            assert_eq!(UseKind::from_wire(k.to_wire()), Some(k));
        }
        assert_eq!(UseKind::from_wire(12), None);
        assert_eq!(UseKind::from_wire(u8::MAX), None);
    }

    /// The tag carries the hand before the use: one of what it consumes,
    /// the state of what it wears, nothing else.
    #[test]
    fn a_tag_carries_what_the_use_spends_and_wears() {
        let meal = mat(MaterialId::Bonemeal);
        let t = tag(UseKind::GrowCrop, [1, 2, 3], 4, Some(&meal));
        assert_eq!((t.x, t.y, t.z, t.kind, t.slot), (1, 2, 3, 5, 4));
        let used = t.used.expect("one bone meal");
        assert_eq!(used.count, 1);
        assert_eq!(t.tool, WireItem::None);
        let mut hoe = Tool::new(ToolType::Hoe, ToolMaterial::Stone);
        hoe.durability -= 3;
        let t = tag(UseKind::Till, [0, 0, 0], 2, Some(&Item::Tool(hoe)));
        assert_eq!(t.used, None, "a hoe consumes nothing");
        assert_eq!(t.tool, crate::inventory::item_to_wire_full(&Item::Tool(hoe)));
        let t = tag(UseKind::TapRubber, [0, 0, 0], 0, Some(&mat(MaterialId::Bucket)));
        assert_eq!((t.used, t.tool), (None, WireItem::None), "a tap keeps its bucket");
    }

    #[test]
    fn every_seed_sows_its_own_stage_zero() {
        assert_eq!(sown_crop(MaterialId::WheatSeeds), Some(block::WHEAT_STAGE_0));
        assert_eq!(sown_crop(MaterialId::ButtercupSeeds), Some(block::BUTTERCUP_STAGE_0));
        assert_eq!(sown_crop(MaterialId::Bonemeal), None);
    }

    /// Bone meal's outcome set is one or two stages; three is no outcome of
    /// it. Fertiliser's is one.
    #[test]
    fn a_crop_accelerator_is_judged_by_its_outcome_set() {
        let w = World::new();
        let meal = mat(MaterialId::Bonemeal);
        let judge_to = |used: &Item, new| judge(UseKind::GrowCrop, Some(used), &bc(0, 70, 0, new), before(block::WHEAT_STAGE_0), &w).legal();
        assert!(judge_to(&meal, block::WHEAT_STAGE_1));
        assert!(judge_to(&meal, block::WHEAT_STAGE_2));
        assert!(!judge_to(&meal, block::WHEAT_STAGE_3), "three stages is no bone meal outcome");
        let fert = mat(MaterialId::Fertiliser);
        assert!(judge_to(&fert, block::WHEAT_STAGE_1));
        assert!(!judge_to(&fert, block::WHEAT_STAGE_2));
    }

    /// A bucket fill is legal on a source only, with an empty bucket, and
    /// makes the bucket of the fluid the server's cell held.
    #[test]
    fn a_bucket_fill_is_judged_on_the_servers_source() {
        let w = World::new();
        let bucket = mat(MaterialId::Bucket);
        let fill = bc(0, 70, 0, block::AIR);
        let j = judge(UseKind::BucketFill, Some(&bucket), &fill, before(block::LAVA), &w);
        assert!(j.legal());
        assert_eq!(j.product, Some(ItemStack::new_material(MaterialId::LavaBucket, 1)));
        let flowing = Before { old: block::WATER, source: false };
        assert!(!judge(UseKind::BucketFill, Some(&bucket), &fill, flowing, &w).legal());
        assert!(!judge(UseKind::BucketFill, Some(&mat(MaterialId::WaterBucket)), &fill, before(block::WATER), &w).legal());
        // Water placed with an empty bucket: no outcome of emptying.
        let pour = bc(0, 70, 0, block::WATER);
        assert!(!judge(UseKind::BucketEmpty, Some(&bucket), &pour, before(block::AIR), &w).legal());
        assert!(judge(UseKind::BucketEmpty, Some(&mat(MaterialId::WaterBucket)), &pour, before(block::AIR), &w).legal());
    }

    /// Lockstep: the copy takes what the client's `consume_one_material`
    /// took, from the same slot, and lands the product where the client's
    /// `add_item` did.
    #[test]
    fn settle_takes_from_the_tags_slot_and_adds_where_the_client_does() {
        let mut client = Inventory::new();
        client.set_slot(0, Some(ItemStack::new_material(MaterialId::Bucket, 1)));
        client.set_slot(1, Some(ItemStack::new_material(MaterialId::Bucket, 3)));
        let mut copy = client.clone();
        let held = client.hotbar_slot(1).map(|s| s.item.clone());
        let t = tag(UseKind::BucketFill, [0, 70, 0], 1, held.as_ref());
        // The client's fill (`fill_bucket_at`).
        assert!(client.consume_one_material(1, MaterialId::Bucket));
        assert!(client.add_item(ItemStack::new_material(MaterialId::WaterBucket, 1)).is_none());
        let used = crate::inventory::stack_from_wire(t.used.as_ref().unwrap(), &crate::block::BlockRegistry::new(), crate::inventory::PlanDecode::Marker)
            .map(|s| s.item);
        let w = World::new();
        let j = judge(UseKind::BucketFill, used.as_ref(), &bc(0, 70, 0, block::AIR), before(block::WATER), &w);
        let s = settle(&mut copy, UseKind::BucketFill, &t, used.as_ref(), &j);
        assert_eq!((s.miss, s.copy_overflow, s.unfit), (None, 0, None));
        for k in 0..36 {
            assert_eq!(copy.slot(k), client.slot(k), "slot {k}");
        }
    }

    /// An illegal outcome still spends what the client says it spent, but
    /// makes nothing; a cost the copy can't pay makes nothing either.
    #[test]
    fn the_mirror_never_makes_an_item_from_nothing() {
        let w = World::new();
        let bucket = mat(MaterialId::Bucket);
        let t = tag(UseKind::BucketFill, [0, 70, 0], 0, Some(&bucket));
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Bucket, 1)));
        let stone = judge(UseKind::BucketFill, Some(&bucket), &bc(0, 70, 0, block::AIR), before(block::STONE), &w);
        // C3c-1-fix (L-1) — a fill whose server cell holds no fluid is drift
        // (the client saw water there), but which bucket it filled isn't on
        // the tag: the cost is mirrored, no product.
        assert_eq!(stone.verdict, Verdict::Drift);
        let s = settle(&mut inv, UseKind::BucketFill, &t, Some(&bucket), &stone);
        assert_eq!(s.miss, Some(UseMiss::Drift));
        assert!(inv.slots_iter().all(|s| s.is_none()), "the bucket is spent, nothing made");
        let water = judge(UseKind::BucketFill, Some(&bucket), &bc(0, 70, 0, block::AIR), before(block::WATER), &w);
        let s = settle(&mut inv, UseKind::BucketFill, &t, Some(&bucket), &water);
        assert_eq!(s.miss, Some(UseMiss::NothingToTake));
        assert!(inv.slots_iter().all(|s| s.is_none()), "no bucket to fill: no water bucket");
    }

    /// A wearing use wears the copy's tool in its slot and gives its product
    /// (an Eraser's sheet) only when that tool was there.
    #[test]
    fn an_eraser_wears_and_gives_back_a_sheet() {
        let w = World::new();
        let eraser = Item::Tool(Tool::new(ToolType::Eraser, ToolMaterial::Wood));
        let mut inv = Inventory::new();
        inv.set_slot(3, Some(ItemStack { item: eraser.clone(), count: 1 }));
        let t = tag(UseKind::Erase, [0, 70, 0], 3, Some(&eraser));
        let j = judge(UseKind::Erase, None, &bc(0, 70, 0, block::AIR), before(block::BLUEPRINT_PAPER), &w);
        assert!(j.legal());
        let s = settle(&mut inv, UseKind::Erase, &t, None, &j);
        assert_eq!(s.miss, None);
        let worn = match inv.slot(3).map(|s| &s.item) {
            Some(Item::Tool(t)) => t.durability,
            other => panic!("eraser gone: {other:?}"),
        };
        assert_eq!(worn + 1, Tool::new(ToolType::Eraser, ToolMaterial::Wood).durability);
        assert_eq!(inv.slot(0), Some(&ItemStack::new_material(MaterialId::PapyrusSheet, 1)));
        // From the wrong slot: no wear, no sheet.
        let s = settle(&mut inv, UseKind::Erase, &tag(UseKind::Erase, [0, 70, 0], 5, Some(&eraser)), None, &j);
        assert_eq!(s.miss, Some(UseMiss::NoTool));
        assert_eq!(inv.slot(0).map(|s| s.count), Some(1));
    }

    /// A tap needs a live log the server hasn't got on cooldown, and a
    /// bucket in the slot (kept).
    #[test]
    fn a_tap_is_judged_on_the_servers_cooldown() {
        let mut w = World::new();
        let tap = bc(4, 70, 4, block::RUBBER_LOG_TAPPED);
        assert!(judge(UseKind::TapRubber, None, &tap, before(block::RUBBER_LOG), &w).legal());
        w.tapped_rubber_logs.insert((4, 70, 4), 10);
        assert!(!judge(UseKind::TapRubber, None, &tap, before(block::RUBBER_LOG), &w).legal());
        assert!(!judge(UseKind::TapRubber, None, &tap, before(block::RUBBER_LOG_TAPPED), &World::new()).legal());
        let mut inv = Inventory::new();
        let j = judge(UseKind::TapRubber, None, &tap, before(block::RUBBER_LOG), &World::new());
        let t = tag(UseKind::TapRubber, [4, 70, 4], 0, Some(&mat(MaterialId::Bucket)));
        assert_eq!(settle(&mut inv, UseKind::TapRubber, &t, None, &j).miss, Some(UseMiss::NothingToTake));
        assert!(inv.slots_iter().all(|s| s.is_none()));
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Bucket, 1)));
        assert_eq!(settle(&mut inv, UseKind::TapRubber, &t, None, &j).miss, None);
        assert_eq!(inv.slot(0), Some(&ItemStack::new_material(MaterialId::Bucket, 1)), "the bucket is kept");
        assert_eq!(inv.slot(1), Some(&ItemStack::new_material(MaterialId::Rubber, 1)));
    }

    /// A door's top half stands on its bottom half, with the top bit set.
    #[test]
    fn a_door_top_half_needs_its_bottom_half() {
        let mut w = World::new();
        let top = BlockChange { x: 2, y: 71, z: 2, new_block: block::OAK_DOOR, meta: door_top_meta(0) };
        assert!(!judge(UseKind::DoorUpper, None, &top, before(block::AIR), &w).legal());
        w.set_block(2, 70, 2, block::OAK_DOOR);
        assert!(judge(UseKind::DoorUpper, None, &top, before(block::AIR), &w).legal());
        let bottom_meta = BlockChange { meta: 0, ..top.clone() };
        assert!(!judge(UseKind::DoorUpper, None, &bottom_meta, before(block::AIR), &w).legal());
        assert!(crate::block_shape::door_is_top(door_top_meta(crate::meta::with_facing(0, crate::meta::Facing::East))));
        // C3c-2-fix (F-L2) — over a door's TOP half it is not legal: a top
        // half stands on a bottom half only (no free door tops up a column).
        w.set_meta((2, 70, 2), door_top_meta(0));
        assert!(!judge(UseKind::DoorUpper, None, &top, before(block::AIR), &w).legal());
        assert!(!below_holds_a_door_bottom(&w, &top));
        w.set_meta((2, 70, 2), 0);
        assert!(below_holds_a_door_bottom(&w, &top));
    }

    // ─── C3c-1-fix ─────────────────────────────────────────────────────────

    /// L-1 — what isn't legal on the server's world is drift (the rule makes
    /// it for what was used), impossible (it never does) or unexplained (the
    /// kind never makes that block at all).
    #[test]
    fn an_illegal_use_is_drift_impossible_or_unexplained() {
        let w = World::new();
        let bucket = mat(MaterialId::Bucket);
        let fill = bc(0, 70, 0, block::AIR);
        // A source that flowed on the server: drift, and the product is the
        // bucket of the fluid still there.
        let flowed = judge(UseKind::BucketFill, Some(&bucket), &fill, Before { old: block::WATER, source: false }, &w);
        assert_eq!(flowed.verdict, Verdict::Drift);
        assert_eq!(flowed.product, Some(ItemStack::new_material(MaterialId::WaterBucket, 1)));
        // Water poured from an EMPTY bucket: the rule never makes it.
        let pour = judge(UseKind::BucketEmpty, Some(&bucket), &bc(0, 70, 0, block::WATER), before(block::AIR), &w);
        assert_eq!((pour.verdict, pour.product), (Verdict::Impossible, None));
        // An emptied bucket into a cell another player filled: drift, the
        // empty bucket comes back.
        let into_stone = judge(UseKind::BucketEmpty, Some(&mat(MaterialId::WaterBucket)), &bc(0, 70, 0, block::WATER), before(block::STONE), &w);
        assert_eq!(into_stone.verdict, Verdict::Drift);
        assert_eq!(into_stone.product, Some(ItemStack::new_material(MaterialId::Bucket, 1)));
        // A hoe tag on a stone placement, a door-top tag on a plank: no
        // output of the kind.
        assert_eq!(judge(UseKind::Till, None, &bc(0, 70, 0, block::STONE), before(block::DIRT), &w).verdict, Verdict::Unexplained);
        let plank = BlockChange { x: 0, y: 70, z: 0, new_block: block::OAK_PLANKS, meta: door_top_meta(0) };
        assert_eq!(judge(UseKind::DoorUpper, None, &plank, before(block::AIR), &w).verdict, Verdict::Unexplained);
        // Salt "sowing" a seed's crop: explained (a crop is sown) but never
        // made with salt.
        let salt = mat(MaterialId::Salt);
        assert_eq!(judge(UseKind::Sow, Some(&salt), &bc(0, 70, 0, block::WHEAT_STAGE_0), before(block::AIR), &w).verdict, Verdict::Impossible);
        // A tap on a log the server holds on cooldown: drift, rubber comes.
        let mut cooling = World::new();
        cooling.tapped_rubber_logs.insert((0, 70, 0), 5);
        let tap = judge(UseKind::TapRubber, None, &bc(0, 70, 0, block::RUBBER_LOG_TAPPED), before(block::RUBBER_LOG), &cooling);
        assert_eq!(tap.verdict, Verdict::Drift);
        assert_eq!(tap.product, Some(ItemStack::new_material(MaterialId::Rubber, 1)));
    }

    /// C3c-2-fix (F-L3) — a no-cost use (an Eraser, a tap, a hoe, a door's
    /// top half) is drift only when the server's `old` is the rule's input or
    /// its output (another player got there first); on anything else it is
    /// impossible: no product, whatever the tag says.
    #[test]
    fn a_no_cost_use_on_an_unrelated_block_is_impossible() {
        let w = World::new();
        let verdict = |kind, new, meta, old| judge(kind, None, &BlockChange { x: 0, y: 70, z: 0, new_block: new, meta }, before(old), &w);
        // The Eraser: input blueprint paper, output AIR.
        let on_stone = verdict(UseKind::Erase, block::AIR, 0, block::STONE);
        assert_eq!((on_stone.verdict, on_stone.product), (Verdict::Impossible, None), "an Eraser on stone yields no sheet");
        let erased_first = verdict(UseKind::Erase, block::AIR, 0, block::AIR);
        assert_eq!(erased_first.verdict, Verdict::Drift, "another player erased it first");
        // The tap: input a tappable log, output a tapped one.
        let on_air = verdict(UseKind::TapRubber, block::RUBBER_LOG_TAPPED, 0, block::AIR);
        assert_eq!((on_air.verdict, on_air.product), (Verdict::Impossible, None), "a tap on AIR yields no rubber");
        let on_oak = verdict(UseKind::TapRubber, block::RUBBER_LOG_TAPPED, 0, block::OAK_LOG);
        assert_eq!(on_oak.verdict, Verdict::Impossible);
        let tapped_first = verdict(UseKind::TapRubber, block::RUBBER_LOG_TAPPED, 0, block::RUBBER_LOG_TAPPED);
        assert_eq!(tapped_first.verdict, Verdict::Drift, "another player tapped it first");
        assert_eq!(tapped_first.product, Some(ItemStack::new_material(MaterialId::Rubber, 1)));
        // The hoe: input dirt or grass, output tilled soil.
        assert_eq!(verdict(UseKind::Till, block::TILLED_SOIL, 0, block::STONE).verdict, Verdict::Impossible);
        assert_eq!(verdict(UseKind::Till, block::TILLED_SOIL, 0, block::TILLED_SOIL).verdict, Verdict::Drift);
        // A door's top half: input AIR, output a door.
        let top = door_top_meta(0);
        assert_eq!(verdict(UseKind::DoorUpper, block::OAK_DOOR, top, block::STONE).verdict, Verdict::Impossible);
        assert_eq!(verdict(UseKind::DoorUpper, block::OAK_DOOR, top, block::OAK_DOOR).verdict, Verdict::Drift);
    }

    /// L-1 — a crop accelerator whose server crop already stands at or past
    /// the stage sent keeps the server's cell (it isn't set back); a crop
    /// behind the stage sent is drift, applied.
    #[test]
    fn a_crop_the_server_grew_first_is_not_set_back() {
        let w = World::new();
        let meal = mat(MaterialId::Bonemeal);
        let to_2 = bc(0, 70, 0, block::WHEAT_STAGE_2);
        for (server, keeps) in [
            (block::WHEAT_STAGE_2, true),
            (block::WHEAT_STAGE_3, true),
            (block::CARROT_STAGE_3, false),
            (block::AIR, false),
        ] {
            let j = judge(UseKind::GrowCrop, Some(&meal), &to_2, before(server), &w);
            assert_eq!(j.keeps_server_cell, keeps, "server {server}");
        }
        let behind = judge(UseKind::GrowCrop, Some(&meal), &bc(0, 70, 0, block::WHEAT_STAGE_3), before(block::WHEAT_STAGE_0), &w);
        assert_eq!((behind.verdict, behind.keeps_server_cell), (Verdict::Drift, false));
        // Bone meal on grass where the server already has tall grass.
        let grass = judge(UseKind::GrowGrass, Some(&meal), &bc(0, 71, 0, block::TALL_GRASS), before(block::TALL_GRASS), &w);
        assert!(grass.keeps_server_cell);
        assert!(!judge(UseKind::GrowGrass, Some(&meal), &bc(0, 71, 0, block::TALL_GRASS), before(block::AIR), &w).keeps_server_cell);
    }

    /// L-1 — log-only tracks the client: a drifted use mirrors its cost AND
    /// its product on the copy.
    #[test]
    fn a_drifted_use_mirrors_cost_and_product() {
        let w = World::new();
        let bucket = mat(MaterialId::Bucket);
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Bucket, 2)));
        let t = tag(UseKind::BucketFill, [0, 70, 0], 0, Some(&bucket));
        let j = judge(UseKind::BucketFill, Some(&bucket), &bc(0, 70, 0, block::AIR), Before { old: block::LAVA, source: false }, &w);
        let s = settle(&mut inv, UseKind::BucketFill, &t, Some(&bucket), &j);
        assert_eq!(s.miss, Some(UseMiss::Drift));
        assert_eq!(inv.slot(0).map(|s| s.count), Some(1));
        assert_eq!(inv.slot(1), Some(&ItemStack::new_material(MaterialId::LavaBucket, 1)));
    }

    fn full_of_stone(inv: &mut Inventory, from: usize) {
        for k in from..36 {
            inv.set_slot(k, Some(ItemStack::new_block(block::STONE, 64)));
        }
    }

    /// M-2 — the copy adds `product − unfit`; the `unfit` part is handed
    /// back, corroborated only when the copy paid and had no room either.
    #[test]
    fn a_uses_unfit_part_is_the_clients_and_corroborated_by_a_full_paying_copy() {
        let w = World::new();
        let bucket = mat(MaterialId::Bucket);
        let fill = bc(0, 70, 0, block::AIR);
        let j = judge(UseKind::BucketFill, Some(&bucket), &fill, before(block::WATER), &w);
        let mut t = tag(UseKind::BucketFill, [0, 70, 0], 0, Some(&bucket));
        t.unfit = 1;
        let water = ItemStack::new_material(MaterialId::WaterBucket, 1);
        // A full copy that holds the buckets: corroborated, nothing added.
        let mut full = Inventory::new();
        full.set_slot(0, Some(ItemStack::new_material(MaterialId::Bucket, 2)));
        full_of_stone(&mut full, 1);
        let s = settle(&mut full, UseKind::BucketFill, &t, Some(&bucket), &j);
        assert_eq!(s.unfit, Some(Unfit { stack: water.clone(), corroborated: true }));
        assert_eq!((s.miss, s.copy_overflow), (None, 0));
        assert_eq!(full.slot(0).map(|s| s.count), Some(1));
        // An EMPTY copy (empty at attach): it can't pay, so it can't say the
        // bag was full: believed.
        let mut empty = Inventory::new();
        let s = settle(&mut empty, UseKind::BucketFill, &t, Some(&bucket), &j);
        assert_eq!(s.unfit, Some(Unfit { stack: water.clone(), corroborated: false }));
        assert!(empty.slots_iter().all(|s| s.is_none()), "nothing added");
        // A copy with room: believed.
        let mut roomy = Inventory::new();
        roomy.set_slot(0, Some(ItemStack::new_material(MaterialId::Bucket, 2)));
        let s = settle(&mut roomy, UseKind::BucketFill, &t, Some(&bucket), &j);
        assert_eq!(s.unfit, Some(Unfit { stack: water, corroborated: false }));
        assert!(roomy.slots_iter().flatten().all(|s| s.item == bucket), "the unfit part never lands on the copy");
        // A copy FULLER than the client (unfit 0): its overflow is counted,
        // never handed back to spawn.
        t.unfit = 0;
        let mut fuller = Inventory::new();
        fuller.set_slot(0, Some(ItemStack::new_material(MaterialId::Bucket, 2)));
        full_of_stone(&mut fuller, 1);
        let s = settle(&mut fuller, UseKind::BucketFill, &t, Some(&bucket), &j);
        assert_eq!((s.copy_overflow, s.unfit), (1, None));
        // A use that makes nothing has no unfit part, whatever the tag says.
        let meal = mat(MaterialId::Bonemeal);
        let mut g = tag(UseKind::GrowCrop, [0, 70, 0], 0, Some(&meal));
        g.unfit = 1;
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Bonemeal, 2)));
        let gj = judge(UseKind::GrowCrop, Some(&meal), &bc(0, 70, 0, block::WHEAT_STAGE_1), before(block::WHEAT_STAGE_0), &w);
        assert_eq!(settle(&mut inv, UseKind::GrowCrop, &g, Some(&meal), &gj).unfit, None);
    }

    fn record(cell: [i32; 3], kind: UseKind, made_at: u64) -> UseRecord {
        UseRecord {
            cell,
            kind: kind.to_wire(),
            slot: 0,
            cost: Some(mat(MaterialId::Bucket)),
            landed: Some(ItemStack::new_material(MaterialId::WaterBucket, 1)),
            made_at,
        }
    }

    /// M-4 — records are matched by cell and kind (the newest), and held
    /// past the acknowledgement of the input that could first carry them.
    #[test]
    fn a_use_record_outlives_its_inputs_ack_by_the_hold() {
        let mut uses = SentUses::default();
        uses.record(record([1, 2, 3], UseKind::BucketFill, 10));
        uses.record(record([1, 2, 3], UseKind::TapRubber, 11));
        uses.record(record([1, 2, 3], UseKind::BucketFill, 12));
        assert_eq!(uses.take([9, 9, 9], UseKind::BucketFill.to_wire()), None, "another cell");
        assert_eq!(uses.take([1, 2, 3], UseKind::BucketFill.to_wire()).map(|r| r.made_at), Some(12), "the newest of that kind");
        uses.acknowledged(12);
        assert_eq!(uses.len(), 2, "acknowledged, but held");
        uses.acknowledged(10 + USE_RECORD_HOLD_INPUTS - 1);
        assert_eq!(uses.len(), 2);
        uses.acknowledged(10 + USE_RECORD_HOLD_INPUTS);
        assert_eq!(uses.len(), 1, "the oldest is past its hold");
        for k in 0..(MAX_USE_RECORDS as u64 + 5) {
            uses.record(record([0, 0, k as i32], UseKind::Salt, 20));
        }
        assert_eq!(uses.len(), MAX_USE_RECORDS);
    }

    /// M-4 — the undo takes back what landed (exact first, wherever it went)
    /// and gives the cost back to its own slot; a product already gone is a
    /// shortfall.
    #[test]
    fn a_refused_fill_is_undone_from_the_clients_record() {
        let mut ui = crate::craft_ui::CraftingUi::new();
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Bucket, 1)));
        inv.set_slot(4, Some(ItemStack::new_block(block::STONE, 3)));
        let before_use = inv.clone();
        // The client's fill: its last bucket spent, the lava bucket landed
        // in the first empty slot (0).
        assert!(inv.consume_one_material(0, MaterialId::Bucket));
        assert!(inv.add_item(ItemStack::new_material(MaterialId::LavaBucket, 1)).is_none());
        let rec = UseRecord {
            cell: [0, 70, 0],
            kind: UseKind::BucketFill.to_wire(),
            slot: 0,
            cost: Some(mat(MaterialId::Bucket)),
            landed: Some(ItemStack::new_material(MaterialId::LavaBucket, 1)),
            made_at: 1,
        };
        assert_eq!(undo(&mut inv, &mut ui, &rec), Undone::default());
        for k in 0..36 {
            assert_eq!(inv.slot(k), before_use.slot(k), "slot {k}");
        }
        // The lava bucket was emptied before the refusal came (the empty gave
        // its bucket back): a shortfall. C3c-2-fix (F-M1) — the cost does NOT
        // come back: the bucket it bought is already spent, so giving it
        // back would mint one (the client may end below the server's copy,
        // never above it).
        let mut gone = Inventory::new();
        gone.set_slot(0, Some(ItemStack::new_material(MaterialId::Bucket, 1)));
        assert_eq!(undo(&mut gone, &mut ui, &rec), Undone { short: 1, lost: 0 });
        assert_eq!(gone.slot(0), Some(&ItemStack::new_material(MaterialId::Bucket, 1)), "one bucket, not two");
        assert!(gone.slots_iter().skip(1).all(|s| s.is_none()));
        // A tap's rubber is taken back, the kept bucket untouched.
        let tap = UseRecord {
            kind: UseKind::TapRubber.to_wire(),
            cost: None,
            landed: Some(ItemStack::new_material(MaterialId::Rubber, 1)),
            ..rec
        };
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Bucket, 1)));
        inv.set_slot(1, Some(ItemStack::new_material(MaterialId::Rubber, 1)));
        assert_eq!(undo(&mut inv, &mut ui, &tap), Undone::default());
        assert_eq!(inv.slot(0), Some(&ItemStack::new_material(MaterialId::Bucket, 1)));
        assert!(inv.slot(1).is_none());
    }

    /// C3c-3a — a hang's tag carries the Plan by marker; the server judges a
    /// developed Plan into an AIR (or water) cell beside a wall, and takes
    /// the copy's marker placeholder by marker (the other Plan stays).
    #[test]
    fn a_hung_print_is_judged_on_a_wall_and_takes_its_plan_by_marker() {
        use crate::plan::{marker, DevelopState, PlanData};
        let developed = PlanData::debug_3x3_stone();
        let latent = PlanData { develop_state: DevelopState::Latent { exposure_ticks: 0 }, ..developed.clone() };
        let t = tag(UseKind::HangPrint, [5, 71, 5], 2, Some(&Item::Plan(developed.clone())));
        assert_eq!(t.kind, 11);
        let used = crate::inventory::stack_from_wire(t.used.as_ref().expect("the Plan"), &crate::block::BlockRegistry::new(), crate::inventory::PlanDecode::Marker)
            .map(|s| s.item);
        let m = marker(&developed);
        assert_eq!(used, Some(Item::Plan(PlanData::marker_placeholder(m, true))), "the server's stand-in");
        assert_eq!(t.tool, WireItem::None);
        let mut w = World::new();
        let hang = bc(5, 71, 5, block::CYANOTYPE_PRINT);
        let verdict = |used: Option<&Item>, change: &BlockChange, old: BlockId, w: &World| judge(UseKind::HangPrint, used, change, before(old), w).verdict;
        assert_eq!(verdict(used.as_ref(), &hang, block::AIR, &w), Verdict::Drift, "no wall on the server: drift");
        w.set_block(6, 71, 5, block::STONE);
        let j = judge(UseKind::HangPrint, used.as_ref(), &hang, before(block::AIR), &w);
        assert!(j.legal() && j.product.is_none());
        assert_eq!(verdict(used.as_ref(), &hang, block::WATER, &w), Verdict::Legal);
        assert_eq!(verdict(used.as_ref(), &hang, block::DIRT, &w), Verdict::Drift, "an occupied cell on the server");
        assert_eq!(verdict(used.as_ref(), &bc(5, 71, 5, block::STONE), block::AIR, &w), Verdict::Unexplained, "not a print");
        let latent_used = Item::Plan(PlanData::marker_placeholder(marker(&latent), false));
        assert_eq!(verdict(Some(&latent_used), &hang, block::AIR, &w), Verdict::Impossible, "a latent Plan doesn't hang");
        assert_eq!(verdict(Some(&mat(MaterialId::Stick)), &hang, block::AIR, &w), Verdict::Impossible);
        // The copy: the latent Plan's placeholder in the tag's slot, the
        // developed one's in the bag. The right one goes.
        let mut copy = Inventory::new();
        copy.set_slot(2, Some(ItemStack { item: latent_used.clone(), count: 1 }));
        copy.set_slot(20, Some(ItemStack { item: Item::Plan(PlanData::marker_placeholder(m, true)), count: 1 }));
        let s = settle(&mut copy, UseKind::HangPrint, &t, used.as_ref(), &j);
        assert_eq!((s.miss, s.copy_overflow, s.unfit), (None, 0, None));
        assert!(copy.slot(20).is_none(), "the hung Plan went");
        assert_eq!(copy.slot(2).map(|s| s.item.clone()), Some(latent_used), "the other stayed");
        let s = settle(&mut copy, UseKind::HangPrint, &t, used.as_ref(), &j);
        assert_eq!(s.miss, Some(UseMiss::NothingToTake), "no second one to hang");
        assert_eq!(t.unfit, 0, "a hang makes nothing, so nothing is unfit");
    }

    /// C3c-2-fix (F-M1) — a frame's refusal notices are undone NEWEST first,
    /// which is exact for chained uses: a fill then an empty of the bucket it
    /// filled, both refused, put the client back to its one empty bucket.
    /// (Oldest first, the fill's undo was short and still gave its bucket
    /// back, and the empty's undo then turned that bucket into a filled one:
    /// a bucket and a water bucket from one bucket.)
    #[test]
    fn a_frames_notices_are_undone_newest_first() {
        let mut ui = crate::craft_ui::CraftingUi::new();
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Bucket, 1)));
        let mut uses = SentUses::default();
        let (pond, edge) = ([5, 70, 5], [6, 70, 5]);
        // The fill: the bucket spent, the water bucket landed in slot 0.
        assert!(inv.consume_one_material(0, MaterialId::Bucket));
        assert!(inv.add_item(ItemStack::new_material(MaterialId::WaterBucket, 1)).is_none());
        uses.record(UseRecord {
            cell: pond,
            kind: UseKind::BucketFill.to_wire(),
            slot: 0,
            cost: Some(mat(MaterialId::Bucket)),
            landed: Some(ItemStack::new_material(MaterialId::WaterBucket, 1)),
            made_at: 1,
        });
        // The empty: the water bucket spent, the bucket back.
        assert!(inv.consume_one_material(0, MaterialId::WaterBucket));
        assert!(inv.add_item(ItemStack::new_material(MaterialId::Bucket, 1)).is_none());
        uses.record(UseRecord {
            cell: edge,
            kind: UseKind::BucketEmpty.to_wire(),
            slot: 0,
            cost: Some(mat(MaterialId::WaterBucket)),
            landed: Some(ItemStack::new_material(MaterialId::Bucket, 1)),
            made_at: 1,
        });
        let notice = |c: [i32; 3], k: UseKind| crate::protocol::RefusedUse { x: c[0], y: c[1], z: c[2], kind: k.to_wire(), note: 0 };
        let undone = undo_refused(&mut uses, &mut inv, &mut ui, &[notice(pond, UseKind::BucketFill), notice(edge, UseKind::BucketEmpty)]);
        assert_eq!(undone.len(), 2);
        assert_eq!(undone[0].0.kind, UseKind::BucketEmpty.to_wire(), "the newest first");
        assert!(undone.iter().all(|(_, u)| *u == Some(Undone::default())), "{undone:?}");
        assert_eq!(inv.slot(0), Some(&ItemStack::new_material(MaterialId::Bucket, 1)));
        assert!(inv.slots_iter().skip(1).all(|s| s.is_none()), "one bucket and nothing else");
        assert_eq!(uses.len(), 0);
    }
}
