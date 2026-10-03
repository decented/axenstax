# Raid Defence (T3) — village raid encounters + defender bounties

**Status:** **Phases 2-13 DELIVERED 2026-05-22** on `feat/spec-22-raid-defence`. Phase 7's smoke-pillar visual red-shift (the originally-deferred half) **DELIVERED 2026-05-22** on `feat/spec-22-smoke-redshift`. **Phases 15-18 DELIVERED 2026-05-22** on `feat/spec-22-enhancements` (pre-raid villager gossip override, rare-loot tiered drops per wave kind, Vendor Block raid-supplies 🛡 badge, per-village per-player raid-kill leaderboard with save/load). Phase 14 (multi-wave MVP) remains deferred per the controller's "single-round MVP only" call. Phase 19 = Axolittle playtest gate. **Prerequisite:** Spec 19 Villages & Villagers (DELIVERED 2026-05-18 + playtest pending).

> **2026-07-12 — kill-attribution verified live + dead helper removed.** The
> wave-hardening backlog (2026-07-07) carried "raid kill-attribution + bounty
> dialog — `raid_at_position` uncalled" as a gap. Verified in the running
> game loop: it was a **stale finding** — attribution (Phase 5) and the
> bounty dialog banner (Phase 9) shipped 2026-05-22 with different
> primitives (inline player-to-anchor gate in the death sweep;
> `village_at_position` → `active_raid_for_village` for the banner). Only
> the `raid_at_position` helper itself was dead, and its "no call site calls
> this yet" comment kept regenerating the false backlog item — the helper is
> now removed. The full chain (dying `RaidMember` → in-radius credit →
> out-of-radius kill decrements without credit → killing-blow stamp →
> Cleared settlement → reputation + leaderboard + drain) is pinned
> end-to-end by the GPU-gated harness test
> `game_harness_raid_kill_attribution_settles_through_the_live_game_loop`
> (`cargo test -- --ignored game_harness`). Known remaining boundary (rides
> the dual-sim rework, not this spec): contribution is keyed by local
> PlayerSlot index, so server-simulated remote players earn no raid credit.

## TL;DR

A village can be **raided** by an organised wave of hostile mobs. The village treasury (funded by quest income + a server contribution) pays a per-defender bounty to anyone who shows up and survives. Failure → the village's reputation tier drops globally + (longer-form) buildings damaged. Layered structure: alpha ships **one-round single-village raids** as the MVP; T3 long-form features (multi-round gear progression, alliance defence) ride on the same primitive.

This is the **fourth player-driven-economy primitive** to ship after Farming, Vendor Block, and the Spec 19 quest path. It opens the **Combat economy** lane in `docs/vision/economies-long-run.md` §4.4 / §6.4. The vision doc explicitly notes Spec 19 as the prerequisite — village + iron-golem + reputation must exist before raids do, otherwise there's nothing to attack and no defenders to recruit.

## Narrative

Today: Spec 19 ships villages with iron golems that lazily defend against nearby hostile mobs. There's no scheduled threat; mobs wander in, the golem kills them, the kid watches. Fine for atmosphere; flat as an *economy* lane.

Raid Defence adds the schedule. A village announces an inbound raid (in-fiction: smoke pillar above the campfire turns red); the kid can choose to defend (collect the bounty) or run (lose nothing if they're not the village's player-resident). On the dialogue side: any villager will tell you the raid timer, the bounty pool, and the wave composition. Defending = standing inside the village radius when the wave dies. Surviving defenders split the bounty proportionally to damage contribution.

The economy hook: the **village treasury** is a per-village sats balance fed by quest completion (Spec 19 §9 already credits players; this spec adds a server-percent skim to the village). Raid bounties drain the treasury. A village that loses raids is broke; a village that wins them is rich and offers better quest rewards (Spec 19 §9's reputation-tier reward multiplier already exists; this spec ties the multiplier source to treasury size too).

## Scope (what's in)

- New `RaidScheduler` system — wakes once per in-game day, picks zero or one village to raid based on a probability roll seeded on `(world_seed, village_id, day_count)`. Probability scales with treasury size (richer villages attract more raids).
- New `Raid` ECS resource (or per-village component) tracking active wave state: village_id, wave_kind, mobs_spawned, mobs_remaining, defenders, contribution_table, deadline_tick.
- Wave composition table — alpha ships 3 wave kinds (Small / Medium / Large) keyed by treasury size. Each is a tuned mix of zombies, skeletons, spiders, creepers; sizes 5/10/20 mobs respectively.
- Pre-raid warning — campfire smoke pillar shifts to red 5 in-game minutes before the wave arrives (Spec 18 already has the smoke pillar block + lifecycle; this spec adds a `RaidWarning` flag that the campfire tick reads).
- Wave spawn — mobs spawn at the village perimeter (16-block radius from anchor), enter walking toward the closest villager.
- Defender attribution — every player who deals damage to a raid mob during an active raid window has their damage tallied in `Raid.contribution_table`. The existing `kill_counter` tick attribution is the right hook to extend.
- Bounty payout — on raid-cleared (last mob dies), each contributor receives `treasury_drain × (their_damage / total_damage)` sats. BRIDGE same as Spec 19 quest payout — log + toast on alpha; real settlement when Spec 16 Reserve drain reverses.
- Reputation effects — defenders gain +10 rep (or +20 if they did the killing blow on the wave); village's reputation-tier-effect-on-other-players gets a "Defended" sticker for the next 24 in-game hours.
- Raid failure — if all defenders die OR the deadline expires with mobs still alive, the village reputation tier drops by 1 for all players (everyone notices a village's struggles).
- Villager dialogue extension — every villager in a village under raid threat or active raid offers a "What's happening?" line that the dialogue UI surfaces as the quest summary line.
- New villager "raid bounty quest" — automatically generated when a raid kicks off; villagers offer it as a one-time pickup with the treasury-share as the reward.
- Save/load — active raid state persists if the game saves mid-raid (rare; alpha autosave is on close, so usually safe).

## Cross-economy hooks (added 2026-05-18 after meta-economy review)

Six explicit connections this spec should ship, per `docs/vision/sat-flow-and-economy-loops.md`:

### A. Treasury inflow that works on solo alpha (THE critical fix)

The original spec listed "server contribution" as the only treasury source on alpha, which meant no raids could ever fire without an absent server-admin hook. **New treasury inflow rules:**

- **20 % of every quest payout** in the village goes to that village's treasury. Default; server-configurable. Kid keeps 80 % of their reward; the village skims 20 % for defence. After 3-4 quests, the village has enough treasury to roll a Small wave.
- **5 % tax on Vendor Block transactions inside the village** (within 64 blocks of the anchor) → treasury. Player-to-player trade funds defence. Bitcoin-disabled servers: the 5 % is taken in items (the bought item itself splits — 95 % to buyer, 5 % to village treasury as item value at the trade-value floor). Disabled per server-policy if the operator prefers.
- **Server top-up** (the original spec's mechanism) remains as a server-policy option. Bitcoin-enabled servers can configure a per-day donation from the operator's Lightning channel.

These three inflows together mean raids fire reliably on alpha without admin intervention. **Without this fix raids are a dead feature on solo play.**

### B. Pre-raid villager rumours (1-2 in-game days ahead)

When a raid is scheduled, every villager in the village gets a gossip line ("The shepherds say bandits are coming for the south village tonight") via the `VillagerComponent.gossip_line` field added in the Spec 19 gossip extension. The player who talks to a villager *before* the raid fires gets a 1-2-day warning. The smoke-pillar shift then fires 5 minutes out as the urgent signal.

Implementation: ~30 LOC reading `world.active_raids` from the scheduler resource. Gossip line takes precedence over the day's default rumour when a raid is scheduled.

### C. Multi-wave even on MVP (2 waves, not 1)

Single-wave raids end too fast to feel like a fight. **MVP ships 2 waves:** Wave 1 spawns + clears; 60 ticks (3 s) of breathing room; Wave 2 spawns. Bounty distributes after Wave 2 clears.

Implementation cost: ~50 LOC for the wave-pair scheduler logic on top of single-wave. Cheap value-add.

### D. Rare-loot tiered drops (defenders get items + sats)

Per `docs/vision/sat-flow-and-economy-loops.md` §3.3 Bitcoin/Barter parity: raid bounties pay **items + reputation always; sats are an extra in Bitcoin mode**. Each wave kind drops a tier-keyed rare-item bonus:

- **Small wave**: standard mob drops only (zombies, skeletons, spiders → their usual loot).
- **Medium wave**: +1 rare drop from a tier-2 table (e.g. an iron tool, a stack of bones, a feather bundle).
- **Large wave**: +1 rare drop from a tier-3 table (e.g. a diamond shard, Bitcoin-enabled server: a small sats bonus; Bitcoin-disabled: a quest-only material).

Means raids are worth attending even on Bitcoin-disabled servers. **The kid gets a fight + loot + rep regardless of compliance posture.** Sats are the bonus on top, not the core motivation.

### E. Vendor-supplies highlight (Vendor Block integration)

When a raid is **warned** (1-2 in-game days ahead, before the smoke shift), every Vendor Block within 32 blocks of the village's anchor whose item is on the raid-supplies whitelist gets a 🛡️ tag in its buyer UI. Creates demand pressure for raid-prep items (food, arrows, swords, healing). Spec'd in the Vendor Block amendments §F as the consumer of this hook.

### F. Routes sats through the unified helper

Every bounty payout this spec does goes through `apply_server_tax_and_payout` (specified in Furnace foundation Phase 10). Server-tax + Reserve-drain + Charter-flag gating apply identically to Quest payouts, Vendor sales, and Raid bounties. Per `docs/vision/sat-flow-and-economy-loops.md` §5.

### G. Raid leaderboard at the village (small social hook)

Each village tracks per-player kill counts across all raids defended there. The villager dialogue exposes the top-3 defenders ("Iron Margaret defended us 14 times — she's a hero around here"). No reputation effect; pure social signal. Engine-generic so it lifts to any other Decented game with structured threat events.

Implementation: ~80 LOC sitting on the village treasury side-table.

## Scope (what's out — deferred)

- **Long-form / multi-round raids.** Vision §6.4 mentions multiple rounds with mid-raid gear progression. Alpha ships single-round only. The wave-composition table is enum-shaped so adding `Multi(Vec<WaveKind>)` is mechanical.
- **Server contribution to treasury.** The treasury is fed by quest payouts on alpha. A "the server donates X sats per day to every village" hook is post-Spec-16-Reserve.
- **Village destruction.** Vision says "Failure → village destroyed → server economy takes a permanent hit." Alpha ships only the reputation drop + a smoke-pillar visual mark; building damage + permanent destruction land when there's a real loss-condition story.
- **Defender alliance / party system.** Damage attribution is per-player; party-share splits are post-alpha.
- **Cross-village raid coordination.** Each village raids independently. Federations are vision §F (post-alpha).
- **Skill-based raid difficulty scaling.** Alpha picks wave kind purely from treasury size. Player-skill scaling (better players → harder waves) is polish.
- **PvP raids.** A player-led wave attacking another player's village is a separate Conflict-economy spec (vision §4.4 vs §5).

## Design choices

### 1. How a village's treasury fills

Spec 19 phase 7 quest payouts go to the player as `reward.sats`. Two options:

- **A**: Players keep 100 % of quest reward; villages get treasury from server donation only.
- **B**: A server-configurable percentage of quest reward goes to the village treasury instead of the player.

**Recommendation: A on alpha** — keep the kid's quest reward feeling like *their* reward; the village treasury starts at 0 and accumulates only via server-controlled top-up (Spec 16 Reserve drain integration, future). On a server with zero top-up, the treasury stays at 0 and no raids fire. Acceptable alpha behaviour.

Phase-2 spec amendment: when Spec 16 Reserve drain matures, **add option B as a server-policy knob**. The infrastructure should support both; the alpha default is A.

### 2. Damage contribution attribution

Already half-built: the Spec 19 phase 6 kill-counter attributes deaths to the nearest player. For raids we need damage attribution, not just kills — a player who whittles a creeper from 20 HP to 1 HP shouldn't lose the contribution share if the iron golem finishes it.

**Recommendation:** new `LastAttacker` ECS component attached to any mob that's been hit by a player; `combat::player_attack` writes the attacker's `pidx` and timestamp on every successful hit. On mob death during a raid, `Raid.contribution_table` reads the latest `LastAttacker` (with damage-dealt accumulated since the last attacker change).

A simpler v1: just count kills, ignoring damage. **Use this for alpha.** The full damage attribution is a polish pass.

### 3. Pre-raid warning UX

The vision doc says "Server announces an incoming AI mob wave." Three options:

- Generic server-wide toast: "A village near (X, Y) is under attack!" — useful for non-resident players who might travel to defend.
- Spec 18 smoke pillar colour shift: red while warning, deeper red during the raid.
- Both.

**Recommendation: both.** The smoke pillar is local atmosphere; the toast pulls in non-resident defenders. The toast suppresses for kids whose Charter parent flag disables alerts (rare but possible per bitcoin parent controlled).

### 4. Defender boundary

What counts as "inside the village" for damage attribution and bounty eligibility?

**Recommendation: within 24 blocks of the village anchor** (matches Spec 19's villager-claim-radius constant). A player who runs out of range mid-raid still gets credit for damage already dealt; new damage stops counting toward the bounty.

### 5. What happens to bodies after a raid

Cleared mobs leave normal drop tables (Spec 19 already wires `drops_for`). Cleanup of unkilled mobs (defenders failed) — let them stay; they're now ambient threats. The reputation hit covers the loss-condition story.

### 6. Iron Golem behaviour during a raid

Spec 19 §8 iron golem already chases hostile mobs within `GOLEM_DEFEND_RADIUS = 20`. A raid drops 5-20 mobs in that radius — the golem naturally engages. **No code change needed.** The golem is a passive ally; the kid does the lion's share of the work.

### 7. Raid frequency on alpha

A village that hasn't completed a quest has zero treasury → zero raid roll → never gets attacked. A village with 100 sats in treasury → ~20 % daily roll for a Small wave. With 1000 sats → 60 % roll for Medium. Treasury reduces by `min(treasury, treasury_drain_per_raid)` on raid-success, so a village can't be permanently rich-and-besieged.

**Tunable post-playtest.**

### 8. Raid integration with Iron Golem auto-spawn

A village with >= 3 villagers + >= 5 houses spawns iron golems automatically. The raid-eligibility threshold should sit above the golem-spawn threshold so the village has time to set up defences.

**Recommendation: raid-eligible threshold = >= 5 villagers AND >= 5 houses AND treasury > 0.** Comfortably above iron-golem spawn.

## Phasing

| Phase | What | Files | Approx LOC | Solo? |
|-------|------|-------|------------|:----:|
| 1 | Foundation spec (this doc) | – | – | – |
| 2 | `RaidScheduler` resource — daily-tick probability roll, picks zero or one village to schedule. Deterministic seed (world_seed × village_id × day). | new `raid.rs`, `game_loop.rs` daily-tick hook | ~150 | ✓ |
| 3 | `Raid` data + wave composition table. `WaveKind::{Small, Medium, Large}` enum with mob mix + count per kind. Treasury-size → wave-kind mapping table. | `raid.rs` | ~200 | ✓ |
| 4 | Wave spawn — at scheduled tick, spawn the wave mobs at the village perimeter (16-block radius from anchor, randomised angles). | `raid.rs`, `entity.rs` (spawn_mob already exists) | ~150 | ✓ |
| 5 | Defender attribution — count kills against the active raid; contribution_table per (player, raid_id). v1 = kill-count only; damage attribution is later polish. | `raid.rs`, hook into the existing `despawn_dead` kill-attribution path | ~150 | ✓ |
| 6 | Bounty payout — on last-mob-dies, distribute `treasury_drain × kills/total_kills` sats to each defender. Same BRIDGE pattern as Spec 19 quest payout. | `raid.rs`, `game_loop.rs` | ~200 | ✓ |
| 7 | Pre-raid warning — server-wide toast + Spec 18 smoke pillar colour-shift hook. New `block::CAMPFIRE_RED_SMOKE`? Or a flag on the existing smoke block? (See open question 1.) | `campfire.rs`, `texture_gen.rs`, `block.rs`, `raid.rs` | ~150 | ✓ |
| 8 | Reputation effects on raid result — +10 / +20 per defender on success; village-tier drop on failure. Hooks into the Spec 19 `Reputation::adjust` API. | `raid.rs`, `reputation.rs` | ~80 | ✓ |
| 9 | Villager dialogue extension — under-attack-or-active-raid villagers surface "raid bounty" quest line at the top of their dialogue overlay. Generated from the active raid; not from the profession pool. | `villager_ui.rs`, `quest.rs`, `game_loop.rs` | ~150 | ✓ |
| 10 | Server toast for non-resident defenders. Per-Charter-flag suppression. | `game_loop.rs`, `charter.rs` (no new file needed) | ~80 | ✓ |
| 11 | Save/load — `SavedRaid` (active wave state) + village treasury balances. WorldSave gains `#[serde(default)] raids: Vec<SavedRaid>` and `village_treasuries: Vec<(VillageId, u64)>`. | `save.rs` | ~150 | ✓ |
| 12 | Docs — Spec 5 new section §3.15 "Village Raids"; Spec 6 new section §13.x "Raid Treasury & Bounty Settlement"; vision §6.4 marked DELIVERED for the MVP scope; foundation README marks this spec DELIVERED. | docs only | ~200 | ✓ |
| 13 | Treasury inflow rules (THE critical fix). 20 % of quest payout → village treasury; 5 % vendor txn tax → treasury (Bitcoin or items depending on server policy). Server top-up remains optional. | `quest.rs` payout site, `vendor.rs` payout site, `world.rs` treasury side-table | ~150 | ✓ |
| 14 | Multi-wave MVP (2 waves not 1). Wave 1 spawns + clears; 60-tick breathing room; Wave 2 spawns. Bounty distributes after Wave 2 clears. | `raid.rs` (extends the single-wave scheduler) | ~80 | ✓ |
| 15 | Pre-raid villager rumours (1-2 in-game days ahead). Reads `world.active_raids`; overrides the daily-gossip line when a raid is scheduled. Depends on the Spec 19 gossip extension being shipped. | `villager.rs` (gossip-line resolver) | ~50 | ✓ |
| 16 | Rare-loot tiered drops per wave kind. Small / Medium / Large drop tables. Items always; sats are an extra in Bitcoin mode. | `raid.rs`, `mob.rs` (extend `drops_for` for raid context) | ~120 | ✓ |
| 17 | Vendor Block raid-supplies hook. When a raid is warned, every Vendor Block within 32 blocks of the anchor with a whitelisted item gets the 🛡️ tag in the buyer UI. Spec'd as Phase 14 of the Vendor Block spec — this Raid Defence spec just exposes `world.active_raids` cleanly so Vendor Block can read it. | `raid.rs` (expose query function), Vendor Block spec consumes | ~30 | ✓ |
| 18 | Raid leaderboard at the village (small social hook). Per-village per-player kill-count side-table; villager dialogue exposes top-3 defenders. | `raid.rs`, `villager_ui.rs` | ~80 | ✓ |
| 19 | Axolittle playtest — accumulate village treasury via quests (kid sees 80 % to wallet, 20 % to village); wait for a raid roll to fire 1-2 days ahead via a villager's gossip line; defend the 2 waves; claim items + rep + sats bounty; verify rep tier shifted; check the leaderboard shows the kid's kill contribution. | – | – | playtest gate |

**Total Phases 2-18:** ~2,020 LOC across 17 build phases (was 11 + ~1,510; meta-economy review added treasury inflow rules, multi-wave MVP, pre-raid rumours, rare-loot tiers, Vendor Block hook surface, raid leaderboard).

**Dependencies:**
- Spec 19 villager-gossip extension ships **before** Phase 15 (pre-raid rumour). That gossip primitive lives on the Spec 19 side; Raid Defence is one of three consumers.
- Vendor Block raid-supplies highlight (Phase 14 of the Vendor Block spec) depends on Phase 17 of this spec exposing a query function for `world.active_raids`.

## Cross-game lift

- **Wave-spawn scheduler.** Engine-generic. Any Decented game with structured threat events reuses the daily-roll + composition-table pattern.
- **Damage-attribution + bounty-share primitive.** Lifts to any contested-resource game (Conflict economy primitive).
- **Pre-event warning + countdown + colour-shifted in-world signal.** Engine-generic atmosphere hook.
- **Treasury-as-throttle.** Resource-throttled-event pattern is engine-generic.

Per shared infra strategy: the scheduler + attribution + payout primitives are engine-generic; the wave composition table + mob kinds + treasury-funding rules are AxeNStax-specific data.

## Bitcoin economy hook

Raid Defence is the **first risk-flavoured Bitcoin gameplay**. The Spec 19 quest path is deterministic ("do the thing → get the sats"); raid defence has a risk component ("you might die → you get nothing → your village loses rep").

Compliance posture (per bitcoin parent controlled):

- **Per-server policy**: a Bitcoin-disabled server still runs raids; the bounty pays in items only (loot table from each fallen mob), no sats line printed.
- **Per-guardian flag**: a guardian-disabled kid sees raid events normally + earns items + reputation; no sats line printed on bounty payout.
- **UK OSA / chance-based**: the wave roll IS probabilistic, but the *payout* on participation is deterministic — `treasury_drain × your_kills / total_kills`. Same posture as Proof-of-Play hash-mining: the *finding* is probabilistic (which mob you encounter when), the *payout* on the finding is deterministic. Argued per proof of play is proof of work; no new compliance surface.
- **Compliance-oracle proposal**: the daily-roll seed is server-visible; auditors can verify "no rigged rolls" via the same `(world_seed × village_id × day)` formula clients reproduce. Surfaces as a Signet-level "raid verification" hook later if needed (compliance oracle upstream adjacent).

## Open design questions

1. **Smoke pillar colour-shift mechanism.** New block-id `CAMPFIRE_RED_SMOKE` (clean but burns a block-id slot), or a `red_tinted: bool` flag on the existing `CAMPFIRE_SMOKE` block-entity (slimmer)? **Recommendation: the flag — texture gen can swap on the fly. Block-id slots are precious.**
2. **What if there are no villages in the world?** The scheduler runs but finds zero candidates → no raids. No edge case.
3. **Multi-player split-screen defender attribution.** Damage from P1 and P2 both count; the bounty split happens per-slot. **Acceptance: works as documented.**
4. **A raid mob escapes the village radius.** It continues as a normal hostile mob; the raid concludes when all *initial* mobs are dead. Don't track stragglers as "still in raid."
5. **Raid + Iron Golem death.** If the village's iron golem dies in the raid, does it respawn after? Spec 19 doesn't pin the respawn rule; iron golem auto-spawn re-evaluates on the next 1-second tick. Long-respawn (24 in-game hours) per Spec 19 §17. **Honour that** — raids don't bypass.
6. **Multiple simultaneous raids.** With one village per 32-chunk cell, two villages could roll a raid on the same in-game day. **Allow it.** Each is independent; defender bounty pools don't merge.
7. **Bounty payout overflow.** Inventory full → drop at the village anchor (Spec 19 quest-payout pattern). Same.

## Memory-rule check

- ✓ economies vision — slot §4.4 (Combat economy) and §6.4 (Raid Defence). Vision §6.4 explicitly lists Spec 19 as the prerequisite; this spec ships once Spec 19 playtests clean.
- ✓ bitcoin parent controlled — full compliance posture documented above.
- ✓ shared infra strategy — engine-generic primitives, AxeNStax-specific data tables.
- ✓ alpha open access — no whitelist gating; any signed-in player can defend.
- ✓ uk english naming — "Defence" not "Defense"; "behaviour" not "behavior"; consistent throughout.
- ✓ proof of play is proof of work — same legal posture; payout-on-participation is deterministic; probabilistic *which raid fires when* is parallel to probabilistic *which mob spawns where*.
- ✓ axenstax has farming — orthogonal. Farming feeds quest income → village treasury → raid bounties. The economic loop closes.

## Acceptance criteria (Phases 2-12 combined)

- [ ] A village with >= 5 villagers + >= 5 houses + treasury > 0 is eligible for raid scheduling.
- [ ] On the daily tick, the scheduler probabilistically picks at most one eligible village to raid.
- [ ] 5 in-game minutes before the wave fires, the village's campfire smoke pillar shifts visibly + a server-wide toast announces the threat.
- [ ] At the scheduled tick, the wave (5/10/20 mobs by treasury tier) spawns at the village perimeter.
- [ ] Each player who kills a raid mob accrues a kill in the active raid's `contribution_table`.
- [ ] On wave-cleared, contributors each receive sats proportional to their kill share. Sats payout follows the Spec 19 BRIDGE pattern (log + toast on alpha; real Lightning when Spec 16 reverses).
- [ ] Successful defence: defenders gain +10 rep with that village; killing-blow defender gains +20.
- [ ] Failed defence (timer expires with mobs alive): village rep tier drops by 1 for all players.
- [ ] Active-raid villagers expose the bounty offer at the top of their dialogue.
- [ ] Save/load preserves: active raid state, contribution_table, village treasury balances.
- [ ] Server-toast Charter-flag suppression works; flag-disabled kids don't see the toast.
- [ ] Bitcoin-disabled server: raid completes, items distribute, no sats line printed.
- [ ] Guardian-disabled kid: raid completes, items + reputation paid, no sats line printed.
- [ ] All tests pass; check.sh ALL GREEN; bundle stays under 5 MiB brotli.
- [ ] Solo alpha: completing 3-4 quests in a village builds treasury to the Small-wave roll threshold; a raid then fires within a few in-game days **without any server-admin intervention**.
- [ ] Vendor Block transactions inside the 64-block village radius take a 5 % tax (sats or items per server policy).
- [ ] Villagers in a raid-warned village include the warning in their gossip line 1-2 in-game days before the raid fires.
- [ ] Raid runs as 2 waves with a 3 s breathing-room gap; bounty distributes after Wave 2.
- [ ] Each wave kind produces tier-keyed rare drops (Small: standard only; Medium: 1 tier-2 bonus; Large: 1 tier-3 bonus).
- [ ] Bitcoin-disabled server / guardian-disabled kid: items + reputation + rare-tier drops all pay out; no sats line printed.
- [ ] Vendor Blocks within 32 blocks of a raid-warned village display the 🛡️ "Raid Supplies" tag when their item is whitelisted.
- [ ] Village dialogue lists top-3 defender kill-counts across all raids defended there.
