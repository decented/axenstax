//! Outbox — queued reports awaiting relay flush. Enqueue is offline-safe,
//! `mark_sent` by id removes the entry (the ticket list keeps only a short
//! snippet; the full body is not retained once it is out), a failure leaves
//! the item queued, never double-sends. Whole-file JSON read-modify-write;
//! callers serialise access (the service holds one lock — see mod.rs).

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::tickets::write_private;

#[derive(Clone, Serialize, Deserialize)]
pub struct QueuedReport {
    /// The ticket (S1): 128 random bits, hex. Travels only inside the
    /// encrypted report; the game keeps it locally.
    pub id: String,
    pub kind: String,
    pub body: String,
    pub created_at: u64,
    /// `"queued"`, or `"sent"` on a file written by a pre-status-board build
    /// (see [`drain_legacy_sent`]).
    #[serde(default)]
    pub status: String,
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// S1: 32 lowercase hex chars = 128 bits from the OS CSPRNG. The id is the
/// ticket, so it must be unguessable, not merely unique.
pub fn random_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| format!("OS RNG unavailable: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn load(path: &Path) -> Vec<QueuedReport> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save(path: &Path, all: &[QueuedReport]) -> Result<(), String> {
    let json = serde_json::to_vec(all).map_err(|e| format!("serialise outbox: {e}"))?;
    write_private(path, &json)
}

pub fn enqueue(path: &Path, kind: &str, body: &str) -> Result<QueuedReport, String> {
    let mut all = load(path);
    let rec = QueuedReport {
        id: random_id()?,
        kind: kind.to_string(),
        body: body.to_string(),
        created_at: now_secs(),
        status: "queued".to_string(),
    };
    all.push(rec.clone());
    save(path, &all)?;
    Ok(rec)
}

/// Queued (unsent) reports, oldest first.
pub fn queued(path: &Path) -> Vec<QueuedReport> {
    let mut q: Vec<QueuedReport> =
        load(path).into_iter().filter(|r| r.status == "queued").collect();
    q.sort_by_key(|r| r.created_at);
    q
}

/// The report is out: drop it. Idempotent.
pub fn mark_sent(path: &Path, id: &str) -> Result<(), String> {
    let mut all = load(path);
    let before = all.len();
    all.retain(|r| r.id != id);
    if all.len() == before {
        return Ok(());
    }
    save(path, &all)
}

/// A pre-status-board build kept sent reports (full body) in this file. Remove
/// them and hand them back so the caller can keep each as a sent ticket.
pub fn drain_legacy_sent(path: &Path) -> Vec<QueuedReport> {
    let all = load(path);
    if !all.iter().any(|r| r.status == "sent") {
        return Vec::new();
    }
    let (sent, keep): (Vec<_>, Vec<_>) = all.into_iter().partition(|r| r.status == "sent");
    if save(path, &keep).is_err() {
        return Vec::new(); // could not drop them: try again next launch
    }
    sent
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("axemb-ob-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("mailbox_outbox.json")
    }

    #[test]
    fn enqueue_persists_and_lists_oldest_first() {
        let p = tmp("basic");
        let a = enqueue(&p, "bug", "doors too tall").unwrap();
        let b = enqueue(&p, "idea", "rainbow sheep").unwrap();
        let q = queued(&p);
        assert_eq!(q.len(), 2);
        assert_eq!(q[0].id, a.id, "oldest first");
        assert!(q.iter().all(|r| r.status == "queued"));
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn ticket_id_is_at_least_128_bits_of_hex() {
        let ids: Vec<String> = (0..64).map(|_| random_id().unwrap()).collect();
        for id in &ids {
            assert_eq!(id.len(), 32, "16 bytes = 128 bits");
            assert!(id.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
        }
        let mut uniq = ids.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), ids.len(), "no collisions across 64 draws");
        // Not a counter / timestamp: bytes vary across draws.
        assert!(ids.iter().any(|i| i[..8] != ids[0][..8]));
    }

    #[test]
    fn mark_sent_removes_from_queue_and_is_idempotent() {
        let p = tmp("sent");
        let a = enqueue(&p, "bug", "one").unwrap();
        enqueue(&p, "bug", "two").unwrap();
        mark_sent(&p, &a.id).unwrap();
        mark_sent(&p, &a.id).unwrap(); // second call: no error, no change
        let q = queued(&p);
        assert_eq!(q.len(), 1, "sent item never re-flushes (no double-send)");
        assert_eq!(q[0].body, "two");
        assert!(!std::fs::read_to_string(&p).unwrap().contains("one"), "sent body not retained");
    }

    #[test]
    fn legacy_sent_entries_are_drained_with_their_ids() {
        let p = tmp("legacy");
        std::fs::write(
            &p,
            br#"[{"id":"aa","kind":"bug","body":"old","persona_hex":"cd","created_at":5,"status":"sent","event_id":"ee"},
                 {"id":"bb","kind":"idea","body":"waiting","persona_hex":null,"created_at":6,"status":"queued","event_id":null}]"#,
        )
        .unwrap();
        let drained = drain_legacy_sent(&p);
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].id, "aa");
        let q = queued(&p);
        assert_eq!(q.len(), 1, "queued item (old persona field ignored) survives");
        assert_eq!(q[0].id, "bb");
        assert!(drain_legacy_sent(&p).is_empty());
    }

    #[test]
    fn corrupt_outbox_degrades_to_empty_not_panic() {
        let p = tmp("corrupt");
        std::fs::write(&p, b"garbage").unwrap();
        assert!(queued(&p).is_empty());
        enqueue(&p, "bug", "still works").unwrap();
        assert_eq!(queued(&p).len(), 1);
    }
}
