//! Phase B2b — the server pushes only TOUCHED columns; a joiner whose
//! terrain generator matches the host's generates the rest itself
//! (`chunk_verdict`, `chunk_push`, `chunk_intake`; Spec 04 §4.1 "Touched
//! columns").
//!
//! Real joins through `HostedServer::tick` on a dedicated server (it streams
//! columns round every player, so every column in a joiner's range gets a
//! verdict), with the shared joiner harness (`push_joiner`).
//!
//! What they pin: a touched-mode join on a fresh world gets notes, not
//! pushes (and how many bytes that saves over `all`); an edit to a column a
//! joiner already generated arrives as a change and lands on its own
//! generation, no hole and no re-push; a pushed column replaces the joiner's
//! whole column; a joiner with another generator gets everything; the
//! verdict budget is never exceeded and never starves a far column; and a
//! local column let go of is decided afresh on return.

use super::push_joiner::{assert_chunk_matches, columns_within, move_body, start_dedicated, Joiner};
use crate::block;
use crate::chunk::CHUNK_SIZE;
use crate::chunk_verdict::{Verdict, VerdictBudget};
use crate::hosted_server::HostedServer;
use crate::protocol::BlockChange;
use crate::world::MAX_CHUNK_Y;

/// The sim distance (and so the push limit) these tests run at.
const SIM: i32 = 3;

fn dedicated(tag: &str) -> HostedServer {
    let mut hs = start_dedicated(tag);
    hs.server.set_sim_distance(SIM);
    hs
}

/// Tick (acknowledging) until the server has decided every column within
/// the push radius of `j`'s body, then until the stream is quiet. The
/// dedicated server streams columns in a couple a tick and decides a few, so
/// a quiet tick alone does not mean it is done.
fn settle_range(hs: &mut HostedServer, j: &mut Joiner, r: i32) {
    for _ in 0..2_000 {
        let me = j.column(hs);
        if columns_within(me, r).iter().all(|&c| j.intake.decided(c)) {
            j.settle(hs);
            return;
        }
        j.ack();
        hs.tick();
        j.take_in();
    }
    panic!("the server never decided every column in range");
}

/// Every column within `r` of `centre` is decided for `j` — pushed whole or
/// noted local — and matches the server's, chunk for chunk, in its world.
fn assert_range_matches(hs: &HostedServer, j: &Joiner, centre: (i32, i32), r: i32) {
    for col in columns_within(centre, r) {
        assert!(j.intake.decided(col), "column {col:?} decided");
        assert!(j.loaded.contains(&col), "column {col:?} held");
        for cy in 0..=MAX_CHUNK_Y {
            assert_chunk_matches(&hs.server.world, &j.world, (col.0, cy, col.1));
        }
    }
}

#[test]
fn a_touched_mode_join_on_a_fresh_world_gets_notes_not_pushes() {
    // Radius 5: 121 columns, past the 64-packet credit window, so the notes
    // must be acknowledged like pushes for the rest to come.
    const R: i32 = 5;
    let mut hs = dedicated("fresh");
    hs.server.set_sim_distance(R);
    let mut j = Joiner::join(&mut hs, R as u8, false);
    settle_range(&mut hs, &mut j, R);
    let me = j.column(&hs);
    assert_range_matches(&hs, &j, me, R);
    let range = columns_within(me, R).len();
    let local = j.intake.local_columns().len();
    let pushed = range - local;
    let touched_bytes = j.stream_bytes;
    // The same world, a fresh joiner, `all`: every chunk pushed.
    hs.set_chunk_sync(crate::chunk_push::ChunkSync::All);
    let mut all = Joiner::join(&mut hs, R as u8, false);
    settle_range(&mut hs, &mut all, R);
    assert!(all.intake.local_columns().is_empty(), "`all` sends no notes");
    let all_bytes = all.stream_bytes;
    println!(
        "B2b fresh-world join, R = {R} ({range} columns): touched mode {local} notes + {pushed} pushed columns \
         = {touched_bytes} bytes; all mode {all_bytes} bytes ({:.1}%)",
        100.0 * touched_bytes as f64 / all_bytes as f64
    );
    assert!(local * 10 >= range * 8, "a fresh world is mostly local: {local}/{range}");
    assert!(touched_bytes * 5 < all_bytes, "{touched_bytes} vs {all_bytes} bytes");
    assert_eq!(j.generated.len(), local, "it generated exactly the columns it was told are local");
}

#[test]
fn an_edit_to_a_column_a_joiner_generated_arrives_as_a_change_on_its_own_copy() {
    let mut hs = dedicated("edit-local");
    let mut j = Joiner::join(&mut hs, SIM as u8, false);
    // The streamer does not get to it first: the change finds the column
    // noted but not generated, and must generate it before applying.
    j.generate_on_note = false;
    settle_range(&mut hs, &mut j, SIM);
    let me = j.column(&hs);
    let col = (me.0 + 2, me.1 - 1);
    assert!(j.intake.is_local(col), "untouched: noted local");
    assert!(!j.loaded.contains(&col), "not generated yet");
    let pushed_before = hs.chunk_push_for_test(j.slot).pushed();
    let cs = CHUNK_SIZE as i32;
    let cell = (col.0 * cs + 5, 2, col.1 * cs + 9); // deep: inside terrain
    hs.server.world.set_block(cell.0, cell.1, cell.2, block::GLASS);
    hs.server.pending_block_changes.push(BlockChange::with_meta(cell.0, cell.1, cell.2, block::GLASS, 0));
    j.ack();
    hs.tick();
    j.take_in();
    assert_eq!(
        hs.verdicts_for_test().get(col),
        Some(Verdict::Touched),
        "the edit touched the column for good"
    );
    assert_eq!(j.world.get_block(cell.0, cell.1, cell.2), block::GLASS, "the change landed");
    assert!(j.loaded.contains(&col), "on the column, generated first");
    for cy in 0..=MAX_CHUNK_Y {
        assert_chunk_matches(&hs.server.world, &j.world, (col.0, cy, col.1));
    }
    assert_eq!(j.world.get_block(col.0 * cs + 8, 0, col.1 * cs + 8), block::BEDROCK, "no hole");
    settle_range(&mut hs, &mut j, SIM);
    assert_eq!(
        hs.chunk_push_for_test(j.slot).pushed(),
        pushed_before,
        "a column already local is never pushed again for an edit"
    );
    // The rest of what it was told is local, generated late, matches too.
    j.generate_noted();
    assert_range_matches(&hs, &j, me, SIM);
}

/// Measurement (Phase B2b): how many columns a fresh dedicated world's own
/// simulation touches round a joiner (water, leaf decay, falling blocks …)
/// over 400 ticks, against the verdicts' false-touched rate on pure
/// generation (`chunk_verdict::tests::measure_verdicts`).
#[test]
#[ignore = "measurement"]
fn measure_touched_after_play() {
    for seed in [42u32, 1, 1234] {
        let mut hs = HostedServer::start(
            0,
            format!("touched-measure-{seed}-{}", std::process::id()),
            seed,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        )
        .expect("starts");
        hs.server.set_sim_distance(6);
        let mut j = Joiner::join(&mut hs, 6, false);
        settle_range(&mut hs, &mut j, 6);
        let at_join = (hs.verdicts_for_test().touched(), hs.verdicts_for_test().len());
        j.play(&mut hs, 400, 8);
        let later = (hs.verdicts_for_test().touched(), hs.verdicts_for_test().len());
        println!(
            "seed {seed}: touched at join {}/{} columns; after 400 ticks {}/{} (stream {} bytes)",
            at_join.0, at_join.1, later.0, later.1, j.stream_bytes
        );
    }
}

#[test]
fn a_pushed_column_replaces_the_joiners_whole_column() {
    let mut hs = dedicated("whole");
    // The server clears the top of a column the joiner will hold its own copy
    // of: every block of its highest chunk (trees, the surface) is gone.
    let spawn = crate::chunk_stream::column_of(hs.server.world_spawn());
    let col = (spawn.0 + 2, spawn.1);
    hs.server.ensure_column_loaded(col.0, col.1);
    let top = (0..=MAX_CHUNK_Y)
        .rev()
        .find(|&cy| hs.server.world.get_chunk(col.0, cy, col.1).is_some_and(|c| !c.is_empty()))
        .expect("terrain");
    let cs = CHUNK_SIZE as i32;
    for x in col.0 * cs..(col.0 + 1) * cs {
        for y in top * cs..(top + 1) * cs {
            for z in col.1 * cs..(col.1 + 1) * cs {
                hs.server.world.set_block(x, y, z, block::AIR);
            }
        }
    }
    let mut j = Joiner::join(&mut hs, SIM as u8, false);
    // The joiner generated that column itself (it was outside its push
    // radius, say) before the server got to it.
    j.generate(col);
    assert!(j.world.get_chunk(col.0, top, col.1).is_some_and(|c| !c.is_empty()), "its own copy has blocks there");
    settle_range(&mut hs, &mut j, SIM);
    assert!(j.intake.column_complete(col), "the touched column was pushed");
    for cy in 0..=MAX_CHUNK_Y {
        assert_chunk_matches(&hs.server.world, &j.world, (col.0, cy, col.1));
    }
    assert!(j.world.get_chunk(col.0, top, col.1).is_none_or(|c| c.is_empty()), "the local top chunk is gone");
}

#[test]
fn a_joiner_with_another_generator_gets_every_chunk_even_in_touched_mode() {
    let mut hs = dedicated("mismatch");
    let mut j = Joiner::join(&mut hs, SIM as u8, true);
    assert!(hs.server.players[j.slot].worldgen_mismatch());
    settle_range(&mut hs, &mut j, SIM);
    let me = j.column(&hs);
    for col in columns_within(me, SIM) {
        assert!(j.intake.column_complete(col), "column {col:?} pushed");
        for cy in 0..=MAX_CHUNK_Y {
            assert_chunk_matches(&hs.server.world, &j.world, (col.0, cy, col.1));
        }
    }
    assert!(j.intake.local_columns().is_empty(), "no notes");
    assert!(j.generated.is_empty(), "it generates nothing");
    assert!(!j.intake.server_decides(), "its JoinAccept says no notes (radius 0)");
}

#[test]
fn the_verdict_budget_is_never_exceeded_and_a_far_column_still_decides() {
    let mut hs = dedicated("budget");
    let budget = VerdictBudget { count: 2, time: std::time::Duration::MAX };
    hs.set_verdict_budget_for_test(budget);
    let mut a = Joiner::join(&mut hs, SIM as u8, false);
    // A second joiner far off: the budget is shared, and both get there.
    let mut b = Joiner::join(&mut hs, SIM as u8, false);
    move_body(&mut hs, b.slot, 12);
    let mut ticks = 0;
    let far_a = |hs: &HostedServer, a: &Joiner| {
        let me = a.column(hs);
        (me.0 + SIM, me.1 + SIM)
    };
    while hs.verdicts_for_test().get(far_a(&hs, &a)).is_none() || !b.intake.decided(b.column(&hs)) {
        a.ack();
        b.ack();
        hs.tick();
        a.take_in();
        b.take_in();
        assert!(
            hs.verdicts_last_tick_for_test() <= budget.count,
            "tick {ticks}: {} verdicts",
            hs.verdicts_last_tick_for_test()
        );
        ticks += 1;
        assert!(ticks < 2_000, "the far corner never decided");
    }
    settle_range(&mut hs, &mut a, SIM);
    settle_range(&mut hs, &mut b, SIM);
    let (ma, mb) = (a.column(&hs), b.column(&hs));
    assert_range_matches(&hs, &a, ma, SIM);
    assert_range_matches(&hs, &b, mb, SIM);
}

#[test]
fn a_local_column_let_go_of_is_decided_afresh_on_return() {
    let mut hs = dedicated("drop-local");
    let mut j = Joiner::join(&mut hs, SIM as u8, false);
    settle_range(&mut hs, &mut j, SIM);
    let me = j.column(&hs);
    let col = (me.0 + SIM, me.1);
    assert!(j.intake.is_local(col));
    j.let_go(col);
    let cs = CHUNK_SIZE as i32;
    let cell = (col.0 * cs + 3, 93, col.1 * cs + 3);
    j.settle(&mut hs);
    assert!(!hs.chunk_push_for_test(j.slot).has_sent((col.0, 0, col.1)), "the drop was taken");
    assert!(!j.intake.decided(col), "held off: not decided again in place");
    // While it is gone the server changes it, with no broadcast: only a push
    // decided afresh can carry it.
    hs.server.world.set_block(cell.0, cell.1, cell.2, block::GLASS);
    move_body(&mut hs, j.slot, -2);
    settle_range(&mut hs, &mut j, SIM);
    move_body(&mut hs, j.slot, 2);
    settle_range(&mut hs, &mut j, SIM);
    assert!(j.intake.column_complete(col), "touched since: pushed on return");
    assert_eq!(j.world.get_block(cell.0, cell.1, cell.2), block::GLASS, "with the change");
}

#[test]
fn a_dedicated_server_tells_a_matching_joiner_its_note_radius() {
    let mut hs = dedicated("radius");
    let j = Joiner::join(&mut hs, SIM as u8, false);
    assert!(j.intake.server_decides(), "touched mode: the server decides round the body");
    let mut hs = dedicated("radius-all");
    hs.set_chunk_sync(crate::chunk_push::ChunkSync::All);
    let j = Joiner::join(&mut hs, SIM as u8, false);
    assert!(!j.intake.server_decides(), "`all`: everything is pushed, no notes");
}
