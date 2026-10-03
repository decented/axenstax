//! Fetching and accepting the projection (WIRE.md §2/§5, spec D4/D7/D10).
//!
//! Filter: `kinds:[30078], authors:[rail], #d:[projection_tag(grant_id)]`,
//! sent to the grant's own relay (the ack's, D4). Every event is then checked
//! again here, because a relay may answer with anything: signature, author ==
//! rail, kind, `d` tag, vault-envelope open, `parse_projection` against the
//! grant's capabilities and staleness ceiling, `grant_id` == the grant's, and
//! only then newest-wins (`is_newer`; a revocation is exempt).

use nostr::{Event, Kind, PublicKey};
use serde_json::{json, Value};

use crate::signet::contacts_wire::constants::PROJECTION_KIND;
use crate::signet::contacts_wire::{
    is_newer, open_vault_envelope_text, parse_projection, projection_tag, Projection,
};

use super::store::{self, Grant, Loaded, Snapshot, StorePaths};

pub fn projection_filter(grant: &Grant) -> Value {
    json!({
        "kinds": [PROJECTION_KIND],
        "authors": [grant.rail_pubkey],
        "#d": [projection_tag(&grant.grant_id)],
        "limit": 5,
    })
}

/// Open and check one event against the grant. `None` for anything that is
/// not this grant's projection.
pub fn open_projection(ev: &Event, grant: &Grant) -> Option<Projection> {
    if ev.kind != Kind::Custom(PROJECTION_KIND) || ev.pubkey.to_hex() != grant.rail_pubkey {
        return None;
    }
    if ev.verify().is_err() {
        return None;
    }
    if ev.tags.identifier() != Some(projection_tag(&grant.grant_id).as_str()) {
        return None;
    }
    let keys = grant.app_keys()?;
    let rail = PublicKey::from_hex(&grant.rail_pubkey).ok()?;
    let body = open_vault_envelope_text(&ev.content, keys.secret_key(), &rail)?;
    let p = parse_projection(&body, &grant.granted_capabilities, grant.max_staleness_seconds)?;
    (p.grant_id == grant.grant_id).then_some(p)
}

/// What a fetch decided.
#[derive(Debug, PartialEq, Eq)]
pub enum Accepted {
    /// Nothing usable, or nothing newer than what is held.
    Unchanged,
    Replace(Box<Projection>),
    /// The grant ended (D10): delete grant + snapshot.
    Revoked,
}

/// Choose from `events` against the projection already `held`.
pub fn accept_projection(events: &[Event], grant: &Grant, held: Option<&Projection>) -> Accepted {
    let mut best: Option<Projection> = None;
    for ev in events {
        let Some(p) = open_projection(ev, grant) else { continue };
        if p.revoked {
            return Accepted::Revoked;
        }
        if best.as_ref().is_none_or(|b| is_newer(&p, b)) {
            best = Some(p);
        }
    }
    match best {
        Some(p) if held.is_none_or(|h| is_newer(&p, h)) => Accepted::Replace(Box::new(p)),
        _ => Accepted::Unchanged,
    }
}

/// The result of one sync pass, for the status line.
#[derive(Debug, PartialEq, Eq)]
pub enum SyncResult {
    NoGrant,
    /// The relay could not be reached. Silent except "last synced" (D11).
    Failed,
    Unchanged,
    Updated,
    Revoked,
}

/// One sync pass: load the grant, fetch from ITS relay, accept, write.
pub fn sync_once(
    paths: &StorePaths,
    now: u64,
    fetch: &mut dyn FnMut(&str, &Value) -> Result<Vec<Event>, String>,
) -> SyncResult {
    let Loaded::Ok(grant) = store::load_grant(&paths.grant) else {
        return SyncResult::NoGrant;
    };
    let events = match fetch(&grant.relay, &projection_filter(&grant)) {
        Ok(evs) => evs,
        Err(e) => {
            log::debug!("[signet-contacts] fetch failed: {e}");
            return SyncResult::Failed;
        }
    };
    // Only a snapshot from THIS grant counts as "held" — a leftover from an
    // older grant must never block a newer projection (review 2026-10-01).
    let held = super::current_snapshot(paths);
    match accept_projection(&events, &grant, held.as_ref().map(|s| &s.projection)) {
        Accepted::Unchanged => SyncResult::Unchanged,
        Accepted::Revoked => {
            if let Err(e) = store::delete_all(paths) {
                log::warn!("[signet-contacts] could not remove a revoked grant: {e}");
            }
            SyncResult::Revoked
        }
        Accepted::Replace(p) => {
            let snap = Snapshot::replacing(held.as_ref(), *p, now);
            match store::save_snapshot(&paths.snapshot, &snap) {
                Ok(()) => SyncResult::Updated,
                Err(e) => {
                    log::warn!("[signet-contacts] could not save the snapshot: {e}");
                    SyncResult::Failed
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
    use base64::Engine;
    use nostr::{EventBuilder, Keys, Tag, Timestamp};

    use crate::signet::contacts_wire::Capability;

    const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

    /// Test-only producer half of the vault envelope (WIRE.md §1).
    fn seal(body: &str, rail: &Keys, app: &PublicKey) -> String {
        let key = [7u8; 32];
        let iv = [9u8; 12];
        let mut padded = vec![0u8; 4096];
        padded[..4].copy_from_slice(&(body.len() as u32).to_be_bytes());
        padded[4..4 + body.len()].copy_from_slice(body.as_bytes());
        let ct = Aes256Gcm::new_from_slice(&key).unwrap().encrypt(Nonce::from_slice(&iv), padded.as_slice()).unwrap();
        let k = nostr::nips::nip44::encrypt(rail.secret_key(), app, B64.encode(key), nostr::nips::nip44::Version::V2).unwrap();
        json!({"v": 2, "k": k, "iv": B64.encode(iv), "ct": B64.encode(ct), "b": 4096}).to_string()
    }

    fn body(grant_id: &str, published_at: u64, revoked: bool) -> String {
        let mut b = json!({
            "v": 2, "grantId": grant_id,
            "scopes": ["signet.contacts.read:directory", "signet.contacts.read:tier", "signet.contacts.blocks.read"],
            "frontier": {"maxClock": 1, "opCount": 1, "publishedAt": published_at, "deviceId": "2".repeat(32)},
            "issuedAt": 100, "expiresAt": 200,
            "contacts": [{"contactId": "3".repeat(32), "identities": [{"pubkey": "4".repeat(64)}], "effectiveTier": "kin"}],
        });
        if revoked {
            b["revoked"] = json!(true);
        }
        b.to_string()
    }

    struct World {
        app: Keys,
        rail: Keys,
        grant: Grant,
    }

    fn world() -> World {
        let app = Keys::generate();
        let rail = Keys::generate();
        let grant = Grant {
            v: 1,
            app_secret_hex: app.secret_key().to_secret_hex(),
            grant_id: "a".repeat(32),
            rail_pubkey: rail.public_key().to_hex(),
            projection_tag: projection_tag(&"a".repeat(32)),
            relay: "wss://relay.example.com".into(),
            granted_capabilities: vec![Capability::ReadDirectory, Capability::ReadTier, Capability::BlocksRead],
            max_staleness_seconds: 21_600,
            paired_at: 1,
        };
        World { app, rail, grant }
    }

    fn event(w: &World, signer: &Keys, body: &str, d: &str) -> Event {
        EventBuilder::new(Kind::Custom(PROJECTION_KIND), seal(body, &w.rail, &w.app.public_key()))
            .tags([Tag::identifier(d)])
            .custom_created_at(Timestamp::from_secs(150))
            .sign_with_keys(signer)
            .unwrap()
    }

    #[test]
    fn the_real_projection_is_accepted() {
        let w = world();
        let d = projection_tag(&w.grant.grant_id);
        let ev = event(&w, &w.rail, &body(&w.grant.grant_id, 10, false), &d);
        assert!(matches!(accept_projection(&[ev], &w.grant, None), Accepted::Replace(_)));
    }

    #[test]
    fn a_projection_from_the_wrong_author_is_rejected() {
        let w = world();
        let d = projection_tag(&w.grant.grant_id);
        // Sealed correctly (the attacker has the rail's envelope), signed by someone else.
        let ev = event(&w, &Keys::generate(), &body(&w.grant.grant_id, 10, false), &d);
        assert_eq!(accept_projection(&[ev], &w.grant, None), Accepted::Unchanged);
    }

    #[test]
    fn a_projection_for_another_grant_is_rejected() {
        let w = world();
        let d = projection_tag(&w.grant.grant_id);
        let ev = event(&w, &w.rail, &body(&"b".repeat(32), 10, false), &d);
        assert_eq!(accept_projection(&[ev], &w.grant, None), Accepted::Unchanged);
    }

    #[test]
    fn a_projection_with_the_wrong_d_tag_is_rejected() {
        let w = world();
        let ev = event(&w, &w.rail, &body(&w.grant.grant_id, 10, false), &"9".repeat(32));
        assert_eq!(accept_projection(&[ev], &w.grant, None), Accepted::Unchanged);
    }

    #[test]
    fn an_older_or_equal_projection_never_replaces_a_newer_one() {
        let w = world();
        let d = projection_tag(&w.grant.grant_id);
        let held = parse_projection(&body(&w.grant.grant_id, 10, false), &w.grant.granted_capabilities, 21_600).unwrap();
        let older = event(&w, &w.rail, &body(&w.grant.grant_id, 9, false), &d);
        let same = event(&w, &w.rail, &body(&w.grant.grant_id, 10, false), &d);
        assert_eq!(accept_projection(&[older, same], &w.grant, Some(&held)), Accepted::Unchanged);
        let newer = event(&w, &w.rail, &body(&w.grant.grant_id, 11, false), &d);
        assert!(matches!(accept_projection(&[newer], &w.grant, Some(&held)), Accepted::Replace(_)));
    }

    #[test]
    fn a_revocation_wins_even_when_older() {
        let w = world();
        let d = projection_tag(&w.grant.grant_id);
        let held = parse_projection(&body(&w.grant.grant_id, 10, false), &w.grant.granted_capabilities, 21_600).unwrap();
        let tomb = event(&w, &w.rail, &body(&w.grant.grant_id, 1, true), &d);
        assert_eq!(accept_projection(&[tomb], &w.grant, Some(&held)), Accepted::Revoked);
    }

    #[test]
    fn sync_once_writes_then_revocation_deletes_everything() {
        let w = world();
        let dir = std::env::temp_dir().join(format!("axenstax-signet-sync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = StorePaths::in_profile(&dir);
        let d = projection_tag(&w.grant.grant_id);

        assert_eq!(sync_once(&paths, 5, &mut |_, _| Ok(vec![])), SyncResult::NoGrant);
        store::save_grant(&paths.grant, &w.grant).unwrap();

        let ev = event(&w, &w.rail, &body(&w.grant.grant_id, 10, false), &d);
        let mut asked = Vec::new();
        let r = sync_once(&paths, 5, &mut |relay, f| {
            asked.push((relay.to_owned(), f.clone()));
            Ok(vec![ev.clone()])
        });
        assert_eq!(r, SyncResult::Updated);
        // D4: the grant's (ack's) relay; the exact projection filter.
        assert_eq!(asked[0].0, "wss://relay.example.com");
        assert_eq!(asked[0].1["authors"], json!([w.grant.rail_pubkey]));
        assert_eq!(asked[0].1["#d"], json!([d]));
        assert!(store::load_snapshot(&paths.snapshot).ok().is_some());

        assert_eq!(sync_once(&paths, 6, &mut |_, _| Ok(vec![ev.clone()])), SyncResult::Unchanged);
        assert_eq!(sync_once(&paths, 6, &mut |_, _| Err("down".into())), SyncResult::Failed);

        let tomb = event(&w, &w.rail, &body(&w.grant.grant_id, 11, true), &d);
        assert_eq!(sync_once(&paths, 7, &mut |_, _| Ok(vec![tomb.clone()])), SyncResult::Revoked);
        assert!(!paths.grant.exists() && !paths.snapshot.exists());
    }
}
