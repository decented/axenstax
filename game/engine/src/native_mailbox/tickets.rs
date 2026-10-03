//! Local ticket list — what `/mailbox` shows (spec S7). One entry per report
//! this install sent: the ticket (the report id, which only ever travels
//! inside the encrypted report), the kind, the first ~60 characters of the
//! body, the sent date, and the latest status read off the public board. All
//! of it is LOCAL; none of it is ever sent anywhere. Whole-file JSON, 0600,
//! pruned after 180 days. Callers serialise access (the service lock in
//! mod.rs).

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::board::{ticket_key, Board, Status};

/// Tickets older than this are dropped (S7), matching the board's own pruning.
pub const MAX_AGE_SECS: u64 = 180 * 24 * 3600;
const SNIPPET_CHARS: usize = 60;
/// How many tickets `/mailbox` lists.
pub const VIEW_LIMIT: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    pub id: String,
    pub kind: String,
    pub snippet: String,
    pub created_at: u64,
    pub sent_at: Option<u64>,
    pub status: Option<Status>,
}

/// Collapse whitespace/control characters and cut to ~60 characters.
pub fn snippet_of(body: &str) -> String {
    let flat: String = body
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if flat.chars().count() <= SNIPPET_CHARS {
        flat
    } else {
        let cut: String = flat.chars().take(SNIPPET_CHARS).collect();
        format!("{}…", cut.trim_end())
    }
}

fn load(path: &Path) -> Vec<Ticket> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Write a file readable by its owner only. Reports and tickets are private
/// player text; the mode is set at creation, not chmod'ed afterwards.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir profile: {e}"))?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    f.write_all(bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        // A pre-existing file keeps its old mode through `mode()`; tighten it.
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn save(path: &Path, all: &[Ticket]) -> Result<(), String> {
    let json = serde_json::to_vec(all).map_err(|e| format!("serialise tickets: {e}"))?;
    write_private(path, &json)
}

fn prune_vec(all: &mut Vec<Ticket>, now: u64) -> bool {
    let before = all.len();
    all.retain(|t| now.saturating_sub(t.created_at) <= MAX_AGE_SECS);
    all.len() != before
}

/// Record a new, not-yet-sent report. Idempotent on `id`.
pub fn record_new(path: &Path, id: &str, kind: &str, body: &str, created_at: u64) -> Result<(), String> {
    let mut all = load(path);
    if all.iter().any(|t| t.id == id) {
        return Ok(());
    }
    all.push(Ticket {
        id: id.to_string(),
        kind: kind.to_string(),
        snippet: snippet_of(body),
        created_at,
        sent_at: None,
        status: None,
    });
    prune_vec(&mut all, created_at);
    save(path, &all)
}

/// A report that an older build already sent: keep it as a sent ticket (its id
/// may well be on the board). Idempotent on `id`.
pub fn record_sent(
    path: &Path,
    id: &str,
    kind: &str,
    body: &str,
    created_at: u64,
    sent_at: u64,
) -> Result<(), String> {
    record_new(path, id, kind, body, created_at)?;
    mark_sent(path, id, sent_at)
}

pub fn mark_sent(path: &Path, id: &str, at: u64) -> Result<(), String> {
    let mut all = load(path);
    for t in all.iter_mut().filter(|t| t.id == id && t.sent_at.is_none()) {
        t.sent_at = Some(at);
    }
    save(path, &all)
}

pub fn is_empty(path: &Path) -> bool {
    load(path).is_empty()
}

/// Drop tickets past their 180 days. Writes only when something went.
pub fn prune(path: &Path, now: u64) {
    let mut all = load(path);
    if prune_vec(&mut all, now) {
        let _ = save(path, &all);
    }
}

/// S7: match the board against the local tickets by `ticket_key`. A ticket the
/// board does not mention keeps whatever status it already had. Returns how
/// many tickets changed.
pub fn apply_board(path: &Path, board: &Board, now: u64) -> Result<usize, String> {
    let mut all = load(path);
    let pruned = prune_vec(&mut all, now);
    let mut changed = 0;
    for t in all.iter_mut() {
        if let Some(st) = board.entries.get(&ticket_key(&t.id))
            && t.status.as_ref() != Some(st)
        {
            t.status = Some(st.clone());
            changed += 1;
        }
    }
    if changed > 0 || pruned {
        save(path, &all)?;
    }
    Ok(changed)
}

/// `Sent` / `Received` / `Fixed in vX` / `Won't fix` (+ `Waiting to send`).
pub fn status_text(t: &Ticket) -> String {
    match &t.status {
        Some(Status::Received) => "Received".into(),
        Some(Status::Fixed(Some(v))) => format!("Fixed in v{v}"),
        Some(Status::Fixed(None)) => "Fixed".into(),
        Some(Status::WontFix) => "Won't fix".into(),
        None if t.sent_at.is_some() => "Sent".into(),
        None => "Waiting to send".into(),
    }
}

/// `YYYY-MM-DD` (UTC) from unix seconds — civil-from-days, no date crate.
pub fn ymd(secs: u64) -> String {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// One chat-ready line for `/mailbox`.
pub fn line(t: &Ticket) -> String {
    format!(
        "{} · {} · \"{}\" — {}",
        t.kind,
        ymd(t.sent_at.unwrap_or(t.created_at)),
        t.snippet,
        status_text(t)
    )
}

/// Newest first, at most [`VIEW_LIMIT`], as chat-ready lines.
pub fn view_lines(path: &Path) -> Vec<String> {
    let mut all = load(path);
    all.sort_by_key(|t| std::cmp::Reverse(t.created_at));
    all.iter().take(VIEW_LIMIT).map(line).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("axemb-tk-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("mailbox_tickets.json")
    }

    fn board(pairs: &[(&str, Status)]) -> Board {
        Board {
            entries: pairs.iter().map(|(t, s)| (ticket_key(t), s.clone())).collect::<HashMap<_, _>>(),
        }
    }

    #[test]
    fn status_matches_only_the_tickets_it_names() {
        let p = tmp("match");
        record_new(&p, "t-mine", "bug", "doors too tall", 1000).unwrap();
        record_new(&p, "t-other", "idea", "rainbow sheep", 1001).unwrap();
        record_new(&p, "t-silent", "bug", "never mentioned", 1002).unwrap();
        let b = board(&[
            ("t-mine", Status::Fixed(Some("0.2.28".into()))),
            ("t-other", Status::WontFix),
            ("t-not-ours", Status::Received),
        ]);
        assert_eq!(apply_board(&p, &b, 2000).unwrap(), 2);
        let all = load(&p);
        let get = |id: &str| all.iter().find(|t| t.id == id).unwrap().clone();
        assert_eq!(status_text(&get("t-mine")), "Fixed in v0.2.28");
        assert_eq!(status_text(&get("t-other")), "Won't fix");
        assert_eq!(get("t-silent").status, None, "unlisted ticket untouched");
        // Re-applying the same board changes nothing.
        assert_eq!(apply_board(&p, &b, 2000).unwrap(), 0);
    }

    #[test]
    fn a_ticket_the_board_forgets_keeps_its_last_known_status() {
        let p = tmp("forget");
        record_new(&p, "t1", "bug", "x", 1000).unwrap();
        apply_board(&p, &board(&[("t1", Status::Received)]), 2000).unwrap();
        apply_board(&p, &board(&[]), 3000).unwrap();
        assert_eq!(load(&p)[0].status, Some(Status::Received));
    }

    #[test]
    fn labels_for_every_state() {
        let mut t = Ticket {
            id: "i".into(),
            kind: "bug".into(),
            snippet: "s".into(),
            created_at: 0,
            sent_at: None,
            status: None,
        };
        assert_eq!(status_text(&t), "Waiting to send");
        t.sent_at = Some(5);
        assert_eq!(status_text(&t), "Sent");
        t.status = Some(Status::Received);
        assert_eq!(status_text(&t), "Received");
        t.status = Some(Status::Fixed(None));
        assert_eq!(status_text(&t), "Fixed");
        t.status = Some(Status::WontFix);
        assert_eq!(status_text(&t), "Won't fix");
    }

    #[test]
    fn snippet_is_flat_and_about_sixty_chars() {
        assert_eq!(snippet_of("  doors\n\ttoo   tall "), "doors too tall");
        let long = "word ".repeat(40);
        let s = snippet_of(&long);
        assert!(s.ends_with('…'));
        assert!(s.chars().count() <= SNIPPET_CHARS + 1);
        assert_eq!(snippet_of("é".repeat(100).as_str()).chars().count(), SNIPPET_CHARS + 1, "char-safe cut");
    }

    #[test]
    fn dates_are_civil_utc() {
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(ymd(951_782_400), "2000-02-29");
        assert_eq!(ymd(1_790_812_800), "2026-10-01");
    }

    #[test]
    fn tickets_older_than_180_days_are_pruned() {
        let p = tmp("prune");
        let now = 400 * 86_400;
        record_new(&p, "old", "bug", "ancient", now - MAX_AGE_SECS - 10).unwrap();
        record_new(&p, "new", "bug", "fresh", now - 10).unwrap();
        // `record_new` prunes relative to the new ticket's own time, so the
        // old one may already be gone; either way prune() leaves only "new".
        prune(&p, now);
        let all = load(&p);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "new");
    }

    #[test]
    fn view_is_newest_first_and_capped() {
        let p = tmp("view");
        for i in 0..15u64 {
            record_new(&p, &format!("t{i}"), "bug", &format!("report {i}"), 1000 + i).unwrap();
        }
        let v = view_lines(&p);
        assert_eq!(v.len(), VIEW_LIMIT);
        assert!(v[0].contains("report 14"));
        assert!(v[0].contains("Waiting to send"));
    }

    #[test]
    fn record_is_idempotent_and_sent_sticks() {
        let p = tmp("idem");
        record_new(&p, "a", "bug", "one", 10).unwrap();
        record_new(&p, "a", "bug", "one again", 11).unwrap();
        mark_sent(&p, "a", 20).unwrap();
        mark_sent(&p, "a", 99).unwrap();
        let all = load(&p);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].snippet, "one");
        assert_eq!(all[0].sent_at, Some(20), "first send time kept");
    }

    #[cfg(unix)]
    #[test]
    fn ticket_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let p = tmp("perm");
        record_new(&p, "a", "bug", "private", 10).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn corrupt_store_degrades_to_empty() {
        let p = tmp("corrupt");
        std::fs::write(&p, b"junk").unwrap();
        assert!(is_empty(&p));
        record_new(&p, "a", "bug", "still works", 10).unwrap();
        assert_eq!(load(&p).len(), 1);
    }
}
