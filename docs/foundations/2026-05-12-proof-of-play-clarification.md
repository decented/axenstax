# Proof of Play — design clarification + spec realignment

**Status**: Design clarification — drives spec edits to Spec 6 §2, Spec 8 §5, CLAUDE.md, marketing-and-branding. No engine code change required (Wave 13 ore blocks are *compatible* with this design; the missing piece is the chunk-stream obfuscation for buried ore, which is its own foundations brief if/when prioritised).
**Date**: 2026-05-12
**Trigger**: Cross-spec audit while answering "have you added diamonds?" surfaced a framing drift — the spec language ("Hash-on-Mine", "Bitcoin rewards on stone/deepslate") reads like the player is a commercial Bitcoin miner. Staxolottle clarified the actual design and the term branded for it.
**Author bias**: Staxolottle (design call) + Claude (write-up).

---

## TL;DR

The pickaxe-strike SHA-256 hash is **three things layered on one computation**, not one thing called "hash-on-mine":

1. **Proof of Play (always-on, educational primitive).** Every strike — grass, dirt, stone, anything — runs `HMAC-SHA256(server_secret, world_seed || epoch_id || x || y || z)`. The hash is for **the player to see and learn from**, the in-game expression of the proof-of-work concept. Branded **"Proof of Play."** No commercial-miner framing.
2. **Material-drop layer (deterministic, reuses the hash).** The same hash bytes decide drops. Visible ore blocks (Wave 13: coal, iron, diamond) are guaranteed drops when mined with the correct tool — Minecraft-style. Plain stone gets *probabilistic* hash-driven rare drops (a "lucky strike" tier) using bytes the proof-of-play layer doesn't consume.
3. **Bitcoin layer (optional, server-operator concern).** A Bitcoin-enabled server may additionally translate proof-of-play effort into sats payouts. **The player is not a Bitcoin miner.** They're playing on a server whose operator wired proof-of-play to a reward pool. "Could mine on-chain" applies to the *server operator pooling effort*, not the player.

Anti-X-ray is achieved by the combination of **(a) visible exposed ore in cave walls** (Minecraft-style; the X-ray cheat reveals nothing new because the ore is already visible) and **(b) chunk-stream obfuscation for buried ore** (unexposed ore is replaced with plain stone in the data sent to the client; the cheat can't render what it didn't receive). Spec 8 §5.2.2 already describes the mechanism — this brief promotes it from "optional PvP-focused" to "core anti-X-ray, on by default."

---

## Why this clarification matters

- **Framing leaks into marketing.** "You earn Bitcoin by mining" reads as "this is a Bitcoin mining product." That's the *wrong* product. The actual product is a voxel game that uses a proof-of-work *concept* as an educational primitive, with optional Bitcoin payouts on top.
- **Regulatory posture.** Per CLAUDE.md Key Architecture Decisions: "Not a money transmitter — platform never touches funds — critical policy constraint." The player-as-miner framing pushes against that posture; the player-as-player-on-a-server-that-happens-to-pay-out framing is cleaner.
- **Spec internal contradiction.** Spec 6 §2.1 says "no 'ore' block type in the world data" (hash-on-mine is by-construction anti-X-ray); Wave 13 ships visible coal/iron/diamond ore blocks. Either the spec or the code is wrong. This brief says **both are right**, because the spec was conflating two layers — anti-X-ray for the *Bitcoin reward* and anti-X-ray for *ore drops*. The first stays architectural-by-construction; the second is handled by chunk-stream obfuscation.
- **Cross-game lift.** Proof of Play is a primitive that lifts beyond AxeNStax (per shared-infra strategy memory). If another game on the same primitives ever wants an educational proof-of-work surface, it reuses this design — the name "Proof of Play" carries that intent better than "hash-on-mine."

---

## The three layers in detail

### Layer 1 — Proof of Play (always-on)

Every pickaxe strike, on any block, runs:

```
proof_hash = HMAC-SHA256(
    key     = server_secret,
    message = world_seed || epoch_id || x || y || z
)
```

The hash is **visible to the player** — surfaced in the HUD (truncated, e.g. first 8 hex chars), counted in a session "strikes" counter, and used to drive a "Genesis Block" celebration animation when the hash falls below a configured difficulty threshold (the same threshold the original hash-on-mine spec used, decoupled from any actual reward).

**Educational intent.** The player learns by playing:

- Hashing is a deterministic function of inputs (same block position + same secrets = same hash; break the same coordinate twice and the hash matches).
- Most hashes are "boring numbers"; rare ones meet a difficulty target. That's proof-of-work in microcosm.
- Difficulty is a tunable — server operators can lower it for a "mining rush" event (already in marketing-and-branding) or raise it.

**Not for Bitcoin by itself.** A server running pure proof-of-play (Bitcoin layer disabled, default) still shows the hashes, still celebrates Genesis Blocks, still teaches the concept. No sats involved.

### Layer 2 — Material drops (deterministic, hash-reusing)

The pickaxe strike resolves a drop. Drop logic depends on the block type:

| Block type | Drop rule | Notes |
|---|---|---|
| **Visible ore block** (e.g. `DIAMOND_ORE`, `IRON_ORE`, `COAL_ORE`) | Guaranteed drop of the ore's material when mined with the correct tool. Wrong tool → block breaks, drops nothing. | Minecraft-style. The visible block *is* the contract. |
| **Plain stone / deepslate** | Probabilistic rare drop using `proof_hash` bytes. E.g.: low byte against a rare-drop threshold → "lucky strike," drops a diamond / iron nugget / coal. | X-ray-proof: nothing visible distinguishes a "lucky" stone block from a normal one — the hash decides. |
| **Other blocks** (grass, dirt, leaves, etc.) | Block-type-specific (existing engine logic). | Proof-of-play hash still runs; doesn't drive drops here. |

**Hash byte budget** (rough; finalise during implementation):

```
proof_hash = 32 bytes
  bytes [0..8]  → reward_value u64 (for Genesis Block celebration + Bitcoin threshold)
  byte  [8]     → tier (dust/nugget/chunk/vein) when applicable
  byte  [9]     → exact reward magnitude within tier
  byte  [10]    → rare-drop probability check (plain stone only)
  byte  [11]    → rare-drop material selector (coal/iron/diamond/...)
  bytes [12..]  → reserved
```

Same hash, different byte slices for different decisions. Cost: one HMAC per strike (already paid).

### Layer 3 — Bitcoin layer (optional, opt-in)

A Bitcoin-enabled server (server config `bitcoin.enabled = true`, see Spec 6 §1.3) additionally:

- Pools proof-of-play effort against a sats reward pool.
- Pays out via LNbits when work-credits cross a threshold (**work-meter model — the only sanctioned real-sats payout**: deterministic, zero per-strike unpredictability). The earlier "probabilistic payout on Genesis-Block landing" secondary model is **retired for real sats** — a chance-based real-Bitcoin payout sits inside the gambling perimeter (UK Gambling Act s.6; free-to-play is not a defence). The Genesis-Block moment remains an educational, no-sats celebration. See Spec 6 §2.3 + `docs/research/2026-06-21-uk-online-safety-gambling-crypto-landscape.md`.
- Subject to all Spec 6 §3 sustainability + rate-limit constraints.

**The player is not a commercial miner.** They're a player whose proof-of-play effort the server has chosen to translate into a small sats payout. The server operator (not the player) is the entity making "do effort, pay out value" decisions. The server operator may also choose to do something on-chain with pooled effort (run a Stratum proxy to a mining pool, accept upstream sats as treasury inflow, etc.) — that's a server-operator concern, **not** a player-facing feature.

**Default posture.** The Bitcoin layer is **off by default** in self-hosted single-binary mode. Servers must explicitly opt in and configure LNbits credentials. Per ADR-002: "Hybrid Bitcoin model — game works without Bitcoin, Bitcoin-enabled servers are flagship." Proof of Play works identically with or without the Bitcoin layer.

---

## Anti-X-ray, restated cleanly

The original Spec 8 §5.1 claim — "anti-X-ray is a consequence of the reward mechanic design" — was over-broad. It assumed no ore blocks would ever exist in world data. Wave 13 shipped visible ore blocks. The claim needs splitting:

### Bitcoin reward layer → still architectural-by-construction

The hash that drives Bitcoin payouts is computed from `server_secret + position`. The client never receives `server_secret`. No client-side data distinguishes "this stone block pays out" from "this one doesn't." X-ray clients have nothing to highlight for Bitcoin purposes. **Architectural defence — unchanged.**

> **Fixed 2026-09-27 (audit).** Until then this defence was not real: the alpha stub derived `server_secret` from the world seed with a fixed public key, and the seed goes to every joiner in `JoinAccept`, so any client could rebuild the secret and map every Satori vein. `server_secret` is now 32 random OS-RNG bytes per world, stored only in the host's `WorldMeta.pop_secret` and never sent over the wire. Old worlds get a fresh one on first load (their future rare-drop placement moves — accepted). Every export/share path strips it and every import generates a fresh one (2026-09-28). See Spec 06 §2.2.

### Rare-drop layer on plain stone → architectural-by-construction (same reason)

The hash that drives rare-drops on plain stone is the same hash. Same defence. **Architectural — unchanged.**

### Ore block layer → chunk-stream obfuscation

Ore blocks exist as distinct block types in the world data. Without mitigation, an X-ray client renders stone as transparent and trivially highlights ore. Mitigation:

- **Exposed ore** (at least one face touching air — visible in cave walls, ravines, exposed cliffs) is sent to the client as the real block type. The X-ray cheat reveals nothing new because the player can already see it. This is the canonical Minecraft caving loop and is intentional.
- **Buried ore** (no air-facing face) is replaced with plain stone in the chunk data sent to the client. The cheat can't render what it didn't receive. When the player breaks adjacent stone and the ore becomes exposed, the server sends a chunk update revealing the real block.

This is the mechanism Spec 8 §5.2.2 already describes. The spec frames it as "optional, cave-focused, PvP-focused, disabled by default" — that framing was about *PvP route-hiding*. The same code path solves ore-X-ray and should be **core anti-X-ray, on by default**, for any server that ships visible ore blocks (which is all of them).

**Cost.** One pass per chunk at chunk-send time, cached until the chunk mutates. Spec 8 §5.2.2 already notes the cost is acceptable. Latency on cave breakthrough is a one-frame chunk update — Minecraft does this; we can too.

---

## What Wave 13 actually shipped vs this design

`grep -n "COAL_ORE\|IRON_ORE\|DIAMOND_ORE" game/engine/src/block.rs`:

- Block IDs 16/17/18 with distinct textures (TEX_COAL_ORE, TEX_IRON_ORE, TEX_DIAMOND_ORE — "cyan flecks" etc.).
- `mine_drop()` returns the correct material when mined.

That's **layer 2 (visible ore guaranteed drop)** in this design — and it's correct. What's missing from the engine right now:

1. **Layer 1 (proof-of-play hash visualisation).** The hash isn't computed yet; it isn't surfaced in the HUD. This was the *intent* of "hash-on-mine" but the engine hasn't built the hash path yet.
2. **Layer 2 rare-drop path on plain stone.** Not implemented. Plain stone currently drops cobblestone (or nothing depending on tool); no probabilistic rare-drop hash check.
3. **Layer 3 (Bitcoin).** Per Spec 6 §1.3, feature-flagged, not in alpha scope.
4. **Anti-X-ray chunk obfuscation.** Spec 8 §5.2.2 is spec-only, no engine implementation.

None of these are alpha-launch blockers per alpha launch posture (the platform is shipping AxeNStax-the-game as a PWA, Bitcoin layer is post-alpha). But they're the building order whenever Phase 5 Bitcoin work begins.

---

## Doc updates flowing from this brief

| File | Edit |
|---|---|
| `docs/spec/06-bitcoin-integration.md` §2 | Rename "Hash-on-Mine Mechanic" → "Proof of Play (Hash Per Strike)". Restructure into the three layers. Add §2.3a "Material drop layer (hash reuse)". Soften §2.1 over-broad anti-X-ray claim; point at Spec 8 §5 for the full anti-X-ray story. Add cross-ref to this brief. |
| `docs/spec/06-bitcoin-integration.md` §3 | Weaken "you mine, you earn sats" language. "Player digs block → server validates → work_credits +=" stays; reframe surrounding prose: server-operator decision to translate effort into sats, not player-as-miner. |
| `docs/spec/08-security-anti-cheat.md` §5 | §5.1: update — Bitcoin-reward + plain-stone rare-drop X-ray defence is still architectural; ore-block X-ray needs the chunk obfuscation. §5.2.2: promote from "Optional, Cave-Focused" to "Core anti-X-ray + cave route hiding, on by default." Update Appendix B traceability matrix accordingly. |
| `CLAUDE.md` line 154 | Replace `Hash-on-mine — every pickaxe strike runs SHA256 (even on grass); Bitcoin rewards on stone/deepslate break via difficulty target comparison` with Proof-of-Play layered framing. Point at this brief. |
| `docs/vision/marketing-and-branding.md` | Already mentions "Proof of play". Add a single-line cross-ref to this brief so marketing copy and engineering spec point at the same definition. |

---

## Acceptance

- [ ] This brief written and committed.
- [ ] Spec 6 §2 reframed per the table above.
- [ ] Spec 8 §5 updated per the table above (incl. Appendix B traceability matrix).
- [ ] CLAUDE.md line 154 updated.
- [ ] marketing-and-branding cross-link added.
- [ ] `./check.sh` not required (docs-only change).
- [ ] No engine code change in this pass. (Wave 13 ore blocks are *compatible*; engine implementation of the three layers is post-alpha.)

---

## Memory-rule check

- **Per shared infra strategy:** "Proof of Play" is a primitive other Forgesworn games can lift. Brand it once, document it once, reuse the name. ✓
- **Per signet boundary:** This is engine-side, no Signet wire impact. ✓
- **Per alpha launch posture:** Layers 1+2 rare-drop + layer 3 + chunk obfuscation are post-alpha; brief intentionally doesn't propose alpha-blocking work. ✓

---

## Cross-refs

- `docs/spec/06-bitcoin-integration.md` — pre-existing reward mechanic spec (gets reframed).
- `docs/spec/08-security-anti-cheat.md` §5 — pre-existing anti-X-ray spec (gets promoted).
- `docs/vision/marketing-and-branding.md` — pre-existing "Proof of play" mention.
- `docs/architecture/ADR-002-tech-stack.md` — "Bitcoin Model: Hybrid — game works without Bitcoin, Bitcoin-enabled servers are flagship."
- `CLAUDE.md` "Key Architecture Decisions" — "Not a money transmitter — platform never touches funds — critical policy constraint."
- Wave 13 commit `dbdeb8d` — visible ore blocks.
- Wave 6 commit `7aeafee` / Wave 7 `697b4cd` — iron + diamond tools, tool progression mining gates.
- Wave 17 commit `ab5b258` — diamond storage block (9↔1 round-trip).
- `docs/foundations/2026-06-03-work-based-hashing.md` "Anti-farming" — player-placed blocks earn no work/hash when re-mined (BUILT 2026-06-08), closing the chop-restand-rebreak / place→break farming loop. Per-voxel "placed" bit; Spec 6 §2.2 + Spec 2 §4.3.
