//! Living world Tier 2 S10 — a joiner's people terms (spec §10, calls T3 and T13).
//!
//! Bloom's built-together term `b` (distinct builders in a region) and
//! lived-in term `u` (days and nights a place has been used) need tables only
//! the authority keeps. A joined client has neither, so the authority tells it,
//! for each region (chunk column) it has been pushed, two small numbers:
//!
//! * `builders`: how many distinct people built there, capped at
//!   [`crate::authorship::MAX_COUNTED_AUTHORS`];
//! * `use_u`: `u` itself (the gate of sixteen placed blocks and the fade
//!   already applied), times 255.
//!
//! Counts only. No identity, key or name ever goes on this wire (T3), and the
//! joiner learns nothing it could use to find out who. The values are computed
//! by the very functions the host's own bloom reads
//! (`World::authors_in_region`, `World::lived_in_term`); there is no second
//! copy of the maths.
//!
//! * [`PeopleSent`] is the authority's per-joiner state (what it last told that
//!   joiner, and when it may speak next). It lives beside the joiner's chunk
//!   push, is dropped with it on a disconnect, and is not saved.
//! * [`RemotePeople`] is the joiner's side: a small runtime map keyed by
//!   region, read by its bloom in place of the tables it does not have.
//!   Not saved. A column the client lets go of is forgotten from it.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::authorship::MAX_COUNTED_AUTHORS;
use crate::world::World;

/// Ticks between two sends to one joiner: ten seconds at 20 TPS.
pub const PEOPLE_EVERY_TICKS: u64 = 200;
/// Most regions in one packet. A longer change list goes over on the next beat.
pub const MAX_REGIONS_PER_PACKET: usize = 512;
/// Most regions a joiner remembers (a hostile server cannot grow the map past it).
pub const MAX_REMEMBERED: usize = 16_384;

/// One region's two numbers, as sent. This is everything the packet says about
/// a region: a position and two counts. There is deliberately no field a name,
/// a key or a person could ride in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionPeople {
    pub cx: i32,
    pub cz: i32,
    /// Distinct builders, at most [`MAX_COUNTED_AUTHORS`].
    pub builders: u8,
    /// `u` x 255, rounded.
    pub use_u: u8,
}

/// The payload of `PacketType::RegionPeople`: the regions whose numbers changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionPeoplePacket {
    pub regions: Vec<RegionPeople>,
}

// ── The shared maths ─────────────────────────────────────────────────

/// The two numbers for region `(cx, cz)`, from the world's own tables: what the
/// host's bloom reads for the same region, quantised for the wire.
pub fn people_of(world: &World, cx: i32, cz: i32) -> (u8, u8) {
    let builders = world.authors_in_region(cx, cz).min(MAX_COUNTED_AUTHORS) as u8;
    (builders, quantise_u(world.lived_in_term(cx, cz)))
}

/// `u` (0 to 1) as a byte.
pub fn quantise_u(u: f32) -> u8 {
    (u.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The bloom `b` a joiner derives from `builders`: the host's formula.
pub fn built_term(builders: u8) -> f32 {
    crate::authorship::built_together(usize::from(builders))
}

/// The bloom `u` a joiner derives from `use_u`.
pub fn lived_term(use_u: u8) -> f32 {
    f32::from(use_u) / 255.0
}

// ── The authority's side ─────────────────────────────────────────────

/// What the authority last told one joiner, and when it may tell it again.
#[derive(Debug, Default)]
pub struct PeopleSent {
    next_due: u64,
    told: HashMap<(i32, i32), (u8, u8)>,
}

impl PeopleSent {
    /// Regions this joiner holds a non-zero number for. Test-only.
    #[cfg(test)]
    pub fn told_len(&self) -> usize {
        self.told.len()
    }

    /// This beat's packet for the joiner, or `None` (not yet due, nothing
    /// pushed to it, or nothing changed).
    ///
    /// `held(cx, cz)` says whether the joiner has been pushed that region; a
    /// region it has not is never mentioned, and one it let go of is forgotten
    /// (so it is told again if pushed again). Only regions that could be
    /// non-zero are looked at: those with a builder on record or a use record,
    /// and those already told (so a fall back to zero is told too).
    pub fn plan(
        &mut self,
        tick: u64,
        world: &World,
        holds_any: bool,
        held: impl Fn(i32, i32) -> bool,
    ) -> Option<RegionPeoplePacket> {
        if !holds_any {
            // Nothing pushed yet: wait without spending the beat.
            self.told.clear();
            return None;
        }
        if tick < self.next_due {
            return None;
        }
        self.next_due = tick + PEOPLE_EVERY_TICKS;
        self.told.retain(|&(cx, cz), _| held(cx, cz));

        let mut candidates: BTreeSet<(i32, i32)> = self.told.keys().copied().collect();
        for (cx, _, cz) in world.authors.chunk_positions() {
            candidates.insert((cx, cz));
        }
        candidates.extend(world.use_record.regions());

        let mut regions = Vec::new();
        for (cx, cz) in candidates {
            if regions.len() >= MAX_REGIONS_PER_PACKET {
                break;
            }
            if !held(cx, cz) {
                continue;
            }
            let now = people_of(world, cx, cz);
            if self.told.get(&(cx, cz)).copied().unwrap_or((0, 0)) == now {
                continue;
            }
            self.told.insert((cx, cz), now);
            regions.push(RegionPeople { cx, cz, builders: now.0, use_u: now.1 });
        }
        // A region back at zero is told once and then forgotten.
        self.told.retain(|_, v| *v != (0, 0));
        (!regions.is_empty()).then_some(RegionPeoplePacket { regions })
    }
}

// ── The joiner's side ────────────────────────────────────────────────

/// What a joined client has been told, by region. Read by its bloom; empty on a
/// host or in single-player (those read their own tables).
#[derive(Debug, Default)]
pub struct RemotePeople {
    map: HashMap<(i32, i32), (u8, u8)>,
    packets: u32,
}

impl RemotePeople {
    /// Take in a packet. A packet with more than [`MAX_REGIONS_PER_PACKET`]
    /// regions is not honest and is ignored whole. Returns whether it was taken.
    pub fn apply(&mut self, packet: &RegionPeoplePacket) -> bool {
        if packet.regions.len() > MAX_REGIONS_PER_PACKET {
            return false;
        }
        self.packets += 1;
        for r in &packet.regions {
            let value = (r.builders.min(MAX_COUNTED_AUTHORS as u8), r.use_u);
            if value == (0, 0) {
                self.map.remove(&(r.cx, r.cz));
            } else if self.map.len() < MAX_REMEMBERED || self.map.contains_key(&(r.cx, r.cz)) {
                self.map.insert((r.cx, r.cz), value);
            }
        }
        true
    }

    /// Packets taken so far. Test-only.
    #[cfg(test)]
    pub fn packets_taken(&self) -> u32 {
        self.packets
    }

    /// `(builders, use_u)` for a region; `(0, 0)` when never told.
    pub fn get(&self, cx: i32, cz: i32) -> (u8, u8) {
        self.map.get(&(cx, cz)).copied().unwrap_or((0, 0))
    }

    /// Regions remembered. Test-only.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Nothing remembered. Test-only.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The client has let go of column `(cx, cz)`: forget its numbers. The server
    /// forgets what it told this client for a column that leaves the push set, so
    /// it tells again if the column is pushed again; a number kept past the drop
    /// could outlive what it described. (A stale entry for a column the client does
    /// not hold is never read: bloom takes the numbers only for a region it scans.)
    pub fn forget(&mut self, cx: i32, cz: i32) {
        self.map.remove(&(cx, cz));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_form_is_two_positions_and_two_bytes_and_round_trips() {
        let p = RegionPeoplePacket {
            regions: vec![
                RegionPeople { cx: -3, cz: 9, builders: 2, use_u: 77 },
                RegionPeople { cx: 0, cz: 0, builders: 8, use_u: 255 },
            ],
        };
        let bytes = crate::protocol::serialize_packet(crate::protocol::PacketType::RegionPeople, &p);
        assert_eq!(bytes[0], 79);
        // Tag, u64 length, then 4 + 4 + 1 + 1 bytes a region: nothing else rides.
        assert_eq!(bytes.len(), 1 + 8 + 2 * 10);
        let (tag, payload) = crate::protocol::deserialize_header(&bytes).unwrap();
        assert_eq!(tag, crate::protocol::PacketType::RegionPeople);
        assert_eq!(crate::protocol::safe_deserialize::<RegionPeoplePacket>(payload).unwrap(), p);
        assert!(crate::protocol::safe_deserialize::<RegionPeoplePacket>(&payload[..payload.len() - 1]).is_err());
    }

    /// T3: nothing identifying is on the wire. The entry type is destructured
    /// exhaustively, so a new field (a name, a key) fails to compile here until
    /// it is looked at.
    #[test]
    fn an_entry_carries_a_place_and_two_counts_and_nothing_that_names_anyone() {
        let RegionPeople { cx: _, cz: _, builders, use_u } = RegionPeople { cx: 1, cz: 2, builders: 3, use_u: 4 };
        assert_eq!((builders, use_u), (3, 4));
        let one = RegionPeople { cx: 1, cz: 2, builders: 3, use_u: 4 };
        assert_eq!(bincode::serialize(&one).unwrap().len(), 10, "two i32 and two u8, nothing else");
    }

    #[test]
    fn quantisation_and_the_joiner_terms_match_the_host_formulas() {
        assert_eq!(quantise_u(0.0), 0);
        assert_eq!(quantise_u(crate::use_record::U_CAP), 153);
        assert_eq!(quantise_u(7.0), 255);
        assert_eq!(built_term(0), 0.0);
        assert_eq!(built_term(1), 0.0);
        assert_eq!(built_term(3), crate::authorship::built_together(3));
        assert_eq!(built_term(200), crate::authorship::built_together(8), "capped");
        assert!((lived_term(quantise_u(0.4)) - 0.4).abs() <= 0.5 / 255.0);
    }

    #[test]
    fn a_joiner_forgets_zero_ignores_a_packet_that_is_too_long_and_forgets_a_column_it_lets_go() {
        let mut p = RemotePeople::default();
        let one = |cx, b, u| RegionPeoplePacket { regions: vec![RegionPeople { cx, cz: 0, builders: b, use_u: u }] };
        assert!(p.apply(&one(1, 2, 30)));
        assert_eq!(p.get(1, 0), (2, 30));
        assert_eq!(p.get(9, 9), (0, 0));
        assert!(p.apply(&one(1, 0, 0)));
        assert!(p.is_empty(), "zero is the default");
        assert!(p.apply(&one(2, 200, 5)));
        assert_eq!(p.get(2, 0), (8, 5), "builders are capped on arrival");
        let long = RegionPeoplePacket {
            regions: (0..=MAX_REGIONS_PER_PACKET as i32)
                .map(|cx| RegionPeople { cx, cz: 1, builders: 2, use_u: 1 })
                .collect(),
        };
        assert!(!p.apply(&long));
        assert_eq!(p.len(), 1);
        p.forget(2, 0);
        assert!(p.is_empty(), "a column let go is forgotten");
    }
}
