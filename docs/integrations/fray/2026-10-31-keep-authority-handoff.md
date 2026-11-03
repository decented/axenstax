# AxeNStax → Fray/Ingle: authority integration handoff (2026-10-31)

Game-owned. Written for the Fray/Ingle design session. AxeNStax owns every change and build on its
side; only slice 1 (§7) is implemented. Section 6 lists the storage and migration choices that need
joint design **before** AxeNStax implements anything beyond slice 1. A full WASM port waits for
that agreement.

Evidence: two read-only passes over AxeNStax `8504a6b8` (engine identical to `3a62eed9`), Ingle
`ingle-design-k5`, and Fray `fray-design-k5`. References to `.claude/epic/…`,
`ingle-design-k5/` and `fray-design-k5/` are local to the owner's machine and not in this repo. Paths are relative to `game/engine/src/` unless
prefixed. Contract = `fray-design-k5/docs/authority-contract.md` v0.1; migration =
`fray-design-k5/docs/host-migration.md` v0.1.

## 1. Tested commit, handover failures, manual checks

**Last fully gated commit: `3a62eed9`** (merge of D4 slice 5, 2026-10-28). Full `check.sh` 13/13
green. Native tests: 7,066 passed, 0 failed, 199 ignored. 30 GPU controls. 4.72 MiB brotli WASM.
The merge tree equals the tested tree `fbf5e972`. Current `main` (`8504a6b8`) adds only
marketing-site files on top, with no full gate. Both are on `origin/main`. The deploy runs for both failed (40728909082 for
`3a62eed9`, 40729696529 for `8504a6b8`). The last successful deploy was S4 (`43164e14`, run
40569424949), so the live web build is S4. The maintainer chose to leave the S5 deploy
pending.

**Newest gated commit (local, not pushed): `55ff1625`** (tree `8c27ded3`, protocol **96**), the
D2c-3 "ordered park" world-handover lane merged by fast-forward on 2026-11-01. Full `check.sh`
13/13 green: 7,086 native tests passed, 0 failed, 207 ignored. Clippy, WASM, forbidden-symbol
check and the 4.72 MiB bundle all passed. The Playwright smoke test was not run. Selected tests on
the gated binary: 74 of 76 pass, including all 3 mandatory carry subjects, both cadence controls
and all 25 new controls. The earlier failure (`handover_carry_fixture/native.rs:336` on
`a81199ee`) is fixed; the fixes were in test setup only. Note that the Keep image pins a protocol,
and a client now needs **96**.

**Remaining handover failures** (both fail identically on the old `main` `8504a6b8`, so neither
came from this lane):
- `test_integration::handover_game::game_harness_handover_recipient_consents_while_joined_and_keeps_its_session_after_leaving`:
  "the session didn't survive leaving the world".
- `test_integration::handover_game::personal_delay::game_harness_handover_delayed_initial_personal_client_blocks_prediction_and_orders_applied`:
  "assertion failed: hs.server.players[i].personal_delivery.pending.is_none()".

Both are `#[ignore]` GPU tests (they need an adapter holding the 506-layer atlas), so `check.sh`
does not run them; they were run on the gated binary. Low-severity review items: the aim loop
has no time limit; the "opens and hosts the activated world" test has a rare race; the Spec 04
"awaiting automated acceptance" header is stale.

**Outstanding manual checks** (none can be done by one machine; all owner):
- Fray step 5 acceptance 3 (two-machine LAN, five minutes, then by code with LAN discovery off,
  reporting join time and peak bytes/s) and acceptance 4 (invite-link regression). Pending since
  2026-10-23.
- Handover two-machine sheet `docs/test-sheets/2026-10-14-handover.md`, and the D2c-3 owner QA
  draft `.claude/epic/handoff/d2c3-ordered-park-owner-qa-draft.md`. Both unexecuted.
- D4 slice 3/4/5 owner sheets (`docs/test-sheets/2026-10-27-d4-slice-{3,4,5}.md`). Unticked.
- D2c-2 shared steed packs owner session (`docs/test-sheets/2026-10-14-shared-steed-packs.md`).
- K4 terrain-load phone acceptance with image `k4-11` (built, not staged). Exploration and
  terrain-load RSS are unmeasured.
- Roadmap "Now": two-machine LAN with a real remote signer, online play by contact across two
  houses, death drops / chunk push / riding walked by a second player, Signet contacts sync and
  QR sign-in on a phone, and data-dir migration plus updater on real hardware.

## 2. The authority in AxeNStax today

The authority is `GameServer`, run headless by `--keep-world` (`keep_world.rs`) behind
`HostedServer` (admission, per-slot queues, outbound diffing). Ingle drives it as a child process
over the K3 pipe (`keep_pipe.rs`). `HeadlessGame` is the client and is not in scope.

What already fits:
- `GameServer::tick` (`server.rs:1763-2309`) is synchronous. It reads no wall clock, does no file
  I/O and uses no unseeded RNG crate. A grep of every non-test, non-client module found none.
  The `sim_*` phases (clock, mobs, death, machines) already run as one ordered pass.
- The pipe's frames already have the contract's shape (`Joined`/`Message`/`Left` in,
  `Send`/`Disconnect` out), and keep-world sets no signing identity. No signing key is in the
  authority (`start_keep_world`, `hosted_server.rs:1479-1496`).
- Chunks already have a canonical byte form: 8,704 B each (4,096 × u16 LE + 64 × u64 LE placed
  mask, light excluded, `chunk.rs:305-317`). A server save writes that form for every non-empty
  chunk it holds, touched or not (§8).

## 3. Mapping onto the six operations

| Contract op | AxeNStax entry point | Fit | Gap |
|---|---|---|---|
| `init(config, seed)` | `keep_world::open_world` → `HostedServer::start_keep_world` → `GameServer::new` + `initial_load` (`keep_world.rs:351-380`, `server.rs:1157`, `:1325`) | medium | The seed is `u32`, drawn from the OS for a new world (`save.rs:3556`). Contract seed is a supplied `u64`. `init` reads and writes files (meta, lock, tick-0 save). `pop_secret`, `run_id` and `DayClock.owner` come from `getrandom`. Config is the pipe `Hello` + env + `WorldMeta`, with no content binding. |
| `on_event(Joined)` | the pipe creates a transport; `accept_new_remote_connections` (`hosted_server.rs:4419`) attaches a reused slot and issues a Signet challenge | low | **Admission runs inside the authority, after `Joined`.** That breaks contract:91 (see §4.1). `load_joiner` reads the per-npub sidecar from disk. The `action_session` is OS-random (`:5187`). |
| `on_event(Message)` | the reader thread queues bytes; `process_inbound_packets` drains them in `HostedServer::tick` (`:4630`) | medium | Messages arrive with no time attached, so the reader thread's timing picks the tick. Per-tick budgets carry messages over (10 packets, 4 edits; window events auto-apply after 200 ticks). The step an event is stamped with is not the step it takes effect. The carry-over queues are authority state. |
| `on_event(Left)` | pipe `Left` → synthetic `Disconnect`; `release_slot` next tick (`:2317`) | medium | Writes the joiner sidecar inside the event. A server-initiated leave is a transport `Drop`, not a returned `Command`. |
| `on_event(Control)` | `Hello`/`Shutdown`/`GateAnswer`/`Pass` (`keep_pipe.rs:752-869`) | medium | `GateAnswer` replies to an authority-originated question, which the contract has no primitive for. `Shutdown` saves (I/O). `Pass` (bearer secret) never touches `GameServer`. Good. |
| `tick(time)` | `HostedServer::tick` (`hosted_server.rs:4341`) driven by a 50 ms `Instant` loop that **skips** missed ticks (`keep_world.rs:288-314`) | medium-high (`GameServer`), low (`HostedServer`) | Takes no time argument. Three counters: `tick_counter` (u64), `world_time` (u32 day clock, starts 6000), `HostedServer.server_tick` (on the wire). Output is pushed through `send_to_client`, not returned as commands. |
| `checkpoint()` | Slice 2b: `GameServer::checkpoint` (Spec 01 §4.1.5), not wired in. Before it, `GameServer::try_save` (`server.rs:3127-3497`) | medium | The rest of this row is `try_save`, the record path it depends on. Writes files, does not return bytes. Refuses while mob deaths are pending (`save.rs:1077`). Non-canonical (§4.3). Omits most live state (§5). `handover/checkpoint` packs saved *files* for transfer and strips `pop_secret` and players. It is not a live-state checkpoint. |
| `restore(config, ckpt)` | Slice 2b: `GameServer::restore_from_checkpoint`, not wired in, over `initial_load` (`server.rs:1325-1450`) | medium | The rest of this row is `initial_load` alone; the restoration ledger in §7 says what slice 2b brings back. Rebuilds from the save rather than restoring. Clocks restart, wildlife re-scatters, entity ids are new, and every saved column is loaded (the live run had streamed some out). |
| `state_hash()` | Slice 2b: `GameServer::state_hash`, `SHA256(b"fray/authority-state/v1\0" \|\| checkpoint())`, not wired in | medium | Covers chunks, sidecars, the meta and all of `world.dat` through the manifest, and needs a persist first. Not comparable across architectures until §4.5 is audited. |

**Time.** 20 TPS, so `Time{tick: n, elapsed_us: n × 50_000}`, with `Time.tick == tick_counter`
after step n. `server_tick` must become the same restored counter. `world_time` stays derived
state but must be checkpointed: it is not `tick mod 24000`, and `/time` writes it.

**PlayerId.** Fray `peer: u32` is connection-local, and wire slots are reused lowest-first
(`hosted_server.rs:4489`). Neither can be the PlayerId. The only stable key is the verified Signet
persona (`ServerPlayer.verified_pubkey`), which already keys sidecars and pet/economy ownership.
The runner should assign `PlayerId(u64)` per persona and persist that map outside the authority.
Note that contract `Player.identity` is the Fray **runtime** key. The persona is a different key,
and today the game verifies it, not the shell (§6, choice 7).

## 4. Where the contract and AxeNStax disagree outright

1. **Admission order.** The contract authenticates before `Joined`, with security clocks in the
   host shell. AxeNStax verifies the signed `JoinRequest` inside the authority: OS-random nonces
   (`signet/challenge.rs:114`), `Instant` expiry, and a ±300 s `SystemTime` skew check
   (`hosted_server.rs:8073`, `:7701`). Only then does it ask Ingle's gate. K3 decision 3 made
   this ordering deliberate.
2. **No RNG object exists.** Randomness is hashes of `(tick, position, seed, hecs entity id)`, for
   example `mob_ai::simple_hash(ai.ticks, entity)` (`mob_ai.rs:740`, about 10 call sites). Entity
   ids depend on allocation and free-list order. "Restore the RNG state" therefore means restoring
   the tick counters, exact entity ids and generations, and the allocator free list. None of these
   is serialised.
3. **Random hash order: the sites named here are fixed in slice 1; others are open (Spec 01
   §4.1.4).** `ahash` 0.8 with default features (`runtime-rng`) gives every `AHashMap`/`AHashSet` a
   random iteration order per map, so two servers in one process already disagree. That changed
   outcomes at fire `.take(EVENT_BUDGET)` (`fire.rs:198`), furnace batch deferral
   (`sim_machines.rs:179`), hopper chain order (`hopper.rs:119`), and mob scatter and despawn
   order, which feed entity ids. All of these now sort. Campfire visits (`sim_machines.rs:258`) sort
   too, but on a closer reading that order changes only the block changes emitted, not simulated
   state. Pistons, dispensers, saplings, keg fuses, hideouts, construction, the power network and
   salt licks are still open. A server's `world.dat` is canonical since slice 3
   (`WorldSave::sort_unordered_rows`); a client's save isn't, and no checkpoint reads one.
4. **Secret in state.** `pop_secret` (the Proof-of-Play / Satori HMAC key, in
   `world_meta.json`) keys drops and every server use-roll. A restore without it changes
   outcomes, yet contract:106 forbids serialising private keys (§6, choice 6).
5. **Floats: `-0.0` is fixed in slice 1 for the digest's explicit encoders (Spec 01 §4.1.4); the
   rest is open.** f32 lives in every body, and mob AI has 16 `sin/cos/atan2/powf/exp/ln/mul_add`
   sites, with glam 0.33 SIMD paths that differ by target. Contract acceptance 5 (x86 vs ARM
   hash) should be expected to fail until these paths are audited. Bytes are IEEE bits.
   `sim_digest::f32_bits` folds `-0.0` onto `0.0`; nothing normalises NaN payloads, and floats the
   digest encodes through `Debug` text are not folded.

## 5. What complete restoration still needs

Legend: saved = in `world.dat`, chunk files or a joiner sidecar today. "Changes outcomes" compares
restore-after-step-n with the uninterrupted run.

| Area | Today | Changes outcomes? |
|---|---|---|
| Chunks, never saved | Regenerated from `seed` + `worldgen_version` | No, *if* generation is bit-exact on every target (unproven cross-arch). |
| Chunks, held at a save | Saved, one 8,704 B file each, **touched or not** (a server save writes every non-empty chunk it holds, 21–31× the touched set in the §8 bench); all loaded at boot | Partly: the **loaded/simulated set** differs after restore. |
| Block entities (all 14 kinds) | Saved | Contents yes; order non-canonical. Power is re-derived on load. |
| RNG | None (§4.2) | **Yes**: AI rolls, spawn rolls, weather/lightning, fire, use-rolls. |
| Wild mobs, night hostiles, golems, raiders | Lost; re-scattered per column | Yes. |
| Tamed pets, carts, quest givers | Saved; attack/target state deliberately dropped; ids change | Mostly kept; `riding: Option<Entity>` dangles. |
| Projectiles, item drops, bees in flight | Lost | Yes (items on the ground vanish). |
| Fire, water/lava fronts, leaf decay queues | Lost; fire re-adopted with fresh timers | Yes. |
| Rubber-tap cooldowns | Rebuilt at the restored tick (0 for a save without a clock) | Yes (every cooldown restarts, by design: no save-scumming). |
| Hoppers, growth | Stateless, keyed on tick | No, now the tick is saved (`4c5a75c6`). A save without a clock still resets it. |
| Pistons mid-push | No record found | Unknown. |
| Joiner inventory and position | Sidecar v5 per npub (≤64 KiB), written on leave and autosave | Kept as of last save. **Fixed** (`9872d926`): position, yaw and pitch are saved, and a joiner goes back to its position if a body can stand exactly there, else to its spawn. The client's camera doesn't get the look: `JoinAccept` has no field for it. A joiner who left beyond a lending host's load radius (about 128 blocks at the default render distance) lands at spawn, because the host can't check a column it hasn't loaded and keeps no anchor for a rejoin. |
| Believed-unit buckets, `PossessionTally`, drop bucket | Lost | Yes: bounds refill to full (a cheater gets a fresh allowance). |
| Action windows/frontier, window events, pending ops | Lost; `action_session` re-drawn | Yes: every connected client's action lane desyncs. |
| Owed correction debt | Saved (bounded) | Kept. |
| Quests: per-player session, incarnation, gameplay clock | Not saved, except via the bound personal record | Yes for quests in progress. |
| Tick counter | **Fixed** (`4c5a75c6`): saved in `WorldSave.authority_clock`, format 60. A save without the field still restarts at 0 | No. Auction deadlines keep their remaining ticks; bounties don't re-roll; use-rolls carry on. |
| Day clock, night calendar, weather, reputation day | **Fixed** (`4c5a75c6`, sleep gate `9872d926`): saved with the tick. The gate is now the saved `calendar_id`, not `run_id`, so a second sleep in one night is refused across a restart. Lightning has no state to keep: a strike is rolled and landed in one tick. | No, except that two pieces of the `clocks` digest section are not saved: the `/time speed` step (every world entry resets it to 1) and `falling_tick_counter` (restarts at 0). |
| Cooldowns, respawn and death timers, spawn protection | Lost | Yes. |
| Inputs: `last_applied_input`, intent queue, step credit | Lost | Yes: prediction re-snaps, queued input dropped. |
| Output state: chunk-push sent-sets, entity interest, ProtocolId baseline, outboxes | Lost | Yes for output: every client is fully re-pushed. Contract acceptance 2 (ordered commands agree) cannot pass. |
| Pending deaths | ECS markers; save refused while pending | No longer: each corpse is in the checkpoint (slice 3), and a save still waits for the death pass, so the runner's `last` waits too. |
| Economy owners | Saved, but five owner enums still carry `LocalPlayer(slot)` | Meaningless across a restore with reused slots; must converge on npub. |
| `HostedServer` per-slot vectors, carry-over queues | Lost | Yes (authority state per §3). |
| `HostedServer.server_tick` | **Fixed** (`4c5a75c6`): carries on from the restored tick at its first tick; audited safe with a large start (Spec 04 §4.2i), no wire change | No. |

**Statics that block two authorities in one process:** `CHUNK_SYNC` (`chunk_push.rs:183`),
`NO_LEND` (`hosted_server.rs:223`), `data_dir::ROOT` (`data_dir.rs:60`), the `save_fence` and
handover lock statics, and the load-failure lists in `save.rs`. Also, `World.disk_chunks` is
mutated by `&self` save, but `checkpoint` must be observational.

## 6. Storage and migration choices for joint design (not implementing until agreed)

Each choice carries AxeNStax's recommendation, for discussion only.

1. **What a tier-L "checkpoint" is.** The contract wants one complete image. A world does not
   fit: migration v0.1 caps the session image at 1 MiB, one edited column is 136 KiB raw, and
   eight sidecars can reach 512 KiB.
   *Recommend:* a hybrid. Durable records (chunks, sidecars, world meta) live in the vault. The
   checkpoint is the volatile authority state plus a sorted manifest of the digests of those
   records. `state_hash` then covers the world through the manifest. This needs a contract
   amendment for L: "required content" is extended to the game's stored records.
2. **Storage import shape** (Fray Q11/Q18): keyed record import vs a checkpoint stream.
   *Recommend:* keyed records in the game's namespace, written in **atomic batches**. Possibly no
   *read* import at all in v1: today every persisted chunk stays in memory once loaded (the
   evicted store is in RAM), so reads happen only at restore. If so, persistence can be a
   returned `Persist(batch)` command that the runner commits, and the authority stays
   import-free. *Estimate, not measured:* 4 GiB holds about 400k edited chunks at 8.7 KB each,
   before other state.
3. **Cadence and durability.** Today: a 60 s autosave on the same thread (it stalls ticks),
   crash loss up to 60 s, multi-file and fenced. Open: who triggers a persist (authority vs
   runner), what the batch-commit guarantee is, and whether pending deaths may block a
   persist.
4. **Encoding.** Reuse WorldSave (positional bincode 1, append-only, 56 fields, format 60) with a
   sort pass, or a new canonical checkpoint schema? A bincode 1→3 migration is already planned
   and changes the save and wire encoding.
   *Recommend:* a new versioned, bounded codec for volatile state (the `quest_npcs::codec`
   pattern). Chunks keep their existing canonical 8,704 B form. WorldSave stays the native save,
   with converters both ways.
5. **Pristine chunks: regenerate or store.** Regenerating needs bit-exact worldgen on x86, ARM
   and WASM. Storing costs about 40 MB raw per anchor at view distance 8 (*estimate*).
   *Recommend:* regenerate, gated by a cross-arch column-hash vector set (`column_hash` already
   exists).
6. **`pop_secret`.** Carry it in the checkpoint (visible to trusted backups only; any backup can
   then predict every Satori roll and use-roll), have the host shell supply it through secret
   config, or derive it per Keep. The contract's "no private key" wording needs a ruling on
   gameplay HMAC keys.
7. **Admission split.** Move Signet join verification (nonces, skew clock) into the host shell
   and pass the verified persona in `Player.metadata`/identity, or add a reply primitive for
   in-game verification plus the gate question.
   *Recommend:* move it to the shell. That is contract-conforming, but it revisits K3 decision 3
   and needs the same binding-exporter pass-through.
8. **Per-player data ownership.** Should joiner sidecars live in the world's namespace or in a
   per-player one (portability, Fray Q18)? Children's data stays on the operator's Keep either
   way (red line 3).
9. **Migration promise for L v1.** *Recommend:* "none live; resume from the last committed
   batch". AxeNStax's game-owned world handover stays the peer-hosted path. Don't build Fray
   standby for L (Fray Q19c).
10. **Version migration on a Keep.** When the module version bumps, who migrates the stored
    records: the module at restore (forward-only, refuse newer, as today) or the runner? This also
    covers how the bincode 3 migration lands on Keep-stored data.
11. **Simulated set after restore.** Today restore loads and simulates every saved column; the
    live run simulates only the columns around anchors. The checkpoint must name the loaded set,
    or restore must re-derive it from player positions, which a save holds only as of each
    joiner's last record (sidecar v5: its leave or an autosave).

### AxeNStax positions, 2026-11-01

> **Superseded where the rulings differ.** The Fray session's rulings of 2 November 2026 (and their
> addenda of 3 November) have landed in the contract: `forgesworn/fray-internal/plans/2026-11-02-tier-l-contract-rulings.md`.
> Where a position below disagrees, the ruling stands: choice 2 gains a bounded `get` import for restore, and
> choice 6 is rejected (`pop_secret` derives from the 32-byte `init` seed, never from the host shell). The
> positions are kept as they were written.

AxeNStax has taken the game-side half of each choice so that slices 1 and 2 can go ahead. The
Fray-side halves were **proposals** when this was written: the contract amendment in 1, the
`Persist(batch)` command shape in 2, and the ruling on gameplay HMAC keys in 6. They have since been ruled on.

1. **Hybrid checkpoint.** It is the only shape that fits the 1 MiB session-image cap. The
   checkpoint holds the volatile authority state plus a sorted digest manifest of the stored
   records. *Fray side:* amend "required content" for tier L to include the game's stored records.
2. **Keyed records in atomic batches** (ruled: with a bounded `get` import, valid only during `restore`, not "no read import"). The authority emits
   `Persist(batch)`, the runner commits it, and records are read only at restore. *Fray side:* the
   command's name and shape.
3. **The authority emits dirty batches at its own cadence (60 s today), and the runner commits
   each batch atomically or not at all.** A persist that falls on a tick with pending deaths
   waits for the next tick boundary, which is today's save rule. Crash loss is up to one interval.
4. **Encoding (the game's own call, contract:106).** Volatile state gets a new versioned,
   bounded codec in the `quest_npcs::codec` pattern, written to canonical bytes. Chunks keep
   their 8,704 B form. WorldSave stays the native save. The bincode 1→3 migration is
   independent.
5. **Regenerate pristine chunks**, gated by a cross-arch `column_hash` vector set. If the gate
   cannot be made to pass on ARM or WASM, the fallback is storing them.
6. **`pop_secret` comes from the host shell as secret config.** (Rejected by ruling 3: it is hidden game state in
   the volatile checkpoint, derived from the 32-byte `init` seed under its own domain label; the host shell passes
   no secret.) As proposed, it was excluded from the checkpoint
   and from `state_hash`, and the checkpoint binds only a key id. *Consequence:* if a world is
   restored on a different Keep without the secret, Satori rolls and use-rolls diverge. That is
   acceptable for L v1 because of choice 9. *Fray side:* rule that gameplay HMAC keys count as
   private keys for contract:106.
7. **Admission moves into the AxeNStax child's own outer shell** (`keep_pipe` plus Signet
   verification, nonces and the skew clock). The authority core it wraps sees only verified
   personas. This is internal to the binary: Ingle still does not depend on the AxeNStax crate
   (K3 decision 1), and the gate is still asked after verification (K3 decision 3). It does not
   revisit K3.
8. **Joiner sidecars stay in the world's namespace for v1, as today.** Portability across Keeps
   is a later product decision. Children's data stays on the operator's Keep either way.
9. **No live migration for L v1.** If a Keep dies, the world resumes from the last committed
   batch and loses up to one interval. The peer-hosted path keeps the game-owned handover. No
   Fray standby is built for L.
10. **The module migrates its own records at restore**, forward-only, and refuses records from a
    newer version, as it does today. The bincode 3 migration lands the same way.
11. **Joiner position, yaw and pitch are added to the saved state** (this also fixes "a restored
    joiner is back at spawn"). The checkpoint names the loaded and evicted columns, and restore puts
    them back rather than re-deriving them from the anchors, because a re-derived set would part
    `chunks` at the next stream-out. (Changed by slice 2b: this position said restore re-derives the
    simulated set from the anchors.)

### For the Fray session, from slice 2b

Measured or built on the game side. The sizes are from `checkpoint_size_at_the_bench_world_sizes`
(§8). Slice 3 has settled the ones below against the Tier-L rulings.

**Settled in the game, 2026-11-03 (checkpoint schema 5; no save-format or protocol change):**

- **"Not now": there is no such code.** `checkpoint` no longer refuses a pending death: each corpse
  is in the bytes (`authority_checkpoint/deaths.rs`) and comes back as a corpse, and the restored
  world's next death pass does what the uninterrupted one's does (credit, loot, cargo, hideout and
  raid counts). Sweeping inside the tick wouldn't do: the pass is bounded at 32 a tick and stops while
  the kill credit has nowhere to go. A save still waits for the pass, so the runner's `last` waits too.
  The refusals left for a server that owns its world are `TooLarge`, `Unrestorable` (below) and `Encode`, which is a
  capture that failed (a pack animal's plan body, or a layout section out of range) and not an over-cap image.
- **Restore order.** Schema field, seal, remaining versions, config binding, record digests, and only
  then the load. Any schema but this build's, older or newer, is `unsupported-version` whatever
  follows it, the cap included. The old check for schema 0 as a bad checkpoint is gone. Bad magic stays
  `invalid-checkpoint`: without it there is no schema field to read.
- **Digests over stored bytes.** `m` is its stored bytes with the secret's value cut, never a parsed
  struct encoded again, so a new `WorldMeta` field doesn't move it, and a space no parser would notice
  does. `c` is taken from memory so a checkpoint moves with an unsaved block, but `as_bytes` is exactly
  what a save writes, and the restore hashes the files. `j` already was. `b` is gone: see `world.dat`
  below.
- **Torn meta.** A restore refuses one as `invalid-checkpoint` before the load, and leaves the folder
  byte-identical. So does a lost meta with a damaged copy kept aside. A normal open keeps its recovery,
  because without it every later save is refused; the digest is taken after it, again whenever the
  server writes the file back and at each persist, so a run's digest can't go stale.
- **Restore-only bounds.** `checkpoint` puts its own bytes through the decoder and range checks a
  restore runs, so every limit is enforced at both ends by one piece of code: `Unrestorable`, no image.
  That covers `MAX_LAYOUT_SECTIONS`, the live-and-evicted section, the body bound and every other value
  check in `read_layout`, `Volatile::validate` and `players::read_all`. This is about the image's own bytes
  and the state records, not a promise about the root: an image is written, and refused at restore, when there
  is no persist yet, when it is of a later tick than the persist, when `world.dat` only part-decodes, or when the
  state changed since the persist (tests pin each). It is true of a root straight after a persist, which is
  where a runner's `last` step takes it; the slice 5 test runner must assert the restore at each commit. A tick holds bodies and spawns at
  ±1,000,000 as at a wall, and `World::insert_chunk` won't make a second copy of an evicted section.
  Tests: at-limit fixtures to the byte, and a seeded walk over the fixture's actions. The section cap's
  fixture goes through the same reader on bytes: a world at it holds 3.4 GiB of chunks.
- **The checkpoint is independent of the world's size for what grew with it.** The checkpoint names the
  root of a manifest tree (`authority_checkpoint/records.rs`, key `t` + prefix), where a key starting `t`
  binds a further manifest record the same way. At most 64 entries is a record; more are shared out by the
  byte after their common prefix. So it is a function of the entries, walking it gives the one sorted list,
  one changed entry rewrites one path, and no manifest record passes 64 KiB. The layout (`l`), the lava
  queue with its source differences (`q`), each cell's exposure clock (`p`), water likewise (`f`), the cell
  sims (`s`: fire, leaf decay, scheduler, power, rubber taps), the player slots (`u`) and the corpses (`k`) are
  stored records cut at 64 KiB, read by the same readers as before. Inline stays for fixtures only
  (`checkpoint_inline`, `#[cfg(test)]`). Numbers are in §8.
- **The image is bounded whatever the world holds.** What is left: the header, the clock, the modes, the
  streamer's settled anchors (one a player), lava's phase, leaf decay's clock, the manifest's root and the
  seal. So about 2.4 KB at the widest server. Two things moved that look bounded and aren't. A player slot: a
  plan in an inventory holds a captured building's cells, up to 32,768 of them, and a joiner's pack feedback
  can hold 1,024 rows. Fire: `MAX_ACTIVE_FIRE` (256) holds a run's ignitions, but a fire block in a column that
  goes out of memory stays and `register_column_fires` re-adopts it uncapped when the column returns, so a world
  can hold more. `TooLarge` can still be produced by state built directly (a test does, with anchors past any
  player count), but not by anything a world can hold. `the_image_is_one_size_whatever_the_world_holds` puts 120,000
  entries in each list, 36 full plans in a slot and 3,000 corpses, and the image is the size it was.
- **A record that can't be read is an error at `checkpoint`.** `note_meta`, `note_world_dat` and
  `note_handover` used to log a failed read or scan and bind nothing, so `checkpoint` could succeed with an
  image the restore refuses. The failure is now kept: `checkpoint` is `Unrestorable` and the authority's
  persist is `Err` until a later note reads the file. A normal save still only logs it, and a file that is
  legally absent isn't a failure. A sidecar `scan_joiners` can't read is kept the same way, as it would
  otherwise be left out of an image whose restore, reading the file after it became readable, reports it as
  not in the manifest.
- **`MAX_LAYOUT_SECTIONS` is 262,144.** The ruling lifts 65,536 once the layout is a record, and something
  must still bound what `apply_chunk_layout` builds for a section with no record (12.8 KB each). 262,144 is
  the runner's key ceiling, and about 3.4 GiB at worst, inside the placeholder 4 GiB. Same constant at both
  ends, through the one reader.
- **The record set is built from memory by one pure function, `GameServer::build_record_set`** (ruling 1,
  slice 3, `authority_checkpoint/record_set.rs`). It returns the records this server holds the bytes of (`w`
  pieces through `file_piece_batch`, `c` from `Chunk::as_bytes`, a present joiner's `j` from its capture, the
  state records, the `t` tree) as Fray's `StoredRecord`s, the entries it has only a digest of (`held`: the
  sidecars of joiners not present, `m`, `h`, whose bytes are the meta writer's and the joiners' own saves',
  not this save's), the tree, and `removed`: the keys the previous bound set had and this one hasn't (a
  shrunk `w`, a dropped chunk, a sidecar no longer bound). The type is its own `RecordSet`, not `PersistBatch`:
  a set has digest-only entries and a tree root, and no `last` or size split, which are the batcher's.
  `StoredRecords` keeps the previous set's keys (and nothing else of it) for `removed`; the dirty map of
  digests that supersedes that is slice 5's. A save and the set share the encoders: `encode_world_dat` (the
  `WorldSave` build, which touches no file) and `joiner_rows_in_memory`; `try_save_selected` calls the first at
  the point it built the same bytes before, so a save writes what it always did.
- **What writes the records: `GameServer::persist_authority`.** It is `try_save` and then the records, in
  `authority/` in the world's folder, one file per key, written from the builder's set (`persist_authority_set`
  returns it) rather than from a second pass over the notes. Nothing but the tests calls it yet: `--keep-world`
  still calls `try_save`. Fray's answer is that `--keep-world` stays unchanged through slice 3 and no native L
  runner comes before the WASM build: only a test runner calls the new paths. A normal
  server's save is untouched, and a world never run as an authority has no such folder.
- **`checkpoint` still does no I/O, and still moves with an unsaved block.** The tree and the state records
  are functions of what the server holds, so it computes them from memory; what a persist wrote is the same
  bytes if nothing has changed. If something has, its digests aren't the store's, and a restore refuses.
- **A handed-over world's records are bound.** Every file in `handover/` and `participants/` but a lock or a
  temporary file is an `h` key, over its stored bytes; one not named is a mismatch, as `open_guard` scans the
  folder. Mutation cases from a real restore: `a_handed_over_worlds_records_are_bound`.
- **The write on that path is gone.** `load_or_bootstrap` turned a participant's frozen seed sidecar into its
  current record and wrote it, during a restore, through `reload_sidecars`. A restore now makes it in memory
  (`load_or_bootstrap_unwritten`), and the first ordinary read after the seat comes back writes it. A normal
  open is unchanged. The fsyncs of `confirm_authority_durable` and `confirm_durable` stay: they change no
  bytes.
- **The seal is a checksum, not a signature.** Ruled so: any authentication is the runner's, and nothing
  tells corruption from anything else.
- **The `world.dat` gap is closed.** The manifest binds `world.dat` over its stored bytes, one key `w` + `u32`
  BE and digest per 64 KiB piece (below), so all 56 fields of `WorldSave` are bound at any world size, and a
  field added to the save is bound by being in the file. The `b` record, its digest of memory and its post-load special
  standing are gone. A mutation test enumerates the fields from the type (a new field doesn't compile
  until it has a row) and runs each through a real restore: another valid value, a flipped byte of its own,
  its rows gone. `c`, `j`, `m` and `w` each also take a flipped byte, absence, another key's valid bytes and
  an older value of their own. All `invalid-checkpoint`.
- **Canonical bytes.** A server's save now writes the rows it takes from hash maps sorted by key, and the
  ones from entity queries sorted by their bytes (`WorldSave::sort_unordered_rows`, the sort pass of §6
  choice 4). That is the one change to what a normal save writes: the order of rows on a dedicated server's
  or role cut's save. The format and every reader are the same, and a client's save is untouched.
- **What moves the hash.** A table's digest is the persist's, so an unsaved table change doesn't move
  `state_hash` until the persist that stores it, where `b` used to. A checkpoint of a later tick than its
  persist is refused by the clock `world.dat` holds, since its digests can all match while its tables are stale.
- **`pop_exposure` is a bound record,** at 20 B a cell, its values unchanged. It is in no save, and only the
  authority's persist writes it, so a normal save is unchanged.
- **A restore writes nothing.** It opens through `world_open::open_bound_world`: `world.dat` only (never the
  autosave), no joiner stage settled, no staged first save published, a torn chunk file no record binds
  skipped where it lies. A bound `world.dat` this build only part-decodes is `invalid-checkpoint`, as is
  one that doesn't read. What is left in the folder is settled by the restored world's next persist. A normal
  open keeps every recovery it had. Still done on a restore that gets past the digests: the fsyncs of
  `confirm_authority_durable` on a handed-over world, which change no bytes.
- **Two value checks.** `read_layout` refuses evicted sections in a column not flagged evicted, and a corpse
  must be at a cell in range. One check each in the shared reader, so `checkpoint` has them too.

**Fray's answers, 3 November 2026** (addenda to rulings 2 and 3, from the game's fit check):

- *First commit:* a world's first commit lands when its initial spread ends; `init` cannot persist. The test
  runner keeps the same batch limit, so the check exercises real batching.
- *Handed-over worlds:* cannot be an L world in v1. The `h` family, `open_guard` and `proof` in restore, and
  `participants/<npub>` keys drop out for Keep.
- *Keys and personas:* a keyed digest of a persona still counts as an identifier. Use the game's own counter,
  never reused, plus a directory record bound by the tree, with the entry removed on erasure.
- *Native path:* `--keep-world` stays unchanged through slice 3; no native L runner before the WASM build.
- *Lava queue:* making its pieces not shift, so it fits a `last` batch, is the game's to fix.
- *A state file in fixed pieces* (ruling 2's addendum): yes, under the file's own keys, with no contract change;
  met below.

**Still open on the game side, not needing a Fray answer:**

- **A corpse's living copy is matched, not identified.** A pet's row in `world.dat` was taken when it
  lived, and pets have no stable id, so a restore removes the nearest living mob of the corpse's kind and
  owner. With two alike, it may be the other one. A quest NPC's row isn't matched at all.
- **A piece is by position.** A queue's order is its state, so a state record is cut by offset: an entry added
  near the start of the sorted layout or the clocks rewrites the pieces after it, up to the whole record in a
  batch. For lava it is always the whole record while the queue drains, as taking the front shifts every byte
  after it: 703,043 B a persist at the 8-joiner bench. At the largest measured, all of `authority/` is 962,259 B (§8)
  against the 1 MiB batch, which staged batches can spread. Fine at a 60 s cadence; a much shorter one makes it
  the dirty-rate limit ruling 1 says to measure. Water and the player slots add to that: slots change at every
  persist with anyone moving, about 440 B a slot, and water only as it flows.
- **A corpse lies in a record, and a save still waits for the sweep.** So a real `last` batch holds an empty
  corpses record, and only a fixture writes a full one (`write_bound_records` without the save). If the runner
  ever persists with deaths pending, the save's wait has to go first.

**For the Fray session, from closing the `world.dat` gap:**

- **`world.dat` against the record-value limit: answered, and built.** The Tier-L rulings' addendum at the end of
  section 2 says yes to fixed pieces under the file's own keys, bound by the record tree, with no contract
  change, and that one changed row rewriting later pieces is acceptable while the file is this small. Per-table
  keys can replace it if a measured commit shows the rewrite matters. Its conditions, and where each is met:
  - *Piece size and count from the game's schema, not the batch limit.* `records::FILE_PIECE_BYTES`, 64 KiB,
    pinned by a test, with its reason beside it; the count is the file's length divided by it. An empty file is
    one empty piece, so that it isn't the same as no file. A file of a whole number of pieces has no empty one
    after them.
  - *A shrink removes the pieces past the new end, so the store equals the bound set.* Nothing writes pieces yet:
    the file stays whole on disk, with no second copy, exactly as a save writes it. `records::file_piece_batch`
    is the pure function that gives the piece keys and slices for the new bytes and the keys to remove given the
    old count. Unit-tested for growth, no change, a shrink by exactly one piece, by more, to empty, and a file
    that has gone. The record set builder (below) calls it for `w`; nothing yet writes a `w` piece as a record.
  - *Restore verifies every piece before assembling.* `manifest::verify_stored` cuts what is on disk and compares
    each piece, before `refuse_a_bound_world_dat_that_isnt_the_state` or the load read the file. Each of these is
    `invalid-checkpoint` from a real restore, naming the piece, in `every_way_world_dat_can_differ_from_its_pieces_is_refused`:
    a flipped piece (first, middle, last), a missing one (short by exactly one piece, by two, by a byte, cut inside
    a piece, emptied, absent), two swapped, a stale piece, and longer than the bound set (by a byte, by a piece).
  - *Keys stay free of persona identifiers.* `w` + `u32`. The contents are unaffected. **Keys that carry one today,
    not touched here (the persona-free keys slice):** `j` + a sidecar's name (`npub1….dat`), `h` + `participants/<npub>.dat`,
    and a `t` key: a manifest-tree record is named `t` + the common prefix of its entries (`records.rs`
    `build_node`), so a world with one joiner and over 64 entries holds `t` + `jnpub1….dat`, and `authority/`
    holds a file named by that prefix in hex. The counter keys fix it with the others.
- **`personal-manifest.dat`: no L world from a handover in v1, addendum at the end of ruling 3.** So it isn't cut
  into pieces, and its `h` binding, like the rest of `handover/` and `participants/`, stays as it was until a
  restore moves off the folder. The extra mutation cases for `prepared-`, `retired-`, `ready-` and `retained-*`
  aren't built for the same reason; `activated-`, the manifest, a participant and an added record have theirs.
- **`pop_exposure` isn't bounded, only out of the image.** Nothing prunes it, and an entry past its decay still
  holds its clock (a re-exposed cell must not start again). It grows at about 60 to 80 B a block mined in pure
  deepslate (§8), as stored bytes against the world's quota rather than against the cap.

**Answered without game code:** what a checkpoint binds (`calendar_id` is in, and two fresh worlds needn't
agree); persist then checkpoint (one root per `last`).

## 7. Smallest next adapter slice (AxeNStax-only, no wire or save change)

**Slice 1: twin-run determinism. Done.** Spec 01 §4.1.4 says what it checks and its limits: one
process, one fixture, dedicated-mode `GameServer::tick` only, event paths not covered. It was the
smallest slice with regression value, and it is useful even without Fray: replays, handover and the
D4 shared-sim all gain from it.
- A game-owned `sim_digest()` on `GameServer`, compiled for tests only: SHA-256 over a **sorted**,
  explicitly encoded view of chunks, light, block entities, world tables, entities (keyed by a
  stable id, f32 as normalised bits), clocks and per-player server state. It is documented as
  **not** the contract `state_hash` (no checkpoint exists yet), and its module docs list what it
  leaves out.
- A test-only seeded constructor on `TestHost` (a new world draws its seed from the OS
  today, `save.rs:3556`). This is test code, not a save change. The twins share the process
  statics in §5 (`CHUNK_SYNC`, `data_dir::ROOT`), which is fine because they don't need to differ.
- A twin-run test in `src/test_integration/`. Two `TestHost` instances get the same seed and the
  same scripted events at the same logical ticks, in a fixture with fire, furnaces, hoppers,
  campfires, water and mobs. It asserts equal digests at ticks 1, 200, 1,200 and 2,000, and
  different digests for a different seed. It failed first, as expected, on the hash-order sites in
  §4.3, before the fixes.
- Fixes at the outcome-affecting order sites (sorting by position or key at fire, furnace, hopper,
  mob scatter and stream-out despawn, plus campfire visits for their emitted order). Global `ahash`
  keys stay as they are: fixed keys would open a hash-collision denial-of-service route from
  joiner-chosen positions, and that is a security decision.
- A lint test that forbids `Instant`, `SystemTime` and `getrandom` in a listed set of tick-path
  files, with an explicit allowlist.
- A measurement: an `#[ignore]` release-profile bench of `GameServer::tick` p50/p99 ms with 0, 2 and
  8 synthetic joiners, sim distance 8, and machines and mobs present, plus the bytes of
  touched-chunk files and sidecars after a scripted build (§8).

**Slice 2: built, native only.** 2a persists the authority clock and a joiner's position (§5). 2b is
`authority_checkpoint` (Spec 01 §4.1.5): `GameServer::checkpoint`, `restore_from_checkpoint` and
`state_hash`, canonical, bounded at 1 MiB, with no pipe, Ingle or wire change, no save-format change
and not called by `--keep-world`. At ticks 1, 200 and 2,000 of the slice 1 fixture, and across a
drained water source: persist, checkpoint, restore into a new server, feed both the same script for
200 ticks. `chunks`, `block_entities`, `clocks`, `players` and `fire_fluid_queues` stay equal.

**The restoration ledger (`NOT_YET_RESTORED`).** What a restored world doesn't yet match:
- `light`: recomputed by the load's column pass, which needn't match a run's increments. Growth reads
  light, so a sapling or crop could part `chunks` later; the fixture has none.
- `wild_mobs`, `tamed_entities`, `item_drops`, `projectiles`: **hecs ids change on restore**, and mob
  AI rolls hash the id. No ids are remapped. Wild mobs re-scatter, tamed ones and carts come from the
  save, and what lay on the ground or was in flight is gone.
- `world_tables`: equal in the fixture, so printed and not promised, as hideout and raid counts follow the
  mobs. The tables read back from `world.dat`, which the manifest binds, and `pop_exposure` is a bound
  record.
- **Pending outputs are dropped:** `pending_item_grants`, `pending_kill_events`, `released_joiners` and
  `pending_sapling_drops` aren't carried. An item grant is value and not just output, so the slice 3
  runner checkpoints only after they drain.

Limits of what does hold:
- `players` depends on entities after all, which the slice 2b decision had assumed it didn't: a push or
  a bite moves a body. **Equality of `players` holds only while no joiner meets a mob during the
  comparison, because restored mobs aren't the originals.** The fixture's joiners don't in those 200
  ticks, and that is the fixture's geometry and not a property of restore.
  `a_mob_on_a_joiner_parts_the_players_section` pins this. From an empty ECS on both sides, every
  section but `light` stays equal. A rider restores on its feet, since `riding` names a hecs id.
- A restored joiner has a body and no connection state. Its action, pack and quest lanes, window events
  and block views are per attach, so it must re-attach before it acts (with slice 3). Its bound
  personal record reloads from the sidecar. Its correction debt stays in the sidecar, which is still
  the truth: the slot isn't saved until it re-attaches (`ServerPlayer::awaiting_reattach`, which the
  autosave, a shutdown save and a leave all respect), and nothing reads it as owing nothing until then.
  Slice 3's re-attach has to load that debt.
- The meta digest is re-noted at each persist and whenever the server writes the file back. `--keep-world`
  must refresh it after any later meta write outside those paths (`StoredRecords::note_meta`).
- Not yet covered as a checkpoint: the §5 rows for `HostedServer` vectors and queues, cooldowns held
  on connection state and output state.

**Slice 3:** return output as commands, and the admission split (choice 7).
**Then:** contract acceptance 2 and the WASM build.

**For the next slices (from the independent review of slice 3a; no code in this branch):**
- Slice 5: the test runner asserts the restore at each commit, as the root is only restorable straight
  after a persist.
- Build the batch as the last act of `tick`, and test a `world.dat`-only table change (a chest, a bell) just
  before it: it moves neither the clock nor `w`, so a checkpoint after the persist at the same tick restores the
  persist's table under the same hash. Past about 1 MiB, the dirty pieces of a `last` step may not fit one batch,
  so `last` must then be withheld, since a row-length change near the start rewrites every later piece.
- Slice 6: read each record once. Today `w` is read 3 to 4 times and `c` twice, against `get`'s 4 GiB budget a
  restore.
- Only `w` has a shrink batch; the state families, `c`, `j` and `t` rely on `commit`'s folder sweep
  (`records.rs`), which a `PersistBatch` won't have.
- `checkpoint` is not quite observational: `deaths::encode` reaches `plan_store::remember_checked`, which writes a
  Plan body for a dead pack animal carrying a full body. On a server these are markers and return early, so no
  real path was found, but the code can't rule it out. A failed capture surfaces as `Encode`.
- `has_bound_records` reads the unbound stage and can refuse a good root over a stale imported stage; it applies
  to handed-over and imported worlds, which Fray rules out of Keep.
- A flip inside the secret's value that breaks the JSON passes the digest and comes back as `Fault`, not
  `InvalidCheckpoint`; ruling 3's plain `m` closes it.
- `write_bound_records` has no gate, so a persist of a state `checkpoint` refuses overwrites the previous
  records, and `persist_authority` isn't atomic. The stand-in store only; the real root is Ingle's.
- `pending_item_grants` and the rest stay on the ledger, and nothing withholds `last` while they are non-empty.
  An item grant is value, so the adapter must enforce it.
- `save_joiner` on a leave changes a bound record outside a persist; `note_joiner` keeps the digest true, but the
  adapter must carry it in a batch.
- The production refusal of an inline manifest (`restore.rs`, under `#[cfg(test)]`) has no test of its own.
- A pending death at every persist step means no commit until the interval escalation, because the death
  pass defers while a delivery queue is full (`sim_death.rs`). For the adapter's wiring.
- Section cap: a real restore past 65,536 sections has passed once, by `a_real_restore_past_65536_empty_sections`
  (`#[ignore]`, run by name with `-- --ignored`): a fixture's own layout with 65,537 more empty live sections,
  bound by the real tree, then a real restore and a re-encode to the same bytes. It took 5 s and a peak of about
  1.6 GB resident on the shared machine, with nothing written to disk beyond the fixture; a real world of that
  many non-empty sections holds about 840 MB of chunks, and has not been restored.

## 8. Tier-L measurements: measured facts vs estimates

Placeholder L budget (`keep-and-ingle.md:82-91`, explicitly unbenchmarked): 20–30 Hz, about
16 ms tick, ≤4 GiB, ≤32 players, persistent world, migration best-effort or none. **No tier-L
benchmark exists.**

**Measured**
- K3-3 world child RSS: 114–124 MB with 2 players at a short view distance, flat over 5 min
  (laptop, 2026-10-25).
- K4-0 engine RSS: 114–117 MiB fresh; 133–136 MiB after reloading a 2,092-chunk world; 182 MiB
  at 2,645 chunks; under 1 MiB per player at view distances 2/5/10 (the test client never walks,
  so exploration is unmeasured); about 17 MiB shared code; **25 MB on disk** for 2,092 chunks.
- Wake of a sleeping world: about 1 s (laptop).
- Chunk-verdict cost in release on an i5-1235U: 1.53 ms fresh, 1.94 ms heavy, per column
  (2026-10-07). This is not total tick time.
- WASM bundle: 4.72 MiB brotli (27.45 MiB raw) at S4 and S5.
- Tier S only, not L: K5-0 synthetic component tick p50/p99 of 21–26 / 45–64 µs.
- Slice 1 bench (`test_integration/tick_bench.rs`), commit `97f9fb29`, release profile, one run,
  i5-1235U laptop, 2026-11-01. `GameServer::tick` only, so no transport, admission or per-client
  diff. Sim distance 8, machine fixture, 65–107 entities, synthetic joiners walking on a ring.
  Three ticks in four skip the fluid and machine pass, so p50 is that cheap path. The fire field
  has burnt out by the end of the 600-tick warm-up, so machines are mostly idle scans.

  | Joiners | Columns | p50 ms | p99 ms | max ms | mean ms |
  |---|---|---|---|---|---|
  | 0 | 289 | 0.05 | 3.59 | 13.56 | 0.95 |
  | 2 | 443 | 0.07 | 17.77 | 22.58 | 2.36 |
  | 8 | 530 | 0.12 | 31.10 | 37.68 | 5.25 |

  **p99 at 2 and 8 joiners (17.77 and 31.10 ms) already exceeds the placeholder budget of about
  16 ms**, and that is before `HostedServer` adds its share. The twin fixture has changed since this
  commit (more hopper items, less campfire fuel), and the bench has not been rerun on it.

- Same run, bytes after a scripted build and a save. Sidecars are about 440 B each with 20 stacks.

  | Joiners | Touched chunks | Touched bytes | `chunks/` files | `chunks/` bytes | Sidecar bytes |
  |---|---|---|---|---|---|
  | 0 | 65 | 565,760 | 1,369 | 11,915,776 | 0 |
  | 2 | 69 | 600,576 | 2,067 | 17,991,168 | 880 |
  | 8 | 79 | 687,616 | 2,480 | 21,585,920 | 3,520 |

  **A server save writes every non-empty chunk it holds, touched or not** (`try_save`, via
  `persistable_chunks`), so `chunks/` is 21–31× the touched set here.

- Slice 2b, `checkpoint_size_at_the_bench_world_sizes`, same scripted build, at the bench's three
  world sizes, with lava waking only the cells that can act, 2026-11-02. `checkpoint()` succeeds at all
  three.

  | Joiners | Columns | Non-empty chunks | Manifest B | Lava queue B | Water B | Layout B | Players B | Total B | Of the cap |
  |---|---|---|---|---|---|---|---|---|---|
  | 0 | 289 | 1,377 | 67,514 | 272,340 | 117,530 | 6,124 | 353 | 464,194 | 44% |
  | 2 | 409 | 1,918 | 94,231 | 502,323 | 117,530 | 8,419 | 1,203 | 724,055 | 69% |
  | 8 | 511 | 2,406 | 118,767 | 703,043 | 117,530 | 10,323 | 3,753 | 953,813 | 91% |

  These are taken about 30 s after the columns loaded, so the queue is near its largest. At about 53
  entries a second it empties in about 7 minutes at 0 joiners and 17 at 8, if nothing else loads. Before
  the filter the totals were about 2.85, 3.99 and 4.99 MB, all `TooLarge`.

  The slice 1 twin fixture, at a load radius of 3 and a sim distance of 2, was at 1,015,610 B at tick
  2,000, 97% of the cap. It is now 77,335 B at tick 1, 113,253 B at 200 and 237,079 B at 2,000, which is
  23%, so it has about 811 KB of margin. (All of the above is before the move below.)

- Slice 3, the same test at the same sizes, debug build, one run, 2026-11-03, with the manifest as a record
  tree and the layout, lava queue and exposure clocks in bound records. Each restores, and encodes back to
  the same bytes.

  | Joiners | Columns | Non-empty chunks | Checkpoint B | Of the cap | Manifest records | Manifest B | Longest B | `authority/` records | `authority/` B | Layout B | Lava B | Water in the image B | Players B | `world.dat` B | Restore ms |
  |---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
  | 0 | 289 | 1,377 | 118,261 | 11% | 120 | 73,662 | 837 | 127 | 352,130 | 6,124 | 272,340 | 117,530 | 353 | 112,555 | 1,460 |
  | 2 | 409 | 1,918 | 119,127 | 11% | 159 | 102,479 | 970 | 169 | 613,225 | 8,419 | 502,323 | 117,530 | 1,203 | 113,339 | 2,142 |
  | 8 | 511 | 2,406 | 121,725 | 11% | 159 | 127,134 | 1,131 | 172 | 840,504 | 10,323 | 703,043 | 117,530 | 3,753 | 114,445 | 3,062 |

  The tree costs about 7% more bytes than the same list inline (127,134 B against 119,300 B), and the image
  holds 41 B of it. **Worst lava inside one drain,** on the 8-joiner world with 540,000 cells queued (ten times
  the real load): 121,725 B, the same as with the real queue, and it restores. The lava record is then about
  7 MB in pieces of 64 KiB.

- Slice 3, `world.dat` in pieces and all that grows with the world out of the image, the same test, debug build,
  one run, 2026-11-03. Each restores, and encodes back to the same bytes. Schema 5.

  | Joiners | Columns | Checkpoint B | Of the cap | Manifest records | Manifest B | Longest B | `authority/` records | `authority/` B | Water B | Cell sims B | Players B | Corpses B | Lava B | Layout B | `world.dat` B (pieces) | Restore ms |
  |---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
  | 0 | 289 | 342 | 0.03% | 124 | 74,098 | 837 | 136 | 470,485 | 117,530 | 32 | 353 | 4 | 272,340 | 6,124 | 112,555 (2) | 2,862 |
  | 2 | 409 | 358 | 0.03% | 163 | 102,915 | 970 | 178 | 732,430 | 117,530 | 32 | 1,203 | 4 | 502,323 | 8,419 | 113,339 (2) | 3,370 |
  | 8 | 511 | 406 | 0.04% | 163 | 127,570 | 1,131 | 181 | 962,259 | 117,530 | 32 | 3,753 | 4 | 703,043 | 10,323 | 114,445 (2) | 4,642 |

  The image's 16 B between 0 and 2 joiners, and 48 B more at 8, is the settled anchors. **Its worst, by arithmetic:**
  about 2.4 KB, at 257 slots' anchors. **Worst lava on the 8-joiner world,** 540,000 queued cells:
  406 B, and it restores. **Worst water, on a fixture with eight joiners,** 400,000 sources and as many queued
  fronts, 10 MB in its record: it restores, and was `TooLarge { bytes: 10010418 }` before the move. **Everything
  at once,** 120,000 entries in each list, 36 slots of full plans and 3,000 corpses: the image is the size it was.

  **All of `authority/` at the 8-joiner bench is now 962,259 B, close to the 1 MiB batch,** with lava's 703,043 B
  of it rewritten at every persist while the queue drains. Staged batches can spread it, as the ruling allows; the
  bytes are the same state the image held before, so this is where the dirty-rate limit will show first. `world.dat`'s
  pieces add a file's worth at most to a batch that wants them, and aren't in that figure.

- **`world.dat` against the record-value limit, which nobody had measured.** At the bench sizes it is 112,555,
  113,339 and 114,445 B, about **11% of 1 MiB**. It barely moves between them, so it follows what the fixture
  built rather than how many columns are loaded. To see how far off the limit is, each table was given 1,000 rows on a
  smaller fixture's save (base 6,651 B), encoded through the real `encode_world_save`, and the bytes per row
  divided into 1 MiB. The row limit is taken as 1 MiB exactly, so these are slightly generous. Nothing was
  written to disk beyond that fixture's save.

  Real rows, as a played world holds them:

  | Table | Row | Bytes a row | Rows to the limit |
  |---|---|---|---|
  | chests | a full wooden chest, 27 stacks of one block | 240 | 4,341 |
  | signs | 120 characters of ASCII | 140 | 7,442 |

  The rest are each table's **smallest** possible row (what zeros decode to, so no contents, no text, no
  lists), which makes these ceilings, not estimates: a real row is larger and a real count lower.

  | Bytes a row | Rows to the limit | Tables |
  |---|---|---|
  | 109 | 9,558 | construction_anchors |
  | 78 | 13,358 | players |
  | 66 to 70 | 14,884 to 15,786 | face_blueprints, latent_prints, auctions, active_raids |
  | 44 to 52 | 20,037 to 23,680 | drying_racks, brigand_hideouts, exhibits, campfires, vendors, saved_mobs |
  | 36 to 41 | 25,412 to 28,942 | carts, plots, tip_jars, rigs |
  | 28 to 31 | 33,610 to 37,211 | waypoints, power_devices, architect_plaques, furnaces, market_hubs |
  | 20 to 25 | 41,677 to 52,096 | dispensers, chests (empty), graves, hostile_acts, composters, village_anchors, raid_kills, bounties, signs (empty) |
  | 12 to 16 | 65,120 to 86,827 | village_treasuries, face_overlays, hives, item_frames, block_meta, face_blueprint_blanks, village_bells |
  | 8 | 130,240 | populated_villages |
  | 4 | 260,481 | inventory, locked_slots, rig_clips |

  `workshop` holds plans and isn't a list of rows in the save, so it isn't here and wasn't measured; a plan's
  size follows what it records, and it may well be the first to reach the limit. `face_overlays` and
  `block_meta` count one row per painted face or per block with meta, so a builder reaches those by the
  thousand. So on chests alone: about 4,000 full chests in one world, fewer with the other tables' rows beside
  them. The bench worlds are nowhere near that; a long-lived shared world could plausibly get there. That was the
  case for fixed pieces, now built: no piece reaches the limit whatever the tables hold (§6).

- **`pop_exposure`'s growth.** The bench's scripted build mines no pure deepslate: 0 cells at all three sizes.
  With the real `mark_exposed_neighbours` in a block of pure deepslate, a two-high tunnel gave 4.00 new cells a
  block (360 blocks, 1,441 cells) and a one-layer floor 3.02 (22,500 blocks, 67,858 cells), so 60 to 80 B a
  block mined. The old image had room for about 52,000 cells, or 13,000 to 17,000 blocks. It has no such
  ceiling now, and nothing prunes it.

**Stated limits and decisions**
- 20 TPS (50 ms).
- Autosave 60 s; crash loss up to 60 s.
- Pipe frame 1 MiB; the pipe stops (saves, exits 5) at 32 MiB queued.
- Outbound queue: Fray 1 MiB default, AxeNStax sets 8 MiB. Inbound 8 MiB; joiner inbound on Fray
  64 MiB.
- Per-client state budget 48 KiB per tick.
- Chunk-push window 64 packets / 512 KiB.
- Keep container: was 256 Mi / 1 CPU; the owner chose 1 Gi.
- AxeNStax profile v1: idle 192 MiB, max 384 MiB, 8 MiB per player, storage 256 MiB.

**Estimates (not measured)**
- Up to 960 KiB/s per client from the 48 KiB budget at 20 TPS.
- About 40 MB raw blocks resident per anchor at sim distance 8.
- WASM 1.5–2× native cost (Fray planning figure).

**Not recorded anywhere:** total server tick ms through `HostedServer` under load, save/autosave
duration, chunk-push burst size and bytes/s on a real link, and RSS with more than 2 players or
after exploration. Slice 1's bench covers `GameServer::tick` only and does not read RSS.

