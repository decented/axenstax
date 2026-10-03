//! What a stored Signet snapshot contributes to the address book (spec D2,
//! D8–D10). Pure: the assembly itself is `contacts::assemble_book`.

use crate::comms::Tier;
use crate::contacts::{AddedVia, Contact, SignetPart};
use crate::signet::contacts_wire::projection::Tier as WireTier;

use super::store::Snapshot;

fn pubkey_bytes(hex_str: &str) -> Option<[u8; 32]> {
    hex::decode(hex_str).ok().and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
}

/// D9 tier map. `none` and an absent tier add nobody.
fn map_tier(t: Option<WireTier>) -> Option<Tier> {
    match t? {
        WireTier::Kin => Some(Tier::Kin),
        WireTier::Kith => Some(Tier::Kith),
        WireTier::Ken => Some(Tier::Ken),
        WireTier::None => None,
    }
}

/// Whether the snapshot is past its own `expiresAt` (D10).
pub fn is_stale(snapshot: &Snapshot, now: u64) -> bool {
    now > snapshot.projection.expires_at
}

/// The part of the book this snapshot supplies at `now`:
///
/// - `revoked` → nothing at all (the worker deletes it on sight; this is the
///   belt to that braces).
/// - every pubkey on a `blocked: true` contact → `blocked`, whatever else the
///   contact says, and even when the snapshot is stale (D8, D10).
/// - otherwise each `identities[].pubkey` of a contact with tier kin/kith/ken
///   → one `Contact` (same tier and name for all of them), `AddedVia::Import`,
///   `is_child: false` (D2), `added_at` = first time seen — unless the
///   snapshot is stale, when nobody is added (fail closed, D10).
pub fn book_part(snapshot: &Snapshot, now: u64) -> Option<SignetPart> {
    let p = &snapshot.projection;
    if p.revoked {
        return None;
    }
    let stale = is_stale(snapshot, now);
    let mut part = SignetPart::default();
    for c in &p.contacts {
        let ids: Vec<([u8; 32], &String)> = c
            .identities
            .iter()
            .flatten()
            .filter_map(|i| pubkey_bytes(&i.pubkey).map(|b| (b, &i.pubkey)))
            .collect();
        if c.blocked == Some(true) {
            part.blocked.extend(ids.iter().map(|(b, _)| *b));
            continue;
        }
        if stale {
            continue;
        }
        let Some(tier) = map_tier(c.effective_tier) else {
            continue;
        };
        let name = c.display_name.clone().filter(|n| !n.trim().is_empty());
        for (pk, hex_pk) in ids {
            part.contacts.push(Contact {
                pubkey: pk,
                display_name: name.clone(),
                tier,
                is_child: false,
                runtime_pubkey: None,
                added_via: AddedVia::Import,
                added_at: snapshot.first_seen.get(hex_pk).copied().unwrap_or(snapshot.fetched_at),
                last_joined: None,
            });
        }
    }
    Some(part)
}

/// How many people (not pubkeys) the snapshot currently adds — the "N
/// contacts" on the Friends row.
pub fn people_count(snapshot: &Snapshot, now: u64) -> usize {
    if snapshot.projection.revoked || is_stale(snapshot, now) {
        return 0;
    }
    snapshot
        .projection
        .contacts
        .iter()
        .filter(|c| c.blocked != Some(true) && map_tier(c.effective_tier).is_some())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts::assemble_book;
    use crate::signet::contacts_wire::projection::{Frontier, Identity, ProjectedContact};
    use crate::signet::contacts_wire::{Capability, Projection};

    fn pk(n: u8) -> [u8; 32] {
        [n; 32]
    }

    fn contact(ids: &[u8], tier: Option<WireTier>, blocked: Option<bool>, name: Option<&str>) -> ProjectedContact {
        ProjectedContact {
            contact_id: format!("{:032x}", ids.first().copied().unwrap_or(0)),
            identities: Some(
                ids.iter().map(|n| Identity { pubkey: hex::encode(pk(*n)), verification: None }).collect(),
            ),
            display_name: name.map(str::to_owned),
            effective_tier: tier,
            tier_source: None,
            roles: None,
            contact_methods: None,
            blocked,
            checks: None,
        }
    }

    fn snapshot(contacts: Vec<ProjectedContact>, expires_at: u64) -> Snapshot {
        let p = Projection {
            v: 2,
            grant_id: "a".repeat(32),
            scopes: vec![Capability::ReadDirectory, Capability::ReadTier, Capability::BlocksRead],
            frontier: Frontier { max_clock: 1, op_count: 1, published_at: 1, device_id: "d".repeat(32) },
            issued_at: 0,
            expires_at,
            contacts,
            revoked: false,
            truncated: false,
        };
        Snapshot::replacing(None, p, 50)
    }

    fn local(n: u8, tier: Tier) -> Contact {
        Contact {
            pubkey: pk(n),
            display_name: Some("mine".into()),
            tier,
            is_child: false,
            runtime_pubkey: None,
            added_via: AddedVia::Paste,
            added_at: 10,
            last_joined: None,
        }
    }

    #[test]
    fn a_block_beats_local_kin_from_the_mirror() {
        let s = snapshot(vec![contact(&[1], Some(WireTier::Kin), Some(true), None)], 1000);
        let book = assemble_book(vec![local(1, Tier::Kith)], vec![local(1, Tier::Kin)], book_part(&s, 100));
        assert!(book.is_empty(), "blocked pubkey must leave the book entirely: {book:?}");
    }

    #[test]
    fn tier_none_and_absent_are_skipped() {
        let s = snapshot(
            vec![
                contact(&[1], Some(WireTier::None), None, None),
                contact(&[2], None, None, Some("x")),
                contact(&[3], Some(WireTier::Ken), Some(false), Some("k")),
            ],
            1000,
        );
        let part = book_part(&s, 100).unwrap();
        assert_eq!(part.contacts.len(), 1);
        assert_eq!(part.contacts[0].pubkey, pk(3));
        assert_eq!(part.contacts[0].tier, Tier::Ken);
        assert_eq!(people_count(&s, 100), 1);
    }

    #[test]
    fn a_multi_identity_contact_is_one_contact_per_pubkey() {
        let s = snapshot(vec![contact(&[4, 5, 6], Some(WireTier::Kith), None, Some("Ada"))], 1000);
        let part = book_part(&s, 100).unwrap();
        assert_eq!(part.contacts.len(), 3);
        for c in &part.contacts {
            assert_eq!(c.tier, Tier::Kith);
            assert_eq!(c.display_name.as_deref(), Some("Ada"));
            assert_eq!(c.added_via, AddedVia::Import);
            assert!(!c.is_child, "D2: Signet contacts are never marked as children");
            assert_eq!(c.added_at, 50);
        }
        assert_eq!(people_count(&s, 100), 1);
    }

    #[test]
    fn a_stale_snapshot_adds_nobody_but_its_blocks_still_apply() {
        let s = snapshot(
            vec![
                contact(&[1], Some(WireTier::Kin), None, None),
                contact(&[2], Some(WireTier::Kin), Some(true), None),
            ],
            1000,
        );
        let part = book_part(&s, 1001).unwrap();
        assert!(part.contacts.is_empty());
        assert_eq!(part.blocked, vec![pk(2)]);
        let book = assemble_book(vec![], vec![local(2, Tier::Kin), local(9, Tier::Kith)], Some(part));
        assert_eq!(book.len(), 1);
        assert_eq!(book[0].pubkey, pk(9));
        assert_eq!(people_count(&s, 1001), 0);
    }

    #[test]
    fn a_revoked_snapshot_contributes_nothing() {
        let mut s = snapshot(vec![contact(&[1], Some(WireTier::Kin), None, None)], 1000);
        s.projection.revoked = true;
        assert!(book_part(&s, 100).is_none());
        let book = assemble_book(vec![], vec![local(3, Tier::Ken)], book_part(&s, 100));
        assert_eq!(book.len(), 1);
    }

    #[test]
    fn signet_never_loosens_what_the_player_set() {
        // Local Kin stays Kin when Signet says Ken; local Ken rises to Signet's Kith.
        let s = snapshot(
            vec![
                contact(&[1], Some(WireTier::Ken), None, None),
                contact(&[2], Some(WireTier::Kith), None, None),
            ],
            1000,
        );
        let book = assemble_book(vec![], vec![local(1, Tier::Kin), local(2, Tier::Ken)], book_part(&s, 100));
        assert_eq!(crate::contacts::tier_of(&book, &pk(1)), Tier::Kin);
        assert_eq!(crate::contacts::tier_of(&book, &pk(2)), Tier::Kith);
        // The player's own name for someone is not replaced by a nameless row.
        assert_eq!(crate::contacts::find(&book, &pk(1)).unwrap().display_name.as_deref(), Some("mine"));
    }
}
