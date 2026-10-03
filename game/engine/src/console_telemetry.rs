#![cfg(not(target_arch = "wasm32"))]
#![allow(dead_code)] // consumed by the snapshot (task 5) + capture wiring (integration)
//! Operator-private session telemetry (Spec B §6).
//!
//! The minimum useful: per-connection sessions (npub + connect/disconnect times)
//! plus a few aggregates. **No IP, geolocation, or behavioural data.** It stays on
//! the box — never relayed (GDPR-safe; player presence is private to the operator).
//! Retention is governed by Spec C (the cutoff handed to `purge_older_than`).
//! Persisted as append-friendly JSONL.

use serde::{Deserialize, Serialize};

use crate::privacy::PrivacyLevel;

/// One player session: a connect, and (once they leave) a disconnect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub npub: String,
    pub connect_unix: u64,
    #[serde(default)]
    pub disconnect_unix: Option<u64>,
}

/// Derived counts for the console (no per-player data).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TelemetryAggregates {
    pub unique_today: u32,
    pub peak_today: u16,
    pub total_sessions: u64,
}

/// The operator-private session log.
#[derive(Clone, Debug, Default)]
pub struct SessionLog {
    pub sessions: Vec<Session>,
}

impl SessionLog {
    /// Record a new connection (an open session).
    pub fn record_connect(&mut self, npub: &str, now_unix: u64) {
        self.sessions.push(Session {
            npub: npub.to_string(),
            connect_unix: now_unix,
            disconnect_unix: None,
        });
    }

    /// Close the most recent OPEN session for this npub (no-op if none open).
    pub fn record_disconnect(&mut self, npub: &str, now_unix: u64) {
        if let Some(s) = self
            .sessions
            .iter_mut()
            .rev()
            .find(|s| s.npub == npub && s.disconnect_unix.is_none())
        {
            s.disconnect_unix = Some(now_unix);
        }
    }

    /// Aggregates: unique npubs connected since `day_start_unix`, peak concurrency
    /// today, and total sessions ever. Open sessions are treated as lasting until
    /// `now_unix`.
    pub fn aggregates(&self, now_unix: u64, day_start_unix: u64) -> TelemetryAggregates {
        let mut seen = std::collections::HashSet::new();
        for s in &self.sessions {
            if s.connect_unix >= day_start_unix {
                seen.insert(s.npub.as_str());
            }
        }
        TelemetryAggregates {
            unique_today: seen.len() as u32,
            peak_today: self.peak_concurrency(day_start_unix, now_unix),
            total_sessions: self.sessions.len() as u64,
        }
    }

    /// Max simultaneous sessions overlapping the window `[from, to]`, via a sweep
    /// over connect (+1) / disconnect (-1) events. Open sessions end at `to`.
    fn peak_concurrency(&self, from: u64, to: u64) -> u16 {
        let mut events: Vec<(u64, i32)> = Vec::new();
        for s in &self.sessions {
            let end = s.disconnect_unix.unwrap_or(to);
            if end < from || s.connect_unix > to {
                continue; // no overlap with the window
            }
            events.push((s.connect_unix, 1));
            events.push((end, -1));
        }
        // +1 before -1 at the same timestamp ⇒ touching intervals count as
        // overlapping (the inclusive "peak" reading).
        events.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        let mut cur = 0i32;
        let mut peak = 0i32;
        for (_, d) in events {
            cur += d;
            peak = peak.max(cur);
        }
        peak.max(0) as u16
    }

    /// Drop sessions that connected before `cutoff_unix` (retention purge).
    pub fn purge_older_than(&mut self, cutoff_unix: u64) {
        self.sessions.retain(|s| s.connect_unix >= cutoff_unix);
    }

    /// Record a connect only if the privacy level persists history (Spec C §4) —
    /// at `None`, nothing is written (the live roster stays in-memory elsewhere).
    pub fn record_connect_gated(&mut self, level: &PrivacyLevel, npub: &str, now_unix: u64) {
        if crate::privacy::should_persist(level) {
            self.record_connect(npub, now_unix);
        }
    }

    /// Apply the level's retention: purge sessions older than its cutoff (Spec C
    /// §6). A no-op at `None` (nothing is retained to purge).
    pub fn apply_retention(&mut self, level: &PrivacyLevel, now_unix: u64) {
        if let Some(cutoff) = crate::privacy::retention_cutoff(level, now_unix) {
            self.purge_older_than(cutoff);
        }
    }

    /// Erase all of one player's sessions, by npub (right to erasure, Spec C §6).
    pub fn forget_player(&mut self, npub: &str) {
        self.sessions.retain(|s| s.npub != npub);
    }

    /// Erase the whole session log.
    pub fn clear(&mut self) {
        self.sessions.clear();
    }

    /// Persist the whole log as JSONL to `<dir>/sessions.jsonl`.
    pub fn save(&self, dir: &std::path::Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let mut out = String::new();
        for s in &self.sessions {
            out.push_str(&serde_json::to_string(s).map_err(|e| e.to_string())?);
            out.push('\n');
        }
        std::fs::write(dir.join("sessions.jsonl"), out).map_err(|e| e.to_string())
    }

    /// Load a log from `<dir>/sessions.jsonl` (empty if absent; unparseable lines
    /// skipped).
    pub fn load(dir: &std::path::Path) -> Self {
        let mut log = SessionLog::default();
        if let Ok(contents) = std::fs::read_to_string(dir.join("sessions.jsonl")) {
            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(s) = serde_json::from_str::<Session>(line) {
                    log.sessions.push(s);
                }
            }
        }
        log
    }
}

/// bech32 npub for a verified pubkey (the `npub-only display rule`); hex fallback.
fn npub_of(pubkey: &[u8; 32]) -> String {
    use nostr::ToBech32;
    nostr::PublicKey::from_slice(pubkey)
        .ok()
        .and_then(|pk| pk.to_bech32().ok())
        .unwrap_or_else(|| hex::encode(pubkey))
}

/// Record a player connect into `log`, gated by privacy level — the capture the
/// server calls when a player completes the join handshake. A guest (`None`
/// pubkey) has no identity to record; a `None` privacy level (the default
/// no-tracking posture) records nothing.
pub fn capture_connect(
    log: &mut SessionLog,
    level: &crate::privacy::PrivacyLevel,
    pubkey: Option<[u8; 32]>,
    now_unix: u64,
) {
    if let Some(pk) = pubkey {
        log.record_connect_gated(level, &npub_of(&pk), now_unix);
    }
}

/// Close a player's open session on disconnect (no-op for a guest / unverified).
pub fn capture_disconnect(log: &mut SessionLog, pubkey: Option<[u8; 32]>, now_unix: u64) {
    if let Some(pk) = pubkey {
        log.record_disconnect(&npub_of(&pk), now_unix);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::PrivacyLevel;

    #[test]
    fn capture_connect_gated_by_identity_and_privacy() {
        let pk = [7u8; 32];
        let on = PrivacyLevel::Sessions { retention_days: 30 };

        let mut log = SessionLog::default();
        capture_connect(&mut log, &on, Some(pk), 1000);
        assert_eq!(log.sessions.len(), 1, "verified player + tracking on → recorded");

        let mut off = SessionLog::default();
        capture_connect(&mut off, &PrivacyLevel::None, Some(pk), 1000);
        assert!(off.sessions.is_empty(), "no-tracking default records nothing");

        let mut guest = SessionLog::default();
        capture_connect(&mut guest, &on, None, 1000);
        assert!(guest.sessions.is_empty(), "guest (no verified identity) not recorded");
    }

    #[test]
    fn capture_disconnect_closes_the_open_session() {
        let pk = [7u8; 32];
        let on = PrivacyLevel::Sessions { retention_days: 30 };
        let mut log = SessionLog::default();
        capture_connect(&mut log, &on, Some(pk), 1000);
        capture_disconnect(&mut log, Some(pk), 1500);
        assert_eq!(log.sessions.len(), 1, "disconnect doesn't add a session");
        assert_eq!(log.sessions[0].disconnect_unix, Some(1500), "open session closed");
    }

    #[test]
    fn connect_appends_and_disconnect_closes_latest_open() {
        let mut log = SessionLog::default();
        log.record_connect("a", 10);
        assert_eq!(log.sessions.len(), 1);
        assert_eq!(log.sessions[0].disconnect_unix, None);
        log.record_connect("a", 20); // a reconnects (second open session)
        log.record_disconnect("a", 30); // closes the LATEST open (t=20)
        assert_eq!(log.sessions[1].disconnect_unix, Some(30));
        assert_eq!(log.sessions[0].disconnect_unix, None, "first session still open");
    }

    #[test]
    fn aggregates_unique_peak_total() {
        let mut log = SessionLog::default();
        log.record_connect("npubA", 10);
        log.record_connect("npubB", 15);
        log.record_disconnect("npubA", 20); // A:[10,20], B:[15,open]
        log.record_connect("npubC", 30);
        log.record_disconnect("npubC", 40);
        let agg = log.aggregates(50, 0);
        assert_eq!(agg.total_sessions, 3);
        assert_eq!(agg.unique_today, 3);
        assert_eq!(agg.peak_today, 2, "A & B overlap at [15,20]");
    }

    #[test]
    fn unique_counts_distinct_npubs_since_day_start() {
        let mut log = SessionLog::default();
        log.record_connect("a", 5); // before day_start
        log.record_connect("a", 100); // today
        log.record_connect("b", 110); // today
        let agg = log.aggregates(200, 50);
        assert_eq!(agg.unique_today, 2, "distinct npubs since day_start");
        assert_eq!(agg.total_sessions, 3);
    }

    #[test]
    fn purge_drops_old_sessions() {
        let mut log = SessionLog::default();
        log.record_connect("a", 10);
        log.record_connect("b", 100);
        log.purge_older_than(50);
        assert_eq!(log.sessions.len(), 1);
        assert_eq!(log.sessions[0].npub, "b");
    }

    #[test]
    fn jsonl_round_trips() {
        let dir = std::env::temp_dir().join(format!("axe_telem_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut log = SessionLog::default();
        log.record_connect("a", 10);
        log.record_disconnect("a", 20);
        log.record_connect("b", 30);
        log.save(&dir).unwrap();
        let loaded = SessionLog::load(&dir);
        assert_eq!(loaded.sessions.len(), 2);
        assert_eq!(loaded.sessions[0].npub, "a");
        assert_eq!(loaded.sessions[0].disconnect_unix, Some(20));
        assert_eq!(loaded.sessions[1].disconnect_unix, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn record_connect_gated_respects_level() {
        let mut log = SessionLog::default();
        log.record_connect_gated(&PrivacyLevel::None, "a", 10);
        assert!(log.sessions.is_empty(), "None persists nothing");
        log.record_connect_gated(&PrivacyLevel::Sessions { retention_days: 30 }, "a", 10);
        assert_eq!(log.sessions.len(), 1);
    }

    #[test]
    fn apply_retention_purges_by_level() {
        let mut log = SessionLog::default();
        let now = 10_000_000u64;
        log.record_connect("old", now - 2 * 86_400); // 2 days ago
        log.record_connect("fresh", now - 3_600); // 1h ago
        log.apply_retention(&PrivacyLevel::Sessions { retention_days: 1 }, now);
        assert_eq!(log.sessions.len(), 1);
        assert_eq!(log.sessions[0].npub, "fresh");
        // `None` is a no-op — it doesn't purge what's already there.
        log.apply_retention(&PrivacyLevel::None, now);
        assert_eq!(log.sessions.len(), 1);
    }

    #[test]
    fn forget_player_and_clear() {
        let mut log = SessionLog::default();
        log.record_connect("a", 10);
        log.record_connect("b", 20);
        log.record_connect("a", 30);
        log.forget_player("a");
        assert_eq!(log.sessions.len(), 1);
        assert_eq!(log.sessions[0].npub, "b");
        log.clear();
        assert!(log.sessions.is_empty());
    }
}
