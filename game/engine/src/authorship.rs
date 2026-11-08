//! Living world S2 — who built a block (spec `docs/superpowers/specs/2026-11-09-living-world-tier2.md` §2).
//!
//! The placed mask says that "a player" put a block somewhere; this table says
//! which player, for the blocks whose author is known. It lives on the machine
//! that owns the world (single-player, a lending host, a dedicated server): it
//! is saved with the world, never sent on the wire, and never shown per person.
//! Three readers: Commons protection (a block in a Commons can be broken only
//! by its author), bloom's built-together term `b`, and succession (S5).
//!
//! * **Identity.** [`AuthorKey`] is `Local(seat)` for a seat on this machine and
//!   `Npub(bech32)` for a verified joiner (matching `PlotOwner`). A guest has no
//!   key: its blocks are anonymous, like every block placed before this build.
//! * **Store.** [`Authorship`]: per chunk, a sparse map from the cell (u16) to an
//!   [`AuthorId`] (u16) into a world key table.
//! * **One clear funnel.** The author entry of a cell goes the moment its placed
//!   bit is cleared (`World::set_placed(.., false)`, which every break, piston
//!   move and falling block goes through). [`World::record_author`] is called
//!   only at the sites that know who is acting.
//! * **Saved** as one appended `WorldSave` field ([`SavedAuthorship`]) in a
//!   canonical order: keys sorted, chunks and cells sorted, so the same world
//!   always encodes (and digests) to the same bytes.
//! * **Plan starters.** A Plan build runs over many ticks and may span a reload,
//!   so the key of whoever started it is kept beside the build's anchor, in the
//!   same table and the same save record (not inside `ConstructionAnchorData`,
//!   which is saved positionally).

use ahash::AHashMap;
use serde::{Deserialize, Serialize};

use crate::chunk::CHUNK_SIZE;

/// Most recorded cells in one chunk (a chunk has 4096 cells, so this is every
/// one of them; the bound is for a record that claims more).
pub const MAX_ENTRIES_PER_CHUNK: usize = 4096;
/// Most keys in the world table. Past it a new author's placements are anonymous.
pub const MAX_KEYS: usize = 65_535;
/// Most distinct authors bloom counts in one region.
pub const MAX_COUNTED_AUTHORS: usize = 8;

/// Index of a key in the world table.
pub type AuthorId = u16;

/// A chunk's position, `(cx, cy, cz)`.
pub type ChunkPos = (i32, i32, i32);

/// Who placed a block.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AuthorKey {
    /// A seat on the machine that owns the world (the host, a split-screen seat).
    Local(u8),
    /// A verified joiner, by npub (bech32).
    Npub(String),
}

impl AuthorKey {
    /// The key of a Builder commission's commissioner: the alpha identity
    /// bridge writes `local-player-N` for a local seat, anything else is an
    /// npub. Empty means unknown.
    pub fn from_commissioner(commissioner: &str) -> Option<Self> {
        if commissioner.is_empty() {
            return None;
        }
        match commissioner.strip_prefix("local-player-") {
            Some(n) => n.parse::<u8>().ok().map(Self::Local),
            None => Some(Self::Npub(commissioner.to_string())),
        }
    }

    /// The key of a local seat.
    pub fn seat(pidx: usize) -> Option<Self> {
        u8::try_from(pidx).ok().map(Self::Local)
    }
}

/// The local cell index of block `(x, y, z)` in its chunk (the chunk's own
/// flat index: `x + 16 z + 256 y`).
#[inline]
pub fn cell_of(x: i32, y: i32, z: i32) -> u16 {
    let s = CHUNK_SIZE as i32;
    (x.rem_euclid(s) + s * z.rem_euclid(s) + s * s * y.rem_euclid(s)) as u16
}

/// The chunk holding block `(x, y, z)`.
#[inline]
pub fn chunk_of(x: i32, y: i32, z: i32) -> ChunkPos {
    let s = CHUNK_SIZE as i32;
    (x.div_euclid(s), y.div_euclid(s), z.div_euclid(s))
}

/// A cell index back to its local `(x, y, z)`.
#[inline]
pub fn local_of(cell: u16) -> (usize, usize, usize) {
    let c = cell as usize;
    (c % CHUNK_SIZE, c / (CHUNK_SIZE * CHUNK_SIZE), (c / CHUNK_SIZE) % CHUNK_SIZE)
}

/// The world's authorship table. Runtime form of [`SavedAuthorship`].
#[derive(Clone, Debug, Default)]
pub struct Authorship {
    keys: Vec<AuthorKey>,
    ids: AHashMap<AuthorKey, AuthorId>,
    chunks: AHashMap<ChunkPos, AHashMap<u16, AuthorId>>,
    /// Plan builds in progress: anchor position to the key of who started it.
    starters: AHashMap<(i32, i32, i32), AuthorId>,
}

impl Authorship {
    pub fn new() -> Self {
        Self::default()
    }

    /// Nothing recorded at all.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty() && self.starters.is_empty()
    }

    /// Forget everything (a world reset).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    fn intern(&mut self, key: &AuthorKey) -> Option<AuthorId> {
        if let Some(&id) = self.ids.get(key) {
            return Some(id);
        }
        if self.keys.len() >= MAX_KEYS {
            return None;
        }
        let id = self.keys.len() as AuthorId;
        self.keys.push(key.clone());
        self.ids.insert(key.clone(), id);
        Some(id)
    }

    /// Record `key` as the author of `cell` in `chunk`; `None` (a guest, an
    /// unknown placer) leaves the cell anonymous, replacing any earlier author.
    /// A full chunk or a full key table also leaves it anonymous.
    pub fn set(&mut self, chunk: ChunkPos, cell: u16, key: Option<&AuthorKey>) {
        let id = key.and_then(|k| self.intern(k));
        let Some(id) = id else {
            self.clear_cell(chunk, cell);
            return;
        };
        let map = self.chunks.entry(chunk).or_default();
        if map.len() >= MAX_ENTRIES_PER_CHUNK && !map.contains_key(&cell) {
            return;
        }
        map.insert(cell, id);
    }

    /// Drop the author of one cell (the clear funnel).
    pub fn clear_cell(&mut self, chunk: ChunkPos, cell: u16) {
        if let Some(map) = self.chunks.get_mut(&chunk) {
            map.remove(&cell);
            if map.is_empty() {
                self.chunks.remove(&chunk);
            }
        }
    }

    /// The author of `cell`, if recorded.
    pub fn get(&self, chunk: ChunkPos, cell: u16) -> Option<&AuthorKey> {
        let id = *self.chunks.get(&chunk)?.get(&cell)?;
        self.keys.get(id as usize)
    }

    /// Cells recorded in `chunk`.
    pub fn len_in_chunk(&self, chunk: ChunkPos) -> usize {
        self.chunks.get(&chunk).map_or(0, |m| m.len())
    }

    /// Total cells recorded (all chunks).
    #[cfg(test)]
    pub fn total(&self) -> usize {
        self.chunks.values().map(|m| m.len()).sum()
    }

    /// The recorded cells of one chunk, in no particular order.
    pub fn cells_in_chunk(&self, chunk: ChunkPos) -> impl Iterator<Item = (u16, AuthorId)> + '_ {
        self.chunks.get(&chunk).into_iter().flat_map(|m| m.iter().map(|(&c, &a)| (c, a)))
    }

    /// Drop every entry of `chunk` for which `keep(cell)` is false; returns how many went.
    pub fn retain_in_chunk(&mut self, chunk: ChunkPos, mut keep: impl FnMut(u16) -> bool) -> usize {
        let Some(map) = self.chunks.get_mut(&chunk) else { return 0 };
        let before = map.len();
        map.retain(|&cell, _| keep(cell));
        let gone = before - map.len();
        if map.is_empty() {
            self.chunks.remove(&chunk);
        }
        gone
    }

    /// Every chunk that holds an entry (unordered).
    pub fn chunk_positions(&self) -> Vec<ChunkPos> {
        self.chunks.keys().copied().collect()
    }

    /// Drop every entry of a chunk (it is gone from the world).
    pub fn drop_chunk(&mut self, chunk: ChunkPos) {
        self.chunks.remove(&chunk);
    }

    // ── Plan starters ────────────────────────────────────────────────

    /// Note who started the Plan build anchored at `anchor`.
    pub fn set_starter(&mut self, anchor: (i32, i32, i32), key: Option<&AuthorKey>) {
        match key.and_then(|k| self.intern(k)) {
            Some(id) => {
                self.starters.insert(anchor, id);
            }
            None => {
                self.starters.remove(&anchor);
            }
        }
    }

    /// Who started the build at `anchor`, if known.
    pub fn starter(&self, anchor: (i32, i32, i32)) -> Option<&AuthorKey> {
        self.keys.get(*self.starters.get(&anchor)? as usize)
    }

    /// The build at `anchor` is over (done or cancelled).
    pub fn clear_starter(&mut self, anchor: (i32, i32, i32)) {
        self.starters.remove(&anchor);
    }

    /// Drop starters whose anchor no longer exists.
    pub fn retain_starters(&mut self, mut anchor_exists: impl FnMut((i32, i32, i32)) -> bool) {
        self.starters.retain(|&a, _| anchor_exists(a));
    }

    // ── Readers ──────────────────────────────────────────────────────

    /// Distinct authors among the cells `live` accepts in the given chunks,
    /// counted up to `cap`. `live(chunk, cell)` says whether the cell still
    /// holds a placed block (so a stale entry never counts).
    pub fn distinct_authors(
        &self,
        chunks: impl IntoIterator<Item = ChunkPos>,
        cap: usize,
        mut live: impl FnMut(ChunkPos, u16) -> bool,
    ) -> usize {
        let mut seen: Vec<AuthorId> = Vec::with_capacity(cap);
        'outer: for chunk in chunks {
            let Some(map) = self.chunks.get(&chunk) else { continue };
            for (&cell, &id) in map {
                if seen.contains(&id) || !live(chunk, cell) {
                    continue;
                }
                seen.push(id);
                if seen.len() >= cap {
                    break 'outer;
                }
            }
        }
        seen.len()
    }

    // ── Saving ───────────────────────────────────────────────────────

    /// The canonical saved form: only referenced keys, sorted by key; chunks and
    /// cells sorted. `None` when there is nothing to keep, so a world without
    /// authorship saves exactly as it did before the field existed.
    pub fn to_saved(&self) -> Option<SavedAuthorship> {
        if self.is_empty() {
            return None;
        }
        let mut used: Vec<&AuthorKey> = Vec::new();
        let mut seen = vec![false; self.keys.len()];
        let mut note = |id: AuthorId| {
            if let Some(slot) = seen.get_mut(id as usize)
                && !*slot
            {
                *slot = true;
                used.push(&self.keys[id as usize]);
            }
        };
        for map in self.chunks.values() {
            for &id in map.values() {
                note(id);
            }
        }
        for &id in self.starters.values() {
            note(id);
        }
        used.sort_unstable();
        let remap = |id: AuthorId| -> AuthorId {
            let key = &self.keys[id as usize];
            used.binary_search(&key).expect("a used key is in the sorted table") as AuthorId
        };
        let mut chunks: Vec<(i32, i32, i32, Vec<(u16, AuthorId)>)> = self
            .chunks
            .iter()
            .map(|(&(cx, cy, cz), map)| {
                let mut cells: Vec<(u16, AuthorId)> = map.iter().map(|(&c, &id)| (c, remap(id))).collect();
                cells.sort_unstable();
                (cx, cy, cz, cells)
            })
            .collect();
        chunks.sort_unstable_by_key(|c| (c.0, c.1, c.2));
        let mut starters: Vec<(i32, i32, i32, AuthorId)> =
            self.starters.iter().map(|(&(x, y, z), &id)| (x, y, z, remap(id))).collect();
        starters.sort_unstable();
        Some(SavedAuthorship { keys: used.into_iter().cloned().collect(), chunks, starters })
    }

    /// The table a saved record describes. A present-but-invalid part (an
    /// unknown key index, a cell out of range, a repeated cell, a repeated
    /// key, more entries than a chunk may hold) is dropped with a warning:
    /// authorship is never a reason to refuse a load.
    pub fn from_saved(saved: &SavedAuthorship) -> Self {
        let mut out = Self::new();
        let mut table: Vec<Option<AuthorId>> = Vec::with_capacity(saved.keys.len().min(MAX_KEYS));
        for (i, key) in saved.keys.iter().enumerate() {
            if i >= MAX_KEYS {
                log::warn!("world.dat: the authorship key table is longer than {MAX_KEYS}; the rest is dropped");
                break;
            }
            if out.ids.contains_key(key) {
                log::warn!("world.dat: a repeated authorship key is dropped");
                table.push(out.ids.get(key).copied());
                continue;
            }
            table.push(out.intern(key));
        }
        let mut dropped = 0usize;
        for (cx, cy, cz, cells) in &saved.chunks {
            let pos = (*cx, *cy, *cz);
            for &(cell, author) in cells {
                let id = table.get(author as usize).copied().flatten();
                let ok = usize::from(cell) < CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE
                    && id.is_some()
                    && out.len_in_chunk(pos) < MAX_ENTRIES_PER_CHUNK
                    && out.get(pos, cell).is_none();
                match (ok, id) {
                    (true, Some(id)) => {
                        out.chunks.entry(pos).or_default().insert(cell, id);
                    }
                    _ => dropped += 1,
                }
            }
        }
        for &(x, y, z, author) in &saved.starters {
            match table.get(author as usize).copied().flatten() {
                Some(id) => {
                    out.starters.insert((x, y, z), id);
                }
                None => dropped += 1,
            }
        }
        if dropped > 0 {
            log::warn!("world.dat: {dropped} invalid authorship record(s) dropped");
        }
        out
    }
}

/// The saved authorship record (`WorldSave.block_authors`). Positional bincode:
/// a change to this type, or to [`AuthorKey`], is a `SAVE_LAYOUT_REVISION` bump.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedAuthorship {
    /// The keys, sorted. An author in `chunks` or `starters` is an index into it.
    pub keys: Vec<AuthorKey>,
    /// `(cx, cy, cz, [(cell, author)])`, sorted by chunk then cell.
    pub chunks: Vec<(i32, i32, i32, Vec<(u16, AuthorId)>)>,
    /// `(x, y, z, author)` of each Plan build's anchor, sorted.
    pub starters: Vec<(i32, i32, i32, AuthorId)>,
}

impl SavedAuthorship {
    /// Does any key name a seat on the machine that wrote the save?
    pub fn has_seat_keys(&self) -> bool {
        self.keys.iter().any(|k| matches!(k, AuthorKey::Local(_)))
    }

    /// For a handover's transfer copy: the source's seat 0 becomes its npub,
    /// and any other seat's blocks become anonymous (a split-screen seat has no
    /// proven identity to carry). Re-canonicalises.
    pub fn neutralise(&mut self, source_npub: &str) {
        if !self.has_seat_keys() {
            return;
        }
        let mut table = Authorship::from_saved(self);
        let local: Vec<AuthorKey> = table.keys.iter().filter(|k| matches!(k, AuthorKey::Local(_))).cloned().collect();
        let mapped = AuthorKey::Npub(source_npub.to_string());
        for key in local {
            let id = table.ids[&key];
            let to = (key == AuthorKey::Local(0)).then(|| table.intern(&mapped)).flatten();
            for map in table.chunks.values_mut() {
                match to {
                    Some(to) => map.values_mut().filter(|v| **v == id).for_each(|v| *v = to),
                    None => map.retain(|_, v| *v != id),
                }
            }
            match to {
                Some(to) => table.starters.values_mut().filter(|v| **v == id).for_each(|v| *v = to),
                None => table.starters.retain(|_, v| *v != id),
            }
        }
        table.chunks.retain(|_, m| !m.is_empty());
        *self = table.to_saved().unwrap_or_default();
    }
}

// ── The rules (pure) ─────────────────────────────────────────────────

/// The toast for a break refused because someone else built the block.
pub const SOMEONE_ELSE_BUILT_TOAST: &str = "Someone else built this.";

/// Bloom's built-together term `b` for `n` distinct authors: 0 for fewer than
/// two, else `1 - exp(-(n-1)/2)` (n capped at [`MAX_COUNTED_AUTHORS`]).
pub fn built_together(n: usize) -> f32 {
    let n = n.min(MAX_COUNTED_AUTHORS);
    if n < 2 {
        0.0
    } else {
        1.0 - (-((n - 1) as f32) / 2.0).exp()
    }
}

/// Is a block someone else's? The cell's recorded author is not `who`. An
/// unknown author is nobody's: free.
pub fn is_someone_elses(recorded: Option<&AuthorKey>, who: Option<&AuthorKey>) -> bool {
    match recorded {
        None => false,
        Some(author) => Some(author) != who,
    }
}

impl crate::GameState {
    /// The author key a local seat's placements are recorded under. `None` on a
    /// joined client: its copy of the world is the server's to record, and its
    /// own table stays empty.
    pub(crate) fn author_for_seat(&self, pidx: usize) -> Option<AuthorKey> {
        if self.joined() {
            return None;
        }
        AuthorKey::seat(pidx)
    }
}

impl crate::world::World {
    /// Record who placed the block at `(x, y, z)`; `None` leaves it anonymous.
    /// Call after the placement has set the placed bit, at a site that knows
    /// who is acting.
    pub fn record_author(&mut self, x: i32, y: i32, z: i32, key: Option<&AuthorKey>) {
        self.authors.set(chunk_of(x, y, z), cell_of(x, y, z), key);
    }

    /// The recorded author of the block at `(x, y, z)`, if its placed bit is
    /// still set (a stale entry never answers).
    pub fn author_at(&self, x: i32, y: i32, z: i32) -> Option<&AuthorKey> {
        if self.authors.is_empty() || !self.is_placed(x, y, z) {
            return None;
        }
        self.authors.get(chunk_of(x, y, z), cell_of(x, y, z))
    }

    /// Set a block, flag it player-placed and record its author.
    pub fn place_player_block_as(&mut self, x: i32, y: i32, z: i32, block: crate::block::BlockId, key: Option<&AuthorKey>) {
        self.place_player_block(x, y, z, block);
        self.record_author(x, y, z, key);
    }

    /// Would `who` breaking (or building over) the block at `(x, y, z)` be
    /// refused by Commons protection? True in a Commons footprint when the
    /// cell has a recorded author who is someone else. The creative bypass is
    /// the caller's.
    pub fn commons_protects(&self, x: i32, y: i32, z: i32, who: Option<&AuthorKey>) -> bool {
        if self.authors.is_empty() {
            return false;
        }
        if !is_someone_elses(self.author_at(x, y, z), who) {
            return false;
        }
        self.commons_areas().iter().any(|c| c.overlaps(x, x, z, z))
    }

    /// Distinct recorded authors of placed blocks in region `(cx, cz)` (a chunk
    /// column), up to [`MAX_COUNTED_AUTHORS`].
    pub fn authors_in_region(&self, cx: i32, cz: i32) -> usize {
        if self.authors.is_empty() {
            return 0;
        }
        let chunks = (0..=crate::world::MAX_CHUNK_Y).map(|cy| (cx, cy, cz));
        self.authors.distinct_authors(chunks, MAX_COUNTED_AUTHORS, |pos, cell| {
            let (lx, ly, lz) = local_of(cell);
            self.get_chunk(pos.0, pos.1, pos.2)
                .is_some_and(|c| c.is_placed(lx, ly, lz) && c.get(lx, ly, lz) != crate::block::AIR)
        })
    }

    /// Crash reconciliation, on a restore: chunk files and the world record are
    /// written separately, so an entry may point at a cell that is no longer
    /// placed (or at a chunk that is gone). Drop those. Also drops Plan
    /// starters whose build is not in the world.
    pub fn reconcile_authors(&mut self) {
        let mut dropped = 0usize;
        for pos in self.authors.chunk_positions() {
            let placed_cells: Option<Vec<u16>> = self.chunk_for_block_read(pos.0, pos.1, pos.2).map(|c| {
                self.authors
                    .cells_in_chunk(pos)
                    .filter(|&(cell, _)| {
                        let (lx, ly, lz) = local_of(cell);
                        c.is_placed(lx, ly, lz)
                    })
                    .map(|(cell, _)| cell)
                    .collect()
            });
            match placed_cells {
                None => {
                    dropped += self.authors.len_in_chunk(pos);
                    self.authors.drop_chunk(pos);
                }
                Some(mut keep) => {
                    keep.sort_unstable();
                    dropped += self.authors.retain_in_chunk(pos, |cell| keep.binary_search(&cell).is_ok());
                }
            }
        }
        let anchors = &self.construction_anchors;
        self.authors.retain_starters(|a| anchors.contains_key(&a));
        if dropped > 0 {
            log::warn!("world load: {dropped} authorship entr(ies) pointed at cells no longer placed; dropped");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::world::World;

    fn npub(s: &str) -> AuthorKey {
        AuthorKey::Npub(s.to_string())
    }

    #[test]
    fn cell_and_local_agree_and_match_the_chunk_index() {
        for &(x, y, z) in &[(0, 0, 0), (15, 15, 15), (-1, -1, -1), (17, 33, -20), (5, 255, 9)] {
            let cell = cell_of(x, y, z);
            let (lx, ly, lz) = local_of(cell);
            assert_eq!(
                (lx as i32, ly as i32, lz as i32),
                (x.rem_euclid(16), y.rem_euclid(16), z.rem_euclid(16)),
                "{x},{y},{z}"
            );
        }
        // The cell index is the chunk's own flat index.
        let mut w = World::new();
        w.set_block(3, 5, 7, block::STONE);
        let c = w.get_chunk(0, 0, 0).unwrap();
        assert_eq!(c.get(3, 5, 7), block::STONE);
        assert_eq!(cell_of(3, 5, 7) as usize, 3 + 7 * 16 + 5 * 256);
    }

    #[test]
    fn commissioner_strings_map_to_keys() {
        assert_eq!(AuthorKey::from_commissioner("local-player-0"), Some(AuthorKey::Local(0)));
        assert_eq!(AuthorKey::from_commissioner("local-player-3"), Some(AuthorKey::Local(3)));
        assert_eq!(AuthorKey::from_commissioner("local-player-x"), None);
        assert_eq!(AuthorKey::from_commissioner(""), None);
        assert_eq!(AuthorKey::from_commissioner("npub1abc"), Some(npub("npub1abc")));
    }

    #[test]
    fn set_get_replace_and_clear() {
        let mut a = Authorship::new();
        assert!(a.is_empty());
        a.set((0, 0, 0), 7, Some(&AuthorKey::Local(0)));
        assert_eq!(a.get((0, 0, 0), 7), Some(&AuthorKey::Local(0)));
        a.set((0, 0, 0), 7, Some(&npub("a")));
        assert_eq!(a.get((0, 0, 0), 7), Some(&npub("a")), "a later placer replaces");
        a.set((0, 0, 0), 7, None);
        assert_eq!(a.get((0, 0, 0), 7), None, "an anonymous placement clears");
        assert!(a.is_empty(), "the empty chunk map is gone");
        a.set((1, 2, 3), 9, Some(&AuthorKey::Local(1)));
        a.clear_cell((1, 2, 3), 9);
        assert!(a.is_empty());
    }

    #[test]
    fn a_full_key_table_and_a_full_chunk_leave_new_placements_anonymous() {
        let mut a = Authorship::new();
        for i in 0..MAX_KEYS {
            assert!(a.intern(&npub(&i.to_string())).is_some());
        }
        a.set((0, 0, 0), 1, Some(&npub("one-more")));
        assert_eq!(a.get((0, 0, 0), 1), None, "a 65,536th key is anonymous");
        a.set((0, 0, 0), 1, Some(&npub("3")));
        assert_eq!(a.get((0, 0, 0), 1), Some(&npub("3")), "an existing key still works");

        let mut b = Authorship::new();
        for cell in 0..MAX_ENTRIES_PER_CHUNK as u16 {
            b.set((0, 0, 0), cell, Some(&AuthorKey::Local(0)));
        }
        assert_eq!(b.len_in_chunk((0, 0, 0)), 4096);
    }

    #[test]
    fn the_saved_form_is_canonical_whatever_the_order_of_building() {
        let build = |order: &[(i32, u16, AuthorKey)]| {
            let mut a = Authorship::new();
            for (cx, cell, k) in order {
                a.set((*cx, 0, 0), *cell, Some(k));
            }
            a.set_starter((5, 6, 7), Some(&npub("zed")));
            a.to_saved().unwrap()
        };
        let one = [(0, 3, npub("zed")), (2, 9, AuthorKey::Local(1)), (0, 1, npub("amy")), (-1, 4, AuthorKey::Local(0))];
        let mut two = one.clone();
        two.reverse();
        assert_eq!(build(&one), build(&two), "same world, same bytes");
        let saved = build(&one);
        assert!(saved.keys.windows(2).all(|w| w[0] < w[1]), "keys sorted: {:?}", saved.keys);
        assert!(saved.chunks.windows(2).all(|w| (w[0].0, w[0].1, w[0].2) < (w[1].0, w[1].1, w[1].2)));
        assert!(saved.chunks.iter().all(|c| c.3.windows(2).all(|w| w[0].0 < w[1].0)), "cells sorted");
        let bytes = bincode::serialize(&saved).unwrap();
        assert_eq!(bytes, bincode::serialize(&build(&two)).unwrap());
    }

    #[test]
    fn a_save_round_trips_and_unused_keys_are_not_kept() {
        let mut a = Authorship::new();
        a.set((0, 0, 0), 1, Some(&npub("a")));
        a.set((0, 0, 0), 2, Some(&npub("b")));
        a.set_starter((1, 2, 3), Some(&AuthorKey::Local(2)));
        a.clear_cell((0, 0, 0), 2);
        let saved = a.to_saved().unwrap();
        assert_eq!(saved.keys, vec![AuthorKey::Local(2), npub("a")], "b has no cell left");
        let back = Authorship::from_saved(&saved);
        assert_eq!(back.get((0, 0, 0), 1), Some(&npub("a")));
        assert_eq!(back.starter((1, 2, 3)), Some(&AuthorKey::Local(2)));
        assert_eq!(back.to_saved().unwrap(), saved);
        assert!(Authorship::new().to_saved().is_none(), "nothing recorded saves as absent");
    }

    #[test]
    fn invalid_records_are_dropped_not_refused() {
        let saved = SavedAuthorship {
            keys: vec![AuthorKey::Local(0), npub("a"), AuthorKey::Local(0)],
            chunks: vec![
                (0, 0, 0, vec![(1, 0), (2, 9), (4096, 0), (3, 1), (3, 0)]),
                (1, 0, 0, vec![(5, 1)]),
            ],
            starters: vec![(1, 2, 3, 77), (4, 5, 6, 1)],
        };
        let t = Authorship::from_saved(&saved);
        assert_eq!(t.get((0, 0, 0), 1), Some(&AuthorKey::Local(0)));
        assert_eq!(t.get((0, 0, 0), 2), None, "unknown key index");
        assert_eq!(t.get((0, 0, 0), 3), Some(&npub("a")), "the first of a repeated cell stands");
        assert_eq!(t.len_in_chunk((0, 0, 0)), 2, "bad cell index 4096 and the repeat are dropped");
        assert_eq!(t.get((1, 0, 0), 5), Some(&npub("a")));
        assert_eq!(t.starter((1, 2, 3)), None);
        assert_eq!(t.starter((4, 5, 6)), Some(&npub("a")));
    }

    #[test]
    fn built_together_calibration() {
        assert_eq!(built_together(0), 0.0);
        assert_eq!(built_together(1), 0.0, "one builder is never together");
        assert!((built_together(2) - 0.3935).abs() < 1e-3);
        assert!((built_together(3) - 0.6321).abs() < 1e-3);
        assert!((built_together(5) - 0.8647).abs() < 1e-3);
        assert!(built_together(8) > 0.96);
        assert_eq!(built_together(8), built_together(100), "capped at eight");
        let mut last = 0.0;
        for n in 0..=8 {
            let b = built_together(n);
            assert!(b >= last && (0.0..1.0).contains(&b), "{n}: {b}");
            last = b;
        }
    }

    #[test]
    fn unknown_author_is_free_and_own_is_free() {
        let me = AuthorKey::Local(0);
        assert!(!is_someone_elses(None, Some(&me)));
        assert!(!is_someone_elses(None, None));
        assert!(!is_someone_elses(Some(&me), Some(&me)));
        assert!(is_someone_elses(Some(&me), Some(&npub("x"))));
        assert!(is_someone_elses(Some(&me), None), "a guest cannot break an authored block");
    }

    #[test]
    fn the_clear_funnel_drops_the_author_and_a_stale_entry_never_answers() {
        let mut w = World::new();
        w.place_player_block_as(3, 70, 3, block::STONE, Some(&AuthorKey::Local(0)));
        assert_eq!(w.author_at(3, 70, 3), Some(&AuthorKey::Local(0)));
        w.set_placed(3, 70, 3, false);
        assert_eq!(w.author_at(3, 70, 3), None);
        assert!(w.authors.is_empty(), "the funnel dropped the entry itself");
        // A natural block has no author even if one is recorded by mistake.
        w.set_block(4, 70, 3, block::STONE);
        w.record_author(4, 70, 3, Some(&AuthorKey::Local(0)));
        assert_eq!(w.author_at(4, 70, 3), None);
    }

    #[test]
    fn reconciliation_drops_entries_that_are_not_placed_and_orphan_starters() {
        let mut w = World::new();
        w.place_player_block_as(3, 70, 3, block::STONE, Some(&AuthorKey::Local(0)));
        w.place_player_block_as(5, 70, 3, block::STONE, Some(&npub("a")));
        // A crash left an entry for a cell whose bit is clear (the chunk file is older than the record).
        w.authors.set(chunk_of(6, 70, 3), cell_of(6, 70, 3), Some(&npub("a")));
        // And one for a chunk that is not there at all.
        w.authors.set((40, 0, 40), 5, Some(&npub("a")));
        w.authors.set_starter((9, 9, 9), Some(&npub("a")));
        w.reconcile_authors();
        assert_eq!(w.authors.total(), 2);
        assert_eq!(w.author_at(3, 70, 3), Some(&AuthorKey::Local(0)));
        assert_eq!(w.author_at(5, 70, 3), Some(&npub("a")));
        assert_eq!(w.authors.starter((9, 9, 9)), None, "no build there");
    }

    #[test]
    fn distinct_authors_counts_only_live_placed_blocks_and_caps() {
        let mut w = World::new();
        for i in 0..12 {
            w.place_player_block_as(i, 70, 1, block::STONE, Some(&npub(&format!("k{i}"))));
        }
        assert_eq!(w.authors_in_region(0, 0), MAX_COUNTED_AUTHORS, "capped");
        let mut w = World::new();
        w.place_player_block_as(1, 70, 1, block::STONE, Some(&npub("a")));
        w.place_player_block_as(2, 70, 1, block::STONE, Some(&npub("a")));
        assert_eq!(w.authors_in_region(0, 0), 1);
        w.place_player_block_as(40, 70, 1, block::STONE, Some(&npub("b")));
        assert_eq!(w.authors_in_region(0, 0), 1, "the other region is not this one");
        w.place_player_block_as(3, 200, 1, block::STONE, Some(&npub("b")));
        assert_eq!(w.authors_in_region(0, 0), 2, "any height of the column counts");
        w.set_block(3, 200, 1, block::AIR);
        assert_eq!(w.authors_in_region(0, 0), 1, "a block burnt away leaves no author behind");
    }

    #[test]
    fn neutralise_maps_seat_zero_and_anonymises_other_seats() {
        let mut a = Authorship::new();
        a.set((0, 0, 0), 1, Some(&AuthorKey::Local(0)));
        a.set((0, 0, 0), 2, Some(&AuthorKey::Local(1)));
        a.set((0, 0, 0), 3, Some(&npub("friend")));
        a.set_starter((1, 1, 1), Some(&AuthorKey::Local(0)));
        let mut saved = a.to_saved().unwrap();
        saved.neutralise("npub1me");
        assert!(!saved.has_seat_keys());
        let t = Authorship::from_saved(&saved);
        assert_eq!(t.get((0, 0, 0), 1), Some(&npub("npub1me")));
        assert_eq!(t.get((0, 0, 0), 2), None);
        assert_eq!(t.get((0, 0, 0), 3), Some(&npub("friend")));
        assert_eq!(t.starter((1, 1, 1)), Some(&npub("npub1me")));
    }
}
