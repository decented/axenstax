//! Wire shapes — rumor construction and NIP-59 wrap, all via rust-nostr (no
//! hand-rolled crypto). The rumor matches web `mailbox.js` exactly, except
//! `client = axenstax-native`. Spec 2026-10-01-feedback-status-board.md (S5):
//! every report is sealed with a fresh per-report burner key and carries NO
//! identity tag — no persona, no handle.

use nostr::prelude::*;

use super::outbox::QueuedReport;

/// The committed public AxeNStax identity — the key reports are encrypted to
/// and the only key whose status board the game believes. Safe to ship.
pub const OFFICIAL_AXENSTAX_PUBKEY_HEX: &str =
    "0bb8a9199a3e3c240a378cf1b5a977945decf05fc7d3021f66faa6b049cb55fd";

/// Kind-14 rumor for one queued report. Unsigned — sealing signs it.
/// `sender` is the report's one-time burner key.
pub fn build_rumor(sender: PublicKey, report: &QueuedReport) -> UnsignedEvent {
    let tags: Vec<Tag> = vec![
        Tag::hashtag(report.kind.clone()),
        Tag::custom(TagKind::custom("report-id"), [report.id.clone()]),
        Tag::custom(TagKind::custom("client"), ["axenstax-native".to_string()]),
        // Which build sent it. A stale install is the first thing to rule out
        // on a bug report — the 2026-07-29 incident was a /bug "from the native
        // AppImage" that turned out to be a month behind the fix, and nothing
        // in the report said so. The reader logs it as `build`.
        Tag::custom(TagKind::custom("build"), [env!("CARGO_PKG_VERSION").to_string()]),
    ];
    EventBuilder::new(Kind::PrivateDirectMessage, report.body.clone())
        .tags(tags)
        .build(sender)
}

/// Seal with the report's burner key + gift-wrap to the official pubkey.
/// rust-nostr backdates seal/wrap timestamps per NIP-59 (matches web).
pub async fn wrap_report(
    burner: &Keys,
    official: &PublicKey,
    rumor: UnsignedEvent,
) -> Result<Event, String> {
    EventBuilder::gift_wrap(burner, official, rumor, [])
        .await
        .map_err(|e| format!("gift wrap: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_mailbox::outbox::QueuedReport;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    fn report() -> QueuedReport {
        QueuedReport {
            id: "aa".repeat(16),
            kind: "bug".into(),
            body: "when making a skin there is overlap".into(),
            created_at: 1_784_900_000,
            status: "queued".into(),
        }
    }

    fn tag_value(ev: &UnsignedEvent, name: &str) -> Option<String> {
        ev.tags.iter().find_map(|t| {
            let v = t.as_slice();
            (v.first().map(String::as_str) == Some(name)).then(|| v.get(1).cloned()).flatten()
        })
    }

    #[test]
    fn rumor_carries_client_build_and_never_an_identity_tag() {
        let keys = Keys::generate();
        let r = build_rumor(keys.public_key(), &report());
        assert_eq!(r.kind.as_u16(), 14);
        assert_eq!(r.content, "when making a skin there is overlap");
        assert_eq!(tag_value(&r, "t").as_deref(), Some("bug"));
        assert_eq!(tag_value(&r, "report-id").as_deref(), Some("aa".repeat(16).as_str()));
        assert_eq!(tag_value(&r, "client").as_deref(), Some("axenstax-native"));
        assert_eq!(
            tag_value(&r, "build").as_deref(),
            Some(env!("CARGO_PKG_VERSION")),
            "every native report names the build that sent it"
        );
        for identity_tag in ["persona", "handle"] {
            assert!(tag_value(&r, identity_tag).is_none(), "no {identity_tag} tag (S5)");
        }
    }

    #[test]
    fn wrap_round_trips_to_the_official_key_with_the_burner_as_seal_author() {
        rt().block_on(async {
            let burner = Keys::generate();
            let maker = Keys::generate(); // stands in for the official key
            let rumor = build_rumor(burner.public_key(), &report());
            let out = wrap_report(&burner, &maker.public_key(), rumor).await.unwrap();
            let unwrapped = nostr::nips::nip59::extract_rumor(&maker, &out).await.unwrap();
            assert_eq!(unwrapped.rumor.content, "when making a skin there is overlap");
            assert_eq!(unwrapped.sender, burner.public_key(), "seal author = the burner");
        });
    }
}
