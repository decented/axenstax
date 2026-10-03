# The Workshop — blow-up-and-sculpt gesture (aim a real block, inflate it, edit it in-world with dyes)

**Status:** ✅ **BUILT + merged to main** (Bellows / blow-up / paint-sculpt — `workshop.rs` / `workshop_painter.rs`; commits 2151660/ebaecba/53e7eed/7e0b0f1). Owner feel/UX playtest pending. Reconciled 2026-06-19. Brainstormed with Axolittle (gameplay/vibe owner of every decision below).

This **evolves the authoring front-end of Spec 40** (the Workshop —
`docs/foundations/2026-06-04-the-workshop-community-redesign.md`, BUILT). Spec 40's backend
(override registry, micro-model bake, the Workshop world, Beacon share) is **reused unchanged**.
What this redesigns is the *gesture*: instead of `/ws place <asset>` → stepwise-pump the nearest
mannequin → paint on a 2D 16×16 grid panel, the player **aims the Bellows at a block they actually
placed**, **holds to blow it up to a fixed working size**, and **edits it in-world with the
crosshair** — painting texels and placing/carving coloured microblocks with **dyes in hand**. It
**unifies Spec 40's Mode A (reskin) and Mode B (reshape) into one in-world editor.**

**Supersedes (within Spec 40, when built):** the nearest-parked stepwise Bellows handler
(`game_loop.rs:4710-4752`), `/ws place` as the *primary* placement path, and the 2D paint-grid
(`workshop_painter.rs` / `/ws edit`) as the *primary* painter. Those are kept as a **power-user /
test fallback**, not removed.

**The Workshop is a *design room*, not a single-tool screen.** This spec is its **appearance** tool
(the Bellows — redesign how blocks *look*). Its **structure** tool — capturing blueprints from
normal blocks, tagged *creative* provenance — is a **sibling spec**
(`2026-06-08-workshop-blueprint-authoring.md`), reusing the existing blueprint/plan system. Same
room, two focused tools — the long-run shape is *the Workshop hosts tools*, each its own concern.

---

## 2026-06-18 — feel/UX hardening (BUILT, owner playtest pending)

Four corrections from owner feedback on the in-world editor. All shipped behind
the existing Workshop gates; `check.sh` green (2957 tests).

- **§5 Original block hidden while blown up.** The placed block stayed in the
  world at the cage's near corner, so its original texture z-fought through the
  inflated copy ("a small corner shows the original"). Fixed **render-only**: a
  `World.render_hidden` set (rebuilt each tick by
  `GameState::reconcile_workshop_render_hidden` from
  `WorkshopProjects::blown_up_origins`) makes the mesher (`greedy_face` +
  `emit_non_solid_blocks`) sample blown-up origins as AIR. Block **data is
  untouched**, so raycast (charge continuation + sneak-collapse both aim at the
  real block), persistence, and physics are unaffected; the block reappears
  (reskinned) on collapse/pin. Test: `mesh::tests::render_hidden_block_emits_no_faces`.
- **§6 Explicit Paint/Sculpt mode.** The old rule (a dye paints, *empty hand or
  any tool* carves) let you chip geometry by accident. Replaced by a deliberate
  room mode `EditMode{Paint(default), Sculpt}`, toggled with **V** (HUD hint +
  toast show the current mode). Carving/placing is **only** possible in Sculpt
  mode — a textured block can't be turned into a microblock by accident.
- **§7 One-way preserved.** Paint mode never sets `shape_dirty`, so a paint-only
  block still pins as a texture; only a deliberate Sculpt carve/place makes it a
  shape. There is still no microblock→texture path. (Already true; §6 makes the
  accidental flip impossible.)
- **§8 Paint = a dye, one pixel.** The single source of truth is the pure fn
  `workshop::resolve_edit_action(mode, dye, break, place)`: only a held **dye**
  (`MaterialId::paint_block()` → `Some`) yields a paint colour, so holding a
  *block* can never stamp its texture; paint hits exactly the aimed cell.
- **Obtainability fix (separate, same batch):** the Bellows + carts + the
  post-`HempSeeds` dyes/seeds were missing from `ALL_MATERIAL_IDS`, and
  Eraser/Slingshot/Drafting Stamp from `all_tool_combos()` — so the Bellows was
  unobtainable from the creative inventory once removed from the hotbar. All now
  enumerated (`inventory_explorer.rs`).

Tests: `workshop::tests::{edit_mode_defaults_to_paint_and_toggles,
paint_mode_only_paints_with_a_dye_and_never_carves,
sculpt_mode_carves_on_break_and_places_with_a_dye, blown_up_origins_lists_parked_balloons_only}`,
`inventory_explorer::tests::{bellows_and_carts_appear_in_inventory_explorer,
workshop_and_blueprint_tools_appear_in_inventory_explorer}`.

**Open (owner):** "no PIN item" — there is no `Pin`; the pin action is the **P**
key, not an item (kept as-is). Texture-pack authoring is a separate spec:
`2026-06-18-texture-pack-authoring.md` (needs an architecture decision).

---

## TL;DR

In the Workshop, you **place a normal block**, **hold the Bellows aimed at it**, and it **blows up
smoothly to ×4** (4 blocks tall — the floor-reach ceiling, so you never need to fly). While you aim,
a **wireframe cage** shows exactly where it will grow — anchored at the block's **near-bottom
corner**, growing **up and away from you**, flipping live as you walk round; **green** = it fits,
**red** = something's in the way (and it refuses, with a message). The blow-up is **fixed ×4**, done
by **holding** (like charging a block-break) through **3 smooth swells** (×2 → ×3 → ×4); **let go
early and it collapses back to ×1** — only ×1 and ×4 are stable. At full size it **locks** so you
can walk around it.

Then you **edit it in-world with a dye in hand**: **left-click paints** the ¼-block cell under your
crosshair; **right-click places** a new **coloured microblock**; a **pickaxe carves** one off; an
**eyedropper** key grabs a colour off a cell. Resolution is the engine-native **16³** — the
microblock grid *is* the texel grid (one cell ↔ one texel), so painting and sculpting are **one
unit, one aim**. Because a block has only **3 texture surfaces** (top, bottom, and one side shared
across all four), a flat reskin never makes you paint a side four times; for 3D sculpts a
**symmetry toggle** (all-sides / left-right / off) keeps identical sides one action. **Pin** saves
the design into your **per-block wardrobe** — you keep several designs per block and choose which one
is worn on every instance (or revert to stock) — then deflates ×4 → ×1 in one smooth motion. **No
sound yet** (the pump rhythm leaves obvious slots for it later).

---

## Context pointers (build-state, audited 2026-06-08)

Line numbers are anchors from a 2026-06-08 code read — re-verify on build.

**Already BUILT (reused):**
- **The override registry + reskin commit.** `override_registry.rs` — `AuthoredFaces` (6× 16×16
  RGBA), `OverrideRegistry`, append-as-layers + the mesher/entity seams; `workshop::commit_project_override`;
  `apply_adopted_override_bytes` enforces the **texture-array layer cap**. (Spec 40 Phase A/C.)
- **The micro-model bake.** `micro_model::from_plan` (16³, single-block exact-build) +
  `MicroModelRegistry` (`block_id → BakedMicroModel`) — shipped (#18,
  `2026-06-03-build-big-micro-models.md`). Spec 40 Phase F captures a build via
  `workshop::capture_box_as_plan` → `from_plan` → shape override. **Our sculpt writes the same
  shape-override hook** (plus per-voxel colour — the new bit).
- **The Workshop world + project model.** `workshop.rs` — `WorkshopProject`, the
  `workshop_projects` side-table (rides the world save, `#[serde(default)]`), `inflation_scale`
  (`1.0 + inflation*0.5`, `MAX_INFLATION=6` ⇒ ×4.0 ceiling — **our fixed ×4 is the same ceiling**),
  `nearest_parked`. The in-world mannequin render: `entity_model::build_workshop_inworld_vertices`
  (`entity_model.rs:953`) already draws each project scaled by `inflation_scale` — **the animated
  blow-up is interpolating this scale per frame.**
- **The Bellows held tool.** `MaterialId::Bellows`, provisioned in the Workshop
  (`chunk_stream.rs:416`); current handler `game_loop.rs:4710-4752` (right-click pumps the *nearest
  parked* project). **This handler is what we rewrite** to aim-charge-fixed-×4.
- **Reach + eye height.** `REACH_DISTANCE = 5.0` (`main.rs:227`, `server.rs:23`) for block
  interaction; `ATTACK_REACH = 3.0` (`combat.rs:11`); `PLAYER_EYE_HEIGHT = 1.62` (`physics.rs:40`,
  `eye_pos()` `:64`). **This is why ×4 (4 blocks tall) is the floor-reach ceiling** — a 4³ outer
  shell is reachable from the floor by walking round it; taller needs flying (rejected — floor-only).
- **The raycast gives an exact hit point.** `raycast::RayHit { block_pos, face_normal, distance,
  block }` (`raycast.rs`). `hit_point = eye + dir * distance` + `face_normal` → the **face UV** →
  which 16×16 texel / 16³ microblock cell. **This is the load-bearing new input math** (sub-block
  picking) — and it works at *any* blow-up size, which is why ×4 is a comfort knob, not a
  requirement.
- **Coloured wireframe overlay (the cage).** `renderer.rs:1327` `set_ghost_wireframe` →
  `build_wireframe_cube_colored(pos, color)` (Spec 24 Phase 8 ghost-placement wireframe, separate
  buffer from the block-highlight `set_block_highlight`/`build_wireframe_cube` `:1353`). **The
  green/red cage is this path, drawn as a 4×4×4 box at a computed origin.**
- **Hold-to-charge feel.** `docs/foundations/2026-05-24-block-break-crack-overlay.md` — the
  hold-builds-progress, release-resets pattern the blow-up mirrors.
- **Dyes + palette.** `docs/foundations/2026-05-27-flowers-dyes-colour-mixing.md`; the 16
  `WALLPAPER_*` coloured blocks (`block.rs:268-286`) — the "paint-with-blocks" colour source.
- **The 3-slot block texture model.** `block.rs:1131-1134` — every `BlockDef` stores **exactly
  `tex_top`, `tex_bottom`, `tex_side`**; the four sides **always share `tex_side`** (Stone: all 3
  equal; Grass: green top / dirt bottom / grassy side ×4; Log: end top+bottom / bark side). There is
  **no per-side face** in the engine.

**NEW work (this doc):**
- **Aim-to-blow-up** (raycast the looked-at block, bind a working copy to its id, compute the
  corner-anchored away-from-player cage, room-check it).
- **The green/red cage preview** recomputed per frame as the player moves.
- **Hold-to-charge fixed-×4 inflation** with 3 smooth swells, collapse-on-release, lock-on-full.
- **Sub-block crosshair picking** (RayHit → face UV → 16³ cell) — the new input primitive.
- **In-world dye editing** (paint texel / place coloured microblock / carve) on a working-copy
  16³ colour+occupancy buffer.
- **Edit symmetry mirroring** (all-sides / left-right / off).
- **Pin bake** of the working copy into `AuthoredFaces` (pure paint) and/or a **coloured
  micro-model** shape override.

---

## The problem (precisely)

Spec 40 shipped the Workshop, but its gesture has three rough edges the playtest exposed (the
trigger for this whole design was *"the bellows don't work"* — they require `/ws place` first, which
isn't discoverable):

1. **You can't blow up a block you placed.** The Bellows acts on the *nearest `/ws place` project*,
   not the block you're aiming at. The natural instinct — *put a block down, point the bellows at
   it, watch it grow* — does nothing, and the error (`no project — /ws place <asset> first`) reads
   as "broken."
2. **Painting is on an abstract 2D grid**, divorced from the thing in the world. Kids want to paint
   the block they're looking at, square by square, in 3D.
3. **Reskin (paint) and reshape (build-at-feet) are two separate modes.** They're really one act:
   *change how this block looks* — recolour its faces and/or add/carve bits.

This doc fixes all three with **one in-world gesture**, reusing every Spec 40 backend. It stays
**purely visual → multiplayer-safe** (the Spec 40 invariant): block ids, saves, behaviour are
untouched; only appearance overrides change.

---

## Design (the resolved decisions)

### 1. The loop

1. **Place** a block normally in the Workshop (from your hotbar).
2. **Aim the Bellows** at it → the **cage preview** appears (next §).
3. **Hold** right-click → it blows up to **×4** (next §).
4. **Edit** with a **dye in hand** (§4).
5. **Pin** → bakes onto every block of that type, deflates ×4 → ×1 in one smooth motion.

### 2. The cage + the blow-up direction

While the Bellows is held-aimed at a placeable block, draw a **4×4×4 wireframe cage** showing where
it will grow:

- The aimed block is the **near-bottom corner** of the cage — **not** the centre. (A 4³ cube has
  **no centre cell** — even dimension — so corner-anchoring is the natural choice *and* it
  guarantees the blow-up never expands back into the player's face.)
- The cage grows **up** (4 tall — floor-only) and **horizontally away from the player**: the 4×4
  footprint occupies the quadrant on the **far side** of the block from the player's position.
  (Perfect alignment edge-case: break the tie by look direction. Detail for build.)
- It **flips live** as the player walks around the block, so they choose the grow direction before
  committing.
- **Colour = the room check:** **green** if the whole 4×4×4 is clear, **red** if any cell is
  occupied. Red → the blow-up **refuses** and the player gets a message
  (*"Not enough room — clear a 4×4 space to blow this up."*).

Because the grow is smooth with a hard stop (§3), the balloon **always stays inside the pre-checked
cage** — so we room-check **once, up front** (the full ×4), and never need a per-stage collision
check. Keep the green cage **drawn while pumping** so the player watches the balloon *fill toward the
outline*.

### 3. Fixed ×4, hold-to-charge, collapse-or-lock

- **Fixed ×4.** One working size (4 blocks tall). Not a variable dial. ×4 = the floor-reach ceiling
  (`REACH 5.0` from a `1.62` eye; a 4³ shell is reachable by walking around it). **No flying** —
  reachability from the floor is a hard rule.
- **Hold-to-work, like breaking a block.** Hold right-click; a charge builds. The balloon **swells
  at each third** — ×2, ×3, ×4 — as the charge climbs (three discrete *visual* steps, one
  *continuous* hold; "fill… settle… fill… settle… fill", never a constant flow).
- **Smooth, no overshoot.** Each swell eases and **hard-stops** — no bounce/boing. (A bounce could
  poke a frame past the target into a neighbour; a hard stop can't. The animation lives entirely
  inside the green cage.)
- **Collapse if released early.** Let go before ×4 and it slides **all the way back to ×1** — it
  never rests at a partial size. **Only ×1 and ×4 are stable states.**
- **Lock at full.** Reach ×4 and it **locks**: release the button and walk around it freely to edit.
- Implementation note: this maps onto the existing `inflation` field (×2/×3/×4 ↔ `inflation` 2/4/6,
  the current ×4.0 ceiling) — but the *interaction* changes from 6 discrete pumps to a hold-charge
  with 3 swells. `WorkshopProject::pump/deflate` are replaced by a charge→target model.

### 4. Editing in-world with dyes (the unified reskin + reshape)

The blown-up working copy is a **16³ grid of ¼-block cells** (engine-native resolution —
`texture_gen SIZE=16`; one cell's face ↔ one texel). With a **dye in hand**, the crosshair
(sub-block pick, §Context — `RayHit.distance`) selects the exact cell:

| Input | Action |
|-------|--------|
| **Left-click** | **Paint** the cell-face under the crosshair the dye's colour (texel reskin) |
| **Right-click** | **Place** a new **coloured microblock** against the aimed face (sculpt / reshape, in colour) |
| **Pickaxe (or empty hand) left-click** | **Carve** — remove a microblock |
| **Eyedropper key** | Grab the colour off the cell under the crosshair |

So the same ¼-block unit is *painted* or *placed*, at one aim — **aiming is identical whether it's a
pixel or a microblock**. (The ¼-block aim comfort is a genuine **playtest unknown** — we find out by
playing.) This **collapses Spec 40's Mode A + Mode B** into one act on one object.

### 5. Faces: 3 surfaces native + a symmetry toggle for sculpts

Every block has **3 texture slots** — `top`, `bottom`, and **one `side` shared across all four
sides** (`block.rs:1131-1134`). Consequences:

- **A flat reskin never repeats work.** Painting "the side" once paints all four — at most 3 paints
  (top / bottom / side), often 1 for a uniform block. This is **built into the block model**, no
  feature needed.
- **A 3D sculpt paints real faces**, so it *could* do all four sides differently → to keep "do it
  once," the blown-up editor carries a **symmetry toggle**:
  - **All-sides (4-fold)** — strokes mirror to the other 3 sides → uniform sides in one pass.
    **Default on** (matches the native block model).
  - **Left–right mirror** — sculpt half, the other half mirrors → symmetric shapes in half the work.
  - **Off** — when each side should differ.

### 6. What Pin writes (reskin vs sculpt output)

- **Pure paint (no shape change)** → an `AuthoredFaces` texture override via the existing
  `override_registry` path (Spec 40 Phase A/C). Sides equal by the symmetry default collapse to a
  single side texture.
- **Shape changed (microblocks placed/carved)** → a **coloured micro-model** shape override: the
  `from_plan` / `MicroModelRegistry` hook (#18 / Spec 40 Phase F) **extended to carry per-voxel
  colour** (today's bake is shape-only). Live apply = the existing texture-rebuild + dirty-chunk
  re-mesh.

Either way, Pin **saves the design into the block's wardrobe** (§7) and wears it on **every**
instance of that block type, then deflates in one smooth motion.

### 7. The wardrobe — keep many designs per block, choose what's worn

Today the registry holds **one** design per block (`block_tex: Vec<(BlockId, AuthoredFaces)>`,
retain-then-push — a new pin *overwrites* the old). "Multiple designs of the same block" turns this
into a **wardrobe**:

- **Pin = save into the block's wardrobe** — it never destroys an earlier design.
- A **per-block active selection** decides which design is *worn*; **"original"** is always
  available (deactivate → stock art).
- **Applying is automatic within your choice:** the active design shows on **every** instance in your
  world; switching or reverting is instant and deliberate.
- **Managed from a Workshop gallery** — your designs per block, where you rename / delete / set-active.

Engine shape: `block_tex` / `mob_tex` become a small **library** per key (`designs: Vec<NamedDesign>`
+ `active: Option<DesignId>`); the render seam reads the active design (stock when `active = None`).
Versioned + `#[serde(default)]` so an existing one-design set upgrades to a one-entry wardrobe.

### 8. Security & ownership (reused from Spec 40)

- **Yours, attributed** — each design carries your **npub** + a derivation chain (origin + what it's
  based on); attribution renders npub, never hex (`feedback_npub_only_display`).
- **Tamper-proof when shared** — published designs are **schnorr-signed** (the Beacon SDK verifies);
  no forging a design under someone else's name.
- **Safe to adopt** — an adopted blob is version-checked, garbage-rejected, and **capped to the
  texture-array layer limit** (`apply_adopted_override_bytes`), so a broken/oversized design can't
  crash your game. Adopt slots the design **into the wardrobe** — it doesn't silently replace your
  active pick.
- **Saved** — your wardrobe rides your stash/save.

### 9. Workshop-only — the gesture never escapes

The Bellows + blow-up + pin are **Workshop-only by construction**: the Bellows is **not craftable**
(only provisioned inside the Workshop — `chunk_stream.rs:416`), the gesture is **gated on
`is_workshop`** (`game_loop.rs:4715`), and the Workshop is a **separate saved world** so its
inventory never bleeds into a normal world. A Bellows in a normal world (which shouldn't happen) is
**inert**. All three guards are a **rule**, not an accident — keep them.

---

## Phased scope

Each phase is independently `./check.sh`-gated; the **look/feel** is the Axolittle playtest boundary.

### Phase 1 — sub-block picking + the cage preview (no blow-up yet)
*Goal: aim a Bellows at a block → a corner-anchored, away-from-you **green/red 4×4×4 cage**; the
crosshair resolves the exact texel/microblock cell. Solo-verifiable geometry + render.*

- New: ray-hit → face-UV → `(face, u, v)` 16-grid cell (pure fn, unit-tested).
- New: cage origin = corner-anchor + away-from-player quadrant (pure fn, unit-tested); 4×4×4
  occupancy room-check → clear/blocked.
- Render the cage via `set_ghost_wireframe` / `build_wireframe_cube_colored` (green/red), recomputed
  per frame.

**Acceptance:** unit tests for the UV→cell map (centre + each corner of each face) and the
cage-origin/room-check (clear vs each blocked cell); headless screenshot of green and red cages
(`--shot-workshop` tooling, `reference_lobby_screenshot_tool`); `check.sh` green.

### Phase 2 — hold-to-charge fixed-×4 blow-up
*Goal: replace the nearest-parked stepwise Bellows with aim-charge-fixed-×4. The rig, no editing.*

- Rewrite `game_loop.rs:4710-4752`: hold-charge on the aimed block; 3 smooth swells (animated
  `inflation_scale`); collapse-to-×1 on release; lock at ×4; red cage / no room → refuse + toast.
- Working copy bound to the aimed **block id** (not nearest parked).

**Acceptance:** state-machine unit tests (idle → charging(t) → ×2/×3/×4 → locked; release<full →
×1; red cage → no state change + message); headless shot mid-swell; `check.sh` green. **Playtest
boundary:** charge duration, swell rhythm, collapse timing → Axolittle.

### Phase 3 — in-world dye editing + symmetry
*Goal: paint texels, place/carve coloured microblocks, on the locked ×4 copy, with a dye in hand.*

- A working-copy 16³ **colour + occupancy buffer**; left-paint / right-place / carve / eyedropper
  via the Phase-1 pick; the symmetry toggle mirrors edits.
- (Persistence of a *parked* working-copy buffer: see Open Questions — may be pin-only for v1.)

**Acceptance:** edit→buffer unit tests (paint sets a cell colour; place adds occupancy+colour; carve
clears; symmetry mirrors to the right cells); `check.sh` green. **Playtest boundary:** the painting
*feel* + ¼-block aim comfort → Axolittle.

### Phase 4 — Pin bake (the payoff)
*Goal: working copy → global override, both paths, live.*

- Pure-paint buffer → `AuthoredFaces` → `override_registry` (reuse `commit_project_override` shape).
- Shape-changed buffer → **coloured micro-model** → `MicroModelRegistry` (extend #18 bake to carry
  per-voxel colour) → live re-texture + re-mesh; one-smooth deflate.

**Acceptance:** a paint-only edit re-textures **every** instance (visual — owner's eyes) and writes
`AuthoredFaces`; a shape edit makes every instance render the coloured micro-model; block id / saves
/ behaviour unchanged; `check.sh` green. **Playtest boundary:** the result look → Axolittle.

*(Sharing rides Spec 40 Phase D/E Beacon. Serialising a **coloured** micro-model into the
`OverrideSet` blob is an extension of the existing wire format — ~~flagged below, not built here~~ **resolved in Phase 5** (v2 blob migration; see §Phase 5 below and `docs/spec/05-gameplay-systems.md §10b`).)*

### Phase 5 — the wardrobe (many designs per block)
*Goal: keep multiple designs per block, pick which is worn, revert to stock — the registry→library
change + the Workshop gallery.*

- `override_registry`: `block_tex` / `mob_tex` → a per-key **library** (`designs` + `active`); the
  render seam reads the active design (stock when `None`). Versioned + `#[serde(default)]` so a
  one-design set upgrades to a one-entry wardrobe. Adopt slots a design **into** the library.
- A **Workshop gallery** UI: per-block designs — set-active / rename / delete / "use original".

**Acceptance:** library round-trips serde (an old one-design set upgrades clean); set-active swaps
the worn design live on every instance; "use original" reverts to stock; an over-cap library is
bounded; `check.sh` green. **Playtest boundary:** the gallery UX → Axolittle.

---

## Out of scope / deferred (explicit)

- **Sound.** The pump/breath/paint SFX are deferred (owner: "we're not doing sound yet"). The design
  leaves per-swell + per-stroke hooks.
- **Blueprint authoring in the Workshop (the room's *second* tool).** Capturing **structures** from
  normal blocks — build → Drafting Stamp → *lock in*, tagged **creative** provenance — reuses the
  existing blueprint/plan system and is a **sibling spec**
  (`2026-06-08-workshop-blueprint-authoring.md`). A different domain (structures, not appearance);
  cross-referenced, not built here.
- **Variable blow-up / flying for bigger sculpts.** Fixed ×4, floor-only, by decision. A
  fly-to-reach larger mode is a possible later knob.
- **Removing the 2D paint-grid + `/ws place`.** Kept as a power-user / test fallback, not deleted.
- ~~**Coloured-micro-model sharing wire format.**~~ **Resolved in Phase 5.** Shape designs now ride the v2 `OverrideSet` blob (`to_blob_bytes` v2; `from_blob_bytes` v1→v2 migration via `migrate_v1`). See `docs/spec/05-gameplay-systems.md §10b` Phase 5 for full wire-format details.
- **Mob blow-up-and-sculpt.** This gesture is **blocks-only v1** (mob reshape = #19, parked; mob
  reskin still rides Spec 40's painter).
- **Behaviour/hitbox changes.** Visual-only — server stays authoritative, multiplayer-safe.

---

## Open questions (resolve during build, not blocking)

1. **¼-block aim comfort.** The headline playtest unknown — is a 0.25-block cell comfortably
   crosshair-pickable at ×4? If not, the fallback is the 2D grid (kept) or an 8³ "coarse" toggle.
2. **Parked working-copy persistence.** A parked sculpt's 16³ colour+occupancy buffer is larger than
   Spec 40's paint-only WIP. v1: **persist on pin only** (in-progress sculpts don't survive leaving)
   vs extend `workshop_projects` to store the buffer. Recommend pin-only for v1; revisit if "potter
   across days" on sculpts is wanted.
3. **Cancel a locked ×4 without pinning.** A deflate/discard gesture (sneak-aim the Bellows? a
   discard key?) — define with Axolittle.
4. **Collapse-on-release timing.** How fast the balloon sags back to ×1 — a feel knob.
5. **Symmetry default + per-block-type sensible default** (uniform blocks default all-sides; already
   the plan, confirm in playtest).
6. **Does in-world editing fully replace the 2D grid?** v1: coexist (grid = precision/fallback).
   Promote one to primary after playtest.
7. **Wardrobe size + adoption.** A per-block design cap (don't let the library grow unbounded);
   adopting another player's design slots it in **inactive** (you opt to wear it) vs auto-wear.
   Recommend a small cap + adopt-inactive.
8. **Active-selection scope.** Per-world vs one global wardrobe across all your worlds. Recommend
   **global per player** (your art is yours everywhere); confirm with Axolittle.

---

## File-touch map (when built — NOT now)

| File | Phase | What |
|------|------|------|
| `game/engine/src/raycast.rs` (+ new pick helper) | 1 | ray-hit → face-UV → 16-grid `(face,u,v)` cell (pure fn + tests) |
| `game/engine/src/workshop.rs` | 1–3 | cage-origin + room-check pure fns; working-copy 16³ colour+occupancy buffer; charge→target model (replaces `pump/deflate`) |
| `game/engine/src/renderer.rs` (reuse ghost-wireframe) | 1 | draw the 4×4×4 green/red cage via `build_wireframe_cube_colored` |
| `game/engine/src/game_loop.rs` (`:4710-4752` rewrite) | 2–3 | aim-charge fixed-×4 Bellows; lock/collapse; dye edit (paint/place/carve/eyedropper); symmetry |
| `game/engine/src/entity_model.rs` (`:953` reuse) | 2 | animate the blow-up by interpolating `inflation_scale` per frame |
| `game/engine/src/micro_model*.rs` (extend #18) | 4 | bake a **coloured** micro-model (per-voxel colour) → `MicroModelRegistry` |
| `game/engine/src/override_registry.rs` | 4–5 | pure-paint → `AuthoredFaces`; honour the texture-array layer cap; **`block_tex`/`mob_tex` → per-key wardrobe (`designs` + `active`), versioned `#[serde(default)]`** |
| Workshop wardrobe gallery (new UI + menu hook) | 5 | per-block design library: set-active / rename / delete / "use original" |
| `game/engine/src/commands/builtins/workshop.rs` | 2–4 | `/ws` help/fallback reflects the new gesture (`place`/`edit` kept as fallback) |
| `docs/foundations/2026-06-04-the-workshop-community-redesign.md` | — | cross-link this as the v2 gesture |
| `docs/spec/05-gameplay-systems.md` | 2–4 | record the in-world blow-up-and-sculpt authoring loop |

*(All new state additive + `#[serde(default)]` where it touches saved structs — the `PlanData`
forward-compat pattern. Reuses Spec 40's registry + #18's bake + the ghost-wireframe + the
block-break charge; re-derives none of them.)*

---

## Memory-rule check

- **`project_shared_infra_strategy` (cross-game lift):** the aim-to-blow-up gesture, sub-block
  crosshair picking, the cage preview, and in-world voxel+colour editing are **engine-generic** — any
  Decented voxel game gets an in-world asset editor from the same primitives. The *assets* are
  AxeNStax's; the *gesture* is shared infra. ✅
- **`feedback_autonomy_to_playtest_boundary`:** Phase 1 (picking + cage) and the pure-fn state
  machines are solo-buildable + `check.sh`-gated; Phases 2–4 end at a **feel** confirmation
  (Axolittle) — build to the boundary, stop cleanly. ✅
- **`feedback_merge_to_main_preauthorised`:** healthy-gate merges to main pre-authorised, scoped to
  decented/axenstax. Design-only now — nothing to merge. ✅
- **`reference_proof_of_play_is_proof_of_work`:** untouched — this is visual authoring, no reward
  mechanic. ✅
- **`feedback_uk_english_naming`:** UK English throughout (colour, not color). ✅
- **`feedback_npub_only_display`:** shared overrides attribute by npub (NIP-19) per Spec 40 — this
  doc adds no new identity surface. ✅
- **`reference_lobby_screenshot_tool`:** Phase 1/2 acceptance uses the headless `--shot-workshop`
  path for the cage + mid-swell shots. ✅
- **`feedback_owner_inbox_curation` (concurrent agent shares the repo):** this doc + the blueprint
  sibling are **new files** only; the `docs/foundations/README.md` queue entry + any code are left
  for an explicit build, so no shared file is touched. ✅
- **No build yet:** design/spec only, per the project rule "do NOT build anything unless explicitly
  asked." The owner says when to action it. ✅
