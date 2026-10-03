# Work-Based Proof-of-Play Hashing

**Date:** 2026-06-03
**Status:** DESIGN → **work-tally BUILT (Goal 1, 2026-06-03).** The work-as-hashes *tally* is live: `crafting::block_work(block_id)` (work = f(`block_hardness`), scaled so a leaf = 1) is added to the world's lifetime `total_work` and to a scenario's work score on every successful `can_harvest` break (survival break path in `game_loop.rs`; `World::add_work`; creative/instant breaks excluded). Per-world `total_work` / `total_ticks` / `genesis_found_at_tick` now persist on `WorldMeta` (Spec 2 §9.1). **Anti-farming added 2026-06-08:** player-placed blocks earn **no** work when re-mined — see "Anti-farming" below. **Still deferred** (a later Spec 6 goal): the actual SHA-256 / nonce rolls, the rare-drop/sats reward layer, and the educational Layer-1 "every strike hashes" visualisation. Fold into Spec 6 proper when the Proof-of-Play section is next revised. (NB: Spec 6's settlement/payout sections are PARKED — this touches only the §2 *hash mechanic*, not settlement.)

## The reframe: hashes = work done, not strikes

**Earlier model** (Spec 6 §2 as written): *one HMAC-SHA256 per strike*, uniform across tools; the tool is a *gate* (which blocks you can drop from) + reward weight, not a hash multiplier. The educational "every strike hashes (even grass)" Layer-1 visualisation is deferred/unbuilt; in code the hash currently only runs on qualifying deepslate breaks inside `maybe_gem_drop` (`game_loop.rs:265`).

**New model** (2026-06-03): **a hash represents a unit of work actually accomplished.** The number of hashes a block is worth = a function of its **hardness** (the work to break it). The tool determines *capability* (can you do the work at all) and *speed* (how many strikes / how long), but **never the hash count.**

### The rules
- **Hashes for a block = f(`block_hardness(block_id)`)** — the block's intrinsic work. Leaves ≈ 1 hash (lowest non-instant hardness); deepslate many (≈ 3× stone).
- **Awarded only on a successful `can_harvest` break.** If the tool can't harvest the block (bare hands on stone/deepslate, wood pick on iron, …), no work is done → **no hashes**. (Exactly "you simply can't do it, so nothing, so no hashes".)
- **Tool = capability + speed, not hash count.** An axe chops wood in fewer strikes than a bare hand, but the wood's work (hashes) is fixed by its hardness.
- **Award on break (atomic).** The unit of work is a *completed* block; no partial credit for hits that don't finish it.
- **Excludes creative / instant breaks** (`break_time == 0`).
- **Excludes player-placed blocks.** A block a *player* placed earns **0** work when re-mined — you still recover the item, but the credit is withheld. See Anti-farming below.

## Anti-farming: player-placed blocks earn no work (BUILT 2026-06-08)

**The hole:** work was awarded on *any* harvestable break, with no notion of where the block came from. So a player could chop a block, restand/replace it, and re-break it — "rehashing their own work" — to farm a Hash Dash score (the playtest-flagged boring win), or, on a Bitcoin-enabled server, mint payouts from a free place→break loop. Spec 6 §2.2's anti-X-ray defence (server_secret) doesn't cover this: the cheat isn't *reading* hidden value, it's *manufacturing* work.

**The rule (every world — survival, scenarios, Bitcoin servers):** a block a player placed earns no proof-of-play hash/work when broken, and yields no hash-driven drop (Satori / Bitcoin). Breaking and item recovery are unchanged — only the reward credit is withheld. "Anything you place" counts: a stone, a workbench, a sown crop (the produce still drops; the work doesn't). Tilling/transforming terrain in place is not a placement.

**Mechanism — a per-voxel "placed" bit:**
- Each `Chunk` carries a 4096-bit mask (`is_placed`/`set_placed`), one bit per voxel, serialized alongside the block array (Spec 2 §4.3). Pre-feature saves have no mask → all blocks read natural (backward-compatible).
- `World::place_player_block` sets the block **and** the bit; plain `World::set_block` (world-gen, lighting, decay, growth) never touches it. The player-placement + crop-planting paths in `game_loop.rs` use the former; breaking clears the bit.
- The decision is one shared pure function — `crafting::break_work(block_id, harvestable, was_player_placed)` — called by both the production break path and `TestHost::mine_block`, so they can't drift.
- **Falling blocks carry their origin:** sand/gravel keeps its placed bit across a fall, so dropping a placed block onto a ledge and re-mining the landing spot doesn't launder it into a natural one.

**Deferred:** the server-authoritative multiplayer path (`server.rs`) doesn't yet set the bit on remote-player placements — harmless today because the reward tally is host/single-player-side only (`game_loop.rs`). Wire it when single-player routes through `HostedServer` (known dual-sim debt) or the multiplayer fleet (Spec 07) lands.

### Maps onto existing engine functions (so it's low-cost)
- `block_hardness(block_id) -> f32` (`crafting.rs:270`) — documented as *"seconds to mine bare-handed"* = the work value. **Already exists.**
- `can_harvest(block_id, tool) -> bool` (`crafting.rs:378`) — the capability gate. **Already exists** (tests confirm bare hands can't harvest stone; wood pick can't harvest iron; etc.).
- `break_time_ticks(block_id, tool)` (`crafting.rs:387`) — tool changes break *time*, not hardness.

The model is mostly *data we already have*: tally `block_hardness` on each `can_harvest` break.

## Design knobs (tune at implementation / playtest)
1. **Scale:** define "1 hash = one leaf's worth of bare-hand work"; everything scales off `block_hardness` from there.
2. **Audit / determinism:** N hashes per block, `N = f(hardness)`, inputs = `HMAC(secret, world_seed‖epoch‖x‖y‖z‖nonce)` for `nonce` in `0..N-1` — fully deterministic + replayable, preserving Spec 6's verifier-replay property. "More work = more hash-rolls = more reward chances" falls out naturally (deepslate is richer *because* it's harder).
3. **Subsumes the deferred Layer-1:** this makes "every break hashes" real in a principled, work-weighted form, unifying the educational + reward layers.

## The score (Hash Dash) is even cheaper
Hash Dash's score = **total work = sum of `block_hardness` over harvested blocks** = the hash *count*. You **don't need to run actual SHA-256 to compute the score number** — the score is the work tally. Real hashing (SHA-256) is the separate educational-visualisation + rare-drop layer.

## Balance note — the leaves-vs-tools tension (Hash Dash)
With **hash ∝ hardness** and **break-time ∝ hardness**, the two largely *cancel* → **hash-rate ≈ tool mining-speed**, roughly independent of which block you hit. Consequences:
- A bare-handed noob is stuck on leaves at a low rate (can't harvest stone at all); a better tool raises the rate **and** unlocks harder/richer blocks. "Invest in tools" = "raise your rate", with setup/positioning cost as the short-game offset.
- **Risk:** under a *linear* hash↔hardness map, "go deep for deepslate" may not out-earn "farm leaves" (the rate cancels) → the choice is solved/boring. Levers to give it real depth:
  1. **Steeper-than-linear** hash value for hard blocks (deepslate worth disproportionately more) so a good-tool-on-hard-block clearly out-rates hand-on-leaves.
  2. **Arena layout:** easy blocks at spawn, rich seams a short trek away (risk/reward of travel).
  3. **Timer as master dial:** ~2-3 min sits on the "is it worth investing?" knife-edge; the round length slides the whole crossover.
- **MVP guidance:** don't over-balance pre-playtest. Ship score = work, tools in the kit (so "investing" is a *positioning* decision, not a crafting grind there's no time for), a slightly steeper-than-linear curve, varied arena; let playtest tune the crossover. Noob-defaults-to-leaves vs savvy-reads-the-clock-and-invests is **skill expression — a feature, not a bug.**

## Relation to the two demo games
- **Hash Dash:** score = total work-hashes — this model *is* its scoring engine.
- **Satori Rush:** unaffected by the score model (its measure is *time to first Satori / Genesis Block*); per-block work still accrues into the world's `total_work` stat.
- **Satori:** the hardest, rarest work → naturally the highest-hash block (a "jackpot" under any reward-roll layer), though Hash Dash's score stays pure work regardless.

## Per-world stat tie-in (CORE — every world)
Each successful `can_harvest` break adds its work to the world's lifetime **`total_work: u64`**, a **universal `WorldMetadata` field present in every world** — see **Spec 2 §9.1 "World Metadata" → Statistics** (the canonical home), alongside the existing `total_ticks` (world-clock) and the genesis fields (`genesis_block_found`, `genesis_found_at_tick`). This is NOT a demo/scenario feature — it's core world data the demo games merely read. So "how much work has this world done" is the cumulative sum of this model — the same number Hash Dash scores within a round.
