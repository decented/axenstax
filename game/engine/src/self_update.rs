//! In-place AppImage self-updater.
//!
//! NATIVE-ONLY, and further gated to a build that IS a running AppImage —
//! `.deb`/cargo-run/tarball builds have no single file to overwrite, so
//! [`running_appimage`] is the gate the caller (`menu.rs`) checks before
//! offering the "Update now" button at all.
//!
//! Spec: `docs/superpowers/specs/2026-07-29-native-version-update-indicator-design.md`
//! (§ "In-place update").
//!
//! ## Shape
//!
//! Same off-thread + process-global pattern as `update_check.rs`: [`start`]
//! spawns a worker thread that downloads, verifies, and installs the new
//! AppImage, reporting progress through a `Mutex<Progress>` the menu polls
//! every frame with [`progress`]. Nothing here touches the render thread.
//!
//! ## Safety model
//!
//! The running binary is never touched until it has been fully downloaded
//! AND its sha256 has been checked against the manifest value fetched over
//! HTTPS from the same origin the version check itself trusts. The new build
//! is streamed to a sibling `.part` file first; only a verified `.part` is
//! renamed over the target, and a rename within the same directory is atomic
//! on Linux — so a crash or a kill mid-download can never leave a half-written
//! or corrupt AppImage in the target's place. Any failure — network, disk,
//! hash mismatch — removes the `.part` and leaves the running binary exactly
//! as it was; the player falls back to the manual download link.
//!
//! ## Privacy
//!
//! The download is a plain GET of the exact URL named in `latest.json` — same
//! posture as `update_check.rs`: no identifiers, no cookies, no telemetry.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use crate::update_check::AppImageRef;

/// What the "Update now" control should show right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// No update has been started this process.
    Idle,
    /// Streaming the new AppImage to the `.part` file. `total` is `None`
    /// when the server didn't send `Content-Length`.
    Downloading { received: u64, total: Option<u64> },
    /// Download complete; checking its sha256 against the manifest value.
    Verifying,
    /// Verified and installed over the running binary. Carries the version so
    /// the UI can say what it's now running (pending a restart).
    Installed { version: String },
    /// Any failure — network, disk, hash mismatch. Short human-readable
    /// reason; the running binary was left untouched.
    Failed(String),
}

/// Process-global, for the same reason as `update_check::RESULT`: the menu
/// redraws every frame and there is exactly one update in flight at most.
static PROGRESS: Mutex<Progress> = Mutex::new(Progress::Idle);

fn set_progress(p: Progress) {
    if let Ok(mut slot) = PROGRESS.lock() {
        *slot = p;
    }
}

/// What to show right now. Cheap enough to call every frame.
pub fn progress() -> Progress {
    PROGRESS.lock().map(|p| p.clone()).unwrap_or(Progress::Idle)
}

/// The path of the AppImage currently running, or `None` when this build
/// wasn't launched as an AppImage (a `.deb` install, `cargo run`, or a
/// bare-binary tarball extraction) — the AppImage runtime sets `APPIMAGE` to
/// the absolute path of the `.AppImage` file itself before exec'ing the
/// embedded binary. Requires the path to actually name an existing regular
/// file, not just a set env var, so a stale/wrong env can't send `install_over`
/// at a target that doesn't exist.
pub fn running_appimage() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("APPIMAGE")?);
    if std::fs::metadata(&path).ok()?.is_file() { Some(path) } else { None }
}

/// Start (or no-op) an in-place update. Safe to call repeatedly — while a
/// download/verify is in flight, or after a successful install, this does
/// nothing; a fresh `Idle` or a previous `Failed` both start a new attempt
/// (so a failed update can be retried by clicking the button again).
///
/// Returns immediately; the work runs on a worker thread.
pub fn start(target: PathBuf, reference: AppImageRef, version: String) {
    {
        let mut guard = match PROGRESS.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if matches!(*guard, Progress::Downloading { .. } | Progress::Verifying | Progress::Installed { .. }) {
            return;
        }
        *guard = Progress::Downloading { received: 0, total: None };
    }
    std::thread::spawn(move || {
        match download_and_install(&target, &reference) {
            Ok(()) => set_progress(Progress::Installed { version }),
            Err(reason) => {
                log::warn!("self-update failed: {reason}");
                set_progress(Progress::Failed(reason));
            }
        }
    });
}

/// Download the new AppImage to a sibling `.part` file (reporting
/// [`Progress::Downloading`] as bytes arrive), then verify + install it over
/// `target`. Tries the signed `reference.url` first, then each mirror; every
/// candidate's bytes are held to the SIGNED `reference.sha256` (the mirrors come
/// from the unsigned HTTP manifest — audit 2026-09-27). On ANY error the `.part`
/// is removed and `target` is left untouched — see the module doc's safety model.
fn download_and_install(target: &Path, reference: &AppImageRef) -> Result<(), String> {
    install_from_candidates(target, reference, download_to)
}

/// The `.part` path for `reference` next to `target`, after checking the
/// filename is a safe basename (`update_check::is_valid_appimage_filename`) so
/// the temp file can only ever land in the target AppImage's own directory.
pub fn part_path(target: &Path, filename: &str) -> Result<PathBuf, String> {
    if !crate::update_check::is_valid_appimage_filename(filename) {
        return Err(format!("refusing unsafe update filename {filename:?}"));
    }
    let dir = target.parent().ok_or_else(|| "update target has no parent directory".to_string())?;
    let part = dir.join(format!(".{filename}.part"));
    if part.parent() != Some(dir) {
        return Err("update temp file would land outside the AppImage's directory".to_string());
    }
    Ok(part)
}

/// Try each candidate URL (signed `url`, then `mirrors`) in turn: `fetch` writes
/// the bytes to the `.part` file, then [`finish_install`] checks the SIGNED
/// sha256 BEFORE the swap. A network failure or a hash mismatch moves on to the
/// next candidate; the first verified download is installed. Split out with an
/// injectable `fetch` so the trust rules are testable without a network.
pub fn install_from_candidates(
    target: &Path,
    reference: &AppImageRef,
    mut fetch: impl FnMut(&str, &Path) -> Result<(), String>,
) -> Result<(), String> {
    let part = part_path(target, &reference.filename)?;
    let mut last_err = "no download location".to_string();
    for url in std::iter::once(&reference.url).chain(reference.mirrors.iter()) {
        let attempt = fetch(url, &part).and_then(|()| {
            set_progress(Progress::Verifying);
            finish_install(&part, target, &reference.sha256)
        });
        match attempt {
            Ok(()) => return Ok(()),
            Err(e) => {
                // Covers every early exit (a `finish_install` failure already
                // cleans up its own part; a redundant remove is harmless).
                let _ = std::fs::remove_file(&part);
                log::warn!("self-update: {url} failed: {e}");
                last_err = e;
            }
        }
    }
    Err(last_err)
}

/// Stream `url` into `part`, reporting progress.
fn download_to(url: &str, part: &Path) -> Result<(), String> {
    // 30s timeout + no auto-follow-redirect: same SSRF/hang posture as
    // `update_check::fetch_latest_version` and `mc_import::http_get_text`,
    // just a longer timeout because this body is megabytes, not a few
    // hundred bytes of JSON.
    // NOT `.timeout(..)`: in ureq 2 that caps the WHOLE request including
    // reading the body, so a 100+ MiB AppImage on a slow link would abort
    // at the deadline. Connect + per-read timeouts bound each step without
    // bounding the total.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(30))
        .redirects(0)
        .build();
    let resp = agent.get(url).call().map_err(|e| format!("download request failed: {e}"))?;
    let total = resp.header("Content-Length").and_then(|s| s.parse::<u64>().ok());
    if total.is_some_and(|t| t > MAX_DOWNLOAD_BYTES) {
        return Err(format!("update is larger than the {MAX_DOWNLOAD_BYTES}-byte limit"));
    }
    set_progress(Progress::Downloading { received: 0, total });
    stream_capped(resp.into_reader(), part, MAX_DOWNLOAD_BYTES, |received| {
        set_progress(Progress::Downloading { received, total })
    })
}

/// Hard ceiling on a downloaded AppImage (review S6). The sha256 is only known
/// at EOF, so without this a hostile mirror serving an endless body would fill
/// the disk holding the AppImage.
pub const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;

/// Stream `body` into `part`, erroring as soon as more than `cap` bytes arrive
/// (the caller deletes the `.part` on any error).
pub fn stream_capped(
    mut body: impl Read,
    part: &Path,
    cap: u64,
    mut on_progress: impl FnMut(u64),
) -> Result<(), String> {
    let mut file = std::fs::File::create(part).map_err(|e| format!("create download file: {e}"))?;
    let mut buf = vec![0u8; 1024 * 1024];
    let mut received: u64 = 0;
    loop {
        let n = body.read(&mut buf).map_err(|e| format!("download read failed: {e}"))?;
        if n == 0 {
            break;
        }
        received += n as u64;
        if received > cap {
            return Err(format!("download exceeded the {cap}-byte limit"));
        }
        file.write_all(&buf[..n]).map_err(|e| format!("write download chunk: {e}"))?;
        on_progress(received);
    }
    Ok(())
}

/// Verify `path` against `expected_hex` (case-insensitive), THEN install it
/// over `target` — in that order, so a corrupt/tampered download is never
/// renamed into place. On failure (mismatch or install error) `path` is
/// removed and `target` is never touched. Factored out of the worker so a
/// test can drive the exact same sequence without a network call.
pub fn finish_install(part: &Path, target: &Path, expected_hex: &str) -> Result<(), String> {
    let result = verify_sha256(part, expected_hex).and_then(|()| install_over(part, target));
    if result.is_err() {
        let _ = std::fs::remove_file(part);
    }
    result
}

/// Streaming sha256 check. `expected_hex` is compared case-insensitively
/// (`update_check::parse_appimage_ref` already lowercases the manifest value,
/// but this function doesn't rely on that).
pub fn verify_sha256(path: &Path, expected_hex: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};

    let mut file = std::fs::File::open(path).map_err(|e| format!("open for verify: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| format!("read for verify: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let got = hex::encode(hasher.finalize());
    if got.eq_ignore_ascii_case(expected_hex) {
        Ok(())
    } else {
        Err("downloaded file's sha256 does not match the signed release".to_string())
    }
}

/// Make `part` executable and atomically move it over `target` (same
/// directory ⇒ `rename` is atomic on Linux — no window where `target` is a
/// truncated or partial file). On rename failure `target` is left untouched;
/// the caller (`finish_install`) is responsible for removing `part`.
pub fn install_over(part: &Path, target: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(part).map_err(|e| format!("stat downloaded file: {e}"))?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(part, perms).map_err(|e| format!("make downloaded file executable: {e}"))?;
    }
    std::fs::rename(part, target).map_err(|e| format!("install: {e}"))
}

/// Launch `appimage` as a new process and return immediately. The caller is
/// responsible for exiting the current process afterwards (`std::process::
/// exit(0)`), so the two AppImage instances never overlap running the same
/// world. Not unit-tested — it's a two-line `Command::spawn` wrapper whose
/// only interesting behaviour is "launched a whole other process," which
/// isn't something a test in this process can safely assert on.
pub fn relaunch(appimage: &Path) -> Result<(), String> {
    std::process::Command::new(appimage).spawn().map_err(|e| format!("relaunch failed: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A fresh, empty directory under the OS temp dir for this test process.
    /// Cleaned up best-effort at the end of each test — these are plain data
    /// files (never executed by the test itself), same pattern as
    /// `wardrobe_store.rs`'s tests.
    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("axe_self_update_test_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(bytes))
    }

    #[test]
    fn verify_sha256_accepts_a_matching_file() {
        let dir = scratch_dir("verify_ok");
        let path = dir.join("payload.bin");
        let bytes = b"the new AppImage bytes";
        fs::write(&path, bytes).unwrap();
        assert_eq!(verify_sha256(&path, &sha256_hex(bytes)), Ok(()));
    }

    #[test]
    fn verify_sha256_is_case_insensitive() {
        let dir = scratch_dir("verify_case");
        let path = dir.join("payload.bin");
        let bytes = b"case insensitive check";
        fs::write(&path, bytes).unwrap();
        assert_eq!(verify_sha256(&path, &sha256_hex(bytes).to_uppercase()), Ok(()));
    }

    #[test]
    fn verify_sha256_rejects_a_mismatched_file() {
        let dir = scratch_dir("verify_mismatch");
        let path = dir.join("payload.bin");
        fs::write(&path, b"actual bytes").unwrap();
        let wrong = sha256_hex(b"different bytes");
        assert!(verify_sha256(&path, &wrong).is_err());
    }

    #[test]
    fn verify_sha256_errors_on_a_missing_file() {
        let dir = scratch_dir("verify_missing");
        let path = dir.join("nope.bin");
        assert!(verify_sha256(&path, &sha256_hex(b"anything")).is_err());
    }

    #[test]
    fn install_over_replaces_the_target_and_makes_it_executable_and_removes_the_part() {
        let dir = scratch_dir("install_ok");
        let part = dir.join(".new.AppImage.part");
        let target = dir.join("axenstax-engine.AppImage");
        fs::write(&part, b"new build bytes").unwrap();
        fs::write(&target, b"old build bytes").unwrap();

        assert_eq!(install_over(&part, &target), Ok(()));

        assert_eq!(fs::read(&target).unwrap(), b"new build bytes");
        assert!(!part.exists(), "part file should be gone after rename");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&target).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755, "installed file should be executable");
        }
    }

    #[test]
    fn finish_install_verifies_before_renaming() {
        let dir = scratch_dir("finish_ok");
        let part = dir.join(".new.AppImage.part");
        let target = dir.join("axenstax-engine.AppImage");
        let bytes = b"verified new build";
        fs::write(&part, bytes).unwrap();
        fs::write(&target, b"old build").unwrap();

        assert_eq!(finish_install(&part, &target, &sha256_hex(bytes)), Ok(()));
        assert_eq!(fs::read(&target).unwrap(), bytes);
        assert!(!part.exists());
    }

    /// The ordering the worker relies on: a hash mismatch must be caught
    /// BEFORE any rename touches the target, and the failed download's `.part`
    /// must not linger.
    #[test]
    fn finish_install_on_mismatch_leaves_the_old_target_untouched_and_removes_the_part() {
        let dir = scratch_dir("finish_mismatch");
        let part = dir.join(".new.AppImage.part");
        let target = dir.join("axenstax-engine.AppImage");
        fs::write(&part, b"corrupt or tampered bytes").unwrap();
        fs::write(&target, b"old build bytes, must survive").unwrap();

        let wrong_hash = sha256_hex(b"not what's in part");
        let result = finish_install(&part, &target, &wrong_hash);

        assert!(result.is_err(), "mismatched hash must fail");
        assert_eq!(
            fs::read(&target).unwrap(),
            b"old build bytes, must survive",
            "target must be untouched on a verify failure"
        );
        assert!(!part.exists(), "part must be removed after a failed verify");
    }

    #[test]
    fn running_appimage_is_none_without_the_env_var() {
        // Doesn't touch the real process env (no other test in this module
        // sets APPIMAGE), just asserts the negative path when it's absent.
        // SAFETY: single-threaded per-test env access is the accepted
        // pattern for these tests; nothing else in this test reads APPIMAGE
        // concurrently.
        unsafe {
            std::env::remove_var("APPIMAGE");
        }
        assert_eq!(running_appimage(), None);
    }

    #[test]
    fn running_appimage_is_none_when_the_env_var_names_a_missing_file() {
        unsafe {
            std::env::set_var("APPIMAGE", "/nonexistent/path/for/axenstax/self_update_test.AppImage");
        }
        assert_eq!(running_appimage(), None);
        unsafe {
            std::env::remove_var("APPIMAGE");
        }
    }

    #[test]
    fn running_appimage_returns_the_path_when_it_names_a_real_file() {
        let dir = scratch_dir("running_ok");
        let path = dir.join("axenstax.AppImage");
        fs::write(&path, b"pretend appimage").unwrap();
        unsafe {
            std::env::set_var("APPIMAGE", &path);
        }
        assert_eq!(running_appimage(), Some(path));
        unsafe {
            std::env::remove_var("APPIMAGE");
        }
    }

    fn signed_ref(sha: &str, mirrors: Vec<String>) -> AppImageRef {
        AppImageRef {
            filename: "axenstax-engine_0.3.0_x86_64.AppImage".to_string(),
            url: "https://signed.example/a.AppImage".to_string(),
            sha256: sha.to_string(),
            mirrors,
        }
    }

    /// Audit 2026-09-27: a mirror (from the unsigned manifest) serving other
    /// bytes is refused, and the target is untouched.
    #[test]
    fn a_mirror_is_installed_only_if_its_bytes_match_the_signed_hash() {
        let dir = scratch_dir("mirror_hash");
        let target = dir.join("AxeNStax.AppImage");
        fs::write(&target, b"old build").unwrap();
        let good = b"the signed build";
        let r = signed_ref(&sha256_hex(good), vec!["https://mirror.example/m.AppImage".into()]);

        // Signed URL unreachable, mirror serves an attacker's binary → refused.
        let res = install_from_candidates(&target, &r, |url, part| {
            if url.starts_with("https://signed") {
                Err("offline".into())
            } else {
                fs::write(part, b"evil").map_err(|e| e.to_string())
            }
        });
        assert!(res.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old build");
        assert!(!dir.join(".axenstax-engine_0.3.0_x86_64.AppImage.part").exists());

        // Same mirror serving the signed bytes → installed.
        let res = install_from_candidates(&target, &r, |url, part| {
            if url.starts_with("https://signed") {
                Err("offline".into())
            } else {
                fs::write(part, good).map_err(|e| e.to_string())
            }
        });
        assert_eq!(res, Ok(()));
        assert_eq!(fs::read(&target).unwrap(), good);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unsafe_filename_is_refused_before_any_download() {
        let dir = scratch_dir("unsafe_name");
        let target = dir.join("AxeNStax.AppImage");
        fs::write(&target, b"old build").unwrap();
        let mut r = signed_ref(&sha256_hex(b"x"), vec![]);
        r.filename = "/../../x".to_string();
        let mut fetched = false;
        let res = install_from_candidates(&target, &r, |_, _| {
            fetched = true;
            Ok(())
        });
        assert!(res.is_err());
        assert!(!fetched, "no download may start with an unsafe filename");
        assert_eq!(fs::read(&target).unwrap(), b"old build");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_part_file_sits_next_to_the_target() {
        let target = Path::new("/opt/apps/AxeNStax.AppImage");
        let part = part_path(target, "axenstax-engine_0.3.0_x86_64.AppImage").unwrap();
        assert_eq!(part.parent(), target.parent());
    }

    #[test]
    fn an_oversized_download_is_aborted_and_the_part_removed() {
        let dir = scratch_dir("oversize");
        let target = dir.join("AxeNStax.AppImage");
        fs::write(&target, b"old build").unwrap();
        let r = signed_ref(&sha256_hex(b"x"), vec![]);
        // An "endless" body: 3 MiB against a 1 MiB cap.
        let res = install_from_candidates(&target, &r, |_, part| {
            stream_capped(std::io::repeat(7).take(3 * 1024 * 1024), part, 1024 * 1024, |_| {})
        });
        assert!(res.unwrap_err().contains("limit"));
        assert!(!dir.join(".axenstax-engine_0.3.0_x86_64.AppImage.part").exists());
        assert_eq!(fs::read(&target).unwrap(), b"old build");
        let _ = fs::remove_dir_all(&dir);
    }
}
