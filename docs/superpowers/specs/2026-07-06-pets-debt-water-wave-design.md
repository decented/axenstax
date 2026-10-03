# Pets, Debt & Water wave — design

**Date:** 2026-07-06 · **Status:** BUILT 2026-07-06 — all three campaigns (P/D/W,
Tasks 1-16) shipped on `worktree-pets-debt-water-wave`; Campaign D3 (clippy Phase
4b, Tasks 18-20) runs after this doc pass. Docs updated + test sheet out at
Task 17 (`AxeNStax-internal/docs/test-sheets/2026-07-06-pets-debt-water-wave.md`).
Owner directive was "crack on until built".
**Scope decided with owner:** fire burn-tiers/afterburn **dropped** from this wave (no spec
exists; it was deferred as "feel-tuning, wants a playtest first" — building it solo would
invent fire-feel without Axolittle). Pets campaign = **the whole gap list**. Campaign 3 =
water step-side seams only.

Three campaigns, one branch, built in sequence with the clippy sweep last:

| # | Campaign | Size | Core deliverable |
|---|----------|------|------------------|
| P | Pets & companions | Large | 1C framework + finish every deferred species feature + Crab |
| D | Debt paydown | Medium | Recipe catalogue gap-close, dead test_board UI removal, clippy Phase 4b |
| W | Water seams | Small-medium | Step-side strip emission between different water levels |

Audit ground truth (2026-07-06, three Explore agents + live clippy run) is baked into the
anchors below. Everything cited was verified against source, not notes.

---

## Campaign P — Pets & companions

Finishes the deferred work from `docs/foundations/2026-06-22-animals-six-wave-master-plan.md`
(Wave 1C + the per-species deferrals). Substrate that already exists and is reused, not
rebuilt: `tameable::OwnershipData` + `attempt_tame_generic`, `CompanionData`
(`companion.rs:31`), `species_ai::dispatch_companions` (`species_ai.rs:480` — the generic
follow driver), wolf state machine (`wolf.rs`), nostrich sit toggle
(`game_loop.rs:~10611`), `HorseData` steed persistence (`horse_ai.rs:47-59`), genetics
(`genetics.rs`), breeding (`breeding.rs`), cart cargo pattern (`cart.rs:145-148`),
shared container dialog (`chest_ui::show_container_dialog`).

### P1 — Command states (the 1C core)

- `CompanionState { Follow, Stay, Wander, Perch }` on `CompanionData` as a new
  `#[serde(default)]` field (default `Follow` — old saves and current behaviour
  unchanged). Bincode-positional discipline: field appended LAST to `CompanionData`;
  the proven precedent is `HorseData.kept_by` (`horse_ai.rs:51-52`).
- Empty-hand right-click on a companion you own cycles the state, with a toast naming
  the new state. Non-shoulder species skip `Perch` in the cycle (Parrot-only for now).
- `dispatch_companions` gates by state: `Follow` = current behaviour; `Stay` = hold
  position (zero drive); `Wander` = generic ambient wander (no follow drive);
  `Perch` = see P6.
- **Wolf parity:** wire the existing, currently-unreachable `wolf::try_toggle_sit`
  (`wolf.rs:106`) to empty-hand right-click on an owned wolf — same input grammar as
  the nostrich toggle. Wolf keeps its own state machine; we are **not** unifying
  `WolfData`/`NostrichData` into `CompanionData` this wave (they are save-enum variants
  1 and 2; migration risk outweighs tidiness). Recorded as future cleanup, not a bridge.

### P2 — Friendly-fire protection

Player melee and projectile hit paths skip entities whose owner pubkey equals the
attacker's (check `CompanionData`/`WolfData`/`NostrichData` ownership, and kept steeds
via `HorseData.kept_by`) — **unless the attacker is sneaking** (the spec's intentional-hit
escape hatch). Unit-tested as a pure target-filter function.

### P3 — Anti-loss: Pet Bed + Recall Whistle

- **Pet Bed** (new block id, next free; craft: wool + planks). Behaviour: when a tamed
  pet **dies** and a Pet Bed exists in the loaded world, the pet is rescued instead —
  teleported to the nearest Pet Bed to its death position and restored to full health
  ("knocked out, crawls home"). No bed in loaded range → normal death. Bed search is
  loaded-chunks only (v1; noted on the test sheet). No per-bed ownership v1 — beds are
  communal rescue points (placer tracking doesn't exist for plain blocks; revisit if
  multiplayer griefing appears).
- **Recall Whistle** (new material item; craft: bone + string). Use while held: every
  tamed/kept animal you own (companions, wolf, nostrich, kept steeds — including a
  cargo-laden donkey) teleports to you; toast reports the count.
- Both get catalogue cards + textures (`texture_gen`), and the block/item count-lock
  tests bump.

### P4 — Wolf combat-assist movement

`WolfAction::AttackTarget` currently resolves to a stop (`block_interact.rs:65-72`).
Implement it: the wolf paths toward the target entity and deals melee damage on contact,
reusing the existing mob-melee pattern. This closes the last half-built 1C item.

### P5 — Cat: threat-ward aura + treat

- **Aura:** hostiles (Brigand/Marauder/Berserker/Hyena) veto targets within
  `CAT_WARD_RADIUS` (start 12) of any tamed cat, and hostiles inside the radius get a
  flee bias away from the cat. Pure-function target-filter + steering tests.
- **Cat Treat** (new material; craft: raw fish + wheat → 2). Taming with a treat is
  guaranteed (vs 1-in-3 with fish). Catalogue card included.

### P6 — Parrot: flight, shoulder-ride, alarm

- **Flight:** attach the `Flying` marker (the bee pattern) to Parrot at spawn; follow
  drive gains a Y component for flying species so it flutters after you rather than
  walking.
- **Shoulder-ride:** `CompanionState::Perch` — position pinned each tick to the owner's
  shoulder offset (~±0.4 side, +1.4 up), AI and physics skipped while perched.
  Dismounts (→ `Follow`) when the owner takes damage or cycles state. Chosen over
  Minecraft's merge-into-player-model: the entity stays alive, so persistence and
  rendering need no special cases.
- **Threat-alarm:** any tamed parrot squawks (existing audio cue if a clip fits, else
  toast + particle burst) when a hostile comes within `PARROT_ALARM_RADIUS` (start 16),
  on a ~10 s cooldown. The parrot→Electricity signal bridge stays deferred with the
  Aether design (owner decision pending there).

### P7 — Donkey/Mule cargo

- Append `pack: Option<ChestData>` to `HorseData` with `#[serde(default)]` (same
  append-last discipline; persistence rides the existing `SavedTamedPetData::Steed`
  variant for free — no new save variant).
- Right-click a kept Donkey/Mule while holding a Chest → equips the pack (consumes the
  chest). Sneak-right-click → opens the 27-slot pack via
  `chest_ui::show_container_dialog`. Horse never carries a pack (speed vs utility niche).
- Killing/losing a packed donkey with the rescue bed keeps the pack (it's in HorseData).

### P8 — Mule cross-breeding

- Give the horse family a breeding food: **Wheat** (`breeding::breeding_food` currently
  `None` for them; sharing wheat with other grazers is fine — food keys are per-species
  lookups, not exclusive).
- Cross-pair exception in `tick_breeding` (`breeding.rs:117` same-species rule):
  Horse×Donkey in-love adjacent pair → baby **Mule** with blended `Genetics` (the
  engine's `breed()` already does inheritance; only the pairing rule + baby-kind mapping
  are new). **Mule is sterile** — never pairs; pinned by a test alongside the existing
  `different_species_do_not_pair`.

### P9 — Crab + reach tool

- `MobType::Crab` appended **LAST** (save-order discipline — MobType is
  bincode-positional, `mob.rs:17-138`). TOML def, placeholder small mesh (the
  established cat-uses-wolf-mesh pattern; model passes are Axo feel work later).
- Spawns coastal: passive, on sand near sea level (spawn-rule mechanics pinned in plan
  against `spawning.rs`/biome tables). Drops **Crab Claw** (new material) on kill.
- **Reach Claw** tool (craft: crab claw + sticks): while held, block place/break ray
  distance +2 over the current reach constant. No durability drain v1 (knob for Axo).
  Catalogue cards for both recipes.

### P10 — Feel knobs → test sheet

Follow/stay distances, ward radius, alarm radius + cooldown, whistle behaviour, bed
rescue feel, pack-equip grammar, crab spawn rate, reach bonus, treat guarantee — all on
Axolittle's sheet. Placeholder models called out explicitly so "cat looks like a wolf"
arrives as a known, not a bug.

---

## Campaign D — Debt paydown

### D1 — Recipe catalogue gap-close (NOT the matcher rewrite)

The "~44 literals" note was stale: the real gap is **~13–18 missing cards** — recipes
live in `match_recipe` (`crafting.rs:682`, 134 arms) with no catalogue card, invisible
to the recipe book and to the card→matcher consistency test
(`crafting_catalogue.rs:1181`). Add cards for: Sticky Piston, Piston, Hopper, Oak Door,
Oak Trapdoor, Oak Fence Gate, Oak Sign, Glass Pane, Iron Bars, Stone Stairs, Stone Slab,
Item Frame, Cobblestone Wall, + the 4 non-oak fence posts
(`fence_post_for_species`, `crafting.rs:948-957`). Bump the count-lock
(`catalogue_is_non_trivial`, 270 → new total, including Campaign P's new cards).

The **full §6 matcher migration stays out**: Spec 43
(`docs/foundations/2026-06-12-recipe-catalogue-and-book.md:183-190`) itself says "own
spec, later — NOT now", and the audit rates it a large hot-path rewrite of a 1,658-line
function. The user-facing debt is catalogue coverage; this closes it. README #43's
Deferred cell gets corrected (stale "~44" count).

### D2 — Dead test_board verdict UI removal

Remove the self-referential dead cluster left by the Trials cutover (2026-06-24):

- `MenuState` fields: `test_board_items/selected/note/status/msg/fetch/loaded`
  (`menu.rs:342-357`, inits `:427-437`).
- Dead functions/blocks: `kick_off_test_board_fetch` (`menu.rs:825`),
  `fetch_test_board_status` (`:837`), the fetch-drain block (`:2482-2489`), menu-local
  `queue_verdict` (`:1066`, zero callers) + `submit_verdict` (`:1054`), the orphaned
  doc comment (`:1280-1289`).
- Dead sub-API in `test_board.rs`: `TestStatus` (`:28`), `board_status` (`:63`),
  `VerdictCounts`/`::status` (`:115/:122`), `parse_status` (`:188`).
- Cosmetic: the Trials panel still reuses `SidePanel::left("test_board")`
  (`menu.rs:2561`) — rename to `"trials"`.

**Fenced and KEPT (live Test Lab path):** `test_board::{queue_verdict, Verdict,
build_id, load_registry, TestItem, rank_items, TestBoardRegistry}`, the
`game_loop.rs:14597-14624` testlab gate, `main.rs:588` `mission_registry`,
`villager_ui.rs:435` `draw_satoshi_mission`, and the `"test-verdict"` mailbox tag
handling (live producer `test_board.rs:176`).

### D3 — Clippy Phase 4b (runs LAST, after all wave code lands)

Live count 2026-07-06: **~700 warnings**, top clusters: collapsible-if 207, doc-list
indentation 158, `is_multiple_of` 75, empty-line-after-doc 48, same-type casts 36,
range-contains 27, egui deprecations ~37.

1. `cargo clippy --fix` mechanical pass (own commit, so review separates it from hand
   edits).
2. Hand-fix the remainder; egui deprecation renames (`Context::style`→`global_style`,
   `CentralPanel/Panel::show`→`show_inside`, `SidePanel`→`Panel::left/right`,
   `exact_width`→`exact_size`, `screen_rect`→`content_rect`, per current egui docs).
3. Crate-level `#![allow(clippy::too_many_arguments, clippy::type_complexity)]` with a
   one-line justification comment (accepted style in this codebase; 31 sites, all
   long-standing signatures). Everything else gets fixed, not allowed.
4. Flip `check.sh:46` to `cargo clippy $PROFILE_FLAG -- -D warnings`; delete the
   Phase 4b note (`check.sh:43-45`) and the now-redundant warn-count report (`:47-48`).

Gate: full `check.sh` green with **zero** clippy warnings; WASM build (`trunk build`)
still green (clippy runs native-only, matching check.sh — wasm-only cfg arms get a
compile check via the trunk step as today).

---

## Campaign W — Water step-side seams

The documented v1 seam (`mesh.rs:1444-1448`): water side faces only emit against AIR
(`mesh.rs:1388`), so where two flow cells of different level touch, the taller cell's
exposed vertical strip between the two surfaces is never meshed — a see-through gap.

- Add a second emission pass in `greedy_water_face` (`mesh.rs:1350-1462`): for the four
  horizontal faces, when the neighbour is WATER and
  `water_surface_height(this) > water_surface_height(neighbour)`, emit a strip spanning
  `[neighbour_surface, this_surface]`. A FULL_COLUMN neighbour (water above it) reads
  full height → no strip (matches the already-correct submerged case).
- New quad helper (generalise `emit_quad_frac`, `mesh.rs:1763-1794`) supporting a
  floating bottom edge — today's side faces hardcode the cell floor.
- Step strips are per-cell (heights vary pair-by-pair) so they don't greedy-merge —
  accepted; the cost is shoreline-local.
- No shader/vertex-format/renderer change (`fs_water` untouched). No save surface.
- Tests: step boundary emits a strip of the right height; equal levels emit nothing;
  submerged neighbour emits nothing. Existing tests
  (`water_depth_level_lowers_rendered_surface`, `submerged_water_column_renders_full_cells`,
  `uniform_water_sheet_still_greedy_merges`) keep passing; the inline "accepted for v1"
  comment is replaced.

Explicitly out (owner-approved): fire burn-tiers/afterburn (no spec; feel-tuning needs
Axo — design it in a playtest session). Weather sync stays a watch item (synced by
construction; no code).

---

## Cross-cutting

- **Branch:** one worktree branch off `main` (`feature/pets-debt-water-wave`).
  Campaign commit order: P → D1 → D2 → W → D3 (clippy sweep last so it covers all new
  code once, and the gate flips when warnings are actually zero).
- **Save discipline (load-bearing):** `MobType::Crab` appended last;
  `CompanionData.state` + `HorseData.pack` appended with `#[serde(default)]`; no
  WorldSave field reorder; byte-layout decode tests extended per
  `save.rs:947-985` rules.
- **Registry bumps:** Pet Bed block; Cat Treat, Recall Whistle, Crab Claw, Reach Claw
  items; textures via `texture_gen`; block/item/texture count-lock tests updated;
  every new recipe gets a catalogue card (the consistency test then covers it).
- **Verification:** `check.sh` after each campaign; TDD for the pure functions (state
  cycling, friendly-fire filter, ward veto, cross-pair rule, sterile-mule, bed rescue,
  whistle recall, reach distance, water strips); multi-agent code review at the end
  (last wave's caught 9 real bugs); test sheet →
  `AxeNStax-internal/docs/test-sheets/2026-07-06-pets-debt-water-wave.md`.
- **Docs to update on ship:** Spec 05 (companion commands, anti-loss, cargo,
  cross-breeding, crab/reach), foundations animals master plan (mark 1C + deferrals
  done), README #43 Deferred cell, Spec 02/03 water-seam note, this doc's status.
- **Red lines:** no networking/discovery/identity/hosting/data surface anywhere in the
  wave. Local gameplay + local refactors only.
- **Not rebuilt:** native AppImage (metered CI — separate owner ask). Web auto-deploys
  on push to main.

---

## Shipped — verified deviations from this plan (Task 17 close-out)

Full task-by-task ledger: `.superpowers/sdd/progress.md`. Headlines that changed
shape between plan and build, each reviewed and verified correct:

- **D1 gap-close was 18 cards, not ~13-18's low end of 17** — `WoodSpecies` has 6
  non-oak species, not 4, and Rubber is correctly excluded from fence-post cards
  (no `Log → Planks` matcher arm for Rubber, so a Rubber Fence Post card would
  advertise an unreachable recipe — mirrors `push_generated_literals`'s exact
  species set). See Spec 05 update + `docs/foundations/README.md` #43.
- **P4 wolf combat-assist shipped in two commits, one feature**: Task 8
  (`e159bb59`) built `AttackTarget` → movement + contact damage; Task 8b
  (`fb4a6a60`, a same-day follow-on recovered from a mid-session machine crash)
  wired the actual trigger — a wolf now organically enters assist/revenge from
  real owner-attacks-something / owner-gets-attacked combat events, gated so a
  sitting wolf (`WolfAiState::Sit`) never self-pivots into a fight.
- **P9 Reach Claw multiplayer gap found and "fixed" in the same task — later
  found to be an anti-cheat REGRESSION, re-hardened 2026-07-07**: Task 13's
  first pass left the server-side reach anti-cheat gate blind to the tool
  (hosted sessions would have silently defeated it); "fixed" same-task in
  `79db0d5b` by widening the gate using the live `ServerPlayer.held_kind`/
  `held_id`. That fix was itself the bug: those fields are purely
  client-asserted for remote (`server_simulated`) players — the server has no
  possession check on them — so a modified client could claim a Reach Claw it
  didn't have and grief with ~7.5-block reach instead of the pre-wave flat 5.5.
  The wave-hardening review (Task 4, commit `472cfe47`) caught this and
  re-established the flat remote cap: the Reach Claw bonus is now honoured only
  for the position-trusted local/hosting player, with a `// BRIDGE:` comment on
  `block_change_within_reach` (`hosted_server.rs`) noting the remote bonus
  returns once remote inventory becomes server-authoritative. See
  `docs/spec/05-gameplay-systems.md` §2.5 for the corrected, current behaviour.
- **P3 Pet Bed rescue is single-player / client-driven-tick only** — the rescue
  runs in `game_loop.rs` (where drops/kill-counter already live); `server.rs`'s
  hosted-session death path has no death-consequence layer at all yet
  (pre-existing dual-sim debt, not new). A pet that dies in a hosted
  multiplayer session does not get the bed rescue today. Tracked in Spec 05 +
  the CLAUDE.md known-debt list; not a regression, a scope boundary.
- **Cat, Parrot, and Crab render with placeholder meshes** (the established
  cat-uses-wolf-mesh pattern) — feel/model work is an Axolittle playtest
  follow-up, not a defect.

## Hardening pass — 2026-07-07 (follow-up wave)

Full task-by-task ledger: `.superpowers/sdd/progress.md` (Tasks 1-20). A
follow-up wave fixed 10 confirmed wave-introduced bugs + 7 pre-existing
clippy-flagged gaps, plus 4 small additions, all on top of this wave's build:

- **Item-loss / ghost-resurrection paths**: furnace mine spilled nothing and
  orphaned the block-entity (re-place resurrected old contents); a dead
  packed donkey/mule destroyed the pack + contents; a kept steed kept its
  `Scattered` tag and could be despawned by ordinary chunk unload.
- **MP anti-cheat**: the server reach gate (P9, above) was widened by an
  unchecked client-asserted held item — re-flattened for remote players.
- **Interaction regressions this wave introduced**: horse-family wheat
  right-click fed instead of mounting (now sneak-gated); Pet Bed rescue
  defeated the deliberate owner sneak-kill bypass (now skipped via a 5s
  cull-mark window); own-pet body-blocking cancelled the whole swing instead
  of retargeting past it; the perched parrot's fixed world-axis shoulder
  anchor sat inside the interaction cone at some yaws; the Recall Whistle
  could teleport a steed out from under a rider in split-screen; a wolf's
  raw attack-state entity bits could go stale across save/load and maul an
  innocent after reload.
- **Pre-existing gameplay gaps wired live** (dead code with zero callers,
  caught by the clippy sweep): Salt Lick's 2× regen multiplier; bears/hyenas
  fighting back when hit; wolves giving up the follow at extreme range
  (with hysteresis); Rubber Boots' sprint multiplier (capped 1.4× to stay
  under the server speed gate); gameplay input suppression while an egui
  text field has focus; the blocked-placement-ghost reason toast; companion
  tames (Cat/Parrot/Fox) firing `TameMob` for Trials; toasts that were
  silently `pidx == 0`-gated.
- **Engagement additions**: two new Tend trials, Mule Maker and A Friend in
  Need; cargo-pack unequip (sneak + empty hand on an empty pack).

Spec 05 updated in place per-topic (furnace §4.5, reach §2.5, armour §6.6,
input §1.1.1, mob AI §9.2, pets/companions §9.7) rather than as a changelog —
each statement now reads as current behaviour, not a diff.
