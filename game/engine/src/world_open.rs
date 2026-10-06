//! Opening a saved world — the load-failure rule (Spec 02 §8.4).
//!
//! A world that is on disk but fails to load is NEVER replaced by a freshly
//! generated one. Before this module, any load error on the native client and the
//! dedicated server logged a warning and generated a fresh world, which was then
//! marked live and saved — deleting the real world's mined-out chunk files and
//! overwriting its spawn chunks and `world.dat`.
//!
//! [`open_world`] tells the two cases apart:
//! - **Nothing saved here** — no `world.dat`, no `autosave/world.dat`, no
//!   `chunks/*.chunk` → [`OpenedWorld::New`]: generate. A `world_meta.json` alone
//!   does not count: the Create dialog (and the dedicated server's bootstrap)
//!   write it before the world's first save.
//! - **A world is here** → it loads, or the caller gets `Err(reason)` and must
//!   not generate, not mark the world live and not write anything. Every loader
//!   it calls is all or nothing, so on `Err` the folder is byte-for-byte as it was.
//!
//! The existing keep-a-damaged-copy-aside recoveries still run when the rest of
//! the load succeeds (a torn chunk, a torn `world_meta.json` rebuilt from
//! `world.dat`, a partly decoded `world.dat`): no data is lost, so the world opens.

use crate::save::WorldSave;
use crate::world::World;

/// Whether [`open_world`] uses the crash-recovery autosave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutosavePolicy {
    /// The client: prefer `autosave/` (crash recovery), fall back to `world.dat`.
    Prefer,
    /// A hosted / dedicated server: `world.dat` only. The server never clears an
    /// autosave, so preferring one would shadow every later server save.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Ignore,
}

/// Which copy of the world was opened.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // the web opens nothing here
pub(crate) enum OpenedFrom {
    /// The last manual save (`world.dat` + `chunks/`).
    LastSave,
    /// The crash-recovery autosave.
    Autosave,
    /// The autosave failed to load (`why`), so the last manual save was opened.
    /// The damaged autosave folder was renamed to `kept_as` (relative to the world
    /// folder), so neither the next autosave nor the leave-time clear can destroy it.
    LastSaveAfterAutosaveFailed { why: String, kept_as: String },
}

/// What [`open_world`] found.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // the web opens nothing here
pub(crate) enum OpenedWorld {
    /// Nothing saved here yet: generate a fresh world.
    New,
    /// Loaded into the `World` passed in.
    Loaded {
        save: Box<WorldSave>,
        chunks: u32,
        from: OpenedFrom,
    },
}

impl std::fmt::Debug for OpenedWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::New => f.write_str("New"),
            Self::Loaded { chunks, from, .. } => f
                .debug_struct("Loaded")
                .field("chunks", chunks)
                .field("from", from)
                .finish_non_exhaustive(),
        }
    }
}

impl OpenedFrom {
    /// What to tell the player about which copy they got, if anything.
    pub(crate) fn player_note(&self) -> Option<String> {
        match self {
            Self::LastSave => None,
            Self::Autosave => Some("Recovered from the crash-recovery autosave.".to_string()),
            Self::LastSaveAfterAutosaveFailed { why, kept_as } => Some(format!(
                "The crash-recovery autosave couldn't be opened ({why}), so this is your last \
                 save. The autosave is kept as {kept_as}."
            )),
        }
    }
}

/// Open the world saved in `worlds/<name>/` into `world` (see the module docs).
///
/// `Err` is a short, file-naming reason; wrap it with
/// `save_format::unopenable_message` for the player. On `Err` the folder is
/// untouched and `world` must be discarded (it is untouched too, except in the
/// one case where the last save loaded but the damaged autosave could not be
/// moved aside).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn open_world(
    name: &str,
    world: &mut World,
    autosave: AutosavePolicy,
) -> Result<OpenedWorld, String> {
    let dir = crate::save::world_dir(name);
    // A newer build's save, or a world.dat / autosave/world.dat that can't be read.
    if let Some(why) = crate::save::world_open_refusal(name) {
        return Err(why);
    }
    let has_dat = present(&dir, "world.dat")?;
    let has_autosave = present(&dir, "autosave/world.dat")?;
    // Damaged-beyond-recovery world info refuses the world (a torn file is rebuilt
    // from world.dat here, the existing recovery). Checked for a new world too: one
    // whose meta can't be read would otherwise play under defaults and never save.
    crate::save::try_load_world_meta(name)?;

    if has_autosave && autosave == AutosavePolicy::Prefer {
        let autosave_err = match crate::save::load_autosave(name, world) {
            Ok((save, chunks)) => {
                return Ok(OpenedWorld::Loaded {
                    save: Box::new(save),
                    chunks,
                    from: OpenedFrom::Autosave,
                });
            }
            Err(e) => e,
        };
        if !has_dat {
            return Err(autosave_err);
        }
        log::error!("world '{name}': the autosave failed to load ({autosave_err}); opening the last save");
        let (save, chunks) = crate::save::load_world(name, world).map_err(|e| {
            format!("{e}; its crash-recovery autosave failed too ({autosave_err})")
        })?;
        let kept = crate::save::quarantine_corrupt(&dir.join("autosave")).map_err(|e| {
            format!("the crash-recovery autosave couldn't be opened ({autosave_err}) or kept aside ({e})")
        })?;
        let kept_as = kept
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| kept.display().to_string());
        return Ok(OpenedWorld::Loaded {
            save: Box::new(save),
            chunks,
            from: OpenedFrom::LastSaveAfterAutosaveFailed { why: autosave_err, kept_as },
        });
    }

    if has_dat {
        let (save, chunks) = crate::save::load_world(name, world)?;
        return Ok(OpenedWorld::Loaded {
            save: Box::new(save),
            chunks,
            from: OpenedFrom::LastSave,
        });
    }

    // Nothing to load. Is anything else of a saved world here?
    if let Some(file) = saved_chunk_file(&dir)? {
        return Err(format!("world.dat is missing, but saved chunks are here ({file})"));
    }
    if has_autosave {
        return Err(
            "world.dat is missing; only a crash-recovery autosave is here (autosave/world.dat) \
             — open the world in the game once to recover it"
                .to_string(),
        );
    }
    Ok(OpenedWorld::New)
}

/// Web: the async IndexedDB / cloud load in `game_loop` reads the world before
/// `begin_load` runs (and refuses it there on failure), so there is nothing to
/// open here.
#[cfg(target_arch = "wasm32")]
pub(crate) fn open_world(
    _name: &str,
    _world: &mut World,
    _autosave: AutosavePolicy,
) -> Result<OpenedWorld, String> {
    Ok(OpenedWorld::New)
}

/// The operator-facing refusal for a server log: the world, its folder, why, and
/// that nothing was changed.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn server_refusal_message(name: &str, why: &str) -> String {
    #[cfg(not(target_arch = "wasm32"))]
    let place = format!(" ({})", crate::save::world_dir(name).display());
    #[cfg(target_arch = "wasm32")]
    let place = String::new();
    format!("world '{name}'{place} couldn't be opened: {why}. Nothing was changed.")
}

/// True when nothing of a saved world is in `worlds/<name>/` — no `world.dat`, no
/// `autosave/world.dat`, no `chunks/*.chunk` — so a fresh world may be generated
/// (and the dedicated server may bootstrap its meta). `Err` when that can't be
/// determined. A `world_meta.json` alone is a new world.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn is_new_world(name: &str) -> Result<bool, String> {
    let dir = crate::save::world_dir(name);
    Ok(!present(&dir, "world.dat")?
        && !present(&dir, "autosave/world.dat")?
        && saved_chunk_file(&dir)?.is_none())
}

/// Whether `dir/rel` exists; `Err` when that can't be determined.
#[cfg(not(target_arch = "wasm32"))]
fn present(dir: &std::path::Path, rel: &str) -> Result<bool, String> {
    dir.join(rel)
        .try_exists()
        .map_err(|e| format!("{rel} can't be checked ({e})"))
}

/// The first `chunks/*.chunk` file in `dir`, if any (`autosave/chunks/` without an
/// `autosave/world.dat` is a torn autosave that nothing ever loads, so it doesn't
/// count). `Err` when the folder can't be listed.
#[cfg(not(target_arch = "wasm32"))]
fn saved_chunk_file(dir: &std::path::Path) -> Result<Option<String>, String> {
    let entries = match std::fs::read_dir(dir.join("chunks")) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("chunks/ can't be read ({e})")),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("chunks/ can't be listed ({e})"))?;
        let file = entry.file_name().to_string_lossy().into_owned();
        if file.ends_with(".chunk") {
            return Ok(Some(format!("chunks/{file}")));
        }
    }
    Ok(None)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::save::{world_dir, WorldMeta, WorldsRootGuard, QUARANTINE_FAILS_FOR};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    /// Every file under `dir` with its bytes (directories by name), so a test can
    /// prove a refused world is byte-for-byte as it was.
    fn snapshot(dir: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
        fn walk(root: &Path, at: &Path, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
            for e in fs::read_dir(at).unwrap().flatten() {
                let p = e.path();
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().to_string();
                if p.is_dir() {
                    out.insert(format!("{rel}/"), None);
                    walk(root, &p, out);
                } else {
                    out.insert(rel, Some(fs::read(&p).unwrap()));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(dir, dir, &mut out);
        out
    }

    /// A real saved world `name`: meta, a footer-bearing `world.dat` and one solid
    /// chunk file (`chunks/0_4_0.chunk`, holding a bedrock block at (3, 64, 5)).
    fn saved_world(name: &str) -> std::path::PathBuf {
        let mut w = World::new();
        w.set_block(3, 64, 5, crate::block::BEDROCK);
        crate::save::write_world_folder(
            name,
            &WorldMeta::new(name),
            &crate::save::minimal_world_save_for_tests(77),
            &w,
        )
        .unwrap();
        let dir = world_dir(name);
        assert!(dir.join("chunks/0_4_0.chunk").is_file());
        dir
    }

    /// A chunk file whose bytes no chunk encoding accepts.
    fn torn_chunk(dir: &Path, file: &str) {
        fs::write(dir.join("chunks").join(file), vec![7u8; 100]).unwrap();
    }

    /// `open_world` refuses the world, names the cause, and leaves every file as
    /// it was; the dedicated server's `initial_load` refuses it the same way.
    fn assert_refused(name: &str, cause: &str) {
        let dir = world_dir(name);
        let before = snapshot(&dir);
        for policy in [AutosavePolicy::Prefer, AutosavePolicy::Ignore] {
            let mut world = World::new();
            let err = open_world(name, &mut world, policy).expect_err("must refuse");
            assert!(err.contains(cause), "{policy:?}: '{err}' should name '{cause}'");
            assert_eq!(snapshot(&dir), before, "{policy:?}: a refused world must be untouched");
        }
        let mut server = crate::server::GameServer::new(0, name.to_string(), 1);
        let err = server.initial_load().expect_err("the server must refuse it too");
        assert!(err.contains(cause), "server: {err}");
        assert_eq!(snapshot(&dir), before, "server: a refused world must be untouched");
        assert!(!is_new_world(name).unwrap_or(false), "never bootstrapped as new");
    }

    #[test]
    fn a_world_never_saved_is_new_even_with_a_meta_file() {
        let _g = WorldsRootGuard::new("open_new");
        let mut world = World::new();
        assert!(matches!(
            open_world("nothing", &mut world, AutosavePolicy::Prefer),
            Ok(OpenedWorld::New)
        ));
        // What the Create dialog leaves before the first entry.
        crate::save::save_world_meta("created", &WorldMeta::new("created")).unwrap();
        let before = snapshot(&world_dir("created"));
        assert!(matches!(
            open_world("created", &mut world, AutosavePolicy::Prefer),
            Ok(OpenedWorld::New)
        ));
        assert!(is_new_world("created").unwrap());
        assert_eq!(snapshot(&world_dir("created")), before);
        // The server generates it too.
        let mut server = crate::server::GameServer::new(0, "created".into(), 1);
        server.render_distance = 1;
        server.initial_load().expect("a new world still generates");
        assert!(server.world.persistable_chunks().count() > 0, "terrain was generated");
    }

    #[test]
    fn a_good_world_opens_from_its_last_save() {
        let _g = WorldsRootGuard::new("open_good");
        saved_world("w");
        let mut world = World::new();
        match open_world("w", &mut world, AutosavePolicy::Prefer).unwrap() {
            OpenedWorld::Loaded { chunks, from, .. } => {
                assert_eq!(chunks, 1);
                assert_eq!(from, OpenedFrom::LastSave);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(world.get_block(3, 64, 5), crate::block::BEDROCK);
        assert!(!is_new_world("w").unwrap());
    }

    #[test]
    fn unreadable_world_dat_is_refused() {
        let _g = WorldsRootGuard::new("open_dat_io");
        let dir = saved_world("w");
        // A directory where the file should be fails every read, even as root.
        fs::remove_file(dir.join("world.dat")).unwrap();
        fs::create_dir(dir.join("world.dat")).unwrap();
        assert_refused("w", "world.dat can't be read");
        let why = crate::save::world_open_refusal("w").expect("the lobby refuses it up front");
        assert!(why.starts_with(crate::save_format::UNOPENABLE_PREFIX), "{why}");
        // Every writer refuses it too (its version is unknown).
        assert!(crate::save::save_world_meta("w", &WorldMeta::new("w")).is_err());
        assert!(crate::server::GameServer::new(0, "w".into(), 1).try_save().is_err());
        let entries = crate::save::list_world_entries();
        assert!(entries.is_empty() || entries[0].meta.display_name.contains("can't be opened"));
    }

    #[test]
    fn unreadable_autosave_world_dat_is_refused() {
        let _g = WorldsRootGuard::new("open_autosave_io");
        let dir = saved_world("w");
        fs::create_dir_all(dir.join("autosave/world.dat")).unwrap();
        assert_refused("w", "autosave/world.dat can't be read");
    }

    #[test]
    fn unreadable_world_meta_is_refused() {
        let _g = WorldsRootGuard::new("open_meta_io");
        let dir = saved_world("w");
        fs::remove_file(dir.join("world_meta.json")).unwrap();
        fs::create_dir(dir.join("world_meta.json")).unwrap();
        assert_refused("w", "world_meta.json can't be read");
    }

    #[test]
    fn undecodable_world_dat_is_refused() {
        let _g = WorldsRootGuard::new("open_dat_garbage");
        let dir = saved_world("w");
        fs::write(dir.join("world.dat"), b"\x01not a world save").unwrap();
        assert_refused("w", "world.dat is damaged");
    }

    #[test]
    fn a_chunk_file_name_that_isnt_three_integers_is_refused() {
        let _g = WorldsRootGuard::new("open_bad_name");
        let dir = saved_world("w");
        // A torn chunk beside it: whatever order the directory lists them in, it
        // must not be quarantined before the bad name fails the load.
        torn_chunk(&dir, "1_4_1.chunk");
        fs::write(dir.join("chunks/a_b_c.chunk"), b"x").unwrap();
        assert_refused("w", "chunks/a_b_c.chunk isn't a chunk file name");
    }

    #[test]
    fn a_chunk_file_that_cant_be_read_is_refused() {
        let _g = WorldsRootGuard::new("open_chunk_io");
        let dir = saved_world("w");
        torn_chunk(&dir, "1_4_1.chunk");
        fs::create_dir(dir.join("chunks/2_4_2.chunk")).unwrap();
        assert_refused("w", "chunks/2_4_2.chunk can't be read");
    }

    #[test]
    fn a_damaged_chunk_that_cant_be_kept_aside_is_refused_and_earlier_moves_undone() {
        let _g = WorldsRootGuard::new("open_quarantine_fail");
        let dir = saved_world("w");
        torn_chunk(&dir, "1_4_1.chunk");
        torn_chunk(&dir, "2_4_2.chunk");
        // Whichever is quarantined first, the second fails — and the first is
        // moved back, so the folder ends exactly as it started.
        for failing in ["1_4_1.chunk", "2_4_2.chunk"] {
            QUARANTINE_FAILS_FOR.with(|f| *f.borrow_mut() = Some(failing.to_string()));
            assert_refused("w", &format!("chunks/{failing} is damaged and couldn't be kept aside"));
        }
        QUARANTINE_FAILS_FOR.with(|f| *f.borrow_mut() = None);
    }

    #[test]
    fn unparseable_meta_whose_quarantine_fails_is_refused() {
        let _g = WorldsRootGuard::new("open_meta_quarantine_fail");
        let dir = saved_world("w");
        fs::write(dir.join("world_meta.json"), b"{ torn").unwrap();
        QUARANTINE_FAILS_FOR.with(|f| *f.borrow_mut() = Some("world_meta.json".to_string()));
        assert_refused("w", "world info damaged");
        QUARANTINE_FAILS_FOR.with(|f| *f.borrow_mut() = None);
    }

    #[test]
    fn unparseable_meta_with_an_undecodable_world_dat_is_refused_untouched() {
        let _g = WorldsRootGuard::new("open_meta_and_dat");
        let dir = saved_world("w");
        fs::write(dir.join("world_meta.json"), b"{ torn").unwrap();
        fs::write(dir.join("world.dat"), b"\x01not a world save").unwrap();
        // world.dat is checked before the meta is moved, so nothing is renamed.
        assert_refused("w", "world info damaged");
    }

    #[test]
    fn saved_chunks_without_a_world_dat_are_refused_not_regenerated() {
        let _g = WorldsRootGuard::new("open_lost_dat");
        let dir = saved_world("w");
        fs::remove_file(dir.join("world.dat")).unwrap();
        assert_refused("w", "world.dat is missing, but saved chunks are here");
    }

    #[test]
    fn the_server_refuses_a_world_with_only_an_autosave() {
        let _g = WorldsRootGuard::new("open_autosave_only_server");
        let mut w = World::new();
        w.set_block(3, 64, 5, crate::block::BEDROCK);
        let slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::new(0.5, 64.0, 0.5), 1.0);
        crate::save::autosave_world("w", &w, std::slice::from_ref(&slot), 1, &[], &[]).unwrap();
        let dir = world_dir("w");
        let before = snapshot(&dir);
        let err = open_world("w", &mut World::new(), AutosavePolicy::Ignore).unwrap_err();
        assert!(err.contains("only a crash-recovery autosave is here"), "{err}");
        assert!(crate::server::GameServer::new(0, "w".into(), 1).initial_load().is_err());
        assert_eq!(snapshot(&dir), before);
        // The client recovers it.
        let mut world = World::new();
        match open_world("w", &mut world, AutosavePolicy::Prefer).unwrap() {
            OpenedWorld::Loaded { from, .. } => assert_eq!(from, OpenedFrom::Autosave),
            other => panic!("{other:?}"),
        }
        assert_eq!(world.get_block(3, 64, 5), crate::block::BEDROCK);
    }

    #[test]
    fn a_failed_autosave_falls_back_to_the_last_save_and_is_kept_aside() {
        let _g = WorldsRootGuard::new("open_autosave_fallback");
        let dir = saved_world("w");
        fs::create_dir_all(dir.join("autosave/chunks")).unwrap();
        fs::write(dir.join("autosave/world.dat"), b"\x01torn autosave").unwrap();
        fs::write(dir.join("autosave/chunks/9_4_9.chunk"), b"kept").unwrap();
        let mut before = snapshot(&dir);

        let mut world = World::new();
        let from = match open_world("w", &mut world, AutosavePolicy::Prefer).unwrap() {
            OpenedWorld::Loaded { from, .. } => from,
            other => panic!("{other:?}"),
        };
        let OpenedFrom::LastSaveAfterAutosaveFailed { why, kept_as } = &from else {
            panic!("{from:?}");
        };
        assert!(why.contains("autosave/world.dat is damaged"), "{why}");
        assert!(kept_as.starts_with("autosave.corrupt-"), "{kept_as}");
        assert!(from.player_note().unwrap().contains(kept_as.as_str()));
        assert_eq!(world.get_block(3, 64, 5), crate::block::BEDROCK, "the last save loaded");

        // The only change: the autosave folder moved aside, byte for byte.
        let after = snapshot(&dir);
        let renamed: BTreeMap<_, _> = before
            .iter()
            .filter(|(k, _)| k.starts_with("autosave/"))
            .map(|(k, v)| (format!("{kept_as}{}", &k["autosave".len()..]), v.clone()))
            .collect();
        before.retain(|k, _| !k.starts_with("autosave/"));
        before.extend(renamed);
        assert_eq!(after, before);
        // So the leave-time clear can't destroy it, and the next open is plain.
        crate::save::clear_autosave("w");
        assert!(dir.join(kept_as).join("world.dat").is_file());
        assert!(matches!(
            open_world("w", &mut World::new(), AutosavePolicy::Prefer),
            Ok(OpenedWorld::Loaded { from: OpenedFrom::LastSave, .. })
        ));
    }

    #[test]
    fn a_failed_autosave_over_a_failed_world_dat_is_refused_untouched() {
        let _g = WorldsRootGuard::new("open_both_fail");
        let dir = saved_world("w");
        fs::write(dir.join("world.dat"), b"\x01not a world save").unwrap();
        fs::create_dir_all(dir.join("autosave")).unwrap();
        fs::write(dir.join("autosave/world.dat"), b"\x01torn autosave").unwrap();
        let before = snapshot(&dir);
        let err = open_world("w", &mut World::new(), AutosavePolicy::Prefer).unwrap_err();
        assert!(err.contains("world.dat is damaged") && err.contains("autosave failed too"), "{err}");
        assert_eq!(snapshot(&dir), before);
    }

    #[test]
    fn an_autosave_that_cant_be_kept_aside_refuses_the_world() {
        let _g = WorldsRootGuard::new("open_autosave_keep_fail");
        let dir = saved_world("w");
        fs::create_dir_all(dir.join("autosave")).unwrap();
        fs::write(dir.join("autosave/world.dat"), b"\x01torn autosave").unwrap();
        let before = snapshot(&dir);
        QUARANTINE_FAILS_FOR.with(|f| *f.borrow_mut() = Some("autosave".to_string()));
        let err = open_world("w", &mut World::new(), AutosavePolicy::Prefer).unwrap_err();
        QUARANTINE_FAILS_FOR.with(|f| *f.borrow_mut() = None);
        assert!(err.contains("kept aside"), "{err}");
        assert_eq!(snapshot(&dir), before);
    }

    /// The recovery that loses nothing still opens the world: a torn chunk is
    /// kept aside (and regenerates); the world is not refused.
    #[test]
    fn a_torn_chunk_is_kept_aside_and_the_world_still_opens() {
        let _g = WorldsRootGuard::new("open_torn_chunk_ok");
        let dir = saved_world("w");
        torn_chunk(&dir, "1_4_1.chunk");
        let mut world = World::new();
        assert!(matches!(
            open_world("w", &mut world, AutosavePolicy::Prefer),
            Ok(OpenedWorld::Loaded { chunks: 1, .. })
        ));
        assert!(!dir.join("chunks/1_4_1.chunk").exists());
        assert!(fs::read_dir(dir.join("chunks"))
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("1_4_1.chunk.corrupt-")));
    }

    // ── Saving never deletes a chunk file it didn't read or write ──

    fn slot() -> crate::player_slot::PlayerSlot {
        crate::player_slot::PlayerSlot::new(0, glam::Vec3::new(0.5, 64.0, 0.5), 1.0)
    }

    /// An all-air chunk where the disk holds a file this session never read (a
    /// fresh world generated over existing files, the old failure) must not delete
    /// that file.
    #[test]
    fn save_leaves_an_unknown_chunk_file_alone() {
        let _g = WorldsRootGuard::new("save_unknown_chunk");
        let dir = saved_world("w");
        let before = fs::read(dir.join("chunks/0_4_0.chunk")).unwrap();
        // A world that never read `w`, with an all-air chunk at (0, 4, 0).
        let mut w = World::new();
        w.set_block(3, 64, 5, crate::block::BEDROCK);
        w.set_block(3, 64, 5, crate::block::AIR);
        assert!(w.persistable_chunks().any(|(k, c)| k == (0, 4, 0) && c.is_empty()));
        crate::save::save_world("w", &w, std::slice::from_ref(&slot()), 1, &[], &[]).unwrap();
        assert_eq!(fs::read(dir.join("chunks/0_4_0.chunk")).unwrap(), before, "left alone");
    }

    /// The mined-out case still deletes: a chunk read from disk, or written by this
    /// session, that is now all-air loses its file (engine audit 2026-06-04, A).
    #[test]
    fn save_deletes_a_mined_out_chunk_it_read_or_wrote() {
        let _g = WorldsRootGuard::new("save_known_chunk");
        let dir = saved_world("w");
        let mut world = World::new();
        assert!(matches!(
            open_world("w", &mut world, AutosavePolicy::Prefer),
            Ok(OpenedWorld::Loaded { .. })
        ));
        // A chunk this session writes, then mines out.
        world.set_block(40, 64, 5, crate::block::BEDROCK); // chunk 2_4_0
        crate::save::save_world("w", &world, std::slice::from_ref(&slot()), 1, &[], &[]).unwrap();
        assert!(dir.join("chunks/2_4_0.chunk").is_file());
        world.set_block(3, 64, 5, crate::block::AIR); // the loaded chunk, mined out
        world.set_block(40, 64, 5, crate::block::AIR); // the written chunk, mined out
        crate::save::save_world("w", &world, std::slice::from_ref(&slot()), 1, &[], &[]).unwrap();
        assert!(!dir.join("chunks/0_4_0.chunk").exists(), "read-in chunk deleted");
        assert!(!dir.join("chunks/2_4_0.chunk").exists(), "written chunk deleted");
        let mut back = World::new();
        crate::save::load_world("w", &mut back).unwrap();
        assert_eq!(back.get_block(3, 64, 5), crate::block::AIR, "no resurrection");
    }

    /// A save over a world whose meta is damaged writes NOTHING — the meta used
    /// to be refused only at the end, after the chunks and world.dat.
    #[test]
    fn save_over_damaged_meta_writes_nothing() {
        let _g = WorldsRootGuard::new("save_damaged_meta");
        let dir = saved_world("w");
        fs::write(dir.join("world_meta.json"), b"{ torn").unwrap();
        let before = snapshot(&dir);
        let mut w = World::new();
        w.set_block(3, 64, 5, crate::block::STONE);
        assert!(crate::save::save_world("w", &w, std::slice::from_ref(&slot()), 1, &[], &[]).is_err());
        assert!(crate::save::write_world_folder(
            "w",
            &WorldMeta::new("w"),
            &crate::save::minimal_world_save_for_tests(1),
            &w
        )
        .is_err());
        assert_eq!(snapshot(&dir), before);
    }
}
