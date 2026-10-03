#![cfg(not(target_arch = "wasm32"))]
#![allow(dead_code)] // consumed by the operator snapshot-stream (task 7)
//! The Operator Console read-model (Spec B). A serde snapshot the server builds
//! from its live state + settings + telemetry, streamed to the verified-operator
//! player (task 7). Pure assembly; no I/O. npub fields are bech32 (`npub-only
//! display rule`).

use serde::{Deserialize, Serialize};

use crate::console_settings::ConsoleSettings;
use crate::console_telemetry::TelemetryAggregates;

/// One connected player, for the live roster.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerRow {
    pub npub: String,
    pub handle: String,
    pub connected_secs: u64,
}

/// Everything the Operator panel renders, in one serialisable read-model.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleSnapshot {
    pub server_name: String,
    pub players_cur: u16,
    pub players_max: u16,
    pub roster: Vec<PlayerRow>,
    pub unique_today: u32,
    pub peak_today: u16,
    pub total_sessions: u64,
    pub announce: bool,
    pub privacy_level: String,
    pub require_signin: bool,
    pub allowlist_count: u32,
    pub blocklist_count: u32,
    pub operator_npub: String,
    pub runtime_npub: String,
    pub delegation_expires_unix: u64,
    pub protocol: u32,
}

/// Assemble a snapshot from the server's live inputs. Pure.
/// `policy` = `(require_signin, allowlist_count, blocklist_count)`.
/// `ident` = `(operator_npub, runtime_npub, delegation_expires_unix)`.
pub fn build_snapshot(
    settings: &ConsoleSettings,
    roster: &[PlayerRow],
    agg: &TelemetryAggregates,
    policy: (bool, u32, u32),
    ident: (String, String, u64),
) -> ConsoleSnapshot {
    ConsoleSnapshot {
        server_name: settings.server_name.clone().unwrap_or_default(),
        players_cur: roster.len() as u16,
        players_max: settings.max_players.unwrap_or(0),
        roster: roster.to_vec(),
        unique_today: agg.unique_today,
        peak_today: agg.peak_today,
        total_sessions: agg.total_sessions,
        announce: settings.announce,
        privacy_level: settings.privacy_level.clone(),
        require_signin: policy.0,
        allowlist_count: policy.1,
        blocklist_count: policy.2,
        operator_npub: ident.0,
        runtime_npub: ident.1,
        delegation_expires_unix: ident.2,
        protocol: crate::protocol::PROTOCOL_VERSION,
    }
}

/// Whether a connected player IS the server operator — both identities present
/// and equal. The gate for the operator snapshot-stream (Spec B task 7).
pub fn is_operator(player_pubkey: Option<[u8; 32]>, operator: Option<[u8; 32]>) -> bool {
    matches!((player_pubkey, operator), (Some(p), Some(o)) if p == o)
}

/// bech32 npub for a verified pubkey (the `npub-only display rule`); hex fallback.
fn npub_of(pubkey: &[u8; 32]) -> String {
    use nostr::ToBech32;
    nostr::PublicKey::from_slice(pubkey)
        .ok()
        .and_then(|pk| pk.to_bech32().ok())
        .unwrap_or_else(|| hex::encode(pubkey))
}

/// Build the connected-player roster the snapshot lists, from parallel server
/// slot state (`verified`/`handles`/`handshake_done`/`disconnected`, indexed by
/// slot). Includes only handshake-complete, non-disconnected slots; a verified
/// slot shows its npub, a guest an empty npub. `connected_secs` is 0 for now
/// (per-slot connect time is a follow-up).
pub fn roster_rows(
    verified: &[Option<[u8; 32]>],
    handles: &[String],
    handshake_done: &[bool],
    disconnected: &[bool],
) -> Vec<PlayerRow> {
    (0..handles.len())
        .filter(|&i| {
            handshake_done.get(i).copied().unwrap_or(false)
                && !disconnected.get(i).copied().unwrap_or(true)
        })
        .map(|i| PlayerRow {
            npub: verified.get(i).copied().flatten().map(|pk| npub_of(&pk)).unwrap_or_default(),
            handle: handles.get(i).cloned().unwrap_or_default(),
            connected_secs: 0,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roster_rows_includes_only_connected_handshaken_slots() {
        let verified = vec![Some([9u8; 32]), None, Some([1u8; 32])];
        let handles = vec!["Op".to_string(), "Guest".to_string(), "Gone".to_string()];

        // Slot 2 disconnected → excluded; verified → npub, guest → empty npub.
        let rows = roster_rows(&verified, &handles, &[true, true, true], &[false, false, true]);
        assert_eq!(rows.len(), 2, "disconnected slot excluded");
        assert_eq!(rows[0].handle, "Op");
        assert!(rows[0].npub.starts_with("npub1"), "verified slot → npub");
        assert_eq!(rows[1].handle, "Guest");
        assert_eq!(rows[1].npub, "", "guest slot → empty npub");

        // Slot 0 not handshake-done → excluded.
        let rows2 = roster_rows(&verified, &handles, &[false, true, true], &[false, false, false]);
        assert_eq!(rows2.len(), 2, "only handshake-complete slots");
        assert_eq!(rows2[0].handle, "Guest");
    }

    #[test]
    fn build_snapshot_maps_inputs() {
        let settings = ConsoleSettings {
            server_name: Some("Cool SMP".into()),
            max_players: Some(20),
            announce: true,
            privacy_level: "sessions".into(),
            ..Default::default()
        };
        let roster = vec![
            PlayerRow {
                npub: "npub1a".into(),
                handle: "Axo".into(),
                connected_secs: 120,
            },
            PlayerRow {
                npub: "npub1b".into(),
                handle: "Stax".into(),
                connected_secs: 30,
            },
        ];
        let agg = TelemetryAggregates {
            unique_today: 5,
            peak_today: 3,
            total_sessions: 42,
        };
        let snap = build_snapshot(
            &settings,
            &roster,
            &agg,
            (true, 4, 1),
            ("npub1op".into(), "npub1rt".into(), 1_900_000_000),
        );
        assert_eq!(snap.server_name, "Cool SMP");
        assert_eq!(snap.players_cur, 2);
        assert_eq!(snap.players_max, 20);
        assert_eq!(snap.roster.len(), 2);
        assert_eq!(snap.unique_today, 5);
        assert_eq!(snap.peak_today, 3);
        assert_eq!(snap.total_sessions, 42);
        assert!(snap.announce);
        assert_eq!(snap.privacy_level, "sessions");
        assert!(snap.require_signin);
        assert_eq!(snap.allowlist_count, 4);
        assert_eq!(snap.blocklist_count, 1);
        assert_eq!(snap.operator_npub, "npub1op");
        assert_eq!(snap.runtime_npub, "npub1rt");
        assert_eq!(snap.delegation_expires_unix, 1_900_000_000);
        assert_eq!(snap.protocol, crate::protocol::PROTOCOL_VERSION);
    }

    #[test]
    fn snapshot_json_round_trips() {
        let snap = ConsoleSnapshot {
            server_name: "X".into(),
            players_cur: 1,
            players_max: 8,
            roster: vec![PlayerRow {
                npub: "npub1a".into(),
                handle: "A".into(),
                connected_secs: 10,
            }],
            unique_today: 1,
            peak_today: 1,
            total_sessions: 1,
            announce: false,
            privacy_level: "none".into(),
            require_signin: false,
            allowlist_count: 0,
            blocklist_count: 0,
            operator_npub: "npub1op".into(),
            runtime_npub: "npub1rt".into(),
            delegation_expires_unix: 0,
            protocol: crate::protocol::PROTOCOL_VERSION,
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: ConsoleSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snap, back);
    }

    #[test]
    fn is_operator_requires_both_present_and_equal() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        assert!(is_operator(Some(a), Some(a)));
        assert!(!is_operator(Some(a), Some(b)));
        assert!(!is_operator(None, Some(a)));
        assert!(!is_operator(Some(a), None));
        assert!(!is_operator(None, None));
    }

    #[test]
    fn snapshot_packet_round_trips_via_protocol() {
        use crate::protocol;
        let snap = ConsoleSnapshot {
            server_name: "X".into(),
            players_cur: 1,
            ..Default::default()
        };
        let pkt = protocol::OperatorSnapshotPacket {
            snapshot_json: serde_json::to_string(&snap).unwrap(),
        };
        let bytes = protocol::serialize_packet(protocol::PacketType::OperatorSnapshot, &pkt);
        let (ptype, payload) = protocol::deserialize_header(&bytes).unwrap();
        assert!(matches!(ptype, protocol::PacketType::OperatorSnapshot));
        let back: protocol::OperatorSnapshotPacket = protocol::safe_deserialize(payload).unwrap();
        let decoded: ConsoleSnapshot = serde_json::from_str(&back.snapshot_json).unwrap();
        assert_eq!(decoded.server_name, "X");
        assert_eq!(decoded.players_cur, 1);
    }
}
