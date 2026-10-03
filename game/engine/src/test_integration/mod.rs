//! Integration test suites that exercise `TestHost` from `test_harness.rs`.
//! Crate-only; `#[cfg(test)] mod test_integration;` in `main.rs`.

pub mod smoke;
pub mod handshake;
pub mod mobs;
pub mod blocks;
pub mod physics;
pub mod save_load;
pub mod villages;
pub mod audits;
pub mod raids;
pub mod builder;
pub mod salt;
pub mod rubber;
pub mod bounty;
pub mod repair;
pub mod plot;
pub mod market_hub;
pub mod auction;
pub mod bazaar;
pub mod avatars;
pub mod skins;
pub mod scenario;
pub mod play_mode;
pub mod third_person;
pub mod rail_freight;
pub mod server_card;
pub mod power;
pub mod electricity;
pub mod electricity_sources;
pub mod explosives;
pub mod death_drops;
pub mod late_join;
pub mod joiner_authority;
pub mod audit_cross_wave;
pub mod trials_lint;
pub mod packaging_copy_lint;
pub mod weather;
pub mod chat;

// Online play by contact — the rendezvous end to end over an in-memory relay.
#[cfg(not(target_arch = "wasm32"))]
pub mod online_play;

// Red line 2 — no AxeNStax-operated relay in any shipped relay default.
#[cfg(not(target_arch = "wasm32"))]
pub mod relay_defaults_lint;
