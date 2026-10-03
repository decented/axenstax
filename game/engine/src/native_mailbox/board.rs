//! The public feedback status board (spec
//! docs/foundations/2026-10-01-feedback-status-board.md, S2/S3/S4).
//!
//! One addressable kind-30078 event, signed by the pinned official AxeNStax
//! key, maps *scrambled ticket hashes* to a status. Nobody is messaged: the
//! game fetches the board and recognises its own tickets. Everything here is
//! pure — fetching lives in `relay.rs`, application in `tickets.rs`.

use std::collections::HashMap;

use nostr::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// `d` tag of the board event (S3).
pub const BOARD_D: &str = "axenstax-feedback-status";
const BOARD_KIND: u16 = 30078;
/// Reject a board larger than this outright (2000 entries are ~200 KB).
const MAX_BOARD_BYTES: usize = 1 << 20;
/// Entries read from one board (the publisher prunes to the same cap).
const MAX_ENTRIES: usize = 2000;
/// Longest version string shown to a player.
const MAX_VERSION_LEN: usize = 24;

/// What the makers have said about one report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Received,
    /// Fixed, in this build when the board names one.
    Fixed(Option<String>),
    WontFix,
}

/// S2: `sha256("axenstax-feedback-ticket:" + ticket)` hex, first 32 chars.
pub fn ticket_key(ticket: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"axenstax-feedback-ticket:");
    h.update(ticket.as_bytes());
    hex::encode(h.finalize())[..32].to_string()
}

/// REQ filter for the board: newest event by the pinned author with our `d`.
pub fn board_filter(author_hex: &str) -> serde_json::Value {
    serde_json::json!({
        "kinds": [BOARD_KIND],
        "authors": [author_hex],
        "#d": [BOARD_D],
        "limit": 10,
    })
}

/// A parsed board: ticket hash -> status.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Board {
    pub entries: HashMap<String, Status>,
}

/// A version is shown in a chat line, so keep it to a short, boring alphabet.
fn clean_version(v: &str) -> Option<String> {
    let ok = !v.is_empty()
        && v.len() <= MAX_VERSION_LEN
        && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    ok.then(|| v.to_string())
}

fn is_ticket_key(k: &str) -> bool {
    k.len() == 32 && k.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Parse one board's JSON content. Junk-tolerant: a bad entry is skipped, an
/// unknown status is ignored, a wrong shape or an oversize body yields `None`.
fn parse_content(content: &str) -> Option<Board> {
    if content.len() > MAX_BOARD_BYTES {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(content).ok()?;
    if v.get("v").and_then(|x| x.as_u64()) != Some(1) {
        return None;
    }
    let t = v.get("t")?.as_object()?;
    let mut entries = HashMap::new();
    for (key, val) in t.iter().take(MAX_ENTRIES) {
        if !is_ticket_key(key) {
            continue;
        }
        let version = val.get("v").and_then(|x| x.as_str()).and_then(clean_version);
        let status = match val.get("s").and_then(|x| x.as_str()) {
            Some("received") => Status::Received,
            Some("fixed") => Status::Fixed(version),
            Some("wontfix") => Status::WontFix,
            _ => continue,
        };
        entries.insert(key.clone(), status);
    }
    Some(Board { entries })
}

/// S3/S4: pick the newest board event that is genuinely the official key's —
/// author pinned, signature valid, right `d` — and parse it. Anything else
/// (strangers' events, forged signatures, junk content) is ignored.
pub fn parse_board(events: &[Event], official_hex: &str) -> Option<Board> {
    let mut valid: Vec<&Event> = events
        .iter()
        .filter(|e| e.kind.as_u16() == BOARD_KIND)
        .filter(|e| e.pubkey.to_hex() == official_hex)
        .filter(|e| {
            e.tags.iter().any(|t| {
                let s = t.as_slice();
                s.first().map(String::as_str) == Some("d")
                    && s.get(1).map(String::as_str) == Some(BOARD_D)
            })
        })
        .filter(|e| e.verify().is_ok())
        .collect();
    valid.sort_by_key(|e| std::cmp::Reverse(e.created_at.as_secs()));
    valid.into_iter().find_map(|e| parse_content(&e.content))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    async fn board_event(keys: &Keys, content: &str, at: u64, d: &str) -> Event {
        EventBuilder::new(Kind::from(BOARD_KIND), content)
            .tags([Tag::identifier(d)])
            .custom_created_at(Timestamp::from_secs(at))
            .sign(keys)
            .await
            .unwrap()
    }

    #[test]
    fn ticket_key_is_a_32_char_domain_separated_hash() {
        let k = ticket_key("abc");
        assert_eq!(k.len(), 32);
        assert!(is_ticket_key(&k));
        let mut h = Sha256::new();
        h.update(b"axenstax-feedback-ticket:abc");
        assert_eq!(k, hex::encode(h.finalize())[..32]);
        // Same vector as tools/feedback-reader/test/board.test.mjs: the game and
        // the reader must derive the identical key or no status ever matches.
        assert_eq!(k, "5ebff49fd28c07c0eedbfb153d274842");
        assert_eq!(
            ticket_key("00112233445566778899aabbccddeeff"),
            "0f90108eb8fc60ddb38e0a477df92cac"
        );
        assert_ne!(k, ticket_key("abd"), "different ticket, different key");
        assert!(!k.contains("abc"), "the ticket is not visible in its key");
    }

    #[test]
    fn parses_a_good_board_and_ignores_junk_entries() {
        let good = ticket_key("t1");
        let fixed = ticket_key("t2");
        let wf = ticket_key("t3");
        let content = format!(
            r#"{{"v":1,"updated":5,"t":{{
              "{good}":{{"s":"received","at":1}},
              "{fixed}":{{"s":"fixed","v":"0.2.28","at":2}},
              "{wf}":{{"s":"wontfix","at":3}},
              "{}":{{"s":"explodes","at":4}},
              "not-a-key":{{"s":"received","at":5}},
              "{}":"junk"
            }}}}"#,
            ticket_key("t4"),
            ticket_key("t5"),
        );
        let b = parse_content(&content).expect("parses");
        assert_eq!(b.entries.len(), 3, "unknown status, bad key and non-object skipped");
        assert_eq!(b.entries[&good], Status::Received);
        assert_eq!(b.entries[&fixed], Status::Fixed(Some("0.2.28".into())));
        assert_eq!(b.entries[&wf], Status::WontFix);
    }

    #[test]
    fn hostile_versions_are_dropped_not_displayed() {
        let k = ticket_key("t");
        for bad in ["", "x".repeat(25).as_str(), "1.0 <b>", "v\u{202e}1", "1\n2"] {
            let c = serde_json::json!({"v":1,"t":{k.clone():{"s":"fixed","v":bad}}}).to_string();
            let b = parse_content(&c).unwrap();
            assert_eq!(b.entries[&k], Status::Fixed(None), "bad version {bad:?} dropped");
        }
    }

    #[test]
    fn wrong_shape_or_oversize_content_is_rejected_without_panic() {
        assert!(parse_content("not json").is_none());
        assert!(parse_content("[]").is_none());
        assert!(parse_content(r#"{"v":2,"t":{}}"#).is_none(), "unknown schema version");
        assert!(parse_content(r#"{"v":1}"#).is_none());
        assert!(parse_content(r#"{"v":1,"t":[]}"#).is_none());
        let big = format!(r#"{{"v":1,"t":{{}},"pad":"{}"}}"#, "x".repeat(MAX_BOARD_BYTES));
        assert!(parse_content(&big).is_none(), "oversize rejected");
    }

    #[test]
    fn entry_count_is_capped() {
        let mut t = serde_json::Map::new();
        for i in 0..(MAX_ENTRIES + 50) {
            t.insert(ticket_key(&i.to_string()), serde_json::json!({"s":"received"}));
        }
        let c = serde_json::json!({"v":1,"t":t}).to_string();
        assert_eq!(parse_content(&c).unwrap().entries.len(), MAX_ENTRIES);
    }

    #[test]
    fn only_the_pinned_author_with_a_valid_signature_counts() {
        rt().block_on(async {
            let official = Keys::generate();
            let stranger = Keys::generate();
            let hex = official.public_key().to_hex();
            let k = ticket_key("mine");
            let content = |s: &str| format!(r#"{{"v":1,"t":{{"{k}":{{"s":"{s}"}}}}}}"#);

            // A stranger's "fixed" board — newer — must lose to the official one.
            let evil = board_event(&stranger, &content("fixed"), 2000, BOARD_D).await;
            let real = board_event(&official, &content("received"), 1000, BOARD_D).await;
            let b = parse_board(&[evil.clone(), real.clone()], &hex).expect("official board found");
            assert_eq!(b.entries[&k], Status::Received);

            // Only a stranger's board: nothing.
            assert!(parse_board(&[evil], &hex).is_none());

            // Wrong `d` tag: not the board.
            let other_d = board_event(&official, &content("fixed"), 3000, "something-else").await;
            assert!(parse_board(&[other_d], &hex).is_none());

            // Tampered content ⇒ signature no longer verifies ⇒ rejected.
            let mut forged = real.clone();
            forged.content = content("fixed");
            assert!(parse_board(&[forged], &hex).is_none(), "bad signature rejected");

            // Wrong kind is ignored.
            let note = EventBuilder::text_note(content("fixed")).sign(&official).await.unwrap();
            assert!(parse_board(&[note], &hex).is_none());
        });
    }

    #[test]
    fn newest_valid_board_wins_and_junk_newest_falls_back() {
        rt().block_on(async {
            let official = Keys::generate();
            let hex = official.public_key().to_hex();
            let k = ticket_key("t");
            let old = board_event(
                &official,
                &format!(r#"{{"v":1,"t":{{"{k}":{{"s":"received"}}}}}}"#),
                100,
                BOARD_D,
            )
            .await;
            let new = board_event(
                &official,
                &format!(r#"{{"v":1,"t":{{"{k}":{{"s":"fixed","v":"1.0.0"}}}}}}"#),
                200,
                BOARD_D,
            )
            .await;
            let b = parse_board(&[old.clone(), new], &hex).unwrap();
            assert_eq!(b.entries[&k], Status::Fixed(Some("1.0.0".into())));

            let junk = board_event(&official, "garbage", 300, BOARD_D).await;
            let b = parse_board(&[old, junk], &hex).unwrap();
            assert_eq!(b.entries[&k], Status::Received, "junk newest falls back to older valid board");
        });
    }

    #[test]
    fn filter_pins_author_kind_and_d() {
        let f = board_filter(&"ab".repeat(32));
        assert_eq!(f["kinds"][0], 30078);
        assert_eq!(f["authors"][0], "ab".repeat(32));
        assert_eq!(f["#d"][0], BOARD_D);
    }
}
