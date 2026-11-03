//! How lava's state lies in stored records, in forms a drain doesn't rewrite (Tier-L
//! rulings, section 4 item 3).
//!
//! **The queues,** `q` (spread) and `r` (retract), are in numbered pieces of a fixed
//! [`QUEUE_PIECE_ENTRIES`] entries. An entry's place in the queue gives its piece and its
//! place in it, so a piece is never rewritten once full, and an append only touches the
//! last. The image holds each queue's cursor, the count of entries taken from the front.
//! Taking an entry moves only that: the piece keeps what lies behind the cursor, and a
//! piece wholly behind it is no longer bound. When a queue empties, its cursor goes back to
//! 0 and it has no piece, so numbering starts again from 0 at the next push.
//!
//! So a tick rewrites at most the last piece and adds what it appended; the rest of a long
//! queue's bytes stay as they were, which also lets a first commit of one spread over staged
//! batches. The price is bytes behind the cursor (at most a piece less one entry, in the
//! store and in memory), and the cursor makes the bytes follow the history: see below.
//!
//! **Sources,** `x` + a section, are the difference from a rescan, by the 16-cube section the
//! cell lies in. A function of the set alone, so one flowing cell rewrites its section's
//! record and no other. A section holding none has no record. A section's record is at
//! most 4,096 cells, so a change costs at most 49,160 B a section.
//!
//! **What "same history" means.** The cursor counts the entries taken since the queue last
//! stood empty (or a load made it). Two runs write the same pieces when they have made the
//! same pushes and pops since then. Two runs of one saved world, fed the same, do so. A restore
//! carries the cursor and the entries behind it, so a restored run goes on writing the bytes
//! the uninterrupted run writes. Two runs that reach the same queue by other histories can
//! differ, in the cursor and the bytes behind it, until it next stands empty: they play alike,
//! and their checkpoints and state hashes differ. The rulings ask that two authorities of one
//! saved world agree, and no lava queue is in `world.dat`, so each load starts from what it wakes.

use std::collections::BTreeMap;

use super::codec::{Malformed, Reader, Writer};
use super::records::{Stored, LAVA_SOURCES};
use super::volatile::{read_sorted_list, write_list};
use crate::fluids::{PositionedQueue, QUEUE_PIECE_ENTRIES};

type Pos = (i32, i32, i32);

/// A queue's pieces: the piece's number, then its bytes.
pub(crate) type NumberedPieces = Vec<(u32, Vec<u8>)>;

pub(super) const SPREAD_ENTRY_BYTES: usize = 13;
pub(super) const RETRACT_ENTRY_BYTES: usize = 12;
/// A source section's key: the family, then the section as three big-endian `i32`s.
const SOURCE_KEY_BYTES: usize = 13;

const _: () = assert!(QUEUE_PIECE_ENTRIES as usize * SPREAD_ENTRY_BYTES <= super::records::MAX_RECORD_BYTES);
// A section's 4,096 cells, each in one of its two lists, and their counts.
const _: () = assert!(4096 * RETRACT_ENTRY_BYTES + 8 <= super::records::MAX_RECORD_BYTES);

const SECTION: i32 = crate::chunk::CHUNK_SIZE as i32;

fn section_of((x, y, z): Pos) -> Pos {
    (x.div_euclid(SECTION), y.div_euclid(SECTION), z.div_euclid(SECTION))
}

fn queue_pieces<T: Copy>(queue: &PositionedQueue<T>, write: impl Fn(&mut Writer, T)) -> Result<NumberedPieces, String> {
    let n = QUEUE_PIECE_ENTRIES as usize;
    let first = queue.head() / QUEUE_PIECE_ENTRIES;
    let mut pieces = Vec::new();
    let mut piece = Writer::uncapped();
    let mut number = first;
    let mut held = 0usize;
    for entry in queue.drained().iter().copied().chain(queue.iter().copied()) {
        write(&mut piece, entry);
        held += 1;
        if held == n {
            pieces.push((numbered(number)?, std::mem::replace(&mut piece, Writer::uncapped()).into_bytes()));
            number += 1;
            held = 0;
        }
    }
    if held > 0 {
        pieces.push((numbered(number)?, piece.into_bytes()));
    }
    Ok(pieces)
}

/// Past thousands of years of the fastest drain there is, and nothing is cut to go on past it.
fn numbered(piece: u64) -> Result<u32, String> {
    u32::try_from(piece).map_err(|_| "a lava queue past the last piece number".to_string())
}

pub(super) fn spread_pieces(queue: &PositionedQueue<(i32, i32, i32, u8)>) -> Result<NumberedPieces, String> {
    queue_pieces(queue, |w, (x, y, z, distance)| {
        w.pos((x, y, z));
        w.u8(distance);
    })
}

pub(super) fn retract_pieces(queue: &PositionedQueue<Pos>) -> Result<NumberedPieces, String> {
    queue_pieces(queue, |w, pos| w.pos(pos))
}

/// A queue's numbered pieces (already known to be contiguous), and its cursor from the
/// image: what lies behind the cursor, then what is queued. Refuses what no run could
/// have written, so a state's cursor and pieces are in one form.
fn read_queue<T>(
    head: u64,
    pieces: &[(u32, Vec<u8>)],
    entry_bytes: usize,
    read: impl Fn(&mut Reader<'_>) -> Result<T, Malformed>,
) -> Result<(Vec<T>, Vec<T>), Malformed> {
    let n = QUEUE_PIECE_ENTRIES;
    let Some((first, _)) = pieces.first() else {
        return if head == 0 { Ok((Vec::new(), Vec::new())) } else { Err(Malformed("a lava cursor in a piece that isn't bound")) };
    };
    let start = u64::from(*first) * n;
    let mut entries = Vec::new();
    for (i, (_, bytes)) in pieces.iter().enumerate() {
        let count = bytes.len() / entry_bytes;
        let whole = bytes.len().is_multiple_of(entry_bytes) && count >= 1 && count as u64 <= n && (i + 1 == pieces.len() || count as u64 == n);
        if !whole {
            return Err(Malformed("a lava piece that isn't whole"));
        }
        let mut r = Reader::new(bytes);
        for _ in 0..count {
            entries.push(read(&mut r)?);
        }
    }
    if head >= start + entries.len() as u64 {
        return Err(Malformed("a lava cursor past the end of its queue"));
    }
    if head < start {
        return Err(Malformed("a lava cursor in a piece that isn't bound"));
    }
    if head / n != u64::from(*first) {
        return Err(Malformed("a lava piece wholly behind its cursor"));
    }
    let live = entries.split_off((head - start) as usize);
    Ok((entries, live))
}

pub(super) fn read_spread(head: u64, pieces: &[(u32, Vec<u8>)]) -> Result<(Vec<(i32, i32, i32, u8)>, Vec<(i32, i32, i32, u8)>), Malformed> {
    read_queue(head, pieces, SPREAD_ENTRY_BYTES, |r| {
        let (x, y, z) = r.pos()?;
        Ok((x, y, z, r.u8()?))
    })
}

pub(super) fn read_retract(head: u64, pieces: &[(u32, Vec<u8>)]) -> Result<(Vec<Pos>, Vec<Pos>), Malformed> {
    read_queue(head, pieces, RETRACT_ENTRY_BYTES, |r| r.pos())
}

fn source_key((cx, cy, cz): Pos) -> Vec<u8> {
    let mut key = Vec::with_capacity(SOURCE_KEY_BYTES);
    key.push(LAVA_SOURCES);
    for v in [cx, cy, cz] {
        key.extend_from_slice(&v.to_be_bytes());
    }
    key
}

/// Is this key a section of lava's sources?
pub(super) fn is_source_key(key: &[u8]) -> bool {
    key.len() == SOURCE_KEY_BYTES && key[0] == LAVA_SOURCES
}

/// The section a source key names, for a refusal or a reader.
pub(super) fn source_section(key: &[u8]) -> Pos {
    let at = |i: usize| i32::from_be_bytes([key[i], key[i + 1], key[i + 2], key[i + 3]]);
    (at(1), at(5), at(9))
}

/// Sorted `extra` and `missing` as one record per section that holds any.
pub(super) fn source_sections(extra: &[Pos], missing: &[Pos]) -> Vec<Stored> {
    let mut sections: BTreeMap<Pos, (Vec<Pos>, Vec<Pos>)> = BTreeMap::new();
    for &cell in extra {
        sections.entry(section_of(cell)).or_default().0.push(cell);
    }
    for &cell in missing {
        sections.entry(section_of(cell)).or_default().1.push(cell);
    }
    sections
        .into_iter()
        .map(|(section, (extra, missing))| {
            let mut w = Writer::uncapped();
            write_list(&mut w, &extra);
            write_list(&mut w, &missing);
            (source_key(section), w.into_bytes())
        })
        .collect()
}

/// Every section's lists back as the two sorted lists, each section checked for one form.
pub(super) fn read_source_sections(sections: &[Stored]) -> Result<(Vec<Pos>, Vec<Pos>), Malformed> {
    let (mut extra, mut missing) = (Vec::new(), Vec::new());
    for (key, bytes) in sections {
        let section = source_section(key);
        let mut r = Reader::new(bytes);
        let (these_extra, these_missing) = (read_sorted_list(&mut r)?, read_sorted_list(&mut r)?);
        if !r.is_empty() {
            return Err(Malformed("bytes left over in a section of lava's sources"));
        }
        if these_extra.is_empty() && these_missing.is_empty() {
            return Err(Malformed("a section of lava's sources that holds none"));
        }
        if !these_extra.iter().chain(&these_missing).all(|&cell| section_of(cell) == section) {
            return Err(Malformed("a lava source in another section's record"));
        }
        extra.extend(these_extra);
        missing.extend(these_missing);
    }
    extra.sort_unstable();
    missing.sort_unstable();
    Ok((extra, missing))
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: u64 = QUEUE_PIECE_ENTRIES;

    fn queue(pushed: u64, popped: u64) -> PositionedQueue<Pos> {
        let mut q = PositionedQueue::default();
        for i in 0..pushed {
            q.push_back((i as i32, 1, 2));
        }
        for _ in 0..popped {
            q.pop_front();
        }
        q
    }

    fn read_back(head: u64, pieces: &[(u32, Vec<u8>)]) -> Result<(Vec<Pos>, Vec<Pos>), Malformed> {
        read_retract(head, pieces)
    }

    fn why<T>(read: Result<T, Malformed>) -> &'static str {
        read.err().expect("refused").0
    }

    /// The numbers of the schema: a change is a new schema.
    #[test]
    fn a_pieces_size_is_the_schemas_number() {
        assert_eq!(N, 1024);
        assert_eq!(N as usize * SPREAD_ENTRY_BYTES, 13_312);
    }

    #[test]
    fn a_piece_full_or_behind_the_cursor_stays_as_it_was_and_only_the_last_changes() {
        let mut q = queue(N * 3 + 10, 5);
        let before = retract_pieces(&q).unwrap();
        assert_eq!(before.iter().map(|p| p.0).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        q.pop_front();
        assert_eq!(retract_pieces(&q).unwrap(), before, "a drain changes no piece");
        q.push_back((9, 9, 9));
        let after = retract_pieces(&q).unwrap();
        assert_eq!(after[..3], before[..3], "an append changes the last piece only");
        assert_ne!(after[3], before[3]);
    }

    #[test]
    fn a_piece_wholly_behind_the_cursor_is_gone() {
        let q = queue(N * 2 + 3, N);
        assert_eq!(retract_pieces(&q).unwrap().iter().map(|p| p.0).collect::<Vec<_>>(), vec![1, 2]);
        assert!(retract_pieces(&queue(N * 2 + 3, N * 2 + 3)).unwrap().is_empty(), "an empty queue has no piece, wherever it stopped");
        assert!(retract_pieces(&queue(N * 2, N * 2)).unwrap().is_empty());
        assert!(retract_pieces(&queue(0, 0)).unwrap().is_empty());
    }

    /// The pieces and the cursor come back as the queue they were: each case's next pushes
    /// and pops make the same pieces as the original's.
    #[test]
    fn a_queue_read_back_goes_on_making_the_same_pieces() {
        for (pushed, popped) in [(0, 0), (1, 0), (1, 1), (N, 0), (N, N), (N + 1, N), (N + 1, N + 1), (N * 3 + 7, N + 20), (N * 3, N * 2 - 1), (N, N - 1)] {
            let mut q = queue(pushed, popped);
            let pieces = retract_pieces(&q).unwrap();
            let (drained, live) = read_back(q.head(), &pieces).unwrap_or_else(|e| panic!("{pushed}, {popped}: {}", e.0));
            assert_eq!(drained, q.drained(), "{pushed}, {popped}");
            assert_eq!(live, q.iter().copied().collect::<Vec<_>>(), "{pushed}, {popped}");
            let mut back = PositionedQueue::restored(q.head(), drained, live);
            for i in 0..(N * 2 + 3) as i32 {
                q.push_back((i, 3, 3));
                back.push_back((i, 3, 3));
                if i % 2 == 0 {
                    q.pop_front();
                    back.pop_front();
                }
                assert_eq!(q.head(), back.head());
            }
            assert_eq!(retract_pieces(&q).unwrap(), retract_pieces(&back).unwrap(), "{pushed}, {popped}");
        }
    }

    /// Same queue, other histories: this is the whole of what a different history changes,
    /// and it lasts until the queue next stands empty.
    #[test]
    fn two_histories_of_one_queue_can_differ_until_it_empties() {
        let mut a = PositionedQueue::default();
        a.push_back((1, 1, 2));
        let mut b = queue(2, 1);
        assert_eq!(a.iter().collect::<Vec<_>>(), b.iter().collect::<Vec<_>>());
        assert_ne!(a.head(), b.head());
        assert_ne!(retract_pieces(&a).unwrap(), retract_pieces(&b).unwrap());
        a.pop_front();
        b.pop_front();
        assert_eq!((a.head(), b.head()), (0, 0));
        assert_eq!(retract_pieces(&a).unwrap(), retract_pieces(&b).unwrap(), "both empty, both alike");
    }

    #[test]
    fn a_cursor_or_pieces_no_run_writes_are_refused() {
        let q = queue(N * 2 + 3, N + 1);
        let pieces = retract_pieces(&q).unwrap();
        let head = q.head();
        assert_eq!(why(read_back(N * 2 + 4, &pieces)), "a lava cursor past the end of its queue");
        assert_eq!(why(read_back(N * 2 + 3, &pieces)), "a lava cursor past the end of its queue");
        assert_eq!(why(read_back(N - 1, &pieces)), "a lava cursor in a piece that isn't bound");
        assert_eq!(why(read_back(N * 2, &pieces)), "a lava piece wholly behind its cursor");
        assert_eq!(why(read_back(N + 7, &[])), "a lava cursor in a piece that isn't bound");
        let mut short = pieces.clone();
        short[0].1.truncate(RETRACT_ENTRY_BYTES * 5);
        assert_eq!(why(read_back(head, &short)), "a lava piece that isn't whole");
        let mut torn = pieces.clone();
        torn[1].1.pop();
        assert_eq!(why(read_back(head, &torn)), "a lava piece that isn't whole");
        let mut empty = pieces.clone();
        empty[1].1.clear();
        assert_eq!(why(read_back(head, &empty)), "a lava piece that isn't whole");
        let mut over = pieces;
        over[1].1.extend(vec![0; RETRACT_ENTRY_BYTES * N as usize]);
        assert_eq!(why(read_back(head, &over)), "a lava piece that isn't whole");
    }

    #[test]
    fn one_cell_of_the_sources_changes_one_sections_record() {
        let cells: Vec<Pos> = (0..1000).map(|i| (i, i % 40, -i)).collect();
        let before = source_sections(&cells, &[]);
        assert!(before.len() > 10, "{} sections", before.len());
        let mut more = cells.clone();
        more.push((3, 3, 3));
        more.sort_unstable();
        let after = source_sections(&more, &[]);
        let changed = after.iter().filter(|s| !before.contains(s)).count();
        assert_eq!(changed, 1, "one cell, one record");
        assert_eq!(read_source_sections(&after).unwrap(), (more, Vec::new()));
    }

    #[test]
    fn a_full_section_fits_a_record() {
        let cells: Vec<Pos> = (0..16).flat_map(|x| (0..16).flat_map(move |y| (0..16).map(move |z| (x, y, z)))).collect();
        let mut extra = cells.clone();
        let missing = extra.split_off(2000);
        let sections = source_sections(&extra, &missing);
        assert_eq!(sections.len(), 1);
        assert!(sections[0].1.len() <= super::super::records::MAX_RECORD_BYTES, "{} B", sections[0].1.len());
        assert_eq!(read_source_sections(&sections).unwrap(), (extra, missing));
    }

    #[test]
    fn sections_no_run_writes_are_refused() {
        let good = source_sections(&[(1, 1, 1)], &[]);
        let mut elsewhere = good.clone();
        elsewhere[0].0 = source_key((0, 0, 1));
        assert_eq!(why(read_source_sections(&elsewhere)), "a lava source in another section's record");
        let mut w = Writer::uncapped();
        write_list(&mut w, &[]);
        write_list(&mut w, &[]);
        assert_eq!(why(read_source_sections(&[(source_key((0, 0, 0)), w.into_bytes())])), "a section of lava's sources that holds none");
        let mut over = good.clone();
        over[0].1.push(0);
        assert_eq!(why(read_source_sections(&over)), "bytes left over in a section of lava's sources");
    }
}
