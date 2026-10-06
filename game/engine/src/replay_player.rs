//! Replay playback engine (Phase 2) — **re-applies recorded state, never
//! re-simulates** (no `Player::tick`, no `mob_ai`, no economy). Loads a
//! `ReplayFile`, reconstructs the world from the initial snapshot, and walks the
//! recorded block-change timeline. The Phase-1 `DirectorCamera` is the viewpoint;
//! `current_targets()` exposes interpolated player transforms so Follow/POV stay
//! smooth at 0.25×. Native-only.
//!
//! No UI drives playback yet (no replay browser/player screen — see the
//! `main.rs::replay_player` field and `replay.rs`'s `list_replays`/
//! `load_replay`, both also unwired), so this whole module is tested but
//! otherwise unreferenced. The type, its `impl` and `sample_targets` carry
//! item-level `allow(dead_code)`.

use glam::Vec3;

use crate::replay::{ReplayFile, ReplayFrame};
use crate::world::World;

#[allow(dead_code)] // no replay browser/player screen drives playback yet (see module doc); tested only
pub struct ReplayPlayer {
    /// The tick-0 world archive, kept for rewind (backward scrub re-unpacks).
    base_blob: Vec<u8>,
    /// The current reconstructed playback world (blocks).
    pub world: World,
    frames: Vec<ReplayFrame>,
    duration_ticks: u32,
    /// Highest frame index whose `block_changes` are applied to `world` (-1 = none).
    applied_upto: i64,
    /// Fractional tick cursor.
    pub cursor: f32,
    pub speed: f32,
    pub playing: bool,
}

#[allow(dead_code)] // no replay browser/player screen drives playback yet (see module doc); tested only
impl ReplayPlayer {
    /// Reconstruct the playback world from the initial snapshot and load the
    /// timeline. Cursor starts at tick 0 (no frames applied yet).
    pub fn load(file: ReplayFile) -> Result<Self, String> {
        let mut world = World::new();
        crate::world_archive::unpack_world(&file.initial.world_archive_blob, &mut world)?;
        Ok(Self::from_parts(world, file))
    }

    fn from_parts(world: World, file: ReplayFile) -> Self {
        Self {
            base_blob: file.initial.world_archive_blob,
            world,
            duration_ticks: file.header.duration_ticks,
            frames: file.frames,
            applied_upto: -1,
            cursor: 0.0,
            speed: 1.0,
            playing: false,
        }
    }

    pub fn duration_ticks(&self) -> u32 {
        self.duration_ticks
    }

    pub fn cursor_tick(&self) -> u32 {
        self.cursor.floor().max(0.0) as u32
    }

    /// Index of the frame at or just before `tick`.
    fn frame_index_at(&self, tick: u32) -> Option<usize> {
        if self.frames.is_empty() {
            return None;
        }
        let mut i = 0;
        while i + 1 < self.frames.len() && self.frames[i + 1].tick <= tick as u64 {
            i += 1;
        }
        Some(i)
    }

    /// Jump to `tick` and reconstruct world state there.
    pub fn seek(&mut self, tick: u32) {
        let tick = tick.min(self.duration_ticks);
        if let Some(idx) = self.frame_index_at(tick) {
            self.rebuild_to(idx as i64);
        }
        self.cursor = tick as f32;
    }

    /// Advance the playback clock by `dt` real seconds × `speed`, applying any
    /// frames crossed. No-op while paused.
    pub fn advance(&mut self, dt: f32) {
        if !self.playing {
            return;
        }
        self.cursor = (self.cursor + dt * self.speed * crate::replay::REPLAY_TICK_RATE as f32)
            .clamp(0.0, self.duration_ticks as f32);
        if let Some(idx) = self.frame_index_at(self.cursor.floor() as u32) {
            self.rebuild_to(idx as i64);
        }
    }

    /// Bring `world` to the block state at frame `target_idx`. Forward = apply
    /// the deltas crossed; backward = re-unpack the base world then forward.
    fn rebuild_to(&mut self, target_idx: i64) {
        if target_idx < self.applied_upto {
            let mut w = World::new();
            if crate::world_archive::unpack_world(&self.base_blob, &mut w).is_ok() {
                self.world = w;
            }
            self.applied_upto = -1;
        }
        if target_idx < 0 {
            return;
        }
        let start = (self.applied_upto + 1).max(0) as usize;
        for i in start..=(target_idx as usize) {
            if let Some(f) = self.frames.get(i) {
                for bc in &f.block_changes {
                    self.world.set_block(bc.x, bc.y, bc.z, bc.new_block);
                }
            }
        }
        self.applied_upto = target_idx;
    }

    /// Interpolated player viewpoints at the current cursor (for Follow/POV).
    pub fn current_targets(&self) -> Vec<crate::director::TargetSnapshot> {
        sample_targets(&self.frames, self.cursor)
    }
}

/// Pure: interpolated player targets at fractional tick `cursor` — `lerp`
/// position, shortest-arc yaw/pitch (so 0.25× slow-mo is smooth, not stepped).
#[allow(dead_code)] // no replay browser/player screen drives playback yet (see module doc); tested only
pub fn sample_targets(frames: &[ReplayFrame], cursor: f32) -> Vec<crate::director::TargetSnapshot> {
    if frames.is_empty() {
        return Vec::new();
    }
    let cursor = cursor.max(0.0);
    // `cursor` is in the TICK domain (matching `seek`/`advance`/`frame_index_at`),
    // NOT a frame index — frames may be sparse or non-zero-based (an MP server tee
    // records absolute ticks). Locate the bracketing frames by `.tick`.
    let mut i = 0;
    while i + 1 < frames.len() && (frames[i + 1].tick as f32) <= cursor {
        i += 1;
    }
    let j = (i + 1).min(frames.len() - 1);
    let a = &frames[i];
    let b = &frames[j];
    // Fractional phase across the [a.tick, b.tick] gap. `max(1.0)` guards i==j
    // (past the last frame) and any zero-width tick gap → frac collapses to a hold.
    let span = (b.tick as f32 - a.tick as f32).max(1.0);
    let frac = ((cursor - a.tick as f32) / span).clamp(0.0, 1.0);
    a.players
        .iter()
        .map(|pa| {
            let pb = b.players.iter().find(|p| p.player_index == pa.player_index).unwrap_or(pa);
            let pos = Vec3::new(pa.x, pa.y, pa.z).lerp(Vec3::new(pb.x, pb.y, pb.z), frac);
            crate::director::TargetSnapshot {
                eye: pos + Vec3::Y * 1.62,
                yaw: crate::camera_path::angle_lerp(pa.yaw, pb.yaw, frac),
                pitch: crate::camera_path::angle_lerp(pa.pitch, pb.pitch, frac),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{BlockChange, PlayerState};
    use crate::replay::{InitialSnapshot, ReplayFile, ReplayFrame, ReplayHeader, ReplaySource};

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

    fn frame(tick: u64, x: f32, yaw: f32, bc: Vec<BlockChange>) -> ReplayFrame {
        ReplayFrame {
            tick,
            players: vec![player(0, x, yaw)],
            block_changes: bc,
            entity_spawns: vec![],
            entity_updates: vec![],
            entity_despawns: vec![],
            world_time: 0,
            is_keyframe: tick == 0,
        }
    }

    fn file(frames: Vec<ReplayFrame>) -> ReplayFile {
        let dur = frames.last().map(|f| f.tick as u32).unwrap_or(0);
        ReplayFile {
            header: ReplayHeader {
                format_version: 1,
                source: ReplaySource::SinglePlayer,
                tick_rate: 20,
                world_seed: 0,
                world_name: "t".into(),
                start_world_time: 0,
                duration_ticks: dur,
                keyframe_interval_ticks: 100,
            },
            initial: InitialSnapshot { world_archive_blob: vec![], players: vec![], entities: vec![] },
            frames,
        }
    }

    // sample_targets interpolates position + shortest-arc angles.
    #[test]
    fn interpolates_targets_midframe() {
        let frames = vec![
            frame(0, 0.0, 350f32.to_radians(), vec![]),
            frame(1, 10.0, 10f32.to_radians(), vec![]),
        ];
        let t = &sample_targets(&frames, 0.5)[0];
        assert!((t.eye.x - 5.0).abs() < 1e-4, "x={}", t.eye.x);
        assert!((t.eye.y - (64.0 + 1.62)).abs() < 1e-4);
        // 350°→10° short arc midpoint = 360° ≡ 0°.
        assert!((t.yaw - 360f32.to_radians()).abs() < 1e-3, "yaw={}", t.yaw.to_degrees());
    }

    #[test]
    fn empty_or_single_frame_targets() {
        assert!(sample_targets(&[], 0.0).is_empty());
        let one = vec![frame(0, 3.0, 0.0, vec![])];
        let t = &sample_targets(&one, 0.9)[0];
        assert!((t.eye.x - 3.0).abs() < 1e-4); // holds the single frame
    }

    // `cursor` is a TICK, not a frame index — sample_targets must locate the
    // bracketing frames by `.tick` (frames can be sparse or non-zero-based, e.g.
    // an MP server tee using absolute ticks). Indexing `frames[cursor.floor()]`
    // would read the wrong frame the moment tick != index.
    #[test]
    fn sample_targets_is_tick_domain_not_index() {
        // Two frames whose .tick (100, 101) != their vec index (0, 1).
        let frames = vec![frame(100, 0.0, 0.0, vec![]), frame(101, 10.0, 0.0, vec![])];
        // Cursor 100.5 ticks = halfway between tick 100 and tick 101 → x = 5.0.
        let t = &sample_targets(&frames, 100.5)[0];
        assert!((t.eye.x - 5.0).abs() < 1e-4, "x={} (expected 5.0 — tick-domain interp)", t.eye.x);
        // Exactly on the second frame's tick → that frame's pose.
        let t2 = &sample_targets(&frames, 101.0)[0];
        assert!((t2.eye.x - 10.0).abs() < 1e-4, "x={}", t2.eye.x);
        // Before the first frame's tick → clamp to the first frame.
        let t0 = &sample_targets(&frames, 50.0)[0];
        assert!((t0.eye.x - 0.0).abs() < 1e-4, "x={}", t0.eye.x);
    }

    // Forward seek applies the block-change timeline; world reflects tick K.
    #[test]
    fn forward_seek_applies_block_changes() {
        let bc = |x: i32, b: u16| BlockChange { x, y: 64, z: 0, new_block: b, meta: 0 };
        let frames = vec![
            frame(0, 0.0, 0.0, vec![bc(0, 5)]),
            frame(1, 0.0, 0.0, vec![bc(1, 6)]),
            frame(2, 0.0, 0.0, vec![bc(2, 7)]),
        ];
        // from_parts bypasses unpack (empty base world = all air); forward-only.
        let mut p = ReplayPlayer::from_parts(World::new(), file(frames));
        p.seek(2);
        assert_eq!(p.world.get_block(0, 64, 0), 5);
        assert_eq!(p.world.get_block(1, 64, 0), 6);
        assert_eq!(p.world.get_block(2, 64, 0), 7);
        assert_eq!(p.cursor_tick(), 2);
    }

    // Integration: pack a REAL world as the tick-0 base (the path the live SP
    // recorder uses via pack_world/export), record block-change frames, then load
    // + reconstruct — proves pack → unpack → forward-apply end to end.
    #[test]
    fn load_reconstructs_packed_world_and_applies_changes() {
        use crate::block;
        let mut w = crate::world::World::new();
        w.set_block(0, 64, 0, block::STONE);
        w.set_block(1, 64, 0, block::DIRT);
        let meta = crate::save::WorldMeta::new("__replay_rt");
        let save = crate::save::minimal_world_save_for_tests(7);
        let blob = crate::world_archive::pack_world(&meta, &save, &w, &[]).expect("pack");

        let bc = |x: i32, y: i32, z: i32, b: u16| BlockChange { x, y, z, new_block: b, meta: 0 };
        let frames = vec![
            // tick 0: mine the stone to air.
            frame(0, 0.0, 0.0, vec![bc(0, 64, 0, block::AIR)]),
            // tick 1: place grass two over.
            frame(1, 0.0, 0.0, vec![bc(2, 64, 0, block::GRASS)]),
        ];
        let mut file = file(frames);
        file.initial.world_archive_blob = blob;
        file.header.world_seed = 7;

        let mut p = ReplayPlayer::load(file).expect("load");
        // Base world restored from the blob before any frame is applied.
        assert_eq!(p.world.get_block(1, 64, 0), block::DIRT, "base DIRT restored");
        // Seek tick 0: stone mined to air; untouched dirt survives.
        p.seek(0);
        assert_eq!(p.world.get_block(0, 64, 0), block::AIR, "tick-0 change applied");
        assert_eq!(p.world.get_block(1, 64, 0), block::DIRT, "untouched block survives");
        // Seek tick 1: grass appears.
        p.seek(1);
        assert_eq!(p.world.get_block(2, 64, 0), block::GRASS, "tick-1 change applied");
        // Seek back to tick 0: backward re-unpack restores the base, re-applies
        // only tick 0 → grass is gone again.
        p.seek(0);
        assert_eq!(p.world.get_block(2, 64, 0), block::AIR, "backward seek undoes tick-1");
        assert_eq!(p.world.get_block(0, 64, 0), block::AIR, "tick-0 change still applied");
    }

    // advance is gated on `playing` and clamps at the end.
    #[test]
    fn advance_respects_playing_and_clamps() {
        let frames = vec![frame(0, 0.0, 0.0, vec![]), frame(1, 0.0, 0.0, vec![])];
        let mut p = ReplayPlayer::from_parts(World::new(), file(frames));
        p.advance(1.0); // paused → no move
        assert_eq!(p.cursor, 0.0);
        p.playing = true;
        p.advance(10.0); // far past the end
        assert_eq!(p.cursor, p.duration_ticks() as f32);
    }
}
