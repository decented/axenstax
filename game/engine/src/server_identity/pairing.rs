#![cfg(not(target_arch = "wasm32"))]
//! Pairing driver + boot gate.
//!
//! `pair_server` / `refresh_delegation` are the **live** NIP-46 flows (they reach
//! the operator's Heartwood over a relay) — exercised at the owner boundary, not
//! in CI. `startup_gate` / `identity_dir` are pure and unit-tested.

use std::path::{Path, PathBuf};

use nostr::{Event, Timestamp, ToBech32};
use signet_nip46_client::{BunkerSession, PersistedSession, SessionOptions};

use crate::server_identity::attestation::mint_attestation;
use crate::server_identity::store::{generate_runtime, load, write_0600, ServerIdentity};

/// Default relay for the operator's NIP-46 pairing round-trip (and, through
/// `server_main::admin_relay_url`, the admin-command relay): the first public
/// default. Overridable with `--pair-relay` / `AXENSTAX_PAIR_RELAY`. No
/// AxeNStax-operated relay is a default anywhere (CLAUDE.md red line 2).
pub const DEFAULT_PAIR_RELAY: &str = crate::server_resolve::PUBLIC_DEFAULT_RELAYS[0];
/// Default attestation validity window minted at pairing time.
pub const DEFAULT_DELEGATION_DAYS: u64 = 90;
/// Event kinds the runtime key is authorised to sign: join-challenge (27420)
/// and authoritative claims (27421). Provisional; documented in the spec.
pub const DEFAULT_KINDS: [u16; 2] = [27420, 27421];
/// App name shown to the operator's signer during pairing.
const APP_NAME: &str = "Axe'n'Stax Server";

/// The identity directory under a worlds dir (`<worlds>/.identity`).
pub fn identity_dir(worlds_dir: &str) -> PathBuf {
    Path::new(worlds_dir).join(".identity")
}

/// Boot gate: with `require_verified`, refuse to start unless a valid attestation
/// is present at `now`. Otherwise always `Ok` (additive enforcement).
pub fn startup_gate(
    id: Option<&ServerIdentity>,
    require_verified: bool,
    now: Timestamp,
) -> Result<(), String> {
    if require_verified && !id.map(|i| i.is_verified(now)).unwrap_or(false) {
        return Err(
            "AXENSTAX_REQUIRE_VERIFIED is set but no valid attestation is present \
             (run --pair-server to provision one)"
                .to_string(),
        );
    }
    Ok(())
}

fn session_path(dir: &Path) -> PathBuf {
    dir.join("session.json")
}

fn save_session(dir: &Path, p: &PersistedSession) -> Result<(), String> {
    let bytes = serde_json::to_vec(p).map_err(|e| e.to_string())?;
    write_0600(&session_path(dir), &bytes)
}

fn load_session(dir: &Path) -> Result<Option<PersistedSession>, String> {
    match std::fs::read(session_path(dir)) {
        Ok(b) => serde_json::from_slice(&b).map(Some).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn delegation_window(validity_days: u64) -> (Timestamp, Timestamp) {
    let now = Timestamp::now();
    let until = Timestamp::from(now.as_secs() + validity_days.saturating_mul(86_400));
    (now, until)
}

/// Render a `nostrconnect://` URI as a terminal QR (best-effort; the raw URI is
/// always printed too). Half-block rows keep it compact; a 2-module quiet zone
/// frames it. Mirrors the colour-grid approach in `menu.rs`.
fn print_qr(data: &str) {
    let code = match qrcode::QrCode::new(data.as_bytes()) {
        Ok(c) => c,
        Err(_) => {
            println!("(could not render a QR — use the URI below)");
            return;
        }
    };
    let colors = code.to_colors();
    let n = (colors.len() as f64).sqrt() as usize;
    let dark = |x: usize, y: usize| x < n && y < n && matches!(colors[y * n + x], qrcode::Color::Dark);
    let quiet = 2usize;
    let total = n + quiet * 2;
    let mut y = 0;
    while y < total {
        let mut line = String::new();
        for x in 0..total {
            let top = dark(x.wrapping_sub(quiet), y.wrapping_sub(quiet));
            let bottom = dark(x.wrapping_sub(quiet), (y + 1).wrapping_sub(quiet));
            line.push(match (top, bottom) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        println!("{line}");
        y += 2;
    }
}

/// Mint a fresh attestation from a live bunker session and persist it + the
/// session. Shared by `pair_server` and `refresh_delegation`. Returns the
/// operator npub.
async fn mint_and_store(
    dir: &Path,
    id: &mut ServerIdentity,
    session: BunkerSession,
    name: &str,
    host: Option<&str>,
    validity_days: u64,
) -> Result<String, String> {
    let operator = session.user_public_key().await.map_err(|e| e.to_string())?;
    let (from, until) = delegation_window(validity_days);
    let ev = mint_attestation(
        &session,
        &id.runtime_pubkey(),
        from,
        until,
        &DEFAULT_KINDS,
        name,
        host,
    )
    .await?;
    id.store_attestation(dir, ev)?;
    let persisted = session.persist().await.map_err(|e| e.to_string())?;
    save_session(dir, &persisted)?;
    session.shutdown().await;
    operator.to_bech32().map_err(|e| e.to_string())
}

/// Interactive first-time pairing: generate a runtime key (if needed), show a
/// `nostrconnect://` QR for the operator's Heartwood, mint + store the
/// attestation on approval. Returns the operator npub. **Live** — owner boundary.
pub async fn pair_server(
    dir: &Path,
    relay: &str,
    name: &str,
    host: Option<&str>,
    validity_days: u64,
) -> Result<String, String> {
    let mut id = match load(dir)? {
        Some(i) => i,
        None => generate_runtime(dir)?,
    };
    let (uri, session) =
        BunkerSession::pair_nostrconnect([relay.to_string()], APP_NAME, SessionOptions::default())
            .map_err(|e| e.to_string())?;
    println!("\nPair this server with your Heartwood — scan the QR or paste the URI into your signer:\n");
    print_qr(&uri);
    println!("\n{uri}\n");
    println!("Waiting for approval on your Heartwood…");
    mint_and_store(dir, &mut id, session, name, host, validity_days).await
}

/// Silent renewal via the stored bunker session — no fresh approval if the grant
/// is still valid. Falls back to an error telling the operator to re-pair.
/// **Live** — owner boundary.
pub async fn refresh_delegation(
    dir: &Path,
    _relay: &str,
    name: &str,
    host: Option<&str>,
    validity_days: u64,
) -> Result<String, String> {
    let mut id = load(dir)?.ok_or_else(|| "not provisioned — run --pair-server first".to_string())?;
    let persisted = load_session(dir)?
        .ok_or_else(|| "no saved bunker session — run --pair-server".to_string())?;
    let session =
        BunkerSession::restore(persisted, SessionOptions::default()).map_err(|e| e.to_string())?;
    mint_and_store(dir, &mut id, session, name, host, validity_days).await
}

/// Sign an admin command (C2) with the operator's restored bunker session —
/// reuses the session saved at pairing time, so no fresh approval if the grant is
/// still valid. **Live** — owner boundary. The signed event is what
/// `admin_relay::publish_event` posts to the relay.
pub async fn sign_admin_command_via_session(
    dir: &Path,
    verb: &str,
    arg: &str,
) -> Result<Event, String> {
    // The command is audience-bound to THIS server's runtime key (Spec 08
    // §9.0.1): the identity lives in the same dir as the saved session.
    let server = load(dir)?
        .ok_or_else(|| "not provisioned — run --pair-server first".to_string())?
        .runtime_pubkey();
    let persisted = load_session(dir)?
        .ok_or_else(|| "no saved bunker session — run --pair-server first".to_string())?;
    let session =
        BunkerSession::restore(persisted, SessionOptions::default()).map_err(|e| e.to_string())?;
    let ev = crate::server_identity::admin::sign_admin_command(&session, verb, arg, &server).await?;
    session.shutdown().await;
    Ok(ev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_allows_anonymous_when_not_required() {
        assert!(startup_gate(None, false, Timestamp::from(1)).is_ok());
    }

    #[test]
    fn gate_blocks_anonymous_when_required() {
        assert!(startup_gate(None, true, Timestamp::from(1)).is_err());
    }

    #[test]
    fn identity_dir_is_under_worlds() {
        assert_eq!(identity_dir("/worlds"), PathBuf::from("/worlds/.identity"));
    }
}
