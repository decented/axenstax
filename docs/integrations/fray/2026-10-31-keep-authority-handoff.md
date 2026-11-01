# AxeNStax → Fray/Ingle: authority integration handoff (2026-10-31)

Game-owned. Written for the Fray/Ingle design session. AxeNStax owns every change and build on its
side; nothing here is implemented yet. Section 6 lists the storage and migration choices that need
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
- Touched chunks already have a canonical byte form: 8,704 B each (4,096 × u16 LE + 64 × u64 LE
  placed mask, light excluded, `chunk.rs:299-311`).

## 3. Mapping onto the six operations

| Contract op | AxeNStax entry point | Fit | Gap |
|---|---|---|---|
| `init(config, seed)` | `keep_world::open_world` → `HostedServer::start_keep_world` → `GameServer::new` + `initial_load` (`keep_world.rs:351-380`, `server.rs:1157`, `:1325`) | medium | The seed is `u32`, drawn from the OS for a new world (`save.rs:3556`). Contract seed is a supplied `u64`. `init` reads and writes files (meta, lock, tick-0 save). `pop_secret`, `run_id` and `DayClock.owner` come from `getrandom`. Config is the pipe `Hello` + env + `WorldMeta`, with no content binding. |
| `on_event(Joined)` | the pipe creates a transport; `accept_new_remote_connections` (`hosted_server.rs:4419`) attaches a reused slot and issues a Signet challenge | low | **Admission runs inside the authority, after `Joined`.** That breaks contract:91 (see §4.1). `load_joiner` reads the per-npub sidecar from disk. The `action_session` is OS-random (`:5187`). |
| `on_event(Message)` | the reader thread queues bytes; `process_inbound_packets` drains them in `HostedServer::tick` (`:4630`) | medium | Messages arrive with no time attached, so the reader thread's timing picks the tick. Per-tick budgets carry messages over (10 packets, 4 edits; window events auto-apply after 200 ticks). The step an event is stamped with is not the step it takes effect. The carry-over queues are authority state. |
| `on_event(Left)` | pipe `Left` → synthetic `Disconnect`; `release_slot` next tick (`:2317`) | medium | Writes the joiner sidecar inside the event. A server-initiated leave is a transport `Drop`, not a returned `Command`. |
| `on_event(Control)` | `Hello`/`Shutdown`/`GateAnswer`/`Pass` (`keep_pipe.rs:752-869`) | medium | `GateAnswer` replies to an authority-originated question, which the contract has no primitive for. `Shutdown` saves (I/O). `Pass` (bearer secret) never touches `GameServer`. Good. |
| `tick(time)` | `HostedServer::tick` (`hosted_server.rs:4341`) driven by a 50 ms `Instant` loop that **skips** missed ticks (`keep_world.rs:288-314`) | medium-high (`GameServer`), low (`HostedServer`) | Takes no time argument. Three counters: `tick_counter` (u64), `world_time` (u32 day clock, starts 6000), `HostedServer.server_tick` (on the wire). Output is pushed through `send_to_client`, not returned as commands. |
| `checkpoint()` | `GameServer::try_save` (`server.rs:3127-3497`) | low | Writes files, does not return bytes. Refuses while mob deaths are pending (`save.rs:1077`). Non-canonical (§4.3). Omits most live state (§5). `handover/checkpoint` packs saved *files* for transfer and strips `pop_secret` and players. It is not a live-state checkpoint. |
| `restore(config, ckpt)` | `initial_load` (`server.rs:1325-1450`) | low | Rebuilds from the save rather than restoring. Clocks restart, wildlife re-scatters, entity ids are new, and every saved column is loaded (the live run had streamed some out). |
| `state_hash()` | none | — | Needs a canonical checkpoint first. The nearest is `chunk_verdict::column_hash` per column. |

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
3. **Process-random hashing.** `ahash` 0.8 with default features (`runtime-rng`) gives every
   `AHashMap`/`AHashSet` a per-process random iteration order. That **changes outcomes**: fire
   `.take(EVENT_BUDGET)` (`fire.rs:178`), furnace batch deferral (`sim_machines.rs:176`), campfire
   order (`:250`), hopper chain order (`hopper.rs:94`), and mob scatter and despawn order, which
   feed entity ids. It also makes `world.dat` bytes non-canonical: every `Vec<Saved*>` is built by
   iterating a hash map, and nothing sorts it.
4. **Secret in state.** `pop_secret` (the Proof-of-Play / Satori HMAC key, in
   `world_meta.json`) keys drops and every server use-roll. A restore without it changes
   outcomes, yet contract:106 forbids serialising private keys (§6, choice 6).
5. **Floats.** f32 lives in every body, and mob AI has 16 `sin/cos/atan2/powf/exp/ln/mul_add`
   sites, with glam 0.33 SIMD paths that differ by target. Contract acceptance 5 (x86 vs ARM
   hash) should be expected to fail until these paths are audited. Bytes are IEEE bits, but
   nothing normalises `-0.0` or NaN payloads.

## 5. What complete restoration still needs

Legend: saved = in `world.dat`, chunk files or a joiner sidecar today. "Changes outcomes" compares
restore-after-step-n with the uninterrupted run.

| Area | Today | Changes outcomes? |
|---|---|---|
| Chunks, pristine | Regenerated from `seed` + `worldgen_version` | No, *if* generation is bit-exact on every target (unproven cross-arch). |
| Chunks, touched | Saved, one 8,704 B file each; all loaded at boot | Partly: the **loaded/simulated set** differs after restore. |
| Block entities (all 14 kinds) | Saved | Contents yes; order non-canonical. Power is re-derived on load. |
| RNG | None (§4.2) | **Yes**: AI rolls, spawn rolls, weather/lightning, fire, use-rolls. |
| Wild mobs, night hostiles, golems, raiders | Lost; re-scattered per column | Yes. |
| Tamed pets, carts, quest givers | Saved; attack/target state deliberately dropped; ids change | Mostly kept; `riding: Option<Entity>` dangles. |
| Projectiles, item drops, bees in flight | Lost | Yes (items on the ground vanish). |
| Fire, water/lava fronts, leaf decay queues | Lost; fire re-adopted with fresh timers | Yes. |
| Rubber-tap cooldowns | Rebuilt at `load_tick = 0` | Yes (every cooldown restarts). |
| Hoppers, growth | Stateless, keyed on tick | Yes (tick counter resets). |
| Pistons mid-push | No record found | Unknown. |
| Joiner inventory | Sidecar v4 per npub (≤64 KiB), written on leave and autosave | Kept as of last save. **Position, yaw and pitch not saved**: a restored joiner is back at spawn. |
| Believed-unit buckets, `PossessionTally`, drop bucket | Lost | Yes: bounds refill to full (a cheater gets a fresh allowance). |
| Action windows/frontier, window events, pending ops | Lost; `action_session` re-drawn | Yes: every connected client's action lane desyncs. |
| Owed correction debt | Saved (bounded) | Kept. |
| Quests: per-player session, incarnation, gameplay clock | Not saved, except via the bound personal record | Yes for quests in progress. |
| Tick counter | **Not saved; restarts at 0** | **Yes, a bug today:** saved auction `deadline_tick` stretches by the run time; bounties re-roll; use-rolls repeat. |
| Day clock, night calendar, weather, lightning, reputation day | Not saved | Yes. Also: a joiner can sleep twice in one night across a restart (`run_id` gate). |
| Cooldowns, respawn and death timers, spawn protection | Lost | Yes. |
| Inputs: `last_applied_input`, intent queue, step credit | Lost | Yes: prediction re-snaps, queued input dropped. |
| Output state: chunk-push sent-sets, entity interest, ProtocolId baseline, outboxes | Lost | Yes for output: every client is fully re-pushed. Contract acceptance 2 (ordered commands agree) cannot pass. |
| Pending deaths | ECS markers; save refused while pending | A checkpoint at an arbitrary tick boundary may be unavailable. |
| Economy owners | Saved, but five owner enums still carry `LocalPlayer(slot)` | Meaningless across a restore with reused slots; must converge on npub. |
| `HostedServer` per-slot vectors, carry-over queues, `server_tick` | Lost | Yes (authority state per §3). |

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
4. **Encoding.** Reuse WorldSave (positional bincode 1, append-only, 55 fields, format 59) with a
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
    or restore must re-derive it from player positions, which are not saved today.

## 7. Smallest next adapter slice (AxeNStax-only, no wire or save change)

**Slice 1: twin-run determinism.** This is the smallest slice with regression value, and it is
useful even without Fray: replays, handover and the D4 shared-sim all gain from it.
- Add a game-owned `sim_digest()` on `GameServer`: SHA-256 over a **sorted**, explicitly encoded
  view of chunks, block entities, entities (keyed by a stable id, f32 as normalised bits), clocks
  and per-player server state. It is documented as **not** the contract `state_hash` (no
  checkpoint exists yet).
- Add a test-only seeded constructor on `TestHost` (a new world draws its seed from the OS
  today, `save.rs:3556`). This is test code, not a save change. The twins share the process
  statics in §5 (`CHUNK_SYNC`, `data_dir::ROOT`), which is fine because they don't need to differ.
- Add a twin-run test in `src/test_integration/`. Two `TestHost` instances get the same seed and
  the same scripted events at the same logical ticks, in a fixture with fire, furnaces, hoppers,
  campfires, water and mobs. Assert equal digests at ticks 1, 200 and 2,000, and different
  digests for a different seed. Expected to **fail first** on the hash-order sites in §4.3: each
  `RandomState` gets its own keys, so two instances in one process already disagree. Confirm
  that red before fixing.
- Fix only the outcome-affecting order sites (sort by position or key at fire, furnace, campfire,
  hopper, mob scatter and stream-out despawn). Global `ahash` keys stay as they are: fixed keys
  would open a hash-collision denial-of-service route from joiner-chosen positions, and that is a
  security decision.
- Add a lint test that forbids `Instant`, `SystemTime` and `getrandom` in `server.rs` tick paths
  and `sim_*`, with an explicit allowlist.
- Add a measurement, in the same slice: an `#[ignore]` release-profile bench of `GameServer::tick`
  p50/p99 ms with 0, 2 and 8 synthetic joiners, sim distance 8, and machines and mobs present.
  Also record the bytes of touched-chunk files and sidecars after a scripted build. This turns
  the tier-L tick row into a measured fact.

**Slice 2 (after choices 1-4 are agreed):** persist the clocks (fixing the auction and bounty
bug), checkpoint the volatile state per the agreed codec, and a checkpoint→restore→continue test
at several tick boundaries comparing `sim_digest`.
**Slice 3:** return output as commands, and the admission split (choice 7).
**Then:** contract acceptance 2 and the WASM build.

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

**Not recorded anywhere:** total server tick ms under load, save/autosave duration, chunk-push
burst size and bytes/s on a real link, and RSS with more than 2 players or after exploration.
Slice 1's bench covers the first and the last.

