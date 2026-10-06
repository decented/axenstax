//! Cross-platform tar+gzip world archive — pack/unpack `.axeworld` blobs.
//!
//! This module is NOT cfg-gated: it compiles on both native (x86_64 / aarch64)
//! and wasm32 targets. The JS/wasm-bindgen glue that drives these functions from
//! the browser stays in `wasm_save.rs` (still wasm-only).
//!
//! Archive format (unchanged from the original WASM implementation):
//!   world_meta.json  — serde_json: WorldMeta
//!   world.dat        — bincode: WorldSave + format-version footer (`save_format`)
//!   chunks/<cx>_<cy>_<cz>.chunk — raw binary: one non-empty chunk each
//!
//! The format is byte-identical to what the WASM client has always written, so
//! files are interchangeable between web and native builds.

use std::io::Read;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;

use crate::save::{WorldMeta, WorldSave, MAX_IMPORT_DECOMPRESSED_BYTES};
use crate::world::World;

/// SHA-256 (lowercase hex) of packed `.axeworld` bytes. The single content
/// fingerprint shared by the publish badge ([`crate::save::WorldMeta::publish_badge`]
/// via [`crate::save::PublishRecord::last_published_hash`]) and the publish
/// endpoint's anti-replay / anti-swap binding (publish build spec §3). Keeping
/// it next to `pack_world` means "the hash of an archive" has exactly one
/// definition across the engine and the console wire format.
///
/// FLAGGED GAP: `save::PublishRecord::last_published_hash` (the field this
/// function is meant to populate) is never assigned in production — only in
/// test literals — so the publish anti-replay/anti-swap binding this
/// docstring describes isn't wired up yet. Matches the tracked "Publish-from-game
/// loop" backlog (game UI + signing is the noted remaining piece). No
/// production caller; exercised by the tests below.
#[cfg_attr(not(test), allow(dead_code))]
pub fn archive_sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// An exhibit's image file, carried inside the archive as `exhibits/<ref>` so a
/// published / exported gallery world brings its art with it — the P2 gallery
/// fix (build spec §2.3). `image_ref` is the flat, sanitised filename; `bytes`
/// is the raw image. Sourced platform-specifically by the caller (native: the
/// world's `exhibits/` folder; web: the in-browser store) and persisted the same
/// way on unpack, which keeps `pack_world`/`unpack_world` cross-platform + pure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExhibitImage {
    pub image_ref: String,
    pub bytes: Vec<u8>,
}

/// Find the bytes for an exhibit `image_ref` among unpacked archive images
/// (#127). The record ref is sanitised the same way the archive members are
/// (`save::sanitize_image_ref`), so a record's "dni.jpg" matches the archive
/// member stored as the flat "dni.jpg". Returns `None` when the archive carried
/// no matching image — e.g. a server-published world whose blob omits it, where
/// the caller falls back to the same-origin `/exhibits/<ref>` fetch.
///
/// WASM-only in non-test builds (native renders exhibits from the on-disk
/// `exhibits/` folder); compiled under `test` too so the unit test runs natively.
#[cfg(any(target_arch = "wasm32", test))]
pub fn exhibit_bytes_for<'a>(images: &'a [ExhibitImage], image_ref: &str) -> Option<&'a [u8]> {
    let want = crate::save::sanitize_image_ref(image_ref);
    if want.is_empty() {
        return None;
    }
    images
        .iter()
        .find(|img| img.image_ref == want)
        .map(|img| img.bytes.as_slice())
}

/// Per-image cap when packing — mirrors the console's 12 MiB studio-upload
/// limit. An oversized image is skipped (with a warning) rather than producing
/// an absurd archive; unpack is separately bounded by the decompression gate.
const MAX_EXHIBIT_IMAGE_BYTES: usize = 12 * 1024 * 1024;

/// Pack a world for SHARING (export, replay, backup download): identical to
/// [`pack_world`] but with the Proof-of-Play `pop_secret` stripped. The secret
/// is host-only (Spec 06 §2.2): a shared file must never let its recipients
/// map the original host's Satori drops. Only the owner's own persistence
/// (the web IndexedDB blob and the encrypted Stash blob) keeps it — that path
/// calls [`pack_world`] directly.
pub fn pack_world_for_export(
    meta: &WorldMeta,
    save: &WorldSave,
    world: &World,
    exhibit_images: &[ExhibitImage],
) -> Result<Vec<u8>, String> {
    let mut shared = meta.clone();
    shared.pop_secret = None;
    pack_world(&shared, save, world, exhibit_images)
}

/// Unpack a world arriving from OUTSIDE (a `.axeworld` import, a web file
/// import, a `.axeprofile` world): identical to [`unpack_world`] but any
/// incoming `pop_secret` is discarded and a fresh random one generated, so an
/// imported world never shares the exporter's secret (Spec 06 §2.2).
pub fn unpack_world_for_import(
    compressed: &[u8],
    world: &mut World,
) -> Result<(WorldMeta, WorldSave, Vec<ExhibitImage>), String> {
    let (mut meta, save, images) = unpack_world(compressed, world)?;
    meta.pop_secret = Some(crate::proof_of_play::gen_world_secret());
    Ok((meta, save, images))
}

/// Re-pack an owner-persistence blob (which carries the secret) as a
/// shareable archive with the secret stripped. Used by the web Backup download
/// and the web `.axeprofile` export, which start from the raw IndexedDB blob.
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
pub fn strip_secret_from_archive(compressed: &[u8]) -> Result<Vec<u8>, String> {
    let mut scratch = World::new();
    let (meta, save, images) = unpack_world(compressed, &mut scratch)?;
    pack_world_for_export(&meta, &save, &scratch, &images)
}

/// Pack world state into a tar+gzip archive in memory.
///
/// Produces bytes that can be written to a `.axeworld` file.  The format is
/// byte-identical to the WASM export path, so files round-trip freely between
/// the browser and a native binary. `exhibit_images` are added as `exhibits/<ref>`
/// members so a gallery world's art travels with it (build spec §2.3); pass `&[]`
/// for worlds with no art.
pub fn pack_world(
    meta: &WorldMeta,
    save: &WorldSave,
    world: &World,
    exhibit_images: &[ExhibitImage],
) -> Result<Vec<u8>, String> {
    let gz_buf = Vec::new();
    let mut gz = GzEncoder::new(gz_buf, Compression::default());
    {
        let mut ar = tar::Builder::new(&mut gz);

        let meta_json = serde_json::to_vec_pretty(meta)
            .map_err(|e| format!("serialise meta: {e}"))?;
        let mut header = tar::Header::new_gnu();
        header.set_size(meta_json.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        ar.append_data(&mut header, "world_meta.json", &meta_json[..])
            .map_err(|e| format!("tar meta: {e}"))?;

        // bincode payload + format-version footer (Spec 02 §8.4).
        let world_dat = crate::save_format::encode_world_save(save)?;
        let mut header = tar::Header::new_gnu();
        header.set_size(world_dat.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        ar.append_data(&mut header, "world.dat", &world_dat[..])
            .map_err(|e| format!("tar world.dat: {e}"))?;

        // Spec 02 §7.5 — loaded + evicted chunks.
        for ((cx, cy, cz), chunk) in world.persistable_chunks() {
            if chunk.is_empty() {
                continue;
            }
            let bytes = chunk.as_bytes();
            let path = format!("chunks/{cx}_{cy}_{cz}.chunk");
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            ar.append_data(&mut header, &path, &bytes[..])
                .map_err(|e| format!("tar chunk: {e}"))?;
        }

        // Exhibit images (the gallery fix). Sanitise the ref into a flat
        // filename — `exhibits/<ref>` — and skip anything unsafe or oversized.
        for img in exhibit_images {
            let safe = crate::save::sanitize_image_ref(&img.image_ref);
            if safe.is_empty() {
                log::warn!("Skipping exhibit image with unsafe ref: {}", img.image_ref);
                continue;
            }
            if img.bytes.len() > MAX_EXHIBIT_IMAGE_BYTES {
                log::warn!(
                    "Skipping oversized exhibit image '{safe}' ({} bytes > {MAX_EXHIBIT_IMAGE_BYTES})",
                    img.bytes.len()
                );
                continue;
            }
            let path = format!("exhibits/{safe}");
            let mut header = tar::Header::new_gnu();
            header.set_size(img.bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            ar.append_data(&mut header, &path, &img.bytes[..])
                .map_err(|e| format!("tar exhibit image: {e}"))?;
        }

        ar.finish().map_err(|e| format!("tar finish: {e}"))?;
    }
    gz.finish().map_err(|e| format!("gzip finish: {e}"))
}

/// Unpack a tar+gzip archive into world state.
///
/// Accepts the bytes of a `.axeworld` file (produced by either the WASM client
/// or this native path) and populates `world` with the decoded chunks.
///
/// The decompression is bounded by [`MAX_IMPORT_DECOMPRESSED_BYTES`] to guard
/// against decompression-bomb attacks on imported files.
///
/// Lenient about chunks: a chunk member that fails to decode is skipped (the
/// archive file itself is untouched, so import, backup and replay lose nothing by
/// it). The web PLAY path uses [`unpack_world_to_play`] instead.
pub fn unpack_world(
    compressed: &[u8],
    world: &mut World,
) -> Result<(WorldMeta, WorldSave, Vec<ExhibitImage>), String> {
    unpack_world_inner(compressed, world, false)
}

/// [`unpack_world`] for a world about to be PLAYED from its only copy (the web's
/// IndexedDB record or cloud blob): a chunk member that fails to decode refuses
/// the whole world instead of being skipped. Skipping it would regenerate that
/// chunk and the next save would repack the record without the original — the
/// web has no side file to keep a damaged chunk in, so it is refused with nothing
/// written (Spec 02 §8.4, the load-failure rule).
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
pub fn unpack_world_to_play(
    compressed: &[u8],
    world: &mut World,
) -> Result<(WorldMeta, WorldSave, Vec<ExhibitImage>), String> {
    unpack_world_inner(compressed, world, true)
}

fn unpack_world_inner(
    compressed: &[u8],
    world: &mut World,
    strict_chunks: bool,
) -> Result<(WorldMeta, WorldSave, Vec<ExhibitImage>), String> {
    // Bounded gunzip — refuse a decompression bomb instead of OOMing the
    // process. The bounded tar transitively bounds every downstream entry +
    // bincode read (engine audit 2026-06-04, A: untrusted cloud/import decode).
    let tar_data = crate::save::read_bounded(
        GzDecoder::new(compressed),
        MAX_IMPORT_DECOMPRESSED_BYTES,
    )?;

    let mut ar = tar::Archive::new(&tar_data[..]);
    let mut meta: Option<WorldMeta> = None;
    let mut save: Option<WorldSave> = None;
    let mut images: Vec<ExhibitImage> = Vec::new();

    for entry in ar.entries().map_err(|e| format!("tar entries: {e}"))? {
        let mut entry = entry.map_err(|e| format!("tar entry: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("tar path: {e}"))?
            .to_string_lossy()
            .to_string();

        // Reject path traversal attempts in tar entries.
        if path.contains("..") || path.starts_with('/') {
            log::warn!("Skipping suspicious tar entry: {path}");
            continue;
        }

        let mut data = Vec::new();
        entry
            .read_to_end(&mut data)
            .map_err(|e| format!("read {path}: {e}"))?;

        if path == "world_meta.json" {
            meta = Some(
                serde_json::from_slice(&data)
                    .map_err(|e| format!("world_meta.json is damaged ({e})"))?,
            );
        } else if path == "world.dat" {
            // Tolerant decode shared with the native load paths. Cloud/Stash
            // blobs are byte-identical to local saves, so this protects
            // cloud-restored worlds from the same old-version data loss.
            save = Some(crate::save::read_world_save(&data).map_err(|e| {
                if e.is_newer_version() {
                    e.to_string()
                } else {
                    format!("world.dat is damaged ({e})")
                }
            })?);
        } else if path.starts_with("chunks/") && path.ends_with(".chunk") {
            let stem = path
                .strip_prefix("chunks/")
                .unwrap()
                .strip_suffix(".chunk")
                .unwrap();
            let parts: Vec<&str> = stem.split('_').collect();
            if parts.len() == 3 {
                let cx: i32 = parts[0].parse().map_err(|_| format!("{path} isn't a chunk file name"))?;
                let cy: i32 = parts[1].parse().map_err(|_| format!("{path} isn't a chunk file name"))?;
                let cz: i32 = parts[2].parse().map_err(|_| format!("{path} isn't a chunk file name"))?;
                match crate::chunk::Chunk::from_bytes(&data) {
                    Some(chunk) => world.insert_chunk(cx, cy, cz, chunk),
                    None if strict_chunks => return Err(format!("{path} is damaged")),
                    None => log::warn!("skipping damaged chunk {path} in archive"),
                }
            }
        } else if let Some(rest) = path.strip_prefix("exhibits/") {
            // Exhibit image (the gallery fix). The traversal guard above already
            // rejected `..` / leading `/`; sanitise the remainder into a flat
            // filename as belt-and-braces so a crafted ref can't escape the
            // world's own exhibits/ folder when the caller persists it.
            let safe = crate::save::sanitize_image_ref(rest);
            if safe.is_empty() {
                log::warn!("Skipping suspicious exhibit image entry: {path}");
            } else {
                images.push(ExhibitImage { image_ref: safe, bytes: data });
            }
        }
    }

    let meta = meta.ok_or("world_meta.json is missing")?;
    let save = save.ok_or("world.dat is missing")?;
    Ok((meta, save, images))
}

/// Pick a stored-world name that doesn't collide with `existing`. If `name` is
/// free, return it unchanged. Otherwise append `" (imported)"`, then
/// `" (imported 2)"`, `" (imported 3)"`, … until one is free. Never overwrites.
pub fn dedupe_world_name(name: &str, existing: &[String]) -> String {
    if !existing.iter().any(|e| e == name) {
        return name.to_string();
    }
    let first = format!("{name} (imported)");
    if !existing.iter().any(|e| e == &first) {
        return first;
    }
    for n in 2.. {
        let candidate = format!("{name} (imported {n})");
        if !existing.iter().any(|e| e == &candidate) {
            return candidate;
        }
    }
    unreachable!("infinite range always yields a free name")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;

    #[test]
    fn exhibit_bytes_for_matches_sanitised_ref() {
        // #127 — the WASM load path looks images up by exhibit `image_ref`
        // among the archive's unpacked members, sanitising the record ref the
        // same way native does (`save::sanitize_image_ref`).
        let images = vec![ExhibitImage {
            image_ref: "dni.jpg".to_string(),
            bytes: vec![1, 2, 3],
        }];
        assert_eq!(exhibit_bytes_for(&images, "dni.jpg"), Some(&[1u8, 2, 3][..]));
        // A record ref carrying junk chars sanitises to the same flat name.
        assert_eq!(exhibit_bytes_for(&images, "d!n@i.jpg"), Some(&[1u8, 2, 3][..]));
        // No matching member → None (caller falls back to the server fetch).
        assert_eq!(exhibit_bytes_for(&images, "missing.png"), None);
        // Traversal junk sanitises to empty → None.
        assert_eq!(exhibit_bytes_for(&images, ".."), None);
    }

    #[test]
    fn archive_sha256_hex_is_deterministic_and_correct() {
        // The publish badge + the publish endpoint's anti-replay binding both
        // depend on this being a stable, standard SHA-256 hex digest.
        let a = archive_sha256_hex(b"hello world");
        assert_eq!(a, archive_sha256_hex(b"hello world"));
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        // Known SHA-256 of the literal "hello world".
        assert_eq!(a, "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9");
        // Any change in bytes changes the digest (anti-swap).
        assert_ne!(archive_sha256_hex(b"hello worle"), a);
    }

    // -----------------------------------------------------------------------
    // dedupe_world_name (moved from wasm_save — these continue to run on
    // native and are the canonical location for the function's tests)
    // -----------------------------------------------------------------------

    #[test]
    fn no_collision_returns_unchanged() {
        let existing = vec!["Other".to_string(), "World".to_string()];
        assert_eq!(dedupe_world_name("Castle", &existing), "Castle");
    }

    #[test]
    fn one_collision_appends_imported() {
        let existing = vec!["Castle".to_string()];
        assert_eq!(dedupe_world_name("Castle", &existing), "Castle (imported)");
    }

    #[test]
    fn chained_collisions_increment() {
        let existing = vec![
            "Castle".to_string(),
            "Castle (imported)".to_string(),
        ];
        assert_eq!(dedupe_world_name("Castle", &existing), "Castle (imported 2)");
    }

    // -----------------------------------------------------------------------
    // pack_world / unpack_world cross-platform round-trip
    // -----------------------------------------------------------------------

    /// Build the smallest valid WorldSave that bincode will accept.
    fn minimal_world_save(seed: u32) -> WorldSave {
        WorldSave {
            seed,
            player_x: 0.0,
            player_y: 64.0,
            player_z: 0.0,
            player_health: 20.0,
            hotbar_slot: 0,
            inventory: Vec::new(),
            players: Vec::new(),
            campfires: Vec::new(),
            furnaces: Vec::new(),
            vendors: Vec::new(),
            drying_racks: Vec::new(),
            hives: Vec::new(),
            chests: Vec::new(),
            tip_jars: Vec::new(),
            plots: Vec::new(),
            market_hubs: Vec::new(),
            auctions: Vec::new(),
            latent_prints: Vec::new(),
            construction_anchors: Vec::new(),
            architect_plaques: Vec::new(),
            village_anchors: Vec::new(),
            populated_villages: Vec::new(),
            village_bells: Vec::new(),
            village_treasuries: Vec::new(),
            active_raids: Vec::new(),
            raid_scheduler: crate::raid::RaidScheduler::new(),
            raid_kills: Vec::new(),
            brigand_hideouts: Vec::new(),
            bounties: Vec::new(),
            bounty_next_id: 0,
            bounty_last_refresh_tick: 0,
            face_overlays: Vec::new(),
            face_blueprints: Vec::new(),
            face_blueprint_blanks: Vec::new(),
            workshop: Default::default(),
            carts: Vec::new(),
            graves: Vec::new(),
            waypoints: Vec::new(),
            block_meta: Vec::new(),
            power_devices: Vec::new(),
            signs: Vec::new(),
            item_frames: Vec::new(),
            locked_slots: Vec::new(),
            hostile_acts: Vec::new(),
            rigs: Vec::new(),
            exhibits: Vec::new(),
            composters: Vec::new(),
            saved_mobs: Vec::new(),
            satoshi: Default::default(),
            dispensers: Vec::new(),
            rig_clips: Vec::new(),
        }
    }

    #[test]
    fn export_carries_no_pop_secret() {
        let world = World::new();
        let mut meta = WorldMeta::new("secret-x");
        meta.pop_secret = Some([7u8; 32]);
        let save = minimal_world_save(1);
        let bytes = pack_world_for_export(&meta, &save, &world, &[]).expect("pack");
        let mut fresh = World::new();
        let (back, _, _) = unpack_world(&bytes, &mut fresh).expect("unpack");
        assert_eq!(back.pop_secret, None, "an exported archive must not carry the secret");
        // Owner persistence (pack_world) still keeps it.
        let owner = pack_world(&meta, &save, &world, &[]).expect("pack");
        let (kept, _, _) = unpack_world(&owner, &mut World::new()).expect("unpack");
        assert_eq!(kept.pop_secret, Some([7u8; 32]));
        // Stripping an owner blob removes it.
        let stripped = strip_secret_from_archive(&owner).expect("strip");
        let (s, _, _) = unpack_world(&stripped, &mut World::new()).expect("unpack");
        assert_eq!(s.pop_secret, None);
    }

    #[test]
    fn import_discards_incoming_secret_and_gets_a_fresh_one() {
        let world = World::new();
        let mut meta = WorldMeta::new("secret-y");
        meta.pop_secret = Some([9u8; 32]);
        let save = minimal_world_save(1);
        // Even a (hand-crafted / pre-fix) archive that still carries a secret.
        let bytes = pack_world(&meta, &save, &world, &[]).expect("pack");
        let (imp, _, _) = unpack_world_for_import(&bytes, &mut World::new()).expect("unpack");
        let got = imp.pop_secret.expect("an import is given a fresh secret");
        assert_ne!(got, [9u8; 32], "an import must not keep the source's secret");
        // A stripped archive also gets one.
        let exp = pack_world_for_export(&meta, &save, &world, &[]).expect("pack");
        let (imp2, _, _) = unpack_world_for_import(&exp, &mut World::new()).expect("unpack");
        assert!(imp2.pop_secret.is_some());
        assert_ne!(imp2.pop_secret, Some([9u8; 32]));
    }

    #[test]
    fn round_trip_preserves_blocks_and_meta() {
        // Build a small world with a few known blocks at distinct coords.
        let mut world = World::new();
        world.set_block(0, 0, 0, block::STONE);
        world.set_block(1, 0, 0, block::DIRT);
        world.set_block(0, 1, 0, block::GRASS);
        // A block in a different chunk to exercise multi-chunk archiving.
        world.set_block(16, 0, 0, block::COAL_ORE);

        let meta = WorldMeta::new("rt-test");
        let save = minimal_world_save(42);

        // Pack → Vec<u8>.
        let bytes = pack_world(&meta, &save, &world, &[])
            .expect("pack_world failed");
        assert!(!bytes.is_empty(), "archive should be non-empty");

        // Unpack into a fresh world.
        let mut fresh = World::new();
        let (meta2, save2, _images) = unpack_world(&bytes, &mut fresh)
            .expect("unpack_world failed");

        // Check metadata survived the round-trip.
        assert_eq!(meta2.display_name, "rt-test");
        assert_eq!(save2.seed, 42);

        // Check blocks in the first chunk.
        assert_eq!(fresh.get_block(0, 0, 0), block::STONE,
            "STONE at (0,0,0) must survive round-trip");
        assert_eq!(fresh.get_block(1, 0, 0), block::DIRT,
            "DIRT at (1,0,0) must survive round-trip");
        assert_eq!(fresh.get_block(0, 1, 0), block::GRASS,
            "GRASS at (0,1,0) must survive round-trip");
        // An unset position must be AIR.
        assert_eq!(fresh.get_block(5, 5, 5), block::AIR,
            "unset block must unpack as AIR");

        // Check the block in the second chunk.
        assert_eq!(fresh.get_block(16, 0, 0), block::COAL_ORE,
            "COAL_ORE at (16,0,0) (different chunk) must survive round-trip");
    }

    #[test]
    fn exhibits_survive_pack_unpack_round_trip() {
        use crate::exhibit::{Exhibit, Presentation};
        let world = World::new();
        let meta = WorldMeta::new("gallery-rt");
        let mut save = minimal_world_save(7);
        save.exhibits = vec![Exhibit {
            x: 4,
            y: 65,
            z: -2,
            presentation: Presentation::Standing,
            image_ref: "statue.png".to_string(),
            width: 1.5,
            height: 3.0,
            yaw: 0.78,
            label: "The Sentinel".to_string(),
            link: None,
            sku: None,
            price: None,
        }];

        // The gallery fix (build spec §2.3): the exhibit's IMAGE bytes travel
        // inside the archive, not just the placement.
        let img_bytes = b"\x89PNG\r\n\x1a\n fake image payload".to_vec();
        let images = vec![ExhibitImage {
            image_ref: "statue.png".to_string(),
            bytes: img_bytes.clone(),
        }];

        let bytes = pack_world(&meta, &save, &world, &images).expect("pack");
        let mut fresh = World::new();
        let (_, save2, images2) = unpack_world(&bytes, &mut fresh).expect("unpack");

        assert_eq!(save2.exhibits.len(), 1, "exhibit must survive round-trip");
        let e = &save2.exhibits[0];
        assert_eq!(e.presentation, Presentation::Standing);
        assert_eq!(e.image_ref, "statue.png");
        assert_eq!(e.label, "The Sentinel");
        assert!((e.width - 1.5).abs() < 1e-6 && (e.height - 3.0).abs() < 1e-6);

        // The image file came back byte-for-byte under its ref.
        assert_eq!(images2.len(), 1, "exhibit image must survive round-trip");
        assert_eq!(images2[0].image_ref, "statue.png");
        assert_eq!(images2[0].bytes, img_bytes, "image bytes must be identical");
    }

    #[test]
    fn pack_skips_exhibit_image_with_unsafe_ref() {
        // A traversal-style ref must not produce an escaping tar member; it's
        // sanitised to empty and skipped on pack.
        let world = World::new();
        let meta = WorldMeta::new("g");
        let save = minimal_world_save(1);
        let images = vec![ExhibitImage {
            image_ref: "../../etc/passwd".to_string(),
            bytes: b"nope".to_vec(),
        }];
        let bytes = pack_world(&meta, &save, &world, &images).expect("pack");
        let mut fresh = World::new();
        let (_, _, images2) = unpack_world(&bytes, &mut fresh).expect("unpack");
        assert!(images2.is_empty(), "unsafe-ref image must be dropped, not carried");
    }

    #[test]
    fn empty_world_round_trips() {
        // An empty world should still pack and unpack cleanly (no chunks emitted).
        let world = World::new();
        let meta = WorldMeta::new("empty");
        let save = minimal_world_save(0);

        let bytes = pack_world(&meta, &save, &world, &[]).expect("pack empty world");
        let mut fresh = World::new();
        let (meta2, _, _) = unpack_world(&bytes, &mut fresh).expect("unpack empty world");
        assert_eq!(meta2.display_name, "empty");
        // All blocks default to AIR.
        assert_eq!(fresh.get_block(0, 0, 0), block::AIR);
    }

    /// A shared world's exhibits AND their image bytes survive the real
    /// export → import pair (`pack_world_for_export` → `unpack_world_for_import`),
    /// which is how an external world pack (e.g. the art Gallery `.axeworld`)
    /// reaches a player. Two images, one reused by two exhibits.
    #[test]
    fn exhibit_world_survives_export_then_import() {
        use crate::exhibit::{Exhibit, Presentation};
        let mut world = World::new();
        world.set_block(3, 80, 0, block::SANDSTONE);
        let mut meta = WorldMeta::new("two-piece-show");
        meta.game_mode = "adventure".into();
        meta.time_lock = "day".into();
        meta.mobs_enabled = false;
        let mut save = minimal_world_save(11);
        let wall = |x: i32, image_ref: &str, label: &str| Exhibit {
            x,
            y: 81,
            z: 0,
            presentation: Presentation::Wall,
            image_ref: image_ref.to_string(),
            width: 1.7,
            height: 1.7,
            yaw: std::f32::consts::FRAC_PI_2,
            label: label.to_string(),
            link: None,
            sku: None,
            price: None,
        };
        save.exhibits = vec![
            wall(4, "one.jpg", "One - Artist A"),
            wall(4, "one-plaque.jpg", ""),
            wall(6, "two.jpg", "Two - Artist B"),
        ];
        let one: Vec<u8> = (0..40_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let two: Vec<u8> = (0..90_000u32).map(|i| (i * 13 % 241) as u8).collect();
        let images = vec![
            ExhibitImage { image_ref: "one.jpg".into(), bytes: one.clone() },
            ExhibitImage { image_ref: "one-plaque.jpg".into(), bytes: two.clone() },
            ExhibitImage { image_ref: "two.jpg".into(), bytes: two.clone() },
        ];

        let bytes = pack_world_for_export(&meta, &save, &world, &images).expect("export");
        let mut fresh = World::new();
        let (meta2, save2, images2) =
            unpack_world_for_import(&bytes, &mut fresh).expect("import");

        assert_eq!(save2.exhibits, save.exhibits, "exhibits must be identical");
        assert_eq!(images2, images, "image bytes must be identical, in order");
        for e in &save2.exhibits {
            assert!(
                exhibit_bytes_for(&images2, &e.image_ref).is_some(),
                "{} does not resolve",
                e.image_ref
            );
        }
        assert_eq!(meta2.game_mode, "adventure");
        assert_eq!(meta2.time_lock, "day");
        assert!(!meta2.mobs_enabled);
        assert_eq!(fresh.get_block(3, 80, 0), block::SANDSTONE);
    }

    // ── world.dat format-version footer (gap-audit T1-7, Spec 02 §8.4) ──

    /// The raw bytes of one archive member.
    fn archive_member(blob: &[u8], name: &str) -> Vec<u8> {
        let tar = crate::save::read_bounded(GzDecoder::new(blob), MAX_IMPORT_DECOMPRESSED_BYTES)
            .unwrap();
        let mut ar = tar::Archive::new(&tar[..]);
        for entry in ar.entries().unwrap() {
            let mut entry = entry.unwrap();
            if entry.path().unwrap().to_string_lossy() == name {
                let mut out = Vec::new();
                entry.read_to_end(&mut out).unwrap();
                return out;
            }
        }
        panic!("no {name} in archive");
    }

    /// A `.axeworld` holding `world_dat` verbatim (e.g. one from a newer build).
    fn archive_with_world_dat(meta: &WorldMeta, world_dat: &[u8]) -> Vec<u8> {
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut ar = tar::Builder::new(&mut gz);
            for (path, bytes) in [
                ("world_meta.json", serde_json::to_vec(meta).unwrap()),
                ("world.dat", world_dat.to_vec()),
            ] {
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                ar.append_data(&mut header, path, &bytes[..]).unwrap();
            }
            ar.finish().unwrap();
        }
        gz.finish().unwrap()
    }

    #[test]
    fn pack_world_writes_the_footer() {
        let blob = pack_world(&WorldMeta::new("f"), &minimal_world_save(5), &World::new(), &[])
            .unwrap();
        let dat = archive_member(&blob, "world.dat");
        assert_eq!(
            crate::save_format::split_save_footer(&dat).1,
            Some(crate::save_format::SAVE_FORMAT_VERSION)
        );
    }

    /// Web, cloud, `.axeworld` and `.axeprofile` imports all unpack through here:
    /// a world from a newer build is refused with the lobby message, and a
    /// footer-less archive from an older build still opens.
    #[test]
    fn unpack_refuses_a_world_from_a_newer_build() {
        let meta = WorldMeta::new("n");
        let mut newer = bincode::serialize(&minimal_world_save(5)).unwrap();
        newer.extend_from_slice(&crate::save_format::footer_bytes(
            crate::save_format::SAVE_FORMAT_VERSION + 1,
        ));
        let blob = archive_with_world_dat(&meta, &newer);
        let err = unpack_world(&blob, &mut World::new()).err().expect("refused");
        assert!(crate::save_format::is_newer_world_error(&err), "{err}");
        let err = unpack_world_for_import(&blob, &mut World::new()).err().expect("refused");
        assert!(crate::save_format::is_newer_world_error(&err), "{err}");

        let legacy = archive_with_world_dat(&meta, &bincode::serialize(&minimal_world_save(6)).unwrap());
        let (_, save, _) = unpack_world(&legacy, &mut World::new()).expect("footer-less loads");
        assert_eq!(save.seed, 6);
    }

    /// The web PLAY path (Spec 02 §8.4): a damaged chunk refuses the world, since
    /// skipping it would regenerate that chunk and the next save would repack the
    /// record without the original. Import / backup / replay stay lenient — the
    /// archive file itself is untouched there.
    #[test]
    fn play_unpack_refuses_a_damaged_chunk_that_import_skips() {
        let mut good = World::new();
        good.set_block(3, 64, 5, block::BEDROCK);
        let packed = pack_world(&WorldMeta::new("d"), &minimal_world_save(5), &good, &[]).unwrap();
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut ar = tar::Builder::new(&mut gz);
            for (path, bytes) in [
                ("world_meta.json", archive_member(&packed, "world_meta.json")),
                ("world.dat", archive_member(&packed, "world.dat")),
                ("chunks/0_4_0.chunk", archive_member(&packed, "chunks/0_4_0.chunk")),
                ("chunks/1_4_1.chunk", vec![7u8; 100]),
            ] {
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                ar.append_data(&mut header, path, &bytes[..]).unwrap();
            }
            ar.finish().unwrap();
        }
        let blob = gz.finish().unwrap();

        let err = unpack_world_to_play(&blob, &mut World::new()).err().expect("refused");
        assert_eq!(err, "chunks/1_4_1.chunk is damaged");
        let mut lenient = World::new();
        unpack_world(&blob, &mut lenient).expect("import skips the damaged chunk");
        assert_eq!(lenient.get_block(3, 64, 5), block::BEDROCK);
        // An intact archive plays.
        unpack_world_to_play(&packed, &mut World::new()).expect("an intact world plays");
    }
}
