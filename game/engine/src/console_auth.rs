#![cfg(not(target_arch = "wasm32"))]
//! Operator console login auth (Spec B). A pure verify primitive: the operator
//! signs a Nostr event over a server-issued nonce; this checks the signature,
//! that it is THE operator, that the nonce matches, and that it is fresh.
//!
//! A future web/HTTP console (or a sidecar) calls this — the engine needs no HTTP
//! server to build or test it (amendment B-0). Modelled on
//! `server_identity::admin::verify_admin_command`.

use nostr::{Event, EventBuilder, JsonUtil, Kind, NostrSigner, Tag, Timestamp};

/// Console-login event kind (provisional — register with the other
/// server-identity kinds in `forgesworn/nips`).
#[allow(dead_code)] // operator-console login primitive; its caller (the console/sidecar) is not built — specs 2026-06-17/18-operator-console
pub const CONSOLE_LOGIN_KIND: u16 = 27423;

/// Sign a console-login proof over `nonce` (the operator's bunker, or a local
/// `Keys` in tests).
///
/// WARNING (audit review B, N4): the event names no audience/host/channel, so a
/// relaying host could harvest a console login for another server. Before this
/// gets a live caller, bind it like the join event (`signet::join_origin`).
#[allow(dead_code)] // operator-console login primitive; its caller (the console/sidecar) is not built — specs 2026-06-17/18-operator-console
pub async fn sign_console_login<S: NostrSigner>(signer: &S, nonce: &str) -> Result<Event, String> {
    let tag = Tag::parse(["nonce", nonce]).map_err(|e| e.to_string())?;
    EventBuilder::new(Kind::Custom(CONSOLE_LOGIN_KIND), "")
        .tags([tag])
        .sign(signer)
        .await
        .map_err(|e| e.to_string())
}

/// Verify a console-login proof: a valid signature by `expected_operator_hex`,
/// the right kind, a `nonce` tag matching the server-issued nonce, and a
/// `created_at` fresh within `max_skew_secs`.
#[allow(dead_code)] // operator-console login primitive; its caller (the console/sidecar) is not built — specs 2026-06-17/18-operator-console
pub fn verify_console_login(
    signed_event_json: &str,
    expected_operator_hex: &str,
    nonce: &str,
    now: Timestamp,
    max_skew_secs: u64,
) -> Result<(), String> {
    let event =
        Event::from_json(signed_event_json).map_err(|e| format!("malformed event: {e}"))?;
    if event.verify().is_err() {
        return Err("bad signature".to_string());
    }
    if event.kind != Kind::Custom(CONSOLE_LOGIN_KIND) {
        return Err("wrong event kind".to_string());
    }
    if event.pubkey.to_hex() != expected_operator_hex {
        return Err("not the operator".to_string());
    }
    let nonce_ok = event.tags.iter().any(|t| {
        let s = t.as_slice();
        s.len() >= 2 && s[0] == "nonce" && s[1] == nonce
    });
    if !nonce_ok {
        return Err("nonce mismatch".to_string());
    }
    if event.created_at.as_secs().abs_diff(now.as_secs()) > max_skew_secs {
        return Err("stale login".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    #[tokio::test]
    async fn valid_login_verifies() {
        let op = Keys::generate();
        let ev = sign_console_login(&op, "nonce123").await.unwrap();
        let op_hex = op.public_key().to_hex();
        assert!(
            verify_console_login(&ev.as_json(), &op_hex, "nonce123", ev.created_at, 300).is_ok()
        );
    }

    #[tokio::test]
    async fn wrong_key_rejected() {
        let op = Keys::generate();
        let ev = sign_console_login(&op, "nonce123").await.unwrap();
        let other_hex = Keys::generate().public_key().to_hex();
        assert!(
            verify_console_login(&ev.as_json(), &other_hex, "nonce123", ev.created_at, 300).is_err()
        );
    }

    #[tokio::test]
    async fn wrong_nonce_rejected() {
        let op = Keys::generate();
        let ev = sign_console_login(&op, "nonce123").await.unwrap();
        let op_hex = op.public_key().to_hex();
        assert!(
            verify_console_login(&ev.as_json(), &op_hex, "WRONG", ev.created_at, 300).is_err()
        );
    }

    #[tokio::test]
    async fn stale_login_rejected() {
        let op = Keys::generate();
        let ev = sign_console_login(&op, "nonce123").await.unwrap();
        let op_hex = op.public_key().to_hex();
        let far = Timestamp::from(ev.created_at.as_secs() + 1000);
        assert!(verify_console_login(&ev.as_json(), &op_hex, "nonce123", far, 300).is_err());
    }

    #[tokio::test]
    async fn tampered_event_rejected() {
        let op = Keys::generate();
        let mut ev = sign_console_login(&op, "nonce123").await.unwrap();
        let created = ev.created_at;
        ev.content = "tampered".to_string();
        let op_hex = op.public_key().to_hex();
        assert!(verify_console_login(&ev.as_json(), &op_hex, "nonce123", created, 300).is_err());
    }
}
