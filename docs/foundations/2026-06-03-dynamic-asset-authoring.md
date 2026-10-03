# Dynamic / Animated Asset Authoring — build the creature, inherit the motion (standard-skeleton rigs)

**Status:** **Phase A + B + authoring UX + in-world render BUILT 2026-06-19** (goal `2026-06-19-solo-buildout-wave-2`). The full runnable loop landed: `skeleton.rs` (Phase A — data model + 5 standard skeletons, biped == `PLAYER_MODEL`) + `anim_set.rs` (Phase B — `eval_anim_set`, biped walk == avatar gait) + **Rig Studio** (`rig_studio_ui.rs`, **Y** key: pick a skeleton, assign your held block to each named part, Spawn → a standing rig appears in front of you; modal gating like the other panels) + **render** (`entity_model::build_rigged_vertices` poses each part as a cuboid of its assigned block via `build_part_vertices`, playing the inherited **walk** gait). Rigs are **world data** (`World.rigs`), **persisted** append-only via `WorldSave.rigs` (round-trip tested). `check.sh` green.

**Shell attach + Phase C scale channel + clip picker DELIVERED 2026-09-06** (`check.sh` ALL GREEN, 4176 tests). Three upgrades landed on top of Phase A/B:
1. **Micro-model shell attach** — a rigged part whose assigned block has a **registered #18 micro-model** now renders that block's **baked shell** instead of a cuboid: the shell is fitted **uniformly** into the skeleton part's `size` box (aspect preserved, bounding box centred on the part origin) and posed through the same pivot transform. The bake is **not** repeated per frame — `MicroModelRegistry::register` bakes once per `BlockId` and the render path only reads the cached `ChunkMesh`, expanding its indices into the entity triangle list. Blocks *without* a micro-model keep the cuboid path, so every rig authored before this looks unchanged.
2. **Phase C rung 2 — squash & stretch.** `PartPose` gains `scale: [f32; 3]` (default `[1,1,1]`); the shared transform multiplies each corner's offset-**from-pivot** by `scale` **before** the X rotation, and **skips the multiply entirely** when the scale is identity — so all existing avatar/mob/cart output is byte-identical (pinned by a regression test that recomputes the pre-Phase-C corner maths for three species). New **`AnimClip::Bounce`**: a gentle whole-rig breathe (squash Y / widen X+Z on a sine), the first clip to use the scale channel — and the only one; every rotation clip is asserted to leave `scale` at identity.
3. **Clip picker in the Rig Studio.** Walk is no longer hardcoded: a **Motion:** row offers **Walk / Idle / Bounce**, and the pick rides on the placed `RigDisplay`. Persistence note: `RigDisplay` is an element of `WorldSave.rigs`, a `Vec` in the *middle* of a positional bincode blob, so appending a field to it would shift every byte after it and corrupt `exhibits`/`composters`/`saved_mobs` in any save that already holds a rig. The clip therefore rides **`WorldSave.rig_clips`**, an index-aligned side table appended at the true end of the blob; a pre-picker save has no side table and every rig loads as `Walk` (tested both ways).

**Still open:** the shared transform now composes pivot → scale → X-rot → yaw, but **parts still pose independently** — Phase C's **segmented bend** (parent-chain composition, the swaying-plant chain) is NOT built, and neither is **Phase D** (animation-frame flipbook). Also still open: the orbit preview, and per-part pivot nudging in the UI. **Feel + the look of an attached shell = Axolittle playtest** (nothing visual here is solo-verifiable). — *Original draft 2026-06-03 below.*

**Branches (when built, not now):** fork off `main` (or off the #18 branch if #18 hasn't merged yet — [[feedback_pr_chains_on_spec_branches]]) → `feat/standard-skeletons` (Phase A) → `feat/skeleton-anim-inherit` (Phase B) → `feat/squash-scale-channel` (Phase C) → `feat/anim-frame-flipbook` (Phase D). Bespoke per-creature keyframe authoring is a **separate later spec**, deliberately deferred.

---

## TL;DR

The owner wants the **same "build it in-game, do-ocracy" pipeline as #18, but for things that move** — mobs, swaying plants, machines — by placing **pivot points** while building to define joints. The reality check that makes this tractable: **pivot-based skeletal animation already exists and ships today.** `entity_model.rs` drives every mob and the player avatar through a hierarchy of cuboid `ModelPart`s, each with a **`pivot`**, and animates them by **rotating parts about their pivot** (the arm swings about a shoulder pivot via `swing_angle`; legs swing about the hip; the head pitches about the neck). The motion machinery (`PartPose`, `build_part_vertices`, `swing_angle`) is real, tested, and cross-platform.

So **#19 is not "invent skeletal animation."** It is two things:

1. **Let the community DEFINE parts + pivots by building** (instead of the parts being hardcoded `*_model()` Rust functions), reusing #18's capture + bake.
2. **Authoring UX so casual authors never hand-keyframe.** Ship a handful of **standard skeletons** (biped, quadruped, bird, fish, swaying-plant) each with a **built-in animation set** (idle / walk / attack / …). The author builds geometry, attaches voxel groups to the skeleton's named parts/pivots, and the model **inherits the standard animation automatically** — exactly how a Minecraft mob re-skin inherits motion. Bespoke per-creature animation is advanced and **deferred**.

**The motion-technique ladder** (escalating cost; each phase adds one rung):
- **(1) Pivot / joint rotation** — rigid parts rotate about joints. **WE ALREADY HAVE THIS** (`entity_model.rs:1394` `build_part_vertices`; `PLAYER_MODEL` shoulder pivot at `entity_model.rs:1874`). = the owner's "pivot points." → **Phase A/B.**
- **(2) Squash & stretch** — per-part non-uniform **scale** keyframes (squash Y, widen X). Cheap. → **Phase C.**
- **(3) Bend** — a **chain of segmented parts**, each pivoting a little (blocky-friendly, reuses rung 1). True vertex skinning (bones+weights) is heavier and less blocky → **deferred.** → **Phase C (segmented) / later (skinning).**
- **(4) 3D "sprite swap"** — a **flipbook of baked model frames** swapped per animation frame (the 3D analogue of swapping 2D sprites; the correct term is **animation frames / keyframe / vertex animation**, NOT "sprite"). Cheap; great for non-rigid changes (chomping mouth, flapping fish). → **Phase D.**

**Performance is a non-issue with the right approach** (same story as #18): each animated part is a **baked, greedy-meshed, instanced** mesh; animation is a **cheap per-part transform/scale** (rungs 1-3) or a **per-frame mesh swap** (rung 4). **No per-micro-voxel cost** — the sub-voxel grid is *authoring resolution*, never draw cost.

---

## Strategic frame (why this is ours, briefly)

#18's strategic case (`docs/backlog/owner-inbox.md` #18 + the cosmetics research in `docs/foundations/2026-06-02-standard-avatar-and-byo-skins.md` §Strategic opportunity) carries straight over: the community's #1 want is **custom 3D models, not flat skins**, and Mojang/Microsoft structurally cannot ship player-authored *animated* creatures — anti-cheat fear (custom models were added to Bedrock then *removed*), revenue protection (the walled $500M Marketplace), and no identity/payment rails. Animated authoring is the deeper version of the same gap: not "re-skin a cow," but "**build and rig your own creature, and it just moves**," shareable over Stash, ownable by your Nostr persona, and (later, with Bitcoin) sellable creator-to-player with no marketplace cut. This is the "do-ocracy" applied to motion. The **curation / "creature show" competition mechanic is explicitly DEFERRED** — same as #18's "flower show" (owner: "good problem to have, not now").

The honest caveats also carry over: richer animated models are a real **performance budget** (mitigated by bake+instance below), and the moment rigs are *sellable* the **UGC IP/moderation** exposure grows — a terms/takedown problem to plan for, not a blocker, and not in scope here.

---

## Context pointers (verified against the tree 2026-06-03)

**Prerequisite (must land first):**
- **#18 static micro-model pipeline** — `docs/foundations/2026-06-03-build-big-micro-models.md` (parallel). Provides: build-big-in-creative → **capture** (reuse `plan::capture`, `plan.rs:544`) → **bake** the captured volume to a sub-voxel micro-model → **greedy-mesh once** → **instance**. #19 rigs the *baked geometry* #18 produces; it does not re-derive capture or bake.

**The animation machinery that already exists (the proof #19 is mostly UX, not new tech):**
- **`ModelPart`** — the per-cuboid rig node: `origin`, `size`, **`pivot`** (the joint), `animated: bool`, `phase: f32` (gait offset), `tex_faces: [u32;6]`, `pitch_tracks_look: bool`. Defined `entity_model.rs:201-217`.
- **`PartPose`** — the per-part animation input: `walk_swing: f32`, `head_pitch: f32`, `arm_override: Option<f32>`. Defined `entity_model.rs:1384-1392`. This is the seam an "animation set" writes into.
- **`build_part_vertices`** (`entity_model.rs:1394-1469`) — the canonical transform composition for one part: build the 8 cuboid corners in local space → **rotate about `part.pivot` on X** (the joint rotation; `x_rot = arm_override.unwrap_or(walk_swing)`, plus `head_pitch` if `pitch_tracks_look`) → **yaw about Y** → **translate to entity world position** → emit 6 faces. **This is rung-1 pivot/joint rotation, shipping today.**
- **`swing_angle(t)`** (`entity_model.rs:1377`, with `SWING_PEAK: f32 = 1.2`, `SWING_TICKS: u32 = 6` at 1374-1375) — the mining/placing arm-swing curve: `SWING_PEAK * (1.0 - t) * sin(π·t)`, 0 at both ends, single peak. A worked example of a **named, parameterised animation clip**.
- **`PLAYER_MODEL`** (`entity_model.rs:1838-1911`, accessor `player_model()` at 1915) — the **biped skeleton, already**: 6 parts [head, body, left arm, right arm, left leg, right leg]. The **head pivot sits at the neck** (`pivot: (0.0, 1.325, 0.0)`, comment at 1841-1843: "look-pitch reads as a nod about the neck, not a ball spinning in place"); the **arms pivot at the shoulder** (`pivot: (±0.375, 1.4, 0.0)`, 1874/1884); legs pivot at the hip (1894/1904). Arms/legs carry alternating **`phase`** (0.0 / 0.5) for a diagonal gait.
- **`build_player_avatar_vertices`** (`entity_model.rs:866-955`) — the **animation-set driver** for the avatar: reads `PlayerState.anim_state` (0 idle / 1 walk / 2 jump — `protocol.rs:185`) + `flags & CROUCHING` and synthesises per-part `walk_swing`: walk = `sin(anim_time*2.5 + phase*TAU)*0.4` (1898), jump = a static airborne pose (1904-908), crouch = a whole-avatar dip of 0.25 (880). The right arm's mine/place swing (`arm_override = swing_angle(progress)`) **wins over** the walk cycle (912). **This is "a skeleton with an inherited animation set," already — just hardcoded for one skeleton.**
- **Mob animation driver** — `build_entity_model_vertices` (`entity_model.rs:799-846`): for every mob it looks up `mob_model(kind)`, computes a walk swing `sin(anim_time*2.5 + phase*TAU)*0.4` for `animated` parts when `speed > 0.001` (820-825), and calls `build_part_vertices`. **One animation set (walk) already drives ~17 different skeletons by data.**
- **Skeleton library, hardcoded today** — `MODEL_CACHE` LazyLock (`entity_model.rs:220`) maps `MobType` → `Vec<ModelPart>`; `mob_model(kind)` accessor at 259. Per-species builder fns (`cow_model` 263, `pig_model` 328, `chicken_model` 462, `bear_model` 530, `horse_model` 688, `squid_model` 786, …) compose parts from helpers **`box_part`** (static box, `entity_model.rs:670`), **`leg_part`** (animated leg pivoting at the hip, 676-679 — *"pivots at the hip… so the walk cycle swings it"*), and **`coat`** (shared texture set, 682). **These hand-written builders are exactly what #19 lets the community replace with built+attached geometry.**
- **The two emit paths the rig drives:** `push_textured_quad` (block-array UV, `entity_model.rs:1549`) for mobs; `push_skin_quad` / `build_skin_part_local` (64×64 atlas UV, 1580 / 1490) for the player skin. A micro-model rig will need its own emit (see Phase A) but reuses the same corner→face winding.

**The performance pattern #19 inherits wholesale (#18 established it; plants prove it):**
- **`PlantInstance`** (`mesh.rs:66-91`) — a **32-byte per-instance record** (`pos`, `size`, `tex_layer`, `light`); the doc-comment is explicit: *"each plant costs this 32-byte record instead of 8 baked vertices."*
- **`plant_unit_cross`** (`mesh.rs:121`) — the **shared** unit geometry, uploaded once.
- **Instanced draw** — `render` loop at `renderer.rs:2485-2500`: one `set_vertex_buffer(1, plants.buffer)` + `draw_indexed(…, 0..plants.count)` per chunk, frustum-culled; upload via `upload_plant_instances` (`renderer.rs:1011`). **This is the template for "one baked part mesh, N instances, one draw per part-type."**
- `mesh.rs:871` test `plants_are_instanced_not_baked` — the invariant #19 must preserve for animated parts too.

**Capture types #18/#19 reuse:**
- `PlanData` / `CapturedCell` (`plan.rs:148-205`) — `CapturedCell { rx, ry, rz: u8, block_id }`; `PlanData { version, name, author_npub, license, width/depth/height, cells, … }`. `capture` (`plan.rs:544`, 3D flood-fill) and `capture_art` (633, 2D slice) are the existing capture entry points.

**Baseline gate:** `./check.sh` currently green — clippy clean, native + WASM build, **1910 tests**, bundle **2.76 MiB** (`docs/backlog/owner-inbox.md:50`). Every phase below holds this line.

Spec cross-refs: gameplay/entities `docs/spec/05-gameplay-systems.md` (mobs/models) and rendering `docs/spec/03-rendering.md` (entity pipeline, instancing) get the new skeleton/rig sections when this is built. There is **no** rigging/animation-authoring section in any spec today — this doc fills that gap, and the eventual build updates Spec 03 + 05 (per [[spec maintenance]]).

---

## Problem

Three gaps, in order:

1. **Animated assets can't be authored at all.** #18 lets the community build + bake *static* props (flowers, décor) and explicitly **scopes out anything that moves** (owner agreed: "Animated things (mobs/animals) need rigging/animation — a different pipeline, out of scope"). So the moment a community author wants a creature, a swaying plant, or a machine, they hit a wall: the only way to add a moving model today is to **hand-write a Rust `*_model()` builder + wire it into `MODEL_CACHE` + write an animation driver** (`entity_model.rs`). That's a code change, not a do-ocracy.

2. **The skeleton + animation are hardcoded, not data the author can supply.** The machinery is *there* (`ModelPart` hierarchy, `PartPose`, `build_part_vertices`, `swing_angle`) but every skeleton is a Rust function and every animation is inline arithmetic in `build_entity_model_vertices` / `build_player_avatar_vertices`. An author building a four-legged creature has no way to say "these voxels are the front-left leg; it pivots here; make it walk."

3. **Casual authors must not be asked to hand-keyframe.** The owner's key simplifier: most authors should get motion **for free**. A re-skin of a cow should walk like a cow without the author touching a keyframe. Today there is no "attach my geometry to a standard skeleton and inherit its animation" path — that's the whole UX this doc designs.

**Non-problems (explicitly):** we are **not** inventing skeletal animation (rung 1 ships), **not** building a Blender, **not** doing true vertex skinning now, and **not** designing the curation/competition mechanic.

---

## Design — the standard-skeleton model

### The core idea: skeleton (motion) ⟂ geometry (look)

Separate the two things that today are fused in a `*_model()` function:

- A **`Skeleton`** = an ordered list of **named parts**, each with a **pivot (joint)**, a parent (for hierarchy), a gait **phase**, and animation flags (does it walk-swing? does it pitch with look?). This is `ModelPart` with a name + parent + the existing fields. We ship **a few standard skeletons** as data.
- A **`RiggedModel`** = "for each skeleton part, here is a **baked micro-model** (from #18) attached at that part's pivot." The author builds the geometry big, marks which voxel-group is which part, and #18 bakes each group to an instanced shell mesh.

A model **inherits** its skeleton's **animation set** automatically. The author supplies geometry; the motion is the skeleton's. This is precisely how `build_entity_model_vertices` already makes one walk-cycle drive ~17 mob skeletons — generalised so the *skeleton itself* is authored data and the *attached geometry* is a baked micro-model rather than a textured cuboid.

### Standard skeletons to ship (Phase A)

Each ships with a built-in **animation set** (Phase B). Part names are the attach points the author targets.

| Skeleton | Parts (named) | Built-in animation set | Real-code analogue |
|---|---|---|---|
| **Biped** | head, body, arm_l, arm_r, leg_l, leg_r | idle, walk (diagonal gait), jump, attack (arm swing), crouch | `PLAYER_MODEL` (`entity_model.rs:1838`) + `build_player_avatar_vertices` — *literally this skeleton already* |
| **Quadruped** | head, body, leg_fl, leg_fr, leg_bl, leg_br, (tail) | idle, walk (4-leg gait), eat (head dip) | `cow_model`/`bear_model`/`hyena_model` + the walk driver at `entity_model.rs:820` |
| **Bird** | head, body, wing_l, wing_r, leg_l, leg_r, (beak) | idle, walk, flap (wing rotation), peck | `chicken_model` (`entity_model.rs:462`) + bee wings (`bee_model:783`) |
| **Fish/aquatic** | body, tail, fin_l, fin_r | idle (drift), swim (tail + fin oscillation) | `squid_model` (`entity_model.rs:786`) — 4 hanging tentacle parts already |
| **Swaying-plant** | base (static), segment_1..n (each pivots a little) | sway (wind-phase sine up the chain) | rung-3 "segmented bend" applied to a vertical part-chain |

These are **data**, defined once. The biped one is a transcription of `PLAYER_MODEL`; the quadruped/bird/fish are transcriptions of the existing mob builders' part layouts. **No new motion maths for Phase A/B** — just structuring what exists as authored skeleton data + a generic animation-set evaluator that writes `PartPose`s.

### The motion-technique ladder (concrete, mapped to phases)

**Rung 1 — Pivot / joint rotation. SHIPS TODAY.** This is the owner's "pivot points." `build_part_vertices` (`entity_model.rs:1424-1435`) rotates a part's 8 corners about `part.pivot` on X by `x_rot`; the avatar/mob drivers feed `x_rot` from a walk sine or `swing_angle`. **Phase A** lets the author *place* the pivot (by building); **Phase B** generalises the driver so any skeleton's animation set writes the `PartPose.walk_swing` per part. Nothing new in the transform — we are exposing the existing rotation to authored data.

**Rung 2 — Squash & stretch.** A new per-part **non-uniform scale** channel: `PartPose` gains `scale: Vec3` (default `(1,1,1)`); `build_part_vertices` multiplies the corner offsets-from-pivot by `scale` *before* the rotation. An animation set can keyframe `scale = (1.1, 0.85, 1.1)` for a bounce/breathe/landing squash. Cheap (3 muls/corner), blocky-friendly. → **Phase C.**

**Rung 3 — Bend.** The owner's "not just pivoting." Two options:
- **(recommended, blocky) Segmented chain:** model the bending part as **several stacked parts**, each parented to the one below and pivoting a small angle; summing small rotations up the chain reads as a smooth bend. This is **pure rung-1** applied to a parent-chain — exactly the swaying-plant skeleton. Reuses `build_part_vertices` per segment; the only new thing is **parent composition** (rotate in the parent's already-rotated frame), which Phase B's hierarchy support provides.
- **(deferred) True vertex skinning** (bones + per-vertex weights, smooth deformation): heavier (per-vertex bone blend in the shader), and *less* blocky — against the art direction. **Deferred** to a later spec; flagged so Phase C's segmented bend doesn't foreclose it.

**Rung 4 — 3D "sprite swap" (animation frames / flipbook).** The 3D analogue of swapping a 2D sprite. **Terminology, since the owner asked:** a *sprite* is a 2D image (our flowers are billboard quads, loosely "sprites"); the 3D equivalent of swapping sprites is **swapping a cached mesh per animation frame** — call it **animation frames** / **keyframe** / **vertex animation**, *not* "sprite." Mechanically: the author bakes **N micro-model frames** (e.g. mouth-open / mouth-closed; fish-tail-left / -right); at runtime the renderer **selects which baked frame mesh to draw** for the current frame index. No interpolation, no skinning — just an index into a small array of pre-baked instanced meshes. Great for non-rigid changes that rotation can't express. Cheap: it's a buffer selection, not extra per-frame geometry work. → **Phase D.**

### The do-ocracy unlock: inherit, don't keyframe

The owner's pivotal simplifier (and the reason this is "nearly as easy as static authoring"):

> Casual authors **do not hand-keyframe.** They pick a standard skeleton, build geometry, attach voxel-groups to the skeleton's named parts, and **inherit the standard animation set.** A re-skinned biped walks like the biped; a re-skinned quadruped walks like the quadruped — automatically, the way a Minecraft mob re-skin inherits motion.

Squash-scale (rung 2) and animation-frames (rung 4) are **optional extra channels** an author *can* opt into per part, but the default path needs none of them. **Bespoke per-creature keyframed animation** (an author authoring entirely new motion curves) is **advanced and DEFERRED** to its own later spec — it's the long tail, not the unlock.

### Authoring UX (where the genuinely new work is)

The hard part isn't the maths — it's the in-game UX for "build geometry → mark parts → place pivots." Sketch (concretised in Phase A):
- **Pick a skeleton** (biped/quad/bird/fish/plant) from a creative menu. A faint **guide overlay** shows the skeleton's part bounds + pivot dots at suggested positions (reuse the depth-biased decal/wireframe overlay primitives — the crack-overlay pipeline pattern and the block-highlight wireframe).
- **Build big** in creative (as #18), then **assign voxel-groups to parts** — the author selects a region (reuse the existing capture **bounded-volume selection**, `plan::capture` at `plan.rs:544`) and tags it `leg_fl`, `head`, etc.
- **Place / nudge pivots** — for each part, accept the skeleton's default pivot or move it (a pivot is one point on a micro-voxel; the owner's "a pivot on one micro-voxel connecting to another"). Default pivots come from the standard skeleton so most authors never touch them.
- **Bake + preview** — #18 bakes each group; the author sees the rig play its inherited idle/walk loop immediately (reuse the #17 "Your Look" render-to-texture orbit-preview approach, `menu.rs:1863` draw_skin_panel + `egui_integration.rs` register_native_texture).

This UX is the bulk of the effort and the part that **cannot be solo-verified** — it gates on an Axolittle build session ([[feedback_autonomy_to_playtest_boundary]], [[feedback_pretest_check]]).

### Performance (owner's recurring concern — answered, same as #18)

The sub-voxel grid (1/8 or 1/16, #18) is **authoring resolution, NOT draw cost.** For every animated part:
- **Bake once** → greedy-meshed shell (interior micro-voxels culled), reusing #18's bake. A rigged part is a compact mesh (tens-to-hundreds of tris), not thousands of micro-cubes.
- **Instance** per creature — one `*Instance`-style record + one draw per (skeleton-part) type, exactly like `PlantInstance`/`upload_plant_instances`/the instanced draw at `renderer.rs:2485-2500`.
- **Animate cheaply** — rungs 1-3 are a **per-part transform/scale** (the `build_part_vertices` corner maths, already per-frame for ~17 mob types today); rung 4 is a **per-frame mesh-buffer selection** (no extra geometry).
- **LOD** — distant rigged creatures fall back to a billboard/impostor or freeze animation, mirroring #18's distant-field LOD.

**The only failure mode** (identical to #18's warning): naively drawing raw animated micro-cubes (4096/part × bones × frames → billions of tris). **Never do that.** Always bake + instance. With that rule, 100 rigged creatures on screen is comfortably within budget — the per-part transform cost is what the engine already pays for mobs.

---

## Phased scope

Each phase is independently shippable behind `./check.sh` and ends at a clear boundary; the **visual/feel/UX** halves gate on an Axolittle playtest. **All of this is BLOCKED until #18 ships** (it produces the baked geometry every phase rigs).

### Phase A — standard-skeleton data model + part-attach authoring
*Goal: a `Skeleton` is data; an author can pick one, build geometry, and attach baked voxel-groups to named parts/pivots. No animation yet — parts render in bind pose.*

- **`skeleton.rs` (new):** `struct SkeletonPart { name: &'static str, parent: Option<usize>, origin: Vec3, size_hint: Vec3, pivot: Vec3, phase: f32, animated: bool, pitch_tracks_look: bool }` — a named, parentable superset of `ModelPart` (`entity_model.rs:201-217`). `struct Skeleton { name, parts: Vec<SkeletonPart>, anim_set: AnimSetId }`. Ship the 5 standard skeletons (§Design) as `LazyLock` data, mirroring `MODEL_CACHE` (`entity_model.rs:220`). The **biped** skeleton is a direct transcription of `PLAYER_MODEL` (`entity_model.rs:1838-1911`); quadruped/bird/fish transcribe `cow_model`/`chicken_model`/`squid_model` part layouts.
- **`RiggedModel` type:** `struct RiggedModel { skeleton: SkeletonId, parts: Vec<RiggedPart> }`, `RiggedPart { skeleton_part: usize, micro_model: MicroModelId, pivot_override: Option<Vec3> }`. `MicroModelId` references a #18 baked micro-model. Serialisable (serde) so it rides Stash like `PlanData`.
- **Authoring flow (creative):** extend the #18 capture/select UX so a selected bounded volume (reuse `plan::capture`, `plan.rs:544`) can be **tagged to a skeleton part** instead of (or in addition to) becoming a standalone prop. A skeleton **guide overlay** (reuse the depth-biased decal/wireframe primitive — crack-overlay pipeline + block-highlight wireframe) shows part bounds + default pivot dots.
- **Render in bind pose:** a `build_rigged_vertices(model, pos, yaw, /* no anim */)` that, for each `RiggedPart`, transforms its baked micro-model by the skeleton part's bind transform (origin + yaw + translate) — reusing the corner→face winding of `push_textured_quad`/`build_part_vertices` (`entity_model.rs:1394`, 1549) but emitting the **baked micro-model geometry** instead of a single cuboid.

**Acceptance criteria (Phase A):**
- `./check.sh` green: clippy clean (no new warnings), native + WASM build, `cargo test` (all existing **1910** + new), `trunk build`, bundle < 5 MiB (currently 2.76 MiB).
- Unit tests: each standard skeleton has the expected named parts + non-degenerate pivots (mirror `player_model_shape`, `entity_model.rs:1947`, and `every_passive_spawn_mob_has_a_model`, 1923); a `RiggedModel` round-trips through serde; the biped skeleton's part count/pivots match `PLAYER_MODEL`.
- A `RiggedModel` built from #18 micro-models renders in bind pose through the instanced path (asserted at the vertex-builder level, headless).
- No protocol/save-format break for existing worlds (new types are additive, `#[serde(default)]` where they touch saved structs — the `PlanData` pattern at `plan.rs:188`).
- **Playtest boundary:** the attach UX (pick skeleton, select volume, tag part, place pivot) is the irreducibly-interactive part → Axolittle session.

### Phase B — animation-set inheritance (reuse the existing hierarchical transforms)
*Goal: a `RiggedModel` automatically inherits its skeleton's built-in animation set (idle/walk/attack/…) — the do-ocracy unlock — by feeding `PartPose`s into the existing transform.*

- **`AnimSet` evaluator:** `fn eval_anim_set(set: AnimSetId, clip: AnimClip, t: f32, part: &SkeletonPart) -> PartPose` — a **data/generic** version of the inline arithmetic already in `build_entity_model_vertices` (`entity_model.rs:820-825`) and `build_player_avatar_vertices` (`entity_model.rs:896-911`). Walk = `sin(t*freq + phase*TAU)*amp` per `animated` part; attack = `swing_angle` (`entity_model.rs:1377`) on the designated swing part; idle = small breathing sine; jump = the static airborne pose (transcribe 1904-1908). Returns `PartPose` (`entity_model.rs:1384`) — **the same struct the renderer already consumes.**
- **Hierarchy composition:** generalise `build_part_vertices` (or a rigged twin) to compose a part's transform **in its parent's already-posed frame** (parent rotation/translation accumulated down the `parent` chain). The biped/quad are 1-deep (limbs off body) so this is the existing single-rotation path; the chain support is what rung-3 (Phase C) and the swaying-plant skeleton need.
- **Drive it:** the rigged renderer picks the clip from entity state (locomotion: idle/walk/jump like `PlayerState.anim_state`, `protocol.rs:185`; attack from a swing timer like `SWING_TICKS`, `entity_model.rs:1375`) and calls `eval_anim_set` per part → `build_rigged_vertices` with the posed transform. **A re-skinned biped now walks using `PLAYER_MODEL`'s exact gait, for free.**

**Acceptance criteria (Phase B):**
- `./check.sh` green (as Phase A).
- Unit tests: `eval_anim_set(walk, …)` produces alternating-phase swings matching the avatar's today (port `arm_override_replaces_walk_swing`, `entity_model.rs:2110`, and `swing_curve_bounds`, 1957); attack clip routes `swing_angle` to the correct part and overrides walk (mirror the avatar's `arm_override` precedence at `entity_model.rs:912`); a 2-deep parent chain composes (child follows posed parent — assert corner positions differ from the 1-deep case).
- A biped `RiggedModel` and `PLAYER_MODEL` produce **equivalent gait poses** for the same `t` (regression anchor: the standard skeleton really is the avatar's motion).
- **Playtest boundary:** "does the inherited walk *read right* on a custom-built creature" is visual → Axolittle.

### Phase C — squash/stretch + segmented-bend (scale channel)
*Goal: optional non-uniform scale keyframes (rung 2) and blocky segmented bend (rung 3) as extra channels — opt-in per part, default off.*

- **Scale channel:** add `scale: Vec3` to `PartPose` (`entity_model.rs:1384`, default `Vec3::ONE`); in `build_part_vertices`/the rigged twin, multiply each corner's offset-from-pivot by `scale` **before** the X-rotation (insert right before the rotation block at `entity_model.rs:1424`). Animation sets gain optional scale keyframes (bounce/breathe/land-squash).
- **Segmented bend:** the swaying-plant skeleton (and any author chain) uses Phase B's parent composition with a small per-segment rotation that increases up the chain (wind-phase sine) — **pure rung-1 per segment**, no new transform. Ship the **sway** animation set for the plant skeleton.

**Acceptance criteria (Phase C):**
- `./check.sh` green; default `scale = ONE` leaves **all existing mob/avatar output byte-identical** (regression test: a part with `PartPose::default()` is unchanged vs pre-Phase-C — this protects the 1910-test baseline and the live avatar/mobs).
- Unit tests: non-uniform scale changes corner spread but not the pivot point; a segmented chain with rising per-segment angle bends monotonically (tip displaces more than base).
- **Playtest boundary:** squash *feel* + sway *look* are visual → Axolittle.

### Phase D — animation-frame flipbook (3D "sprite swap")
*Goal: per-part flipbook of baked micro-model frames for non-rigid changes (rung 4).*

- **Multi-frame parts:** `RiggedPart` gains `frames: Vec<MicroModelId>` (1 = static). An `AnimClip` can carry a **frame-index track** per part (e.g. mouth open/closed at 2 fps).
- **Render selection:** the rigged renderer selects `frames[frame_index]`'s baked instanced mesh for the current frame — a **buffer selection**, mirroring how `upload_plant_instances`/the instanced draw (`renderer.rs:1011`, 2485) pick a chunk's plant buffer. No interpolation, no extra per-frame geometry.
- **Authoring:** the author bakes N frames of the part (build each pose, capture, bake via #18) and orders them in a clip. Optional channel — most rigs use 1 frame.

**Acceptance criteria (Phase D):**
- `./check.sh` green; single-frame parts behave exactly as Phase A-C (regression).
- Unit tests: frame-index track advances at the clip's rate and wraps; a 2-frame part selects the correct baked mesh per index.
- **Playtest boundary:** flipbook *reads as motion* (chomp/flap) → Axolittle.

### Later (separate spec, DEFERRED) — bespoke per-creature animation
Author-defined motion curves (custom keyframes per part, not an inherited standard set) and **true vertex skinning** (bones + per-vertex weights for smooth, non-blocky deformation). This is the advanced long tail; the standard-skeleton inheritance (Phases A-D) covers the do-ocracy case. Its own foundation doc when prioritised. Phases A-D are shaped so this is **additive** (a custom `AnimSet` + an optional skinning path), not a rewrite.

---

## Risk / confidence

- **Overall: XL effort, Medium confidence.** The owner-inbox triage (`docs/backlog/owner-inbox.md:39`) rates #19 XL — correct. But the **motion-maths risk is LOW**: rung 1 ships and is tested; rungs 2-4 are small, well-understood additions (a scale mul, a parent-frame compose, a buffer index). The **bake/perf risk is LOW** *conditional on #18* — the instance pattern is proven by plants (`mesh.rs:871`). The **real risk is the authoring UX** (Medium): "build geometry → tag parts → place pivots → preview" is a genuinely new, interaction-heavy surface that **cannot be solo-verified** and will need iteration with Axolittle.
- **Hard dependency on #18.** Nothing here is buildable until #18's bake pipeline lands and is playtested. If #18 changes its micro-model representation, `RiggedPart.micro_model` follows. **Don't start #19 before #18 merges** ([[feedback_pr_chains_on_spec_branches]] — branch off #18 if it's not yet on main).
- **Regression risk to the live avatar/mobs.** Phases B/C touch shared code (`PartPose`, `build_part_vertices`) that drives the *current* player + ~17 mobs. Mitigation: every shared-code change is gated by a **"default pose / default scale = byte-identical output"** regression test, and the biped skeleton is asserted equivalent to `PLAYER_MODEL`'s gait. The 1910-test baseline is the tripwire.
- **Confidence by phase:** A High (data structuring + reuse of capture), B High (transcribing existing arithmetic into a generic evaluator), C High (tiny transform additions), D Medium (frame-selection plumbing is straightforward; the *authoring* of frames is the unknown), bespoke/skinning Low (deferred for a reason).
- **Single-player-verifiable up to the playtest boundary.** Vertex-builder + anim-set logic is unit-testable headlessly; everything visual/feel/UX is the Axolittle boundary ([[feedback_autonomy_to_playtest_boundary]]).

---

## Deferred / out of scope

- **Bespoke per-creature keyframed animation** — the advanced "author your own motion curves" path. Standard-skeleton inheritance is the do-ocracy case; bespoke is the long tail. Own later spec.
- **True vertex skinning (bones + weights)** — smooth deformation; heavier and less blocky (against art direction). Segmented-chain bend (rung 3, Phase C) covers the blocky need. Own later spec.
- **Curation / "creature show" competition mechanic + sats prizes** — same posture as #18's deferred "flower show" (owner: "good problem to have, not now"). Ties into `docs/vision/economies-long-run.md` (spectator/creation economies) + Proof-of-Play when designed. **Do NOT design now.**
- **Creator-sold rigged creatures (Bitcoin)** — the eventual strategic payoff (sell a rigged creature direct over Lightning, no marketplace cut, owned by the buyer's persona via Stash). Needs the cosmetics/asset-as-purchasable-asset format + a buy flow + **UGC moderation/terms**. Sketched, not built — consistent with [[project_wallet_escrow_settlement_model]] / [[project_settlement_model_decision_parked]] (don't pre-commit a settlement model here).
- **Networked multiplayer of authored rigs** — broadcasting a custom creature's identity/state to other clients is the same wire/per-instance-asset problem the cosmetics Phase 3 hits (`docs/foundations/2026-06-02-standard-avatar-and-byo-skins.md` §The prize, piece C; `PlayerState` has no model field, `protocol.rs:175`). Out of scope; rides the multiplayer-identity work (Spec 1 Phase 4) when it lands.
- **Physics-driven motion** (ragdoll, IK, cloth) — not on the ladder; not now.
- **Animating #18's existing static props** as a migration — possible later (give a flower the swaying-plant skeleton) but not a goal of this spec.

---

## File-touch map (when built — NOT now)

| File | Phase | What |
|------|------|------|
| `game/engine/src/skeleton.rs` (new) | A | `SkeletonPart` / `Skeleton` / `RiggedModel` / `RiggedPart` + the 5 standard skeletons (`LazyLock`, mirroring `MODEL_CACHE`) + serde + tests |
| `game/engine/src/entity_model.rs` | A-C | bind-pose + posed rigged vertex builder reusing `build_part_vertices` winding (1394); `PartPose` gains `scale` (C, at 1384); hierarchy/parent composition (B) |
| `game/engine/src/anim_set.rs` (new) | B | `AnimSet`/`AnimClip` + `eval_anim_set` (generic port of the inline walk/jump/attack arithmetic at `entity_model.rs:820`, 896, `swing_angle` 1377) + tests |
| skeleton-attach authoring UX (creative; new module + `plan_ui.rs`/menu hooks) | A | pick skeleton, tag selected volume → part (reuse `plan::capture`, `plan.rs:544`), place pivots, guide overlay (reuse decal/wireframe primitive) |
| rig preview (reuse #17 render-to-texture, `menu.rs:1863` + `egui_integration.rs`) | A | in-creative orbit preview playing the inherited idle/walk loop |
| `game/engine/src/renderer.rs` | A, D | rigged-part instanced draw (mirror plant path 1011 / 2485); per-frame mesh selection (D) |
| `game/engine/src/mesh.rs` | A | rigged-part instance record (mirror `PlantInstance` 66-91) if a dedicated instance format is needed |
| `docs/spec/03-rendering.md`, `docs/spec/05-gameplay-systems.md` | A-D | new skeleton/rig/animation-authoring sections ([[spec maintenance]]) |

*(All new types are additive + `#[serde(default)]` where they touch saved structs — the `PlanData` forward-compat pattern at `plan.rs:158-204`. Reuses #18's micro-model bake; does not re-derive capture.)*

---

## Memory-rule check

- **[[reference_foundations_queue]] / owner-inbox #19:** this is the foundation doc the inbox calls for ("XL; defer behind #18, own foundation doc later"). Expanded from the note, not beyond it. ✅
- **Sequencing:** explicitly **behind #18** (static micro-models) and **blocked until #18 ships** — the prerequisite that bakes the geometry this rigs. ✅
- **[[feedback_merge_to_main_preauthorised]]:** when built, healthy-gate (`check.sh` green) merges to main are pre-authorised; each phase holds the 1910-test / 2.76-MiB baseline. Design-only now — nothing to merge. ✅
- **[[feedback_autonomy_to_playtest_boundary]] / [[feedback_pretest_check]]:** logic is solo-verifiable; the authoring UX + all visual/feel confirmation is the Axolittle playtest boundary — build to it, then stop cleanly. ✅
- **[[project_shared_infra_strategy]]:** skeletons/rigs are generic (biped/quad/bird/fish/plant), ride Stash as per-persona assets like #18 packs, and reuse cross-game primitives (capture, bake, instance, render-to-texture preview) — no AxeNStax-specific assumptions baked into the rig format. ✅
- **[[project_economies_vision]] / Bitcoin posture:** creator-sold rigged creatures sketched as the deferred payoff (direct Lightning, no marketplace cut, persona-owned) — consistent, not designed here; settlement model stays [[project_settlement_model_decision_parked]]. ✅
- **[[reference_proof_of_play_is_proof_of_work]]:** any future "creature show" prize ties to Proof-of-Play/spectator economy — deferred, untouched here. ✅
- **UK English** throughout; **no Mojang IP** (the skeleton/rig concept and our own art only — same posture as the cosmetics IP §); **npub never hex** in user-facing surfaces. ✅
- **[[project_launch_scope_axenstax_only]]:** only AxeNStax launches; the rig system is designed platform-generic but isn't a launch-gating feature — sequenced well behind the alpha. ✅