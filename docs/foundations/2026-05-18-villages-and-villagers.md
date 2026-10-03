# Foundation — Villages & Villagers

**Status:** READY TO BUILD (Phase 1 — foundation spec only)
**Date:** 2026-05-18
**Author:** Staxolottle (design) + Claude (implementation)
**Vision parent:** [Player-Driven Economies — Long-Run](../vision/economies-long-run.md) §6.4 (T3 Raid Defence)
**Sibling specs:** Spec 11/12 (Farming), Spec 17/18 (Campfire), forthcoming Vendor Block

## TL;DR

Villages are small clusters of procgen houses inhabited by **Villager NPCs** with **professions** (Farmer, Blacksmith, Librarian, Cleric, Carpenter). Players interact with villagers by **giving them quests they need done** — not by trading raw goods. Completing a quest pays out items + (on Bitcoin-enabled servers) **satoshis from the server reserve**. Villages have **Iron Golem** defender mobs. Hostile mobs attack villagers, and players gain village reputation by defending. Long-term, villages feed the **T3 Raid Defence economy** from the vision doc.

**Critical design choice:** villagers do **not** sell goods for sats. That role belongs to the upcoming **Vendor Block** (player-run shops). Villagers offer **work for pay** — preserving Vendor Block as THE marketplace primitive, and giving NPC interactions a unique purpose (quests, not commerce). This avoids the Minecraft trap where NPC trade hollows out the player economy.

## Narrative

A village is a place that's *alive when you arrive*. Smoke rises from chimneys (lit campfires in front yards — leverages Spec 18). Villagers wander between their houses, the farmer's field, the well. The blacksmith stands at his forge. The librarian sits in the library. **You can talk to them.** Each one needs something done — the farmer wants 10 wheat hauled to the granary; the blacksmith wants 5 iron ingots smelted; the librarian wants a rare book from the dungeon. Complete a quest → you're paid (items + sats on Bitcoin servers) and your standing in the village rises.

Villages are also **vulnerable**. Hostile mobs wander toward them at night (the campfire heat-mob mechanic from Spec 18 magnifies this — village campfires attract creatures, the village walls + iron golem hold them off). A village under raid pays defenders. A village destroyed means a permanent loss of that village's quest economy until rebuilt.

**Players can build new villages.** Place a **Village Bell** block (craftable) in a clear area. Wandering "unsettled villagers" (a rare passive mob that spawns in the wilderness) will migrate to your bell over time. Build houses (any block enclosure with door + bed + roof) and they'll claim them. Build a forge → a blacksmith claims it. The world rewards players who build infrastructure, not just stockpiles.

## Design choices

### 1. Quests, not goods-trade

Villagers offer **quests** with three flavours:
- **Fetch**: "Bring me N of item X." Output goes to a destination block (e.g., farmer's chest at the granary).
- **Make**: "Craft N of item X for me." Output is consumed on hand-in.
- **Kill**: "Slay N mobs of type X within radius Y." Tracked server-side.

Rewards always have a **fixed item component** (e.g., 1 emerald, 1 iron ingot) and an **optional sats component** (only on Bitcoin-enabled servers, drawn from the server reserve per Spec 6 §7's pool). The sats amount is **small and deterministic** — quests are not lotteries. See Spec 6 §10.5 (not gambling).

This means villagers **never undercut player Vendor Blocks**. A player Vendor Block selling iron ingots at market rate is the place to buy iron. A blacksmith villager paying for iron ingots is the place to *earn* by smelting and delivering. The two halves form a loop instead of competing.

### 2. Profession determines quest pool

| Profession | Typical quests | Workstation block (T1.x → T1.5) |
|---|---|---|
| **Farmer** | Bring wheat/carrot/potato/corn; plant a field | Tilled soil + chest |
| **Blacksmith** | Bring iron ingots; craft tools; bring coal | Furnace (T1.5) + anvil (future) |
| **Cook** | Bring cooked meat; bring baked vegetables | Campfire (Spec 17) |
| **Librarian** | Bring books; explore-and-report quests | Bookshelf + lectern (future) |
| **Carpenter** | Bring planks/logs; build a structure | Crafting table |

Each villager's profession is **set at first interaction** based on the nearest workstation block (Minecraft parity). Re-claim by destroying their workstation and placing a different one.

### 3. Iron Golem — village defender

A passive-to-villagers, hostile-to-hostile-mobs tall mob. Spawns automatically when a village has ≥ 3 villagers + ≥ 5 houses (Minecraft-ish trigger). Iron Golems:
- Walk around the village perimeter.
- Aggro on any hostile mob within ~20 blocks of any villager.
- Cannot be hurt by the player (kid-friendly — no "accidentally griefed the village defender" pain).
- One golem per ~10 villagers.

### 4. Reputation

Per-player, per-village. Tracked server-side. Affects quest payouts:
- **+rep**: complete quests, defend during raids, donate sats to the village fund.
- **−rep**: kill villagers (large hit, with cooldown — kid-friendly: don't let them lock themselves out by accident).
- **Effects**: high rep unlocks rare quests + bigger sats payouts. Low rep → villagers refuse to give quests until rep recovers (slow natural decay back to 0).

### 5. Village structure-gen

Procgen via the structure-gen pipeline placeholdered in Spec 2 §6. Phase 4 of this spec is where we actually wire it up.

- **Biomes**: Plains + Forest only at launch. (Desert/Tundra variants are future polish.)
- **Density**: roughly one village per 32×32 chunk area (matches Spec 2's `is_cave` / `tree_hash` density patterns).
- **Layout**: a small cluster of 3-6 procgen houses around a central well + village bell. Lit campfire in the centre (uses Spec 17 + 18 — village smoke pillar visible from far!).
- **Determinism**: same world seed + position = same village layout. Survives save/load.
- **Initial population**: 3-5 villagers spawned in claimed houses on first chunk-load.

### 6. Player-built villages

The **Village Bell** is a craftable block. Placing it claims a 32×32 area as a "potential village" anchor. The world spawns **Wandering Villagers** (a rare passive mob, ~1 per loaded 1000-block region) that will migrate to the nearest unclaimed bell. Once a wandering villager arrives, claims a house (any enclosed structure with door + bed), and the village has 3+ such claimed dwellings, it counts as a "village" for golem spawning + raid eligibility.

Recipe (T1.x; refines in T1.5):
```
[ - ][ I ][ - ]
[ I ][ S ][ I ]    I = Iron Ingot, S = Stick, plus 4 planks for the housing
[ P ][ P ][ P ]
```

## Phasing

13 phases. Comparable in shape to Specs 11 + 17 — comprehensive but bounded.

| Phase | What | Files | Approx LOC |
|-------|------|-------|------------|
| 1 | Foundation spec (this doc) | new | – |
| 2 | `MobType::Villager` + `MobType::IronGolem` + `MobType::WanderingVillager` — basic passive AI + texture + drops | `mob.rs`, `entity.rs`, `mob_ai.rs`, `texture_gen.rs`, `entity_model.rs` | ~250 |
| 3 | `Profession` enum on Villager — initial random assignment on spawn; data-driven `ProfessionDef` (name, quest pool reference, claim-workstation rule) | `mob.rs`, new `villager.rs` module | ~150 |
| 4 | Procgen village structure — house templates + cluster placement on `tree_hash`-style determinism. Plains + Forest biomes only | `world.rs::generate_column`, new `village_gen.rs` | ~400 |
| 5 | Right-click villager → dialogue overlay (egui). Shows profession + current quest offer | `villager.rs`, `game_loop.rs`, new `villager_ui.rs` | ~250 |
| 6 | Quest data model + 3 quest flavours (Fetch, Make, Kill) — server-side tracking + completion detection | new `quest.rs` module | ~300 |
| 7 | Quest reward payout — items into player inventory; sats hook stub (BRIDGE → resolved when Reserve sats path lands) | `quest.rs`, `game_loop.rs` | ~100 |
| 8 | Iron Golem mob — passive-to-player, hostile-to-hostile-mobs AI state; tall hitbox; auto-spawn on village threshold | `mob_ai.rs`, `mob.rs`, `entity.rs` | ~200 |
| 9 | Reputation system — per-player-per-village counter; quest payout modifier; villager-kill penalty | `quest.rs`, new `reputation.rs` | ~150 |
| 10 | Village Bell block + Wandering Villager mob + claim-and-migrate behaviour | `block.rs`, `mob.rs`, `mob_ai.rs`, `crafting.rs` | ~250 |
| 11 | Save/load — villager state, profession claims, reputation, village positions all survive | `save.rs` | ~150 |
| 12 | Spec 2 §6 update (structure-gen) + new Spec 5 §X.Y (villagers + villages) + foundations README | various docs | ~50 |
| 13 | Axolittle playtest gate | – | – |

**Total:** ~2,250 LOC across 12 build phases. Comparable to Spec 11 (~1,900 LOC) and Spec 12 (~2,500 LOC est).

Phases 2-12 are solo-buildable. Phase 13 is the playtest gate. The spec deliberately avoids parallel structure with Specs 1/2 (HostedServer / Signet) — no shared files; can ship independently.

## Per-phase detail

### Phase 2 — Villager + IronGolem mob types

- Add `MobType::Villager` and `MobType::IronGolem` (and `MobType::WanderingVillager` reserved for Phase 10).
- New `mobs.toml` entries: villager (health 20, passive, walking), iron golem (health 100, passive-to-players but in its own category, large hitbox).
- 7 new texture layers per mob (head front/side/top, body side/top, legs front/side — matches existing cow/zombie/pig pattern).
- Villager drops: **nothing** on death (intentional — encourages defending, not farming).
- Iron Golem drops: 3-5 iron ingots (motivates raids but not griefing — see below).
- **Anti-grief**: Villager has a damage-cooldown protecting it from accidental player hits — first hit is a "warning" toast, second hit within 30s actually damages. (Kid-friendly. Tunes per Axolittle's playtest.)

### Phase 3 — Profession

- `enum Profession { None, Farmer, Blacksmith, Cook, Librarian, Carpenter }`.
- `VillagerComponent { profession: Profession, claimed_workstation: Option<(i32, i32, i32)> }`.
- Initial spawn: `Profession::None` until they walk near a recognised workstation block.
- Workstation table — block → profession map:
  - `TILLED_SOIL` (any) → Farmer
  - `FURNACE` (future) → Blacksmith
  - `CAMPFIRE` (any state) → Cook
  - `BOOKSHELF` (future) → Librarian
  - `CRAFTING_TABLE` → Carpenter
- Claim is sticky until the workstation is destroyed.

### Phase 4 — Procgen village

- New `village_gen` module on the same hash-determinism pattern as `tree_hash` in `world.rs`.
- A village seed = `hash(world_seed, cx_centre, cz_centre)`.
- House templates as small block patterns (3×3×4 footprint with door + bed + roof). 3-4 template variants for variety.
- Cluster of 3-6 houses around an anchor point. Lit campfire (uses Spec 17 + 18) + a well (water source block in a 1×1 hole, bordered by cobblestone).
- One villager pre-spawned per house on first chunk-load — assigned to a profession matching the nearest workstation.
- Biome gate: only places villages in Plains or Forest. Desert/Tundra variants are future polish.

### Phase 5 — Dialogue overlay

- Right-click a villager when within 4 blocks → opens a dialogue UI (egui modal).
- Shows: villager name (procgen — "Villager #N" until Axolittle adds nicer names in playtest feedback), profession, current quest offer + reward preview.
- Buttons: **Accept**, **Decline**, **Close**.
- Only one active quest per (player, villager) pair. Decline puts it on a 5-minute cooldown.

### Phase 6 — Quest data model

```rust
pub enum QuestFlavour {
    Fetch { item: MaterialId, count: u8, destination: Option<(i32, i32, i32)> },
    Make  { item: MaterialId, count: u8 },
    Kill  { mob: MobType, count: u8, within_radius: f32 },
}

pub struct Quest {
    pub id: u64,
    pub giver: hecs::Entity,  // the villager
    pub flavour: QuestFlavour,
    pub reward: QuestReward,
    pub expires_tick: Option<u64>,  // None = no expiry; future polish
    pub accepted_by: Option<PlayerPubkey>,
    pub progress: u8,  // current count toward `count`
}

pub struct QuestReward {
    pub items: Vec<ItemStack>,
    pub sats: u64,           // BRIDGE — payout path lands when Reserve drain lands
    pub reputation: i16,     // signed: most quests give positive; failure gives negative
}
```

Quest generation: each villager has a profession-keyed quest pool. On dialogue-open, server picks a random quest from the pool seeded by `(tick, villager_id)`.

### Phase 7 — Quest payout

- **Re-check completion at TurnIn before paying.** The payout gates on
  `quest::may_turn_in` (= `is_complete` against the player's *current*
  inventory + kill count), not on the dialogue-open state. The dialogue only
  shows "Ready to turn in" when complete, but the player can drop the required
  items (or the kill state can change) between opening the dialogue and clicking
  Turn In — the old path discarded `fetch_make_consume`'s result and paid
  regardless, so accept→satisfy→drop→turn-in collected a free reward (engine
  audit 2026-06-04, A). On a failed re-check the quest stays active and a
  "missing something" toast fires. Regression: `quest::may_turn_in_blocks_after_required_items_dropped`, `quest::turn_in_gate_rejects_incomplete_kill_quest_that_consume_would_accept`.
- Items → player inventory via `inventory.add_item`; if full, drop at villager's feet.
- Reputation update → `reputation::adjust(player, village_id, delta)`.
- Sats hook: **BRIDGE** — for now, log to console + add a toast "you would have earned N sats on a Bitcoin server". Real payout path lands when Spec 16 Reserve drain integration matures (it currently has the gauge but no actual settlement).

### Phase 8 — Iron Golem

- New AI state `AiState::GolemGuard { home_village: (i32, i32, i32) }`.
- Patrols the village perimeter (cycles between a few stored waypoints from `village_gen`).
- Switches to Chase when any hostile mob is within ~20 blocks of any villager in the home village.
- Auto-spawn on village condition (≥3 claimed villagers AND ≥5 houses); cap at `(villager_count / 10).max(1)`.
- Hitbox: 2 blocks tall (vs 1 block for villagers). Reuses existing 2-tall hitbox logic from the player.

### Phase 9 — Reputation

- `pub struct Reputation { pub per_village: HashMap<VillageId, i16> }` on `PlayerSlot`.
- Tiers: ≤ -50 (Hostile — villagers refuse interaction), -50 to -10 (Wary — reduced rewards), -10 to 10 (Neutral — normal), 10 to 50 (Friendly — small reward boost), > 50 (Beloved — bonus rare quests).
- Quest completion: + flavour-dependent rep (5-20).
- Villager kill: -25 with 30s cooldown to prevent kid rage-loops.
- Natural decay: ±1 per in-game day toward 0.

### Phase 10 — Village Bell + Wandering Villager

- `block::VILLAGE_BELL` (id 50). Tall thin bell-shape mesh. Crafted from iron ingots + sticks + planks.
- Place → claims a 32×32 area as a potential-village zone (one bell per zone; overlapping placements show a toast).
- `MobType::WanderingVillager` — passive mob, very rare spawn (~1 per loaded chunk-cluster), wanders aimlessly.
- Within range of an unclaimed bell, a Wandering Villager pathfinds toward it, then claims the nearest house with door + bed + roof (volume check via `village_gen::is_valid_dwelling`).
- 3 claimed dwellings → village is "active" → Iron Golem can spawn, raid trigger is eligible.

### Phase 11 — Save/load

- Each entity-with-Villager-component serialises profession + workstation + reputation contributions.
- Village positions + bell positions saved as a `Vec<VillageMeta>` in WorldSave (analog to `campfires`).
- Quest state — server-side, transient (quests don't persist across saves on alpha; tracked in Phase 13 playtest whether kids find this annoying).

### Phase 12 — Docs

- Spec 2 §6 (structure-gen) — replace the placeholder with the village_gen wiring.
- Spec 5 — new section §3.13 (Villages & Villagers).
- Vision doc — note that Spec 19 phases 1-12 are the T2/T3 prerequisite for "Raid Defence" T3 content in §6.4.
- Foundations README — mark Spec 19 DELIVERED with file touchpoints.

### Phase 13 — Axolittle playtest

Open questions for the playtest:
- Does the village feel alive? Or do villagers wander too much / too little?
- Is the dialogue UI legible? Does an 11-year-old understand the quest screen?
- Quest pacing — too easy? too grindy? right ratio of fetch/make/kill?
- Reputation — does the system feel rewarding, or punishing when you accidentally hit a villager?
- Player-built villages — does the migration mechanic feel magical or tedious?
- Sats payouts — is the "+N sats" toast motivating, or noise?
- Iron Golem — is it scary (good)? Does the kid-friendly invulnerability feel right, or paternalistic?

## Cross-game lift

- **Villager mob + profession pattern** — engine-generic. Any Decented game with NPCs reuses the same `Profession` + workstation-claim mechanic.
- **Quest framework** (fetch/make/kill) — lifts everywhere. Game-specific quest pools data-drive.
- **Reputation tier system** — applies to any persistent-state game with NPC factions.
- **Structure-gen pattern** — lifts to any voxel game.
- **Bell-marker player-built-village mechanic** — applies anywhere players build infrastructure that wants persistent NPC inhabitants.

Per shared infra strategy: don't hardcode AxeNStax-specific assumptions into `villager.rs` or `quest.rs`. The Profession enum is game-specific (data); the mechanic is engine-generic.

## Bitcoin economy hook

On Bitcoin-enabled servers, quest sats payouts come from the **Spec 6 §7 server reserve** — the same pool that funds the proof-of-play hash-meter rewards. This means:
- Villages add a **work-for-pay** sats path **alongside** the existing **work-against-rock** (mining) path.
- The two paths compete for the same reserve — server operators can tune the split.
- For UK OSA / age-gate compliance (per bitcoin parent controlled): same per-server-policy + parent-flag controls apply. A village quest pays sats only if both server policy AND player's guardian flag allow it. Otherwise the quest still pays items + reputation; sats line just doesn't print.

This deepens the Bitcoin economy without complicating compliance.

## Open design questions

These are decisions to settle during build (or kick to Axolittle):

1. **Villager voice lines?** — silent ambient text only? Future voice-feedback-server integration (Spec 4) for actual TTS? Skip for now; ambient text fits Spec 5's tone.
2. **Trading items _between_ villagers (Minecraft's emerald economy)?** — explicitly skipped. Villager-to-villager trade would re-introduce the "NPC market" trap.
3. **Marriage / family trees?** — no. Out of scope. Wandering villagers spawn de novo; no genealogy.
4. **Multiple villages cooperating (alliances)?** — no for alpha. Each village stands alone.
5. **Should Iron Golems drop iron?** — currently yes (3-5 ingots). Tradeoff: motivates raids on enemy villages (T3 PvP), but also incentivises kid-grinds-village-defender exploit. Mitigation: very long respawn (24 in-game hours).
6. **Hostile mobs spawning INSIDE houses at night?** — no. Doors + roofs are villager safety. Hostile mobs siege from outside (Spec 18 campfire-attraction already pulls them toward villages — perfect synergy).

## Memory-rule check

- ✓ axenstax has farming — villages are the consumption side of farming (Farmer villagers buy crops).
- ✓ economies vision — slots into the services economy (work-for-pay) without competing with Vendor Block (player trade).
- ✓ bitcoin parent controlled — sats payouts respect per-server + per-guardian flags. No fresh compliance surface.
- ✓ shared infra strategy — Villager mob + Profession + Quest patterns are engine-generic; game-specific data drives them.
- ✓ uk english naming — UK English throughout (no "neighbor" vs "neighbour" issues).
- ✓ alpha open access — no whitelist gating; any signed-in player can interact with villages.
- ✓ proof of play is proof of work — quest sats payouts are deterministic work-for-pay; not chance-based. Same legal posture as Proof of Play.

## Acceptance criteria (Phases 2-12 combined)

- [ ] A loaded chunk in Plains or Forest within ~32 chunks of the player has at least one village.
- [ ] Each village has 3-6 procgen houses + 3-5 villagers + at least one lit campfire.
- [ ] Right-clicking a villager opens a dialogue with a quest offer.
- [ ] Accepting a Fetch quest, gathering the items, and returning to the villager pays out the reward.
- [ ] Iron Golems spawn when the village has ≥3 villagers and ≥5 houses; they kill nearby hostile mobs.
- [ ] Killing a villager costs 25 reputation; the second kill within 30s actually does damage.
- [ ] Placing a Village Bell in the wilderness eventually attracts Wandering Villagers (within a few in-game days).
- [ ] Save/load preserves: villagers + their professions + claimed workstations + village positions + bell positions + player reputation per village.
- [ ] On a Bitcoin-enabled server with a guardian-flag-allowed player, quest completion logs/toasts a satoshi payout amount (real settlement gated on Spec 16 Reserve drain landing).
- [ ] On a non-Bitcoin server OR a guardian-disabled player, quest completion still pays items + reputation; no sats line printed.
- [ ] All tests pass; check.sh ALL GREEN; bundle still under 5 MiB brotli.
