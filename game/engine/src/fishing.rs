//! Fishing (P6 gap-closure).
//!
//! Cast a Fishing Rod at water → wait a randomised time → a bite → right-click
//! again to reel in a catch. The catch is mostly Raw Fish (cook it on a
//! campfire), occasionally a double, rarely a bit of junk treasure.
//!
//! State is the ephemeral per-player [`FishingLine`] (not persisted — a cast in
//! progress doesn't survive a save, same as other transient player state). The
//! wait timer + loot roll are pure functions here so they're unit-testable; the
//! `game_loop` owns the rod right-click, the per-tick bite, and the reel-in.
//!
//! C3c-2 (protocol v81) — the rules are shared by every seat: the cast's water
//! ray ([`finds_water`]), the bite's wait ([`wait_ticks`]) and the catch
//! ([`roll_catch`]). A joiner's cast and reel are requests (`ItemAction::Cast`,
//! `Reel`): the SERVER casts from its own position for the player, draws the
//! wait and rolls the catch on its own seed, and grants the catch; the client
//! only shows the line and the bite (`ItemActionOutcomePacket::bite_after`).
//! A landed catch wears the rod by `Inventory::use_tool_at` on every seat, so
//! a rod at 0 breaks like every other tool; on a seat that owns its inventory
//! a catch that doesn't fit drops at the player ([`reel_in`]).

use glam::Vec3;

use crate::item::{ItemStack, MaterialId};

/// The cast's aim ray: this many steps of [`CAST_STEP`] blocks from the eye
/// along the look (12 blocks).
pub const CAST_STEPS: u32 = 24;
/// One step of the cast's aim ray, in blocks.
pub const CAST_STEP: f32 = 0.5;

/// C3c-2 — how early a joiner's reel may reach the server and still land the
/// catch: the server's bite tick less this. An honest client shows the bite
/// `bite_after` ticks after the cast's outcome ARRIVES, so its reel reaches
/// the server at least a round trip after the server's bite; the slack only
/// absorbs a client clock running ahead of a loaded server's (one second).
/// A hooked line has no closing window (single-player's never did): it waits
/// until it is reeled in.
pub const REEL_SLACK_TICKS: u64 = 20;

/// Shortest wait before a bite (5 s @ 20 TPS).
pub const MIN_WAIT_TICKS: u64 = 100;
/// Longest wait before a bite (15 s @ 20 TPS).
pub const MAX_WAIT_TICKS: u64 = 300;

/// An in-progress cast. `hooked` flips true once `catch_at_tick` is reached;
/// the player then right-clicks again to land the catch.
///
/// `target` and `cast_tick` are drawing state only (`fishing_view`): where the
/// bobber floats (the water the cast found, [`cast_target`]) and the client tick
/// the line went out. No rule reads them. A joiner's client computes its own
/// target from its own world; with no water in sight there it has none and
/// draws no bobber, while the server's rules are unchanged.
#[derive(Clone, Copy, Debug)]
pub struct FishingLine {
    pub catch_at_tick: u64,
    pub hooked: bool,
    pub cast_tick: u64,
    pub target: Option<Vec3>,
    /// The hand the cast was made with (drawing only: the line leaves THAT rod).
    pub hand: crate::hands::Hand,
    /// The hotbar slot the rod was in when cast (a main-hand rod; the line is
    /// drawn only while a rod is still there).
    pub slot: usize,
    /// A joiner's cast still waiting for the server's word: the request's
    /// sequence number. An outcome applies to the line only if it answers THIS
    /// request, so a late reply to an earlier cast never touches a later one.
    pub request: Option<u32>,
}

impl FishingLine {
    /// A line cast at client tick `now` that bites at `catch_at_tick` (cast with
    /// the main hand's rod in hotbar slot 0 until [`Self::with_rod`] says otherwise).
    pub fn cast(now: u64, catch_at_tick: u64, target: Option<Vec3>) -> Self {
        FishingLine { catch_at_tick, hooked: false, cast_tick: now, target, hand: crate::hands::Hand::Main, slot: 0, request: None }
    }

    /// This line was cast with the rod in `hand` (hotbar slot `slot`).
    pub fn with_rod(self, hand: crate::hands::Hand, slot: usize) -> Self {
        FishingLine { hand, slot, ..self }
    }

    /// This line waits for the server's answer to request `seq`.
    pub fn awaiting(self, seq: Option<u32>) -> Self {
        FishingLine { request: seq, ..self }
    }

    /// Is the rod this line was cast with still where it was cast from? `main` and
    /// `off` say whether the hands hold a rod now and `hotbar_slot` is the selected
    /// slot: a main-hand cast follows its slot (selecting another slot, even one
    /// with a second rod, is a different rod), an off-hand cast its hand.
    pub fn rod_in_place(&self, hotbar_slot: usize, main_is_rod: bool, off_is_rod: bool) -> bool {
        match self.hand {
            crate::hands::Hand::Main => main_is_rod && hotbar_slot == self.slot,
            crate::hands::Hand::Off => off_is_rod,
        }
    }

    /// Does an outcome for request `seq` answer this line's cast?
    pub fn answers(&self, seq: u32) -> bool {
        self.request == Some(seq)
    }
}

/// How far above the water cell's floor the bobber's surface is: the top of a
/// cell of depth `level`, as the mesher draws it (`water::water_surface_height`).
fn water_top(level: u8) -> f32 {
    crate::water::water_surface_height(level)
}

/// The most cells of water above the first one the cast finds that it climbs to
/// reach the surface.
const SURFACE_CLIMB: i32 = 8;

/// Where the cast from `eye` along `dir` (unit) lands: the first water along the
/// ray within [`CAST_STEPS`] steps, snapped to the top of that column of water
/// (the surface the bobber floats on), or `None` when there is none. `is_water`
/// reads the caster's world: single-player's own, or the server's for a joiner's
/// `Cast`.
pub fn cast_target(eye: Vec3, dir: Vec3, is_water: impl Fn(i32, i32, i32) -> bool) -> Option<Vec3> {
    cast_target_levelled(eye, dir, is_water, |_, _, _| 0)
}

/// [`cast_target`] with the surface cell's depth level (`water::level_at`: 0 a
/// source or falling water, up to 7 the thinnest flow) so the bobber floats at the
/// height the mesher draws that cell's surface, not at a source's.
pub fn cast_target_levelled(
    eye: Vec3,
    dir: Vec3,
    is_water: impl Fn(i32, i32, i32) -> bool,
    level_at: impl Fn(i32, i32, i32) -> u8,
) -> Option<Vec3> {
    let (p, cell) = (1..=CAST_STEPS).find_map(|step| {
        let p = eye + dir * (step as f32 * CAST_STEP);
        let cell = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        is_water(cell.0, cell.1, cell.2).then_some((p, cell))
    })?;
    let mut top = cell.1;
    while top - cell.1 < SURFACE_CLIMB && is_water(cell.0, top + 1, cell.2) {
        top += 1;
    }
    Some(Vec3::new(p.x, top as f32 + water_top(level_at(cell.0, top, cell.2)), p.z))
}

/// Is there water along the cast from `eye` along `dir` (unit), within
/// [`CAST_STEPS`] steps? The rule every seat shares ([`cast_target`] is its
/// answer with the place).
pub fn finds_water(eye: Vec3, dir: Vec3, is_water: impl Fn(i32, i32, i32) -> bool) -> bool {
    cast_target(eye, dir, is_water).is_some()
}

/// C3c-2 — has a line cast to bite at server tick `bite_tick` hooked by
/// server tick `now` (with [`REEL_SLACK_TICKS`])?
pub fn hooked(now: u64, bite_tick: u64) -> bool {
    now.saturating_add(REEL_SLACK_TICKS) >= bite_tick
}

/// What reeling in a hooked line did on a seat that owns its inventory.
#[derive(Debug)]
pub struct Reeled {
    /// What was caught.
    pub catch: ItemStack,
    /// The part of the catch the inventory had no room for: the caller drops
    /// it at the player (C3c-2: it used to vanish).
    pub leftover: Option<ItemStack>,
    /// The rod's wear (`Inventory::use_hotbar_tool`; C3c-2: a rod at 0
    /// breaks, as every tool does — it used to stay at 0 for ever).
    pub wear: Option<crate::inventory::ToolUseInfo>,
}

/// Single-player's and a host's seat's reel of a hooked line with the rod in
/// hotbar slot `hot`: the catch rolled from `seed` is added to `inv`, then the
/// rod wears.
pub fn reel_in(inv: &mut crate::inventory::Inventory, hot: usize, seed: u64) -> Reeled {
    let catch = roll_catch(seed);
    let leftover = inv.add_item(catch.clone());
    let wear = inv.use_hotbar_tool(hot);
    Reeled { catch, leftover, wear }
}

/// [`reel_in`] with the rod in some other cell than a hotbar slot (the
/// off-hand, `hands.rs`): the catch goes into `inv`, the rod in `rod` wears.
pub fn reel_in_with(inv: &mut crate::inventory::Inventory, rod: &mut Option<ItemStack>, seed: u64) -> Reeled {
    let catch = roll_catch(seed);
    let leftover = inv.add_item(catch.clone());
    let wear = crate::inventory::use_tool_cell(rod);
    Reeled { catch, leftover, wear }
}

/// Randomised wait (ticks) until a bite. Seeded so it's deterministic +
/// replay-stable (typically `tick ^ player-something`).
pub fn wait_ticks(seed: u64) -> u64 {
    let span = MAX_WAIT_TICKS - MIN_WAIT_TICKS + 1;
    MIN_WAIT_TICKS + seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) % span
}

/// Roll a catch. ~70% one Raw Fish, ~20% two, ~7% a Bone (driftwood/junk
/// stand-in), ~3% Leather (the classic "old boot"). Pure + seeded.
pub fn roll_catch(seed: u64) -> ItemStack {
    let r = seed.wrapping_mul(0x2545_F491_4F6C_DD1D) % 100;
    if r < 70 {
        ItemStack::new_material(MaterialId::RawFish, 1)
    } else if r < 90 {
        ItemStack::new_material(MaterialId::RawFish, 2)
    } else if r < 97 {
        ItemStack::new_material(MaterialId::Bone, 1)
    } else {
        ItemStack::new_material(MaterialId::Leather, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_is_always_in_range() {
        for seed in 0..1000u64 {
            let w = wait_ticks(seed);
            assert!((MIN_WAIT_TICKS..=MAX_WAIT_TICKS).contains(&w), "seed {seed} -> {w}");
        }
    }

    #[test]
    fn catch_is_mostly_fish_but_varied() {
        let mut fish = 0;
        let mut junk = 0;
        for seed in 0..1000u64 {
            let c = roll_catch(seed);
            match c.item {
                crate::item::Item::Material(MaterialId::RawFish) => fish += 1,
                crate::item::Item::Material(MaterialId::Bone)
                | crate::item::Item::Material(MaterialId::Leather) => junk += 1,
                other => panic!("unexpected catch {other:?}"),
            }
        }
        // Fish dominate; junk shows up but is the minority.
        assert!(fish > junk, "fish ({fish}) should dominate junk ({junk})");
        assert!(junk > 0, "some junk should appear across 1000 rolls");
    }

    #[test]
    fn the_cast_finds_water_within_twelve_blocks_of_the_eye() {
        let eye = Vec3::new(0.5, 70.5, 0.5);
        let dir = Vec3::new(1.0, 0.0, 0.0);
        let pond = |at: i32| move |x: i32, y: i32, z: i32| x == at && y == 70 && z == 0;
        assert!(finds_water(eye, dir, pond(5)));
        assert!(finds_water(eye, dir, pond(12)), "the last step reaches x = 12.5");
        assert!(!finds_water(eye, dir, pond(13)));
        assert!(!finds_water(eye, -dir, pond(5)), "behind the caster");
    }

    /// A pond of full water cells `depth` deep, its top layer at y = 70, between
    /// x = 5 and x = 9 (z = 0 only).
    fn pond_column(depth: i32) -> impl Fn(i32, i32, i32) -> bool {
        move |x, y, z| (5..=9).contains(&x) && z == 0 && (71 - depth..=70).contains(&y)
    }

    #[test]
    fn the_cast_lands_on_the_surface_of_the_first_water_it_meets() {
        let eye = Vec3::new(0.5, 72.5, 0.5);
        let dir = Vec3::new(1.0, -0.25, 0.0).normalize();
        let t = cast_target(eye, dir, pond_column(3)).expect("water ahead");
        assert!((t.y - (70.0 + crate::water::water_surface_height(0))).abs() < 1e-5, "the top of the top layer: {t:?}");
        assert!((5.0..9.0).contains(&t.x), "over the pond: {t:?}");
        assert_eq!(t.z.floor(), 0.0);
        // A ray that first meets a deep cell (low pitch from a high bank) still floats on the top.
        let low = cast_target(Vec3::new(0.5, 70.5, 0.5), Vec3::X, |x, y, z| (5..=9).contains(&x) && z == 0 && (60..=70).contains(&y)).unwrap();
        assert!((low.y - (70.0 + crate::water::water_surface_height(0))).abs() < 1e-5, "{low:?}");
        let climbed = cast_target(Vec3::new(0.5, 66.5, 0.5), Vec3::X, |x, y, z| (5..=9).contains(&x) && z == 0 && (60..=70).contains(&y)).unwrap();
        assert!((climbed.y - (70.0 + crate::water::water_surface_height(0))).abs() < 1e-5, "{climbed:?}");
    }

    #[test]
    fn the_cast_target_is_there_exactly_when_the_cast_finds_water() {
        let eye = Vec3::new(0.5, 70.5, 0.5);
        for at in [3, 5, 12, 13, 20] {
            for dir in [Vec3::X, -Vec3::X, Vec3::new(1.0, -0.1, 0.0).normalize(), Vec3::Y] {
                let pond = move |x: i32, y: i32, z: i32| x == at && (68..=70).contains(&y) && z == 0;
                assert_eq!(cast_target(eye, dir, pond).is_some(), finds_water(eye, dir, pond), "pond at {at}, look {dir:?}");
            }
        }
        assert!(cast_target(eye, Vec3::X, |_, _, _| false).is_none());
        assert!(cast_target(eye, Vec3::X, pond_column(1)).is_some(), "the pond is 5 blocks ahead");
    }

    #[test]
    fn a_new_line_remembers_where_and_when_it_was_cast() {
        let line = FishingLine::cast(40, 190, Some(Vec3::new(1.0, 2.0, 3.0)));
        assert_eq!((line.cast_tick, line.catch_at_tick, line.hooked), (40, 190, false));
        assert_eq!(line.target, Some(Vec3::new(1.0, 2.0, 3.0)));
    }

    #[test]
    fn the_bobber_floats_at_the_height_of_the_cells_own_surface() {
        // A level-7 flow is 1/9 of a block deep: the bobber sits there, not at a
        // source's 8/9.
        let eye = Vec3::new(0.5, 72.5, 0.5);
        let dir = Vec3::new(1.0, -0.25, 0.0).normalize();
        let still = cast_target_levelled(eye, dir, pond_column(1), |_, _, _| 0).expect("water");
        let shallow = cast_target_levelled(eye, dir, pond_column(1), |_, _, _| 7).expect("water");
        assert!((still.y - (70.0 + crate::water::water_surface_height(0))).abs() < 1e-5);
        assert!((shallow.y - (70.0 + crate::water::water_surface_height(7))).abs() < 1e-5, "{shallow:?}");
        assert!(still.y - shallow.y > 0.7, "about 0.78 of a block lower: {} vs {}", still.y, shallow.y);
        // The level read is the TOP cell's: a deep column's surface cell, not the
        // cell the ray first met.
        let top_only = cast_target_levelled(eye, dir, pond_column(3), |_, y, _| if y == 70 { 7 } else { 0 }).expect("water");
        assert!((top_only.y - (70.0 + crate::water::water_surface_height(7))).abs() < 1e-5, "{top_only:?}");
        // The plain cast is the source-height one (what the server's rule uses).
        assert_eq!(cast_target(eye, dir, pond_column(1)), Some(still));
    }

    #[test]
    fn a_line_is_drawn_only_while_the_rod_it_was_cast_with_is_where_it_was() {
        use crate::hands::Hand;
        let main = FishingLine::cast(0, 100, None).with_rod(Hand::Main, 3);
        assert!(main.rod_in_place(3, true, false));
        assert!(!main.rod_in_place(4, true, false), "a second rod on another slot is a different rod");
        assert!(!main.rod_in_place(3, false, false), "the rod went from the slot");
        assert!(!main.rod_in_place(3, false, true), "an off-hand rod is not the main hand's");
        let off = FishingLine::cast(0, 100, None).with_rod(Hand::Off, 3);
        assert!(off.rod_in_place(7, false, true), "the off-hand's rod ignores the hotbar slot");
        assert!(!off.rod_in_place(3, true, false), "a main-hand rod is not the off-hand's");
    }

    #[test]
    fn an_outcome_answers_only_the_cast_it_was_asked_for() {
        let a = FishingLine::cast(0, u64::MAX, None).awaiting(Some(7));
        assert!(a.answers(7) && !a.answers(8));
        // A line with no request pending (single-player's, or one already answered)
        // answers nothing.
        assert!(!FishingLine::cast(0, 100, None).answers(7));
        assert!(!a.awaiting(None).answers(7));
    }

    #[test]
    fn a_line_hooks_at_its_bite_less_the_slack_and_stays_hooked() {
        assert!(!hooked(100, 100 + REEL_SLACK_TICKS + 1));
        assert!(hooked(100, 100 + REEL_SLACK_TICKS));
        assert!(hooked(1_000_000, 100), "no closing window");
    }

    /// C3c-2 — single-player's reel: the rod wears by `use_hotbar_tool`, so
    /// a rod at 0 breaks; a catch that doesn't fit comes back to be dropped.
    #[test]
    fn a_rod_at_zero_breaks_and_a_full_bags_catch_comes_back() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        use crate::inventory::Inventory;
        let rod = |durability| ItemStack::new_tool(Tool { durability, ..Tool::new(ToolType::FishingRod, ToolMaterial::Wood) });
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(rod(5)));
        let r = reel_in(&mut inv, 0, 1);
        assert!(r.leftover.is_none());
        assert!(!r.wear.as_ref().unwrap().just_broke);
        assert_eq!(inv.slot(0), Some(&rod(4)), "worn once");
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(rod(0)));
        let r = reel_in(&mut inv, 0, 1);
        assert!(r.wear.unwrap().just_broke, "a rod at 0 breaks");
        assert!(inv.slot(0).is_none(), "and leaves the hand");
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(rod(9)));
        for k in 1..36 {
            inv.set_slot(k, Some(ItemStack::new_block(crate::block::STONE, 64)));
        }
        let r = reel_in(&mut inv, 0, 1);
        assert_eq!(r.leftover.as_ref(), Some(&r.catch), "the whole catch comes back to be dropped");
    }

    #[test]
    fn double_fish_sometimes_drops_two() {
        let saw_two = (0..1000u64).any(|s| roll_catch(s).count == 2);
        assert!(saw_two, "a double-fish catch should occur across 1000 rolls");
    }
}
