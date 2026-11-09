#![cfg(not(target_arch = "wasm32"))]
//! The Operator Console read-model (Spec B). A serde snapshot the server builds
//! from its live state + settings + telemetry, streamed to the verified-operator
//! player (task 7). Pure assembly; no I/O. npub fields are bech32 (`npub-only
//! display rule`).

use serde::{Deserialize, Serialize};

use crate::access_policy::Allow;
use crate::console_settings::ConsoleSettings;
use crate::console_telemetry::TelemetryAggregates;

/// One connected player, for the live roster.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerRow {
    pub npub: String,
    pub handle: String,
    pub connected_secs: u64,
}

/// Living world S5 (spec section 5) — world bloom as a dedicated server sees it,
/// as whole thousandths (so the snapshot stays `Eq`): the mean level and its
/// two measured terms over the regions round its present players.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BloomFigures {
    /// Regions the means are over; 0 means none has been measured yet.
    pub regions: u32,
    pub level_permille: u16,
    /// Life: the mean eco term.
    pub life_permille: u16,
    /// People together: the mean people term.
    pub people_permille: u16,
    /// Ideas (S9): Plan designs built at least once in this world, and the
    /// builds in all. World totals; a snapshot from a build before S9 reads 0.
    #[serde(default)]
    pub ideas_designs: u32,
    #[serde(default)]
    pub ideas_builds: u32,
    /// Time (S9): the mean lived-in term over places that have a use record;
    /// `None` when none has one (and from a build before S9).
    #[serde(default)]
    pub time_permille: Option<u16>,
}

impl BloomFigures {
    pub fn from_summary(s: &crate::bloom::WorldSummary) -> Self {
        let permille = |x: f32| (x.clamp(0.0, 1.0) * 1000.0).round() as u16;
        Self {
            regions: s.regions as u32,
            level_permille: permille(s.mean_level),
            life_permille: permille(s.mean_eco),
            people_permille: permille(s.mean_people),
            ..Self::default()
        }
    }

    /// The same figures with the world's four named terms (S9) filled in:
    /// ideas and time, and life and people from the one function the panel uses.
    pub fn with_terms(self, t: &crate::bloom::WorldTerms) -> Self {
        let permille = |x: f32| (x.clamp(0.0, 1.0) * 1000.0).round() as u16;
        Self {
            life_permille: permille(t.life),
            people_permille: permille(t.people),
            ideas_designs: t.ideas.0,
            ideas_builds: t.ideas.1,
            time_permille: t.time.map(permille),
            ..self
        }
    }
}

/// Living world S5 — the Tally as a dedicated server counts it: the block
/// edits it accepted from joiners. Crafts, smelts and trades happen in window
/// operations the server does not count, so they read 0 here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TallyFigures {
    pub mined: u64,
    pub placed: u64,
    pub crafted: u64,
    pub smelted: u64,
    pub trades: u64,
}

impl From<&crate::tally::Tally> for TallyFigures {
    fn from(t: &crate::tally::Tally) -> Self {
        Self { mined: t.mined, placed: t.placed, crafted: t.crafted, smelted: t.smelted, trades: t.trades }
    }
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
    /// Whether the server runs an allowlist. `false` is an open server;
    /// `true` with `allowlist_count` 0 is a closed one that admits nobody.
    /// A snapshot from a build before this field reads as off.
    #[serde(default)]
    pub allowlist_on: bool,
    pub allowlist_count: u32,
    pub blocklist_count: u32,
    pub operator_npub: String,
    pub runtime_npub: String,
    pub delegation_expires_unix: u64,
    pub protocol: u32,
    /// World bloom on a dedicated server (S5). `None` from a build before the
    /// field, or a server that does not measure it; the console site does not
    /// read it.
    #[serde(default)]
    pub world_bloom: Option<BloomFigures>,
    /// The Tally on a dedicated server (S5); `None` as for `world_bloom`.
    #[serde(default)]
    pub tally: Option<TallyFigures>,
}

/// Assemble a snapshot from the server's live inputs. Pure.
/// `policy` = `(require_signin, allow, blocklist_count)`.
/// `ident` = `(operator_npub, runtime_npub, delegation_expires_unix)`.
/// `world` = `(world bloom, the Tally)`, each `None` where the server keeps none.
pub fn build_snapshot(
    settings: &ConsoleSettings,
    roster: &[PlayerRow],
    agg: &TelemetryAggregates,
    policy: (bool, &Allow, u32),
    ident: (String, String, u64),
    world: (Option<BloomFigures>, Option<TallyFigures>),
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
        allowlist_on: matches!(policy.1, Allow::Only(_)),
        allowlist_count: policy.1.listed() as u32,
        blocklist_count: policy.2,
        operator_npub: ident.0,
        runtime_npub: ident.1,
        delegation_expires_unix: ident.2,
        protocol: crate::protocol::PROTOCOL_VERSION,
        world_bloom: world.0,
        tally: world.1,
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
            (true, &Allow::only([[1u8; 32], [2u8; 32], [3u8; 32], [4u8; 32]]), 1),
            ("npub1op".into(), "npub1rt".into(), 1_900_000_000),
            (None, None),
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
        assert!(snap.allowlist_on);
        assert_eq!(snap.allowlist_count, 4);
        assert_eq!(snap.blocklist_count, 1);
        assert_eq!(snap.operator_npub, "npub1op");
        assert_eq!(snap.runtime_npub, "npub1rt");
        assert_eq!(snap.delegation_expires_unix, 1_900_000_000);
        assert_eq!(snap.protocol, crate::protocol::PROTOCOL_VERSION);
    }

    /// An open server and a closed one both list nobody: only the mode tells them apart.
    #[test]
    fn the_snapshot_says_open_or_only_these() {
        let build = |allow: &Allow| {
            build_snapshot(
                &ConsoleSettings::default(),
                &[],
                &TelemetryAggregates::default(),
                (true, allow, 0),
                (String::new(), String::new(), 0),
                (None, None),
            )
        };
        let open = build(&Allow::Open);
        assert!(!open.allowlist_on, "no allowlist: open");
        assert_eq!(open.allowlist_count, 0);
        let closed = build(&Allow::only([]));
        assert!(closed.allowlist_on, "an allowlist with nobody on it is still on");
        assert_eq!(closed.allowlist_count, 0);
        assert_ne!(open, closed);
        let some = build(&Allow::only([[7u8; 32]]));
        assert!(some.allowlist_on);
        assert_eq!(some.allowlist_count, 1);
        // A snapshot from a build that predates the field still reads.
        let mut json: serde_json::Value = serde_json::to_value(&closed).unwrap();
        json.as_object_mut().unwrap().remove("allowlist_on");
        let old: ConsoleSnapshot = serde_json::from_value(json).unwrap();
        assert!(!old.allowlist_on);
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
            allowlist_on: false,
            allowlist_count: 0,
            blocklist_count: 0,
            operator_npub: "npub1op".into(),
            runtime_npub: "npub1rt".into(),
            delegation_expires_unix: 0,
            protocol: crate::protocol::PROTOCOL_VERSION,
            world_bloom: Some(BloomFigures { regions: 9, level_permille: 620, life_permille: 700, people_permille: 200, ideas_designs: 4, ideas_builds: 11, time_permille: Some(350) }),
            tally: Some(TallyFigures { mined: 5, placed: 4, crafted: 0, smelted: 0, trades: 0 }),
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: ConsoleSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snap, back);
    }

    /// Living world S5 — the world-bloom and Tally figures reach the snapshot, and
    /// a snapshot from a build before them still reads (the console site's
    /// readers, and an older server, see them absent).
    #[test]
    fn the_snapshot_carries_world_bloom_and_the_tally_and_reads_without_them() {
        let summary = crate::bloom::WorldSummary { mean_level: 0.6234, mean_eco: 0.7, mean_people: 1.4, regions: 9 };
        let bloom = BloomFigures::from_summary(&summary);
        assert_eq!(
            bloom,
            BloomFigures { regions: 9, level_permille: 623, life_permille: 700, people_permille: 1000, ..Default::default() }
        );
        let tally = crate::tally::Tally { mined: 11, placed: 7, crafted: 3, smelted: 2, trades: 1 };
        let snap = build_snapshot(
            &ConsoleSettings::default(),
            &[],
            &TelemetryAggregates::default(),
            (true, &Allow::Open, 0),
            (String::new(), String::new(), 0),
            (Some(bloom), Some(TallyFigures::from(&tally))),
        );
        assert_eq!(snap.world_bloom, Some(bloom));
        assert_eq!(snap.tally.map(|t| (t.mined, t.placed, t.crafted, t.smelted, t.trades)), Some((11, 7, 3, 2, 1)));
        let back: ConsoleSnapshot = serde_json::from_str(&serde_json::to_string(&snap).unwrap()).unwrap();
        assert_eq!(back, snap);
        // An older snapshot has neither key.
        let mut json = serde_json::to_value(&snap).unwrap();
        json.as_object_mut().unwrap().remove("world_bloom");
        json.as_object_mut().unwrap().remove("tally");
        let old: ConsoleSnapshot = serde_json::from_value(json).unwrap();
        assert_eq!((old.world_bloom, old.tally), (None, None));
        // S9: the four named terms ride along, and a bloom entry from a build
        // before them (no ideas or time keys) still reads, as absent.
        let terms = crate::bloom::WorldTerms { life: 0.7, ideas: (4, 11), people: 0.2, time: Some(0.35) };
        let full = bloom.with_terms(&terms);
        assert_eq!(
            (full.life_permille, full.people_permille, full.ideas_designs, full.ideas_builds, full.time_permille),
            (700, 200, 4, 11, Some(350))
        );
        let none_time = bloom.with_terms(&crate::bloom::WorldTerms { time: None, ..terms });
        assert_eq!(none_time.time_permille, None);
        let mut snap2 = snap.clone();
        snap2.world_bloom = Some(full);
        let back: ConsoleSnapshot = serde_json::from_str(&serde_json::to_string(&snap2).unwrap()).unwrap();
        assert_eq!(back, snap2);
        let mut json = serde_json::to_value(&snap2).unwrap();
        let wb = json["world_bloom"].as_object_mut().unwrap();
        for k in ["ideas_designs", "ideas_builds", "time_permille"] {
            assert!(wb.remove(k).is_some());
        }
        let old: ConsoleSnapshot = serde_json::from_value(json).unwrap();
        let ob = old.world_bloom.unwrap();
        assert_eq!((ob.ideas_designs, ob.ideas_builds, ob.time_permille), (0, 0, None));
        assert_eq!((ob.life_permille, ob.people_permille), (700, 200));
        // A server that keeps none sends none.
        let none = build_snapshot(
            &ConsoleSettings::default(),
            &[],
            &TelemetryAggregates::default(),
            (false, &Allow::Open, 0),
            (String::new(), String::new(), 0),
            (None, None),
        );
        assert!(none.world_bloom.is_none() && none.tally.is_none());
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
