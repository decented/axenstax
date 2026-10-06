//! Native-only world export / import helpers.
//!
//! These functions bridge the cross-platform `world_archive` tar+gzip format to
//! on-disk world folders managed by `save.rs`.  They compile only on native
//! targets because they use `std::fs` directly.
//!
//! API surface:
//!   - [`export_world_native`] — pack a worlds/<name>/ folder into `.axeworld` bytes.
//!   - [`import_world_native`] — unpack `.axeworld` bytes into a new worlds/<…>/ folder.

#[cfg(not(target_arch = "wasm32"))]
use crate::save::{
    exhibit_image_path, load_world, load_world_meta, sanitize_folder_name, sanitize_image_ref,
    slugify_folder_name, worlds_root, write_world_folder, WorldMeta, WorldSave,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::world::World;
#[cfg(not(target_arch = "wasm32"))]
use crate::world_archive::{
    dedupe_world_name, pack_world_for_export, unpack_world_for_import, ExhibitImage,
};

/// Read an exported world's exhibit images off disk (`worlds/<name>/exhibits/<ref>`)
/// so they travel inside the `.axeworld` archive (the gallery fix, build spec §2.3).
/// Dedupes by ref (several exhibits may reuse one image) and silently skips refs
/// that don't resolve to a readable file.
#[cfg(not(target_arch = "wasm32"))]
fn read_exhibit_images_from_disk(name: &str, save: &WorldSave) -> Vec<ExhibitImage> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for ex in &save.exhibits {
        let safe = sanitize_image_ref(&ex.image_ref);
        if safe.is_empty() || !seen.insert(safe.clone()) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(exhibit_image_path(name, &safe)) {
            out.push(ExhibitImage { image_ref: safe, bytes });
        }
    }
    out
}

/// Pack a saved world folder into a `.axeworld` byte blob.
///
/// The world is loaded from `worlds/<name>/` via the normal `load_world` /
/// `load_world_meta` path and then passed to `world_archive::pack_world`, which
/// produces the same tar+gzip format used by the WASM client.
///
/// The returned bytes can be written directly to a file or transmitted across
/// the network; the format is byte-identical to what the browser export path
/// produces, so files round-trip freely between web and native builds.
#[cfg(not(target_arch = "wasm32"))]
pub fn export_world_native(name: &str) -> Result<Vec<u8>, String> {
    let meta = load_world_meta(name);
    let mut scratch = World::new();
    let (save, _loaded_chunks) = load_world(name, &mut scratch)?;
    let images = read_exhibit_images_from_disk(name, &save);
    // Every caller shares the result (Save-As, world-transfer folder, replay
    // snapshot, publish), so the host-only PoP secret is stripped (Spec 06 §2.2).
    pack_world_for_export(&meta, &save, &scratch, &images)
}

/// Write an unpacked archive into `worlds/<final_name>/`.
///
/// Shared by every import path (single `.axeworld` file, folder scan, and the
/// `.axeprofile` bundle) so they can't drift. When the folder name had to be
/// changed to avoid a collision, the display name follows it — otherwise the
/// world list would show two identically-named worlds sitting in different
/// folders.
#[cfg(not(target_arch = "wasm32"))]
fn write_unpacked_world(
    final_name: &str,
    base_slug: &str,
    meta: WorldMeta,
    save: &WorldSave,
    world: &World,
    images: &[ExhibitImage],
) -> Result<(), String> {
    // Belt and braces behind the collision set: an import only ever creates a
    // folder, never writes into one that is there.
    let dir = crate::save::world_dir(final_name);
    if dir.exists() {
        return Err(format!("{} already exists; not importing over it", dir.display()));
    }
    let mut meta2 = meta;
    if final_name != base_slug {
        meta2.display_name = final_name.to_string();
    }
    // The folder is this import's own (checked above): one that failed part-way
    // is removed, never left in the lobby with chunks missing (review 2026-10-06).
    if let Err(why) = write_world_folder(final_name, &meta2, save, world) {
        if let Err(e) = std::fs::remove_dir_all(&dir)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::error!("a failed import's folder {} couldn't be removed: {e}", dir.display());
        }
        return Err(why);
    }

    // Persist the exhibit images that travelled inside the archive into the new
    // world's `exhibits/` folder (the gallery fix, build spec §2.3).
    for img in images {
        let path = exhibit_image_path(final_name, &img.image_ref);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&path, &img.bytes) {
            log::warn!("Failed to write imported exhibit image '{}': {e}", img.image_ref);
        }
    }
    Ok(())
}

/// Every name already taken under `worlds_root()` — the collision set an import
/// names against. EVERY entry counts, not just the lobby's world cards
/// (`list_world_entries` lists only folders holding a `world.dat` and skips the
/// Workshop): naming against that list let an import write over the native
/// Workshop, a folder holding only chunks or a crash-recovery autosave, or a
/// stray file (Spec 02 §8.4). `Err` when the folder can't be listed — then no
/// name is known to be free and the import is refused.
#[cfg(not(target_arch = "wasm32"))]
fn existing_world_folders() -> Result<Vec<String>, String> {
    let root = worlds_root();
    let entries = match std::fs::read_dir(&root) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{} can't be listed ({e})", root.display())),
    };
    entries
        .map(|e| {
            e.map(|e| e.file_name().to_string_lossy().into_owned())
                .map_err(|e| format!("{} can't be listed ({e})", root.display()))
        })
        .collect()
}

/// Unpack a `.axeworld` byte blob into a new world folder.
///
/// The folder name is derived by sanitising the archive's `display_name` into a
/// filesystem-safe slug (via [`sanitize_folder_name`]) and then deduplicating it
/// against existing world folder names (via [`dedupe_world_name`]).  The
/// deduplication appends `" (imported)"`, `" (imported 2)"`, … so an import
/// never silently overwrites an existing world.
///
/// Returns the final folder name (i.e. the key to pass to `load_world` /
/// `load_world_meta` afterwards).
#[cfg(not(target_arch = "wasm32"))]
pub fn import_world_native(bytes: &[u8]) -> Result<String, String> {
    // Unpack archive into a scratch World.
    let mut world = World::new();
    let (meta, save, images) = unpack_world_for_import(bytes, &mut world)?;

    // Derive a filesystem-safe folder name from the archive's display name.
    let base_slug = sanitize_folder_name(&meta.display_name);
    let final_name = dedupe_world_name(&base_slug, &existing_world_folders()?);

    let display = meta.display_name.clone();
    write_unpacked_world(&final_name, &base_slug, meta, &save, &world, &images)?;

    log::info!(
        "Imported world '{display}' as folder '{final_name}' ({} exhibit image(s))",
        images.len()
    );
    Ok(final_name)
}

// ---------------------------------------------------------------------------
// `.axeprofile` — whole-profile import ("Take your worlds to native")
// ---------------------------------------------------------------------------

/// Name an incoming web world so it can never displace one already here.
///
/// If `base` is free it is used as-is; otherwise the world lands as
/// `"<base> (web)"`, then `"<base> (web 2)"`, `"<base> (web 3)"`, … Pure, so
/// the rule is testable without touching the filesystem.
///
/// INTERIM RULE — owner decision pending (roadmap "Conflict / merge"). Keeping
/// both copies is the only choice that can't lose a child's work; picking a
/// winner (newest wins / merge / prompt) is a design call, not an
/// implementation detail, so it is deliberately not made here.
#[cfg(not(target_arch = "wasm32"))]
pub fn keep_both_name(base: &str, existing: &[String]) -> String {
    if !existing.iter().any(|e| e == base) {
        return base.to_string();
    }
    let first = format!("{base} (web)");
    if !existing.iter().any(|e| e == &first) {
        return first;
    }
    for n in 2.. {
        let candidate = format!("{base} (web {n})");
        if !existing.iter().any(|e| e == &candidate) {
            return candidate;
        }
    }
    unreachable!("infinite range always yields a free name")
}

/// Import one World entry out of a `.axeprofile` bundle.
///
/// `entry_name` is the name the world had in the browser — that, not the
/// archive's display name, is what the player recognises, so it seeds the
/// folder slug. Returns `(final_folder_name, was_renamed)`; `was_renamed` is
/// true when the keep-both rule had to step aside from an existing world.
///
/// Note this deliberately slugifies with [`slugify_folder_name`] rather than
/// `sanitize_folder_name`: the latter's own `_2` collision suffix would fire
/// first and the keep-both `(web)` naming would never be reached.
#[cfg(not(target_arch = "wasm32"))]
pub fn import_profile_world(bytes: &[u8], entry_name: &str) -> Result<(String, bool), String> {
    let mut world = World::new();
    let (meta, save, images) = unpack_world_for_import(bytes, &mut world)?;

    let base_slug = slugify_folder_name(entry_name);
    let final_name = keep_both_name(&base_slug, &existing_world_folders()?);
    let renamed = final_name != base_slug;

    write_unpacked_world(&final_name, &base_slug, meta, &save, &world, &images)?;
    Ok((final_name, renamed))
}

/// What a `.axeprofile` import did, for the summary line the lobby shows.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileImportSummary {
    /// Worlds written (including any that had to be renamed).
    pub imported: usize,
    /// How many of those landed under a `(web)` name because the original was taken.
    pub renamed: usize,
    /// Worlds in the bundle that could not be unpacked or written.
    pub failed: usize,
    /// Whether the bundle carried Trials records that were merged in.
    pub trials_merged: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl ProfileImportSummary {
    /// The one-line lobby toast. Counts are always stated — a silent skip is
    /// exactly the failure mode the keep-both rule exists to avoid.
    pub fn message(&self) -> String {
        let mut parts = Vec::new();
        parts.push(match self.imported {
            0 => "No worlds imported".to_string(),
            1 => "Imported 1 world".to_string(),
            n => format!("Imported {n} worlds"),
        });
        if self.renamed > 0 {
            parts.push(format!(
                "{} kept alongside a world of the same name (renamed \"(web)\")",
                self.renamed
            ));
        }
        if self.trials_merged {
            parts.push("Trials records merged".to_string());
        }
        if self.failed > 0 {
            parts.push(format!("{} could not be read", self.failed));
        }
        parts.join(" · ")
    }
}

/// Apply a `.axeprofile` bundle: every World entry becomes a world folder
/// (keep-both naming), and a Trials entry is merged into the local records
/// keeping the better of each.
///
/// Purely local — reads bytes the player picked off their own disk and writes
/// into `worlds/` and `profile/trials.json`. No network, no identity.
#[cfg(not(target_arch = "wasm32"))]
pub fn import_profile_native(bytes: &[u8]) -> Result<ProfileImportSummary, String> {
    use crate::profile_bundle::{unpack, EntryKind};

    let entries = unpack(bytes).map_err(|e| e.to_string())?;
    let mut summary = ProfileImportSummary::default();

    for entry in &entries {
        match entry.kind {
            EntryKind::World => match import_profile_world(&entry.bytes, &entry.name) {
                Ok((name, renamed)) => {
                    summary.imported += 1;
                    if renamed {
                        summary.renamed += 1;
                    }
                    log::info!("Profile import: world '{}' → folder '{name}'", entry.name);
                }
                Err(e) => {
                    summary.failed += 1;
                    log::warn!("Profile import: world '{}' failed: {e}", entry.name);
                }
            },
            EntryKind::Trials => {
                let incoming = crate::trials::TrialBests::from_json(
                    std::str::from_utf8(&entry.bytes).unwrap_or("{}"),
                );
                let mut local = crate::trials::TrialBests::load();
                local.merge(&incoming);
                local.save();
                summary.trials_merged = true;
            }
        }
    }
    Ok(summary)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::save::{load_world, world_dir};
    use crate::world::World;

    /// Round-trip through disk: build a world → export → import → reload and
    /// assert that the blocks survived.
    ///
    /// Uses a unique world name with a fixed nonce so parallel test runs don't
    /// collide.  Both temp folders are cleaned up at the end.
    #[test]
    fn disk_round_trip_preserves_blocks() {
        // Use a name unlikely to collide with any real save; the nonce is fixed
        // so test runs are deterministic (no timestamp).
        let src_name = "__nwio_rt_src_8a3f7c";

        // ----------------------------------------------------------------
        // Build a source world with a few distinct blocks.
        // ----------------------------------------------------------------
        let mut world = World::new();
        world.set_block(0, 0, 0, block::STONE);
        world.set_block(1, 0, 0, block::DIRT);
        world.set_block(0, 1, 0, block::GRASS);
        // A block in a separate chunk to exercise multi-chunk export.
        world.set_block(16, 0, 0, block::COAL_ORE);

        // Build a minimal WorldSave so we have something to write to disk.
        let meta = crate::save::WorldMeta::new(src_name);
        let save = crate::save::minimal_world_save_for_tests(77);

        // Write the source world folder directly (no PlayerSlots needed).
        write_world_folder(src_name, &meta, &save, &world)
            .expect("write_world_folder (source) failed");

        // ----------------------------------------------------------------
        // Export the source world to bytes.
        // ----------------------------------------------------------------
        let bytes = export_world_native(src_name).expect("export_world_native failed");
        assert!(!bytes.is_empty(), "exported bytes must be non-empty");

        // ----------------------------------------------------------------
        // Import the bytes back as a NEW world.
        // ----------------------------------------------------------------
        let imported_name = import_world_native(&bytes).expect("import_world_native failed");

        // PoP secret (Spec 06 §2.2): the export carries none; the import gets a
        // fresh one that differs from the source's.
        let src_secret = meta.pop_secret.expect("a new world has a secret");
        let (exp_meta, _, _) =
            crate::world_archive::unpack_world(&bytes, &mut World::new()).expect("unpack");
        assert_eq!(exp_meta.pop_secret, None, "an .axeworld export must not carry the secret");
        let imp_secret = load_world_meta(&imported_name)
            .pop_secret
            .expect("an imported world is given a fresh secret");
        assert_ne!(imp_secret, src_secret, "an import must not keep the source's secret");

        // The import must NOT have clobbered the source folder.
        assert_ne!(
            imported_name, src_name,
            "import must land in a different folder than the source"
        );

        // ----------------------------------------------------------------
        // Reload from the imported folder and assert block survival.
        // ----------------------------------------------------------------
        let mut reloaded = World::new();
        let (reloaded_save, _) =
            load_world(&imported_name, &mut reloaded).expect("load_world (imported) failed");

        assert_eq!(reloaded_save.seed, 77, "seed must survive round-trip");

        assert_eq!(
            reloaded.get_block(0, 0, 0),
            block::STONE,
            "STONE at (0,0,0) must survive disk round-trip"
        );
        assert_eq!(
            reloaded.get_block(1, 0, 0),
            block::DIRT,
            "DIRT at (1,0,0) must survive disk round-trip"
        );
        assert_eq!(
            reloaded.get_block(0, 1, 0),
            block::GRASS,
            "GRASS at (0,1,0) must survive disk round-trip"
        );
        assert_eq!(
            reloaded.get_block(16, 0, 0),
            block::COAL_ORE,
            "COAL_ORE at (16,0,0) (second chunk) must survive disk round-trip"
        );
        assert_eq!(
            reloaded.get_block(5, 5, 5),
            block::AIR,
            "unset position must reload as AIR"
        );

        // ----------------------------------------------------------------
        // Clean up both temp folders so the test is hermetic.
        // ----------------------------------------------------------------
        let _ = std::fs::remove_dir_all(world_dir(src_name));
        let _ = std::fs::remove_dir_all(world_dir(&imported_name));
    }

    // ── `.axeprofile` keep-both naming (INTERIM RULE) ──

    #[test]
    fn keep_both_leaves_a_free_name_alone() {
        let existing = vec!["other".to_string()];
        assert_eq!(keep_both_name("my_world", &existing), "my_world");
    }

    #[test]
    fn keep_both_never_overwrites_and_never_skips() {
        // First collision → "(web)". Then "(web 2)", "(web 3)", … Each returned
        // name is new, so an import always lands somewhere.
        let mut existing = vec!["my_world".to_string()];
        let first = keep_both_name("my_world", &existing);
        assert_eq!(first, "my_world (web)");
        existing.push(first);

        let second = keep_both_name("my_world", &existing);
        assert_eq!(second, "my_world (web 2)");
        existing.push(second);

        let third = keep_both_name("my_world", &existing);
        assert_eq!(third, "my_world (web 3)");
        assert!(!existing.contains(&third), "the chosen name must be free");
    }

    #[test]
    fn keep_both_skips_a_gap_it_did_not_make() {
        // "(web)" already taken but "(web 2)" free → take "(web 2)".
        let existing = vec!["w".to_string(), "w (web)".to_string()];
        assert_eq!(keep_both_name("w", &existing), "w (web 2)");
    }

    #[test]
    fn profile_summary_message_states_every_count() {
        let s = ProfileImportSummary {
            imported: 3,
            renamed: 1,
            failed: 1,
            trials_merged: true,
        };
        let msg = s.message();
        assert!(msg.contains("Imported 3 worlds"), "{msg}");
        assert!(msg.contains("1 kept alongside"), "renamed count must be stated: {msg}");
        assert!(msg.contains("Trials records merged"), "{msg}");
        assert!(msg.contains("1 could not be read"), "failures must never be silent: {msg}");

        // Trials-only bundle (a player with no worlds yet).
        let t = ProfileImportSummary { trials_merged: true, ..Default::default() };
        assert_eq!(t.message(), "No worlds imported · Trials records merged");
    }

    /// End-to-end on disk: build two worlds, pack them (plus Trials) into a
    /// `.axeprofile`, import it back, and check the keep-both rule fired for
    /// the name that already existed.
    #[test]
    fn profile_round_trip_keeps_both_copies() {
        use crate::profile_bundle::{pack, Entry, EntryKind};

        // NB: no leading underscores — `slugify_folder_name` trims those, so an
        // `__`-prefixed folder would never collide with its own slug and the
        // keep-both branch under test would not fire.
        let src_name = "zz-prof-rt-src-5c1d";
        let mut world = World::new();
        world.set_block(0, 0, 0, block::STONE);
        let meta = crate::save::WorldMeta::new(src_name);
        let save = crate::save::minimal_world_save_for_tests(1234);
        write_world_folder(src_name, &meta, &save, &world).expect("write source world");

        let world_bytes = export_world_native(src_name).expect("export source world");

        let mut bests = crate::trials::TrialBests::default();
        bests.mark_challenge_done("__prof_rt_challenge");

        // The Trials store is a real file in the data dir (path mirrors
        // `trials.rs::trials_path`). Snapshot it so the test neither disturbs a
        // dev's own records nor leaves a stray file behind.
        let trials_file = crate::data_dir::profile_dir().join("trials.json");
        let trials_before = std::fs::read(&trials_file).ok();

        let bundle = pack(&[
            Entry {
                kind: EntryKind::World,
                name: src_name.to_string(),
                bytes: world_bytes,
            },
            Entry {
                kind: EntryKind::Trials,
                name: "trials".to_string(),
                bytes: bests.to_json().into_bytes(),
            },
        ]);

        let summary = import_profile_native(&bundle).expect("profile import must succeed");
        assert_eq!(summary.imported, 1, "the bundle's one world must land");
        assert_eq!(summary.failed, 0);
        assert!(summary.trials_merged, "the Trials entry must be merged");
        assert_eq!(
            summary.renamed, 1,
            "the source folder already exists, so keep-both must rename the copy"
        );

        // The original is untouched and the copy sits beside it.
        let copy = format!("{src_name} (web)");
        assert!(world_dir(src_name).exists(), "the existing world must survive");
        assert!(world_dir(&copy).exists(), "the imported copy must be a second folder");

        let mut reloaded = World::new();
        let (rs, _) = load_world(&copy, &mut reloaded).expect("load the imported copy");
        assert_eq!(rs.seed, 1234);
        assert_eq!(reloaded.get_block(0, 0, 0), block::STONE);

        assert!(
            crate::trials::TrialBests::load().is_challenge_done("__prof_rt_challenge"),
            "the bundle's Trials record must be merged into the local store"
        );

        let _ = std::fs::remove_dir_all(world_dir(src_name));
        let _ = std::fs::remove_dir_all(world_dir(&copy));
        match trials_before {
            Some(bytes) => {
                let _ = std::fs::write(&trials_file, bytes);
            }
            None => {
                let _ = std::fs::remove_file(&trials_file);
                let _ = std::fs::remove_dir(crate::data_dir::profile_dir()); // only if now empty
            }
        }
    }

    /// Review 2026-10-06 — an import names against EVERY entry under the worlds
    /// root, not the lobby list (folders with a `world.dat`, minus the Workshop):
    /// it must never write into the native Workshop, a folder holding only
    /// chunks or only a crash-recovery autosave, or over a stray file.
    #[test]
    fn an_import_never_writes_into_a_name_the_lobby_does_not_list() {
        let _g = crate::save::WorldsRootGuard::new("nwio_import_collisions");
        let root = crate::save::worlds_root();
        let mut world = World::new();
        world.set_block(0, 0, 0, block::STONE);
        let save = crate::save::minimal_world_save_for_tests(9);
        write_world_folder("src", &crate::save::WorldMeta::new("src"), &save, &world).unwrap();
        let bytes = export_world_native("src").expect("export");
        write_world_folder(
            crate::workshop::WORKSHOP_FOLDER,
            &crate::save::WorldMeta::new("The Workshop"),
            &save,
            &world,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("chunks-only/chunks")).unwrap();
        std::fs::write(root.join("chunks-only/chunks/0_0_0.chunk"), b"chunk").unwrap();
        std::fs::create_dir_all(root.join("autosave-only/autosave")).unwrap();
        std::fs::write(root.join("autosave-only/autosave/world.dat"), b"autosave").unwrap();
        std::fs::write(root.join("stray"), b"a file").unwrap();
        let workshop_dat = std::fs::read(world_dir(crate::workshop::WORKSHOP_FOLDER).join("world.dat")).unwrap();

        for (entry, folder) in [
            ("The Workshop", "the_workshop (web)"),
            ("chunks-only", "chunks-only (web)"),
            ("autosave-only", "autosave-only (web)"),
            ("stray", "stray (web)"),
        ] {
            let (name, renamed) = import_profile_world(&bytes, entry).expect(entry);
            assert_eq!(name, folder, "{entry}");
            assert!(renamed, "{entry}");
        }
        // Every taken name is exactly as it was.
        assert_eq!(
            std::fs::read(world_dir(crate::workshop::WORKSHOP_FOLDER).join("world.dat")).unwrap(),
            workshop_dat
        );
        for (dir, files) in [
            ("chunks-only", vec!["chunks"]),
            ("autosave-only", vec!["autosave"]),
        ] {
            let mut found: Vec<String> = std::fs::read_dir(root.join(dir))
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            found.sort();
            assert_eq!(found, files, "{dir} was written into");
        }
        assert_eq!(std::fs::read(root.join("stray")).unwrap(), b"a file");
    }

    /// Review 2026-10-06 — an import that failed part-way left its half-written
    /// folder behind, which the lobby listed once `world.dat` was in it: a world
    /// with chunks missing. A failed import removes the folder it created.
    #[test]
    fn a_failed_import_leaves_no_folder_behind() {
        use crate::save::{FirstSaveCut, FIRST_SAVE_CUT};
        let _g = crate::save::WorldsRootGuard::new("nwio_import_fails");
        let mut world = World::new();
        world.set_block(3, 64, 5, block::STONE);
        world.set_block(40, 64, 5, block::STONE);
        let save = crate::save::minimal_world_save_for_tests(9);
        write_world_folder("src", &crate::save::WorldMeta::new("src"), &save, &world).unwrap();
        let bytes = export_world_native("src").expect("export");
        let folders = || {
            let mut f = existing_world_folders().unwrap();
            f.sort();
            f
        };
        let before = folders();
        for cut in [FirstSaveCut::AfterChunks(1), FirstSaveCut::BeforePublish] {
            FIRST_SAVE_CUT.with(|c| c.set(Some(cut)));
            let result = import_world_native(&bytes);
            FIRST_SAVE_CUT.with(|c| c.set(None));
            assert!(result.is_err(), "{cut:?}");
            assert_eq!(folders(), before, "{cut:?}: the half-written folder is removed");
        }
        let name = import_world_native(&bytes).expect("imports once nothing fails");
        let mut back = World::new();
        load_world(&name, &mut back).expect("the import opens");
        assert_eq!(back.get_block(40, 64, 5), block::STONE);
    }

    #[test]
    fn profile_import_rejects_a_file_that_is_not_a_profile() {
        let err = import_profile_native(b"definitely not a bundle")
            .expect_err("a non-profile file must be refused");
        assert!(
            err.contains("not an Axe'n'Stax profile"),
            "the refusal must be readable, got: {err}"
        );
    }
}
