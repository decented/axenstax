//! Bloom, the living world (spec: `docs/superpowers/specs/2026-11-08-living-world-tier1.md` §1).
//!
//! How alive a place is, worked out on each client from what it can see: the
//! blocks and creatures of a region (the life term), the people together in it
//! (the people term), eased slowly toward a target level. Presentation only:
//! no wire packet, no save field, no write to the world.
//!
//! The slice S1 API is read by later slices (colour, sound, Feast, Commons,
//! trials, the Tally), so it carries `dead_code` allowances until they land.
#![allow(dead_code, unused_imports)]

pub mod eco;
pub mod field;
pub mod kinds;
pub mod level;
pub mod natural;
pub mod people;
pub mod present;
pub mod probe;
pub mod scan;
pub mod terms;

mod glue;

#[cfg(test)]
mod test_world;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod shelf_tests;

pub use eco::EcoTally;
pub use field::{BloomField, TickInput, TickOutput, WorldSummary};
pub use level::Stage;
pub use people::{CommonsArea, FeastMarker};
pub use probe::{ProbeId, ProbeState};
pub use terms::{world_terms, WorldTerms};

/// One region: one chunk column, `(cx, cz)`.
pub type RegionKey = (i32, i32);
