//! Kind-30063 Nostr release event — a second, server-less source for
//! `update_check`'s "what's the newest published build" question.
//!
//! NATIVE-ONLY, like the rest of the update-check machinery.
//!
//! ## Why this exists
//!
//! `update_check`'s only source used to be `docs.axenstax.org/download/
//! latest.json`, a single web host. Publishing to it goes over SSH to a box
//! whose sshd went unresponsive in 2026-09, leaving the manifest three
//! versions stale for days — the update path went inert even though the code
//! was fine. A release event on a Nostr relay needs no web server at all: the
//! event itself carries the version and the artefact's sha256/mirrors.
//!
//! ## Shape
//!
//! [`fetch_latest`] is a synchronous, blocking call, matching
//! `update_check::fetch_latest_version`'s shape exactly so `start_once` can
//! call the two back to back on the one worker thread it already spawns —
//! see that module's doc for why nothing here needs to touch the render
//! thread. Internally it builds its own current-thread tokio runtime, the
//! same "spawn a thread, build a runtime, block_on" idiom
//! `game_loop::native_join_sign_driver` and `native_mailbox`'s worker use —
//! except there is no nested-runtime hazard here (unlike `native_mailbox::
//! run_on_worker_thread`'s doc comment), because this function is always
//! called from a plain `std::thread::spawn` closure that has never entered a
//! tokio context, so building and blocking on a runtime directly is safe: no
//! extra thread hop needed.
//!
//! ## Trust
//!
//! An update channel is a way to hand somebody a binary. [`RELEASE_PUBKEY_HEX`]
//! is the one thing standing between "a signed release note" and "whatever
//! showed up on a relay" — [`parse_release_event`] verifies the signature AND
//! checks the author against it, and an event from any other key is dropped
//! whole, not partially trusted. The sha256 an event carries flows straight
//! into [`crate::update_check::AppImageRef`], and this signed event is the ONLY
//! authority `update_check::decide` accepts for the version, URL and sha256 of
//! an install — the unsigned HTTP manifest can only add a mirror URL held to
//! this signed hash (audit 2026-09-27).
//!
//! The pinned key here MUST equal `RELEASE_PUBKEY_HEX` in
//! `tools/release/release-helpers.mjs` (the publish side) — see that
//! constant's doc comment for the ceremony that minted it and the
//! never-drift-silently rule between the two copies.

use nostr::prelude::*;

use crate::update_check::{self, AppImageRef, Latest};

/// The release-signing key. **Must match `RELEASE_PUBKEY_HEX` in
/// `tools/release/release-helpers.mjs`** — that is the ONLY other place this
/// value is allowed to live; the two must never drift, so if this ever
/// changes, change that file in the same commit (and vice versa). Minted by
/// the release key ceremony (`tools/release/new-release-key.mjs`) and pinned
/// 2026-09-06 — npub1efp7wgvdqwjsug9s0a2044v20n6wlxkp4zp78gqsgf26gesqu5pqksl8py.
///
/// `None` disables this update source entirely: an empty/placeholder value
/// must never be read as "trust nothing" in a way that accidentally trusts
/// everything, so every call site below treats `None` as "don't even ask a
/// relay", not as a filter that happens to match zero events. Now that the
/// key exists, this stays `Some` permanently — the pin IS the whole security
/// property here (an update channel that accepts any author is a way to hand
/// somebody a binary), so an event from any other author is still ignored
/// entirely by `parse_release_event` below, unconditionally.
pub const RELEASE_PUBKEY_HEX: Option<&str> =
    Some("ca43e7218d03a50e20b07f54fad58a7cf4ef9ac1a883e3a0104255a46600e502");

/// The kind-30063 release-manifest event's `d` tag — one addressable event
/// per app, replaced on each release rather than accumulating a feed of them.
const RELEASE_D_TAG: &str = "axenstax-appimage";

const RELEASE_KIND: u16 = 30063;

// No relay list of its own: the release feed is read from the player's
// "Your relays" list (`GraphicsSettings.online_relays`), which defaults to
// `server_resolve::PUBLIC_DEFAULT_RELAYS`. `tools/release/` publishes to those
// same public defaults. A player who removes every public relay still learns of
// updates through the HTTPS check in `update_check` — there is deliberately no
// hidden fallback relay here.

/// Pull a `version` + [`AppImageRef`] out of one candidate kind-30063 event,
/// or `None` if it fails ANY check: wrong kind, bad signature, wrong `d` tag,
/// wrong author, or a missing/empty `version`/`x`/`url` tag. The signature
/// and author checks are NOT optional extras — `fetch_latest` also filters by
/// author server-side in the relay query, but a relay is not a trust
/// boundary, so this re-checks from scratch against the raw event exactly as
/// if the filter hadn't run.
///
/// `size` is read from the wire (per the event shape) but not carried further
/// — nothing downstream needs it, unlike `x` (verified in `self_update`) and
/// `url` (fetched in `self_update`).
fn parse_release_event(event: &Event, pinned_hex: &str) -> Option<Latest> {
    if event.kind.as_u16() != RELEASE_KIND {
        return None;
    }
    if event.verify().is_err() {
        return None;
    }
    if event.pubkey.to_hex() != pinned_hex {
        log::warn!(
            "[update] dropped release event from non-pinned author {}…",
            &event.pubkey.to_hex()[..12.min(event.pubkey.to_hex().len())]
        );
        return None;
    }
    if event.tags.identifier() != Some(RELEASE_D_TAG) {
        return None;
    }

    let tag_value = |name: &str| -> Option<String> {
        event.tags.iter().find_map(|t| {
            let v = t.as_slice();
            (v.first().map(String::as_str) == Some(name)).then(|| v.get(1).cloned()).flatten()
        })
    };

    let version = tag_value("version").filter(|v| !v.is_empty())?;
    let sha256 = update_check::normalize_sha256(&tag_value("x")?)?;
    // First `url` tag wins. The tag is repeatable (mirrors), but nothing
    // downstream of `Latest` (self_update) fetches more than one URL, so
    // trying additional mirrors on a failed download is future work, not a
    // regression here — today's HTTP path only ever carried one URL either.
    let url = tag_value("url").filter(|v| !v.is_empty())?;
    let filename = url.rsplit('/').next().filter(|s| !s.is_empty())?.to_string();
    // `self_update` builds its `.part` path from this name: an unsafe one is
    // refused outright (no install), even from the signed event.
    if !update_check::is_valid_appimage_filename(&filename) {
        log::warn!("[update] release event names an unsafe AppImage filename; ignoring it");
        return None;
    }

    Some(Latest {
        version,
        appimage: Some(AppImageRef { filename, url, sha256, mirrors: Vec::new() }),
    })
}

/// OWNER BOUNDARY (live relay). Open a one-shot subscription on `url` for
/// kind-30063 events from `pinned_hex` with `d = axenstax-appimage`,
/// collecting events until EOSE or an 8s cap. Mirrors
/// `native_mailbox::relay::fetch_events`'s shape; not reused directly because
/// that module's `Frame::Wrap` is gift-wrap-shaped (kind 1059) and naming a
/// release event a "wrap" would be misleading.
async fn fetch_release_events(url: &str, pinned_hex: &str) -> Result<Vec<Event>, String> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let (ws, _) =
        tokio_tungstenite::connect_async(url).await.map_err(|e| format!("connect {url}: {e}"))?;
    let (mut sink, mut stream) = ws.split();
    let sub_id = "rel";
    let req = serde_json::json!(["REQ", sub_id, {
        "kinds": [RELEASE_KIND],
        "authors": [pinned_hex],
        "#d": [RELEASE_D_TAG],
        "limit": 10,
    }])
    .to_string();
    sink.send(Message::Text(req.into())).await.map_err(|e| format!("send: {e}"))?;

    let mut out = Vec::new();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(8), async {
        while let Some(Ok(msg)) = stream.next().await {
            let Message::Text(t) = msg else { continue };
            let Ok(serde_json::Value::Array(arr)) = serde_json::from_str::<serde_json::Value>(&t)
            else {
                continue;
            };
            match arr.first().and_then(|x| x.as_str()) {
                Some("EVENT") if arr.get(1).and_then(|x| x.as_str()) == Some(sub_id) => {
                    if let Some(Ok(ev)) = arr.get(2).map(|v| serde_json::from_value::<Event>(v.clone())) {
                        out.push(ev);
                    }
                }
                Some("EOSE") if arr.get(1).and_then(|x| x.as_str()) == Some(sub_id) => break,
                _ => {}
            }
        }
    })
    .await;
    Ok(out)
}

/// Query every relay in `relays` for the pinned author's release event, verify every
/// candidate, and keep the one with the latest `created_at` (the addressable-
/// event replacement rule — a relay serving a stale copy loses to a fresher
/// one from another relay). `None` when the source is disabled
/// ([`RELEASE_PUBKEY_HEX`] is `None`), every relay is unreachable, or nothing
/// verifiable came back — same "absent, not current" contract as
/// `update_check::fetch_latest_version`.
async fn fetch_latest_async(pinned_hex: &str, relays: &[String]) -> Option<Latest> {
    let mut best: Option<Event> = None;
    for url in relays {
        let url = url.as_str();
        match fetch_release_events(url, pinned_hex).await {
            Ok(events) => {
                for ev in events {
                    if ev.verify().is_err() || ev.pubkey.to_hex() != pinned_hex {
                        continue;
                    }
                    if best.as_ref().is_none_or(|b| ev.created_at > b.created_at) {
                        best = Some(ev);
                    }
                }
            }
            Err(e) => log::debug!("[update] relay {url} unreachable for release event: {e}"),
        }
    }
    best.and_then(|ev| parse_release_event(&ev, pinned_hex))
}

/// Synchronous entry point — see the module doc for why building a runtime
/// directly here (rather than hopping to another thread first) is safe.
/// Returns `None` immediately, with no network call at all, if
/// [`RELEASE_PUBKEY_HEX`] is ever reverted to `None` (e.g. a build with the
/// key deliberately stripped) — this source is disabled by construction
/// rather than by a runtime check, so there is no way to half-enable it.
pub fn fetch_latest(relays: &[String]) -> Option<Latest> {
    let pinned_hex = RELEASE_PUBKEY_HEX?;
    if relays.is_empty() {
        return None;
    }
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
    rt.block_on(fetch_latest_async(pinned_hex, relays))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_SHA: &str = "988cb00c22c80df26bb4c3a5281f8ab5e36cf00c006798a3593b41149457b794";
    const _: () = assert!(REAL_SHA.len() == 64);

    /// Builds a signed kind-30063 event with the given tag values. Any of
    /// `sha`/`url` can be `""` to omit that tag entirely, for the
    /// missing-field tests.
    fn release_event(keys: &Keys, version: &str, sha: &str, url: &str) -> Event {
        let mut tags = vec![Tag::parse(["d", RELEASE_D_TAG]).unwrap()];
        if !version.is_empty() {
            tags.push(Tag::parse(["version", version]).unwrap());
        }
        if !sha.is_empty() {
            tags.push(Tag::parse(["x", sha]).unwrap());
        }
        tags.push(Tag::parse(["size", "123456"]).unwrap());
        if !url.is_empty() {
            tags.push(Tag::parse(["url", url]).unwrap());
        }
        EventBuilder::new(Kind::Custom(RELEASE_KIND), "release notes")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    #[test]
    fn a_well_formed_event_from_the_pinned_author_parses() {
        let keys = Keys::generate();
        let pinned = keys.public_key().to_hex();
        let url = "https://mirror.example/axenstax-engine_0.3.0_x86_64.AppImage";
        let ev = release_event(&keys, "0.3.0", REAL_SHA, url);

        let latest = parse_release_event(&ev, &pinned).expect("parses");
        assert_eq!(latest.version, "0.3.0");
        let r = latest.appimage.expect("appimage ref present");
        assert_eq!(r.sha256, REAL_SHA);
        assert_eq!(r.url, url);
        assert_eq!(r.filename, "axenstax-engine_0.3.0_x86_64.AppImage");
    }

    #[test]
    fn an_event_from_the_wrong_author_is_rejected() {
        let signer = Keys::generate();
        let pinned = Keys::generate().public_key().to_hex(); // a DIFFERENT key
        let ev = release_event(&signer, "0.3.0", REAL_SHA, "https://mirror.example/a.AppImage");
        assert_eq!(parse_release_event(&ev, &pinned), None);
    }

    #[test]
    fn an_event_missing_the_sha256_tag_is_rejected() {
        let keys = Keys::generate();
        let pinned = keys.public_key().to_hex();
        let ev = release_event(&keys, "0.3.0", "", "https://mirror.example/a.AppImage");
        assert_eq!(parse_release_event(&ev, &pinned), None);
    }

    #[test]
    fn an_event_missing_the_url_tag_is_rejected() {
        let keys = Keys::generate();
        let pinned = keys.public_key().to_hex();
        let ev = release_event(&keys, "0.3.0", REAL_SHA, "");
        assert_eq!(parse_release_event(&ev, &pinned), None);
    }

    #[test]
    fn a_malformed_sha256_is_rejected_same_as_the_http_path() {
        let keys = Keys::generate();
        let pinned = keys.public_key().to_hex();
        let ev = release_event(&keys, "0.3.0", "not-a-sha", "https://mirror.example/a.AppImage");
        assert_eq!(parse_release_event(&ev, &pinned), None);
    }

    #[test]
    fn a_tampered_event_fails_signature_verification() {
        let keys = Keys::generate();
        let pinned = keys.public_key().to_hex();
        let mut ev = release_event(&keys, "0.3.0", REAL_SHA, "https://mirror.example/a.AppImage");
        // Flip the content after signing — same id, now-invalid signature.
        ev.content = "tampered".to_string();
        assert_eq!(parse_release_event(&ev, &pinned), None);
    }

    #[test]
    fn a_wrong_d_tag_is_rejected() {
        let keys = Keys::generate();
        let pinned = keys.public_key().to_hex();
        let tags = vec![
            Tag::parse(["d", "some-other-app"]).unwrap(),
            Tag::parse(["version", "0.3.0"]).unwrap(),
            Tag::parse(["x", REAL_SHA]).unwrap(),
            Tag::parse(["url", "https://mirror.example/a.AppImage"]).unwrap(),
        ];
        let ev = EventBuilder::new(Kind::Custom(RELEASE_KIND), "")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(parse_release_event(&ev, &pinned), None);
    }

    /// The release key ceremony has been run and the pubkey pinned — this
    /// source is live. Pins the exact value against silent drift as much as a
    /// same-repo test can: the OTHER copy lives in a separate JS runtime
    /// (`tools/release/release-helpers.mjs`) that this test cannot import, so
    /// this only catches an accidental edit here, not a mismatch between the
    /// two files — that's on the "must equal" doc comments on both constants.
    #[test]
    fn release_pubkey_hex_is_pinned_to_the_minted_release_key() {
        assert_eq!(
            RELEASE_PUBKEY_HEX,
            Some("ca43e7218d03a50e20b07f54fad58a7cf4ef9ac1a883e3a0104255a46600e502")
        );
    }


    #[test]
    fn an_event_naming_an_unsafe_filename_is_rejected() {
        let keys = Keys::generate();
        let pinned = keys.public_key().to_hex();
        for url in ["https://mirror.example/payload.sh", "https://mirror.example/a%2F..%2Fb.AppImage"] {
            let ev = release_event(&keys, "0.3.0", REAL_SHA, url);
            assert_eq!(parse_release_event(&ev, &pinned), None, "{url}");
        }
    }
}
