# Living world, Tier 2: who built it, how it's used, and what it becomes

Status: build spec for the overnight run of 2026-11-08/09. Tier 1 (bloom, Feast, Commons claims, the Grey Quarry, the Tally) is shipped in 0.2.30; its spec is `2026-11-08-living-world-tier1.md` and this spec builds on its §1 bloom maths. The vision is `AxeNStax-internal/docs/vision/living-world-long-run.md` §3, §5, §6, §7, §9 and §14 (Tier 2 row). The code seams are mapped in the build's seam file; each slice brief repeats the parts it needs.

Tier 2 buys the **people-together** and **time** terms of bloom: the world learns who built what (locally, never shown per person), how places are used, and what happens to land when its builder leaves.

## 0. Calls taken (recorded so nobody re-litigates them)

- **T1. No ten-kids gate tonight.** The vision puts a playtest gate between Tier 1 and Tier 2. The owner said on 2026-11-08 to keep building towards abundance overnight without play. The build proceeds; nothing here re-plumbs the act-one economy (the Hytale-trap risk, vision §15).
- **T2. `game_loop.rs` is not split tonight.** Other sessions are working in it; a 24,000-line move would collide with them. New code lives in modules; `game_loop.rs` gets call sites only (Tier 1 D4 stands).
- **T3. Authority-only side tables.** Authorship, the use record, Plan build counts and last-seen live on the machine that owns the world (single-player, a lending host, a dedicated server). They are never sent on the wire and never shown per person. The display rule from vision §9 holds: the Tally and world bloom show world totals only, and no message ever names who built a block.
- **T4. Author identity.** `AuthorKey { Local(u8 seat), Npub(String) }`, matching `PlotOwner`. A guest (no verified key) is not recorded: its blocks are anonymous, like every block placed before this build.
- **T5. Commons protect work.** In a Commons footprint a block can be broken only by its author. A block with no recorded author (natural, pre-Tier-2, a guest's) is free to break. Creative bypasses, as for Plots. There is no consent step yet; blasts, pistons, fire and falling blocks are not stopped. The refusal never names the author: "Someone else built this."
- **T6. Two clocks.** The use record counts **lived days**: world days (night calendar, `NightCalendar`, saved in the clock) on which at least one player was present in the world. Its fades are measured in lived days too, so an idle 24/7 dedicated server, a bed skipping nights or `/time` cannot grey a place overnight. **Succession uses the wall clock** (native `current_unix_ts()`; joiners exist only on native): a dedicated server runs about 72 world days per real day, and anyone could sleep the calendar forward to take a Plot.
- **T7. Path wear makes a new block.** Grass walked over often becomes a Worn Path (new block, existing textures). Partial wear is runtime-only; the block persists. Bloom excludes it exactly as it excludes built ground: removed from the cover denominator, neither bare nor life. A road is honour, not damage. Off in creative (a creative lawn round a build must not wear away).
- **T8. Succession for joiners' Plots.** Joiners can now own Plots (closing a Tier 1 limit). A joiner's Plot whose owner has not joined the world for 60 real days (wall clock, T6) becomes a Commons: its marker turns into a Commons Stone, the Plot is released, and the owner's authorship inside the footprint is cleared so the village can tend it. A local seat's Plot (the household's) never passes on.
- **T9. Site boards hold picked tasks, not free text.** A shared, live, multiplayer text board would be a chat surface and fall under the comms ceiling. Tasks are picked from a fixed list of plain jobs. Signs stay as they are.
- **T10. Crew moods are presentation.** Villagers show a mood from their place's bloom and recent Feasts (a small particle cue). Moods change nothing in the simulation; consequences are Tier 3.
- **T11. Blight comes from monoculture only, and only big ones.** A large single-crop field (at least 64 crops in a region, 90% one kind) can blight; a child's first wheat patch or a mixed field never starts one. Blight spreads only to the same crop. Off in creative and inside trial arenas (a farming trial must not turn flaky).
- **T12. Plan licences stay.** Retiring Master/Licence scarcity on Plans is an economy change (vision §14 "owner calls") and is not made here.
- **T13. Joiners' bloom.** The co-building and lived-in terms need authority data. A late slice (S10) sends joiners per-region summaries (counts, no identities). Until it lands, a joiner sees less of those two terms than the host does; this is a known limit.
- **T14. Bundle size is a target.** 5 MiB brotli is a target, not a gate (owner, 2026-11-08). Keep the web build lean: no new dependencies, native-only code `cfg`'d out of WASM.

## 1. Bloom: the new people and time terms

Tier 1: `P = 1 − (1−c)(1−f)(1−k)` from co-presence, Feast and Commons claim; `target = 1 − (1−eco)(1−0.7P)`, creative floor 0.6.

Tier 2 extends P to `P = 1 − (1−c)(1−f)(1−k)(1−b)(1−u)(1−s)` and adds a looted dim:

- **b, built together.** `n` = distinct recorded authors of placed blocks in the region (capped at 8). `b = 0` for n < 2, else `1 − exp(−(n−1)/2)`.
- **u, lived in.** From the region's use record (§3): `days` = lived days (T6) with a player present in the region, `nights` = nights slept in a bed in the region. Only where the region holds at least 16 placed blocks (a place, not a field walked through). `u = 0.6·(1 − exp(−(days + 2·nights)/10))`, faded linearly to 0 between 14 and 28 lived days since the region was last used.
- **s, a tended shelf.** From the fullest Commons Shelf in the region (§6): `s = 0.5·fill`, fill in 0..1.
- **Looted.** A shelf marked looted (§6) multiplies the region's target by 0.8 until it is restocked.

Worn Path and Blighted Crop are classified for the eco term: Worn Path is excluded (like built ground); Blighted Crop is bare (0).

World bloom for adults (the F3 grown-up panel and the console snapshot) gains its four named terms from vision §9: **life** (mean eco), **ideas** (Plan designs built in the world, and builds from them), **people together** (mean P), **time** (mean u). World totals only.

## 2. Authorship (slice S2)

- **Store.** `World.authors`: a per-chunk sparse map, local cell index (u16) → `AuthorId` (u16 index into a world author table of `AuthorKey`). Saved as one appended `WorldSave` field, `block_authors: Option<SavedAuthorship { keys, chunks: Vec<(cx,cy,cz, Vec<(u16 cell, u16 author)>)> }>`. Absent → empty (every older block is anonymous).
- **Write sites (identity known):** local place arm (seat `pidx` → `Local(pidx)`), seeds/saplings, joiner edits in `apply_edit_world` (`verified_npub` → `Npub`; guest → none), Plan stamping in `tick_build` (the anchor's starter: add `starter: Option<AuthorKey>` beside the anchor in a side map, not inside `ConstructionAnchorData`, which is saved positionally), NPC commission builds (the commissioner's key; a `local-player-N` string maps to `Local(N)`).
- **Clear sites:** every place the placed bit is cleared clears the author, and so does every `set_placed` call (a placement overwrites, or for a guest clears, the cell's entry). A block moved by a piston or a fall lands anonymous (known limit).
- **Readers:** Commons protection (§2.1), bloom's `b` term, succession (§5).
- **Saved and checked:** follow the `authority_clock` append touch-list in full (struct, literals, tolerant reader, absent-tail arm, footer table, restore, writers take it explicitly, checkpoint/digest/mutation consumers, tripwire tests). A present-but-invalid record (unknown author index, bad cell index) is dropped with a warning, never a refusal to load.
- **Size bound.** A chunk holds at most 4096 entries and the key table at most 65,535 keys; beyond that new placements are anonymous.
- **Determinism.** The maps are hash maps: the save, `sim_digest` and any checkpoint must iterate them sorted (chunk, then cell), the way `sort_unordered_rows` already orders other unordered rows, or a lent host's digests diverge.
- **Load reconciliation.** Chunk files and `WorldSave` are written separately, so after a crash an entry can point at a cell that is no longer placed: when a chunk loads, entries whose placed bit is clear are dropped.
- **One funnel.** Every `World::set_placed` call drops the cell's entry, `true` included, and so does `World::set_block` when a cell becomes AIR; `record_author` is called after `set_placed(true)`, only at the identity-knowing sites.
- **Plan-stamp starters are saved** inside `SavedAuthorship` (anchor position → `AuthorKey`), so a build resumed after a reload still credits its starter.
- **`--no-lend`** hosts keep two world copies, so their authorship diverges between the copies (known limit; the flag is a one-release bridge).

### 2.1 Commons protection

In a Commons footprint (marker ±16, the Tier 1 rule), a break of a block whose author is someone else is refused: local break arm beside the Plot check (toast "Someone else built this."), and the authority's `remote_may_touch` (a new `EditRefusal`, the joiner's ghost undone as for `ForeignPlot`). Creative bypasses. A Plan or NPC build stamping over another's block in a Commons is skipped (as `placement_refused` already skips claims).

**As built (S2).** Decisions taken building it:
- *Module and store.* `authorship.rs` owns `AuthorKey`, `Authorship` (per-chunk sparse `cell -> AuthorId`, a key table, the Plan starters), `SavedAuthorship` and the pure rules; `World.authors` holds it. `World::record_author`, `author_at`, `place_player_block_as`, `commons_protects`, `authors_in_region` and `reconcile_authors` are the API.
- *Writers need no new argument.* Unlike the clock, the table lives in `World`, and every writer is handed the world, so `build_world_save` (client), the three `save.rs` builders and the server's own builder all call `world.authors.to_saved()`; the explicit-argument threading of 4c5a75c6 was not needed. The rest of the touch list was done: struct, `sort_unordered_rows` destructure, literals, tolerant reader, `absent_tail_encoded_len` arm, `FOOTER_FIELD_COUNTS` row `(62, [57])`, restore, the checkpoint/handover consumers, `sim_digest`, `joiner_save` pin, the mutation table and every tripwire test (field count 57, version 62, layout revision unchanged at 5 because no block id or nested type changed).
- *Canonical encoding.* Only used keys are saved, sorted, and cells are renumbered against that table, so the encoding does not depend on the order things were placed in or on the in-memory key table. An empty table is written as `None`.
- *Clear funnel.* See "S2 review fixes" below for the final rule (the first version cleared on `set_placed(.., false)` only).
- *Write sites.* The local place arm (and the door's top half), hang print, seeds and papyrus record `Local(pidx)` through `GameState::author_for_seat` (nothing on a joined client, whose server records); `apply_edit_world` records the joiner's verified npub for a placement and a guest's as anonymous (Mirror, Strict and the strict door all pass through it). `plan::tick_build` and `complete_build` read the starter from `World.authors`; `choose_auto_build` records `Local(pidx)` as the starter after `start_build` succeeds; the Builder's StartBuilding records `AuthorKey::from_commissioner`. A joiner cannot start a Plan build at all (the Auto build arm is refused while joined, C3c-3r), so there is no joiner starter path.
- *Protection.* Local: `commons_break_blocked` beside the Plot check. Authority: `validate_block_edit` calls the new `remote_authored_allows` after `remote_may_touch` for any edit that changes the cell, returning `EditRefusal::SomeoneElsesBuild`. `remote_may_touch` itself keeps its (x, z) signature because a dozen non-edit touches (containers, doors, packs) call it; `EditRefusal` is host-internal, so there is no wire change and no protocol bump. A guest (no key) is refused another author's block in a Commons, as it has no claim on it. A Plan's plaque in a Commons goes on the first free cell above a protected block rather than replacing it.
- *Restore.* `restore_world_save_state` rebuilds the table, then `reconcile_authors` drops entries whose placed bit is clear or whose chunk is absent, and starters with no anchor. The handover transfer copy maps seat 0 to the source's npub and anonymises other seats (`SavedAuthorship::neutralise`); `validate_neutral` refuses a copy that still names a seat.
- *Bloom.* `Region.built` is set in `scan_regions` from `World::authors_in_region` (distinct authors of placed, non-air blocks over the column's 16 chunks, capped at 8), so it follows the region scan cadence (a full sweep of 289 tracked regions is about 72 ticks) and never runs per tick. Probes (trial boxes) take no `b`.

**As built (S2 review fixes).** An independent review of S1 and S2 found seven things to fix; the calls taken:
- *Funnel (SF1, SF2).* `World::set_placed` now drops the cell's author on every call, `true` included, and `World::set_block` drops it when a cell becomes AIR (a blast, a fire, a drained source, a break). A block that moves (a piston, a falling block) therefore lands anonymous: it can no longer inherit a stale entry and become protected against its own builder, and it is free for anyone to break. The writers that know who is acting call `record_author` after `set_placed(true)` (`place_player_block_as`, `apply_edit_world`, the plan stamps), so they are unaffected. `author_at` also returns nothing for an AIR cell, so a stale entry on empty space (an older save, a crash) protects nothing and counts for no `b`. The local door break (creative and survival arms) now clears the paired half's placed bit, as the server's `clear_door_pair` always did.
- *The Stone (SF3).* `commons_protects` exempts `COMMONS_STONE` itself, so anyone can take a Commons down (the Tier 1 rule, and the `commons.rs` doc). The stone is still authored and still counts toward `b`.
- *Retry (SF4).* `arena_build::lay_fills` clears the placed bit of every cell it lays, so "Try again" re-lays the terraces as the arena's terrain (and drops any author), and the second run's bloom scan equals the first's.
- *Export (SF5).* `pack_world_for_export` (which every export, backup, transfer, replay-snapshot and publish path uses) sets `block_authors` to `None`, and `unpack_world_for_import` drops an incoming table: who built what is the owner's own record and never leaves the machine. (`pack_world_for_export` now takes the `WorldSave` by value.) The Keep/handover copy is the one intended carrier, with seats neutralised.
- *Commission refunds (SF6).* `plan::tick_build_reporting` also reports the blocks of the cells it skipped. `builder::process_builder_commissions` returns them on `CommissionTickOutcome.skipped_materials` with the commissioner, and `GameState::return_commission_materials` puts them back in that seat's bag (overflow at its feet) as they are skipped. A commission refunds nothing after the build starts, so there is no double refund.
- *Building over is breaking (SF7).* On the authority this was already true (`validate_block_edit` refuses any change of an authored cell, a replaceable such as a placed water source or snow layer included). The local place arm (over a placed water source) and the local use arms that change the target cell (Eraser, hoe, bone meal or fertiliser on a crop, rubber tap, salt, bucket fill and empty) now ask the same `commons_protects` rule through `GameState::commons_blocks_local_edit` (toast "Someone else built this.", creative bypasses). Uses are therefore refused where a break would be; this is the documented behaviour, not an accident (§2.1 said "break").
- *Smaller.* A Plan's plaque is skipped when all eight cells above its centre hold others' blocks (`CompletedBuild.plaque` is now optional); a handover copy whose only authors were split-screen seats has `block_authors = None` rather than an empty record; `arena_spawn` is cleared on a world change.

## 3. The use record and Worn Paths (slice S3)

- **Region use record.** Per region (chunk column): `RegionUse { days: u16, last_day: u32, nights: u16 }`. Saved as an appended `WorldSave` field (`region_use`). Fed by one pure function `use_record::note_presence(record, region, world_day)` called on the authority for every present, alive player about once a second (a phase iterating `GameServer.players`; for standalone single-player, the seat loop). A day counts once per region.
- **Nights slept.** Both sleep paths (`item_actions::serve_sleep` on the authority; the local bed branch) add one night to the bed's region, once per night per region.
- **Path wear.** Not in creative. A runtime map of grass cell → footfalls. A footfall is a player standing on a Grass block entering a new cell. At 24 footfalls the cell becomes a **Worn Path** (block id 376, existing textures: path top from the gravel or salt-path layer, dirt sides; slightly lower collision is NOT needed). Changes are `BlockChange`s from the authority (standalone: the same function's output applied locally). A Worn Path is not "placed" and drops dirt. Partial counts are not saved; the map is bounded (oldest entries forgotten past 16,384).
- **Grass growth.** Worn Paths never turn back to grass at Tier 2.

**As built (S3).** Decisions taken building it:
- *Modules.* `use_record.rs` owns `RegionUse`, `UseRecord` (the lived-day counter, `last_world_day` and a `BTreeMap` region table, so every iteration is already sorted), the pure rules (`note_presence`, `note_night`, `u_term`), the saved form `SavedRegionUse` and `phase`, the one entry point. `path_wear.rs` owns `PathWear`, `Foot`, `ground_cell` and `tick_wear`. `World` holds both (`use_record`, saved; `path_wear`, runtime only); `World::placed_blocks_in_region` and `lived_in_term` are the bloom reads.
- *Format arithmetic.* Field 58 and layout revision 6 (block 376) make the footer **64**, not 63: `SAVE_FORMAT_VERSION` is the sum. `FOOTER_FIELD_COUNTS` gains `(64, [58])`; no commit wrote footer 63. Protocol 97 to 98 (block id, no packet change). The three `ordered_park` goldens that hash the protocol were recomputed with an independent script over the documented layouts (which first reproduced the protocol-97 values).
- *Cadence.* Path wear is sampled **every tick** (a walker at 4 blocks a second would otherwise be sampled on one cell in four, and a path would never form); presence is noted once a second (`PRESENCE_PERIOD` = 20 ticks). Both are one system, `SimSystem::UseRecord` (server-owned on a lent world, every tick), so the tally's once-per-tick tripwire covers it. `phase` takes the present, living bodies; the server builds them from `players` (a lending host's own seats included), the standalone seat loop from its slots; a joined client runs none.
- *Grounded.* (Corrected in the review fixes below.) Every body passes its real `on_ground`; a footfall also needs the sampled ground cell to be solid, so a hop's apex over air is no footfall.
- *Sleeping is presence.* `note_night` also counts the lived day in the bed's region (once, whichever of the two notes comes first, so the table does not depend on their order) and refreshes `last_day`: a bed whose region the sleeper's own feet are not in would otherwise carry nights and look unused. `RegionUse.last_night` is an `Option` (night 0 is a real night).
- *Sleep wiring.* `item_actions::serve_sleep` now takes `&mut World` and notes the night after the judge; the local bed arm notes it before `write_clock` jumps to morning.
- *Lived day.* The world day is `NightCalendar::tonight()` (the dusk-to-dusk night number). `begin_day` is called only when a body is present, so nothing ages while the world is empty; a calendar jump of any size is one step.
- *Bloom.* `Region.lived` is refreshed beside `built` on the scan cadence (`World::lived_in_term`: the placed count is taken only for a region that has a record); `PeopleState::value_full` carries `u`. A Worn Path column is `TopClass::Path`: out of the cover average like built ground and, **unlike** built ground, out of the wall term as well (a village of roads is not "walls"; `EcoTally.paths`). **Corrected in the review fixes below:** the wall term's denominator is every column, so a path is open ground there.
- *Texture.* Dirt on every face (the salt-path top is white-speckled and gravel is grey; neither reads as a dirt track).
- *A worn cell* stops being "placed" if it was (a lawn a player laid): `tick_wear` clears the bit, which also drops any author.
- *Exports.* Unlike authorship, `region_use` is kept in exports and in a handover's transfer copy (it holds no identities).
- *Bounds.* 16,384 regions (the least recently used is forgotten on the next new one) and 16,384 wear cells (the least recently stepped eighth is dropped past the bound).

**As built (S3 review fixes).** Four findings fixed, no format or protocol change (save 65, protocol 98):
- *Redraw in single-player (SF1).* `Chunk.mesh_dirty` has no reader, so a standalone game never redrew a worn cell. The seat loop now hands each change `use_record::phase` returns to `rebuild_chunk_at` (`use_record::redraw_cells` is the tested seam). The local bed's night note was checked: it runs before `write_clock`, and `tonight()` is unchanged by the skip.
- *Footfalls (SF2).* The server passes the body's real `on_ground` for every body on a lent world and for server-simulated joiners (a lending host's seat is refreshed into its shadow each tick); only an owning `--no-lend` host's local shadow still passes `true`. `PathWear::step` also skips a sample whose ground cell is not solid (a hop's apex sits on the top face of the air above the grass), without moving the body's last cell. Tested on `tick_wear` (flag true throughout) and on a lending host (flag as physics has it, and a lying `true`).
- *Walls (SF3).* `EcoTally::wall` divides built columns by every column, so roads are open ground there; they stay out of the cover average. A ring of paths round a village leaves its eco term unchanged (tested).
- *Play mode on a LAN host (SF4).* Not fixed, see §13.
- *Also.* `sort_unordered_rows` sorts `region_use` by region (N4); the spec's "build before S2" limit now reads "before S3" (N3).

## 4. Plan lineage and counts (slice S4)

- **Build counts.** Appended `WorldSave` field `plan_builds: Vec<(content_hash [u8;32], built: u32, built_from: u32)>`. When a build completes (the single authority seam in `sim_machines::tick_construction` and the commission completion in `builder.rs`), `built` += 1 for the Plan's content hash and `built_from` += 1 for every ancestor `plan_hash` in its derivation chain.
- **The plaque** shows the lineage it already carries in plain words ("From a Plan by …, after …", authors as npub per the npub-only display rule, "a local Plan" when empty) and the counts: "Built 3 times in this world. 2 more were built from it." No names of who built.
- World bloom's **ideas** term reads this table.

**As built (S4).** Decisions taken building it:
- *Format arithmetic.* Field 59 with layout revision 6 (no new block) makes the footer **65**; `FOOTER_FIELD_COUNTS` gains `(65, [59])`. No protocol bump (no wire change, no block).
- *One seam, not two.* Both completions (the generic `tick_construction` sweep and the commission's `BuildComplete`) end in `plan::complete_build`, so the count is taken there, before the plaque is placed: a build whose plaque is left out (every candidate cell is someone else's block in a Commons) still counts. A cancelled, refunded or still-running build never reaches it. A creative build counts.
- *Module.* `plan_builds.rs` owns `PlanBuilds` (a `BTreeMap`, so saving and digesting are canonical), `note_completed`, `ancestors_of`, `ideas_total` and the saved form `SavedPlanBuilds`. `World.plan_builds` holds it; it is cleared with the world and restored at load.
- *Hash used.* `plan::content_hash(&anchor.plan)`, the function the derivation chain's links already use. Ancestors are the chain's `plan_hash` values less the Plan's own, each once, at most 64 per completion.
- *Bounds.* 8,192 designs; past it a new design evicts the least-counted row (the lowest hash on a tie), so throwaway Plans cannot grow the table without limit. Counts saturate.
- *The plaque.* `plan_ui::plaque_lines` is the pure text builder (in `plan_ui.rs` so the money-word lint scans its strings). The counts line is looked up by the plaque chain's last link's `plan_hash` (the Plan's own hash at capture). A design with no count shows no counts line, never "Built 0 times", so a plaque made before the counts reads honestly. A joined client passes no counts.
- *Exports.* Unlike authorship, `plan_builds` is kept in exports, imports and a handover's transfer copy (no identities).

## 5. Joiners' Plots, last-seen and succession (slice S5)

- **Joiner Plots.** A joiner placing a Plot Marker on the authority creates `PlotData { owner: PlotOwner::Npub(npub), .. }` under the same claim rules (no overlap with Plots or Commons); breaking it releases the claim (already wired). A guest's marker claims nothing (it stays a plain block, as today). Protection for an `Npub` owner already works through `is_in_foreign_plot_for_npub` / `economy_owner_allows`; the local break arm must treat an `Npub` Plot as foreign to every local seat.
- **Last seen.** Appended `WorldSave` field `owner_last_seen: Vec<(npub, unix_secs u64)>`, updated on the authority while that npub is present (native wall clock; on a build with no wall clock nothing is updated and nothing passes on). Only owners of Plots are tracked; an owner with no record is stamped "now" the first time the table is checked, so upgrading a world never passes a Plot at once.
- **Succession.** Checked on each new world day and on load. A Plot whose `Npub` owner was last seen 60 or more real days ago passes on: the marker cell becomes a Commons Stone, the Plot is released, and that owner's authorship inside the footprint is cleared. Satoshi has a line for when a player first finds one ("Someone built this and moved on. It's the village's now.").

## 6. The Commons Shelf (slice S6)

- **Block.** Commons Shelf (id 377): a container shared like a chest (reuse the chest container and its joiner window path; a new chest tier or a sibling block entity, whichever the chest code takes most cleanly). Textures: existing chest or crafting-table layers. Recipe: a chest in a ring of planks is taken; the agent picks a free shape and records it.
- **Fill in meta.** The authority keeps the shelf's `block_meta` byte as its state: low 7 bits = fill (0..127 = share of slots holding anything), top bit = looted. Meta is already synced by `BlockChange` and the chunk push, so every client's bloom reads it. First confirm the chest family does not already use its meta byte (facing); if it does, make the shelf a sibling block entity whose meta is free.
- **Looted.** If the fill falls by at least half within one world day while it was at least a quarter full, the looted bit is set (the day's starting fill is runtime-only; after a reload the day starts from the current fill). Restocking to half full or more clears it.
- A Commons Shelf may be placed anywhere; it counts towards bloom anywhere.

## 7. Site boards and crew moods (slice S7)

- **Site Board** (id 378, existing board textures, e.g. the bounty board layer). Holds up to 8 tasks, each a job picked from a fixed list (plant trees, build walls, build a roof, dig a well, light the way, make a garden, bring food, gather wood, clear the ground, build a bridge, fence the animals, decorate) with a done tick. Anyone may add, tick or remove a task. Saved as an appended `WorldSave` field `site_boards`.
- **Wire.** Joiners send `ItemAction::SiteBoard { pos, op: Add(job) | Toggle(i) | Remove(i) }` (append-only enum growth, protocol bump); the authority applies it and pushes the board's state to clients that hold the chunk (a new small packet or a `BlockView`, whichever the code takes most cleanly), and the chunk push carries it for late joiners.
- **Bloom.** Ticking a task done adds a short people surge to the board's region (the Feast surge path with two guests), at most once per board per world day, so ticking and unticking cannot pump it (the Tier 1 L4 lesson).
- **UI.** Right-click opens a small egui window: the list, a picker for a new job, a tick per task. Copy-linted (`*_ui.rs`).
- **Crew moods.** Client-side presentation. Every few seconds, for villagers near the camera: bloom stage Alive or a Feast surge active → an occasional happy mote (confetti or green poof); Grey and still → an occasional grey puff; otherwise nothing. Works on joiners from the mirrored villagers.

## 8. Blight (slice S8)

- In `growth::advance_crops`, once per growth pass, count crops by kind per region from the positions already collected. A region with at least 64 crops whose most common kind is 90% or more of them is a monoculture.
- Each crop of the dominant kind in a monoculture that would advance rolls a position+tick hash (the sapling pattern, no RNG state) and blights 1 time in 40: it becomes a **Blighted Crop** (id 379; a plant shape on an existing withered-looking layer such as tall grass or a stage-0 crop, tinted if the tint path allows it without a new layer).
- A Blighted Crop next to a crop of the same kind spreads to it 1 time in 8 per growth pass, wherever it is: once present, blight moves through a field whatever the region's mix. It breaks to nothing and drops nothing; bonemeal does nothing to it. A Blighted Crop has to remember its kind for spreading: keep the kind in its `block_meta`.
- Off in creative and in trial arenas. Runs wherever growth runs (authority on all paths).

## 9. Tier 1 leftovers (slice S1)

- The "Somewhere grey is waiting." hint shows for the Grey Quarry while it is locked (fix `next_hints` for a one-`after` trial with an `on` alternative).
- A Plan that skips a Commons Stone or Plot Marker cell returns that item with the build's leftovers instead of discarding it.
- "Try again" on a Place trial re-applies the arena's grey start (`drained`).
- Feast repeat: a Feast needs at least 4 edible items in the held stack, and a column's Feast surge cannot be renewed within one world day (L4).

**As built (S1).**
- **Hint.** `next_hints_in` treats a locked trial with an `on` alternative that is the only trial on its shelf as always near (`sole_on_shelf`). The Grey Quarry is alone on the Bloom shelf, so the existing shelf-started rule could never fire for it, and a single `after` is either undone or unlocks it. Every other trial's hint behaviour is unchanged (pinned against the bundled catalogue: a fresh store hints only the Grey Quarry). Rancher (`on` + one `after`, shared shelf) has the same shape and still never shows its hint; see §13.
- **Plan leftovers.** `plan::complete_build` now returns `CompletedBuild { plaque, leftovers }`; the leftovers are the locked items for the cells `tick_build` skipped. The one caller that can have any (`sim_machines::tick_construction`, which every path runs) calls `plan::spill_leftovers`, which drops them as ground items one block outside the build's lowest-XZ corner, split to stack limits. Nothing in the engine handed leftovers back at completion before (`cancel_build`'s refund has no player-facing caller), so a ground spill is the handback. NPC commissions lock nothing on their anchor and ignore the field; their skipped cells' materials go back to the commissioner as they are skipped (see S2 review fixes).
- **Retry.** The fresh arena's spawn is remembered (`GameState.arena_spawn`, set by `apply_arena_setup`). "Try again" on a Place trial in its fresh arena calls `place_trial::rewind_arena`: it re-lays the arena's box `fills` (via `arena_build::build_fills`) and re-applies `drained`, re-registers the Place probe at the arena origin (it used to follow wherever the player stood), and puts the player back at the spawn. The arena's single `blocks` and `mobs` are not re-run (the chest, the spring and the five animals stand already). `BloomField::set_initial_level` now replaces an existing box's level instead of pushing a duplicate.
- **Feast (L4 closed).** `feast::hold` takes a fourth argument and refuses `NeedsFood` ("A feast needs at least four things to eat.") unless the held stack is food by the rule eating uses (`Item::food_value`) and holds `feast::MIN_FOOD` = 4 or more, and `FeastedToday` ("This place has feasted today.") when the table's column has feasted on the current night-calendar number. The column is the bloom region (a 16 x 16 chunk column), `World::feast_days` (a sorted map, authority-side, unsaved) records it in `feast::convert`, and older days' entries are dropped on each record. Both refusals go through `feast::hold` on every path: `feast_client::use_feast_table` (single-player, a lending host's seats; `GameState.night_calendar`) and `hosted_server/feast.rs::serve_feast` (joiners; `GameServer.night_calendar`), as two new `ItemNote` codes, `NeedsFood` (31) and `FeastedToday` (32). The Tier 1 spec §6.3 entry is marked closed.

## 10. Joiners' people terms (slice S10, stretch)

The authority sends each joiner, for regions in its chunk interest, `(cx, cz, builders: u8, use_u: u8)` when they change (changed-only, at most every 10 s). The joiner's bloom uses them for `b` and `u`. Counts only, no identities. Protocol bump.

## 11. Grown-up panel (slice S9)

The F3 grown-up panel and `ConsoleSnapshot.world_bloom` show the four world bloom terms (§1). Satoshi lines for Worn Paths, blight and a passed-on Plot (gardener voice, no money words, no morality: a grey place is a puzzle).

## 12. Build slices

One at a time in worktree `../AxeNStax-living-world`, branch `living-world-t2`. Each slice ends with `./check.sh` all green (TMPDIR on the main disk, at least 12 GiB free), a spec as-built note in this file and the relevant main spec (02 world format, 04 networking, 05 gameplay), then merge to main and push. Slices marked **review** get an independent review of their diff before merging.

| Slice | Content | Save / wire | Review |
|---|---|---|---|
| S1 | §9 Tier 1 leftovers | none | reviewed with S2 (the Feast cooldown is on the authority path) |
| S2 | §2 authorship, §2.1 Commons protection, §1 `b` | field `block_authors` | **review** |
| S3 | §3 use record, nights, Worn Path, §1 `u` | field `region_use`, block 376, layout + protocol bump | **review** |
| S4 | §4 Plan build counts and plaque | field `plan_builds` | **review** (with S8) |
| S5 | §5 joiner Plots, last seen, succession | field `owner_last_seen` | **review** |
| S6 | §6 Commons Shelf, §1 `s` and looted | block 377, layout + protocol bump | **review** |
| S7 | §7 Site Board, crew moods | field `site_boards`, block 378, wire | **review** |
| S8 | §8 blight | block 379, layout + protocol bump | **review** (with S4) |
| S9 | §11 panel terms and Satoshi lines | none | no |
| S10 | §10 joiners' people terms (stretch) | wire | **review** |

After the last slice: version bump to 0.2.31, `.deb` and release APK to the Desktop beside 0.2.30, and a "things to try" addendum for Axe.

## 13. Known limits at Tier 2

- `--no-lend` hosts: authorship and the use record live in each copy separately.
- A block moved by a piston or a fall lands anonymous (nobody's, so anyone may break it). Undermining a support, or removing the natural block under an authored crop, reed or torch, is not checked; whether those pop through neighbour updates is unverified.
- Until S10, joiners' bloom lacks the `b` and `u` terms.
- A local seat's Plot never passes on; a guest's Plot Marker claims nothing.
- S1: the Rancher trial (`on` + one `after`, shared Tend shelf) has the same unshown-hint shape as the Grey Quarry had; left alone so no other trial's hints move.
- S1: "Try again" on the Grey Quarry re-lays the terraces but not what the player planted above them (saplings, crops, trees stand at top+1, outside the fills), nor water already flowing, so a second run can finish faster than the first.
- S1: the once-a-day Feast rule is the night-calendar number of the authority, kept in an unsaved table: a reload (or a `--no-lend` host, which has two worlds) forgets that a column feasted. The refusal for a joiner on an older client is silent (an unknown `ItemNote` reads as nothing); no protocol bump was taken for the two new codes.
- S1: a Plan's skipped claim items spill on the ground at the build site rather than into the starter's bag; anyone near can take them.
- S2: `world.dat` grows by about four bytes per authored block (two `u16`) plus the key table. The real bound against the Keep checkpoint (`MAX_RECORD_BYTES` 64 KiB per record, world.dat bound in pieces; the handover cap `WORLD_BYTES` 64 MiB) was not re-derived, so the earlier "about 250,000 authored blocks" figure is unverified; nothing prunes old authors yet.
- S2: a block lost to fire, a blast or water may keep its placed bit (a pre-existing gap) but loses its author entry the moment the cell becomes AIR. A stale entry that survives some other way (an older save) is ignored on an AIR cell; if a non-placing writer (a fluid) later fills that cell the entry could answer again until reconciliation.
- S2: a field-57 record that is structurally invalid bincode (a bad key tag, a non-UTF-8 npub) is refused like any other malformed field outside Recover; only semantically invalid records (unknown key index, a cell out of range, a repeat) are dropped with a warning.
- S2: a Plan or commission that starts and then fails or is refunded leaves its starter row until the next load's `retain_starters`; harmless, but saved and digested until then. `distinct_authors` does a chunk lookup per unseen-author entry per region scan; fine at today's sizes.
- S2: the Keep restore reconciles authorship before it drops unbound chunk files, so an entry in a stale unbound file can survive reconcile; impact unverified (the binding comes from disk).
- S2: a joined client records nothing (its table is empty), so its own bloom has `b = 0`, and the local break arm cannot pre-check protection there; the server refuses and undoes the ghost.
- S2: protection is judged at the cell: a block's second half (a door's top) is separate, and a door's two halves are recorded to the same author at placement and cleared together on a break.
- S2: the Keep checkpoint's restore of a world with authorship was exercised by the mutation table (a changed `block_authors` is refused) but not by a live handover between two machines.
- S3: a joined client records nothing, so its `u` is 0 (S10 sends joiners the per-region figure); nothing about the use record is shown per person or sent on the wire except the Worn Path block itself.
- S3: `--no-lend` hosts wear paths and count days in each world copy separately (the owning server's copy and the host client's); the flag is a one-release bridge.
- S3: the placed-block count behind `u` counts placed bits on non-air cells; a placed bit left on a cell burnt or blasted to air (the known S2 gap) is not counted, but a placed bit kept by a block that was swapped for another still is.
- S3: path wear is not stopped by trials, Plots or Commons (a road across someone's Plot wears like any other grass); a seated rider or a swimmer over grass is judged by position and the body's own `on_ground` like anyone (not specially handled or tested).
- S3: the footfall rule counts a body entering a grass cell, so a long straight run counts once per cell per pass and a hop between two cells counts each time: a determined player can wear a lane quickly; this is the intended honour of a road, not a limit to close.
- S3: a Worn Path is not turned back by anything, including snowfall and tilling; breaking it gives dirt (the player can till or re-grass that).
- S3: the footer is 64; a build before S3 refuses a world written now (it names a newer version).
- S3 review: a survival world opened to LAN keeps the server's play mode when the host types `/gamemode creative` (`/gamemode` writes only the client's `play_mode`; the server is aligned only for a local-only authority, `align_local_rules`). So path wear, and every other server-side creative gate, still judges the world as survival. Syncing it on a lending host would also change the rules joiners were given at join (`WorldRules`, drops, damage), which is beyond path wear; left for the owner.
- S3 review: `/time set` toggles or `/time speed N` can still start a new night number about once a second while a player is present, so a place can age fast (a one-off jump is one lived day). `/time` is an op, ledger-flagged cheat; no cap on lived-day advances was added.
- S3 review: a plant (flower, tall grass, sapling) on a worn grass cell stays standing, so saplings and papyrus cannot then be planted on that path.
- S3 review: path wear overwrites another player's authored grass in a Commons (24 footfalls by a non-author), clearing its author, as the Commons does not protect against wear.
- S3 review: no test drives a real `GameState` lending tick through both halves of the phase (the client's seat loop is skipped while lent; the gate's `one_tick_faults` tripwire covers it at runtime). A seat removal shifts the standalone `PathWear.last` keys and a first sample after a teleport counts one footfall: at most one footfall each.
- S4: a joined client's plaque shows the lineage but no counts line (the table is authority-only; S10 may send per-region summaries but not this table). Counts begin at format 65: plaques made before it show no counts line until the design is built again.
- S4: a design is identified by its content hash, which ignores authorship, mode and kind, so two Plans with the same cells share one row (the same design); a Plan whose chain was not built by `commit_capture` (a hand-made file) may carry a last link whose hash is not its own content hash, and its plaque then finds no row.
- S4: the table evicts its least-counted design at 8,192 rows, so a world with more designs than that forgets the rarest.
(More are added as slices land.)
