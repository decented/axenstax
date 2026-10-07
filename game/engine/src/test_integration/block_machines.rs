//! T1-3 (2026-10-05) — the dedicated server ticks the block machines.
//!
//! Every test drives the real `GameServer::tick` through [`TestHost`]. With
//! `simulates_block_machines` ON (what `HostedServer::start_inner` sets for a
//! 0-local-player dedicated server) a furnace smelts, a crop grows, a piston
//! extends and a hopper moves an item — and each change that touches a block
//! rides `pending_block_changes`, the queue `HostedServer` drains into every
//! joiner's `StateUpdate`. With the flag OFF (a LAN host, whose CLIENT ticks
//! these) the server leaves them alone: no double tick.
//!
//! The flag's wiring itself (0 local players ⇔ on) is pinned in
//! `hosted_server.rs`'s `block_machine_flag_tests`.

use crate::block;
use crate::item::{Item, ItemStack, MaterialId};
use crate::meta::Facing;
use crate::protocol::BlockChange;
use crate::test_harness::{TestConfig, TestHost};

/// Build height for every rig — comfortably inside the world's 0..95.
const Y: i32 = 64;

fn host(machines: bool) -> TestHost {
    let mut h = TestHost::start_with(TestConfig::default());
    h.server.simulates_block_machines = machines;
    h
}

/// Did the server queue a broadcast of `block` at `cell`? `TestHost` never
/// drains `pending_block_changes`, so this sees every change since start.
fn queued(h: &TestHost, cell: (i32, i32, i32), block: block::BlockId) -> bool {
    h.server
        .pending_block_changes
        .iter()
        .any(|bc: &BlockChange| (bc.x, bc.y, bc.z) == cell && bc.new_block == block)
}

// ── Furnace ────────────────────────────────────────────────────────────────

/// One furnace pass runs every 4 server ticks (the client's 4-tick machine
/// cadence), so a full smelt needs 4 × SMELT_TICKS_PER_ITEM ticks plus slack
/// for the fuel-catch beat.
fn ticks_for_one_smelt() -> u32 {
    4 * (crate::furnace::SMELT_TICKS_PER_ITEM + 8)
}

fn load_copper_furnace(h: &mut TestHost) -> (i32, i32, i32) {
    let f = (6, Y, 6);
    h.load_furnace(
        f.0,
        f.1,
        f.2,
        ItemStack::new_material(MaterialId::Copper, 1),
        ItemStack::new_material(MaterialId::GreenLog, 1),
    );
    f
}

#[test]
fn a_machine_ticking_server_smelts_and_broadcasts_the_lit_furnace() {
    let mut h = host(true);
    let f = load_copper_furnace(&mut h);
    h.tick(ticks_for_one_smelt());
    let out = h
        .server
        .world
        .furnace_at(f)
        .and_then(|d| d.output.clone())
        .expect("the server smelted the copper");
    assert!(matches!(out.item, Item::Material(MaterialId::CopperIngot)));
    assert_eq!(out.count, 1);
    assert!(
        queued(&h, f, block::FURNACE_LIT),
        "the lit flip rides pending_block_changes so joiners see the furnace burn"
    );
}

#[test]
fn a_host_backed_server_does_not_smelt() {
    // Flag off = a LAN host: its client owns the furnace sweep (and the mirror
    // copies the result in). The server must not smelt a second time.
    let mut h = host(false);
    let f = load_copper_furnace(&mut h);
    h.tick(ticks_for_one_smelt());
    let data = h.server.world.furnace_at(f).expect("furnace entity still there");
    assert!(data.output.is_none(), "no server-side smelt without the flag");
    assert_eq!(h.get_block(f.0, f.1, f.2), block::FURNACE, "never lit");
}

// ── Crops ──────────────────────────────────────────────────────────────────

/// A stage-0 wheat on tilled soil in full sky light. Growth is deterministic:
/// it fires on `tick_counter` multiples of the per-stage interval (200) when
/// effective light ≥ 9 — no RNG.
fn plant_wheat(h: &mut TestHost) -> (i32, i32, i32) {
    let c = (0, 70, 0);
    h.set_block(c.0, c.1 - 1, c.2, block::TILLED_SOIL);
    h.set_block(c.0, c.1, c.2, block::WHEAT_STAGE_0);
    h.server.world.set_sky_light_at(c.0, c.1, c.2, 15);
    c
}

#[test]
fn a_machine_ticking_server_grows_a_crop_and_broadcasts_it() {
    let mut h = host(true);
    let c = plant_wheat(&mut h);
    h.tick(crate::growth::CROP_GROWTH_TICKS_PER_STAGE as u32);
    let now = h.get_block(c.0, c.1, c.2);
    assert_ne!(now, block::WHEAT_STAGE_0, "the wheat advanced on the growth cadence");
    assert!(queued(&h, c, now), "the new stage rides pending_block_changes");
}

#[test]
fn a_host_backed_server_does_not_grow_crops() {
    let mut h = host(false);
    let c = plant_wheat(&mut h);
    h.tick(2 * crate::growth::CROP_GROWTH_TICKS_PER_STAGE as u32);
    assert_eq!(h.get_block(c.0, c.1, c.2), block::WHEAT_STAGE_0);
}

// ── Pistons ────────────────────────────────────────────────────────────────

/// `electricity.rs`'s rig: a piston at the origin facing East, a lever on its
/// west face, a stone block in front to shove.
fn piston_rig(h: &mut TestHost) {
    h.set_block(0, Y, 0, block::PISTON);
    h.set_meta_facing(0, Y, 0, Facing::East);
    h.set_block(1, Y, 0, block::STONE);
    h.place_power_block(-1, Y, 0, block::LEVER, Facing::East);
}

#[test]
fn a_machine_ticking_server_extends_a_powered_piston() {
    let mut h = host(true);
    piston_rig(&mut h);
    h.toggle_lever(-1, Y, 0);
    // Tick 1's power_tick energises the run; the tick-4 machine pass reads it.
    h.tick(4);
    assert_eq!(h.get_block(1, Y, 0), block::PISTON_HEAD, "the arm comes out");
    assert_eq!(h.get_block(2, Y, 0), block::STONE, "the stone is shoved one cell along");
    assert!(queued(&h, (1, Y, 0), block::PISTON_HEAD), "joiners see the arm");
    assert!(queued(&h, (2, Y, 0), block::STONE), "…and the shoved block");
}

#[test]
fn a_host_backed_server_does_not_fire_pistons() {
    let mut h = host(false);
    piston_rig(&mut h);
    h.toggle_lever(-1, Y, 0);
    h.tick(8);
    assert_eq!(h.get_block(1, Y, 0), block::STONE, "the host client owns the push");
}

// ── Hoppers ────────────────────────────────────────────────────────────────

fn hopper_rig(h: &mut TestHost) {
    h.set_block(0, Y + 1, 0, block::CHEST);
    h.insert_chest_with(0, Y + 1, 0, &[ItemStack::new_material(MaterialId::Copper, 3)]);
    h.set_block(0, Y, 0, block::HOPPER);
    h.set_block(0, Y - 1, 0, block::CHEST);
    h.insert_chest_with(0, Y - 1, 0, &[]);
}

fn copper_in(h: &TestHost, x: i32, y: i32, z: i32) -> u32 {
    h.chest_at(x, y, z)
        .expect("chest")
        .slots
        .iter()
        .flatten()
        .filter(|s| matches!(s.item, Item::Material(MaterialId::Copper)))
        .map(|s| s.count as u32)
        .sum()
}

#[test]
fn a_machine_ticking_server_runs_hoppers_on_their_cadence() {
    let mut h = host(true);
    hopper_rig(&mut h);
    h.tick(crate::hopper::HOPPER_INTERVAL_TICKS as u32);
    assert_eq!(copper_in(&h, 0, Y - 1, 0), 1, "one item per hopper interval");
    assert_eq!(copper_in(&h, 0, Y + 1, 0), 2);
}

#[test]
fn a_host_backed_server_does_not_run_hoppers() {
    let mut h = host(false);
    hopper_rig(&mut h);
    h.tick(4 * crate::hopper::HOPPER_INTERVAL_TICKS as u32);
    assert_eq!(copper_in(&h, 0, Y - 1, 0), 0);
    assert_eq!(copper_in(&h, 0, Y + 1, 0), 3);
}

// ── Leaf decay ─────────────────────────────────────────────────────────────

#[test]
fn server_leaf_decay_is_applied_and_broadcast() {
    // A leaf with no log within reach, queued the way a broken log queues it.
    // Before T1-3 the server computed this decay and threw the result away:
    // the leaf vanished from the server's world but no joiner was told.
    let mut h = host(true);
    let leaf = (10, Y, 10);
    h.set_block(leaf.0, leaf.1, leaf.2, block::OAK_LEAVES);
    h.server
        .leaf_decay
        .on_log_broken(leaf.0, leaf.1 - 1, leaf.2, &h.server.world);
    // Support check + a 27..=108-pass random delay, one pass per 4 ticks.
    h.tick(4 * 120);
    assert_eq!(h.get_block(leaf.0, leaf.1, leaf.2), block::AIR, "the leaf decayed");
    assert!(queued(&h, leaf, block::AIR), "the decay rides pending_block_changes");
}

// ── A remote player's log break feeds the SERVER's leaf decay (T1-3 review B) ─
//
// Drives real joins through `HostedServer::tick` (the `joiner_authority`
// helpers). A joiner's client runs no leaf decay of its own any more (it rolled
// a second, independent set of saplings on top of the server's), so the server
// must decay a remote break's canopy on EVERY host kind — the LAN host as well
// as the dedicated server — and roll its saplings exactly once. A LAN host's
// OWN break stays with the host client's decay: feeding it to the server too
// would roll every sapling twice.

use super::joiner_authority::{block_changes_seen, cell_beside, join_guest, send_edits, start_open_server};
use crate::hosted_server::{HostedServer, RemoteTransport};

/// Server ticks to cover the support checks plus the 27..=108-pass random
/// decay delay (one pass per 4 ticks), with slack for 100 leaves.
const LEAF_DECAY_TICKS: u32 = 4 * 200;

/// A 0-local-player server — what `server_main` runs.
fn start_dedicated_server(tag: &str) -> HostedServer {
    HostedServer::start(
        0,
        format!("block-machines-dedicated-{tag}-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("dedicated server starts")
}

/// Build an isolated tree beside `slot`: one oak log and a 5×5×4 canopy of oak
/// leaves (100 of them) above and beside it — every leaf inside
/// `on_log_broken`'s radius-5 scan, none in the player's own column. The box
/// around the canopy is cleared first, so once the log goes nothing (no
/// worldgen trunk) can support a single leaf: every one must decay.
/// Returns (log cell, leaf cells).
fn build_lone_tree(
    hs: &mut HostedServer,
    slot: usize,
) -> ((i32, i32, i32), Vec<(i32, i32, i32)>) {
    let log = cell_beside(hs, slot, 2, 0, 0);
    let world = &mut hs.server.world;
    for x in log.0 - 1..=log.0 + 5 {
        for z in log.2 - 3..=log.2 + 3 {
            for y in log.1 + 1..=log.1 + 6 {
                world.set_block(x, y, z, block::AIR);
            }
        }
    }
    let mut leaves = Vec::new();
    for x in log.0..=log.0 + 4 {
        for z in log.2 - 2..=log.2 + 2 {
            for y in log.1 + 2..=log.1 + 5 {
                world.set_block(x, y, z, block::OAK_LEAVES);
                leaves.push((x, y, z));
            }
        }
    }
    world.set_block(log.0, log.1, log.2, block::OAK_LOG);
    (log, leaves)
}

/// Every oak sapling the server holds: on the ground as item entities, plus
/// (for a joiner) whatever its pickup pass already granted into that player's
/// server-side inventory — the magnet vacuums drops near a server-simulated
/// player, so the ground alone would undercount.
fn oak_saplings_on_server(hs: &HostedServer, picker: Option<usize>) -> u32 {
    let on_ground: u32 = hs
        .server
        .ecs
        .query::<&crate::entity::ItemEntity>()
        .iter()
        .filter_map(|(_, ie)| match &ie.stack.item {
            Item::Material(m) if *m == MaterialId::OakSapling => Some(u32::from(ie.stack.count)),
            _ => None,
        })
        .sum();
    let picked = picker.map_or(0, |slot| {
        u32::from(hs.server.players[slot].inventory.count_material(MaterialId::OakSapling))
    });
    on_ground + picked
}

fn leaves_left(hs: &HostedServer, leaves: &[(i32, i32, i32)]) -> usize {
    leaves
        .iter()
        .filter(|&&(x, y, z)| block::is_any_leaves(hs.server.world.get_block(x, y, z)))
        .count()
}

/// A joiner breaks the log; the SERVER decays the whole canopy, tells the
/// joiner about every leaf, and drops saplings — at most one per leaf.
fn assert_a_joiners_log_break_decays_on_the_server(mut hs: HostedServer) {
    let (client, slot) = join_guest(&mut hs, "Lumberjack");
    // Let a joiner who spawned in the air land before building beside them.
    for _ in 0..60 {
        hs.tick();
    }
    let (log, leaves) = build_lone_tree(&mut hs, slot);
    let _ = block_changes_seen(&client);
    assert_eq!(oak_saplings_on_server(&hs, Some(slot)), 0, "no saplings before the break");

    send_edits(&hs, &client, slot, 1, &[(log, block::AIR)]);
    for _ in 0..LEAF_DECAY_TICKS {
        hs.tick();
    }

    assert_eq!(
        hs.server.world.get_block(log.0, log.1, log.2),
        block::AIR,
        "the joiner's log break was accepted"
    );
    assert_eq!(
        leaves_left(&hs, &leaves),
        0,
        "the server decays a remote player's canopy — the joiner's client no longer does"
    );
    let seen = block_changes_seen(&client);
    assert!(
        leaves.iter().all(|&c| seen
            .iter()
            .any(|bc| (bc.x, bc.y, bc.z) == c && bc.new_block == block::AIR)),
        "every decayed leaf reaches the joiner as a broadcast BlockChange"
    );
    let saplings = oak_saplings_on_server(&hs, Some(slot)) as usize;
    assert!(
        saplings >= 1,
        "~1 in 20 decayed leaves drops a sapling; 100 leaves dropped none — the server \
         decayed the canopy but never spawned its saplings"
    );
    assert!(
        saplings <= leaves.len(),
        "{saplings} saplings from {} leaves — a leaf rolled its sapling more than once",
        leaves.len()
    );
}

#[test]
fn a_joiners_log_break_decays_its_canopy_on_a_lan_host() {
    // The LAN host (1 local player, machines OFF) — before the review fix the
    // feed was flag-gated, so a joiner's break here fed nobody's decay.
    let hs = start_open_server("leaf-decay-joiner");
    assert!(!hs.server.simulates_block_machines);
    assert_a_joiners_log_break_decays_on_the_server(hs);
}

#[test]
fn a_joiners_log_break_decays_its_canopy_on_a_dedicated_server() {
    let hs = start_dedicated_server("leaf-decay-joiner");
    assert!(hs.server.simulates_block_machines);
    assert_a_joiners_log_break_decays_on_the_server(hs);
}

#[test]
fn a_lan_hosts_own_log_break_leaves_decay_to_the_host_client() {
    // The host's own (local, position-trusted) break: its CLIENT queues the
    // canopy and rolls the saplings. The server must not decay it as well, or
    // a second, independent sapling roll lands in the server's world.
    let mut hs = start_open_server("leaf-decay-host-own");
    let (log, leaves) = build_lone_tree(&mut hs, 0);

    send_edits(&hs, &hs.local_transports[0], 0, 1, &[(log, block::AIR)]);
    for _ in 0..LEAF_DECAY_TICKS {
        hs.tick();
    }

    assert_eq!(
        hs.server.world.get_block(log.0, log.1, log.2),
        block::AIR,
        "the host's own log break reached the server"
    );
    assert_eq!(
        leaves_left(&hs, &leaves),
        leaves.len(),
        "the server leaves a host-local break's decay to the host client"
    );
    assert_eq!(
        oak_saplings_on_server(&hs, None),
        0,
        "no server-side sapling roll for a host-local break (the host client rolls them)"
    );
}

// ── A joiner client runs no growth or leaf decay of its own (review A + B) ──

#[test]
fn a_joiner_client_never_grows_crops_or_decays_leaves_itself() {
    // Source lint, in the `electricity::no_client_block_change_push_guesses_its_metadata`
    // tradition (`TestHost` has no client, so the gate can't be driven here).
    // A joiner's own growth pushes were accepted by the server as edits on top
    // of its own growth (two crop stages per cycle), and its own leaf decay
    // rolled a second set of saplings: both must stay behind
    // `remote_client.is_none()` in the client tick.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("game_loop.rs");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "joiner-sim lint: cannot read {} ({e}). If the client tick moved, update \
             this lint's path — do not delete the lint.",
            path.display()
        )
    });
    let lines: Vec<&str> = raw.lines().collect();
    let mut sites = 0;
    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        if line.contains("crate::growth::tick_growth(") || line.contains(".on_log_broken(") {
            sites += 1;
            let from = i.saturating_sub(6);
            assert!(
                lines[from..=i].iter().any(|l| l.contains("remote_client.is_none()")),
                "game_loop.rs:{}: growth / leaf decay runs on a joiner client — gate it \
                 behind `self.remote_client.is_none()` (the server it joined owns both)",
                i + 1
            );
        }
    }
    assert!(
        sites >= 3,
        "joiner-sim lint found only {sites} call sites (expected tick_growth + 2 \
         on_log_broken) — if they moved, update the lint, don't delete it"
    );
}

// ── A joiner client's own machine sims push no edits (FU4b, Q9 row 3) ───────

#[test]
fn a_joiner_client_runs_no_machine_sim_that_pushes_edits_to_the_server() {
    // Source lint, as above (the client tick needs a GPU; the driven version is
    // the `#[ignore]`d game-harness test
    // `game_harness_a_joined_clients_machines_push_no_edits`). A joined
    // client's power tick, dispensers, pistons, keg fuses and lightning fire
    // pushed their block changes to the server as the joiner's own edits, where
    // they fought the server's own machines and grew its edit FIFO (FU3 verify
    // Q9). Each stays behind `remote_client.is_none()`; the furnace sweep still
    // runs on a joiner (its own furnace UI cooks until C3) but its changes are
    // not queued for the server.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("game_loop.rs");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "joiner-sim lint: cannot read {} ({e}). If the client tick moved, update this \
             lint's path — do not delete the lint.",
            path.display()
        )
    });
    let lines: Vec<&str> = raw.lines().collect();
    // (needle on the call line, lines of code above it to search for the gate)
    let sites: [(&str, usize); 6] = [
        ("crate::power::power_tick(", 14),
        ("crate::dispenser::tick_dispensers(", 4),
        ("crate::piston::tick_pistons(", 3),
        ("crate::power::tick_keg_fuses(", 4),
        ("self.fire.ignite(&mut self.world, bx, surface_y + 1, bz", 3),
        ("self.pending_block_changes.extend(furnace_sweep.changes)", 3),
    ];
    for (needle, back) in sites {
        let mut found = 0;
        for (i, line) in lines.iter().enumerate() {
            if line.trim_start().starts_with("//") || !line.contains(needle) {
                continue;
            }
            found += 1;
            let gated = lines[i.saturating_sub(back)..=i]
                .iter()
                .any(|l| !l.trim_start().starts_with("//") && l.contains("remote_client.is_none()"));
            assert!(
                gated,
                "game_loop.rs:{}: `{needle}` runs on a joiner client and pushes its changes to \
                 the server as the joiner's own edits — gate it behind \
                 `self.remote_client.is_none()` (the server it joined runs it)",
                i + 1
            );
        }
        assert!(found >= 1, "joiner-sim lint found no `{needle}` — if it moved, update the lint, don't delete it");
    }
}
