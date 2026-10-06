# Old-save data integrity — backward-compatible `WorldSave` loading

**Status: DECISION TAKEN → BUILDING 2026-06-03.** Foundation doc for **Goal 3,
Task 1** (`docs/backlog/2026-06-03-tier4-goal-3-engine-hardening.md`), graduated from
owner-inbox **"Engine hardening — found by the wallpaper audit"** item #1
(`docs/backlog/owner-inbox.md` §"Old-save data loss on any `WorldSave` field-add").

The backlog flagged the save-versioning **approach** as "partly a product decision —
write a foundation doc presenting the options + a recommendation FIRST; if the choice is
a genuine product fork (invest in a versioning scheme vs. document-the-limitation),
surface it and STOP rather than guess." This doc is that artifact. **Conclusion up
front: it is _not_ a genuine product fork** — the spec already mandates the outcome and
the "document-the-limitation" option is factually broken — so the doc records the
decision and proceeds. The reasoning is below for the owner to veto if they disagree.

---

## TL;DR

`WorldSave` is a plain `#[derive(Serialize, Deserialize)]` bincode struct with **no
version envelope** (`save.rs:41-163`, 33 fields, newest = `face_overlays` #33). bincode 1
is **positional and non-self-describing**: appending a field (which has happened 25+
times) lengthens the stream at the tail, so a save written by an **older** engine is
**shorter** than a **newer** engine expects. On load the newer engine reads the old
fields, hits **EOF** on the first appended field, the `bincode::deserialize::<WorldSave>`
returns `Err`, and a catch-all `Err(_)` arm falls back to the 8-field `LegacyWorldSave`,
whose `upgrade()` **hard-codes every block-entity Vec to empty**. Net result on that
one-way upgrade load: **all** block-entities (chests + their items, furnaces, vendors,
**tip-jar escrow holding real sats**, plots, villages, raids, bounties, auctions, latent
prints, face-overlay wallpaper) + extended per-player fields are silently dropped. Player
position / health / inventory / hotbar survive.

**The `#[serde(default)]` on fields 8-33 does _nothing_ here** — serde only honours
`default` when the format reports a field *absent by name*, which a self-describing
format (JSON) can do but bincode cannot. On bincode it just hits EOF and errors. The
in-tree comment at `protocol.rs:568` already states this: *"bincode v1 cannot honour that
on missing trailing bytes — old saves … won't load; accepted pre-launch."*

**Decision: FIX IT, minimally.** Replace the all-or-nothing decode with a **tolerant
positional decoder** that reads `WorldSave` field-by-field and, on hitting EOF for any
**trailing** field, fills the remainder with `Default`. This recovers maximal data from
any append-only-older save, changes **nothing on the write side** (zero on-disk format
change, zero detection ambiguity), and protects local **and** cloud saves in one place
(they are the same code path). It is the "Option-tail the trailing fields" mechanism the
inbox itself suggested, and it is the least-invasive option that closes the gap.

---

## Why this is not a genuine product fork

The backlog framed the choice as **"invest in a versioning scheme vs. document the
limitation + lean on Stash."** Three verified facts collapse that fork:

1. **Spec §8.4 already mandates the outcome.** `docs/spec/02-world-format.md §8.4
   Format Versioning`: *"The engine MUST be able to read any format version ≤ its
   built-in version."* "Document the limitation" (let old saves lose data) **directly
   contradicts the spec.** §8.4 even names `#[serde(default)]` as the intended mechanism
   for minor field-adds — the code _tried_ to follow the spec (every appended field has
   `#[serde(default)]`) but the mechanism is inert on bincode. The fix makes the code
   deliver what the spec already promises; it is not a new product direction.

2. **Cloud/Stash shares the identical bug — "lean on Stash" is self-defeating.**
   Verified end-to-end (`save.rs:977` pack → `save.rs:994-1010` the cloud copy is literally
   `compressed.clone()` of the local blob → `wasm_save.rs:144` `bincode::serialize(save)`;
   restore: `cloud_restore_wasm` returns raw bytes → the same single `unpack_world`
   (`wasm_save.rs:213-221`) with the same `Err(_) → LegacyWorldSave → upgrade()` fallback).
   There is exactly **one** serializer and **one** deserializer for both local and cloud.
   Stash stores the very same versionless bincode blob, so a cloud-only copy suffers the
   identical silent loss the moment a newer engine adds a field and re-loads. Stash gives
   byte-durability, **not** format-version safety. So option (iii) protects neither local
   nor cloud.

3. **Real value is at stake, now.** Tip-jar escrow holds **real sats**; the PWA alpha
   (Chromium) is going live (5 Jun 2026, ahead of BTC Prague). Reachable whenever an
   existing world is opened by a newer engine — i.e. every future build that adds a
   `WorldSave` field, which is the established pattern.

Given a spec mandate, a factually-broken alternative, and real money at risk, the
"recommendation" is forced. Per the autonomy posture (merge-to-main pre-authorised on a
green gate; push to the playtest boundary), I proceed with the fix and record it here
rather than blocking. **Owner veto point:** if you actually want to ship known old-save
data loss for alpha, say so and I'll revert Task 1 to a documented limitation — but note
that abandons cloud saves too.

---

## The bug, precisely (verified against live code, 2026-06-03)

- **Struct:** `WorldSave`, `save.rs:41-163`. Derive `#[derive(Serialize, Deserialize)]`,
  no container attrs. Fields 1-7 (`seed`, `player_x/y/z`, `player_health`, `hotbar_slot`,
  `inventory: Vec<SavedSlot>`) have no serde attr; fields 8-33 each carry
  `#[serde(default)]`. Newest field `face_overlays: Vec<SavedFaceOverlay>` (#33, line 162).
- **Write (one logical format, three call sites):** `bincode::serialize(&save)` —
  `save.rs:812` (`save_world`), `save.rs:1758` (`autosave_world`), `wasm_save.rs:144`
  (`pack_world`, tar+gzip-wrapped). Default bincode 1 config: little-endian, fixint, **no
  magic / version / length framing** on the record. The only "version" is
  `WorldMeta.version` in the separate `world_meta.json` — a save-counter, never read on
  the deserialize path.
- **Read (three sites, identical shape):** `load_world` `save.rs:1025-1034`, `load_autosave`
  `save.rs:1812-1819`, WASM/cloud `unpack_world` `wasm_save.rs:213-221`. Each: try
  `bincode::deserialize::<WorldSave>`; on **any** `Err` (the kind is discarded — EOF and
  genuine corruption are not distinguished) decode as `LegacyWorldSave` then `.upgrade()`.
- **Loss:** `LegacyWorldSave` (`save.rs:399-409`) is the original 8 fields; `upgrade()`
  (`save.rs:421-492`) sets every later Vec to `Vec::new()` and the post-legacy per-player
  fields to defaults. Note `LegacyWorldSave` is a strict **prefix of the _original_**
  `WorldSave`. Pre-fix, feeding the legacy decoder a "modern-minus-newest-field" stream is
  lossy in one of two ways, because bincode's top-level `deserialize` **tolerates trailing
  bytes**: with an **empty / aligned** primary inventory the legacy decode succeeds and
  returns **`Ok` lossily** — the 8 legacy fields read, the trailing block-entity bytes are
  silently ignored, and `upgrade()` yields a world with `chests = Vec::new()` (a **silent**
  data drop); with a **non-empty** modern inventory the per-element enum tags misalign, the
  legacy decode **errors**, and `load_world` fails the load outright. Either way data
  integrity is lost.

---

## Options considered

| # | Option | Closes local? | Closes cloud? | On-disk change | Verdict |
|---|--------|---------------|---------------|----------------|---------|
| i | `u32` SaveVersion magic + branch the deserialiser | Only **future** saves; **existing** magic-less saves still EOF unless _also_ tolerant-decoded | same | yes (new writes get a header) | **Partial** — needs (ii) anyway for existing saves; magic-vs-legacy detection is ambiguous (see below) |
| ii | **Tolerant positional decode** (Option-tail: default trailing fields on EOF) | **Yes — all existing + future append-only saves** | **Yes** (same code path) | **none** | **CHOSEN** |
| iii | Document the limitation + lean on Stash | No | **No** (Stash shares the bug) | none | **Rejected** — contradicts spec §8.4; cloud not saved |

### Why not the write-side version magic (option i), now

A `u32` magic+version prepended to *new* writes does **not** help the saves already on
disk (they have no magic). To recover those you need the tolerant decode regardless — so
(i) is strictly *additional* work on top of (ii), not an alternative. Worse, detecting
"is this a versioned save?" is **ambiguous**: an old magic-less save begins with
`seed: u32`, so any 4-byte magic can collide with a real `seed` (1/2³²), a silent-
corruption mode unacceptable for a "production-grade" save path. An explicit version
envelope is the **right** move — but only **when a non-append change first happens**
(field reorder / removal / retype), which is exactly when a migration is unavoidable and
a version discriminator earns its keep. Until then it is "redesign beyond what closes the
gap," which the backlog explicitly rules out (YAGNI). **Deferred, not rejected** — see
spec note below.

---

## Chosen mechanism — tolerant positional decode

A single free function in `save.rs`, called at all three read sites (replacing the
`bincode::deserialize::<WorldSave>` first-try; the `LegacyWorldSave` fallback stays for
genuinely pre-item-era saves):

```rust
/// Read a `WorldSave` from a bincode stream, tolerating a stream that ends early
/// because it was written by an OLDER engine (fewer trailing fields). Required fields
/// (1-7) must be present; every appended field (8-33, all `#[serde(default)]`) defaults
/// to empty if the stream ends at its boundary. Any field whose bytes are present but
/// don't decode — incl. a mid-field EOF from a misaligned legacy/corrupt stream —
/// propagates so the caller can fall back to LegacyWorldSave.
fn deserialize_world_save_tolerant(data: &[u8]) -> Result<WorldSave, bincode::Error> {
    let mut cur = std::io::Cursor::new(data);
    // read_tail::<T>() reads one field, returning T::default() ONLY if the cursor is
    // already at a clean field boundary (older writer stopped here); if bytes remain it
    // decodes and propagates any error (incl. a mid-field EOF = legacy/corrupt shape).
    Ok(WorldSave {
        seed:           bincode::deserialize_from(&mut cur)?,   // 1-7 required
        player_x:       bincode::deserialize_from(&mut cur)?,
        /* …player_y, player_z, player_health, hotbar_slot, inventory… */
        players:        read_tail(&mut cur)?,                   // 8-33 default at clean EOF
        /* …all appended fields… */
        face_overlays:  read_tail(&mut cur)?,
    })
}
```

**Key properties**

- **Recovers maximal data** from any append-only-older save: fields present are read;
  the first missing trailing field and everything after default to empty. The chest,
  plot, tip-jar escrow, etc. that the old engine *did* write are preserved.
- **Self-maintaining at compile time.** The struct literal lists **all 33 fields**, so
  adding field #34 to `WorldSave` without updating the decoder is a **compile error** —
  the decoder can never silently drift out of sync (unlike the hand-maintained
  `LegacyWorldSave::upgrade()`).
- **Routes genuine legacy / corrupt streams to the fallback.** `read_tail` defaults a
  field **only** when the cursor is already at a clean end-of-stream (the older writer
  stopped at this exact boundary). If bytes **remain**, the field is present: it decodes
  and propagates **any** error — including a **mid-field** `UnexpectedEof`. That
  distinction is load-bearing: a genuine legacy multi-player save with an empty primary
  inventory reads the strict prefix cleanly, then its misaligned legacy `players` bytes
  run off the end mid-decode; propagating that EOF (rather than swallowing it) makes the
  tolerant decode fail so the `LegacyWorldSave` fallback recovers the players. (This was a
  real regression caught by the branch review — pinned by a dedicated test.)
- **Zero write-side / on-disk change**, so no magic-collision risk and full
  cross-version interop with saves already in the wild (local **and** cloud).

**Touch points:** `deserialize_world_save_tolerant` + `read_tail` in `save.rs`; called at
`save.rs:1025`, `save.rs:1812`, `wasm_save.rs:214`. (Task 2 factors the *restore/apply*
step into `apply_world_save_state`; this task DRYs the *decode* step into
`read_world_save`/`deserialize_world_save_tolerant`. After both, every site reads
`let save = read_world_save(&data)?; apply_world_save_state(&mut world, &save);`.)

---

## Test plan — the regression that proves it

The existing round-trip tests **never exercise the EOF→legacy path** (they all serialise
and deserialise the *same-engine* `WorldSave`, so the first decode always succeeds). The
new test must construct a genuinely-older byte stream:

1. Build a **real** current-format `WorldSave` carrying a **chest** (a block-entity the
   legacy path would drop), with an empty `face_overlays`. Using the real struct — not a
   hand-replicated mirror — guarantees the canonical bincode field order/types.
2. `bincode::serialize` it, then **drop the trailing 8-byte empty-`face_overlays` length**
   to mint the on-disk shape of a pre-2026-06-03 save (self-verified: assert those 8 bytes
   are zero first). Write to `save::world_dir(name)/world.dat`, call the real path-based
   `save::load_world(name, &mut world)`.
3. Assert the **chest survives** — `loaded.chests.len() == 1` and
   `world.chest_at((2,70,3)).is_some()` — i.e. it was **not** routed to
   `LegacyWorldSave::upgrade()` (which hard-codes `chests = Vec::new()`), and
   `face_overlays` defaulted to empty without erroring.

On **current** code (the empty-primary-inventory fixture) the legacy fallback succeeds
**lossily**, so the test goes red via the `chests.len() == 1` assertion (it gets `0` — a
**silent** chest drop), not via a panic. After the fix it **passes**. Classic red→green.

A second regression test (added after the branch review) pins the inverse: a **genuine
legacy multi-player save with an empty primary inventory** must fall back to
`LegacyWorldSave` and keep its extra players — the tolerant decode must NOT swallow the
misaligned-`players` EOF and silently drop them (see the `read_tail` clean-boundary rule
above). A third locks the `load_autosave` wiring of the tolerant decode. All live in
`test_integration/save_load.rs` (native-only, already registered).

---

## Spec updates (done as part of Task 1)

- **`docs/spec/02-world-format.md §8.4 Format Versioning`** — correct the claim that
  `#[serde(default)]` handles minor field-adds; it is **inert on the bincode prototype
  format**. Document the tolerant-positional-decode as the alpha mechanism that delivers
  §8.4's "MUST read older versions" mandate for append-only changes, and record that an
  explicit `u32` version envelope (App-B-style magic) is the prescribed mechanism for the
  first **non-append** change / the production region-file format.
- **§"Current Implementation (Step 11 — Prototype)"** — note that old-save load is now
  backward-compatible for appended fields.

---

## Out of scope (do not do)

- No write-side magic / version header now (deferred to the first non-append change — see
  option i). *Superseded 2026-10-06: a version FOOTER shipped — see below.*
- No migration to the production rkyv region-file format (`§4.4`) — that is the
  multiplayer-era rebuild, not this hardening pass.
- No touching the shipped+audited wallpaper feature; no redesign of the save format
  beyond the tolerant decode.

---

## Review follow-ups (2026-06-03)

A multi-agent adversarial review of the shipped branch surfaced issues beyond the
original three; all resolved on top of the merge:

- **Torn-write silent loss (the real-sats one).** `world.dat` was a plain `fs::write`
  with no atomicity and no CRC, and the tolerant decode can't tell a cleanly-shorter OLD
  save from a *truncated NEW* one — so a crash mid-write could silently "load" with the
  tail (incl. tip-jar escrow) defaulted away, and `load_autosave` then deletes the
  autosave. **Fixed** by writing `world.dat` atomically (temp → `fsync` → rename;
  `save::write_atomic`), so torn writes are impossible at the source. (Spec §8.2.)
- **`raid_kills` written but never restored** on any load path — the per-(village, player)
  raid-defender leaderboard reset on every reload (leaderboard display only, not sats).
  **Fixed** with the inverse restore in `apply_world_save_state` (one line repairs all
  three load paths) + a load-path test.
- **Decoder test coverage** — `deserialize_world_save_tolerant`/`read_tail` were only hit
  end-to-end through `load_world`. Added direct unit tests (full-save identity,
  clean-boundary default, mid-field-EOF propagation) + a `load_autosave` recovery test,
  and replaced a mis-named toothless trailing-bytes test with a real forward-compat one.
- **Forward-compat downgrade** was flagged as a `BRIDGE` on `read_world_save` (a newer
  save's extra fields are dropped on re-save). **Closed 2026-10-06 (gap-audit T1-7)** —
  see "Update: the format-version footer" below.
- **Write-side drift** is already compile-time-safe — every `WorldSave` builder uses an
  exhaustive literal (no `..`) and the struct has no `Default`, so a new field cannot be
  added without updating every builder *and* the tolerant decoder. A DRY save-side
  collector was considered and deliberately **not** built (pure maintainability, not a
  correctness fix — the refactor risk isn't justified on the real-sats path).

---

## Update 2026-10-06: the format-version footer (gap-audit T1-7)

The forward-compat BRIDGE is replaced. Every `world.dat` is now
`bincode(WorldSave) || format_version: u32 LE || b"AXSAVEv1"` (`save_format.rs`), and
a build refuses — clearly, without writing — a save whose version is newer than its own
`SAVE_FORMAT_VERSION` (52 = the `WorldSave` field count, plus a hand-bumped layout
revision for nested-type changes).

Why this does not reopen the "Why not option i" objections above:

- **It is a footer, not a header.** Option i prepended a magic, which (a) would make
  every new save unreadable to the builds already shipped and (b) collides with a
  legacy save's leading `seed: u32`. A footer sits after the last field. The tolerant
  decoder every shipped build runs ignores trailing bytes (verified by the test
  `pre_footer_decoder_reads_a_footer_bearing_save_unchanged`), so shipped builds keep
  opening new saves exactly as before, and the 8-byte magic sits at the very end of the
  file, where a footer-less save has no realistic chance of matching it.
- **The tolerant decode stays.** Footer-less saves (everything written before
  2026-10-06) take the same path as before; a footer-bearing save has the footer
  stripped first, then the same decode.

What a refusal does: the lobby card is labelled "(needs a newer version)", Play / Host /
Workshop / Trials / online host stay in the lobby with *"This world was saved by a newer
version of Axe'n'Stax. Update the game to open it."*, the dedicated server exits at boot,
and every native writer (save, autosave, server save, meta) refuses the folder before
touching it. No quarantine rename, no rebuilt meta. Full description: Spec 02 §8.4.

Still open: a real migration for the first non-append change to `WorldSave` and for the
bincode 1 → 3 move, which will key on this version.

## Update 2026-10-06: a world that fails to load is never replaced

The footer closed the "newer build" hole; the same review found the wider one beneath
it. On the native client and the dedicated server, ANY load failure only logged a
warning and generated a fresh world, which was then marked live and saved — the save
deleted every chunk file that was all-air in the fresh world, overwrote the spawn-area
chunks and `world.dat`, and wrote the meta last, so a damaged-meta refusal protected
nothing. Triggers included an I/O error on `world.dat` or the meta (permissions after a
restore), an undecodable `world.dat`, a chunk file whose name isn't three integers, a
chunk read error or failed quarantine rename, and unparseable meta whose quarantine
failed. An unreadable `world.dat` also read as "no footer", so it was neither refused
nor guarded.

Now (`world_open::open_world`, Spec 02 §8.4 "The load-failure rule"):

- **"Nothing saved here" is told apart from "saved but failed to load".** On disk =
  `world.dat`, `autosave/world.dat` or a `chunks/*.chunk`; a meta alone is a new world.
  A world on disk that fails to load is refused — nothing generated, never marked live,
  nothing written. The client goes back to the lobby with *"This world couldn't be
  opened: <reason>. Nothing was changed."*; the dedicated server exits before its tick-0
  save, naming the folder, the file and why.
- **Loads are all or nothing**, so a refused folder is byte-for-byte unchanged (tested
  by hashing the folder before and after, for every trigger above). The keep-a-copy-aside
  recoveries (torn chunk, torn meta, partly decoded `world.dat`) still open the world —
  they lose nothing.
- **A failed autosave falls back to the last save**, with the damaged autosave renamed
  to `autosave.corrupt-<ts>` so nothing later deletes it; the player is told which copy
  they got.
- **A save deletes only chunk files it read or wrote**, and checks the meta before it
  touches anything.
- **An unreadable `world.dat` is refused** (`WorldSaveError::Unreadable`), like a newer
  one.
- **Web**: the play path refuses a damaged chunk inside the IndexedDB / cloud blob
  instead of skipping it (the next save would have repacked the record without it), and
  every load failure shows the notice.

Known, not fixed here: ~~a successful autosave recovery clears the autosave at once (a
second crash within five minutes loses the recovered progress)~~ (fixed below); autosave
recovery never reads the main `chunks/`, so a chunk mined to all-air before the autosave
regenerates as terrain; a LAN host's server reads `world.dat` while its client prefers
the autosave.

## Update 2026-10-06: review follow-ups on the save, load and exit paths

An independent review of the two changes above found these, all fixed (Spec 02 §8.4):

- **A failed save was silent.** `save_world` now refuses a damaged meta or unreadable
  `world.dat` up front, but Pause → Save then cleared the crash-recovery autosave anyway,
  and a failed Save & Quit only logged and left — the session was lost with no word.
  Now the autosave is cleared only once a save landed, every failure shows *"Couldn't
  save: <reason>. Your last save is safe."*, and `leave_world` returns whether the
  player left: a failed save on any exit that saves (Save & Quit, Trial Leave, end
  cards, the arena and skin-paint hops, the window close) keeps the player in the world.
- **Imports could write into a folder the lobby doesn't list.** The `.axeprofile`
  import named against the lobby list (folders with a `world.dat`, minus the Workshop),
  so it could write over the native Workshop or a chunks-only / autosave-only folder.
  Imports now name against every entry under the worlds root and refuse an existing
  folder outright.
- **Trial arenas were planned from the lobby list.** A Resume arena with chunks but no
  `world.dat` got a fresh meta written over its real one before the open refused it; a
  Reuse arena in that state was refused on every launch. `world_open::plan_arena_folder`
  now plans from the disk (`is_new_world`) before anything is written.
- **Reading a world wrote it.** The torn-meta recovery ran on every read (lobby list,
  `load_world_meta`, server boot, the top of `open_world`), so a world then refused for
  another reason had already changed. Readers now use the read-only `peek_world_meta`;
  the repair runs last, once the world has loaded.
- **A first save cut short was refused forever.** Chunks were written before
  `world.dat` even on a world's first save, so a dedicated server killed during its
  tick-0 save left chunks without a `world.dat`. This pass wrote `world.dat` first;
  the second review (below) found that still left holes, and replaced it with a
  staged first save. The refusal message says how to recover a folder already in
  the chunks-without-`world.dat` state.
- **The dedicated server never deleted a mined-out chunk's file**, so mined-out chunks
  came back after a restart. It now applies the client's rule (read or written this
  session, now all-air).
- **Opening from the autosave deleted it at once.** With a damaged `world.dat` that was
  the only good copy. It is now kept until a save lands.

## Update 2026-10-06: second review of the follow-ups

A second independent review of the follow-ups above found these, all fixed (Spec 02
§8.4; the red run before the fix failed the six new disk tests):

- **A failed save led the player to delete the autosave.** After a failed save, "Quit
  without saving" (`SaveChoice::Discard`) cleared the crash-recovery autosave, which
  could be the newest copy of the session there was; so did quitting a session that
  opened from the autosave. `world_exit::SessionSaves` (`save_failed`,
  `opened_from_autosave`, `close_save_failed`) now makes a discard keep the autosave
  while either of the first two is set, and the pause menu says so — *"Quit — your
  autosave from <age> is kept"* — instead of promising a discard. A failed
  window-close save keeps the player in the world with a hint to close again, and
  the second close quits without saving (now: tries the save once more first — see the
  third review, below), keeping the autosave, so a save that keeps
  failing never traps them. A save that lands resets it all.
- **The "write `world.dat` first" rule still left holes.** Every loader treats a
  column with any saved chunk as loaded and never generates the rest, so a first save
  cut short after some of its chunks (written one by one, in hash-map order) left
  permanent holes under a perfectly valid `world.dat`. A first save now stages its
  chunks in `chunks.new/`, writes `world.dat` as the commit point, then renames
  `chunks.new/` to `chunks/` (`save::write_first_save`). Cut short before the commit
  the world is still new; cut short after it, the next load or save finishes the
  publish (`finish_staged_first_save`). A stray `chunks/` already in the way is kept
  aside as `chunks.corrupt-<ts>`, never deleted. Tests cut a first save short at every
  chunk boundary and just before the publish, for the client, the server and an import.
- **A failed import left its half-written folder behind**, which the lobby listed once
  `world.dat` was in it: a world with chunks missing. An import that fails now removes
  the folder it created.
- **A conjured empty chunk deleted a real chunk file.** `World::set_block` creates an
  empty chunk for a cell whose column the streamer had dropped; the next save read
  that all-air chunk as "mined out" and deleted the file under it. Deletion now also
  requires the chunk to be `persist` (read from disk or really edited), on the client
  and the server.
- **Scenario and replay saves left a stale autosave behind.** A resumable scenario's
  start and the replay snapshot saved without dropping the autosave they superseded,
  and the loader prefers an autosave, so a crash rolled the fresh save back. Every
  save of the live session now goes through `save::save_world_superseding_autosave`.

Known, not fixed here — needs a design call (a manifest, or a loader that can tell a
complete column from a partial one): only a world's FIRST save is all-or-nothing. A
LATER save cut short can still leave a partial NEW column (the world-generation chunks
of a column first saved in that save), the same hole class. And `World::set_block`
still auto-creating a chunk means a non-air write into a column the streamer had
dropped can overwrite the real file (known debt in the project notes).

## Update 2026-10-06: third review of the save follow-ups

A third review found these, all fixed (Spec 02 §8.4; the red run before the fix failed
the seven new disk tests):

- **A save that failed after its commit rolled itself back.** When a save failed AFTER
  `world.dat` was renamed into place (the meta write, or a first save's publish), the
  live session kept the older crash-recovery autosave — and the next open preferred any
  autosave over `world.dat`, so the newer save was lost. Both halves are fixed: the
  live session's save now drops the autosave the moment `world.dat` commits
  (`AtCommit::DropAutosave`, after one directory fsync), and the client opens the
  autosave first only when it is newer than `world.dat` (modification times — neither
  file records its save time; a tie goes to `world.dat`). A stale, older autosave is
  cleared once `world.dat` has opened. If a `world.dat` newer than the autosave fails to
  load, the older autosave opens instead, with the damaged `world.dat` copied aside as
  `world.dat.corrupt-<ts>` (left in place, so the lobby still lists the world).
- **A failed window-close save made every later close quit unsaved.** The flag never
  expired. Now the second close tries the save once more and quits either way, keeping
  the autosave if it fails again; any save that lands clears the flag.
- **Staged first-save chunks were noted as on disk only after the publish.** If the
  rename failed, the next save published them without knowing they were on disk, so a
  chunk mined out since came back. They are noted the moment `world.dat` commits, and a
  test now lets the next SAVE — not an open — finish the publish.
- **No directory fsync between `world.dat` and the publish**, so the `chunks/` rename
  could be durable without `world.dat`. One is made at the commit (no in-process test
  can observe it).
- **After a downgrade a folder could hold `world.dat`, `chunks/*.chunk` and a
  `chunks.new/`** and was refused forever. That `chunks.new/` is stale and is now set
  aside as `chunks.new.stale-<ts>`.
