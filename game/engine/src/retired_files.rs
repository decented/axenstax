//! One-off start-up clean-up of files the retired in-game feedback feature left
//! in the profile directory, so queued reports and burner keys do not linger on
//! players' machines. Local file deletes only: no network, and a failure is
//! logged and never stops the game.
#![cfg(not(target_arch = "wasm32"))]

use std::path::Path;

// BRIDGE: remove when no supported install can still hold these files — replace with nothing, trigger: two releases after this one
/// What the retired feature wrote: the report outbox, the ticket list, the
/// burner/device key and the reply inbox.
const RETIRED_MAILBOX_FILES: [&str; 4] = [
    "mailbox_outbox.json",
    "mailbox_tickets.json",
    "mailbox_key.json",
    "mailbox_inbox.json",
];

/// Delete every retired mailbox file found in `profile_dir`. Absent files are
/// the normal case and are silent; any other failure is logged and skipped.
pub fn remove_retired_mailbox_files(profile_dir: &Path) {
    for name in RETIRED_MAILBOX_FILES {
        match std::fs::remove_file(profile_dir.join(name)) {
            Ok(()) => log::info!("removed retired file {name}"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("could not remove retired file {name}: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("axenstax-retired-files-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn removes_all_four_retired_files_and_leaves_everything_else() {
        let dir = temp_dir("all");
        for name in RETIRED_MAILBOX_FILES {
            std::fs::write(dir.join(name), b"{}").unwrap();
        }
        std::fs::write(dir.join("settings.json"), b"{}").unwrap();

        remove_retired_mailbox_files(&dir);

        for name in RETIRED_MAILBOX_FILES {
            assert!(!dir.join(name).exists(), "{name} should be deleted");
        }
        assert!(dir.join("settings.json").exists(), "unrelated files are untouched");
        // A second launch finds nothing and is still fine.
        remove_retired_mailbox_files(&dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_files_and_a_missing_directory_are_not_errors() {
        let dir = temp_dir("absent");
        remove_retired_mailbox_files(&dir);
        remove_retired_mailbox_files(&dir.join("does-not-exist"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_cannot_be_deleted_does_not_stop_the_rest() {
        let dir = temp_dir("blocked");
        // A directory with the file's name makes remove_file fail (not NotFound).
        std::fs::create_dir(dir.join("mailbox_outbox.json")).unwrap();
        std::fs::write(dir.join("mailbox_key.json"), b"{}").unwrap();

        remove_retired_mailbox_files(&dir);

        assert!(dir.join("mailbox_outbox.json").is_dir(), "left in place, logged");
        assert!(!dir.join("mailbox_key.json").exists(), "later files still deleted");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
