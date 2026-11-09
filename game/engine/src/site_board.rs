//! Living world Tier 2 S7 — Site Boards (spec
//! `docs/superpowers/specs/2026-11-09-living-world-tier2.md` §7).
//!
//! A Site Board is a block that holds a short list of jobs for a crew: up to
//! [`MAX_TASKS`] tasks, each one job picked from a fixed list ([`Job`]) with a
//! done tick. There is no free text anywhere (call T9): a job is a number, its
//! words are ours. Anyone may add, tick or remove a task, so the table holds no
//! identities and an export or a handover keeps it.
//!
//! ONE pure rule, [`apply`], changes a board, and [`use_board`] is the one
//! world-level entry every path calls (a local seat's right-click, and the
//! authority serving a joiner's `ItemAction::SiteBoard`). Ticking a task done
//! also lifts the board's region with a short people surge (the Feast surge
//! path with [`SURGE_GUESTS`] guests), at most once per board per world day, so
//! ticking and unticking cannot pump bloom.
//!
//! The board's state reaches joiners as a block view
//! (`protocol::BlockView::SiteBoard`, `block_views`), which also covers a
//! joiner who arrives later (a view is re-sent for any chunk pushed to it).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::world::World;

/// Most tasks one board holds.
pub const MAX_TASKS: usize = 8;

/// Guests a done tick counts for in the surge: a short lift, well under a feast.
pub const SURGE_GUESTS: u8 = 2;

/// A job a crew can be asked to do. Stored as a `u8` on the wire and in saves,
/// APPEND ONLY: the numbers below never change meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Job {
    PlantTrees = 0,
    BuildWalls = 1,
    BuildRoof = 2,
    DigWell = 3,
    LightTheWay = 4,
    MakeGarden = 5,
    BringFood = 6,
    GatherWood = 7,
    ClearGround = 8,
    BuildBridge = 9,
    FenceAnimals = 10,
    Decorate = 11,
}

impl Job {
    /// Every job, in the order the picker lists them.
    pub const ALL: [Job; 12] = [
        Job::PlantTrees,
        Job::BuildWalls,
        Job::BuildRoof,
        Job::DigWell,
        Job::LightTheWay,
        Job::MakeGarden,
        Job::BringFood,
        Job::GatherWood,
        Job::ClearGround,
        Job::BuildBridge,
        Job::FenceAnimals,
        Job::Decorate,
    ];

    /// The stored number.
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// The job a stored number names, if any.
    pub fn from_u8(n: u8) -> Option<Job> {
        Job::ALL.get(usize::from(n)).copied().filter(|j| j.to_u8() == n)
    }

    /// The words shown for it.
    pub fn label(self) -> &'static str {
        match self {
            Job::PlantTrees => "Plant trees",
            Job::BuildWalls => "Build walls",
            Job::BuildRoof => "Build a roof",
            Job::DigWell => "Dig a well",
            Job::LightTheWay => "Light the way",
            Job::MakeGarden => "Make a garden",
            Job::BringFood => "Bring food",
            Job::GatherWood => "Gather wood",
            Job::ClearGround => "Clear the ground",
            Job::BuildBridge => "Build a bridge",
            Job::FenceAnimals => "Fence the animals",
            Job::Decorate => "Decorate",
        }
    }
}

/// One line on a board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Task {
    pub job: Job,
    pub done: bool,
}

/// A board's list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SiteBoard {
    pub tasks: Vec<Task>,
}

/// What a player asks of a board. On the wire (`ItemAction::SiteBoard`), so
/// APPEND ONLY: Add = 0, Toggle = 1, Remove = 2. A job is the raw number, so a
/// bad one is refused ([`Refusal::UnknownJob`]) rather than failing to decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    /// Add the job with this number ([`Job::to_u8`]).
    Add(u8),
    /// Flip the done tick of the task at this index.
    Toggle(u8),
    /// Take the task at this index off the board.
    Remove(u8),
}

impl Op {
    /// Add `job`.
    pub fn add(job: Job) -> Op {
        Op::Add(job.to_u8())
    }
}

/// What a granted op did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Added,
    /// The task is now `done` (or not).
    Toggled { done: bool },
    Removed,
}

impl Outcome {
    /// Did this op tick a task to done (the one thing that can lift bloom)?
    pub fn ticked_done(self) -> bool {
        self == Outcome::Toggled { done: true }
    }
}

/// Why an op was refused. Nothing changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The board already holds [`MAX_TASKS`].
    Full,
    /// No task at that index.
    NoSuchTask,
    /// Not a job we know.
    UnknownJob,
    /// There is no Site Board at that cell.
    NotABoard,
}

/// THE rule: apply `op` to `board`, or say why not. Anyone may do any op.
pub fn apply(board: &mut SiteBoard, op: Op) -> Result<Outcome, Refusal> {
    match op {
        Op::Add(n) => {
            let job = Job::from_u8(n).ok_or(Refusal::UnknownJob)?;
            if board.tasks.len() >= MAX_TASKS {
                return Err(Refusal::Full);
            }
            board.tasks.push(Task { job, done: false });
            Ok(Outcome::Added)
        }
        Op::Toggle(i) => {
            let task = board.tasks.get_mut(usize::from(i)).ok_or(Refusal::NoSuchTask)?;
            task.done = !task.done;
            Ok(Outcome::Toggled { done: task.done })
        }
        Op::Remove(i) => {
            let i = usize::from(i);
            if i >= board.tasks.len() {
                return Err(Refusal::NoSuchTask);
            }
            board.tasks.remove(i);
            Ok(Outcome::Removed)
        }
    }
}

/// Lift the region of the board at `cell` with a short people surge, at most
/// once per board per world day (`tonight`, the night calendar's number both
/// the local path and the authority keep). Returns whether it did. The surge
/// rides `World::note_feast`, which the clients drain for the confetti and
/// `BloomField::note_feast`. The table is runtime only: a reload forgets it.
pub fn surge_if_due(world: &mut World, cell: (i32, i32, i32), tonight: u32) -> bool {
    // Only today's entries matter; older ones are dropped so the table stays small.
    world.site_board_days.retain(|_, n| *n == tonight);
    if world.site_board_days.insert(cell, tonight).is_some() {
        return false;
    }
    world.note_feast(cell, SURGE_GUESTS);
    true
}

/// The world-level entry both paths call: `op` on the Site Board at `cell`.
/// The block must be a Site Board. A board's entry is created by its first
/// accepted op. A task ticked to done lifts bloom once per day ([`surge_if_due`]).
pub fn use_board(world: &mut World, cell: (i32, i32, i32), op: Op, tonight: u32) -> Result<Outcome, Refusal> {
    if world.get_block(cell.0, cell.1, cell.2) != crate::block::SITE_BOARD {
        return Err(Refusal::NotABoard);
    }
    let mut board = world.site_boards.get(&cell).cloned().unwrap_or_default();
    let outcome = apply(&mut board, op)?;
    world.site_boards.insert(cell, board);
    if outcome.ticked_done() {
        surge_if_due(world, cell, tonight);
    }
    Ok(outcome)
}

// ── Wire and save forms ──────────────────────────────────────────────────────

impl SiteBoard {
    /// The board as `(job number, done)` pairs, for the wire and the save.
    pub fn to_pairs(&self) -> Vec<(u8, bool)> {
        self.tasks.iter().map(|t| (t.job.to_u8(), t.done)).collect()
    }

    /// A board from pairs, keeping what is valid: an unknown job is dropped and
    /// the list is cut at [`MAX_TASKS`]. Returns how many pairs were dropped.
    pub fn from_pairs(pairs: &[(u8, bool)]) -> (SiteBoard, usize) {
        let mut board = SiteBoard::default();
        let mut dropped = 0;
        for &(n, done) in pairs {
            match Job::from_u8(n) {
                Some(job) if board.tasks.len() < MAX_TASKS => board.tasks.push(Task { job, done }),
                _ => dropped += 1,
            }
        }
        (board, dropped)
    }
}

/// Saved boards (`WorldSave.site_boards`): `(cell, tasks)` rows sorted by cell,
/// no empty board. Positional bincode: a change to this type is a
/// `SAVE_LAYOUT_REVISION` bump. Holds no identities, so an export and a
/// handover keep it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSiteBoards {
    pub boards: Vec<((i32, i32, i32), Vec<(u8, bool)>)>,
}

/// The canonical saved form of `boards`; `None` when no board holds a task.
pub fn to_saved(boards: &BTreeMap<(i32, i32, i32), SiteBoard>) -> Option<SavedSiteBoards> {
    let rows: Vec<_> = boards
        .iter()
        .filter(|(_, b)| !b.tasks.is_empty())
        .map(|(cell, b)| (*cell, b.to_pairs()))
        .collect();
    (!rows.is_empty()).then_some(SavedSiteBoards { boards: rows })
}

/// Boards back from a save. A present-but-invalid row (an unknown job, a
/// repeated cell, a list cut at the cap) is dropped or trimmed with a warning;
/// the table never refuses to load.
pub fn from_saved(saved: &SavedSiteBoards) -> BTreeMap<(i32, i32, i32), SiteBoard> {
    let mut out = BTreeMap::new();
    let mut dropped = 0usize;
    for (cell, pairs) in &saved.boards {
        if out.contains_key(cell) {
            dropped += pairs.len().max(1);
            continue;
        }
        let (board, d) = SiteBoard::from_pairs(pairs);
        dropped += d;
        if !board.tasks.is_empty() {
            out.insert(*cell, board);
        }
    }
    if dropped > 0 {
        log::warn!("world.dat: {dropped} invalid site board task(s) dropped");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;

    fn board_world() -> World {
        let mut w = World::new();
        w.set_block(4, 64, 4, block::SITE_BOARD);
        w
    }

    #[test]
    fn the_job_numbers_are_pinned_and_round_trip() {
        let pinned: [(Job, u8, &str); 12] = [
            (Job::PlantTrees, 0, "Plant trees"),
            (Job::BuildWalls, 1, "Build walls"),
            (Job::BuildRoof, 2, "Build a roof"),
            (Job::DigWell, 3, "Dig a well"),
            (Job::LightTheWay, 4, "Light the way"),
            (Job::MakeGarden, 5, "Make a garden"),
            (Job::BringFood, 6, "Bring food"),
            (Job::GatherWood, 7, "Gather wood"),
            (Job::ClearGround, 8, "Clear the ground"),
            (Job::BuildBridge, 9, "Build a bridge"),
            (Job::FenceAnimals, 10, "Fence the animals"),
            (Job::Decorate, 11, "Decorate"),
        ];
        for (job, n, label) in pinned {
            assert_eq!(job.to_u8(), n, "{label}: its number is part of the format");
            assert_eq!(Job::from_u8(n), Some(job));
            assert_eq!(job.label(), label);
            assert_eq!(Job::ALL[usize::from(n)], job, "the picker lists them in number order");
        }
        assert_eq!(Job::from_u8(12), None);
        assert_eq!(Job::from_u8(255), None);
    }

    #[test]
    fn add_toggle_and_remove() {
        let mut b = SiteBoard::default();
        assert_eq!(apply(&mut b, Op::add(Job::DigWell)), Ok(Outcome::Added));
        assert_eq!(apply(&mut b, Op::add(Job::BuildRoof)), Ok(Outcome::Added));
        assert_eq!(b.tasks.len(), 2);
        assert_eq!(apply(&mut b, Op::Toggle(1)), Ok(Outcome::Toggled { done: true }));
        assert!(b.tasks[1].done && !b.tasks[0].done);
        assert_eq!(apply(&mut b, Op::Toggle(1)), Ok(Outcome::Toggled { done: false }));
        assert_eq!(apply(&mut b, Op::Remove(0)), Ok(Outcome::Removed));
        assert_eq!(b.tasks, vec![Task { job: Job::BuildRoof, done: false }]);
    }

    #[test]
    fn a_board_holds_eight_and_refuses_the_ninth() {
        let mut b = SiteBoard::default();
        for _ in 0..MAX_TASKS {
            assert_eq!(apply(&mut b, Op::add(Job::Decorate)), Ok(Outcome::Added));
        }
        assert_eq!(apply(&mut b, Op::add(Job::Decorate)), Err(Refusal::Full));
        assert_eq!(b.tasks.len(), MAX_TASKS, "a refusal changes nothing");
        // Taking one off makes room again.
        assert_eq!(apply(&mut b, Op::Remove(3)), Ok(Outcome::Removed));
        assert_eq!(apply(&mut b, Op::add(Job::Decorate)), Ok(Outcome::Added));
    }

    #[test]
    fn an_out_of_range_index_or_an_unknown_job_is_refused() {
        let mut b = SiteBoard::default();
        assert_eq!(apply(&mut b, Op::Toggle(0)), Err(Refusal::NoSuchTask));
        assert_eq!(apply(&mut b, Op::Remove(0)), Err(Refusal::NoSuchTask));
        assert_eq!(apply(&mut b, Op::Add(12)), Err(Refusal::UnknownJob));
        assert_eq!(apply(&mut b, Op::Add(255)), Err(Refusal::UnknownJob));
        apply(&mut b, Op::add(Job::PlantTrees)).unwrap();
        assert_eq!(apply(&mut b, Op::Toggle(1)), Err(Refusal::NoSuchTask));
        assert_eq!(apply(&mut b, Op::Remove(255)), Err(Refusal::NoSuchTask));
        assert_eq!(b.tasks, vec![Task { job: Job::PlantTrees, done: false }], "refusals changed nothing");
    }

    #[test]
    fn the_op_encoding_is_pinned() {
        // Add = 0, Toggle = 1, Remove = 2: the leading u32 variant index.
        for (op, variant, arg) in [(Op::Add(7), 0u32, 7u8), (Op::Toggle(3), 1, 3), (Op::Remove(255), 2, 255)] {
            let bytes = bincode::serialize(&op).unwrap();
            assert_eq!(bytes[..4], variant.to_le_bytes(), "{op:?}");
            assert_eq!(bytes[4], arg);
            assert_eq!(bytes.len(), 5);
            assert_eq!(bincode::deserialize::<Op>(&bytes).unwrap(), op);
        }
    }

    #[test]
    fn use_board_needs_a_site_board_and_creates_the_entry_on_first_use() {
        let mut w = board_world();
        assert_eq!(use_board(&mut w, (9, 64, 9), Op::add(Job::DigWell), 1), Err(Refusal::NotABoard));
        assert!(w.site_boards.is_empty());
        assert_eq!(use_board(&mut w, (4, 64, 4), Op::add(Job::DigWell), 1), Ok(Outcome::Added));
        assert_eq!(w.site_boards[&(4, 64, 4)].tasks.len(), 1);
        // A refused op does not create an entry.
        let mut w = board_world();
        assert_eq!(use_board(&mut w, (4, 64, 4), Op::Toggle(0), 1), Err(Refusal::NoSuchTask));
        assert!(w.site_boards.is_empty());
    }

    #[test]
    fn breaking_the_board_removes_its_entry() {
        let mut w = board_world();
        use_board(&mut w, (4, 64, 4), Op::add(Job::DigWell), 1).unwrap();
        assert!(w.site_boards.contains_key(&(4, 64, 4)));
        w.set_block(4, 64, 4, block::AIR);
        assert!(w.site_boards.is_empty(), "the entry left with the block");
        // A board put back starts empty.
        w.set_block(4, 64, 4, block::SITE_BOARD);
        assert!(w.site_boards.is_empty());
        // Swapping it for another block does the same.
        use_board(&mut w, (4, 64, 4), Op::add(Job::DigWell), 1).unwrap();
        w.set_block(4, 64, 4, block::STONE);
        assert!(w.site_boards.is_empty());
    }

    #[test]
    fn a_done_tick_surges_once_per_board_per_day_and_spam_cannot_pump_it() {
        let mut w = board_world();
        w.set_block(20, 64, 20, block::SITE_BOARD);
        let cell = (4, 64, 4);
        use_board(&mut w, cell, Op::add(Job::DigWell), 3).unwrap();
        assert!(w.feast_events.is_empty(), "adding a job surges nothing");
        // Tick it: one surge, two guests.
        use_board(&mut w, cell, Op::Toggle(0), 3).unwrap();
        assert_eq!(w.feast_events, vec![(4, 64, 4, SURGE_GUESTS)]);
        // Untick, tick, untick, tick again the same day: nothing more.
        for _ in 0..10 {
            use_board(&mut w, cell, Op::Toggle(0), 3).unwrap();
        }
        assert_eq!(w.feast_events.len(), 1, "ticking and unticking cannot pump bloom");
        // Ten flips leave it done again; unticking on a new day surges nothing.
        use_board(&mut w, cell, Op::Toggle(0), 4).unwrap();
        let after_new_day = w.feast_events.len();
        assert_eq!(after_new_day, 1);
        // Another board has its own allowance on the same day.
        use_board(&mut w, (20, 64, 20), Op::add(Job::Decorate), 3).unwrap();
        use_board(&mut w, (20, 64, 20), Op::Toggle(0), 3).unwrap();
        assert_eq!(w.feast_events.len(), after_new_day + 1, "a second board has its own surge");
        // A new day allows the first board to surge again.
        w.feast_events.clear();
        use_board(&mut w, cell, Op::Toggle(0), 5).unwrap(); // undone -> done on day 5
        assert_eq!(w.feast_events, vec![(4, 64, 4, SURGE_GUESTS)]);
    }

    #[test]
    fn untick_alone_lifts_nothing() {
        let mut w = board_world();
        let cell = (4, 64, 4);
        use_board(&mut w, cell, Op::add(Job::DigWell), 1).unwrap();
        use_board(&mut w, cell, Op::Toggle(0), 1).unwrap(); // done: surge
        w.feast_events.clear();
        use_board(&mut w, cell, Op::Toggle(0), 2).unwrap(); // undone on a new day
        assert!(w.feast_events.is_empty(), "unticking does nothing to bloom");
    }

    #[test]
    fn the_saved_form_is_canonical_and_round_trips() {
        let mut boards = BTreeMap::new();
        let mut b = SiteBoard::default();
        apply(&mut b, Op::add(Job::BuildWalls)).unwrap();
        apply(&mut b, Op::add(Job::Decorate)).unwrap();
        apply(&mut b, Op::Toggle(1)).unwrap();
        boards.insert((9, 70, -3), b.clone());
        boards.insert((-4, 64, 2), b.clone());
        boards.insert((0, 64, 0), SiteBoard::default()); // an emptied board saves nothing
        let saved = to_saved(&boards).expect("two boards");
        assert_eq!(saved.boards.iter().map(|(c, _)| *c).collect::<Vec<_>>(), vec![(-4, 64, 2), (9, 70, -3)], "sorted by cell");
        assert_eq!(saved.boards[0].1, vec![(1, false), (11, true)]);
        let back = from_saved(&saved);
        assert_eq!(back.len(), 2);
        assert_eq!(back[&(9, 70, -3)], b);
        // The same table encodes to the same bytes.
        assert_eq!(bincode::serialize(&to_saved(&boards)).unwrap(), bincode::serialize(&to_saved(&back)).unwrap());
        // Nothing to save is None.
        assert_eq!(to_saved(&BTreeMap::new()), None);
        let mut only_empty = BTreeMap::new();
        only_empty.insert((0, 0, 0), SiteBoard::default());
        assert_eq!(to_saved(&only_empty), None);
    }

    #[test]
    fn bad_saved_rows_are_dropped_not_refused() {
        let saved = SavedSiteBoards {
            boards: vec![
                ((1, 64, 1), vec![(0, true), (99, false), (3, false)]),      // an unknown job
                ((1, 64, 1), vec![(2, false)]),                              // a repeated cell
                ((2, 64, 2), (0..12).map(|i| (i as u8, false)).collect()),   // past the cap
                ((3, 64, 3), vec![(77, false)]),                             // nothing valid
            ],
        };
        let back = from_saved(&saved);
        assert_eq!(back[&(1, 64, 1)].tasks.len(), 2, "the unknown job went, the rest stay; the repeat is ignored");
        assert_eq!(back[&(2, 64, 2)].tasks.len(), MAX_TASKS, "cut at the cap");
        assert!(!back.contains_key(&(3, 64, 3)), "a board with nothing valid is not kept");
    }
}
