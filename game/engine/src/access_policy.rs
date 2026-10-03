// Used by the native join path (hosted_server); unused on the wasm build.
//! Deterministic server access precedence (Spec B §5).
//!
//! One pure decision used by the join path: a **blocked** npub is refused first
//! (block always wins), then a non-empty **allowlist** gates, then a **sign-in**
//! requirement gates guests. Capacity admission is a separate seam
//! ([`crate::admission`]); this is identity-based access only.

/// Why a join was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessReject {
    /// The npub is on the operator's blocklist.
    Blocked,
    /// A non-empty allowlist is set and this identity isn't on it (or is a guest).
    NotAllowlisted,
    /// The server requires a verified sign-in and this is a guest.
    SignInRequired,
}

/// Decide whether a (possibly anonymous) identity may join — precedence
/// **blocklist → allowlist → sign-in**. `pubkey` is `None` for a guest.
pub fn decide_access(
    pubkey: Option<[u8; 32]>,
    blocklist: &[[u8; 32]],
    whitelist: &[[u8; 32]],
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
            if require_signin || !whitelist.is_empty() {
                return Err(AccessReject::SignInRequired);
            }
        }
        // A verified identity is gated by a non-empty allowlist.
        Some(pk) => {
            if !whitelist.is_empty() && !whitelist.contains(&pk) {
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
        assert!(decide_access(None, &[], &[], false).is_ok());
        assert!(decide_access(Some(A), &[], &[], false).is_ok());
    }

    #[test]
    fn blocked_wins_even_if_whitelisted() {
        // A is both blocked AND allowlisted → Blocked still wins.
        assert_eq!(
            decide_access(Some(A), &[A], &[A], false),
            Err(AccessReject::Blocked)
        );
    }

    #[test]
    fn allowlist_gates_non_listed_and_guests() {
        // A signed-in npub not on the list → NotAllowlisted.
        assert_eq!(
            decide_access(Some(B), &[], &[A], false),
            Err(AccessReject::NotAllowlisted)
        );
        // A guest on an allowlist server is asked to sign in (can't be allowlisted).
        assert_eq!(
            decide_access(None, &[], &[A], false),
            Err(AccessReject::SignInRequired)
        );
        assert!(decide_access(Some(A), &[], &[A], false).is_ok());
    }

    #[test]
    fn signin_required_rejects_guest_only() {
        assert_eq!(
            decide_access(None, &[], &[], true),
            Err(AccessReject::SignInRequired)
        );
        assert!(decide_access(Some(A), &[], &[], true).is_ok());
    }

    #[test]
    fn reject_reason_strings() {
        assert!(reject_reason(AccessReject::Blocked).contains("blocked"));
        assert!(reject_reason(AccessReject::NotAllowlisted).contains("allowlist"));
        assert!(reject_reason(AccessReject::SignInRequired).contains("sign-in"));
    }
}
