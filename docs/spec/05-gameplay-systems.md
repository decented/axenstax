# 05 — Gameplay Systems

**Status**: Draft
**Last updated**: 2026-03-03
**Addendum**: `_audit-2026-04-18.md` — `PlayerIntent` is analog (`move_forward:
f32`, `move_right: f32`) with split `jump_held`/`jump_pressed` events, not the
all-boolean form §12.1 describes. Mob spawning is a pure free function
(`spawning::tick_mob_spawning`). Server-side remote-player physics path + speed
cap anti-cheat landed in Task 1d. World chat (§4.6) supersedes §10.1's
broadcast-to-all/formatting-codes/3-per-second design for anything native —
see `docs/foundations/2026-09-05-world-chat.md`.
**Depends on**: ADR-001 (Full Custom Engine), ADR-002 (Tech Stack), Platform Overview

---

## 0. Design Philosophy

Every system in this document serves one overriding goal: **a Minecraft veteran must feel at home within 30 seconds of spawning**. The benchmark player is "Axolittle" — an 11-year-old with thousands of hours in Minecraft Java Edition. If Axolittle picks up the controls and something feels wrong — movement is floaty, block placement is laggy, breaking feels unresponsive — we have failed, regardless of how architecturally clean the code is.

Minecraft is the reference point, not the ceiling. Where Minecraft has known pain points (combat timing, redstone inconsistency, inventory management), Axe'n'Stax improves. Where Minecraft nailed it (movement physics, block interaction rhythm, hotbar flow), we match or exceed.

All gameplay systems are **server-authoritative**. The client predicts, the server validates. Cheating must be structurally difficult, not merely discouraged.

All values in this document are **tuneable constants** exposed through the game's configuration system. Plugin authors can override per-server. The values listed here are the defaults — chosen to match Minecraft feel unless explicitly noted as a divergence.

---

## 1. Player Movement

This is the single most important system in the game. Movement feel determines whether a player stays past the first minute.

### 1.1 Movement Speeds

All speeds measured in blocks per second (b/s). One block = 1 metre.

| Mode | Speed (b/s) | Minecraft Reference | Notes |
|---|---:|---:|---|
| Walking | 4.317 | 4.317 | Exact match. This is muscle memory. |
| Sprinting | 5.612 | 5.612 | 30% faster than walking. |
| Sneaking / Crouching | 1.295 | 1.295 | 30% of walking speed. |
| Flying (Creative) | 10.89 | 10.89 | Same as Minecraft creative flight. |
| Flying + Sprint | 21.78 | 21.78 | Double creative flight speed. |
| Swimming (surface) | 2.20 | 2.20 | Reduced from walking. |
| Swimming (underwater) | 1.97 | 1.97 | Slightly slower than surface. |
| Sprint-swimming | 5.612 | 5.612 | Matches sprint speed on land. |
| Climbing (ladder/vine) | 2.35 | ~2.35 | Vertical movement on climbable blocks. |
| Falling (terminal velocity) | 78.4 | 78.4 | 3.92 b/s/s acceleration (= 0.08 b/tick at 20 TPS). |

### 1.1.1 Camera / Mouse Look

Mouse input controls yaw (horizontal) and pitch (vertical) rotation.

```
yaw   -= mouse_dx * sensitivity    // Negative: mouse-right turns view right
pitch -= mouse_dy * sensitivity    // Negative: mouse-down looks down (standard, non-inverted)
```

- **Pitch clamped** to [-89°, +89°] to prevent camera flipping
- **Sensitivity**: default 0.003 (tuneable)
- **Mouse look runs per-frame** (not per-tick) for smoothness — only the position/velocity update at 20 TPS
- **Forward direction**: `(-sin(yaw) * cos(pitch), sin(pitch), -cos(yaw) * cos(pitch))` — yaw=0 looks along -Z (Minecraft convention)

**Bug history**: The original prototype had `yaw += dx` (positive), which inverted horizontal look. Must be `yaw -= dx`.

**Hold-to-zoom (#44)**: holding **C** (keyboard, player 0) or the touch **Z** button sets
`Camera.zooming`, which transiently swaps the render FOV to `Camera.zoom_fov` (default 20°,
`ZOOM_FOV_MIN..=ZOOM_FOV_MAX` = 15–45°) in `effective_fov_y()`. It is render-only: it bypasses
the 60–100° personal-FOV clamp and never mutates the stored `fov_y`, so releasing restores the
configured FOV exactly. Gated on cursor capture, so an open menu/chat/inventory releases the zoom.
The 8-point facing compass shown in the F3 debug overlay derives from this same yaw convention
(`hud_ui::yaw_to_compass`: -Z=N, -X=W, +Z=S, +X=E).

**Window focus loss (alt-tab / tab switch)**: on `WindowEvent::Focused(false)` the client drops
**all held and continuous input** — every key in `keys_held`, both mouse-button held flags, the
sprint latch, and the accumulated look-delta — via `InputState::release_all_inputs()`, and releases
cursor capture (`release_cursor()`). This is mandatory because focus loss suppresses OS/browser
key-up and pointer events: without it a movement key held at tab-out stays latched (the avatar keeps
walking on return), and `cursor_captured` stays stale-true so the next click hits the break/place
path instead of re-grabbing — and on web no `MouseMotion` deltas arrive without pointer lock, so the
view won't turn ("trackpad is off"). Re-capture is **click-driven** on focus regain (a user gesture
is the only way to re-acquire pointer lock on web anyway); on regain the client also zeroes any
stray accumulated look-delta so the camera doesn't snap. Wired in the `window_event` match
(`main.rs`); covered by `input::tests::release_all_inputs_drops_held_and_continuous_state`.

**Text-field focus suppresses gameplay input (hardened 2026-07-07).** Per-tick gameplay
input (movement/look/break/place/etc., player 0) is zeroed whenever an egui text field
holds keyboard or pointer focus (`egui_integration::{wants_keyboard_input,
wants_pointer_input}`, folded into the existing modal-open gate via
`gameplay_input_suppressed`), in addition to the existing per-panel modal flags. This
closes a held-key gap: pressing and holding a movement key, then clicking into a text
field (world-name/sign/search/rename box) without releasing it, used to keep the
character walking because only *new* keypresses were blocked by the window-event-level
egui filter — the already-held key was never cleared. The egui-focus check is a
blanket safety net so a future text field can't reopen the same gap by forgetting to
add its own flag to the modal enumeration.

#### 1.1.2 Camera perspective (first / third person)

The camera carries a `CameraMode` (`camera.rs`): `FirstPerson` (default) `|` `OverShoulder` `|`
`OrbitBehind`. A perspective toggle (keyboard F5-style cycle, a gamepad button, a touch HUD
button) advances it per-player, so split-screen Just Works (each `PlayerSlot` owns its
`Camera`). Third-person draws the player's own avatar (`entity_model::build_player_avatar_vertices`)
and hides the first-person viewmodel.

**The render camera moves; aim does not.** Only the *render-view origin* is pulled back in
third-person (a pure `render_eye(eye, forward, right, mode)` plus a per-profile distance
scale and a free-look orbit offset); `camera.position`/`forward()` keep their first-person
meaning as the **eye + look direction that every gameplay ray reads** (see §2.5). This split
is the single invariant a future rebuild must preserve — it is the difference between
third-person that feels right and third-person that feels broken. Render refinements layered
on top (all render-only): no-snap collision pull-in with slow ease-out, per-block
`camera_occlusion` (`Squeeze`/`PassThrough` derived from `solid`+`transparent`, so glass/leaves
behave by registry intent not visual box), avatar fade-on-occlusion (screen-door dither), and
per-context feel profiles (Build/Combat) + velocity-gated auto-recenter. Full design + the
phase ladder: `docs/foundations/2026-06-09-third-person-camera.md`.

### 1.2 Jumping

| Parameter | Value | Notes |
|---|---:|---|
| Jump height | 1.2522 blocks | Clears 1 block + margin. Matches MC exactly. |
| Jump initial velocity | 0.42 b/tick (at 20 TPS) | Applied as instantaneous upward impulse. |
| Jump cooldown | 0 ticks (immediate re-jump on landing) | Bunny-hopping must feel natural. |
| Sprint-jump distance | ~4.0 blocks horizontal | Critical for parkour. Must match MC. |
| Sprint-jump + momentum | Preserves horizontal velocity through arc | No air-speed capping during sprint-jump. |

**Sprint-jumping is a litmus test.** If a Minecraft player cannot sprint-jump across a 4-block gap on the first try, movement is broken.

### 1.3 Acceleration and Deceleration

Movement uses a **momentum-based model** with per-tick drag, matching Minecraft's internal physics:

```
velocity_next = (velocity_current + acceleration) * drag_factor
```

| Surface | Drag Factor | Acceleration Factor | Feel |
|---|---:|---:|---|
| Ground (normal) | 0.91 | 0.1 | Snappy start, quick stop. ~2-3 ticks to full speed. |
| Ground (ice) | 0.98 | 0.02 | Sliding. Low friction. |
| Ground (soul sand) | 0.91 | 0.1 | Same drag, but speed cap reduced to 2.508 b/s. |
| Air | 0.91 (horizontal) | 0.02 | Limited air control — committed to trajectory. |
| Water | 0.80 | 0.04 | Heavy drag. Swimming feels effortful. |

**Air control**: Players can influence horizontal direction mid-air but cannot reverse momentum. The 0.02 air acceleration means a sprint-jump commits your trajectory. This is intentional — it rewards planning and creates the "Minecraft parkour feel" where you aim before jumping.

**Deceleration**: When no movement input is held, only the drag factor applies. On normal ground (0.91 drag), the player stops within 2-3 ticks. There is no "skating" unless on ice.

### 1.4 Collision Detection

Collision uses **AABB (Axis-Aligned Bounding Box) vs voxel grid** intersection.

**Player hitbox**:
- Standing: 0.6 blocks wide, 1.8 blocks tall (centred on feet position).
- Sneaking: 0.6 blocks wide, 1.5 blocks tall.
- Swimming / elytra: 0.6 blocks wide, 0.6 blocks tall (prone).

**Collision resolution order**: Y-axis first (gravity/jumping), then X, then Z. This prevents corner-catching artifacts that feel "sticky."

**Sub-stepped collision (2026-09-06 — fixes "you still go through blocks").** Each axis pass inspects only the single cell column the leading edge lands in *after* the move, so applying a whole tick's displacement in one shot let a fast mover land past a 1-block wall/floor, which was then never examined. Creative sprint flight (21.78 b/s = **1.089 blocks/tick**) tunnelled reliably. **Rule:** movement code computes velocity only; `Player::tick` then integrates in `n = ceil(|v| / MAX_STEP)` equal sub-moves with **`MAX_STEP = 0.4`** blocks (well under `min(PLAYER_WIDTH, 1.0)`), resolving collisions after each. An axis stopped by a sub-step is dropped from the remaining delta; `on_ground` is OR-ed across sub-steps. `n == 1` at walking/sprinting speed, so normal movement is unchanged. `MAX_FALL_SPEED` (0.78 b/tick) is consequently **no longer a tunnelling guard** — it is now purely a feel constant (terminal velocity + fall-damage tuning) and may be re-tuned freely.

**Swept landing (2026-09-06 — fixes the sprint-jump wall vault).** The Y pass runs first, so with a sub-move that applies all three axes at once it could see a horizontal penetration of a few millimetres that the X/Z passes were about to undo — and it accepted *any* box in the foot's cell with `pos.y < box.top` as ground, however far above the foot that top was. Sprint-jumping into a wall ≥2 blocks tall therefore snapped the player onto the wall top and walked them over it. **Two rules now gate the Y pass, both keyed on the pre-sub-move position `prev_pos` that `integrate_substepped` hands to `resolve_collisions`:** (1) *swept* — a falling player lands on a box only if the foot was at or above its top before the sub-move (`prev_foot_y >= box.top - LAND_EPS`, `LAND_EPS = 1e-3`), and symmetrically a rising player bonks a ceiling only if the head was at or below its underside; (2) *pre-move footprint* — the Y pass takes its horizontal extent from `prev_pos.x/z` (inset by `FOOT_INSET = 1e-3` so contact needs real overlap, not a float tie on a block face), i.e. it behaves as if Y were integrated before X/Z, which is what "resolve Y first" means. Only the foot HEIGHT comes from the post-move position. **Stuck-case exception:** if the hitbox was *already* intersecting that box at the start of the sub-move (spawned inside terrain, a block placed on the player, a teleport), the old unconditional push-out to the box top is kept, so "get out of a block you are inside" still works. Consequence, and the intended behaviour: arriving at a block's side with your feet below its top is a wall bump, not a free step up — a 1-block obstacle must be jumped (rail ramps excepted).

**Step-up**: The player automatically steps up onto solid blocks that are at most **0.5 blocks tall** (half-slabs, carpet, etc.) without jumping. This is resolved during horizontal collision — if the player would collide with a block edge that is <= 0.5 blocks above their foot position, translate them up instead of stopping them. Step-up must feel instantaneous (single-tick).

**Per-block collision shapes (F1 — block-shape foundation, 2026-06-19).** Collision no longer treats every solid as a unit cube. `World::collision_boxes_at(x,y,z,registry)` returns the world-space solid boxes at a cell — one unit cube for ordinary solids (byte-identical to the old `is_solid` box), or the meta-derived sub-boxes for **shaped** blocks (`block_shape::{shape_of, collision_aabbs}`). `Player::resolve_collisions` iterates those boxes per axis with a perpendicular-axis overlap test. Full-cube behaviour is unchanged — scanning `floor(pos.y)` for the foot still works because gravity nudges the foot into the supporting box's cell each tick. First shaped blocks: `STONE_SLAB` (half-height; the occupied half — incl. the four #27 **vertical** slabs — chosen by `meta.facing`) and `STONE_STAIRS` (a half-slab + a quarter step, two boxes; `meta.facing` = ascent direction, `meta.state` = upside-down). Orientation is stamped at place-time from the clicked face + camera; render geometry shares the same `block_shape::render_cuboids` source (Spec 03 §rendering). The building-detail family (doors, trapdoors, fence gates, walls, glass panes, signs, frames + #29 double doors) builds on this foundation. Spec: `docs/foundations/2026-06-19-block-shape-foundation.md` (finding: `docs/foundations/2026-06-16-building-detail-blocks.md`).

**Wave 2c interactions (2026-06-19).** The building-detail family completed:
- **Walls + connecting panes** collide against their neighbour-derived boxes (`World::connection_mask`); a wall/pane between two blocks fills the gap so you can't pass, while a lone one is just a thin post. Recipe: `COBBLESTONE_WALL` = 6 cobble (3×2 → 6).
- **Signal inputs (button/lever/pressure-plate)** are **pass-through** (no collision) and sit flush — you walk onto a plate / past a button. The Electricity power behaviour is unchanged; the F1 work is purely the flush shape. Right-clicking a lever still drives the power handler (it is deliberately NOT in `is_toggleable`, which would hijack it).
- **Signs** are pass-through. Right-click (or placement) opens a text editor (`sign_ui`, gated like the chest UI); looking at a placed sign surfaces its text in the HUD (`hud_ui::draw_sign_text`). Text persists in `BlockEntityData::Sign`. Recipe: `OAK_SIGN` = 6 planks + 1 stick (→ 3).
- **Item frames** are pass-through. Right-click an empty frame to mount the held hotbar item (one unit), right-click a filled frame to rotate it (8 steps), break to drop the framed item. Block items render on the frame; any item's name shows on look. Recipe: `ITEM_FRAME` = 8 sticks + 1 leather (→ 1).

**Edge sneaking**: When sneaking, the player's bounding box is tested against the block grid at the next movement position. If the next position would place the player's centre beyond the edge of a solid block (i.e., they would walk off a ledge), horizontal movement is clamped. The player can peer over edges without falling. This is essential — experienced Minecraft players rely on sneak-edge mechanics for bridging (placing blocks while walking backward off an edge).

**Head bonking**: When jumping and hitting a solid block above, vertical velocity is immediately zeroed. No bounce, no upward carry. This matches Minecraft.

### 1.5 Fall Damage

**Built 2026-10-05 (audit wave W2).** Rules: `survival::fall_damage` (pure, unit-tested); tracking: `physics::Player::track_fall`.

| Parameter | Value |
|---|---:|
| Safe fall distance | 3 blocks |
| Damage | `ceil(d − 3)` HP — 1 HP (half a heart) per block beyond 3, rounded up |
| Boundaries | 3.0 → 0 HP · 3.1 → 1 · 4.0 → 1 · 4.1 → 2 · 10 → 7 |
| Lethal fall height | > 22 blocks (at 20 HP): 22 → 19 HP, 23 → 20 HP |
| Water negation | Landing in (or splashing into) water negates all fall damage, any depth |
| Hay bale | −80%: `ceil((d − 3) · 0.2)` (Minecraft's formula) |
| Slime block | −100% — **not built: no slime block exists in the registry yet**; add it to `survival::landing_multiplier` when it ships |
| Creative / Spectator | Immune |
| Difficulty | Applies on every difficulty, **Peaceful included** (Minecraft parity); never scaled by difficulty |
| Armour | **Bypassed** — applied straight to `PlayerCombat` (§6.3) |

**Tracking.** `Player::tick` records the foot height before its move and, after integration, accumulates any *downward* motion into `Player.fall_distance` while airborne. Flight, noclip, swimming (`in_water`) and ladders (`World::is_climbable`, so vines too when they ship) reset it. On the tick the body touches ground — or its foot cell becomes water — it emits a `physics::Landing { fall_distance, landing_block }` (`landing_block` is the cell a hair below the foot, or `WATER` for a splash-down) and resets. Only motion *inside* `tick` counts, so a teleport (`/tp`, respawn) is never a fall; respawn and mounting a cart call `Player::reset_fall` explicitly. Upward motion (a jump, knockback) never adds. A small epsilon (1e-3) is subtracted before `ceil` so the accumulated float sum of an exact 3-block drop stays free.

**Who applies it (dual-sim, as built).** One shared driver, `survival::tick_player_survival(player, combat, world, mode)`, consumes the landing and applies the damage. **Local players** (single-player, split-screen, the host's own seat, and a joined client's own body): `game_loop.rs` per-player combat pass. **Server-simulated remote players**: `GameServer::tick_player_physics` → `tick_player_survival`. A remote player's own client and the host each compute it from their own copy of the physics — the client's copy drives that player's HUD and death, the host's copy is what other players see in `PlayerState.health`. **Since MP-D2a (protocol v68) the server's copy is the joiner's health:** the joined client runs `survival::survival_hits` on its own body for breath only and drops the hits; the server's fall / drowning damage arrives in the joiner's own `PlayerState.health` (Spec 04 §5.3.2). (Single-player still bypasses `GameServer` — CLAUDE.md known debt.)

**I-frames.** Fall damage goes through `PlayerCombat::take_damage_from`, so it respects the 10-tick invulnerability window after another hit: a landing within 0.5 s of a mob hit does no fall damage. Deliberate simplification (Minecraft lets the larger hit through); not a bug.

### 1.5.1 Drowning

**Built 2026-10-05 (W2).** Rules: `survival::tick_breath` (pure, unit-tested); state: `PlayerCombat.breath` (transient — never saved, full on respawn).

| Parameter | Value |
|---|---:|
| Trigger | Head (eye cell) in water — `survival::head_in_water` |
| Air supply | 300 ticks (15 s) |
| Out of air | 2 HP every 20 ticks — first hit on tick 320 of a dive from full lungs, then 340, 360… |
| Refill | 8 air per tick with the head out of water → full in 38 ticks (~1.9 s); surfacing clears the drowning counter |
| Armour | Bypassed |
| Creative / Spectator | Immune — lungs stay full |
| HUD | 10 bubbles (`ceil(air · 10 / 300)`), one row above hunger, shown **only** while underwater or refilling (`survival::show_breath_bar`) |

Same two call sites as fall damage (§1.5). Server-side the breath pass runs in its own loop over every connected server-simulated player, so a tick with no queued intent still advances breath.

### 1.6 Ladder and Vine Climbing

- Ladders and vines are climbable blocks. **Implemented 2026-06-16 (#30)**: the `LADDER` block (non-solid, so you stand *inside* it). While the player's feet overlap a climbable block (`World::is_climbable`, mirroring `is_water`), the physics climb branch suppresses gravity and `physics::ladder_climb_vy` drives vertical motion — **jump climbs up, sneak descends, neither clings in place** — at `LADDER_CLIMB_SPEED` 2.5 b/s. Horizontal walking is unchanged so you step off normally; collision still runs after. The climb-velocity rule is a pure, unit-tested function.
- *Alpha simplification*: climb is jump/sneak-driven (not the "hold-forward / slow-descent-on-release" variant originally sketched), and the ladder renders as a centred rear panel — **per-wall orientation waits on the block-state foundation** (`docs/foundations/2026-06-16-building-detail-blocks.md`). Feel = playtest. Vines reuse the same `is_climbable` path when they ship.
- Moving horizontally off a ladder/vine resumes normal gravity.

### 1.7 Creative Flight

- Toggle: double-tap jump to enter/exit flight.
- While flying, the player hovers at current Y. Pressing jump ascends; pressing sneak descends.
- Holding sprint doubles flight speed.
- No gravity while in flight mode.
- Collision detection still applies — the player cannot fly through blocks.
- Exiting flight mode resumes normal gravity (and potential fall damage if high up).

### 1.7.1 Swimming

Swimming uses **direct velocity control with drag**, not the acceleration-based model used for ground/air movement. This is a critical design decision discovered through testing: the acceleration model's multiple multiplicative scaling factors (speed_per_tick, accel, drag) made it nearly impossible to produce an upward force that reliably overcame water gravity. The player sank even while holding jump.

**Swimming physics model**:
- **Horizontal movement**: Player intent sets a target horizontal velocity. Water drag (0.80) is applied each tick, creating the heavy/effortful feel.
- **Upward (jump/Space)**: Directly sets upward velocity. The player reliably swims up.
- **Downward (sneak/Shift)**: Directly sets downward velocity.
- **No input**: Water gravity pulls the player down slowly. Drag prevents runaway sinking speed.
- **Sprint-swimming**: Holding sprint while swimming matches sprint speed on land (5.612 b/s).

This is analogous to creative flight but with water drag applied, giving the "heavy" feel of water without the broken arithmetic of acceleration-based swimming.

**Why not acceleration-based**: With the ground movement model (`velocity = (velocity + accel) * drag`), the jump input produced a per-tick upward force of ~0.009 blocks/tick after all scaling factors. Water gravity was 0.02 blocks/tick. Net: sinking. Tuning the constants to fix swimming would have broken ground movement. Direct velocity control sidesteps the problem entirely.

### 1.7.2 Sprint-Jumping

Sprint-jumping requires an explicit **forward velocity boost of 0.2 blocks/tick** applied in the player's facing direction at the moment of jump initiation (when `on_ground && jumping && sprinting`). This boost is in addition to the normal jump vertical impulse.

Without this boost, sprint-jumping covered only ~2 blocks horizontally — far short of Minecraft's ~4 block standard. The sprint speed multiplier alone is insufficient because drag immediately reduces velocity after leaving the ground. The 0.2 boost compensates for the drag-induced speed loss during the airborne arc.

**Sprint-jump is a litmus test** (see section 1.2). If it doesn't cover ~4 blocks, movement is broken.

### 1.8 Client-Side Prediction and Reconciliation

Movement is **predicted on the client and validated on the server**.

1. Client computes next position locally using the same physics simulation.
2. Client sends input state (movement direction, jump, sprint, sneak) each tick.
3. Server runs authoritative physics, produces canonical position.
4. Server sends position corrections to client.
5. If client position diverges from server position by more than **0.1 blocks**, the client snaps to the server position. Divergences under 0.1 blocks are smoothly interpolated over 3-5 ticks to avoid visible rubber-banding.
6. If divergence exceeds **4.0 blocks** (indicative of speed hacking or extreme lag), the server forcibly teleports the player and logs a flag.

**Tick rate**: The server simulation runs at **20 ticks per second (TPS)**, matching Minecraft. The client renders at the display refresh rate and interpolates between ticks for smooth visuals. Client-side physics also step at 20 TPS for prediction consistency, with visual interpolation filling in the gaps.

---

## 2. Block Interaction

### 2.1 Block Breaking

Block breaking is **progressive** — the player holds the attack/break input, and the block gradually cracks over time until it breaks.

**Break time formula**:

```
base_break_time = block_hardness / tool_speed_multiplier
```

If the tool is the wrong type for the block (e.g., pickaxe vs dirt), `tool_speed_multiplier` = 1.0 (hand speed). If the correct tool is used, the multiplier depends on tool tier.

> **Proof-of-play credit (Spec 6 §2.2).** Breaking a block tallies `block_work` into the world's `total_work` / scenario score on a successful harvest — **except** for blocks a *player* placed, which earn **no** work or hash-driven drop when re-mined (you still recover the item). This closes the chop-restand-rebreak / place→break farming loop. The origin is tracked by a per-voxel "placed" bit (Spec 2 §4.3). BUILT 2026-06-08.

| Tool Tier | Speed Multiplier | Example |
|---|---:|---|
| Hand (no tool) | 1.0 | Punching anything. |
| Wood | 2.0 | Wooden pickaxe on stone. |
| Stone | 4.0 | Stone pickaxe on stone. |
| Iron | 6.0 | Iron pickaxe on stone. |
| Diamond | 8.0 | Diamond pickaxe on stone. |
| Satori | 9.0 | Top-tier tool on stone. See §3.8 for material provenance. |

**Hardness values** (selected blocks):

| Block | Hardness | Hand Break Time | Best Tool | Best Tool Break Time |
|---|---:|---:|---|---:|
| Dirt | 0.5 | 0.75s | Shovel | 0.1s (instant feel) |
| Sand | 0.5 | 0.75s | Shovel | 0.1s |
| Wood (log) | 2.0 | 3.0s | Axe | 0.5s (wood tier) |
| Stone | 1.5 | 7.5s (hand cannot harvest) | Pickaxe | 0.375s (iron) |
| Iron Ore | 3.0 | Cannot harvest by hand | Pickaxe | 0.5s (iron) |
| Obsidian | 50.0 | Cannot harvest by hand | Pickaxe | 6.25s (diamond) |
| Bedrock | Infinity | Indestructible | None | N/A |

**Harvest rules**: Certain blocks require a minimum tool tier to drop items. Mining stone without a pickaxe breaks the block but drops nothing. Mining iron ore without at least a stone pickaxe drops nothing. These rules are defined in the block registry and are plugin-extensible.

**Drops — one set of rules, two places they run (as built, C1 2026-10-07).** What a survival break yields lives in one module, `break_drops` (`break_yield`), read from the world *before* the block leaves it: a harvested crop's drops and replacement (`growth::crop_break`); **lava, water, fire, smoke and air yield nothing to every breaker** (`break_drops::yields_drops`, FU2 2026-10-07: the survival raycast can target and dig lava and fire, and single-player used to get a placeable LAVA block from it while a joiner got nothing); otherwise, when `crafting::can_harvest` allows the held tool, the mine drop (`BlockRegistry::mine_drop_with_seed`) plus any bonus stack (`bonus_mine_drop`); and a Satori alongside when §3.8's conditions pass, never from a player-placed block. Chance rolls are seeded by tick and cell (`drop_seed`). Single-player and a host's own players run it in the client's break arm and take the yield straight into the breaker's inventory (`take_yield`; **what doesn't fit spills as ground items at the breaker's feet** (`spill_at_feet`, FU2 2026-10-07), the rule a joiner's overflowing grant has — the pickup pass takes it back when space frees up, so nothing is lost; a crop harvest that overflows also says "Inventory full — harvest dropped at your feet", because players repeat-harvest; a Satori that finds no slot spills too and is still counted as found). **A joiner's break is the server's to yield:** the joined client still breaks the block in its own world, tells the server it mined it (and with which tool) and takes nothing itself; the server runs the same `break_yield` on its world and Proof-of-Play secret and grants the stacks by `InventoryGrant` (Spec 04 §4.2e). The joiner gets each drop exactly once; a stack that doesn't fit spills at its feet (the grant rule). Face attachments and a drying rack's logs recovered on a break are still granted by the breaker's own client, and tool wear stays client-side.

### 2.2 Break Animation

Breaking is displayed as a 10-stage crack overlay on the target block:

- Stage 0 (0-10% progress): Hairline cracks.
- Stage 9 (90-100% progress): Heavy fracturing.
- Block breaks (progress ≥ break_time): particles emit, block entity spawns (or item goes to inventory if close enough).

The crack animation is **client-predicted**. The client knows the block hardness and current tool, so it can animate locally without waiting for server confirmation. If the server determines the break should not happen (moved too far, wrong tool, anti-cheat flag), the crack animation resets.

**Implementation (delivered 2026-05-24)**: The stage is a pure function
`crafting::crack_stage(break_progress, break_time) -> Option<u8>` —
`floor(progress / break_time * 10)` clamped to 0-9, returning `None` before any
progress accrues (so the overlay only appears once mining starts) and for
instant/creative breaks (`break_time == 0`). The ten crack textures are
procedurally generated (`texture_gen::gen_crack_stage`, atlas layers
`block::TEX_CRACK_BASE`..+10): near-black cracks on a transparent ground that
grow in count, length, thickness and opacity with the stage. Rendering: a
slightly-inflated textured cube over the targeted block
(`renderer::build_crack_cube` + `set_crack_overlay`), drawn by a dedicated
alpha-blended, depth-read-only crack pass (Pass 2.6, after the wireframe and
before the viewmodel) using the chunk shader's `fs_crack` entry. The game loop
recomputes the overlay each frame next to the block highlight
(`game_loop.rs`, alongside `set_block_highlight`), so it tracks the existing
`break_progress` state and clears the moment mining stops. Per-player, so
split-screen works. **Deferred**: crack stages for *remote* players mining are
not broadcast (single-viewport/split-screen only); break particles on
completion are a separate feature. See
`docs/foundations/2026-05-24-block-break-crack-overlay.md` and the rendering
detail in `docs/spec/03-rendering.md §"Block-break crack overlay"`.

### 2.3 Block Placement

**Placement rules**:

1. The target block must be air (or a replaceable block like tall grass or water).
2. The player must be targeting an adjacent solid face (ray-cast from eye position).
3. The placed block must not overlap any entity's bounding box (including the player). Exceptions: (a) the player can place blocks at their feet if they have room to be pushed up (e.g., pillar-jumping — placing a block below yourself while jumping); (b) **non-solid blocks (cable, track, torch, sapling…) are exempt entirely** — they have no collision, so they may share a player's cell. Fixed 2026-07-04 ("you cant put the cables on the ground"): the overlap guard used to reject *all* blocks in an occupied cell, which silently no-oped the natural lay-a-cable-at-your-feet gesture. Pure rule: `placement::placement_blocked_by_player` (unit-tested); solidity of the would-be block is peeked via `Inventory::hotbar_placeable_id` before the guard runs. The "step back a little" toast now fires only when a *solid* placement is actually blocked.
4. The player must have the block in their active hotbar slot.
5. Placement reach: **5.0 blocks** from eye position (single value in code: `REACH_DISTANCE = 5.0`, in `lib.rs` and `server.rs`; the server gate adds a small tolerance margin, see §2.5). There is no separate survival (4.5) and creative (5.0) reach; both game modes use 5.0.

**Placement direction**: Blocks that have directional variants (furnaces, stairs, pistons) orient based on the player's facing direction and the face they clicked. Logs orient based on the face clicked (place on top = vertical, place on side = horizontal).

**Server validation**: Every place action is validated server-side:
- Is the target position within reach?
- Does the player have the block in inventory?
- Is the target position valid (air/replaceable, no entity overlap)?
- Is the placement rate reasonable (anti-spam: the client paces normal placements via a 6-tick cooldown ≈ 3.3/s — see §2.8; the server enforces a ceiling no looser than this)?

### 2.4 Immediate Feedback with Server Confirmation

Block placement and breaking use **optimistic client-side prediction**:

1. **Client places block**: Client immediately renders the block at the target position and decrements the local inventory count. Client sends a `PlaceBlock` packet to the server.
2. **Server validates**: Server checks all placement rules. If valid, the block is committed to the world state and the server broadcasts the change to all nearby clients (including the placer, as a confirmation).
3. **On rejection**: Server sends a `RejectPlace` packet. Client removes the predicted block and restores the inventory count. A subtle "pop" animation makes the block vanish cleanly rather than just disappearing.
4. **Latency window**: Under normal conditions (< 100ms RTT), the player never notices the round-trip. The block appears instantly. Under high latency (> 200ms), there is a brief window where the block is "ghosted" (slightly transparent) until confirmed.

Breaking follows the same pattern: the client predicts the break, plays the particle effect, and removes the block. If the server rejects, the block reappears.

### 2.5 Block Reach and Ray Casting

The player's eye position is calculated as:

```
eye_pos = player_pos + (0, 1.62, 0)   // standing
eye_pos = player_pos + (0, 1.27, 0)   // sneaking
```

A ray is cast from `eye_pos` in the look direction up to the reach distance. The ray is tested against the voxel grid using a **DDA (Digital Differential Analyzer)** algorithm, which steps through voxels one at a time along the ray. The first non-air block hit is the target. The face of entry determines the placement side.

**Eye-anchored-aim invariant (CRITICAL — must survive rebuilds).** Every gameplay ray —
block break, block place, combat, item-drop — originates from `player.eye_pos()` + the look
direction (`camera.forward()`), **never** from the third-person render camera's pulled-back
origin (`camera.render_eye()`). In third-person the view is rendered from a moved-back (and,
under free-look, decoupled) render eye, but the crosshair still means exactly what it shows
because aim keeps raying from the true eye + look. A `TestHost::crosshair_target()` integration
test asserts the hit cell is **identical** in `FirstPerson`, `OverShoulder`, and `OrbitBehind`
for the same yaw/pitch/position, and re-runs on every build so the split can't silently
regress. See §1.1.2 and `docs/foundations/2026-06-09-third-person-camera.md`.

**Reach cap and multiplayer anti-cheat (hardened 2026-07-07).** Base block place/break
reach is `5.0` blocks (server gate: `(5.0 + 0.5)^2` squared-distance, the 0.5 a
tolerance margin). The **Reach Claw** tool (§9.7) adds `REACH_CLAW_BONUS = 2.0` blocks
while held. On a hosted session, this bonus is honoured **only for the position-trusted
local/hosting player** — a remote (`server_simulated`) player's block-change reach is a
**flat cap, independent of whatever held item the client claims**. This is deliberate:
the server has no authoritative knowledge of a remote player's actual inventory (only
the client-asserted `held_kind`/`held_id`, used for broadcasting the visible held item —
see Spec 08 §9.0.1), so trusting it for a reach *bonus* would let a modified client claim
a Reach Claw it doesn't have and grief at +2 blocks. (This was a real regression: an
earlier fix widened the server-side gate using the unchecked client claim; re-flattened
for remote players in the 2026-07-07 hardening pass. `// BRIDGE` in `hosted_server.rs`
notes the bonus returns for remote players once remote inventory becomes
server-authoritative.) Melee/combat reach is unaffected — this gate is block-edits only.

### 2.6 Continuous Breaking

Holding the break button mines continuously. When one block breaks, the system immediately begins breaking the next block the crosshair is pointing at (if within reach). The break progress does **not** carry over between blocks — switching targets resets progress to zero. This prevents exploits where players wiggle between blocks to accumulate break progress.

If the player moves the crosshair off the current target and back, break progress resets. This matches Minecraft behavior and is intentional.

### 2.7 Survival vs Creative Breaking

**Survival mode**: Block breaking is progressive. The player holds the break input and a `break_progress` counter increments each tick. The target tick count is computed by `break_time_ticks(block_type, held_tool)` which uses the break time formula from section 2.1. When progress reaches the target, the block breaks. Progress resets if the player looks at a different block or releases the break input.

**Creative mode**: A targeted block breaks in one tick (instant) regardless of tool — no progressive break timer. **But** instant breaking is rate-limited by a **break cooldown** (`PlayerSlot.break_cooldown`, `BLOCK_BREAK_COOLDOWN_TICKS` = 5 ticks ≈ 0.25s ≈ 4 breaks/s). Without it, a held break button destroyed one block **every tick (~20/s)** — Axolittle flagged this as "breaking too fast" in creative (2026-06-07). The cooldown is armed when a creative break is committed (`arm_break_cooldown`) and ticked down every frame (`tick_break_cooldown`), exactly mirroring the placement cooldown. **Survival is exempt** — it's already paced by `break_time` (block hardness), so the cooldown only gates the otherwise-instant creative path. Mechanically: the creative branch is `else if is_creative && break_ready()`; while it's on cooldown the survival branch (now guarded by `else if !is_creative`) is also skipped, so the held button simply does nothing that tick.

This distinction must be implemented from the start. A common bug is implementing instant breaking first and forgetting to add the timer for survival mode. The `break_time_ticks()` function (in `crafting.rs`) is the source of truth for break durations.

### 2.8 Placement Cooldown

Block placement has a **6-tick (300ms) cooldown** between successive placements for normal blocks, regardless of input source. After placing a block, the next placement is blocked for 6 ticks (~3.3 placements/s). This prevents:
- Controller trigger spam (triggers report as held every tick, producing ~20 placements/second without cooldown).
- Click-spam on mouse.
- Macro/autoclicker abuse.

This was **4 ticks (200ms)** through Wave 25; Axolittle found held-placement too twitchy in creative (2026-06-07), so the generic block-place path (`game_loop.rs`) was raised to 6 ticks. Special placements set their own cooldowns on the same `place_cooldown` field — e.g. saplings/campfires stay at 4 ticks, slingshot/bow shots at 8, beds at 16 — so this change only affects ordinary building. The cooldown applies equally to keyboard+mouse and gamepad inputs.

The 4-tick workstation cooldown (campfire, drying rack, blueprint stamp, furnace slot-clicks, etc.) is a named constant, `player_slot::PLACE_COOLDOWN_TICKS`, used at every `place_cooldown = ...` call site of that value (2026-09-06 hardening pass, replacing a repeated bare `4` literal). Decrementing happens through `PlayerSlot::tick_place_cooldown`, mirroring `tick_break_cooldown` above; both are pinned by unit tests in `player_slot.rs` (a cooldown of 4 reaches 0 after exactly 4 ticks, never underflows).

### 2.9 Blueprint Paper — paper-thin floor attachment and build capture

Blueprint Paper (`BLUEPRINT_PAPER`, id 56) is the **capture surface** for the Build
Schematics system (see `docs/foundations/2026-06-04-blueprint-face-attachment.md` for
the face-attachment design; `docs/foundations/2026-06-04-blueprint-column-capture.md`
for the capture semantics and Drafting Stamp; `docs/foundations/2026-05-27-blueprint-cyanotype.md`
for the cyanotype/develop mechanic).

**Lay.** Holding Blueprint Paper and right-clicking the **top face of a floor block**
lays a `BlueprintBlank` face attachment — paper-thin, zero physical height, zero
collision. A held Blueprint Paper **never** places a solid block-56 cube; right-clicking
any non-floor face is a no-op. The paper is purely a render-layer marking, not a world
block: builds above it sit at the correct world height (one block above the floor),
with no unwanted offset.

**Build.** Build the desired structure **directly on the paper, connected to the ground**.
The attachment does not impede movement or block placement. This is a 1:1 drawing: the
paper is the footprint, the structure you build above it is what gets captured.

**Lock-in (capture).** Use the **Drafting Stamp** (`ToolType::DraftingStamp`) — a
craftable, reusable tool (recipe v1 placeholder: Iron Ingot over Blueprint Paper; hotbar
abbreviation "Ds"; `/give drafting_stamp` works). Right-click **any block of your build**
(or the paper directly) with the Stamp: the helper `blueprint_attach::paper_tile_under`
scans straight **down the clicked column** until it finds a floor tile carrying a
`BlueprintBlank` attachment. That tile anchors the capture and `plan::capture` runs from
there. Clicking a column with no paper underneath is a no-op and shows a "no blueprint
here" feedback toast. The old empty-hand right-click trigger has been **removed**.

**Capture rule (`plan::capture_connected_volume`).** The capture algorithm seeds from
every solid block sitting directly on a paper tile (`base_y + 1`), then floods
6-connected (face neighbours) through **solid blocks only**, constrained to the columns
above the paper footprint and the height cap (`MAX_HEIGHT` = 32). **Air breaks the
flood.** The footprint/height envelope guards (`MAX_FOOTPRINT`/`MAX_HEIGHT` = 32,
`CaptureRefusal::TooLarge`) and the `EmptyVolume`/`Disconnected` refusals remain. This
rule produces the expected exclusion behaviours:

| Case | Result |
|------|--------|
| Connected build over the paper | ✅ captured |
| Cave ceiling above an air gap | ❌ excluded — the gap is the protection |
| Overhanging tree / stray disconnected block | ❌ excluded — air gap cuts the flood |
| Neighbour's wall over non-papered ground | ❌ excluded — outside the footprint |
| Open-floor gazebo/pergola (no floor blocks) | ✅ structure captured; floor cells are air → not captured |

6-connected only; diagonal-only joins are not captured (acceptable edge case — real
builds are face-connected).

The inspect/naming/licence dialog and `commit_capture` are unchanged.
- **Creative mode** → `DevelopState::Developed` immediately (blue; usable now).
- **Survival mode** → `DevelopState::Latent { exposure_ticks: 0 }` (pale; requires
  sun-develop, see below).

The `BlueprintBlank` attachments are **consumed** on lock-in (`remove_face_attachment`);
the underlying floor blocks **survive** (the old implementation incorrectly set them to
air — fixed in the face-attachment build).

**Survival sun-develop.** To develop a Latent plan, lay the `Item::Plan` back on any
floor top face (`blueprint_attach::lay_blueprint_on_floor`). This places a
`Blueprint(Latent)` face attachment. `latent_print::tick_develop_attachments` advances
the exposure counter while the attachment is in **full daytime sky-light**; shade, a
roof, or night pauses it. At `DEVELOP_THRESHOLD_TICKS` (1800 ticks ≈ 90 s) the state
flips to `Developed` and the decal recolours pale → blueprint-blue. Peel it to recover
the finished `Item::Plan(Developed)`.

**Two-stage peel.** First mining strike on any face attachment peels it and returns the
right item: `BlueprintBlank` → `Item::Block(BLUEPRINT_PAPER)`, `Blueprint(plan)` →
`Item::Plan(plan)`. Second strike breaks the underlying floor block. Destroying a block
clears and returns all its attachments.

**Persistence.** Attachments are saved as `SavedFaceBlankPaper` / `SavedFaceBlueprint`
rows (`#[serde(default)]`). Existing saves without these rows load safely.

**Block ids kept for decode compatibility.** `BLUEPRINT_PAPER` (56), `LATENT_PRINT`
(158), and `CYANOTYPE_PRINT` (172) remain defined so legacy saves that contain full-cube
versions of these blocks continue to decode. Migration to face attachments is deferred
(Phase G, no live alpha saves yet). The block-based `latent_print::tick_develop` path
also still exists for legacy cubes.

---

## 3. Inventory System

### 3.1 Inventory Layout

The player inventory is an ECS component (`Inventory`) on the player entity, containing:

| Slot Group | Slot Count | Purpose |
|---|---:|---|
| Hotbar | 9 | Quick-access bar, always visible at screen bottom. |
| Main inventory | 27 (3 rows of 9) | General storage, accessible via inventory screen. |
| Armour slots | 4 | Helmet, chestplate, leggings, boots. |
| Offhand | 1 | Shield or secondary item. |
| Crafting input (player) | 4 (2x2 grid) | Small crafting, always available in inventory screen. |
| Crafting output | 1 | Result slot for the 2x2 grid. |

**Total player slots**: 9 + 27 + 4 + 1 + 4 + 1 = 46.

**HUD vital counts (#44)**: a compact bottom-right read-out shows `Inventory::free_slot_count()`
(empty slots across the 36-slot main+hotbar) and `Inventory::arrow_count()` (plain + Nostrich
arrows). Survival-only (Creative-suppressed like hearts/hunger); the free-slot figure warms
green→amber→red as the inventory fills. Per-tool durability is shown as a bar overlay on the
hotbar slot. AppleSkin-style saturation overlay is deferred until the food model tracks a
saturation value.

### 3.2 Item Stacking

| Category | Max Stack Size | Examples |
|---|---:|---|
| Standard blocks | 64 | Stone, dirt, planks, ores. |
| Special items | 16 | Ender pearls equivalent, signs, banners. |
| Tools and weapons | 1 | Pickaxes, swords, bows. Each is unique (durability tracking). |
| Armour | 1 | Each piece has independent durability. |

**Stack merging**: When picking up items or moving stacks, partial stacks of the same item type merge automatically. Shift-clicking in the inventory moves the full stack to the first available slot in the other section (hotbar to main inventory, or vice versa). Double-clicking an item gathers all matching items into one stack (up to max stack size).

### 3.2.1 Inventory QoL (#45)

The 2026-06-16 QoL pass (`docs/foundations/2026-06-15-inventory-qol.md`) added the most-installed
Minecraft inventory conveniences. The **load-bearing invariant** is count conservation — no path
creates or drops items (covered by unit tests).

- **Sort** — `inventory::sort_slots(slots, locked)` is a pure function: it merges partial stacks of
  the same item (filling to `max_stack`), orders by `Item::sort_key()` = `(category, id)` with larger
  stacks first (blocks → materials → tools → armour → plans), and re-lays the result into the
  **unlocked** positions only. `Inventory::sort_region(start, end)` applies it; the player Sort button
  tidies the main region (9..36) and the container Sort button (`chest_ui::sort_chest`) tidies the
  chest.
- **Locked slots** — `Inventory::{is_locked, toggle_lock}` + a per-slot `locked: [bool; 36]` flag.
  A locked slot keeps its item + index through a sort and is skipped by quick-stack and auto-refill.
  **Alt+click** a slot toggles its lock (amber corner marker). **Persisted** (Wave 3, 2026-06-19) via
  `WorldSave.locked_slots` (player 0's `Inventory::locked_indices`, appended LAST in the bincode wire
  order; reapplied on load with `Inventory::set_locked_from`). Split-screen players' + dedicated-server
  locks aren't persisted yet (single-player is the primary case).
- **Quick-stack family** (`chest_ui`) — **Sort / Dump matching / Restock / Take all** buttons on the
  chest dialog. "Dump matching" moves every (unlocked) player stack whose item the chest already
  holds; "Restock" pulls back kinds the player already carries; "Take all" empties the chest into the
  bag. All re-insert only the genuinely-unplaced remainder (no dupes).
- **Trash** (#28) — a Trash button in the inventory screen destroys the stack currently held on the
  cursor (`WindowClick::Trash`). It only ever bins the held item — you must pick something up
  first, so it can't nuke an inventory by accident.
- **Auto-refill** (`Inventory::auto_refill`, default on) — when a hotbar **stackable** is exhausted by
  placing, the next matching stack is pulled from the bag (main region first), skipping locked
  sources. Tools never stack, so they aren't auto-refilled. Setting toggle in the graphics/device
  settings panel.
- **Scroll-wheel transfer** (Wave 3, 2026-06-19) — with a container open, scroll over a slot to
  transfer between the two inventories: a chest slot scrolls items **out** to the bag, an inventory
  slot scrolls them **into** the chest (shift = whole stack). Capped at one move per frame so a flick
  never dumps a stack; `chest_ui::{withdraw_chest_slot, deposit_to_chest}` keep the count conserved.
- **Cursor-drag paint** (2026-06-19) — Mouse-Tweaks parity in `craft_ui`: **RMB-drag** across slots
  deposits one carried item into each (grid + inventory); **LMB-drag** gathers matching items into the
  cursor. Count-conserving (the cursor is the only buffer); each slot painted once per gesture via a
  visited set; `click_and_drag` sense so a drag-paint never also fires a single-slot click. Feel =
  playtest-tuned.

### 3.3 Item Data Model

Each item stack is represented as:

```rust
struct ItemStack {
    item_id: ItemId,        // Registry ID (namespaced: "genesis:stone")
    count: u8,              // 1-64 (or item-specific max)
    durability: Option<u16>, // Only for tools/armour
    metadata: ItemMetadata,  // NBT-like data (enchantments, custom name, etc.)
}
```

`ItemMetadata` is a key-value store (similar to Minecraft's NBT). It supports nested structures for complex items (written books, maps, etc.). The metadata is opaque to the inventory system itself — item behaviours are handled by the item's registered handler.

### 3.4 ECS Integration

The `Inventory` component lives on the player entity:

```rust
#[derive(Component)]
struct Inventory {
    hotbar: [Option<ItemStack>; 9],
    main: [Option<ItemStack>; 27],
    armour: [Option<ItemStack>; 4],          // BUILT elsewhere — as-built it lives on PlayerSlot.armour_slots (see below), not in Inventory
    offhand: Option<ItemStack>,              // DEFERRED — offhand mechanic not yet wired
    crafting_input: [Option<ItemStack>; 4],  // 2x2 — DEFERRED — handled via separate crafting-table UI today
    crafting_output: Option<ItemStack>,      // DEFERRED — see above
    selected_slot: u8,  // 0-8, active hotbar slot
}
```

**Current ship state (as of Wave 25):** the engine implements the **hotbar + main = 36 slots** flat (`game/engine/src/inventory.rs:11`). The `armour`, `offhand`, and `crafting_*` fields above are spec'd for the destination shape but not yet wired — armour is **built** but stored as `PlayerSlot.armour_slots: [Option<ArmourItem>; 4]` (equip UI in `craft_ui.rs`, HUD readout, damage reduction, saved), not inside `Inventory`; offhand is still unbuilt; offhand and crafting-input/output land when their respective UI surfaces do. Save-format compatibility is preserved by keeping the slot count at 36 until those fields go live.

The inventory system is a dedicated ECS system that:
- Processes `InventoryAction` events (move, swap, drop, pick up).
- Validates all actions server-side.
- Sends delta updates to the client (only changed slots, not the full inventory).

### 3.5 Inventory Synchronisation

> **As built (C1, 2026-10-07).** None of the design below exists yet: there is no join-time inventory send, `SlotUpdate` or resync. A joined client's inventory is its own; the server learns only the changes it decides itself and grants them by `InventoryGrant` (pickups, the joiner's break drops, interaction products), and keeps a drifting **shadow** of each joiner's inventory from those plus the consumes it accepts (plain placements, interaction outcomes), with a log-only possession check on placements. Since C2b (v74) the shadow also mirrors a joiner's crafting and Q-drops, and a grant that doesn't fit it spills as a real item. What is server-computed and what is still client-trusted: Spec 04 §4.2e.

- On join, the server sends the full inventory state to the client.
- During gameplay, only **delta updates** are sent: `SlotUpdate { slot_id, new_contents }`.
- The client predicts inventory changes (e.g., placing a block decrements the count) and applies them locally. Server confirmations reconcile.
- If the server and client inventory diverge (detected via sequence numbers on inventory actions), the server sends a full inventory resync.

### 3.6 Drag-and-Drop UI

The inventory UI supports standard drag-and-drop interactions:

- **Left-click**: Pick up full stack / place full stack.
- **Right-click**: Pick up half stack (rounded up) / place one item.
- **Shift-click**: Move full stack to the other inventory section (hotbar <-> main).
- **Number keys (1-9)**: Swap hovered item with the corresponding hotbar slot.
- **Q key**: Drop one item — from the cursor-held stack when the inventory UI is open, or from the active hotbar slot during gameplay. The dropped stack spawns as an `ItemEntity` in front of the player's eye with a small forward + upward velocity. Tools and other unstackable items drop the whole stack regardless. (Implemented via `Inventory::take_one_from_hotbar` + `entity::q_drop_launch` + `entity::spawn_thrown_item`; Q is wired in `InputState::key_pressed` and surfaced through `PlayerIntent.drop_item`.) *(As built: only the hotbar drop exists — the panel ignores Q while open. Q is edge-triggered, one drop per press. A joiner's drop is spawned by the server as a real item, at most one every `DROP_INTERVAL_TICKS` = 4, C2b — §6.4.1, Spec 04 §4.2f.)*

  **Pickup delay**: A Q-dropped item has `ITEM_DROP_PICKUP_DELAY_TICKS = 30` (1.5 s @ 20 TPS) before its dropper can pick it up again, so the player can't instantly hoover their own throw. The `ItemEntity.dropper` field carries the dropper's player index and is honoured per-player by `tick_item_pickups` — **other players bypass the delay and pick it up immediately**. Natural drops (mining, mob death) use `dropper = None` + the shorter `ITEM_PICKUP_DELAY_TICKS = 10` (0.5 s), which blocks every player during the window. The slice signature is `&mut [(real_player_index, position, &mut Inventory)]` so the real index — not the alive-player slot — is what gets matched against `dropper`.
- **Ctrl+Q**: Drop the entire held stack. (Not yet implemented.)

**Inventory-open input gate**: while a player's crafting/inventory UI is open, their gameplay intent (movement, look, jump, break, place, drop, flight toggle) is zeroed in `game_loop` before physics ticks. Without this gate, WASD reads `keys_held` unconditionally and the world visible through the 82%-opaque overlay scrolls behind the panel — a held movement key can walk the player off a cliff or into water while they're menu-clicking. `toggle_inventory`, `pause`, and `toggle_debug` are deliberately preserved so E, Esc, and F3 still work to close the panel / pause / toggle debug.

**Rendering pitfalls (egui 0.31)** — guard against two layout traps that have caused the crafting UI to disappear entirely:

1. **Full-screen overlay must use `ctx.layer_painter`, not `Area + ui.painter()`.** An `egui::Area` with no widgets has zero layout size; `ui.painter()` is clipped to that zero rect, so painting a 1280×720 `rect_filled` produces no pixels. The current implementation paints the overlay via `ctx.layer_painter(LayerId::new(Order::Background, ...))` which is unclipped.

2. **Pin Area widths with both `set_min_width` AND `set_max_width`.** `set_min_width` alone is a *post-layout* constraint — it only affects the Area's reported size after children finish rendering. While children are laying out, `ui.available_width()` still returns the parent Area's *current* (essentially zero) width, so `(ui.available_width() - 300.0) / 2.0` is a large negative space and `horizontal` children collapse onto one another. Result: the entire panel renders into a zero-pixel column and is invisible. Use `ui.set_min_width(W); ui.set_max_width(W);` together so children layout against the final width on the first pass.

3. **Cursor-following decorations must use `ctx.layer_painter`, never an `Area`.** The cursor-held item icon is drawn at the pointer position and *moves with* every click. If it's a `egui::Area` it claims hit-test space even with `interactable(false)`, which silently swallows the next click instead of letting it reach the destination slot — pick-up works, place-down does nothing. Paint directly via `ctx.layer_painter(LayerId::new(Order::Tooltip, ...))` so there's no widget and no hit-test region.
- **Double-click**: Gather all matching items from the inventory into the cursor stack.

**Inventory clicks follow Minecraft left/right semantics (2026-06-07).** Across **every** slot in the inventory/crafting screen — the 36 inventory+hotbar slots, the 3×3 table grid, and the inventory 2×2 grid — **left-click works on the whole stack, right-click works on a single item**. This is the Minecraft Java convention, so a veteran's muscle memory carries over (Axolittle's call after the buttons were initially described the other way round). Concretely, with the **cursor holding** items:

| | Empty target slot | Same item (room) | Same item (full) | Different item |
|---|---|---|---|---|
| **Left** | place the whole stack | merge whole, remainder stays on cursor | no-op | swap |
| **Right** | place one | add one, cursor keeps the rest | no-op | swap |

…and with an **empty cursor** clicking a filled slot: **left** picks up the whole stack, **right** picks up the ceil-half (e.g. 5 → 3 to cursor, 2 left behind). All paths are lossless (swaps exchange full stacks; the cursor never strands items). The merge/cap respects `Item::max_stack()` (64 for blocks/materials; tools/plans/armour are 1 and never stack).

This means **crafting-grid cells can hold more than one item** — fill the grid with stacks (left-click to dump a whole stack into a cell, or right-click to place one) and pull many results out in a row (batch crafting), instead of re-laying the recipe after every craft. Axolittle flagged "you can't stack blocks in the crafting table or inventory grid" (2026-06-07) — left/right stacking is the fix. The button is plumbed via `ClickTarget::{GridSlot, InventorySlot}(.., right: bool)` (egui `secondary_clicked()` → `right=true`) and turned into a `WindowClick::{Grid, Slot}` for `window::apply` (below).

**Crafting-panel interactions are lossless (engine audit 2026-06-04, A; revised 2026-06-07).** Two paths used to delete items: (1) dropping a multi-count cursor onto an *occupied* grid cell placed one and discarded `count-1` — now left/right merge-or-swap (see the table above) never discards anything; (2) closing the panel returned grid+cursor items to the inventory but dropped whatever didn't fit — now `CraftingUi::close` keeps un-returnable items in the grid/cursor and **stays open** (returns `false`), so the player frees space rather than losing items. The E-press close surfaces a "make room" toast on a blocked close; the pause path simply leaves the panel open under the menu.

**One window model (C3a-1, 2026-10-07).** Every item move the inventory screen makes is one pure transition, `window::apply(&mut WindowMut, &WindowClick, &ClickCtx) -> ClickResult` in `window.rs`, over a borrowed view of the state that screen touches: the 36 slots (with locks and `auto_refill`), the four armour slots, the cursor, the crafting grid, and an open container's slots (`None` until C3b). `ClickCtx` carries what the view can't: the station (the player's 2×2, or the 3×3 of the crafting table at a cell — `Station::Table { cell }` since C3a-2a — which bounds a grid click and what the result click matches against), the acting body's eye, the block the acting side's world holds at the table's cell, and creative. `ClickResult` reports what the caller does outside the window: `Crafted(stack)` (challenge events, first-craft hints), `Binned(stack)` (the trash toast), `NeedsTable` (its toast) or `Refused`. The transitions (`WindowClick`): `Slot`, `Grid`, `Armour`, `Result`, `Trash`, `DragDistribute`/`DragGather` (a slot list over inventory and grid cells; the screen sends one slot per paint), `Sort` (the bag 9..36), `ToggleLock`, `Autofill` (recipe-book "Fill from bag", carrying the card's example grid) and `Close` (grid and cursor back to the inventory). `craft_ui.rs` keeps all egui drawing, hover and open state and maps a `ClickTarget` to a `WindowClick` (`ClickTarget::window_click`); `CraftingUi::apply_click` builds the view from the player's inventory and armour plus the screen's grid and cursor (and the context from the player's eye and its world), applies it and refreshes the result shown. Every rule is unit-tested on plain values in `window.rs`. The same rules are what the server runs on its copy of a joiner's window (C3a-2a, below; `docs/foundations/2026-10-07-c3-server-owned-inventory.md` §2). A slot, lock or drag index past the end is refused (the screen never sends one; `Inventory::set_slot` would have dropped the cursor).

**C2b-verify fixes in the window rules (2026-10-07)** — the server will run them too:

- **The 2×2 stays 2×2 (M3).** At the player's grid an item in row or column 2 crafts nothing (`window::recipe_output`), and "Fill from bag" refuses a recipe bigger than 2×2 with the toast "Needs a crafting table" before anything moves (`ClickResult::NeedsTable`); a smaller one is laid in the 2×2's corner. Fill from bag used to lay a 3×3 recipe into the 2×2's hidden cells, so a player crafted a table recipe with no table.
- **A non-ingredient blocks a craft (L1).** A tool, armour piece or Plan anywhere in the grid makes the result empty. The matcher used to read it as an empty cell, and the craft destroyed it with the ingredients.
- **The result click re-matches the grid (L2).** `WindowClick::Result` crafts what `recipe_output` matches at the click, never a cached result, and a refused close refreshes the result shown. A close that returned only part of the grid used to keep offering what the grid had held (three planks crafting a table).
- **A table's screen closes when the table goes (L3).** Each frame, an open table screen whose cell no longer holds a crafting table, or is beyond the body's block reach (`window::table_in_reach`: `item_actions::cell_in_reach`, the rule the server judges a joiner's table craft by, measured from the eye to the cell's centre, no Reach Claw bonus), closes as E would and returns the grid; a full inventory keeps it open, as on any close. Minecraft does the same.

**The table's reach is part of the rule (C3a-2a, 2026-10-08).** `ClickCtx::table_present`: the player's 2×2 always; a table only while `window::table_in_reach` holds for the acting body. The result click at a table out of reach (or broken) is `Refused` and `Autofill` there is `NeedsTable`, both before anything moves; grid clicks still work, so a screen whose forced close couldn't return everything can be emptied by hand but can no longer craft at, or lay a recipe into, a table that's gone. A table opens only while `table_in_reach` holds too (right-click on a crafting table, in every mode): a Reach Claw's longer ray (7 blocks against the rule's 6.37) used to open a table whose screen L3 then closed. A close that returns everything clears the screen's table (`CraftingUi.table = None`), as the server's station resets. A drag paints at most `window::MAX_DRAG_SLOTS` = 45 slots (36 + 9); a longer list is refused. `window::wear_armour` is the one armour-wear rule (a player's own hits through `PlayerSlot::wear_armour`, a joiner's `ArmourWorn`, and the server's copy of a joiner's armour).

**A joiner's window is mirrored on the server, click for click (C3a-2a, protocol v75, log-only).** Every window transition a joined client applies — each `WindowClick`, the close included, opening its inventory (`OpenPlayer`) or a table (`OpenTable { cell }`), and its auto-refill setting (`SetAutoRefill`, at join and whenever it changes) — is logged by `CraftingUi` (`window_ops::OpLog`) with `window::digest` of the window after it and sent as a `WindowOp` at the start of the next tick, before that tick's input. The server applies the same `window::apply` to its copy of that joiner's window (the shadow's 36 slots plus `ServerPlayer.armour`, `cursor`, `craft_grid` and `station`), judging a table's reach from the server body in the server's world, behind the client's waiting edits and in order, and tallies digest mismatches (`PossessionTally::window_mismatch`; a creative joiner is mirrored, not tallied). Nothing is refused or sent back. The result click is the craft: `ItemAction::Craft` is unused since v75. Single-player and a host's own seats send nothing. Spec 04 §4.2g.

All drag-and-drop actions generate `InventoryAction` events that are sent to the server. The client applies them optimistically. The server validates (e.g., cannot place a helmet in the boot slot; cannot exceed max stack size) and either confirms or reverts. *(As built, C3a-2a: they are `WindowOp`s, applied on the server by the client's own rule, so the two windows agree by construction; the server neither confirms nor reverts yet — a mismatch is only tallied. Refusal and resync come with C3d.)*

### 3.7 Container Inventories

Non-player inventories (chests, furnaces, hoppers) are ECS components on their respective block entities. When a player opens a container, the server streams the container's current contents to the client and locks it for that player (or allows shared access with contention handling).

**Container slots cap at the item's `max_stack` (64), never `u8::MAX`.** Chest `try_insert` and the furnace input/fuel placers used to stack to 255 — a cap violation that (a) let furnace placement consume-then-`saturating_add`, silently eating items the slot couldn't hold, and (b) produced >64 stacks which then overflowed the `u8` add in `click_inventory_slot` (debug panic / release wrap) once withdrawn onto a normal inventory slot. All container stacking now caps at `max_stack` and spills the remainder into further slots; the inventory-merge path uses `saturating_sub`/`min` so an over-max legacy slot is a safe no-op. Regression tests: `chest::try_insert_caps_slot_at_max_stack_not_255`, `furnace::furnace_input_caps_at_max_and_does_not_over_consume`, `craft_ui::click_inventory_slot_does_not_overflow_on_overfull_slot`.

**`add_item` is non-atomic — it returns the unplaced remainder, and every transfer/recovery path MUST honour it.** `Inventory::add_item` places what fits and hands back an `Option<ItemStack>` carrying the leftover (`None` = all placed). This is a hard contract, not a convenience: a `bool` cannot distinguish "all placed" from "some placed", and that ambiguity was a **live item-duplication bug** (2026-06-04 engine audit, finding A#1). The chest-withdraw recovery treated the old `false` return as "nothing landed" and re-inserted the *whole* withdrawn stack into the chest while part had already reached the inventory — duplicating exactly the amount that fit. The same class of bug lived in the ground-item pickup path (a partially-picked stack stayed whole on the ground, re-granting itself next magnet tick). **Rule:** any code that withdraws/transfers a stack and then recovers on failure must re-insert *only the remainder* `add_item` returns, never the original stack. Container withdrawal goes through `chest_ui::withdraw_chest_slot`; ground pickup updates `ItemEntity.stack` to the remainder. Regression tests: `chest_ui::withdraw_partial_fit_conserves_total_no_dupe`, `entity::partial_pickup_leaves_only_remainder_on_ground_no_dupe`.

Container types and sizes:

| Container | Slots | Notes |
|---|---:|---|
| Chest | 27 | Single chest. |
| Double chest | 54 | Two adjacent chests. |
| Furnace | 3 | Input, fuel, output. |
| Crafting table | 10 | 3x3 input + 1 output. |
| Hopper | 5 | Transfer pipe. |
| Dispenser / Dropper | 9 | Redstone-activated. |

---

### 3.8 Special Materials — Satori

**Satori** is the in-world physical manifestation of Bitcoin in AxeNStax — a luminous orange gem that drops from pure deepslate at depth. It is a regular `Item` from the inventory system's perspective (stacks to 64; tradeable in-game) but has unique drop rules + economic significance distinguishing it from every other material.

**Etymology.** *Satori* (悟り) is the Japanese Zen term for sudden awakening — the moment understanding crystallises. It fits the gameplay beat where a player strikes deep deepslate and the first orange flash appears. The name is also a phonetic nod to Satoshi Nakamoto without being a literal naming-after — the gem is Bitcoin's in-world face, not a worship object. Pronunciation: *sah-toh-ree*. Singular and plural are both "Satori" (mass noun); use "Satori gem(s)" only when grammar forces a count.

**Status across server modes:**

| Server mode | Gem behaviour |
|---|---|
| **Sandbox / non-Bitcoin** | Gems drop as collectible items only. No real-money tie. Usable for crafting (tools / armour) and in-game trading. |
| **Bitcoin-enabled (faucet, pay-to-play, tournament)** | Each gem drop is matched 1:1 with a sat in the player's internal balance. Player may craft, trade, or *withdraw to Lightning* (subject to the parental flag + server policy in `docs/spec/06-bitcoin-integration.md §10.3 / §11`). |

The gem itself is **universal** — possession, in-game trading, and crafting are unrestricted. Only **Lightning withdrawal** is subject to the parental flag / age / jurisdiction controls; see Spec 6.

**Drop trigger — high-level (full algorithm in Spec 6 §2.2c):**

A gem drops when a player breaks a **pure-deepslate block** that the Proof-of-Play hash determines to be part of a deterministic vein. Conditions:

- Block type **must be pure deepslate**. Deepslate variants (deepslate-coal-ore, deepslate-iron-ore, deepslate-diamond-ore, polished deepslate, etc.) are **not eligible**. The block has one identity; ore-bearing deepslate cannot also bear a gem.
- Depth **must be at or below** `Y_dp - 21`, where `Y_dp` is the Y level at which pure deepslate begins in world generation (server-configurable; default `Y_dp = 0` per Minecraft baseline, so default gate is `y <= -21`; reduced further into any custom deepslate layer the operator configures).
- The vein-generation hash must produce a positive result for this block — deterministic from `server_secret + world_seed + epoch + position`.
- The block's **exposure age** (time since the block gained an air-facing neighbour) must not exceed the decay threshold (default: 1 in-game day, server-configurable).
- Player's **pickaxe tier must be Diamond or higher** — under-tier pickaxes still break the deepslate (the block still mines normally) but **no gem drops**. Progression-gated reward.

**Vein behaviour:**

- Veins are **spatially coherent**, not per-block independent. Adjacent eligible blocks reached by the same vein origin all drop gems. Find one, dig around → find more. Like real-world ore.
- Vein origins are **rarer near the top** of pure deepslate, more common with depth. Reward for committing to depth.
- Veins have a **finite extent** (default radius ~8 blocks from origin, with probabilistic decay). No single vein is a runaway.
- **Find-one-find-more is natural**: when you hit a gem, you've hit one block in a vein; the rest are adjacent.

**Exposure decay (oxidisation):**

- A gem block's drop probability decays once the block is **exposed to air** (gains at least one air-facing neighbour, e.g., when the player mines an adjacent block or a natural cave intersects it).
- **Default decay duration**: 1 in-game day = 20,000 ticks at 1× world-time speed. At current alpha `world_time_step = 4` this is ~5 real minutes; at the post-alpha 1× default (per CLAUDE.md technical-debt note) it is ~20 real minutes.
- **Default decay curve**: linear — 0% decay at exposure, 100% decay (zero gem chance) at full duration.
- Server operators can override both duration and curve via the bitcoin-economy config (see Spec 6 §6 — server economy modes, TBD).
- Practical effect: mine into deepslate quickly to keep the gem; cave-spelunk past gems and the value oxidises away. Encourages real mining commitment over opportunistic cave-walking.

**Crafting recipes — tools and armour:**

The gem can be combined into tools and armour at the top tier (above diamond — see §5.1 and §6.6):

| Recipe | Inputs | Output |
|---|---|---|
| Satori Pickaxe | 3 Satori + 2 sticks (pickaxe pattern) | 1 Satori Pickaxe (durability 2031, mining speed 9.0, level 4) |
| Satori Sword | 2 Satori + 1 stick (sword pattern) | 1 Satori Sword |
| Satori Axe | 3 Satori + 2 sticks (axe pattern) | 1 Satori Axe |
| Satori Shovel | 1 Satori + 2 sticks (shovel pattern) | 1 Satori Shovel |
| Satori Hoe | 2 Satori + 2 sticks (hoe pattern) | 1 Satori Hoe |
| Satori Helmet | 5 Satori (helmet pattern) | 1 Satori Helmet |
| Satori Chestplate | 8 Satori (chestplate pattern) | 1 Satori Chestplate |
| Satori Leggings | 7 Satori (leggings pattern) | 1 Satori Leggings |
| Satori Boots | 4 Satori (boots pattern) | 1 Satori Boots |
| Block of Satori | 9 Satori (3×3 fill) | 1 Block of Satori (round-trip recipe, same as diamond storage) |

The full armour set provides **20 total armour points** with **12 toughness** — same armour points as diamond, more toughness (greater protection against heavy hits per Spec 6 §6 damage formula). Notably: in a future meteor-survival mechanic (Axolittle's must-have from `docs/research/2026-03-06-blocks-qa-round2-gameplay.md`), the gem-tier armour set would be the survival threshold.

**On-chain note:** the gem ≡ sats equivalence in Bitcoin-enabled servers is a 1:1 mapping at *drop time*. The internal balance is server-tracked; the in-inventory gem is the visible artefact. Players can craft with their gems (consumes the inventory gem AND deducts the corresponding sats from the internal balance), trade in-game (transfers both), or withdraw the sat-equivalent of their inventory gems to Lightning (consumes inventory gems, debits internal balance, queues Lightning payout). The unified-token framing is documented in Spec 6 §6 (server economy modes, TBD).

**X-ray defence:** vein-membership bitmasks are computed server-side at chunk-load time from `server_secret`; never sent to clients. Plus the chunk-stream obfuscation per `docs/spec/08-security-anti-cheat.md §5.2.2` hides buried deepslate just like every other buried block. The gem mechanic preserves the architectural anti-X-ray property by construction.

### 3.9 Farming — Tier 1

> **Status (2026-05-18):** Tier 1 DELIVERED on main, foundation `docs/foundations/2026-05-14-farming-system.md` Phases 2-9. Tier 2 (animal-drawn plough + breeding + fences) is sketched in the foundation doc and needs its own spec post-T1 playtest.

Tier 1 farming is the Minecraft-baseline loop: till → plant → grow → harvest → bake bread → eat. Three crop families to start (wheat, carrot, potato — the canonical Minecraft trio); growth is time-based with a water-adjacency boost; harvest resets to tilled soil so the player can immediately replant.

**Blocks (ids 30, 31..=42):**

| Block | ID | Purpose |
|---|---:|---|
| Tilled Soil | 30 | Hoe-tilled dirt or grass. Solid, opaque; top face renders the furrow texture, sides + bottom render as plain dirt so a partly-tilled patch blends cleanly. The substrate crops plant into. |
| Wheat / Carrot / Potato stages 0-3 | 31..=42 | One block ID per stage (4 stages × 3 crops). All transparent + non-solid + no-gravity (walk-through). Visual progression encodes growth. |

**Tool — Hoe (5 tiers):** wood, stone, iron, diamond, satori. Recipe pattern: 2 head pieces top-left + 2 sticks vertical (and mirrored). Attack damage 1.0 regardless of tier (utility tool, not a weapon). Per-tier durability matches the pickaxe ladder: 59 / 131 / 250 / 1561 / 2031.

**Items added:**

| Item | Food value (HP) | Role |
|---|---:|---|
| Wheat Seeds | — | Plant on tilled soil. Drop from breaking grass (existing) + 1-3 per mature wheat harvest. |
| Wheat | — (bake first) | Harvest output of mature wheat. Craft into bread. |
| Bread | **5** | 3 wheat horizontal → 1 bread. Staple food, kid-friendly survival arc. |
| Carrot | 3 | Dual-purpose: plant directly on tilled soil and/or eat raw. |
| Potato | 1 | Dual-purpose. (Baked-potato variant is post-Tier 1 polish.) |

**Till action:** right-click dirt or grass top face with any hoe → tilled soil. Side-face clicks fall through. Hoe loses 1 durability per till.

**Plant action:** right-click tilled-soil top face with wheat seeds / carrot / potato → stage-0 crop in the air block above. Consumes 1 seed. Requires the air block above to be empty.

**Growth tick:** every 200 ticks (10s @ 20 TPS) a crop advances one stage. Sprout → mature ≈ 30 s. Water within 4 blocks horizontally (or one block below) halves the interval to 100 ticks. Mature crops (`*_STAGE_3`) never advance further. The growth tick is a pure free function (`growth::advance_crops`) returning a `Vec<BlockChange>` so the multiplayer broadcast path picks it up identically to falling-block changes.

**Harvest:** breaking a mature crop drops:

- Wheat: 1 wheat + 1-3 wheat seeds (random per position+tick).
- Carrot: 1-4 carrots.
- Potato: 1-4 potatoes.

Mature crops break to **tilled soil**, not air — so the player can immediately re-plant without re-tilling. Immature crops (stages 0-2) drop nothing and break to air.

**Light gate:** **active as of Spec 30 (2026-05-21).** Crops require `world.effective_light_at(pos) >= 9` to advance. Sky-light is 15 at noon and decays to 4 at midnight, with a −4 offset baked into `effective_light_at`, so a sky-lit crop's effective light stays well above 9 round the clock and grows naturally outdoors. Indoors/under-roof crops need a torch (block-light 14) within range to keep advancing.

**No Bitcoin economy hook:** Tier 1 farming does not interact with Proof-of-Play. You don't "mine" dirt with a hoe in a way that should yield Bitcoin. Tier 2's plough-on-tilled-soil is a future open question.

**Papyrus Reed — Spec 23 (wild plant; not a tilled-soil crop).** Foundation A of Build Schematics. 4-stage water-adjacent plant (`PAPYRUS_STAGE_0..3` = ids 52..=55). Placed by right-clicking `dirt`, `grass`, or `sand` with a `PapyrusReed` material in hand, provided `block::WATER` is within 1 block (4-orthogonal neighbours + cell-below). Same 200-tick-per-stage growth as the field crops, with the water-adjacency fast-fire kicking in by default (papyrus is water-adjacent at placement). Harvest is **sugarcane-style**: mining a mature stage drops 1-2 reeds and replaces with `PAPYRUS_STAGE_0` (the root keeps growing — distinct from the field crops which break to tilled soil and require explicit replant). Recipe: `3 reeds horizontal → 3 Papyrus Sheets` (first paper material; Plan Tile in Spec 24 consumes it via the new `crafting::is_paperish_slot` predicate). See foundation `docs/foundations/2026-05-19-papyrus-reed.md`.

### 3.10 Campfire

> **Status (2026-05-18):** Wave 27 DELIVERED on main, foundation `docs/foundations/2026-05-18-campfire.md` Phases 2-8. Phase 9 (Axolittle playtest) is the remaining gate.

The Campfire is Tier 1's cooking workstation. Pre-furnace (furnace lands in T1.5 with the broader workstation framework); wood-only fuel; doubles as a light source. Replaces the earlier `raw_meat + coal vertical` crafting-table hack — meat now cooks at a real workstation, not via a one-shot recipe.

**Blocks (ids 43, 44):**

| Block | ID | Purpose |
|---|---:|---|
| Campfire | 43 | Lit. Cooks meat. Emits light (level 14). |
| Campfire (unlit) | 44 | Inert. Right-click with stick or flint-and-steel to ignite (Phase 6). |

**Recipe:** 3 sticks across the top, 3 logs across the middle, 3 sticks across the bottom. Output is **unlit**.

**Fuel ladder** (burn time per item, at 20 TPS):

| Fuel | Burn time | Role |
|---|---:|---|
| 1 oak leaf | 1 s | Kindling — leaves drop on direct break + at 25% on natural decay (Phase 2 added the drop) |
| 1 stick | 2 s | Kindling+ |
| 1 oak plank | 12 s | Light fuel |
| 1 oak log | 60 s | Standard fuel |
| 1 coal | 240 s | Premium endurance fuel (4× a log) |

**Cooking**: 10 s per meat at full burn. Up to 4 meats cooking simultaneously (one per top-face quadrant). Mature cooked items wait in the slot until the player picks them up (right-click empty hand).

**Block-entity state**: per-campfire `(fuel_ticks, [CookSlot; 4])` lives in `World::block_entities`. Saves/loads with the world. T1.5's full block-entity framework will absorb this in a one-line refactor.

**Fire-starting** (added as a deliberate beat, separate from fueling — see §5.5):

| Method | Trigger | Speed | Reliability |
|---|---|---:|---|
| Friction | Right-click hold with a stick on an unlit **fuelled** campfire | 5 s hold | 70% success per attempt. Stick consumed regardless. |
| Flint and Steel | Right-click on an unlit campfire | Instant | 100%. Tool durability -1 per use (65 uses total). Refuses on unfuelled with toast. |

**Ignition requires fuel (both methods).** Lighting an unfuelled campfire would extinguish on the very next fuel-burn tick (it's lit only while `fuel_ticks > 0`), so neither method lights a fuel-less fire. Striking an unfuelled campfire with a stick shows *"Add fuel first (wood), then strike to light."* — it does **not** start the 5-s friction hold and does **not** consume the stick (so the stick can't be silently spent as 2-s fuel by the fuel-add fall-through). Flint-and-steel behaves the same with its own toast. Gate helper: `campfire::can_ignite(Option<&CampfireData>)` — true iff `fuel_ticks > 0`. (2026-06-22 bugfix: the friction path had drifted from this spec and would light empty campfires that snuffed out instantly — "lit it, stayed cold". Now realigned.)

**Hover panel states** (`campfire_ui::campfire_hover_text`, shown when the crosshair targets a campfire within reach). Four states walk the player through *add fuel → light → cook*:
- **Lit** — fuel seconds left, heat radius, smoke, + per-slot cooking lines.
- **Ready to light** *(unlit but fuelled)* — shows the fuel and *"Strike with a stick, or use flint & steel"*. (Previously this state read "Cold" and hid the fuel, so players couldn't tell their fuel went in.)
- **Smouldering** — warm-window countdown + *"Drop fuel to relight"*.
- **Cold** *(no fuel)* — *"Add fuel (wood), then light…"*.
- **Cooking lines** appear in every state (so meat on an unlit fire is never invisible): `Cooking: Raw Beef — 45%` while cooking, `Ready: Cooked Beef (empty hand to take)` once mature. Placing raw meat also toasts *"On the fire — cooking…"* (lit) or *"…light it to start cooking"* (unlit).

**In multiplayer (FU3, 2026-10-07).** The world that holds the campfire runs its rules: the fuel burn, cooking and smoke-pillar sweep runs in the single-player or LAN-host client (on the world a host lends its server), never in a joiner (a joiner's sweep, on its own copy of a campfire's state, pushed lit/unlit flips and pillar cells as its own edits). When a joiner breaks, lights or puts out a campfire it sends the one block edit; the server runs `campfire::on_block_edit` on it — the rule the client's own break and light arms run (`cleanup_campfire`, `smoke_on_light`) — clearing a broken fire's smoke and spilling what was cooking, raising a smoky fire's pillar from the server's own campfire state, and broadcasting the cells. (The client used to send the campfire plus up to six pillar cells; the server's 4-edit budget refused three, and the refused smoke floated in the shared world for good, untargetable.) **A joiner's own pillar clear is skipped too (FU4b, L5):** a joined client whose edits reach the server breaks a campfire with `campfire::cleanup_campfire_keep_smoke` (the entry removed and the cook slots spilled, the smoke cells left), so a refused break (reach, plot) cannot leave a restored fire with no smoke on its screen; the server's derived clear arrives as block changes. Open: a joiner's fuel, meat and cooked-pickup clicks change only its own copy of the campfire (the server's smoke state may differ from what the joiner put in), and a dedicated server runs no campfire sweep at all, so its fires never burn down (tick parity, D4).

**Block drops** (Wave 27 additions):

- **Gravel → Flint at 15%** (per break). Deterministic per (tick × position). 85% chance still drops gravel itself.
- **Oak Leaves → Oak Leaves at 100%** on direct break. (Default `mine_drop` already returned the block; behaviour confirmed.) Leaf decay still drops nothing in the current code — a 25% decay-drop is in scope for a future polish phase but did not ship in Wave 27.

### 3.11 Items Added by Campfire (Wave 27)

| Item | Source | Use |
|---|---|---|
| **Flint** (material) | 15% drop from gravel | Recipe ingredient for Flint and Steel |
| **Flint and Steel** (tool) | 1 Flint + 1 Iron Ingot (1×2 vertical) | Instant campfire ignition; 65 durability |
| **Campfire** (block, unlit) | 3 sticks + 3 logs + 3 sticks (3×3) | Place + fuel + ignite to cook |

### 3.12 Campfire Extensions (Wave 28 — Spec 18)

Three independent additions to the campfire:

**Smoke signal pillar.** Burning oak leaves grants 60 ticks (~3 s) of smoke per leaf added. While the campfire is lit AND smoke_ticks > 0, a 6-block-tall pillar of `CAMPFIRE_SMOKE` (id 45) is rendered above the campfire — a non-solid, transparent, grey block visible from a long way off. Use case: a "I'm here!" signal for friends. The pillar is managed entirely by the campfire tick; on `campfire::cleanup_campfire` (called when the campfire block is broken or destroyed), the pillar is cleared.

**Corn — new crop.** Single-block-tall stereotypical crop matching the Wheat/Carrot/Potato pattern. 4 growth stages (`CORN_STAGE_0..3` = ids 46..=49). Planted from `CornSeeds` on tilled soil; mature drops 1 `Corn` + 1-3 `CornSeeds` (wheat-style separate-seed pattern so the player can sustain a plot without grinding for seeds). Same 200-tick-per-stage timer + water-adjacency boost as the other Tier 1 crops.

**Bakeable vegetables at the campfire.** `Potato → BakedPotato`, `Carrot → BakedCarrot`, `Corn → BakedCorn` (display "Corn on the Cob"). Same 10-s cook time per slot as cooked meat. Food values: Raw Corn 2.0, Baked Potato 5.0, Baked Carrot 4.0, Baked Corn 5.0. Closes the loop between Wave 26 farming (raw harvests) and Wave 27 cooking (campfire as the workstation).

**Mob behaviour — heat-aware beacon (Spec 18).** Replaces the lazy "fire deters mobs" Minecraft trope. Mobs see smoke + glow from a lit campfire and walk toward it from up to 12 blocks away. At the heat boundary (radius scaled by fuel: 2 blocks at 200 fuel → 6 blocks at 4800 fuel, log2 curve clamped) they hold position for ~10 s, then lose interest. Hostile-mob priority: a player closer than the campfire while a hostile mob is investigating switches the mob to Chase. The fire is a beacon — it tells the world where you are, but heat holds them at a distance.

**Items added (Wave 28):**

| Item | Source | Use |
|---|---|---|
| **Corn Seeds** (material) | Drops from mature corn (1-3 per harvest) | Plant on tilled soil → corn |
| **Corn** (material) | Drops from mature corn (1 per harvest) | Eat (2.0 HP) or bake at campfire |
| **Baked Potato** (material) | Potato in campfire cook slot | Eat (5.0 HP) |
| **Baked Carrot** (material) | Carrot in campfire cook slot | Eat (4.0 HP) |
| **Corn on the Cob** (BakedCorn material) | Corn in campfire cook slot | Eat (5.0 HP) |

---

## 4. Crafting System

### 3.13 Villages & Villagers (Spec 19)

Procedurally placed villages dot Plains and Forest biomes — roughly one
per `VILLAGE_GRID = 32`-chunk cell, biome-gated, deterministic on
`hash(world_seed, grid_x, grid_z)`. Each village has 3–6 cobblestone
houses (planks roof + bed + ceiling torch + inward-facing door), a
cobblestone well around a 1×1 water source, and a Spec 17 lit campfire
on a 5-block cross-shaped cobblestone hearth.

**Villager mob** (`MobType::Villager`): passive, 1.85 m bipedal, 20 HP,
brown-robe humanoid texture set. **Drops nothing on death** — Spec 19
anti-farming decision; reputation is the only currency for "is the
village happy with me." Right-clicking within 4 blocks opens the
dialogue overlay (Spec 19 phase 5).

**Anti-grief swing-gate**: the first swing on a villager within any
rolling 30-second window deals no damage and toasts "Careful — that's a
villager. Hit again to attack." Subsequent swings within the window
damage normally. Per-player (`PlayerSlot.last_villager_warn_tick`); the
kill-rep penalty (next section) has its own separate 30 s cooldown.

**Profession** (`villager::Profession`): one of `None`, `Farmer`, `Cook`,
`Carpenter`, `Blacksmith`, `Librarian`. Villagers spawn `None` and bind
to the first unclaimed workstation block within 3 blocks of where they
stand. Workstation table: `TilledSoil → Farmer`, `Campfire → Cook`,
`CraftingTable → Carpenter`. Blacksmith + Librarian register their
workstation hooks but wait on Furnace + Bookshelf blocks shipping.
Claim is sticky until the block is destroyed (`release_orphan_claims`).

**Quests** (`quest::Quest`) — three flavours: `Fetch { item, count }`,
`Make { item, count }`, `Kill { mob, count }`. Each profession has a
hand-tuned pool (4 entries for Farmer, 3 each for Cook + Carpenter,
2 each for Blacksmith + Librarian) with reward sats + reputation. The
dialogue offer is deterministic per `(tick, villager_id)` seed.

Dialogue modes (Spec 19 phase 7):
- `Offer` — Accept / Decline / Close.
- `InProgress` — Close only, with `(have/need)` progress on the offer.
- `ReadyToTurnIn` — Turn-in button consumes Fetch/Make resources,
  awards items + reputation + (sandbox-mocked) sats.

Decline = 5-minute per-villager cooldown so the kid can't spam-reject.

**Knight** (`MobType::Knight`, Spec 19 phase 8): sole village
defender (replaced the Iron Golem in HP-4/HP-6). Auto-spawns when a
village has ≥3 claimed villagers and ≥5 houses;
cap = `max(1, claimed_villagers / 10)`. AI state
`Guard { home_x, home_z }` patrols within 14 blocks of the anchor;
the guard-combat tick runs every tick, finds nearest hostile mob within
20 blocks, deals 7 damage per swing (20-tick cooldown) when within 1.8
blocks. Drops on death motivate raids on enemy villages (Tier 3 PvP)
without enabling grief-grinding (long respawn gates that).

**Reputation** (`reputation::Reputation`, Spec 19 phase 9): per-
player-per-village `i16` ledger clamped to `[-100, +100]`. Tiers:
Hostile ≤ -50, Wary [-50, -10], Neutral [-10, 10], Friendly [10, 50],
Beloved > 50. Quest completion adds the reward's `reputation` (5–20
typically); killing a villager subtracts 25 once per 30 s per player
(rage-loops cost rep once, not per kill). Natural decay drifts every
entry one point toward zero per in-game day. The tier's
`reward_multiplier` (Hostile 0× → Beloved 1.25×) is wired in but the
dialogue path doesn't apply it yet — future polish.

**Village Bell** (`block::VILLAGE_BELL`, id 50, Spec 19 phase 10):
craftable marker. Recipe is a 3-tall, 1-wide column: IronIngot top,
Stick middle, OakPlanks bottom → 1 Village Bell. Placing one registers
it in `World.village_bells`.

**Wandering Villager** (`MobType::WanderingVillager`): rare-spawn
drifter. `tick_wanderer_migration` spawns one near each unclaimed bell
every 30 s, steers nearby wanderers toward the closest bell, and on
arrival (< 2.5 blocks) despawns the wanderer and spawns a regular
Villager + registers the bell as a village anchor. Player-founded
villages enter the same code paths as procgen ones from there.

**Save format**: village state lives in `WorldSave` as
`village_anchors: Vec<SavedVillageAnchor>`,
`populated_villages: Vec<(i32, i32)>`, and `village_bells: Vec<[i32;3]>`.
Quest state + per-villager profession + reputation are intentionally
transient on alpha — workstation claims rebuild from world state on
load; the kid loses any in-flight quest on save/quit, which is fine for
playtest while we figure out whether persistence here adds value or
just complicates the kid's mental model.

### 3.14 Farming — Tier 1.5 (Processed Economy)

> **Status (2026-05-21):** Tier 1.5 DELIVERED on main, foundation
> `docs/foundations/2026-05-14-farming-tier-1.5-processed-economy.md`.
> Workstation framework + Furnace + Mill + Oven + Aging Rack ship as
> block-entities; all sourcing crops (sugarcane, sugar beet, beetroot,
> pumpkin, berries) + animal products (eggs, milk via Bucket) ship as
> `MaterialId` variants. Recipe ladder consumes `complexity_tier` +
> `food_value` + `trade_value` on `Item`. Phase 13 (Axolittle playtest)
> is the remaining gate.

Tier 1.5 turns the Tier-1 till→plant→harvest loop into an **economy**:
more sourcing inputs, processing workstations, multi-step recipe
chains, and a value ladder tagged with `complexity_tier`. Every food
item carries:

- `food_value(&self) -> Option<f32>` — HP per bite (None = not edible).
- `complexity_tier(&self) -> u8` — number of processing steps from raw
  inputs (0 = raw drop, 1 = single workstation, 2 = two-step recipe,
  etc.). Drives `trade_value` defaults.
- `trade_value(&self) -> Option<u64>` — internal sats equivalent.
  Default scales from `complexity_tier` (T0 = 1, T1 = 3, T2 = 8 …);
  recipes that should diverge from the default are hand-tuned in
  `item.rs::trade_value`. Server-operator overridable per Spec 6 §13.

**New sourcing crops** (Tier 0–1 raw drops):

| Crop | Stages | Source | Yields |
|---|---|---|---|
| Sugarcane | 4 | water-adjacent (Tier-1 papyrus-style placement) | 1–2 cane on mining mature; replants stage-0 |
| Sugar Beet | 4 | tilled-soil plant from `SugarBeetSeeds` | 1–2 sugar beet + 1–2 seeds |
| Beetroot | 4 | tilled-soil plant from `BeetrootSeeds` | 1–2 beetroot + 1–2 seeds (UK English: distinct cultivar from Sugar Beet) |
| Pumpkin | 4 | tilled-soil plant, vine-grows adjacent block | 1 pumpkin (block) on stem maturity |
| Berries | bush | shrub variant, hand-harvest | 1–3 berries on right-click |

**Animal products**:

- `Bucket` (item) — crafted 3 iron in V-shape. Right-click on cow → `MilkBucket`. A refused milk or shear (the cow isn't ready, the wool is growing back) does not eat the click: it shows its toast and falls through to the next right-click interaction as if the mob were not there, with no cooldown, so a bucket aimed at water beside a cow just milked still fills (`local_mob_click::MobClick::eats_click`, FU2 2026-10-07; the arm order and the refusal rule are the pure `local_mob_click::plan_right_click`, FU4b — Lead attach, feed, milk / shear, where a refused milk or shear is `MobArm::Refused` and fires no mob arm, so a tethered cow just milked is not untied; the later arms (tame, pack, mount, detach, villager / pet) are still decided in `game_loop`; a joined client's refusal already took a 16-tick cooldown on the server's `NotYet`, unchanged).
  Milking, shearing and offering a companion its taming food act **only on a
  right-click** (the place gesture pressed, the cursor captured, off the
  8-tick place cooldown — `local_mob_click::right_click_ready`, the gate every
  other right-click branch has). Bug fixed 2026-10-07 (review D2b B1): those
  three ran in the per-frame player loop with no gate, so a bucket or shears
  held with a ready cow or sheep in the crosshair milked / sheared it with no
  click, companion food rolled a tame — and was eaten — every frame (~3 rolls
  a tick) until one landed, and a not-ready cow or shorn sheep in the cone
  swallowed every later right-click (a bucket couldn't fill at water there).
- `Egg` — chicken passive lay (timed drop near the chicken).
- `MilkBucket` — drink restores 4 hunger + clears negative status (matches Minecraft baseline).
- `WaterBucket` / `LavaBucket` (Buckets MC-parity, 2026-06-22) — an empty
  `Bucket` right-clicked on a liquid **source** block fills it (flowing liquid
  can't be bottled, matching Minecraft); right-clicking a filled bucket empties
  it into the AIR cell against the clicked face, registering a new liquid source
  there and returning the empty `Bucket`. Water emptied beside lava freezes to
  obsidian via the lava sim. The fill/empty *rules* live as pure, unit-tested
  functions in `bucket::{fill_result, empty_result}` (mirroring the
  `papyrus`/`rubber` interaction pattern); the world/inventory orchestration is
  in the block-interaction handler. Single-player/client path only for now (the
  server path is the same tracked dual-sim debt as other interactions).
  **The empty bucket aims with its own ray (FU4b, 2026-10-07; FU3 verify M3).**
  The ordinary aim ray (`raycast::cast_ray`: mining, placing, combat) passes
  through water, so before this fix a bucket could never target the pond it
  was meant to fill — only lava — and The Plumber Trial could not be finished.
  An empty bucket now casts `raycast::cast_ray_fluid`: the first WATER or LAVA
  **source** cell within reach (a flowing cell is passed through, as water is
  for every other item; a wall, plant or gate still stops the ray, so it can't
  fill through one) and `GameState::try_bucket_fill` fills from it, on every
  path (single-player, a host's seats, a joiner, whose fill is the same block
  edit as ever). It runs after every mob arm, so a ready cow still milks.
  Every other item keeps `cast_ray`; a FULL bucket's pour still targets the
  solid block behind the water. `target_block` (highlight, mining) is
  unchanged.

**Processing workstations** (block-entities on the shared workstation
framework introduced in Spec 19 §"Furnace" cross-reference):

| Workstation | Block ID | Recipes |
|---|---:|---|
| Furnace | 59 (unlit), 60 (lit) | **Ore-only** post-Spec-29 (2026-05-21). Smelts RawIron → IronIngot, Copper → CopperIngot, Tin → TinIngot. Iron ore smelting was a BRIDGE in `crafting.rs` — removed when Furnace landed. Fuel ladder mirrors the Campfire. **Food cooking lives at the Campfire** (Spec 17) — raw meats no longer smelt in the furnace following Axolittle's playtest call to align with Minecraft's workstation roster. |
| Mill | 108 | Wheat → Flour. Sugar Beet → Sugar. Sugarcane → Sugar. |
| Oven | 109 | Flour + Egg + Milk → Dough → Cake. Pumpkin + Sugar → Pumpkin Pie. Berries + Dough → Berry Pie. Bread baking lifts here from the crafting table when the player has an Oven (still craftable at the table for kitchenless play). |
| Aging Rack | 110 | Time-based: Milk → Cream → Butter → Cheese. Real-time recipes — the "set and come back later" depth mechanic; aging tick uses the same block-entity progress model as Campfire fuel-burn. |

**Recipe ladder** (~20 new entries spanning intermediate → finished
food). Selected examples — full table in `crafting.rs::ALL_RECIPES`:

| Recipe | Inputs | Workstation | Tier | Food | Trade |
|---|---|---|---:|---:|---:|
| Flour | 1 Wheat | Mill | 1 | — | 3 |
| Sugar | 1 Sugar Beet OR 1 Sugarcane | Mill | 1 | — | 3 |
| Dough | 1 Flour + 1 Egg + 1 MilkBucket | Crafting table | 2 | — | 8 |
| Cake | 1 Dough + 1 Sugar + 1 Butter | Oven | 4 | 12 HP | 60 |
| Pumpkin Pie | 1 Dough + 1 Pumpkin + 1 Sugar | Oven | 3 | 8 HP | 30 |
| Beetroot Soup | 6 Beetroot + 1 Bowl | Crafting table | 2 | 6 HP | 12 |
| Butter | 1 Cream (3 in Aging Rack) | Aging Rack | 3 | 1 HP | 18 |
| Cheese | 1 Butter (4 in Aging Rack) | Aging Rack | 4 | 3 HP | 40 |

The value ladder rewards multi-step crafting: a Cake's 60 sats vs.
3 sats of raw Wheat is the **20× crafting premium** that sustains the
hunter → cook → vendor commerce loop documented in §6.2.1.

**`Bowl` material**: 3 OakPlanks U-shape → 4 Bowls. Required for soup-
type recipes (Stew, Beetroot Soup); reusable across cooking tiers.

**No Bitcoin economy hook in Tier 1.5 itself.** Trade values are the
spec-default ladder; whether a server pays sats per trade-value unit
is the server operator's call under Spec 6 §13 (`sats_per_unit`).

**Save format**: workstation block-entities serialise via the
`BlockEntityData` tagged enum (Spec 02 §"Block Entity Data") — Furnace,
Vendor, Mill, Oven, Aging Rack all carry their progress state in
`World.block_entities` and survive save/load.

### 4.1 Recipe Types

**Shaped recipes**: The arrangement of items in the crafting grid matters. A pickaxe requires three materials across the top row and two sticks down the middle.

```
Grid pattern (pickaxe):
[stone] [stone] [stone]
[     ] [stick] [     ]
[     ] [stick] [     ]
```

Shaped recipes can be mirrored horizontally by default (unless flagged as non-mirrorable). Position within the grid is relative — a 2x2 recipe works anywhere in a 3x3 grid.

**Shapeless recipes**: Only the ingredients matter, not the arrangement. Example: combining a mushroom, a bowl, and a flower in any arrangement produces mushroom stew.

### 4.2 Recipe Data Model

```rust
enum Recipe {
    Shaped {
        pattern: Vec<Vec<Option<Ingredient>>>,  // 2D grid, row-major
        result: ItemStack,
        mirrorable: bool,                        // Default: true
    },
    Shapeless {
        ingredients: Vec<Ingredient>,
        result: ItemStack,
    },
    Smelting {
        input: Ingredient,
        result: ItemStack,
        smelt_time: f32,      // Seconds. Default: 10.0
        experience: f32,      // XP granted on extraction.
    },
}

enum Ingredient {
    Item(ItemId),
    Tag(TagId),   // e.g., "genesis:planks" matches any wood plank type
}
```

**Tags** (item groups) allow recipes to accept any item in a category. "Any planks" for crafting sticks, "any wool" for beds. Tags are defined in the item registry and are plugin-extensible.

### 4.3 Crafting Grid Sizes

| Context | Grid Size | Available |
|---|---|---|
| Player inventory | 2x2 | Always, via inventory screen. |
| Crafting table | 3x3 | When interacting with a placed crafting table block. |

**Crafting flow**:

1. Player places items in the crafting grid.
2. The crafting system scans the recipe registry for a match.
3. If a match is found, the result appears in the output slot (ghosted until taken).
4. Taking the output item consumes one of each input ingredient. If ingredients remain, the output repopulates for continuous crafting.
5. Closing the crafting interface returns all input items to the player's inventory (or drops them if inventory is full).

### 4.4 Recipe Registry

The recipe registry is a global, indexed data structure optimised for fast lookup:

- **Primary index**: By result item (for recipe book lookups: "how do I craft X?").
- **Secondary index**: By ingredient set (for grid matching: "what can I craft with these items in this pattern?").

Plugins register recipes during world initialisation. Recipes can be added, removed, or overridden by plugins. Recipe IDs are namespaced (`genesis:wooden_pickaxe`, `myplugin:custom_sword`).

### 4.5 Furnace / Smelting

The furnace is a block entity with three slots: input, fuel, and output. **Spec 29 (2026-05-21)** narrowed its role to **ore smelting only** following Axolittle's playtest: the campfire (Spec 17) is the canonical cooking station, and the furnace + campfire have no overlap.

**Smelting process**:
1. Right-click the furnace block to open its UI.
2. Click the **Input** slot while holding an ore (raw iron / copper / tin) — left-click moves 1; shift-click moves the whole hotbar stack. Click **Fuel** with coal / planks / logs / sticks held to load fuel. Click **Output** (with empty hand or any held item) to take the smelted ingot.
3. The furnace burns fuel and smelts the input item over the defined `smelt_time`.
4. Default smelt time: **10 seconds** (200 ticks at 20 TPS).
5. The output item appears in the output slot. If the output slot is full or contains a different item, smelting pauses.
6. Press **'E'** or **Esc** to close the UI (matches the inventory close-key idiom).
7. **Mining the furnace spills its contents.** Breaking a furnace block spawns every
   item in its input, fuel, and output slots as item-drop entities and removes the
   block-entity outright — the cell is genuinely empty afterward. (Fixed 2026-07-07:
   the cleanup path existed but had no caller, so mining silently destroyed the
   contents *and* left the block-entity orphaned in the world map; placing a fresh
   furnace at the same cell then resurrected the old contents — the "ghost iron"
   class of bug. Mirrors the chest break-cleanup path exactly.)

**Fuel values** (burn time per item):

| Fuel | Burn Time | Items Smelted |
|---|---:|---:|
| Wooden plank | 15s | 1.5 |
| Stick | 5s | 0.5 |
| Coal | 80s | 8 |
| Block of coal | 800s | 80 |
| Lava bucket | 1000s | 100 |
| Blaze rod equivalent | 120s | 12 |

**Furnace variants**: Blast furnace (2x speed for ores, half fuel efficiency), smoker (2x speed for food). Both are defined as separate block types with modified `smelt_time` multipliers.

### 4.6 Recipe Discovery and Display

**Recipe book**: A UI panel accessible from the inventory screen showing all recipes the player has "unlocked."

- Recipes unlock when the player picks up any ingredient for that recipe (matching Minecraft's advancement triggers).
- Unlocked recipes can be clicked to auto-fill the crafting grid (if the player has the ingredients).
- The recipe book is searchable by item name.
- Server authoritative: the unlock state is stored per-player in the server's player data.

**Divergence from Minecraft**: Axe'n'Stax shows **all recipes** by default (togglable). New players should not feel lost. The "unlocked" state controls a "new!" indicator, not visibility. This is a deliberate accessibility improvement — Genesis (the 12-year-old) already knows most recipes from Minecraft, but new players should not need a wiki open in another tab.

---

## 5. Tool System

### 5.1 Tool Tiers

| Tier | Material | Durability | Mining Speed Multiplier | Mining Level |
|---|---|---:|---:|---:|
| Wood | Planks | 59 | 2.0 | 0 (stone-tier blocks) |
| Stone | Cobblestone | 131 | 4.0 | 1 (iron ore) |
| Iron | Iron Ingots | 250 | 6.0 | 2 (diamond ore, redstone) |
| Diamond | Diamonds | 1561 | 8.0 | 3 (obsidian) |
| Satori | Satori (rare, deep) | 2031 | 9.0 | 4 (top tier) |

**Mining level** determines which blocks can be harvested (drop items). A block with mining level 2 (e.g., diamond ore) requires at least an iron pickaxe. Lower-tier tools still break the block (at hand speed) but drop nothing.

Satori is the **top tier** — it sits above diamond in both gameplay progression (tool durability + mining speed + level) and economic significance (the in-world manifestation of Bitcoin sats — see §3.8 and `docs/spec/06-bitcoin-integration.md §2.2c` for vein generation). Acquiring Satori-tier gear is a real commitment: Satori only drops from pure-deepslate at depth, in deterministic veins, with exposure decay on exterior surfaces.

### 5.2 Tool Types and Effectiveness

| Tool Type | Effective Against | Special Action |
|---|---|---|
| Pickaxe | Stone, ores, metal blocks, brick, concrete | None (core mining tool). |
| Axe | Wood (logs, planks), pumpkins, melons | Strip logs (right-click on log). |
| Shovel | Dirt, sand, gravel, clay, snow | Create path blocks (right-click on grass). |
| Hoe | N/A (not a mining tool) | Till dirt/grass into farmland (right-click). |
| Sword | Not a mining tool (1.0x speed on all) | Melee combat (see section 6). |
| Shears | Leaves, wool, cobwebs, vines | Harvest blocks that normally drop nothing or different items. |

Using the wrong tool type on a block uses hand speed (1.0x). The tool still takes durability damage (1 point per block broken with wrong tool, same as correct tool). **Divergence from Minecraft**: In Minecraft, swords take 2 durability on block break. Axe'n'Stax unifies this to 1 for simplicity.

### 5.3 Tool Durability

- Each block broken with the correct tool type costs **1 durability**.
- Each block broken with the wrong tool type costs **1 durability** (discourages using swords to mine).
- Each melee attack with a tool costs **1 durability** (swords cost 1, other tools cost 2 on attack).
- When durability reaches 0, the tool breaks and is removed from inventory (with a breaking sound + particle effect).
- Durability is displayed as a colour bar on the item icon (green -> yellow -> red -> breaks).

### 5.4 Enchantment / Upgrade Hooks (Future-Proofing)

The tool system is designed with upgrade extensibility in mind, even though enchantments are not in the initial alpha:

```rust
struct ToolData {
    tier: ToolTier,
    tool_type: ToolType,
    durability_current: u16,
    durability_max: u16,
    upgrades: Vec<ToolUpgrade>,  // Empty in alpha. Future: Efficiency, Unbreaking, etc.
}

struct ToolUpgrade {
    upgrade_id: UpgradeId,   // "genesis:efficiency"
    level: u8,               // 1-5
}
```

The mining speed formula will incorporate upgrades:

```
effective_speed = base_speed * tool_tier_multiplier * (1.0 + efficiency_level * 0.3)
```

The durability damage formula will incorporate durability upgrades:

```
// Unbreaking: each durability event has a chance to not consume durability
if random() > (1.0 / (unbreaking_level + 1)) {
    // No durability consumed this hit
}
```

These hooks exist in the data model from day one so that plugins can add enchantment-like systems immediately.

### 5.5 Tool-Specific Actions

Right-click (use) actions for tools:

| Tool | Target Block | Action | Result |
|---|---|---|---|
| Hoe | Grass / dirt (top face) | Till | Converts to tilled soil. -1 hoe durability. |
| Shovel | Grass | Path | Converts to path block (0.9375 blocks tall). |
| Axe | Log | Strip | Converts to stripped log variant. |
| Stick | Unlit fueled campfire | Friction-ignite | Hold right-click 5s — 70% chance to light the campfire; stick consumed either way (Wave 27, Spec 17). |
| Flint and Steel | Unlit campfire | Instant ignite | Lights instantly if fueled; -1 durability per strike (65 uses); refuses with toast on unfueled (Wave 27, Spec 17). |
| Shears | Sheep (entity) | Shear | Drops wool, sheep becomes "sheared" variant. |

These actions are defined in the tool's registered `UseHandler` and are plugin-extensible. New tools with custom right-click behaviours can be registered by plugins.

---

## 6. Health and Combat

### 6.1 Health Points

| Parameter | Value | Notes |
|---|---:|---|
| Max HP | 20 | Displayed as 10 hearts (2 HP per heart). **T7 planned**: dynamic 4-12 capacity — see §6.2 callout. |
| Natural regeneration rate | 1 HP / 4 seconds | Only when hunger is >= 18 (9 full shanks). |
| Rapid regeneration | 1 HP / 0.5 seconds | When hunger is exactly 20 (full). |
| Starvation damage | 1 HP / 4 seconds | When hunger reaches 0. Floor per difficulty (§8.5 table, W2): Easy 10 HP, Normal 1 HP, Hard kills, Peaceful half a heart. |
| Respawn HP | 20 | Full health on respawn. |

### 6.2 Hunger / Stamina

| Parameter | Value |
|---|---:|
| Max hunger | 20 (displayed as 10 shanks) |
| Max saturation | Equal to current hunger level |
| Sprint threshold | Hunger > 6 (can only sprint if hunger > 6) |
| Regeneration threshold | Hunger >= 18 |

**Hunger depletion**: Actions cost hunger points (technically, saturation drains first):

| Action | Exhaustion Cost |
|---|---:|
| Sprinting (per metre) | 0.1 |
| Jumping | 0.05 |
| Sprint-jumping | 0.2 |
| Breaking a block | 0.005 |
| Taking damage | 0.1 per HP lost |
| Swimming (per metre) | 0.01 |

When exhaustion accumulates to 4.0, one saturation point is consumed (or one hunger point if saturation is 0). Exhaustion then resets.

**Food items** restore hunger, saturation, and HP. The full table + the
raw-meat poison risk model + the cooked-meat saturation differentiation
live in **§6.2.1 Food Values and Cooking Risk** below.

> **T7 changes — dynamic health + hunger capacity** (planned, not yet
> implemented). The fixed Max HP = 20 and Max hunger = 20 values above
> are replaced at T7 by a **dynamic capacity system**: both bars start at
> a smaller slot count (8/8 in the design proposal), with a range of 4
> (min) to 12 (max), drifting up or down based on diet (`diet_score` on
> consumed foods, with superfoods scoring high) and movement
> (`activity_score` from sprint / mining / combat). Display ratios will
> rescale accordingly. Until T7 ships, the fixed model in this section
> remains authoritative — no behaviour change for T1-T6.
>
> Full design: `docs/vision/farming-economy-long-run.md` §8.4. The
> `superfood: bool` and `diet_score: i8` per-item data fields are laid
> down from T1.5 (data-only); the runtime that consumes them ships
> alongside the status-effect runtime at T7.

### 6.2.1 Food Values and Cooking Risk

> **Status (2026-05-18).** This subsection supersedes the small "example
> values" table previously in §6.2. The values + risk model are a
> deliberate redesign over the original Minecraft-clone numbers to:
> (a) align with real-world food safety (raw chicken is dangerous; raw
> beef is borderline-fine), (b) make cooking effort *meaningfully*
> better than raw (not just +66%), and (c) open commerce niches so
> beef-ranchers, chicken-farmers, and cooks each have distinct economic
> roles. Code currently ships the pre-redesign numbers from earlier
> waves; a foundation spec lands the implementation in a separate
> cycle. **The values in this section are the canonical target.**

#### Design principles

1. **Raw meat is edible but risky, varied by meat type.** Real-world
   parasite + bacterial risk maps onto a per-meat poison chance.
   Eating raw chicken is a coin-flip; raw beef is nearly always fine.
2. **Cooking is meaningfully better.** Eliminates poison risk; +50% HP
   over raw; substantially higher saturation. A cooked meal sustains a
   long mining session in a way raw meat cannot.
3. **Variety creates commerce.** Different cooked meats sit at
   different trade-value tiers so each producer + cook combination has
   a distinct economic niche. Cooking premium is **5–6× the raw
   value**, sustaining a hunter → cook → vendor commerce loop.

#### Raw meats — edible-but-risky safety net

Eating raw meat heals partial HP and rolls a poison chance. On a
poison roll, the player receives a 60-tick (3 s) poison effect via the
shared poison machinery (`PlayerCombat::apply_poison`). The
HP is still gained — the player absorbed the calories before the
bacteria took hold — but the toast `"Food poisoning!"` teaches the
lesson.

| Raw meat | HP | Saturation | Poison chance | Real-world rationale |
|---|---:|---:|---:|---|
| **Raw Beef** | 3.0 | 1.8 | 5% | Safest raw (steak tartare; farmed cattle have low parasite load) |
| **Raw Mutton** | 2.0 | 1.2 | 15% | Toxoplasmosis + cysticerci risk |
| **Raw Porkchop** | 2.0 | 1.2 | 30% | Trichinosis — historically *the* meat-cooking warning |
| **Raw Chicken** | 1.0 | 0.6 | 50% | Salmonella + campylobacter — coin-flip food poisoning |

**Rotten Flesh removed (2026-05-24).** A `RottenFlesh` desperation-food
row used to sit at the bottom of this table (1.5 HP / 40% poison, below
all four proper raw meats). The item was removed entirely in the
Fantasy-Roster Excision along with the Skeleton/Zombie roster that
sourced it (`docs/foundations/2026-05-24-fantasy-roster-excision.md`).

#### Cooked meats — the reward + saturation differentiation

Cooking eliminates poison risk and boosts both HP and saturation
(saturation = exhaustion budget; higher saturation means hunger ticks
down later). Saturation differentiation is where the *real* commerce
ladder lives — cooked beef sustains a long expedition; cooked chicken
is a quick refill.

| Cooked meat | HP | Saturation | Commerce niche |
|---|---:|---:|---|
| **Cooked Beef (steak)** | 6.0 | 12.0 | **Premium** — highest single-bite value, highest saturation. The export crop. |
| **Cooked Porkchop** | 5.5 | 10.0 | High value. Pigs grow fastest of the big meats — volume + quality combo. |
| **Cooked Mutton** | 5.0 | 8.0 | Specialty — sheep are slow-growing, but the wool side-product subsidises. Cheese-and-roast economy. |
| **Cooked Chicken** | 4.5 | 6.0 | Commodity — cheap, fast, lowest cooked value. The wheat of meat. |

**Cooking premium = 1.5–2× raw HP, but 5–10× raw saturation.** Saturation
is the load-bearing differentiator — players who only eat raw refill
their hunger constantly; players with cooked food set off on long
expeditions without snacking. Cooking's value compounds at scale.

#### Crop foods (Tier 1 + Tier 1.5 farming)

For completeness — Tier 1 ships as of Wave 26
(`2026-05-14-farming-system.md`); Tier 1.5 ships as of Wave 30
(`2026-05-14-farming-tier-1.5-processed-economy.md`).

The `complexity_tier` column counts processing steps from raw inputs
(0 = raw drop, 1 = one workstation step, 2 = two-step recipe, etc.) and
drives the `trade_value` default ladder. See §3.14 + foundation §6.

| Food | HP | Saturation | Complexity | Notes |
|---|---:|---:|---:|---|
| **Bread** | 5.0 | 6.0 | 1 | Crafted from 3 wheat. Staple food, kid-friendly survival arc. |
| **Carrot** | 3.0 | 3.6 | 0 | Edible raw; doubles as a seed when planted on tilled soil. |
| **Potato — raw** | 1.0 | 0.6 | 0 | Edible raw but low value; Baked Potato cooks at the Campfire (post-Spec-29 — see Spec 17). |
| **Baked Potato** | 5.0 | 6.0 | 1 | Campfire 10 s; significantly upgrades the raw value. |
| **Baked Carrot** | 4.0 | 4.8 | 1 | Campfire 10 s. |
| **Wheat** | — (inedible — bake it) | — | 0 | Not a food per Minecraft convention. Mill into Flour or craft to Bread. |
| **Wheat Seeds** | — | — | 0 | Not a food. Plant on tilled soil. |
| **Beetroot** | 1.0 | 1.2 | 0 | T1.5 raw root; primary input to Beetroot Soup. |
| **Beetroot Soup** | 6.0 | 7.2 | 2 | T1.5: 6 Beetroot + 1 Bowl. Not stackable while in bowl. |
| **PumpkinFood (raw)** | 1.0 | 0.6 | 0 | Eaten raw; primary input to Pumpkin Pie. |
| **Pumpkin Pie** | 8.0 | 9.6 | 3 | T1.5 Oven: Dough + Pumpkin + Sugar. |
| **Berries** | 2.0 | 1.4 | 0 | Hand-harvested from bush. |
| **Berry Pie** | 8.0 | 9.6 | 3 | T1.5 Oven: Dough + Berries + Sugar. |
| **Cake** | 12.0 (split across slices) | 14.4 | 4 | T1.5 Oven peak food: Dough + Sugar + Butter. |
| **Cookie** | 2.0 | 0.4 | 2 | T1.5 Oven: Flour + Cocoa or similar. |
| **Cheese** | 3.0 | 4.0 | 4 | T1.5 Aging Rack — real-time recipe (Cream → Butter → Cheese). |
| **MilkBucket** | 4.0 | 2.4 | 1 | T1.5 — clears negative status (Minecraft baseline). Empties bucket. |
| **Egg** | — | — | 0 | Not a food directly; input to Dough / Cake / Cookie / Pancakes. |

Crop foods carry **no poison risk** — agriculture is a controlled-
hygiene activity in a way mob meat isn't.

#### Other foods (existing)

| Food | HP | Saturation | Notes |
|---|---:|---:|---|
| **Golden apple** | 4.0 | 9.6 | Rare loot — see future progression spec |

#### Commerce ladder (trade-value tags — Tier 1.5 economy hook)

`trade_value` is the cross-economy lingua franca shipped in farming
Tier 1.5 (foundation
`2026-05-14-farming-tier-1.5-processed-economy.md` §6). The per-item
default ladder is computed from `complexity_tier`; specific recipes
hand-tune their value via `Item::trade_value`. Server operators can
override per item via `ServerEconomyConfig` (Spec 6 §13). For meats,
the target values are:

| Item | Raw trade-value | Cooked trade-value | Cooking premium |
|---|---:|---:|---:|
| **Beef** | 5 sats | **30 sats** | **6×** |
| **Pork** | 3 sats | 18 sats | 6× |
| **Mutton** | 2 sats | 12 sats | 6× |
| **Chicken** | 1 sat | 5 sats | 5× |
| **Bread** | 4 sats | n/a | — |

The 5–6× cooking premium is the commerce engine. A typical loop:

- **Hunter** kills cows, sells raw beef at 5 sats each.
- **Cook** buys cheap raw + fuels the furnace + sells cooked beef at 30 sats.
- **Market-stall vendor** rents space; takes a 2–5% slice on commerce.

Three actors making a living from the same source meat, none of them
dominating. Add fence-protected chicken farms + brewery beer +
restaurant menus and you have a real medieval-village economic
ecosystem on top of the same Tier 1.5 workstation framework.

#### Effort gradient — pointing to future tiers

The Tier 1 numbers set up clean ladders for later content:

| Tier | What it adds | Net effect on the food economy |
|---|---|---|
| **T1 (current)** | Raw + cooked split | Risk gradient + cooking-pays-off + per-meat niche |
| **T1.5 (Furnace, spec'd)** | Furnace as proper workstation (vs. crafting-table-with-coal) | Cooking scales; trade-value tags activate |
| **T4 (Preservation, vision-doc'd)** | Bacon, ham, jerky, salami, cured meats | No spoilage, higher trade value, longer saturation. Smoked beef = premium luxury. |
| **T7 (Mastery, vision-doc'd)** | Aged steak, dry-cured prosciutto, quality grades | Buff effects + restaurant economy unlocks |

Each future tier layers value on the same source meat. The kid who
farms cows in T1 can become a cook in T1.5, a butcher in T4, a master
chef in T7 — without re-learning farming.

#### Implementation gap (code lags spec)

As of 2026-05-18 the code at `game/engine/src/item.rs` ships the
pre-redesign Wave-2/Wave-6 numbers (raw meats 2–3 HP, cooked 4–5 HP,
no poison on any raw meat; the old `RottenFlesh` item — 4.0 HP, no
penalty — was removed entirely on 2026-05-24 in the Fantasy-Roster
Excision). The
values in this section are the **canonical target**; a separate
foundation spec lands the implementation:

- Add `food_poison_chance(&self) -> u8` (0–100) to `Item`.
- Add `saturation_value(&self) -> f32` (separate from `food_value`).
- Extend `combat::feed` to track saturation as a real ladder against
  exhaustion (currently feed is HP-equivalent).
- Eat path rolls poison + toasts `"Food poisoning!"` on hit.
- Update raw + cooked numbers per the tables above.

Estimated scope: ~200 lines code + tests + spec edits, single
foundation spec.

### 6.3 Damage Types

Each damage type is a tagged enum, allowing armour and enchantments to selectively reduce specific types:

| Damage Type | Source | Armour Reduces? |
|---|---|---|
| `Fall` | Falling > 3 blocks | No |
| `Melee` | Entity attack | Yes |
| `Projectile` | Arrow, thrown item | Yes |
| `Fire` | Standing in fire / lava surface | Yes |
| `FireTick` | Being on fire (DoT) | Yes |
| `Lava` | Submerged in lava | Yes |
| `Drowning` | Underwater with no air | No |
| `Suffocation` | Head inside solid block | No |
| `Explosion` | Blasting Keg (Spec 49) | Yes — distance falloff + line-of-sight reduction. **Implemented Spec 49** (`explosion::blast_damage` / `line_of_sight_factor`). **Player hits route through equipped armour** (Spec 28e) via `explosion::apply_player_blast_damage`, and a landed hit dismounts perched parrots like any other damage — fixed 2026-07-11; before that, blast damage hit `combat.take_damage` raw, bypassing armour. A **joiner's** body is hit on the server (`explosion::apply_joiner_blast_damage`, soaked by its reported armour points) and its own client lands no blast on itself (MP-D2a, Spec 04 §5.3.2). |
| `Void` | Falling below Y = -64 | No (instant kill) |
| `Starvation` | Hunger at 0 | No |
| `Magic` | Potions, status effects | No (bypasses armour) |

### 6.4 Melee Combat

**Divergence note**: Minecraft's combat system has been contentious (1.8 spam-click vs 1.9+ cooldown). Axe'n'Stax uses a **hybrid approach** designed to feel good for both camps, configurable per server.

**Default combat model** (Axe'n'Stax standard):

| Parameter | Value |
|---|---:|
| Attack reach | 3.0 blocks |
| Attack cooldown | 0.5 seconds (10 ticks) |
| Cooldown penalty | Attacks before cooldown deal proportionally less damage (0% at 0 ticks, 100% at 10 ticks). |
| Sweep attack | Enabled: full-cooldown sword attacks hit nearby entities within 1 block of the target for 1 HP each. |
| Knockback (base) | 0.4 blocks horizontal impulse away from attacker. |
| Knockback (sprinting) | Additional 0.4 blocks (total 0.8). |
| Critical hit | Triggered when falling (not on ground, vertical velocity < 0). Deals 150% damage. |

**Weapon damage values**:

| Weapon | Base Damage | Attack Speed | DPS |
|---|---:|---:|---:|
| Hand (fist) | 1 | 4.0/s (no cooldown) | 4.0 |
| Wooden sword | 4 | 1.6/s | 6.4 |
| Stone sword | 5 | 1.6/s | 8.0 |
| Iron sword | 6 | 1.6/s | 9.6 |
| Diamond sword | 7 | 1.6/s | 11.2 |
| Satori sword | 8 | 1.6/s | 12.8 |
| Wooden axe | 7 | 0.8/s | 5.6 |
| Diamond axe | 9 | 1.0/s | 9.0 |
| Satori axe | 10 | 1.0/s | 10.0 |

**Damage values are code-confirmed** (`game/engine/src/crafting.rs::Tool::attack_damage`); Stone-axe and Iron-axe rows are intentionally omitted because their speed pacing depends on the per-tier-attack-speed system this table assumes — that system **does not exist in code yet**. Today the engine uses a uniform `ATTACK_COOLDOWN = 10` ticks for every tool (= 0.5s = 2.0/s, see `game/engine/src/combat.rs:13`). The per-tool attack-speed column in this table is the design target; treat the Wood/Stone/Iron/Diamond/Satori speeds as aspirational until the per-tier cooldown system lands. DPS column reflects the design target speeds, not the current 2.0/s reality.

**Server authority**: Hit detection is server-authoritative. The client sends an `Attack` packet with the target entity ID. The server validates:
- Is the target within reach (3.0 blocks)?
- Is the attack cooldown satisfied?
- Are both entities loaded and alive?
- Line-of-sight check (no attacking through walls).

**Configurable combat modes** (per-server):
- `Classic`: No cooldown, spam-click. Faster, more accessible (Minecraft 1.8 style).
- `Standard`: Cooldown-based (default, described above). More strategic.
- `Custom`: Server/plugin defines custom timing via configuration.

### 6.4.1 Combat for joiners (as built, MP-D2a v68 + MP-D2b v70)

A player who has joined someone else's world (LAN / online host or dedicated
server) fights the **server's** mobs — there is no private mob world on a
joiner any more (Spec 04 §4.2c).

- **Mobs hurt joiners server-side.** `GameServer::tick_player_hazards` runs the
  same hostile-melee rule the client runs for its local players
  (`combat::hostile_melee_tick`: every `Hostile` mob within 1.5 blocks
  horizontally, overlapping the 1.8-block body, hits for 3 HP × the difficulty
  scale, knockback 0.4 away + 0.3 up; Peaceful: no attacks) and the same
  lava / fire contact rule (`survival::contact_hazard`: 2 HP lava / 1 HP fire
  every 10 ticks at the feet or head) on every present, living, non-flying
  joiner's body. Both are reduced by the armour points the joiner's input
  reports (`InputPacket.armour_points`, client-asserted). The joiner's own
  client no longer runs either for its body.
- **Keg blasts hurt joiners server-side too** (`explosion::apply_joiner_blast_damage`,
  the same falloff and line-of-sight rule, wherever the keg goes off), and a
  blast that lands wears the joiner's armour, as it wears a local player's
  (review D2b LOW-4); a joined client lands no blast on itself.
- **The joiner's health is the server's.** It shows the server's value plus
  any loss its own client made that the server has not applied yet, reported
  per input as `InputPacket.health_delta` — since C2a a loss only: the server
  counts a reported heal as nothing (Spec 04 §5.3.2, which tables every
  source). A lethal server-side hit — or a reported loss that finishes off a
  body the server holds lower than the client knew — reaches the death screen
  through `PlayerEvent::DiedOf` (v70; `Died` before), which names the cause
  for the death screen ("Killed by a Brigand", "You tried to swim in lava");
  if the client's own sum reaches zero first, zero is dead and it enters the
  death screen itself, and the server's `DiedOf` arriving a moment later still
  names the cause on it (review D2b LOW-7; a `Died` arriving after it has
  pressed Respawn is dropped as stale — Spec 04 §5.3.2).
- **A joiner's hunger is the server's (C2a, v73).** The server runs the
  joiner's metabolism — the same `PlayerCombat::tick_metabolism` (drain every
  600 ticks, +1 HP every 80 ticks at hunger ≥ 18 costing a point, starvation
  every 80 ticks at 0 down to the difficulty's floor) — and sends the joiner
  its hunger each update (`own_hunger`); the joined client runs none of its
  own. Starvation on Hard kills the joiner on the server ("You starved",
  `DiedOf`). A creative joiner's body is kept whole, the client's own creative
  rule. A host's own players are unchanged: their client runs their hunger.
- **A joiner eats by asking (C2a).** Right-clicking food (hungry or hurt, as
  in single-player) sends an `ItemAction::Eat`; the server feeds and heals the
  body by the food value and takes the food from its copy of the joiner's
  inventory, and the client takes the food only when the server says yes. A
  refusal says why ("You're not hungry."; an eat refused only for coming too
  soon is silent). **Eating is paced in fixed ticks everywhere (C2a-fix):**
  one bite per 16 ticks (0.8 s) at any frame rate, and a joiner has one
  request in flight at a time. The server accepts an eat once its cooldown is
  within 4 ticks of spent, to forgive arrival skew. This is a feel change for
  single-player and a host's own players: the cooldown used to count frames
  (one bite per 16 frames, 0.27 s at 60 fps; faster the higher the frame
  rate), so a held right-click now eats at 0.8 s intervals regardless of fps
  (owner's test sheet).
  **A held click stays a meal (C2b-fix):** the block on placing after a bite
  is counted in the same fixed ticks (0.8 s, true below 20 fps too), and a
  right-click with food in hand while the body wants to eat but no bite is due
  (the cooldown, or a joiner's request in flight) is swallowed. It never falls
  through to planting a carrot, opening a chest or crafting table, or going
  to bed between bites. With a full stomach food is not a meal, and the click
  goes on as before.
- **A joiner sleeps by asking (C2a).** Right-clicking a bed sends an
  `ItemAction::Sleep`. The server checks the bed is real and in reach, that it
  is night by its clock (the rule the single-player bed uses) and that the
  joiner has not slept this night (once a night per player); then it sets the
  joiner's spawn point to the bed — a later respawn lands there — and heals
  the body to full, leaving hunger alone ("You feel rested. Spawn point
  set."). Refusals: "You can only sleep at night.", "You've already slept
  tonight.", "That bed is too far away.". **A joiner's sleep does not skip the
  night**: the clock is the host's (until D4); the host's own sleep still
  skips it for everyone. The bed spawn, health, hunger and the
  slept-tonight mark all reset when the joiner reconnects or the server
  restarts, until the per-npub sidecar step.
  `/kill` and `/heal` are op-only, never available to a joiner (were one run,
  `/heal` would heal only its own view: the server ignores reported heals).
- **A joiner's crafting and Q-drops reach the server (C2b, v74, Spec 04
  §4.2f).** A joiner crafts exactly as in single-player (2×2 grid or a
  crafting table's 3×3, one craft per click on the result); after each craft
  its client tells the server the grid it crafted from, and the server repeats
  the craft on its copy of the joiner's inventory (inputs out, output in) — a
  recipe bigger than 2×2 only at a real crafting table within reach. Nothing
  is undone on the client. A joiner's Q-drop becomes a real item in the
  server's world, thrown from the joiner's body, which everyone sees and can
  pick up (the dropper waits the usual 1.5 s); its client spawns nothing
  itself. A joiner can Q-drop at most once every 4 ticks (single-player is
  unchanged: one drop per press). A Plan still drops only in the joiner's own
  view (no wire form). Neither a drop nor a craft may spend an item a request
  still waiting on the server needs (milking with the only bucket, feeding
  the only wheat): the key or click then does nothing. An item the server
  gives a joiner whose inventory (as the server holds it) is full lands at
  the joiner's feet as a real item, as single-player's full-inventory spill.
- **A joiner fights and handles the server's mobs (MP-D2b, Spec 04 §4.2d).**
  Its swing at the mob under the crosshair (in front of the first block, not
  the swing's wide cone) goes to the server as `EntityAttack`; the server
  checks the mob is alive, within reach of the body IT holds (3 + 1.5
  blocks) and ahead of it (never a parrot perched on a shoulder), holds the
  joiner to the client's swing rate on average (a swing may come up to 3
  ticks early, but each moves the server's schedule a full 10-tick cooldown
  on — review D2b LOW-2), and lands the hit with `combat::strike` —
  single-player's melee rule: damage from the held item, ×1.5 if the server's
  body is airborne, knockback, the sweep, the prey bolt, a provoked Bear or
  Hyena, and `LastAttacker` naming the joiner; then `combat::after_swing`
  (its wolves rally, a Nostrich kicks back). The weapon wears only when the
  server confirms the swing. Between swings a held break goes on, so a chicken
  at your feet doesn't stop you digging. The single-player villager warning
  ("Careful — that's a villager.") runs on the joiner's client first.
- **One-shot interactions** — breeding feed, taming, shearing, milking, a
  Lead on and off, a Lead moved onto a fence post (review D2b B3), a pet's
  sit / follow (own pets only) — go to the server as `EntityInteract` and run
  through `mob_interact`, the same functions single-player's right-click
  calls; the joiner gives up the food, bucket or Lead only when the server
  accepts — from wherever the item has moved meanwhile, and never the same
  item for two requests in flight (review D2b LOW-1) — and products (milk,
  the Lead back, wool and loot picked up) arrive as `InventoryGrant`s. A pet
  tamed by a joiner is owned by its verified npub; a guest can't tame ("Sign
  in to tame animals in someone else's world."). Riding (and a steed's pack)
  and villager trading are not yet available to a joiner (D2c): "Riding and
  trading aren't available in someone else's world yet."
- **Where the server doesn't simulate it, it's refused** (review D2b
  MEDIUM-2). Breeding, Leads and pets following run only in a host's own sim,
  so only a lending LAN / online host takes a joiner's breeding feed, tame,
  Lead (on a mob or a post) or pet command. A dedicated or `--no-lend` server
  answers "This server doesn't support that yet." and nothing is used (it
  used to take the wheat or the Lead and nothing ever came of it). Shearing,
  milking and taking a Lead off work everywhere.
- **A breed credits whoever fed the parents** (review D2b B2): a fed animal
  remembers its feeder, and a baby born of a joiner's feed completes that
  joiner's "breed animals" step (`PlayerEvent::Bred`), not the host's; the
  host's `BreedAnimals` fires only when one of its own players fed a parent.
- **Kills credit the joiner who made them** (`combat::attribute_kill`, the one
  rule): the server tells it with a `KillEvent`, and its client runs the same
  attribution single-player's death sweep runs (kill counter for bounties,
  the KillMob challenge, the Nostrich's Vow, the villager-kill reputation
  penalty). A host's own player is never credited with a joiner's kill. A
  death no player's hit caused (lava, a fall, another mob, a pet wolf) goes to
  the nearest living player, a joiner included (review D2b LOW-5) — the
  single-player rule. A joiner who leaves takes its credit with it: a mob it
  hit that dies later credits nobody — not the next joiner given its slot,
  not the host — and a Bear, Hyena or bee it provoked calms down (review D2b
  MEDIUM-1).
- **Server-landed hits wear the joiner's armour** (v70, `ArmourWorn`): one
  durability per worn piece per landed hit (keg blasts included), the
  single-player rule. On a
  lending host, species-AI attacks (bee sting, goat charge, shark bite) reach
  joiners as well as the host's players, a Bear or Hyena a joiner provoked
  charges it, and a joiner's pet follows it.
- **Not yet on a joiner:** mob push-out and knockback are not predicted (each
  shows as a position correction); its bow and slingshot shots fly only in its
  own world; a dedicated server runs no species AI, breeding or Leads, so it
  refuses those interactions (above) until D4.

### 6.5 Ranged Combat

**Bow**:
- Draw time: 0-1.0 seconds (hold right-click to charge).
- At full draw (1.0s), arrows deal **6 damage** with maximum velocity.
- Partial draw scales linearly: 0.5s draw = 3 damage.
- Arrow trajectory is affected by gravity (0.05 b/tick downward acceleration).
- Maximum arrow range at full draw, 45-degree angle: ~64 blocks.
- Arrows are entities. They stick into blocks and can be picked up (unless fired by a mob or a player in Creative mode).
- Critical: Full-draw arrows have a 20% chance to deal +1 bonus damage (particles show on crit).

**Crossbow equivalent** (future): Higher damage, longer reload, can be pre-loaded.

### 6.6 Armour

**Armour points and damage reduction**:

| Armour Piece | Material | Armour Points | Toughness |
|---|---|---:|---:|
| Leather helmet | Leather | 1 | 0 |
| Iron chestplate | Iron | 6 | 0 |
| Diamond leggings | Diamond | 6 | 2 |
| Diamond full set | Diamond | 20 total | 8 total |
| Satori full set | Satori | 20 total | 12 total |

**Damage reduction formula**:

```
damage_after_armour = damage * (1 - min(20, max(armour_points / 5, armour_points - (4 * damage / (toughness + 8)))) / 25)
```

This matches Minecraft's formula and provides diminishing returns at high armour values while toughness helps against heavy hits.

**Rubber Boots — sprint multiplier (wired 2026-07-07).** Equipping Rubber boots
multiplies grounded sprint speed (§1.1) by `armour::sprint_multiplier` = **1.4×**,
recomputed every tick from the equipped boots material (`PlayerSlot::refresh_sprint_boots_mult`)
— not sticky, drops back to 1.0× the instant the boots are unequipped. Capped at 1.4
(not the originally-advertised 2.0×) to stay comfortably under the server's
anti-cheat horizontal-speed gate; grounded only — Creative/Spectator flying speed is
untouched. Remote (hosted-session) players don't yet get the bonus — armour isn't
tracked server-side for remote players today (same BRIDGE as reach, above).

### 6.7 Death and Respawn

On death (Survival):
1. **Graves (#47, implemented 2026-06-16)** — the inventory is **NOT scattered**. The 36 slots are snapshotted into a recoverable `GRAVE` block placed at a safe cell at the death spot (`grave::find_safe_grave_pos` searches the death cell, then upward, then outward — never the void, never a hazard, never destroying a build; on the rare no-safe-cell it falls back to the legacy scatter so nothing is lost). The grave's `GraveData.slots` is **index-aligned** to the inventory, so recovery returns each stack to its **original slot** (Corpse-mod parity). Right-click the grave to reclaim (best-effort into free slots if the original is taken); it's removed when emptied. Breaking the grave spills its contents. Persisted in `WorldSave.graves` (append-only, serde-default). The death position is also toasted so the player can walk back.
2. **Keep-inventory** — `WorldMeta.keep_inventory` (runtime-mirrored on `World`, default `false`; **`true` for blank-canvas/parkour worlds**). When on, death leaves the inventory intact and creates no grave. Toggle live with `/keepinventory [on|off]` (`/ki`).
3. **No penalty** — death is sats/score/proof-of-play **penalty-free** (it only ever MOVES items; verified — the death path touches no `economy`/`proof_of_play`). XP retention is moot (no XP system).
4. The death screen shows "You Died!", **a cause line** from the last damage that landed (`PlayerCombat.last_damage`, a `survival::DamageCause`, wording in `survival::death_message`): "You fell from a high place", "You drowned", "Killed by a <mob display name>", "You starved", "You tried to swim in lava", "You burned to death", "You were blown up", fallback "You died" — plus **"Your items are in a grave at x, y, z"** when this death placed a grave (`PlayerSlot.last_grave`), then Respawn. (W2, 2026-10-05. The cause is computed by whichever sim owns that player's body — a joined client's own sim for its own death — so it needs no wire field.) **No auto-respawn** (owner decision 2026-10-06, Minecraft-style — the old 2 s timer left no time to read these lines): the death screen stays up until the player chooses Respawn. Per-seat inputs: the **Respawn button** (mouse click, or a tap on touch — egui receives touch as pointer events); **Enter** for the keyboard seat (P1 only, ignored while chat is open, so one key press can't respawn every split-screen seat); **A on the seat's own controller** (`local_join::ui_pad_index`; pads can't reach egui buttons in-game, so this is their path). A hint line under the button names the seat's input. The press sets `PlayerCombat.respawn_requested` (`request_respawn`, a no-op while alive); the client death loop respawns on `should_respawn()` and `respawn()` clears it. P1's pointer is released on death (web included) and a stray click while dead does not re-lock it. **A joiner's death is server-held (MP-A3, protocol v67).** The server marks a joined (server-simulated) player dead when its copy of them dies (a fall or drowning in `GameServer::tick_player_survival`) or when their input reports zero health (a death their own client caused — since MP-D2a only the sources it still owns, such as starvation on Hard, or the client's own sum of the server's value and its unapplied losses reaching zero; mobs, lava/fire and blasts hit the server's copy, §6.4.1 — believed only downward). Since MP-D2a a reported `health_delta` loss that takes the server's copy to zero also kills it, and is announced with `Died` like any server-side death (the client didn't compute it). While dead the body runs no physics, picks nothing up (its death drops stay on the ground for others), is no mob's target, presses no pressure plate, and its moves, edits and device interactions are ignored (each edit is sent back so the ghost block un-places). Nothing revives it on a timer — the old 40-tick `respawn_timer` restore is gone. When its own sim kills the body the server tells every client `PlayerEventType::Died`, so a joiner whose server copy died unseen still reaches its death screen (a reported death is not echoed back — the reporting client already knows); the Respawn button sends `PacketType::Respawn`, and only then does the server respawn the body (`GameServer::respawn_player`: full health, hunger and breath, standing on the ground in the column of the spawn point it holds — the join spawn; a bed or `/spawnpoint` is client-side only) and answer `PlayerEventType::Respawned { x, y, z }`, which the joiner snaps to. A Respawn from a living player is ignored (no free teleport). A player who disconnects while dead is dropped as usual and never revived. A subtle red screen-edge vignette (alpha ≤ 0.35, fading over 0.4 s — `combat::PLAYER_HURT_FLASH_TICKS`, `hud_ui::draw_hurt_vignette`) marks every hit that lands; never a full-screen flash.
5. Respawn location: the player's bed (if set and unobstructed) or the world spawn point.
6. On respawn: **full health, full hunger**. Respawning hungry adds frustration without depth.

*(Superseded:* the original spec dropped all items as scattered entities + XP orbs; graves replace the scatter, XP never existed.*)*

---

## 7. World Interaction Systems

### 7.1 Signal System (Redstone Equivalent)

Axe'n'Stax uses a **signal propagation system** inspired by Minecraft redstone but with cleaner semantics. Internally referred to as the "circuit system."

> **Implemented as Spec 48 (Electricity) — Phase 1 delivered 2026-06-17.** The circuit system
> shipped as the **Electricity tier** (insulated **Cable**, not bare "signal wire"). Phase 1
> diverges from the 0–15 model the table below sketches, in two ways that table predates:
> (1) **Phase 1 is binary** (on / off) — the per-terminal `energised: u8` is *reserved* for the
> Phase-3 analog economy (dimmer lamp, motor speed). (2) **Phase 1 cables have NO distance
> decay** — a circuit lights at any length; the "signal loses 1 per block / 15-block limit"
> below is the original Minecraft-parity plan and is now an **open design question for Axolittle**
> (no-limit, the differentiator, vs a repeater-gated limit). Shipped: Lever / Button / Pressure
> Plate / Hand Crank / Steam Generator / Battery / **Water Wheel** → Cable → Electric Lamp / Powered Rail, plus a
> **Logic Gate (AND/OR/NOT/XOR)** in place of repeater + comparator. **Phase 2 (also delivered
> 2026-06-17)** adds light-beam triplines: a **Beam Sensor** emits an IR beam that arms via a
> second sensor (through-beam) or a **Mirror** (retroreflect 180° / turn 90°); crossing the
> armed beam triggers it. A **Motion Sensor** is a beamless proximity trigger. **Pistons shipped
> 2026-06-21** (plain piston — see Piston mechanics below). Full design:
> `docs/foundations/2026-06-17-electricity-power-logic.md`.

**Core concepts**:

| Concept | Minecraft Equivalent | Description |
|---|---|---|
| Signal wire | Redstone dust | Carries signal along surfaces. Signal strength 0-15. |
| Signal source | Redstone torch, lever, button | Emits a signal of strength 15 (or variable for some sources). |
| Signal repeater | Redstone repeater | Extends signal, adds configurable delay (1-4 ticks). |
| Signal comparator | Redstone comparator | Compares or subtracts signal strengths. Reads container fill levels. |
| Signal consumer | Lamp, piston, door, dispenser | Activates when receiving signal strength > 0. |

**Signal strength**: Signals propagate at strength 15 from the source and lose 1 strength per wire block. This limits unboosted wire range to 15 blocks, matching Minecraft.

**Tick model**: The signal system evaluates during a dedicated phase of the game tick, after entity updates but before block updates. Signal updates are **deterministic** — given the same inputs, the same outputs occur every time. Minecraft's redstone has known ordering bugs (quasi-connectivity, update order dependencies). Axe'n'Stax does **not** replicate these bugs.

**Divergence from Minecraft**: Quasi-connectivity (where pistons and dispensers can be activated by signals meant for the block above them) is **not implemented**. This is Minecraft's most confusing and inconsistent redstone behavior. Axe'n'Stax trades "quirky parity" for "learnable consistency." Servers that want quasi-connectivity can enable it via a config flag for players who rely on it.

**Piston mechanics** (plain piston SHIPPED 2026-06-21, `game/engine/src/piston.rs`):
- On rising power the piston extends a **`PISTON_HEAD`** block one cell ahead and shoves the
  contiguous column of pushable blocks forward by one into empty space; on falling power it
  retracts (the head is removed). The head block *is* the extended state, so pistons need **no
  new save format** — they round-trip through the existing block + meta store.
- Pushes up to **12** blocks (`piston::MAX_PUSH`). If the column has nowhere to go (blocked by a
  non-pushable block or the world edge), the piston stays retracted.
- **Pushable = a strict allow-list** of plain solid full-cube building/terrain blocks (stone,
  dirt, planks, logs, leaves, glass, metal/gem blocks, …). **Immovable:** bedrock, Satori,
  fluids, pistons/heads, and **anything with a block entity** (chests, furnaces, hoppers,
  vendors, signs, …) — so a push can never move or orphan block-entity data.
- **Facing** is stored in the shared `meta::Facing` field (low 3 bits), stamped from the
  placer's look direction at place time (pitch included → up/down pistons work).
- Powered by any Electricity consumer signal (energised cable / adjacent source). Runs on the
  4-tick block-sim cadence reading the previous tick's energised map — a ~0.2 s activation
  delay that reads as natural redstone-style timing.
- Mining the extended arm (`PISTON_HEAD`) yields the piston body back.
- **v2 (not yet built):** sticky pistons (pull 1 block back on retract), a smooth multi-step
  push animation, and a dedicated directional arm-shape mesh (v1 reuses iron/planks textures).

#### Dispensers + Droppers — SHIPPED 2026-07-04 (gap-fill wave)

`DISPENSER` (305) / `DROPPER` (306), implemented in `dispenser.rs`. A 9-slot container
block that, on a **power rising edge**, ejects its first occupied slot's item out of its
meta-facing side. Dispenser + Arrow → a real arrow projectile (`spawn_arrow`, owner-less);
everything else — and every Dropper eject — tosses an item entity (`spawn_thrown_item`,
0.25 b/t + lift). New dispense behaviours are new arms in `dispenser::eject_decision`.
On a **dedicated server** (MP-A3, 2026-10-06) the arrow is as real as in single-player:
`GameServer::tick` runs the same `entity::tick_projectiles` the client runs (only where
`simulates_block_machines` is set), so it flies, hits the first mob in its path through
`combat::Health` (mobs only, on both sides — a projectile never damages a player), and
reaches joiners as an `EntityKind::Projectile` (Spec 04, "Server projectiles").

**Use-arms (2026-07-05, deferred-lists wave):** the Dispenser *uses* what it can —
- **Filled bucket** → pours the liquid source into the facing AIR cell (registered with
  the water/lava sim exactly like the right-click empty path); the **empty Bucket stays
  in the dispenser**. Facing cell blocked → the bucket is returned unused.
- **Bonemeal** → `growth::bonemeal_advance` on the facing block, same +1–2 stage rule and
  seed idiom as hand use (with a murmur-avalanched seed — the 4-tick cadence keeps raw
  tick parity constant). Mature / not a crop → returned, no waste.
- **Flint & steel / Magnesium Firestarter** → ignites the facing cell through the same
  `FireSystem::ignite` seam as hand use; flint & steel pays durability and breaks at 0,
  the firestarter is reusable. The igniter stays in the dispenser.
- Failed/returned items go back via `dispenser::return_stack` (hopper `dest_slot`
  merge-or-first-empty); if the inventory is full they are tossed, never destroyed.
- Droppers still always toss.

- **State**: `BlockEntityData::Dispenser(DispenserData)` — an embedded 9-slot `ChestData`
  (so all chest-UI helpers + hopper transfers work unchanged) + the `on` rising-edge latch
  (keg idiom; NEVER in `PowerDeviceData`, whose bincode layout is frozen). Created
  **eagerly at place time** (the latch must exist from tick one) with piston-style facing
  stamp (pitch-aware, up/down work).
- **Tick**: `tick_dispensers` on the 4-tick block-sim cadence beside pistons; a held wire
  fires once (latch), a pulse per edge.
- **UI**: right-click opens a 9-slot dialog — `chest_ui::show_container_dialog`, the chest
  dialog engine extracted with a caller-supplied title (rows derive from slot count).
- **Hopper interop**: `hopper::container_at{,_mut}` resolves chest OR dispenser/dropper on
  either end of a hopper — hoppers auto-feed dispensers (the MC auto-farm idiom).
- **Persistence**: `WorldSave.dispensers: Vec<SavedDispenser>` — the NEWEST appended field
  (after `satoshi`); contents + latch round-trip; old saves default empty.
- **Recipes**: cobble ring + Cable at bottom-centre (piston convention); centre **Arrow**
  = Dispenser, centre empty = Dropper (tools can't be ingredients, so the MC bow is
  replaced by the arrow — reads as "the block that shoots"). Catalogue cards added.
- Break spills the 9 slots (`cleanup_dispenser`, chest idiom).

### 7.2 Fluid Simulation

Fluids (water, lava) use a **cellular automaton** flow model, simulated server-side.

**Water**:
- Source blocks are persistent (placed by player or generated).
- Flowing water spreads up to **7 blocks** horizontally from a source on flat ground.
- Water flows downward instantly (one block per tick).
- Water searches for the shortest path to a downward drop (within 4 blocks) and preferentially flows toward it.
- Two flowing water blocks meeting at the same position create a new source block (infinite water source mechanic).
- Water pushes entities in the flow direction at 1.39 b/s.
- Water breaks non-solid blocks it flows into (torches, flowers, redstone wire).

**Lava**:
- Same spreading model as water but **slower**: spreads 1 block every 30 ticks (1.5 seconds) in the overworld. A faster spread rate (e.g., every 10 ticks) is reserved for any future AxeNStax-native fire-rich dimension; the dimension itself is TBD and not part of alpha scope.
- Lava flows **3 blocks** horizontally (vs water's 7).
- Lava does not create source blocks when two flows meet.
- Lava ignites flammable blocks within 1 block.
- Contact with water: lava source + water = obsidian. Flowing lava + water = cobblestone. Water flowing onto lava source from above = obsidian.

**Performance**: Fluid updates are batched and processed with a limited budget per tick. If more than **1024 fluid updates** are pending in a single chunk, excess updates are deferred to the next tick. This prevents cascading fluid events from stalling the server.

#### Water Flow (Step 6 — Simple Spread; depth/flow/push delivered 2026-07-04)

Water source blocks spread to adjacent air blocks (4 cardinal directions + downward).
Horizontal spread limited to 7 blocks from nearest source. Water always tries to
flow down first.

- Spread rate: 5 Hz (every 4 game ticks)
- Budget: 64 blocks per tick to prevent frame spikes
- Removing a source block retracts dependent water via BFS (and clears depth meta)
- World-gen water (oceans, caves) registered as sources on chunk load
- Sand/gravel falling into water displaces it

**Depth levels (2026-07-04).** Every flow cell carries a depth level 0–7 in the
**meta AUX field** (bits 5–7, `meta.rs`): 0 = source or full falling water, 1–7 fades
with horizontal distance. Worldgen ocean cells carry no meta entry (⇒ level 0) so the
greedy top-face merge is preserved (level is part of the merge key, `mesh.rs
greedy_water_face`). Renders at `water_surface_height(level) = (8−level)/9` (source
≈ 0.889 like MC); a cell with water above it renders full-height (`FULL_COLUMN` key).
Levels persist via `WorldSave.block_meta` and reach remote clients in
`BlockChange.meta` (the client apply path now applies meta —
`World::apply_remote_block_change`). **No protocol bump, no chunk-format change.**

**Infinite pool (MC parity, 2026-07-04).** A flow cell with ≥ 2 horizontal source
neighbours is promoted to a source (the classic 2×2 infinite pool),
`water.rs::horizontal_source_neighbours`.

**Flow vector + current push (2026-07-04).** `water::flow_vector(world, x, y, z)
-> Option<[f32;3]>` — pure, unit-length; derived from the depth-level gradient,
pulled toward edge-drops, downward while falling; `None` for still water. Flowing
water pushes entities (`entity::tick_entities`) and swimming players
(`physics.rs` swim branch) by `FLOW_PUSH = 0.02` blocks/tick² horizontally — items
ride streams; you can always swim against the current. **This fn is the designated
query API for the Electricity Phase 4 **Water Wheel** (its first consumer, shipped
2026-09-06 — `power::has_current_neighbour` calls it on the wheel's four horizontal
neighbours and the cell below), boats, and item streams — build against it, do not
re-derive flow.**

**Step-side strips — SHIPPED 2026-07-06 (pets-debt-water wave, water seam
fix).** The v1 seam is closed: side faces previously only emitted against AIR,
so where two flow cells of different level touched, the taller cell's exposed
vertical strip between the two surfaces was never meshed (a see-through gap).
`greedy_water_face` (`mesh.rs`) now runs a second emission pass per horizontal
face — when the neighbour is WATER and this cell's surface is higher than the
neighbour's, it emits a strip spanning `[neighbour_surface, this_surface]`
(a `FULL_COLUMN` neighbour reads full height, so the already-correct submerged
case is untouched). Step strips are per-cell-pair (heights vary) so they don't
greedy-merge — cost stays shoreline-local. No shader/vertex-format/renderer
change (`fs_water` untouched), no save surface.

Not yet implemented: shortest-path-to-drop flow preference, water breaking
non-solid blocks it flows into, waterlogged blocks.

### 7.3 Fire Spread — SHIPPED 2026-07-04 (gap-fill wave)

Implemented in `fire.rs` (`FireSystem`, scheduled-event queue — the leaf-decay
idiom), driven from BOTH `GameState::tick` and `GameServer::tick` on the 5 Hz
fluid cadence. `FIRE` block id 304: non-solid, light 15, no drop, procedural
flame texture (`TEX_FIRE` 449).

- **Ignition**: flint & steel / Magnesium Firestarter right-click on a flammable
  block lights the clicked-face cell (campfire `is_fas` idiom, durability wear);
  lava ignites adjacent exposed flammables (deterministic one-in-12 roll per
  settled lava cell, `ignite_from_lava`).
- **Flammability**: `block::flammable(id)` — derived match over the wood
  families (logs | leaves | planks), the `light_emission` idiom. Extend there,
  never inside FireSystem (mod-API friendly).
- **Spread = consumption**: every ~30 ticks (+ position jitter) a burning cell
  converts one hash-picked adjacent flammable into more fire. Gated by
  **`WorldMeta.fire_spread_enabled`** (default true, `explosives_enabled`
  pattern end-to-end; off = MC `doFireTick` off — ignite/burnout still work).
- **Burnout**: no adjacent fuel → out after ~1 event; absolute age cap 12
  events; **active-cell cap 256** (`MAX_ACTIVE_FIRE`) — a forest fire
  self-limits, no runaway world-burn.
- **Rain douses** (40%/event roll), gated on the server's OWN authoritative
  weather window (`GameServer.weather`, see "Weather sync (P9)" below) — a
  hosted/dedicated world's fire-dousing rain now agrees with the rain every
  client is shown, instead of each side rolling a private (and different)
  window.
- **Keg interop**: fire adjacent to a Blasting Keg lights its fuse
  (`charge = KEG_FUSE_TICKS`); `detonation_permitted` still gates the blast.
- **Contact damage**: standing in fire = 1 HP per 0.5 s (half lava's 2 HP),
  armour-reduced. No lingering "on fire" status yet.
- Persistence: FIRE cells survive as blocks; `register_column_fires` re-adopts
  them on load/stream (lava idiom).

Deferred: burn-time/flammability tiers per block family, lingering after-burn
status, fire-charge/dispenser ignition, animated flame texture (disk-pack
strip), fire spreading diagonally/upward MC-style.

#### Thunderstorms + lightning — SHIPPED 2026-07-05

~1 rain in 3 upgrades to a thunderstorm for the same window
(`weather_storm_until`, transient like the rain timer — nothing saved). While
storming, a strike rolls every 8 s (55%, deterministic tick hash) within ±24
blocks of player 0: a white **sky flash** (decaying boost layered onto the sky
brightness — the P1 light split means terrain relights for the blink), a
spark-column **bolt**, a long **thunder** rumble (`audio::play_thunder`), and
the strike **starts a real fire** at the surface through `FireSystem::ignite`
— so the fire toggle, burnout, rain-douse and cap rules all apply (a storm
usually douses its own fires: rain is falling). The strike roll + presentation
(flash/bolt/thunder/ignite) stay client-side, driven from whichever window the
client currently has (self-rolled in single-player, synced on a joined
client — see below) — this is the known **dual-sim** debt (CLAUDE.md "Single-
player bypasses GameServer"): a remote client's own lightning bolt is not
server-broadcast to other players, and its ignite-through-`FireSystem` call
races the server's own fire sim rather than being routed through it. Left as
is; fixing it is the general dual-sim unification, not a weather-specific fix.

#### Weather sync (P9) — SHIPPED 2026-09-03

**Bug:** the dedicated/hosted server rolled its OWN weather via
`weather::server_raining` — `(tick / 1200)` hashed, a formula that differed
from the client's `weather::advance` (which hashes the tick directly) — so a
hosted world's fire-dousing rain never matched the rain any player actually
saw, and nothing about weather crossed the wire at all: every remote client
rolled its own private window, independently, and so did the server for fire.

**Fix — the server is now the single source of truth:**

- `GameServer` owns a `weather: Weather` field (`server.rs`), advanced every
  tick with the SAME `weather::advance` formula the client uses — `raining_now`
  (the fire-douse gate) is simply `tick_counter < weather.rain_until`.
  `weather::server_raining` is deleted; the old client/server formula
  divergence no longer exists.
- **Wire**: `StateUpdatePacket` (protocol v59) gains
  `rain_ticks_left: u32` + `storm_ticks_left: u32` — from
  `Weather::ticks_left(tick)`, a *duration* (ticks remaining), not the
  server's absolute `rain_until`/`storm_until`. This matters: the sync stays
  correct even when the server's and a client's `tick_counter`s have drifted
  apart (different join times, wrap, etc). `hosted_server.rs`'s
  `broadcast_state` fills both fields on every broadcast.
- **Client apply**: a joined client (`remote_client.is_some()`) does NOT roll
  its own window in `GameState::tick` any more — `network_receive` reads
  `rain_ticks_left`/`storm_ticks_left` off the latest `StateUpdatePacket` and
  reconstructs the absolute window relative to the CLIENT's own tick counter
  via `Weather::from_ticks_left(tick_counter, rain, storm)`, mirroring how
  `reserve_richness`/`reserve_target_sats`/`reserve_current_sats` are applied
  from the same packet. `GameState::tick`'s weather block then just reads that
  synced window back (rather than calling `advance`) before rolling the
  (still client-side/presentation) lightning strike from it.
- **Hosting**: when a player hosts (`HostedServer` driving a `GameServer` from
  the host's own main loop), the HOST's `GameState` stays the sim owner for
  its own local player's weather (unchanged — it still calls `advance` every
  tick, since `remote_client` is `None` for the host). Before each
  `HostedServer::tick()`, `game_loop.rs` pushes the host's current
  `weather_rain_until`/`weather_storm_until` into `hs.server.weather`, so
  remote players receive exactly what the host sees and the server's own
  fire-dousing agrees with the host's rain, not a separately-rolled window.
  The dedicated `--server` path (no host `GameState`) has no one to push from,
  so it just rolls itself via `GameServer.weather`'s own per-tick `advance`.
- **Version compat**: a pre-v59 (v58) server sends no `rain_ticks_left`/
  `storm_ticks_left` at all; the fields are `#[serde(default)]` so those slots
  read as `0`, and a v59 client reads that as permanently clear — an
  acceptable degrade. In practice this is moot: `hosted_server.rs` rejects a
  `protocol_version` mismatch at JOIN, before a `StateUpdatePacket` is ever
  exchanged, so a v58-shaped packet never actually reaches a v59 decoder.
  (bincode's positional decode is, in fact, NOT graceful about a genuinely
  shorter byte stream — `#[serde(default)]` only helps a self-describing
  format like JSON; see `game/engine/src/protocol.rs`'s
  `state_update_from_a_shorter_older_peer_is_rejected_not_defaulted` test and
  `save.rs`'s `read_tail` for the documented gotcha and its real fix pattern,
  not applied here since the version gate already makes it unreachable.)

#### The Workshop has no weather — FIXED 2026-07-31

**Bug (Axolittle, 2026-07-31):** it rained, and lightning struck, inside the
Workshop. The Workshop is an indoor authoring space on a void platform, so
weather over it is nonsense on its face — but a strike is worse than cosmetic:
the bolt ignites through `FireSystem` like any other fire, so a storm could set
light to the parked, half-finished projects the player deliberately left
standing there.

**Correct approach:** both rolls are gated on `world_has_weather(is_workshop)`
— the same hard `is_workshop` guard the mob spawner (`entity.rs`) and Satoshi
(`satoshi.rs`) already use, keyed off the **runtime** `World` flag rather than a
saved meta field so existing Workshop saves are fixed without a migration.

The rolls moved out of the middle of the client tick into `weather.rs` as pure
functions (`advance`, `strike_roll`), which is what makes the exclusion
testable. Guarding at the **source** is what makes it complete: every
downstream consumer — 3D rain particles, the 2D rain overlay, crop watering,
fire dousing, the sky flash — reads `weather_rain_until` / `weather_storm_until`
/ `lightning_flash`, so zeroing those three covers all of them with no
consumer-side special cases. `advance` returns `CLEAR` rather than merely
skipping the roll, which also **cancels a window carried in from the world you
just left**: walking out of a downpour into the Workshop stops the rain at the
door instead of waiting it out. `lightning_flash` is zeroed on the same branch
so entering mid-storm doesn't strobe the sky on the way in.

The dedicated-server leg (`GameServer.weather`, gated the same way via
`world_has_weather(is_workshop)` in `server.rs::tick`) is gated too, or a fire
in a hosted Workshop would be put out by rain that isn't falling. **The
client/server formula divergence this originally documented (`tick / 1200`
hashed vs. hashing the tick directly) was resolved by the "Weather sync (P9)"
work below** — the server now runs the exact same `advance` the client does,
so `server_raining` (and the divergence) no longer exist.

#### Wind — SHIPPED 2026-09-07 (Wind, Copper & Electricity wave)

`wind.rs` is a **pure, deterministic** derived signal — never saved, never
synced, no protocol change — so a hosted world's Windmills behave identically
on the server and on every client. `wind::sample(tick, weather, seed, y)` is a
function of state every side of the wire already agrees on: the tick counter,
the synced weather window (`GameServer.weather`, "Weather sync (P9)" above)
and the world seed, plus the querying position's height.

**Formula:**

- **Base breeze** — two octaves of smooth value noise over `tick` (periods
  ≈2400 and ≈9000 ticks, so a gust builds and dies over 2–7 real minutes, not
  frame to frame), mapped to `0.10..=0.60`.
- **Weather** — `+0.25` while it rains, `+0.50` in a thunderstorm. A storm is
  a subset of the rain window, so these do **not** stack — the storm figure is
  the total.
- **Altitude** — `+0.010` per block above `SEA_LEVEL`, capped at `+0.30`. The
  cap lands **30 blocks up** — inside the alpha world (ceiling `SEA_LEVEL +
  33`), so "build it high" is a real, reachable lever. (The first tuning used
  `+0.004`/block, needing +75 blocks for the cap — outside any legal build, so
  altitude wasn't really a lever at all; retuned before ship.)
- Speed is clamped to `0.0..=1.0`. Direction is slow seeded noise (~5-minute
  period) quantised to the eight compass points; the Windmill itself is
  omnidirectional this wave — direction currently only drives the F3
  read-out and hover label.
- One sea-level sample is taken per tick and lifted per-Windmill via
  `wind::with_altitude` (cheaper than re-running the noise per mill; exactly
  equal to sampling at that height directly, since `altitude_bonus(SEA_LEVEL)`
  is zero).

**Player-facing words** (`wind::word`, hover text + F3 overlay): **calm**
`< 0.20`, **light** `< 0.35`, **fresh** `< 0.60`, **strong** `< 0.85`,
**gale** `≥ 0.85`.

**The Windmill turn rule** (`power::windmill_turns` / `windmill_is_exposed`):

- **Exposed** — clear sky for `WINDMILL_SKY_SCAN` (8) cells directly above the
  mill, **and** at least two of its four horizontal neighbours clear. An
  enclosed mill never turns, however hard it blows.
- **"Clear" is SOLIDITY OR FLUID, not "not AIR."** `blocks_wind` reads the
  block registry's `solid` flag plus `block::is_fluid` (water/lava collide
  like solids but were never `solid`) — so glass, leaves, water, lava and snow
  layers all block the wind, while a Cable, torch or sapling standing on the
  mill's own output face does **not** (found as a real bug: without this, a
  mill could not be wired up through the one face that makes sense).
- **Hysteresis** — `WINDMILL_START = 0.35` to start turning from still,
  `WINDMILL_STOP = 0.30` to stop once turning; the gap stops a lull flickering
  the mill on and off. Sea-level duty cycle on a clear day sits ~0.38–0.51
  across seeds (by design — intermittent); at +30 it is ~1.00 (a hilltop mill
  effectively never stops on a clear day — noted as an Axolittle feel
  question: if a hilltop mill should still stutter occasionally, the lever is
  `ALTITUDE_CAP`, not `ALTITUDE_PER_BLOCK`).
- A transition (still → turning or turning → still) swaps the block to/from
  the `WINDMILL_TURNING` twin exactly once, pushes a `BlockChange`, marks
  dirty and notifies neighbours — the Water Wheel's shape exactly.

**`weather_lock` is a LOCAL-tick pin, not a world rule.** A Trial can pin the
sky (`ScenarioDef.weather_lock` — `"clear"` / `"rain"` / `"storm"`), which is
how a windmill trial guarantees a breeze. It is applied in exactly one place:
the single-player / local client tick, after `weather::advance` and before the
power step samples the wind. `GameServer::tick` knows nothing about it, so on a
**joined** client the v59 weather sync would overwrite the pinned window with the
host's own the next time a `StateUpdatePacket` landed. Trials are therefore run
in a fresh local world (the Trials arena, or `/scenario <token>` in your own),
which is where they are launched from anyway. The pin is re-stamped every tick so
it cannot elapse mid-run, and `weather::apply_lock` drops the residue the moment
the lock is released or changes value — otherwise the last stamp, a full in-game
minute ahead, would have left the player standing in the trial's storm for up to
60 s after it ended. The Workshop still wins over any lock: an indoor room never
has weather, so a trial run in one cannot conjure a storm indoors.

Two block ids (`WINDMILL` 318 / `WINDMILL_TURNING` 319, the Steam-Generator
idle/active twin pattern), `PowerDeviceKind::Windmill` appended last
(bincode-positional). No light emission, no meta byte, no protocol bump for
the block itself. Full design + implementation notes:
`docs/foundations/2026-06-17-electricity-power-logic.md` §5 Phase 4,
`docs/superpowers/specs/2026-09-07-wind-copper-electricity-wave-design.md`.

### 7.4 Crop Growth and Farming

> **Supersession (2026-05-18).** This section was an early Minecraft-clone draft (moisture levels 0-7, 8 growth stages, random-tick model, monoculture penalty, ~24-minute growth, bone-meal force-grow). **None of that shipped.** Tier 1 farming intentionally diverges to a simpler, deterministic model — see §3.9 above for the actual shipped design (4 stages, 200-tick interval, water-adjacency 2× multiplier, no light gate, deterministic per-position growth math, harvest resets to tilled soil). The Minecraft-style moisture / random-tick / monoculture model below is **future / aspirational** and may or may not appear in a later tier; treat it as a reference catalogue of "things Minecraft does that AxeNStax does not (yet)".

~~**Farmland**: Created by using a hoe on dirt or grass. Farmland is a block with a moisture level (0-7). Farmland within **4 blocks** (horizontally, including diagonals) of a water source block has maximum moisture (7). Dry farmland (moisture 0) reverts to dirt after a random interval.~~

~~**Crop growth**:~~
- ~~Crops are planted on farmland and progress through growth stages (typically 8 stages, 0-7).~~
- ~~Each random tick, a crop has a chance to advance one growth stage. The chance depends on:~~
  - ~~Light level (minimum 9 for growth).~~
  - ~~Hydrated farmland (2x growth rate vs dry).~~
  - ~~Surrounding crops (monoculture penalty: same crop adjacent slows growth. Alternating rows is optimal).~~
- ~~Average growth time (wheat, hydrated, optimal layout): ~24 minutes (real time).~~
- ~~Bone meal equivalent forces 1-3 growth stages immediately (with particle effect).~~

~~**Growth is driven by random ticks**: Each game tick, the server selects `randomTickSpeed` (default: 3) random block positions per chunk section (16x16x16) and applies random tick behaviour. This is the same model Minecraft uses. Crop growth, leaf decay, grass spread, and other gradual processes all use this system.~~

#### Leaf Decay (Step 6)

When a log block is broken, leaves within 5 blocks are queued for support checks.
A leaf is supported if BFS through adjacent leaves reaches a log within 4 steps.
Unsupported leaves decay with random 5.4-21.6s delay for natural appearance.

- Check rate: 5 Hz, budget 16 leaves per tick
- Decay rate: 8 leaves per tick
- **Sapling drops SHIPPED 2026-07-04**: ~1 in 20 decayed leaves drops its species'
  sapling (deterministic position+tick hash; `LeafDecaySystem::take_sapling_drops`
  drained by the game loop, which spawns the item entity). Sticks/apples still future.

#### Saplings — plant + grow, SHIPPED 2026-07-04 (gap-fill wave)

The renewable-tree loop, previously data-only. One planted-sapling block per
species (`SAPLING_OAK`..`SAPLING_RUBBER`, ids 307–313, non-solid, shared
`TEX_SAPLING` sprout tinted by block colour):

- **Plant**: sapling materials map via `material_as_placeable_block`; the place
  path gates on the existing `sapling::can_plant_sapling_at` (target AIR + grass/
  dirt below; tilled soil reserved for crops) with a toast on refusal.
- **Grow**: `growth::advance_saplings` on the crop cadence — each sapling rolls a
  deterministic 1-in-30 per scan (~5 min average), light ≥ 9 gate (crops parity);
  a hit consumes the sapling and stamps `tree_shapes::place_tree` (the REAL
  worldgen shapes) writing only into AIR, so builds beside a sapling are safe.
- **Obtain**: leaf-decay drops (above) + breaking a planted sapling returns it.
- Client-loop only today, like crops — moves server-side when crops do.

#### Bee-honey loop — LIVE 2026-07-04 (gap-fill wave)

The fully-tested-but-never-called Spec 28d hive machinery is wired:

- **Wild hives**: ~1 tree in 12 carries a `BEE_HIVE` on its trunk side at gen
  time (deterministic per (x,z,seed)), starting part-stocked (2 honey, 1 bee) —
  THE honey source; no crafting recipe (`bee_hive` block-entity inserted at gen).
- **Accumulate**: every 1200 ticks a hive with a Bee entity within 8 blocks
  gains 1 honey (`bee_hive::bee_within` + `deposit_honey`, cap 5). The full
  enter/leave-hive bee-AI wiring can replace this sweep without touching the
  harvest side.
- **Harvest** (right-click, `resolve_right_click`): Bucket → 1 Honey Jar
  (bucket consumed, milking idiom); Shears → 3 Honeycomb (durability); empty
  hive/hand → guidance toast.
- **Quest closure**: HoneyBottle restored to the Brewer pool (removed 2026-06-22
  as an un-completable dead-end); the obtainability guard test updated.

#### Snow layers — SHIPPED 2026-07-04 (gap-fill wave)

`SNOW_LAYER` (314), `BlockShape::SnowLayer`: a thin bottom slice `(AUX+1)/8`
blocks tall (collision = render). The tundra/mountain snowfall painter now
ACCUMULATES: AIR → 1 layer, existing layer +1, the 8th converts to the full
`SNOW` block. Hand-placed layers start at 1. Still biome-gated + deterministic
(no weather-snow state exists yet).

#### Schematic-UX hints (TB-10 "i dont get it") — SHIPPED 2026-07-04

Two persistent HUD hints replace one 4-second toast: (a) while a **Plan is the
held item** — bottom-centre guidance that switches on aim ("right-click the
GROUND to build" vs a warning that a WALL click hangs it as a picture and
consumes it — the overload that confused the playtest); (b) while the **ghost
placer is active** — controls stay on screen ("Left-click build · Q/E rotate ·
Right-click cancel"). `hud_ui::{plan_hint_text (pure, tested), draw_plan_hint,
draw_ghost_controls}`.

#### Steed persistence — SHIPPED 2026-07-04 (gap-fill wave)

First ride marks a Horse/Donkey/Mule as **kept** (`HorseData.kept_by`, the
rider's slot). Kept steeds persist via `SavedTamedPetData::Steed { kind, data }`
(APPENDED enum variant — positional bincode, never reorder) and respawn on load
like wolves/nostriches/companions. Wild steeds still re-scatter. Previously a
ridden horse silently despawned on save/load.

### 7.5 Day/Night Cycle

| Parameter | Value |
|---|---:|
| Full day/night cycle | 20 minutes (real time) |
| Daytime duration | 10 minutes |
| Nighttime duration | 7 minutes |
| Sunrise/sunset transition | 1.5 minutes each |
| Time at dawn | 0 ticks |
| Time at noon | 6000 ticks |
| Time at sunset | 12000 ticks |
| Time at midnight | 18000 ticks |
| Ticks per cycle | 24000 |

**Light level**: The sky light level varies with time of day. At noon, sky light = 15. At midnight, sky light = 4. This affects:

- **Mob spawning**: Hostile mobs spawn in light level <= 0 (block light + sky light adjusted for time). During the day, surface light is too high. At night, unlit surfaces become spawn-eligible.
- **Crop growth**: Requires light level >= 9.
- **Player visibility**: Affects render brightness (gamma). The client renders the day/night transition smoothly.

**Sleeping**: If all players in a world (or a configurable percentage, e.g., 50%) sleep in beds, the night is skipped and time advances to dawn. Beds also set the player's spawn point. *(As built: a single player's night-time bed click skips to morning, sets their spawn and heals them to full. A joiner in someone else's world asks the server (C2a, Spec 04 §4.2f): at night, once a night, a bed in reach sets its server spawn point and heals it to full, but never skips the night — the clock is the host's until D4.)* Sleeping in a dimension without a day/night cycle (any future AxeNStax-native dimension where day/night is suspended) causes the bed to explode — a classic loot-pinata gag carried forward for the amusement of veteran block-game players.

### 7.6 Rail & Carts (Phase 1 — delivered 2026-06-10)

> Full spec and confirmed wire values: `docs/foundations/2026-06-09-rail-freight-logistics.md`.

Rail is the grounded replacement for "waystones" — timed, physical transport over real
in-world ticks. The same system serves freight haulage and player fast-travel.

**`genesis:track` block (id 260)** — flat, non-solid, transparent rail. Renders as a thin
ground slab. No rotation or facing field; carts auto-navigate by pathing to the single
onward track neighbour. `TEX_TRACK = 373`.

**Cart entity** — engine's first vehicle. ECS: `(Position, CartEntity, CartData)`. Omits
`OnGround`, so `tick_entities` gravity/collision skips it; driven by `tick_carts` (called
after `tick_entities` each tick). `CART_SPEED = 0.08` cells/tick (≈ 1.6 blocks/s).

**Pathing** — pure `next_track_step(at, came_from, is_track)`: finds the single onward
track neighbour. Returns `None` at a **terminus** (dead-end) or a **junction** (> 1
onward neighbour) — carts park at junctions in Phase 1 (no switching).

**Depot** — no dedicated block; a depot is **a chest placed adjacent to a track terminus**.
- *Load on dispatch:* parked cart at a depot chest pulls cargo aboard and departs.
- *Unload on arrival:* cart parking at a terminus with an adjacent chest dumps cargo into
  it. Overflow stays on the cart — freight is never silently lost.

**Ride-along** — right-click a parked cart with an empty hand to mount. Camera follows
the cart for the real transit duration. Rider cannot act at distance while in transit.
Dismount on arrival or jump. `PlayerSlot.riding` is transient (not persisted).

**Persistence** — `CartData` (cell / came\_from / progress / speed / facing / cargo)
survives save/load via `WorldSave.carts: Vec<SavedCart>` (trailing `#[serde(default)]`
field; old saves load with `carts == []`). Track blocks persist via normal chunk data.

**Multiplayer broadcast** — carts emit as `EntityKind::Cart` (= 26) via the entity diff
(`entity_broadcast` since v68: spawn when a joiner's interest takes it in, then changed-only updates). `PROTOCOL_VERSION` bumped to **45** for this addition.
Two-client visual rendering from the receive path = playtest boundary (same systemic
deferred gap as mob remote rendering).

**Transit robbery + hostile-act ledger (Phase 3, 2026-06-19)** — breaching a cart **in
transit** (`CartData.speed > 0`) is a robbery: the cart breaks open + spills its cargo
(the shipping breach mechanic) AND the act is recorded to the new **`World.hostile_acts`**
ledger (`hostile_acts::HostileActLedger`, `HostileActKind::CartRobbery`), persisted append-only
as `WorldSave.hostile_acts`. Breaking a **parked** cart at a depot stays the owner reclaiming
it — no record. The reputation hit + bounty fire + per-victim/perpetrator identity need the
multiplayer-identity work (Spec 1 Phase 4); the ledger is the audit substrate they'll read.

**Commercial freight — Bulk Vendor depot sale (Phase 2, 2026-06-19)** — a **Bulk** vendor sources
its wholesale stock from an adjacent **freight depot chest** (the cart's unload destination): on
open, `vendor::restock_bulk_from_depot` moves matching freight into the vendor's `stock`, then lots
sell through the existing `try_buy` → `escrow_sats` path — completing load→haul→unload→**pay**.
Single-item `Sell` vendors stay pocket-fed. Earnings are the in-world escrow score; real
cross-boundary Lightning settlement stays owner economics (untouched). **Still gated:** Phase 4
Aether security (info/flagging only) — gated on the Aether tier.

**Vendor stock accounting (2026-09-27 audit fix).** `stock` means different things per mode, so
it never crosses a mode boundary raw. **Bulk:** `slot` is a *template* the sale never consumes and
`stock` is the real unit count (deposits + depot freight); the template clears only when the last
unit sells (`vendor::apply_sale`), so freight stays sellable after the first lot. **Sell / Barter:**
`stock` equals `slot.count`. **Plan modes:** `stock` is a licence counter (or the `UNLIMITED_STOCK`
sentinel), never an item count. `vendor::change_mode` reconciles `stock` with the real slot items on
every mode switch (so the Unlimited sentinel can't leak into Sell, and Bulk freight carries into the
new slot; a switch is refused if the stock won't fit one slot). Breaking or withdrawing spills
`vendor::stocked_items` — the real stored count, never the sentinel — and whatever doesn't fit the
owner's inventory drops as item entities at the vendor. Nothing is deleted. A vendor saved in Bulk by
pre-fix code with a Plan template has its leftover licence count clamped to the Plan held on load
(`vendor::repair_legacy_bulk`). A **chance drop** (Satori, or any item crafted from it through the
recipe graph) can't be listed in any sats mode or bid on for sats — Barter only (Spec 06 §2.2c.1).

**Craftable carts + hull armour (2026-06-10)** — delivered as a same-day follow-up.
Carts are now craftable in three hull tiers and placed from an item: right-click a track
cell with a Cart item (`can_edit_world`, no cart present) to spawn a parked cart of that
tier. Hull tiers: **Wood Cart** (5 planks) → **Iron Cart** (8 iron + 1 Wood Cart) →
**Diamond Cart** (8 diamond + 1 Iron Cart). Track now has a survival recipe: 6 Iron Ingot
+ 1 Stick → 16 Track. Breaking a track cell with a cart present **breaches the cart**
instead of the rail — breach time by hull: Wood ≈ 20 ticks / Iron ≈ 60 / Diamond ≈ 120;
on break the cart item and cargo drop, the track stays. This realises the Phase 3 armour
concept early and fills the cart-pickup gap. `PROTOCOL_VERSION` bumped 45 → 46
(`CartData.hull` appended to save; wire unchanged). Full detail and confirmed
`MaterialId`s: `docs/foundations/2026-06-09-rail-freight-logistics.md`.

---

## 8. Game Modes

### 8.0 Implementation status (2026-06-09)

The four modes below are the **target**. What's wired in the engine today:

- **Code type:** `PlayMode` (`game/engine/src/play_mode.rs`) — Survival / Creative / Adventure / Spectator. Named `PlayMode`, **not** `GameMode` (which is the top-level UI state machine: Splash/Menu/Loading/Playing/Paused). `PlayMode` derives the predicates the engine branches on: `is_creative()`, `is_survival()`, `can_edit_world()` (Survival|Creative), `flies()` (Creative|Spectator), `noclip()` (Spectator), plus `as_meta_str()`/`from_meta_str()` for persistence (`WorldMeta.game_mode`, unknown→Survival).
- **Source of truth + cache:** `play_mode` is the single source of truth on `GameState` and `GameServer`. The legacy `is_creative: bool` survives as a **single-writer cached projection** of `play_mode == Creative`, written ONLY via `set_play_mode()` (so the ~130 read sites stay valid and can't drift). The `/gamemode` command and the join-protocol consumer set it through the same choke point.
- **Granularity — world-level (BRIDGE):** today one mode applies to the whole world/session (matching the flag it replaced). §8.4's **per-player** target (`PlayerSlot.play_mode` + `/gamemode <mode> [player]`) is the documented follow-up; trigger: 3-tier Spectator / tournament work, which needs one player spectating while others play.
- **Adventure (shipped, v1 = default-deny):** the world is read-only to the player — `can_edit_world()` gates the break-commit path, the generic place path, AND the item-use placement paths (papyrus reed, crop seeds, cyanotype print). The per-block `CanDestroy`/`CanPlaceOn` **allowlist** in the §8.1 row is the reserved target, **not yet built** (v1 denies all break/place). Other interactions (doors, containers, mobs, trade) remain ungated — Adventure players can still interact, just not edit blocks. **Airtightness follow-up:** block-*modifying* interactions that aren't placements — bonemeal growth, hoe-tilling — and blueprint/Drafting-Stamp paste are not yet mode-gated.
- **Spectator (shipped = local form):** noclip fly (`flies()` + `noclip()` skips collision) and no break/place. The full **3-tier networked spectator** (§8.3: entity-POV view, invisibility-to-others, teleport UI, and the cost-optimised "no simulation" path) is **Spec 04 / deferred** — it needs the per-player model and the networked spectator transport.
- **Switching (shipped):** `/gamemode survival|creative|adventure|spectator` (aliases `s|c|a|sp`, plus `0|1|2|3`). The `[player]` argument in §8.4 is **deferred** (modes are world-level today). Entering Creative trips the ledger (`ever_creative`, breaks `pure_survival`); Adventure and Spectator do **not** trip the ledger.
- **Persistence/protocol:** `WorldMeta.game_mode` is the JSON string (`survival`/`creative`/`adventure`/`spectator`); `JoinAcceptPacket`/`ServerAnnouncePacket` carry `play_mode` (with `is_creative` kept as the derived-on-wire projection). `PROTOCOL_VERSION` bumped on the wire-format change.

### 8.1 Mode Definitions

| Mode | Description | Inventory | Damage | Mob Interaction | Block Access |
|---|---|---|---|---|---|
| **Survival** | The core experience. Gather, craft, build, fight. | Normal (limited, must gather) | Takes and deals damage | Full (mobs hostile/passive) | Mine and place (costs durability, consumes items) |
| **Creative** | Unlimited building. All blocks available. Flight. | Creative inventory (all items, unlimited stacks) | Invulnerable | Can attack mobs, mobs do not target | Instant break, unlimited placement |
| **Spectator** | Observe the world without interacting. | None (no inventory access) | Invulnerable, invisible | Cannot interact, mobs ignore | Cannot break or place |
| **Adventure** | Explore without modifying. For custom maps. | Normal | Takes and deals damage | Full | Cannot break blocks (unless tool has `CanDestroy` tag). Cannot place blocks (unless block has `CanPlaceOn` tag). |

### 8.2 Creative Mode Details

- **Creative inventory**: A searchable panel containing every registered block and item, organised by category tabs (Building, Decoration, Redstone, Combat, Food, Tools, Misc).
- **Instant break**: All blocks break in one tick regardless of tool. No durability cost.
- **Unlimited placement**: Placing a block does not decrement the stack.
- **Flying**: Enabled by default (double-tap jump). See section 1.7.
- **No fall damage, no fire damage, no drowning**: The player takes no environmental damage.
- **Mob AI**: Hostile mobs do not target creative players. The player can still attack mobs.
- **Pick block (middle-click)**: Adds the targeted block to the hotbar, even if not currently in inventory.
- **Reach**: 5.0 blocks, the same as survival (the code has one `REACH_DISTANCE`).

### 8.3 Spectator Mode Details

- **No-clip movement**: Spectators pass through all blocks. Movement speed matches creative flight.
- **Entity view**: Spectators can left-click an entity to view the world from its perspective.
- **Invisibility**: Spectators are invisible to all non-spectator players and all mobs.
- **No interaction**: Cannot open containers, use items, or trigger pressure plates.
- **Teleportation**: Can teleport to any player via a UI menu.
- **Cost optimisation**: Spectators generate minimal server load (see Platform Overview, section 5.3). They receive chunk data but do not trigger mob spawning, physics updates, or block tick events. This is critical for event scaling — thousands of spectators should not proportionally increase simulation cost.

### 8.4 Mode Switching

- **Permissions**: Mode switching requires the `gamemode` permission (default: operators only).
- **Command**: `/gamemode survival|creative|spectator|adventure [player]`.
- **Per-player**: Each player has an independent game mode. A survival player and a creative player can coexist in the same world.
- **Switching effects**:
  - Survival -> Creative: Inventory is preserved. Flight is enabled.
  - Creative -> Survival: Inventory is preserved (creative items remain but no longer replenish). Flight is disabled (potential fall damage).
  - Any -> Spectator: Inventory is hidden (not deleted). Player becomes invisible and no-clip.
  - Spectator -> Any: Inventory is restored. Player is placed at current position (or the nearest safe block if inside a wall).

### 8.5 World Integrity Ledger

Every world carries an append-only integrity record in `world_meta.json`. These fields are **one-way doors** — once set, they never revert. The ledger exists to make a world's history transparent without restricting gameplay.

| Field | Type | Default | Behaviour |
|---|---|---|---|
| `pure_survival` | `bool` | `true` | Flips to `false` permanently if creative mode or cheats are ever used |
| `ever_creative` | `bool` | `false` | Flips to `true` permanently if creative mode is ever activated |
| `cheats_used` | `bool` | `false` | Flips to `true` permanently if any command is ever used |
| `difficulty` | `String` | `"normal"` | Current difficulty: `peaceful`, `easy`, `normal`, `hard` |
| `difficulty_history` | `Vec<{level, timestamp}>` | `[]` | Append-only log of every difficulty change |
| `forked_from` | `Option<String>` | `None` | Parent world folder name (set when world is forked) |

**Fork inheritance**: When a world is forked, ALL integrity fields are copied from the parent. The fork's `forked_from` field points to the parent. A fork of a creative-touched world is also creative-touched — there is no clean-slate workaround.

**Difficulty table** (built 2026-10-05, W2) — one data table, `survival::DIFFICULTY_TABLE`; nothing else branches on the difficulty string:

| Difficulty | Hostiles attack? | Mob → player damage `d` (before armour) | Starvation floor |
|---|---|---|---|
| Peaceful | No | ×1 (neutral bites — bee, goat, shark — unchanged) | 0.5 HP (unchanged) |
| Easy | Yes | `min(d / 2 + 1, d)` | 10 HP |
| Normal (default) | Yes | ×1 | 1 HP |
| Hard | Yes | ×1.5 | none — starvation kills |

Applied to hostile melee (`combat::tick_mob_attacks`) and the neutral bee sting / goat charge / shark bite. Not applied to fall damage, drowning, lava, fire or explosions. A joined client adopts the host's difficulty from `JoinAccept.difficulty` (the field was always on the wire — no protocol change). Server-side, `GameServer.difficulty` (from `WorldMeta` on `initial_load`) scales the hostile melee it lands on joiners (§6.4.1) and turns it off on Peaceful. It runs no hunger or starvation for any player's copy since MP-D2a — a joiner's metabolism is its client's, applying the floor from the joined world's difficulty, and arrives as `InputPacket.health_delta`; a host's local slot is health-trusted (its client writes the health) — which retired the old non-lethal starvation-floor BRIDGE.

**Bitcoin integration** (Phase 5): The integrity ledger provides the data layer for reward eligibility decisions. Server operators will configure policies (e.g., "creative worlds earn zero Bitcoin", "hard mode gets 2x multiplier"). The engine records facts; the server applies rules; the protocol delivers rewards.

### 8.6 Modifiers (orthogonal to mode)

These compose **with** a `PlayMode` rather than being modes themselves. Keeping them as axes (not a combinatorial explosion of modes) is deliberate.

- **Difficulty** (`peaceful`→`hard`) — a `WorldMeta.difficulty` field, independent of mode. See §8.5.
- **Hardcore / permadeath** — a *survival modifier* (a `permadeath` flag), **not** a 5th mode. **Not yet built** — it needs a player death/respawn system to key on. When built, it rides on `PlayMode::Survival`, not a parallel mode.
- **Bitcoin-enabled** — a *server policy*, not a player mode. See §8.5 / Spec 06.
- **Scenario overlay** (e.g. Hash Dash, Satori Rush) — runs **on top of** Adventure or Survival via the Scenario runner; it is not a `PlayMode`. A scenario may pin the mode (e.g. lock to Creative is already gated by `lock_creative`), but the scenario itself is the overlay, the `PlayMode` is the substrate.

---

## 9. Mob System

#### Entity System (Step 7 — Foundation)

hecs ECS stores all mob entities with components: Position, Velocity, MobKind,
Hitbox, OnGround. Entity physics runs at 20 TPS (gravity, ground collision).

Initial mob types rendered as coloured cubes:
- Cow: 0.9x1.4x0.9, brown, 10 HP
- Brigand: 0.6x1.95x0.6, tunic-clad humanoid, 20 HP
- Chicken: 0.4x0.7x0.4, white, 4 HP

Test spawning: ~1 mob per 4 chunks, hash-deterministic placement on terrain surface.
Weighted distribution: 50% cow, 30% chicken, 20% Brigand.

#### Mob AI (Step 8 — Idle, Wander, Chase)

Hierarchical state machine per mob, ticked at 20 TPS:
- **Idle**: Stand still for 60-160 ticks (3-8 seconds), then pick wander target.
- **Wander**: Walk in a straight line toward a random point within 8 blocks. Return to Idle on arrival or if stuck (wall, cliff edge).
- **Chase** (hostile only): Walk directly toward player. Triggered when player is within 16 blocks. Give up at 32 blocks.

Movement is direct-line (no A* pathfinding). Mobs follow terrain with 1-block step-up. Passive mobs avoid edges; hostile mobs drop up to 3 blocks.

Speeds: Cow 0.7 b/s, Brigand 1.1 b/s, Chicken 0.5 b/s. (Tuned down for better feel.)

Scan interval: Hostile mobs check for player every 10 ticks (0.5s).

Since built: a flee state (`AiState::Flee`, prey bolt when struck) and a chase state. Still not implemented: A* pathfinding (movement is direct-line with ground following, `mob_ai.rs`), line-of-sight and group behaviour.

#### Textured Mob Models (Step 9)

Multi-cuboid entity models with per-face procedural textures and walk animation.

Each mob is defined as a list of `ModelPart` structs: origin, size, pivot point, per-face texture layers, animation flag + phase offset. Models defined in `entity_model.rs`:
- Cow: 6 parts (body, head, 4 legs)
- Brigand: 6 parts (torso, head, 2 arms, 2 legs)
- Chicken: 6 parts (body, head, 2 wings, 2 legs)

Rendering: Entity pipeline reuses the chunk shader (lighting, fog, textures) but with `cull_mode: None` (no back-face culling — required because yaw rotation can flip winding). 18 procedural mob textures added to the block texture array (layers 16-33).

Walk animation: Animated parts rotate around their pivot on the X axis. Frequency 2.5 Hz, amplitude ±0.4 radians. Phase offset alternates legs (0.0 / 0.5).

Entity hide distance: Entities within 0.5 blocks of camera are not rendered (prevents seeing insides).

#### Combat System (Step 10)

**Player health:** 20 HP (10 hearts). Displayed as red squares above the hotbar via the crosshair pipeline. Half-hearts shown as half-width squares.

**Melee attack (left click):**
- Reach: 3.0 blocks
- Fist damage: 1.0
- Cooldown: 10 ticks (0.5 seconds)
- Critical hit: 1.5x damage when falling (not on ground)
- Knockback: 0.4 blocks horizontal + 0.3 upward pop. +0.4 extra when sprinting.
- Target selection: Closest entity along look direction with dot product > 0.5.

**Entity health:** Health component (current, max, flash_timer, invincible_timer). Damage flash: all vertex normals overridden to [0,1,0] for 6 ticks (bright white flash). Invincibility: 10 ticks after being hit.

**Hostile contact damage:** 3.0 damage when player within 1.5 blocks horizontally and overlapping vertically. Uses same invincibility/knockback system.

**Death & respawn:** Player dies at 0 HP → 40-tick delay (2 seconds) → auto-respawn at world spawn with full health. Dead entities despawned from ECS.

**Player-entity collision:** AABB push-apart on X/Z axes. Player is pushed out of mob hitboxes each tick (prevents walking through mobs).

Left click priority: Entity attack first, fall through to block break if no entity hit.

Since built: armour (`armour.rs`, `PlayerSlot.armour_slots`), hunger (`combat.rs`), item drops on death (`death_drops.rs`) and a death screen (`hud_ui::draw_death_screen`). Weapons exist as tools (e.g. `ToolType::Sword`).

#### World Persistence (Step 11)

Simple prototype format (production region files are future):
- `worlds/<folder>/world.dat` — bincode-serialized `WorldSave` struct: seed, player position, health, hotbar slot, inventory (36 slots as block_id + count).
- `worlds/<folder>/world_meta.json` — JSON: display name, description, game mode, creation date, icon (reserved). See World Metadata below.
- `worlds/<folder>/chunks/<cx>_<cy>_<cz>.chunk` — raw little-endian u16 array (4096 blocks × 2 bytes = 8192 bytes per chunk). Only non-empty chunks saved.

`Chunk::as_bytes()` / `Chunk::from_bytes()` for serialization.

**Save triggers:** Escape→quit, window close. **Load triggers:** On initial_load if save exists.

On load: restore player state, mark loaded columns, register water sources, generate missing columns within render distance, mesh everything, scatter mobs.

Since built: an autosave timer and entity / block-entity persistence (`WorldSave`, see Spec 02 as-built banner). Still not implemented: region files, zstd compression, CRC checksums.

##### World Metadata (`world_meta.json`)

```json
{
  "display_name": "My Castle World",
  "description": "A massive castle project with underground mines",
  "game_mode": "survival",
  "created_at": "2026-03-28T14:30:00Z",
  "icon": null
}
```

- `display_name` — player-facing name, independent of the folder name on disk. Renaming changes only this field, never the folder.
- `description` — optional short text shown on the world card in the menu.
- `game_mode` — `"survival"` or `"creative"`. Displayed as a badge. Default: `"survival"`.
- `created_at` — ISO 8601 timestamp, set once at creation.
- `icon` — reserved for future custom world icons/thumbnails. Always `null` for now.

**Backward compatibility:** Existing worlds without `world_meta.json` get auto-generated defaults on menu scan: folder name becomes display name, empty description, `"survival"` game mode, `world.dat` mtime as creation date. Auto-generated metadata is not written to disk until the player edits something.

**Derived data (not stored):** Last played = `world.dat` mtime. World size = sum of all files in the directory. Both computed at menu load time.

##### World Entry (Menu Display)

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct WorldMeta {
    pub display_name: String,
    pub description: String,
    pub game_mode: String,
    pub created_at: String,
    pub icon: Option<String>,
}

pub struct WorldEntry {
    pub folder_name: String,
    pub meta: WorldMeta,
    pub last_played: std::time::SystemTime,
    pub size_bytes: u64,
}
```

#### World Select Menu + Pause Menu (Step 12)

All menus rendered via **egui** (immediate-mode GUI). See Spec 03 Section 8 for the UI rendering architecture and design philosophy. The UI is modern and clean — anti-aliased fonts, smooth panels, thematic colours — not pixel-art.

##### Main Menu — World List (`menu.rs`)

Dark slate background (`#0d121c`), "AXE'N'STAX" title in gold accent, "Your Worlds" subtitle.

Each world is a **card** showing:
- Display name (large, white text)
- Game mode badge (green pill "SURVIVAL" or blue pill "CREATIVE")
- Description (small grey text, or italic "No description" if empty)
- Last played (relative time: "2 hours ago", "Yesterday", "3 days ago")
- World size ("27 MB")

Cards sorted by last played (most recent first).

**Selection and actions:**
- Click a card to select it (blue border highlight, action bar revealed).
- Double-click to play immediately.
- Action bar on selected card: **Play** (green, prominent), **Edit** (name + description dialog), **Fork** (duplicate world), **Delete** (red, type-to-confirm).
- "+ Create New World" button below the list (dashed border).

**Empty state:** If no worlds exist, centred "No worlds yet" message with prominent "Create Your First World" button.

##### Create New World Dialog

Modal overlay with:
- **World Name** — text input, required. Placeholder: "My New World". Becomes `display_name` in metadata and sanitised folder name on disk.
- **Seed** — text input, optional. Placeholder: "Leave blank for random". Hashed to u32 for world generation.
- Buttons: **Create & Play** (green), **Cancel**.

Folder name: lowercased, spaces to underscores, non-alphanumeric stripped, truncated to 32 chars. Collision appends `_2`, `_3`, etc.

##### World Actions (from main menu)

- **Play** — load and enter the world.
- **Edit** — modal dialog with name (pre-filled) and description (pre-filled) fields. Changes `world_meta.json` only. Folder name unchanged.
- **Fork** — duplicates entire world directory to a new folder. New world gets display name `"<original> (fork)"`, description `"Forked from <original>"`, fresh `created_at`. Brief "Forking..." status on card.
- **Delete** — type-to-confirm modal:
  1. Warning: "This will permanently delete **"<name>"** and all its data. This cannot be undone."
  2. Text input: "Type **<name>** to confirm" (must match exactly, case-sensitive).
  3. "Delete Forever" button disabled until text matches.
  4. Cancel button.
  This pattern prevents accidental deletion — especially important for younger players. Muscle memory cannot delete a world.

##### Pause Menu (in-game)

Semi-transparent dark overlay. Three buttons only:
1. **Resume** (green)
2. **Save** (blue)
3. **Save and Quit** (orange, → main menu)

Delete is deliberately absent from the pause menu. Destructive actions belong in the main menu where you can see all worlds and make a deliberate choice, not in the heat of gameplay.

##### Menu State Machine

```
GameMode::Menu { worlds, selected, dialog }
  dialog variants:
    None              — world list visible
    CreateDialog      — name + seed input
    EditDialog        — name + description input for selected world
    DeleteDialog      — type-to-confirm for selected world
    Forking           — brief progress indicator (auto-dismisses)

GameMode::Playing
GameMode::Paused
```

Esc from dialog → close dialog, return to world list.
Esc from world list → quit game.
Esc from Playing → Paused.
Esc from Paused → resume Playing.

**Tick timing reset**: Whenever the game transitions to `GameMode::Playing` (from menu, from pause, from any UI state), the tick accumulator must be zeroed and `last_tick` reset to `Instant::now()`. Without this, time accumulated during pause/menu causes the tick loop to run up to 10 ticks in a single frame on resume, making everything run in fast-forward. This was a real bug — `reset_tick_timing()` must be called at every transition into Playing.

Current world folder name stored in `Mutex<Option<String>>` static. Cursor captured in Playing, free in Menu/Paused.

##### HUD (during gameplay, `hud_ui.rs`)

- Hotbar: 9 slots at bottom centre, styled with slate backgrounds and selection highlight. Item icons rendered from block texture atlas as egui textures. Stack counts in clean anti-aliased text.
- Hearts: above hotbar, red for full, grey for empty, half-heart support.
- Block name: held item name displayed above hotbar in clean text.
- Debug overlay (F3): position, chunk info, FPS, world time — left-aligned, semi-transparent background.
- Crosshair: rendered via dedicated GPU pipeline (not egui) — see Spec 03 Section 8.6.

##### Crafting UI (`craft_ui.rs`)

- 2×2 (player inventory) or 3×3 (crafting table) grid with result slot
- Arrow indicator between grid and result
- Full inventory grid below with hotbar and 27 main slots
- Drag cursor item follows mouse
- All rendered via egui with block textures as managed textures
- Drawing, hover and open state only: every item move is a `WindowClick` applied by `window::apply` (§3.6, "One window model")

##### Touch platforms + Android (`touch_input.rs`, 2026-10-03)

- **One gate.** `touch_input::TOUCH_PLATFORM` (`cfg!(any(wasm32, android))`) gates
  every touch site: event intake (`WindowEvent::Touch`), resize/pixels-per-point,
  the 20 Hz tick and 60 Hz frame intent merges, hold-to-zoom, touch look, the
  pause/chat buttons, the overlay draw, and per-frame edge clearing. Never add a
  bare `#[cfg(target_arch = "wasm32")]` touch gate: if one site drifts the game
  draws controls it never reads, or replays every tap because edges are never
  cleared. `GameState.touch` exists on every target for the same reason.
- `android_main` calls `set_touch_device(true)`, so the on-screen controls show
  immediately (the web sets it from `navigator.maxTouchPoints`).
- **Text entry.** `OS_KEYBOARD_PROMPT` is web-only (`window.prompt()`). On
  Android there is no IME bridge yet, so menu text fields stay egui `TextEdit`s
  (usable with a hardware keyboard) and the touch Chat button does nothing.
  BRIDGE until a soft-keyboard bridge lands.
- **Back = Escape.** Android's Back arrives as `PhysicalKey::Unidentified` +
  `NamedKey::BrowserBack`; `lib.rs` maps it to `KeyCode::Escape`, so it runs the
  same layered close (menu dialog → explorer → recipe book → inventory →
  villager → build choice → any open panel → pause; quit only from the lobby
  root). The manifest's `android:enableOnBackInvokedCallback="false"` is
  required: with targetSdk 36 on Android 16+, predictive back otherwise finishes
  the Activity without ever dispatching KEYCODE_BACK.
- **Widget + dialog sizing.** On Android only, egui uses thumb-sized targets
  (interact height 44, wider non-floating scrollbars). `draw_dialog_frame`
  switches to a compact card — height-capped, scrolling under a pinned title
  with an always-visible bar, width-capped — on Android **or** any viewport
  under 620 points tall (a landscape phone is ~411 points). Taller desktop and
  browser windows render exactly as before. Dialogs that bypass
  `draw_dialog_frame` (egui `Window`s such as Relays and Playing online) set
  `.vscroll(true)` on Android so their buttons stay reachable.

#### Day-Night Cycle + Mob Spawning (Step 13)

World time: 0-23999 ticks, wraps. 0=sunrise, 6000=noon, 12000=sunset, 18000=midnight.
24000 ticks = 20 minutes real time per full day.

Sun direction computed from world time via sinusoidal elevation. Passed to shader as
`camera.sun_dir` uniform (xyz=direction, w=sky brightness 0.05-1.0). Ambient light
and fog colour scale with brightness. Sky clear colour transitions blue→dark.

Mob spawning: every 400 ticks (20 seconds), when brightness < 0.3 (night), try to
spawn up to 3 Brigands 24-64 blocks from player on solid ground. Soft mob cap of 80.

**Passive wildlife (column scatter) is reclaimed on chunk-column unload.**
`scatter_mobs_in_column` deterministically spawns passive wildlife and tags each
with the `Scattered` marker. When a column unloads, `despawn_mobs_in_column`
removes its `Scattered` mobs, so re-entering re-scatters the *same* deterministic
set rather than piling new mobs on the old — the latter grew passive-mob counts
without bound (the 80-cap above is hostile-only; engine audit 2026-06-04, B).
Villagers, iron golems and tamed pets are **not** `Scattered`, so they persist
across unload. A global `MAX_SCATTERED_MOBS` (512) backstops the count against a
teleport outrunning the unload pass. Regressions:
`entity::despawn_mobs_in_column_removes_only_scattered_mobs_in_that_column`,
`entity::reloading_a_column_does_not_accumulate_scattered_mobs`.

**Scatter surface = the real column top, not generator noise (#129, 2026-06-22).**
`scatter_mobs_in_column` previously placed land animals at
`BiomeGenerator::terrain_height` — which is sea-level *noise* (~`SEA_LEVEL` 62),
**not** the actual top of the column. Any world whose real floor differs buried
animals in the ground: authored worlds (the 600 Billion world's y64 floor) and
flat worlds (y79 floor) both spawned passives ~2–15 blocks underground. Fix: a
land spawn now uses `entity::real_surface_y`, which scans down from the world
ceiling to the first non-air block (mirroring the night/hostile spawner in
`spawning.rs`); **Ocean** keeps `terrain_height` (the seabed) so Squid still
spawns inside the water column at `surface + 1`. The existing AIR / WATER /
below-sea-level guards are unchanged. Regression:
`entity::real_surface_y_finds_authored_floor_not_noise_height`.

**Authored placed animals (#129 request, 2026-06-22).** A world-builder can pin
a *guaranteed* wild animal — e.g. a donkey by the donkey statue — with
**`/place <mob>`** (alias `/placemob`, `/anchor`; commands-gated). It spawns the
mob beside the player tagged with the `entity::Authored` ECS marker (no
`Scattered` marker → it survives chunk unload like a tamed pet, but it has no
owner). Authored mobs **persist in the `.axeworld`** by riding the existing
tamed-pet list: `tamed_mobs_to_saved` collects every `Authored`-marked mob into
`WorldSave.saved_mobs` as `SavedTamedPetData::Authored { kind }` (a new,
last-appended enum variant — discriminant-safe), and `chunk_stream::initial_load`
respawns each on world entry and re-attaches the marker. So a placed creature
reappears every load (even if killed in a prior session). Native + web both
persist it (shared `tamed_mobs_to_saved`). Mob names resolve via the safe
`MobType::from_name` (full roster incl. donkey/mule). Regressions:
`commands::builtins::place::places_donkey_as_authored_mob_offset_from_player`,
`save::tamed_mobs_to_saved_extracts_authored_placed_mobs`.

F3 debug overlay shows HH:MM game time.

#### Tools and Crafting Foundation (Step 14)

Tool system in `crafting.rs`: ToolMaterial (Wood/Stone/Iron/Diamond) × ToolType
(Pickaxe/Axe/Sword/Shovel). Each has durability, attack damage, mining speed multiplier.

Weapon damage: Wood Sword 4, Stone 5, Iron 6, Diamond 7. Axes: 7/7/7/9.
Durability: Wood 59, Stone 131, Iron 250, Diamond 1561.

Mining speed multipliers: Wood 2x, Stone 4x, Iron 6x, Diamond 8x (only when correct
tool type matches block). Block hardness values defined per block type.

Crafting recipes (stub): only log→planks implemented. Full 3×3 grid recipes coming
with crafting table UI.

#### Unified Item System (Step 15)

`item.rs` defines the core `Item` enum: `Block(BlockId)` or `Tool(Tool)`. Future
variants: Food, Armour, etc.

`ItemStack` wraps Item + count. Blocks stack to 64, tools stack to 1.

Inventory rewritten to use `ItemStack`. Tools and blocks share the same inventory.

**Starting inventory:**
- **Survival**: empty. Players must mine and craft to acquire items.
- **Creative**: hotbar populated with a starter block set (stone, dirt, grass, oak log, oak planks, cobblestone, sand, sandstone, crafting table) by `chunk_stream::initial_load`. Future change: replace this with a creative item picker so the hotbar isn't pre-spent.

`held_tool` field removed — the selected hotbar slot IS the held item. Attack damage
reads from `inventory.hotbar_attack_damage(slot)`. Tool durability via
`inventory.use_hotbar_tool(slot)`.

Save format: **resolved** — all item types (blocks, tools, materials, armour) are serialised with full data; legacy block-only saves auto-upgrade on load.

### 4.5 Slash Commands

Chat-driven slash commands available during play. Press **T** to open the chat
overlay; **/** opens it pre-filled with `/`. Esc closes without submitting,
Enter submits, ↑/↓ walks the history. While the chat is open, gameplay input is
suspended (no walking, looking, or hotbar input — keystrokes go to egui).

Spec: `docs/foundations/2026-05-07-engine-commands.md`. Module:
`game/engine/src/commands/` (parser, registry, dispatcher) + `chat_ui.rs`
(overlay). Plugin-shaped: registry + parser + UI are game-agnostic. Built-in
commands assume the AxeNStax voxel surface; lifting to other games means
swapping `builtins/` and the `CommandContext` field types.

Built-in surface (v1, single-player only — multiplayer dispatch is Phase 5,
gated behind the Spec 1 + Spec 2 `hosted_server.rs` queue):

| Command | Op | Cheat? | Description |
|---------|:--:|:------:|-------------|
| `/help [cmd]` | — | no | List commands or show details for one |
| `/time get` | Op | no | Show current world time + speed |
| `/time set <day\|noon\|night\|midnight\|N>` | Op | yes | Set world time |
| `/time speed <1..64>` | Op | yes | Set day-length multiplier (1=20min, 4=alpha-fast 5min) |
| `/gamemode <survival\|creative\|adventure\|spectator>` (alias `/gm`, shorthands `s\|c\|a\|sp`) | Op | yes | Set world play mode (world-level; per-`[player]` deferred) |
| `/tp <x> <y> <z>` | Op | yes | Teleport self |
| `/seed` | — | no | Show world seed |
| `/clear` | Op | yes | Empty inventory |
| `/give <item> [count]` | Op | yes | Add item to inventory |
| `/bug <what went wrong>` | — | no | Native only: queue a bug report to the makers |
| `/idea <your idea>` | — | no | Native only: queue an idea to the makers |

`/bug` and `/idea` (and `/mailbox`) are **off by default** — an alpha-tester feature behind
a hidden Settings unlock (tap the version line 7 times); when off they are hidden from
`/help` and answer as unknown commands (2026-10-03, `native_mailbox::feedback_enabled`).
They are the feedback commands, **native only**: the report goes to
the on-disk native outbox (`native_mailbox`) and is flushed as a NIP-17 DM. The
**browser build has no feedback channel at all** (removed 2026-10-01, owner
decision): the commands are not compiled on wasm32, so on the web they are
unknown commands; there is no lobby composer and no `/mailbox`. >2000 chars is
rejected. Neither is a cheat (`OpLevel::None`, `is_cheat()==false`). Spec:
`docs/foundations/2026-06-07-lobby-mailbox-feedback.md`.

Cheat semantics: the World Integrity Ledger flag `cheats_used` is set when any
command marked `is_cheat()` succeeds. `/gamemode creative` additionally sets
`ever_creative` and clears `pure_survival` (one-way ledger flags). The dispatch
layer uses out-param markers on `CommandContext` so the commands module stays
free of save-format dependencies; the game loop persists `WorldMeta` after
each command that mutates ledger state.

The `world_time_step` field on `GameState` (default `4` for alpha 5-min day,
`1` for standard 20-min day) replaces the previous `+ 4` tick BRIDGE in
`game_loop.rs`. Commands change it at runtime; the default remains 4 until
pre-release polish flips it to 1.

### 4.6 World Chat

A submitted line that does **not** start with `/` used to be echoed straight
back into the local log and go nowhere — chat and commands ran entirely
client-local, in-process. That dead end is closed: a non-`/` line now becomes
a `ChatSay` packet (Spec 04 §2.3 addendum, v60) instead of a local echo.
`/` lines are unaffected and still dispatch through the local commands
pipeline described above.

`ChatLineKind` (`commands/dispatch.rs`) gains two variants alongside the
existing `Info | Echo | Success | Error | System`:

| Kind | Meaning |
|---|---|
| `Player` | Somebody in the world said something (delivered to this client because the tier/level rule in `docs/foundations/2026-09-05-world-chat.md` §2.3 permitted it). |
| `Room` | Somebody in the attached KithMoot room said something (§4 of the same foundations doc). |

**The T and `/` open gates are unchanged.** Chat is drawn behind the same
overlay as commands and inherits both existing gate conditions —
`GameMode::Playing` and `state.is_commands_enabled` — with no new gate of its
own. A world created with Commands OFF has no chat, which falls out of this
for free rather than needing a separate switch.

**Length**: 256 bytes max. A line that is empty/whitespace-only, over length,
or contains a control character is **rejected, not truncated** — a truncated
sentence is a changed sentence — and produces a `System` line back to the
sender only, naming which rule was broken.

**Rate limit**: 30 lines per minute per player, checked server-side before
any permission evaluation. Going over produces one `System` line to the
sender and then silence until the bucket refills.

**Guardian copy indicator**: whenever a child's chat is being copied to their
guardian (§3.4 of the foundations doc), the HUD shows a **persistent,
non-dismissable indicator** for as long as copying is on. This is not
cosmetic — a child who does not know they are being copied is being
surveilled, not parented.

Native only — the web build has no chat overlay and none of these packet
types (`check.sh`'s forbidden-symbol gate enforces this on the built bundle).

### 9.1 Mob Categories

| Category | Behaviour | Despawn? | Examples |
|---|---|---|---|
| **Passive** | Never attacks. Flees when hurt. | No (persistent once spawned). | Cow, pig, chicken, sheep, rabbit, goat, horse. |
| **Hostile** | Attacks players on sight (or when provoked). | Yes (if > 128 blocks from nearest player). | Brigand, Marauder, Berserker (the human-bandit roster). |
| **Neutral** | Passive until provoked, then hostile. | No (persistent). | Bear, Hyena, untamed Wolf. |
| **Defender** | Guards villages; attacks hostiles, not players. | No (persistent). | Knight (sole village defender). |
| **Ambient** | Decorative. Minimal AI. | Yes (if > 128 blocks from nearest player). | Bee, squid, Nostrich. |
| **Boss** | Scripted combat encounter. | No (persistent). | Future content (TBD). |

The hostile roster is the **human-bandit family** — `Brigand`, `Marauder`,
`Berserker`. There are no Minecraft-flavoured undead/arthropod/slime mobs
(Zombie/Skeleton/Spider/Creeper/Slime/WitherSkeleton) and no Iron Golem;
those were removed entirely from the engine on 2026-05-24 for open-source IP
cleanliness (see `docs/foundations/2026-05-24-fantasy-roster-excision.md`).

### 9.2 Mob AI Architecture

Mob AI uses a **hierarchical state machine** with utility-based state selection:

```
States:
  Idle        -> Wander (after random interval 3-8 seconds)
  Wander      -> Idle (reached destination or timeout)
  Chase       -> Attack (target within attack range)
  Attack      -> Chase (target moved out of range)
  Chase       -> Idle (target lost: > 32 blocks or line-of-sight lost for 5 seconds)
  Flee        -> Wander (distance from threat > 16 blocks or timeout)
  Any         -> Panic (on fire: move erratically, seek water)
```

**State selection** uses a priority system:
1. **Panic** (on fire, highest priority).
2. **Target acquisition** (hostile mobs scan for players within detection range; neutral mobs check if damaged).
3. **Chase/Attack** (if target exists and reachable).
4. **Flee** (passive mobs, when damaged).
5. **Wander/Idle** (default).

**Species-specific hit reaction — Bear and Hyena (wired 2026-07-07).** Hitting a Bear
or Hyena (melee or arrow) now provokes it: it enters an `Aggro` state keyed to the
attacking player and beelines that player for the aggro duration, rather than the
nearest player generically (distinguishes it from Hyena's separate `Hunt` chase, which
does target nearest). Melee contact damage back needs no separate wiring — both
species are `MobCategory::Hostile`, and hostile contact damage already applies
unconditionally at melee range regardless of AI sub-state. (Both species' data files
mark them `Hostile`, not `Neutral`, so — as a pre-existing, separately-tracked design
tension — they were already capable of chasing/hitting players on sight via the generic
hostile AI before this fix; this wave's change is specifically "hitting one makes it
beeline *you*," not "it was previously harmless.")

### 9.3 Pathfinding

Pathfinding uses **A\* on the voxel grid** with modifications for the 3D block world:

- **Navigation mesh**: Pre-computed walkability per block. A block is walkable if it is solid on top and has at least 2 blocks of air above it (for standard-height mobs). Climbing mobs (if any future mob has the trait) can ascend vertical surfaces. Swimming mobs navigate water columns.
- **Path update frequency**: Mobs recalculate paths every **10 ticks** (0.5 seconds) when chasing, every **40 ticks** (2 seconds) when wandering.
- **Max path length**: 32 blocks for standard mobs, 64 for boss mobs. If no path is found within this range, the mob gives up and returns to Idle.
- **Jump-capable**: Pathfinding includes 1-block vertical steps. Mobs can jump up 1 block as part of their path.
- **Drop-capable**: Hostile mobs will path over edges (drop down) if the fall is <= 3 blocks (no fall damage). Passive mobs avoid all edges.

**Performance budget**: Maximum **200 pathfinding requests per tick** across all mobs. Excess requests are queued for the next tick. This prevents pathfinding from consuming disproportionate server resources during mob-heavy scenarios.

### 9.4 Spawn Rules

**Natural spawning** occurs in spawn-eligible chunks around each player:

- **Spawn radius**: 24-128 blocks from the player (nothing spawns within 24 blocks).
- **Despawn radius**: Mobs > 128 blocks from the nearest player are despawned (hostile/ambient only).
- **Mob cap**: Maximum mobs per category per player:
  - Hostile: 70
  - Passive: 10
  - Ambient: 15
  - Water: 5

**Spawn conditions**:

| Mob Category | Light Level | Surface | Biome | Block |
|---|---|---|---|---|
| Hostile (surface) | <= 0 (effective) | Solid, opaque | Any (unless biome-specific) | Not on slabs, glass, etc. |
| Hostile (cave) | <= 0 (effective) | Solid, opaque, underground | Any | Standard |
| Passive | Any | Solid, opaque, grass/dirt | Biome-specific | Daylight preferred (initial spawn) |
| Water (fish) | Any | In water, 2+ deep | Ocean, river | Water source blocks |

**Spawn cycle**: Every 400 ticks (20 seconds), the server performs a spawn cycle for each loaded chunk within player range. It selects random positions, checks spawn conditions, and spawns mobs up to the cap.

> **As built (Phase B1 review, 2026-10-06).** The night spawner
> (`spawning::tick_mob_spawning`) caps hostiles at 80 world-wide and counts only
> hostiles in a *present* column (`World::is_column_present_at`: generated or
> restored, not dropped, evicted or never loaded). Each night spawn carries
> `entity::NightSpawn`, and when its column streams out (`ColumnSims::stream_out`,
> on the client and the dedicated server) it is despawned along with the
> column's `Scattered` wildlife. Each spawn cycle also despawns any night spawn
> standing in a column that is not present, because one that walked out of the
> loaded area into a never-loaded column gets no stream-out. Night spawns are
> never saved, so this is the as-built despawn radius: the streaming radius
> plus `UNLOAD_HYSTERESIS`. Every
> other entity in a column that is not present (pets, villagers, hideout
> brigands, whose hideout counts them) is frozen: no physics
> (`entity::tick_entities` zeroes its velocity) and no AI (`mob_ai::tick_mob_ai`)
> until the column streams back in. Before this, such entities fell through the
> missing terrain and were put back at y=80 in a loop, still ticked and
> broadcast, and far hostiles filled the cap.

### 9.5 Loot Tables

Mob drops are defined by **loot tables** — data-driven JSON structures that specify what items a mob drops on death and with what probability.

> **Implementation note (2026-07-11).** The live drop routing is
> `death_drops.rs` — a shared free function (`spawn_drops_for_death`) called
> by BOTH simulation sides: the client-side `GameState` death sweep and
> `GameServer::tick`. It covers the per-species table (`mob::drops_for`), the
> wolf tamed/untamed table, a dead steed's cargo-pack spill, and the
> salt-lick bonus, seeded deterministically on (x, z, world_time). Before
> this, `GameServer::tick` **discarded** `despawn_dead`'s result, so a mob
> killed in the hosted/dedicated-server sim dropped nothing at all.
>
> **Phase 2 closed the wire (2026-07-11, protocol v57+v58).** Server-side
> drops now broadcast as `EntityKind::Item` spawns (stack payload in the
> `ItemRef` wire encoding on `EntitySpawn.item_kind/item_id/item_count`),
> with per-tick position updates and exactly-once despawn. A remote client
> folds the diff into a render-only `remote_entities::RemoteItems` table
> (never its ECS — the client sim would re-run pickup/lifetime on the
> ghosts) and renders them via the shared drop-cube mesh. Pickup is
> server-authoritative: `GameServer::tick` runs `tick_item_lifetimes` +
> `tick_item_pickups` over server-simulated, connected, living players; the
> stack lands in `ServerPlayer.inventory` AND rides a per-connection
> `InventoryGrantPacket` (v58) that the client decodes into its local
> inventory (overflow spills at the player's feet). Until v61 only stacks
> that survived the `ItemRef` encoding were granted remotely — blocks and
> materials — because tools/plans/armour collapse on the wire (tier-only /
> `Empty`); see the phase-3 note below for what replaced that.
>
> **Superseded by MP-D2a (protocol v68):** each client now has its own
> interest set (`entity_broadcast::ClientInterest`, Spec 04 §4.2c); a late
> joiner's starts empty, so every entity near it enters — with its full
> payload — on its first broadcast, and the separate backfill below is gone.
>
> **Late-joiner backfill (2026-07-12, no wire change).** The server's
> `known_ids` diff set stays global (a spawn broadcasts once), but a client
> whose handshake completes later now gets a one-shot **backfill**: at
> join-accept the server snapshots every already-broadcast entity
> (`hosted_server::backfill_entity_events` — mobs, carts, and items with
> their stack payload) and *prepends* those spawns to that client's next
> StateUpdate. It is merged into the regular packet — never sent separately
> — so each client still receives at most one StateUpdate per tick; an
> entity that dies the same tick nets out (spawn + despawn ride the same
> packet, despawns apply last). Entities born the same tick are excluded
> from the backfill — the regular diff owns them, so nothing arrives twice.
> Client side, `RemoteClient` now **accumulates** entity spawns/updates/
> despawns and block changes across StateUpdates (`pending_entity_*`,
> `pending_block_changes`) instead of the last-write-wins `latest_state`
> those deltas used to ride — a frame hitch that batches two server ticks
> into one poll no longer drops loot spawns/despawns or desyncs blocks.
> (`latest_state` remains last-write-wins for genuine snapshot fields:
> player positions, world_time, reserve.)
>
> **Full-fidelity item wire — phase 3 (2026-09-06, protocol v61).** The
> `(item_kind, item_id)` pair is lossy: it collapses a tool to its bare
> material tier and an armour piece to `Empty`. Both `EntitySpawn` and
> `InventoryGrantPacket` now carry a trailing `full_item: protocol::WireItem`
> — a **bincode-positional, append-only** enum (`None`, `Tool { tool_type,
> material, durability }`, `Armour { slot, material, durability }`) whose u8
> fields map to the gameplay enums by explicit match in both directions
> (`inventory::item_to_wire_full` / `item_from_wire_full`; no `as`-casts, an
> unrecognised byte refuses the item). Both server encode sites populate it —
> the per-tick `diff_entities` item pass **and** `backfill_entity_events` for
> late joiners. Server pickup eligibility (`GameServer::tick`) and
> `build_grant_packets` widened from `Block | Material` to *everything except*
> `Item::Plan`, so a dead steed's half-worn iron pickaxe reaches a remote
> player's inventory as Iron Pickaxe, durability 37. Decode precedence: a
> `full_item` other than `None` **wins** over the pair, and a payload that
> fails validation refuses the grant outright rather than falling back to the
> pair (which for a tool would mint a fabricated full-durability item).
> Validation clamps over-max durability to the kind's cap and refuses zero
> durability; grants are server → client only, so this is defence-in-depth,
> not a trust boundary. Client-side, `RemoteItems` stores the payload and
> `entity_model::item_textures` is now shared by the local and remote drop
> passes, so armour drops render (they previously encoded `Empty` and drew
> nothing). **Plans stay floor-bound by design** — `plan::PlanData` is far too
> heavy for a per-tick broadcast; the append-only enum leaves room to add a
> `Plan` variant later without renumbering. Spec:
> `docs/foundations/2026-07-12-full-fidelity-item-wire.md`. Remaining gap,
> riding the dual-sim rework: kill attribution / bounty / Vow stay
> client-side.

```json
{
    "entity": "genesis:brigand",
    "pools": [
        {
            "rolls": { "min": 0, "max": 2 },
            "entries": [
                { "item": "genesis:coin", "weight": 1 }
            ]
        },
        {
            "rolls": 1,
            "conditions": [{ "type": "killed_by_player" }],
            "entries": [
                { "item": "genesis:iron_ingot", "weight": 1 },
                { "item": "genesis:carrot", "weight": 1 },
                { "item": "genesis:potato", "weight": 1 }
            ],
            "chance": 0.025
        }
    ]
}
```

Loot tables are plugin-extensible. Plugins can add, modify, or replace loot tables for any mob. The `conditions` system supports: `killed_by_player`, `random_chance`, `looting_enchantment` (future), and custom conditions registered by plugins.

**Bone re-sourcing (2026-05-24).** With the fantasy roster removed, the Skeleton no longer exists to drop `Bone`. To keep the `Bone → 3 Bonemeal` fertiliser loop (and Bone Block crafting) reachable, **livestock now drop bones**: Cow, Sheep, and Pig each drop 0–1 `Bone` (~12.5%) on death, alongside their meat/leather/wool. `Bone` and `Bonemeal` are generic items and were kept. See `docs/foundations/2026-05-24-fantasy-roster-excision.md` §3.3.

### 9.6 Data-Driven Mob Definitions

Mob types are defined in data files (TOML or JSON), not hardcoded:

```toml
[mob]
id = "genesis:brigand"
category = "hostile"
health = 20
width = 0.6
height = 1.95
speed = 2.28          # blocks/second
attack_damage = 3     # Easy: 2, Normal: 3, Hard: 4
detection_range = 40  # blocks
attack_range = 1.5    # blocks (melee)
drops = "genesis:loot_tables/brigand.json"
spawn_weight = 100    # relative likelihood in spawn cycle
spawn_group_size = { min = 1, max = 4 }

[mob.ai]
states = ["idle", "wander", "chase", "attack", "panic"]
hostile_to = ["player"]
can_break_doors = false   # true on "hard" difficulty
```

This data-driven approach means:
- Modders can add new mobs by creating definition files and registering them via plugins.
- Balance tweaks do not require code changes.
- Server operators can override mob stats per-server.

### 9.7 Pets & Companions — SHIPPED 2026-07-06 (pets-debt-water wave)

Finishes the deferred "1C pet framework" work from
`docs/foundations/2026-06-22-animals-six-wave-master-plan.md` plus the
per-species deferrals (Cat, Parrot, Donkey/Mule, Crab). Substrate reused, not
rebuilt: `tameable::OwnershipData` + `attempt_tame_generic`, `CompanionData`
(`companion.rs`), `species_ai::dispatch_companions`, the wolf state machine
(`wolf.rs`), and `HorseData` steed persistence (`horse_ai.rs`, §7.6).

**Command states.** `CompanionState::{Follow, Stay, Wander, Perch}` on
`CompanionData` (appended `#[serde(default)]`, default `Follow` — old saves
unchanged). Empty-hand right-click on an owned companion cycles the state with
a toast naming it; non-shoulder species skip `Perch`. `Follow` = current
behaviour, `Stay` = hold position, `Wander` = generic ambient wander, `Perch` =
shoulder-ride (Parrot, see below). The wolf keeps its own state machine —
`wolf::try_toggle_sit` is wired to the same empty-hand-right-click grammar
(same input, different underlying store; `WolfData`/`NostrichData` were **not**
unified into `CompanionData` this wave — migration risk outweighs tidiness,
recorded as future cleanup).

**Friendly-fire protection.** Attack **target selection** itself (melee and the
right-click interaction picker) skips entities owned by the attacker
(`CompanionData`/`WolfData`/`NostrichData` ownership, and kept steeds via
`HorseData.kept_by`) **unless the attacker is sneaking** (an intentional-hit escape
hatch) — the swing retargets to the next-nearest hostile in the cone rather than
cancelling outright (hardened 2026-07-07: previously the exclusion cancelled the
whole swing *after* targeting, so an own pet body-blocking a hostile — e.g.
wolf-assist pushing the wolf in front of the enemy — silently swung at nothing
instead of hitting the enemy behind it). The sweep-attack arc (§6.4) honours the
same filter. A **perched** parrot (see below) is excluded from both selectors
unconditionally, sneaking or not — it's cosmetic while perched, never a valid
melee/interaction target.

**Anti-loss — Pet Bed + Recall Whistle.** A **Pet Bed** block (craft: wool +
planks) rescues a tamed pet on death instead of letting it die: the pet
teleports to the nearest loaded Pet Bed and is restored to full health
("knocked out, crawls home"). Bed search is loaded-chunks only and beds are
communal (no per-bed ownership v1). **This rescue runs in the client-driven
tick (`game_loop.rs`, alongside drops/kill-counter) and does not yet exist for
a hosted multiplayer session** — `server.rs`'s death path has no
death-consequence layer at all today (pre-existing single-player/GameServer
dual-sim debt, not a regression introduced here). A pet that dies in a hosted
multiplayer game is not rescued yet. Rescue is **skipped when the owner
deliberately sneak-culls their own pet** (hardened 2026-07-07): a sneak-hit
landing on the attacker's own pet marks it `DeliberateCull` for a 5-second
window (`CULL_MARK_TTL_TICKS = 100`), and a marked pet's death is not eligible
for Pet Bed rescue during that window — otherwise the friendly-fire
escape hatch above was unusable within range of any bed, since the pet just
revived every time. The mark auto-expires so one accidental non-lethal sneak
tap can't permanently disable rescue for that pet. A hostile's kill on the
same pet is never marked and rescues exactly as before. A **Recall Whistle**
(craft: bone + string) teleports every tamed/kept animal you own —
companions, wolf, nostrich, kept steeds including a cargo-laden donkey — to
you on use, reporting the count; it fires regardless of what's under the
crosshair (no block-target gate) and its landing ring avoids solid cells.
**Excludes any steed a player is currently riding** (hardened 2026-07-07,
split-screen: whistling no longer teleports a mount out from under whoever is
riding it, dragging them across the world via ride-follow — the recall count
and toast already reflect the exclusion).

**Wolf combat-assist.** `WolfAction::AttackTarget` now drives real movement +
melee damage (the wolf paths to the target and hits on contact), and the
trigger is wired end-to-end: a wolf organically enters assist/revenge from
real "owner attacks something" / "owner is attacked" combat events
(`combat.rs`/`block_interact.rs`, feeding the existing pure
`should_pivot_assist`/`should_pivot_revenge` gates), gated so a sitting wolf
(`WolfAiState::Sit`) never self-pivots into a fight. **Give-up-follow
(wired 2026-07-07):** a following wolf gives up at `FOLLOW_GIVE_UP_DISTANCE =
32` blocks from its owner (state → `Idle`, stops chasing across the world),
and resumes following once the owner comes back within `FOLLOW_MAX_DISTANCE =
8` blocks — the asymmetric thresholds are deliberate hysteresis so an owner
hovering near either boundary doesn't flap the wolf between states every
tick.

**Cat.** Tame with fish (1-in-3) or a guaranteed-tame **Cat Treat** (craft: raw
fish + wheat → 2). Tamed cats project a threat-ward aura
(`CAT_WARD_RADIUS = 12.0` blocks): hostiles (Brigand/Marauder/Berserker/Hyena)
veto targets inside the radius and get a flee bias away from the cat.

**Parrot.** Flies (the `Flying` marker, bee pattern) and follows with a Y
component. `CompanionState::Perch` gives a shoulder-ride — position pinned to
the owner's shoulder offset each tick, AI/physics skipped while perched,
dismounts to `Follow` on owner damage or a state cycle. The shoulder anchor is
**rotated by the owner's yaw** (body-space offset, hardened 2026-07-07 — it
was previously a fixed world-axis offset, so facing certain directions put the
parrot inside the first-person interaction cone and every swing/right-click
near an animal hit the parrot instead: melee no-op'd, feed/milk/shear/mount
silently misfired). A perched parrot never blocks attack or interaction
targeting at all (see Friendly-fire protection, above) — it's excluded from
both target selectors unconditionally, independent of the yaw fix. Any tamed
parrot squawks (toast + particle burst) when a hostile comes within
`PARROT_ALARM_RADIUS = 16.0` blocks, on a `PARROT_ALARM_COOLDOWN = 200`-tick
(10 s) cooldown. The parrot→Electricity signal bridge stays deferred with the
Aether design.

**Donkey/Mule cargo.** `HorseData.pack: Option<ChestData>` (appended
`#[serde(default)]`, wire shape frozen — `pack` is `serde(skip)` and persists
via the existing `SavedTamedPetData::Steed2.pack`, no new save variant).
Right-click a kept Donkey/Mule while holding a Chest to equip the pack
(consumes the chest); sneak-right-click with an empty hand on a packed
mount opens the 27-slot pack via `chest_ui::show_container_dialog` **if it
still has contents**. Sneak-right-click with an empty hand on an **empty**
pack instead **unequips it** (wired 2026-07-07): the pack is removed and one
Chest is returned to the player's inventory (spawned as a world item if the
inventory is full), toast "Pack removed."; a non-empty pack still just opens
(with a "Empty the pack first." toast) — so a pack can always be filled and
later emptied by hand, not only recovered via death-spill. Horse never
carries a pack (speed vs utility niche). A packed donkey rescued by a Pet Bed
keeps its pack. **A donkey/mule that dies away from a Pet Bed now spills its
pack's contents plus one Chest item** (hardened 2026-07-07 — previously the
pack and everything in it were silently destroyed on death unless rescue
happened to fire; a rescued pet still keeps its pack with zero spill, no
double-drop). Mounting a wild Horse/Donkey/Mule for the first time now also
**removes the `Scattered` tag** the instant it's kept (hardened 2026-07-07 —
previously a kept, possibly packed, steed retained `Scattered` and could be
permanently despawned by ordinary chunk unload before the next save, unlike
every other tame path).

**Mule cross-breeding.** The horse family now breeds on Wheat
(`breeding::breeding_food`). A **plain right-click with Wheat mounts** a
rideable adult Horse/Donkey/Mule as before (wheat is not consumed, no love
mode) — **sneak + right-click with Wheat feeds** it into love mode instead
(hardened 2026-07-07: enabling cross-breeding briefly broke plain wheat-mount
by routing every wheat right-click into the feed branch first; the sneak gate
restores the mount gesture while keeping the feed gesture one sneak away).
Non-horse-family breeding feed (Cow, Pig, Chicken, Sheep, Goat, Rabbit) is
un-gated, as before. A Horse×Donkey in-love adjacent pair is the one
cross-species exception to the same-species pairing rule and produces a
**Mule** with blended `Genetics`. **Mules are sterile** — they never pair,
regardless of partner.

**Salt Lick.** A Salt Lick block (`docs/foundations/2026-05-23-salt.md`)
doubles the natural HP regen rate for livestock (Cow, Sheep, Horse, Pig, Goat)
within 8 blocks (`salt_lick::regen_multiplier_at_pos`). The 2× multiplier was
documented at ship but never actually applied — the regen tick used a flat
`+1.0` and ignored the (tested) multiplier helper; fixed 2026-07-07.

**Crab + Reach Claw.** `MobType::Crab` (appended last, save-order discipline)
spawns passively on sand near sea level (coastal biome). Drops **Crab Claw**
on kill. The **Reach Claw** tool (craft: crab claw + sticks) extends block
place/break ray distance by `REACH_CLAW_BONUS = 2.0` blocks over the base
reach while held — see §2.5 for the current (2026-07-07-hardened) multiplayer
reach-gate rules: the bonus is honoured for the local/hosting player, but a
hosted session's remote players get a flat, client-claim-immune base reach
regardless of what item they claim to hold. No durability drain v1.

**Trials integration.** Taming a companion (Cat, Parrot, or Fox) now fires the
same `ChallengeEvent::TameMob` the wolf-tame path already fired, so it counts
toward Trials (hardened 2026-07-07 — previously only wolf tames were visible
to the challenge system). The Tend track gained two trials this wave: **Mule
Maker** (breed a Mule) and **A Friend in Need** (tame any one of Cat, Parrot,
or Fox).

**Known, not a bug: placeholder meshes.** Cat, Parrot, and Crab all render with
a placeholder mesh (the established cat-uses-wolf-mesh pattern) — dedicated
models are Axolittle-playtest-boundary work, not shipped this wave.

---

## 10. Chat and Commands

### 10.1 In-Game Chat

**Chat system**:
- Press `T` to open the chat input (or `/` to open with command prefix).
- Chat messages are broadcast to all players within the same world by default.
- Maximum message length: **256 characters**.
- Chat history: last **100 messages** are retained client-side (scrollable).
- Messages are prefixed with the sender's display name: `<PlayerName> message here`.

**Chat formatting**: Messages support a limited markup syntax for colour and style:
- `&1`-`&f` for colours (hex palette, 16 colours).
- `&l` bold, `&o` italic, `&n` underline, `&r` reset.
- Only operators can use formatting codes in regular chat (to prevent abuse). Players can use formatting in signs, books, and named items.

**Chat moderation**:
- Server-side word filter (configurable list, disabled by default).
- Rate limiting: max **3 messages per second** per player. Excess messages are silently dropped and a warning is sent to the sender.
- Mute command: `/mute <player> [duration]`.
- Chat logging: all messages are logged server-side with timestamps for moderation review.

### 10.2 Command System

Commands begin with `/` and are processed server-side.

**Core commands** (built-in):

| Command | Permission Level | Description |
|---|---:|---|
| `/help [command]` | 0 (all) | List commands or show command help. |
| `/msg <player> <message>` | 0 | Private message. |
| `/me <action>` | 0 | Emote / action message. |
| `/gamemode <mode> [player]` | 2 (operator) | Change play mode (`survival\|creative\|adventure\|spectator`). `[player]` targeting deferred — modes are world-level today. |
| `/tp <target> [destination]` | 2 | Teleport player. |
| `/give <player> <item> [count]` | 2 | Give items. |
| `/time set <value>` | 2 | Set world time. |
| `/weather <type> [duration]` | 2 | Set weather. |
| `/kill [target]` | 2 | Kill entity/player. |
| `/ban <player> [reason]` | 3 (admin) | Ban player. |
| `/kick <player> [reason]` | 2 | Kick player. |
| `/op <player>` | 3 | Grant operator status. |
| `/deop <player>` | 3 | Revoke operator status. |
| `/save-all` | 3 | Force world save. |
| `/stop` | 3 | Gracefully shut down the server. |
| `/seed` | 2 | Display world seed. |
| `/difficulty <level>` | 2 | Set difficulty (peaceful, easy, normal, hard). |
| `/spawn` | 0 | Teleport to world spawn (if permitted by server config). |

### 10.3 Permission Levels

| Level | Name | Description |
|---:|---|---|
| 0 | Player | Default. Chat, basic commands. |
| 1 | Moderator | Kick, mute, view reports. |
| 2 | Operator | Game mode, teleport, give, world management. |
| 3 | Admin | Ban, op/deop, server control, plugin management. |
| 4 | Console | Server console only. Full access. Not assignable to players. |

Permissions are stored per-player in the server's player data. The permission system supports **granular permission nodes** in addition to levels. A plugin can register `myplugin.command.foo` and grant it to specific players or groups regardless of their level.

### 10.4 Plugin-Provided Commands

Plugins register commands during initialisation:

```rust
// WASM plugin API (conceptual)
fn register_commands(registrar: &mut CommandRegistrar) {
    registrar.register(
        Command::new("economy")
            .description("View your balance")
            .permission("genesis.economy.balance")
            .handler(handle_economy_command)
    );
}
```

Plugin commands are namespaced: `/economy:balance` (or just `/balance` if no conflict). If two plugins register the same command name, the server logs a warning and the first registration wins. The conflicting command remains accessible via its fully qualified name.

**Tab completion**: Commands support tab completion for arguments. The server sends completion suggestions to the client based on the command's registered argument types (player names, item IDs, coordinates, etc.).

---

## 10b. The Workshop — community visual redesign (Spec 40)

Foundation: `docs/foundations/2026-06-04-the-workshop-community-redesign.md`. Built (local) 2026-06-05; Phases A/B/C/F on `main`. Distribution (open-stash share / official adoption) deferred to the owner.

**What it is.** A blank/void **Workshop world**, entered from the Lobby ("🔧 The Workshop" button → `MenuAction::EnterWorkshop`), where players **redesign assets that already exist** — repaint a flower and *every* flower updates; repaint a cow and *every* cow updates. Purely visual (block ids, mob types, saves, behaviour untouched) ⇒ multiplayer-safe by construction.

**The space.** The Workshop is a real saved world (`WorldMeta.is_workshop`, always creative), generated as a single flat floor at `workshop::WORKSHOP_FLOOR_Y` (= 79, one below the default spawn) with no terrain (`World::generate_workshop_column`). Projects are persistent objects: the `workshop_projects` side-table (`workshop::WorkshopProjects`) rides the world save (`#[serde(default)]`, append-only invariant), so **many parked, half-finished, still-inflated projects survive save/quit**.

**The gesture (design).** Place an asset → **bellows**-inflate to a working size (`WorkshopProject::pump`/`deflate`, scale `inflation_scale`) → edit → **pin** (`pin()` — deflate + commit). Un-pinned = *parked WIP* (persists inflated); pinned = *committed* to the override registry. The lifecycle is `Placed → Inflated(n) → Pinned`.

**Two authoring modes:**
- **Mode A — reskin** (`commit_project_override`): paint the asset's faces (16×16, the block's native texel grid); pinning writes an `AuthoredFaces` into the `OverrideRegistry` (Spec 03 §3.2). Works on blocks AND mobs (mobs are textured exactly like blocks; the same painter reskins both — NOT the player-avatar 64×64 atlas).
- **Mode B — reshape** (`capture_box_as_plan` → `micro_model::from_plan` → `micro_registry`): build the new form (single-block, ≤16³) and capture it into a baked micro-model shape override (reuses #18). Multi-block sculptures + mob reshape (#19) are parked.

**Control surface.** The **16×16 paint-grid editor** (`workshop_painter.rs`) opens with `/ws edit <asset>`: six face grids seeded from the asset's current texture, a 16-colour palette, click/drag to paint, Pin → global reskin (headless-rendered via `--shot-painter`). The **`/ws` command** (`commands/builtins/workshop.rs`: `place` / `edit` / `paint` / `pump` / `deflate` / `pin` / `reshape` / `list` / `reset`) drives the whole loop. Remaining tactile *feel/flavour* — the **bellows-inflate gesture** as a held tool, an **in-world 3D inflated copy**, the **mob mannequin + play toggle** — is the **Axolittle playtest boundary**.

**The v2 gesture — blow-up-and-sculpt (Spec 40 evolution, `docs/foundations/2026-06-08-workshop-blow-up-and-sculpt.md`).** The authoring *front-end* was redesigned from `/ws place` + 2D paint-grid into one in-world gesture (the 2D grid + `/ws place` are kept as a power-user/test fallback). The player **places a normal block**, **aims the Bellows at it**, and **holds right-click** to blow it up to a fixed **×4** working size:

- **Cage preview + room-check (Phase 1).** While aiming, a green/red 4×4×4 wireframe cage shows where it will grow — anchored at the block's near-bottom corner, growing up and away from the player (`workshop::blow_up_cage_min`/`_cells`/`_clear`, drawn via the ghost-wireframe path). Green = clear, red = blocked (the blow-up refuses with a toast). The crosshair resolves the exact texel via `raycast::face_texel`.
- **Hold-to-charge fixed ×4 (Phase 2).** A tick-based charge (`workshop::BlowUp`, `BlowUpPhase::{Charging,Locked,Collapsing}`) drives 3 smooth swells (×2/×3/×4) via `blow_up_scale_for_charge`; release before full → collapse to ×1 (only ×1 and ×4 are stable); reach full → **lock**. Sneak+right-click on a locked copy collapses it. ×4 (4 blocks tall) is the floor-reach ceiling — no flying. The balloon renders from its cage corner at the animated scale (`entity_model::build_workshop_inworld_vertices`).
- **In-world dye editing (Phase 3).** A locked ×4 copy carries a **16³ `EditBuffer`** (`workshop.rs`) — each cell is `Option<BlockId>`: `Some(id)` = an occupied coloured microblock (colour = the dye's WALLPAPER block via `MaterialId::paint_block`; 16 dyes), `None` = carved-away. One field is **both colour and occupancy** (the paint-with-blocks model — there is no per-vertex colour). The crosshair selects a cell via `raycast::pick_cell_in_cage` (a DDA into the cage AABB, since the balloon is a render overlay, not world blocks). With a dye in hand: **left-click paints** the aimed cell, **right-click places** a coloured microblock against the aimed face; a **pickaxe/empty hand left-click carves**; the **G key eyedrops** a colour; the **M key cycles symmetry** (off / left-right / all-sides, default all-sides via the 90° XZ rotation — matches the native 3-surface block model). Edits are click-edge (one cell per click) and gated to a `Locked` Block balloon. The balloon renders live from the buffer by baking it through `micro_model::bake_micro_model` (greedy-merged, `block_id`-textured) scaled ×4 to the cage corner — so paint and sculpt are visible. The WIP buffer is transient (`#[serde(skip)]`; persists on pin only). **Playtest boundary (Axolittle):** the paint *feel*, the ¼-block aim comfort, the tiled-wallpaper look of an edited balloon, and the symmetry default.
- **Pin bake (Phase 4).** The player presses **P** (or `/ws pin` on a locked, edited balloon) to commit the working copy as a global appearance override; every placed instance of that block type immediately wears it. The balloon then collapses ×4→×1 (reusing the Phase-2 collapse animation). Pin output path depends on whether the buffer was shape-changed (`EditBuffer::shape_changed`):
  - **Pure-paint** (no cells added/removed): `EditBuffer::to_authored_faces` collapses the 16³ colour buffer down to the block's 3 native surfaces and writes an `AuthoredFaces` entry via `override_registry`. The greedy mesher and texture array continue as normal.
  - **Shape-changed** (any place or carve): `EditBuffer::to_micro_model` bakes a coloured micro-model that is registered in `world.micro_registry`. The chunk mesher now handles registered micro-models for **solid** blocks: the greedy pass detects the micro-registration and skips that block's faces, instead emitting a per-instance micro-model draw (plus a far-LOD billboard) — completing the previously-deferred solid-block micro hook. Transparent/fluid blocks are unaffected.
  - **Re-pin / last-pin-wins:** re-pinning a block replaces its design cleanly (paint dedups texture-array layers via `rebuild_layers`; shape calls `Renderer::clear_micro_geo` before `sync_micro_models` so the new shell actually uploads — `sync_micro_models` skips already-uploaded ids). If a block is pinned as a shape after a paint (or vice-versa), the shape path wins visually (the mesher routes any micro-registered block to the shell and ignores its texture override); the earlier kind goes dormant. An all-carved (empty) shape pin is refused so a block can't be made invisible.
  - **Visual-only / multiplayer-safe:** block ids, world saves, and game behaviour are untouched by both paths. See `docs/foundations/2026-06-08-workshop-blow-up-and-sculpt.md`.

- **Phase 5 — the wardrobe (many designs per block).** Phase 4 pin-bake landed a single-slot override (re-pin = last-wins). Phase 5 promotes each slot to a **per-key `DesignLibrary`** (`designs: Vec<NamedDesign>` + `active: Option<DesignId>`) so a block can carry multiple named looks:

  - **Pin APPENDS** — pinning a new design (paint or shape) appends a `NamedDesign` into the library and immediately sets it active (wears it). It never silently overwrites an earlier design. Library cap is **16 per key**; if full, the **oldest non-active** entry is evicted (with a toast) to make room. The active design is never evicted.
  - **Adopt slots inactive** — adopting another player's design appends it into the library in an *inactive* state; it does not silently replace what you are wearing. Adoption is also bounded at 16 (same eviction rule).
  - **Render reads the active design** — the greedy mesher / texture-array path reads the active `DesignLibrary` entry for each key; stock appearance is used when `active = None` (use-original). `apply_active_designs` rebuilds the block texture array (`Renderer::rebuild_block_textures`) and/or re-syncs `micro_registry` (shape) live across all loaded chunks when the selection changes.
  - **The Wardrobe egui panel** — **K** key (Workshop-only) or `/ws gallery` opens a per-block design browser; actions: **set-active** (wear this), **rename** (label a design), **delete** (remove non-active entry), **use-original** (set `active = None`, revert to stock).
  - **v1→v2 blob migration** — the `OverrideSet` blob gains a **version byte 2** (`to_blob_bytes` writes v2). `from_blob_bytes` version-branches: v1 blobs (previously omitted shape data) are decoded as `OverrideSetV1` via a private `migrate_v1` helper that promotes them to the library layout; v2 is decoded direct; v>2 is rejected with an error (forward-compat: an old client sees an unknown version and refuses cleanly rather than misparsing). **Shape designs now ride the v2 blob** — this supersedes the earlier "coloured-micro-model sharing wire format deferred" note in `docs/foundations/2026-06-08-workshop-blow-up-and-sculpt.md §Out of scope`; that deferral is resolved. `world.dat` carries no `OverrideSet` so no world-save migration is needed. Official catalogue blobs live-published before v2 must be re-baked by the owner (`tools/bake-beacon.js`); the alpha PWA auto-updates so old live blobs are a one-time owner re-bake.

- **Persistence — the wardrobe survives reload (Spec 40 follow-up).** Phases 4/5 authored a wardrobe that lived only in memory (lost on world reload — a known gap). It now persists:
  - **Per-player-global by default.** A player's wardrobe (the `OverrideSet` of per-block `DesignLibrary`s + active selections) is **global** — it follows them into every world. Storage seam: `wardrobe_store`. On **WASM/PWA** it rides the **private Stash** (NIP-44 encrypted-to-self) as a player-scoped, cross-world singleton blob (`kind:"wardrobe"/name:"designs"`, mirroring the cosmetic skin) via `cloud.js`. On **native** it rides a local `profile/wardrobe.blob`. Wire format is the existing version-prefixed v2 `OverrideSet` blob (`to_blob_bytes`/`from_blob_bytes`) — no new format.
  - **Per-world override.** A world MAY carry its own override wardrobe on `WorldMeta.world_override` (a trailing `#[serde(default)] Option<Vec<u8>>` — an `OverrideSet` blob) that **supersedes** the player-global *for that world only* (e.g. a creator pinning a fixed look). Absent (the default) ⇒ the player-global is used. Append-only / `#[serde(default)]` ⇒ old `world_meta.json` loads clean.
  - **Load order (single apply point, `GameState::reapply_overrides` via `chunk_stream::initial_load`).** Render view = `official_overrides::resolve_render_set(official, player_wardrobe, world_override, remember)`: official catalogue **beneath** → the **player-global** wardrobe → the **per-world override** supersedes. On native the player-global loads synchronously on world enter; on WASM it loads asynchronously from the Stash (a `wardrobe_load_slot` drained by the main loop), with a `wardrobe_user_acted` guard so a late load can't clobber a same-session edit.
  - **Remember-last setting.** A Wardrobe-panel checkbox "Remember my designs when I enter a world" (default **on**). When **off** ("Start at standard"), `resolve_render_set` drops the personal + per-world layers (stock + official only) — but the saved wardrobe is **not erased** (toggling back restores it). Persisted per-pubkey (localStorage on WASM, a `profile/` file on native).
  - **Save triggers.** Authoring (pin, Wardrobe-panel set-active/rename/delete/use-original, adopt) sets a dirty flag; a debounced flush (`WARDROBE_SAVE_DEBOUNCE_TICKS`) batches rapid edits, and world exit (`PAUSE_SAVE`/`PAUSE_SAVE_QUIT`) flushes immediately. Best-effort (native profile file; WASM Stash, which no-ops cheaply when no capable signer is present).
  - **Architecture.** `World.player_wardrobe` is the authored set (saved); `World.override_registry` is the **derived** render view the renderer reads. All authoring mutates `player_wardrobe` then `reapply_overrides` rebuilds the render view. Authoring is Workshop-only; play worlds are render-only. Visual-only / multiplayer-safe is unchanged (block ids, gameplay, world block data untouched).

**Render apply.** Pinning a reskin/reshape (or `/ws revert`) raises `CommandResult::ApplyWorkshopOverrides` / `ApplyWorkshopReshape`; the game loop rebuilds the block texture array (`Renderer::rebuild_block_textures`) and/or registers the micro-model, then re-meshes loaded chunks so block faces/shapes update live. Entity (mob) faces resolve per-build — no re-mesh.

**Sharing + adoption (Spec 40 Phases D/E, on `@forgesworn/beacon`).** A pinned reskin produces an `OverrideSet` (serde + `content_hash` + `author_npub` + derivation chain) that a player can publish to the world and others can adopt:

- **Publish** — `/ws publish <name>` then `/ws publish confirm`. The set serialises to a **v2 1-byte-version-prefixed bincode** blob (`OverrideSet::to_blob_bytes`; carries both paint and shape designs; a newer version is rejected by an older client, never misparsed) and publishes as a Beacon kind-30820 `d:axenstax` item (`contentType="override-set"`) under the player's npub. **Public-content safety contract** (kids' platform): publishing is public, under your name, in the clear, effectively permanent — so it is a **deliberate two-step act** (`beacon_publish_pending`, session-transient, never persisted), distinct from the private cloud save, with explicit "share with EVERYONE" copy and an explicit `confirm`. Never auto-published. PWA-only (`beacon_available()`); native shows a web-only message.
- **Discover + adopt** — `/ws follow|unfollow|following <npub>`, `/ws browse [npub]`, `/ws adopt <n>`. Follow set is NIP-51; browse lists `override-set` items across followed creators (item-capped, attributed by **npub**); adopt fetches the blob → `apply_adopted_override_bytes` (version-checked + a **texture-array layer guard** that rejects an over-cap set *before* mutating the registry, so a huge shared blob can't crash the renderer) → `merge_adopted` (whole-set, **last-adopted-wins** per asset key, appends exactly one derivation link for attribution) → live re-texture + re-mesh. Adopted designs slot into the wardrobe **inactive** (Phase 5). Per-asset on/off + a conflict-picker are deferred (Spec 40 Open Q #2).
- **Official catalogue** — an owner-curated bundle baked at build time (`tools/bake-beacon.js` → `official_overrides.json`, bounded + sha256-verified) and embedded via `include_str!`, applied **beneath** personal overrides at world entry (`apply_official_beneath` — personal wins per key; official is a base layer, no provenance bleed). Ships **empty** ⇒ byte-identical to a stock game until the owner bakes the real official npub's content. Offline on both platforms (data, not live Beacon).

The transport is **`@forgesworn/beacon`** (the SDK owns manifest build/parse/**schnorr-verify**); the engine keeps only thin byte-marshalling externs to `window.AxeBeacon`. Author attribution renders **npub** (NIP-19); hex stays internal (decoded at the input boundary, encoded at the display boundary). The live publish→adopt round-trip + the real official npub are owner-verified (no relay/Blossom on the build host). Full design: `docs/foundations/2026-06-04-the-workshop-community-redesign.md` (Phases D/E) + `<workspace>/forgesworn/beacon/CONSUMING.md`.

---

## 11. Particle and Sound Systems

### 11.1 Particle System Overview

Particles are **client-side visual effects** with server-triggered events. The server tells clients "emit particles at position X with type Y," and the client handles rendering.

**Particle event types**:

| Event | Trigger | Particle Description |
|---|---|---|
| Block break | Block destroyed | 8-16 fragments of the block's texture, scattered with physics. |
| Block place | Block placed | Subtle puff at placement position. |
| Footstep | Player/mob walks | Small dust puffs at feet. Surface-dependent (snow particles on snow, etc.). |
| Critical hit | Melee critical | Star-burst particles around target. |
| Enchantment glint | Future: enchanted item | Sparkle particles spiralling toward the item. |
| Flame | Fire, lava, torches | Animated flame sprites. |
| Smoke | Fire, furnace, torch | Upward-drifting dark particles. |
| Water splash | Entity enters water | Radial splash particles + ripple. |
| Explosion | Blasting Keg (Spec 49) | Large expanding sphere of smoke and debris. **Deferred** — no particle framework yet; Spec 49 ships the boom sound + crater, particles follow when the framework lands. |
| Portal | Nether portal equiv | Purple drifting particles near portal blocks. |
| Rain / snow | Weather | Falling particle sprites across the visible sky. |
| Redstone | Signal wire active | Small red particles above active wire. |
| Heart | Animal breeding | Floating heart sprites above mobs. |
| XP orb | XP collection | Green-yellow glow orbs that travel toward the player. |

### 11.2 Particle Rendering Architecture

**Hybrid CPU/GPU approach**:

- **Simple particles** (dust, smoke, flame): Rendered as **GPU-instanced billboarded quads**. The CPU updates a position/velocity buffer each frame; the GPU renders thousands of sprites in a single draw call. This is the default path and handles the vast majority of particles.
- **Complex particles** (block fragments with per-fragment textures): Rendered as **small meshes** (2-4 triangle quads per fragment) with the block's texture. CPU-managed but batched by texture atlas page.
- **Weather particles** (rain, snow): Rendered in **screen-space** as a post-process effect rather than individual world-space particles. This allows dense weather without per-particle overhead.

**Particle budget**: The client enforces a maximum of **4096 active particles** at any time. When the budget is exceeded, oldest particles are culled first. This prevents particle storms from tanking frame rate. The budget is configurable in client settings (lower for weaker hardware).

**LOD (Level of Detail)**: Particles beyond **32 blocks** from the camera are simplified (reduced count, no physics simulation, simple fade-out). Particles beyond **64 blocks** are not rendered at all.

### 11.3 Sound System — as built (2026-10-05, audit wave W2)

**Status: a small procedural sound-effect layer. There is no sound-asset pipeline, no music, no positional audio, no category mixer.** The earlier draft of this section described a full audio system (nine volume categories, 3D positional audio, an OGG asset pipeline, a trait with `play_music` / `set_listener`); none of that exists. What does exist is `game/engine/src/audio.rs`:

**Seven synthesised sounds.** No audio files exist anywhere in the repo or the bundle. Each sound is a short recipe of sine-tone and white-noise layers with a linear decay envelope (`audio::layers`), rendered to mono 44.1 kHz PCM by the pure function `audio::render_mix` — the **same samples on every target**.

| Sound | Recipe | Call |
|---|---|---|
| Block break | noise 0.08 s, vol 0.30 | `play_break` |
| Block place | 600 Hz tone 0.06 s, vol 0.25 | `play_place` |
| Footstep | noise 0.04 s, vol 0.15 | `play_footstep(sprinting)` (throttled: 0.45 s walking, 0.35 s sprinting) |
| Explosion (Blasting Keg, Spec 49) | 70 Hz tone 0.45 s + noise 0.50 s | `play_explosion` |
| Thunder | 52 Hz tone 1.1 s + noise 1.4 s | `play_thunder` |
| Gem pickup (routine Satori) | 880 Hz tone 0.12 s | `play_gem_pickup` |
| Genesis Block fanfare (first Satori in a world) | C5 → E5 → G5 → C6 arpeggio, 0.2 s notes 0.1 s apart | `play_genesis_block` |

**Not built** (design targets, kept in §11.9): jump / landing / hurt / item-pickup / menu-click / door / chest / eating sounds, per-material variants, mob sounds, ambient and weather loops, music, positional audio, obstruction, per-category volume sliders. Note that fall damage and drowning (§1.5, §1.5.1) currently make **no** sound.

### 11.4 Master volume and mute (built)

One master volume (`0..=1`, default 1.0) and one mute switch, both in `GraphicsSettings` (`master_volume`, `audio_muted`) — per-device, persisted on both targets (native `settings.json`, web `localStorage` `axenstax_gfx`), clamped on load, untouched by graphics-preset clicks. The Settings panel ("Graphics", reachable from the lobby and the pause menu) has a **Sound** section: a percentage slider and a "Mute all sound" checkbox, applied live. The engine squares the slider (`audio::effective_gain`: 50% → 0.25 amplitude) so the lower half of the travel is usable; 100% leaves the sounds exactly as authored. The setting reaches the engine through `AudioEngine::set_master` on three paths: GameState start-up, `sync_graphics_to_engine` (world entry + the in-game panel) and the lobby panel.

### 11.5 Cross-platform audio (built)

The game calls one API (`AudioEngine::play_*`, `set_master`) with two backends selected by `cfg(target_arch = "wasm32")` inside `audio.rs`, so call sites do not fork:

| Platform | Backend | Notes |
|---|---|---|
| Native (desktop, Android) | `rodio` over `cpal` | The rendered PCM is played with `play_raw`. If no output device exists the engine runs silent. The master gain is baked into the rendered samples. |
| Web (WASM) | Web Audio API via `web-sys` | The rendered PCM is copied into an `AudioBuffer` (cached per sound), started through an `AudioBufferSourceNode`, routed through one master `GainNode` (volume + mute). **No assets are fetched and none are embedded** (no `include_bytes!`), so the bundle-size gate is unaffected beyond a few KiB of code. |

**Autoplay unlock (web).** Browsers start an `AudioContext` suspended. `AudioEngine::new` registers capture-phase `pointerdown` / `keydown` / `touchend` / `click` listeners on `window` that call `resume()`. A sound requested while the context is not running is **dropped, never queued** (so nothing bursts out when the context wakes), and the request also tries `resume()`. Consequence: the first click or key press is silent; every sound after it plays.

**Not built:** the `AudioEngine` trait with positions / pitch / music from the earlier draft; `PannerNode` positional audio; OGG assets and the transcode pipeline.

### 11.6 Sound event timing

Sounds are requested from the game loop in the same frame as the event (break completion, placement, Satori drop, keg detonation, lightning strike) and play immediately. Footsteps are throttled to every 0.45 s walking / 0.35 s sprinting. The full event wish-list is in §11.9.

### 11.7 Music

**Not built.** The design target is in §11.9.

### 11.8 Discoverability polish — UX polish sweep (2026-07-07)

On top of the particle categories in §11.1, twelve previously silent-or-weak
action beats now carry concrete particle/toast feedback: baby-born on
breeding success (green poof + "A baby &lt;mob&gt; was born!" toast — the
only one of the twelve that was 100% silent before), bonemeal green sparkle
(hand and dispenser paths), tame-success poofs (Cat/Parrot/Fox/Wolf/Nostrich),
breeding love-mode feed puff, seed-plant poof, blueprint-capture burst, Recall
Whistle puff per recalled pet, mob-kill smoke, furnace smelt-complete smoke,
and milk/shear puffs. All additive, no new mechanics; audio cues for the same
beats are deferred (there are only seven sounds, §11.3).

The same sweep rewrote the free-play `H` help panel (previously near-empty)
into a controls cheat-sheet (since rebuilt from the shared controls table —
see §11.10), and added a session-only (not persisted to save)
first-encounter hint system: the first successful tame of any species teaches
the pet-command gesture in the same toast that confirms the tame (see §9.7),
and a one-time hint fires on each of a player's first Recall Whistle, Reach
Claw, and Cat Treat craft. The recipe book (§4.6) gained a one-line "what it
does" `usage` field on 7 non-obvious cards: Reach Claw, Cat Treat, Recall
Whistle, Salt Lick, Bone Meal, Lead, Shears.

### 11.9 Design targets — NOT built

Everything below is the original audio design, kept so a rebuild has a destination. **None of it is implemented** (see §11.3-11.5 for what is). The only parts that exist are: the Master slider (as `master_volume` + mute), the break / place / footstep / explosion sounds, and Web Audio + `rodio` as the two backends.

#### Sound categories and volume sliders

Sound design is critical for game feel. Every interaction must have audio feedback.

**Sound categories**:

| Category | Examples | Volume Control |
|---|---|---|
| Master | Everything | Global volume. |
| Music | Background music, biome themes | Separate slider. |
| Blocks | Breaking, placing, stepping on | Separate slider. |
| Hostile | Brigand shouts, weapon clangs | Separate slider. |
| Friendly | Cow moo, chicken cluck | Separate slider. |
| Players | Footsteps, damage grunts | Separate slider. |
| Ambient | Wind, cave drips, underwater hum | Separate slider. |
| Weather | Rain, thunder | Separate slider. |
| UI | Button clicks, inventory sounds | Separate slider. |

#### Positional audio

All in-world sounds are **3D positional** (except music and UI):

- Sounds attenuate with distance using an **inverse distance model** with rolloff.
- Full volume within **1 block** of the source.
- Linear falloff to silence at **16 blocks** (default; configurable per sound).
- Stereo panning based on the listener's (camera's) orientation relative to the source.
- Vertical attenuation: sounds above or below the player attenuate slightly faster (1.2x multiplier) to account for the fact that vertical distance in voxel worlds often means "through solid blocks." This is a subtle improvement over Minecraft, which does not account for obstruction.

**Sound obstruction** (enhancement over Minecraft): If a direct line between the listener and the sound source passes through more than **3 solid blocks**, the sound is muffled (low-pass filter + 50% volume reduction). This makes caves sound like caves and rewards building enclosed spaces.

#### Audio abstraction and format

Axe'n'Stax targets both web (WASM + WebGPU) and native (desktop) from a single codebase (see ADR-002).

**Audio abstraction layer**:

| Platform | Audio Backend | Notes |
|---|---|---|
| Web (WASM) | Web Audio API | Browser-native. Supports 3D positional audio via `PannerNode`. Limited to formats supported by the browser (typically OGG Vorbis, MP3, WAV). Web Audio API handles mixing and effects natively. |
| Native (Desktop) | `cpal` + `rodio` (Rust crates) | Low-level audio output via `cpal`. `rodio` provides decoding and mixing. Supports OGG Vorbis, WAV, FLAC. |

**Unified audio interface** (Rust trait):

```rust
trait AudioEngine {
    fn play_sound(&self, sound_id: SoundId, position: Vec3, volume: f32, pitch: f32);
    fn play_music(&self, track_id: MusicId, fade_in: f32);
    fn stop_music(&self, fade_out: f32);
    fn set_listener(&self, position: Vec3, forward: Vec3, up: Vec3);
    fn set_category_volume(&self, category: SoundCategory, volume: f32);
}
```

The `AudioEngine` trait is implemented separately for Web Audio API (via `wasm-bindgen` bindings) and native (`rodio`). Game code calls the trait interface and is platform-agnostic.

**Audio format**: All sound assets ship as **OGG Vorbis** (good compression, wide support, royalty-free). The asset pipeline can accept WAV inputs and transcode to OGG during build.

#### Key sound events

Sound responsiveness is as important as visual responsiveness. These sounds must play **within 1 frame** of their trigger:

| Event | Sound | Timing Requirement |
|---|---|---|
| Block break | Material-specific break sound | Immediate on break completion. |
| Block place | Material-specific place sound | Immediate on placement. |
| Footstep | Surface-dependent step sound | Every 0.45 seconds while walking, 0.35 while sprinting. |
| Jump | Soft "hup" + surface sound | On jump initiation. |
| Landing | Impact thud (scaled by fall distance) | On ground contact. |
| Damage taken | Hurt grunt + damage type sound | On HP reduction. |
| Item pickup | "Pop" sound | On item collection. |
| Menu click | Click sound | On UI interaction. |
| Door open/close | Wooden/iron door sound | On state change. |
| Chest open/close | Creak sound | On interaction. |
| Eat food | Crunching (4 bites over 1.6 seconds) | During consumption animation. |
| Explosion | Boom + debris | On detonation. **Implemented Spec 49** (`audio::play_explosion` — low boom + debris noise). |
| Ambient cave | Random cave sounds (every 5-15 minutes) | Low volume, slightly eerie. Not too scary (Genesis is 12). |

#### Music system

- **Background music** plays intermittently: a track plays, then silence for 5-15 minutes (randomised), then another track.
- Music selection is **biome-aware**: different track pools for overworld, caves (below Y=50), and any future AxeNStax-native alternate dimensions when they ship.
- Music cross-fades over 3 seconds when transitioning between biomes.
- Music pauses (not stops) when the game is paused (single-player) or the inventory is open.
- Music volume defaults to **50%** of master volume. Many players play with music off, so this must be easily togglable.

### 11.10 Controls card and the H help sheet (built 2026-10-05, W2)

`game/engine/src/controls.rs` is the **single source of truth** for what the keys do. Two tables (keyboard + mouse, touch) of rows `{keys, action, source, native_only}`; every row cites the real binding it documents (`input.rs`, `lib.rs` key handling, `touch_input.rs`) and unit tests press the documented keys on a real `InputState`. Three surfaces read it:

- **First-spawn controls card** — shown once per device when the first world finishes loading (`GraphicsSettings::controls_card_seen`, persisted like `has_seen_license_onboarding`, so it survives a web reload). A player-0 modal (`GameState.controls_card_open`): frees the cursor, freezes movement, closes with "Got it", Enter or Esc, then re-captures the cursor.
- **Pause menu "Controls" button** (normal and Trial pause menus) — reopens the same card any time.
- **Free-play H help sheet** — built from the same tables (`controls::help_sheet_text`), replacing the hand-written cheat-sheet.

**Platform:** the touch table is used when `touch_input::is_touch_device()` is true (set from `navigator.maxTouchPoints` on the web, unconditionally on Android); keyboard + mouse otherwise. `TOUCH_PLATFORM` is deliberately not used — it is true for every web build, including a desktop with a mouse. **Gamepad is parked: the card makes no gamepad claim.**

**Loading tips** (`assets/loading_tips.json`, shared by the engine and the web loader) no longer contain "New — tell us how it feels" cards or gamepad mentions: the web build has no feedback channel and native `/bug` is off by default behind a hidden tester unlock, so no tip may ask for feedback (a unit test in `loading_screen.rs` enforces it).

---

## 12. Local Co-op and Split-Screen Systems

Local co-op allows two players to share a single machine, window, and game session. Player 0 owns the keyboard and mouse; Player 1 owns a gamepad. All gameplay systems tick independently per player within a single shared world simulation.

### 12.1 Multi-Player Input Routing

Input is collected from all sources (keyboard, mouse, gamepads) and dispatched to the correct player each tick via a `PlayerIntent` abstraction:

```rust
pub struct PlayerIntent {
    pub move_forward: bool,
    pub move_backward: bool,
    pub move_left: bool,
    pub move_right: bool,
    pub jump: bool,
    pub sneak: bool,
    pub sprint: bool,
    pub attack: bool,
    pub use_item: bool,
    pub scroll_delta: f32,
    pub look_delta: (f32, f32),   // (dx, dy) in radians
    pub hotbar_slot: Option<u8>,  // direct slot select (1-9 keys or D-pad)
}
```

**Default two-player routing**:

| Source | Player |
|---|---|
| Keyboard + mouse | Player 0 |
| Gamepad 0 (first connected) | Player 1 |

**With two gamepads**:

| Source | Player |
|---|---|
| Keyboard + mouse + Gamepad 0 | Player 0 |
| Gamepad 1 (second connected) | Player 1 |

The routing table is determined at session start and is not hot-swapped mid-game (to avoid accidental reassignment). A future settings menu will allow explicit source-to-player assignment.

**Look delta sources**:

- Mouse: `mouse_dx * sensitivity`, `mouse_dy * sensitivity` — accumulated since last tick, applied to Player 0's yaw/pitch. `sensitivity` defaults to `graphics_settings::DEFAULT_SENSITIVITY` (0.002, lowered from 0.003 on 2026-06-16). A persisted profile (native `settings.json` / web `axenstax_gfx`) still carrying the old 0.003 default is migrated onto 0.002 on load (`GraphicsSettings::load` → `migrate`, 2026-09-06) — a deliberately-chosen value that merely differs from both defaults is left untouched.
- Gamepad right stick: `stick_x * gamepad_look_speed`, `stick_y * gamepad_look_speed` — per-tick, with deadzone (inner 15% of stick travel ignored) and non-linear curve (square root of stick magnitude for fine control near centre).

**Gamepad button mapping** (default, remappable). Table corrected 2026-06-12
to match the shipped code (`gamepad.rs::state_to_intent`) — the earlier
version of this table (B=sneak, X=sprint, Select=inventory) had drifted from
the implementation:

| Gamepad Button | Action |
|---|---|
| Left stick | Move (analog) |
| Right stick | Look |
| A (South) | Jump, double-tap = toggle flight (in-game) / take-or-place whole stack (crafting slot-cursor) / Confirm (menus, injected as Enter) |
| B (East) | Toggle inventory open/closed (in-game) / Back (menus, injected as Escape) |
| X (West) | Cycle camera perspective (in-game) / split stack or place one (crafting slot-cursor) |
| Y (North) | Toggle debug overlay |
| Right trigger (RT) | Attack / break |
| Left trigger (LT) | Use item / place |
| Right bumper (RB) | Scroll hotbar right |
| Left bumper (LB) | Scroll hotbar left |
| L3 (left stick click) | Toggle sprint |
| R3 (right stick click) | Toggle sneak (2026-06-12) |
| D-pad down | Drop one held item (in-game, Bedrock standard, 2026-06-12) |
| D-pad | Move crafting slot-cursor (inventory open) / navigate menus |
| Start | Pause menu |

**Controller mapping matches Minecraft console standard**: RT = break (right index finger, dominant action), LT = place (left index finger). This was initially mapped backwards and caused confusion during testing.

**Menu navigation** (reworked 2026-06-12): d-pad/A/B are injected into the
egui `RawInput` **before** `Context::begin_pass` — egui's focus system
consumes Arrow/Tab keys from the RawInput at the start of the pass, so the
previous post-pass `ctx.input_mut` injection reached `key_pressed()` checks
(A=Enter activated, B=Escape dismissed) but never actually moved focus; the
d-pad was a silent no-op on menus. If no widget has focus yet, the first
d-pad press is sent as **Tab** to seed focus on the first interactive widget;
subsequent presses ride egui 0.34's directional arrow-key navigation
(`FocusDirection`). Focused widgets show a gold ring: standard widgets via
the global `active.bg_stroke` style (egui renders `has_focus` with the
"active" visuals), custom-styled lobby buttons and world cards via
`menu::focus_ring`. Enter/Space fake-click the focused widget
(`Sense::click()` is focusable), so A activates world cards and buttons.

**Inventory/crafting slot-cursor** (2026-06-12): with the crafting UI open,
the d-pad moves a gold-highlighted slot cursor (`craft_ui::PadSlot`,
`pad_move` — pure, clamped, unit-tested) across armour column | crafting
grid | result | main rows | hotbar. A emits the slot's left-click
`ClickTarget` (take/place whole stack), X the right-click one (split-half /
place-one); armour and result have no right-click semantics so both buttons
act the same there. The cursor stays `None` until the first d-pad press, so
mouse users never see it; the first press seeds it at hotbar slot 0 without
moving. While active, the carried item anchors beside the focused slot
(there is no meaningful pointer on a pad) and the footer legend swaps to
button hints. Presses are read per-frame in the render path (snappier than
the 20 TPS tick) and feed the same `craft_clicks` queue as mouse clicks.
The press-A-to-join rule is suppressed while any player's crafting UI is
open — solo KB+M P1's UI listens to the unowned `gamepads[0]`
(`local_join::ui_pad_index`), which is exactly the pad the join rule
watches, so an A press aimed at a slot must not seat P2. The menu-open
intent gate also zeroes `camera_cycle` so X doesn't cycle the camera behind
the panel. Deferred: chest/furnace/vendor panels reuse the same PadSlot
pattern (own spec), console-style recipe book (blocked on a data-driven
recipe registry — `crafting::match_recipe` is procedural), left-stick
navigation with key-repeat, d-pad direct hotbar select.

### 12.2 Player Join Flow (up to 4 local players)

Player assignment follows two simple rules:

1. **Whoever pressed Play on the main menu is Player 1** (keyboard+mouse, or controller if they clicked Play with A).
2. **Any unowned controller that presses A joins as the next player** (up to 4 total). No popup, no prompt, no configuration dialog. The controller that pressed A is the one bound to the new character — the per-tick intent collector uses the same formula (`local_join::expected_controller_index`).

An earlier design showed an A/B popup when a controller connected, asking the player to choose their role. This was over-engineered and confusing (especially for kids). The implicit join-by-pressing-A approach is simpler and matches console game conventions.

**Join sequence**:

1. **No gamepad connected**: single-player mode. One viewport fills the window. Keyboard+mouse only.
2. **Gamepad presses A** (at any point — startup or mid-game): the engine detects the button press via the `gilrs` event stream.
3. **Transition to split-screen**:
   a. A new `PlayerSlot` is created (spawn position: fanned along +X from Player 0's current position so successive joiners don't pile on each other; full health; fresh inventory; flying flag inherited from world creative mode).
   b. `compute_screen_layout` is called with the new player count and returns 2 side-by-side / 3 in a 2×2 with the bottom-right empty / 4 full quad viewports (see §12.6).
   c. A new `PlayerGpuResources` (camera buffer, bind group, entity buffer, wire buffer) is allocated.
   d. The chunk streaming system switches to union mode (see 12.4).
   e. The new layout takes effect immediately on the next frame; in the 3-player case the bottom-right quadrant shows a "Waiting for Player 4" join prompt.
4. **Gamepad disconnected mid-game**: that player's `PlayerIntent` falls back to `PlayerIntent::default()` (no movement, no look, no actions). The character stops moving. The split-screen layout is retained until the player explicitly leaves via the pause menu or the controller reconnects. A HUD message ("Controller disconnected") appears in their viewport.

### 12.3 Per-Player Gameplay Systems

Every gameplay system that has per-player state ticks independently for each player. The world simulation (blocks, mobs, fluids, signals) is shared and authoritative. Player systems that tick per-player:

| System | Per-player behaviour |
|---|---|
| **Movement / physics** | Each player has independent position, velocity, on-ground state, and yaw/pitch. Physics tick at 20 TPS per player. |
| **Camera** | Each player has an independent camera derived from their position + look angles. Different FOV computation (half-width viewport). |
| **Block interaction** | Each player has an independent block target (ray from their own eye position), break progress, and placement preview. |
| **Inventory** | Fully independent inventories. Players do not share items unless they drop and pick up. |
| **Crafting UI** | Independent open/close state. Player 0 can have crafting open while Player 1 is mining. |
| **Health / hunger** | Independent HP, hunger, and saturation bars. Damage to one player does not affect the other. |
| **Combat** | Independent attack cooldowns, hit detection, knockback. |
| **HUD** | Independent hotbar, hearts, block name label, debug overlay. Each rendered in that player's viewport (see Spec 03 section 12.6). |
| **Crosshair** | Independent crosshair in each player's viewport, centred on that viewport. |

**Shared systems** (single instance, affects all players):

- World block data (chunk grid) — all players see the same blocks.
- Mob simulation (AI, health, position) — mobs react to all players.
- Day/night cycle, weather, fluid simulation, signal system.
- World save — one world file, all player states serialised together.

### 12.4 Union Chunk Streaming

In single-player, the chunk loader maintains chunks within render distance of the one player. In co-op, chunks must be loaded for the **union of all player positions**:

```rust
fn required_chunks(players: &[PlayerState], render_distance: u32) -> HashSet<ChunkPos> {
    let mut required = HashSet::new();
    for player in players {
        let player_chunk = world_pos_to_chunk(player.position);
        for dx in -(render_distance as i32)..=(render_distance as i32) {
            for dz in -(render_distance as i32)..=(render_distance as i32) {
                required.insert(ChunkPos {
                    x: player_chunk.x + dx,
                    y: player_chunk.y,
                    z: player_chunk.z + dz,
                });
            }
        }
    }
    required
}
```

A chunk is **kept loaded** if it is within render distance of any player. It is **unloaded** only when all players have moved beyond render distance.

When players are close together (same area), the union is almost identical to single-player — no extra chunks. When players separate significantly, the union grows: at maximum separation across a world, both halves of the loaded region are retained. The chunk streaming budget (load rate, mesh upload rate) is not doubled — the same budget is distributed across the larger required set, which may mean slower loading at the far player's position when both players are exploring different areas simultaneously.

**Frustum culling** is still per-player: chunks in the union set but outside a player's view frustum are not drawn for that player.

### 12.5 Multi-Player Save and Load

The world save format stores a `Vec<PlayerSaveData>` to accommodate multiple players:

```rust
#[derive(Serialize, Deserialize)]
pub struct PlayerSaveData {
    pub player_index: usize,
    pub position: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub health: f32,
    pub hotbar_slot: u8,
    pub inventory: Vec<(u8, ItemStack)>,  // (slot_index, item)
}

#[derive(Serialize, Deserialize)]
pub struct WorldSave {
    pub seed: u32,
    pub world_time: u64,
    pub players: Vec<PlayerSaveData>,
    // ... other world fields
}
```

**Save**: All active players' states are serialised into `players`. A single-player save has one entry (`player_index: 0`).

**Load — backward compatibility**: When loading a save that has only one `PlayerSaveData` entry, Player 0 is restored from it. If a second player joins mid-session, Player 1 spawns at the world spawn point with a fresh inventory (the save had no data for them). This is acceptable: the save captured the state at save time; new joiners start fresh.

**Load — two-player save into single-player session**: If a save has two `PlayerSaveData` entries but no gamepad is connected, only `player_index: 0` is loaded. `player_index: 1` data is retained in the file but ignored until a gamepad is connected.

**Auto-save**: The world auto-saves every 5 minutes and on pause-menu "Save" or "Save and Quit". All active player states are captured at the moment of save.

### 12.6 Quad Split-Screen and Window-Size Floors

Local co-op supports 1–4 players via `compute_screen_layout` (Spec 03 §12.6). Layouts:

| Players | Layout |
|---------|--------|
| 1 | Single viewport fills the window. |
| 2 | Side-by-side (vertical split). |
| 3 | 2×2 quad with bottom-right empty; the empty quadrant shows a "Waiting for Player 4 — press A on any controller to join" prompt. |
| 4 | 2×2 quad, all four quadrants active. |

**Recommended minimum window size for 4-player split-screen is 1280×720** (so each quadrant is at least 640×360). The HUD elements (hotbar, hearts, hunger) shrink to 0.75× scale at viewport widths below 720 px to keep an ergonomic margin inside each quadrant. The crafting UI refuses to open in viewports narrower than 600 px — the player is shown a toast "Window too small for crafting — make the window bigger" and the inventory remains closed until the window grows or they switch to a smaller player count.

Below 1024×600 (per-quadrant 512×300) the game still runs but is below comfortable play. Above 1080p the HUD stays pixel-identical to single-player.

---

## 12.15 Creator Gallery — visitor kiosk loop (showcase mode) (Phase 2, 2026-06-19)

A **showcase / kiosk-containment** mode turns a dedicated-server world into an
unattended booth: a guest opening the `:8443` page is contained (no escape to the
normal lobby), clicks exhibits to **collect** them into a personal **basket**, and
**exits** to one terminal screen. Spec
`docs/superpowers/specs/2026-06-19-creator-gallery-showcase-design.md` (§8/§10).

- **A flag, NOT a `Scenario` and NOT a `GameMode` variant.** A Scenario's end-card
  returns to the lobby — the exact back-door showcase removes. The pure
  `showcase` module holds `ShowcaseConfig` (`enabled`/`exit_action`/`auto_loop_secs`),
  `should_dead_end_exit`, `ExitDisposition`, the ephemeral per-visitor `Basket`
  (dedupe-by-`image_ref`, `total_price`; **never saved**), and `pick_exhibit`
  (ray-vs-quad, nearest in reach — exhibits are free quads the block raycast can't
  hit). `GameState` carries `showcase`/`basket`/`showcase_exited`.
- **Armed by flag, off by default.** Native server: `--showcase`/`AXENSTAX_SHOWCASE`
  (+ `--exit-action`, `--auto-loop-secs`). Web kiosk: the same `window.AXENSTAX_*`
  globals on `index.dedicated.html` (default OFF), read via
  `ws_transport_web::showcase_config`. Every kiosk branch is flag-guarded, so a
  normal server and the normal PWA are byte-identical.
- **Safe default = Adventure** (read-only) so an anonymous guest can't grief
  (spec §13 #4); building locked; operator moderation via the console.
- **Containment.** When `should_dead_end_exit`, the three exit transitions
  (ESC-quit + both pause-menu quit arms) set `showcase_exited` instead of going to
  the lobby/quitting; `update_and_render` then short-circuits to the full-screen
  terminal `showcase_ui::draw_exit_screen` (rendered from the basket via the
  `ExitAction` slot — `Board`/CTA now, checkout in Phase 4). Optional auto-loop
  resets the session for an unattended booth.
- **2a demo-kiosk slice:** containment + Adventure + exit CTA on a pre-built world
  with **no exhibits** — an independently shippable events demo needing none of
  the curator tooling.
- **Click-to-collect** is app-glue verified by playtest
  (`docs/test-sheets/2026-06-19-creator-gallery-test-session-2.md`); the pure
  cores (config/basket/picker/summary) are unit-tested. No save-format change
  (the basket is session-ephemeral).

## 12.14 Creator Gallery — exhibit authoring (`/exhibit`) (Phase 1, 2026-06-19)

An artist curates a walkable gallery **in-world, natively** (authoring never
depends on the web edit path; spec
`docs/superpowers/specs/2026-06-19-creator-gallery-showcase-design.md`). An
**Exhibit** is a placed, sized 2D art surface (Spec 02 §1.5; rendering Spec 03
§13): a **Wall** piece (flat on a wall, faces the room) or a **Standing**
Y-axis billboard (faces the viewer, transparent PNG cut-out). The artist drops
image files in `worlds/<name>/exhibits/` and places/edits exhibits with the
`/exhibit` command (alias `/ex`), which mutates the live `World.exhibits` list
(snapshotted to the save, travels in `.axeworld`).

- **Commands** (`OpLevel::Op`, `is_cheat` — authoring is creative power, flags the
  World Integrity Ledger; **`/exhibit list` is read-only**, `OpLevel::Op` but no
  cheat flag):
  - `place <image> <wall|standing> [w] [h]` — place at what you're looking at. A
    **wall** anchors in the cell adjacent to the struck face and faces the room
    (floor/ceiling faces rejected); a **standing** piece sits on top of the
    looked-at block, its starting yaw the camera yaw snapped to 45° (the billboard
    re-points each frame anyway). Default size 2×2.
  - `list` — every exhibit with its index, kind, size, position, label.
  - `move <i>` — re-anchor #i to where you're looking (re-resolves by its kind).
  - `resize <i> <w> <h>` · `yaw <i> <deg>` · `image <i> <ref>` · `label <i> <text>` · `delete <i>`.
- **Pure core.** The placement/edit logic (`resolve_placement`, `yaw_from_face`,
  `apply_{resize,yaw,move,set_image,set_label,delete}`, `EditError`) lives in the
  pure `exhibit` module and is unit-tested; the command is a thin parser over it,
  so a future config panel / image-picker (deferred) reuses the same functions.
- **Reserved payload** (`link`/`sku`/`price`) exists from Phase 1, consumed by the
  Phase 2 kiosk loop (basket/collect) and Phase 4 commerce. Phase 1 excludes the
  visitor kiosk, basket, collect, and exit screens.
- **Affordance feel** (is typing filenames ergonomic? default sizes? billboard
  feel?) is a playtest:
  `docs/test-sheets/2026-06-19-creator-gallery-test-session-1.md`.

## 12.13 Combat sweep attack (#23)

A melee swing hits the primary (crosshair) mob for full damage and **every other mob in the swing arc** for `SWEEP_DAMAGE_FRACTION` (0.4) × damage + half-knockback — the Minecraft-style sweep. `combat::in_swing_arc(to_target, look_dir, reach, min_dot)` is the shared pure cone+reach predicate; `player_attack` picks the target and `combat::strike` (shared with a joiner's server-side swing since MP-D2b) collects the other arc targets and applies sweep damage + `LastAttacker` credit. PvP balance is multiplayer-gated; this is the vs-mob model. Swing animation + true swept-volume deferred. Spec: `docs/foundations/2026-06-16-combat-sweep-attack.md`.

## 12.12 Accessibility narration (#24)

Opt-in Web Speech (TTS) that speaks the hotbar selection on change ("Slot 3: Stone"). `narration.rs`: pure `hotbar_phrase`/`health_phrase` + `Narrator` throttle; `speak()` uses `speechSynthesis` on web (no-op native). `GraphicsSettings.narration_enabled` (default off) + Settings checkbox. Broader UI narration + 3D sound-cue orientation deferred. Spec: `docs/foundations/2026-06-16-accessibility-narration.md`.

## 12.11 Minecraft schematic import (#10)

`/import <path.schem>` reads a Sponge `.schem` (gzipped NBT) → `PlanData` and adds it to `world.plan_registry` (use it with `/buildguide`). Hand-rolled NBT reader (`nbt.rs`, no new crate) + `schematic.rs` (v2/v3 palette + LEB128 `BlockData` + best-effort `map_mc_block`: known→equivalent, air→skip, unmapped→stone to keep the shape; oversize >255/axis rejected). `PlanData::from_imported` + `PlanRegistry::add`. Native-only for now (`.litematic` + WASM in-page picker deferred). Spec: `docs/foundations/2026-06-16-schematic-import.md`.

## 12.10a Guided build-along for schematics — SHIPPED 2026-09-06

Laying an `Item::Plan` is no longer a one-shot insta-build: it **branches on game mode + materials** (`game_loop`, the Phase 8 ghost-confirm handler).

- **Survival, short of materials** → the plan is laid as a **build-guide** in layer mode instead of erroring. Nothing is consumed; the HUD panel leads with a **"Go and gather"** shortfall measured against the player's inventory (`build_steps::shortfall` over `plan::inventory_block_count`).
- **All materials, or Creative** → a modal picker (`hud_ui::draw_build_choice` → `BuildChoice::{Auto, GuideBlocks, GuideLayers, Cancel}`) offers **Build it automatically** (the existing `plan::start_build` animated builder, unchanged) or a **guided build-along** in one of two step modes. `pending_build_choice` gates gameplay input like any other modal; Esc closes it via the centralised `EscAction` ladder (`EscAction::CloseBuildChoice`).

The step sequencer is a new pure module **`build_steps.rs`**: `StepMode::{Whole, BlockByBlock, Layers}`, `plan_steps` (cell indices per step; ordering mirrors `plan::order_cells_for_build`, so guided and automatic builds lay a plan in the same order), `step_complete`/`advance_step` (a step advances **only** when every cell in it verifies `Correct` — a `Wrong` block holds it, reusing `build_guide::verify`, no new validation), `step_of_cell`/`focus_of` (current / future for the ghost render), `step_materials` (what this step still wants) and `shortfall`. `BuildGuide` carries `mode`, `steps` and `current_step`; all three are **transient** on `GameState` — a guide re-lays in one click from the Plan item, so no `WorldSave` field was spent.

Render (`game_loop::refresh_build_guide`): the live step's missing cells are **bright**, later steps **faint**, wrong cells **red**, and correct cells draw nothing (the placed block *is* the done state). HUD: **"Step N of M"** plus the per-step material line. Mode picker = the lay dialog, or `/buildguide mode <block|layer|whole>` on an active guide (`CommandResult::SetBuildGuideMode`); `/buildguide <plan> [mode]` takes a trailing mode word. Satoshi's gifted starter hut (`satoshi::starter_hut_plan`) runs this exact flow — the gift teaches building rather than handing over a house. Deferred (P4): smarter logical grouping (connected wall sections, roof as a unit), back-a-step, pacing tuning. Spec: `docs/superpowers/specs/2026-06-23-guided-build-along-schematics-spec.md`.

## 12.10 Blueprint build-guide (#9)

`/buildguide <plan>` projects a captured `PlanData` (from `world.plan_registry`) as a build-along ghost anchored at the player's feet: white wireframe ghosts where blocks are missing, red where a wrong block sits — they vanish as you build correctly (`build_guide.rs` `verify`; `renderer::set_build_guide_markers` per-cell colours, Pass 2.5c). A HUD panel shows progress + the named material list (`remaining_materials`). `/buildguide off|list`. Not a cheat (places nothing). The reverse of plan capture. **Superseded as the player-facing path by §12.10a** (2026-09-06) — laying a Plan item is now the way in, and this command gained the step modes described there. Spec: `docs/foundations/2026-06-16-blueprint-build-guide.md`.

## 12.9 Spawn-proof overlay (#8)

**F7** toggles a builder overlay (`spawn_overlay.rs`): red wireframe markers on every surface cell where a hostile mob can spawn at night. The engine's spawn gate is **binary block-light** — `is_spawnable_surface` = solid, non-water surface with `block_light_at(x, y+1, z) == 0` (mirrors `spawning::tick_mob_spawning`) — so it's one colour, not Minecraft's graded scale. Rendered via a dedicated `spawn_marker_buffer` (Pass 2.5b, parallel to ghost placement); scan throttled, player 0. Spec: `docs/foundations/2026-06-16-spawn-proof-overlay.md`.

## 12.8 WorldEdit region editing (#7)

Creative-power mass editing via the `/we` command group (`worldedit.rs` pure ops + per-player `WorldEditSession` on `PlayerSlot`): `pos1`/`pos2` (corners at your feet), `set`/`replace`/`walls`, `copy`/`paste`, `stack <n> [x|y|z]`, `size`/`clear`. Every op is volume-capped (`MAX_REGION_VOLUME`) and returns `CommandResult::RebuildRegion { min, max }` so the game loop re-meshes the touched chunks. **Deferred**: in-world selection-box render (shared overlay buffer with #8), a click-wand, brushes/rotate/flip/sphere/undo. Spec: `docs/foundations/2026-06-16-worldedit-region-editing.md`.

## 12.7 Tiered storage (#15)

Storage **tiers** (`chest.rs` `ChestTier`): the wood `CHEST` is tier 0; Copper / Iron / Diamond / Satori chests add capacity (27/36/45/54/72 slots) — `ChestData.tier` (serde-default `Wood`, append-only) sizes the slot vec, and `ChestData::for_tier` is used on first open. Recipes ring 8× the tier material around a wood chest. **Auto-collect**: Diamond+ chests (`ChestTier::auto_collects`) hoover nearby dropped items within `CHEST_ABSORB_RADIUS` via `tick_chest_autocollect` (throttled). **Deferred** (the Sophisticated-Storage upgrade slots): upgrade-module items + install slots, item filters, stack-size multipliers, backpacks. Gold is omitted (no gold material). Spec: `docs/foundations/2026-06-16-tiered-storage.md`.

## 12.6 Map & Waypoints (#6)

The minimap + full-screen map render is specified in **Spec 03 §10**; the gameplay-facing pieces:

- **Waypoints** are per-world named markers (`Waypoint { id, name, pos, colour, kind: Manual|Death }`), persisted in `WorldSave.waypoints` (append-only, serde-default). They show on the minimap (in-range dots) and the full-screen map (dots + labels + list).
- **Death markers** auto-drop a `Death` waypoint where the player falls, on the `just_died` one-shot, **regardless of keep-inventory** (so the way back is always recorded). Rolling cap `MAX_DEATH_MARKERS` — the oldest death marker rolls off; manual pins are never evicted.
- **Commands** (`/waypoint` aka `/wp`, `OpLevel::None` — navigation is not a cheat): `add <name>` drops a pin at your feet; `list`; `remove <name>` (case-insensitive); `tp <name>` teleports to a waypoint (**creative only**). The full-screen map UI mirrors these (pin map-centre, per-row remove, TP-in-creative).
- **Full-screen map** opens with **M** outside the Workshop (M cycles edit-symmetry inside it); gameplay input is frozen while open, like the chat overlay.

## 13. Player Identity

A player's identity has two parts: a **persona pubkey** (32-byte x-only Nostr key from their Signet-app install) and a **handle** (display name). Both come from Signet — the pubkey is derived via `nsec-tree`, the handle is published by Signet-app as a `kind 31000` display-name credential signed by the persona key.

The handle is never typed into AxeNStax or sent by the client as an untrusted string. The game — both client HUD rendering and server authority — reads it from the signed credential. A user changing their handle in Signet-app publishes a superseding credential; games pick up the new handle on the next handshake (Phase 2+ multiplayer will periodically re-query to pick up supersessions between sessions).

**Consequences for gameplay systems:**

- **Nameplates / chat author / leaderboard**: all render `ServerPlayer.handle`, which is credential-sourced. If no credential is attached, the host's contacts book names a known npub first (the only name rendered bare); failing that the credential or typed join name is shown but ALWAYS tagged `-<npub suffix>` (display fallback only; generic `Player` ignored), and failing that a short npub (`npub1abcd…wxyz`); guests always read `<name> (guest)` — never hex (Spec 04 §1.8.1 step 3, T2-8).
- **Bans**: keyed on persona pubkey, not on handle. A banned player can't escape by renaming; a legitimate player can safely pick any handle.
- **Personas are first-class for gaming**: the expected pattern is that players maintain a **gaming-specific persona** in Signet-app, separate from their natural-person identity. Their real name stays on-device; their gamer handle is what every game platform sees.
- **Alpha**: website access-request and `/play/` gate both use the same persona pubkey + kind 31000 credential. Multiplayer will use the same artefacts once it lands.

Protocol details (handshake shape, signature verification, supersession handling, open questions) live in `docs/spec/04-networking.md §1.8`. Threat model for impersonation lives in `docs/spec/08-security-anti-cheat.md §9.0.1`.

---

## Appendix A: Gameplay Constants Registry

All tuneable gameplay values in this document are collected in a single configuration namespace. Default values are defined in the engine. Server operators can override any value in their server configuration. Plugin authors can override values programmatically.

```toml
[movement]
walk_speed = 4.317
sprint_speed = 5.612
sneak_speed = 1.295
fly_speed = 10.89
fly_sprint_speed = 21.78
swim_speed = 2.20
swim_sprint_speed = 5.612
climb_speed = 2.35
jump_velocity = 0.42
terminal_velocity = 78.4
ground_drag = 0.91
air_drag = 0.91
ice_drag = 0.98
water_drag = 0.80
air_acceleration = 0.02
ground_acceleration = 0.1
step_up_height = 0.5
sneak_edge_detection = true

[combat]
attack_reach = 3.0
attack_cooldown_ticks = 10
critical_hit_multiplier = 1.5
knockback_base = 0.4
knockback_sprint_bonus = 0.4
sweep_attack_enabled = true
sweep_attack_damage = 1
combat_mode = "standard"  # "classic" | "standard" | "custom"

[interaction]
block_reach_survival = 4.5
block_reach_creative = 5.0
max_placements_per_second = 5   # 4-tick cooldown at 20 TPS = 5 placements/sec
break_animation_stages = 10

[health]
max_hp = 20
natural_regen_rate = 0.25     # HP per second (= 1 HP per 4 seconds)
rapid_regen_rate = 2.0        # HP per second
starvation_rate = 0.25        # HP per second
max_hunger = 20
sprint_hunger_threshold = 6
regen_hunger_threshold = 18

[world]
day_length_ticks = 24000
random_tick_speed = 3
fluid_updates_per_chunk_limit = 1024
mob_spawn_cycle_interval = 400
hostile_mob_cap = 70
passive_mob_cap = 10

[fall_damage]
safe_distance = 3.0
damage_per_block = 1.0

[server]
tick_rate = 20
position_correction_threshold = 0.1
position_teleport_threshold = 4.0
max_pathfinding_requests_per_tick = 200
chat_rate_limit = 3  # messages per second

[particles]
max_active = 4096
lod_simplify_distance = 32.0
lod_cull_distance = 64.0

[audio]
default_sound_range = 16.0
obstruction_block_threshold = 3
obstruction_volume_reduction = 0.5
footstep_interval_walk = 0.45
footstep_interval_sprint = 0.35
```

---

## Appendix B: System Interaction Map

How the gameplay systems interact with each other:

```
Movement ──> Collision Detection ──> Block Grid (Chunks)
    │                                      │
    ├──> Fall Damage ──> Health System      │
    │                        │              │
    │                   Death/Respawn       │
    │                        │              │
    │                   Inventory (drops)    │
    │                                      │
Block Interaction ──> Inventory (consume/add items)
    │                     │
    │                Crafting System ──> Recipe Registry
    │                     │
    │                Tool System ──> Tool Durability
    │                     │
    │                Enchantment Hooks (future)
    │
    ├──> Signal System (redstone) ──> Block Updates
    │         │
    │    Fluid Simulation ──> Block Updates
    │         │
    │    Fire Spread ──> Block Updates
    │
    ├──> Crop Growth ──> Random Tick System
    │
    └──> Day/Night Cycle ──> Mob Spawning
                                  │
                             Mob AI ──> Pathfinding
                                  │         │
                             Combat ──> Health System
                                  │
                             Loot Tables ──> Item Entities ──> Inventory

Chat/Commands ──> Permission System ──> All Systems (admin control)

Particle System <── All interaction events (visual feedback)
Sound System <── All interaction events (audio feedback)
```

---

## Appendix C: Alpha Milestone Priorities

Per the Platform Overview, early alpha priorities are **movement feel, block placement responsiveness, and chunk streaming smoothness**. This translates to the following implementation order:

### Phase 1 (Must-have for first playable)
1. **Player movement** (section 1) — All speeds, jumping, sprint-jumping, collision detection, sneaking edge behavior.
2. **Block breaking and placement** (section 2) — Progressive breaking, placement rules, client-side prediction, server validation.
3. **Hotbar and basic inventory** (section 3) — Hotbar selection, item stacking, pick up/drop. Full inventory UI can follow.
4. **Particle and sound feedback** (section 11) — Block break/place particles and sounds. Footstep sounds. These are mandatory for the game to "feel" right.

### Phase 2 (Core survival loop)
5. **Full inventory and crafting** (sections 3, 4) — Main inventory grid, crafting table, shaped/shapeless recipes.
6. **Tool system** (section 5) — Tool tiers, durability, mining speed modifiers.
7. **Health, hunger, and fall damage** (section 6) — HP, damage types, food, basic combat.
8. **Day/night cycle** (section 7.5) — Visual cycle, light level changes.

### Phase 3 (World comes alive)
9. **Mob system** (section 9) — Passive mobs first (cows, pigs), then hostile (Brigands, Marauders). Spawning, AI, pathfinding.
10. **Furnace/smelting** (section 4.5) — Smelting recipes, fuel system.
11. **Fluid simulation** (section 7.2) — Water and lava flow.

### Phase 4 (Depth and systems)
12. **Signal system / redstone** (section 7.1) — Wire, sources, consumers, repeaters.
13. **Farming** (section 7.4) — Crops, farmland, bone meal.
14. **Combat polish** (section 6) — Ranged combat (bow), armour, full combat modes.
15. **Game modes** (section 8) — Creative, spectator, adventure.
16. **Chat and commands** (section 10) — Full command system, permissions.

This ordering ensures the **Genesis test** passes at every phase: at any point, the game should feel good for what it has, even if incomplete.
