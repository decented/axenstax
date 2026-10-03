#![cfg(not(target_arch = "wasm32"))]
//! Heartwood-backed server identity (Track 2 — the keystone).
//!
//! A dedicated server holds a disposable **runtime keypair**; the operator's
//! Heartwood (any NIP-46 bunker) signs a one-time **delegation attestation** over
//! that runtime pubkey. The server then signs locally with the runtime key, and
//! trust verifies the chain `runtime key → attestation → operator npub`. The
//! operator's real key never reaches the box.
//!
//! Design: `docs/superpowers/specs/2026-06-17-heartwood-signed-server-identity-design.md`.
//! Bunker-agnostic — the only dependency is the NIP-46 `NostrSigner` contract.

pub mod admin;
pub mod admin_relay;
pub mod attestation;
pub mod claim;
pub mod connect;
pub mod pairing;
pub mod proof;
pub mod server_card;
pub mod store;

// `attestation::*` is consumed via the submodule path (store, pairing) and by
// Track 3/6 later; no flat re-export needed yet.
pub use admin::*;
// `claim::*` is forward-API (Track 6); consumed via the submodule path for now.
pub use connect::*;
pub use pairing::*;
pub use proof::*;
pub use store::*;
