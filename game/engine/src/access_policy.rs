// Used by the native join path (hosted_server); unused on the wasm build.
//! Deterministic server access precedence (Spec B §5).
//!
//! One pure decision used by the join path and by the re-check that runs when
//! the policy changes: a **blocked** npub is refused first (block always wins),
//! then an **allowlist**, if the server runs one, gates, then a **sign-in**
//! requirement gates guests. Capacity admission is a separate seam
//! ([`crate::admission`]); this is identity-based access only.

use std::collections::BTreeSet;

/// Whether a server runs an allowlist. A mode, not a possibly-empty list: an
/// allowlist with nobody on it admits nobody, it does not open the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Allow {
    /// No allowlist: any identity the sign-in rule admits.
    Open,
    /// Only these verified identities. Guests are refused.
    Only(BTreeSet<[u8; 32]>),
}

impl Allow {
    pub fn only(keys: impl IntoIterator<Item = [u8; 32]>) -> Self {
        Allow::Only(keys.into_iter().collect())
    }

    /// Is this identity one the allowlist lets through?
    pub fn admits(&self, pubkey: &[u8; 32]) -> bool {
        match self {
            Allow::Open => true,
            Allow::Only(keys) => keys.contains(pubkey),
        }
    }

    /// How many identities are listed (0 for [`Allow::Open`] too).
    pub fn listed(&self) -> usize {
        match self {
            Allow::Open => 0,
            Allow::Only(keys) => keys.len(),
        }
    }
}

/// Why a join was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessReject {
    /// The npub is on the operator's blocklist.
    Blocked,
    /// An allowlist is on and this identity isn't on it.
    NotAllowlisted,
    /// The server requires a verified sign-in and this is a guest.
    SignInRequired,
}

/// Decide whether a (possibly anonymous) identity may join — precedence
/// **blocklist → allowlist → sign-in**. `pubkey` is `None` for a guest.
pub fn decide_access(
    pubkey: Option<[u8; 32]>,
    blocklist: &[[u8; 32]],
    allow: &Allow,
    require_signin: bool,
) -> Result<(), AccessReject> {
    // 1. Block always wins (even for an allowlisted npub).
    if pubkey.is_some_and(|pk| blocklist.contains(&pk)) {
        return Err(AccessReject::Blocked);
    }
    match pubkey {
        // A guest must sign in when the server requires it OR runs an allowlist —
        // an anonymous guest can't be allowlisted, so the actionable ask is sign-in.
        None => {
            if require_signin || matches!(allow, Allow::Only(_)) {
                return Err(AccessReject::SignInRequired);
            }
        }
        // A verified identity is gated by the allowlist, if there is one.
        Some(pk) => {
            if !allow.admits(&pk) {
                return Err(AccessReject::NotAllowlisted);
            }
        }
    }
    Ok(())
}

/// The human-readable reject reason sent verbatim to the client.
pub fn reject_reason(r: AccessReject) -> String {
    match r {
        AccessReject::Blocked => "You are blocked from this server.".to_string(),
        AccessReject::NotAllowlisted => {
            "Your npub is not on this server's allowlist.".to_string()
        }
        AccessReject::SignInRequired => {
            "This server requires sign-in (verified Signet identity).".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: [u8; 32] = [1u8; 32];
    const B: [u8; 32] = [2u8; 32];

    #[test]
    fn open_server_admits_guest_and_signed() {
        assert!(decide_access(None, &[], &Allow::Open, false).is_ok());
        assert!(decide_access(Some(A), &[], &Allow::Open, false).is_ok());
    }

    #[test]
    fn blocked_wins_even_if_whitelisted() {
        // A is both blocked AND allowlisted → Blocked still wins.
        assert_eq!(
            decide_access(Some(A), &[A], &Allow::only([A]), false),
            Err(AccessReject::Blocked)
        );
        assert_eq!(decide_access(Some(A), &[A], &Allow::Open, false), Err(AccessReject::Blocked));
    }

    #[test]
    fn allowlist_gates_non_listed_and_guests() {
        // A signed-in npub not on the list → NotAllowlisted.
        assert_eq!(
            decide_access(Some(B), &[], &Allow::only([A]), false),
            Err(AccessReject::NotAllowlisted)
        );
        // A guest on an allowlist server is asked to sign in (can't be allowlisted).
        assert_eq!(
            decide_access(None, &[], &Allow::only([A]), false),
            Err(AccessReject::SignInRequired)
        );
        assert!(decide_access(Some(A), &[], &Allow::only([A]), false).is_ok());
    }

    #[test]
    fn an_empty_allowlist_admits_nobody() {
        let nobody = Allow::only([]);
        assert_eq!(decide_access(Some(A), &[], &nobody, true), Err(AccessReject::NotAllowlisted));
        assert_eq!(decide_access(Some(A), &[], &nobody, false), Err(AccessReject::NotAllowlisted));
        assert_eq!(decide_access(None, &[], &nobody, false), Err(AccessReject::SignInRequired));
    }

    #[test]
    fn signin_required_rejects_guest_only() {
        assert_eq!(
            decide_access(None, &[], &Allow::Open, true),
            Err(AccessReject::SignInRequired)
        );
        assert!(decide_access(Some(A), &[], &Allow::Open, true).is_ok());
    }

    #[test]
    fn reject_reason_strings() {
        assert!(reject_reason(AccessReject::Blocked).contains("blocked"));
        assert!(reject_reason(AccessReject::NotAllowlisted).contains("allowlist"));
        assert!(reject_reason(AccessReject::SignInRequired).contains("sign-in"));
    }
}
