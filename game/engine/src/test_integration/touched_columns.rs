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
//! pushes (and how many bytes that saves over `all`); a joiner that
//! generated its range before any verdict keeps it, the notes confirming it
//! (B2b fix D1); an edit to a column a joiner already generated arrives as a
//! change and lands on its own generation, no hole and no re-push; a lone
//! pushed chunk for a noted column not generated yet lands on the generated
//! column (fix D3); a pushed column replaces the joiner's whole column; a
//! joiner with another generator gets everything; a column whose generation
//! does not hash as its note said switches the joiner to everything pushed
//! (fix D4); the verdict budget is never exceeded and never starves a far
//! column; and a local column let go of is decided afresh on return.

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
    assert_eq!(j.intake.column_mismatch(), None, "every local column hashed as its note said");
    assert!(!hs.chunk_push_for_test(j.slot).pushes_everything());
}

#[test]
fn a_joiner_that_generated_its_range_before_any_verdict_keeps_it_and_the_notes_confirm_it() {
    // B2b fix D1: the joiner generates inside R as a single-player client
    // would — no void moat while the verdicts come — and a note only
    // confirms a column it already holds.
    let mut hs = dedicated("speculative");
    let mut j = Joiner::join(&mut hs, SIM as u8, false);
    let me = j.column(&hs);
    j.generate_around(me, SIM + 1);
    let speculative = j.generated.len();
    assert_eq!(speculative, columns_within(me, SIM + 1).len(), "the whole range, before any verdict");
    settle_range(&mut hs, &mut j, SIM);
    assert_eq!(j.generated.len(), speculative, "nothing generated twice");
    assert_eq!(j.notes_received, columns_within(me, SIM).len(), "every column noted local");
    assert_eq!(j.intake.column_mismatch(), None, "matching hashes: no switch");
    assert!(!hs.chunk_push_for_test(j.slot).pushes_everything());
    assert_range_matches(&hs, &j, me, SIM);
    // A column it generated outside R was never decided: letting it go is
    // none of the server's business.
    let outside = (me.0 + SIM + 1, me.1);
    assert!(!j.intake.decided(outside));
    j.let_go(outside);
    assert_eq!(j.intake.pending_drops(), 0, "never reported");
}

#[test]
fn a_lone_pushed_chunk_for_a_noted_column_not_generated_yet_lands_on_the_generated_column() {
    // B2b fix D3 (review MEDIUM-1): an overflow resync pushes one chunk of a
    // column noted local with no change before it. Applied alone it left the
    // column part-pushed and never generated, every later change conjuring a
    // stray chunk.
    let mut hs = dedicated("lone-push");
    let mut j = Joiner::join_without_generating(&mut hs, SIM as u8);
    settle_range(&mut hs, &mut j, SIM);
    let me = j.column(&hs);
    let col = (me.0 - 2, me.1 + 1);
    assert!(j.intake.is_local(col) && !j.loaded.contains(&col), "noted, not generated");
    // The server edits chunk cy 2 (no broadcast) and pushes it again.
    let cs = CHUNK_SIZE as i32;
    let cell = (col.0 * cs + 7, 2 * cs + 3, col.1 * cs + 7);
    hs.server.world.set_block(cell.0, cell.1, cell.2, block::GLASS);
    hs.resync_for_test(j.slot, (col.0, 2, col.1));
    j.ack();
    hs.tick();
    j.take_in();
    assert!(j.intake.holds_chunk((col.0, 2, col.1)), "the lone chunk came");
    assert!(j.loaded.contains(&col), "generated first");
    assert_eq!(j.world.get_block(cell.0, cell.1, cell.2), block::GLASS, "with the push on top");
    for cy in 0..=MAX_CHUNK_Y {
        assert_chunk_matches(&hs.server.world, &j.world, (col.0, cy, col.1));
    }
    assert_eq!(j.intake.column_mismatch(), None, "checked as generated, before the push landed");
    // A later change to another chunk of it writes into a real chunk.
    let deep = (col.0 * cs + 2, 3, col.1 * cs + 12);
    hs.server.world.set_block(deep.0, deep.1, deep.2, block::GLASS);
    hs.server.pending_block_changes.push(BlockChange::with_meta(deep.0, deep.1, deep.2, block::GLASS, 0));
    j.ack();
    hs.tick();
    j.take_in();
    assert_eq!(j.world.get_block(deep.0, deep.1, deep.2), block::GLASS);
    assert_chunk_matches(&hs.server.world, &j.world, (col.0, 0, col.1));
    assert_eq!(j.world.get_block(col.0 * cs + 8, 0, col.1 * cs + 8), block::BEDROCK, "no conjured chunk");
}

#[test]
fn a_forged_column_hash_switches_the_joiner_to_every_column_pushed() {
    // B2b fix D4: a joiner whose generation of a noted column does not hash
    // as the note said lets it go, reports it and asks for everything; the
    // server pushes again every column it had noted and notes no more.
    let mut hs = dedicated("forged");
    let mut j = Joiner::join(&mut hs, SIM as u8, false);
    // The spawn ring went out with the join, with true hashes; every note
    // from now on carries a wrong one.
    hs.forge_note_hashes_for_test(j.slot);
    for _ in 0..200 {
        if j.intake.column_mismatch().is_some() {
            break;
        }
        j.ack();
        hs.tick();
        j.take_in();
    }
    let m = j.intake.column_mismatch().expect("a forged note was caught");
    let bad = (m.cx, m.cz);
    assert_ne!(m.server_hash, m.client_hash);
    assert!(!j.intake.is_local(bad) && !j.loaded.contains(&bad), "the column was let go of");
    let noted_before: Vec<(i32, i32)> = j.intake.local_columns();
    assert!(!noted_before.is_empty(), "the ring at least was noted and kept");
    // The next input carries the switch (and the drop).
    j.ack();
    hs.tick();
    assert!(hs.chunk_push_for_test(j.slot).pushes_everything(), "the server switched");
    assert_eq!(hs.chunk_push_for_test(j.slot).noted_len(), 0);
    assert_eq!(hs.column_mismatch_warnings_for_test(), 1, "warned once");
    j.take_in();
    let notes_at_switch = j.notes_received;
    settle_range(&mut hs, &mut j, SIM);
    let me = j.column(&hs);
    for col in columns_within(me, SIM) {
        assert!(j.intake.column_complete(col), "column {col:?} pushed whole");
        for cy in 0..=MAX_CHUNK_Y {
            assert_chunk_matches(&hs.server.world, &j.world, (col.0, cy, col.1));
        }
    }
    for col in &noted_before {
        assert!(j.intake.column_complete(*col), "noted column {col:?} pushed again");
    }
    assert!(j.intake.column_complete(bad), "the mismatched one too");
    // Notes no more: new ground is pushed.
    move_body(&mut hs, j.slot, 3);
    settle_range(&mut hs, &mut j, SIM);
    assert_eq!(j.notes_received, notes_at_switch, "no note after the switch took effect");
    let me = j.column(&hs);
    assert!(columns_within(me, SIM).iter().all(|&c| j.intake.column_complete(c)));
    assert!(j.input().column_mismatch.is_some(), "the switch rides every input");
    assert_eq!(hs.column_mismatch_warnings_for_test(), 1, "and the server warned once a session");
}

#[test]
fn a_column_the_joiner_wrote_to_before_its_note_came_is_kept_without_a_switch() {
    // B2b fix HIGH-1: the joiner's own writes to a column it generated before
    // the column's note (its snowfall, its fluids, its player's edit) are no
    // determinism bug. The note's hash matches a scratch generation, so the
    // column is checked and kept as it stands: no switch, no warning, no
    // report, no regeneration.
    let mut hs = dedicated("drift");
    // One verdict a tick: the join decides no farther than the spawn ring.
    let budget = hs.verdict_budget_for_test();
    hs.set_verdict_budget_for_test(VerdictBudget { count: 1, ..budget });
    let mut j = Joiner::join_without_generating(&mut hs, SIM as u8);
    let me = j.column(&hs);
    let col = (me.0 + SIM, me.1 - 1);
    assert!(!j.intake.decided(col), "not decided yet");
    j.generate(col);
    let cs = CHUNK_SIZE as i32;
    let cell = (col.0 * cs + 4, 3, col.1 * cs + 11); // deep: inside terrain
    assert_ne!(j.world.get_block(cell.0, cell.1, cell.2), block::GLASS);
    j.world.set_block(cell.0, cell.1, cell.2, block::GLASS); // this client's own write
    hs.set_verdict_budget_for_test(budget);
    settle_range(&mut hs, &mut j, SIM);
    assert!(j.intake.is_local(col), "noted local");
    assert!(!j.intake.has_pending_check(col), "and checked");
    assert!(j.loaded.contains(&col), "kept");
    assert_eq!(j.world.get_block(cell.0, cell.1, cell.2), block::GLASS, "as it stands");
    assert_eq!(j.generated.iter().filter(|&&c| c == col).count(), 1, "never generated again");
    assert_eq!(j.intake.column_mismatch(), None, "no switch");
    assert!(!hs.chunk_push_for_test(j.slot).pushes_everything());
    assert_eq!(hs.column_mismatch_warnings_for_test(), 0, "no warning");
    assert_eq!(j.intake.pending_drops(), 0, "nothing reported");
}

#[test]
fn a_mismatch_report_from_a_joiner_never_noted_changes_nothing() {
    // B2b fix LOW-4: an honest client reports a mismatch only from a note. One
    // that was never sent a note (here: `all` mode) and reports one anyway
    // changes nothing and is not logged.
    let mut hs = dedicated("never-noted");
    hs.set_chunk_sync(crate::chunk_push::ChunkSync::All);
    let mut j = Joiner::join(&mut hs, SIM as u8, false);
    j.settle(&mut hs);
    assert!(!hs.chunk_push_for_test(j.slot).noted_ever());
    let mut input = j.input();
    input.column_mismatch =
        Some(crate::protocol::ColumnMismatch { cx: 0, cz: 0, server_hash: 1, client_hash: 2 });
    j.rc.send_input(&input).expect("connected");
    hs.tick();
    assert!(!hs.chunk_push_for_test(j.slot).pushes_everything(), "no mode change");
    assert_eq!(hs.column_mismatch_warnings_for_test(), 0, "nothing logged");
}

#[test]
fn an_edit_to_a_column_a_joiner_generated_arrives_as_a_change_on_its_own_copy() {
    let mut hs = dedicated("edit-local");
    // The streamer does not get to it first: the change finds the column
    // noted but not generated, and must generate it before applying.
    let mut j = Joiner::join_without_generating(&mut hs, SIM as u8);
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
    // The joiner generates that column itself (B2b fix D1: as its streamer
    // does, before any verdict) before the server gets to it: one verdict a
    // tick until then, so the join decides no farther than the spawn ring.
    let budget = hs.verdict_budget_for_test();
    hs.set_verdict_budget_for_test(VerdictBudget { count: 1, ..budget });
    let mut j = Joiner::join(&mut hs, SIM as u8, false);
    assert!(!j.intake.decided(col), "not decided yet");
    j.generate(col);
    assert!(j.world.get_chunk(col.0, top, col.1).is_some_and(|c| !c.is_empty()), "its own copy has blocks there");
    hs.set_verdict_budget_for_test(budget);
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
fn a_lending_host_notes_untouched_columns_and_its_own_edits_touch_them() {
    // The LAN / online host: its server reads the host client's own world,
    // lent each tick, and the host's edits land there BETWEEN the windows.
    let (mut hs, mut host) = super::lent_world::start_lent("touched");
    let mut j = Joiner::join_with(&mut hs, 2, false, |hs| host.lend_tick(hs));
    assert!(j.intake.server_decides(), "a lending host sends notes");
    let me = j.column(&hs);
    for _ in 0..500 {
        if columns_within(me, 2).iter().all(|&c| j.intake.decided(c)) {
            break;
        }
        j.ack();
        host.lend_tick(&mut hs);
        j.take_in();
    }
    j.settle_with(&mut hs, |hs| host.lend_tick(hs));
    for col in columns_within(me, 2) {
        assert!(j.intake.is_local(col), "column {col:?}: untouched, noted local");
        for cy in 0..=MAX_CHUNK_Y {
            assert_chunk_matches(&host.world, &j.world, (col.0, cy, col.1));
        }
    }
    // The host builds in its own world between two windows — no broadcast,
    // only the edit tracking on the lent world can see it.
    let col = (me.0 + 1, me.1 + 1);
    let pushed = hs.chunk_push_for_test(j.slot).pushed();
    let cs = CHUNK_SIZE as i32;
    host.world.set_block(col.0 * cs + 4, 90, col.1 * cs + 4, block::GLASS);
    host.lend_tick(&mut hs);
    assert_eq!(hs.verdicts_for_test().get(col), Some(Verdict::Touched), "the lent world tracked the edit");
    j.settle_with(&mut hs, |hs| host.lend_tick(hs));
    assert_eq!(hs.chunk_push_for_test(j.slot).pushed(), pushed, "already local: never pushed for it");
    // A joiner arriving now is pushed that column, with the edit in it.
    let mut late = Joiner::join_with(&mut hs, 2, false, |hs| host.lend_tick(hs));
    for _ in 0..500 {
        if late.intake.decided(col) {
            break;
        }
        late.ack();
        host.lend_tick(&mut hs);
        late.take_in();
    }
    assert!(late.intake.column_complete(col), "touched: pushed to a late joiner");
    assert_eq!(late.world.get_block(col.0 * cs + 4, 90, col.1 * cs + 4), block::GLASS);
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
