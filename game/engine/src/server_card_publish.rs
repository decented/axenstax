#![cfg(not(target_arch = "wasm32"))]
#![allow(dead_code)] // wired from server_main on the announce path
//! Native opt-in Server Card publisher (Spec A task 9).
//!
//! Builds the Address Card from server config and publishes it to relays,
//! runtime-signed (no bunker). Announcing is **opt-in** (`--announce`); a server
//! that isn't announced publishes nothing. **Owner boundary:** the live relay
//! publish needs a real relay.

use crate::server_identity::server_card::{build_card_event, ServerCard};
use crate::server_identity::store::ServerIdentity;


/// Build the Address Card from server config. `tls` ⇒ `wss://` (Caddy-fronted),
/// else `ws://`; the dedicated server's socket path is `/ws`.
#[allow(clippy::too_many_arguments)] // each field is a distinct card attribute
pub fn build_card_from_config(
    public_host: &str,
    port: u16,
    tls: bool,
    name: &str,
    about: &str,
    region: &str,
    players_cur: u16,
    players_max: u16,
    protocol: u32,
    privacy: &str,
) -> ServerCard {
    let scheme = if tls { "wss" } else { "ws" };
    ServerCard {
        endpoints: vec![format!("{scheme}://{public_host}:{port}/ws")],
        name: name.to_string(),
        about: about.to_string(),
        region: region.to_string(),
        players_cur,
        players_max,
        protocol,
        privacy: privacy.to_string(),
    }
}

/// OWNER BOUNDARY (live relay). Build + sign the card with the runtime key and
/// publish it to each relay. Returns how many relays accepted it. `0` (and a
/// warning) if the server is unprovisioned (no card to sign).
pub async fn publish_card(id: &ServerIdentity, card: &ServerCard, relays: &[String]) -> usize {
    let Some(event) = build_card_event(id, card) else {
        log::warn!("card publish: server is unprovisioned — nothing to publish");
        return 0;
    };
    use nostr::JsonUtil;
    let json = event.as_json();
    let mut sent = 0usize;
    for relay in relays {
        match crate::server_identity::admin_relay::publish_event(relay, &json).await {
            Ok(()) => {
                sent += 1;
                log::info!("card publish: announced on {relay}");
            }
            Err(e) => log::warn!("card publish: {relay} failed: {e}"),
        }
    }
    sent
}

/// Whether the heartbeat should (keep) announcing. Announcing is opt-in: the
/// `--announce` CLI/env flag forces it on for the run; otherwise it follows the
/// operator's console setting, and `announce false` turns it fully off.
pub fn should_announce(cli_forced: bool, settings_announce: bool) -> bool {
    cli_forced || settings_announce
}

/// OWNER BOUNDARY (live relay). Publish a NIP-09 deletion of this server's card
/// to each relay, so turning announce off withdraws it rather than leaving the
/// last card discoverable. Returns how many relays accepted it.
pub async fn retract_card(id: &ServerIdentity, relays: &[String]) -> usize {
    let Some(event) = crate::server_identity::server_card::build_card_delete_event(id) else {
        return 0;
    };
    use nostr::JsonUtil;
    let json = event.as_json();
    let mut sent = 0usize;
    for relay in relays {
        match crate::server_identity::admin_relay::publish_event(relay, &json).await {
            Ok(()) => {
                sent += 1;
                log::info!("card publish: retracted on {relay}");
            }
            Err(e) => log::warn!("card publish: retract on {relay} failed: {e}"),
        }
    }
    sent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_endpoint_is_wss_with_ws_path() {
        let c = build_card_from_config(
            "play.example.com",
            8080,
            true,
            "Cool SMP",
            "no griefing",
            "eu-west",
            0,
            20,
            50,
            "none",
        );
        assert_eq!(c.endpoints, vec!["wss://play.example.com:8080/ws".to_string()]);
        assert_eq!(c.name, "Cool SMP");
        assert_eq!(c.players_max, 20);
        assert_eq!(c.protocol, 50);
        assert_eq!(c.privacy, "none");
    }

    #[test]
    fn announce_is_opt_in_and_setting_can_turn_it_off() {
        assert!(!should_announce(false, false), "off by default / `announce false`");
        assert!(should_announce(false, true));
        assert!(should_announce(true, false), "CLI flag forces it on for the run");
    }

    #[test]
    fn bare_endpoint_is_ws() {
        let c = build_card_from_config("10.0.0.5", 7700, false, "S", "", "", 3, 8, 49, "sessions");
        assert_eq!(c.endpoints, vec!["ws://10.0.0.5:7700/ws".to_string()]);
        assert_eq!(c.players_cur, 3);
        assert_eq!(c.privacy, "sessions");
    }
}
