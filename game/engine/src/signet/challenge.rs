//! Per-client challenge nonce table — Phase 2 of the foundations spec
//! `docs/foundations/2026-04-20-engine-signet-auth.md`.
//!
//! On new transport connection, the server issues a 32-byte random nonce keyed
//! by transport identity (slot index, peer address — the caller decides the
//! key shape). The client signs that nonce inside the kind-21236 auth event it
//! presents in `JoinRequestPacket`. On JoinRequest the server `consume`s the
//! entry and verifies the signature against the same nonce.
//!
//! Bounded by `cap` (drop new entries above it) and `ttl` (entries expire and
//! are GC'd). Mirrors the pattern used in `tools/sites/game/auth.py:_sessions`
//! (MAX_SESSIONS=100). 30 s TTL gives the user enough time to scan a QR and
//! approve on the phone, while keeping the replay window tight.
//!
//! Phase 2 was unwired; Phase 3 issued the nonce; Phase 4 (v48) made it live —
//! the server issues a challenge on connect, an authenticated client signs it,
//! and `resolve_join_identity` consumes it (single-use) on the JoinRequest.

#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

/// Default capacity. Sized like the website's `MAX_SESSIONS` with headroom for
/// LAN multiplayer (multiple clients reconnecting after a server tick).
pub const DEFAULT_CAP: usize = 128;

/// Default TTL in seconds. Long enough to scan a QR + approve on phone; short
/// enough that a captured nonce expires before a replay attempt is plausible.
pub const DEFAULT_TTL_SECS: u64 = 30;

#[cfg(not(target_arch = "wasm32"))]
pub struct ChallengeTable {
    entries: HashMap<String, (Instant, [u8; 32])>,
    cap: usize,
    ttl: Duration,
}

#[cfg(not(target_arch = "wasm32"))]
impl ChallengeTable {
    pub fn new(cap: usize, ttl_secs: u64) -> Self {
        Self {
            entries: HashMap::with_capacity(cap),
            cap,
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Issue a fresh nonce for `client_key`.
    ///
    /// Returns `Some(nonce)` on success. Returns `None` when the table is at
    /// capacity AND `client_key` is not already present — i.e. only new keys
    /// are refused above cap; re-issuing for an existing key always succeeds
    /// (clients may legitimately reconnect inside the TTL window). Callers
    /// should log the `None` case as a 429-equivalent.
    pub fn issue(&mut self, client_key: String) -> Option<[u8; 32]> {
        let is_new = !self.entries.contains_key(&client_key);
        if is_new && self.entries.len() >= self.cap {
            // Opportunistic GC before refusing — frees expired slots.
            self.gc();
            if self.entries.len() >= self.cap {
                return None;
            }
        }
        let nonce = random_nonce();
        self.entries.insert(client_key, (Instant::now(), nonce));
        Some(nonce)
    }

    /// Consume the nonce for `client_key` if present and not expired.
    /// Removes the entry on hit — single-use. Returns `None` for missing,
    /// expired, or already-consumed.
    pub fn consume(&mut self, client_key: &str) -> Option<[u8; 32]> {
        let (issued_at, nonce) = self.entries.remove(client_key)?;
        if issued_at.elapsed() > self.ttl {
            return None;
        }
        Some(nonce)
    }

    /// Drop expired entries. Called opportunistically by `issue` on overflow;
    /// callers may also invoke periodically.
    pub fn gc(&mut self) {
        let ttl = self.ttl;
        self.entries
            .retain(|_, (issued_at, _)| issued_at.elapsed() <= ttl);
    }

    /// Number of live entries (post-GC count is via `gc` then `len`).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// No production caller — `hosted_server.rs` never checks emptiness
    /// before issuing/consuming; exercised by the tests below.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for ChallengeTable {
    fn default() -> Self {
        Self::new(DEFAULT_CAP, DEFAULT_TTL_SECS)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn random_nonce() -> [u8; 32] {
    let mut buf = [0u8; 32];
    // getrandom is the same RNG used elsewhere in the engine; failure here
    // would only happen if the OS RNG is unavailable, which is fatal anyway.
    getrandom::fill(&mut buf).expect("OS RNG unavailable");
    buf
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn issue_then_consume_round_trip() {
        let mut t = ChallengeTable::new(8, 30);
        let nonce = t.issue("client-a".into()).expect("first issue");
        assert_eq!(t.consume("client-a"), Some(nonce));
    }

    #[test]
    fn consume_missing_returns_none() {
        let mut t = ChallengeTable::new(8, 30);
        assert!(t.consume("not-issued").is_none());
    }

    #[test]
    fn consume_is_single_use() {
        let mut t = ChallengeTable::new(8, 30);
        let _ = t.issue("client-a".into()).unwrap();
        let _ = t.consume("client-a").unwrap();
        assert!(t.consume("client-a").is_none(), "second consume must fail");
    }

    #[test]
    fn nonces_differ_across_issues() {
        let mut t = ChallengeTable::new(8, 30);
        let a = t.issue("client-a".into()).unwrap();
        let b = t.issue("client-b".into()).unwrap();
        assert_ne!(a, b, "32 random bytes must differ between clients");
    }

    #[test]
    fn reissue_for_same_key_overwrites() {
        let mut t = ChallengeTable::new(8, 30);
        let n1 = t.issue("client-a".into()).unwrap();
        let n2 = t.issue("client-a".into()).unwrap();
        assert_ne!(n1, n2, "re-issue must mint a fresh nonce");
        assert_eq!(t.len(), 1, "re-issue must not grow the table");
        assert_eq!(t.consume("client-a"), Some(n2), "consume returns the latest");
    }

    #[test]
    fn expired_entry_consume_returns_none() {
        // 1-second TTL keeps the test fast.
        let mut t = ChallengeTable::new(8, 1);
        let _ = t.issue("client-a".into()).unwrap();
        thread::sleep(Duration::from_millis(1100));
        assert!(t.consume("client-a").is_none(), "expired entry must not be consumable");
    }

    #[test]
    fn gc_drops_expired_entries() {
        let mut t = ChallengeTable::new(8, 1);
        let _ = t.issue("a".into()).unwrap();
        let _ = t.issue("b".into()).unwrap();
        thread::sleep(Duration::from_millis(1100));
        // Fresh entry should survive.
        let _ = t.issue("c".into()).unwrap();
        t.gc();
        assert_eq!(t.len(), 1, "GC must keep only fresh entries");
        assert!(t.consume("a").is_none());
        assert!(t.consume("b").is_none());
        assert!(t.consume("c").is_some());
    }

    #[test]
    fn overflow_refuses_new_keys() {
        let mut t = ChallengeTable::new(2, 30);
        assert!(t.issue("a".into()).is_some());
        assert!(t.issue("b".into()).is_some());
        assert!(t.issue("c".into()).is_none(), "new key over cap must be refused");
        // The two existing keys remain consumable.
        assert!(t.consume("a").is_some());
        assert!(t.consume("b").is_some());
    }

    #[test]
    fn overflow_allows_reissue_for_existing_key() {
        let mut t = ChallengeTable::new(2, 30);
        let _ = t.issue("a".into()).unwrap();
        let _ = t.issue("b".into()).unwrap();
        // Same key — should re-issue, not refuse.
        let n = t.issue("a".into());
        assert!(n.is_some(), "re-issue for existing key must succeed even at cap");
    }

    #[test]
    fn overflow_recovers_after_gc() {
        // Cap=2, TTL=1s. Fill the table, let entries expire, issue a new key —
        // the opportunistic GC inside `issue` should free a slot.
        let mut t = ChallengeTable::new(2, 1);
        let _ = t.issue("a".into()).unwrap();
        let _ = t.issue("b".into()).unwrap();
        thread::sleep(Duration::from_millis(1100));
        let n = t.issue("c".into());
        assert!(n.is_some(), "issue must succeed after expired entries are GC'd");
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn nonce_is_32_random_bytes() {
        let mut t = ChallengeTable::new(8, 30);
        let n = t.issue("a".into()).unwrap();
        // All-zero would indicate a broken RNG — vanishingly unlikely otherwise.
        assert!(n.iter().any(|&b| b != 0), "nonce must not be all zeros");
    }
}
