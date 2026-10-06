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
pub mod entity_mirror; // MP-D2a — joiners see and are hurt by the server's mobs.
pub mod state_budget; // T1-5 — bounded server→client StateUpdates.
pub mod joiner_authority;
// D1 — a host lends its world to its server: one world, one sim.
pub mod lent_world;
pub mod audit_cross_wave;
pub mod trials_lint;
pub mod packaging_copy_lint;
pub mod weather;
pub mod chat;
pub mod block_machines; // T1-3 — dedicated server ticks the block machines.
// W2 survival basics — server-side fall damage + drowning for remote players.
pub mod survival;
// T2-9 — worldgen golden hash + determinism (WORLDGEN_VERSION).
pub mod worldgen_golden;
// T2-9 — a joiner builds the host's world (seed + rules + spawn) from JoinAccept.
pub mod join_world;
// MP-A3 — server-side projectiles reach joiners; a dead joiner stays dead.
pub mod server_projectiles;
pub mod joiner_death;
// MP step 1 — a joiner's one position: server-simulated, predicted + reconciled.
pub mod position_truth;
// Phase B1 — the dedicated server streams columns around every player.
pub mod server_streaming;

// Online play by contact — the rendezvous end to end over an in-memory relay.
#[cfg(not(target_arch = "wasm32"))]
pub mod online_play;

// Red line 2 — no AxeNStax-operated relay in any shipped relay default.
#[cfg(not(target_arch = "wasm32"))]
pub mod relay_defaults_lint;
