//! Signet contacts app-access wire v2 — the pure CONSUMER half, ported from
//! `forgesworn/signet-contacts` (`docs/WIRE.md`, v2 frozen; upstream fbb13cd).
//! AxeNStax choices on top: `docs/foundations/2026-10-01-signet-contacts-sync.md`.
//!
//! No relay I/O, no storage, no game types: this module is liftable to a
//! standalone crate unchanged. Where this port and WIRE.md disagree, WIRE.md
//! wins and this is the bug. Conformance: `tests.rs` against the frozen
//! vectors copied into `vectors/`.
//!
//! Consumer flow (step 2 wires it): `build_pairing_uri` → QR/`web_carrier` →
//! for each candidate event `ack_event_is_fresh` + `decrypt_ack_event` →
//! show `format_pairing_code(pairing_code(..))` and wait for the person's
//! Continue → fetch kind 30078 `#d=[projection_tag(grant_id)]` authored by the
//! rail → `open_vault_envelope_text` → `parse_projection` (+ check `grant_id`)
//! → keep it only if `is_newer` than what is held.

// Step 1 of the contacts wire is the pure codec: the re-exports below are its
// public surface, and the ones no caller uses yet (relay I/O, storage and the
// Friends UI are step 2) carry `allow(unused_imports)` / `allow(dead_code)`.

pub mod ack;
pub mod code;
pub mod constants;
mod coverage;
pub mod envelope;
mod guards;
pub mod pairing;
pub mod projection;
pub mod sanitise;
pub mod tags;

#[cfg(test)]
mod tests;

#[allow(unused_imports)] // step-2 surface
pub use ack::{ack_event_is_fresh, decrypt_ack_event, parse_ack, Ack};
pub use code::{format_pairing_code, pairing_code};
#[allow(unused_imports)] // step-2 surface
pub use constants::{clamp_staleness, normalise_capabilities, Capability};
#[allow(unused_imports)] // step-2 surface
pub use envelope::{open_vault_envelope, open_vault_envelope_text};
pub use pairing::{build_pairing_uri, web_carrier, Directory, PairingError};
#[allow(unused_imports)] // step-2 surface
pub use projection::{
    frontier_newer, is_newer, parse_projection, Frontier, Identity, ProjectedContact, Projection,
    Tier,
};
#[allow(unused_imports)] // step-2 surface
pub use sanitise::sanitize_wire_text;
#[allow(unused_imports)] // step-2 surface
pub use tags::{ack_tag, projection_tag, proposal_tag, scoped_contact_id};
