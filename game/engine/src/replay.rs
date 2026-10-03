//! `.axereplay` format + recorder + native store (Phase 2).
//!
//! A replay is an **initial world snapshot** plus a per-tick timeline of state
//! deltas (the same `protocol` types the network already serialises) — playback
//! re-applies recorded state, never re-simulates (a generalisation of the Trials
//! ghost from one runner to the whole scene). The whole file is bincode'd +
//! lz4'd as one blob; `ReplayPlayer` loads it fully into RAM and seeks over the
//! in-memory frame vector, so no byte-offset index is needed. Native-only (the
//! web/localStorage store is a Phase-3 follow-up).
//!
//! Source-agnostic: single-player records the local sim, multiplayer (Phase 3)
//! tees the inbound/authoritative `StateUpdate` stream — same file, same player.

use serde::{Deserialize, Serialize};

use crate::protocol::{BlockChange, EntitySpawn, EntityUpdate, PlayerState};

pub const REPLAY_MAGIC: &[u8; 8] = b"AXEREPL1";
pub const REPLAY_FORMAT_VERSION: u32 = 1;
pub const REPLAY_TICK_RATE: u32 = 20;
/// 10 minutes @ 20 TPS — mirrors the `MAX_GHOST_FRAMES` discipline.
pub const MAX_REPLAY_DURATION_TICKS: u32 = 12_000;
/// A full-state frame every 5 s — the seek anchors.
pub const KEYFRAME_INTERVAL_TICKS: u64 = 100;
/// Decompression bomb guard (a local file is trusted, but never OOM on a corrupt one).
#[cfg_attr(not(test), allow(dead_code))]
const MAX_DECOMPRESSED: usize = 512 * 1024 * 1024;

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum ReplaySource {
    SinglePlayer,
    MpClientTee,
    MpServerSession,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ReplayHeader {
    pub format_version: u32,
    pub source: ReplaySource,
    pub tick_rate: u32,
    pub world_seed: u32,
    pub world_name: String,
    pub start_world_time: u32,
    pub duration_ticks: u32,
    pub keyframe_interval_ticks: u64,
}

/// The world + actors at tick 0. `world_archive_blob` is `pack_world(...)` bytes.
#[derive(Clone, Serialize, Deserialize)]
pub struct InitialSnapshot {
    pub world_archive_blob: Vec<u8>,
    pub players: Vec<PlayerState>,
    pub entities: Vec<EntitySpawn>,
}

/// One sim tick — a `StateUpdatePacket` minus server bookkeeping. On a keyframe
/// tick the caller fills `players` + `entity_spawns` with the FULL state (the
/// seek anchor); otherwise they are deltas.
#[derive(Clone, Serialize, Deserialize)]
pub struct ReplayFrame {
    pub tick: u64,
    pub players: Vec<PlayerState>,
    pub block_changes: Vec<BlockChange>,
    pub entity_spawns: Vec<EntitySpawn>,
    pub entity_updates: Vec<EntityUpdate>,
    pub entity_despawns: Vec<u32>,
    pub world_time: u32,
    pub is_keyframe: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ReplayFile {
    pub header: ReplayHeader,
    pub initial: InitialSnapshot,
    pub frames: Vec<ReplayFrame>,
}

/// Serialise: `magic[8] | format_version u32 | lz4(bincode(ReplayFile))`.
pub fn encode_replay(file: &ReplayFile) -> Vec<u8> {
    let body = bincode::serialize(file).expect("replay serialization failed");
    let mut out = Vec::with_capacity(body.len() / 2 + 16);
    out.extend_from_slice(REPLAY_MAGIC);
    out.extend_from_slice(&REPLAY_FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&lz4_flex::compress_prepend_size(&body));
    out
}

/// Live caller: `load_replay` (native-only, itself not yet called from any
/// UI — see below); extensively round-trip tested here.
#[cfg_attr(not(test), allow(dead_code))]
pub fn decode_replay(bytes: &[u8]) -> Result<ReplayFile, String> {
    if bytes.len() < 12 || &bytes[..8] != REPLAY_MAGIC {
        return Err("not an .axereplay (bad magic)".into());
    }
    let ver = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    if ver != REPLAY_FORMAT_VERSION {
        return Err(format!("unsupported replay format version {ver}"));
    }
    let comp = &bytes[12..];
    // The lz4 prepended-size header is the first 4 bytes — reject a bomb before
    // allocating.
    if comp.len() >= 4 {
        let claimed = u32::from_le_bytes(comp[..4].try_into().unwrap()) as usize;
        if claimed > MAX_DECOMPRESSED {
            return Err("replay decompressed size exceeds the cap".into());
        }
    }
    let body = lz4_flex::decompress_size_prepended(comp).map_err(|e| e.to_string())?;
    bincode::deserialize(&body).map_err(|e| e.to_string())
}

/// Append-only frame buffer for a recording in progress.
pub struct ReplayRecorder {
    header: ReplayHeader,
    initial: InitialSnapshot,
    frames: Vec<ReplayFrame>,
    capped: bool,
}

impl ReplayRecorder {
    pub fn start(
        source: ReplaySource,
        world_seed: u32,
        world_name: String,
        start_world_time: u32,
        initial: InitialSnapshot,
    ) -> Self {
        Self {
            header: ReplayHeader {
                format_version: REPLAY_FORMAT_VERSION,
                source,
                tick_rate: REPLAY_TICK_RATE,
                world_seed,
                world_name,
                start_world_time,
                duration_ticks: 0,
                keyframe_interval_ticks: KEYFRAME_INTERVAL_TICKS,
            },
            initial,
            frames: Vec::new(),
            capped: false,
        }
    }

    /// Append a frame. The recorder OWNS the timeline: `frame.tick` is rebased to
    /// push order (0-based, contiguous) so the player's 0-based cursor lines up
    /// even when the caller feeds an absolute sim-tick counter. Flags it as a
    /// keyframe on the interval; no-op (sets `capped`) once the ceiling is hit.
    pub fn push(&mut self, mut frame: ReplayFrame) {
        if self.frames.len() as u32 >= MAX_REPLAY_DURATION_TICKS {
            self.capped = true;
            return;
        }
        frame.tick = self.frames.len() as u64;
        frame.is_keyframe = frame.tick.is_multiple_of(KEYFRAME_INTERVAL_TICKS);
        self.frames.push(frame);
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Idiomatic companion to `len` (which is live) — no caller of its own yet.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn capped(&self) -> bool {
        self.capped
    }

    /// Finalise to encoded `.axereplay` bytes.
    pub fn finalize(mut self) -> Vec<u8> {
        self.header.duration_ticks = self.frames.last().map(|f| f.tick as u32).unwrap_or(0);
        encode_replay(&ReplayFile {
            header: self.header,
            initial: self.initial,
            frames: self.frames,
        })
    }
}

// ── Native store (mirrors `TrialBests`: native file; web is a Phase-3 follow-up) ──

#[cfg(not(target_arch = "wasm32"))]
pub fn replays_dir() -> std::path::PathBuf {
    crate::data_dir::profile_dir().join("replays")
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_replay(name: &str, bytes: &[u8]) -> Result<std::path::PathBuf, String> {
    let dir = replays_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let path = dir.join(format!("{safe}.axereplay"));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

// BRIDGE: `list_replays` + `load_replay` are the "browse and play back a saved
// replay" half of the feature — the recorder (`save_replay`, `ReplayRecorder`)
// is live (game_loop.rs), but no UI calls these yet (no replay browser/player
// screen exists — see project memory "Phase 2c SP recorder BUILT+tested, rest
// = GPU/2-machine/browser boundary").
#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
pub fn list_replays() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(replays_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("axereplay")
                && let Some(stem) = p.file_stem().and_then(|x| x.to_str()) {
                    out.push(stem.to_string());
                }
        }
    }
    out.sort();
    out
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
pub fn load_replay(name: &str) -> Result<ReplayFile, String> {
    let path = replays_dir().join(format!("{name}.axereplay"));
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    decode_replay(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::EntityKind;

    fn player(idx: u32, x: f32, yaw: f32) -> PlayerState {
        PlayerState {
            player_index: idx,
            x,
            y: 64.0,
            z: 0.0,
            yaw,
            pitch: 0.0,
            health: 20.0,
            held_kind: 0,
            held_id: 0,
            anim_state: 0,
            flags: 0,
            skin_key: 0,
        }
    }
    fn block(x: i32, b: u16) -> BlockChange {
        BlockChange { x, y: 64, z: 0, new_block: b, meta: 0 }
    }
    fn frame(tick: u64) -> ReplayFrame {
        ReplayFrame {
            tick,
            players: vec![player(0, tick as f32, 0.1 * tick as f32)],
            block_changes: vec![block(tick as i32, (tick % 7) as u16 + 1)],
            entity_spawns: vec![],
            entity_updates: vec![],
            entity_despawns: vec![],
            world_time: tick as u32 * 2,
            is_keyframe: false,
        }
    }

    fn file_with(n: u64) -> ReplayFile {
        let initial = InitialSnapshot {
            world_archive_blob: vec![1, 2, 3, 4],
            players: vec![player(0, 0.0, 0.0)],
            entities: vec![EntitySpawn { id: 1, kind: EntityKind::Cow, x: 1.0, y: 64.0, z: 1.0, yaw: 0.0, health: 10, item_kind: 0, item_id: 0, item_count: 0, full_item: crate::protocol::WireItem::None }],
        };
        ReplayFile {
            header: ReplayHeader {
                format_version: REPLAY_FORMAT_VERSION,
                source: ReplaySource::SinglePlayer,
                tick_rate: REPLAY_TICK_RATE,
                world_seed: 42,
                world_name: "test".into(),
                start_world_time: 0,
                duration_ticks: n as u32,
                keyframe_interval_ticks: KEYFRAME_INTERVAL_TICKS,
            },
            initial,
            frames: (0..n).map(frame).collect(),
        }
    }

    #[test]
    fn encode_decode_round_trips() {
        let f = file_with(3);
        let bytes = encode_replay(&f);
        let back = decode_replay(&bytes).expect("decode");
        assert_eq!(back.frames.len(), 3);
        assert_eq!(back.header.world_seed, 42);
        assert_eq!(back.initial.world_archive_blob, vec![1, 2, 3, 4]);
        assert_eq!(back.frames[2].tick, 2);
        assert_eq!(back.frames[1].block_changes[0].new_block, frame(1).block_changes[0].new_block);
        assert_eq!(back.frames[0].players[0].player_index, 0);
        assert_eq!(back.initial.entities[0].kind, EntityKind::Cow);
    }

    #[test]
    fn rejects_bad_magic_and_truncation() {
        assert!(decode_replay(b"nope").is_err());
        assert!(decode_replay(&[]).is_err());
        let mut bytes = encode_replay(&file_with(2));
        bytes.truncate(20); // corrupt tail
        assert!(decode_replay(&bytes).is_err()); // Err, not panic
    }

    #[test]
    fn rejects_decompression_bomb() {
        // magic + version + a claimed-size prefix far over the cap.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(REPLAY_MAGIC);
        bytes.extend_from_slice(&REPLAY_FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // 4 GiB claim
        bytes.extend_from_slice(&[0u8; 8]);
        assert!(decode_replay(&bytes).is_err());
    }

    #[test]
    fn recorder_flags_keyframes_and_caps() {
        let initial = file_with(0).initial;
        let mut rec = ReplayRecorder::start(ReplaySource::SinglePlayer, 1, "w".into(), 0, initial);
        for t in 0..205u64 {
            rec.push(frame(t));
        }
        assert_eq!(rec.len(), 205);
        let bytes = rec.finalize();
        let back = decode_replay(&bytes).unwrap();
        assert!(back.frames[0].is_keyframe, "tick 0 is a keyframe");
        assert!(back.frames[100].is_keyframe, "tick 100 is a keyframe");
        assert!(!back.frames[50].is_keyframe);
        assert_eq!(back.header.duration_ticks, 204);
    }

    #[test]
    fn recorder_owns_a_zero_based_contiguous_timeline() {
        // The recorder must rebase frame.tick to push order so the player's
        // 0-based cursor lines up — even if the caller feeds absolute sim ticks.
        let initial = file_with(0).initial;
        let mut rec = ReplayRecorder::start(ReplaySource::SinglePlayer, 1, "w".into(), 0, initial);
        // Feed frames whose .tick starts at 5000 (an absolute sim counter).
        for t in 5000..5003u64 {
            rec.push(frame(t));
        }
        let bytes = rec.finalize();
        let back = decode_replay(&bytes).unwrap();
        let ticks: Vec<u64> = back.frames.iter().map(|f| f.tick).collect();
        assert_eq!(ticks, vec![0, 1, 2], "frames rebased to 0-based contiguous");
        assert_eq!(back.header.duration_ticks, 2);
        assert!(back.frames[0].is_keyframe, "first frame is the seek anchor");
    }

    #[test]
    fn recorder_cap_clamps_length() {
        let initial = file_with(0).initial;
        let mut rec = ReplayRecorder::start(ReplaySource::SinglePlayer, 1, "w".into(), 0, initial);
        for t in 0..(MAX_REPLAY_DURATION_TICKS as u64 + 50) {
            rec.push(frame(t));
        }
        assert!(rec.capped());
        assert_eq!(rec.len(), MAX_REPLAY_DURATION_TICKS as usize);
    }
}
