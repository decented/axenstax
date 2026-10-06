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
//! The client opens the crash-recovery autosave first only when it is newer than
//! `world.dat` (by modification time, [`autosave_is_newer`]); the other copy is
//! the fallback either way, and an older autosave is cleared once `world.dat`
//! has opened.
//!
//! The existing keep-a-damaged-copy-aside recoveries still run when the rest of
//! the load succeeds (a torn chunk, a torn `world_meta.json` rebuilt from
//! `world.dat`, a partly decoded `world.dat`): no data is lost, so the world opens.
//! Reading a world's info never writes (`save::peek_world_meta`): the torn-meta
//! rebuild runs LAST, once everything else has loaded. If that rebuild itself
//! fails, the world is refused, and a damaged chunk or autosave the load had
//! already kept aside stays aside — moved, never lost.

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
    /// `world.dat` — newer than the autosave — failed to load (`why`), so the
    /// older crash-recovery autosave was opened. The damaged `world.dat` was
    /// COPIED to `kept_as` (relative to the world folder) and left in place (the
    /// lobby lists only folders with one), so the session's next save can't
    /// destroy the only copy.
    AutosaveAfterLastSaveFailed { why: String, kept_as: String },
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
            Self::AutosaveAfterLastSaveFailed { why, kept_as } => Some(format!(
                "Your last save couldn't be opened ({why}), so this is your older \
                 crash-recovery autosave. A copy of that save is kept as {kept_as}."
            )),
        }
    }

    /// Did the world open FROM the crash-recovery autosave? Then it is kept until
    /// a save lands (`world_exit::SessionSaves::opened`): `world.dat` may be
    /// damaged, leaving it the only good copy.
    pub(crate) fn opened_from_autosave(&self) -> bool {
        matches!(self, Self::Autosave | Self::AutosaveAfterLastSaveFailed { .. })
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
    // Damaged-beyond-recovery world info refuses the world. Checked for a new world
    // too: one whose meta can't be read would otherwise play under defaults and
    // never save. Read-only — a torn but recoverable meta is rebuilt from
    // world.dat only once the world has loaded (`repaired`), so a world refused
    // below is never written (review 2026-10-06).
    crate::save::peek_world_meta(name)?;
    // Every `Loaded` return goes through here: the torn-meta recovery (keep the
    // damaged file aside, write the rebuilt meta) runs last, like the other
    // keep-a-copy-aside recoveries.
    let repaired = |opened: OpenedWorld| -> Result<OpenedWorld, String> {
        crate::save::try_load_world_meta(name)?;
        Ok(opened)
    };

    // The client opens the autosave first only when it was written after
    // world.dat (third review, 2026-10-06): one older than world.dat is stale —
    // a save that committed but failed later, before this build dropped it at
    // the commit — and opening it rolled the newer save back.
    let use_autosave = has_autosave && autosave == AutosavePolicy::Prefer;
    if use_autosave && (!has_dat || autosave_is_newer(&dir)) {
        let autosave_err = match crate::save::load_autosave(name, world) {
            Ok((save, chunks)) => {
                return repaired(OpenedWorld::Loaded {
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
        return repaired(OpenedWorld::Loaded {
            save: Box::new(save),
            chunks,
            from: OpenedFrom::LastSaveAfterAutosaveFailed { why: autosave_err, kept_as },
        });
    }

    if has_dat {
        let world_dat_err = match crate::save::load_world(name, world) {
            Ok((save, chunks)) => {
                let opened = repaired(OpenedWorld::Loaded {
                    save: Box::new(save),
                    chunks,
                    from: OpenedFrom::LastSave,
                })?;
                if use_autosave {
                    // Older than the world.dat that just opened: superseded, and
                    // left here it would mix its stale chunk files into the next
                    // autosave.
                    log::warn!("world '{name}': clearing a crash-recovery autosave older than world.dat");
                    crate::save::clear_autosave_in(&dir);
                }
                return Ok(opened);
            }
            Err(e) => e,
        };
        if !use_autosave {
            return Err(world_dat_err);
        }
        // world.dat is newer but won't load: the older autosave is the newest
        // copy that opens. The damaged world.dat is copied aside first — the
        // session's next save overwrites it.
        log::error!("world '{name}': world.dat failed to load ({world_dat_err}); opening the older autosave");
        let (save, chunks) = crate::save::load_autosave(name, world).map_err(|e| {
            format!("{world_dat_err}; its older crash-recovery autosave failed too ({e})")
        })?;
        let kept = crate::save::copy_damaged_aside(&dir.join("world.dat")).map_err(|e| {
            format!("world.dat couldn't be opened ({world_dat_err}) or kept aside ({e})")
        })?;
        let kept_as = kept
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| kept.display().to_string());
        return repaired(OpenedWorld::Loaded {
            save: Box::new(save),
            chunks,
            from: OpenedFrom::AutosaveAfterLastSaveFailed { why: world_dat_err, kept_as },
        });
    }

    // Nothing to load. Is anything else of a saved world here?
    if let Some(file) = saved_chunk_file(&dir)? {
        return Err(format!(
            "world.dat is missing, but saved chunks are here ({file}). If the world's first \
             save was cut short, move its chunks folder aside and it starts again from its \
             seed; otherwise restore world.dat from a backup"
        ));
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

/// Was `autosave/world.dat` written after `world.dat`? Neither file records when
/// it was saved, and each is written whole by tmp + rename, so its modification
/// time is its save time. Strictly later: a tie goes to `world.dat`. When either
/// time can't be read the autosave counts as newer — the old rule (it is crash
/// recovery), and `world.dat` is still the fallback if it fails to load.
#[cfg(not(target_arch = "wasm32"))]
fn autosave_is_newer(dir: &std::path::Path) -> bool {
    let written = |rel: &str| std::fs::metadata(dir.join(rel)).and_then(|m| m.modified());
    match (written("autosave/world.dat"), written("world.dat")) {
        (Ok(autosave), Ok(world_dat)) => autosave > world_dat,
        (autosave, world_dat) => {
            log::warn!(
                "{}: can't tell which save is newer (autosave {:?}, world.dat {:?}); trying the autosave first",
                dir.display(),
                autosave.err(),
                world_dat.err()
            );
            true
        }
    }
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

/// How a Trial arena launch treats its folder, decided from what is ON DISK and
/// before anything is written (Spec 02 §8.4). A world is there when
/// [`is_new_world`] says so — not when the lobby lists it (only folders with a
/// `world.dat`): a Resume arena holding chunks but no `world.dat` used to be
/// re-created with a fresh meta over its real one before the open refused it,
/// and a Reuse arena in that state was never wiped, so it was refused on every
/// launch. `Err` (the lobby notice) for a world this build must not open
/// (`save::world_open_refusal`) or a folder that can't be checked — then
/// nothing is wiped, created or entered. Reads only.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn plan_arena_folder(
    mode: crate::scenario::ArenaMode,
    name: &str,
) -> Result<crate::scenario::ArenaLaunch, String> {
    if let Some(why) = crate::save::world_open_refusal(name) {
        return Err(why);
    }
    let on_disk = !is_new_world(name).map_err(|why| crate::save_format::unopenable_message(&why))?;
    Ok(crate::scenario::plan_arena_launch(mode, on_disk))
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
pub(crate) fn saved_chunk_file(dir: &std::path::Path) -> Result<Option<String>, String> {
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
        // The refused world keeps its lobby card, labelled with why.
        let entries = crate::save::list_world_entries();
        assert_eq!(entries.len(), 1, "a refused world is listed, not hidden");
        assert!(
            entries[0].meta.display_name.contains("can't be opened"),
            "{}",
            entries[0].meta.display_name
        );
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
        // The refusal tells the operator / player how to get the world back.
        let err = open_world("w", &mut World::new(), AutosavePolicy::Ignore).unwrap_err();
        assert!(err.contains("move its chunks folder aside"), "{err}");
        assert!(err.contains("restore world.dat from a backup"), "{err}");
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
        // Written after world.dat (an autosave the next open tries first).
        written_ago(&dir.join("world.dat"), 120);
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
        written_ago(&dir.join("world.dat"), 120);
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

    // ── Review 2026-10-06 follow-ups ──

    /// Opening never writes before it knows the world opens: a torn (but
    /// recoverable) meta used to be quarantined and rebuilt at the top of
    /// `open_world` — and by the lobby list and every `load_world_meta` read —
    /// so a world then refused for another reason was no longer as it was.
    #[test]
    fn a_torn_meta_is_left_alone_by_reads_and_by_a_refused_open() {
        let _g = WorldsRootGuard::new("open_torn_meta_refused");
        let dir = saved_world("w");
        fs::write(dir.join("world_meta.json"), b"{ torn").unwrap();
        fs::write(dir.join("chunks/a_b_c.chunk"), b"x").unwrap();
        let before = snapshot(&dir);
        // The lobby card and the display / seed reads see the recovered info...
        let entries = crate::save::list_world_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(crate::save::load_world_meta("w").seed, 77, "seed recovered from world.dat");
        // ...and write nothing.
        assert_eq!(snapshot(&dir), before, "reading a world's info never writes");
        assert_refused("w", "chunks/a_b_c.chunk isn't a chunk file name");
    }

    /// The torn-meta recovery still runs — once the world has opened.
    #[test]
    fn a_torn_meta_is_rebuilt_once_the_world_has_opened() {
        let _g = WorldsRootGuard::new("open_torn_meta_ok");
        let dir = saved_world("w");
        fs::write(dir.join("world_meta.json"), b"{ torn").unwrap();
        let mut world = World::new();
        assert!(matches!(
            open_world("w", &mut world, AutosavePolicy::Prefer),
            Ok(OpenedWorld::Loaded { from: OpenedFrom::LastSave, .. })
        ));
        let meta: WorldMeta =
            serde_json::from_slice(&fs::read(dir.join("world_meta.json")).unwrap()).unwrap();
        assert_eq!(meta.seed, 77, "rebuilt from world.dat");
        assert!(meta.pop_secret.is_some(), "the rebuilt meta carries a real secret");
        assert!(fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("world_meta.json.corrupt-")));
        // A later save is no longer refused.
        crate::save::save_world("w", &world, std::slice::from_ref(&slot()), 77, &[], &[]).unwrap();
    }

    /// A Trial arena is planned from what is ON DISK, not from the lobby list
    /// (folders with a `world.dat`): a Resume arena holding chunks but no
    /// `world.dat` used to be re-created with a fresh meta written over its real
    /// one before the open refused it, and a Reuse arena in that state was
    /// refused on every launch, never wiped.
    #[test]
    fn an_arena_is_planned_from_the_disk_and_planning_writes_nothing() {
        use crate::scenario::{ArenaLaunch, ArenaMode};
        let _g = WorldsRootGuard::new("open_arena_plan");
        let dir = saved_world("exp-a");
        fs::remove_file(dir.join("world.dat")).unwrap();
        let before = snapshot(&dir);
        // Resume: a saved run is here — resumed (so the open refuses it with
        // nothing written), never re-created over it.
        assert_eq!(plan_arena_folder(ArenaMode::Resume, "exp-a"), Ok(ArenaLaunch::Resume));
        // Reuse regenerates every play: the stale folder is wiped, not a lock-out.
        assert_eq!(
            plan_arena_folder(ArenaMode::Reuse, "exp-a"),
            Ok(ArenaLaunch::Fresh { wipe: true })
        );
        assert_eq!(snapshot(&dir), before, "planning writes nothing");
        // An autosave alone counts as saved too.
        let slot = slot();
        crate::save::autosave_world("exp-b", &World::new(), std::slice::from_ref(&slot), 1, &[], &[])
            .unwrap();
        assert_eq!(plan_arena_folder(ArenaMode::Resume, "exp-b"), Ok(ArenaLaunch::Resume));
        // Nothing here (a meta alone is a new world): fresh, nothing to wipe.
        crate::save::save_world_meta("exp-c", &WorldMeta::new("exp-c")).unwrap();
        assert_eq!(
            plan_arena_folder(ArenaMode::Resume, "exp-c"),
            Ok(ArenaLaunch::Fresh { wipe: false })
        );
        assert_eq!(
            plan_arena_folder(ArenaMode::Reuse, "exp-none"),
            Ok(ArenaLaunch::Fresh { wipe: false })
        );
        // A world this build must not open is refused before any wipe.
        let d = saved_world("exp-d");
        fs::remove_file(d.join("world.dat")).unwrap();
        fs::create_dir(d.join("world.dat")).unwrap();
        let before = snapshot(&d);
        for mode in [ArenaMode::Reuse, ArenaMode::Resume, ArenaMode::KeepNew] {
            let err = plan_arena_folder(mode, "exp-d").unwrap_err();
            assert!(err.contains("world.dat can't be read"), "{mode:?}: {err}");
        }
        assert_eq!(snapshot(&d), before);
    }

    /// A world with chunks in several columns, several chunks tall — every
    /// non-empty chunk key it holds, so a reload can be checked for holes.
    fn many_chunk_world() -> (World, std::collections::BTreeSet<(i32, i32, i32)>) {
        let mut w = World::new();
        for (x, z) in [(3, 5), (40, 5), (90, 90)] {
            for y in [20, 40, 64, 80] {
                w.set_block(x, y, z, crate::block::BEDROCK);
            }
        }
        let keys = solid_chunks(&w);
        assert_eq!(keys.len(), 12);
        (w, keys)
    }

    fn solid_chunks(w: &World) -> std::collections::BTreeSet<(i32, i32, i32)> {
        w.persistable_chunks().filter(|(_, c)| !c.is_empty()).map(|(k, _)| k).collect()
    }

    /// What a reopened world holds: `None` when it is still new (nothing a loader
    /// reads), else its solid chunks. Both openers must agree.
    fn reopened(name: &str) -> Option<std::collections::BTreeSet<(i32, i32, i32)>> {
        let mut client = World::new();
        let got = match open_world(name, &mut client, AutosavePolicy::Ignore)
            .unwrap_or_else(|e| panic!("{name}: a first save cut short is never refused: {e}"))
        {
            OpenedWorld::New => None,
            OpenedWorld::Loaded { .. } => Some(solid_chunks(&client)),
        };
        assert_eq!(is_new_world(name).unwrap(), got.is_none(), "{name}");
        got
    }

    /// Review 2026-10-06 — a world's FIRST save wrote its chunks one by one in
    /// hash-map order, and every loader marks a column with ANY saved chunk as
    /// loaded and never generates it: cut short part-way, it left permanent holes
    /// (and a first save that wrote `world.dat` first left it beside some of the
    /// chunks). Now a first save stages every chunk in `chunks.new/`, commits with
    /// `world.dat`, then publishes `chunks.new/` as `chunks/`: cut short anywhere,
    /// the world is still new or is whole — never holed — and the next save or
    /// open recovers.
    #[test]
    fn a_first_save_cut_short_anywhere_leaves_a_new_or_whole_world_never_holes() {
        use crate::save::{FirstSaveCut, FIRST_SAVE_CUT};
        let _g = WorldsRootGuard::new("open_first_save_cut");
        let all = many_chunk_world().1;
        let cuts = (0..=all.len())
            .map(FirstSaveCut::AfterChunks)
            .chain([FirstSaveCut::BeforePublish]);
        for (i, cut) in cuts.enumerate() {
            for writer in ["client", "server"] {
                let name = format!("{writer}-{i}");
                // The client's writer, or the dedicated server's (its tick-0 save).
                let w = many_chunk_world().0;
                let mut server = crate::server::GameServer::new(0, name.clone(), 5);
                server.world = many_chunk_world().0;
                let save = |w: &World, server: &crate::server::GameServer| {
                    if writer == "client" {
                        crate::save::save_world(&name, w, std::slice::from_ref(&slot()), 5, &[], &[])
                    } else {
                        server.try_save()
                    }
                };
                if writer == "client" {
                    crate::save::save_world_meta(&name, &WorldMeta::new(&name)).unwrap();
                }
                FIRST_SAVE_CUT.with(|c| c.set(Some(cut)));
                let err = save(&w, &server).expect_err("cut short");
                FIRST_SAVE_CUT.with(|c| c.set(None));
                assert!(err.contains("cut short"), "{name}: {err}");
                match reopened(&name) {
                    None => assert!(
                        !matches!(cut, FirstSaveCut::BeforePublish),
                        "{name}: a committed first save is never lost"
                    ),
                    Some(got) => assert_eq!(got, all, "{name} ({cut:?}): holes"),
                }
                // The next save — still a first save if nothing committed —
                // leaves the whole world.
                save(&w, &server).unwrap_or_else(|e| panic!("{name}: the next save: {e}"));
                assert_eq!(reopened(&name), Some(all.clone()), "{name}: after the next save");
                let left: Vec<_> = fs::read_dir(world_dir(&name))
                    .unwrap()
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|f| f.starts_with("chunks.") || f.ends_with(".tmp"))
                    .collect();
                assert!(left.is_empty(), "{name}: staging left behind: {left:?}");
            }
        }
    }

    /// A committed first save cut short before its chunks were published is
    /// finished by the next open (and by the next save), never read as an empty
    /// world under `world.dat`'s block entities.
    #[test]
    fn a_committed_first_save_is_finished_by_the_next_open() {
        use crate::save::{FirstSaveCut, FIRST_SAVE_CUT};
        let _g = WorldsRootGuard::new("open_first_save_publish");
        let (w, all) = many_chunk_world();
        crate::save::save_world_meta("w", &WorldMeta::new("w")).unwrap();
        FIRST_SAVE_CUT.with(|c| c.set(Some(FirstSaveCut::BeforePublish)));
        assert!(crate::save::save_world("w", &w, std::slice::from_ref(&slot()), 5, &[], &[]).is_err());
        FIRST_SAVE_CUT.with(|c| c.set(None));
        let dir = world_dir("w");
        assert!(dir.join("world.dat").is_file(), "committed");
        assert!(saved_chunk_file(&dir).unwrap().is_none(), "not yet published");
        let mut back = World::new();
        assert!(matches!(
            open_world("w", &mut back, AutosavePolicy::Prefer),
            Ok(OpenedWorld::Loaded { chunks: 12, .. })
        ));
        assert_eq!(solid_chunks(&back), all);
        assert!(!dir.join("chunks.new").exists());
    }

    /// A first save stages beside a `chunks/` folder holding no chunk (a stray
    /// tmp left by a crash): that folder is kept aside, never lost, and the
    /// staged chunks are published.
    #[test]
    fn a_first_save_keeps_a_stray_chunks_folder_aside() {
        let _g = WorldsRootGuard::new("open_first_save_stray");
        let (w, all) = many_chunk_world();
        let dir = world_dir("w");
        fs::create_dir_all(dir.join("chunks")).unwrap();
        fs::write(dir.join("chunks/0_4_0.chunk.tmp"), b"half").unwrap();
        crate::save::save_world("w", &w, std::slice::from_ref(&slot()), 5, &[], &[]).unwrap();
        assert_eq!(reopened("w"), Some(all));
        let kept: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("chunks.corrupt-"))
            .collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(fs::read(kept[0].path().join("0_4_0.chunk.tmp")).unwrap(), b"half");
    }

    /// Review 2026-10-06 — setting a block to air where the streamer had dropped
    /// the column conjures an EMPTY chunk (`World::set_block` creates one), which
    /// the mined-out rule then took for a mined-out chunk and deleted the REAL
    /// file: a hole after the restart. Only a chunk that came from disk or was
    /// really edited (`persist`) loses its file.
    #[test]
    fn save_never_deletes_a_chunk_file_under_a_conjured_empty_chunk() {
        let _g = WorldsRootGuard::new("save_conjured_empty");
        // A world-gen (never edited) chunk at 0_4_0, written by a save.
        let mut w = World::new();
        let mut generated = crate::chunk::Chunk::new();
        generated.set(3, 0, 5, crate::block::STONE);
        assert!(!generated.persist());
        w.insert_chunk(0, 4, 0, generated);
        crate::save::save_world("w", &w, std::slice::from_ref(&slot()), 1, &[], &[]).unwrap();
        let dir = world_dir("w");
        let before = fs::read(dir.join("chunks/0_4_0.chunk")).unwrap();
        assert!(w.knows_disk_chunk((0, 4, 0)));
        // The column streams out (pristine: dropped, not kept), then a block is
        // set to air in it.
        assert!(!w.evict_column(0, 0));
        w.set_block(3, 64, 5, crate::block::AIR);
        let (_, delete) = crate::save::partition_chunks_for_save(&w);
        assert!(!delete.contains(&(0, 4, 0)), "a conjured empty chunk is not a mined-out one");
        crate::save::save_world("w", &w, std::slice::from_ref(&slot()), 1, &[], &[]).unwrap();
        assert_eq!(fs::read(dir.join("chunks/0_4_0.chunk")).unwrap(), before, "the real file stays");
        // The dedicated server shares the rule.
        let mut server = crate::server::GameServer::new(0, "w".into(), 1);
        server.world = w;
        server.try_save().unwrap();
        assert_eq!(fs::read(dir.join("chunks/0_4_0.chunk")).unwrap(), before);
    }

    /// Review 2026-10-06 — every save of the live session drops the autosave it
    /// superseded once it lands (the resumable-scenario start and replay
    /// snapshot saves left it behind, so a crash before the next autosave rolled
    /// the world back to it); a failed save keeps it.
    #[test]
    fn a_session_save_that_lands_drops_the_autosave_and_a_failed_one_keeps_it() {
        let _g = WorldsRootGuard::new("save_supersedes_autosave");
        let dir = saved_world("w");
        let s = slot();
        let mut w = World::new();
        w.set_block(3, 64, 5, crate::block::STONE);
        crate::save::autosave_world("w", &w, std::slice::from_ref(&s), 77, &[], &[]).unwrap();
        assert!(crate::save::autosave_age("w").is_some());
        let good_meta = fs::read(dir.join("world_meta.json")).unwrap();
        fs::write(dir.join("world_meta.json"), b"{ torn").unwrap();
        assert!(
            crate::save::save_world_superseding_autosave("w", &w, std::slice::from_ref(&s), 77, &[], &[])
                .is_err()
        );
        assert!(dir.join("autosave/world.dat").is_file(), "a failed save keeps the autosave");
        fs::write(dir.join("world_meta.json"), good_meta).unwrap();
        crate::save::save_world_superseding_autosave("w", &w, std::slice::from_ref(&s), 77, &[], &[])
            .unwrap();
        assert!(!dir.join("autosave").exists(), "a landed save drops it");
        assert_eq!(crate::save::autosave_age("w"), None);
        let mut back = World::new();
        assert!(matches!(
            open_world("w", &mut back, AutosavePolicy::Prefer),
            Ok(OpenedWorld::Loaded { from: OpenedFrom::LastSave, .. })
        ));
        assert_eq!(back.get_block(3, 64, 5), crate::block::STONE);
    }

    /// Only the FIRST save reorders: a later save still writes chunks first and
    /// `world.dat` last (its commit point), so when every chunk write fails the
    /// last consistent `world.dat` is left as it was.
    #[test]
    fn a_later_save_keeps_world_dat_as_the_commit_point() {
        let _g = WorldsRootGuard::new("open_later_save_commit");
        let dir = saved_world("w");
        let before = fs::read(dir.join("world.dat")).unwrap();
        let mut world = World::new();
        assert!(matches!(
            open_world("w", &mut world, AutosavePolicy::Prefer),
            Ok(OpenedWorld::Loaded { .. })
        ));
        fs::create_dir_all(dir.join("chunks/0_4_0.chunk.tmp")).unwrap();
        world.set_block(4, 64, 5, crate::block::STONE);
        assert!(crate::save::save_world("w", &world, std::slice::from_ref(&slot()), 1, &[], &[])
            .is_err());
        assert_eq!(fs::read(dir.join("world.dat")).unwrap(), before, "world.dat untouched");
    }

    // ── Third review (2026-10-06): an autosave never rolls a newer world.dat back ──

    /// Set `path`'s modification time `secs_ago` seconds back, so a test states
    /// which copy is newer instead of relying on write order.
    fn written_ago(path: &Path, secs_ago: u64) {
        let at = std::time::SystemTime::now() - std::time::Duration::from_secs(secs_ago);
        fs::File::options().write(true).open(path).unwrap().set_modified(at).unwrap();
    }

    /// The block at (3, 64, 5) in the copy `open_world` (client policy) opens.
    fn opened_block(name: &str) -> (OpenedFrom, crate::block::BlockId) {
        let mut world = World::new();
        match open_world(name, &mut world, AutosavePolicy::Prefer).unwrap() {
            OpenedWorld::Loaded { from, .. } => (from, world.get_block(3, 64, 5)),
            other => panic!("{other:?}"),
        }
    }

    /// A later save that fails AFTER `world.dat` committed (here its meta write)
    /// kept the older crash-recovery autosave, and the next open preferred it over
    /// the newer `world.dat`: the save rolled back. The autosave now goes the
    /// moment `world.dat` commits.
    #[test]
    fn a_save_that_fails_after_world_dat_commits_still_drops_the_older_autosave() {
        let _g = WorldsRootGuard::new("save_post_commit_fail");
        let dir = saved_world("w");
        let s = slot();
        let mut w = World::new();
        w.set_block(3, 64, 5, crate::block::STONE);
        crate::save::autosave_world("w", &w, std::slice::from_ref(&s), 77, &[], &[]).unwrap();
        // The session goes on, then saves; the meta write fails after world.dat.
        w.set_block(3, 64, 5, crate::block::DIRT);
        fs::create_dir_all(dir.join("world_meta.json.tmp")).unwrap();
        let err = crate::save::save_world_superseding_autosave("w", &w, std::slice::from_ref(&s), 77, &[], &[])
            .unwrap_err();
        assert!(err.contains("world_meta.json"), "{err}");
        assert!(!dir.join("autosave").exists(), "world.dat committed: the older autosave is superseded");
        fs::remove_dir(dir.join("world_meta.json.tmp")).unwrap();
        assert_eq!(opened_block("w"), (OpenedFrom::LastSave, crate::block::DIRT));
    }

    /// The same for a world's FIRST save cut short after its commit (the publish
    /// rename failed): the older autosave is dropped at the commit.
    #[test]
    fn a_first_save_that_fails_after_its_commit_still_drops_the_older_autosave() {
        use crate::save::{FirstSaveCut, FIRST_SAVE_CUT};
        let _g = WorldsRootGuard::new("first_save_post_commit_fail");
        let (w, all) = many_chunk_world();
        let dir = world_dir("w");
        crate::save::save_world_meta("w", &WorldMeta::new("w")).unwrap();
        // An autosave from before, holding a chunk the save no longer has.
        let mut older = many_chunk_world().0;
        older.set_block(200, 64, 200, crate::block::STONE);
        crate::save::autosave_world("w", &older, std::slice::from_ref(&slot()), 5, &[], &[]).unwrap();
        FIRST_SAVE_CUT.with(|c| c.set(Some(FirstSaveCut::BeforePublish)));
        let err = crate::save::save_world_superseding_autosave("w", &w, std::slice::from_ref(&slot()), 5, &[], &[])
            .unwrap_err();
        FIRST_SAVE_CUT.with(|c| c.set(None));
        assert!(err.contains("cut short"), "{err}");
        assert!(dir.join("world.dat").is_file() && dir.join(crate::save::STAGED_CHUNKS).is_dir());
        assert!(!dir.join("autosave").exists(), "world.dat committed: the older autosave is superseded");
        let mut back = World::new();
        assert!(matches!(
            open_world("w", &mut back, AutosavePolicy::Prefer),
            Ok(OpenedWorld::Loaded { from: OpenedFrom::LastSave, .. })
        ));
        assert_eq!(solid_chunks(&back), all);
    }

    /// The reviewer found every first-save test reopened the world before the
    /// next save, so the SAVE path's publish never ran. And the staged chunks
    /// were noted as on disk only after the publish: when that rename failed, the
    /// next save published them but never deleted one mined out since — it came
    /// back. They are now noted the moment `world.dat` commits.
    #[test]
    fn a_first_save_cut_after_its_commit_is_published_by_the_next_save() {
        use crate::save::{FirstSaveCut, FIRST_SAVE_CUT};
        let _g = WorldsRootGuard::new("first_save_published_by_save");
        let (mut w, all) = many_chunk_world();
        let dir = world_dir("w");
        crate::save::save_world_meta("w", &WorldMeta::new("w")).unwrap();
        FIRST_SAVE_CUT.with(|c| c.set(Some(FirstSaveCut::BeforePublish)));
        assert!(crate::save::save_world("w", &w, std::slice::from_ref(&slot()), 5, &[], &[]).is_err());
        FIRST_SAVE_CUT.with(|c| c.set(None));
        assert!(dir.join(crate::save::STAGED_CHUNKS).join("5_5_5.chunk").is_file(), "staged");
        // The session goes on: chunk (5, 5, 5)'s only solid block is mined out.
        w.set_block(90, 80, 90, crate::block::AIR);
        let mined = (5, 5, 5);
        assert!(all.contains(&mined) && !solid_chunks(&w).contains(&mined));
        // The next save — no open in between — publishes the staged chunks and
        // deletes the mined-out one.
        crate::save::save_world("w", &w, std::slice::from_ref(&slot()), 5, &[], &[]).unwrap();
        assert!(!dir.join(crate::save::STAGED_CHUNKS).exists(), "published by the save");
        assert!(dir.join("chunks/0_1_0.chunk").is_file(), "published as chunks/");
        assert!(!dir.join("chunks/5_5_5.chunk").exists(), "the mined-out chunk stays gone");
        let mut expected = all.clone();
        expected.remove(&mined);
        assert_eq!(reopened("w"), Some(expected));
    }

    /// The client preferred ANY autosave over `world.dat`. It now opens the
    /// autosave first only when it was written after `world.dat` (a tie goes to
    /// `world.dat`); an older one is stale and is cleared once `world.dat` has
    /// loaded. The server never reads it either way.
    #[test]
    fn the_autosave_is_opened_first_only_when_it_is_newer_than_world_dat() {
        let _g = WorldsRootGuard::new("open_autosave_newer_only");
        let dir = saved_world("w");
        let mut a = World::new();
        a.set_block(3, 64, 5, crate::block::STONE);
        crate::save::autosave_world("w", &a, std::slice::from_ref(&slot()), 77, &[], &[]).unwrap();
        let (dat, autosave) = (dir.join("world.dat"), dir.join("autosave/world.dat"));

        // Written after world.dat (a crash since): crash recovery.
        written_ago(&dat, 120);
        written_ago(&autosave, 60);
        assert_eq!(opened_block("w"), (OpenedFrom::Autosave, crate::block::STONE));
        assert!(autosave.is_file(), "an autosave the world opened from is kept");

        // A tie goes to world.dat, and so does an older autosave.
        let at = std::time::SystemTime::now() - std::time::Duration::from_secs(30);
        for f in [&dat, &autosave] {
            fs::File::options().write(true).open(f).unwrap().set_modified(at).unwrap();
        }
        let mut server = World::new();
        assert!(matches!(
            open_world("w", &mut server, AutosavePolicy::Ignore),
            Ok(OpenedWorld::Loaded { from: OpenedFrom::LastSave, .. })
        ));
        assert!(autosave.is_file(), "the server never touches the autosave");
        assert_eq!(opened_block("w"), (OpenedFrom::LastSave, crate::block::BEDROCK));
        assert!(!dir.join("autosave").exists(), "a stale autosave is cleared once world.dat loaded");
        assert_eq!(opened_block("w"), (OpenedFrom::LastSave, crate::block::BEDROCK));
    }

    /// A `world.dat` newer than the autosave that fails to load: the older
    /// autosave is the newest copy that opens. The damaged `world.dat` is COPIED
    /// aside (left in place, so the lobby still lists the world), the autosave is
    /// kept until a save lands, and the player is told.
    #[test]
    fn a_newer_world_dat_that_fails_falls_back_to_the_older_autosave() {
        let _g = WorldsRootGuard::new("open_newer_dat_fails");
        let dir = saved_world("w");
        let mut a = World::new();
        a.set_block(3, 64, 5, crate::block::STONE);
        crate::save::autosave_world("w", &a, std::slice::from_ref(&slot()), 77, &[], &[]).unwrap();
        fs::write(dir.join("world.dat"), b"\x01not a world save").unwrap();
        written_ago(&dir.join("autosave/world.dat"), 120);
        written_ago(&dir.join("world.dat"), 60);

        let (from, block) = opened_block("w");
        assert_eq!(block, crate::block::STONE, "opened from the autosave");
        assert!(from.opened_from_autosave(), "a discard must keep the only good copy");
        let OpenedFrom::AutosaveAfterLastSaveFailed { why, kept_as } = &from else {
            panic!("{from:?}");
        };
        assert!(why.contains("world.dat is damaged"), "{why}");
        assert!(kept_as.starts_with("world.dat.corrupt-"), "{kept_as}");
        assert_eq!(fs::read(dir.join(kept_as)).unwrap(), b"\x01not a world save");
        assert!(from.player_note().unwrap().contains(kept_as.as_str()));
        assert_eq!(fs::read(dir.join("world.dat")).unwrap(), b"\x01not a world save", "left in place");
        assert!(dir.join("autosave/world.dat").is_file(), "the autosave is kept");
    }

    /// ...and if that damaged `world.dat` can't be kept aside, or the older
    /// autosave fails too, the world is refused untouched.
    #[test]
    fn a_newer_world_dat_that_fails_is_refused_when_it_cant_be_kept_or_the_autosave_fails() {
        let _g = WorldsRootGuard::new("open_newer_dat_refused");
        let dir = saved_world("w");
        let mut a = World::new();
        a.set_block(3, 64, 5, crate::block::STONE);
        crate::save::autosave_world("w", &a, std::slice::from_ref(&slot()), 77, &[], &[]).unwrap();
        fs::write(dir.join("world.dat"), b"\x01not a world save").unwrap();
        written_ago(&dir.join("autosave/world.dat"), 120);
        written_ago(&dir.join("world.dat"), 60);
        let before = snapshot(&dir);
        QUARANTINE_FAILS_FOR.with(|f| *f.borrow_mut() = Some("world.dat".to_string()));
        let err = open_world("w", &mut World::new(), AutosavePolicy::Prefer).unwrap_err();
        QUARANTINE_FAILS_FOR.with(|f| *f.borrow_mut() = None);
        assert!(err.contains("kept aside"), "{err}");
        assert_eq!(snapshot(&dir), before);

        fs::write(dir.join("autosave/world.dat"), b"\x01torn autosave").unwrap();
        written_ago(&dir.join("autosave/world.dat"), 120);
        let before = snapshot(&dir);
        let err = open_world("w", &mut World::new(), AutosavePolicy::Prefer).unwrap_err();
        assert!(err.contains("world.dat is damaged") && err.contains("autosave failed too"), "{err}");
        assert_eq!(snapshot(&dir), before);
    }

    /// After a downgrade a folder can hold `world.dat`, `chunks/*.chunk` AND a
    /// `chunks.new/` (an older build saved over a committed first save it couldn't
    /// see), and it was refused forever. That `chunks.new/` is stale — the
    /// `chunks/` beside `world.dat` is newer — so it is set aside, never lost.
    #[test]
    fn a_stale_chunks_new_beside_saved_chunks_is_set_aside_not_refused() {
        let _g = WorldsRootGuard::new("open_stale_chunks_new");
        let dir = saved_world("w");
        let stage = |dir: &Path| {
            fs::create_dir_all(dir.join(crate::save::STAGED_CHUNKS)).unwrap();
            fs::write(dir.join(crate::save::STAGED_CHUNKS).join("9_4_9.chunk"), b"stale").unwrap();
        };
        let kept = |dir: &Path| -> Vec<std::path::PathBuf> {
            fs::read_dir(dir)
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("chunks.new.stale-"))
                .map(|e| e.path())
                .collect()
        };
        stage(&dir);
        assert_eq!(opened_block("w"), (OpenedFrom::LastSave, crate::block::BEDROCK));
        assert!(!dir.join(crate::save::STAGED_CHUNKS).exists());
        let aside = kept(&dir);
        assert_eq!(aside.len(), 1);
        assert_eq!(fs::read(aside[0].join("9_4_9.chunk")).unwrap(), b"stale");

        // A save finds it the same way.
        stage(&dir);
        let mut w = World::new();
        w.set_block(3, 64, 5, crate::block::DIRT);
        crate::save::save_world("w", &w, std::slice::from_ref(&slot()), 77, &[], &[]).unwrap();
        assert!(!dir.join(crate::save::STAGED_CHUNKS).exists());
        assert_eq!(kept(&dir).len(), 2);
        assert_eq!(opened_block("w"), (OpenedFrom::LastSave, crate::block::DIRT));
    }
}
