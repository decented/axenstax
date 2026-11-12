//! Integration test suites that exercise `TestHost` from `test_harness.rs`.
//! Crate-only; `#[cfg(test)] mod test_integration;` in `main.rs`.

pub mod smoke;
pub mod handshake;
pub mod mobs;
pub mod spawn_protection;
pub mod blocks;
pub mod physics;
pub mod save_load;
mod save_pristine_columns; // Spec 02 §7.5.1 — a save writes the edited columns only; untouched land regenerates.
pub mod villages;
pub mod audits;
pub mod raids;
pub mod builder;
pub mod salt;
pub mod rubber;
pub mod bounty;
pub mod repair;
pub mod plot;
mod authorship; // Living world Tier 2 S2 — who built a block: save, Commons protection, Plan starters.
mod commons_shelf; // Living world Tier 2 S6 — the Commons Shelf: local, hopper, save, recipe and item-list halves.
mod blight; // Living world Tier 2 S8 — blight: joiner, creative and trial gates, save round trip, break.
mod region_people; // Living world Tier 2 S10 — a joiner is told b and u per region it holds: counts, cadence, sent-set, lending host.
mod growth_after_reload; // Living world Tier 2 S4/S8 review: crops grow after a reload at any tick phase.
mod use_record; // Living world Tier 2 S3 — the use record, nights slept and Worn Paths: save, joiner, lending host.
mod plan_builds; // Living world Tier 2 S4 — Plan build counts: completion on every path, save, digest.
mod plot_succession; // Living world Tier 2 S5 — joiners' Plots, last seen and succession: claims, protection, pass-on, save.
mod site_board; // Living world Tier 2 S7 — Site Boards: joiner ops, shown lists, reach, surge, break, save.
pub mod market_hub;
pub mod auction;
pub mod bazaar;
pub mod avatars;
pub mod skins;
pub mod scenario;
mod grey_quarry; // Living world S4: the Grey Quarry, from arena to end card, without a window.
pub mod play_mode;
pub mod third_person;
pub mod rail_freight;
pub mod server_card;
pub mod power;
pub mod electricity;
pub mod electricity_sources;
pub mod explosives;
pub mod death_drops;
mod sim_death;
pub mod late_join;
pub mod entity_mirror; // MP-D2a — joiners see and are hurt by the server's mobs.
pub mod joiners_act; // MP-D2b — joiners attack, interact with and kill the server's mobs.
pub mod joiner_inventory; // C1 — the server yields joiners' breaks and shadows their inventory.
pub mod use_edits; // C3c-1 — a joiner's block-edit uses are mirrored on the server's copy of its inventory.
pub mod joiner_hunger; // C2a — the server runs joiners' hunger, eating and sleep.
pub mod joiner_craft_drop; // C2b — the server mirrors joiners' crafts and spawns their Q-drops.
pub mod window_ops; // C3a-2a — the server mirrors a joiner's inventory window, click for click.
pub mod shared_containers; // C3b-1 — shared chests, dispensers and furnaces for joiners.
pub mod feast; // Living world S3 — the Feast and Commons through the real HostedServer.
pub mod block_use; // C3b-2 — composters, drying racks, campfires, item frames and hives for joiners.
pub mod use_requests; // C3c-2 — a joiner's bow, slingshot, carts, fishing and campfire lighting run on the server.
pub mod joiner_refusals; // C3c-3r — joiners are refused the Plan and economy actions the server cannot mirror yet.
pub mod attachments; // C3c-3b — face attachments live in the server's world; joiners paint, lay paper and peel by request.
pub mod riding; // MP-D2c-1 — riding runs on the server: joiners ride steeds and carts, steered per tick.
pub mod state_budget; // T1-5 — bounded server→client StateUpdates.
pub mod joiner_authority;
mod access_revocation; // A policy change reaches players already in the world.
// D1 — a host lends its world to its server: one world, one sim.
pub mod lent_world;
#[cfg(not(target_arch = "wasm32"))]
mod handover_live;
#[cfg(not(target_arch = "wasm32"))]
mod handover_component;
mod handover_game;
#[cfg(not(target_arch = "wasm32"))]
mod handover_chain;
#[cfg(not(target_arch = "wasm32"))]
mod handover_carry_fixture;
pub mod audit_cross_wave;
pub mod trials_lint;
pub mod packaging_copy_lint;
pub mod weather;
mod clock_authority; // D4 slice 2 — clock + weather owned by the server on a lent world.
pub mod chat;
pub mod block_machines; // T1-3 — dedicated server ticks the block machines.
// W2 survival basics — server-side fall damage + drowning for remote players.
pub mod survival;
// T2-9 — worldgen golden hash + determinism (WORLDGEN_VERSION).
pub mod worldgen_golden;
pub mod worldgen_v3_golden;
pub mod worldgen_v4_golden;
// T2-9 — a joiner builds the host's world (seed + rules + spawn) from JoinAccept.
pub mod join_world;
// MP-A3 — server-side projectiles reach joiners; a dead joiner stays dead.
pub mod server_projectiles;
pub mod joiner_death;
mod death_window;
mod death_boundaries;
// MP step 1 — a joiner's one position: server-simulated, predicted + reconciled.
pub mod position_truth;
// Phase B1 — the dedicated server streams columns around every player.
pub mod server_streaming;
// Phase B2a — the server pushes chunks to joiners; joiners take them in.
pub mod chunk_push;

// Online play by contact — the rendezvous end to end over an in-memory relay.
#[cfg(not(target_arch = "wasm32"))]
pub mod online_play;

// Red line 2 — no AxeNStax-operated relay in any shipped relay default.
#[cfg(not(target_arch = "wasm32"))]
pub mod relay_defaults_lint;
// Phase B2b — only touched columns are pushed; joiners generate the rest.
pub mod touched_columns;
// The joiner harness the chunk-push suites share.
pub mod push_joiner;
pub mod render_depth;
mod torch_render; // Small cubes show the right part of their texture: torch, button, plate, lever handle, framed block.
pub mod building_variants; // Building variants — every wood's and stone's slab, stairs, door, trapdoor, gate and wall.
pub mod machine_index;
// Terrain T1 — the 256-block world and the per-world terrain generator.
pub mod tall_world;
pub mod twin_run; // Keep slice 1 — twin-run determinism (two seeded servers, one digest).
pub mod tick_determinism_lint; // Keep slice 1 — no wall clock or OS randomness in tick paths.
pub(crate) mod tick_bench; // Keep slice 1 — #[ignore] release bench of GameServer::tick (handoff §8).
pub mod clock_save; // Keep slice 2a: the authority clock and joiner position across a restart.
pub(crate) mod authority_checkpoint; // Keep slice 2b: checkpoint and restore of the authority.
mod authority_checkpoint_refusals; // Keep slice 2b: what they refuse, under which code.
mod authority_checkpoint_resealed; // Keep slice 2b: bytes someone resealed, refused before they can hurt.
mod authority_checkpoint_records; // Keep slice 2b: what a restore does with the stored records.
mod authority_checkpoint_order; // Keep slice 3: restore order, digests over stored bytes, torn meta.
mod authority_checkpoint_limits; // Keep slice 3: every restore limit enforced at checkpoint.
mod authority_checkpoint_deaths; // Keep slice 3: a pending death is in the checkpoint.
mod authority_checkpoint_mutation; // Keep slice 3: every field of the save is bound, driven from the type.
mod authority_checkpoint_size; // Keep slice 3: the checkpoint's size doesn't follow the world's.
mod authority_checkpoint_pieces; // Keep slice 3: world.dat in fixed pieces, and reads that fail.
mod authority_checkpoint_builder; // Keep slice 3, ruling 1: the record set built from memory, equal to the disk.
mod authority_checkpoint_lava; // Keep slice 3, ruling 1: lava records that don't shift as the queue drains.
