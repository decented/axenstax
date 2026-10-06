//! Chunk storage — 16x16x16 sub-chunks.
//!
//! Spec 02 Section 2: Chunks are 16x16x16 blocks. Each sub-chunk stores block data
//! in a palette-compressed format. For the prototype, we use a flat u16 array.
//! Palette compression is a future optimisation.

use crate::block::{BlockId, AIR};

pub const CHUNK_SIZE: usize = 16;
pub const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;

/// Number of `u64` words in the per-voxel "player-placed" bitmask
/// (one bit per voxel). `CHUNK_VOLUME / 64`.
const PLACED_WORDS: usize = CHUNK_VOLUME / 64;
/// Serialized byte length of the placed mask (`CHUNK_VOLUME / 8`).
/// Internal to the chunk codec — not part of the crate API.
const PLACED_MASK_BYTES: usize = CHUNK_VOLUME / 8;

#[cfg(test)]
thread_local! {
    /// Test-only: how many full recounts ([`Chunk::recount_non_air`]) this
    /// thread has run. Per-thread, so parallel tests cannot disturb each other.
    static FULL_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Test-only reader for the thread's full-recount counter.
#[cfg(test)]
pub(crate) fn full_scans_on_this_thread() -> usize {
    FULL_SCANS.with(std::cell::Cell::get)
}

/// A 16x16x16 sub-chunk of block data.
pub struct Chunk {
    /// Flat array of block IDs. Index = x + z * 16 + y * 256.
    blocks: [BlockId; CHUNK_VOLUME],
    /// How many cells of `blocks` are not `AIR`. Maintained on every write
    /// (`set`, `from_bytes`) so [`Chunk::is_empty`] is O(1) — the world sims ask
    /// "does this column hold a real block?" per entity per tick. `blocks` is
    /// private, so those two are the only places it can change; the
    /// `non_air_count_*` tests pin the count against a full recount.
    non_air: u16,
    /// Spec 30 — packed per-voxel light: `(sky_light << 4) | block_light`.
    /// Each nibble is 0..15. Initialised to 0 (dark); world-gen + place
    /// hooks fill in via the `lighting` BFS module. Not persisted on
    /// save — recomputed on load via `lighting::initial_pass_for_column`.
    light: [u8; CHUNK_VOLUME],
    /// Spec 06 §2.2 anti-farming — one bit per voxel: set when a *player*
    /// placed the block in this cell. Breaking a player-placed block earns
    /// no proof-of-play hash/work (you still recover the item). Natural /
    /// world-gen blocks have the bit clear and earn work normally.
    /// Persisted alongside `blocks`; absent in pre-feature saves (all-natural).
    placed: [u64; PLACED_WORDS],
    /// True if the mesh needs rebuilding.
    pub mesh_dirty: bool,
    /// Spec 02 §7.5 — "this chunk must survive an unload": it differs from pure
    /// world-gen (a post-generation block / placed-bit change) or it came from a
    /// save / the network (`from_bytes`). An unloaded column with any `persist`
    /// chunk moves to `World::evicted` instead of being dropped. Runtime-only:
    /// NOT serialised (`as_bytes` / `from_bytes` layout unchanged). Set by the
    /// `World` block setters outside world-gen; light writes never touch it.
    persist: bool,
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            blocks: [AIR; CHUNK_VOLUME],
            non_air: 0,
            light: [0; CHUNK_VOLUME],
            placed: [0; PLACED_WORDS],
            mesh_dirty: true,
            persist: false,
        }
    }

    /// Spec 02 §7.5 — must this chunk survive an unload? See the field doc.
    #[inline]
    pub fn persist(&self) -> bool {
        self.persist
    }

    /// Mark this chunk as persist-worthy (edited after generation, or loaded).
    #[inline]
    pub fn mark_persist(&mut self) {
        self.persist = true;
    }

    /// Convert local (x, y, z) to flat array index.
    /// x, y, z must be in 0..16.
    #[inline]
    fn index(x: usize, y: usize, z: usize) -> usize {
        debug_assert!(x < CHUNK_SIZE && y < CHUNK_SIZE && z < CHUNK_SIZE);
        x + z * CHUNK_SIZE + y * CHUNK_SIZE * CHUNK_SIZE
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize, z: usize) -> BlockId {
        self.blocks[Self::index(x, y, z)]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, z: usize, block: BlockId) {
        let slot = &mut self.blocks[Self::index(x, y, z)];
        let was_air = *slot == AIR;
        *slot = block;
        match (was_air, block == AIR) {
            (true, false) => self.non_air += 1,
            (false, true) => self.non_air -= 1,
            _ => {}
        }
        self.mesh_dirty = true;
    }

    /// Check if chunk is entirely air. O(1): reads the maintained
    /// [`Chunk::non_air_count`], never scans the cells.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.non_air == 0
    }

    /// How many cells hold a block other than `AIR`. O(1). Test-only: the
    /// engine itself only asks the yes/no [`Chunk::is_empty`].
    #[cfg(test)]
    #[inline]
    pub(crate) fn non_air_count(&self) -> usize {
        usize::from(self.non_air)
    }

    /// Full recount of the non-air cells — the oracle the maintained count is
    /// tested against. Test-only; it bumps [`full_scans_on_this_thread`] so a
    /// test can pin that a code path never calls it.
    #[cfg(test)]
    pub(crate) fn recount_non_air(&self) -> usize {
        FULL_SCANS.with(|c| c.set(c.get() + 1));
        self.blocks.iter().filter(|&&b| b != AIR).count()
    }

    // ── Spec 06 §2.2 — per-voxel "player-placed" mask ────────────────

    /// True if the block at (x, y, z) was placed by a player (and so earns
    /// no proof-of-play work when broken). x, y, z must be in 0..16.
    #[inline]
    pub fn is_placed(&self, x: usize, y: usize, z: usize) -> bool {
        let idx = Self::index(x, y, z);
        (self.placed[idx / 64] >> (idx % 64)) & 1 == 1
    }

    /// Mark / unmark the block at (x, y, z) as player-placed.
    #[inline]
    pub fn set_placed(&mut self, x: usize, y: usize, z: usize, placed: bool) {
        let idx = Self::index(x, y, z);
        let word = &mut self.placed[idx / 64];
        let bit = 1u64 << (idx % 64);
        if placed {
            *word |= bit;
        } else {
            *word &= !bit;
        }
    }

    // ── Spec 30 — per-voxel light data ─────────────────────────────

    /// Raw packed light byte at (x, y, z): `(sky << 4) | block`. Superseded
    /// by the split `block_light_at`/`sky_light_at` below, which is what
    /// lighting.rs actually uses.
    #[inline]
    #[allow(dead_code)]
    pub fn light_at(&self, x: usize, y: usize, z: usize) -> u8 {
        self.light[Self::index(x, y, z)]
    }

    /// 4-bit block-light value at (x, y, z), 0..=15.
    #[inline]
    pub fn block_light_at(&self, x: usize, y: usize, z: usize) -> u8 {
        self.light[Self::index(x, y, z)] & 0x0F
    }

    /// 4-bit sky-light value at (x, y, z), 0..=15.
    #[inline]
    pub fn sky_light_at(&self, x: usize, y: usize, z: usize) -> u8 {
        (self.light[Self::index(x, y, z)] >> 4) & 0x0F
    }

    /// Set the 4-bit block-light value at (x, y, z). Clamps to 0..=15.
    pub fn set_block_light_at(&mut self, x: usize, y: usize, z: usize, v: u8) {
        let idx = Self::index(x, y, z);
        let v = v & 0x0F;
        self.light[idx] = (self.light[idx] & 0xF0) | v;
    }

    /// Set the 4-bit sky-light value at (x, y, z). Clamps to 0..=15.
    pub fn set_sky_light_at(&mut self, x: usize, y: usize, z: usize, v: u8) {
        let idx = Self::index(x, y, z);
        let v = v & 0x0F;
        self.light[idx] = (self.light[idx] & 0x0F) | (v << 4);
    }

    /// Replace the entire light array (used by lighting BFS for bulk
    /// fills). Caller's responsibility to pass a properly-sized array.
    /// lighting.rs uses the per-cell `set_block_light_at`/`set_sky_light_at`
    /// instead — no caller for this bulk path.
    #[allow(dead_code)]
    pub fn set_light_bulk(&mut self, light: [u8; CHUNK_VOLUME]) {
        self.light = light;
        self.mesh_dirty = true;
    }

    /// Reset all light to 0 — used before a re-propagation pass. Same
    /// "no caller" story as `set_light_bulk` above.
    #[allow(dead_code)]
    pub fn clear_light(&mut self) {
        self.light = [0; CHUNK_VOLUME];
    }

    /// Serialize chunk to raw bytes: the little-endian `u16` block array
    /// followed by the little-endian `u64` placed mask. Light is not
    /// persisted (recomputed on load).
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CHUNK_VOLUME * 2 + PLACED_MASK_BYTES);
        for &block in &self.blocks {
            bytes.extend_from_slice(&block.to_le_bytes());
        }
        for &word in &self.placed {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes
    }

    /// Deserialize chunk from raw bytes. Accepts two lengths: the legacy
    /// pre-feature length (block array only — placed mask reads all-natural)
    /// and the current length (block array + placed mask). Any other length
    /// is rejected rather than silently truncated, matching the append-only
    /// WorldSave invariant.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let block_bytes = CHUNK_VOLUME * 2;
        let has_mask = match data.len() {
            n if n == block_bytes => false,
            n if n == block_bytes + PLACED_MASK_BYTES => true,
            _ => return None,
        };
        let mut blocks = [AIR; CHUNK_VOLUME];
        let mut non_air = 0u16;
        for (i, chunk) in data[..block_bytes].as_chunks::<2>().0.iter().enumerate() {
            blocks[i] = u16::from_le_bytes(*chunk);
            non_air += u16::from(blocks[i] != AIR);
        }
        let mut placed = [0u64; PLACED_WORDS];
        if has_mask {
            for (i, word) in data[block_bytes..].as_chunks::<8>().0.iter().enumerate() {
                placed[i] = u64::from_le_bytes(*word);
            }
        }
        Some(Self {
            blocks,
            non_air,
            light: [0; CHUNK_VOLUME],
            placed,
            mesh_dirty: true,
            // From a save or the network — never regenerate over it.
            persist: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{STONE, DIRT, GRASS};

    #[test]
    fn new_is_all_air_and_empty() {
        let c = Chunk::new();
        assert!(c.is_empty());
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    assert_eq!(c.get(x, y, z), AIR);
                }
            }
        }
    }

    #[test]
    fn set_then_get_matches() {
        let mut c = Chunk::new();
        c.set(1, 2, 3, STONE);
        c.set(15, 15, 15, GRASS);
        c.set(0, 0, 0, DIRT);
        assert_eq!(c.get(1, 2, 3), STONE);
        assert_eq!(c.get(15, 15, 15), GRASS);
        assert_eq!(c.get(0, 0, 0), DIRT);
        assert_eq!(c.get(2, 2, 2), AIR);
        assert!(!c.is_empty());
    }

    #[test]
    fn bytes_roundtrip_preserves_every_block() {
        // Deterministic fill across every id/position combo we can cheaply exercise.
        let mut c = Chunk::new();
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    let id = ((x + y * 7 + z * 31) as u16) & 0x0F;
                    c.set(x, y, z, id);
                }
            }
        }
        let bytes = c.as_bytes();
        assert_eq!(bytes.len(), CHUNK_VOLUME * 2 + PLACED_MASK_BYTES);
        let back = Chunk::from_bytes(&bytes).expect("valid roundtrip");
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    assert_eq!(back.get(x, y, z), c.get(x, y, z), "x={x} y={y} z={z}");
                }
            }
        }
    }

    #[test]
    fn from_bytes_accepts_legacy_and_masked_lengths_only() {
        assert!(Chunk::from_bytes(&[]).is_none());
        assert!(Chunk::from_bytes(&[0u8; 1]).is_none());
        assert!(Chunk::from_bytes(&[0u8; CHUNK_VOLUME * 2 - 1]).is_none());
        // Legacy pre-feature length (no mask) is still valid.
        assert!(Chunk::from_bytes(&[0u8; CHUNK_VOLUME * 2]).is_some());
        // A partial mask is rejected, not silently truncated.
        assert!(Chunk::from_bytes(&[0u8; CHUNK_VOLUME * 2 + 1]).is_none());
        // Full block array + full placed mask is the new canonical length.
        assert!(Chunk::from_bytes(&[0u8; CHUNK_VOLUME * 2 + PLACED_MASK_BYTES]).is_some());
        assert!(Chunk::from_bytes(&[0u8; CHUNK_VOLUME * 2 + PLACED_MASK_BYTES + 1]).is_none());
    }

    // ── Spec 06 §2.2 — placed mask ───────────────────────────────────

    #[test]
    fn placed_defaults_false() {
        let c = Chunk::new();
        assert!(!c.is_placed(0, 0, 0));
        assert!(!c.is_placed(15, 15, 15));
    }

    #[test]
    fn set_placed_then_is_placed() {
        let mut c = Chunk::new();
        c.set_placed(3, 4, 5, true);
        assert!(c.is_placed(3, 4, 5));
        assert!(!c.is_placed(3, 4, 6), "neighbour bit must stay clear");
        c.set_placed(3, 4, 5, false);
        assert!(!c.is_placed(3, 4, 5), "clearing must reset the bit");
    }

    #[test]
    fn placed_mask_membership_is_exact_across_every_voxel() {
        // Deterministic scatter, then confirm exact per-voxel membership —
        // catches any word/bit indexing error.
        let mut c = Chunk::new();
        let mut want = std::collections::HashSet::new();
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    if (x + y * 7 + z * 13) % 5 == 0 {
                        c.set_placed(x, y, z, true);
                        want.insert((x, y, z));
                    }
                }
            }
        }
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    assert_eq!(
                        c.is_placed(x, y, z),
                        want.contains(&(x, y, z)),
                        "x={x} y={y} z={z}"
                    );
                }
            }
        }
    }

    #[test]
    fn placed_mask_survives_bytes_roundtrip() {
        let mut c = Chunk::new();
        c.set(1, 2, 3, STONE);
        c.set_placed(1, 2, 3, true);
        c.set_placed(15, 0, 7, true);
        let bytes = c.as_bytes();
        assert_eq!(bytes.len(), CHUNK_VOLUME * 2 + PLACED_MASK_BYTES);
        let back = Chunk::from_bytes(&bytes).expect("roundtrip");
        assert!(back.is_placed(1, 2, 3));
        assert!(back.is_placed(15, 0, 7));
        assert!(!back.is_placed(0, 0, 0));
        assert_eq!(back.get(1, 2, 3), STONE);
    }

    #[test]
    fn from_bytes_legacy_without_mask_is_all_natural() {
        // A pre-feature save: exactly CHUNK_VOLUME*2 block bytes, no mask.
        let legacy = vec![0u8; CHUNK_VOLUME * 2];
        let c = Chunk::from_bytes(&legacy).expect("legacy decodes");
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    assert!(!c.is_placed(x, y, z), "legacy block must be natural");
                }
            }
        }
    }

    #[test]
    fn set_marks_mesh_dirty() {
        let mut c = Chunk::new();
        c.mesh_dirty = false;
        c.set(0, 0, 0, STONE);
        assert!(c.mesh_dirty, "set() must flag mesh dirty");
    }

    // ── maintained non-air count (O(1) `is_empty`) ───────────────────

    /// Tiny deterministic PRNG (xorshift64*) so the random-edit tests need no
    /// dependency and reproduce exactly.
    fn next(state: &mut u64) -> u64 {
        *state ^= *state >> 12;
        *state ^= *state << 25;
        *state ^= *state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn assert_count_exact(c: &Chunk, what: &str) {
        assert_eq!(c.non_air_count(), c.recount_non_air(), "{what}: maintained count != full recount");
        assert_eq!(c.is_empty(), c.recount_non_air() == 0, "{what}: is_empty != (recount == 0)");
    }

    #[test]
    fn non_air_count_matches_full_recount_after_random_edits() {
        let mut c = Chunk::new();
        assert_count_exact(&c, "new");
        let mut rng = 0x9E37_79B9_7F4A_7C15u64;
        for step in 0..20_000 {
            let r = next(&mut rng);
            let (x, y, z) = ((r & 15) as usize, ((r >> 4) & 15) as usize, ((r >> 8) & 15) as usize);
            // A third of the writes are AIR (X->AIR and AIR->AIR), the rest
            // non-air (AIR->X and X->Y), so every transition is exercised.
            let block = if (r >> 16) % 3 == 0 { AIR } else { 1 + ((r >> 20) % 40) as u16 };
            c.set(x, y, z, block);
            assert_eq!(c.get(x, y, z), block);
            if step % 97 == 0 {
                assert_count_exact(&c, &format!("step {step}"));
            }
        }
        assert_count_exact(&c, "after 20,000 random edits");
        // Dig it all out: back to empty, count back to zero.
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    c.set(x, y, z, AIR);
                }
            }
        }
        assert_eq!(c.non_air_count(), 0);
        assert!(c.is_empty());
        assert_count_exact(&c, "dug out");
    }

    #[test]
    fn non_air_count_covers_every_transition_and_the_full_volume() {
        let mut c = Chunk::new();
        c.set(3, 3, 3, AIR); // AIR -> AIR: no change
        assert_eq!((c.non_air_count(), c.is_empty()), (0, true));
        c.set(3, 3, 3, STONE); // AIR -> X
        assert_eq!((c.non_air_count(), c.is_empty()), (1, false));
        c.set(3, 3, 3, DIRT); // X -> Y: no change
        assert_eq!(c.non_air_count(), 1);
        c.set(3, 3, 3, STONE); // same block again: no change
        assert_eq!(c.non_air_count(), 1);
        c.set(3, 3, 3, AIR); // X -> AIR
        assert_eq!((c.non_air_count(), c.is_empty()), (0, true));
        // A completely solid chunk is CHUNK_VOLUME (4096) — fits the field.
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    c.set(x, y, z, STONE);
                }
            }
        }
        assert_eq!(c.non_air_count(), CHUNK_VOLUME);
        assert_count_exact(&c, "solid");
    }

    #[test]
    fn non_air_count_is_exact_after_from_bytes_and_a_byte_roundtrip() {
        let mut c = Chunk::new();
        let mut rng = 0xDEAD_BEEF_CAFE_F00Du64;
        for _ in 0..3_000 {
            let r = next(&mut rng);
            c.set((r & 15) as usize, ((r >> 4) & 15) as usize, ((r >> 8) & 15) as usize, ((r >> 16) % 50) as u16);
        }
        let back = Chunk::from_bytes(&c.as_bytes()).expect("roundtrip");
        assert_eq!(back.non_air_count(), c.non_air_count());
        assert_count_exact(&back, "from_bytes");
        // The legacy (no placed mask) layout counts the same way.
        let legacy = &c.as_bytes()[..CHUNK_VOLUME * 2];
        assert_count_exact(&Chunk::from_bytes(legacy).expect("legacy"), "from_bytes legacy");
        // An all-air payload is empty; an edit after loading keeps the count exact.
        let mut empty = Chunk::from_bytes(&[0u8; CHUNK_VOLUME * 2]).expect("all-air");
        assert!(empty.is_empty());
        empty.set(0, 0, 0, STONE);
        assert_count_exact(&empty, "edit after load");
        assert_eq!(empty.non_air_count(), 1);
    }
}
