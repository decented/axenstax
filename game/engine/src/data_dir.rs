//! Native per-user data directory — the one root every piece of native state
//! (worlds, `profile/` secrets and blobs, settings, texture packs) lives under.
//!
//! Audit 2026-09-27 ("All native state, including secret keys, lives under the
//! current working directory"): every path used to be CWD-relative, so launching
//! the AppImage from `~/Downloads` or a file manager made all worlds "vanish" and
//! wrote the persona runtime key / bunker session into whatever
//! folder the player happened to be in. Now:
//!
//! - [`data_root`] resolves once: `$AXENSTAX_DATA_DIR` if set, else the platform
//!   data dir (`$XDG_DATA_HOME/axenstax` or `~/.local/share/axenstax` on Linux,
//!   `~/Library/Application Support/axenstax` on macOS, `%APPDATA%\axenstax` on
//!   Windows). No crate dependency — the rule is a few env lookups.
//! - `AXENSTAX_WORLDS_DIR` (the dedicated server's Docker volume) still overrides
//!   the worlds folder alone — see `save::worlds_root`.
//! - [`init`] runs once at startup and, unless the root carries the
//!   `.migrated` marker, COPIES any legacy CWD-relative state into it via a
//!   staging dir renamed into place (never moves or deletes the originals). An
//!   explicit `AXENSTAX_DATA_DIR` is used in place and never migrated into.
//!
//! Web (WASM) has no filesystem: [`profile_dir`] keeps its old relative value
//! there so the (unused) shared call sites still compile.

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;

/// Override the data root (tests, portable installs, a dev's scratch dir).
pub const DATA_DIR_ENV: &str = "AXENSTAX_DATA_DIR";

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
/// Top-level CWD entries that the pre-data-dir engine wrote, copied by the
/// one-time migration.
const LEGACY_ENTRIES: &[&str] = &["worlds", "profile", "texturepacks", "settings.json", "my_servers.json"];

/// The platform data dir for AxeNStax, from an env lookup (injected for tests).
/// `None` when the platform's variables are unset — the caller then falls back
/// to the CWD, the old behaviour, with a warning.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn resolve_with(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let get = |k: &str| env(k).filter(|v| !v.is_empty());
    if let Some(d) = get(DATA_DIR_ENV) {
        return Some(PathBuf::from(d));
    }
    if cfg!(windows) {
        return get("APPDATA").map(|a| PathBuf::from(a).join("axenstax"));
    }
    if cfg!(target_os = "macos") {
        return get("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/axenstax"));
    }
    // XDG base-dir spec: a relative XDG_DATA_HOME is invalid and ignored.
    if let Some(x) = get("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
        return Some(x.join("axenstax"));
    }
    get("HOME").map(|h| PathBuf::from(h).join(".local/share/axenstax"))
}

/// The data root chosen for this process (set by [`init`], or resolved lazily).
#[cfg(not(target_arch = "wasm32"))]
static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// The resolved data root, and whether it was set explicitly via
/// [`DATA_DIR_ENV`] (then nothing is ever migrated into it).
#[cfg(not(target_arch = "wasm32"))]
fn resolved() -> (PathBuf, bool) {
    let explicit = std::env::var(DATA_DIR_ENV).is_ok_and(|v| !v.is_empty());
    let root = resolve_with(|k| std::env::var(k).ok()).unwrap_or_else(|| {
        log::warn!("no per-user data dir (HOME/APPDATA unset); using the current directory");
        PathBuf::from(".")
    });
    (root, explicit)
}

/// The root of all native state. Fixed once per process by [`init`].
#[cfg(all(not(target_arch = "wasm32"), not(test)))]
pub fn data_root() -> PathBuf {
    ROOT.get_or_init(|| resolved().0).clone()
}

/// Tests never touch the developer's real data dir: a per-process temp root.
#[cfg(all(not(target_arch = "wasm32"), test))]
pub fn data_root() -> PathBuf {
    std::env::temp_dir().join(format!("axenstax-test-data-{}", std::process::id()))
}

#[cfg(target_arch = "wasm32")]
pub fn data_root() -> PathBuf {
    PathBuf::new()
}

/// `<data root>/profile` — secrets (runtime key, Signet session, feedback tickets)
/// and profile blobs (skins, wardrobe, trials, contacts, replays).
pub fn profile_dir() -> PathBuf {
    data_root().join("profile")
}

/// Written last into a migrated (or freshly created) data root. A root without
/// it is never treated as a finished migration.
pub const MIGRATED_MARKER: &str = ".migrated";

/// Startup: fix the data root for this process and run the one-time migration
/// from the current directory. Call before anything reads native state.
///
/// - `AXENSTAX_DATA_DIR` set → that dir is used in place; nothing is migrated or
///   copied (the dedicated server points it at its `/worlds` volume).
/// - Otherwise [`prepare_root`] copies legacy CWD state in via a staging dir. If
///   that fails, THIS session runs on the legacy CWD state in place (nothing is
///   written to the new root), so the next launch retries the migration.
#[cfg(not(target_arch = "wasm32"))]
pub fn init() {
    let (root, explicit) = resolved();
    let chosen = choose_root(root, explicit, std::env::current_dir().ok());
    log::info!("native data dir: {}", chosen.display());
    let _ = ROOT.set(chosen);
}

/// Android startup: the app's private storage (`internal_data_path()`) IS the
/// data root. Used instead of [`init`] by `android_main`.
///
/// [`init`] cannot work there: an Android app process has no `HOME`,
/// `XDG_DATA_HOME` or `APPDATA`, so `resolve_with` returns `None` and the
/// migration would run against `"."` — and the process starts with its cwd at
/// `/`, which is not writable. There is also nothing to migrate: no pre-data-dir
/// APK ever shipped. Private storage needs no permission and is removed on
/// uninstall, which is the right lifetime for saves.
#[cfg(target_os = "android")]
pub fn init_at(root: PathBuf) {
    if let Err(e) = std::fs::create_dir_all(&root) {
        log::warn!("could not create data dir {}: {e}", root.display());
    }
    log::info!("native data dir: {}", root.display());
    let _ = ROOT.set(root);
}

/// The testable half of [`init`]: which dir this session uses.
#[cfg(not(target_arch = "wasm32"))]
fn choose_root(root: PathBuf, explicit: bool, cwd: Option<PathBuf>) -> PathBuf {
    if explicit {
        // Explicit override (e.g. the server's /worlds volume): used in place,
        // never migrated into or copied from.
        if let Err(e) = std::fs::create_dir_all(&root) {
            log::warn!("could not create data dir {}: {e}", root.display());
        }
        return root;
    }
    let Some(cwd) = cwd else {
        log::warn!("no current dir to migrate from");
        let _ = create_private_dir(&root);
        return root;
    };
    match prepare_root(&root, &cwd) {
        Ok(true) => {
            log::info!(
                "copied existing worlds/profile from {} into {} (originals left in place)",
                cwd.display(),
                root.display()
            );
            root
        }
        Ok(false) => root,
        Err(e) => {
            log::error!(
                "data-dir migration from {} failed ({e}); using the old location this \
                 session, will retry next launch",
                cwd.display()
            );
            cwd
        }
    }
}

/// Make `root` ready, migrating legacy state from `legacy` if needed. Returns
/// whether anything was copied. Crash/failure-safe (review B1):
///
/// 1. A root holding [`MIGRATED_MARKER`] is done — no-op.
/// 2. Legacy state is copied into a staging dir BESIDE the root
///    (`.<name>.migrating`), the marker is written into it last, and only then is
///    the staging dir renamed into place — so a half-copied root never exists.
/// 3. Any failure deletes the staging dir and returns `Err`; the root is left
///    absent, so the next launch retries from the legacy dir.
///
/// Never moves or deletes anything in `legacy`. `fs::copy` keeps mode bits, so
/// `0600` secrets stay `0600`. Only dirs this function creates are chmodded 0700.
#[cfg(not(target_arch = "wasm32"))]
pub fn prepare_root(root: &Path, legacy: &Path) -> Result<bool, String> {
    if root.join(MIGRATED_MARKER).exists() {
        return Ok(false);
    }
    let parent = root.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "axenstax".into());
    let staging = parent.join(format!(".{name}.migrating"));
    let _ = std::fs::remove_dir_all(&staging); // leftovers of an interrupted run

    if root.exists() {
        let empty = std::fs::read_dir(root)
            .map_err(|e| format!("read {}: {e}", root.display()))?
            .next()
            .is_none();
        if !empty {
            // A populated root without our marker was made by something else
            // (the player, an older build): use it as it is, never overwrite it.
            write_marker(root)?;
            return Ok(false);
        }
        std::fs::remove_dir(root).map_err(|e| format!("clear empty {}: {e}", root.display()))?;
    }

    let same = matches!((legacy.canonicalize(), parent.canonicalize()), (Ok(a), Ok(p)) if a == p.join(&name));
    let present: Vec<&str> = if same {
        Vec::new()
    } else {
        LEGACY_ENTRIES.iter().copied().filter(|e| legacy.join(e).exists()).collect()
    };
    if present.is_empty() {
        create_private_dir(root)?;
        write_marker(root)?;
        return Ok(false);
    }

    let result = (|| {
        create_private_dir(&staging)?;
        for entry in &present {
            copy_recursive(&legacy.join(entry), &staging.join(entry))?;
        }
        write_marker(&staging)?;
        std::fs::rename(&staging, root)
            .map_err(|e| format!("rename {} -> {}: {e}", staging.display(), root.display()))
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    Ok(true)
}

#[cfg(not(target_arch = "wasm32"))]
fn create_private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn write_marker(dir: &Path) -> Result<(), String> {
    std::fs::write(dir.join(MIGRATED_MARKER), b"1\n")
        .map_err(|e| format!("write migration marker in {}: {e}", dir.display()))
}

#[cfg(not(target_arch = "wasm32"))]
fn copy_recursive(from: &Path, to: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(from).map_err(|e| format!("stat {}: {e}", from.display()))?;
    if meta.file_type().is_symlink() {
        // Don't follow links out of the legacy tree.
        log::warn!("migration: skipping symlink {}", from.display());
        return Ok(());
    }
    if meta.is_dir() {
        std::fs::create_dir_all(to).map_err(|e| format!("mkdir {}: {e}", to.display()))?;
        #[cfg(unix)]
        {
            let _ = std::fs::set_permissions(to, meta.permissions());
        }
        for child in std::fs::read_dir(from).map_err(|e| format!("read {}: {e}", from.display()))? {
            let child = child.map_err(|e| format!("entry: {e}"))?;
            copy_recursive(&child.path(), &to.join(child.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to)
            .map(|_| ())
            .map_err(|e| format!("copy {} -> {}: {e}", from.display(), to.display()))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axe_datadir_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn the_override_env_wins() {
        let got = resolve_with(env_of(&[(DATA_DIR_ENV, "/data/axe"), ("HOME", "/usr/u")]));
        assert_eq!(got, Some(PathBuf::from("/data/axe")));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_uses_xdg_data_home_then_home() {
        assert_eq!(
            resolve_with(env_of(&[("XDG_DATA_HOME", "/x/share"), ("HOME", "/usr/u")])),
            Some(PathBuf::from("/x/share/axenstax"))
        );
        assert_eq!(
            resolve_with(env_of(&[("XDG_DATA_HOME", "rel/share"), ("HOME", "/usr/u")])),
            Some(PathBuf::from("/usr/u/.local/share/axenstax")),
            "a relative XDG_DATA_HOME is ignored per the spec"
        );
        assert_eq!(resolve_with(env_of(&[])), None);
    }

    #[test]
    fn migration_copies_legacy_state_and_leaves_the_originals() {
        let src = scratch("mig_src");
        let dst = scratch("mig_dst").join("axenstax");
        fs::create_dir_all(src.join("worlds/w1")).unwrap();
        fs::write(src.join("worlds/w1/world.dat"), b"world").unwrap();
        fs::create_dir_all(src.join("profile")).unwrap();
        fs::write(src.join("profile/runtime_key.json"), b"secret").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(src.join("profile/runtime_key.json"), fs::Permissions::from_mode(0o600))
                .unwrap();
        }
        fs::write(src.join("settings.json"), b"{}").unwrap();
        fs::write(src.join("unrelated.txt"), b"x").unwrap();

        assert_eq!(prepare_root(&dst, &src), Ok(true));
        assert!(dst.join(MIGRATED_MARKER).exists());
        assert_eq!(fs::read(dst.join("worlds/w1/world.dat")).unwrap(), b"world");
        assert_eq!(fs::read(dst.join("profile/runtime_key.json")).unwrap(), b"secret");
        assert!(dst.join("settings.json").exists());
        assert!(!dst.join("unrelated.txt").exists(), "only engine state is copied");
        // Originals untouched.
        assert_eq!(fs::read(src.join("profile/runtime_key.json")).unwrap(), b"secret");
        assert!(src.join("worlds/w1/world.dat").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dst.join("profile/runtime_key.json")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "secret keys keep 0600");
        }
        // Second launch: marker present → no-op.
        assert_eq!(prepare_root(&dst, &src), Ok(false));
        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(dst.parent().unwrap());
    }

    /// Review B1: a copy that fails part-way leaves NO root (so nothing uses a
    /// half-populated dir) and the next launch retries and completes.
    #[cfg(unix)]
    #[test]
    fn a_failed_migration_leaves_no_root_and_the_next_launch_retries() {
        use std::os::unix::fs::PermissionsExt;
        let src = scratch("mig_fail_src");
        let dst = scratch("mig_fail_dst").join("axenstax");
        fs::create_dir_all(src.join("worlds/w1")).unwrap();
        fs::write(src.join("worlds/w1/world.dat"), b"world").unwrap();
        fs::create_dir_all(src.join("profile")).unwrap();
        let key = src.join("profile/runtime_key.json");
        fs::write(&key, b"secret").unwrap();
        fs::set_permissions(&key, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&key).is_ok() {
            // Running as root: permissions can't simulate the failure.
            fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
            let _ = fs::remove_dir_all(&src);
            return;
        }

        assert!(prepare_root(&dst, &src).is_err(), "the unreadable file fails the copy");
        assert!(!dst.exists(), "no half-populated root may exist");
        assert!(!dst.parent().unwrap().join(".axenstax.migrating").exists(), "staging cleaned up");

        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(prepare_root(&dst, &src), Ok(true), "the next launch retries");
        assert!(dst.join(MIGRATED_MARKER).exists());
        assert_eq!(fs::read(dst.join("profile/runtime_key.json")).unwrap(), b"secret");
        assert_eq!(fs::read(dst.join("worlds/w1/world.dat")).unwrap(), b"world");
        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(dst.parent().unwrap());
    }

    #[test]
    fn a_populated_root_without_a_marker_is_never_overwritten() {
        let src = scratch("mig2_src");
        let dst = scratch("mig2_dst");
        fs::create_dir_all(src.join("worlds/old")).unwrap();
        fs::create_dir_all(dst.join("worlds/current")).unwrap();
        assert_eq!(prepare_root(&dst, &src), Ok(false));
        assert!(!dst.join("worlds/old").exists());
        assert!(dst.join("worlds/current").exists());
        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn nothing_to_migrate_creates_a_marked_root() {
        let src = scratch("mig3_src");
        let dst = scratch("mig3_dst").join("axenstax");
        assert_eq!(prepare_root(&dst, &src), Ok(false));
        assert!(dst.join(MIGRATED_MARKER).exists());
        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(dst.parent().unwrap());
    }

    #[test]
    fn an_explicit_data_dir_is_never_migrated_into() {
        let src = scratch("mig4_src");
        let dst = scratch("mig4_dst").join("explicit");
        fs::create_dir_all(src.join("worlds/w")).unwrap();
        let got = choose_root(dst.clone(), true, Some(src.clone()));
        assert_eq!(got, dst);
        assert!(!dst.join("worlds").exists(), "nothing copied into an explicit dir");
        assert!(!dst.join(MIGRATED_MARKER).exists());
        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(dst.parent().unwrap());
    }
}
