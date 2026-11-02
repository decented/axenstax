//! The dedicated server's access policy, as it stands in its identity directory.
//!
//! Three sources, each loaded on its own, because a problem with one must not
//! disturb the others:
//!
//! | Source | Absent | Present |
//! |---|---|---|
//! | `whitelist.txt` + `AXENSTAX_WHITELIST` | [`Allow::Open`] | [`Allow::Only`] the union of the valid entries, however few |
//! | `blocklist.txt` | nobody blocked | the valid entries |
//! | `require_signin` | the boot default | exactly `true` or `false`, after trimming, any case |
//!
//! An entry that isn't a key is skipped with a warning and never widens access.
//! A file that can't be read (anything but not existing), or a `require_signin`
//! that says something else, is a problem and is never guessed at: at boot
//! [`PolicyFiles::load`] refuses, and on a reload [`PolicyFiles::reload`] keeps
//! that source's last good value and warns once per distinct problem.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::access_policy::Allow;

const ALLOWLIST_FILE: &str = "whitelist.txt";
const BLOCKLIST_FILE: &str = "blocklist.txt";
const REQUIRE_SIGNIN_FILE: &str = "require_signin";

/// What boot says when the allowlist is on with nobody on it. An operator who
/// relied on an empty file meaning "no allowlist" would otherwise see only refusals.
pub const EMPTY_ALLOWLIST_WARNING: &str = "the allowlist is on and empty, so nobody can join: \
     add an npub, or turn it off by deleting whitelist.txt (and unsetting AXENSTAX_WHITELIST)";

/// The advisory lock file in the identity directory. The engine's admin commands
/// and the console both read, change and rewrite these files; each holds an
/// exclusive `flock` on this file for the whole of that, so neither loses the
/// other's change. (`tools/sites/console/identity.py` takes the same lock.)
pub const LOCK_FILE: &str = ".policy.lock";

/// An exclusive hold on the identity directory's policy files, released on drop.
#[derive(Debug)]
pub struct PolicyLock(#[allow(dead_code)] std::fs::File);

/// Wait for, and take, the policy lock in `dir` (created if need be). An error is
/// a refusal to change the policy: nothing is written without the lock.
pub fn lock(dir: &Path) -> Result<PolicyLock, String> {
    let path = dir.join(LOCK_FILE);
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // Whoever creates it, the other process (another user, in another
        // container) must be able to open it.
        options.mode(0o666);
    }
    let file = match options.open(&path) {
        Ok(file) => file,
        // Made by the other side under a mode we can't write: a read-only handle
        // locks just as well.
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => std::fs::File::open(&path)
            .map_err(|e| format!("{}: can't be opened for locking: {e}", path.display()))?,
        Err(e) => return Err(format!("{}: can't be opened for locking: {e}", path.display())),
    };
    file.lock().map_err(|e| format!("{}: can't be locked: {e}", path.display()))?;
    Ok(PolicyLock(file))
}

/// The policy to hand `HostedServer::set_access_policy`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessPolicy {
    pub require_signin: bool,
    pub allow: Allow,
    pub blocklist: Vec<[u8; 32]>,
}

/// The sources, and the last good value of each.
pub struct PolicyFiles {
    dir: PathBuf,
    /// Whether sign-in is required when there is no `require_signin` file.
    signin_default: bool,
    env_allow: Option<String>,
    current: AccessPolicy,
    /// The problems in force at the last look, already warned about.
    warned: BTreeSet<String>,
}

impl PolicyFiles {
    /// The boot load. `Err` names the file and the problem: the server must not
    /// start on a policy it could not read.
    pub fn load(dir: &Path, signin_default: bool, env_allow: Option<String>) -> Result<Self, String> {
        let mut warnings = Vec::new();
        let current = AccessPolicy {
            require_signin: read_require_signin(dir, signin_default)?,
            allow: read_allow(dir, env_allow.as_deref(), &mut warnings)?,
            blocklist: read_blocklist(dir, &mut warnings)?,
        };
        let mut files = Self {
            dir: dir.to_path_buf(),
            signin_default,
            env_allow,
            current,
            warned: BTreeSet::new(),
        };
        files.warn_new(warnings);
        Ok(files)
    }

    /// Look at the sources again. Returns the problems warned about for the
    /// first time by this look.
    pub fn reload(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut keep = |problem: String| warnings.push(format!("{problem}; keeping the last good value"));
        let require_signin = read_require_signin(&self.dir, self.signin_default).map_err(&mut keep);
        let mut skipped = Vec::new();
        let allow = read_allow(&self.dir, self.env_allow.as_deref(), &mut skipped).map_err(&mut keep);
        let blocklist = read_blocklist(&self.dir, &mut skipped).map_err(&mut keep);
        if let Ok(v) = require_signin {
            self.current.require_signin = v;
        }
        if let Ok(v) = allow {
            self.current.allow = v;
        }
        if let Ok(v) = blocklist {
            self.current.blocklist = v;
        }
        warnings.append(&mut skipped);
        self.warn_new(warnings)
    }

    pub fn policy(&self) -> &AccessPolicy {
        &self.current
    }

    /// Warn about each problem that wasn't already in force, once.
    fn warn_new(&mut self, mut problems: Vec<String>) -> Vec<String> {
        if self.current.allow == Allow::only([]) {
            problems.push(EMPTY_ALLOWLIST_WARNING.to_string());
        }
        let now: BTreeSet<String> = problems.into_iter().collect();
        let fresh: Vec<String> = now.difference(&self.warned).cloned().collect();
        for problem in &fresh {
            log::warn!("access policy: {problem}");
        }
        self.warned = now;
        fresh
    }
}

/// A policy file's contents. `None` only when it doesn't exist.
fn read_source(dir: &Path, name: &str) -> Result<Option<String>, String> {
    let path = dir.join(name);
    match std::fs::read_to_string(&path) {
        Ok(contents) => Ok(Some(contents)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: can't be read: {e}", path.display())),
    }
}

fn read_require_signin(dir: &Path, default: bool) -> Result<bool, String> {
    let Some(contents) = read_source(dir, REQUIRE_SIGNIN_FILE)? else {
        return Ok(default);
    };
    match contents.trim().to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!(
            "{}: must say exactly true or false",
            dir.join(REQUIRE_SIGNIN_FILE).display()
        )),
    }
}

fn read_allow(dir: &Path, env: Option<&str>, warnings: &mut Vec<String>) -> Result<Allow, String> {
    let env = env.filter(|v| !v.trim().is_empty());
    let file = read_source(dir, ALLOWLIST_FILE)?;
    if env.is_none() && file.is_none() {
        return Ok(Allow::Open);
    }
    let mut keys = Vec::new();
    if let Some(v) = env {
        keys.extend(parse_keys(v.split(','), "AXENSTAX_WHITELIST entry", warnings));
    }
    if let Some(contents) = &file {
        keys.extend(parse_keys(contents.lines(), "whitelist.txt line", warnings));
    }
    Ok(Allow::only(keys))
}

fn read_blocklist(dir: &Path, warnings: &mut Vec<String>) -> Result<Vec<[u8; 32]>, String> {
    let contents = read_source(dir, BLOCKLIST_FILE)?.unwrap_or_default();
    Ok(parse_keys(contents.lines(), "blocklist.txt line", warnings))
}

/// npub (bech32) or hex entries as x-only pubkey bytes, skipping blanks and `#`
/// comments. An entry that is neither is skipped with a warning naming its
/// position, not its contents.
fn parse_keys<'a>(
    entries: impl Iterator<Item = &'a str>,
    label: &str,
    warnings: &mut Vec<String>,
) -> Vec<[u8; 32]> {
    let mut keys = Vec::new();
    for (i, entry) in entries.enumerate() {
        let entry = entry.trim();
        if entry.is_empty() || entry.starts_with('#') {
            continue;
        }
        match nostr::PublicKey::parse(entry) {
            Ok(pk) => keys.push(pk.to_bytes()),
            Err(_) => warnings.push(format!("{label} {} is not a valid npub or hex key; skipped", i + 1)),
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Keys, ToBech32};

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("axe_policy_{tag}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, name: &str, contents: &str) {
            std::fs::write(self.0.join(name), contents).unwrap();
        }

        fn load(&self) -> Result<PolicyFiles, String> {
            PolicyFiles::load(&self.0, true, None)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn key() -> ([u8; 32], String) {
        let pk = Keys::generate().public_key();
        (pk.to_bytes(), pk.to_bech32().unwrap())
    }

    /// Unreadable but present, or `false` when running as root, which reads anything.
    #[cfg(unix)]
    fn unreadable(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
        std::fs::read(path).is_err()
    }

    #[test]
    fn no_sources_is_open_with_the_boot_default() {
        let dir = Dir::new("absent");
        let policy = dir.load().unwrap();
        assert_eq!(policy.policy().allow, Allow::Open);
        assert!(policy.policy().require_signin);
        assert!(policy.policy().blocklist.is_empty());
        assert!(!PolicyFiles::load(&dir.0, false, None).unwrap().policy().require_signin);
    }

    #[test]
    fn an_empty_file_is_an_allowlist_of_nobody() {
        let dir = Dir::new("empty");
        dir.write(ALLOWLIST_FILE, "");
        assert_eq!(dir.load().unwrap().policy().allow, Allow::only([]));
    }

    #[test]
    fn garbage_lines_are_skipped_and_named_by_position_only() {
        let dir = Dir::new("garbage");
        let (pk, npub) = key();
        dir.write(ALLOWLIST_FILE, &format!("# my friends\n\nsecret-looking-garbage\n{npub}\n"));
        let mut files = dir.load().unwrap();
        assert_eq!(files.policy().allow, Allow::only([pk]));
        dir.write(ALLOWLIST_FILE, "secret-looking-garbage\n");
        let warned = files.reload();
        assert_eq!(files.policy().allow, Allow::only([]), "garbage alone admits nobody");
        assert!(warned.iter().any(|w| w.contains("whitelist.txt line 1")), "{warned:?}");
        assert!(warned.iter().all(|w| !w.contains("secret-looking")), "{warned:?}");
        assert!(warned.iter().any(|w| w == EMPTY_ALLOWLIST_WARNING), "{warned:?}");
    }

    #[test]
    fn the_environment_and_the_file_are_a_union_and_a_non_blank_value_turns_it_on() {
        let dir = Dir::new("env");
        let (a, a_npub) = key();
        let (b, b_npub) = key();
        dir.write(ALLOWLIST_FILE, &format!("{b_npub}\n"));
        let both = PolicyFiles::load(&dir.0, true, Some(a_npub.clone())).unwrap();
        assert_eq!(both.policy().allow, Allow::only([a, b]));

        let dir = Dir::new("env-only");
        let env = PolicyFiles::load(&dir.0, true, Some(a_npub)).unwrap();
        assert_eq!(env.policy().allow, Allow::only([a]));
        let junk = PolicyFiles::load(&dir.0, true, Some("not-a-key".into())).unwrap();
        assert_eq!(junk.policy().allow, Allow::only([]), "non-blank counts as present");
        let blank = PolicyFiles::load(&dir.0, true, Some("  ".into())).unwrap();
        assert_eq!(blank.policy().allow, Allow::Open, "blank does not");
    }

    #[test]
    fn deleting_the_file_turns_the_allowlist_off() {
        let dir = Dir::new("off");
        dir.write(ALLOWLIST_FILE, "");
        let mut files = dir.load().unwrap();
        std::fs::remove_file(dir.0.join(ALLOWLIST_FILE)).unwrap();
        files.reload();
        assert_eq!(files.policy().allow, Allow::Open);
    }

    #[test]
    fn require_signin_is_exactly_true_or_false() {
        let dir = Dir::new("signin");
        for (contents, want) in [("true", true), (" TRUE\n", true), ("false\n", false), ("False", false)] {
            dir.write(REQUIRE_SIGNIN_FILE, contents);
            assert_eq!(PolicyFiles::load(&dir.0, !want, None).unwrap().policy().require_signin, want, "{contents:?}");
        }
        for contents in ["", "\n", "yes", "0", "true false"] {
            dir.write(REQUIRE_SIGNIN_FILE, contents);
            let err = dir.load().err().unwrap_or_else(|| panic!("{contents:?} must refuse boot"));
            assert!(err.contains(REQUIRE_SIGNIN_FILE), "{err}");
        }
    }

    #[test]
    fn invalid_require_signin_on_reload_keeps_the_last_good_value_and_warns_once() {
        let dir = Dir::new("signin-reload");
        dir.write(REQUIRE_SIGNIN_FILE, "true");
        let mut files = PolicyFiles::load(&dir.0, false, None).unwrap();

        dir.write(REQUIRE_SIGNIN_FILE, "");
        let warned = files.reload();
        assert!(files.policy().require_signin, "the last good value stands");
        assert_eq!(warned.len(), 1, "{warned:?}");
        assert!(warned[0].contains(REQUIRE_SIGNIN_FILE) && warned[0].contains("last good"), "{warned:?}");
        assert!(files.reload().is_empty(), "not every 5 seconds");

        dir.write(REQUIRE_SIGNIN_FILE, "false");
        assert!(files.reload().is_empty());
        assert!(!files.policy().require_signin, "a good value applies again");

        dir.write(REQUIRE_SIGNIN_FILE, "maybe");
        assert_eq!(files.reload().len(), 1, "a problem that comes back is warned about again");
        assert!(!files.policy().require_signin);
    }

    #[cfg(unix)]
    #[test]
    fn a_read_failure_refuses_boot_and_keeps_the_last_good_list_on_reload() {
        for name in [ALLOWLIST_FILE, BLOCKLIST_FILE, REQUIRE_SIGNIN_FILE] {
            let dir = Dir::new(&format!("unreadable-{name}"));
            let (a, a_npub) = key();
            dir.write(ALLOWLIST_FILE, &format!("{a_npub}\n"));
            dir.write(BLOCKLIST_FILE, &format!("{a_npub}\n"));
            dir.write(REQUIRE_SIGNIN_FILE, "true");
            let mut files = dir.load().unwrap();

            if !unreadable(&dir.0.join(name)) {
                return;
            }
            let err = dir.load().err().expect("boot refuses");
            assert!(err.contains(name), "{err}");

            let warned = files.reload();
            assert_eq!(warned.len(), 1, "{warned:?}");
            assert!(warned[0].contains(name), "{warned:?}");
            assert!(files.reload().is_empty(), "warned once");
            assert_eq!(
                files.policy(),
                &AccessPolicy { require_signin: true, allow: Allow::only([a]), blocklist: vec![a] },
                "nothing opens and nobody is unblocked"
            );
        }
    }

    #[test]
    fn each_source_moves_on_its_own() {
        let dir = Dir::new("independent");
        let (a, a_npub) = key();
        dir.write(REQUIRE_SIGNIN_FILE, "true");
        let mut files = dir.load().unwrap();
        dir.write(REQUIRE_SIGNIN_FILE, "garbage");
        dir.write(BLOCKLIST_FILE, &format!("{a_npub}\n"));
        files.reload();
        assert!(files.policy().require_signin);
        assert_eq!(files.policy().blocklist, vec![a], "a bad sign-in file doesn't hold up a block");
    }
}
