//! Charter comms ceiling — the guardian-set ceiling on a persona's world chat.
//!
//! Spec: `docs/foundations/2026-09-05-world-chat.md` §2.5, §2.6, §3.3.
//!
//! One function is the only way the engine learns a level:
//!
//! ```ignore
//! pub fn comms_level(pubkey: &[u8; 32]) -> CommsLevel
//! ```
//!
//! **Checked 2026-09-05: Charter has no comms capability upstream**, even
//! though the word "comms" appears in Charter's README and a source comment,
//! which makes it look shipped. The published SDK type is
//! `ChartedClause { kind: 'schedule', … }` — a single string literal, not a
//! union — and the device-broker wire's fifteen clause kinds do not include
//! `comms`; it is a *reserved* row in the prose contract, alongside `spend`,
//! and the NIP-46 method `charter_set_comms` is likewise named-but-not-callable.
//!
//! So this function's body is the **BRIDGE** from day one: a local policy
//! file. The bridge is confined to this one function so the eventual cutover
//! to a real Charter relay-read is a body swap with no callers changed.
//!
//! **The file can only LOWER the ceiling, never raise it** (owner decision,
//! audit 2026-09-28 C2). An earlier version verified a signature against a
//! `guardian_npub` read from the same file, which proved nothing: anyone with
//! filesystem access (the child included) could name their own key and grant
//! themselves `Anyone`. So the file is now a tightening-only hint — any
//! signature or npub in it is ignored, and the result is clamped to at most
//! the safe default (`Approved`). Raising the ceiling waits for a real
//! capability boundary: a Charter comms clause or a Signet guardian
//! attestation.
//!
//! Native-only: the policy file lives in the native config dir and the only
//! caller (`hosted_server`'s join path) is native.
#![cfg(not(target_arch = "wasm32"))]

use crate::comms::CommsLevel;

/// The safe default, and the highest level the local file can ever produce.
const SAFE_DEFAULT: CommsLevel = CommsLevel::Approved;

/// One entry in the local policy file: a subject pubkey (64 hex) and the
/// comms level to restrict them to.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct PolicyEntry {
    subject: String,
    comms: String,
}

/// The on-disk shape of `~/.config/axenstax/guardian-policy.json`. Older files
/// also carry `guardian_npub` + `sig`; those are ignored (serde skips unknown
/// fields) because a self-named key is not an authority.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct GuardianPolicyFile {
    entries: Vec<PolicyEntry>,
}

/// `~/.config/axenstax/guardian-policy.json`, or `None` if `HOME` isn't set
/// (no policy is the same as "no guardian record" — §2.6 fails closed to
/// `Approved`, never panics).
fn policy_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    if home.is_empty() {
        return None;
    }
    Some(
        std::path::Path::new(&home)
            .join(".config")
            .join("axenstax")
            .join("guardian-policy.json"),
    )
}

/// The engine's only way to learn a Charter comms ceiling for `pubkey`.
///
/// Always at most `CommsLevel::Approved`. No file, no entry, or an unreadable
/// file return that default; an entry can only lower it (to `Blocked`).
// BRIDGE: local tightening-only policy file — replace when a real capability
// boundary ships (the Charter comms clause upstream, or a Signet guardian
// attestation); only that may ever RAISE the ceiling above Approved.
// Confined to this function; nothing else reads the file.
pub fn comms_level(pubkey: &[u8; 32]) -> CommsLevel {
    let Some(path) = policy_path() else {
        return SAFE_DEFAULT;
    };
    let Ok(json) = std::fs::read_to_string(&path) else {
        return SAFE_DEFAULT;
    };
    level_from_policy(&json, pubkey)
}

/// Parse `subject`'s entry out of the policy file's raw JSON and clamp it to
/// the safe default. Anything unreadable, or no entry, is the default. Split
/// out so it's unit-testable without a real config dir.
fn level_from_policy(json: &str, subject: &[u8; 32]) -> CommsLevel {
    let requested = serde_json::from_str::<GuardianPolicyFile>(json)
        .ok()
        .and_then(|file| {
            let subject_hex = hex::encode(subject);
            file.entries
                .iter()
                .find(|e| e.subject.trim().eq_ignore_ascii_case(&subject_hex))
                .and_then(|e| CommsLevel::parse(&e.comms))
        })
        .unwrap_or(SAFE_DEFAULT);
    requested.min(SAFE_DEFAULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(entries: &[(&[u8; 32], &str)]) -> String {
        let entries: Vec<_> = entries
            .iter()
            .map(|(pk, lvl)| serde_json::json!({"subject": hex::encode(pk), "comms": lvl}))
            .collect();
        serde_json::json!({ "entries": entries }).to_string()
    }

    const CHILD: [u8; 32] = [0x11u8; 32];

    #[test]
    fn a_file_that_tries_to_raise_the_ceiling_leaves_it_at_the_default() {
        assert_eq!(level_from_policy(&policy(&[(&CHILD, "anyone")]), &CHILD), CommsLevel::Approved);
    }

    #[test]
    fn a_self_signed_legacy_file_granting_anyone_is_not_a_grant() {
        // The old shape: a guardian_npub + sig the file itself vouches for.
        let json = serde_json::json!({
            "guardian_npub": "npub1selfnamed",
            "entries": [{"subject": hex::encode(CHILD), "comms": "anyone"}],
            "sig": "00".repeat(64),
        })
        .to_string();
        assert_eq!(level_from_policy(&json, &CHILD), CommsLevel::Approved);
    }

    #[test]
    fn a_file_that_lowers_the_ceiling_lowers_it() {
        assert_eq!(level_from_policy(&policy(&[(&CHILD, "blocked")]), &CHILD), CommsLevel::Blocked);
        assert_eq!(level_from_policy(&policy(&[(&CHILD, "approved")]), &CHILD), CommsLevel::Approved);
    }

    #[test]
    fn entry_for_a_different_subject_leaves_the_default() {
        let someone_else = [0x22u8; 32];
        assert_eq!(
            level_from_policy(&policy(&[(&CHILD, "blocked")]), &someone_else),
            CommsLevel::Approved
        );
    }

    #[test]
    fn malformed_or_unknown_values_fall_back_to_the_default_without_panicking() {
        for bad in ["{ not json", "", "null", r#"{"entries": 3}"#] {
            assert_eq!(level_from_policy(bad, &CHILD), CommsLevel::Approved, "{bad:?}");
        }
        assert_eq!(
            level_from_policy(&policy(&[(&CHILD, "everyone")]), &CHILD),
            CommsLevel::Approved
        );
    }

    #[test]
    fn no_input_can_ever_produce_anyone() {
        for lvl in ["anyone", "ANYONE", " anyone ", "approved", "blocked", "x"] {
            assert_ne!(level_from_policy(&policy(&[(&CHILD, lvl)]), &CHILD), CommsLevel::Anyone);
        }
    }

    // `comms_level` itself is not exercised here: it reads `$HOME`, and
    // mutating process-global env vars from a test would race every other
    // test in this binary's shared process. `level_from_policy` carries every
    // case that matters without touching the filesystem or the environment.
}
