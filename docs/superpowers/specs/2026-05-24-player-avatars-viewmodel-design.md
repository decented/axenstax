# Player Avatars, Animation & Held-Tool Rendering — Design

**Date:** 2026-05-24
**Status:** Approved (design) — ready for implementation plan
**Author:** Claude (brainstormed with Staxolottle)

This is **Spec 1 of 2**. Spec 2 — *Networked 3-Tier Spectator System* (Spec 04 §8) —
builds on the avatar rendering delivered here and gets its own design + plan once
this lands. Decomposition and the maximal-fidelity choices (3D arm+tool viewmodel,
full animation set on the wire) were agreed with the owner on 2026-05-24.

---

## 1. Problem & Goal

Today a player cannot see the tool in their own hand, and other players in
multiplayer appear as featureless stone **ghost boxes** (`build_player_ghost_vertices`,
`game_loop.rs`). Held tools aren't even representable on the wire
(`PlayerState.held_item: u16` is "block id or 0").

**Goal:** Players see a proper animated humanoid avatar for every other player —
with the correct held item in hand and a correct look direction — and see their
own equipped item as a 3D first-person arm+tool that swings on use.

### Success criteria

- Remote players render as an animated humanoid avatar (head/body/arms/legs),
  not a stone box, in every viewport (incl. native split-screen).
- The avatar's head tracks the remote player's pitch; body tracks yaw.
- The avatar plays walk/idle, jump, crouch, and a swing animation on mine/place.
- The avatar holds the correct item (block **or tool** or material) in its right hand.
- The local player sees a 3D arm + the equipped item, lower-right, that swings on
  mine/place and bobs gently while walking.
- Held-item rendering (hand attachment + first-person) is driven by **one** shared
  item→model mapping, so remote hand and first-person view always agree.
- `./check.sh` is green (clippy, build, `cargo test`, trunk WASM, bundle-size gate).

### Non-goals (explicitly deferred)

- **Networked 3-tier spectator** — Spec 2.
- **Custom / uploadable skins** — one default skin, per-index tint for now.
- **Armour rendering on the avatar** — leave a documented hook; no geometry.
- **WASM split-screen** — stays native-only. On WASM the local player still gets a
  first-person viewmodel, and remote players (single-player has none) are unaffected;
  the avatar/animation code compiles and runs cross-platform.

---

## 2. Architecture

Extend the **existing mob model/animation infrastructure** (`entity_model.rs`)
rather than building a parallel player-rendering subsystem. This was the agreed
architecture: it reuses `ModelPart`, `build_part_vertices`, the texture-array
pipeline, and the entity render pass; aligns with Spec 03's `player.json` bone
intent; and avoids a duplicate skinning/animation path.

Five cooperating units, each independently testable:

| Unit | File(s) | Responsibility |
|------|---------|----------------|
| **Player model** | `entity_model.rs` (+ `texture_gen.rs`) | Humanoid `ModelPart` set + `TEX_PLAYER_*` layers |
| **Animation** | `entity_model.rs` | Head-pitch, swing trigger, jump/crouch poses on top of existing walk cycle |
| **Wire state** | `protocol.rs` (+ `server.rs`, `hosted_server.rs`) | Extended `PlayerState`: anim state, flags, tool-capable held item; version bump |
| **Avatar render** | `game_loop.rs` | Build avatar vertices per remote `PlayerState`; replace ghost boxes; attach held item; name tag |
| **Viewmodel** | `renderer.rs` (+ `game_loop.rs`) | Camera-attached first-person arm + held item, swing + bob, per local player |
| **Item→model map** | new `held_item_model.rs` | Shared `ItemRef → renderable mesh` used by avatar hand AND viewmodel |

---

## 3. Component detail

### 3.1 Player model (`entity_model.rs`, `texture_gen.rs`)

- New `player_model() -> Vec<ModelPart>`: `head`, `body`, `arm_l`, `arm_r`,
  `leg_l`, `leg_r` in Steve-ish block proportions (head 0.5³, body 0.5×0.75×0.25,
  arms/legs 0.25×0.75×0.25). Arms/legs `animated: true` with alternating `phase`
  (0.0 / 0.5) exactly like the zombie. Head gets a new `pitch_tracks_look: bool`.
- `ModelPart` gains `pitch_tracks_look: bool` (default `false` for all existing
  mob parts — additive, no behaviour change for mobs).
- New procedural skin layers in `texture_gen.rs`: `TEX_PLAYER_HEAD_FRONT/SIDE/TOP`,
  `TEX_PLAYER_BODY`, `TEX_PLAYER_ARM`, `TEX_PLAYER_LEG` (mirror the villager
  generator). Bump `ENTITY_TEXTURE_COUNT` / layer accounting accordingly.
- Per-player tint: avatar vertices multiply skin colour by a hue derived from
  `player_index` (reuse the Brigand/`MobDef.color` tint approach) so players are
  distinguishable until real skins exist.

### 3.2 Animation (`entity_model.rs`)

A pure function derives per-part rotations from a small **`PlayerAnimInput`**
struct `{ anim_state, swinging, crouching, on_ground, walk_phase_seconds }`:

- **Walk/idle** — reuse existing sine swing (`freq 2.5`, `±0.4 rad`), gated on
  `anim_state == Walk`.
- **Head pitch** — head part rotates by `pitch` (from `PlayerState`) around X.
- **Swing** — when `swinging`, the right arm plays a decaying forward rotation:
  `angle = SWING_PEAK * (1 - t) * sin(π t)` over a fixed `SWING_TICKS` window,
  `t ∈ [0,1]`. Client tracks per-player swing progress from the wire flag's rising
  edge. Pure, unit-tested decay curve.
- **Jump** — `!on_ground`: legs tuck (small symmetric rotation), arms raise slightly.
- **Crouch** — `crouching`: body+head dip (lower Y offset + slight forward tilt);
  avatar visibly shorter.

`build_part_vertices` is extended to accept an optional **head-pitch** angle and a
**per-part override rotation** (for the swing arm), additive to the existing yaw +
walk-swing path. Existing mob calls pass `None` / `0.0` — unchanged.

### 3.3 Wire state (`protocol.rs`, `server.rs`, `hosted_server.rs`)

Extend `PlayerState`:

```rust
pub struct PlayerState {
    pub player_index: u32,
    pub x: f32, pub y: f32, pub z: f32,
    pub yaw: f32, pub pitch: f32,
    pub health: f32,
    // Held item — tool-capable (replaces the block-only `held_item: u16`).
    pub held_kind: u8,   // 0 empty, 1 block, 2 tool, 3 material
    pub held_id: u16,    // block id / tool tier / material id within `held_kind`
    // Animation
    pub anim_state: u8,  // 0 idle, 1 walk, 2 jump, 3 crouch
    pub flags: u8,       // bit0 swinging, bit1 crouching, bit2 on_ground
}
```

- Bump `PROTOCOL_VERSION` (wire-breaking; both ends rebuild together — no
  cross-version negotiation needed at alpha).
- `ServerPlayer` (server.rs) tracks the new fields; `HostedServer` populates them
  each tick from the authoritative player sim + the most recent input
  (`hotbar_slot` → resolve `ItemRef` via inventory; `swinging`/`crouch`/`on_ground`
  from input + physics). The existing `ServerPlayer`↔`PlayerSlot` BRIDGE is noted,
  not resolved here.
- `InputPacket` already carries `hotbar_slot`; add a 1-bit **swing** intent
  (rising edge on mine/place) and a **crouch** bit if not already present, so the
  server can author `flags`.

### 3.4 Avatar render (`game_loop.rs`)

- Delete the `build_player_ghost_vertices` stone-box path; replace with
  `build_player_avatar_vertices(&PlayerState, anim_input, time)` that emits
  `player_model()` parts via the animation in §3.2.
- Resolve `ItemRef` from `held_kind`/`held_id`; if non-empty, append the held-item
  mesh from §3.6 transformed to the right-hand anchor (a fixed offset on `arm_r`,
  following the arm's swing rotation).
- Name tag: a camera-facing textured quad above the head showing the player's
  handle (npub fallback) — per the npub-only display memory, render handle/npub,
  never hex. Billboarded; small; optional kill-switch.

### 3.5 First-person viewmodel (`renderer.rs`, `game_loop.rs`)

- A dedicated **viewmodel pass** drawn after the world for each local player's
  viewport, with the depth buffer cleared first (classic FPS approach) so the arm
  never clips into geometry, and rendered with a narrower FOV to avoid distortion.
- Geometry: `arm_r` cuboid (player skin) + the held-item mesh (§3.6), positioned in
  **view space** lower-right via a fixed transform.
- Motion: walk-bob (sine on the local player's horizontal speed) + the same swing
  curve (§3.2) triggered on the local mine/place action. Empty hand (`held_kind == 0`)
  → render the bare arm/fist (no item mesh); the arm still swings on use.
- One viewmodel per local split-screen player, in their own scissor/viewport.
  Hidden when a future spectator/free-cam is active (hook only; no spectator now).

### 3.6 Item→model mapping (`held_item_model.rs`, new)

`fn held_item_mesh(item: ItemRef, registry: &BlockRegistry) -> Vec<Vertex>` (local,
untransformed; caller positions it):

- **Block** → small textured cube using the block's six face textures.
- **Tool** → thin 3D slab textured with `TEX_ITEM_TOOL_<tier>` (reuse the existing
  per-tier tool textures), oriented like a held tool.
- **Material** → small cube with the material's item texture.
- **Empty** → no mesh.

Used by both §3.4 (hand) and §3.5 (viewmodel) so they never diverge — this is the
single source of truth the success criteria require.

---

## 4. Data flow

```
local input (mine/place, move, crouch, hotbar)
  → InputPacket (adds swing/crouch intent)
  → HostedServer authors ServerPlayer {pos, yaw, pitch, held ItemRef, anim_state, flags}
  → StateUpdatePacket.players: Vec<PlayerState>  (extended)
  → client: per remote PlayerState
        → swing edge-tracker updates per-player swing progress
        → build_player_avatar_vertices(...) [model + anim + held_item_mesh]
        → entity vertex buffer → entity render pass (all viewports)
local player (own state, not from wire)
  → viewmodel pass: arm_r + held_item_mesh, view-space, bob + swing
```

---

## 5. Testing

**Pure-function units (`#[cfg(test)]` in-module):**
- Swing decay curve: 0 at t=0 and t=1, single peak, bounded by `SWING_PEAK`.
- Anim-state derivation from input (speed→walk, `!on_ground`→jump, crouch flag).
- `flags` pack/unpack round-trip.
- `held_item` `ItemRef` ↔ (`held_kind`,`held_id`) encode/decode round-trip incl.
  tools and empty.
- `player_model()` invariants: 6 parts, arms/legs animated with alternating phase,
  head `pitch_tracks_look`.
- `held_item_mesh`: block→cube vertex count, tool→slab, empty→0 verts.
- Viewmodel view-space transform: deterministic placement (lower-right, in front of
  near plane) for a known camera.

**Protocol round-trip:** extended `PlayerState` bincode encode→decode equality.

**Integration (`TestHost`, `src/test_integration/`):**
- Server broadcasts a player holding a **tool** with `swinging` set → emitted
  `PlayerState` carries `held_kind == 2` and `flags & SWING`.
- A remote `PlayerState` with a known item yields a non-empty avatar mesh with the
  expected part count **plus** an attached-item sub-mesh; ghost-box path is gone.

**`./check.sh`** is the gate (clippy → build → `cargo test` → trunk → bundle size).

---

## 6. Verification boundary (autonomy limit)

Logic and wire formats are fully covered by tests + `./check.sh` and will be run
green before any merge. **The *feel* cannot be verified solo:** animation
smoothness, swing timing, viewmodel placement/scale, avatar readability, and
multiplayer behaviour all need eyes on a running client (and ideally a second
client / split-screen). Per the standing autonomy boundary, the build stops at that
point with a **test sheet** (`docs/test-sheets/`) enumerating what Axolittle should
check, rather than claiming visual success.

---

## 7. File-change summary

- `protocol.rs` — extend `PlayerState`, bump `PROTOCOL_VERSION`, extend `InputPacket`.
- `server.rs` / `hosted_server.rs` — author new `ServerPlayer` fields each tick.
- `entity_model.rs` — `player_model()`, `ModelPart.pitch_tracks_look`, animation
  extensions to `build_part_vertices` + a pure anim-rotation helper.
- `texture_gen.rs` — `TEX_PLAYER_*` procedural skin layers.
- `held_item_model.rs` (new) — shared item→mesh mapping + `ItemRef` wire helpers.
- `game_loop.rs` — replace ghost path with avatar build; swing edge-tracker; name tag.
- `renderer.rs` — first-person viewmodel pass (depth-clear, view-space, per viewport).
- tests — units across the above + two `test_integration` cases.
- `docs/spec/03-rendering.md` + `docs/spec/04-networking.md` — update to reflect the
  implemented avatar/viewmodel/held-item-on-wire reality (spec-maintenance rule).
- `docs/test-sheets/2026-05-24-player-avatars.md` — playtest sheet.
