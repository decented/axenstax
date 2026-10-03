//! Cross-branch checks for the 2026-09-27 audit waves. Each test needs code
//! from more than one wave, so it can only exist on the integrated tree:
//!
//! - wave-items (blasted containers spill) + wave-net (the server mirrors the
//!   host's live container state and spills a joiner's break exactly once);
//! - wave-persist (torn-meta recovery from `world.dat`) + wave-items (the
//!   per-world random PoP secret, stripped from every export).

use super::joiner_authority::{
    block_changes_seen, bones_on_server, cell_beside, chest_of_bones, join_guest, send_edits,
    send_host_edits, start_open_server,
};
use crate::block;
use crate::item::{Item, MaterialId};

fn bones_in(stacks: &[((i32, i32, i32), crate::item::ItemStack)]) -> u32 {
    stacks
        .iter()
        .filter(|(_, s)| s.item == Item::Material(MaterialId::Bone))
        .map(|(_, s)| u32::from(s.count))
        .sum()
}

// ── Spills: host keg blast + joiner break ─────────────────────────────────

/// A HOST keg blast beside a filled chest: the host's client spills the chest
/// (wave-items' blast path) and the server, which mirrors the host's live copy
/// (wave-net), must not spill it again. Exactly one spill in total.
#[test]
fn a_host_keg_blast_beside_a_filled_chest_spills_exactly_once() {
    let mut hs = start_open_server("host-keg");
    hs.tick();
    let chest = cell_beside(&hs, 0, 1, 0, 0);
    let keg = cell_beside(&hs, 0, 2, 0, 0);

    // The chest existed at load (server holds a copy) and the host's client
    // holds the same live state; the keg sits right next to it.
    hs.server.world.set_block(chest.0, chest.1, chest.2, block::CHEST);
    hs.server.world.insert_chest(chest, chest_of_bones(7));
    hs.server.world.set_block(keg.0, keg.1, keg.2, block::BLASTING_KEG);
    let mut host = crate::world::World::new();
    host.set_block(chest.0, chest.1, chest.2, block::CHEST);
    host.insert_chest(chest, chest_of_bones(7));
    host.set_block(keg.0, keg.1, keg.2, block::BLASTING_KEG);
    hs.mirror_host_world_state(&host);
    hs.tick();

    // The host's client detonates (the same world-side call `detonate_keg`
    // makes) and spawns what spilled; its breaks ride the next ClientInput.
    let outcome = crate::explosion::resolve_blast(
        &mut host,
        keg,
        crate::explosion::BLAST_RADIUS,
        crate::explosion::KEG_BLAST_POWER,
    );
    let host_drops = bones_in(&outcome.spilled);
    assert_eq!(host_drops, 7, "the host's blast spills the chest");
    assert!(host.chest_at(chest).is_none());
    let edits: Vec<_> = outcome.destroyed.iter().map(|&(p, _)| (p, block::AIR)).collect();
    assert!(edits.iter().any(|&(p, _)| p == chest), "the chest cell is broadcast");

    // Same frame order as the game loop: host input queued, mirror, tick.
    send_host_edits(&hs, 2, &edits);
    hs.mirror_host_world_state(&host);
    hs.tick();
    // …and a few more frames, in case anything lingered.
    for _ in 0..3 {
        hs.mirror_host_world_state(&host);
        hs.tick();
    }

    assert_eq!(hs.server.world.get_block(chest.0, chest.1, chest.2), block::AIR);
    assert!(hs.server.world.chest_at(chest).is_none(), "server copy gone");
    assert_eq!(
        host_drops + bones_on_server(&hs),
        7,
        "host-side + server-side drops: exactly one spill in total"
    );
}

/// Same blast, but the server processes the host's breaks BEFORE the mirror
/// has caught up (its stale load-time copy is still there). Still one spill.
#[test]
fn a_host_keg_blast_spills_once_even_before_the_mirror_catches_up() {
    let mut hs = start_open_server("host-keg-stale");
    hs.tick();
    let chest = cell_beside(&hs, 0, 1, 0, 0);
    let keg = cell_beside(&hs, 0, 2, 0, 0);
    hs.server.world.set_block(chest.0, chest.1, chest.2, block::CHEST);
    hs.server.world.insert_chest(chest, chest_of_bones(7));
    hs.server.world.set_block(keg.0, keg.1, keg.2, block::BLASTING_KEG);
    let mut host = crate::world::World::new();
    host.set_block(chest.0, chest.1, chest.2, block::CHEST);
    host.insert_chest(chest, chest_of_bones(7));
    host.set_block(keg.0, keg.1, keg.2, block::BLASTING_KEG);

    let outcome = crate::explosion::resolve_blast(
        &mut host,
        keg,
        crate::explosion::BLAST_RADIUS,
        crate::explosion::KEG_BLAST_POWER,
    );
    let host_drops = bones_in(&outcome.spilled);
    let edits: Vec<_> = outcome.destroyed.iter().map(|&(p, _)| (p, block::AIR)).collect();
    send_host_edits(&hs, 1, &edits);
    hs.tick();
    hs.mirror_host_world_state(&host);
    hs.tick();

    assert!(hs.server.world.chest_at(chest).is_none());
    assert_eq!(host_drops + bones_on_server(&hs), 7, "exactly one spill in total");
}

/// A joiner breaks a chest the HOST filled after load (so only the host's
/// client held its contents): the server spills the live contents once, the
/// host's client drops its entity without spilling, and later mirrors never
/// resurrect or re-spill it.
#[test]
fn a_joiner_break_of_a_chest_the_host_filled_after_load_spills_once() {
    let mut hs = start_open_server("joiner-live-chest");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let chest = cell_beside(&hs, slot, 1, 0, 0);
    // Loaded empty on both sides…
    hs.server.world.set_block(chest.0, chest.1, chest.2, block::CHEST);
    hs.server.world.insert_chest(chest, crate::chest::ChestData::new());
    let mut host = crate::world::World::new();
    host.set_block(chest.0, chest.1, chest.2, block::CHEST);
    host.insert_chest(chest, crate::chest::ChestData::new());
    hs.mirror_host_world_state(&host);
    hs.tick();
    // …then the host fills it in the chest UI (client-only state).
    host.insert_chest(chest, chest_of_bones(9));
    hs.mirror_host_world_state(&host);
    hs.tick();

    send_edits(&hs, &client, slot, 1, &[(chest, block::AIR)]);
    hs.mirror_host_world_state(&host);
    hs.tick();
    assert_eq!(bones_on_server(&hs), 9, "the host's live contents spill on the server");

    // The host applies the broadcast break. `apply_remote_block_change` hands
    // back no stacks and spawns nothing, so the host side drops 0; it must
    // still clear the entity (no orphan the next mirror could resurrect).
    let host_side_drops = 0u32;
    let seen = block_changes_seen(&hs.local_transports[0]);
    assert!(seen.iter().any(|bc| (bc.x, bc.y, bc.z) == chest), "the break is broadcast");
    for bc in seen {
        host.apply_remote_block_change(&bc);
    }
    assert!(host.chest_at(chest).is_none(), "no orphan chest on the host");
    for _ in 0..3 {
        hs.mirror_host_world_state(&host);
        hs.tick();
    }
    assert!(hs.server.world.chest_at(chest).is_none(), "never resurrected");
    assert_eq!(host_side_drops + bones_on_server(&hs), 9, "exactly one spill in total");
}

// ── PoP secret across meta recovery + export ─────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
mod pop_secret {
    use crate::save::{self, WorldMeta, WorldsRootGuard};
    use std::fs;

    /// A world on disk with an intact `world.dat` (seed 4242) and a
    /// pretty-printed meta; returns (meta bytes, secret).
    fn write_world(name: &str) -> (Vec<u8>, [u8; 32]) {
        let dir = save::world_dir(name);
        fs::create_dir_all(&dir).unwrap();
        let data = save::minimal_world_save_for_tests(4242);
        fs::write(dir.join("world.dat"), bincode::serialize(&data).unwrap()).unwrap();
        let mut meta = WorldMeta::new(name);
        meta.seed = 4242;
        let secret = meta.pop_secret.expect("a new world has a secret");
        let json = serde_json::to_vec_pretty(&meta).unwrap();
        fs::write(dir.join("world_meta.json"), &json).unwrap();
        (json, secret)
    }

    fn exported_meta_has_no_secret(name: &str) {
        let bytes = crate::native_world_io::export_world_native(name).expect("export");
        let (meta, _, _) =
            crate::world_archive::unpack_world(&bytes, &mut crate::world::World::new()).unwrap();
        assert_eq!(meta.pop_secret, None, "an .axeworld export carries no PoP secret");
        // And not a byte of it anywhere in the raw archive.
        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&bytes[..]), &mut raw)
            .unwrap();
        assert!(
            !raw.windows(b"pop_secret".len()).any(|w| w == b"pop_secret"),
            "no pop_secret key in the archive"
        );
    }

    #[test]
    fn a_torn_meta_with_an_intact_secret_keeps_the_original_secret() {
        let _g = WorldsRootGuard::new("cross_torn_meta_keeps");
        let (json, secret) = write_world("w");
        // Tear the tail: `pop_secret` is the last field, so its array survives.
        let torn = &json[..json.len() - 2];
        fs::write(save::world_dir("w").join("world_meta.json"), torn).unwrap();

        let meta = save::try_load_world_meta("w").expect("recovered from world.dat");
        assert_eq!(meta.seed, 4242, "seed from world.dat");
        assert_eq!(meta.pop_secret, Some(secret), "the ORIGINAL secret survives recovery");
        // The load path keeps it (no regeneration) and it persists.
        let mut slot = meta.pop_secret;
        assert_eq!(crate::proof_of_play::ensure_world_secret(&mut slot), (secret, false));
        assert_eq!(save::load_world_meta("w").pop_secret, Some(secret));

        // Meta gone entirely, only the quarantined copy left: still the original.
        fs::remove_file(save::world_dir("w").join("world_meta.json")).unwrap();
        assert_eq!(save::try_load_world_meta("w").unwrap().pop_secret, Some(secret));

        exported_meta_has_no_secret("w");
    }

    #[test]
    fn a_torn_meta_whose_secret_is_lost_gets_a_fresh_nonzero_one() {
        let _g = WorldsRootGuard::new("cross_torn_meta_fresh");
        let (json, secret) = write_world("w");
        // Cut INSIDE the secret's array: unrecoverable.
        let key = json.windows(12).position(|w| w == b"\"pop_secret\"").unwrap();
        let torn = &json[..key + 40];
        fs::write(save::world_dir("w").join("world_meta.json"), torn).unwrap();

        let meta = save::try_load_world_meta("w").expect("recovered from world.dat");
        assert_eq!(meta.seed, 4242);
        let fresh = meta.pop_secret.expect("never None after recovery");
        assert_ne!(fresh, [0u8; 32], "never zero");
        assert_ne!(fresh, secret, "the lost secret is replaced, not guessed");
        // Written back: the next load keeps the fresh one instead of rolling again.
        assert_eq!(save::load_world_meta("w").pop_secret, Some(fresh));

        exported_meta_has_no_secret("w");
    }

    #[test]
    fn salvage_is_strict_about_the_array() {
        let ok = format!("{{\"x\":1, \"pop_secret\" : [{}]", ["7"; 32].join(" ,\n"));
        assert_eq!(save::salvage_pop_secret(ok.as_bytes()), Some([7u8; 32]));
        let zero = format!("\"pop_secret\":[{}]", ["0"; 32].join(","));
        assert_eq!(save::salvage_pop_secret(zero.as_bytes()), None, "all-zero is rejected");
        let short = format!("\"pop_secret\":[{}]", ["7"; 31].join(","));
        assert_eq!(save::salvage_pop_secret(short.as_bytes()), None);
        let big = format!("\"pop_secret\":[256,{}]", ["7"; 31].join(","));
        assert_eq!(save::salvage_pop_secret(big.as_bytes()), None);
        assert_eq!(save::salvage_pop_secret(b"\"pop_secret\":[1,2,3"), None);
        assert_eq!(save::salvage_pop_secret(b"{\"display_name\":\"x\""), None);
    }
}
