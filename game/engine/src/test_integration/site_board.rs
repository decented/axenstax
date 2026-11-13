//! Living world Tier 2 S7 — Site Boards through a real `HostedServer`: a joiner's
//! `ItemAction::SiteBoard`, the list shown to every joiner that holds the chunk
//! (and to one that arrives later), the reach and block checks, the bloom surge
//! once per board per day, the break that takes the list, and the save.
//!
//! Joiners and the server are the block-use rig's (`block_use::Rig`): a REAL
//! server over the in-process transport, a dedicated one or a lending host, and
//! joiner halves with worlds of their own that take in the server's block views.
use super::block_use::{refused, Client, Rig};
use crate::block;
use crate::item_actions::ItemNote;
use crate::joiner_actions::{Asked, Pending};
use crate::protocol::{self, BlockView, ItemActionOutcomePacket};
use crate::site_board::{Job, Op, SiteBoard, Task, MAX_TASKS, SURGE_GUESTS};
use crate::transport::ClientTransport;

use super::joiner_authority::join_guest;
use super::joiners_act::floor_and_stand;

impl Client {
    /// Send `ItemAction::SiteBoard { cell, op }` as `GameState::send_site_board_op`
    /// does: recorded as a request in flight, claiming nothing.
    fn site_board_op(&mut self, cell: [i32; 3], op: Op) {
        let seq = self.actions.record(
            Pending { kind: Asked::SiteBoard { cell }, mob: None, hotbar_slot: 0, held: None },
            self.input_seq + 1,
        );
        let pkt = protocol::ItemActionPacket {
            seq,
            action: protocol::ItemAction::SiteBoard { pos: cell, op },
            events_applied: self.events, hand: crate::hands::Hand::Main };
        self.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
    }
}

/// Joiner `n` asks for `op` on the board at `cell`; the server answers.
fn ask(rig: &mut Rig, n: usize, cell: [i32; 3], op: Op) -> ItemActionOutcomePacket {
    let before = rig.cs[n].outcomes.len();
    rig.cs[n].site_board_op(cell, op);
    rig.ticks(3);
    assert_eq!(rig.cs[n].outcomes.len(), before + 1, "answered once");
    rig.cs[n].last().clone()
}

fn ok(out: &ItemActionOutcomePacket) {
    assert!(out.accepted, "accepted: {out:?}");
    assert_eq!((out.consume_held, out.window_event, out.wear_held), (0, 0, false), "it costs nothing");
    assert_eq!(out.note, 0);
}

fn pos(cell: [i32; 3]) -> (i32, i32, i32) {
    (cell[0], cell[1], cell[2])
}

fn tasks(pairs: &[(Job, bool)]) -> Vec<Task> {
    pairs.iter().map(|&(job, done)| Task { job, done }).collect()
}

/// A joiner adds, ticks and removes; another joiner sees every step in its own
/// world, as the list the server holds.
#[test]
fn a_joiner_adds_ticks_and_removes_and_every_joiner_sees_the_servers_list() {
    let mut rig = Rig::dedicated("sb-two", 2);
    let cell = rig.place(2, block::SITE_BOARD);
    ok(&ask(&mut rig, 0, cell, Op::add(Job::DigWell)));
    ok(&ask(&mut rig, 0, cell, Op::add(Job::BuildRoof)));
    ok(&ask(&mut rig, 1, cell, Op::add(Job::Decorate)));
    assert_eq!(
        rig.world().site_boards[&pos(cell)].tasks,
        tasks(&[(Job::DigWell, false), (Job::BuildRoof, false), (Job::Decorate, false)]),
        "the server's list, in the order it was asked"
    );
    // Anyone may tick anyone's job.
    ok(&ask(&mut rig, 1, cell, Op::toggle(0, Job::DigWell)));
    ok(&ask(&mut rig, 0, cell, Op::toggle(2, Job::Decorate)));
    ok(&ask(&mut rig, 0, cell, Op::remove(1, Job::BuildRoof)));
    let want = tasks(&[(Job::DigWell, true), (Job::Decorate, true)]);
    assert_eq!(rig.world().site_boards[&pos(cell)].tasks, want);
    for n in 0..2 {
        assert_eq!(rig.cs[n].world.site_boards[&pos(cell)].tasks, want, "joiner {n} is shown the same list");
        assert_eq!(
            rig.cs[n].view_at(cell),
            Some(&BlockView::SiteBoard { tasks: vec![(Job::DigWell.to_u8(), true), (Job::Decorate.to_u8(), true)] })
        );
    }
    // Nothing changed: nothing more is sent.
    let seen = rig.cs[1].views.len();
    rig.ticks(5);
    assert_eq!(rig.cs[1].views.len(), seen, "an unchanged list is not sent again");
}

/// A joiner who arrives after the list was made is shown it with its chunk: the
/// view goes again after any push of the chunk to it.
#[test]
fn a_late_joiner_is_shown_the_board() {
    let mut rig = Rig::dedicated("sb-late", 1);
    let cell = rig.place(2, block::SITE_BOARD);
    ok(&ask(&mut rig, 0, cell, Op::add(Job::PlantTrees)));
    ok(&ask(&mut rig, 0, cell, Op::add(Job::GatherWood)));
    ok(&ask(&mut rig, 0, cell, Op::toggle(1, Job::GatherWood)));

    // The late joiner: attached, stood by the board and holding its column (as
    // the chunk push would leave it), with the board's block in its world.
    let (transport, slot) = join_guest(&mut rig.hs, "Late");
    let mut world = std::mem::replace(&mut rig.hs.server.world, crate::world::World::new());
    // (Standing the joiner clears the air above the floor, board included: put the
    // block and its list back, as they were.)
    let boards = world.site_boards.clone();
    let at = floor_and_stand(&mut world, &mut rig.hs, slot);
    world.set_block(cell[0], cell[1], cell[2], block::SITE_BOARD);
    world.site_boards = boards;
    rig.hs.server.world = world;
    rig.hs.hold_column_for_test(slot, crate::chunk_stream::column_of(at));
    let mut late = Client::new((transport, slot));
    late.world.set_block(cell[0], cell[1], cell[2], block::SITE_BOARD);
    rig.cs.push(late);
    rig.ticks(8);

    let want = tasks(&[(Job::PlantTrees, false), (Job::GatherWood, true)]);
    assert_eq!(
        rig.cs[1].world.site_boards.get(&pos(cell)).map(|b| &b.tasks),
        Some(&want),
        "shown the list on arrival"
    );
    assert!(rig.cs[1].view_at(cell).is_some());
}

/// A joiner acting on a view that has gone stale (another joiner took a job off
/// first) is refused rather than ticking or removing a different task.
#[test]
fn a_stale_view_cannot_tick_or_remove_another_task() {
    let mut rig = Rig::dedicated("sb-stale", 2);
    let cell = rig.place(2, block::SITE_BOARD);
    for job in [Job::DigWell, Job::BuildRoof, Job::BuildWalls] {
        ok(&ask(&mut rig, 0, cell, Op::add(job)));
    }
    // Joiner 0 sees [Dig, Roof, Walls]; joiner 1 takes Dig off. Joiner 0 acts on
    // what it saw: "tick Roof" is index 1, which now holds Walls.
    ok(&ask(&mut rig, 1, cell, Op::remove(0, Job::DigWell)));
    let before = rig.world().site_boards[&pos(cell)].clone();
    refused(&ask(&mut rig, 0, cell, Op::toggle(1, Job::BuildRoof)), ItemNote::NothingToTake);
    refused(&ask(&mut rig, 0, cell, Op::remove(1, Job::BuildRoof)), ItemNote::NothingToTake);
    assert_eq!(rig.world().site_boards[&pos(cell)], before, "nothing changed");
    assert!(rig.world().feast_events.is_empty(), "and no surge");
    // With the right index for the job it saw, it works.
    ok(&ask(&mut rig, 0, cell, Op::toggle(0, Job::BuildRoof)));
}

/// Out of reach, a cell that is not a board, a ninth task, a missing task and a
/// job number nobody has: each refused, and nothing changed.
#[test]
fn refusals_change_nothing_and_say_why() {
    let mut rig = Rig::dedicated("sb-refuse", 1);
    let cell = rig.place(2, block::SITE_BOARD);
    let far = rig.place(9, block::SITE_BOARD);
    refused(&ask(&mut rig, 0, far, Op::add(Job::DigWell)), ItemNote::OutOfReach);
    assert!(rig.world().site_boards.is_empty(), "no list made for a refusal");
    let stone = rig.place(1, block::STONE);
    refused(&ask(&mut rig, 0, stone, Op::add(Job::DigWell)), ItemNote::NotThatBlock);
    assert!(rig.world().site_boards.is_empty());
    // Not a board in the server's world even where the joiner's copy thinks so.
    rig.cs[0].world.set_block(stone[0], stone[1], stone[2], block::SITE_BOARD);
    refused(&ask(&mut rig, 0, stone, Op::toggle(0, Job::DigWell)), ItemNote::NotThatBlock);

    // A task that is not there, and a job we do not know.
    refused(&ask(&mut rig, 0, cell, Op::toggle(0, Job::DigWell)), ItemNote::NothingToTake);
    refused(&ask(&mut rig, 0, cell, Op::remove(3, Job::DigWell)), ItemNote::NothingToTake);
    refused(&ask(&mut rig, 0, cell, Op::Add(200)), ItemNote::NothingToTake);
    assert!(rig.world().site_boards.is_empty(), "still no list");

    // Fill it; the ninth is refused and the eight stand.
    for _ in 0..MAX_TASKS {
        ok(&ask(&mut rig, 0, cell, Op::add(Job::Decorate)));
    }
    refused(&ask(&mut rig, 0, cell, Op::add(Job::DigWell)), ItemNote::NothingToTake);
    assert_eq!(rig.world().site_boards[&pos(cell)].tasks.len(), MAX_TASKS);
    assert!(rig.world().site_boards[&pos(cell)].tasks.iter().all(|t| t.job == Job::Decorate));
}

/// Ticking a job lifts the board's region (the Feast surge path) once per board
/// per world day, on a lending host where the host's own seat and a joiner use
/// the one list; spam cannot pump it.
#[test]
fn a_done_tick_lifts_bloom_once_per_board_per_day_on_a_lending_host() {
    let mut rig = Rig::lent("sb-surge", 1);
    let cell = rig.place(2, block::SITE_BOARD);
    let second = rig.place(4, block::SITE_BOARD);
    ok(&ask(&mut rig, 0, cell, Op::add(Job::BuildWalls)));
    ok(&ask(&mut rig, 0, cell, Op::add(Job::BuildRoof)));
    assert!(rig.world().feast_events.is_empty(), "adding a job lifts nothing");
    ok(&ask(&mut rig, 0, cell, Op::toggle(0, Job::BuildWalls)));
    assert_eq!(rig.world().feast_events, vec![(cell[0], cell[1], cell[2], SURGE_GUESTS)], "one surge, two guests");
    // Tick and untick over and over, the other job too: no more today.
    for _ in 0..6 {
        ok(&ask(&mut rig, 0, cell, Op::toggle(0, Job::BuildWalls)));
        ok(&ask(&mut rig, 0, cell, Op::toggle(1, Job::BuildRoof)));
    }
    assert_eq!(rig.world().feast_events.len(), 1, "spam cannot pump bloom");
    // The host's own seat uses the same rule on the lent world and the joiner sees it.
    let tonight = rig.hs.server.night_calendar.tonight();
    crate::site_board::use_board(rig.world(), pos(second), Op::add(Job::MakeGarden), tonight).unwrap();
    crate::site_board::use_board(rig.world(), pos(second), Op::toggle(0, Job::MakeGarden), tonight).unwrap();
    assert_eq!(rig.world().feast_events.len(), 2, "another board has its own surge");
    rig.ticks(2);
    assert_eq!(rig.cs[0].world.site_boards[&pos(second)].tasks, tasks(&[(Job::MakeGarden, true)]), "the joiner sees the host's tick");
}

/// Breaking the board takes its list from the server and from every joiner;
/// a board put in its place starts empty.
#[test]
fn breaking_the_board_removes_its_list_everywhere() {
    let mut rig = Rig::dedicated("sb-break", 2);
    let cell = rig.place(2, block::SITE_BOARD);
    ok(&ask(&mut rig, 0, cell, Op::add(Job::LightTheWay)));
    assert!(rig.cs[1].world.site_boards.contains_key(&pos(cell)));
    rig.edit(1, pos(cell), block::AIR);
    rig.ticks(3);
    assert_eq!(rig.world().get_block(cell[0], cell[1], cell[2]), block::AIR, "broken");
    assert!(rig.world().site_boards.is_empty(), "the server's list went with it");
    for n in 0..2 {
        assert!(rig.cs[n].world.site_boards.is_empty(), "joiner {n}'s copy went too");
    }
    // Asking after the break is refused: there is no board.
    refused(&ask(&mut rig, 0, cell, Op::add(Job::DigWell)), ItemNote::NotThatBlock);
    // Put back, it starts empty.
    rig.edit(0, pos(cell), block::SITE_BOARD);
    rig.ticks(3);
    assert_eq!(rig.world().get_block(cell[0], cell[1], cell[2]), block::SITE_BOARD);
    assert!(rig.world().site_boards.is_empty());
}

// ── Save ────────────────────────────────────────────────────────────

fn saved_world_save(world: &str) -> crate::save::WorldSave {
    crate::save::read_world_save(&std::fs::read(crate::save::world_dir(world).join("world.dat")).unwrap()).unwrap()
}

fn board(jobs: &[(Job, bool)]) -> SiteBoard {
    SiteBoard { tasks: tasks(jobs) }
}

#[test]
fn the_lists_round_trip_through_a_real_save_and_an_empty_table_is_not_written() {
    let _g = crate::save::WorldsRootGuard::new("s7-roundtrip");
    let mut s = crate::server::GameServer::new(0, "w".into(), 42);
    s.world.place_player_block(5, 64, 5, block::STONE);
    s.try_save().unwrap();
    assert!(saved_world_save("w").site_boards.is_none(), "no board: the field is absent");

    for c in [(9, 70, -3), (-4, 64, 2)] {
        s.world.set_block(c.0, c.1, c.2, block::SITE_BOARD);
    }
    s.world.site_boards.insert((9, 70, -3), board(&[(Job::BuildWalls, false), (Job::Decorate, true)]));
    s.world.site_boards.insert((-4, 64, 2), board(&[(Job::DigWell, true)]));
    s.world.set_block(1, 64, 1, block::SITE_BOARD);
    s.world.site_boards.insert((1, 64, 1), SiteBoard::default()); // an emptied board saves nothing
    s.try_save().unwrap();
    let saved = saved_world_save("w").site_boards.expect("written");
    assert_eq!(
        saved.boards,
        vec![((-4, 64, 2), vec![(3, true)]), ((9, 70, -3), vec![(1, false), (11, true)])],
        "sorted by cell, no empty board"
    );

    let mut back = crate::server::GameServer::new(0, "w".into(), 42);
    back.initial_load().unwrap();
    assert_eq!(back.world.site_boards.len(), 2);
    assert_eq!(back.world.site_boards[&(9, 70, -3)], board(&[(Job::BuildWalls, false), (Job::Decorate, true)]));
    // The same lists encode to the same bytes, whatever order they were made in.
    assert_eq!(
        bincode::serialize(&crate::site_board::to_saved(&back.world.site_boards)).unwrap(),
        bincode::serialize(&Some(saved)).unwrap()
    );
}

/// A world from before the field (footer 67, 60 fields) loads with no boards.
#[test]
fn a_footer_67_save_loads_with_no_boards() {
    let mut save = crate::save::minimal_world_save_for_tests(7);
    save.site_boards = None;
    let mut body = bincode::serialize(&save).unwrap();
    assert_eq!(body.pop(), Some(0), "the absent off-hand column is one None byte");
    assert_eq!(body.pop(), Some(0), "the absent weather hold is one None byte");
    assert_eq!(body.pop(), Some(0), "the absent field is one None byte");
    body.extend_from_slice(&crate::save_format::footer_bytes(67));
    let (old, damaged) = crate::save::read_world_save_reporting(&body).expect("an older save loads");
    assert!(!damaged);
    assert!(old.site_boards.is_none());
    let mut w = crate::world::World::new();
    w.site_boards.insert((0, 64, 0), board(&[(Job::DigWell, false)]));
    crate::save::apply_world_save_state(&mut w, &old);
    assert!(w.site_boards.is_empty(), "restoring a save without the field leaves no list");
}

/// The digest the Keep binds covers the lists.
#[test]
fn the_digest_sees_a_changed_list() {
    let mut s = crate::server::GameServer::new(0, "w".into(), 42);
    s.world.set_block(0, 64, 0, block::SITE_BOARD);
    let before = s.sim_digest();
    s.world.site_boards.insert((0, 64, 0), board(&[(Job::DigWell, false)]));
    let with_task = s.sim_digest();
    assert_ne!(before, with_task);
    s.world.site_boards.get_mut(&(0, 64, 0)).unwrap().tasks[0].done = true;
    assert_ne!(with_task, s.sim_digest(), "a tick changes it");
}
